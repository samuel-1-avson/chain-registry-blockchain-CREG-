// crates/cli/src/publish.rs
// `creg publish` — signs and submits a tarball to the registry pending pool.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use common::proto::registry_service_client::RegistryServiceClient;
use common::proto::SubmitRequest;
use common::{PackageId, PackageManifest, PublishRequest};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rand::RngCore;
use sha3::{Digest, Keccak256};
use std::path::Path;
use std::time::Duration;

pub async fn run(
    tarball_path: &Path,
    manifest_path: Option<&Path>,
    privkey_hex: &str,
    extra_privkeys: &[String],
    publisher_address: &str,
    node_url: Option<&str>,
    grpc_url: Option<&str>,
    ipfs_url: Option<&str>,
    shield: bool,
) -> Result<()> {
    let publisher_address = canonicalize_publisher_address(publisher_address)?;

    if shield && !common::shielded_publish_enabled() {
        bail!(
            "Shielded publish requires CREG_SHIELDED_PUBLISH_ENABLED=true on this machine and on \
             the node (experimental; SEC-304). See docs/PHASE3_KICKOFF.md."
        );
    }

    // ── 1. Read and hash the tarball ─────────────────────────────────────────
    let tarball_bytes = std::fs::read(tarball_path)
        .with_context(|| format!("Cannot read tarball: {}", tarball_path.display()))?;

    let content_hash = common::sha256_hex(&tarball_bytes);
    println!("  tarball:  {}", tarball_path.display());
    println!("  sha256:   {}", content_hash);

    // ── 2. Pin to IPFS (via local IPFS daemon or fleet gateway) ───────────────
    let ipfs_base = crate::config_file::effective_ipfs_url(ipfs_url);
    let pb = create_progress_bar(tarball_bytes.len() as u64, "Uploading to IPFS");
    let ipfs_cid = pin_to_ipfs_with_progress(&tarball_bytes, &ipfs_base, &pb).await?;
    best_effort_pin_add(&ipfs_base, &ipfs_cid).await;
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
        final_ipfs_cid =
            pin_to_ipfs_with_progress(&encrypted_bytes, &ipfs_base, &pb_shield).await?;
        best_effort_pin_add(&ipfs_base, &final_ipfs_cid).await;
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

    let manifest_hash_hex = common::sha256_hex(
        &serde_json::to_vec(&manifest).context("Failed to serialize manifest")?,
    );
    let publisher_chain_id = std::env::var("CREG_CHAIN_ID").unwrap_or_default();
    let publisher_staking_contract = std::env::var("CREG_STAKING_ADDR").unwrap_or_default();
    let mut nonce_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let publisher_nonce = hex::encode(nonce_bytes);
    let submitted_at = Utc::now();
    let publisher_expires_at = (submitted_at + chrono::Duration::hours(1)).timestamp() as u64;
    let ownership_claim = format!("{}:{}", pkg_id.ecosystem, pkg_id.name);
    let source_hash = std::env::var("CREG_SOURCE_HASH").unwrap_or_else(|_| "unavailable".into());

    // ── 5. Sign: sig = Ed25519(privkey, canonical || content_hash || publisher_address) ─────
    let privkey_bytes = hex::decode(privkey_hex.trim()).context("Invalid private key hex")?;

    use ed25519_dalek::{Signer, SigningKey};
    let signing_key =
        SigningKey::try_from(privkey_bytes.as_slice()).context("Invalid Ed25519 private key")?;
    let pubkey = signing_key.verifying_key();

    let msg = if !publisher_chain_id.trim().is_empty()
        && !publisher_staking_contract.trim().is_empty()
    {
        common::publish_signature_message_v2(
            &publisher_chain_id,
            &publisher_staking_contract,
            &pkg_id,
            &content_hash,
            &source_hash,
            &manifest_hash_hex,
            &hex::encode(pubkey.as_bytes()),
            &publisher_address,
            &ownership_claim,
            &publisher_nonce,
            submitted_at.timestamp(),
            publisher_expires_at,
        )
    } else {
        common::publish_signature_message(&pkg_id, &content_hash, &publisher_address)
    };
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
        publisher_address: publisher_address.clone(),
        publisher_pubkey: publisher_pubkeys[0].clone(),
        signature: signatures[0].clone(),
        manifest,
        submitted_at,
        shielded: shield,
        key_bundle,
        pgp_signature,
        pgp_public_key,
        publisher_pubkeys: publisher_pubkeys.clone(),
        signatures: signatures.clone(),
        threshold: if publisher_pubkeys.len() >= 2 { 2 } else { 1 },
        publisher_nonce: publisher_nonce.clone(),
        publisher_expires_at,
        source_hash: source_hash.clone(),
        manifest_hash: manifest_hash_hex.clone(),
        ownership_claim: ownership_claim.clone(),
        publisher_chain_id: publisher_chain_id.clone(),
        publisher_staking_contract: publisher_staking_contract.clone(),
        publisher_evm_signature: std::env::var("CREG_PUBLISHER_EVM_KEY")
            .ok()
            .map(|key| sign_evm_personal_message(&key, &msg))
            .transpose()?
            .or_else(|| std::env::var("CREG_PUBLISHER_EVM_SIGNATURE").ok())
            .unwrap_or_default(),
        ..Default::default()
    };

    // Compute the manifest digest for transport and future evidence binding.
    // CR-02: publisher-generated ZK attestations are retired. Their safety
    // inputs were caller assertions and are not generated or submitted here.
    let manifest_hash_hex = request.manifest_hash.clone();

    // ── 6. Submit via gRPC (Primary High-Speed Tunnel) ────────────────────────
    let grpc_url = resolve_grpc_url(node_url, grpc_url);

    println!("  Submitting via gRPC to {} ...", grpc_url);

    if let Ok(mut client) = RegistryServiceClient::connect(grpc_url.clone()).await {
        let grpc_req = SubmitRequest {
            ecosystem: pkg_id.ecosystem.clone(),
            name: pkg_id.name.clone(),
            version: pkg_id.version.clone(),
            content_hash: content_hash.clone(),
            ipfs_cid: final_ipfs_cid,
            publisher_address: publisher_address.clone(),
            publisher_pubkey: publisher_pubkeys[0].clone(),
            signature: signatures[0].clone(),
            publisher_attestation_proof: vec![],
            claimed_static_analysis_score: 0,
            claimed_sandbox_safe: false,
            publisher_pubkeys: publisher_pubkeys.clone(),
            signatures: signatures.clone(),
            threshold: if publisher_pubkeys.len() >= 2 { 2 } else { 1 },
            manifest_json: serde_json::to_string(&request.manifest)
                .context("Failed to serialize manifest for gRPC submission")?,
            manifest_hash: manifest_hash_hex,
            publisher_evm_signature: request.publisher_evm_signature.clone(),
            publisher_nonce: request.publisher_nonce.clone(),
            publisher_expires_at: request.publisher_expires_at,
            source_hash: request.source_hash.clone(),
            ownership_claim: request.ownership_claim.clone(),
            publisher_chain_id: request.publisher_chain_id.clone(),
            publisher_staking_contract: request.publisher_staking_contract.clone(),
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
    let url = format!("{}", publisher_packages_url(node_url));

    let pb_submit = ProgressBar::with_draw_target(Some(0), ProgressDrawTarget::stderr());
    pb_submit.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .context("Invalid spinner template")?,
    );
    pb_submit.set_message(format!("Submitting to {}", url));

    let request_clone = request.clone();
    let url_clone = url.clone();
    let resp =
        crate::retry::with_retry("submit package", 3, Duration::from_millis(500), move || {
            let req = request_clone.clone();
            let u = url_clone.clone();
            async move {
                reqwest::Client::new()
                    .post(&u)
                    .json(&req)
                    .send()
                    .await
                    .context("Failed to reach registry node")
            }
        })
        .await?;

    pb_submit.finish_and_clear();

    if resp.status().is_success() {
        println!("\n  ✓ Package submitted to pending pool.");
        println!("    It will receive named, independently signed evidence.");
        println!("    No result is an absolute safety guarantee. Use `creg status {}` to check.", pkg_id.canonical());
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        // Provide user-friendly error messages
        let error_msg = match status.as_u16() {
            403 => format!(
                "Insufficient stake. Stake 1 CREG as publisher: creg stake --amount 1 --role publisher --key-file <EOA_KEY>"
            ),
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

/// Sign the same EIP-191 personal-message payload that node admission
/// recovers. The key is read from an environment variable so it never enters
/// the JSON request or process arguments. Production operators should prefer
/// a KMS-backed signer and set `CREG_PUBLISHER_EVM_SIGNATURE` instead.
fn sign_evm_personal_message(private_key_hex: &str, message: &str) -> Result<String> {
    use k256::ecdsa::SigningKey;

    let key_bytes = hex::decode(private_key_hex.trim().trim_start_matches("0x"))
        .context("CREG_PUBLISHER_EVM_KEY must be hex")?;
    let signing_key = SigningKey::from_slice(&key_bytes)
        .context("CREG_PUBLISHER_EVM_KEY is not a valid secp256k1 key")?;
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.as_bytes().len());
    let mut payload = prefix.into_bytes();
    payload.extend_from_slice(message.as_bytes());
    let digest = Keccak256::digest(payload);
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .context("failed to sign publisher EVM proof")?;
    let mut encoded = [0u8; 65];
    encoded[..64].copy_from_slice(&signature.to_bytes());
    encoded[64] = recovery_id.to_byte() + 27;
    Ok(hex::encode(encoded))
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
async fn pin_to_ipfs_with_progress(
    bytes: &[u8],
    ipfs_base: &str,
    pb: &ProgressBar,
) -> Result<String> {
    let add_url = format!("{}/api/v0/add", ipfs_base.trim_end_matches('/'));

    // We do not simulate progress here anymore. Instead, we let reqwest handle the actual network transfer.
    pb.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} {bytes}/{total_bytes}")
            .expect("Valid progress bar template"),
    );

    let bytes_owned = bytes.to_vec();
    let add_url_owned = add_url.clone();
    let ipfs_base_owned = ipfs_base.to_string();
    let local = crate::retry::with_retry("IPFS upload", 3, Duration::from_millis(500), move || {
        use reqwest::multipart as mp;
        let form = mp::Form::new().part(
            "file",
            mp::Part::bytes(bytes_owned.clone()).file_name("package.tgz"),
        );
        let url = add_url_owned.clone();
        async move {
            reqwest::Client::new()
                .post(&url)
                .multipart(form)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("IPFS daemon not reachable: {}", e))
        }
    })
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
                "IPFS daemon not reachable at {} after 3 attempts. \
                 Please start 'ipfs daemon'. Error: {}",
                ipfs_base_owned,
                e
            )
        }
    }
}

