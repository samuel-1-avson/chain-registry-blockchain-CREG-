//! Authenticated Inter-Zone IPC Transport (`ADR-018` Four-Zone Validator Split,
//! hardened in `ADR-019` `ECA-04`).
//!
//! Strictly dependency-minimal crate linking Zone 1 (`creg-gateway`), Zone 2
//! (`creg-consensusd`), Zone 3 (`creg-sandboxd`), Zone 4 (`creg-agent-runner`),
//! and the read-only local `ZoneId::Operator` diagnostic role.
//!
//! Security properties:
//! - HKDF-SHA256 per-link keys (`creg-zone-link-v1|<caller>|<server>`) so
//!   compromising one zone never exposes another zone's inbound or outbound keys
//! - Standard RFC 2104 HMAC-SHA256 frame authentication (domain, sender, target,
//!   request_id, payload length + payload)
//! - Per-link monotonic sliding replay window (128-entry bitmask, rejecting
//!   `request_id == 0`, duplicates, and stale sequence numbers)
//! - Unix domain socket `0600` inside `0700` dir + `SO_PEERCRED` UID allowlist
//! - Strict `ZoneId::Operator` privilege cap (`Health`, `Capabilities`, and
//!   `AgentRunner`-only `ProbeProvider`) enforced inside `ZoneIpcServer`

pub mod diagnostics;

use anyhow::{bail, Context, Result};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
#[cfg(unix)]
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use zeroize::Zeroize;

pub use diagnostics::{
    OperatorIpcKeyBundleV1, OperatorZoneEndpointKeyV1, ProviderProbeEntryV1, ProviderProbeReportV1,
    ZoneCapabilitiesV1, ZoneCapabilityReportV1, OPERATOR_KEY_FILE_ENV,
};

pub const ZONE_IPC_DOMAIN: &str = "creg-zone-ipc-v2";
pub const ZONE_IPC_HKDF_SALT: &[u8] = b"creg-zone-ipc-v2";
pub const MAX_ZONE_FRAME_BYTES: usize = 4 * 1024 * 1024;
pub const ZONE_IPC_TOKEN_ENV: &str = "CREG_ZONE_IPC_TOKEN";
pub const REPLAY_WINDOW_SIZE: u64 = 128;

type HmacSha256 = Hmac<Sha256>;

/// Every signing secret the consensus node reads, under the names it reads
/// them (`chain_registry_secrets::HotKeyRole::env_var`, the shielded-bundle
/// X25519 key) plus the legacy `CREG_VALIDATOR_PRIVKEY` alias. Only the
/// consensus zone may hold these.
pub const CONSENSUS_SIGNING_SECRETS_ENV: &[&str] = &[
    "CREG_VALIDATOR_KEY",
    "CREG_VALIDATOR_PRIVKEY",
    "CREG_VALIDATOR_PRIVKEY_X25519",
    "CREG_VALIDATOR_PQ_KEY",
    "CREG_BRIDGE_KEY",
    "CREG_GOVERNANCE_KEY",
    "CREG_L1_PRIVATE_KEY",
    "VAULT_TOKEN",
];

pub const GATEWAY_FORBIDDEN_ENV: &[&str] = &[
    "CREG_VALIDATOR_KEY",
    "CREG_VALIDATOR_PRIVKEY",
    "CREG_VALIDATOR_PRIVKEY_X25519",
    "CREG_VALIDATOR_PQ_KEY",
    "CREG_BRIDGE_KEY",
    "CREG_GOVERNANCE_KEY",
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
    "CREG_VALIDATOR_KEY",
    "CREG_VALIDATOR_PRIVKEY",
    "CREG_VALIDATOR_PRIVKEY_X25519",
    "CREG_VALIDATOR_PQ_KEY",
    "CREG_BRIDGE_KEY",
    "CREG_GOVERNANCE_KEY",
    "CREG_L1_PRIVATE_KEY",
    "VAULT_TOKEN",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "CREG_AI_API_KEY",
];

pub const AGENT_RUNNER_FORBIDDEN_ENV: &[&str] = &[
    "CREG_VALIDATOR_KEY",
    "CREG_VALIDATOR_PRIVKEY",
    "CREG_VALIDATOR_PRIVKEY_X25519",
    "CREG_VALIDATOR_PQ_KEY",
    "CREG_BRIDGE_KEY",
    "CREG_GOVERNANCE_KEY",
    "CREG_L1_PRIVATE_KEY",
    "VAULT_TOKEN",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneId {
    Gateway,
    Consensus,
    Sandbox,
    AgentRunner,
    Operator,
}

impl ZoneId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Consensus => "consensus",
            Self::Sandbox => "sandbox",
            Self::AgentRunner => "agent_runner",
            Self::Operator => "operator",
        }
    }

    pub const fn env_upper(self) -> &'static str {
        match self {
            Self::Gateway => "GATEWAY",
            Self::Consensus => "CONSENSUS",
            Self::Sandbox => "SANDBOX",
            Self::AgentRunner => "AGENT_RUNNER",
            Self::Operator => "OPERATOR",
        }
    }
}

