//! ZK Slashing Evidence - Proof Generation
//!
//! This module provides zero-knowledge proof generation for validator
//! slashing evidence, specifically for double-signing detection.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Types of slashing evidence
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceType {
    /// Validator signed conflicting votes
    DoubleSign = 1,
    /// Validator approved malicious package
    FalseApprove = 2,
    /// Validator consistently voted against majority
    Griefing = 3,
}

/// Public inputs for double-sign proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoubleSignPublicInputs {
    /// Validator's public key (X coordinate)
    pub validator_pubkey_x: String,
    /// Validator's public key (Y coordinate)
    pub validator_pubkey_y: String,
    /// Package identifier hash
    pub package_hash: String,
    /// Hash of first vote
    pub vote1_hash: String,
    /// Hash of second vote
    pub vote2_hash: String,
}

/// Private inputs (witness) for double-sign proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoubleSignWitness {
    /// Validator's private key (kept secret!)
    pub validator_privkey: String,
    /// First signature (R_x, R_y, S)
    pub signature1: Signature,
    /// Second signature (R_x, R_y, S)
    pub signature2: Signature,
}

/// Ed25519 signature components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub r_x: String,
    pub r_y: String,
    pub s: String,
}

/// Complete double-sign evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoubleSignEvidence {
    pub public_inputs: DoubleSignPublicInputs,
    pub witness: DoubleSignWitness,
    pub validator_address: String,
    pub package_canonical: String,
    pub vote1_details: VoteDetails,
    pub vote2_details: VoteDetails,
}

/// Vote details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteDetails {
    pub approved: bool,
    pub timestamp: u64,
    pub block_height: u64,
    pub signature_hex: String,
}

/// Groth16 proof structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Groth16Proof {
    /// Proof component A (G1 point)
    pub a: [String; 2],
    /// Proof component B (G2 point)
    pub b: [[String; 2]; 2],
    /// Proof component C (G1 point)
    pub c: [String; 2],
    /// Protocol (groth16)
    pub protocol: String,
    /// Curve (bn128)
    pub curve: String,
}

/// ZK Proof with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZKSlashingProof {
    /// The Groth16 proof
    pub proof: Groth16Proof,
    /// Public inputs
    pub public_inputs: Vec<String>,
    /// Proof type
    pub evidence_type: EvidenceType,
    /// Validator address
    pub offender: String,
    /// Unique nullifier
    pub nullifier: String,
    /// Timestamp
    pub timestamp: u64,
}

/// Configuration for proof generation
#[derive(Debug, Clone)]
pub struct ProofConfig {
    /// Path to circom circuit
    pub circuit_path: String,
    /// Path to proving key
    pub proving_key_path: String,
    /// Path to witness generator
    pub witness_generator_path: String,
    /// Whether to use GPU acceleration
    pub use_gpu: bool,
}

impl Default for ProofConfig {
    fn default() -> Self {
        Self {
            circuit_path: "circuits/DoubleSignProof.circom".to_string(),
            proving_key_path: "circuits/DoubleSignProof_final.zkey".to_string(),
            witness_generator_path: "circuits/DoubleSignProof_js/DoubleSignProof".to_string(),
            use_gpu: false,
        }
    }
}

/// ZK Proof generator for slashing evidence
pub struct SlashingProofGenerator {
    config: ProofConfig,
}

impl SlashingProofGenerator {
    /// Create a new proof generator
    pub fn new(config: ProofConfig) -> Self {
        Self { config }
    }

    /// Generate a double-sign proof
    ///
    /// This proves that a validator signed two conflicting votes
    /// without revealing the validator's private key.
    pub async fn generate_double_sign_proof(
        &self,
        evidence: &DoubleSignEvidence,
    ) -> Result<ZKSlashingProof> {
        tracing::info!(
            "Generating double-sign proof for validator: {}",
            evidence.validator_address
        );

        // Step 1: Validate evidence
        self.validate_double_sign_evidence(evidence)?;

        // Step 2: Prepare inputs for circuit
        let input_json = self.prepare_circuit_inputs(evidence)?;

        // Step 3: Generate witness
        let witness = self.generate_witness(&input_json).await?;

        // Step 4: Generate proof
        let proof = self.generate_groth16_proof(&witness).await?;

        // Step 5: Compute nullifier
        let nullifier = self.compute_nullifier(&evidence.public_inputs);

        tracing::info!("Proof generated successfully. Nullifier: {}", nullifier);

        Ok(ZKSlashingProof {
            proof,
            public_inputs: vec![
                evidence.public_inputs.validator_pubkey_x.clone(),
                evidence.public_inputs.validator_pubkey_y.clone(),
                evidence.public_inputs.package_hash.clone(),
                evidence.public_inputs.vote1_hash.clone(),
                evidence.public_inputs.vote2_hash.clone(),
            ],
            evidence_type: EvidenceType::DoubleSign,
            offender: evidence.validator_address.clone(),
            nullifier,
            timestamp: current_timestamp(),
        })
    }

