// crates/cli/src/keygen.rs
// Generates Ed25519 keypairs for publishers and validator nodes.
// Private key is saved to a file; public key is printed to stdout
// so it can be registered on-chain.

use anyhow::{Context, Result};
use colored::Colorize;
use std::path::{Path, PathBuf};

/// Generate a new keypair and save it to `output_path`.
/// If `output_path` is None, defaults to ~/.creg/publisher.key
pub fn run(output_path: Option<&Path>, role: &str) -> Result<()> {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use rand::RngCore;

    let mut secret_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut secret_bytes);
    let signing_key = SigningKey::from_bytes(&secret_bytes);
    let pubkey = signing_key.verifying_key();

    let privkey_hex = hex::encode(signing_key.as_bytes());
    let pubkey_hex = hex::encode(pubkey.as_bytes());

    // ── Save private key ──────────────────────────────────────────────────────
    let key_path = output_path
        .map(PathBuf::from)
        .unwrap_or_else(|| default_key_path(role));

    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create key directory: {}", parent.display()))?;
    }

    // Write with restricted permissions on Unix.
    write_key_file(&key_path, &privkey_hex)?;

    // ── Print summary ─────────────────────────────────────────────────────────
    println!("\n  {} keypair generated for role: {}", "✓", role);
    println!("  Private key: {}", key_path.display());
    println!("  Public key:  {}\n", pubkey_hex);

    match role {
        "publisher" => {
            println!("  Next steps:");
            println!("  1. Stake tokens:  creg stake --amount 0.01eth");
            println!(
                "  2. Publish:       creg publish <tarball.tgz> --key {}",
                key_path.display()
            );
        }
        "validator" => {
            println!("  Next steps:");
            println!(
                "  1. Set env:       export CREG_VALIDATOR_KEY={}",
                privkey_hex
            );
            println!("  2. Stake tokens:  Call staking.joinAsValidator{{value: 1 ether}}()");
            println!("  3. Start node:    creg-node");
        }
        _ => {}
    }

    println!("  Keep your private key safe and never share it.\n");

    Ok(())
}

/// Rotate an existing keypair: generate a new key, back up the old one,
/// write the new one to the same path, and print the new public key.
pub fn rotate(key_path: Option<&Path>, role: &str) -> Result<()> {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use rand::RngCore;

    let path = key_path
        .map(PathBuf::from)
        .unwrap_or_else(|| default_key_path(role));

    if !path.exists() {
        anyhow::bail!(
            "No existing key found at {}. Run: creg keygen",
            path.display()
        );
    }

    // Back up old key
    let backup_path = path.with_extension("key.bak");
    std::fs::copy(&path, &backup_path)
        .with_context(|| format!("Failed to backup old key to {}", backup_path.display()))?;
    println!(
        "  {} Old key backed up to {}",
        "✓".green(),
        backup_path.display()
    );

    // Print old public key for reference
    let old_privkey_hex = std::fs::read_to_string(&path).context("Failed to read old key")?;
    if let Ok(old_bytes) = hex::decode(old_privkey_hex.trim()) {
        if let Ok(old_sk) = SigningKey::try_from(old_bytes.as_slice()) {
            println!(
                "  Old pubkey: {}",
                hex::encode(old_sk.verifying_key().as_bytes())
            );
        }
    }

    // Generate new key
    let mut secret_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut secret_bytes);
    let new_sk = SigningKey::from_bytes(&secret_bytes);
    let new_pk = new_sk.verifying_key();
    let new_priv = hex::encode(new_sk.as_bytes());
    let new_pub = hex::encode(new_pk.as_bytes());

    write_key_file(&path, &new_priv)?;

    println!("  {} New key written to {}", "✓".green(), path.display());
    println!("  New pubkey: {}", new_pub);
    println!();
    println!("  {} Action required:", "⚠".yellow().bold());
    println!("  Register the new public key on-chain before publishing:");
    println!(
        "  creg stake --amount 0 --pubkey {} (re-stake with new key)",
        new_pub
    );
    println!(
        "  The old backup at {} can be deleted once confirmed.",
        backup_path.display()
    );

    Ok(())
}

fn default_key_path(role: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".creg")
        .join(format!("{}.key", role))
}

#[cfg(unix)]
fn write_key_file(path: &Path, content: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600) // owner read/write only
        .open(path)
        .with_context(|| format!("Cannot write key to {}", path.display()))?;

    file.write_all(content.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_key_file(path: &Path, content: &str) -> Result<()> {
    std::fs::write(path, content).with_context(|| format!("Cannot write key to {}", path.display()))
}
