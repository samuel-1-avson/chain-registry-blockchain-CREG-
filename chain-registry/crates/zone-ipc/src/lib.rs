//! Authenticated Inter-Zone IPC Transport (`ADR-018` Four-Zone Validator Split).
//!
//! Strictly dependency-minimal crate linking Zone 1 (`creg-gateway`), Zone 2
//! (`creg-consensusd`), Zone 3 (`creg-sandboxd`), and Zone 4 (`creg-agent-runner`).
//! Contains zero consensus engines, archive parsers, sandboxes, vaults, or HTTP
//! LLM clients (`INV-01`).

use anyhow::{bail, Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
#[cfg(unix)]
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const ZONE_IPC_DOMAIN: &str = "creg-zone-ipc-v1";
pub const MAX_ZONE_FRAME_BYTES: usize = 4 * 1024 * 1024;
pub const ZONE_IPC_TOKEN_ENV: &str = "CREG_ZONE_IPC_TOKEN";

pub const GATEWAY_FORBIDDEN_ENV: &[&str] = &[
    "CREG_VALIDATOR_PRIVKEY",
    "CREG_VALIDATOR_PQ_KEY",
    "CREG_L1_PRIVATE_KEY",
    "VAULT_TOKEN",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "CREG_AI_API_KEY",
];

pub const CONSENSUS_FORBIDDEN_ENV: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "CREG_AI_API_KEY",
];

pub const SANDBOX_FORBIDDEN_ENV: &[&str] = &[
    "CREG_VALIDATOR_PRIVKEY",
    "CREG_VALIDATOR_PQ_KEY",
    "CREG_L1_PRIVATE_KEY",
    "VAULT_TOKEN",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "CREG_AI_API_KEY",
];

pub const AGENT_RUNNER_FORBIDDEN_ENV: &[&str] = &[
    "CREG_VALIDATOR_PRIVKEY",
    "CREG_VALIDATOR_PQ_KEY",
    "CREG_L1_PRIVATE_KEY",
    "VAULT_TOKEN",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneId {
    Gateway,
    Consensus,
    Sandbox,
    AgentRunner,
}

impl ZoneId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Consensus => "consensus",
            Self::Sandbox => "sandbox",
            Self::AgentRunner => "agent_runner",
        }
    }
}

impl fmt::Display for ZoneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 256-bit per-boot symmetric authentication token shared across the four
/// validator zones via `CREG_ZONE_IPC_TOKEN`.
#[derive(Clone)]
pub struct ZoneIpcToken([u8; 32]);

impl fmt::Debug for ZoneIpcToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ZoneIpcToken([REDACTED])")
    }
}

impl ZoneIpcToken {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_hex(raw: &str) -> Result<Self> {
        let clean = raw.trim().strip_prefix("0x").unwrap_or(raw.trim());
        let decoded = hex::decode(clean).context("invalid ZoneIpcToken hex")?;
        if decoded.len() != 32 {
            bail!(
                "invalid ZoneIpcToken length: expected 32 bytes, got {}",
                decoded.len()
            );
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&decoded);
        Ok(Self(arr))
    }

