// crates/consensus/src/pbft.rs
// Three-phase PBFT: PRE-PREPARE, PREPARE, COMMIT.
// Safety guarantee: the network is correct as long as fewer than ⌊n/3⌋
// validators are faulty or Byzantine.

use crate::ValidatorSet;
use anyhow::{bail, Result};
use common::{Block, BlockSignature};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Default maximum time a round can stay in any single phase before it is
/// considered timed-out. Overridden via `CREG_PBFT_TIMEOUT` env var.
const DEFAULT_ROUND_PHASE_TIMEOUT_SECS: u64 = 30;

/// Default maximum number of view-change retries before a round is abandoned.
/// Overridden via `CREG_PBFT_MAX_VIEW_CHANGES` env var.
const DEFAULT_MAX_VIEW_CHANGES: u32 = 3;

/// Default age after which a terminal (Finalised / Failed) round is eligible
/// for garbage collection. Overridden via `CREG_PBFT_STALE_TTL` env var.
const DEFAULT_STALE_ROUND_TTL_SECS: u64 = 120;

/// Whether three-validator clusters may use a simple-majority quorum instead
/// of strict PBFT quorum. Disabled by default and intended for explicit local
/// bootstrap opt-in only.
const DEFAULT_ALLOW_SMALL_CLUSTER_QUORUM: bool = false;

/// Configuration for PBFT consensus parameters.
/// All values have sensible defaults and can be overridden via environment
/// variables at startup.
#[derive(Debug, Clone)]
pub struct PbftConfig {
    pub round_phase_timeout: Duration,
    pub max_view_changes: u32,
    pub stale_round_ttl: Duration,
    pub allow_small_cluster_quorum: bool,
}

impl Default for PbftConfig {
    fn default() -> Self {
        Self {
            round_phase_timeout: Duration::from_secs(
                std::env::var("CREG_PBFT_TIMEOUT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_ROUND_PHASE_TIMEOUT_SECS),
            ),
            max_view_changes: std::env::var("CREG_PBFT_MAX_VIEW_CHANGES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_VIEW_CHANGES),
            stale_round_ttl: Duration::from_secs(
                std::env::var("CREG_PBFT_STALE_TTL")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_STALE_ROUND_TTL_SECS),
            ),
            allow_small_cluster_quorum: env_flag(
                "CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM",
                DEFAULT_ALLOW_SMALL_CLUSTER_QUORUM,
            ),
        }
    }
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        })
        .unwrap_or(default)
}

/// Current phase of a PBFT round for a given block proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PbftPhase {
    PrePrepare,
    Prepare,
    Commit,
    Finalised,
    Failed,
}

/// A view-change signal returned by `timeout_rounds()` so the block producer
/// or P2P layer can broadcast the certificate to peers.
#[derive(Debug, Clone)]
pub struct ViewChangeSignal {
    /// Hash of the block whose round timed out.
    pub block_hash: String,
    /// The new view number after the increment.
    pub new_view: u32,
}

/// State of a single PBFT consensus round.
pub struct PbftRound {
    pub block: Block,
    pub phase: PbftPhase,
    /// validator_id → their PREPARE message signature
    pub prepare_sigs: HashMap<String, BlockSignature>,
    /// validator_id → their COMMIT message signature
    pub commit_sigs: HashMap<String, BlockSignature>,
    pub validator_set: ValidatorSet,
    /// Wall-clock time the current phase was entered.
    pub phase_entered_at: Instant,
    /// Monotonically increasing view number (incremented on view-change).
    pub view_number: u32,
    /// How many view-changes have occurred for this round.
    pub view_change_count: u32,
    /// When the round was first created (for stale-round GC).
    pub created_at: Instant,
    /// Runtime configuration for timeouts and view-change limits.
    pub config: PbftConfig,
    /// view_number → set of validator IDs that sent a ViewChange certificate
    /// for that view.  A view-change is only executed once ⌊n/3⌋+1 certificates
    /// have been received, preventing a single Byzantine node from forcing it.
    pub view_change_votes: HashMap<u32, HashSet<String>>,
}

impl PbftRound {
    pub fn new(block: Block, validator_set: ValidatorSet) -> Self {
        Self::with_config(block, validator_set, PbftConfig::default())
    }

    pub fn with_config(block: Block, validator_set: ValidatorSet, config: PbftConfig) -> Self {
        let now = Instant::now();
        Self {
            block,
            phase: PbftPhase::PrePrepare,
            prepare_sigs: HashMap::new(),
            commit_sigs: HashMap::new(),
            validator_set,
            phase_entered_at: now,
            view_number: 0,
            view_change_count: 0,
            created_at: now,
            config,
            view_change_votes: HashMap::new(),
        }
    }

