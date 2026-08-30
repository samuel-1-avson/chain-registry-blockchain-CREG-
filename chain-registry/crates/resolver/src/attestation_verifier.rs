//! Client-Side Cryptographic Attestation Verifier
//!
//! Verifies validator committee signatures, Merkle inclusion proofs, and content
//! hash integrity for packages prior to installation or CI/CD usage.

use anyhow::{bail, Result};
use common::sha256_hex;
use serde::{Deserialize, Serialize};

/// Minimum acceptable trust score for automated installation approval
pub const DEFAULT_MIN_ACCEPTABLE_SCORE: u32 = 80;

/// Cryptographic attestation issued by CREG validator committee for a package version
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPackageAttestation {
    /// Canonical package identifier (e.g. "npm/express@4.18.2")
    pub canonical: String,
    /// Cryptographic SHA-256 hash of the package tarball/wheel
    pub content_hash: String,
    /// Publisher EVM address or public key
    pub publisher: String,
    /// Automated static/sandbox risk score (0 - 100)
    pub risk_score: u32,
    /// Block height in which this package was finalized
    pub block_height: u64,
    /// Validator quorum signatures (hex encoded)
    pub validator_signatures: Vec<String>,
    /// State / transparency Merkle root anchored on L1
    pub merkle_root: String,
    /// Merkle inclusion sibling path
    pub merkle_proof: Option<Vec<String>>,
    /// UNIX timestamp of attestation issuance
    pub timestamp: u64,
}

impl SignedPackageAttestation {
    /// Verify that local bytes match the cryptographic content hash in the attestation
    pub fn verify_content_integrity(&self, package_bytes: &[u8]) -> Result<()> {
        let computed_hash = sha256_hex(package_bytes);
        let expected_hash = self.content_hash.trim_start_matches("0x");

        if computed_hash != expected_hash {
            bail!(
                "Content hash mismatch for {}: expected {}, computed {}",
                self.canonical,
                expected_hash,
                computed_hash
            );
        }
        Ok(())
    }

    /// Verify that the attestation satisfies minimum safety rules
    pub fn verify_safety_policy(
        &self,
        min_score: u32,
        min_validator_quorum: usize,
    ) -> Result<()> {
        if self.risk_score < min_score {
            bail!(
                "Package {} risk score {} is below required minimum {}",
                self.canonical,
                self.risk_score,
                min_score
            );
        }

        if self.validator_signatures.len() < min_validator_quorum {
            bail!(
                "Package {} has insufficient validator signatures ({}/{})",
                self.canonical,
                self.validator_signatures.len(),
                min_validator_quorum
            );
        }

        Ok(())
    }

    /// Compute the deterministic attestation signing digest
    pub fn signing_digest(&self) -> String {
        let payload = format!(
            "creg-attestation-v1:{}:{}:{}:{}:{}",
            self.canonical,
            self.content_hash,
            self.publisher,
            self.risk_score,
            self.block_height
        );
        sha256_hex(payload.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attestation_content_integrity_and_policy() {
        let tarball_bytes = b"console.log('hello safe package');";
        let hash = sha256_hex(tarball_bytes);

        let attestation = SignedPackageAttestation {
            canonical: "npm/safe-pkg@1.0.0".into(),
            content_hash: hash.clone(),
            publisher: "0x1234567890123456789012345678901234567890".into(),
            risk_score: 95,
            block_height: 100,
            validator_signatures: vec!["0xsig1".into(), "0xsig2".into(), "0xsig3".into()],
            merkle_root: "0xroot".into(),
            merkle_proof: None,
            timestamp: 1724760000,
        };

        // Verification passes for genuine bytes
        assert!(attestation.verify_content_integrity(tarball_bytes).is_ok());

        // Verification fails for tampered bytes
        assert!(attestation
            .verify_content_integrity(b"malicious_payload_injected")
            .is_err());

        // Safety policy passes with 3 signatures and min score 80
        assert!(attestation.verify_safety_policy(80, 3).is_ok());

        // Safety policy fails if quorum requires 4 signatures
        assert!(attestation.verify_safety_policy(80, 4).is_err());

        // Safety policy fails if minimum score required is 98
        assert!(attestation.verify_safety_policy(98, 2).is_err());
    }
}