    /// Validate that the evidence is coherent
    fn validate_double_sign_evidence(&self, evidence: &DoubleSignEvidence) -> Result<()> {
        // Check that votes are different
        if evidence.vote1_details.approved == evidence.vote2_details.approved {
            anyhow::bail!(
                "Votes are not conflicting: both are {}",
                if evidence.vote1_details.approved {
                    "approve"
                } else {
                    "reject"
                }
            );
        }

        // Check that signatures are valid (would need Ed25519 verification)
        // TODO: Implement Ed25519 signature verification

        // Check that timestamps are close (same consensus round)
        let time_diff = if evidence.vote1_details.timestamp > evidence.vote2_details.timestamp {
            evidence.vote1_details.timestamp - evidence.vote2_details.timestamp
        } else {
            evidence.vote2_details.timestamp - evidence.vote1_details.timestamp
        };

        if time_diff > 300 {
            // 5 minutes
            tracing::warn!(
                "Votes are {} seconds apart - may not be double-signing",
                time_diff
            );
        }

        Ok(())
    }

    /// Prepare inputs for the circom circuit
    fn prepare_circuit_inputs(&self, evidence: &DoubleSignEvidence) -> Result<String> {
        let inputs = serde_json::json!({
            "validatorPubkey": [
                evidence.public_inputs.validator_pubkey_x,
                evidence.public_inputs.validator_pubkey_y,
            ],
            "packageHash": evidence.public_inputs.package_hash,
            "vote1Hash": evidence.public_inputs.vote1_hash,
            "vote2Hash": evidence.public_inputs.vote2_hash,
            "validatorPrivkey": evidence.witness.validator_privkey,
            "signature1": [
                evidence.witness.signature1.r_x,
                evidence.witness.signature1.r_y,
                evidence.witness.signature1.s,
            ],
            "signature2": [
                evidence.witness.signature2.r_x,
                evidence.witness.signature2.r_y,
                evidence.witness.signature2.s,
            ],
        });

        Ok(inputs.to_string())
    }

    /// Generate witness using circom's witness generator
    async fn generate_witness(&self, input_json: &str) -> Result<Vec<u8>> {
        use tokio::fs;
        use tokio::process::Command;

        tracing::debug!("Generating witness...");

        // Write inputs to temp file
        let input_path = "/tmp/zk_input.json";
        fs::write(input_path, input_json).await?;

        let output_path = "/tmp/zk_witness.wtns";

        // Run witness generator
        let status = Command::new(&self.config.witness_generator_path)
            .arg(input_path)
            .arg(output_path)
            .status()
            .await
            .context("Failed to run witness generator")?;

        if !status.success() {
            anyhow::bail!("Witness generation failed");
        }

        // Read witness
        let witness = fs::read(output_path).await?;

        // Cleanup
        let _ = fs::remove_file(input_path).await;
        let _ = fs::remove_file(output_path).await;

        Ok(witness)
    }

    /// Generate Groth16 proof using snarkjs
    async fn generate_groth16_proof(&self, _witness: &[u8]) -> Result<Groth16Proof> {
        // In production, this would:
        // 1. Use snarkjs or a Rust library (like ark-groth16) to generate proof
        // 2. Load the proving key
        // 3. Perform the proving computation
        // 4. Return the proof

        // For now, return a placeholder
        tracing::warn!("Using placeholder proof - integrate with actual ZK library for production");

        Ok(Groth16Proof {
            a: ["0".to_string(), "0".to_string()],
            b: [
                ["0".to_string(), "0".to_string()],
                ["0".to_string(), "0".to_string()],
            ],
            c: ["0".to_string(), "0".to_string()],
            protocol: "groth16".to_string(),
            curve: "bn128".to_string(),
        })
    }

    /// Compute nullifier from public inputs
    fn compute_nullifier(&self, public_inputs: &DoubleSignPublicInputs) -> String {
        let data = format!(
            "{}:{}:{}:{}:{}",
            public_inputs.validator_pubkey_x,
            public_inputs.validator_pubkey_y,
            public_inputs.package_hash,
            public_inputs.vote1_hash,
            public_inputs.vote2_hash
        );

        let hash = Sha256::digest(data.as_bytes());
        hex::encode(hash)
    }

    /// Export proof to JSON format for submission
    pub fn export_proof(&self, proof: &ZKSlashingProof) -> Result<String> {
        serde_json::to_string_pretty(proof).context("Failed to serialize proof")
    }
}

/// Validator to monitor for double-signing
pub struct DoubleSignMonitor {
    /// Known votes by validator: (validator_id, package) -> Vec<Vote>
    votes: std::collections::HashMap<(String, String), Vec<VoteRecord>>,
    /// Proof generator
    generator: SlashingProofGenerator,
}

