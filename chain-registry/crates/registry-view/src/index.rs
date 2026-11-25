//! Read models served by the standalone PostgreSQL indexer.

use super::{CommitteeSummaryV1, DisplayText, RevocationAuthorityV1};
use common::CregFindingCategoryV1;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineQueueV1 {
    pub server_time_unix: i64,
    pub entries: Vec<QuarantineQueueEntryV1>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineQueueEntryV1 {
    pub canonical: DisplayText,
    pub content_hash: String,
    pub reason_code: CregFindingCategoryV1,
    pub opened_height: u64,
    pub opened_at_unix: i64,
    pub expires_at_unix: i64,
    pub committee: CommitteeSummaryV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEvidenceHistoryV1 {
    pub canonical: DisplayText,
    pub entries: Vec<IndexEvidenceEntryV1>,
    pub next_before: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEvidenceEntryV1 {
    pub height: u64,
    /// -1 denotes a deterministic expiry that precedes the block's transactions.
    pub tx_index: i32,
    pub source_tx_type: String,
    pub content_hash: Option<String>,
    pub status: Option<IndexPackageStatusV1>,
    pub reason_code: Option<CregFindingCategoryV1>,
    pub reason: Option<DisplayText>,
    pub quarantine_expires_at_unix: Option<i64>,
    pub committee: Option<CommitteeSummaryV1>,
    pub revocation_authority: Option<RevocationAuthorityV1>,
    pub evidence_hash: Option<String>,
    pub witness_digest: Option<String>,
    pub certificate_digest: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexPackageStatusV1 {
    Pending,
    Verified,
    Quarantined,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitteeDiversityV1 {
    /// First height of the contiguous validator-set run, with genesis epoch 0.
    pub epoch: u64,
    pub validator_set_hash: String,
    pub observations: u64,
    pub approvals: u64,
    pub quarantine_reviews: u64,
    pub rejections: u64,
    pub abstentions: u64,
    pub hybrid_signers: u64,
    pub classical_signers: u64,
    pub model_families: Vec<ModelFamilyCountV1>,
    pub model_families_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFamilyCountV1 {
    pub model_family: DisplayText,
    pub votes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BountyLeaderboardV1 {
    pub entries: Vec<BountyProverV1>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BountyProverV1 {
    pub l1_chain_id: u64,
    pub prover_address: String,
    pub awards: u64,
    pub matched_witnesses: u64,
    /// Decimal wei strings preserve precision in browser clients.
    pub prover_bonus_wei: String,
    pub committee_reserved_wei: String,
    pub committee_distributed_wei: String,
    pub latest_award: BountyAwardV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BountyAwardV1 {
    pub l1_block_number: u64,
    pub l1_block_hash: String,
    pub l1_tx_hash: String,
    pub log_index: u64,
    pub witness_digest: String,
    pub package_hash: String,
    pub canonical: Option<DisplayText>,
    pub witness_height: Option<u64>,
}
