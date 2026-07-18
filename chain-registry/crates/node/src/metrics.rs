// crates/node/src/metrics.rs
// Lightweight Prometheus-compatible metrics endpoint.
// Exposed at GET /metrics — scrape with any Prometheus-compatible system.
//
// Local chain (RocksDB):
//   creg_chain_tip_height      — height index of the tip block (genesis alone => 0)
//   creg_chain_blocks_stored   — count of blocks in local storage (genesis alone => 1)
//   creg_package_count, creg_pending_pool_size, creg_publisher_count
//
// PBFT block consensus:
//   creg_pbft_active_rounds, creg_pbft_round_phase, creg_pbft_round_age_seconds
//
// Gossip validation:
//   creg_gossip_votes_rejected_total, creg_gossip_pbft_rejected_total
//
// L1 validator set sync (Sepolia staking logs):
//   creg_validator_set_sync_*    — mirrors /v1/health.validator_set_sync

use crate::NodeState;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Build a Prometheus text-format metrics response.
pub async fn render(state: Arc<RwLock<NodeState>>) -> String {
    let s = state.read().await;
    let stats = s.chain.stats();
    let sync = &s.validator_set_sync;

    let mut out = String::with_capacity(2048);

    metric(
        &mut out,
        "creg_chain_tip_height",
        "Height index of the local chain tip (only genesis => 0; not Sepolia block number)",
        "gauge",
        stats.tip_height as f64,
    );

    metric(
        &mut out,
        "creg_chain_blocks_stored",
        "Blocks stored in the local chain DB (genesis at height 0 counts as 1)",
        "gauge",
        stats.block_count as f64,
    );

    // Deprecated alias: same series as creg_chain_tip_height (kept for existing dashboards).
    metric(
        &mut out,
        "creg_chain_height",
        "DEPRECATED: use creg_chain_tip_height (same value)",
        "gauge",
        stats.tip_height as f64,
    );

    metric(
        &mut out,
        "creg_block_count",
        "DEPRECATED: use creg_chain_blocks_stored (same value)",
        "gauge",
        stats.block_count as f64,
    );

    metric(
        &mut out,
        "creg_package_count",
        "Total verified packages on chain",
        "gauge",
        stats.package_count as f64,
    );

    metric(
        &mut out,
        "creg_pending_pool_size",
        "Packages currently awaiting consensus",
        "gauge",
        s.pending_pool.len() as f64,
    );

    metric(
        &mut out,
        "creg_publisher_count",
        "Unique publishers tracked",
        "gauge",
        s.publisher_index.publisher_count() as f64,
    );

    metric(
        &mut out,
        "creg_pending_block_txs",
        "Finalized transactions buffered awaiting block production",
        "gauge",
        crate::finalized_tx::pending_buffer_depth() as f64,
    );

    match s.chain.finalized_work_stats() {
        Ok(stats) => {
            metric(
                &mut out,
                "creg_durable_finalized_work_depth",
                "Persisted finalized work awaiting durable block inclusion",
                "gauge",
                stats.active_depth as f64,
            );
            metric(
                &mut out,
                "creg_durable_finalized_work_ready_depth",
                "Persisted finalized work currently eligible for production",
                "gauge",
                stats.ready_depth as f64,
            );
            metric(
                &mut out,
                "creg_durable_finalized_work_deferred_depth",
                "Persisted finalized work waiting for retry backoff",
                "gauge",
                stats.deferred_depth as f64,
            );
            metric(
                &mut out,
                "creg_durable_finalized_work_retrying_depth",
                "Persisted finalized work with one or more failed deliveries",
                "gauge",
                stats.retrying_depth as f64,
            );
            metric(
                &mut out,
                "creg_durable_finalized_work_retry_attempts",
                "Sum of recorded retry attempts across active durable work",
                "gauge",
                stats.retry_attempts as f64,
            );
            metric(
                &mut out,
                "creg_durable_finalized_work_oldest_age_seconds",
                "Age of the oldest persisted finalized work item",
                "gauge",
                stats.oldest_age_seconds.unwrap_or(0) as f64,
            );
        }
        Err(error) => {
            tracing::error!(
                "Failed to collect durable finalized-work metrics: {}",
                error
            );
            for name in [
                "creg_durable_finalized_work_depth",
                "creg_durable_finalized_work_ready_depth",
                "creg_durable_finalized_work_deferred_depth",
                "creg_durable_finalized_work_retrying_depth",
                "creg_durable_finalized_work_retry_attempts",
                "creg_durable_finalized_work_oldest_age_seconds",
            ] {
                metric(
                    &mut out,
                    name,
                    "Durable finalized-work metric unavailable",
                    "gauge",
                    -1.0,
                );
            }
        }
    }
    match s.chain.finalized_work_depths() {
        Ok((_, dead_letter_depth)) => metric(
            &mut out,
            "creg_durable_finalized_work_dead_letter_depth",
            "Finalized work retained for operator review",
            "gauge",
            dead_letter_depth as f64,
        ),
        Err(error) => {
            tracing::error!("Failed to collect durable dead-letter depth: {}", error);
            metric(
                &mut out,
                "creg_durable_finalized_work_dead_letter_depth",
                "Finalized work retained for operator review",
                "gauge",
                -1.0,
            );
        }
    }

    let node_id = &s.config.node_id;
    labeled_metric(
        &mut out,
        "creg_node_info",
        "Static node information",
        "gauge",
        &[
            ("node_id", node_id.as_str()),
            ("version", env!("CARGO_PKG_VERSION")),
        ],
        1.0,
    );

    metric(
        &mut out,
        "creg_validator_set_sync_enabled",
        "1 when chain-authoritative validator set sync is enabled",
        "gauge",
        if sync.enabled { 1.0 } else { 0.0 },
    );

    metric(
        &mut out,
        "creg_validator_set_sync_state_code",
        "Sync state enum: 0=disabled 1=syncing 2=reorg-replaying 3=degraded 4=synced",
        "gauge",
        validator_set_sync_state_code(&sync.state) as f64,
    );

    metric(
        &mut out,
        "creg_validator_set_sync_last_finalized_source_block",
        "Last L1 block height applied to the authoritative validator set (0 if unknown)",
        "gauge",
        sync.last_finalized_source_block.unwrap_or(0) as f64,
    );

    metric(
        &mut out,
        "creg_validator_set_sync_cursor_block",
        "L1 log cursor block height (0 if unknown)",
        "gauge",
        sync.cursor_block.unwrap_or(0) as f64,
    );

    metric(
        &mut out,
        "creg_validator_set_sync_has_error",
        "1 when validator_set_sync.last_error is set",
        "gauge",
        if sync.last_error.is_some() { 1.0 } else { 0.0 },
    );

    labeled_metric(
        &mut out,
        "creg_validator_set_sync_info",
        "Validator set sync mode and state (value is always 1 when labeled series is present)",
        "gauge",
        &[("mode", sync.mode.as_str()), ("state", sync.state.as_str())],
        1.0,
    );

    // ── Consensus / validator set health ────────────────────────────────────
    let active_validators = s
        .validator_set
        .validators
        .iter()
        .filter(|v| v.status == "online" || v.status == "self")
        .count();
    metric(
        &mut out,
        "creg_active_validators",
        "Validators currently eligible for PBFT consensus (status online/self)",
        "gauge",
        active_validators as f64,
    );
    metric(
        &mut out,
        "creg_validator_set_total",
        "Total validators in the active set (any status)",
        "gauge",
        s.validator_set.validators.len() as f64,
    );

    metric(
        &mut out,
        "creg_reorg_events_total",
        "L2 chain reorganizations recorded since start (retained window)",
        "gauge",
        s.reorgs.len() as f64,
    );

    metric(
        &mut out,
        "creg_gossip_votes_rejected_total",
        "Gossip vote messages rejected before mesh propagation",
        "counter",
        s.p2p_status.gossip_votes_rejected_total as f64,
    );
    metric(
        &mut out,
        "creg_gossip_pbft_rejected_total",
        "PBFT Prepare/Commit gossip messages rejected at validation boundary",
        "counter",
        s.p2p_status.gossip_pbft_rejected_total as f64,
    );

    // ── PBFT block consensus ─────────────────────────────────────────────────
    let pbft_rounds = s.pbft_engine.observability_snapshot();
    metric(
        &mut out,
        "creg_pbft_active_rounds",
        "Number of PBFT block rounds currently tracked",
        "gauge",
        pbft_rounds.len() as f64,
    );
    metric(
        &mut out,
        "creg_pbft_max_round_age_seconds",
        "Maximum phase age across non-terminal PBFT rounds",
        "gauge",
        s.pbft_engine.max_non_terminal_phase_age_seconds(),
    );
    for round in pbft_rounds {
        let hash_label = if round.block_hash.len() > 12 {
            &round.block_hash[..12]
        } else {
            round.block_hash.as_str()
        };
        labeled_metric(
            &mut out,
            "creg_pbft_round_phase",
            "PBFT block round phase (1=pre_prepare 2=prepare 3=commit 4=finalised 5=failed)",
            "gauge",
            &[("block_hash", hash_label), ("phase", round.phase.as_str())],
            round.phase_code as f64,
        );
        labeled_metric(
            &mut out,
            "creg_pbft_round_age_seconds",
            "Seconds since the PBFT round entered its current phase",
            "gauge",
            &[("block_hash", hash_label)],
            round.phase_age_seconds,
        );
    }

    metric(
        &mut out,
        "creg_sync_lag_blocks",
        "Blocks behind the highest known peer tip",
        "gauge",
        s.sync_lag_blocks as f64,
    );

    metric(
        &mut out,
        "creg_forced_inclusion_pending",
        "Finalized transactions awaiting block inclusion (forced-inclusion tracker)",
        "gauge",
        s.forced_inclusion_tracker.pending_count() as f64,
    );

    // ── L1 checkpoint bridge ────────────────────────────────────────────────
    let bridge = &s.bridge_status;
    metric(
        &mut out,
        "creg_bridge_anchor_count",
        "Number of L2->L1 checkpoint anchors committed (persisted journal length)",
        "gauge",
        bridge.anchor_count as f64,
    );
    metric(
        &mut out,
        "creg_bridge_last_anchor_eth_block",
        "L1 block of the most recent checkpoint anchor (0 if none yet)",
        "gauge",
        bridge.last_finalized_eth_block as f64,
    );
    metric(
        &mut out,
        "creg_bridge_finalized_l1_block",
        "Most recent L1 block tagged finalized as observed by the bridge (0 if unknown)",
        "gauge",
        bridge.finalized_l1_block.unwrap_or(0) as f64,
    );

    // ── Incentives / Rewards monitoring (§9.12, §15) ─────────────────────────
    // Emission counters from pipeline L1 hook (analysis work rewards).
    metric(
        &mut out,
        "creg_reward_emissions_succeeded",
        "Number of successful L1 distributeWorkRewards calls (Verified packages)",
        "counter",
        s.reward_emissions_succeeded as f64,
    );
    metric(
        &mut out,
        "creg_reward_emissions_failed",
        "Number of failed L1 reward emission attempts",
        "counter",
        s.reward_emissions_failed as f64,
    );
    if let Some(treasury) = &s.config.treasury_addr {
        labeled_metric(
            &mut out,
            "creg_incentives_treasury",
            "Configured treasury address for reward funding (value=1)",
            "gauge",
            &[("addr", treasury.as_str())],
            1.0,
        );
    }

    // Live treasury balance from L1 (small best-effort query for actual monitoring when eth_rpc + addrs configured)
    // Use cached values refreshed by background task (avoids per-scrape L1 cost)
    metric(
        &mut out,
        "creg_treasury_balance",
        "Current balance of the configured treasury address (in token units, from background L1 cache)",
        "gauge",
        s.treasury_balance as f64,
    );
    metric(
        &mut out,
        "creg_pending_rewards_estimate",
        "Estimated pending rewards across system (from state, via background L1 cache)",
        "gauge",
        s.pending_rewards_estimate as f64,
    );
    metric(
        &mut out,
        "creg_total_rewards_distributed",
        "Total rewards distributed via ValidatorRewards (from background L1 cache)",
        "gauge",
        s.total_rewards_distributed as f64,
    );
    metric(
        &mut out,
        "creg_rewards_pool_balance",
        "Current PinningRewards pool balance (from background L1 cache)",
        "gauge",
        s.rewards_pool_balance as f64,
    );
    metric(
        &mut out,
        "creg_last_incentives_refresh",
        "Unix timestamp of last background incentives refresh (0 if never)",
        "gauge",
        s.last_incentives_refresh.unwrap_or(0) as f64,
    );

    // ── MAL-001 sandbox posture (validators) ─────────────────────────────────
    let sandbox = validator::sandbox::engine_status().await;
    metric(
        &mut out,
        "creg_sandbox_dev_bypass",
        "1 when CREG_DEV_SANDBOX=true (behavioural analysis skipped); must be 0 on public validators",
        "gauge",
        if sandbox.dev_bypass { 1.0 } else { 0.0 },
    );
    metric(
        &mut out,
        "creg_sandbox_isolated",
        "1 when the active sandbox engine runs packages with real isolation",
        "gauge",
        if sandbox.isolated { 1.0 } else { 0.0 },
    );
    labeled_metric(
        &mut out,
        "creg_sandbox_info",
        "Sandbox engine selected for behavioural analysis (value is always 1)",
        "gauge",
        &[
            ("engine", sandbox.engine.as_str()),
            ("degraded", if sandbox.degraded { "true" } else { "false" }),
        ],
        1.0,
    );

    out
}

