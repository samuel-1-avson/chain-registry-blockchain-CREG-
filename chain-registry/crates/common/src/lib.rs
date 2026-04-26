// crates/common/src/lib.rs
// Core types shared across the entire chain-registry workspace.

pub mod block;
pub mod chain_spec;
pub mod error;
pub mod package;
pub mod verdict;

/// gRPC and Protobuf definitions (Generated Choice)
pub mod proto {
    tonic::include_proto!("node.v1");
}

pub use block::*;
pub use chain_spec::*;
pub use error::*;
pub use package::*;
pub use verdict::*;

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
    pub stake: u64,
    pub reputation: u32,
    pub status: String, // "online", "self", "offline"
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct ValidatorSet {
    pub validators: Vec<Validator>,
}

impl ValidatorSet {
    pub fn new(validators: Vec<Validator>) -> Self {
        Self { validators }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum GossipMessage {
    PublishRequest(PublishRequest),
    VrfProof {
        validator_id: String,
        pubkey: String,
        epoch_seed: String,
        output: String,
        proof: String,
    },
    /// A validator's view-change certificate, broadcast when a PBFT round
    /// times out.  Peers accumulate these and only execute the view-change
    /// locally once they have seen ⌊n/3⌋+1 certificates for the same
    /// (block_hash, new_view) pair, preventing a single Byzantine node from
    /// forcing a view-change.
    ViewChange {
        /// Hex-encoded hash of the block this round is for.
        block_hash: String,
        /// The new view number being proposed.
        new_view: u32,
        /// ID of the validator requesting the view-change.
        validator_id: String,
        /// Ed25519 signature over `"{block_hash}:view_change:{new_view}"`.
        signature: String,
    },
}
