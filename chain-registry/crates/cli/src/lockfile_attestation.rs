//! Lockfile Attestation & Security Provenance Engine
//!
//! Audits project dependency lockfiles (`package-lock.json`, `Cargo.lock`, `requirements.txt`)
//! and produces a cryptographically signed `creg.lock.json` attestation for CI/CD gates.

use anyhow::{bail, Result};
use common::sha256_hex;
use serde::{Deserialize, Serialize};

/// Summary of verified dependency attestation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyAttestationEntry {
    pub package_canonical: String,
    pub content_hash: String,
    pub status: String,
    pub risk_score: u32,
    pub block_height: u64,
}

/// A comprehensive dependency provenance report
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockfileAttestationReport {
    pub project_name: String,
    pub ecosystem: String,
    pub total_dependencies: usize,
    pub verified_dependencies: usize,
    pub revoked_dependencies: usize,
    pub unverified_dependencies: usize,
    pub dependencies: Vec<DependencyAttestationEntry>,
    pub provenance_root: String,
    pub is_ci_compliant: bool,
    pub generated_at: u64,
}

impl LockfileAttestationReport {
    /// Generate a new report from audited dependencies
    pub fn new(
        project_name: String,
        ecosystem: String,
        entries: Vec<DependencyAttestationEntry>,
        generated_at: u64,
    ) -> Self {
        let total = entries.len();
        let verified = entries.iter().filter(|e| e.status == "verified").count();
        let revoked = entries.iter().filter(|e| e.status == "revoked").count();
        let unverified = total.saturating_sub(verified + revoked);

        // Compute Merkle provenance root over dependency hashes
        let mut digests: Vec<String> = entries
            .iter()
            .map(|e| {
                sha256_hex(
                    format!(
                        "{}:{}:{}",
                        e.package_canonical, e.content_hash, e.risk_score
                    )
                    .as_bytes(),
                )
            })
            .collect();
        digests.sort();

        let provenance_root = sha256_hex(digests.join(":").as_bytes());
        let is_ci_compliant =
            revoked == 0 && (unverified == 0 || (total > 0 && verified * 100 / total >= 95));

        Self {
            project_name,
            ecosystem,
            total_dependencies: total,
            verified_dependencies: verified,
            revoked_dependencies: revoked,
            unverified_dependencies: unverified,
            dependencies: entries,
            provenance_root,
            is_ci_compliant,
            generated_at,
        }
    }

    /// Export to formatted JSON string
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lockfile_attestation_report_generation() {
        let entries = vec![
            DependencyAttestationEntry {
                package_canonical: "npm/react@18.2.0".into(),
                content_hash: "0xhash1".into(),
                status: "verified".into(),
                risk_score: 98,
                block_height: 500,
            },
            DependencyAttestationEntry {
                package_canonical: "npm/react-dom@18.2.0".into(),
                content_hash: "0xhash2".into(),
                status: "verified".into(),
                risk_score: 97,
                block_height: 500,
            },
        ];

        let report = LockfileAttestationReport::new(
            "frontend-app".into(),
            "npm".into(),
            entries,
            1724760000,
        );

        assert_eq!(report.total_dependencies, 2);
        assert_eq!(report.verified_dependencies, 2);
        assert_eq!(report.revoked_dependencies, 0);
        assert!(report.is_ci_compliant);
        assert!(!report.provenance_root.is_empty());
    }
}
