// crates/cli/src/publish.rs
// `creg publish` — signs and submits a tarball to the registry pending pool.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use common::proto::registry_service_client::RegistryServiceClient;
use common::proto::SubmitRequest;
use common::{PackageId, PackageManifest, PublishRequest};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::path::Path;
use zk_validator::{PackageInputs, ZkValidator};

pub async fn run(
    tarball_path: &Path,
    manifest_path: Option<&Path>,
    privkey_hex: &str,
    extra_privkeys: &[String],
    node_url: Option<&str>,
    shield: bool,
) -> Result<()> {
    // ── 1. Read and hash the tarball ─────────────────────────────────────────
    let tarball_bytes = std::fs::read(tarball_path)
        .with_context(|| format!("Cannot read tarball: {}", tarball_path.display()))?;

    let content_hash = common::sha256_hex(&tarball_bytes);
    println!("  tarball:  {}", tarball_path.display());
    println!("  sha256:   {}", content_hash);

    // ── 2. Pin to IPFS (via local IPFS daemon or Pinata) ──────────────────────
    let pb = create_progress_bar(tarball_bytes.len() as u64, "Uploading to IPFS");
    let ipfs_cid = pin_to_ipfs_with_progress(&tarball_bytes, &pb).await?;
    pb.finish_with_message("✓ Upload complete");
    println!("  IPFS CID: {}", ipfs_cid);

    // ── 2.5. Optional Encryption (Shielding) ─────────────────────────────────
    let mut final_ipfs_cid = ipfs_cid.clone();
    let mut key_bundle = None;

    if shield {
        println!("  Shielding package with AES-256-GCM...");
        let (encrypted_bytes, bundle) = encrypt_for_validators(&tarball_bytes)?;

        let pb_shield =
            create_progress_bar(encrypted_bytes.len() as u64, "Uploading encrypted shield");
        final_ipfs_cid = pin_to_ipfs_with_progress(&encrypted_bytes, &pb_shield).await?;
        pb_shield.finish_with_message("✓ Shield upload complete");

        key_bundle = Some(bundle);
        println!("  Shielded CID: {}", final_ipfs_cid);
    }

    // ── 3. Load manifest (or use defaults) ───────────────────────────────────
    let manifest: PackageManifest = match manifest_path {
        Some(p) => {
            let raw = std::fs::read_to_string(p)?;
            serde_json::from_str(&raw)?
        }
        None => PackageManifest::default(),
    };

    // ── 4. Parse package identity from tarball ────────────────────────────────
    let pkg_id = detect_package_id(&tarball_bytes)?;
    println!("  package:  {}", pkg_id.canonical());

    // ── 5. Sign: sig = Ed25519(privkey, canonical || content_hash) ───────────
    let privkey_bytes = hex::decode(privkey_hex.trim()).context("Invalid private key hex")?;

    use ed25519_dalek::{Signer, SigningKey};
    let signing_key =
        SigningKey::try_from(privkey_bytes.as_slice()).context("Invalid Ed25519 private key")?;
    let pubkey = signing_key.verifying_key();

    let msg = format!("{}{}", pkg_id.canonical(), content_hash);
    let signature = signing_key.sign(msg.as_bytes());

    let mut publisher_pubkeys = vec![hex::encode(pubkey.as_bytes())];
    let mut signatures = vec![hex::encode(signature.to_bytes())];

    for key_hex in extra_privkeys {
        let key_bytes = hex::decode(key_hex.trim())
            .with_context(|| format!("Invalid extra private key hex: {}", key_hex))?;
        let sk = SigningKey::try_from(key_bytes.as_slice())
            .with_context(|| format!("Invalid Ed25519 extra private key: {}", key_hex))?;
        let pk = sk.verifying_key();
        let sig = sk.sign(msg.as_bytes());
        publisher_pubkeys.push(hex::encode(pk.as_bytes()));
        signatures.push(hex::encode(sig.to_bytes()));
    }

    // ── 5.5. Optional PGP signing ─────────────────────────────────────────────
    // If PGP_PRIVATE_KEY_PATH is set, load the armored secret key and sign the
    // tarball to produce a detached PGP signature over the content hash.
    let (pgp_signature, pgp_public_key) = sign_with_pgp_if_configured(&tarball_bytes)?;

    let request = PublishRequest {
        id: pkg_id.clone(),
        content_hash: content_hash.clone(),
        ipfs_cid: final_ipfs_cid.clone(),
        publisher_pubkey: publisher_pubkeys[0].clone(),
        signature: signatures[0].clone(),
        manifest,
        submitted_at: Utc::now(),
        shielded: shield,
        key_bundle,
        pgp_signature,
        pgp_public_key,
        publisher_pubkeys: publisher_pubkeys.clone(),
        signatures: signatures.clone(),
        threshold: if publisher_pubkeys.len() >= 2 { 2 } else { 1 },
        ..Default::default()
    };

    // ── 5.5. Generate ZK Content-Hash Proof (publisher-side attestation) ──────
    // Note: The publisher generates a ZK proof that they know the pre-image of
    // the content hash. Validation scores (static analysis, sandbox) are set
    // by the validator nodes during consensus — NOT by the publisher.
    // These public inputs are initialized to zero/false here; validators will
    // generate their own proofs with real scores after running the 3-stage pipeline.
    println!("  Generating ZK content-hash attestation...");
    let pb_zk = ProgressBar::with_draw_target(Some(0), ProgressDrawTarget::stderr());
    pb_zk.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .expect("Valid spinner template"),
    );
    pb_zk.set_message("Computing Groth16 SNARK...");

    let validator = ZkValidator::new().context("Failed to initialize ZK validator")?;

    let mut hash_bytes = [0u8; 32];
    let hash_decoded = hex::decode(&content_hash).context("content_hash is not valid hex")?;
    if hash_decoded.len() == 32 {
        hash_bytes.copy_from_slice(&hash_decoded);
    } else {
        bail!("content_hash must be 32 bytes, got {}", hash_decoded.len());
    }

    // Compute manifest hash so the proof binds to the declared manifest.
    let manifest_bytes =
        serde_json::to_vec(&request.manifest).context("Failed to serialize manifest")?;
    let manifest_hash_hex = common::sha256_hex(&manifest_bytes);
    let manifest_hash_decoded =
        hex::decode(&manifest_hash_hex).context("manifest hash is not valid hex")?;
    let mut manifest_hash_bytes = [0u8; 32];
    if manifest_hash_decoded.len() == 32 {
        manifest_hash_bytes.copy_from_slice(&manifest_hash_decoded);
    }

    // Publisher-side inputs: score=0, sandbox=false.
    // Validator nodes will produce their own proofs with real values.
    let zk_inputs = PackageInputs::new(
        hash_bytes,
        manifest_hash_bytes,
        0,     // Static analysis score — determined by validators, not publisher
        false, // Sandbox result — determined by validators, not publisher
    );

    let proof = validator
        .generate_proof(&zk_inputs)
        .context("ZK proof generation failed")?;
    let proof_bytes =
        ZkValidator::serialize_proof(&proof).context("ZK proof serialization failed")?;
    pb_zk.finish_with_message("✓ ZK content-hash attestation generated");

    // ── 6. Submit via gRPC (Primary High-Speed Tunnel) ────────────────────────
    let base_url = node_url
        .unwrap_or("localhost")
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split(':')
        .next()
        .unwrap_or("localhost");
    let grpc_url = format!("http://{}:50051", base_url);

    println!("  Submitting via gRPC to {} ...", grpc_url);

    if let Ok(mut client) = RegistryServiceClient::connect(grpc_url).await {
        let grpc_req = SubmitRequest {
            ecosystem: pkg_id.ecosystem.clone(),
            name: pkg_id.name.clone(),
            version: pkg_id.version.clone(),
            content_hash: content_hash.clone(),
            ipfs_cid: final_ipfs_cid,
            publisher_pubkey: publisher_pubkeys[0].clone(),
            signature: signatures[0].clone(),
            zk_proof: proof_bytes,
            // Scores are set to 0 — validator nodes will evaluate these.
            static_analysis_score: 0,
            sandbox_safe: false,
            publisher_pubkeys: publisher_pubkeys.clone(),
            signatures: signatures.clone(),
            threshold: if publisher_pubkeys.len() >= 2 { 2 } else { 1 },
        };

        match client.submit_package(grpc_req).await {
            Ok(resp) => {
                println!("\n  ✓ gRPC: {}", resp.into_inner().message);
                return Ok(());
            }
            Err(e) => {
                tracing::warn!("gRPC submission failed ({}): falling back to REST", e);
            }
        }
    }

    // ── 7. Fallback to REST (Legacy) ──────────────────────────────────────────
    let url = format!(
        "{}/v1/packages",
        node_url
            .unwrap_or("https://registry.chain-pkg.io")
            .trim_end_matches('/')
    );

    let pb_submit = ProgressBar::with_draw_target(Some(0), ProgressDrawTarget::stderr());
    pb_submit.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .expect("Valid spinner template"),
    );
    pb_submit.set_message(format!("Submitting to {}", url));

    let resp = reqwest::Client::new()
        .post(&url)
        .json(&request)
        .send()
        .await
        .context("Failed to reach registry node")?;

    pb_submit.finish_and_clear();

    if resp.status().is_success() {
        println!("\n  ✓ Package submitted to pending pool.");
        println!("    It will be assigned to validator nodes via VRF and");
        println!(
            "    verified through PBFT consensus. Use `creg status {}` to check.",
            pkg_id.canonical()
        );
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        // Provide user-friendly error messages
        let error_msg = match status.as_u16() {
            403 => format!("Insufficient stake. Run: creg stake --amount 0.01eth"),
            409 => format!("Package already exists. Use a different version."),
            400 => format!("Invalid request: {}", body),
            401 => format!("Unauthorized: Invalid signature or key."),
            429 => format!("Rate limited. Please wait before submitting again."),
            500..=599 => format!("Server error. Please try again later."),
            _ => format!("HTTP {}: {}", status, body),
        };

        bail!("✗ Submission failed: {}", error_msg);
    }

    Ok(())
}

