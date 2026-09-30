// crates/cli/src/auth.rs
// Cryptographic derivation and loopback authentication for CREG Dual-Key Architecture.
// Enables seamless login with MetaMask/EVM wallets via EIP-712 and HKDF-SHA512.

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const DEFAULT_EIP712_DOMAIN_NAME: &str = "ChainRegistry";
pub const DEFAULT_EIP712_DOMAIN_VERSION: &str = "1";
pub const DEFAULT_CHAIN_ID: u64 = 11155111; // Ethereum Sepolia Testnet
pub const DEFAULT_VERIFYING_CONTRACT: &str = "0x30604Aa6f763fbde6aDCffbd8960af3031037331";
pub const EIP712_ACTION: &str = "Derive CREG Consensus Key";
pub const EIP712_SALT: &str = "CREG_IDENTITY_V1";
pub const HKDF_INFO: &[u8] = b"creg-ed25519-identity";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedIdentity {
    pub wallet_address: String,
    pub ed25519_privkey_hex: String,
    pub ed25519_pubkey_hex: String,
}

#[derive(Deserialize)]
struct CallbackPayload {
    wallet: String,
    signature: String,
    state: String,
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
        .unwrap_or(addr_hex.trim());
    let bytes = hex::decode(clean).context("Invalid Ethereum address hex")?;
    if bytes.len() != 20 {
        anyhow::bail!("EVM address must be 20 bytes, got {}", bytes.len());
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

pub fn compute_derivation_digest(
    wallet_address: &str,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<[u8; 32]> {
    let domain_separator = compute_domain_separator(
        DEFAULT_EIP712_DOMAIN_NAME,
        DEFAULT_EIP712_DOMAIN_VERSION,
        chain_id,
        verifying_contract,
    )?;

    let struct_typehash = keccak256(b"DeriveIdentity(string action,address wallet,string salt)");
    let action_hash = keccak256(EIP712_ACTION.as_bytes());
    let wallet_bytes = encode_address(wallet_address)?;
    let salt_hash = keccak256(EIP712_SALT.as_bytes());

    let mut struct_buf = [0u8; 128];
    struct_buf[0..32].copy_from_slice(&struct_typehash);
    struct_buf[32..64].copy_from_slice(&action_hash);
    struct_buf[64..96].copy_from_slice(&wallet_bytes);
    struct_buf[96..128].copy_from_slice(&salt_hash);
    let struct_hash = keccak256(&struct_buf);

    let mut digest_buf = [0u8; 66];
    digest_buf[0] = 0x19;
    digest_buf[1] = 0x01;
    digest_buf[2..34].copy_from_slice(&domain_separator);
    digest_buf[34..66].copy_from_slice(&struct_hash);

    Ok(keccak256(&digest_buf))
}

pub fn parse_signature_bytes(sig_hex: &str) -> Result<[u8; 65]> {
    let clean = sig_hex.trim().strip_prefix("0x").unwrap_or(sig_hex.trim());
    let bytes = hex::decode(clean).context("Invalid signature hex string")?;
    if bytes.len() != 65 {
        anyhow::bail!("Signature must be 65 bytes, got {}", bytes.len());
    }
    let mut arr = [0u8; 65];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

pub fn recover_signer(digest: &[u8; 32], sig_bytes: &[u8; 65]) -> Result<String> {
    use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};
    use k256::elliptic_curve::sec1::ToEncodedPoint;

    let v = sig_bytes[64];
    let rec_byte = match v {
        0 | 1 => v,
        27 | 28 => v - 27,
        _ => anyhow::bail!("Unsupported signature recovery id (v): {}", v),
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
        anyhow::bail!("Uncompressed public key is too short");
    }
    let hashed = keccak256(&pubkey_bytes[1..]);
    Ok(format!("0x{}", hex::encode(&hashed[12..32])))
}

pub fn hkdf_sha512(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Result<Vec<u8>> {
    use hmac::{Hmac, Mac};
    use sha2::Sha512;

    type HmacSha512 = Hmac<Sha512>;

    let mut mac_extract = HmacSha512::new_from_slice(salt)
        .map_err(|e| anyhow::anyhow!("HMAC salt setup error: {e}"))?;
    mac_extract.update(ikm);
    let prk = mac_extract.finalize().into_bytes();

    let mut mac_expand = HmacSha512::new_from_slice(&prk)
        .map_err(|e| anyhow::anyhow!("HMAC PRK expand error: {e}"))?;
    mac_expand.update(info);
    mac_expand.update(&[1u8]);
    let t1 = mac_expand.finalize().into_bytes();

    if len <= t1.len() {
        Ok(t1[..len].to_vec())
    } else {
        anyhow::bail!("Requested HKDF output length exceeds single block capacity");
    }
}

pub fn derive_ed25519_key_from_signature(
    sig_bytes: &[u8; 65],
) -> Result<(ed25519_dalek::SigningKey, String)> {
    let seed = hkdf_sha512(EIP712_SALT.as_bytes(), sig_bytes, HKDF_INFO, 32)?;
    let seed_arr: [u8; 32] = seed
        .as_slice()
        .try_into()
        .context("Seed length must be 32 bytes")?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed_arr);
    let pubkey_hex = hex::encode(signing_key.verifying_key().as_bytes());
    Ok((signing_key, pubkey_hex))
}

pub fn derive_from_private_key(
    privkey_hex: &str,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<DerivedIdentity> {
    use k256::ecdsa::SigningKey as K256SigningKey;
    use k256::elliptic_curve::sec1::ToEncodedPoint;

    let clean = privkey_hex
        .trim()
        .strip_prefix("0x")
        .unwrap_or(privkey_hex.trim());
    let priv_bytes = hex::decode(clean).context("Invalid private key hex string")?;
    let signing_key = K256SigningKey::from_bytes(priv_bytes.as_slice().into())
        .map_err(|e| anyhow::anyhow!("Invalid secp256k1 private key: {e}"))?;
    let verifying_key = signing_key.verifying_key();
    let uncompressed = verifying_key.to_encoded_point(false);
    let pubkey_bytes = uncompressed.as_bytes();
    let hashed = keccak256(&pubkey_bytes[1..]);
    let wallet_address = format!("0x{}", hex::encode(&hashed[12..32]));

    let digest = compute_derivation_digest(&wallet_address, chain_id, verifying_contract)?;
    let (sig, rec_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .map_err(|e| anyhow::anyhow!("ECDSA prehash signing failed: {e}"))?;

    let mut sig_bytes = [0u8; 65];
    sig_bytes[..64].copy_from_slice(sig.to_bytes().as_slice());
    sig_bytes[64] = rec_id.to_byte() + 27;

    let (ed_key, ed_pubkey) = derive_ed25519_key_from_signature(&sig_bytes)?;

    Ok(DerivedIdentity {
        wallet_address,
        ed25519_privkey_hex: hex::encode(ed_key.as_bytes()),
        ed25519_pubkey_hex: ed_pubkey,
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

#[derive(Deserialize)]
pub struct AuthTokenPayload {
    pub wallet: String,
    pub signature: String,
    #[serde(default)]
    pub state: Option<String>,
}

pub fn derive_from_token(
    token_str: &str,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<DerivedIdentity> {
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
        anyhow::bail!("Invalid token format: payload must be JSON, Base64, or Hex");
    };

    let payload: AuthTokenPayload =
        serde_json::from_slice(&json_bytes).context("Failed to parse authorization token JSON")?;

    let sig_bytes = parse_signature_bytes(&payload.signature)?;
    let digest = compute_derivation_digest(&payload.wallet, chain_id, verifying_contract)?;
    let recovered = recover_signer(&digest, &sig_bytes)?;

    if recovered.to_ascii_lowercase() != payload.wallet.to_ascii_lowercase() {
        anyhow::bail!(
            "Token signature mismatch: expected wallet {}, but signature belongs to {}",
            payload.wallet,
            recovered
        );
    }

    let (ed_key, ed_pubkey) = derive_ed25519_key_from_signature(&sig_bytes)?;

    Ok(DerivedIdentity {
        wallet_address: payload.wallet,
        ed25519_privkey_hex: hex::encode(ed_key.as_bytes()),
        ed25519_pubkey_hex: ed_pubkey,
    })
}

pub async fn start_loopback_auth(
    port: u16,
    no_browser: bool,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<DerivedIdentity> {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind local loopback server on {}", addr))?;

    let state_nonce = hex::encode(rand::random::<[u8; 16]>());
    let login_url = format!(
        "https://spec.testnet.cregnet.dev/auth.html?port={}&state={}&chainId={}&contract={}",
        port, state_nonce, chain_id, verifying_contract
    );

    println!();
    println!(
        "{}",
        "╭────────────────────────────────────────────────────────────╮".cyan()
    );
    println!(
        "{}",
        "│   CREG Wallet Authentication                               │"
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "│   Opening browser to connect your Ethereum wallet...       │".cyan()
    );
    println!(
        "{}",
        "╰────────────────────────────────────────────────────────────╯".cyan()
    );
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
        res = accept_auth_loop(listener, &state_nonce, chain_id, verifying_contract) => {
            res
        }
        Some(token_str) = read_terminal_token() => {
            println!("  Processing pasted authorization token...");
            derive_from_token(&token_str, chain_id, verifying_contract)
        }
        _ = tokio::time::sleep(timeout_duration) => {
            anyhow::bail!("Authentication timed out (180s). Please run 'creg login' to try again.");
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

async fn accept_auth_loop(
    listener: TcpListener,
    expected_state: &str,
    chain_id: u64,
    verifying_contract: &str,
) -> Result<DerivedIdentity> {
    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .context("Error accepting incoming loopback connection")?;

        let mut buf = [0u8; 8192];
        let n = match socket.read(&mut buf).await {
            Ok(bytes_read) if bytes_read > 0 => bytes_read,
            _ => continue,
        };

        let request = String::from_utf8_lossy(&buf[..n]);

        if request.starts_with("OPTIONS") {
            let cors_response = "HTTP/1.1 200 OK\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Access-Control-Allow-Methods: POST, GET, OPTIONS\r\n\
                Access-Control-Allow-Headers: Content-Type, Access-Control-Request-Private-Network\r\n\
                Access-Control-Allow-Private-Network: true\r\n\
                Content-Length: 0\r\n\
                Connection: close\r\n\r\n";
            let _ = socket.write_all(cors_response.as_bytes()).await;
            continue;
        }

        if request.starts_with("GET /health") {
            let health_res = "HTTP/1.1 200 OK\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Access-Control-Allow-Private-Network: true\r\n\
                Content-Type: application/json\r\n\
                Connection: close\r\n\r\n\
                {\"status\":\"ok\",\"service\":\"creg-cli-loopback\"}";
            let _ = socket.write_all(health_res.as_bytes()).await;
            continue;
        }

        if request.starts_with("POST") {
            let body = if let Some(idx) = request.find("\r\n\r\n") {
                &request[idx + 4..]
            } else {
                ""
            };

            let payload: CallbackPayload = match serde_json::from_str(body.trim()) {
                Ok(p) => p,
                Err(e) => {
                    let err_res = format!(
                        "HTTP/1.1 400 Bad Request\r\n\
                        Access-Control-Allow-Origin: *\r\n\
                        Access-Control-Allow-Private-Network: true\r\n\
                        Content-Type: application/json\r\n\
                        Connection: close\r\n\r\n\
                        {{\"error\":\"Invalid JSON: {}\"}}",
                        e
                    );
                    let _ = socket.write_all(err_res.as_bytes()).await;
                    continue;
                }
            };

            if payload.state != expected_state {
                let err_res = "HTTP/1.1 403 Forbidden\r\n\
                    Access-Control-Allow-Origin: *\r\n\
                    Access-Control-Allow-Private-Network: true\r\n\
                    Content-Type: application/json\r\n\
                    Connection: close\r\n\r\n\
                    {\"error\":\"State mismatch\"}";
                let _ = socket.write_all(err_res.as_bytes()).await;
                continue;
            }

            let sig_bytes = match parse_signature_bytes(&payload.signature) {
                Ok(sb) => sb,
                Err(e) => {
                    let err_res = format!(
                        "HTTP/1.1 400 Bad Request\r\n\
                        Access-Control-Allow-Origin: *\r\n\
                        Access-Control-Allow-Private-Network: true\r\n\
                        Content-Type: application/json\r\n\
                        Connection: close\r\n\r\n\
                        {{\"error\":\"{}\"}}",
                        e
                    );
                    let _ = socket.write_all(err_res.as_bytes()).await;
                    continue;
                }
            };

            let digest =
                match compute_derivation_digest(&payload.wallet, chain_id, verifying_contract) {
                    Ok(d) => d,
                    Err(e) => {
                        let err_res = format!(
                            "HTTP/1.1 500 Internal Error\r\n\
                        Access-Control-Allow-Origin: *\r\n\
                        Access-Control-Allow-Private-Network: true\r\n\
                        Content-Type: application/json\r\n\
                        Connection: close\r\n\r\n\
                        {{\"error\":\"{}\"}}",
                            e
                        );
                        let _ = socket.write_all(err_res.as_bytes()).await;
                        continue;
                    }
                };

            let recovered = match recover_signer(&digest, &sig_bytes) {
                Ok(r) => r,
                Err(e) => {
                    let err_res = format!(
                        "HTTP/1.1 400 Bad Request\r\n\
                        Access-Control-Allow-Origin: *\r\n\
                        Access-Control-Allow-Private-Network: true\r\n\
                        Content-Type: application/json\r\n\
                        Connection: close\r\n\r\n\
                        {{\"error\":\"Signer recovery failed: {}\"}}",
                        e
                    );
                    let _ = socket.write_all(err_res.as_bytes()).await;
                    continue;
                }
            };

            if recovered.to_ascii_lowercase() != payload.wallet.to_ascii_lowercase() {
                let err_res = "HTTP/1.1 401 Unauthorized\r\n\
                    Access-Control-Allow-Origin: *\r\n\
                    Access-Control-Allow-Private-Network: true\r\n\
                    Content-Type: application/json\r\n\
                    Connection: close\r\n\r\n\
                    {\"error\":\"Recovered address does not match wallet\"}";
                let _ = socket.write_all(err_res.as_bytes()).await;
                continue;
            }

            let (ed_key, ed_pubkey) = match derive_ed25519_key_from_signature(&sig_bytes) {
                Ok(res) => res,
                Err(e) => {
                    let err_res = format!(
                        "HTTP/1.1 500 Internal Error\r\n\
                        Access-Control-Allow-Origin: *\r\n\
                        Access-Control-Allow-Private-Network: true\r\n\
                        Content-Type: application/json\r\n\
                        Connection: close\r\n\r\n\
                        {{\"error\":\"Key derivation failed: {}\"}}",
                        e
                    );
                    let _ = socket.write_all(err_res.as_bytes()).await;
                    continue;
                }
            };

            let success_res = format!(
                "HTTP/1.1 200 OK\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Access-Control-Allow-Private-Network: true\r\n\
                Content-Type: application/json\r\n\
                Connection: close\r\n\r\n\
                {{\"status\":\"ok\",\"wallet\":\"{}\",\"pubkey\":\"{}\"}}",
                payload.wallet, ed_pubkey
            );
            let _ = socket.write_all(success_res.as_bytes()).await;

            return Ok(DerivedIdentity {
                wallet_address: payload.wallet,
                ed25519_privkey_hex: hex::encode(ed_key.as_bytes()),
                ed25519_pubkey_hex: ed_pubkey,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_deterministic_private_key_derivation() {
        let privkey = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let id1 =
            derive_from_private_key(privkey, DEFAULT_CHAIN_ID, DEFAULT_VERIFYING_CONTRACT).unwrap();
        let id2 =
            derive_from_private_key(privkey, DEFAULT_CHAIN_ID, DEFAULT_VERIFYING_CONTRACT).unwrap();

        // Anvil account #0 standard address
        assert_eq!(
            id1.wallet_address.to_lowercase(),
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
        );
        assert_eq!(id1.wallet_address, id2.wallet_address);
        assert_eq!(id1.ed25519_privkey_hex, id2.ed25519_privkey_hex);
        assert_eq!(id1.ed25519_pubkey_hex, id2.ed25519_pubkey_hex);
        assert_eq!(id1.ed25519_privkey_hex.len(), 64);
        assert_eq!(id1.ed25519_pubkey_hex.len(), 64);
    }

    #[test]
    fn test_recover_signer_matches() {
        let privkey = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";
        let identity =
            derive_from_private_key(privkey, DEFAULT_CHAIN_ID, DEFAULT_VERIFYING_CONTRACT).unwrap();

        // Anvil account #1
        assert_eq!(
            identity.wallet_address.to_lowercase(),
            "0x70997970c51812dc3a010c7d01b50e0d17dc79c8"
        );
    }

    #[test]
    fn test_derive_from_token() {
        use base64::Engine;
        let privkey = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let expected =
            derive_from_private_key(privkey, DEFAULT_CHAIN_ID, DEFAULT_VERIFYING_CONTRACT).unwrap();

        let clean = privkey.trim().strip_prefix("0x").unwrap();
        let signing_key =
            k256::ecdsa::SigningKey::from_bytes(hex::decode(clean).unwrap().as_slice().into())
                .unwrap();
        let digest = compute_derivation_digest(
            &expected.wallet_address,
            DEFAULT_CHAIN_ID,
            DEFAULT_VERIFYING_CONTRACT,
        )
        .unwrap();
        let (sig, rec_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
        let mut sig_bytes = [0u8; 65];
        sig_bytes[..64].copy_from_slice(sig.to_bytes().as_slice());
        sig_bytes[64] = rec_id.to_byte() + 27;

        let json = serde_json::json!({
            "wallet": expected.wallet_address,
            "signature": format!("0x{}", hex::encode(sig_bytes)),
            "state": "test-state"
        })
        .to_string();
        let b64_token = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());

        let derived =
            derive_from_token(&b64_token, DEFAULT_CHAIN_ID, DEFAULT_VERIFYING_CONTRACT).unwrap();
        assert_eq!(derived.wallet_address, expected.wallet_address);
        assert_eq!(derived.ed25519_privkey_hex, expected.ed25519_privkey_hex);
        assert_eq!(derived.ed25519_pubkey_hex, expected.ed25519_pubkey_hex);
    }
}
