// crates/cli/src/auth.rs
// Publisher key binding and loopback authentication for CREG Dual-Key Architecture
// (`ADR-019` §4.2, `ECA-20`).
//
// Security properties:
// - The publisher's Ed25519 signing keypair is generated locally using OS CSPRNG
//   (`OsRng`) — NEVER deterministically derived from a secp256k1 wallet signature.
// - The EVM wallet signs an EIP-712 `BindPublisherKey` attestation binding the
//   wallet address, newly generated `publisherPubkey`, and single-use `nonce`.
// - The local loopback HTTP bridge enforces an exact `Origin` match against the
//   configured auth bridge origin (no wildcard `*`, HTTP 403 without CORS headers
//   on mismatch).

use anyhow::{bail, Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::fmt;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const DEFAULT_AUTH_BRIDGE_URL: &str = "https://spec.testnet.cregnet.dev/auth.html";
pub const DEFAULT_EIP712_DOMAIN_NAME: &str = "ChainRegistry";
pub const DEFAULT_EIP712_DOMAIN_VERSION: &str = "1";
pub const DEFAULT_CHAIN_ID: u64 = 11155111; // Ethereum Sepolia Testnet
pub const DEFAULT_VERIFYING_CONTRACT: &str = "0x30604Aa6f763fbde6aDCffbd8960af3031037331";
pub const EIP712_ACTION: &str = "Bind CREG Publisher Key";

const MAX_LOOPBACK_REQUEST_BYTES: usize = 8192;

#[derive(Clone, PartialEq, Eq)]
pub struct LocalPublisherKey {
    privkey_hex: String,
    pub pubkey_hex: String,
}

impl LocalPublisherKey {
    pub fn pubkey_hex(&self) -> &str {
        &self.pubkey_hex
    }
}

impl fmt::Debug for LocalPublisherKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalPublisherKey")
            .field("privkey_hex", &"[REDACTED]")
            .field("pubkey_hex", &self.pubkey_hex)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BoundIdentity {
    pub wallet_address: String,
    pub ed25519_privkey_hex: String,
    pub ed25519_pubkey_hex: String,
    pub binding_signature: String,
    pub binding_nonce: String,
}

impl fmt::Debug for BoundIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundIdentity")
            .field("wallet_address", &self.wallet_address)
            .field("ed25519_privkey_hex", &"[REDACTED]")
            .field("ed25519_pubkey_hex", &self.ed25519_pubkey_hex)
            .field("binding_signature", &self.binding_signature)
            .field("binding_nonce", &self.binding_nonce)
            .finish()
    }
}

#[derive(Deserialize)]
struct CallbackPayload {
    wallet: String,
    signature: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    publisher_pubkey: Option<String>,
}

#[derive(Deserialize)]
pub struct AuthTokenPayload {
    pub wallet: String,
    pub signature: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub publisher_pubkey: Option<String>,
}

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    out
}

pub fn encode_address(addr_hex: &str) -> Result<[u8; 32]> {
    let clean = addr_hex
        .trim()
        .strip_prefix("0x")
        .or_else(|| addr_hex.trim().strip_prefix("0X"))
        .unwrap_or(addr_hex.trim());
    let bytes = hex::decode(clean).context("Invalid Ethereum address hex")?;
    if bytes.len() != 20 {
        bail!("EVM address must be 20 bytes, got {}", bytes.len());
    }
    let mut word = [0u8; 32];
    word[12..32].copy_from_slice(&bytes);
    Ok(word)
}

pub fn encode_uint256(val: u64) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[24..32].copy_from_slice(&val.to_be_bytes());
    word
}

pub fn normalize_ed25519_pubkey_hex(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let clean = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let bytes = hex::decode(clean).context("Invalid Ed25519 public key hex")?;
    if bytes.len() != 32 {
        bail!("Ed25519 public key must be 32 bytes, got {}", bytes.len());
    }
    Ok(hex::encode(bytes))
}

/// Generate a fresh random Ed25519 publisher keypair from the OS CSPRNG.
pub fn generate_local_publisher_key() -> Result<LocalPublisherKey> {
    let mut seed = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);
    let signing_key = common::crypto::SigningKey::from_ed25519_seed(&seed);
    let privkey_hex = hex::encode(seed);
    seed.fill(0);
    let pubkey_hex = signing_key.public_key().to_hex();
    Ok(LocalPublisherKey {
        privkey_hex,
        pubkey_hex,
    })
}

/// Load an existing owner-only Ed25519 publisher key seed file (32-byte hex).
pub fn load_local_publisher_key(path: &Path) -> Result<LocalPublisherKey> {
    let raw_hex = crate::secret_input::read_secret_file(path)
        .with_context(|| format!("load publisher key from {}", path.display()))?;
    let trimmed = raw_hex.trim();
    let clean = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let bytes = hex::decode(clean).context("publisher key file must contain 32-byte hex seed")?;
    let mut seed: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("publisher key seed must be 32 bytes (64 hex chars)"))?;
    let signing_key = common::crypto::SigningKey::from_ed25519_seed(&seed);
    let privkey_hex = hex::encode(seed);
    seed.fill(0);
    let pubkey_hex = signing_key.public_key().to_hex();
    Ok(LocalPublisherKey {
        privkey_hex,
        pubkey_hex,
    })
}