/// Best-effort pin so fleet gateways retain the CID for admission fetches.
async fn best_effort_pin_add(ipfs_base: &str, cid: &str) {
    let pin_url = format!(
        "{}/api/v0/pin/add?arg={}",
        ipfs_base.trim_end_matches('/'),
        urlencoding::encode(cid)
    );
    match reqwest::Client::new().post(&pin_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::debug!(cid = %cid, "IPFS pin/add succeeded");
        }
        Ok(resp) => {
            tracing::warn!(
                cid = %cid,
                status = %resp.status(),
                "IPFS pin/add returned non-success (continuing)"
            );
        }
        Err(error) => {
            tracing::warn!(cid = %cid, %error, "IPFS pin/add failed (continuing)");
        }
    }
}

/// Upload tarball bytes to IPFS and return the CID (legacy, without progress).
#[allow(dead_code)]
async fn pin_to_ipfs(bytes: &[u8], ipfs_url: Option<&str>) -> Result<String> {
    let ipfs_base = crate::config_file::effective_ipfs_url(ipfs_url);
    let cid = pin_to_ipfs_with_progress(bytes, &ipfs_base, &ProgressBar::hidden()).await?;
    best_effort_pin_add(&ipfs_base, &cid).await;
    Ok(cid)
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

/// First fingerprint from `gpg --list-secret-keys --with-colons`.
fn first_secret_key_fingerprint() -> Option<String> {
    let output = std::process::Command::new("gpg")
        .args(["--batch", "--with-colons", "--list-secret-keys"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| line.starts_with("fpr:"))
        .and_then(|line| line.split(':').nth(9))
        .map(str::to_string)
}

/// Sign the tarball with PGP if configured.
///
/// Three modes (checked in order):
///  1. `CREG_PGP_PRIVATE_KEY_PATH` — armored secret key file; imports into the
///     default keyring and signs with `gpg --detach-sign`. Use
///     `CREG_PGP_PASSPHRASE` for passphrase-protected keys, or
///     `CREG_PGP_KEY_FINGERPRINT` when multiple secret keys are present.
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

    // Import into the user's default keyring (idempotent). Avoids obsolete
    // --secret-keyring, which modern GnuPG ignores in batch mode.
    let import = std::process::Command::new("gpg")
        .args([
            "--batch",
            "--yes",
            "--import",
            key_path.to_str().unwrap_or(""),
        ])
        .output()
        .context("Failed to invoke gpg --import — ensure GnuPG is installed")?;
    if !import.status.success() {
        let stderr = String::from_utf8_lossy(&import.stderr);
        anyhow::bail!(
            "gpg --import failed ({}). Check CREG_PGP_PRIVATE_KEY_PATH.\n{}",
            import.status,
            stderr.trim()
        );
    }

    let fingerprint = std::env::var("CREG_PGP_KEY_FINGERPRINT")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| first_secret_key_fingerprint())
        .context(
            "Could not determine PGP fingerprint. Set CREG_PGP_KEY_FINGERPRINT or import the key with gpg.",
        )?;

    // Write tarball to a temp file so gpg can sign it.
    let tmp_dir = std::env::temp_dir();
    let tarball_tmp = tmp_dir.join("creg_publish_gpg.tgz");
    std::fs::write(&tarball_tmp, tarball)
        .context("Failed to write temp tarball for GPG signing")?;

    let sig_tmp = tmp_dir.join("creg_publish_gpg.sig");

    let mut sign_cmd = std::process::Command::new("gpg");
    sign_cmd.args([
        "--batch",
        "--yes",
        "--no-tty",
        "--local-user",
        &fingerprint,
        "--detach-sign",
        "--armor",
        "--output",
        sig_tmp.to_str().unwrap_or(""),
        tarball_tmp.to_str().unwrap_or(""),
    ]);
    if let Ok(passphrase) = std::env::var("CREG_PGP_PASSPHRASE") {
        sign_cmd.args(["--pinentry-mode", "loopback", "--passphrase", &passphrase]);
    }

    let status = sign_cmd
        .status()
        .context("Failed to invoke gpg — ensure GnuPG is installed")?;

    if !status.success() {
        anyhow::bail!(
            "gpg sign failed ({}). For passphrase-protected keys set CREG_PGP_PASSPHRASE, \
             or unset CREG_PGP_PRIVATE_KEY_PATH to publish without PGP.",
            status
        );
    }

    let sig_armored = std::fs::read_to_string(&sig_tmp)
        .context("Failed to read GPG detached signature output")?;

    // Export the corresponding public key in armored form.
    let pubkey_output = std::process::Command::new("gpg")
        .args(["--batch", "--no-tty", "--export", "--armor", &fingerprint])
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
    let validator_pubkey = match std::env::var("CREG_VALIDATOR_PUBKEY_X25519") {
        Ok(pubkey_hex) => {
            let bytes: [u8; 32] = hex::decode(pubkey_hex.trim())
                .context("Invalid CREG_VALIDATOR_PUBKEY_X25519 hex")?
                .try_into()
                .map_err(|_| anyhow::anyhow!("X25519 pubkey must be 32 bytes"))?;
            Some(bytes)
        }
        Err(_) => {
            eprintln!(
                "  ⚠ CREG_VALIDATOR_PUBKEY_X25519 not set — key bundle is plaintext (dev mode only)"
            );
            None
        }
    };
    common::encrypt_shielded_package(data, validator_pubkey.as_ref())
}