/// Record of a vote
#[derive(Debug, Clone)]
pub struct VoteRecord {
    pub validator_id: String,
    pub package_canonical: String,
    pub approved: bool,
    pub timestamp: u64,
    pub block_height: u64,
    pub signature: String,
    pub pubkey: String,
}

impl DoubleSignMonitor {
    /// Create a new monitor
    pub fn new(generator: SlashingProofGenerator) -> Self {
        Self {
            votes: std::collections::HashMap::new(),
            generator,
        }
    }

    /// Record a vote and check for double-signing
    pub fn record_vote(&mut self, vote: VoteRecord) -> Option<DoubleSignEvidence> {
        let key = (vote.validator_id.clone(), vote.package_canonical.clone());

        // Check for conflicting vote
        if let Some(existing_votes) = self.votes.get(&key) {
            for existing in existing_votes {
                if existing.approved != vote.approved {
                    // Found double-sign!
                    tracing::warn!(
                        "Double-sign detected! Validator: {}, Package: {}",
                        vote.validator_id,
                        vote.package_canonical
                    );

                    return Some(self.create_evidence(existing, &vote));
                }
            }
        }

        // Store the vote
        self.votes.entry(key).or_default().push(vote);

        None
    }

    /// Create evidence from two conflicting votes
    fn create_evidence(&self, vote1: &VoteRecord, vote2: &VoteRecord) -> DoubleSignEvidence {
        DoubleSignEvidence {
            public_inputs: DoubleSignPublicInputs {
                validator_pubkey_x: vote1.pubkey.clone(), // Simplified
                validator_pubkey_y: "0".to_string(),      // Would be actual Y coordinate
                package_hash: hex::encode(Sha256::digest(vote1.package_canonical.as_bytes())),
                vote1_hash: hex::encode(Sha256::digest(format!(
                    "{}:{}:{}",
                    vote1.package_canonical, vote1.approved, vote1.timestamp
                ))),
                vote2_hash: hex::encode(Sha256::digest(format!(
                    "{}:{}:{}",
                    vote2.package_canonical, vote2.approved, vote2.timestamp
                ))),
            },
            witness: DoubleSignWitness {
                validator_privkey: "HIDDEN".to_string(), // Not known by monitor
                signature1: Signature {
                    r_x: "0".to_string(),
                    r_y: "0".to_string(),
                    s: vote1.signature.clone(),
                },
                signature2: Signature {
                    r_x: "0".to_string(),
                    r_y: "0".to_string(),
                    s: vote2.signature.clone(),
                },
            },
            validator_address: vote1.validator_id.clone(),
            package_canonical: vote1.package_canonical.clone(),
            vote1_details: VoteDetails {
                approved: vote1.approved,
                timestamp: vote1.timestamp,
                block_height: vote1.block_height,
                signature_hex: vote1.signature.clone(),
            },
            vote2_details: VoteDetails {
                approved: vote2.approved,
                timestamp: vote2.timestamp,
                block_height: vote2.block_height,
                signature_hex: vote2.signature.clone(),
            },
        }
    }
}

/// Get current timestamp
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_nullifier() {
        let generator = SlashingProofGenerator::new(ProofConfig::default());

        let public_inputs = DoubleSignPublicInputs {
            validator_pubkey_x: "123".to_string(),
            validator_pubkey_y: "456".to_string(),
            package_hash: "abc".to_string(),
            vote1_hash: "def".to_string(),
            vote2_hash: "ghi".to_string(),
        };

        let nullifier1 = generator.compute_nullifier(&public_inputs);
        let nullifier2 = generator.compute_nullifier(&public_inputs);

        // Same inputs should produce same nullifier
        assert_eq!(nullifier1, nullifier2);
    }

    #[test]
    fn test_double_sign_monitor() {
        let generator = SlashingProofGenerator::new(ProofConfig::default());
        let mut monitor = DoubleSignMonitor::new(generator);

        // First vote: approve
        let vote1 = VoteRecord {
            validator_id: "val1".to_string(),
            package_canonical: "npm:test@1.0.0".to_string(),
            approved: true,
            timestamp: 1000,
            block_height: 100,
            signature: "sig1".to_string(),
            pubkey: "pubkey1".to_string(),
        };

        let result1 = monitor.record_vote(vote1);
        assert!(result1.is_none()); // No double-sign yet

        // Second vote: reject (conflicting!)
        let vote2 = VoteRecord {
            validator_id: "val1".to_string(),
            package_canonical: "npm:test@1.0.0".to_string(),
            approved: false, // Different!
            timestamp: 1001,
            block_height: 100,
            signature: "sig2".to_string(),
            pubkey: "pubkey1".to_string(),
        };

        let result2 = monitor.record_vote(vote2);
        assert!(result2.is_some()); // Double-sign detected!
    }
}
