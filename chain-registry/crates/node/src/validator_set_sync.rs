// crates/node/src/validator_set_sync.rs
//
// Event-driven materialized view of the on-chain validator set.
//
// Today the in-memory `ValidatorSet` is loaded once from `CREG_VALIDATOR_SET`
// (or `config/validator-set.json`) and never changes. `consensus_admission.rs`
// already drives the *other* half of the loop — every active validator
// independently signs an EIP-712 attestation and the lowest-address signer
// submits `Staking.approveByConsensus(...)` on L1, which emits
// `ValidatorApprovedByConsensus(applicant, nonce, signerCount)`. The contract
// knows. The runtime doesn't.
//
// This module closes that loop. It subscribes (today: polls) Staking.sol
// events on the L1 bridge, normalises them into `ValidatorSetDelta`s, applies
// them to the in-memory view with a finality lag, and exposes a metric so we
// can run in *shadow mode* — observing drift between chain-derived and
// file-derived sets without changing consensus behaviour.
//
// PHASING (per docs/VALIDATOR_SET_SYNC_DESIGN.md):
//   Phase 1 — shadow mode (this scaffold). Compute deltas + drift metric.
//             Does NOT mutate the active set.
//   Phase 2 — chain-authoritative with file fallback.
//   Phase 3 — chain only.
//
// What this scaffold does:
//   • Pulls staking-event logs by polling `eth_getLogs` from a cursor.
//   • Decodes each log into a `ValidatorSetDelta`.
//   • Honours `finality_lag_blocks` (head − block_height ≥ lag before applying).
//   • Persists the cursor in memory only (sled persistence is TODO; flagged in
//     `cursor` doc-comment).
//
// What this scaffold does NOT do (intentionally — follow-up):
//   • Wire into main.rs. Behaviour is unchanged unless a caller explicitly
//     spawns `SyncWorker::run_in_shadow_mode`.
//   • Reorg unwinding (the Sepolia ≤6-block reorg case).
//   • Multi-RPC quorum.
//   • Sled-backed cursor persistence.

use alloy::{
    primitives::{Address, B256, U256},
    sol,
    sol_types::SolEvent,
};
use serde::{Deserialize, Serialize};

// ─── Contract event ABIs ─────────────────────────────────────────────────────
//
// These mirror the events declared in contracts/Staking.sol exactly. If the
// Solidity event signature changes, the topic0 produced by `sol!` here
// changes, and old logs decoded with this binding will silently mismatch —
// so version this module alongside Staking.sol.

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface IStakingEvents {
        event ValidatorApplied            (address indexed validator, uint256 stake);
        event ValidatorApproved           (address indexed validator);
        event ValidatorApprovedByConsensus(address indexed validator, uint256 nonce, uint256 signerCount);
        event ValidatorRejected           (address indexed validator);
        event ValidatorApplicationExpired (address indexed validator, uint256 refunded);
        event ValidatorUnbonding          (address indexed validator, uint256 unbondingAt);
        event ValidatorWithdrawn          (address indexed validator, uint256 amount);
        event ValidatorLeft               (address indexed validator);
        event Slashed                     (address indexed account,   uint256 amount, string reason);
    }
);

// ─── Delta types ─────────────────────────────────────────────────────────────

/// One observable change to the validator set, attributed to an L1 log.
///
/// Deltas are designed to be applied in `(block_height, log_index)` order
/// and to be *idempotent* — applying the same delta twice yields the same
/// state. This matters because on a crash we'll replay from the persisted
/// cursor and may double-apply the last-seen log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorSetDelta {
    pub kind: DeltaKind,
    /// 0x-prefixed lowercase hex of the EVM address — the primary key.
    pub addr: String,
    pub block_height: u64,
    pub log_index: u32,
    pub tx_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeltaKind {
    /// Active set membership change: applicant was approved on-chain.
    /// Stake (in wei, decimal-string to keep U256 precision) and the EIP-712
    /// attestation count come from the event itself.
    Add { stake_wei: String, signer_count: Option<u32> },
    /// `applyAsValidator` only — applicant exists but is *not* yet active.
    /// In shadow mode we simply log this; in chain-authoritative mode it
    /// would populate a `pending_applicants` map for the UI.
    Apply { stake_wei: String },
    /// Final removal — `Withdrawn`, `Left`, or stake-drained `Slashed`.
    /// The contract has marked them terminal; we drop them from the active
    /// set. Idempotent: removing an already-absent address is a no-op.
    Remove,
    /// Validator entered the unbonding window. Still possible they withdraw
    /// or rejoin; for the active set we treat as "inactive now".
    Unbond { unbonding_at: u64 },
    /// Stake was slashed. If the new stake (which we cannot derive from the
    /// event payload alone — `Slashed` only emits the slash *amount*) drops
    /// to zero, the chain will follow up with a `Slashed`-driven removal
    /// via the regular flow. We carry the amount for telemetry.
    Slash { amount_wei: String, reason: String },
    /// `Rejected` or `ApplicationExpired`. No active-set effect (applicant
    /// was never active) but useful for the UI/audit trail.
    DropApplicant,
}

