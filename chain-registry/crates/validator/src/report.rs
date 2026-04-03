// crates/validator/src/report.rs

use crate::sandbox::SandboxResult;
use crate::static_analysis::StaticAnalysisResult;
pub use common::{Finding, FindingSeverity, PackageId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditProof {
    /// Cryptographic signature from the authorized AI model (hex).
    pub signature: String,
    /// The public key of the AI auditor that produced this verdict.
    pub auditor_pubkey: String,
    /// Detailed rationales for the verdict.
    pub rationales: Vec<Rationale>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rationale {
    pub finding_id: String,
    pub logic: String,
    pub confidence: u8,
}

pub struct ValidationReport {
    pub package: PackageId,
    pub findings: Vec<Finding>,
    pub aaa_verdict: Option<AuditProof>,
}

impl ValidationReport {
    pub fn new(package: PackageId) -> Self {
        Self {
            package,
            findings: Vec::new(),
            aaa_verdict: None,
        }
    }

    pub fn apply_static(&mut self, result: StaticAnalysisResult) {
        self.findings.extend(result.findings);
    }

    pub fn apply_sandbox(&mut self, result: SandboxResult) {
        self.findings.extend(result.findings);
    }

    pub fn apply_diff(&mut self, result: crate::diff::DiffResult) {
        self.findings.extend(result.findings);
    }

    pub fn apply_pgp(&mut self, result: crate::pgp::PgpResult) {
        self.findings.extend(result.findings);
    }

    /// True if any Critical or High findings were recorded.
    pub fn has_critical_findings(&self) -> bool {
        self.findings.iter().any(|f| {
            matches!(
                f.severity,
                FindingSeverity::Critical | FindingSeverity::High
            )
        })
    }

    /// Concise summary of all critical/high findings for the rejection reason.
    pub fn critical_finding_summary(&self) -> String {
        self.findings
            .iter()
            .filter(|f| {
                matches!(
                    f.severity,
                    FindingSeverity::Critical | FindingSeverity::High
                )
            })
            .map(|f| format!("[{}] {}", f.id, f.description))
            .collect::<Vec<_>>()
            .join("; ")
    }
}
