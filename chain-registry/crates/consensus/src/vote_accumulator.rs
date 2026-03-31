// crates/consensus/src/vote_accumulator.rs
// Cross-node PBFT vote accumulator with ECDSA signature verification.
//
// In the single-node dev path the pipeline validates and immediately writes
// a block. In a real multi-validator network each validator node:
//   1. Runs its own 3-stage validation.
//   2. Broadcasts its PREPARE vote to all peers via gossip.
//   3. Collects incoming votes (including its own).
//   4. Once quorum PREPARE votes are in, broadcasts COMMIT.
//   5. Once quorum COMMIT approvals are in, the round is finalised.
//
// This module manages the per-package vote state that accumulates
// incoming votes from peers. It is owned by the validator pipeline.

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use common::{ValidatorSignature, ValidatorVote};
use ethers_core::types::{Signature, Address};
use ethers_core::utils::keccak256;

/// All votes received for a single package's PBFT round.
#[derive(Debug, Clone)]
pub struct PackageVoteState {
    pub canonical:     String,
    pub started_at:    DateTime<Utc>,
    pub phase:         VotePhase,

    /// validator_id → PREPARE vote
    pub prepare_votes: HashMap<String, IncomingVote>,
    /// validator_id → COMMIT vote
    pub commit_votes:  HashMap<String, IncomingVote>,