// ────────────────────────────────────────────────────────────────────────────
// Offline signing (I3 improvement)
// ────────────────────────────────────────────────────────────────────────────

/// Produce a signed publish request and write it to a JSON file on disk
/// instead of submitting it to the node.  The file can later be submitted
/// from a network-connected machine with `creg submit-signed <file>`.
pub async fn sign_offline(
    tarball_path: &Path,
    manifest_path: Option<&Path>,
    privkey_hex: &str,
    extra_privkeys: &[String],
    publisher_address: &str,
    shield: bool,
    ipfs_url: Option<&str>,
    output_path: &Path,
) -> Result<()> {
    use ed25519_dalek::{Signer, SigningKey};

    let publisher_address = canonicalize_publisher_address(publisher_address)?;

    // 1. Read and hash the tarball
    let tarball_bytes = std::fs::read(tarball_path)
        .with_context(|| format!("Cannot read tarball: {}", tarball_path.display()))?;
    let content_hash = common::sha256_hex(&tarball_bytes);

    // 2. Pin to IPFS (IPFS must still be available)
    let ipfs_cid = pin_to_ipfs(&tarball_bytes, ipfs_url).await?;
    let mut final_ipfs_cid = ipfs_cid.clone();
    let mut key_bundle = None;

    if shield {
        let (encrypted_bytes, bundle) = encrypt_for_validators(&tarball_bytes)?;
        final_ipfs_cid = pin_to_ipfs(&encrypted_bytes, ipfs_url).await?;
        key_bundle = Some(bundle);
    }

    // 3. Load manifest
    let manifest: PackageManifest = match manifest_path {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)?,
        None => PackageManifest::default(),
    };

    // 4. Detect package identity
    let pkg_id = detect_package_id(&tarball_bytes)?;
    let manifest_hash = common::sha256_hex(
        &serde_json::to_vec(&manifest).context("Failed to serialize manifest")?,
    );
    let chain_id = std::env::var("CREG_CHAIN_ID").unwrap_or_default();
    let staking_contract = std::env::var("CREG_STAKING_ADDR").unwrap_or_default();
    let mut nonce_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = hex::encode(nonce_bytes);
    let submitted_at = Utc::now();
    let expires_at = (submitted_at + chrono::Duration::hours(1)).timestamp() as u64;
    let ownership_claim = format!("{}:{}", pkg_id.ecosystem, pkg_id.name);
    let source_hash = std::env::var("CREG_SOURCE_HASH").unwrap_or_else(|_| "unavailable".into());

    // 5. Sign with Ed25519
    let privkey_bytes = hex::decode(privkey_hex.trim()).context("Invalid private key hex")?;
    let signing_key =
        SigningKey::try_from(privkey_bytes.as_slice()).context("Invalid Ed25519 private key")?;
    let pubkey = signing_key.verifying_key();

    let msg = if !chain_id.trim().is_empty() && !staking_contract.trim().is_empty() {
        common::publish_signature_message_v2(
            &chain_id,
            &staking_contract,
            &pkg_id,
            &content_hash,
            &source_hash,
            &manifest_hash,
            &hex::encode(pubkey.as_bytes()),
            &publisher_address,
            &ownership_claim,
            &nonce,
            submitted_at.timestamp(),
            expires_at,
        )
    } else {
        common::publish_signature_message(&pkg_id, &content_hash, &publisher_address)
    };
    let signature = signing_key.sign(msg.as_bytes());

    let mut publisher_pubkeys = vec![hex::encode(pubkey.as_bytes())];
    let mut signatures = vec![hex::encode(signature.to_bytes())];

    for key_hex in extra_privkeys {
        let key_bytes = hex::decode(key_hex.trim())?;
        let sk = SigningKey::try_from(key_bytes.as_slice())?;
        let pk = sk.verifying_key();
        let sig = sk.sign(msg.as_bytes());
        publisher_pubkeys.push(hex::encode(pk.as_bytes()));
        signatures.push(hex::encode(sig.to_bytes()));
    }

    let (pgp_signature, pgp_public_key) = sign_with_pgp_if_configured(&tarball_bytes)?;

    let request = PublishRequest {
        id: pkg_id.clone(),
        content_hash,
        ipfs_cid: final_ipfs_cid,
        publisher_address: publisher_address.clone(),
        publisher_pubkey: publisher_pubkeys[0].clone(),
        signature: signatures[0].clone(),
        manifest,
        submitted_at,
        shielded: shield,
        key_bundle,
        pgp_signature,
        pgp_public_key,
        publisher_pubkeys: publisher_pubkeys.clone(),
        signatures: signatures.clone(),
        threshold: if publisher_pubkeys.len() >= 2 { 2 } else { 1 },
        publisher_nonce: nonce,
        publisher_expires_at: expires_at,
        source_hash,
        manifest_hash,
        ownership_claim,
        publisher_chain_id: chain_id,
        publisher_staking_contract: staking_contract,
        publisher_evm_signature: std::env::var("CREG_PUBLISHER_EVM_KEY")
            .ok()
            .map(|key| sign_evm_personal_message(&key, &msg))
            .transpose()?
            .or_else(|| std::env::var("CREG_PUBLISHER_EVM_SIGNATURE").ok())
            .unwrap_or_default(),
        ..Default::default()
    };

    // 6. Write to file
    let json =
        serde_json::to_string_pretty(&request).context("Failed to serialize publish request")?;
    std::fs::write(output_path, &json)
        .with_context(|| format!("Cannot write to {}", output_path.display()))?;

    println!(
        "  ✓ Signed publish request written to {}",
        output_path.display()
    );
    println!("    Package: {}", pkg_id.canonical());
    println!("    Address: {}", publisher_address);
    println!("    Pubkey:  {}", publisher_pubkeys[0]);
    println!();
    println!("  Submit from a networked machine with:");
    println!("    creg submit-signed {}", output_path.display());

    Ok(())
}

