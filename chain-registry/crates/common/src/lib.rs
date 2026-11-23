// crates/common/src/lib.rs
// Core types shared across the entire chain-registry workspace.

pub mod block;
pub mod chain_spec;
pub mod consensus_encoding;
pub mod crypto;
pub mod display_text;
pub mod error;
pub mod feature_flags;
pub mod hot_key;
pub mod intelligence;
pub mod keyfile;
#[cfg(feature = "l1-rpc")]
pub mod l1_rpc;
pub mod ownership;
pub mod package;
pub mod shielded_wire;
pub mod test_support;
pub mod transparency_log;
pub mod validator_snapshot;
pub mod verdict;
pub mod vote_v4;

/// gRPC and Protobuf definitions (Generated Choice)
#[allow(clippy::large_enum_variant, clippy::result_large_err)]
pub mod proto {
    tonic::include_proto!("node.v1");
}

pub use block::*;
pub use chain_spec::*;
pub use consensus_encoding::*;
pub use display_text::{sanitize_for_terminal, DisplayText, DEFAULT_DISPLAY_MAX_CHARS};
pub use error::*;
pub use feature_flags::*;
pub use hot_key::*;
pub use intelligence::*;
pub use ownership::*;
pub use package::*;
pub use shielded_wire::*;
pub use transparency_log::*;
pub use validator_snapshot::*;
pub use verdict::*;
pub use vote_v4::*;

// ── Cryptographic helpers ─────────────────────────────────────────────────────

use sha2::{Digest, Sha256};

/// SHA-256 of any byte slice, returned as a 32-byte array.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// SHA-256 of any byte slice, returned as a lowercase hex string.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(sha256(data))
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidatorIdentity {
    #[serde(default, alias = "address")]
    pub evm_address: String,
    #[serde(default, alias = "id")]
    pub node_id: String,
    #[serde(default, alias = "pubkey")]
    pub ed25519_pubkey: String,
}

impl ValidatorIdentity {
    pub fn normalized(&self) -> Self {
        Self {
            evm_address: self.evm_address.trim().to_ascii_lowercase(),
            node_id: self.node_id.trim().to_string(),
            ed25519_pubkey: self.ed25519_pubkey.trim().to_ascii_lowercase(),
        }
    }

    pub fn is_complete(&self) -> bool {
        !self.evm_address.trim().is_empty()
            && !self.node_id.trim().is_empty()
            && !self.ed25519_pubkey.trim().is_empty()
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Validator {
    pub id: String,
    pub alias: String,
    /// Hex-encoded Ed25519 public key used to verify validator votes.
    #[serde(default)]
    pub pubkey: String,
    /// Canonical 0x-prefixed EVM address used as the on-chain membership key.
    #[serde(default)]
    pub eth_address: String,
    pub stake: u64,
    pub reputation: u32,
    pub status: String, // "online", "self", "active", "offline"

    // --- Performance / resources proxy (for committee selection) ---
    /// 0-1000 composite score reflecting observed ability to successfully and
    /// timely complete expensive analysis work (sandbox + evidence production).
    /// Higher == better demonstrated resources + reliability.
    /// Updated from pipeline contributions and reward emissions.
    /// Used to bias mandatory committee selection toward capable nodes.
    #[serde(default = "default_perf_score")]
    pub performance_score: u32,
    /// Number of packages this validator has contributed to (for averaging).
    #[serde(default)]
    pub contribution_count: u32,
    /// Optional FIPS 204 ML-DSA-44 public key (2,624 lowercase hex chars = 1,312 bytes)
    /// bound alongside the Ed25519 `pubkey` (`ADR-017`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pq_pubkey: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct ValidatorSet {
    pub validators: Vec<Validator>,
}

fn default_perf_score() -> u32 {
    500
}

impl Default for Validator {
    fn default() -> Self {
        Self {
            id: String::new(),
            alias: String::new(),
            pubkey: String::new(),
            eth_address: String::new(),
            stake: 0,
            reputation: 0,
            status: String::new(),
            performance_score: default_perf_score(),
            contribution_count: 0,
            pq_pubkey: None,
        }
    }
}

impl Validator {
    /// Decode this validator's verifying key for either classical Ed25519
    /// (`require_hybrid == false`) or ADR-017 Hybrid (`require_hybrid == true`).
    pub fn verifying_key(&self, require_hybrid: bool) -> anyhow::Result<crate::crypto::PublicKey> {
        let ed_clean = self
            .pubkey
            .trim()
            .strip_prefix("0x")
            .unwrap_or(self.pubkey.trim());
        let ed_bytes = hex::decode(ed_clean).map_err(|e| {
            anyhow::anyhow!("invalid Ed25519 public key hex for '{}': {e}", self.id)
        })?;
        if !require_hybrid {
            return crate::crypto::PublicKey::from_bytes(
                crate::crypto::SignatureScheme::Ed25519,
                &ed_bytes,
            )
            .map_err(|e| anyhow::anyhow!("invalid Ed25519 public key for '{}': {e}", self.id));
        }

        let pq_hex = self
            .pq_pubkey
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("validator '{}' missing required pq_pubkey", self.id))?;
        let pq_clean = pq_hex.trim().strip_prefix("0x").unwrap_or(pq_hex.trim());
        if pq_clean.len() != crate::vote_v4::MLDSA44_PUBLIC_KEY_HEX_LEN
            || !pq_clean
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            anyhow::bail!(
                "validator '{}' pq_pubkey must be {} lowercase hex chars",
                self.id,
                crate::vote_v4::MLDSA44_PUBLIC_KEY_HEX_LEN
            );
        }
        let pq_bytes = hex::decode(pq_clean).map_err(|e| {
            anyhow::anyhow!("invalid ML-DSA-44 public key hex for '{}': {e}", self.id)
        })?;
        crate::crypto::PublicKey::hybrid_from_parts(&ed_bytes, &pq_bytes)
            .map_err(|e| anyhow::anyhow!("invalid hybrid public key for '{}': {e}", self.id))
    }
}

