// crates/cli/src/secret_input.rs
// ECA-19 (ADR-019 Increment 1): secrets never travel on `creg`'s argv, and
// token amounts are exact integers.
//
// A private key passed as `--private-key 0x…` or `--key 0x…` is visible to
// every local user through the process list, and lands in shell history and
// CI logs. Commands now take `--*-file <path>` or `--*-ref env:VAR|file:PATH`.
// The old flags stay as hidden aliases that refuse to run unless the operator
// explicitly opts in with `CREG_ALLOW_ARGV_SECRETS=1`.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Opt-in for the deprecated argv secret flags.
pub const ALLOW_ARGV_SECRETS_ENV: &str = "CREG_ALLOW_ARGV_SECRETS";

/// Largest secret file accepted (a hex key is 66 bytes).
const MAX_SECRET_FILE_BYTES: u64 = 4096;

/// Decimals of the CREG / tCREG token.
pub const TOKEN_DECIMALS: u32 = 18;

/// Where a secret comes from, in precedence order.
pub struct SecretSources<'a> {
    pub file: Option<&'a Path>,
    pub reference: Option<&'a str>,
    /// Value given on argv through a deprecated flag.
    pub argv: Option<&'a str>,
    /// Name of the deprecated flag, for messages (e.g. `--private-key`).
    pub argv_flag: &'a str,
    /// Name of the replacement flags, for messages.
    pub replacement: &'a str,
}

/// Resolve a secret from a file, an `env:`/`file:` reference, or (only with
/// the explicit opt-in) a deprecated argv flag. `Ok(None)` when none is set.
pub fn resolve_secret(sources: SecretSources<'_>) -> Result<Option<String>> {
    let given = [
        sources.file.is_some(),
        sources.reference.is_some(),
        sources.argv.is_some(),
    ]
    .into_iter()
    .filter(|set| *set)
    .count();
    if given > 1 {
        bail!(
            "pass the secret exactly once ({} or {})",
            sources.replacement,
            sources.argv_flag
        );
    }
    if let Some(path) = sources.file {
        return read_secret_file(path).map(Some);
    }
    if let Some(reference) = sources.reference {
        return read_secret_ref(reference).map(Some);
    }
    if let Some(value) = sources.argv {
        if !argv_secrets_allowed() {
            bail!(
                "{} puts a private key on the command line, where other local users, shell history and CI logs can read it. Use {} instead (set {}=1 only for a throwaway test key).",
                sources.argv_flag,
                sources.replacement,
                ALLOW_ARGV_SECRETS_ENV
            );
        }
        eprintln!(
            "warning: {} is deprecated and exposes the key on argv; use {}",
            sources.argv_flag, sources.replacement
        );
        return Ok(Some(value.trim().to_string()));
    }
    Ok(None)
}

/// Staking key for `creg testnet stake-*`; one source is required.
pub fn staking_key(
    file: Option<&Path>,
    reference: Option<&str>,
    argv: Option<&str>,
) -> Result<String> {
    resolve_secret(SecretSources {
        file,
        reference,
        argv,
        argv_flag: "--key",
        replacement: "--key-file <path> or --key-ref env:VAR|file:PATH",
    })?
    .context("a staking key is required: pass --key-file <path> or --key-ref env:VAR|file:PATH")
}