/// Submit a previously signed publish request from a JSON file.
pub async fn submit_signed(signed_file: &Path, node_url: Option<&str>) -> Result<()> {
    let json = std::fs::read_to_string(signed_file)
        .with_context(|| format!("Cannot read signed file: {}", signed_file.display()))?;
    let mut request: PublishRequest =
        serde_json::from_str(&json).context("Invalid signed publish request JSON")?;

    request.publisher_address = canonicalize_publisher_address(&request.publisher_address)?;

    println!(
        "  Submitting offline-signed package: {}",
        request.id.canonical()
    );

    let url = format!("{}", publisher_packages_url(node_url));

    let request_clone2 = request.clone();
    let url_clone2 = url.clone();
    let resp = crate::retry::with_retry(
        "submit signed package",
        3,
        Duration::from_millis(500),
        move || {
            let req = request_clone2.clone();
            let u = url_clone2.clone();
            async move {
                reqwest::Client::new()
                    .post(&u)
                    .json(&req)
                    .send()
                    .await
                    .context("Failed to reach registry node")
            }
        },
    )
    .await?;

    if resp.status().is_success() {
        println!("  ✓ Package submitted to pending pool from offline signature.");
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("✗ Submission failed: HTTP {}: {}", status, body);
    }

    Ok(())
}

