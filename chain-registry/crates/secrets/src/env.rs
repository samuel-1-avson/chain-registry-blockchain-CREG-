use crate::{normalize_signing_key_secret, HotKeyRole, SecretString};
use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, Copy, Default)]
pub struct EnvSecretsBackend;

impl EnvSecretsBackend {
    pub fn secp256k1_signing_key_hex(&self, role: HotKeyRole) -> Result<String> {
        self.signing_key_secret(role)
            .map(|secret| secret.expose_secret().to_owned())
    }

    pub fn signing_key_secret(&self, role: HotKeyRole) -> Result<SecretString> {
        let raw = zeroize::Zeroizing::new(
            std::env::var(role.env_var())
                .with_context(|| format!("{} is not set", role.env_var()))?,
        );
        if raw.trim().is_empty() {
            bail!("{} is empty", role.env_var());
        }
        normalize_signing_key_secret(&raw, role)
    }
}