impl fmt::Display for ZoneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Canonical per-link env var name shared between the supervisor and each zone.
pub fn link_key_env_var(caller: ZoneId, target: ZoneId) -> String {
    format!(
        "CREG_ZONE_LINK_KEY_{}_TO_{}",
        caller.env_upper(),
        target.env_upper()
    )
}

/// Callers permitted to initiate IPC requests into `zone`.
pub const fn allowed_inbound_callers(zone: ZoneId) -> &'static [ZoneId] {
    match zone {
        ZoneId::Gateway => &[ZoneId::Operator],
        ZoneId::Consensus => &[ZoneId::Gateway, ZoneId::Operator],
        ZoneId::Sandbox => &[ZoneId::AgentRunner, ZoneId::Consensus, ZoneId::Operator],
        ZoneId::AgentRunner => &[ZoneId::Consensus, ZoneId::Operator],
        ZoneId::Operator => &[],
    }
}

/// Targets that `zone` is permitted to initiate IPC requests toward.
pub const fn allowed_outbound_targets(zone: ZoneId) -> &'static [ZoneId] {
    match zone {
        ZoneId::Gateway => &[ZoneId::Consensus],
        ZoneId::Consensus => &[ZoneId::AgentRunner, ZoneId::Sandbox],
        ZoneId::Sandbox => &[],
        ZoneId::AgentRunner => &[ZoneId::Sandbox],
        ZoneId::Operator => &[
            ZoneId::Gateway,
            ZoneId::Consensus,
            ZoneId::Sandbox,
            ZoneId::AgentRunner,
        ],
    }
}

/// 256-bit per-boot master token held ONLY by the supervisor (or legacy single-token
/// dev environments) to derive isolated HKDF-SHA256 per-link keys.
#[derive(Clone)]
pub struct ZoneIpcToken([u8; 32]);

impl Drop for ZoneIpcToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

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

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_hex(raw: &str) -> Result<Self> {
        let arr = parse_32_byte_hex(raw, "ZoneIpcToken")?;
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

    /// Derive the isolated 256-bit HKDF-SHA256 link key for `(caller, target)`.
    pub fn derive_link_key(&self, caller: ZoneId, target: ZoneId) -> ZoneLinkKey {
        ZoneLinkKey::derive(self, caller, target)
    }

    /// Convenience helper authenticating a directional frame under the HKDF key
    /// derived for `(sender, target)`. For server replies, pass the inbound
    /// caller's `ZoneLinkKey` via [`AuthenticatedZoneFrame::sign_with_link_key`].
    pub fn compute_mac(
        &self,
        sender: ZoneId,
        target: ZoneId,
        request_id: u64,
        payload: &[u8],
    ) -> [u8; 32] {
        self.derive_link_key(sender, target)
            .compute_mac(sender, target, request_id, payload)
    }

    pub fn verify_mac(
        &self,
        sender: ZoneId,
        target: ZoneId,
        request_id: u64,
        payload: &[u8],
        mac_hex: &str,
    ) -> bool {
        self.derive_link_key(sender, target)
            .verify_mac(sender, target, request_id, payload, mac_hex)
    }
}

/// 256-bit HKDF-SHA256 link key scoped strictly to one `(caller, target)` pair.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneLinkKey([u8; 32]);

impl Drop for ZoneLinkKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for ZoneLinkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ZoneLinkKey([REDACTED])")
    }
}

impl ZoneLinkKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_hex(raw: &str) -> Result<Self> {
        let arr = parse_32_byte_hex(raw, "ZoneLinkKey")?;
        Ok(Self(arr))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn derive(master: &ZoneIpcToken, caller: ZoneId, target: ZoneId) -> Self {
        let hkdf = Hkdf::<Sha256>::new(Some(ZONE_IPC_HKDF_SALT), master.as_bytes());
        let info = format!("creg-zone-link-v1|{}|{}", caller.as_str(), target.as_str());
        let mut okm = [0u8; 32];
        // 32-byte expand always succeeds for SHA-256 HKDF (max = 255 * 32).
        let _ = hkdf.expand(info.as_bytes(), &mut okm);
        Self(okm)
    }