pub(crate) fn canonicalize_publisher_address(publisher_address: &str) -> Result<String> {
    if !crate::faucet_client::is_valid_evm_address(publisher_address) {
        bail!("Publisher EVM address must be a valid 0x-prefixed address")
    }
    Ok(common::canonical_publisher_address(publisher_address))
}

fn publisher_packages_url(node_url: Option<&str>) -> String {
    format!(
        "{}/v1/publisher/packages",
        crate::config_file::effective_node_url(node_url).trim_end_matches('/')
    )
}

fn resolve_grpc_url(node_url: Option<&str>, grpc_url: Option<&str>) -> String {
    if let Some(explicit) = grpc_url.map(str::trim).filter(|value| !value.is_empty()) {
        if explicit.contains("://") {
            return explicit.to_string();
        }
        return format!("http://{}", explicit);
    }

    let base = crate::config_file::effective_node_url(node_url);
    let host = base
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("localhost:8080")
        .split(':')
        .next()
        .unwrap_or("localhost");
    format!("http://{}:50051", host)
}

#[cfg(test)]
mod tests {
    use super::{publisher_packages_url, resolve_grpc_url};

    #[test]
    fn publish_rest_fallback_uses_grouped_publisher_route() {
        assert_eq!(
            publisher_packages_url(Some("http://localhost:8080/")),
            "http://localhost:8080/v1/publisher/packages"
        );
    }

    #[test]
    fn grpc_url_prefers_explicit_value() {
        assert_eq!(
            resolve_grpc_url(Some("http://localhost:8080"), Some("grpc.internal:9000")),
            "http://grpc.internal:9000"
        );
    }

    #[test]
    fn grpc_url_derives_from_node_host_when_unset() {
        assert_eq!(
            resolve_grpc_url(Some("https://registry.example.com:8443"), None),
            "http://registry.example.com:50051"
        );
    }

    #[test]
    fn ipfs_pin_add_url_encodes_cid() {
        let base = "https://ipfs.testnet.cregnet.dev";
        let cid = "bafybeigdyrzt5sfp7udm7hx76q86x4nk";
        let url = format!("{}/api/v0/pin/add?arg={}", base, urlencoding::encode(cid));
        assert!(url.contains("pin/add?arg="));
        assert!(url.contains(cid));
    }
}