/// Numeric encoding for alert rules (e.g. `== 4` for synced).
pub fn validator_set_sync_state_code(state: &str) -> i32 {
    match state {
        "disabled" => 0,
        "syncing" => 1,
        "reorg-replaying" => 2,
        "degraded" => 3,
        "synced" => 4,
        _ => -1,
    }
}

fn metric(buf: &mut String, name: &str, help: &str, kind: &str, value: f64) {
    buf.push_str(&format!("# HELP {} {}\n", name, help));
    buf.push_str(&format!("# TYPE {} {}\n", name, kind));
    buf.push_str(&format!("{} {}\n\n", name, value));
}

fn labeled_metric(
    buf: &mut String,
    name: &str,
    help: &str,
    kind: &str,
    labels: &[(&str, &str)],
    value: f64,
) {
    buf.push_str(&format!("# HELP {} {}\n", name, help));
    buf.push_str(&format!("# TYPE {} {}\n", name, kind));
    let label_str = labels
        .iter()
        .map(|(k, v)| format!("{}=\"{}\"", k, escape_label(v)))
        .collect::<Vec<_>>()
        .join(",");
    buf.push_str(&format!("{}{{{}}} {}\n\n", name, label_str, value));
}

fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_set_sync_state_codes() {
        assert_eq!(validator_set_sync_state_code("synced"), 4);
        assert_eq!(validator_set_sync_state_code("syncing"), 1);
        assert_eq!(validator_set_sync_state_code("unknown"), -1);
    }

    #[test]
    fn escape_label_quotes() {
        assert_eq!(escape_label(r#"a"b"#), r#"a\"b"#);
    }
}
