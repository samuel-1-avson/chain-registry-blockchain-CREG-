//! Hot secp256k1 secret loading for bridge, faucet, and relayer (SEC-301b).
//!
//! See `docs/adr/ADR-KMS-HOT-KEYS.md`.

mod env;
mod vault;

use anyhow::{bail, Context, Result};
use std::fmt;

pub use env::EnvSecretsBackend;
pub use vault::VaultSecretsBackend;

/// Heap-allocated string wrapper that zeroizes its contents on drop and
/// redacts secret material in `Debug` output (`VNA-25`).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretString(zeroize::Zeroizing<String>);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(zeroize::Zeroizing::new(value.into()))
    }

    pub fn expose_secret(&self) -> &str {
        self.0.as_str()
    }
}

impl std::ops::Deref for SecretString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.expose_secret()
    }
}

impl AsRef<str> for SecretString {
    fn as_ref(&self) -> &str {
        self.expose_secret()
    }
}

impl AsRef<[u8]> for SecretString {
    fn as_ref(&self) -> &[u8] {
        self.expose_secret().as_bytes()
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Services that use signing keys loaded through this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotKeyRole {
    Bridge,
    Faucet,
    Relayer,
    /// Ed25519 validator key stored as hex in Vault or env (node only).
    ValidatorEd25519,
    /// Governance.sol execution signer key (node only, `VNA-32`).
    Governance,
}

impl HotKeyRole {
    pub fn env_var(self) -> &'static str {
        match self {
            Self::Bridge => "CREG_BRIDGE_KEY",
            Self::Faucet => "FAUCET_PRIVATE_KEY",
            Self::Relayer => "RELAYER_PRIVATE_KEY",
            Self::ValidatorEd25519 => "CREG_VALIDATOR_KEY",
            Self::Governance => "CREG_GOVERNANCE_KEY",
        }
    }

    pub fn default_vault_path(self) -> &'static str {
        match self {
            Self::Bridge => "secret/data/creg/bridge",
            Self::Faucet => "secret/data/creg/faucet",
            Self::Relayer => "secret/data/creg/relayer",
            Self::ValidatorEd25519 => "secret/data/creg/validator",
            Self::Governance => "secret/data/creg/governance",
        }
    }

    pub fn vault_path_env(self) -> &'static str {
        match self {
            Self::Bridge => "CREG_VAULT_SECRET_PATH_BRIDGE",
            Self::Faucet => "CREG_VAULT_SECRET_PATH_FAUCET",
            Self::Relayer => "CREG_VAULT_SECRET_PATH_RELAYER",
            Self::ValidatorEd25519 => "CREG_VAULT_SECRET_PATH_VALIDATOR",
            Self::Governance => "CREG_VAULT_SECRET_PATH_GOVERNANCE",
        }
    }

    fn vault_field_candidates(self) -> &'static [&'static str] {
        match self {
            Self::Bridge => &["private_key", "bridge_key", "key"],
            Self::Faucet => &["private_key", "faucet_key", "key"],
            Self::Relayer => &["private_key", "relayer_key", "key"],
            Self::ValidatorEd25519 => &["validator_key", "private_key", "key"],
            Self::Governance => &["governance_key", "private_key", "key"],
        }
    }

    pub fn is_ed25519(self) -> bool {
        matches!(self, Self::ValidatorEd25519)
    }
}

/// Secret backend selected at process startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretsBackend {
    Env,
    Vault,
}

impl fmt::Display for SecretsBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env => write!(f, "env"),
            Self::Vault => write!(f, "vault"),
        }
    }
}

/// Loads hot keys from environment variables or HashiCorp Vault KV v2.
#[derive(Debug, Clone)]
pub struct SecretsProvider {
    backend: SecretsBackend,
    env: EnvSecretsBackend,
    vault: Option<VaultSecretsBackend>,
}

impl SecretsProvider {
    /// Build from `CREG_SECRETS_BACKEND` (`env` default, or `vault`).
    pub fn from_env() -> Result<Self> {
        let backend = parse_backend(std::env::var("CREG_SECRETS_BACKEND").ok().as_deref())?;
        let env = EnvSecretsBackend;
        let vault = if backend == SecretsBackend::Vault {
            Some(VaultSecretsBackend::from_env()?)
        } else {
            None
        };
        Ok(Self {
            backend,
            env,
            vault,
        })
    }