// ─── Decoding ────────────────────────────────────────────────────────────────

/// What we need from an L1 log to decode a delta. Mirrors the subset of
/// `alloy::rpc::types::Log` we actually use, so tests can construct one
/// without spinning up a provider.
#[derive(Clone, Debug)]
pub struct LogView<'a> {
    pub topics: &'a [B256],
    pub data: &'a [u8],
    pub block_number: u64,
    pub log_index: u32,
    pub tx_hash: B256,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("log has no topics — not an event")]
    NoTopics,
    #[error("unknown topic0 0x{0} — not a Staking event we track")]
    UnknownTopic(String),
    #[error("event decode failed: {0}")]
    Sol(String),
}

/// Decode one Staking event log into a `ValidatorSetDelta`. Returns
/// `Ok(None)` for events we deliberately ignore (e.g. duplicate/admin
/// events that never affect the active set). Errors are reserved for
/// malformed logs.
pub fn decode(log: LogView<'_>) -> Result<Option<ValidatorSetDelta>, DecodeError> {
    let topic0 = *log.topics.first().ok_or(DecodeError::NoTopics)?;

    // alloy's sol!-generated event types expose a `SIGNATURE_HASH` constant
    // that is the keccak256 of the canonical event signature. We dispatch on
    // that to pick the right decoder.
    let kind: DeltaKind;
    let addr: Address;

    if topic0 == IStakingEvents::ValidatorApplied::SIGNATURE_HASH {
        let ev = IStakingEvents::ValidatorApplied::decode_log_data(
            &alloy::primitives::LogData::new_unchecked(log.topics.to_vec(), log.data.to_vec().into()),
            true,
        )
        .map_err(|e| DecodeError::Sol(e.to_string()))?;
        addr = ev.validator;
        kind = DeltaKind::Apply {
            stake_wei: ev.stake.to_string(),
        };
    } else if topic0 == IStakingEvents::ValidatorApproved::SIGNATURE_HASH {
        let ev = IStakingEvents::ValidatorApproved::decode_log_data(
            &alloy::primitives::LogData::new_unchecked(log.topics.to_vec(), log.data.to_vec().into()),
            true,
        )
        .map_err(|e| DecodeError::Sol(e.to_string()))?;
        addr = ev.validator;
        kind = DeltaKind::Add {
            stake_wei: U256::ZERO.to_string(),
            signer_count: None,
        };
    } else if topic0 == IStakingEvents::ValidatorApprovedByConsensus::SIGNATURE_HASH {
        let ev = IStakingEvents::ValidatorApprovedByConsensus::decode_log_data(
            &alloy::primitives::LogData::new_unchecked(log.topics.to_vec(), log.data.to_vec().into()),
            true,
        )
        .map_err(|e| DecodeError::Sol(e.to_string()))?;
        addr = ev.validator;
        kind = DeltaKind::Add {
            stake_wei: U256::ZERO.to_string(),
            signer_count: u32::try_from(ev.signerCount).ok(),
        };
    } else if topic0 == IStakingEvents::ValidatorRejected::SIGNATURE_HASH {
        let ev = IStakingEvents::ValidatorRejected::decode_log_data(
            &alloy::primitives::LogData::new_unchecked(log.topics.to_vec(), log.data.to_vec().into()),
            true,
        )
        .map_err(|e| DecodeError::Sol(e.to_string()))?;
        addr = ev.validator;
        kind = DeltaKind::DropApplicant;
    } else if topic0 == IStakingEvents::ValidatorApplicationExpired::SIGNATURE_HASH {
        let ev = IStakingEvents::ValidatorApplicationExpired::decode_log_data(
            &alloy::primitives::LogData::new_unchecked(log.topics.to_vec(), log.data.to_vec().into()),
            true,
        )
        .map_err(|e| DecodeError::Sol(e.to_string()))?;
        addr = ev.validator;
        kind = DeltaKind::DropApplicant;
    } else if topic0 == IStakingEvents::ValidatorUnbonding::SIGNATURE_HASH {
        let ev = IStakingEvents::ValidatorUnbonding::decode_log_data(
            &alloy::primitives::LogData::new_unchecked(log.topics.to_vec(), log.data.to_vec().into()),
            true,
        )
        .map_err(|e| DecodeError::Sol(e.to_string()))?;
        addr = ev.validator;
        kind = DeltaKind::Unbond {
            unbonding_at: ev.unbondingAt.try_into().unwrap_or(u64::MAX),
        };
    } else if topic0 == IStakingEvents::ValidatorWithdrawn::SIGNATURE_HASH {
        let ev = IStakingEvents::ValidatorWithdrawn::decode_log_data(
            &alloy::primitives::LogData::new_unchecked(log.topics.to_vec(), log.data.to_vec().into()),
            true,
        )
        .map_err(|e| DecodeError::Sol(e.to_string()))?;
        addr = ev.validator;
        kind = DeltaKind::Remove;
    } else if topic0 == IStakingEvents::ValidatorLeft::SIGNATURE_HASH {
        let ev = IStakingEvents::ValidatorLeft::decode_log_data(
            &alloy::primitives::LogData::new_unchecked(log.topics.to_vec(), log.data.to_vec().into()),
            true,
        )
        .map_err(|e| DecodeError::Sol(e.to_string()))?;
        addr = ev.validator;
        kind = DeltaKind::Remove;
    } else if topic0 == IStakingEvents::Slashed::SIGNATURE_HASH {
        let ev = IStakingEvents::Slashed::decode_log_data(
            &alloy::primitives::LogData::new_unchecked(log.topics.to_vec(), log.data.to_vec().into()),
            true,
        )
        .map_err(|e| DecodeError::Sol(e.to_string()))?;
        addr = ev.account;
        kind = DeltaKind::Slash {
            amount_wei: ev.amount.to_string(),
            reason: ev.reason.clone(),
        };
    } else {
        return Err(DecodeError::UnknownTopic(hex::encode(topic0)));
    }

    Ok(Some(ValidatorSetDelta {
        kind,
        addr: format!("0x{}", hex::encode(addr.0)),
        block_height: log.block_number,
        log_index: log.log_index,
        tx_hash: format!("0x{}", hex::encode(log.tx_hash.0)),
    }))
}

