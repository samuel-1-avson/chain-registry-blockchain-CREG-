// crates/node/src/state.rs
// Shared NodeState and associated types.  Factored into its own module so
// that the library target (lib.rs) and binary target (main.rs) can both
// include it, enabling integration tests to construct and inspect the state.

use std::{collections::HashMap, sync::Arc};

use serde::Serialize;
use tokio::sync::RwLock;

use common::ValidatorIdentity;

use crate::{
    chain_store::ChainStore,
    config::NodeConfig,
    events::EventBus,
    finalized_tx::FinalizedTxSender,
    p2p::P2PHandle,
    pending_pool::PendingPool,
    publisher_index::PublisherIndex,
};

// ─── Validator registration ───────────────────────────────────────────────────

#[derive(Serialize, Clone, Debug, Default)]
pub struct ValidatorRegistrationStatus {
    pub alias: String,
    pub identity: ValidatorIdentity,
    pub registered_with_node: bool,
    pub applied_on_chain: bool,
    pub governance_approved: bool,
    pub admitted_to_consensus: bool,
    pub active: bool,
    pub staking_state: String,
    pub status: String,
    pub stake: u64,
    pub reputation: u32,
    pub last_error: Option<String>,
    pub last_synced_at: Option<String>,
}

pub fn normalized_validator_key(evm_address: &str) -> String {
    evm_address.trim().to_ascii_lowercase()
}

pub fn validator_registration_status_text(registration: &ValidatorRegistrationStatus) -> String {
    if registration.active {
        "active".to_string()
    } else if registration.admitted_to_consensus {
        "admitted-to-consensus".to_string()
    } else if registration.governance_approved {
        "governance-approved".to_string()
    } else if registration.applied_on_chain {
        "applied-on-chain".to_string()
    } else if registration.registered_with_node {
        "identity-registered".to_string()
    } else {
        "unregistered".to_string()
    }
}

// ─── Live status snapshots ─────────────────────────────────────────────────────

#[derive(Serialize, Clone, Default)]
pub struct P2PStatus {
    pub peers: Vec<String>,
    pub protocols: Vec<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct BridgeStatus {
    pub last_finalized_eth_block: u64,
    pub registry_address: String,
    pub bridge_sync_status: String,
    pub current_state_root: String,
}

// ─── NodeState ─────────────────────────────────────────────────────────────────

/// Shared mutable state passed to every subsystem via `Arc<RwLock<_>>`.
pub struct NodeState {
    pub chain: ChainStore,
    pub pending_pool: PendingPool,
    pub publisher_index: PublisherIndex,
    pub validator_set: common::ValidatorSet,
    /// Accumulated validator votes per block hash / package canonical.
    pub votes: HashMap<String, Vec<common::ValidatorSignature>>,
    pub config: NodeConfig,
    pub event_bus: EventBus,
    pub p2p: P2PHandle,
    pub zk_validator: Arc<zk_validator::ZkValidator>,
    pub tx_sender: FinalizedTxSender,
    // Live metrics for the Explorer UI
    pub p2p_status: P2PStatus,
    pub bridge_status: BridgeStatus,
    /// Cached VRF proofs from other validators: validator_id → (output, proof).
    pub vrf_proofs: HashMap<String, (String, String)>,
    /// Decryption shares received from peers: canonical → Vec<KeyShare>.
    pub decryption_shares: HashMap<String, Vec<threshold_encryption::KeyShare>>,
    /// Validator registrations keyed by canonical EVM address.
    pub validator_registrations: HashMap<String, ValidatorRegistrationStatus>,
    /// View-change certificates accumulated from peers.
    /// Outer key: block_hash. Middle key: proposed new_view number.
    /// Inner set: validator IDs that have sent a certificate for this (block, view).
    ///
    /// A view-change is applied once ⌊n/3⌋+1 certificates are received,
    /// preventing a single Byzantine node from forcing a view-change.
    pub view_change_certs: HashMap<String, HashMap<u32, std::collections::HashSet<String>>>,
}

pub type SharedState = Arc<RwLock<NodeState>>;
