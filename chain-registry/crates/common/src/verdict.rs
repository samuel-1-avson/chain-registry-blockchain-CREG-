// crates/common/src/verdict.rs

use crate::PackageId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The trust decision the CLI uses to allow, warn, or block an install.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustVerdict {
    pub package: PackageId,
    pub status: VerdictStatus,
    /// ISO-8601 timestamp when the verdict was determined.
    pub resolved_at: DateTime<Utc>,
    /// Whether this verdict came from the local cache or a live chain query.
    pub source: VerdictSource,
    /// MAL-004/LLM-002: risk band, deterministic vs advisory (LLM) finding
    /// split from the finalized record. None for gRPC fast path, legacy
    /// cache entries, and nodes that predate the field.
    #[serde(default)]
    pub deterministic_risk: Option<crate::DeterministicRiskSummary>,
    /// Evidence vote stage from the node API (`none`, `voting`, `quorum-reached`, `rejected`).
    #[serde(default)]
    pub evidence_phase: Option<String>,
    /// Block production stage (`none`, `queued`, `proposed`, `committed`).
    #[serde(default)]
    pub block_phase: Option<String>,
    /// Chain record stage (`pending`, `verified`, `rejected`).
    #[serde(default)]
    pub chain_status: Option<String>,
    /// Publisher Ed25519 pubkey from the chain record, when the node returned one.
    /// gRPC success overlays REST to fill this; still missing on REST failure
    /// and on legacy cache entries.
    #[serde(default)]
    pub publisher: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerdictStatus {
    /// Legacy chain has a consensus record. This is not a guarantee that the
    /// package is safe and must not bypass client freshness or revocation checks.
    Verified {
        block_hash: String,
        content_hash: String,
        /// IPFS CID of the verified tarball — used for P2P download.
        ipfs_cid: String,
        findings: Vec<crate::Finding>,
    },
    /// Package exists but is not yet through consensus — warn the user.
    Unverified,
    /// Package was explicitly revoked — hard block.
    Revoked {
        reason: String,
        findings: Vec<crate::Finding>,
    },
    /// Held out of `Verified` for Class C committee review (ADR-018) — hard
    /// block with **no** break-glass override (ECA-07, ADR-019 open decision
    /// #2). Fails closed into `Revoked` once `expires_at_unix` passes, the same
    /// rule consensus applies in `resolve_quarantine_timeout`.
    Quarantined {
        reason: String,
        /// Content hash the hold applies to, when the node reported one.
        #[serde(default)]
        content_hash: String,
        /// Unix seconds at which the hold fails closed into revocation.
        /// `None` when the node gave no expiry; the hold is then indefinite.
        #[serde(default)]
        expires_at_unix: Option<i64>,
    },
    /// Package is entirely unknown to the chain.
    Unknown,
}

impl VerdictStatus {
    /// Compatibility helper for legacy callers. `true` means only that the
    /// legacy record is verified; it is not a package-safety guarantee.
    pub fn is_safe(&self) -> bool {
        matches!(self, VerdictStatus::Verified { .. })
    }

    /// `true` for statuses that no flag or environment variable can install.
    pub fn is_blocked(&self) -> bool {
        matches!(
            self,
            VerdictStatus::Revoked { .. } | VerdictStatus::Quarantined { .. }
        )
    }

    /// `true` when this is a quarantine whose hold has run out, i.e. it must
    /// now be treated exactly like a revocation.
    pub fn quarantine_expired(&self, now_unix: i64) -> bool {
        matches!(
            self,
            VerdictStatus::Quarantined {
                expires_at_unix: Some(expires_at),
                ..
            } if now_unix >= *expires_at
        )
    }

    /// Human-readable label for terminal output.
    pub fn label(&self) -> &'static str {
        match self {
            VerdictStatus::Verified { .. } => "VERIFIED",
            VerdictStatus::Unverified => "UNVERIFIED",
            VerdictStatus::Revoked { .. } => "REVOKED",
            VerdictStatus::Quarantined { .. } => "QUARANTINED",
            VerdictStatus::Unknown => "UNKNOWN",
        }
    }

    /// ANSI colour code for the label (used in the CLI terminal output).
    pub fn ansi_color(&self) -> &'static str {
        match self {
            VerdictStatus::Verified { .. } => "\x1b[32m", // green
            VerdictStatus::Unverified => "\x1b[33m",      // yellow
            VerdictStatus::Revoked { .. } => "\x1b[31m",  // red
            VerdictStatus::Quarantined { .. } => "\x1b[35m", // magenta
            VerdictStatus::Unknown => "\x1b[33m",         // yellow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quarantined(expires_at_unix: Option<i64>) -> VerdictStatus {
        VerdictStatus::Quarantined {
            reason: "committee hold".into(),
            content_hash: "abc".into(),
            expires_at_unix,
        }
    }

    #[test]
    fn quarantine_is_blocked_and_never_safe() {
        let status = quarantined(Some(100));
        assert!(status.is_blocked());
        assert!(!status.is_safe());
        assert_eq!(status.label(), "QUARANTINED");
    }

    #[test]
    fn quarantine_expiry_is_inclusive_and_needs_a_deadline() {
        assert!(!quarantined(Some(100)).quarantine_expired(99));
        assert!(quarantined(Some(100)).quarantine_expired(100));
        assert!(!quarantined(None).quarantine_expired(i64::MAX));
        assert!(!VerdictStatus::Unknown.quarantine_expired(i64::MAX));
    }

    #[test]
    fn legacy_cache_entries_still_deserialize() {
        let legacy = r#"{"Revoked":{"reason":"x","findings":[]}}"#;
        let status: VerdictStatus = serde_json::from_str(legacy).unwrap();
        assert!(status.is_blocked());
        let minimal = r#"{"Quarantined":{"reason":"hold"}}"#;
        let status: VerdictStatus = serde_json::from_str(minimal).unwrap();
        assert_eq!(status, quarantined_minimal());
    }

    fn quarantined_minimal() -> VerdictStatus {
        VerdictStatus::Quarantined {
            reason: "hold".into(),
            content_hash: String::new(),
            expires_at_unix: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerdictSource {
    /// Read from the local TTL cache — fast path.
    Cache { expires_at: DateTime<Utc> },
    /// Fetched live from a chain node.
    Chain { node_url: String },
}