    /// Quorum threshold.
    ///
    /// PBFT requires unanimity in a three-validator set to preserve the usual
    /// Byzantine tolerance guarantees. Local bootstrap can opt into a 2-of-3
    /// majority via `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=true`.
    pub fn quorum(&self) -> usize {
        quorum_threshold(
            self.validator_set.len(),
            self.config.allow_small_cluster_quorum,
        )
    }

    // ── Phase 1: PRE-PREPARE ─────────────────────────────────────────────────
    /// The primary (proposer) broadcasts the block. Other validators verify
    /// the block hash and move to PREPARE.
    pub fn pre_prepare(&mut self, proposer_id: &str) -> Result<String> {
        if self.phase != PbftPhase::PrePrepare {
            bail!("Not in PRE-PREPARE phase");
        }
        if !self.validator_set.is_member(proposer_id) {
            bail!("Proposer {} is not in the validator set", proposer_id);
        }
        // If the block includes a VRF proof, verify it. Proposer selection itself
        // remains deterministic until VRF proof propagation is synchronized.
        if let (Some(ref proof), Some(ref output)) =
            (&self.block.header.vrf_proof, &self.block.header.vrf_output)
        {
            let validator = self
                .validator_set
                .validators
                .get(proposer_id)
                .ok_or_else(|| {
                    anyhow::anyhow!("Proposer {} not found in validator set", proposer_id)
                })?;
            let epoch_seed = &self.block.header.prev_hash;
            crate::vrf::verify(epoch_seed.as_bytes(), &validator.pubkey, output, proof).map_err(
                |e| {
                    anyhow::anyhow!(
                        "VRF verification failed for proposer {}: {}",
                        proposer_id,
                        e
                    )
                },
            )?;
        }

        let active: Vec<crate::vrf::VrfValidator> = self
            .validator_set
            .validators
            .values()
            .filter(|v| v.is_active)
            .map(|v| crate::vrf::VrfValidator {
                id: v.id.clone(),
                pubkey: v.pubkey.clone(),
                vrf_output: None,
                vrf_proof: None,
            })
            .collect();
        let selected = crate::vrf::select_proposer_deterministic(&active, &self.block.header.prev_hash)
            .ok_or_else(|| anyhow::anyhow!("No active validators to select proposer"))?;
        if &selected != proposer_id {
            bail!(
                "Proposer {} is not the selected proposer for this epoch (expected {})",
                proposer_id,
                selected
            );
        }

        // Broadcast the block hash — validators use this as the message digest.
        let block_hash = self.block.hash();
        tracing::info!(
            "[PBFT] PRE-PREPARE: block {} from {}",
            &block_hash[..12],
            proposer_id
        );
        self.phase = PbftPhase::Prepare;
        self.phase_entered_at = Instant::now();
        Ok(block_hash)
    }

    // ── Phase 2: PREPARE ─────────────────────────────────────────────────────
    /// A validator casts its PREPARE vote (approve or reject) over the block hash.
    pub fn receive_prepare(&mut self, validator_id: &str, sig: BlockSignature) -> Result<bool> {
        if self.phase != PbftPhase::Prepare {
            bail!("Not in PREPARE phase");
        }
        if !self.validator_set.is_member(validator_id) {
            bail!("Validator {} is not in the active set", validator_id);
        }
        self.prepare_sigs.insert(validator_id.to_string(), sig);
        tracing::debug!(
            "[PBFT] PREPARE: {}/{} votes",
            self.prepare_sigs.len(),
            self.quorum()
        );

        if self.prepare_sigs.len() >= self.quorum() {
            self.phase = PbftPhase::Commit;
            self.phase_entered_at = Instant::now();
            tracing::info!("[PBFT] PREPARE quorum reached — moving to COMMIT");
            return Ok(true); // caller should now broadcast COMMIT
        }
        Ok(false)
    }

