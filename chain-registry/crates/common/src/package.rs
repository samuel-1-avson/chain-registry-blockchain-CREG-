// crates/common/src/package.rs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FindingSeverity {
    /// Package must be rejected — direct evidence of malice.
    Critical,
    /// Strongly suspicious — requires human appeal to override.
    High,
    /// Notable but possibly legitimate — shown as warning.
    Medium,
    /// Informational only.
    #[default]
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: FindingSeverity,
    pub description: String,
    pub file: String,
    pub line: Option<usize>,
}

/// Uniquely identifies a package across all ecosystems.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct PackageId {
    /// Ecosystem: "npm" | "pypi" | "cargo" | "rubygems" | "maven"
    pub ecosystem: String,
    pub name: String,
    pub version: String,
}

impl PackageId {
    pub fn new(
        ecosystem: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            ecosystem: ecosystem.into(),
            name: name.into(),
            version: version.into(),
        }
    }

    /// Canonical string used as a cache key and chain identifier.
    pub fn canonical(&self) -> String {
        format!("{}:{}@{}", self.ecosystem, self.name, self.version)
    }
}

impl std::fmt::Display for PackageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.canonical())
    }
}

/// Declared package behaviors submitted alongside the tarball.
/// Validators check *against* this manifest rather than blanket policy —
/// an HTTP client that declares outbound HTTPS calls is legitimate.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackageManifest {
    /// Allowed outbound hosts, e.g. ["api.example.com"]
    pub allowed_network_hosts: Vec<String>,
    /// Allowed filesystem paths the package may write to.
    pub allowed_fs_writes: Vec<String>,
    /// Whether the package spawns child processes.
    pub spawns_processes: bool,
    /// Fine-grained process spawn allowlist — binary names or full paths.
    /// Only checked when `spawns_processes` is true.  If empty and
    /// `spawns_processes` is true, all spawns are permitted (backwards compat).
    #[serde(default)]
    pub allowed_process_spawns: Vec<String>,
    /// Free-text description for human reviewers.
    pub description: Option<String>,
}

/// Submitted by a publisher to place a package in the pending pool.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PublishRequest {
    pub id: PackageId,
    /// SHA-256 of the tarball bytes.
    pub content_hash: String,
    /// IPFS CID where the tarball is already pinned.
    pub ipfs_cid: String,
    /// Publisher's Ed25519 public key (hex-encoded).
    pub publisher_pubkey: String,
    /// Ed25519 signature over canonical(id) + content_hash.
    pub signature: String,
    pub manifest: PackageManifest,
    pub submitted_at: DateTime<Utc>,
    /// Whether the tarball is encrypted (AES-256-GCM).
    pub shielded: bool,
    /// Ephemeral symmetric key encrypted for the validator set (ECIES bundle).
    pub key_bundle: Option<String>,
    /// Optional detached PGP signature for the tarball.
    pub pgp_signature: Option<String>,
    /// Optional PGP public key for verification.
    pub pgp_public_key: Option<String>,
    /// Multi-sig: minimum signatures required (default 2).
    #[serde(default)]
    pub threshold: usize,
    /// Multi-sig: list of publisher pubkeys (2-of-3 support).
    #[serde(default)]
    pub publisher_pubkeys: Vec<String>,
    /// Multi-sig: signatures corresponding to `publisher_pubkeys`.
    #[serde(default)]
    pub signatures: Vec<String>,
}

/// A single entry in the on-chain package index.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChainRecord {
    pub id: PackageId,
    pub content_hash: String,
    pub ipfs_cid: String,
    pub publisher_pubkey: String,
    /// Hex block hash of the block that included this record.
    pub block_hash: String,
    pub published_at: DateTime<Utc>,
    /// Signatures from the N-of-M validators that approved this package.
    pub validator_signatures: Vec<ValidatorSignature>,
    pub status: PackageStatus,
    /// Whether this record represents a private (encrypted) package.
    pub shielded: bool,
    /// Encrypted key bundle required for decryption (available to authorized nodes).
    pub key_bundle: Option<String>,
    /// Verified PGP fingerprint (if any).
    pub pgp_fingerprint: Option<String>,
    /// Security validation findings (Sandbox, Static, Diff).
    pub findings: Vec<Finding>,
    /// Real-time access metrics (Kind Enhancement)
    pub access_count: u32,
    pub last_accessed: Option<DateTime<Utc>>,
    /// Multi-sig: minimum signatures required.
    #[serde(default)]
    pub threshold: usize,
    /// Multi-sig: list of publisher pubkeys.
    #[serde(default)]
    pub publisher_pubkeys: Vec<String>,
    /// Package manifest from the publisher (declared behaviors).
    /// Stored at finalization so that future versions can diff against it.
    #[serde(default)]
    pub manifest: Option<PackageManifest>,
}

/// Current lifecycle state of a package on the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PackageStatus {
    /// Accepted by consensus — safe to install.
    Verified,
    /// Submitted but not yet through consensus — pending pool only.
    #[default]
    Pending,
    /// Rejected by consensus or later found malicious.
    Revoked { reason: String },
}

/// A validator's signature over a package hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorSignature {
    pub validator_id: String,
    pub validator_pubkey: String,
    pub signature: String,
    pub vote: ValidatorVote,
    pub signed_at: DateTime<Utc>,
    /// ML model version used for deep scan (e.g., "codebert-v0.1.0" or "degraded-no-model").
    /// Allows consensus to verify validators used compatible model versions.
    #[serde(default)]
    pub ml_model_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ValidatorVote {
    #[default]
    Approve,
    Reject {
        reason: String,
    },
}
