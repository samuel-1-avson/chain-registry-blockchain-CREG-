// crates/cli/src/keygen.rs
// Generates Ed25519 keypairs for publishers and validator nodes.
// Private keys are encrypted at rest using AES-256-GCM with a
// passphrase-derived key (scrypt KDF).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use colored::Colorize;
use rand::RngCore;
use std::path::{Path, PathBuf};

/// Encrypted key file format:
/// Line 1: "CREG-ENC-V1"
/// Line 2: hex(salt)       (32 bytes)
/// Line 3: hex(nonce)      (12 bytes)
/// Line 4: hex(ciphertext) (privkey_hex encrypted)
const ENC_HEADER: &str = "CREG-ENC-V1";

/// Derive a 256-bit key from a passphrase using scrypt.
fn derive_key(passphrase: &str, salt: &[u8; 32]) -> [u8; 32] {
    let params = scrypt::Params::new(15, 8, 1, 32).expect("valid scrypt params");
    let mut key = [0u8; 32];
    scrypt::scrypt(passphrase.as_bytes(), salt, &params, &mut key)
        .expect("scrypt output length matches");
    key
}

/// Encrypt a hex-encoded private key with a passphrase.
fn encrypt_key(privkey_hex: &str, passphrase: &str) -> Result<String> {
    let mut salt = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut salt);

    let mut nonce_bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).context("AES key init")?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, privkey_hex.as_bytes())
        .map_err(|e| anyhow::anyhow!("encryption failed: {}", e))?;

    Ok(format!(
        "{}\n{}\n{}\n{}",
        ENC_HEADER,
        hex::encode(salt),
        hex::encode(nonce_bytes),
        hex::encode(ciphertext)
    ))
}

/// Decrypt an encrypted key file with a passphrase.
pub fn decrypt_key_file(path: &Path, passphrase: &str) -> Result<String> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read key file: {}", path.display()))?;

    let lines: Vec<&str> = content.trim().lines().collect();
    if lines.len() == 1 && !lines[0].starts_with(ENC_HEADER) {
        // Legacy unencrypted key — return as-is.
        return Ok(lines[0].to_string());
    }

    if lines.len() != 4 || lines[0] != ENC_HEADER {
        anyhow::bail!(
            "Invalid encrypted key file format at {}",
            path.display()
        );
    }

    let salt: [u8; 32] = hex::decode(lines[1])
        .context("bad salt hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("salt must be 32 bytes"))?;

    let nonce_bytes: [u8; 12] = hex::decode(lines[2])
        .context("bad nonce hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("nonce must be 12 bytes"))?;

    let ciphertext = hex::decode(lines[3]).context("bad ciphertext hex")?;

    let key = derive_key(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).context("AES key init")?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| anyhow::anyhow!("Decryption failed — wrong passphrase?"))?;

    String::from_utf8(plaintext).context("decrypted key is not valid UTF-8")
}

/// Generate a new keypair and save it to `output_path`.
/// If `output_path` is None, defaults to ~/.creg/publisher.key
pub fn run(output_path: Option<&Path>, role: &str) -> Result<()> {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

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

    // Prompt for encryption passphrase.
    let passphrase = dialoguer::Password::new()
        .with_prompt("Enter passphrase to encrypt the private key (empty for no encryption)")
        .allow_empty_password(true)
        .with_confirmation("Confirm passphrase", "Passphrases do not match")
        .interact()
        .context("Failed to read passphrase")?;

    if passphrase.is_empty() {
        write_key_file(&key_path, &privkey_hex)?;
        println!("  {} Private key saved (unencrypted)", "⚠".yellow());
    } else {
        let encrypted = encrypt_key(&privkey_hex, &passphrase)?;
        write_key_file(&key_path, &encrypted)?;
        println!("  {} Private key saved (AES-256-GCM encrypted)", "✓".green());
    }

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

    // Try to decrypt old key to print its pubkey.
    let old_content = std::fs::read_to_string(&path).context("Failed to read old key")?;
    let old_privkey_hex = if old_content.starts_with(ENC_HEADER) {
        let pw = dialoguer::Password::new()
            .with_prompt("Enter passphrase for the old key")
            .interact()
            .context("passphrase input")?;
        decrypt_key_file(&path, &pw)?
    } else {
        old_content.trim().to_string()
    };

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

    // Prompt for encryption passphrase for the new key.
    let passphrase = dialoguer::Password::new()
        .with_prompt("Enter passphrase to encrypt the new key (empty for no encryption)")
        .allow_empty_password(true)
        .with_confirmation("Confirm passphrase", "Passphrases do not match")
        .interact()
        .context("Failed to read passphrase")?;

    if passphrase.is_empty() {
        write_key_file(&path, &new_priv)?;
    } else {
        let encrypted = encrypt_key(&new_priv, &passphrase)?;
        write_key_file(&path, &encrypted)?;
    }

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