    pub fn backend(&self) -> SecretsBackend {
        self.backend
    }

    /// Load a 32-byte secp256k1 private key as hex (optional `0x` prefix).
    pub async fn secp256k1_signing_key_hex(&self, role: HotKeyRole) -> Result<String> {
        self.signing_key_secret(role)
            .await
            .map(|secret| secret.expose_secret().to_owned())
    }

    /// Optional load: returns `None` if the role's secret is not configured.
    pub async fn try_secp256k1_signing_key_hex(&self, role: HotKeyRole) -> Result<Option<String>> {
        self.try_signing_key_secret(role)
            .await
            .map(|opt| opt.map(|secret| secret.expose_secret().to_owned()))
    }

    /// Load a 32-byte signing key as a zeroizing [`SecretString`].
    pub async fn signing_key_secret(&self, role: HotKeyRole) -> Result<SecretString> {
        match self.backend {
            SecretsBackend::Env => self.env.signing_key_secret(role),
            SecretsBackend::Vault => {
                let vault = self
                    .vault
                    .as_ref()
                    .context("vault backend not initialized")?;
                vault.signing_key_secret(role).await
            }
        }
    }

    /// Optional zeroizing load: returns `None` when the role's key is unconfigured
    /// (in env mode: unset/empty; in vault mode: 404/not found). Malformed keys
    /// and transport/auth errors are always returned as `Err` (`VNA-25`).
    pub async fn try_signing_key_secret(&self, role: HotKeyRole) -> Result<Option<SecretString>> {
        if self.backend == SecretsBackend::Env {
            match std::env::var(role.env_var()) {
                Err(_) => return Ok(None),
                Ok(v) if v.trim().is_empty() => return Ok(None),
                Ok(_) => {}
            }
        }
        match self.signing_key_secret(role).await {
            Ok(key) => Ok(Some(key)),
            Err(e) => {
                let msg = e.to_string();
                if self.backend == SecretsBackend::Vault
                    && (msg.contains("404") || msg.contains("not found"))
                {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Emit SEC-101b warning when the active backend is env and not testnet.
    pub fn warn_hot_key_if_env(
        &self,
        service: &str,
        role: HotKeyRole,
        key_hex: &str,
        testnet_mode: bool,
    ) {
        if self.backend == SecretsBackend::Env {
            common::warn_hot_key_from_env(service, role.env_var(), key_hex, testnet_mode);
        }
    }
}

fn parse_backend(raw: Option<&str>) -> Result<SecretsBackend> {
    match raw.unwrap_or("env").trim().to_ascii_lowercase().as_str() {
        "" | "env" | "environment" => Ok(SecretsBackend::Env),
        "vault" | "hashicorp" | "hashicorp-vault" => Ok(SecretsBackend::Vault),
        other => bail!("unsupported CREG_SECRETS_BACKEND={other:?} (use 'env' or 'vault')"),
    }
}

/// Rejects plaintext environment overrides when `CREG_SECRETS_BACKEND=vault` (`VNA-25`).
pub fn validate_vault_exclusivity(backend: SecretsBackend) -> Vec<String> {
    if backend != SecretsBackend::Vault {
        return Vec::new();
    }
    let mut errors = Vec::new();
    for role in [
        HotKeyRole::ValidatorEd25519,
        HotKeyRole::Bridge,
        HotKeyRole::Governance,
    ] {
        if let Ok(value) = std::env::var(role.env_var()) {
            if !value.trim().is_empty() {
                errors.push(format!(
                    "{} must not be set when CREG_SECRETS_BACKEND=vault — \
                     plaintext environment keys cannot override HashiCorp Vault custody",
                    role.env_var()
                ));
            }
        }
    }
    errors
}

/// Production gate: plaintext env hot keys are not allowed off testnet (ADR SEC-301).
pub fn validate_production_secrets_policy(is_testnet: bool) -> Vec<String> {
    if is_testnet {
        return Vec::new();
    }

    let mut errors = Vec::new();
    // Unset defaults to env in `parse_backend`; production must fail closed.
    let backend = std::env::var("CREG_SECRETS_BACKEND").unwrap_or_default();
    let backend = backend.trim().to_ascii_lowercase();
    if backend.is_empty() || backend == "env" || backend == "environment" {
        errors.push(
            "CREG_SECRETS_BACKEND=env (or unset) is not allowed when CREG_TESTNET=false — \
             configure HashiCorp Vault (CREG_SECRETS_BACKEND=vault) for production hot keys."
                .into(),
        );
    }
    errors
}

pub fn normalize_signing_key_secret(key: &str, role: HotKeyRole) -> Result<SecretString> {
    let raw = zeroize::Zeroizing::new(key.trim().trim_start_matches("0x").to_string());
    let bytes = zeroize::Zeroizing::new(
        hex::decode(raw.as_str())
            .with_context(|| format!("{} is not valid hex", role.env_var()))?,
    );
    if bytes.len() != 32 {
        bail!(
            "{} must be 32 bytes (64 hex chars), got {} bytes",
            role.env_var(),
            bytes.len()
        );
    }
    if role.is_ed25519() {
        Ok(SecretString::new(hex::encode(bytes.as_slice())))
    } else {
        Ok(SecretString::new(format!(
            "0x{}",
            hex::encode(bytes.as_slice())
        )))
    }
}

pub(crate) fn extract_vault_field(data: &serde_json::Value, role: HotKeyRole) -> Option<String> {
    let override_field = std::env::var("CREG_VAULT_SECRET_FIELD").ok();
    if let Some(name) = override_field {
        if let Some(v) = data.get(&name).and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
    }
    for field in role.vault_field_candidates() {
        if let Some(v) = data.get(field).and_then(|v| v.as_str()) {
            return Some(v.to_string());
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretRefTarget<'a> {
    Env(&'a str),
    Vault { path: &'a str, field: &'a str },
}

fn validate_secret_ref_format(secret_ref: &str) -> Result<SecretRefTarget<'_>> {
    let trimmed = secret_ref.trim();
    if trimmed.is_empty() || trimmed != secret_ref {
        bail!("secret reference must be non-empty and trimmed");
    }
    if trimmed.chars().any(char::is_control) {
        bail!("secret reference must not contain control characters");
    }
    if let Some(var_name) = trimmed.strip_prefix("env:") {
        let valid_var = !var_name.is_empty()
            && var_name.len() <= 128
            && var_name
                .as_bytes()
                .first()
                .is_some_and(|b| b.is_ascii_uppercase())
            && var_name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if !valid_var {
            bail!("invalid env: secret reference '{trimmed}'");
        }
        return Ok(SecretRefTarget::Env(var_name));
    }
    if let Some(rest) = trimmed.strip_prefix("vault:") {
        let Some((path, field)) = rest.split_once('#') else {
            bail!("vault: secret reference must use 'vault:<path>#<field>' syntax");
        };
        if path.is_empty()
            || field.is_empty()
            || path.contains("..")
            || path.chars().any(char::is_whitespace)
            || field.chars().any(char::is_whitespace)
        {
            bail!("invalid vault: secret reference '{trimmed}'");
        }
        return Ok(SecretRefTarget::Vault { path, field });
    }
    bail!("inline plaintext secrets are forbidden; use 'env:<VAR_NAME>' or 'vault:<path>#<field>'")
}

impl SecretsProvider {
    /// Resolve an operator secret reference (`env:<VAR>` or `vault:<path>#<field>`)
    /// into a zeroizing [`SecretString`] (`VNA-32`). Inline secret values are
    /// always rejected.
    pub async fn resolve_secret_ref(&self, secret_ref: &str) -> Result<SecretString> {
        match validate_secret_ref_format(secret_ref)? {
            SecretRefTarget::Env(var_name) => {
                if self.backend == SecretsBackend::Vault && !common::parse_env_bool("CREG_TESTNET")
                {
                    bail!(
                        "env: secret reference '{secret_ref}' is disallowed when CREG_SECRETS_BACKEND=vault and CREG_TESTNET=false"
                    );
                }
                let val = std::env::var(var_name)
                    .with_context(|| format!("missing secret environment variable '{var_name}'"))?;
                let trimmed = val.trim();
                if trimmed.is_empty() {
                    bail!("secret environment variable '{var_name}' is empty");
                }
                Ok(SecretString::new(trimmed))
            }
            SecretRefTarget::Vault { path, field } => {
                let vault = self.vault.as_ref().with_context(|| {
                    format!(
                        "vault: secret reference '{secret_ref}' requires CREG_SECRETS_BACKEND=vault"
                    )
                })?;
                vault.read_kv2_field(path, field).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn production_rejects_env_backend_off_testnet() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CREG_SECRETS_BACKEND", "env");
        let errs = validate_production_secrets_policy(false);
        std::env::remove_var("CREG_SECRETS_BACKEND");
        assert!(!errs.is_empty());
        assert!(!validate_production_secrets_policy(false).is_empty());
    }

    #[test]
    fn production_allows_vault_backend_off_testnet() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CREG_SECRETS_BACKEND", "vault");
        let errs = validate_production_secrets_policy(false);
        std::env::remove_var("CREG_SECRETS_BACKEND");
        assert!(errs.is_empty());
    }

    #[test]
    fn extract_vault_field_prefers_role_specific() {
        let data = serde_json::json!({
            "bridge_key": "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
        });
        let key = extract_vault_field(&data, HotKeyRole::Bridge).unwrap();
        assert!(key.starts_with("0x59"));
    }

    #[test]
    fn secret_string_redacts_debug_and_normalizes_ed25519() {
        let secret = normalize_signing_key_secret(&"aa".repeat(32), HotKeyRole::ValidatorEd25519)
            .expect("valid ed25519 seed");
        assert_eq!(secret.expose_secret(), "aa".repeat(32));
        assert_eq!(format!("{secret:?}"), "[REDACTED]");

        let gov =
            normalize_signing_key_secret(&format!("0x{}", "bb".repeat(32)), HotKeyRole::Governance)
                .expect("valid governance key");
        assert_eq!(gov.expose_secret(), format!("0x{}", "bb".repeat(32)));
        assert_eq!(HotKeyRole::Governance.env_var(), "CREG_GOVERNANCE_KEY");
    }

    #[test]
    fn vault_exclusivity_rejects_plaintext_env_overrides() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CREG_VALIDATOR_KEY", &"11".repeat(32));
        std::env::set_var("CREG_GOVERNANCE_KEY", &"22".repeat(32));
        let errs = validate_vault_exclusivity(SecretsBackend::Vault);
        std::env::remove_var("CREG_VALIDATOR_KEY");
        std::env::remove_var("CREG_GOVERNANCE_KEY");
        assert_eq!(errs.len(), 2);
        assert!(errs[0].contains("CREG_VALIDATOR_KEY"));
        assert!(errs[1].contains("CREG_GOVERNANCE_KEY"));
        assert!(validate_vault_exclusivity(SecretsBackend::Env).is_empty());
    }

    #[test]
    fn resolve_secret_ref_enforces_schemes_and_custody() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("CREG_TEST_LLM_TOKEN", "  secret-token-value  ");
        std::env::remove_var("CREG_SECRETS_BACKEND");

        let provider = SecretsProvider::from_env().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let resolved = rt
            .block_on(provider.resolve_secret_ref("env:CREG_TEST_LLM_TOKEN"))
            .unwrap();
        assert_eq!(resolved.expose_secret(), "secret-token-value");
        assert_eq!(format!("{resolved:?}"), "[REDACTED]");

        std::env::remove_var("CREG_TEST_LLM_TOKEN");

        assert!(rt
            .block_on(provider.resolve_secret_ref("sk-raw-inline-key"))
            .is_err());
        assert!(rt
            .block_on(provider.resolve_secret_ref("env:lowercase_bad"))
            .is_err());
        assert!(rt
            .block_on(provider.resolve_secret_ref("vault:secret/data/no_hash"))
            .is_err());
        assert!(rt
            .block_on(provider.resolve_secret_ref("vault:secret/data/creg/llm#gemini_api_key"))
            .is_err());
    }
}