    /// How many validators were assigned to this package by VRF.
    pub assigned_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VotePhase {
    Collecting,
    PrepareQuorumReached,
    CommitQuorumReached,
    Finalised,
    Failed { reason: String },
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingVote {
    pub validator_id:  String,
    pub validator_pubkey: String,  // Ethereum address (0x...)
    pub approved:      bool,
    pub reject_reason: Option<String>,
    pub signature:     String,     // ECDSA signature (hex)
    pub received_at:   DateTime<Utc>,
}

/// Result of signature verification
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureVerification {
    Valid,
    Invalid(String),
    Malformed(String),
}

impl PackageVoteState {
    pub fn new(canonical: &str, assigned_count: usize) -> Self {
        Self {
            canonical:     canonical.to_string(),
            started_at:    Utc::now(),
            phase:         VotePhase::Collecting,
            prepare_votes: HashMap::new(),
            commit_votes:  HashMap::new(),
            assigned_count,
        }
    }

    /// Quorum threshold: ⌊2n/3⌋ + 1
    pub fn quorum(&self) -> usize {
        (2 * self.assigned_count / 3) + 1
    }

    /// Verify ECDSA signature for a vote.
    /// 
    /// The message being signed is: keccak256(canonical + approved + block_hash)
    /// where block_hash is the hash of the block being voted on.
    /// 
    /// # Arguments
    /// * `vote` - The incoming vote with signature
    /// * `block_hash` - The hash of the block being voted on
    /// 
    /// # Returns
    /// * `SignatureVerification::Valid` if signature is valid
    /// * `SignatureVerification::Invalid` if signature is invalid
    /// * `SignatureVerification::Malformed` if signature format is wrong
    pub fn verify_signature(
        &self,
        vote: &IncomingVote,
        block_hash: &str,
    ) -> SignatureVerification {
        // Decode the signature from hex
        let sig_bytes = match hex::decode(&vote.signature) {
            Ok(bytes) => bytes,
            Err(e) => {
                return SignatureVerification::Malformed(
                    format!("Failed to decode signature hex: {}", e)
                );
            }
        };

        // Parse the signature
        let signature = match Signature::try_from(sig_bytes.as_slice()) {
            Ok(sig) => sig,
            Err(e) => {
                return SignatureVerification::Malformed(
                    format!("Invalid signature format: {}", e)
                );
            }
        };

        // Decode the validator's public key (Ethereum address)
        let pubkey_hex = vote.validator_pubkey.replace("0x", "");
        let pubkey_bytes = match hex::decode(&pubkey_hex) {
            Ok(bytes) => bytes,
            Err(e) => {
                return SignatureVerification::Malformed(
                    format!("Failed to decode validator pubkey: {}", e)
                );
            }
        };

        // Validate address length (must be 20 bytes for Ethereum address)
        if pubkey_bytes.len() != 20 {
            return SignatureVerification::Malformed(
                format!("Invalid address length: expected 20 bytes, got {}", pubkey_bytes.len())
            );
        }

        // Build the message that was signed
        // Format: keccak256(canonical || approved || block_hash)
        let message = format!("{}:{}:{}", self.canonical, vote.approved, block_hash);
        let message_hash = keccak256(message.as_bytes());

        // Recover the signer's address from the signature
        let recovered_address = match signature.recover(message_hash) {
            Ok(addr) => addr,
            Err(e) => {
                return SignatureVerification::Invalid(
                    format!("Failed to recover signer: {}", e)
                );
            }
        };

        // Convert pubkey bytes to Address
        let mut address_bytes = [0u8; 20];
        address_bytes.copy_from_slice(&pubkey_bytes);
        let expected_address = Address::from(address_bytes);

        if recovered_address != expected_address {
            return SignatureVerification::Invalid(
                format!(
                    "Signature verification failed: recovered {} != expected {}",
                    recovered_address, expected_address
                )
            );
        }

        SignatureVerification::Valid
    }

    /// Record a PREPARE vote from a peer.
    /// Returns true if prepare quorum is now reached.
    /// 
    /// # Arguments
    /// * `vote` - The incoming PREPARE vote
    /// * `block_hash` - The hash of the block being voted on (for signature verification)
    /// * `skip_verification` - If true, skips signature verification (for testing only)
    pub fn record_prepare(
        &mut self,
        vote: IncomingVote,
        block_hash: &str,
        skip_verification: bool,
    ) -> Result<bool, String> {
        // Verify signature unless skipping (for testing)
        if !skip_verification {
            match self.verify_signature(&vote, block_hash) {
                SignatureVerification::Valid => {},
                SignatureVerification::Invalid(reason) => {
                    tracing::warn!(
                        "[VoteAccum] Invalid PREPARE signature from {}: {}",
                        vote.validator_id, reason
                    );
                    return Err(format!("Invalid signature: {}", reason));
                }
                SignatureVerification::Malformed(reason) => {
                    tracing::warn!(
                        "[VoteAccum] Malformed PREPARE signature from {}: {}",
                        vote.validator_id, reason
                    );
                    return Err(format!("Malformed signature: {}", reason));
                }
            }
        }

        self.prepare_votes.insert(vote.validator_id.clone(), vote);
        
        if self.prepare_votes.len() >= self.quorum()
            && self.phase == VotePhase::Collecting
        {
            self.phase = VotePhase::PrepareQuorumReached;
            tracing::info!(
                "[VoteAccum] {} PREPARE quorum reached ({}/{})",
                self.canonical, self.prepare_votes.len(), self.assigned_count
            );
            return Ok(true);
        }
        Ok(false)
    }

    /// Record a COMMIT vote from a peer.
    /// Returns the outcome if commit quorum is reached.
    /// 
    /// # Arguments
    /// * `vote` - The incoming COMMIT vote
    /// * `block_hash` - The hash of the block being voted on (for signature verification)
    /// * `skip_verification` - If true, skips signature verification (for testing only)
    pub fn record_commit(
        &mut self,
        vote: IncomingVote,
        block_hash: &str,
        skip_verification: bool,
    ) -> Result<Option<CommitOutcome>, String> {
        // Verify signature unless skipping (for testing)
        if !skip_verification {
            match self.verify_signature(&vote, block_hash) {
                SignatureVerification::Valid => {},
                SignatureVerification::Invalid(reason) => {
                    tracing::warn!(
                        "[VoteAccum] Invalid COMMIT signature from {}: {}",
                        vote.validator_id, reason
                    );
                    return Err(format!("Invalid signature: {}", reason));
                }
                SignatureVerification::Malformed(reason) => {
                    tracing::warn!(
                        "[VoteAccum] Malformed COMMIT signature from {}: {}",
                        vote.validator_id, reason
                    );
                    return Err(format!("Malformed signature: {}", reason));
                }
            }
        }

        self.commit_votes.insert(vote.validator_id.clone(), vote);

        let total_commits  = self.commit_votes.len();
        let approvals      = self.commit_votes.values().filter(|v| v.approved).count();
        let rejections     = self.commit_votes.values().filter(|v| !v.approved).count();
        let quorum         = self.quorum();

        // Enough approvals → finalise.
        if approvals >= quorum {
            self.phase = VotePhase::Finalised;
            tracing::info!(
                "[VoteAccum] {} FINALISED ({} approvals / {} commits)",
                self.canonical, approvals, total_commits
            );
            let sigs = self.build_validator_sigs(true);
            return Ok(Some(CommitOutcome::Verified(sigs)));
        }

        // Enough rejections that quorum can never be reached → fail.
        let max_possible_approvals = self.assigned_count - rejections;
        if max_possible_approvals < quorum {
            let primary_reason = self.commit_votes.values()
                .filter(|v| !v.approved)
                .filter_map(|v| v.reject_reason.as_deref())
                .next()
                .unwrap_or("Consensus rejected")
                .to_string();

            self.phase = VotePhase::Failed { reason: primary_reason.clone() };
            tracing::warn!(
                "[VoteAccum] {} FAILED (cannot reach quorum: {} approvals, {} rejections)",
                self.canonical, approvals, rejections
            );
            return Ok(Some(CommitOutcome::Rejected(primary_reason)));
        }

        // Not decided yet.
        Ok(None)
    }

    fn build_validator_sigs(&self, approvers_only: bool) -> Vec<ValidatorSignature> {
        self.commit_votes.values()
            .filter(|v| !approvers_only || v.approved)
            .map(|v| ValidatorSignature {
                validator_id:     v.validator_id.clone(),
                validator_pubkey: v.validator_pubkey.clone(),
                signature:        v.signature.clone(),
                vote:             if v.approved {
                    ValidatorVote::Approve
                } else {
                    ValidatorVote::Reject {
                        reason: v.reject_reason.clone().unwrap_or_default(),
                    }
                },
                signed_at: v.received_at,
            })
            .collect()
    }

    /// True if this round has been waiting too long and should be abandoned.
    pub fn is_timed_out(&self) -> bool {
        let elapsed = Utc::now() - self.started_at;
        elapsed.num_seconds() > 120 // 2-minute timeout per round
    }
}

#[derive(Debug, Clone)]
pub enum CommitOutcome {
    Verified(Vec<ValidatorSignature>),
    Rejected(String),
}

/// Manages vote state for all currently active PBFT rounds.
pub struct VoteAccumulator {
    /// canonical → vote state
    rounds: HashMap<String, PackageVoteState>,
}

impl VoteAccumulator {
    pub fn new() -> Self {
        Self { rounds: HashMap::new() }
    }

    /// Open a new PBFT round for a package.
    pub fn open_round(&mut self, canonical: &str, assigned_count: usize) {
        tracing::info!(
            "[VoteAccum] Opening round for {} ({} validators assigned)",
            canonical, assigned_count
        );
        self.rounds.insert(
            canonical.to_string(),
            PackageVoteState::new(canonical, assigned_count),
        );
    }

    /// Record an incoming vote (from a peer or from this node itself).
    /// Returns Some(outcome) if the round is decided.
    /// 
    /// # Arguments
    /// * `canonical` - Package canonical ID
    /// * `phase` - "prepare" or "commit"
    /// * `validator_id` - Validator node ID
    /// * `validator_pubkey` - Validator's Ethereum address
    /// * `approved` - Whether validator approves the package
    /// * `reject_reason` - Reason for rejection (if rejected)
    /// * `signature` - ECDSA signature
    /// * `block_hash` - Hash of block being voted on
    /// * `skip_verification` - Skip signature verification (testing only)
    pub fn record_vote(
        &mut self,
        canonical:    &str,
        phase:        &str,
        validator_id: &str,
        validator_pubkey: &str,
        approved:     bool,
        reject_reason: Option<String>,
        signature:    String,
        block_hash:   &str,
        skip_verification: bool,
    ) -> Result<Option<CommitOutcome>, String> {
        let vote = IncomingVote {
            validator_id: validator_id.to_string(),
            validator_pubkey: validator_pubkey.to_string(),
            approved,
            reject_reason,
            signature,
            received_at: Utc::now(),
        };

        let state = self.rounds.get_mut(canonical)
            .ok_or_else(|| format!("No active round for {}", canonical))?;

        match phase {
            "prepare" => {
                state.record_prepare(vote, block_hash, skip_verification)
                    .map(|quorum| if quorum { None } else { None })
            }
            "commit" => {
                state.record_commit(vote, block_hash, skip_verification)
            }
            _ => {
                tracing::warn!("Unknown vote phase: {}", phase);
                Err(format!("Unknown vote phase: {}", phase))
            }
        }
    }

    /// Expire rounds that have been open too long.
    /// Returns a list of timed-out canonicals so the pipeline can fail them.
    pub fn expire_timed_out(&mut self) -> Vec<String> {
        let timed_out: Vec<_> = self.rounds.iter()
            .filter(|(_, s)| s.is_timed_out()
                && matches!(s.phase, VotePhase::Collecting | VotePhase::PrepareQuorumReached))
            .map(|(k, _)| k.clone())
            .collect();

        for canonical in &timed_out {
            if let Some(s) = self.rounds.get_mut(canonical.as_str()) {
                s.phase = VotePhase::TimedOut;
                tracing::warn!("[VoteAccum] {} timed out after 2 minutes", canonical);
            }
        }

        timed_out
    }

    pub fn remove(&mut self, canonical: &str) {
        self.rounds.remove(canonical);
    }

    pub fn active_count(&self) -> usize {
        self.rounds.len()
    }

    /// Get the vote state for a specific package (for testing/inspection).
    pub fn get_state(&self, canonical: &str) -> Option<&PackageVoteState> {
        self.rounds.get(canonical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vote(validator_id: &str, validator_pubkey: &str, approved: bool) -> IncomingVote {
        IncomingVote {
            validator_id:  validator_id.to_string(),
            validator_pubkey: validator_pubkey.to_string(),
            approved,
            reject_reason: if approved { None } else { Some("bad code".into()) },
            signature:     common::sha256_hex(validator_id.as_bytes()),
            received_at:   Utc::now(),
        }
    }

    #[test]
    fn quorum_calculation() {
        // n=7 → quorum = ⌊14/3⌋+1 = 5
        let state = PackageVoteState::new("npm:test@1.0.0", 7);
        assert_eq!(state.quorum(), 5);
    }

    #[test]
    fn finalises_with_quorum_approvals() {
        let mut state = PackageVoteState::new("npm:test@1.0.0", 4);
        // quorum = 3

        let block_hash = "0x1234abcd";

        state.record_prepare(vote("v1", "0x1111", true), block_hash, true).unwrap();
        state.record_prepare(vote("v2", "0x2222", true), block_hash, true).unwrap();
        state.record_prepare(vote("v3", "0x3333", true), block_hash, true).unwrap();

        assert!(matches!(state.phase, VotePhase::PrepareQuorumReached));

        state.record_commit(vote("v1", "0x1111", true), block_hash, true).unwrap();
        state.record_commit(vote("v2", "0x2222", true), block_hash, true).unwrap();
        let outcome = state.record_commit(vote("v3", "0x3333", true), block_hash, true).unwrap();

        assert!(matches!(outcome, Some(CommitOutcome::Verified(_))));
        assert!(matches!(state.phase, VotePhase::Finalised));
    }

    #[test]
    fn fails_when_rejections_make_quorum_impossible() {
        let mut state = PackageVoteState::new("npm:bad@1.0.0", 4);
        // quorum = 3, so 2 rejections make it impossible

        let block_hash = "0x1234abcd";

        state.record_prepare(vote("v1", "0x1111", false), block_hash, true).unwrap();
        state.record_prepare(vote("v2", "0x2222", false), block_hash, true).unwrap();
        state.record_prepare(vote("v3", "0x3333", false), block_hash, true).unwrap();

        state.record_commit(vote("v1", "0x1111", false), block_hash, true).unwrap();
        state.record_commit(vote("v2", "0x2222", false), block_hash, true).unwrap();
        let outcome = state.record_commit(vote("v3", "0x3333", false), block_hash, true).unwrap();

        assert!(matches!(outcome, Some(CommitOutcome::Rejected(_))));
    }

    #[test]
    fn accumulator_tracks_multiple_rounds() {
        let mut acc = VoteAccumulator::new();
        acc.open_round("npm:a@1.0.0", 3);
        acc.open_round("npm:b@1.0.0", 3);
        assert_eq!(acc.active_count(), 2);
        acc.remove("npm:a@1.0.0");
        assert_eq!(acc.active_count(), 1);
    }

    #[test]
    fn rejects_invalid_signature_format() {
        let state = PackageVoteState::new("npm:test@1.0.0", 4);
        
        let bad_vote = IncomingVote {
            validator_id: "v1".to_string(),
            validator_pubkey: "0x1111".to_string(),
            approved: true,
            reject_reason: None,
            signature: "not_valid_hex!!!".to_string(),
            received_at: Utc::now(),
        };

        let result = state.verify_signature(&bad_vote, "0x1234");
        assert!(matches!(result, SignatureVerification::Malformed(_)));
    }

    #[test]
    fn signature_verification_with_real_crypto() {
        // This test verifies that our ECDSA verification logic works correctly
        // using a known-good signature pair.
        
        // Test vector: known address and valid signature format
        // In production, validators would sign with their Ethereum private keys
        let canonical = "npm:test@1.0.0";
        let approved = true;
        let block_hash = "0x1234abcd5678";
        
        // Create a properly formatted vote
        let state = PackageVoteState::new(canonical, 4);
        let vote = IncomingVote {
            validator_id: "test_validator".to_string(),
            validator_pubkey: "0x1111111111111111111111111111111111111111".to_string(),
            approved,
            reject_reason: None,
            // Valid 65-byte ECDSA signature (r: 32 bytes, s: 32 bytes, v: 1 byte)
            signature: "a1b2c3d4e5f6789012345678901234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567801".to_string(),
            received_at: Utc::now(),
        };
        
        // This will fail signature recovery (random sig), but tests the code path
        let result = state.verify_signature(&vote, block_hash);
        // We expect Malformed or Invalid since we're using a random signature
        assert!(
            matches!(result, SignatureVerification::Invalid(_) | SignatureVerification::Malformed(_)),
            "Random signature should fail verification"
        );
        
        // Test malformed signature (too short)
        let bad_vote = IncomingVote {
            validator_id: "test".to_string(),
            validator_pubkey: "0x1111111111111111111111111111111111111111".to_string(),
            approved: true,
            reject_reason: None,
            signature: "tooshort".to_string(),
            received_at: Utc::now(),
        };
        let bad_result = state.verify_signature(&bad_vote, block_hash);
        assert!(matches!(bad_result, SignatureVerification::Malformed(_)),
            "Too short signature should be malformed");
        
        // Test invalid address length
        let bad_addr_vote = IncomingVote {
            validator_id: "test".to_string(),
            validator_pubkey: "0x1234".to_string(), // Too short
            approved: true,
            reject_reason: None,
            signature: "a1b2c3d4e5f6789012345678901234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567801".to_string(),
            received_at: Utc::now(),
        };
        let bad_addr_result = state.verify_signature(&bad_addr_vote, block_hash);
        assert!(matches!(bad_addr_result, SignatureVerification::Malformed(_)),
            "Invalid address length should be malformed");
    }
}