fn argv_secrets_allowed() -> bool {
    std::env::var(ALLOW_ARGV_SECRETS_ENV)
        .map(|value| {
            let value = value.trim();
            value == "1" || value.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false)
}

/// `env:VAR` or `file:PATH`. Inline values are refused.
pub fn read_secret_ref(reference: &str) -> Result<String> {
    if let Some(var) = reference.strip_prefix("env:") {
        let var = var.trim();
        if var.is_empty() {
            bail!("secret reference 'env:' needs a variable name");
        }
        let value =
            std::env::var(var).with_context(|| format!("environment variable {var} is not set"))?;
        let value = value.trim().to_string();
        if value.is_empty() {
            bail!("environment variable {var} is empty");
        }
        return Ok(value);
    }
    if let Some(path) = reference.strip_prefix("file:") {
        return read_secret_file(Path::new(path.trim()));
    }
    bail!("secret reference must be 'env:VAR' or 'file:PATH' (inline secrets are not accepted)")
}

/// Read a secret file: at most 4 KiB and, on Unix, not readable by group or
/// others.
pub fn read_secret_file(path: &Path) -> Result<String> {
    let meta =
        std::fs::metadata(path).with_context(|| format!("read secret file {}", path.display()))?;
    if !meta.is_file() {
        bail!("secret path {} is not a regular file", path.display());
    }
    if meta.len() > MAX_SECRET_FILE_BYTES {
        bail!(
            "secret file {} is larger than {MAX_SECRET_FILE_BYTES} bytes",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            bail!(
                "secret file {} is accessible by group/others (mode {:o}); run chmod 600",
                path.display(),
                mode & 0o777
            );
        }
    }
    let value = std::fs::read_to_string(path)
        .with_context(|| format!("read secret file {}", path.display()))?;
    let value = value.trim().to_string();
    if value.is_empty() {
        bail!("secret file {} is empty", path.display());
    }
    Ok(value)
}

/// Parse a decimal token amount (e.g. `"100"`, `"0.5"`, `"1.000000000000000001"`)
/// into integer wei without floating point. Rejects signs, exponents,
/// separators, more than 18 fractional digits and overflow.
pub fn parse_token_amount_wei(raw: &str) -> Result<u128> {
    let text = raw.trim();
    if text.is_empty() {
        bail!("amount is empty");
    }
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (text, ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        bail!("amount '{text}' has no digits");
    }
    if !whole.bytes().all(|b| b.is_ascii_digit()) || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        bail!("amount '{text}' must be a plain decimal number such as 100 or 0.5");
    }
    if fraction.len() > TOKEN_DECIMALS as usize {
        bail!("amount '{text}' has more than {TOKEN_DECIMALS} decimal places");
    }
    let scale = 10u128.pow(TOKEN_DECIMALS);
    let whole_value: u128 = if whole.is_empty() {
        0
    } else {
        whole
            .parse::<u128>()
            .with_context(|| format!("amount '{text}' is too large"))?
    };
    let fraction_value: u128 = if fraction.is_empty() {
        0
    } else {
        let padded = format!("{fraction:0<width$}", width = TOKEN_DECIMALS as usize);
        padded
            .parse::<u128>()
            .with_context(|| format!("amount '{text}' is invalid"))?
    };
    whole_value
        .checked_mul(scale)
        .and_then(|wei| wei.checked_add(fraction_value))
        .with_context(|| format!("amount '{text}' is too large"))
}

/// Render integer wei as a decimal token amount with trailing zeros removed.
pub fn format_wei(wei: u128) -> String {
    let scale = 10u128.pow(TOKEN_DECIMALS);
    let whole = wei / scale;
    let fraction = wei % scale;
    if fraction == 0 {
        return whole.to_string();
    }
    let digits = format!("{fraction:0>width$}", width = TOKEN_DECIMALS as usize);
    format!("{whole}.{}", digits.trim_end_matches('0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_are_exact() {
        assert_eq!(parse_token_amount_wei("100").unwrap(), 100 * 10u128.pow(18));
        assert_eq!(parse_token_amount_wei("0.5").unwrap(), 5 * 10u128.pow(17));
        assert_eq!(parse_token_amount_wei(".25").unwrap(), 25 * 10u128.pow(16));
        assert_eq!(parse_token_amount_wei("1.").unwrap(), 10u128.pow(18));
        // f64 would round this to exactly 1e18.
        assert_eq!(
            parse_token_amount_wei("1.000000000000000001").unwrap(),
            10u128.pow(18) + 1
        );
        assert_eq!(format_wei(10u128.pow(18) + 1), "1.000000000000000001");
        assert_eq!(format_wei(25 * 10u128.pow(16)), "0.25");
        assert_eq!(format_wei(100 * 10u128.pow(18)), "100");
    }

    #[test]
    fn malformed_amounts_are_rejected() {
        for bad in [
            "",
            ".",
            "-1",
            "+1",
            "1e18",
            "1,000",
            "0x10",
            "1.0000000000000000001",
            "NaN",
            "inf",
            " 1 2 ",
        ] {
            assert!(parse_token_amount_wei(bad).is_err(), "{bad:?}");
        }
        assert!(parse_token_amount_wei(&"9".repeat(40)).is_err());
    }

    #[test]
    fn inline_refs_are_refused() {
        assert!(read_secret_ref("0xdeadbeef").is_err());
        assert!(read_secret_ref("env:").is_err());
    }

    #[test]
    fn argv_secret_requires_opt_in_and_single_source() {
        let sources = |argv: Option<&'static str>, reference: Option<&'static str>| SecretSources {
            file: None,
            reference,
            argv,
            argv_flag: "--key",
            replacement: "--key-file / --key-ref",
        };
        if !argv_secrets_allowed() {
            let err = resolve_secret(sources(Some("0xabc"), None)).unwrap_err();
            assert!(err.to_string().contains(ALLOW_ARGV_SECRETS_ENV));
        }
        assert!(resolve_secret(sources(Some("0xabc"), Some("env:X"))).is_err());
        assert!(resolve_secret(sources(None, None)).unwrap().is_none());
    }

    #[test]
    fn secret_file_is_read_and_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        std::fs::write(&path, "  0xabc\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert_eq!(read_secret_file(&path).unwrap(), "0xabc");
        assert_eq!(
            read_secret_ref(&format!("file:{}", path.display())).unwrap(),
            "0xabc"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(read_secret_file(&path).is_err());
        }
    }
}
