// crates/cli/src/multisig.rs
// Multi-sig publish — collect M-of-N Ed25519 partial signatures before submitting.
//
// Workflow:
//   1. creg multisig init <tarball>       → writes a .creg-multisig.json session file
//   2. creg multisig sign <session.json>  → co-signer adds their signature
//   3. creg multisig submit <session.json>→ once M sigs collected, submits to chain

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MultisigSession {
    /// Package canonical name
    pub canonical: String,
    /// Content hash (sha256 hex)
    pub content_hash: String,
    /// IPFS CID
    pub ipfs_cid: String,
    /// Minimum signatures required
    pub threshold: usize,
    /// Collected signatures: (pubkey_hex, signature_hex)
    pub signatures: Vec<(String, String)>,
    /// Ecosystem
    pub ecosystem: String,
    /// Package version
    pub version: String,
}

impl MultisigSession {
    pub fn is_ready(&self) -> bool {
        self.signatures.len() >= self.threshold
    }

    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read session file: {}", path.display()))?;
        serde_json::from_str(&raw).context("Invalid multisig session file")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)
            .with_context(|| format!("Cannot write session file: {}", path.display()))
    }
}

/// Initialize a new multisig session from a tarball.
pub async fn init(
    tarball_path: &Path,
    threshold: usize,
    node_url: Option<&str>,
    output: &Path,
) -> Result<()> {
    let ipfs_url =
        std::env::var("CREG_IPFS_URL").unwrap_or_else(|_| "http://127.0.0.1:5001".into());

    println!(
        "{} Initializing multisig publish session (threshold: {}/N)...",
        "→".cyan(),
        threshold
    );

    let tarball_bytes = tokio::fs::read(tarball_path)
        .await
        .context("Failed to read tarball")?;
    let content_hash = common::sha256_hex(&tarball_bytes);

    // Pin to IPFS
    println!("{} Uploading to IPFS...", "→".cyan());
    let add_url = format!("{}/api/v0/add", ipfs_url.trim_end_matches('/'));
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(tarball_bytes).file_name("package.tgz"),
    );

    let resp = reqwest::Client::new()
        .post(&add_url)
        .multipart(form)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .context("IPFS upload failed")?;

    #[derive(serde::Deserialize)]
    struct IpfsResp {
        #[serde(rename = "Hash")]
        hash: String,
    }
    let ipfs_resp: IpfsResp = resp.json().await.context("IPFS response parse error")?;
    let ipfs_cid = ipfs_resp.hash;

    // Detect package identity
    let name = tarball_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("package");

    let session = MultisigSession {
        canonical: format!("npm:{}@0.0.0", name),
        content_hash: content_hash.clone(),
        ipfs_cid: ipfs_cid.clone(),
        threshold,
        signatures: vec![],
        ecosystem: "npm".into(),
        version: "0.0.0".into(),
    };

    session.save(output)?;

    println!("{} Multisig session initialized:", "✓".green());
    println!("  File:         {}", output.display());
    println!("  Content hash: {}", &content_hash[..16]);
    println!("  IPFS CID:     {}", ipfs_cid);
    println!("  Threshold:    {}/N", threshold);
    println!("\n  Share {} with each co-signer.", output.display());
    println!(
        "  Each co-signer runs: creg multisig sign {}",
        output.display()
    );

    Ok(())
}

/// Add a co-signer's signature to the session.
pub fn sign(session_path: &Path, privkey_hex: &str) -> Result<()> {
    use ed25519_dalek::{Signer, SigningKey};

    let mut session = MultisigSession::load(session_path)?;

    let privkey_bytes = hex::decode(privkey_hex.trim()).context("Invalid private key hex")?;
    let signing_key =
        SigningKey::try_from(privkey_bytes.as_slice()).context("Invalid Ed25519 private key")?;
    let pubkey = signing_key.verifying_key();
    let pubkey_hex = hex::encode(pubkey.as_bytes());

    // Check for duplicate signer
    if session.signatures.iter().any(|(pk, _)| pk == &pubkey_hex) {
        println!("{} This key has already signed this session.", "ℹ".blue());
        return Ok(());
    }

    // Sign: message = canonical || content_hash  (same as single-sig publish)
    let msg = format!("{}{}", session.canonical, session.content_hash);
    let signature = signing_key.sign(msg.as_bytes());
    let sig_hex = hex::encode(signature.to_bytes());

    session.signatures.push((pubkey_hex.clone(), sig_hex));
    session.save(session_path)?;

    println!(
        "{} Signature added ({}/{} collected):",
        "✓".green(),
        session.signatures.len(),
        session.threshold
    );
    println!("  Signer: {}...", &pubkey_hex[..16]);
    if session.is_ready() {
        println!(
            "\n  {} Threshold reached! Run: creg multisig submit {}",
            "✓".green().bold(),
            session_path.display()
        );
    } else {
        println!(
            "  {} more signature(s) needed.",
            session.threshold - session.signatures.len()
        );
    }

    Ok(())
}

/// Submit the package once M signatures are collected.
pub async fn submit(
    session_path: &Path,
    manifest_path: Option<&Path>,
    node_url: Option<&str>,
) -> Result<()> {
    let session = MultisigSession::load(session_path)?;

    if !session.is_ready() {
        anyhow::bail!(
            "Only {}/{} signatures collected. Need {} more.",
            session.signatures.len(),
            session.threshold,
            session.threshold - session.signatures.len()
        );
    }

    let base = node_url.map(String::from).unwrap_or_else(|| {
        std::env::var("CREG_NODE_URL").unwrap_or_else(|_| "http://localhost:8080".into())
    });

    println!(
        "{} Submitting multisig package ({}/{} signatures)...",
        "→".cyan(),
        session.signatures.len(),
        session.threshold
    );

    // Build a publish request using the first signer as the primary publisher
    // and the new first-class multi-sig fields.
    let (primary_pubkey, primary_sig) = session
        .signatures
        .first()
        .context("No signatures in session")?;

    let manifest: common::PackageManifest = match manifest_path {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)?,
        None => common::PackageManifest::default(),
    };

    let (publisher_pubkeys, signatures): (Vec<String>, Vec<String>) =
        session.signatures.iter().cloned().unzip();

    let request = common::PublishRequest {
        id: common::PackageId {
            ecosystem: session.ecosystem.clone(),
            name: session
                .canonical
                .split(':')
                .nth(1)
                .and_then(|s| s.split('@').next())
                .unwrap_or("unknown")
                .to_string(),
            version: session.version.clone(),
        },
        content_hash: session.content_hash.clone(),
        ipfs_cid: session.ipfs_cid.clone(),
        publisher_pubkey: primary_pubkey.clone(),
        signature: primary_sig.clone(),
        manifest,
        submitted_at: chrono::Utc::now(),
        shielded: false,
        key_bundle: None,
        pgp_signature: None,
        pgp_public_key: None,
        publisher_pubkeys,
        signatures,
        threshold: session.threshold,
        ..Default::default()
    };

    let url = format!("{}/v1/packages", base.trim_end_matches('/'));
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&request)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .context("Failed to reach registry node")?;

    if resp.status().is_success() {
        println!(
            "{} Multisig package submitted successfully!",
            "✓".green().bold()
        );
        println!(
            "  Run: creg status {} to track verification.",
            session.canonical
        );
    } else {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Submission failed: {}", body);
    }

    Ok(())
}
