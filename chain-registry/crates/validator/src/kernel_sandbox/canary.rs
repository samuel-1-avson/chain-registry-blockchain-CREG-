//! Ephemeral developer credential canaries & multi-encoding egress detector.
//!
//! Each Tier 1 / Tier 2 sandbox execution receives uniquely minted honeypot
//! tokens placed at common credential paths and environment variables. Any leak
//! appearing in stdout, stderr, sinkhole DNS/TLS/TCP payloads, or overlayfs
//! artifacts (raw, base64, base64url, hex, or URL-encoded) triggers a
//! deterministic `KB-CANARY-EXFIL` critical violation.

use super::schema::{CanaryChannel, CanaryEncoding, CanaryExfiltration};
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct CanarySecret {
    pub name: String,
    pub rel_file: Option<&'static str>,
    pub env_var: Option<&'static str>,
    pub token: String,
    patterns: Vec<(CanaryEncoding, Vec<u8>)>,
}

#[derive(Debug, Clone)]
pub struct CanaryProfile {
    pub secrets: Vec<CanarySecret>,
}

impl CanaryProfile {
    /// Generate a fresh per-run canary profile with high-entropy tokens bound to
    /// `run_nonce` (e.g. a UUID or SHA-256 seed).
    pub fn for_run(run_seed: &[u8]) -> Self {
        let specs: [(&str, Option<&'static str>, Option<&'static str>); 5] = [
            ("ssh_id_rsa", Some(".ssh/id_rsa"), None),
            (
                "aws_credentials",
                Some(".aws/credentials"),
                Some("AWS_SECRET_ACCESS_KEY"),
            ),
            ("npmrc_token", Some(".npmrc"), Some("NPM_TOKEN")),
            ("pypirc_token", Some(".pypirc"), Some("GITHUB_TOKEN")),
            (
                "dotenv_openai",
                Some("workspace/.env"),
                Some("OPENAI_API_KEY"),
            ),
        ];

        let secrets = specs
            .iter()
            .enumerate()
            .map(|(idx, (name, rel_file, env_var))| {
                let mut h = Sha256::new();
                h.update(b"CREG-CANARY-V1:");
                h.update([idx as u8]);
                h.update(run_seed);
                let digest = hex::encode(h.finalize());
                let token = format!("CREG_CANARY_{}", &digest[..24]);
                let patterns = build_token_variants(&token);
                CanarySecret {
                    name: (*name).to_string(),
                    rel_file: *rel_file,
                    env_var: *env_var,
                    token,
                    patterns,
                }
            })
            .collect();

        Self { secrets }
    }

    pub fn from_env_run() -> Self {
        let id = uuid::Uuid::new_v4();
        Self::for_run(id.as_bytes())
    }

    /// Key-value environment variables injected into the sandboxed process.
    pub fn env_vars(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for s in &self.secrets {
            if let Some(env_key) = s.env_var {
                map.insert(env_key.to_string(), s.token.clone());
            }
        }
        map
    }

    /// Materialize realistic decoy files under `decoy_home` (bind-mounted as
    /// `/home/sandbox` inside nsjail or copied into the Firecracker overlay).
    pub fn stage_decoy_home(&self, decoy_home: &Path) -> std::io::Result<()> {
        for s in &self.secrets {
            let Some(rel) = s.rel_file else {
                continue;
            };
            let full = decoy_home.join(rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let body = match s.name.as_str() {
                "ssh_id_rsa" => format!(
                    "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
                    s.token
                ),
                "aws_credentials" => format!(
                    "[default]\naws_access_key_id = AKIACREGCANARYTEST\naws_secret_access_key = {}\n",
                    s.token
                ),
                "npmrc_token" => format!("//registry.npmjs.org/:_authToken={}\n", s.token),
                "pypirc_token" => format!("[pypi]\nusername = __token__\npassword = {}\n", s.token),
                _ => format!("OPENAI_API_KEY={}\n", s.token),
            };
            std::fs::write(full, body)?;
        }
        Ok(())
    }

    /// Scan an arbitrary byte payload captured on `channel` for any encoded form
    /// of any canary secret in this profile.
    pub fn scan_payload(&self, channel: CanaryChannel, payload: &[u8]) -> Vec<CanaryExfiltration> {
        if payload.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for secret in &self.secrets {
            for (encoding, needle) in &secret.patterns {
                if !needle.is_empty() && find_subslice(payload, needle).is_some() {
                    hits.push(CanaryExfiltration {
                        canary_name: secret.name.clone(),
                        channel,
                        encoding: *encoding,
                        context: format!("matched {} byte needle", needle.len()),
                    });
                    break;
                }
            }
        }
        hits
    }
}

fn build_token_variants(token: &str) -> Vec<(CanaryEncoding, Vec<u8>)> {
    let raw = token.as_bytes();
    let raw_lower = token.to_ascii_lowercase().into_bytes();
    let b64 = STANDARD.encode(raw).into_bytes();
    let b64_nopad = STANDARD_NO_PAD.encode(raw).into_bytes();
    let b64_url = URL_SAFE.encode(raw).into_bytes();
    let b64_url_nopad = URL_SAFE_NO_PAD.encode(raw).into_bytes();
    let hex_low = hex::encode(raw).into_bytes();
    let hex_up = hex::encode_upper(raw).into_bytes();
    let url_enc = url_encode_all(raw);
    let url_pct_underscore = token.replace('_', "%5F").into_bytes();
    let url_pct_underscore_lower = token.replace('_', "%5f").into_bytes();

    vec![
        (CanaryEncoding::Raw, raw.to_vec()),
        (CanaryEncoding::Raw, raw_lower),
        (CanaryEncoding::Base64, b64),
        (CanaryEncoding::Base64, b64_nopad),
        (CanaryEncoding::Base64Url, b64_url),
        (CanaryEncoding::Base64Url, b64_url_nopad),
        (CanaryEncoding::HexLower, hex_low),
        (CanaryEncoding::HexUpper, hex_up),
        (CanaryEncoding::UrlEncoded, url_enc),
        (CanaryEncoding::UrlEncoded, url_pct_underscore),
        (CanaryEncoding::UrlEncoded, url_pct_underscore_lower),
    ]
}

fn url_encode_all(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 3);
    for &b in bytes {
        out.extend_from_slice(format!("%{:02X}", b).as_bytes());
    }
    out
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