    // ── Phase 3: COMMIT ──────────────────────────────────────────────────────
    /// A validator sends its COMMIT signature. Once quorum is reached the
    /// block is finalised and can be written to the chain.
    pub fn receive_commit(&mut self, validator_id: &str, sig: BlockSignature) -> Result<bool> {
        if self.phase != PbftPhase::Commit {
            bail!("Not in COMMIT phase");
        }
        self.commit_sigs.insert(validator_id.to_string(), sig);
        tracing::debug!(
            "[PBFT] COMMIT: {}/{} votes",
            self.commit_sigs.len(),
            self.quorum()
        );

        if self.commit_sigs.len() >= self.quorum() {
            let approvals = self.commit_sigs.len();

            if approvals >= self.quorum() {
                self.phase = PbftPhase::Finalised;
                tracing::info!(
                    "[PBFT] FINALISED block {} ({} approvals / {} commits)",
                    &self.block.hash()[..12],
                    approvals,
                    self.commit_sigs.len()
                );
                return Ok(true);
            } else {
                self.phase = PbftPhase::Failed;
                tracing::warn!(
                    "[PBFT] FAILED — insufficient approvals ({}/{})",
                    approvals,
                    self.quorum()
                );
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// Returns the finalised signatures to embed in the ChainRecord.
    pub fn finalised_signatures(&self) -> Vec<BlockSignature> {
        self.commit_sigs.values().cloned().collect()
    }

    /// Returns `true` when the current (non-terminal) phase has exceeded
    /// the configured round phase timeout.
    pub fn is_phase_timed_out(&self) -> bool {
        match self.phase {
            PbftPhase::Finalised | PbftPhase::Failed => false,
            _ => self.phase_entered_at.elapsed() > self.config.round_phase_timeout,
        }
    }

    /// Record a view-change certificate received from a peer validator.
    ///
    /// The view-change is **only executed locally** once ⌊n/3⌋+1 certificates
    /// have been received for the same `(block_hash, new_view)` pair.  This
    /// prevents a single Byzantine node from forcing a view-change unilaterally.
    ///
    /// Returns `Ok(true)` when the threshold is reached and the view-change
    /// was applied, `Ok(false)` when more votes are still needed.
    pub fn record_view_change(&mut self, validator_id: &str, new_view: u32) -> Result<bool> {
        // Byzantine-fault threshold for forcing a view-change: ⌊n/3⌋+1
        // (the smallest set that is guaranteed to contain at least one honest node).
        let n = self.validator_set.len();
        let threshold = n / 3 + 1;

        let votes = self
            .view_change_votes
            .entry(new_view)
            .or_default();
        votes.insert(validator_id.to_string());

        let count = votes.len();
        tracing::debug!(
            "[PBFT] ViewChange cert for view={} from {} ({}/{} needed)",
            new_view, validator_id, count, threshold
        );

        if count >= threshold && self.view_number < new_view {
            tracing::warn!(
                "[PBFT] ViewChange quorum reached ({}/{}) for view={} — executing view-change",
                count, threshold, new_view
            );
            self.trigger_view_change()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Attempt a view-change: increment the view number, reset PREPARE/COMMIT
    /// state, and return to PrePrepare so a new proposer can drive the round.
    ///
    /// Returns `Err` if the configured max view-changes have already been exhausted,
    /// in which case the round should be abandoned.
    pub fn trigger_view_change(&mut self) -> Result<u32> {
        if self.view_change_count >= self.config.max_view_changes {
            self.phase = PbftPhase::Failed;
            bail!(
                "View-change limit ({}) exhausted — round abandoned",
                self.config.max_view_changes
            );
        }
        self.view_change_count += 1;
        self.view_number += 1;
        self.prepare_sigs.clear();
        self.commit_sigs.clear();
        self.phase = PbftPhase::PrePrepare;
        self.phase_entered_at = Instant::now();
        tracing::warn!(
            "[PBFT] VIEW-CHANGE #{} (view={})",
            self.view_change_count,
            self.view_number
        );
        Ok(self.view_number)
    }

    /// Whether this round is in a terminal state (Finalised or Failed).
    pub fn is_terminal(&self) -> bool {
        matches!(self.phase, PbftPhase::Finalised | PbftPhase::Failed)
    }
}

/// Top-level engine managing multiple concurrent PBFT rounds (one per pending block).
pub struct PbftEngine {
    rounds: HashMap<String, PbftRound>, // block_hash → round
    config: PbftConfig,
}

fn quorum_threshold(validator_count: usize, allow_small_cluster_quorum: bool) -> usize {
    match validator_count {
        0 => 0,
        3 if allow_small_cluster_quorum => 2,
        n => (2 * n / 3) + 1,
    }
}

impl PbftEngine {
    pub fn new() -> Self {
        Self {
            rounds: HashMap::new(),
            config: PbftConfig::default(),
        }
    }

    pub fn with_config(config: PbftConfig) -> Self {
        Self {
            rounds: HashMap::new(),
            config,
        }
    }

    pub fn start_round(&mut self, block: Block, vs: ValidatorSet) -> Result<String> {
        let hash = block.hash();
        let mut round = PbftRound::with_config(block, vs, self.config.clone());
        let proposer = round.block.header.proposer_id.clone();
        round.pre_prepare(&proposer)?;
        self.rounds.insert(hash.clone(), round);
        Ok(hash)
    }

    pub fn prepare(
        &mut self,
        block_hash: &str,
        vid: &str,
        sig: BlockSignature,
    ) -> Result<bool> {
        let round = self
            .rounds
            .get_mut(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        round.receive_prepare(vid, sig)
    }

    pub fn commit(&mut self, block_hash: &str, vid: &str, sig: BlockSignature) -> Result<bool> {
        let round = self
            .rounds
            .get_mut(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        round.receive_commit(vid, sig)
    }

    pub fn finalised_sigs(&self, block_hash: &str) -> Vec<BlockSignature> {
        self.rounds
            .get(block_hash)
            .map(|r| r.finalised_signatures())
            .unwrap_or_default()
    }

    pub fn get_finalised_block(&self, block_hash: &str) -> Option<Block> {
        let round = self.rounds.get(block_hash)?;
        if round.phase == PbftPhase::Finalised {
            let mut final_block = round.block.clone();
            final_block.pbft_signatures = round.finalised_signatures();
            Some(final_block)
        } else {
            None
        }
    }

    /// Check all active rounds for phase timeouts and trigger view-changes
    /// where needed.  Rounds that exhaust their view-change budget are moved
    /// to `Failed`.
    ///
    /// Returns `ViewChangeSignal` structs for every round that had a
    /// view-change triggered.  The caller should broadcast a
    /// `GossipMessage::ViewChange` for each signal so that peers can
    /// accumulate certificates and apply the view-change once they reach
    /// their own ⌊n/3⌋+1 threshold.
    pub fn timeout_rounds(&mut self) -> Vec<ViewChangeSignal> {
        let timed_out: Vec<String> = self
            .rounds
            .iter()
            .filter(|(_, r)| r.is_phase_timed_out())
            .map(|(h, _)| h.clone())
            .collect();

        let mut signals = Vec::new();
        for hash in timed_out {
            if let Some(round) = self.rounds.get_mut(&hash) {
                match round.trigger_view_change() {
                    Ok(new_view) => {
                        tracing::warn!(
                            "[PBFT] Timeout on block {} — triggered view-change to view {}",
                            &hash[..12],
                            new_view
                        );
                        signals.push(ViewChangeSignal {
                            block_hash: hash,
                            new_view,
                        });
                    }
                    Err(e) => {
                        tracing::error!("[PBFT] Round {} abandoned: {}", &hash[..12], e);
                    }
                }
            }
        }
        signals
    }

    /// Record a view-change certificate received from a peer for the given block.
    /// Forwards to the matching `PbftRound::record_view_change()`.
    pub fn receive_view_change(
        &mut self,
        block_hash: &str,
        validator_id: &str,
        new_view: u32,
    ) -> Result<bool> {
        let round = self
            .rounds
            .get_mut(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        round.record_view_change(validator_id, new_view)
    }

    /// Remove rounds that have been in a terminal state (Finalised / Failed)
    /// for longer than the configured stale round TTL. Returns the number of
    /// rounds removed.
    pub fn cleanup_stale_rounds(&mut self) -> usize {
        let ttl = self.config.stale_round_ttl;
        let stale: Vec<String> = self
            .rounds
            .iter()
            .filter(|(_, r)| r.is_terminal() && r.created_at.elapsed() > ttl)
            .map(|(h, _)| h.clone())
            .collect();
        let count = stale.len();
        for hash in stale {
            self.rounds.remove(&hash);
        }
        if count > 0 {
            tracing::info!("[PBFT] Cleaned up {} stale rounds", count);
        }
        count
    }

    /// Number of currently-tracked rounds (for metrics / observability).
    pub fn active_round_count(&self) -> usize {
        self.rounds.len()
    }
}

#[cfg(test)]
mod tests {
    use super::quorum_threshold;

    #[test]
    fn small_cluster_quorum_is_explicitly_gated() {
        assert_eq!(quorum_threshold(0, false), 0);
        assert_eq!(quorum_threshold(1, false), 1);
        assert_eq!(quorum_threshold(2, false), 2);
        assert_eq!(quorum_threshold(3, false), 3);
        assert_eq!(quorum_threshold(3, true), 2);
    }

    #[test]
    fn larger_validator_sets_use_pbft_quorum_regardless_of_flag() {
        assert_eq!(quorum_threshold(4, false), 3);
        assert_eq!(quorum_threshold(4, true), 3);
        assert_eq!(quorum_threshold(5, false), 4);
        assert_eq!(quorum_threshold(7, true), 5);
    }
}