impl ValidatorSet {
    pub fn new(validators: Vec<Validator>) -> Self {
        Self { validators }
    }
}

/// Whether a validator status string counts as PBFT-active.
///
/// Signed chain-spec copies use `"active"`. Runtime L1 sync remaps admitted
/// members to `"online"` / `"self"`. `"offline"`, `"pending"`, and `"jailed"`
/// stay inactive.
pub fn validator_status_is_active(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "online" | "self" | "active"
    )
}

/// Canonical boolean spelling check shared across `common`, `node`, and analyzers.
pub fn is_truthy_flag(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes"
    )
}

/// Parses an environment variable as a boolean flag using [`is_truthy_flag`].
pub fn parse_env_bool(name: &str) -> bool {
    std::env::var(name)
        .map(|value| is_truthy_flag(&value))
        .unwrap_or(false)
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SignedRevocationRequest {
    pub chain_id: String,
    pub genesis_hash: String,
    pub package_canonical: String,
    pub reason: String,
    pub revoker_pubkey: String,
    pub signature: String,
    #[serde(default)]
    pub signature_version: u8,
}

impl SignedRevocationRequest {
    pub fn signing_message(&self) -> String {
        if self.signature_version == 1 {
            format!(
                "creg-revoke-v1:{}:{}:{}:revoke:{}",
                self.chain_id, self.genesis_hash, self.package_canonical, self.reason
            )
        } else {
            format!("{}:revoke:{}", self.package_canonical, self.reason)
        }
    }

    pub fn verify_signature(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.signature_version <= 1,
            "unsupported revocation signature version"
        );
        anyhow::ensure!(
            !self.reason.is_empty()
                && self.reason.len() <= 4096
                && !self.package_canonical.is_empty()
                && self.package_canonical.len() <= 256
                && self.revoker_pubkey.len() == 64
                && self.signature.len() == 128,
            "invalid revocation proof bounds"
        );
        let key = hex::decode(&self.revoker_pubkey)?;
        let key =
            crate::crypto::PublicKey::from_bytes(crate::crypto::SignatureScheme::Ed25519, &key)?;
        let signature = hex::decode(&self.signature)?;
        key.verify(
            self.signing_message().as_bytes(),
            &crate::crypto::Signature::from_bytes(
                crate::crypto::SignatureScheme::Ed25519,
                &signature,
            )?,
            crate::crypto::VerifyMode::Strict,
        )?;
        Ok(())
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum GossipMessage {
    PublishRequest(PublishRequest),
    /// Original owner/validator proof is independently checked by every peer.
    /// Delivery IDs change on retries; the signed request and transaction do not.
    RevocationRequest {
        request: SignedRevocationRequest,
        delivery_id: String,
    },
    VrfProof {
        validator_id: String,
        pubkey: String,
        epoch_seed: String,
        output: String,
        proof: String,
    },
    /// A validator's signed view-change vote. Peers apply the view only after
    /// verifying a PBFT quorum for the same height, parent, and target view.
    ViewChange {
        vote: ViewChangeVote,
    },
    PbftPrePrepare {
        block: Block,
    },
    /// Quorum-authorized replacement primary retransmits the exact block that
    /// reached PREPARE quorum. The envelope is signed and excluded from the
    /// immutable block hash.
    PbftRepropose {
        block: Block,
        target_view: u32,
        leader_id: String,
        view_change_certificate: Vec<ViewChangeVote>,
        leader_signature: String,
    },
    PbftPrepare {
        block_hash: String,
        validator_id: String,
        signature: String,
    },
    PbftCommit {
        block_hash: String,
        validator_id: String,
        signature: String,
    },
    /// A validator identity registration, gossiped so one POST to any node
    /// propagates the (evm_address, node_id, ed25519_pubkey) binding to the
    /// whole fleet. Carries the original ownership proofs so every receiver
    /// independently re-verifies before applying — no node has to trust the
    /// sender. Eliminates the manual "POST to every node" drift class.
    ValidatorRegistration {
        evm_address: String,
        node_id: String,
        ed25519_pubkey: String,
        alias: Option<String>,
        nonce: String,
        evm_signature: String,
        ed25519_signature: String,
    },
    /// A quorum-signed validator-set transition. Receivers must verify the
    /// certificate against their own persisted prior set before accepting it.
    ValidatorSetTransition {
        snapshot: AuthenticatedValidatorSetSnapshot,
    },
}

#[cfg(test)]
mod validator_status_tests {
    use super::validator_status_is_active;

    #[test]
    fn spec_and_runtime_active_statuses_count() {
        assert!(validator_status_is_active("active"));
        assert!(validator_status_is_active("ACTIVE"));
        assert!(validator_status_is_active(" online "));
        assert!(validator_status_is_active("self"));
    }

    #[test]
    fn inactive_statuses_do_not_count() {
        assert!(!validator_status_is_active("offline"));
        assert!(!validator_status_is_active("pending"));
        assert!(!validator_status_is_active("jailed"));
        assert!(!validator_status_is_active(""));
    }

    #[test]
    fn truthy_flag_parses_all_canonical_spellings() {
        use super::is_truthy_flag;

        for yes in ["1", "true", "TRUE", " True ", "yes", "YES"] {
            assert!(is_truthy_flag(yes), "expected truthy for {yes:?}");
        }
        for no in ["", "0", "false", "FALSE", "no", "on", "maybe"] {
            assert!(!is_truthy_flag(no), "expected falsy for {no:?}");
        }
    }
}
