// crates/consensus/src/pbft.rs
// Three-phase PBFT: PRE-PREPARE, PREPARE, COMMIT.
// Safety guarantee: the network is correct as long as fewer than ⌊n/3⌋
// validators are faulty or Byzantine.

use crate::ValidatorSet;
use anyhow::{bail, Result};
use common::{Block, ValidatorSignature, ValidatorVote};
use std::collections::HashMap;
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

/// Configuration for PBFT consensus parameters.
/// All values have sensible defaults and can be overridden via environment
/// variables at startup.
#[derive(Debug, Clone)]
pub struct PbftConfig {
    pub round_phase_timeout: Duration,
    pub max_view_changes: u32,
    pub stale_round_ttl: Duration,
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
        }
    }
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

/// State of a single PBFT consensus round.
pub struct PbftRound {
    pub block: Block,
    pub phase: PbftPhase,
    /// validator_id → their PREPARE message signature
    pub prepare_sigs: HashMap<String, ValidatorSignature>,
    /// validator_id → their COMMIT message signature
    pub commit_sigs: HashMap<String, ValidatorSignature>,
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
        }
    }

    /// Quorum threshold: ⌊(2n/3)⌋ + 1
    pub fn quorum(&self) -> usize {
        let n = self.validator_set.len();
        (2 * n / 3) + 1
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
        // If the block includes a VRF proof, verify it and the proposer selection.
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
            let mut active: Vec<crate::vrf::VrfValidator> = self
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
            // Inject the proposer's VRF output+proof so select_proposer can verify it.
            for v in &mut active {
                if v.id == proposer_id {
                    v.vrf_output = Some(output.clone());
                    v.vrf_proof = Some(proof.clone());
                }
            }
            let selected = crate::vrf::select_proposer(&active, epoch_seed)
                .ok_or_else(|| anyhow::anyhow!("No active validators to select proposer"))?;
            if &selected != proposer_id {
                bail!(
                    "Proposer {} is not the VRF-selected proposer (expected {})",
                    proposer_id,
                    selected
                );
            }
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
    pub fn receive_prepare(&mut self, validator_id: &str, sig: ValidatorSignature) -> Result<bool> {
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
    pub fn receive_commit(&mut self, validator_id: &str, sig: ValidatorSignature) -> Result<bool> {
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
            // Check if enough commits are approvals (not rejections).
            let approvals = self
                .commit_sigs
                .values()
                .filter(|s| s.vote == ValidatorVote::Approve)
                .count();

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
    pub fn finalised_signatures(&self) -> Vec<ValidatorSignature> {
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
        sig: ValidatorSignature,
    ) -> Result<bool> {
        let round = self
            .rounds
            .get_mut(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        round.receive_prepare(vid, sig)
    }

    pub fn commit(&mut self, block_hash: &str, vid: &str, sig: ValidatorSignature) -> Result<bool> {
        let round = self
            .rounds
            .get_mut(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        round.receive_commit(vid, sig)
    }

    pub fn finalised_sigs(&self, block_hash: &str) -> Vec<ValidatorSignature> {
        self.rounds
            .get(block_hash)
            .map(|r| r.finalised_signatures())
            .unwrap_or_default()
    }

    /// Check all active rounds for phase timeouts and trigger view-changes
    /// where needed.  Rounds that exhaust their view-change budget are moved
    /// to `Failed`.
    ///
    /// Returns the list of block hashes that had a view-change triggered.
    pub fn timeout_rounds(&mut self) -> Vec<String> {
        let timed_out: Vec<String> = self
            .rounds
            .iter()
            .filter(|(_, r)| r.is_phase_timed_out())
            .map(|(h, _)| h.clone())
            .collect();

        let mut changed = Vec::new();
        for hash in timed_out {
            if let Some(round) = self.rounds.get_mut(&hash) {
                match round.trigger_view_change() {
                    Ok(view) => {
                        tracing::warn!(
                            "[PBFT] Timeout on block {} — triggered view-change to view {}",
                            &hash[..12],
                            view
                        );
                        changed.push(hash);
                    }
                    Err(e) => {
                        tracing::error!("[PBFT] Round {} abandoned: {}", &hash[..12], e);
                    }
                }
            }
        }
        changed
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