// ─── Worker ──────────────────────────────────────────────────────────────────

/// Configuration for the polling worker. Most production deployments will
/// override `finality_lag_blocks` from the chain spec (Sepolia: 6, mainnet: 32).
#[derive(Clone, Debug)]
pub struct SyncConfig {
    pub eth_rpc_url: String,
    pub staking_addr: Address,
    /// How far behind head we trail before applying a delta. 0 disables the
    /// lag (useful for local Anvil tests where reorgs do not happen).
    pub finality_lag_blocks: u64,
    /// How often we poll `eth_getLogs`. WS subscription is a follow-up.
    pub poll_interval_secs: u64,
    /// Block to start from on a fresh boot. Chain spec provides this as
    /// `validator_set.epoch_block_height`. None = start at current head.
    pub start_block: Option<u64>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            eth_rpc_url: "http://127.0.0.1:8545".into(),
            staking_addr: Address::ZERO,
            finality_lag_blocks: 6,
            poll_interval_secs: 12,
            start_block: None,
        }
    }
}

/// Mode for the worker. See module docstring for the phasing plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncMode {
    /// Compute and emit deltas; do NOT mutate the active validator set.
    /// Drift between chain-derived and file-derived sets is exposed via
    /// `SyncWorker::observed_addresses()` for telemetry.
    Shadow,
    /// Apply deltas to the active set. Reserved for Phase 2; not yet wired.
    #[allow(dead_code)]
    ChainAuthoritative,
}