    pub fn from_env(var_name: &str) -> Result<Self> {
        let raw = std::env::var(var_name)
            .with_context(|| format!("missing required env var '{var_name}'"))?;
        Self::from_hex(&raw)
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn compute_mac(
        &self,
        sender: ZoneId,
        target: ZoneId,
        request_id: u64,
        payload: &[u8],
    ) -> [u8; 32] {
        // Pre-hashed keyed MAC binding token, direction, sequence id, and exact payload length.
        let mut inner = Sha256::new();
        inner.update(ZONE_IPC_DOMAIN.as_bytes());
        inner.update([0x00]);
        inner.update(self.0);
        inner.update(sender.as_str().as_bytes());
        inner.update([0x00]);
        inner.update(target.as_str().as_bytes());
        inner.update([0x00]);
        inner.update(request_id.to_be_bytes());
        inner.update((payload.len() as u64).to_be_bytes());
        inner.update(payload);
        let inner_digest: [u8; 32] = inner.finalize().into();

        let mut outer = Sha256::new();
        outer.update(self.0);
        outer.update(inner_digest);
        outer.finalize().into()
    }

    pub fn verify_mac(
        &self,
        sender: ZoneId,
        target: ZoneId,
        request_id: u64,
        payload: &[u8],
        mac_hex: &str,
    ) -> bool {
        let Ok(provided) = hex::decode(mac_hex.trim()) else {
            return false;
        };
        if provided.len() != 32 {
            return false;
        }
        let expected = self.compute_mac(sender, target, request_id, payload);
        bool::from(provided.as_slice().ct_eq(&expected))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticatedZoneFrame {
    pub sender: ZoneId,
    pub target: ZoneId,
    pub request_id: u64,
    pub payload: String,
    pub mac_hex: String,
}

impl AuthenticatedZoneFrame {
    pub fn sign(
        sender: ZoneId,
        target: ZoneId,
        request_id: u64,
        payload: String,
        token: &ZoneIpcToken,
    ) -> Result<Self> {
        if payload.len() > MAX_ZONE_FRAME_BYTES {
            bail!(
                "zone IPC payload {} bytes exceeds {} byte limit",
                payload.len(),
                MAX_ZONE_FRAME_BYTES
            );
        }
        let mac = token.compute_mac(sender, target, request_id, payload.as_bytes());
        Ok(Self {
            sender,
            target,
            request_id,
            payload,
            mac_hex: hex::encode(mac),
        })
    }

    pub fn verify(&self, token: &ZoneIpcToken, expected_target: ZoneId) -> Result<()> {
        if self.target != expected_target {
            bail!(
                "zone frame target mismatch: expected {}, got {}",
                expected_target,
                self.target
            );
        }
        if self.payload.len() > MAX_ZONE_FRAME_BYTES {
            bail!(
                "zone frame payload {} bytes exceeds {} byte limit",
                self.payload.len(),
                MAX_ZONE_FRAME_BYTES
            );
        }
        if !token.verify_mac(
            self.sender,
            self.target,
            self.request_id,
            self.payload.as_bytes(),
            &self.mac_hex,
        ) {
            bail!(
                "zone IPC MAC verification failed from {} to {}",
                self.sender,
                self.target
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ZoneRequest {
    Health {},
    ForwardPublicHttp {
        method: String,
        path: String,
        #[serde(default)]
        query: Option<String>,
        body_b64: String,
    },
    ForwardAdminHttp {
        method: String,
        path: String,
        #[serde(default)]
        query: Option<String>,
        body_b64: String,
    },
    RunSandboxReplay {
        package_canonical: String,
        content_hash: String,
        tarball_b64: String,
        manifest_json: String,
        trigger_spec_json: String,
        timeout_secs: u64,
    },
    RunAgentInvestigation {
        package_canonical: String,
        content_hash: String,
        tarball_b64: String,
        manifest_json: String,
        static_findings_json: String,
        initial_kernel_graph_json: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ZoneResponse {
    Health {
        zone: ZoneId,
        status: String,
    },
    HttpReply {
        status: u16,
        content_type: String,
        body_b64: String,
    },
    SandboxReplayResult {
        sandbox_result_json: String,
    },
    AgentInvestigationResult {
        report_json: String,
    },
    Error {
        code: u16,
        message: String,
    },
}

pub fn encode_b64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

pub fn decode_b64(input: &str) -> Result<Vec<u8>> {
    fn val(b: u8) -> Option<u32> {
        match b {
            b'A'..=b'Z' => Some((b - b'A') as u32),
            b'a'..=b'z' => Some((b - b'a' + 26) as u32),
            b'0'..=b'9' => Some((b - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(4) {
        bail!("invalid base64 length {}", clean.len());
    }
    let mut out = Vec::with_capacity((clean.len() / 4) * 3);
    let (quads, _) = clean.as_chunks::<4>();
    for (idx, quad) in quads.iter().enumerate() {
        let is_last = (idx + 1) * 4 == clean.len();
        let a = val(quad[0]).context("invalid base64 char 0")?;
        let b = val(quad[1]).context("invalid base64 char 1")?;
        let pad2 = quad[2] == b'=';
        let pad3 = quad[3] == b'=';
        if (pad2 && !pad3) || ((pad2 || pad3) && !is_last) {
            bail!("invalid base64 padding");
        }
        let c = if pad2 {
            0
        } else {
            val(quad[2]).context("invalid base64 char 2")?
        };
        let d = if pad3 {
            0
        } else {
            val(quad[3]).context("invalid base64 char 3")?
        };
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        out.push(((triple >> 16) & 0xff) as u8);
        if !pad2 {
            out.push(((triple >> 8) & 0xff) as u8);
        }
        if !pad3 {
            out.push((triple & 0xff) as u8);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneEndpoint {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}

impl ZoneEndpoint {
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if let Some(path) = trimmed.strip_prefix("unix://") {
            #[cfg(unix)]
            {
                if path.is_empty() {
                    bail!("empty unix socket path");
                }
                return Ok(Self::Unix(PathBuf::from(path)));
            }
            #[cfg(not(unix))]
            {
                let _ = path;
                bail!("unix domain sockets are unavailable on this platform");
            }
        }
        let host_port = trimmed.strip_prefix("tcp://").unwrap_or(trimmed);
        let addr: SocketAddr = host_port
            .parse()
            .with_context(|| format!("invalid ZoneEndpoint address '{raw}'"))?;
        Ok(Self::Tcp(addr))
    }
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &AuthenticatedZoneFrame,
) -> Result<()> {
    let encoded = serde_json::to_vec(frame).context("encode AuthenticatedZoneFrame")?;
    if encoded.len() > MAX_ZONE_FRAME_BYTES {
        bail!(
            "encoded zone frame {} bytes exceeds {} byte limit",
            encoded.len(),
            MAX_ZONE_FRAME_BYTES
        );
    }
    let len_be = (encoded.len() as u32).to_be_bytes();
    writer.write_all(&len_be).await?;
    writer.write_all(&encoded).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<AuthenticatedZoneFrame> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let frame_len = u32::from_be_bytes(len_buf) as usize;
    if frame_len == 0 || frame_len > MAX_ZONE_FRAME_BYTES {
        bail!(
            "invalid zone frame length {} (allowed 1..={})",
            frame_len,
            MAX_ZONE_FRAME_BYTES
        );
    }
    let mut body = vec![0u8; frame_len];
    reader.read_exact(&mut body).await?;
    let frame: AuthenticatedZoneFrame =
        serde_json::from_slice(&body).context("decode AuthenticatedZoneFrame")?;
    Ok(frame)
}

enum BoundListener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener, PathBuf),
}

pub struct ZoneIpcServer {
    local_zone: ZoneId,
    token: ZoneIpcToken,
    listener: BoundListener,
}

impl ZoneIpcServer {
    pub async fn bind(
        local_zone: ZoneId,
        endpoint: ZoneEndpoint,
        token: ZoneIpcToken,
    ) -> Result<Self> {
        let listener = match endpoint {
            ZoneEndpoint::Tcp(addr) => {
                let tcp = TcpListener::bind(addr)
                    .await
                    .with_context(|| format!("bind {} TCP IPC listener on {}", local_zone, addr))?;
                BoundListener::Tcp(tcp)
            }
            #[cfg(unix)]
            ZoneEndpoint::Unix(path) => {
                if path.exists() {
                    let _ = std::fs::remove_file(&path);
                }
                let unix = tokio::net::UnixListener::bind(&path).with_context(|| {
                    format!(
                        "bind {} Unix IPC listener at {}",
                        local_zone,
                        path.display()
                    )
                })?;
                BoundListener::Unix(unix, path)
            }
        };
        Ok(Self {
            local_zone,
            token,
            listener,
        })
    }

    pub fn local_endpoint(&self) -> Result<ZoneEndpoint> {
        match &self.listener {
            BoundListener::Tcp(tcp) => Ok(ZoneEndpoint::Tcp(tcp.local_addr()?)),
            #[cfg(unix)]
            BoundListener::Unix(_, path) => Ok(ZoneEndpoint::Unix(path.clone())),
        }
    }

    pub async fn serve<F>(self, handler: F) -> Result<()>
    where
        F: Fn(ZoneId, ZoneRequest) -> Pin<Box<dyn Future<Output = ZoneResponse> + Send>>
            + Send
            + Sync
            + 'static,
    {
        let handler = Arc::new(handler);
        let token = Arc::new(self.token);
        let local_zone = self.local_zone;
        loop {
            match &self.listener {
                BoundListener::Tcp(tcp) => {
                    let (stream, _) = tcp.accept().await?;
                    let h = Arc::clone(&handler);
                    let t = Arc::clone(&token);
                    tokio::spawn(async move {
                        let _ = handle_stream(stream, local_zone, t, h).await;
                    });
                }
                #[cfg(unix)]
                BoundListener::Unix(unix, _) => {
                    let (stream, _) = unix.accept().await?;
                    let h = Arc::clone(&handler);
                    let t = Arc::clone(&token);
                    tokio::spawn(async move {
                        let _ = handle_stream(stream, local_zone, t, h).await;
                    });
                }
            }
        }
    }
}

async fn handle_stream<S, F>(
    mut stream: S,
    local_zone: ZoneId,
    token: Arc<ZoneIpcToken>,
    handler: Arc<F>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: Fn(ZoneId, ZoneRequest) -> Pin<Box<dyn Future<Output = ZoneResponse> + Send>>
        + Send
        + Sync
        + 'static,
{
    let frame = read_frame(&mut stream).await?;
    // Authenticate frame BEFORE deserializing ZoneRequest (fail-closed).
    frame.verify(&token, local_zone)?;
    let resp = match serde_json::from_str::<ZoneRequest>(&frame.payload) {
        Ok(req) => handler(frame.sender, req).await,
        Err(err) => ZoneResponse::Error {
            code: 400,
            message: format!("invalid zone request JSON: {err}"),
        },
    };
    let resp_payload = serde_json::to_string(&resp)?;
    let reply = AuthenticatedZoneFrame::sign(
        local_zone,
        frame.sender,
        frame.request_id,
        resp_payload,
        &token,
    )?;
    write_frame(&mut stream, &reply).await
}

#[derive(Clone)]
pub struct ZoneIpcClient {
    sender: ZoneId,
    target: ZoneId,
    endpoint: ZoneEndpoint,
    token: ZoneIpcToken,
}

impl ZoneIpcClient {
    pub fn new(
        sender: ZoneId,
        target: ZoneId,
        endpoint: ZoneEndpoint,
        token: ZoneIpcToken,
    ) -> Self {
        Self {
            sender,
            target,
            endpoint,
            token,
        }
    }

    pub async fn call(&self, request_id: u64, request: &ZoneRequest) -> Result<ZoneResponse> {
        let payload = serde_json::to_string(request).context("serialize ZoneRequest")?;
        let frame = AuthenticatedZoneFrame::sign(
            self.sender,
            self.target,
            request_id,
            payload,
            &self.token,
        )?;
        match &self.endpoint {
            ZoneEndpoint::Tcp(addr) => {
                let mut stream = TcpStream::connect(addr)
                    .await
                    .with_context(|| format!("connect TCP zone endpoint {addr}"))?;
                send_and_recv(&mut stream, &frame, &self.token, self.sender, request_id).await
            }
            #[cfg(unix)]
            ZoneEndpoint::Unix(path) => {
                let mut stream = tokio::net::UnixStream::connect(path)
                    .await
                    .with_context(|| format!("connect Unix zone endpoint {}", path.display()))?;
                send_and_recv(&mut stream, &frame, &self.token, self.sender, request_id).await
            }
        }
    }
}

async fn send_and_recv<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    outgoing: &AuthenticatedZoneFrame,
    token: &ZoneIpcToken,
    expected_target: ZoneId,
    expected_request_id: u64,
) -> Result<ZoneResponse> {
    write_frame(stream, outgoing).await?;
    let reply = read_frame(stream).await?;
    reply.verify(token, expected_target)?;
    if reply.sender != outgoing.target {
        bail!(
            "unexpected zone reply sender: expected {}, got {}",
            outgoing.target,
            reply.sender
        );
    }
    if reply.request_id != expected_request_id {
        bail!(
            "zone reply request_id mismatch: expected {}, got {}",
            expected_request_id,
            reply.request_id
        );
    }
    let resp: ZoneResponse = serde_json::from_str(&reply.payload).context("decode ZoneResponse")?;
    Ok(resp)
}

/// Fail-closed runtime guard proving no forbidden cross-zone secrets were
/// inherited by this process (`INV-01`).
pub fn enforce_zone_forbidden_env(zone: ZoneId, forbidden_keys: &[&str]) -> Result<()> {
    let entries: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    check_forbidden_env_map(zone, &entries, forbidden_keys)
}

/// Pure helper checking an arbitrary env slice/list against a zone's forbidden list.
pub fn check_forbidden_env_map<K, V>(
    zone: ZoneId,
    entries: &[(K, V)],
    forbidden_keys: &[&str],
) -> Result<()>
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    for (k, _v) in entries {
        if forbidden_keys.iter().any(|&f| k.as_ref() == f) {
            bail!(
                "INV-01 violation: forbidden env var '{}' present in zone '{}'",
                k.as_ref(),
                zone
            );
        }
    }
    Ok(())
}

/// Helper converting a `BTreeMap` into sortable `(String, String)` pairs.
pub fn check_forbidden_env_btree(
    zone: ZoneId,
    env: &BTreeMap<String, String>,
    forbidden_keys: &[&str],
) -> Result<()> {
    for key in env.keys() {
        if forbidden_keys.iter().any(|&f| key == f) {
            bail!(
                "INV-01 violation: forbidden env var '{}' present in zone '{}'",
                key,
                zone
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn authenticated_loopback_roundtrip_and_tamper_rejection() {
        let token = ZoneIpcToken::generate();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let server = ZoneIpcServer::bind(ZoneId::Consensus, ZoneEndpoint::Tcp(addr), token.clone())
            .await
            .unwrap();
        let endpoint = server.local_endpoint().unwrap();

        let _handle = tokio::spawn(server.serve(|_sender, req| {
            Box::pin(async move {
                match req {
                    ZoneRequest::Health {} => ZoneResponse::Health {
                        zone: ZoneId::Consensus,
                        status: "ok".into(),
                    },
                    other => ZoneResponse::Error {
                        code: 501,
                        message: format!("unhandled: {other:?}"),
                    },
                }
            })
        }));

        // Happy path authenticated roundtrip.
        let client = ZoneIpcClient::new(
            ZoneId::Gateway,
            ZoneId::Consensus,
            endpoint.clone(),
            token.clone(),
        );
        let resp = client.call(42, &ZoneRequest::Health {}).await.unwrap();
        assert_eq!(
            resp,
            ZoneResponse::Health {
                zone: ZoneId::Consensus,
                status: "ok".into(),
            }
        );

        // Wrong token fails MAC check.
        let wrong_client = ZoneIpcClient::new(
            ZoneId::Gateway,
            ZoneId::Consensus,
            endpoint.clone(),
            ZoneIpcToken::generate(),
        );
        assert!(wrong_client
            .call(43, &ZoneRequest::Health {})
            .await
            .is_err());

        // Mismatched target zone fails target check before handler runs.
        let wrong_target = ZoneIpcClient::new(
            ZoneId::Gateway,
            ZoneId::Sandbox,
            endpoint.clone(),
            token.clone(),
        );
        assert!(wrong_target
            .call(44, &ZoneRequest::Health {})
            .await
            .is_err());

        // Raw tampered MAC over socket is rejected.
        #[cfg(not(unix))]
        let ZoneEndpoint::Tcp(tcp_addr) = endpoint;
        #[cfg(unix)]
        let tcp_addr = match endpoint {
            ZoneEndpoint::Tcp(addr) => addr,
            ZoneEndpoint::Unix(_) => unreachable!(),
        };
        let mut raw_stream = TcpStream::connect(tcp_addr).await.unwrap();
        let mut tampered = AuthenticatedZoneFrame::sign(
            ZoneId::Gateway,
            ZoneId::Consensus,
            45,
            serde_json::to_string(&ZoneRequest::Health {}).unwrap(),
            &token,
        )
        .unwrap();
        tampered.mac_hex = "00".repeat(32);
        write_frame(&mut raw_stream, &tampered).await.unwrap();
        assert!(read_frame(&mut raw_stream).await.is_err());
    }

    #[tokio::test]
    async fn oversized_frame_and_unknown_fields_are_rejected() {
        let token = ZoneIpcToken::generate();
        assert!(AuthenticatedZoneFrame::sign(
            ZoneId::Gateway,
            ZoneId::Consensus,
            1,
            "A".repeat(MAX_ZONE_FRAME_BYTES + 1),
            &token,
        )
        .is_err());

        // Length prefix claiming > MAX_ZONE_FRAME_BYTES must fail read_frame immediately.
        let oversized_len = ((MAX_ZONE_FRAME_BYTES + 1) as u32).to_be_bytes();
        let mut cursor = std::io::Cursor::new(oversized_len.to_vec());
        assert!(read_frame(&mut cursor).await.is_err());

        // Strict deny_unknown_fields check on ZoneRequest and AuthenticatedZoneFrame.
        assert!(serde_json::from_str::<ZoneRequest>(r#"{"op":"health","extra":1}"#).is_err());
        assert!(serde_json::from_str::<ZoneRequest>(
            r#"{"op":"forward_public_http","method":"GET","path":"/","body_b64":"","extra":1}"#
        )
        .is_err());
        assert!(serde_json::from_str::<AuthenticatedZoneFrame>(
            r#"{"sender":"gateway","target":"consensus","request_id":1,"payload":"{}","mac_hex":"","rogue":true}"#
        )
        .is_err());
    }

    #[test]
    fn forbidden_env_guard_blocks_cross_zone_secrets() {
        let ok_env = vec![("RUST_LOG", "info"), ("CREG_ZONE_IPC_TOKEN", "abcd")];
        assert!(check_forbidden_env_map(ZoneId::Gateway, &ok_env, GATEWAY_FORBIDDEN_ENV).is_ok());

        let leaked_llm = vec![("OPENAI_API_KEY", "sk-live-secret")];
        assert!(
            check_forbidden_env_map(ZoneId::Gateway, &leaked_llm, GATEWAY_FORBIDDEN_ENV).is_err()
        );
        assert!(
            check_forbidden_env_map(ZoneId::Consensus, &leaked_llm, CONSENSUS_FORBIDDEN_ENV)
                .is_err()
        );

        let leaked_key = vec![("CREG_VALIDATOR_PRIVKEY", "deadbeef")];
        assert!(check_forbidden_env_map(
            ZoneId::AgentRunner,
            &leaked_key,
            AGENT_RUNNER_FORBIDDEN_ENV
        )
        .is_err());
        assert!(
            check_forbidden_env_map(ZoneId::Sandbox, &leaked_key, SANDBOX_FORBIDDEN_ENV).is_err()
        );
    }
}
