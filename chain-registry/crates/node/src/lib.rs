// crates/node/src/lib.rs
// Library target for the chain-registry-node crate (crate name: "node").
// Exposes internal subsystems so that integration tests in tests/ can spin
// up a real in-process node and drive the full publish → verify lifecycle.

// ── Public modules (used directly by integration tests) ──────────────────────
pub mod api;
pub mod block_producer;
pub mod chain_store;
pub mod config;
pub mod events;
pub mod finalized_tx;
pub mod gossip;
pub mod p2p;
pub mod pending_pool;
pub mod publisher_index;
pub mod rate_limit;
pub mod state;
pub mod validator_pipeline;

// ── Private modules required by the public ones above ────────────────────────
mod bridge;
mod db_sync_proxy;
mod explorer;
mod grpc;
mod metrics;
mod p2p_rate_limit;
mod pidlock;
mod proof;
mod sync;

// ── Re-export state types at the crate root ───────────────────────────────────
// api.rs, block_producer.rs, etc. reference these as `crate::NodeState`,
// `crate::SharedState`, etc. — they must live at the lib root.
pub use state::{
    BridgeStatus, NodeState, P2PStatus, SharedState, ValidatorRegistrationStatus,
    normalized_validator_key, validator_registration_status_text,
};
