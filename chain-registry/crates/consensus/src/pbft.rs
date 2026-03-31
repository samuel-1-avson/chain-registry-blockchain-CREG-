// crates/consensus/src/pbft.rs
// Three-phase PBFT: PRE-PREPARE, PREPARE, COMMIT.
// Safety guarantee: the network is correct as long as fewer than ⌊n/3⌋
// validators are faulty or Byzantine.

use anyhow::{bail, Result};
use common::{Block, ValidatorSignature, ValidatorVote};
use std::collections::HashMap;
use crate::ValidatorSet;

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
}

impl PbftRound {
    pub fn new(block: Block, validator_set: ValidatorSet) -> Self {
        Self {
            block,
            phase: PbftPhase::PrePrepare,
            prepare_sigs: HashMap::new(),
            commit_sigs: HashMap::new(),
            validator_set,
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
        // Broadcast the block hash — validators use this as the message digest.
        let block_hash = self.block.hash();
        tracing::info!("[PBFT] PRE-PREPARE: block {} from {}", &block_hash[..12], proposer_id);
        self.phase = PbftPhase::Prepare;
        Ok(block_hash)
    }

    // ── Phase 2: PREPARE ─────────────────────────────────────────────────────
    /// A validator casts its PREPARE vote (approve or reject) over the block hash.
    pub fn receive_prepare(
        &mut self,
        validator_id: &str,
        sig: ValidatorSignature,
    ) -> Result<bool> {
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
            tracing::info!("[PBFT] PREPARE quorum reached — moving to COMMIT");
            return Ok(true); // caller should now broadcast COMMIT
        }
        Ok(false)
    }

    // ── Phase 3: COMMIT ──────────────────────────────────────────────────────
    /// A validator sends its COMMIT signature. Once quorum is reached the
    /// block is finalised and can be written to the chain.
    pub fn receive_commit(
        &mut self,
        validator_id: &str,
        sig: ValidatorSignature,
    ) -> Result<bool> {
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
            let approvals = self.commit_sigs.values()
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
                tracing::warn!("[PBFT] FAILED — insufficient approvals ({}/{})", approvals, self.quorum());
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// Returns the finalised signatures to embed in the ChainRecord.
    pub fn finalised_signatures(&self) -> Vec<ValidatorSignature> {
        self.commit_sigs.values().cloned().collect()
    }
}

/// Top-level engine managing multiple concurrent PBFT rounds (one per pending block).
pub struct PbftEngine {
    rounds: HashMap<String, PbftRound>, // block_hash → round
}

impl PbftEngine {
    pub fn new() -> Self {
        Self { rounds: HashMap::new() }
    }

    pub fn start_round(&mut self, block: Block, vs: ValidatorSet) -> Result<String> {
        let hash = block.hash();
        let mut round = PbftRound::new(block, vs);
        let proposer = round.block.header.proposer_id.clone();
        round.pre_prepare(&proposer)?;
        self.rounds.insert(hash.clone(), round);
        Ok(hash)
    }

    pub fn prepare(&mut self, block_hash: &str, vid: &str, sig: ValidatorSignature) -> Result<bool> {
        let round = self.rounds.get_mut(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        round.receive_prepare(vid, sig)
    }

    pub fn commit(&mut self, block_hash: &str, vid: &str, sig: ValidatorSignature) -> Result<bool> {
        let round = self.rounds.get_mut(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        round.receive_commit(vid, sig)
    }

    pub fn finalised_sigs(&self, block_hash: &str) -> Vec<ValidatorSignature> {
        self.rounds.get(block_hash)
            .map(|r| r.finalised_signatures())
            .unwrap_or_default()
    }
}