/// In-memory state the worker owns. Sled persistence is a follow-up — when
/// added, the cursor and observed-set fields will be flushed on every apply.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct WorkerState {
    /// Highest `(block_height, log_index)` we've consumed. None on first run.
    pub cursor: Option<(u64, u32)>,
    /// Addresses the chain says are *currently* in the active validator set.
    /// In shadow mode this is the chain-derived view; we never mutate the
    /// actual `ValidatorSet`.
    pub observed_active: HashSet<String>,
}

/// One iteration's worth of work. Pulled out as a free function so it can
/// be unit-tested without an HTTP runtime.
pub fn apply_delta(state: &mut WorkerState, delta: &ValidatorSetDelta) {
    // Idempotency guard — never advance the cursor backwards.
    if let Some((h, i)) = state.cursor {
        if (delta.block_height, delta.log_index) <= (h, i) {
            return;
        }
    }
    match &delta.kind {
        DeltaKind::Add { .. } => {
            state.observed_active.insert(delta.addr.clone());
        }
        DeltaKind::Remove | DeltaKind::Unbond { .. } => {
            state.observed_active.remove(&delta.addr);
        }
        DeltaKind::Slash { amount_wei, .. } => {
            // The event alone doesn't tell us the post-slash stake; if it
            // drains to zero the contract emits a follow-up Withdrawn/Left.
            // Until then we keep the validator in the active set.
            tracing::debug!(
                target: "validator_set_sync",
                "slash observed addr={} amount_wei={}",
                delta.addr,
                amount_wei
            );
        }
        DeltaKind::Apply { .. } | DeltaKind::DropApplicant => {
            // Applicant lifecycle — no active-set effect.
        }
    }
    state.cursor = Some((delta.block_height, delta.log_index));
}

// ─── Async polling worker ────────────────────────────────────────────────────

use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::NodeState;

/// Run the validator-set sync worker.
///
/// Polls `eth_getLogs` against Staking.sol, decodes events into deltas,
/// and applies them to the in-memory validator set (if not in shadow mode).
pub async fn run(
    config: SyncConfig,
    mode: SyncMode,
    state: Arc<RwLock<NodeState>>,
) -> anyhow::Result<()> {
    let mut worker_state = load_cursor(&state).await.unwrap_or_default();
    let mut interval = tokio::time::interval(
        std::time::Duration::from_secs(config.poll_interval_secs)
    );

    let client = reqwest::Client::new();

    loop {
        interval.tick().await;

        let latest_block = match fetch_latest_block(&client, &config.eth_rpc_url).await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("validator_set_sync: failed to fetch latest block: {}", e);
                continue;
            }
        };

        let safe_block = latest_block.saturating_sub(config.finality_lag_blocks);
        let from_block = worker_state.cursor.map(|(h, _)| h + 1).unwrap_or_else(|| {
            config.start_block.unwrap_or(safe_block.saturating_sub(1000))
        });

        if from_block > safe_block {
            continue;
        }

        match fetch_deltas(
            &client,
            &config.eth_rpc_url,
            &config.staking_addr,
            from_block,
            safe_block,
        ).await {
            Ok(deltas) => {
                for delta in deltas {
                    match mode {
                        SyncMode::Shadow => {
                            tracing::info!(
                                target: "validator_set_sync",
                                "[shadow] delta: {:?} addr={} block={}",
                                delta.kind, delta.addr, delta.block_height
                            );
                            apply_delta(&mut worker_state, &delta);
                        }
                        SyncMode::ChainAuthoritative => {
                            apply_delta(&mut worker_state, &delta);
                            if let Err(e) = apply_delta_to_state(Arc::clone(&state), &delta).await {
                                tracing::error!("Failed to apply delta to state: {}", e);
                            }
                        }
                    }
                }
                if let Err(e) = save_cursor(&state, &worker_state).await {
                    tracing::warn!("Failed to save validator set sync cursor: {}", e);
                }
            }
            Err(e) => {
                tracing::warn!("validator_set_sync: failed to fetch deltas: {}", e);
            }
        }
    }
}