    pub fn compute_mac(
        &self,
        sender: ZoneId,
        target: ZoneId,
        request_id: u64,
        payload: &[u8],
    ) -> [u8; 32] {
        // Key length is fixed 32 bytes, so HmacSha256::new_from_slice cannot fail.
        let mut mac =
            HmacSha256::new_from_slice(&self.0).expect("32-byte key valid for HMAC-SHA256");
        mac.update(ZONE_IPC_DOMAIN.as_bytes());
        mac.update(b"|");
        mac.update(sender.as_str().as_bytes());
        mac.update(b"|");
        mac.update(target.as_str().as_bytes());
        mac.update(b"|");
        mac.update(&request_id.to_be_bytes());
        mac.update(b"|");
        mac.update(&(payload.len() as u64).to_be_bytes());
        mac.update(b"|");
        mac.update(payload);
        mac.finalize().into_bytes().into()
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

fn parse_32_byte_hex(raw: &str, label: &str) -> Result<[u8; 32]> {
    let clean = raw.trim().strip_prefix("0x").unwrap_or(raw.trim());
    let decoded = zeroize::Zeroizing::new(
        hex::decode(clean).with_context(|| format!("invalid {label} hex"))?,
    );
    if decoded.len() != 32 {
        bail!(
            "invalid {label} length: expected 32 bytes, got {}",
            decoded.len()
        );
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&decoded);
    Ok(arr)
}

/// Isolated keyring holding ONLY the inbound and outbound link keys permitted
/// for a single zone.
#[derive(Debug, Clone)]
pub struct ZoneKeyring {
    local_zone: ZoneId,
    inbound: BTreeMap<ZoneId, ZoneLinkKey>,
    outbound: BTreeMap<ZoneId, ZoneLinkKey>,
}

impl ZoneKeyring {
    pub fn empty(local_zone: ZoneId) -> Self {
        Self {
            local_zone,
            inbound: BTreeMap::new(),
            outbound: BTreeMap::new(),
        }
    }

    pub fn with_inbound(mut self, caller: ZoneId, key: ZoneLinkKey) -> Self {
        self.inbound.insert(caller, key);
        self
    }

    pub fn with_outbound(mut self, target: ZoneId, key: ZoneLinkKey) -> Self {
        self.outbound.insert(target, key);
        self
    }

    /// Derive ONLY this zone's allowed inbound and outbound link keys from a master token.
    pub fn from_master_token(local_zone: ZoneId, master: &ZoneIpcToken) -> Self {
        let mut inbound = BTreeMap::new();
        for &caller in allowed_inbound_callers(local_zone) {
            inbound.insert(caller, master.derive_link_key(caller, local_zone));
        }
        let mut outbound = BTreeMap::new();
        for &target in allowed_outbound_targets(local_zone) {
            outbound.insert(target, master.derive_link_key(local_zone, target));
        }
        Self {
            local_zone,
            inbound,
            outbound,
        }
    }

    /// Load per-link keys from `CREG_ZONE_LINK_KEY_<CALLER>_TO_<TARGET>` env vars.
    /// Falls back to `CREG_ZONE_IPC_TOKEN` only if NO per-link env vars are set
    /// for `local_zone`.
    pub fn from_env_for_zone(local_zone: ZoneId) -> Result<Self> {
        let mut inbound = BTreeMap::new();
        let mut outbound = BTreeMap::new();
        let mut found_link_key = false;

        for &caller in allowed_inbound_callers(local_zone) {
            let var = link_key_env_var(caller, local_zone);
            if let Ok(raw) = std::env::var(&var) {
                if !raw.trim().is_empty() {
                    inbound.insert(
                        caller,
                        ZoneLinkKey::from_hex(&raw)
                            .with_context(|| format!("parse inbound link key {var}"))?,
                    );
                    found_link_key = true;
                }
            }
        }
        for &target in allowed_outbound_targets(local_zone) {
            let var = link_key_env_var(local_zone, target);
            if let Ok(raw) = std::env::var(&var) {
                if !raw.trim().is_empty() {
                    outbound.insert(
                        target,
                        ZoneLinkKey::from_hex(&raw)
                            .with_context(|| format!("parse outbound link key {var}"))?,
                    );
                    found_link_key = true;
                }
            }
        }

        if found_link_key {
            return Ok(Self {
                local_zone,
                inbound,
                outbound,
            });
        }

        let master = ZoneIpcToken::from_env(ZONE_IPC_TOKEN_ENV).with_context(|| {
            format!(
                "missing per-link keys for zone '{}' and fallback env '{}'",
                local_zone, ZONE_IPC_TOKEN_ENV
            )
        })?;
        Ok(Self::from_master_token(local_zone, &master))
    }

    pub fn local_zone(&self) -> ZoneId {
        self.local_zone
    }

    pub fn inbound_key(&self, caller: ZoneId) -> Option<&ZoneLinkKey> {
        self.inbound.get(&caller)
    }

    pub fn outbound_key(&self, target: ZoneId) -> Result<ZoneLinkKey> {
        self.outbound.get(&target).cloned().with_context(|| {
            format!(
                "missing outbound link key {} from {} to {}",
                link_key_env_var(self.local_zone, target),
                self.local_zone,
                target
            )
        })
    }
}

/// 128-slot sliding replay window tracking monotonic request IDs per caller link.
#[derive(Debug, Default, Clone)]
pub struct ReplayWindow {
    max_seen: u64,
    seen_mask: u128,
}

impl ReplayWindow {
    pub fn check_and_record(&mut self, request_id: u64) -> Result<()> {
        if request_id == 0 {
            bail!("zone IPC request_id must be >= 1");
        }
        if self.max_seen == 0 {
            self.max_seen = request_id;
            self.seen_mask = 1;
            return Ok(());
        }
        if request_id > self.max_seen {
            let shift = request_id - self.max_seen;
            self.seen_mask = if shift >= REPLAY_WINDOW_SIZE {
                1
            } else {
                (self.seen_mask << shift) | 1
            };
            self.max_seen = request_id;
            return Ok(());
        }
        let offset = self.max_seen - request_id;
        if offset >= REPLAY_WINDOW_SIZE {
            bail!(
                "stale zone IPC request_id {} (window max {})",
                request_id,
                self.max_seen
            );
        }
        let bit = 1u128 << offset;
        if (self.seen_mask & bit) != 0 {
            bail!("replayed zone IPC request_id {}", request_id);
        }
        self.seen_mask |= bit;
        Ok(())
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
    pub fn sign_with_link_key(
        sender: ZoneId,
        target: ZoneId,
        request_id: u64,
        payload: String,
        link_key: &ZoneLinkKey,
    ) -> Result<Self> {
        if payload.len() > MAX_ZONE_FRAME_BYTES {
            bail!(
                "zone IPC payload {} bytes exceeds {} byte limit",
                payload.len(),
                MAX_ZONE_FRAME_BYTES
            );
        }
        let mac = link_key.compute_mac(sender, target, request_id, payload.as_bytes());
        Ok(Self {
            sender,
            target,
            request_id,
            payload,
            mac_hex: hex::encode(mac),
        })
    }

    pub fn verify_with_link_key(
        &self,
        link_key: &ZoneLinkKey,
        expected_target: ZoneId,
    ) -> Result<()> {
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
        if !link_key.verify_mac(
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

    pub fn sign(
        sender: ZoneId,
        target: ZoneId,
        request_id: u64,
        payload: String,
        token: &ZoneIpcToken,
    ) -> Result<Self> {
        let link_key = token.derive_link_key(sender, target);
        Self::sign_with_link_key(sender, target, request_id, payload, &link_key)
    }

    pub fn verify(&self, token: &ZoneIpcToken, expected_target: ZoneId) -> Result<()> {
        let link_key = token.derive_link_key(self.sender, expected_target);
        self.verify_with_link_key(&link_key, expected_target)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ZoneRequest {
    Health {},
    Capabilities {},
    ProbeProvider {
        #[serde(default)]
        provider: Option<String>,
    },
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
    /// Lane C (ADR-019 `ECA-03`): generate the non-binding intelligence
    /// report for a committed `ChainRecord`. The agent runner fetches the
    /// tarball itself (bounded) and checks it against `content_hash`.
    GenerateIntelligenceReport {
        record_json: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ZoneResponse {
    Health {
        zone: ZoneId,
        status: String,
    },
    Capabilities {
        report_json: String,
    },
    ProviderProbeResult {
        report_json: String,
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
    /// `PackageIntelligenceReport` JSON answering `GenerateIntelligenceReport`.
    IntelligenceReportResult {
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
    keyring: ZoneKeyring,
    listener: BoundListener,
    #[cfg(unix)]
    allowed_uids: Vec<u32>,
}

#[cfg(unix)]
fn current_euid() -> u32 {
    // SAFETY: `geteuid()` has no invariants or failure modes.
    unsafe { libc::geteuid() }
}

impl ZoneIpcServer {
    pub async fn bind(
        local_zone: ZoneId,
        endpoint: ZoneEndpoint,
        token: ZoneIpcToken,
    ) -> Result<Self> {
        let keyring = ZoneKeyring::from_master_token(local_zone, &token);
        Self::bind_with_keyring(local_zone, endpoint, keyring).await
    }

    pub async fn bind_with_keyring(
        local_zone: ZoneId,
        endpoint: ZoneEndpoint,
        keyring: ZoneKeyring,
    ) -> Result<Self> {
        if keyring.local_zone() != local_zone {
            bail!(
                "keyring belongs to zone '{}', cannot bind server for '{}'",
                keyring.local_zone(),
                local_zone
            );
        }
        let listener = match endpoint {
            ZoneEndpoint::Tcp(addr) => {
                let tcp = TcpListener::bind(addr)
                    .await
                    .with_context(|| format!("bind {} TCP IPC listener on {}", local_zone, addr))?;
                BoundListener::Tcp(tcp)
            }
            #[cfg(unix)]
            ZoneEndpoint::Unix(path) => {
                use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    if !parent.exists() {
                        std::fs::DirBuilder::new()
                            .recursive(true)
                            .mode(0o700)
                            .create(parent)
                            .with_context(|| format!("create socket dir {}", parent.display()))?;
                    }
                }
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
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    .with_context(|| {
                        format!("set 0600 permissions on Unix socket {}", path.display())
                    })?;
                BoundListener::Unix(unix, path)
            }
        };
        Ok(Self {
            local_zone,
            keyring,
            listener,
            #[cfg(unix)]
            allowed_uids: vec![current_euid()],
        })
    }

    #[cfg(unix)]
    pub fn with_allowed_uids(mut self, uids: Vec<u32>) -> Self {
        self.allowed_uids = uids;
        self
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
        let keyring = Arc::new(self.keyring);
        let replay_windows: Arc<Mutex<BTreeMap<ZoneId, ReplayWindow>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let local_zone = self.local_zone;
        loop {
            match &self.listener {
                BoundListener::Tcp(tcp) => {
                    let (stream, _) = tcp.accept().await?;
                    let h = Arc::clone(&handler);
                    let k = Arc::clone(&keyring);
                    let rw = Arc::clone(&replay_windows);
                    tokio::spawn(async move {
                        let _ = handle_stream(stream, local_zone, k, rw, h).await;
                    });
                }
                #[cfg(unix)]
                BoundListener::Unix(unix, _) => {
                    let (stream, _) = unix.accept().await?;
                    match stream.peer_cred() {
                        Ok(cred) => {
                            let peer_uid = cred.uid();
                            if !self.allowed_uids.contains(&peer_uid) {
                                tracing::warn!(
                                    zone = %local_zone,
                                    peer_uid,
                                    "rejected Unix zone-ipc connection from unauthorized UID"
                                );
                                drop(stream);
                                continue;
                            }
                        }
                        Err(err) => {
                            tracing::warn!(
                                zone = %local_zone,
                                error = %err,
                                "failed to read peer credentials on Unix zone-ipc socket"
                            );
                            drop(stream);
                            continue;
                        }
                    }
                    let h = Arc::clone(&handler);
                    let k = Arc::clone(&keyring);
                    let rw = Arc::clone(&replay_windows);
                    tokio::spawn(async move {
                        let _ = handle_stream(stream, local_zone, k, rw, h).await;
                    });
                }
            }
        }
    }
}

fn is_operator_request_allowed(local_zone: ZoneId, req: &ZoneRequest) -> bool {
    match req {
        ZoneRequest::Health {} | ZoneRequest::Capabilities {} => true,
        ZoneRequest::ProbeProvider { .. } => local_zone == ZoneId::AgentRunner,
        ZoneRequest::ForwardPublicHttp { .. }
        | ZoneRequest::ForwardAdminHttp { .. }
        | ZoneRequest::RunSandboxReplay { .. }
        | ZoneRequest::RunAgentInvestigation { .. }
        | ZoneRequest::GenerateIntelligenceReport { .. } => false,
    }
}

async fn handle_stream<S, F>(
    mut stream: S,
    local_zone: ZoneId,
    keyring: Arc<ZoneKeyring>,
    replay_windows: Arc<Mutex<BTreeMap<ZoneId, ReplayWindow>>>,
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
    // 1. Lookup per-link key for declared sender & verify MAC BEFORE parsing or updating replay state.
    let link_key = keyring
        .inbound_key(frame.sender)
        .with_context(|| {
            format!(
                "unauthorized zone-ipc sender '{}' for target '{}'",
                frame.sender, local_zone
            )
        })?
        .clone();
    frame.verify_with_link_key(&link_key, local_zone)?;

    // 2. Enforce per-link monotonic request_id replay window.
    {
        let mut windows = replay_windows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        windows
            .entry(frame.sender)
            .or_default()
            .check_and_record(frame.request_id)?;
    }

    // 3. Parse request and enforce ZoneId::Operator read-only privilege boundary.
    let resp = match serde_json::from_str::<ZoneRequest>(&frame.payload) {
        Ok(req) => {
            if frame.sender == ZoneId::Operator && !is_operator_request_allowed(local_zone, &req) {
                ZoneResponse::Error {
                    code: 403,
                    message: format!(
                        "operator zone is restricted to Health, Capabilities, and AgentRunner ProbeProvider (denied on '{}')",
                        local_zone
                    ),
                }
            } else {
                handler(frame.sender, req).await
            }
        }
        Err(err) => ZoneResponse::Error {
            code: 400,
            message: format!("invalid zone request JSON: {err}"),
        },
    };
    let resp_payload = serde_json::to_string(&resp)?;
    let reply = AuthenticatedZoneFrame::sign_with_link_key(
        local_zone,
        frame.sender,
        frame.request_id,
        resp_payload,
        &link_key,
    )?;
    write_frame(&mut stream, &reply).await
}

#[derive(Clone)]
pub struct ZoneIpcClient {
    sender: ZoneId,
    target: ZoneId,
    endpoint: ZoneEndpoint,
    link_key: ZoneLinkKey,
}

impl ZoneIpcClient {
    pub fn new(
        sender: ZoneId,
        target: ZoneId,
        endpoint: ZoneEndpoint,
        token: ZoneIpcToken,
    ) -> Self {
        let link_key = token.derive_link_key(sender, target);
        Self::with_link_key(sender, target, endpoint, link_key)
    }

    pub fn with_link_key(
        sender: ZoneId,
        target: ZoneId,
        endpoint: ZoneEndpoint,
        link_key: ZoneLinkKey,
    ) -> Self {
        Self {
            sender,
            target,
            endpoint,
            link_key,
        }
    }

    pub async fn call(&self, request_id: u64, request: &ZoneRequest) -> Result<ZoneResponse> {
        let payload = serde_json::to_string(request).context("serialize ZoneRequest")?;
        let frame = AuthenticatedZoneFrame::sign_with_link_key(
            self.sender,
            self.target,
            request_id,
            payload,
            &self.link_key,
        )?;
        match &self.endpoint {
            ZoneEndpoint::Tcp(addr) => {
                let mut stream = TcpStream::connect(addr)
                    .await
                    .with_context(|| format!("connect TCP zone endpoint {addr}"))?;
                send_and_recv(&mut stream, &frame, &self.link_key, self.sender, request_id).await
            }
            #[cfg(unix)]
            ZoneEndpoint::Unix(path) => {
                let mut stream = tokio::net::UnixStream::connect(path)
                    .await
                    .with_context(|| format!("connect Unix zone endpoint {}", path.display()))?;
                send_and_recv(&mut stream, &frame, &self.link_key, self.sender, request_id).await
            }
        }
    }
}

async fn send_and_recv<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    outgoing: &AuthenticatedZoneFrame,
    link_key: &ZoneLinkKey,
    expected_target: ZoneId,
    expected_request_id: u64,
) -> Result<ZoneResponse> {
    write_frame(stream, outgoing).await?;
    let reply = read_frame(stream).await?;
    reply.verify_with_link_key(link_key, expected_target)?;
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

    #[test]
    fn hkdf_per_link_keys_are_distinct_and_domain_separated() {
        let master = ZoneIpcToken::generate();
        let gw_cons = master.derive_link_key(ZoneId::Gateway, ZoneId::Consensus);
        let cons_agent = master.derive_link_key(ZoneId::Consensus, ZoneId::AgentRunner);
        let op_cons = master.derive_link_key(ZoneId::Operator, ZoneId::Consensus);
        let op_gw = master.derive_link_key(ZoneId::Operator, ZoneId::Gateway);

        assert_ne!(gw_cons, cons_agent);
        assert_ne!(gw_cons, op_cons);
        assert_ne!(op_cons, op_gw);
        assert_eq!(format!("{gw_cons:?}"), "ZoneLinkKey([REDACTED])");
        assert_eq!(format!("{master:?}"), "ZoneIpcToken([REDACTED])");
    }

    #[test]
    fn replay_window_rejects_zero_duplicates_and_stale_ids() {
        let mut w = ReplayWindow::default();
        assert!(w.check_and_record(0).is_err());
        assert!(w.check_and_record(1).is_ok());
        assert!(w.check_and_record(1).is_err());

        // Out-of-order within window works once, rejects on duplicate.
        assert!(w.check_and_record(5).is_ok());
        assert!(w.check_and_record(3).is_ok());
        assert!(w.check_and_record(4).is_ok());
        assert!(w.check_and_record(3).is_err());

        // Jump forward past REPLAY_WINDOW_SIZE makes earlier sequence stale.
        assert!(w.check_and_record(200).is_ok());
        assert!(w.check_and_record(72).is_err()); // 200 - 72 == 128 >= REPLAY_WINDOW_SIZE
        assert!(w.check_and_record(73).is_ok());
    }

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

        // Replaying the same request_id must fail closed.
        assert!(client.call(42, &ZoneRequest::Health {}).await.is_err());

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

        // Compromised Gateway link key cannot forge an Operator or AgentRunner frame.
        let stolen_gw_key = token.derive_link_key(ZoneId::Gateway, ZoneId::Consensus);
        let forged_operator = ZoneIpcClient::with_link_key(
            ZoneId::Operator,
            ZoneId::Consensus,
            endpoint.clone(),
            stolen_gw_key,
        );
        assert!(forged_operator
            .call(45, &ZoneRequest::Capabilities {})
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
            46,
            serde_json::to_string(&ZoneRequest::Health {}).unwrap(),
            &token,
        )
        .unwrap();
        tampered.mac_hex = "00".repeat(32);
        write_frame(&mut raw_stream, &tampered).await.unwrap();
        assert!(read_frame(&mut raw_stream).await.is_err());
    }

    #[tokio::test]
    async fn operator_zone_is_restricted_to_readonly_diagnostics_before_handler() {
        let token = ZoneIpcToken::generate();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let cons_srv =
            ZoneIpcServer::bind(ZoneId::Consensus, ZoneEndpoint::Tcp(addr), token.clone())
                .await
                .unwrap();
        let cons_ep = cons_srv.local_endpoint().unwrap();
        tokio::spawn(cons_srv.serve(|_sender, req| {
            Box::pin(async move {
                match req {
                    ZoneRequest::Health {} => ZoneResponse::Health {
                        zone: ZoneId::Consensus,
                        status: "ok".into(),
                    },
                    ZoneRequest::Capabilities {} => ZoneResponse::Capabilities {
                        report_json: "{}".into(),
                    },
                    _ => panic!(
                        "mutating or unauthorized op from operator should never reach handler"
                    ),
                }
            })
        }));

        let op_cons =
            ZoneIpcClient::new(ZoneId::Operator, ZoneId::Consensus, cons_ep, token.clone());
        assert!(matches!(
            op_cons.call(1, &ZoneRequest::Health {}).await.unwrap(),
            ZoneResponse::Health { .. }
        ));
        assert!(matches!(
            op_cons
                .call(2, &ZoneRequest::Capabilities {})
                .await
                .unwrap(),
            ZoneResponse::Capabilities { .. }
        ));
        assert!(matches!(
            op_cons
                .call(
                    3,
                    &ZoneRequest::ForwardAdminHttp {
                        method: "POST".into(),
                        path: "/v1/operator/shutdown".into(),
                        query: None,
                        body_b64: String::new(),
                    },
                )
                .await
                .unwrap(),
            ZoneResponse::Error { code: 403, .. }
        ));
        assert!(matches!(
            op_cons
                .call(4, &ZoneRequest::ProbeProvider { provider: None })
                .await
                .unwrap(),
            ZoneResponse::Error { code: 403, .. }
        ));

        // AgentRunner permits Operator ProbeProvider, while refusing RunAgentInvestigation.
        let agent_srv =
            ZoneIpcServer::bind(ZoneId::AgentRunner, ZoneEndpoint::Tcp(addr), token.clone())
                .await
                .unwrap();
        let agent_ep = agent_srv.local_endpoint().unwrap();
        tokio::spawn(agent_srv.serve(|_sender, req| {
            Box::pin(async move {
                match req {
                    ZoneRequest::ProbeProvider { .. } => ZoneResponse::ProviderProbeResult {
                        report_json: r#"{"schema_version":1,"ok":true,"providers":[]}"#.into(),
                    },
                    _ => panic!("non-probe op from operator should never reach handler"),
                }
            })
        }));
        let op_agent = ZoneIpcClient::new(ZoneId::Operator, ZoneId::AgentRunner, agent_ep, token);
        assert!(matches!(
            op_agent
                .call(1, &ZoneRequest::ProbeProvider { provider: None })
                .await
                .unwrap(),
            ZoneResponse::ProviderProbeResult { .. }
        ));
        assert!(matches!(
            op_agent
                .call(
                    2,
                    &ZoneRequest::GenerateIntelligenceReport {
                        record_json: "{}".into(),
                    },
                )
                .await
                .unwrap(),
            ZoneResponse::Error { code: 403, .. }
        ));
    }

    #[tokio::test]
    async fn operator_key_bundle_roundtrip_and_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let bundle_path = dir.path().join("keys").join("operator-ipc-keys.json");

        let token = ZoneIpcToken::generate();
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let gw_srv = ZoneIpcServer::bind(ZoneId::Gateway, ZoneEndpoint::Tcp(addr), token.clone())
            .await
            .unwrap();
        let gw_ep = match gw_srv.local_endpoint().unwrap() {
            ZoneEndpoint::Tcp(a) => a.to_string(),
            #[cfg(unix)]
            ZoneEndpoint::Unix(p) => format!("unix://{}", p.display()),
        };
        tokio::spawn(gw_srv.serve(|_, req| {
            Box::pin(async move {
                match req {
                    ZoneRequest::Health {} => ZoneResponse::Health {
                        zone: ZoneId::Gateway,
                        status: "ok".into(),
                    },
                    _ => ZoneResponse::Error {
                        code: 403,
                        message: "forbidden".into(),
                    },
                }
            })
        }));

        let bundle = OperatorIpcKeyBundleV1 {
            schema_version: 1,
            gateway: OperatorZoneEndpointKeyV1 {
                endpoint: gw_ep,
                link_key_hex: token
                    .derive_link_key(ZoneId::Operator, ZoneId::Gateway)
                    .to_hex(),
            },
            consensus: OperatorZoneEndpointKeyV1 {
                endpoint: "127.0.0.1:19002".into(),
                link_key_hex: token
                    .derive_link_key(ZoneId::Operator, ZoneId::Consensus)
                    .to_hex(),
            },
            sandbox: OperatorZoneEndpointKeyV1 {
                endpoint: "127.0.0.1:19003".into(),
                link_key_hex: token
                    .derive_link_key(ZoneId::Operator, ZoneId::Sandbox)
                    .to_hex(),
            },
            agent_runner: OperatorZoneEndpointKeyV1 {
                endpoint: "127.0.0.1:19004".into(),
                link_key_hex: token
                    .derive_link_key(ZoneId::Operator, ZoneId::AgentRunner)
                    .to_hex(),
            },
        };
        bundle.write_owner_only(&bundle_path).unwrap();
        let loaded = OperatorIpcKeyBundleV1::read_owner_only(&bundle_path).unwrap();
        let client = loaded.client_for(ZoneId::Gateway).unwrap();
        let resp = client.call(1, &ZoneRequest::Health {}).await.unwrap();
        assert_eq!(
            resp,
            ZoneResponse::Health {
                zone: ZoneId::Gateway,
                status: "ok".into(),
            }
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_socket_enforces_0600_mode_and_peercred_uid_allowlist() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("ipc").join("consensus.sock");
        let token = ZoneIpcToken::generate();
        let server = ZoneIpcServer::bind(
            ZoneId::Consensus,
            ZoneEndpoint::Unix(sock_path.clone()),
            token.clone(),
        )
        .await
        .unwrap()
        .with_allowed_uids(vec![u32::MAX]);

        let sock_mode = std::fs::metadata(&sock_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(sock_mode, 0o600);
        let dir_mode = std::fs::metadata(sock_path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);

        tokio::spawn(
            server.serve(|_, _| {
                Box::pin(async { panic!("rejected UID should never reach handler") })
            }),
        );
        let client = ZoneIpcClient::new(
            ZoneId::Gateway,
            ZoneId::Consensus,
            ZoneEndpoint::Unix(sock_path),
            token,
        );
        assert!(client.call(1, &ZoneRequest::Health {}).await.is_err());
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

        // Every signing secret the node reads, under the name it reads it, is
        // refused outside the consensus zone (the key the node actually loads
        // is `CREG_VALIDATOR_KEY`).
        for &secret in CONSENSUS_SIGNING_SECRETS_ENV {
            let leaked = vec![(secret, "deadbeef")];
            for (zone, list) in [
                (ZoneId::Gateway, GATEWAY_FORBIDDEN_ENV),
                (ZoneId::Sandbox, SANDBOX_FORBIDDEN_ENV),
                (ZoneId::AgentRunner, AGENT_RUNNER_FORBIDDEN_ENV),
            ] {
                assert!(
                    check_forbidden_env_map(zone, &leaked, list).is_err(),
                    "{secret} must be refused in {zone}"
                );
            }
            assert!(
                check_forbidden_env_map(ZoneId::Consensus, &leaked, CONSENSUS_FORBIDDEN_ENV)
                    .is_ok()
            );
        }
    }
}
