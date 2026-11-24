//! Secret-reference parsing and owner-only `file:` secret inspection
//! (ADR-019 §4.3).
//!
//! Supported schemes:
//! - `env:<VAR_NAME>`
//! - `vault:<path>#<field>`
//! - `file:<abs_path>` (owner-only permissions on Unix, regular non-symlink file, <= 4 KiB)

use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};
use zeroize::Zeroizing;

/// Upper bound for `file:` API key files (ADR-019 §4.3).
pub const MAX_SECRET_FILE_BYTES: u64 = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretRefKind {
    Env(String),
    Vault { path: String, field: String },
    File(PathBuf),
}

impl SecretRefKind {
    pub const fn scheme(&self) -> &'static str {
        match self {
            Self::Env(_) => "env",
            Self::Vault { .. } => "vault",
            Self::File(_) => "file",
        }
    }
}

/// Validate a secret reference string without touching the underlying secret.
/// Inline plaintext keys are always rejected.
pub fn parse_secret_ref(raw: &str) -> Result<SecretRefKind> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed != raw {
        bail!("secret reference must be non-empty and must not have leading/trailing whitespace");
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
        return Ok(SecretRefKind::Env(var_name.to_string()));
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
        return Ok(SecretRefKind::Vault {
            path: path.to_string(),
            field: field.to_string(),
        });
    }
    if let Some(raw_path) = trimmed.strip_prefix("file:") {
        if raw_path.is_empty() {
            bail!("file: secret reference must specify an absolute path");
        }
        let path = PathBuf::from(raw_path);
        validate_secret_file_path(&path)?;
        return Ok(SecretRefKind::File(path));
    }
    bail!(
        "inline plaintext secrets are forbidden; use 'env:<VAR_NAME>', 'vault:<path>#<field>', or 'file:<abs_path>'"
    )
}

pub fn validate_secret_file_path(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!(
            "file: secret reference path '{}' must be absolute",
            path.display()
        );
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!(
            "file: secret reference path '{}' must not contain '..'",
            path.display()
        );
    }
    Ok(())
}

/// Verify file existence, non-symlink regular file type, `1..=4096` byte bound,
/// and owner-only permissions (`0600` on Unix) without exposing secret content.
pub fn check_secret_file(path: &Path) -> Result<()> {
    validate_secret_file_path(path)?;
    let meta = std::fs::symlink_metadata(path)
        .with_context(|| format!("stat secret file '{}'", path.display()))?;
    let file_type = meta.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        bail!(
            "secret file '{}' must be a regular non-symlink file",
            path.display()
        );
    }
    if meta.len() == 0 {
        bail!("secret file '{}' is empty", path.display());
    }
    if meta.len() > MAX_SECRET_FILE_BYTES {
        bail!(
            "secret file '{}' is {} bytes, exceeding {} byte limit",
            path.display(),
            meta.len(),
            MAX_SECRET_FILE_BYTES
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if (mode & 0o077) != 0 {
            bail!(
                "secret file '{}' has mode {:04o}; owner-only permissions (0600 or 0400) required",
                path.display(),
                mode
            );
        }
    }
    Ok(())
}

/// Read and trim a `file:` secret into a zeroizing buffer.
pub fn read_secret_file_zeroized(path: &Path) -> Result<Zeroizing<String>> {
    check_secret_file(path)?;
    let raw = Zeroizing::new(
        std::fs::read(path).with_context(|| format!("read secret file '{}'", path.display()))?,
    );
    if raw.len() as u64 > MAX_SECRET_FILE_BYTES {
        bail!(
            "secret file '{}' exceeded {} byte limit during read",
            path.display(),
            MAX_SECRET_FILE_BYTES
        );
    }
    let text = Zeroizing::new(
        String::from_utf8(raw.to_vec())
            .with_context(|| format!("secret file '{}' must be valid UTF-8", path.display()))?,
    );
    let trimmed = Zeroizing::new(text.trim().to_string());
    if trimmed.is_empty() {
        bail!("secret file '{}' contains only whitespace", path.display());
    }
    Ok(trimmed)
}

/// Check that a `file:` secret exists, satisfies permissions/size rules, and
/// contains non-empty valid UTF-8, immediately dropping the zeroized secret.
pub fn check_secret_file_non_empty(path: &Path) -> Result<()> {
    let secret = read_secret_file_zeroized(path)?;
    drop(secret);
    Ok(())
}
