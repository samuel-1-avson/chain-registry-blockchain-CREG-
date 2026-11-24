// crates/cli/src/profile.rs
// Profile and credential management for CREG Dual-Key Architecture.
// Stores authenticated wallet, locally generated wallet-bound publisher keys, and network preferences.

use crate::auth::BoundIdentity;
use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub wallet_address: String,
    pub ed25519_pubkey: String,
    pub key_path: PathBuf,
    pub network_url: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_nonce: Option<String>,
}

pub fn creg_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".creg"))
        .unwrap_or_else(|| PathBuf::from(".creg"))
}

pub fn profile_path() -> PathBuf {
    creg_dir().join("profile.json")
}

pub fn default_publisher_key_path() -> PathBuf {
    creg_dir().join("publisher.key")
}

pub fn load_active_profile() -> Result<Option<Profile>> {
    let path = profile_path();
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read profile at {}", path.display()))?;
    let profile: Profile = serde_json::from_str(&data)
        .with_context(|| format!("Failed to parse profile JSON at {}", path.display()))?;
    Ok(Some(profile))
}

pub fn save_profile(profile: &Profile) -> Result<()> {
    let dir = creg_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create directory {}", dir.display()))?;
    let path = profile_path();
    let data = serde_json::to_string_pretty(profile).context("Failed to serialize profile")?;
    std::fs::write(&path, data)
        .with_context(|| format!("Failed to write profile to {}", path.display()))?;
    Ok(())
}

pub fn save_identity(identity: &BoundIdentity, network_url: &str) -> Result<Profile> {
    save_identity_to(identity, network_url, None)
}

pub fn save_identity_to(
    identity: &BoundIdentity,
    network_url: &str,
    target_key_path: Option<&Path>,
) -> Result<Profile> {
    let dir = creg_dir();
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create directory {}", dir.display()))?;

    let key_path = target_key_path
        .map(PathBuf::from)
        .unwrap_or_else(default_publisher_key_path);
    if let Some(parent) = key_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create key directory {}", parent.display()))?;
    }
    std::fs::write(&key_path, identity.ed25519_privkey_hex.trim())
        .with_context(|| format!("Failed to write key to {}", key_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&key_path)?.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(&key_path, perms);
    }

    let profile = Profile {
        wallet_address: identity.wallet_address.clone(),
        ed25519_pubkey: identity.ed25519_pubkey_hex.clone(),
        key_path,
        network_url: network_url.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        binding_signature: Some(identity.binding_signature.clone()),
        binding_nonce: Some(identity.binding_nonce.clone()),
    };

    save_profile(&profile)?;
    Ok(profile)
}

pub fn clear_profile() -> Result<bool> {
    let path = profile_path();
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove profile at {}", path.display()))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn print_profile(profile: &Profile) {
    println!();
    println!(
        "{}",
        "╭────────────────────────────────────────────────────────────╮".cyan()
    );
    println!(
        "{}",
        "│   Active CREG Authenticated Profile                        │"
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "╰────────────────────────────────────────────────────────────╯".cyan()
    );
    println!();
    println!(
        "  {:<22} {}",
        "EVM Staking Wallet:".bold(),
        profile.wallet_address.green()
    );
    println!(
        "  {:<22} {}",
        "Ed25519 Pubkey:".bold(),
        profile.ed25519_pubkey.cyan()
    );
    println!(
        "  {:<22} {}",
        "Consensus Key File:".bold(),
        profile.key_path.display()
    );
    println!(
        "  {:<22} {}",
        "Network RPC URL:".bold(),
        profile.network_url
    );
    if let Some(nonce) = profile.binding_nonce.as_deref() {
        println!("  {:<22} {}", "Binding Nonce:".bold(), nonce.dimmed());
    }
    println!(
        "  {:<22} {}",
        "Authenticated At:".bold(),
        profile.created_at.dimmed()
    );
    println!();
}

/// Resolves publisher credentials from explicit CLI arguments, environment variables,
/// or the active profile.
pub fn resolve_publisher_credentials(
    explicit_key_path: Option<&Path>,
    explicit_publisher_addr: Option<&str>,
) -> Result<(String, String)> {
    let active_profile = load_active_profile().ok().flatten();

    // 1. Resolve key
    let key_str = if let Some(path) = explicit_key_path {
        std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read key file: {}", path.display()))?
            .trim()
            .to_string()
    } else if let Ok(env_path) = std::env::var("CREG_KEY_FILE") {
        std::fs::read_to_string(&env_path)
            .with_context(|| format!("Cannot read key file from CREG_KEY_FILE: {}", env_path))?
            .trim()
            .to_string()
    } else if let Some(ref prof) = active_profile {
        if prof.key_path.exists() {
            std::fs::read_to_string(&prof.key_path)
                .with_context(|| {
                    format!("Cannot read profile key file: {}", prof.key_path.display())
                })?
                .trim()
                .to_string()
        } else {
            anyhow::bail!(
                "Active profile key file not found at {}. Run 'creg login' to re-authenticate.",
                prof.key_path.display()
            );
        }
    } else {
        anyhow::bail!(
            "No publisher key found.\n\
            To authenticate your wallet, run:\n\
              creg login\n\
            Or supply an explicit key file using:\n\
              creg publish <tarball> --key-file <path> --publisher-address <0x...>"
        );
    };

    // 2. Resolve publisher address
    let publisher_address = if let Some(addr) = explicit_publisher_addr {
        addr.trim().to_string()
    } else if let Ok(env_addr) = std::env::var("CREG_PUBLISHER_ADDRESS") {
        env_addr.trim().to_string()
    } else if let Some(ref prof) = active_profile {
        prof.wallet_address.clone()
    } else {
        anyhow::bail!(
            "No publisher address found.\n\
            To authenticate your wallet, run:\n\
              creg login\n\
            Or supply an explicit address using:\n\
              creg publish <tarball> --publisher-address <0x...>"
        );
    };

    Ok((key_str, publisher_address))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_serialization() {
        let profile = Profile {
            wallet_address: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".to_string(),
            ed25519_pubkey: "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234"
                .to_string(),
            key_path: PathBuf::from("/tmp/test.key"),
            network_url: "https://api.testnet.cregnet.dev".to_string(),
            created_at: "2026-09-19T00:00:00Z".to_string(),
            binding_signature: Some("0x11".to_string()),
            binding_nonce: Some("nonce-123".to_string()),
        };

        let json = serde_json::to_string(&profile).unwrap();
        let deserialized: Profile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.wallet_address, profile.wallet_address);
        assert_eq!(deserialized.ed25519_pubkey, profile.ed25519_pubkey);
        assert_eq!(deserialized.binding_signature.as_deref(), Some("0x11"));
        assert_eq!(deserialized.binding_nonce.as_deref(), Some("nonce-123"));

        // Legacy profiles saved prior to ECA-20 must still deserialize cleanly.
        let legacy_json = serde_json::json!({
            "wallet_address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
            "ed25519_pubkey": "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
            "key_path": "/tmp/legacy.key",
            "network_url": "https://api.testnet.cregnet.dev",
            "created_at": "2026-09-19T00:00:00Z"
        })
        .to_string();
        let legacy: Profile = serde_json::from_str(&legacy_json).unwrap();
        assert!(legacy.binding_signature.is_none());
        assert!(legacy.binding_nonce.is_none());
    }
}