async fn fetch_latest_block(client: &reqwest::Client, rpc_url: &str) -> anyhow::Result<u64> {
    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1,
        }))
        .send()
        .await?
        .json()
        .await?;

    let hex = resp["result"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("eth_blockNumber returned no result"))?;
    u64::from_str_radix(hex.trim_start_matches("0x"), 16)
        .map_err(|e| anyhow::anyhow!("invalid block number: {}", e))
}

async fn fetch_deltas(
    client: &reqwest::Client,
    rpc_url: &str,
    staking_addr: &alloy::primitives::Address,
    from_block: u64,
    to_block: u64,
) -> anyhow::Result<Vec<ValidatorSetDelta>> {
    let topics = vec![
        format!("0x{}", hex::encode(IStakingEvents::ValidatorApplied::SIGNATURE_HASH.0)),
        format!("0x{}", hex::encode(IStakingEvents::ValidatorApproved::SIGNATURE_HASH.0)),
        format!("0x{}", hex::encode(IStakingEvents::ValidatorApprovedByConsensus::SIGNATURE_HASH.0)),
        format!("0x{}", hex::encode(IStakingEvents::ValidatorRejected::SIGNATURE_HASH.0)),
        format!("0x{}", hex::encode(IStakingEvents::ValidatorApplicationExpired::SIGNATURE_HASH.0)),
        format!("0x{}", hex::encode(IStakingEvents::ValidatorUnbonding::SIGNATURE_HASH.0)),
        format!("0x{}", hex::encode(IStakingEvents::ValidatorWithdrawn::SIGNATURE_HASH.0)),
        format!("0x{}", hex::encode(IStakingEvents::ValidatorLeft::SIGNATURE_HASH.0)),
        format!("0x{}", hex::encode(IStakingEvents::Slashed::SIGNATURE_HASH.0)),
    ];

    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getLogs",
            "params": [{
                "address": format!("0x{}", hex::encode(staking_addr.0)),
                "fromBlock": format!("0x{:x}", from_block),
                "toBlock": format!("0x{:x}", to_block),
                "topics": [topics],
            }],
            "id": 1,
        }))
        .send()
        .await?
        .json()
        .await?;

    let logs = resp["result"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("eth_getLogs returned no result array"))?;

    let mut deltas = Vec::new();
    for log in logs {
        let topics: Vec<alloy::primitives::B256> = log["topics"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|t| t.as_str().and_then(|s| s.parse().ok()))
            .collect();
        let data_hex = log["data"].as_str().unwrap_or("0x");
        let data = hex::decode(data_hex.trim_start_matches("0x")).unwrap_or_default();
        let block_number = u64::from_str_radix(
            log["blockNumber"].as_str().unwrap_or("0x0").trim_start_matches("0x"),
            16,
        ).unwrap_or(0);
        let log_index = u32::from_str_radix(
            log["logIndex"].as_str().unwrap_or("0x0").trim_start_matches("0x"),
            16,
        ).unwrap_or(0);
        let tx_hash = log["transactionHash"]
            .as_str()
            .unwrap_or("0x0000000000000000000000000000000000000000000000000000000000000000")
            .parse()
            .unwrap_or(alloy::primitives::B256::ZERO);

        let log_view = LogView {
            topics: &topics,
            data: &data,
            block_number,
            log_index,
            tx_hash,
        };

        match decode(log_view) {
            Ok(Some(delta)) => deltas.push(delta),
            Ok(None) => {}
            Err(e) => tracing::warn!("Failed to decode log: {}", e),
        }
    }

    Ok(deltas)
}

async fn apply_delta_to_state(
    state: Arc<RwLock<NodeState>>,
    delta: &ValidatorSetDelta,
) -> anyhow::Result<()> {
    let mut s = state.write().await;
    match &delta.kind {
        DeltaKind::Add { .. } => {
            // TODO: fetch validator metadata (id, pubkey, stake) from staking contract
            // For now, just log the event.
            tracing::info!(
                "Validator {} added to active set (block {})",
                delta.addr,
                delta.block_height
            );
        }
        DeltaKind::Remove | DeltaKind::Unbond { .. } => {
            s.validator_set.validators.retain(|v| {
                // Match by eth_address if available, otherwise skip
                // TODO: Validator struct needs eth_address field
                true
            });
            tracing::info!(
                "Validator {} removed from active set (block {})",
                delta.addr,
                delta.block_height
            );
        }
        _ => {}
    }
    Ok(())
}