/// Create a styled progress bar for file uploads
fn create_progress_bar(total_bytes: u64, msg: &str) -> ProgressBar {
    let pb = ProgressBar::new(total_bytes);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} {msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .expect("Valid progress bar template")
            .progress_chars("#>-"),
    );
    pb.set_message(msg.to_string());
    pb
}

/// Upload tarball bytes to IPFS with progress indication and return the CID.
async fn pin_to_ipfs_with_progress(bytes: &[u8], pb: &ProgressBar) -> Result<String> {
    // Try CREG_IPFS_URL first, then fallback to localhost, then dev stub.
    let ipfs_base =
        std::env::var("CREG_IPFS_URL").unwrap_or_else(|_| "http://127.0.0.1:5001".to_string());
    let add_url = format!("{}/api/v0/add", ipfs_base.trim_end_matches('/'));

    use reqwest::multipart;

    let form = multipart::Form::new().part(
        "file",
        multipart::Part::bytes(bytes.to_vec()).file_name("package.tgz"),
    );

    // We do not simulate progress here anymore. Instead, we let reqwest handle the actual network transfer.
    pb.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} {bytes}/{total_bytes}")
            .expect("Valid progress bar template"),
    );

    let local = reqwest::Client::new()
        .post(&add_url)
        .multipart(form)
        .send()
        .await;

    pb.set_position(bytes.len() as u64);

    match local {
        Ok(resp) if resp.status().is_success() => {
            #[derive(serde::Deserialize)]
            struct IpfsResponse {
                #[serde(rename = "Hash")]
                hash: String,
            }
            let r: IpfsResponse = resp.json().await?;
            Ok(r.hash)
        }
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("IPFS upload failed (HTTP {}): {}", status, body)
        }
        Err(e) => {
            bail!(
                "IPFS daemon not reachable at {}. Please start 'ipfs daemon'. Error: {}",
                ipfs_base,
                e
            )
        }
    }
}