pub fn compute_domain_separator(
    name: &str,
    version: &str,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<[u8; 32]> {
    let domain_typehash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let name_hash = keccak256(name.as_bytes());
    let version_hash = keccak256(version.as_bytes());
    let chain_id_bytes = encode_uint256(chain_id);
    let contract_bytes = encode_address(verifying_contract)?;

    let mut buf = [0u8; 160];
    buf[0..32].copy_from_slice(&domain_typehash);
    buf[32..64].copy_from_slice(&name_hash);
    buf[64..96].copy_from_slice(&version_hash);
    buf[96..128].copy_from_slice(&chain_id_bytes);
    buf[128..160].copy_from_slice(&contract_bytes);

    Ok(keccak256(&buf))
}

/// Compute the EIP-712 digest for `BindPublisherKey(string action,address wallet,string publisherPubkey,string nonce)`.
pub fn compute_binding_digest(
    wallet_address: &str,
    publisher_pubkey_hex: &str,
    nonce: &str,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<[u8; 32]> {
    if nonce.trim().is_empty() {
        bail!("binding challenge nonce must not be empty");
    }
    let normalized_pubkey = normalize_ed25519_pubkey_hex(publisher_pubkey_hex)?;
    let domain_separator = compute_domain_separator(
        DEFAULT_EIP712_DOMAIN_NAME,
        DEFAULT_EIP712_DOMAIN_VERSION,
        chain_id,
        verifying_contract,
    )?;

    let struct_typehash = keccak256(
        b"BindPublisherKey(string action,address wallet,string publisherPubkey,string nonce)",
    );
    let action_hash = keccak256(EIP712_ACTION.as_bytes());
    let wallet_bytes = encode_address(wallet_address)?;
    let pubkey_hash = keccak256(normalized_pubkey.as_bytes());
    let nonce_hash = keccak256(nonce.as_bytes());

    let mut struct_buf = [0u8; 160];
    struct_buf[0..32].copy_from_slice(&struct_typehash);
    struct_buf[32..64].copy_from_slice(&action_hash);
    struct_buf[64..96].copy_from_slice(&wallet_bytes);
    struct_buf[96..128].copy_from_slice(&pubkey_hash);
    struct_buf[128..160].copy_from_slice(&nonce_hash);
    let struct_hash = keccak256(&struct_buf);

    let mut digest_buf = [0u8; 66];
    digest_buf[0] = 0x19;
    digest_buf[1] = 0x01;
    digest_buf[2..34].copy_from_slice(&domain_separator);
    digest_buf[34..66].copy_from_slice(&struct_hash);

    Ok(keccak256(&digest_buf))
}

pub fn parse_signature_bytes(sig_hex: &str) -> Result<[u8; 65]> {
    let clean = sig_hex
        .trim()
        .strip_prefix("0x")
        .or_else(|| sig_hex.trim().strip_prefix("0X"))
        .unwrap_or(sig_hex.trim());
    let bytes = hex::decode(clean).context("Invalid signature hex string")?;
    if bytes.len() != 65 {
        bail!("Signature must be 65 bytes, got {}", bytes.len());
    }
    let mut arr = [0u8; 65];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

pub fn recover_signer(digest: &[u8; 32], sig_bytes: &[u8; 65]) -> Result<String> {
    use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};

    let v = sig_bytes[64];
    let rec_byte = match v {
        0 | 1 => v,
        27 | 28 => v - 27,
        _ => bail!("Unsupported signature recovery id (v): {}", v),
    };

    let rec_id = RecoveryId::from_byte(rec_byte)
        .ok_or_else(|| anyhow::anyhow!("Invalid recovery id: {}", rec_byte))?;
    let signature = K256Signature::try_from(&sig_bytes[..64])
        .map_err(|e| anyhow::anyhow!("Invalid ECDSA signature: {}", e))?;
    let verifying_key = VerifyingKey::recover_from_prehash(digest, &signature, rec_id)
        .map_err(|e| anyhow::anyhow!("Failed to recover ECDSA signer from signature: {}", e))?;

    let uncompressed = verifying_key.to_encoded_point(false);
    let pubkey_bytes = uncompressed.as_bytes();
    if pubkey_bytes.len() < 65 {
        bail!("Uncompressed public key is too short");
    }
    let hashed = keccak256(&pubkey_bytes[1..]);
    Ok(format!("0x{}", hex::encode(&hashed[12..32])))
}

/// Generate a fresh local Ed25519 keypair and bind it to an Ethereum wallet
/// using an explicit EVM secp256k1 private key (headless/CI mode).
pub fn bind_with_private_key(
    privkey_hex: &str,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<BoundIdentity> {
    use k256::ecdsa::SigningKey as K256SigningKey;

    let clean = privkey_hex
        .trim()
        .strip_prefix("0x")
        .or_else(|| privkey_hex.trim().strip_prefix("0X"))
        .unwrap_or(privkey_hex.trim());
    let priv_bytes = hex::decode(clean).context("Invalid private key hex string")?;
    let signing_key = K256SigningKey::from_bytes(priv_bytes.as_slice().into())
        .map_err(|e| anyhow::anyhow!("Invalid secp256k1 private key: {e}"))?;
    let verifying_key = signing_key.verifying_key();
    let uncompressed = verifying_key.to_encoded_point(false);
    let pubkey_bytes = uncompressed.as_bytes();
    let hashed = keccak256(&pubkey_bytes[1..]);
    let wallet_address = format!("0x{}", hex::encode(&hashed[12..32]));

    let local_key = generate_local_publisher_key()?;
    let binding_nonce = hex::encode(rand::random::<[u8; 16]>());
    let digest = compute_binding_digest(
        &wallet_address,
        local_key.pubkey_hex(),
        &binding_nonce,
        chain_id,
        verifying_contract,
    )?;
    let (sig, rec_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .map_err(|e| anyhow::anyhow!("ECDSA prehash signing failed: {e}"))?;

    let mut sig_bytes = [0u8; 65];
    sig_bytes[..64].copy_from_slice(sig.to_bytes().as_slice());
    sig_bytes[64] = rec_id.to_byte() + 27;

    Ok(BoundIdentity {
        wallet_address,
        ed25519_privkey_hex: local_key.privkey_hex,
        ed25519_pubkey_hex: local_key.pubkey_hex,
        binding_signature: format!("0x{}", hex::encode(sig_bytes)),
        binding_nonce,
    })
}

pub fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        // Use rundll32 to launch default browser directly without invoking cmd.exe shell delimiter parsing on '&'
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    Ok(())
}

/// Verify an authorization token (`BindPublisherKey`) against an explicit local
/// Ed25519 publisher keypair (`local_key`).
pub fn verify_binding_token(
    token_str: &str,
    local_key: &LocalPublisherKey,
    expected_nonce: Option<&str>,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<BoundIdentity> {
    use base64::Engine;

    let clean = token_str.trim().trim_matches('"').trim_matches('\'');
    let json_bytes = if clean.starts_with('{') {
        clean.as_bytes().to_vec()
    } else if let Ok(b) = base64::engine::general_purpose::STANDARD.decode(clean) {
        b
    } else if let Ok(b) = base64::engine::general_purpose::URL_SAFE.decode(clean) {
        b
    } else if let Ok(b) = hex::decode(clean) {
        b
    } else {
        bail!("Invalid token format: payload must be JSON, Base64, or Hex");
    };

    let payload: AuthTokenPayload =
        serde_json::from_slice(&json_bytes).context("Failed to parse authorization token JSON")?;

    if let Some(ref token_pubkey) = payload.publisher_pubkey {
        let normalized = normalize_ed25519_pubkey_hex(token_pubkey)?;
        if normalized != local_key.pubkey_hex {
            bail!(
                "Token publisher_pubkey {} does not match local publisher key {}",
                normalized,
                local_key.pubkey_hex
            );
        }
    }

    let token_nonce = payload
        .nonce
        .as_deref()
        .or(payload.state.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .context("Authorization token missing challenge nonce")?
        .to_string();

    if let Some(expected) = expected_nonce {
        if token_nonce != expected {
            bail!("Authorization token nonce mismatch");
        }
    }

    let sig_bytes = parse_signature_bytes(&payload.signature)?;
    let digest = compute_binding_digest(
        &payload.wallet,
        &local_key.pubkey_hex,
        &token_nonce,
        chain_id,
        verifying_contract,
    )?;
    let recovered = recover_signer(&digest, &sig_bytes)?;

    if !recovered.eq_ignore_ascii_case(&payload.wallet) {
        bail!(
            "Token signature mismatch: expected wallet {}, but signature belongs to {}",
            payload.wallet,
            recovered
        );
    }

    Ok(BoundIdentity {
        wallet_address: payload.wallet,
        ed25519_privkey_hex: local_key.privkey_hex.clone(),
        ed25519_pubkey_hex: local_key.pubkey_hex.clone(),
        binding_signature: format!("0x{}", hex::encode(sig_bytes)),
        binding_nonce: token_nonce,
    })
}

/// Extract a canonical HTTP/HTTPS origin (`scheme://host[:port]`, default ports
/// `:80`/`:443` stripped, rejecting credentials, non-HTTP schemes, and invalid hosts).
pub fn extract_origin(raw_url: &str) -> Result<String> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("null")
        || trimmed == "*"
        || trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        bail!("invalid origin or URL '{raw_url}'");
    }

    let (scheme, rest) = if let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("HTTPS://"))
    {
        ("https", rest)
    } else if let Some(rest) = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("HTTP://"))
    {
        ("http", rest)
    } else if let Some((prefix, rest)) = trimmed.split_once("://") {
        let lower = prefix.to_ascii_lowercase();
        match lower.as_str() {
            "https" => ("https", rest),
            "http" => ("http", rest),
            _ => bail!("unsupported origin scheme '{prefix}' (only http/https supported)"),
        }
    } else {
        bail!("URL/origin '{raw_url}' must start with http:// or https://");
    };

    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        bail!("invalid authority in URL/origin '{raw_url}'");
    }

    let (host_canonical, port_opt) = if let Some(after_open) = authority.strip_prefix('[') {
        let Some(close_idx) = after_open.find(']') else {
            bail!("unclosed IPv6 host in URL/origin '{raw_url}'");
        };
        let ipv6_inner = &after_open[..close_idx];
        if ipv6_inner.is_empty() {
            bail!("empty IPv6 host in URL/origin '{raw_url}'");
        }
        let remainder = &after_open[close_idx + 1..];
        let port = if remainder.is_empty() {
            None
        } else if let Some(port_str) = remainder.strip_prefix(':') {
            Some(parse_valid_port(port_str, raw_url)?)
        } else {
            bail!("invalid trailing characters after IPv6 host in '{raw_url}'");
        };
        (format!("[{}]", ipv6_inner.to_ascii_lowercase()), port)
    } else if let Some((host, port_str)) = authority.rsplit_once(':') {
        if host.is_empty() || host.contains(':') {
            bail!("invalid host in URL/origin '{raw_url}'");
        }
        let port = parse_valid_port(port_str, raw_url)?;
        (host.to_ascii_lowercase(), Some(port))
    } else {
        (authority.to_ascii_lowercase(), None)
    };

    let include_port = match (scheme, port_opt) {
        ("https", Some(443)) | ("http", Some(80)) | (_, None) => None,
        (_, Some(p)) => Some(p),
    };

    match include_port {
        Some(port) => Ok(format!("{scheme}://{host_canonical}:{port}")),
        None => Ok(format!("{scheme}://{host_canonical}")),
    }
}

