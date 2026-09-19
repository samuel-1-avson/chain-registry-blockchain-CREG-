//! Equivocation Slashing Relay Bridge
//!
//! Tracks PBFT and evidence voting messages across rounds and automatically
//! constructs cryptographic double-sign slashing evidence against Byzantine validators.

use anyhow::{bail, Result};
use common::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A recorded vote observed on the network
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedVote {
    pub validator_id: String,
    pub validator_pubkey_hex: String,
    pub block_height: u64,
    pub view_number: u32,
    pub phase: String,
    pub block_hash: String,
    pub signature_hex: String,
    pub timestamp: u64,
}

/// Cryptographic evidence proving a validator double-signed conflicting blocks
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquivocationEvidence {
    pub validator_id: String,
    pub validator_pubkey_hex: String,
    pub block_height: u64,
    pub view_number: u32,
    pub phase: String,
    pub vote_a: ObservedVote,
    pub vote_b: ObservedVote,
    pub evidence_hash: String,
}

/// On-chain slashing submission payload for L1 Staking & ZKSlashingVerifier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlashingRelayPayload {
    pub target_validator_eth_address: Option<String>,
    pub target_validator_pubkey: String,
    pub offense_type: String,
    pub block_height: u64,
    pub message1: String,
    pub signature1: String,
    pub message2: String,
    pub signature2: String,
    pub proof_hash: String,
}

/// State tracker that detects double-signing offenses across active rounds
#[derive(Default)]
pub struct EquivocationDetector {
    /// Key: (validator_id, height, view, phase) -> ObservedVote
    seen_votes: HashMap<(String, u64, u32, String), ObservedVote>,
    /// Accumulated slashing evidence ready for relay
    pending_slashes: Vec<EquivocationEvidence>,
}

impl EquivocationDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingest a vote message. If a conflicting vote is detected from the same
    /// validator for the same (height, view, phase), an EquivocationEvidence is generated.
    pub fn record_vote(&mut self, vote: ObservedVote) -> Option<EquivocationEvidence> {
        let key = (
            vote.validator_id.clone(),
            vote.block_height,
            vote.view_number,
            vote.phase.clone(),
        );

        if let Some(existing) = self.seen_votes.get(&key) {
            // Check if this is an equivocation (different block hash signed)
            if existing.block_hash != vote.block_hash {
                let evidence_hash = sha256_hex(
                    format!(
                        "creg-equivocation-v1:{}:{}:{}:{}:{}",
                        vote.validator_id,
                        vote.block_height,
                        vote.view_number,
                        existing.block_hash,
                        vote.block_hash
                    )
                    .as_bytes(),
                );

                let evidence = EquivocationEvidence {
                    validator_id: vote.validator_id.clone(),
                    validator_pubkey_hex: vote.validator_pubkey_hex.clone(),
                    block_height: vote.block_height,
                    view_number: vote.view_number,
                    phase: vote.phase.clone(),
                    vote_a: existing.clone(),
                    vote_b: vote.clone(),
                    evidence_hash,
                };

                self.pending_slashes.push(evidence.clone());
                tracing::warn!(
                    "[SlashingRelay] EQUIVOCATION DETECTED: Validator {} double-signed at height {} view {}",
                    vote.validator_id,
                    vote.block_height,
                    vote.view_number
                );
                return Some(evidence);
            }
        } else {
            self.seen_votes.insert(key, vote);
        }

        None
    }

    /// Retrieve all pending slashing evidence
    pub fn pending_evidence(&self) -> &[EquivocationEvidence] {
        &self.pending_slashes
    }

    /// Generate an on-chain relay payload from evidence
    pub fn format_slashing_payload(
        evidence: &EquivocationEvidence,
        eth_address: Option<String>,
    ) -> SlashingRelayPayload {
        let msg1 = format!(
            "creg-pbft-v1:{}:{}",
            evidence.phase, evidence.vote_a.block_hash
        );
        let msg2 = format!(
            "creg-pbft-v1:{}:{}",
            evidence.phase, evidence.vote_b.block_hash
        );

        SlashingRelayPayload {
            target_validator_eth_address: eth_address,
            target_validator_pubkey: evidence.validator_pubkey_hex.clone(),
            offense_type: "DoubleSign".into(),
            block_height: evidence.block_height,
            message1: msg1,
            signature1: evidence.vote_a.signature_hex.clone(),
            message2: msg2,
            signature2: evidence.vote_b.signature_hex.clone(),
            proof_hash: evidence.evidence_hash.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equivocation_detection_and_payload_generation() {
        let mut detector = EquivocationDetector::new();

        let vote1 = ObservedVote {
            validator_id: "val-node-1".into(),
            validator_pubkey_hex: "0xdeadbeef".into(),
            block_height: 100,
            view_number: 0,
            phase: "prepare".into(),
            block_hash: "0xAAAA".into(),
            signature_hex: "0x1111".into(),
            timestamp: 1000,
        };

        // Same vote duplicate should not trigger equivocation
        assert!(detector.record_vote(vote1.clone()).is_none());
        assert!(detector.record_vote(vote1).is_none());

        // Conflicting vote for different block hash at same height/view/phase
        let vote2 = ObservedVote {
            validator_id: "val-node-1".into(),
            validator_pubkey_hex: "0xdeadbeef".into(),
            block_height: 100,
            view_number: 0,
            phase: "prepare".into(),
            block_hash: "0xBBBB".into(),
            signature_hex: "0x2222".into(),
            timestamp: 1005,
        };

        let evidence = detector
            .record_vote(vote2)
            .expect("should detect equivocation");
        assert_eq!(evidence.validator_id, "val-node-1");
        assert_eq!(evidence.vote_a.block_hash, "0xAAAA");
        assert_eq!(evidence.vote_b.block_hash, "0xBBBB");

        let payload = EquivocationDetector::format_slashing_payload(
            &evidence,
            Some("0x1234567890123456789012345678901234567890".into()),
        );
        assert_eq!(payload.offense_type, "DoubleSign");
        assert_eq!(payload.message1, "creg-pbft-v1:prepare:0xAAAA");
        assert_eq!(payload.message2, "creg-pbft-v1:prepare:0xBBBB");
    }
}