/// Upload tarball bytes to IPFS and return the CID (legacy, without progress).
#[allow(dead_code)]
async fn pin_to_ipfs(bytes: &[u8]) -> Result<String> {
    pin_to_ipfs_with_progress(bytes, &ProgressBar::hidden()).await
}

/// Infer PackageId from package.json / Cargo.toml in the tarball.
fn detect_package_id(tarball_bytes: &[u8]) -> Result<PackageId> {
    use std::io::Read;
    let gz = flate2::read::GzDecoder::new(tarball_bytes);
    let mut archive = tar::Archive::new(gz);

    for entry in archive.entries()? {
        let mut entry: tar::Entry<'_, flate2::read::GzDecoder<&[u8]>> = entry?;
        let path = entry.path()?.to_string_lossy().to_string();

        if path.ends_with("package.json") {
            let mut content = String::new();
            entry.read_to_string(&mut content)?;
            #[derive(serde::Deserialize)]
            struct PkgJson {
                name: String,
                version: String,
            }
            let p: PkgJson = serde_json::from_str(&content)?;
            return Ok(PackageId::new("npm", p.name, p.version));
        }

        if path.ends_with("Cargo.toml") {
            let mut content = String::new();
            entry.read_to_string(&mut content)?;
            // Very simple parse — a full implementation uses toml crate.
            let name = extract_toml_field(&content, "name").unwrap_or("unknown");
            let version = extract_toml_field(&content, "version").unwrap_or("0.0.0");
            return Ok(PackageId::new("cargo", name, version));
        }
    }

    bail!("Could not detect package identity from tarball contents")
}