fn parse_valid_port(port_str: &str, raw_url: &str) -> Result<u16> {
    if port_str.is_empty() || !port_str.bytes().all(|b| b.is_ascii_digit()) {
        bail!("invalid port '{port_str}' in URL/origin '{raw_url}'");
    }
    let port: u16 = port_str
        .parse()
        .with_context(|| format!("port out of range in '{raw_url}'"))?;
    if port == 0 {
        bail!("port 0 is invalid in '{raw_url}'");
    }
    Ok(port)
}

fn is_loopback_origin(canonical_origin: &str) -> bool {
    let rest = canonical_origin
        .strip_prefix("http://")
        .or_else(|| canonical_origin.strip_prefix("https://"))
        .unwrap_or("");
    let host = if rest.starts_with('[') {
        rest.find(']').map(|i| &rest[..=i]).unwrap_or("")
    } else {
        rest.split(':').next().unwrap_or("")
    };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

/// Resolve the configured web auth bridge URL and its canonical HTTP origin.
pub fn resolve_auth_bridge_url() -> Result<(String, String)> {
    let raw = std::env::var("CREG_AUTH_BRIDGE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_AUTH_BRIDGE_URL.to_string());
    let origin = extract_origin(&raw)?;
    if origin.starts_with("http://") && !is_loopback_origin(&origin) {
        bail!("non-loopback CREG_AUTH_BRIDGE_URL must use https:// (got origin '{origin}')");
    }
    Ok((raw, origin))
}

/// Extract a unique HTTP header value from the header section (never scans body).
/// Returns `None` if the header is missing or duplicated (to prevent smuggling).
pub fn extract_http_header<'a>(raw_headers: &'a str, header_name: &str) -> Option<&'a str> {
    let mut found: Option<&'a str> = None;
    for line in raw_headers.lines().skip(1) {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, val)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case(header_name) {
                if found.is_some() {
                    return None;
                }
                found = Some(val.trim());
            }
        }
    }
    found
}