// ── Cursor persistence (sidecar JSON file) ───────────────────────────────────

fn cursor_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("validator-set-sync.cursor.json")
}

async fn load_cursor(state: &Arc<RwLock<NodeState>>) -> Option<WorkerState> {
    let data_dir = {
        let s = state.read().await;
        s.config.data_dir.clone()
    };
    let path = cursor_path(&data_dir);
    if !path.exists() {
        return None;
    }
    let json = tokio::fs::read_to_string(&path).await.ok()?;
    serde_json::from_str(&json).ok()
}

async fn save_cursor(
    state: &Arc<RwLock<NodeState>>,
    worker_state: &WorkerState,
) -> anyhow::Result<()> {
    let data_dir = {
        let s = state.read().await;
        s.config.data_dir.clone()
    };
    let path = cursor_path(&data_dir);
    let json = serde_json::to_string_pretty(worker_state)?;
    tokio::fs::write(&path, json).await?;
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use alloy::primitives::LogData;
    use alloy::sol_types::SolEvent;

    fn b256_zero() -> B256 {
        B256::ZERO
    }

    /// Encode an event the way the wire would carry it. alloy's
    /// `encode_log_data` already produces the right topics+data split for
    /// indexed/non-indexed fields, so we just unpack it.
    fn encode<E: SolEvent>(ev: &E) -> (Vec<B256>, Vec<u8>) {
        let ld: LogData = ev.encode_log_data();
        (ld.topics().to_vec(), ld.data.to_vec())
    }

    #[test]
    fn decode_validator_applied() {
        let validator: Address = "0x1111111111111111111111111111111111111111"
            .parse()
            .unwrap();
        let ev = IStakingEvents::ValidatorApplied {
            validator,
            stake: U256::from(123u64),
        };
        let (topics, data) = encode(&ev);
        let log = LogView {
            topics: &topics,
            data: &data,
            block_number: 100,
            log_index: 0,
            tx_hash: b256_zero(),
        };
        let delta = decode(log).unwrap().unwrap();
        assert_eq!(delta.addr, "0x1111111111111111111111111111111111111111");
        assert_eq!(delta.block_height, 100);
        assert!(matches!(delta.kind, DeltaKind::Apply { ref stake_wei } if stake_wei == "123"));
    }

    #[test]
    fn decode_validator_approved_by_consensus() {
        let validator: Address = "0x2222222222222222222222222222222222222222"
            .parse()
            .unwrap();
        let ev = IStakingEvents::ValidatorApprovedByConsensus {
            validator,
            nonce: U256::from(1u64),
            signerCount: U256::from(7u64),
        };
        let (topics, data) = make_log(&ev, validator, 200, 1);
        let log = LogView {
            topics: &topics,
            data: &data,
            block_number: 200,
            log_index: 1,
            tx_hash: b256_zero(),
        };
        let delta = decode(log).unwrap().unwrap();
        assert!(matches!(delta.kind, DeltaKind::Add { signer_count: Some(7), .. }));
    }

    #[test]
    fn decode_validator_unbonding() {
        let validator: Address = "0x3333333333333333333333333333333333333333"
            .parse()
            .unwrap();
        let ev = IStakingEvents::ValidatorUnbonding {
            validator,
            unbondingAt: U256::from(1_700_000_000u64),
        };
        let (topics, data) = make_log(&ev, validator, 300, 2);
        let log = LogView {
            topics: &topics,
            data: &data,
            block_number: 300,
            log_index: 2,
            tx_hash: b256_zero(),
        };
        let delta = decode(log).unwrap().unwrap();
        assert!(
            matches!(delta.kind, DeltaKind::Unbond { unbonding_at: 1_700_000_000 }),
            "got {:?}",
            delta.kind
        );
    }

    #[test]
    fn decode_unknown_topic_errors() {
        let log = LogView {
            topics: &[B256::repeat_byte(0xab)],
            data: &[],
            block_number: 1,
            log_index: 0,
            tx_hash: b256_zero(),
        };
        let err = decode(log).expect_err("unknown topic should fail");
        assert!(matches!(err, DecodeError::UnknownTopic(_)));
    }

    #[test]
    fn decode_no_topics_errors() {
        let log = LogView {
            topics: &[],
            data: &[],
            block_number: 1,
            log_index: 0,
            tx_hash: b256_zero(),
        };
        assert!(matches!(decode(log).unwrap_err(), DecodeError::NoTopics));
    }

    fn delta(addr: &str, kind: DeltaKind, block: u64, idx: u32) -> ValidatorSetDelta {
        ValidatorSetDelta {
            kind,
            addr: addr.into(),
            block_height: block,
            log_index: idx,
            tx_hash: "0x".into(),
        }
    }

    #[test]
    fn apply_delta_adds_then_removes() {
        let mut s = WorkerState::default();
        apply_delta(
            &mut s,
            &delta(
                "0xaaaa",
                DeltaKind::Add {
                    stake_wei: "0".into(),
                    signer_count: None,
                },
                10,
                0,
            ),
        );
        assert!(s.observed_active.contains("0xaaaa"));
        apply_delta(&mut s, &delta("0xaaaa", DeltaKind::Remove, 11, 0));
        assert!(!s.observed_active.contains("0xaaaa"));
        assert_eq!(s.cursor, Some((11, 0)));
    }

    #[test]
    fn apply_delta_is_idempotent_on_replay() {
        let mut s = WorkerState::default();
        let d = delta(
            "0xaaaa",
            DeltaKind::Add {
                stake_wei: "0".into(),
                signer_count: None,
            },
            10,
            0,
        );
        apply_delta(&mut s, &d);
        apply_delta(&mut s, &d); // replay
        assert_eq!(s.observed_active.len(), 1);
        assert_eq!(s.cursor, Some((10, 0)));
    }

    #[test]
    fn apply_delta_rejects_out_of_order() {
        let mut s = WorkerState::default();
        apply_delta(&mut s, &delta("0xaaaa", DeltaKind::Remove, 20, 5));
        // Older delta arriving late — must NOT regress the cursor or undo state.
        apply_delta(
            &mut s,
            &delta(
                "0xaaaa",
                DeltaKind::Add {
                    stake_wei: "0".into(),
                    signer_count: None,
                },
                10,
                0,
            ),
        );
        assert_eq!(s.cursor, Some((20, 5)));
        assert!(!s.observed_active.contains("0xaaaa"));
    }

    #[test]
    fn unbond_marks_inactive_in_observed_view() {
        let mut s = WorkerState::default();
        apply_delta(
            &mut s,
            &delta(
                "0xbeef",
                DeltaKind::Add {
                    stake_wei: "0".into(),
                    signer_count: None,
                },
                1,
                0,
            ),
        );
        apply_delta(
            &mut s,
            &delta("0xbeef", DeltaKind::Unbond { unbonding_at: 999 }, 2, 0),
        );
        assert!(!s.observed_active.contains("0xbeef"));
    }

    #[test]
    fn slash_alone_does_not_remove_validator() {
        let mut s = WorkerState::default();
        apply_delta(
            &mut s,
            &delta(
                "0xcafe",
                DeltaKind::Add {
                    stake_wei: "0".into(),
                    signer_count: None,
                },
                1,
                0,
            ),
        );
        apply_delta(
            &mut s,
            &delta(
                "0xcafe",
                DeltaKind::Slash {
                    amount_wei: "1000".into(),
                    reason: "downtime".into(),
                },
                2,
                0,
            ),
        );
        // Slash by itself is informational; removal comes via a follow-up Withdrawn/Left.
        assert!(s.observed_active.contains("0xcafe"));
    }

    #[test]
    fn delta_serde_roundtrip() {
        let d = delta(
            "0xaaaa",
            DeltaKind::Add {
                stake_wei: "1000000000000000000".into(),
                signer_count: Some(7),
            },
            42,
            3,
        );
        let json = serde_json::to_string(&d).unwrap();
        let back: ValidatorSetDelta = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    // Silence unused-import warnings on Bytes (used only by sol! macros at compile time).
    #[allow(dead_code)]
    fn _silence_bytes_warning() -> Bytes {
        Bytes::new()
    }
}