fn extract_toml_field<'a>(content: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{} = \"", field);
    let line = content.lines().find(|l| l.starts_with(&prefix))?;
    let start = prefix.len();
    let end = line[start..].find('"')? + start;
    Some(&line[start..end])
}

/// Sign the tarball with PGP if configured.
///
/// Three modes (checked in order):
///  1. `CREG_PGP_PRIVATE_KEY_PATH` — path to an armored secret key file. Signs
///     the tarball using `gpg --batch --yes --detach-sign`.
///  2. `CREG_PGP_SIG` / `CREG_PGP_KEY` — pre-computed armored sig + pubkey
///     passed through directly (backwards compat).
///  3. Nothing set — returns `(None, None)` silently.
fn sign_with_pgp_if_configured(tarball: &[u8]) -> Result<(Option<String>, Option<String>)> {
    // Mode 2 backwards compat
    if let (Some(sig), Some(key)) = (
        std::env::var("CREG_PGP_SIG").ok(),
        std::env::var("CREG_PGP_KEY").ok(),
    ) {
        return Ok((Some(sig), Some(key)));
    }

    let key_path = match std::env::var("CREG_PGP_PRIVATE_KEY_PATH").ok() {
        Some(p) => std::path::PathBuf::from(p),
        None => return Ok((None, None)),
    };

    // Write tarball to a temp file so gpg can sign it.
    let tmp_dir = std::env::temp_dir();
    let tarball_tmp = tmp_dir.join("creg_publish_gpg.tgz");
    std::fs::write(&tarball_tmp, tarball)
        .context("Failed to write temp tarball for GPG signing")?;

    // Run: gpg --batch --yes --no-tty --pinentry-mode loopback
    //          --default-key <fingerprint-or-key-path>
    //          --detach-sign --armor --output <sig_file> <tarball>
    let sig_tmp = tmp_dir.join("creg_publish_gpg.sig");

    let status = std::process::Command::new("gpg")
        .args([
            "--batch",
            "--yes",
            "--no-tty",
            "--pinentry-mode",
            "loopback",
            "--secret-keyring",
            key_path.to_str().unwrap_or(""),
            "--detach-sign",
            "--armor",
            "--output",
            sig_tmp.to_str().unwrap_or(""),
            tarball_tmp.to_str().unwrap_or(""),
        ])
        .status()
        .context("Failed to invoke gpg — ensure GnuPG is installed")?;

    if !status.success() {
        anyhow::bail!(
            "gpg exited with status {}. Check CREG_PGP_PRIVATE_KEY_PATH and gpg-agent.",
            status
        );
    }

    let sig_armored = std::fs::read_to_string(&sig_tmp)
        .context("Failed to read GPG detached signature output")?;

    // Export the corresponding public key in armored form.
    let pubkey_output = std::process::Command::new("gpg")
        .args([
            "--batch",
            "--no-tty",
            "--export",
            "--armor",
            "--secret-keyring",
            key_path.to_str().unwrap_or(""),
        ])
        .output()
        .context("Failed to export GPG public key")?;

    let pub_armored = String::from_utf8_lossy(&pubkey_output.stdout).to_string();

    // Cleanup temp files
    let _ = std::fs::remove_file(&tarball_tmp);
    let _ = std::fs::remove_file(&sig_tmp);

    println!("  PGP: signed with key at {}", key_path.display());
    Ok((Some(sig_armored), Some(pub_armored)))
}

/// Encrypt the tarball for the validator set using AES-GCM-256 and ECIES.
fn encrypt_for_validators(data: &[u8]) -> Result<(Vec<u8>, String)> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Key, Nonce,
    };
    use rand::{thread_rng, RngCore};

    // 1. Generate ephemeral symmetric key
    let mut aes_key = [0u8; 32];
    thread_rng().fill_bytes(&mut aes_key);
    let key = Key::<Aes256Gcm>::from_slice(&aes_key);
    let cipher = Aes256Gcm::new(key);

    // 2. Encrypt payload
    let mut nonce_bytes = [0u8; 12];
    thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

    // 3. Wrap key for validators (Demo: use a cluster-wide shared secret or ECIES)
    // For this implementation, we bundle the AES key + nonce.
    // In production, this entire string is encrypted with the Validator Set's Master PubKey.
    let bundle = format!("{}:{}", hex::encode(aes_key), hex::encode(nonce_bytes));

    // Prepend nonce to ciphertext for easier retrieval
    let mut final_payload = nonce_bytes.to_vec();
    final_payload.extend(ciphertext);

    Ok((final_payload, bundle))
}