/// Strictly match an HTTP `Origin` header against the exact bridge origin.
/// Rejects bare paths/queries/fragments, `null`, `*`, and differing scheme/host/port.
pub fn is_origin_allowed(origin_header: &str, allowed_origin: &str) -> bool {
    let trimmed = origin_header.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") || trimmed == "*" {
        return false;
    }
    if let Some((_, rest)) = trimmed.split_once("://") {
        if rest.contains('/') || rest.contains('?') || rest.contains('#') {
            return false;
        }
    } else {
        return false;
    }
    let Ok(parsed_header) = extract_origin(trimmed) else {
        return false;
    };
    let Ok(parsed_allowed) = extract_origin(allowed_origin) else {
        return false;
    };
    parsed_header == parsed_allowed
}

async fn read_http_request(socket: &mut TcpStream) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(2048);
    let mut tmp = [0u8; 2048];
    let mut header_end: Option<usize> = None;
    let mut expected_total_len: Option<usize> = None;

    loop {
        if let (Some(h_end), Some(total)) = (header_end, expected_total_len) {
            if buf.len() >= total.min(MAX_LOOPBACK_REQUEST_BYTES) {
                break;
            }
            if h_end > MAX_LOOPBACK_REQUEST_BYTES {
                break;
            }
        }
        let read_fut = socket.read(&mut tmp);
        let n = match tokio::time::timeout(Duration::from_secs(5), read_fut).await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        let remaining = MAX_LOOPBACK_REQUEST_BYTES.saturating_sub(buf.len());
        buf.extend_from_slice(&tmp[..n.min(remaining)]);

        if header_end.is_none() {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let end = pos + 4;
                header_end = Some(end);
                let head_str = String::from_utf8_lossy(&buf[..pos]);
                let content_len = extract_http_header(&head_str, "content-length")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                expected_total_len = Some(end.saturating_add(content_len));
                if content_len == 0 || buf.len() >= end.saturating_add(content_len) {
                    break;
                }
            }
        }
        if buf.len() >= MAX_LOOPBACK_REQUEST_BYTES {
            break;
        }
    }
    Ok(buf)
}

fn http_forbidden_no_cors(message: &str) -> String {
    let body = format!("{{\"error\":\"{message}\"}}");
    format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn http_json_with_origin(status_line: &str, allowed_origin: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status_line}\r\nAccess-Control-Allow-Origin: {allowed_origin}\r\nVary: Origin\r\nAccess-Control-Allow-Private-Network: true\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

pub async fn start_loopback_auth(
    port: u16,
    no_browser: bool,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<BoundIdentity> {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind local loopback server on {}", addr))?;

    let local_key = generate_local_publisher_key()?;
    let state_nonce = hex::encode(rand::random::<[u8; 16]>());
    let (bridge_url, allowed_origin) = resolve_auth_bridge_url()?;
    let sep = if bridge_url.contains('?') { '&' } else { '?' };
    let login_url = format!(
        "{bridge_url}{sep}port={port}&state={state_nonce}&chainId={chain_id}&contract={verifying_contract}&pubkey={}",
        local_key.pubkey_hex()
    );

    println!();
    println!(
        "{}",
        "╭────────────────────────────────────────────────────────────╮".cyan()
    );
    println!(
        "{}",
        "│   CREG Wallet Authentication & Key Binding                 │"
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "│   Opening browser to bind a locally generated publisher key│".cyan()
    );
    println!(
        "{}",
        "╰────────────────────────────────────────────────────────────╯".cyan()
    );
    println!();
    println!("  Local Ed25519 Pubkey: {}", local_key.pubkey_hex().cyan());
    println!("  Allowed Bridge Origin: {}", allowed_origin.dimmed());
    println!();
    println!("  If the browser did not open automatically, visit:");
    println!("  {}", login_url.underline());
    println!();
    println!("  Waiting for browser authorization...");
    println!(
        "  {}",
        "(Tip: If your browser cannot reach localhost, copy the token and paste it here)".dimmed()
    );

    if !no_browser {
        if let Err(e) = open_browser(&login_url) {
            tracing::warn!("Could not launch default browser automatically: {}", e);
        }
    }

    let timeout_duration = Duration::from_secs(180);
    tokio::select! {
        res = accept_auth_loop(listener, &local_key, &state_nonce, &allowed_origin, chain_id, verifying_contract) => {
            res
        }
        Some(token_str) = read_terminal_token() => {
            println!("  Verifying pasted binding authorization token...");
            verify_binding_token(&token_str, &local_key, Some(&state_nonce), chain_id, verifying_contract)
        }
        _ = tokio::time::sleep(timeout_duration) => {
            bail!("Authentication timed out (180s). Please run 'creg login' to try again.");
        }
    }
}

async fn read_terminal_token() -> Option<String> {
    use tokio::io::AsyncBufReadExt;
    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin).lines();
    if let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

pub(crate) async fn accept_auth_loop(
    listener: TcpListener,
    local_key: &LocalPublisherKey,
    expected_state: &str,
    allowed_origin: &str,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<BoundIdentity> {
    let canonical_allowed = extract_origin(allowed_origin)?;
    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .context("Error accepting incoming loopback connection")?;

        let raw = match read_http_request(&mut socket).await {
            Ok(bytes) if !bytes.is_empty() => bytes,
            _ => continue,
        };

        let request = String::from_utf8_lossy(&raw);
        let (headers_part, body_part) = match request.split_once("\r\n\r\n") {
            Some((h, b)) => (h, b),
            None => (request.as_ref(), ""),
        };

        let origin_header = extract_http_header(headers_part, "origin");
        let origin_ok = origin_header.is_some_and(|hdr| is_origin_allowed(hdr, &canonical_allowed));

        if !origin_ok {
            let res = http_forbidden_no_cors("Origin not allowed");
            let _ = socket.write_all(res.as_bytes()).await;
            continue;
        }

        if request.starts_with("OPTIONS ") {
            let cors_response = format!(
                "HTTP/1.1 200 OK\r\n\
Access-Control-Allow-Origin: {canonical_allowed}\r\n\
Vary: Origin\r\n\
Access-Control-Allow-Methods: POST, GET, OPTIONS\r\n\
Access-Control-Allow-Headers: Content-Type, Access-Control-Request-Private-Network\r\n\
Access-Control-Allow-Private-Network: true\r\n\
Content-Length: 0\r\n\
Connection: close\r\n\r\n"
            );
            let _ = socket.write_all(cors_response.as_bytes()).await;
            continue;
        }

        if request.starts_with("GET /health") {
            let health_res = http_json_with_origin(
                "200 OK",
                &canonical_allowed,
                "{\"status\":\"ok\",\"service\":\"creg-cli-loopback\"}",
            );
            let _ = socket.write_all(health_res.as_bytes()).await;
            continue;
        }

        if request.starts_with("POST ") {
            let payload: CallbackPayload = match serde_json::from_str(body_part.trim()) {
                Ok(p) => p,
                Err(e) => {
                    let body =
                        serde_json::json!({ "error": format!("Invalid JSON: {e}") }).to_string();
                    let err_res =
                        http_json_with_origin("400 Bad Request", &canonical_allowed, &body);
                    let _ = socket.write_all(err_res.as_bytes()).await;
                    continue;
                }
            };

            let callback_nonce = payload
                .nonce
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(payload.state.as_str())
                .trim();

            if callback_nonce != expected_state {
                let err_res = http_json_with_origin(
                    "403 Forbidden",
                    &canonical_allowed,
                    "{\"error\":\"State/nonce mismatch\"}",
                );
                let _ = socket.write_all(err_res.as_bytes()).await;
                continue;
            }

            if let Some(ref echoed_pubkey) = payload.publisher_pubkey {
                match normalize_ed25519_pubkey_hex(echoed_pubkey) {
                    Ok(norm) if norm == local_key.pubkey_hex => {}
                    _ => {
                        let err_res = http_json_with_origin(
                            "403 Forbidden",
                            &canonical_allowed,
                            "{\"error\":\"Publisher public key mismatch\"}",
                        );
                        let _ = socket.write_all(err_res.as_bytes()).await;
                        continue;
                    }
                }
            }

            let sig_bytes = match parse_signature_bytes(&payload.signature) {
                Ok(sb) => sb,
                Err(e) => {
                    let body = serde_json::json!({ "error": e.to_string() }).to_string();
                    let err_res =
                        http_json_with_origin("400 Bad Request", &canonical_allowed, &body);
                    let _ = socket.write_all(err_res.as_bytes()).await;
                    continue;
                }
            };

            let digest = match compute_binding_digest(
                &payload.wallet,
                &local_key.pubkey_hex,
                callback_nonce,
                chain_id,
                verifying_contract,
            ) {
                Ok(d) => d,
                Err(e) => {
                    let body = serde_json::json!({ "error": e.to_string() }).to_string();
                    let err_res =
                        http_json_with_origin("500 Internal Error", &canonical_allowed, &body);
                    let _ = socket.write_all(err_res.as_bytes()).await;
                    continue;
                }
            };

            let recovered = match recover_signer(&digest, &sig_bytes) {
                Ok(r) => r,
                Err(e) => {
                    let body =
                        serde_json::json!({ "error": format!("Signer recovery failed: {e}") })
                            .to_string();
                    let err_res =
                        http_json_with_origin("400 Bad Request", &canonical_allowed, &body);
                    let _ = socket.write_all(err_res.as_bytes()).await;
                    continue;
                }
            };

            if !recovered.eq_ignore_ascii_case(&payload.wallet) {
                let err_res = http_json_with_origin(
                    "401 Unauthorized",
                    &canonical_allowed,
                    "{\"error\":\"Recovered address does not match wallet\"}",
                );
                let _ = socket.write_all(err_res.as_bytes()).await;
                continue;
            }

            let success_body = serde_json::json!({
                "status": "ok",
                "wallet": payload.wallet,
                "pubkey": local_key.pubkey_hex,
            })
            .to_string();
            let success_res = http_json_with_origin("200 OK", &canonical_allowed, &success_body);
            let _ = socket.write_all(success_res.as_bytes()).await;

            return Ok(BoundIdentity {
                wallet_address: payload.wallet,
                ed25519_privkey_hex: local_key.privkey_hex.clone(),
                ed25519_pubkey_hex: local_key.pubkey_hex.clone(),
                binding_signature: format!("0x{}", hex::encode(sig_bytes)),
                binding_nonce: callback_nonce.to_string(),
            });
        }

        let res = http_json_with_origin(
            "404 Not Found",
            &canonical_allowed,
            "{\"error\":\"Unknown loopback endpoint\"}",
        );
        let _ = socket.write_all(res.as_bytes()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn sign_binding_with_evm_privkey(
        evm_privkey: &str,
        wallet: &str,
        publisher_pubkey: &str,
        nonce: &str,
    ) -> String {
        let clean = evm_privkey.trim().strip_prefix("0x").unwrap();
        let signing_key =
            k256::ecdsa::SigningKey::from_bytes(hex::decode(clean).unwrap().as_slice().into())
                .unwrap();
        let digest = compute_binding_digest(
            wallet,
            publisher_pubkey,
            nonce,
            DEFAULT_CHAIN_ID,
            DEFAULT_VERIFYING_CONTRACT,
        )
        .unwrap();
        let (sig, rec_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
        let mut sig_bytes = [0u8; 65];
        sig_bytes[..64].copy_from_slice(sig.to_bytes().as_slice());
        sig_bytes[64] = rec_id.to_byte() + 27;
        format!("0x{}", hex::encode(sig_bytes))
    }

    #[test]
    fn test_encode_address() {
        let addr = "0x30604Aa6f763fbde6aDCffbd8960af3031037331";
        let encoded = encode_address(addr).unwrap();
        assert_eq!(&encoded[0..12], &[0u8; 12]);
        assert_eq!(
            hex::encode(&encoded[12..32]),
            "30604aa6f763fbde6adcffbd8960af3031037331"
        );
    }

    #[test]
    fn test_encode_uint256() {
        let val: u64 = 11155111;
        let encoded = encode_uint256(val);
        assert_eq!(&encoded[0..24], &[0u8; 24]);
        assert_eq!(&encoded[24..32], &val.to_be_bytes());
    }

    #[test]
    fn bind_with_private_key_generates_distinct_random_ed25519_keys() {
        let privkey = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let id1 =
            bind_with_private_key(privkey, DEFAULT_CHAIN_ID, DEFAULT_VERIFYING_CONTRACT).unwrap();
        let id2 =
            bind_with_private_key(privkey, DEFAULT_CHAIN_ID, DEFAULT_VERIFYING_CONTRACT).unwrap();

        // Anvil account #0 standard address
        assert_eq!(
            id1.wallet_address.to_lowercase(),
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
        );
        assert_eq!(id1.wallet_address, id2.wallet_address);
        // ECA-20: local CSPRNG Ed25519 keypair must NOT be deterministically derived from EVM signature.
        assert_ne!(id1.ed25519_privkey_hex, id2.ed25519_privkey_hex);
        assert_ne!(id1.ed25519_pubkey_hex, id2.ed25519_pubkey_hex);
        assert_ne!(id1.binding_nonce, id2.binding_nonce);
        assert_eq!(id1.ed25519_privkey_hex.len(), 64);
        assert_eq!(id1.ed25519_pubkey_hex.len(), 64);

        // Both binding signatures must verify cleanly against their own bound pubkeys.
        let digest1 = compute_binding_digest(
            &id1.wallet_address,
            &id1.ed25519_pubkey_hex,
            &id1.binding_nonce,
            DEFAULT_CHAIN_ID,
            DEFAULT_VERIFYING_CONTRACT,
        )
        .unwrap();
        let sig1 = parse_signature_bytes(&id1.binding_signature).unwrap();
        assert_eq!(
            recover_signer(&digest1, &sig1).unwrap().to_lowercase(),
            id1.wallet_address.to_lowercase()
        );
    }

    #[test]
    fn verify_binding_token_checks_local_key_and_rejects_mismatched_pubkey() {
        use base64::Engine;

        let dir = TempDir::new().unwrap();
        let local_key = generate_local_publisher_key().unwrap();
        let key_file = dir.path().join("publisher.key");
        {
            let mut f = std::fs::File::create(&key_file).unwrap();
            writeln!(f, "{}", local_key.privkey_hex).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
        }
        let reloaded = load_local_publisher_key(&key_file).unwrap();
        assert_eq!(reloaded.pubkey_hex, local_key.pubkey_hex);

        let privkey = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
        let wallet = "0x70997970C51812dc3a010C7d01b50e0d17dc79C8";
        let nonce = "deadbeefcafebabe";
        let sig_hex = sign_binding_with_evm_privkey(privkey, wallet, &local_key.pubkey_hex, nonce);

        let json = serde_json::json!({
            "wallet": wallet,
            "signature": sig_hex,
            "nonce": nonce,
            "publisher_pubkey": local_key.pubkey_hex
        })
        .to_string();
        let b64_token = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());

        let bound = verify_binding_token(
            &b64_token,
            &reloaded,
            Some(nonce),
            DEFAULT_CHAIN_ID,
            DEFAULT_VERIFYING_CONTRACT,
        )
        .unwrap();
        assert_eq!(bound.wallet_address, wallet);
        assert_eq!(bound.ed25519_pubkey_hex, local_key.pubkey_hex);
        assert_eq!(bound.binding_nonce, nonce);

        // Wrong local key (even if token omits publisher_pubkey) must fail signature recovery check.
        let other_key = generate_local_publisher_key().unwrap();
        assert!(verify_binding_token(
            &b64_token,
            &other_key,
            Some(nonce),
            DEFAULT_CHAIN_ID,
            DEFAULT_VERIFYING_CONTRACT,
        )
        .is_err());

        // Wrong expected nonce must also fail.
        assert!(verify_binding_token(
            &b64_token,
            &reloaded,
            Some("other-nonce"),
            DEFAULT_CHAIN_ID,
            DEFAULT_VERIFYING_CONTRACT,
        )
        .is_err());
    }

    #[test]
    fn origin_parser_and_matcher_handle_ports_ipv6_and_invalid_origins() {
        assert_eq!(
            extract_origin("https://Spec.Testnet.CregNet.dev:443/auth.html?port=18888").unwrap(),
            "https://spec.testnet.cregnet.dev"
        );
        assert_eq!(
            extract_origin("http://127.0.0.1:8080/auth.html").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            extract_origin("http://[::1]:3000/callback").unwrap(),
            "http://[::1]:3000"
        );
        assert!(extract_origin("https://user@spec.testnet.cregnet.dev").is_err());
        assert!(extract_origin("javascript:alert(1)").is_err());
        assert!(extract_origin("null").is_err());

        let allowed = "https://spec.testnet.cregnet.dev";
        assert!(is_origin_allowed(
            "https://spec.testnet.cregnet.dev",
            allowed
        ));
        assert!(is_origin_allowed(
            "https://SPEC.TESTNET.CREGNET.DEV:443",
            allowed
        ));
        assert!(!is_origin_allowed(
            "http://spec.testnet.cregnet.dev",
            allowed
        ));
        assert!(!is_origin_allowed(
            "https://evil.spec.testnet.cregnet.dev",
            allowed
        ));
        assert!(!is_origin_allowed(
            "https://spec.testnet.cregnet.dev/path",
            allowed
        ));
        assert!(!is_origin_allowed("*", allowed));
        assert!(!is_origin_allowed("null", allowed));
    }

    #[tokio::test]
    async fn loopback_cors_returns_403_on_mismatch_and_exact_origin_on_match() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let local_key = generate_local_publisher_key().unwrap();
        let expected_nonce = "nonce-loopback-cors-test".to_string();
        let allowed_origin = "https://spec.testnet.cregnet.dev".to_string();

        let privkey = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let wallet = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
        let sig_hex =
            sign_binding_with_evm_privkey(privkey, wallet, local_key.pubkey_hex(), &expected_nonce);

        let key_for_srv = local_key.clone();
        let nonce_for_srv = expected_nonce.clone();
        let origin_for_srv = allowed_origin.clone();
        let server_task = tokio::spawn(async move {
            accept_auth_loop(
                listener,
                &key_for_srv,
                &nonce_for_srv,
                &origin_for_srv,
                DEFAULT_CHAIN_ID,
                DEFAULT_VERIFYING_CONTRACT,
            )
            .await
        });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base_url = format!("http://{addr}");

        // 1. Disallowed origin on OPTIONS preflight -> 403 with zero CORS headers.
        let evil_opt = client
            .request(reqwest::Method::OPTIONS, &base_url)
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(evil_opt.status(), reqwest::StatusCode::FORBIDDEN);
        assert!(evil_opt
            .headers()
            .get("access-control-allow-origin")
            .is_none());
        assert!(evil_opt
            .headers()
            .get("access-control-allow-private-network")
            .is_none());

        // 2. Disallowed origin on POST -> 403 with zero CORS headers, server stays alive.
        let evil_post = client
            .post(&base_url)
            .header("Origin", "https://evil.example")
            .json(&serde_json::json!({
                "wallet": wallet,
                "signature": sig_hex,
                "nonce": expected_nonce,
                "publisher_pubkey": local_key.pubkey_hex(),
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(evil_post.status(), reqwest::StatusCode::FORBIDDEN);
        assert!(evil_post
            .headers()
            .get("access-control-allow-origin")
            .is_none());

        // 3. Exact bridge origin on OPTIONS preflight -> 200 with exact origin (never '*').
        let ok_opt = client
            .request(reqwest::Method::OPTIONS, &base_url)
            .header("Origin", &allowed_origin)
            .send()
            .await
            .unwrap();
        assert_eq!(ok_opt.status(), reqwest::StatusCode::OK);
        assert_eq!(
            ok_opt
                .headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some(allowed_origin.as_str())
        );

        // 4. Exact bridge origin on POST -> 200 with exact origin and completes binding.
        let ok_post = client
            .post(&base_url)
            .header("Origin", &allowed_origin)
            .json(&serde_json::json!({
                "wallet": wallet,
                "signature": sig_hex,
                "nonce": expected_nonce,
                "state": expected_nonce,
                "publisher_pubkey": local_key.pubkey_hex(),
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(ok_post.status(), reqwest::StatusCode::OK);
        assert_eq!(
            ok_post
                .headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some(allowed_origin.as_str())
        );

        let bound = server_task.await.unwrap().unwrap();
        assert_eq!(bound.wallet_address, wallet);
        assert_eq!(bound.ed25519_pubkey_hex, local_key.pubkey_hex);
        assert_eq!(bound.binding_nonce, expected_nonce);
    }
}
