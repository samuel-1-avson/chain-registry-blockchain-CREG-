// crates/node/src/openapi.rs
// OpenAPI spec for the node's public REST surface.
//
// Kept in its own module so that adding utoipa schema derives does not
// couple to the existing 1.7k-line api.rs handler soup. The schemas here
// mirror the shapes those handlers emit today; the explorer's TypeScript
// codegen reads the resulting /v1/openapi.json.
//
// When adding a new endpoint: define its response schema here, add a stub
// handler with `#[utoipa::path]` to the per-tag modules below, then list
// the path + schema in the `ApiDoc` struct.

use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme},
    Modify, OpenApi, ToSchema,
};

// ─── Shared response schemas ──────────────────────────────────────────────────

#[derive(serde::Serialize, ToSchema)]
pub struct ApiError {
    /// Human-readable error message. Intended for display.
    pub error: String,
}

#[derive(serde::Serialize, ToSchema)]
pub struct Health {
    /// Always "ok" when the node is responsive.
    #[schema(example = "ok")]
    pub status: String,
    /// Semver of the node binary (from Cargo.toml).
    #[schema(example = "0.1.0")]
    pub version: String,
}

#[derive(serde::Serialize, ToSchema)]
pub struct ChainStats {
    /// Current tip height.
    pub current_height: u64,
    /// Highest finalized height. Trails tip by the finality window.
    pub finalized_height: u64,
    /// Finalized tip hash, hex.
    pub finalized_hash: Option<String>,
    pub genesis_hash: Option<String>,
    /// Active validator count.
    pub validator_count: usize,
    /// Sum of validator stakes (native units).
    pub total_stake: u64,
    /// Connected P2P peer count.
    pub peer_count: usize,
    /// Bridge sync state. "Synced" | "Syncing" | "Unknown" | error strings.
    pub bridge_status: String,
    /// Last L1 block that the bridge read.
    pub l1_block: u64,
    pub package_count: Option<usize>,
    pub publisher_count: Option<usize>,
    pub pending_tx_count: Option<usize>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct RuntimeConfig {
    pub is_testnet: bool,
    /// Registry.sol contract address on the configured L1, if set.
    pub registry_address: Option<String>,
    pub token_contract: Option<String>,
    pub staking_contract: Option<String>,
    /// Machine-readable registration flow id (e.g. "staking-plus-identity-sync").
    pub validator_registration_mode: String,
    /// Operator-facing note describing the validator onboarding flow.
    pub validator_registration_note: String,
}

#[derive(serde::Serialize, ToSchema)]
pub struct ValidatorIdentityInfo {
    pub evm_address: String,
    pub node_id: String,
    pub ed25519_pubkey: String,
}

#[derive(serde::Serialize, ToSchema)]
pub struct ValidatorRegistration {
    pub alias: String,
    pub identity: ValidatorIdentityInfo,
    pub status: String,
    pub registered_with_node: bool,
    pub reputation: i64,
    pub stake: Option<String>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct BlockSummary {
    pub height: u64,
    pub hash: String,
    pub prev_hash: String,
    pub timestamp_ms: i64,
    pub producer: String,
    pub tx_count: usize,
    pub finalized: bool,
}

#[derive(serde::Serialize, ToSchema)]
pub struct BlockDetail {
    pub height: u64,
    pub hash: String,
    pub prev_hash: String,
    pub timestamp_ms: i64,
    pub producer: String,
    pub finalized: bool,
    pub signature: Option<String>,
    pub transactions: Vec<TransactionSummary>,
    /// Vote-producing validator addresses that approved this block.
    pub votes: Vec<String>,
    /// Quorum threshold this block was tested against.
    pub quorum: Option<usize>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct TransactionSummary {
    pub canonical: String,
    pub publisher: String,
    pub block_height: Option<u64>,
    pub status: String,
}

#[derive(serde::Serialize, ToSchema)]
pub struct TransactionDetail {
    pub canonical: String,
    pub version: String,
    pub publisher: String,
    pub block_height: Option<u64>,
    pub included_at: Option<String>,
    pub ipfs_cid: Option<String>,
    pub payload_hash: Option<String>,
    pub status: String,
    /// Arbitrary validation metadata. Shape stabilises once the pipeline
    /// stages are finalised — for now the explorer just surfaces key/value rows.
    #[schema(value_type = Object)]
    pub validation: serde_json::Value,
}

#[derive(serde::Serialize, ToSchema)]
pub struct PackageSummary {
    pub canonical: String,
    pub ecosystem: String,
    pub name: String,
    pub version: String,
    pub status: String,
    pub publisher: String,
    pub published_at: String,
    pub analysis_bundles: AnalysisBundleRefs,
    pub evidence_digest: String,
    pub deterministic_risk: DeterministicRiskSummary,
}

#[derive(serde::Serialize, ToSchema)]
pub struct AnalysisBundleRefs {
    pub policy_bundle_id: String,
    pub feature_schema_id: String,
    pub expert_bundle_id: String,
    pub embedding_model_id: String,
    pub index_epoch: String,
    pub threshold_profile_id: String,
    pub llm_prompt_profile_id: String,
    pub osv_snapshot_epoch: String,
}

#[derive(serde::Serialize, ToSchema)]
pub struct DeterministicRiskSummary {
    pub score: u8,
    pub deterministic_score: u8,
    pub advisory_score: u8,
    pub band: String,
    pub disposition: String,
    pub deterministic_findings: usize,
    pub advisory_findings: usize,
    pub critical_findings: usize,
    pub high_findings: usize,
    pub medium_findings: usize,
    pub low_findings: usize,
    pub reasons: Vec<String>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct PackageList {
    pub packages: Vec<PackageSummary>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

#[derive(serde::Serialize, ToSchema)]
pub struct PackageDetail {
    pub canonical: String,
    pub status: String,
    pub block_hash: Option<String>,
    pub content_hash: Option<String>,
    pub ipfs_cid: Option<String>,
    pub publisher: Option<String>,
    pub published_at: Option<String>,
    pub revocation_reason: Option<String>,
    pub analysis_bundles: Option<AnalysisBundleRefs>,
    pub evidence_digest: Option<String>,
    pub deterministic_risk: Option<DeterministicRiskSummary>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct Pending {
    pub canonical: String,
    pub publisher: String,
    pub received_at: String,
    pub stage: Option<String>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct PendingList {
    pub pending: Vec<Pending>,
    pub total: usize,
}

#[derive(serde::Serialize, ToSchema)]
pub struct NodeEntry {
    pub id: String,
    pub address: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct BridgeStatus {
    pub l1_chain_id: Option<u64>,
    pub bridge_contract: Option<String>,
    pub last_anchor_block: Option<u64>,
    pub last_anchor_root: Option<String>,
    pub signer_address: Option<String>,
    pub bridge_sync_status: String,
    pub last_finalized_eth_block: u64,
}

#[derive(serde::Serialize, ToSchema)]
pub struct ConsensusVote {
    pub validator_id: String,
    pub decision: String,
    pub reject_reason: Option<String>,
    pub ml_model_version: String,
    pub analysis_bundles: AnalysisBundleRefs,
    pub evidence_digest: String,
    pub deterministic_risk: DeterministicRiskSummary,
}

#[derive(serde::Serialize, ToSchema)]
pub struct ConsensusRound {
    pub consensus_subject: String,
    pub vote_count: usize,
    pub approvals: usize,
    pub rejections: usize,
    /// "collecting-votes" | "contested" | "quorum-reached".
    pub phase: String,
    pub voters: Vec<String>,
    pub approvers: Vec<String>,
    pub rejecters: Vec<String>,
    pub votes: Vec<ConsensusVote>,
    /// Milliseconds since the earliest vote in this round.
    pub age_ms: i64,
}

#[derive(serde::Serialize, ToSchema)]
pub struct ConsensusValidator {
    pub id: String,
    pub alias: String,
    pub stake: u64,
    pub reputation: u32,
    pub status: String,
}

#[derive(serde::Serialize, ToSchema)]
pub struct ConsensusState {
    pub total_validators: usize,
    pub quorum: usize,
    pub active_rounds: Vec<ConsensusRound>,
    pub pending_count: usize,
    pub validators: Vec<ConsensusValidator>,
}

/// Validator membership committed by a signed set-transition certificate.
#[derive(serde::Serialize, ToSchema)]
pub struct ValidatorSetMember {
    pub id: String,
    pub alias: String,
    /// Hex-encoded Ed25519 public key.
    pub pubkey: String,
    pub eth_address: String,
    pub stake: u64,
    pub reputation: u32,
    pub status: String,
    pub performance_score: u32,
    pub contribution_count: u32,
}

/// Canonical validator-set document carried by authenticated transition and
/// namespace-authority certificates.
#[derive(serde::Serialize, ToSchema)]
pub struct ValidatorSet {
    pub validators: Vec<ValidatorSetMember>,
}

/// One distinct member of the preceding validator set authorizing a transition.
#[derive(serde::Serialize, ToSchema)]
pub struct ValidatorSetTransitionSignature {
    pub validator_id: String,
    pub validator_pubkey: String,
    /// Hex-encoded Ed25519 signature over the domain-separated transition message.
    pub signature: String,
}

/// Quorum-certified validator-set transition. The receiving node independently
/// verifies the prior-set quorum, next-height continuity, and local tip anchor.
#[derive(serde::Serialize, ToSchema)]
pub struct ValidatorSetTransitionRequest {
    #[schema(example = 1)]
    pub schema_version: u16,
    pub effective_from_height: u64,
    /// Canonical hash of the final block governed by the preceding set.
    pub previous_block_hash: String,
    pub previous_validator_set_hash: String,
    pub validators: ValidatorSet,
    pub validator_set_hash: String,
    pub transition_signatures: Vec<ValidatorSetTransitionSignature>,
}

#[derive(serde::Serialize, ToSchema)]
pub struct ValidatorSetTransitionAccepted {
    pub accepted: bool,
    pub effective_from_height: u64,
    pub validator_set_hash: String,
}

/// Upstream authority selected by a signed chain-spec policy.
#[derive(serde::Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceOwnershipAuthority {
    /// npm trusted-publisher OIDC identity and npm provenance evidence.
    NpmTrustedPublisher,
    /// PyPI trusted-publisher OIDC identity and PyPI attestation evidence.
    PypiTrustedPublisher,
}

/// State transition authorized by an upstream namespace-authority certificate.
#[derive(serde::Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceAuthorityPurpose {
    Claim,
    Recover,
}

/// Fresh controller intent embedded in the registry-published artifact.
///
/// `next_controller_signature` is an Ed25519 signature over the
/// domain-separated intent. Current-controller fields may be omitted for a
/// first claim, but are mandatory for recovery.
#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceAuthorityChallenge {
    #[schema(example = 1)]
    pub schema_version: u16,
    pub purpose: NamespaceAuthorityPurpose,
    /// `creg-claim.json` for a claim or `creg-recovery.json` for recovery.
    #[schema(example = "creg-claim.json")]
    pub challenge_document: String,
    pub chain_id: String,
    /// Lowercase SHA-256 genesis hash.
    pub genesis_hash: String,
    /// Canonical `<ecosystem>:<name>@` namespace.
    #[schema(example = "npm:example-package@")]
    pub namespace: String,
    /// Required for recovery; empty or omitted for a first claim.
    #[serde(default)]
    pub current_controller_ed25519_pubkey: String,
    /// Required for recovery; empty or omitted for a first claim.
    #[serde(default)]
    pub current_controller_evm_address: String,
    /// Lowercase hex Ed25519 public key of the proposed controller.
    pub next_controller_ed25519_pubkey: String,
    /// Canonical 0x-prefixed EVM address of the proposed controller.
    pub next_controller_evm_address: String,
    /// Non-zero, purpose-bound replay-protection nonce.
    pub nonce: u64,
    /// Whole Unix seconds.
    pub issued_at: i64,
    /// Whole Unix seconds; the challenge lifetime is bounded by node policy.
    pub expires_at: i64,
    /// Lowercase hex Ed25519 signature by the proposed controller.
    pub next_controller_signature: String,
}

/// Exact upstream proof independently verified by a validator.
#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceAuthorityProof {
    #[schema(example = 2)]
    pub schema_version: u16,
    pub authority: NamespaceOwnershipAuthority,
    pub namespace: String,
    pub issuer: String,
    pub subject: String,
    pub audience: String,
    /// Immutable content-addressed archive location.
    pub evidence_uri: String,
    /// Official npm/PyPI URL from which the exact evidence bytes were fetched.
    pub registry_evidence_uri: String,
    pub evidence_sha256: String,
    /// SHA-256 of the artifact-independent controller intent.
    pub challenge_sha256: String,
    /// `sha512` for npm or `sha256` for PyPI.
    pub artifact_digest_algorithm: String,
    pub artifact_digest: String,
    pub controller_evm_address: String,
    pub controller_ed25519_pubkey: String,
    /// RFC 3339 UTC timestamp from the verified upstream proof.
    pub issued_at: String,
    /// RFC 3339 UTC timestamp from the verified upstream proof.
    pub expires_at: String,
}

/// Deterministic, chain-bound statement signed independently by validators.
#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceAuthorityStatement {
    #[schema(example = 2)]
    pub schema_version: u16,
    pub chain_id: String,
    pub genesis_hash: String,
    pub purpose: NamespaceAuthorityPurpose,
    pub namespace: String,
    pub current_controller_ed25519_pubkey: String,
    pub current_controller_evm_address: String,
    pub next_controller_ed25519_pubkey: String,
    pub next_controller_evm_address: String,
    pub authority_nonce: u64,
    pub authority_intent_digest: String,
    pub authority_proof: NamespaceAuthorityProof,
    pub adapter_id: String,
    pub adapter_version: String,
    /// SHA-256 of the exact verifier executable selected by signed policy.
    pub verifier_sha256: String,
    pub policy_sha256: String,
    pub trust_root_sha256: String,
    pub anchor_height: u64,
    pub anchor_block_hash: String,
    pub validator_set_hash: String,
    /// Whole Unix seconds.
    pub issued_at: i64,
    /// Whole Unix seconds.
    pub expires_at: i64,
}

/// One active validator's Ed25519 signature over a namespace-authority
/// statement.
#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceAuthoritySignature {
    pub validator_id: String,
    pub validator_pubkey: String,
    pub signature: String,
}

/// Request for one validator to independently fetch and verify upstream
/// namespace-authority evidence.
#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceAuthorityAttestationRequest {
    pub adapter_id: String,
    pub adapter_version: String,
    pub authority: NamespaceOwnershipAuthority,
    /// Official HTTPS npm/PyPI evidence URL. Redirects are rejected.
    pub evidence_url: String,
    /// Official immutable registry artifact URL. The node separately fetches
    /// bounded bytes without redirects; the verifier must bind their native
    /// digest to the provenance and find the exact fresh challenge inside.
    pub artifact_url: String,
    pub expected_issuer: String,
    pub expected_subject: String,
    pub challenge: NamespaceAuthorityChallenge,
    /// Recent canonical block authenticated by the validator set below.
    pub anchor_height: u64,
    pub anchor_block_hash: String,
    pub validator_set_hash: String,
}

/// One independently produced validator attestation. Callers aggregate
/// matching statements until the validator set's BFT quorum is reached.
#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceAuthorityAttestationResponse {
    pub statement: NamespaceAuthorityStatement,
    pub validators: ValidatorSet,
    pub signature: NamespaceAuthoritySignature,
}

/// Quorum-certified namespace authority. The included validator set is replay
/// material and is not a self-authenticating trust root.
#[derive(serde::Serialize, ToSchema)]
pub struct CertifiedNamespaceAuthority {
    pub statement: NamespaceAuthorityStatement,
    pub validators: ValidatorSet,
    pub signatures: Vec<NamespaceAuthoritySignature>,
}

/// Successful validation of an aggregated namespace-authority certificate.
#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceAuthorityCertificateResponse {
    pub certificate_digest: String,
    pub certificate: CertifiedNamespaceAuthority,
}

/// Namespace-controller state transition selected by the request body.
#[derive(serde::Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceLifecycleAction {
    Delegate,
    Transfer,
    Recover,
}

/// Legacy namespace lifecycle event. This format has no chain/genesis or EVM
/// controller binding and is rejected for new submissions after the configured
/// namespace-authority activation height. Legacy recovery is always rejected.
#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceLifecycleEventV1 {
    #[schema(example = 1)]
    pub schema_version: u16,
    pub action: NamespaceLifecycleAction,
    /// Canonical `<ecosystem>:<name>@` namespace.
    pub namespace: String,
    pub current_controller_ed25519_pubkey: String,
    pub next_controller_ed25519_pubkey: String,
    pub nonce: u64,
    /// RFC 3339 UTC timestamp.
    pub issued_at: String,
    /// RFC 3339 UTC timestamp.
    pub expires_at: String,
    pub current_controller_signature: String,
    pub next_controller_signature: String,
    /// Empty for delegate/transfer. A digest does not make v1 recovery
    /// admissible.
    #[serde(default)]
    pub authority_proof_digest: String,
}

/// Chain-bound namespace lifecycle event used at and after CR-07 activation.
///
/// Delegate and transfer require signatures from both current and next
/// controllers. Recovery deliberately leaves `current_controller_signature`
/// empty and instead requires the next-controller signature plus a recent,
/// purpose-bound validator-quorum authority certificate.
#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceLifecycleEventV2 {
    #[schema(example = 2)]
    pub schema_version: u16,
    pub chain_id: String,
    pub genesis_hash: String,
    pub action: NamespaceLifecycleAction,
    /// Canonical `<ecosystem>:<name>@` namespace.
    pub namespace: String,
    pub current_controller_ed25519_pubkey: String,
    pub current_controller_evm_address: String,
    pub next_controller_ed25519_pubkey: String,
    pub next_controller_evm_address: String,
    /// Exactly `["publish"]` for delegation; empty otherwise.
    #[serde(default)]
    pub delegation_scopes: Vec<String>,
    /// Whole Unix seconds after `issued_at` for delegation; absent otherwise.
    #[serde(default)]
    pub delegation_expires_at: Option<i64>,
    /// Monotonically increasing controller nonce.
    pub nonce: u64,
    /// Whole Unix seconds.
    pub issued_at: i64,
    /// Whole Unix seconds.
    pub expires_at: i64,
    /// Empty only for recovery.
    pub current_controller_signature: String,
    pub next_controller_signature: String,
    /// Required for recovery and equal to the included proof's digest; empty
    /// for delegate/transfer.
    #[serde(default)]
    pub authority_proof_digest: String,
    /// Required only for recovery.
    #[serde(default)]
    pub authority_certificate: Option<CertifiedNamespaceAuthority>,
}

/// Untagged lifecycle request accepted by the generic and action-specific
/// routes. `schema_version` and the v2 chain-bound fields distinguish the two
/// wire formats; no wrapper object or variant tag is present in JSON.
#[derive(serde::Serialize, ToSchema)]
#[serde(untagged)]
pub enum NamespaceLifecycleSubmission {
    V2(NamespaceLifecycleEventV2),
    V1(NamespaceLifecycleEventV1),
}

#[derive(serde::Serialize, ToSchema)]
pub struct NamespaceLifecycleQueuedResponse {
    #[schema(example = "queued")]
    pub status: String,
    #[schema(example = "Namespace lifecycle event will be included in the next block")]
    pub message: String,
}

#[derive(serde::Serialize, ToSchema)]
pub struct AddressProfile {
    pub address: String,
    pub is_validator: bool,
    pub is_active_validator: bool,
    pub validator: Option<ValidatorRegistration>,
    /// Status from the active validator set when applicable ("online" | "self" | "offline").
    pub active_status: Option<String>,
    pub stake: Option<String>,
    pub reputation: Option<u32>,
    /// Blocks proposed by this address within `scanned_blocks`.
    pub blocks_proposed: u32,
    /// Txs referencing this address within `scanned_blocks`.
    pub tx_count: u32,
    /// How many most-recent blocks were inspected to build this profile.
    pub scanned_blocks: u64,
}

#[derive(serde::Serialize, ToSchema)]
pub struct AddressTxRef {
    pub block_height: u64,
    pub block_hash: String,
    pub tx_index: usize,
    /// One of: "publish" | "revoke" | "slash" | "validator-join" | "validator-leave" | "rotate-key" | "propose".
    pub kind: String,
    pub canonical: Option<String>,
    pub timestamp: String,
}

#[derive(serde::Serialize, ToSchema)]
pub struct AddressTxList {
    pub address: String,
    pub transactions: Vec<AddressTxRef>,
    pub scanned_blocks: u64,
    pub total: usize,
}

#[derive(serde::Serialize, ToSchema)]
pub struct ValidatorProfile {
    pub address: String,
    pub registration: Option<ValidatorRegistration>,
    pub in_active_set: bool,
    pub stake: String,
    pub reputation: u32,
    pub status: String,
    /// Blocks proposed by this validator within the recent window (newest first).
    pub recent_proposals: Vec<BlockSummary>,
}

// ─── Path stubs ───────────────────────────────────────────────────────────────
//
// utoipa introspects these stub functions for the OpenAPI metadata. They are
// never called — axum wires the real handlers in `api.rs`. The pairing is
// maintained by convention: the stub's `path = "..."` must match the router.

pub mod paths {
    use super::*;

    /// Health probe. Returns 200 with build info when the node is live.
    #[utoipa::path(
        get,
        path = "/v1/health",
        tag = "system",
        responses((status = 200, body = Health, description = "Node is alive")),
    )]
    pub async fn health() {}

    /// Chain stats — tip height, finality, validator and peer counts.
    #[utoipa::path(
        get,
        path = "/v1/chain/stats",
        tag = "system",
        responses((status = 200, body = ChainStats)),
    )]
    pub async fn chain_stats() {}

    /// Runtime configuration — testnet flag, contract addresses, validator flow mode.
    #[utoipa::path(
        get,
        path = "/v1/runtime/config",
        tag = "system",
        responses((status = 200, body = RuntimeConfig)),
    )]
    pub async fn runtime_config() {}

    /// Paginated block list. Supports offset/limit and `before_height` / `after_height` cursors.
    ///
    /// - `before_height=H`: return blocks with height < H, newest first.
    /// - `after_height=H`: return blocks with height > H, newest first.
    /// - Omit cursors and use `offset`/`limit` for classic pagination.
    #[utoipa::path(
        get,
        path = "/v1/blocks",
        tag = "blocks",
        params(
            ("limit" = Option<usize>, Query, description = "Max blocks to return (default 20, max 100)"),
            ("offset" = Option<usize>, Query, description = "Offset into the list (ignored when cursors are set)"),
            ("before_height" = Option<u64>, Query, description = "Return blocks below this height"),
            ("after_height" = Option<u64>, Query, description = "Return blocks above this height"),
        ),
        responses((status = 200, body = [BlockSummary])),
    )]
    pub async fn list_blocks() {}

    /// Fetch a block by height.
    #[utoipa::path(
        get,
        path = "/v1/blocks/{height}",
        tag = "blocks",
        params(("height" = u64, Path, description = "Block height")),
        responses(
            (status = 200, body = BlockDetail),
            (status = 404, body = ApiError),
        ),
    )]
    pub async fn get_block_by_height() {}

    /// Fetch a block by its 0x-prefixed hash.
    #[utoipa::path(
        get,
        path = "/v1/blocks/hash/{hash}",
        tag = "blocks",
        params(("hash" = String, Path, description = "0x-prefixed block hash")),
        responses(
            (status = 200, body = BlockDetail),
            (status = 404, body = ApiError),
        ),
    )]
    pub async fn get_block_by_hash() {}

    /// Fetch a transaction by its canonical id (`name@version`) or tx hash.
    #[utoipa::path(
        get,
        path = "/v1/transactions/{canonical}",
        tag = "transactions",
        params(("canonical" = String, Path, description = "Package canonical or tx hash")),
        responses(
            (status = 200, body = TransactionDetail),
            (status = 404, body = ApiError),
        ),
    )]
    pub async fn get_transaction() {}

    /// Paginated package list.
    #[utoipa::path(
        get,
        path = "/v1/packages",
        tag = "packages",
        params(
            ("limit" = Option<usize>, Query, description = "Max packages to return (default 50, max 200)"),
            ("offset" = Option<usize>, Query, description = "Offset into the result set (default 0)"),
            ("ecosystem" = Option<String>, Query, description = "Filter by ecosystem (npm, pypi, …)"),
            ("status" = Option<String>, Query, description = "Filter by status (verified | pending | revoked)"),
        ),
        responses((status = 200, body = PackageList)),
    )]
    pub async fn list_packages() {}

    /// Fetch a package by canonical id (`<ecosystem>:<name>@<version>`).
    #[utoipa::path(
        get,
        path = "/v1/packages/{canonical}",
        tag = "packages",
        params(("canonical" = String, Path, description = "Package canonical id (`<ecosystem>:<name>@<version>`)")),
        responses(
            (status = 200, body = PackageDetail),
            (status = 404, body = ApiError),
        ),
    )]
    pub async fn get_package() {}

    /// Pending pool contents — not yet finalized txs.
    #[utoipa::path(
        get,
        path = "/v1/pending",
        tag = "transactions",
        responses((status = 200, body = PendingList)),
    )]
    pub async fn list_pending() {}

    /// Validator set as understood by this node (includes "self" marker).
    #[utoipa::path(
        get,
        path = "/v1/nodes",
        tag = "network",
        responses((status = 200, body = [NodeEntry])),
    )]
    pub async fn get_nodes() {}

    /// Bridge health + last L1 anchor commit.
    #[utoipa::path(
        get,
        path = "/v1/bridge/status",
        tag = "bridge",
        responses((status = 200, body = BridgeStatus)),
    )]
    pub async fn bridge_status() {}

    /// Validator identity registrations known to this node.
    #[utoipa::path(
        get,
        path = "/v1/validators/registrations",
        tag = "validators",
        responses((status = 200, body = [ValidatorRegistration])),
    )]
    pub async fn list_validator_registrations() {}

    /// Live PBFT round state — in-flight proposals with quorum progress.
    #[utoipa::path(
        get,
        path = "/v1/consensus/state",
        tag = "consensus",
        responses((status = 200, body = ConsensusState)),
    )]
    pub async fn consensus_state() {}

    /// Aggregate profile for an EVM address: validator identity (if any) plus
    /// recent on-chain activity counts.
    #[utoipa::path(
        get,
        path = "/v1/addresses/{address}",
        tag = "addresses",
        params(("address" = String, Path, description = "EVM address (0x…)")),
        responses(
            (status = 200, body = AddressProfile),
            (status = 400, body = ApiError),
        ),
    )]
    pub async fn get_address() {}

    /// Transactions touching an address, scanned from the most recent blocks.
    #[utoipa::path(
        get,
        path = "/v1/addresses/{address}/transactions",
        tag = "addresses",
        params(
            ("address" = String, Path, description = "EVM address"),
            ("limit" = Option<usize>, Query, description = "Max txs to return (default 50, max 500)"),
            ("scan" = Option<u64>, Query, description = "Max blocks to scan backwards (default 500, max 5000)"),
        ),
        responses(
            (status = 200, body = AddressTxList),
            (status = 400, body = ApiError),
        ),
    )]
    pub async fn get_address_transactions() {}

    /// Validator profile — registration status, active-set info, and recent block proposals.
    #[utoipa::path(
        get,
        path = "/v1/validators/{address}",
        tag = "validators",
        params(("address" = String, Path, description = "Validator EVM address")),
        responses(
            (status = 200, body = ValidatorProfile),
            (status = 404, body = ApiError),
        ),
    )]
    pub async fn get_validator_profile() {}

    /// Register the caller's validator identity (EVM address ↔ ed25519 pubkey ↔ node id).
    /// Requires an Ethereum personal-sign signature from the EVM address and
    /// an Ed25519 signature from the validator key over the same binding message.
    #[utoipa::path(
        post,
        path = "/v1/validators/register",
        tag = "validators",
        request_body(content = Object, description = "alias, evm_address, node_id, ed25519_pubkey, nonce, evm_signature, ed25519_signature"),
        responses(
            (status = 200, body = ValidatorRegistration),
            (status = 400, body = ApiError),
        ),
    )]
    pub async fn register_validator_identity() {}

    /// Fetch the complete persisted quorum-certified validator-set transition
    /// history in ascending effective-height order. Rebuilding nodes treat
    /// this response as untrusted input: every signature, authority link,
    /// configured weak-subjectivity checkpoint, and canonical block anchor is
    /// independently verified before any certificate is persisted.
    #[utoipa::path(
        get,
        path = "/v1/validator/set-transitions",
        tag = "validators",
        responses(
            (status = 200, body = [ValidatorSetTransitionRequest]),
        ),
    )]
    pub async fn list_validator_set_transitions() {}

    /// Submit a signed validator-set transition for independent validation,
    /// durable persistence, and propagation on
    /// `creg/v1/validator-set-transitions`. Authorization is the Ed25519 quorum
    /// certificate from the preceding set; an individual operator credential
    /// cannot replace it.
    #[utoipa::path(
        post,
        path = "/v1/validator/set-transitions",
        tag = "validators",
        request_body = ValidatorSetTransitionRequest,
        responses(
            (status = 202, body = ValidatorSetTransitionAccepted),
            (status = 401, body = ApiError, description = "Invalid or insufficient prior-set certificate"),
            (status = 409, body = ApiError, description = "Stale height, replay/checkpoint conflict, or local fork-anchor mismatch"),
            (status = 413, description = "Transition body exceeds the 1 MiB endpoint limit"),
            (status = 500, body = ApiError, description = "Local validator-history read, retention, serialization, or durable persistence failure"),
            (status = 503, body = ApiError, description = "Configured weak-subjectivity checkpoint is malformed"),
        ),
    )]
    pub async fn submit_validator_set_transition() {}

    /// Ask this validator to independently attest an npm/PyPI namespace claim
    /// or recovery.
    ///
    /// The proposed controller's Ed25519 signature authenticates the intent,
    /// while an operator API key protects this expensive validator-local
    /// service from anonymous resource exhaustion. Neither credential can
    /// substitute for the validator quorum ultimately required. The node
    /// fetches the exact evidence bytes from the official
    /// registry origins, runs the hash-pinned verifier selected by the signed
    /// chain spec over both the evidence and artifact bytes, binds the result
    /// to a recent canonical block and its authenticated validator set,
    /// archives both inputs by digest, and returns this validator's signature.
    /// Callers must obtain matching attestations from a BFT quorum before
    /// constructing a certificate.
    ///
    /// During rollout, requests that do not match exactly one signed policy
    /// are rejected. A matched policy without its matching hash-pinned
    /// verifier binary fails closed with 503 rather than issuing an
    /// under-specified attestation.
    #[utoipa::path(
        post,
        path = "/v1/validator/namespace-authority/attest",
        tag = "validators",
        request_body = NamespaceAuthorityAttestationRequest,
        responses(
            (status = 200, body = NamespaceAuthorityAttestationResponse, description = "Independent validator attestation"),
            (status = 400, body = ApiError, description = "Invalid controller challenge, authority evidence, adapter result, or canonical anchor"),
            (status = 401, body = ApiError, description = "Operator API credentials are missing or invalid"),
            (status = 403, body = ApiError, description = "Node is not a validator or is absent from the anchor validator set"),
            (status = 429, body = ApiError, description = "All bounded authority-verifier slots are busy"),
            (status = 500, body = ApiError, description = "Local signing or evidence-archive failure"),
            (status = 503, body = ApiError, description = "Signed policy has no usable hash-pinned verifier configuration"),
        ),
        security(("OperatorKey" = []), ("OperatorBearer" = [])),
    )]
    pub async fn attest_namespace_authority() {}

    /// Validate an aggregated namespace-authority quorum certificate against
    /// this node's canonical chain state.
    ///
    /// An operator API key protects the endpoint, but the certificate's
    /// validator signatures remain the consensus authorization and the API
    /// credential cannot replace quorum. The carried validator set is checked
    /// against the set authenticated at
    /// the recent canonical anchor, along with chain/genesis, purpose,
    /// controller intent, nonce, validity interval, adapter, verifier, policy,
    /// and trust-root bindings. This endpoint validates and returns the
    /// canonical digest; it does not collect attestations for the caller.
    #[utoipa::path(
        post,
        path = "/v1/validator/namespace-authority/certificates",
        tag = "validators",
        request_body = CertifiedNamespaceAuthority,
        responses(
            (status = 200, body = NamespaceAuthorityCertificateResponse, description = "Certificate is valid for the node's next block context"),
            (status = 400, body = ApiError, description = "Invalid quorum, expiry, policy mismatch, or stale/conflicting canonical anchor"),
            (status = 401, body = ApiError, description = "Operator API credentials are missing or invalid"),
            (status = 503, body = ApiError, description = "Private API is locked or the signed authority policy is unavailable"),
        ),
        security(("OperatorKey" = []), ("OperatorBearer" = [])),
    )]
    pub async fn assemble_namespace_authority_certificate() {}

    /// Submit a signed namespace delegation, transfer, or recovery event.
    ///
    /// The JSON body is an untagged v1 or v2 event. V1 delegate/transfer
    /// remains available only before the configured CR-07 activation height;
    /// v1 is rejected after activation and v1 recovery is never accepted. V2
    /// binds the event to chain id, genesis, both Ed25519/EVM controller
    /// identities, a durable monotonic nonce, and whole-second validity.
    /// Delegate/transfer require both controller signatures. Recovery requires
    /// a v2 event signed by the next controller and an independently verified,
    /// recent canonical-anchor validator-quorum authority certificate.
    #[utoipa::path(
        post,
        path = "/v1/publisher/namespaces/lifecycle",
        tag = "publishers",
        request_body = NamespaceLifecycleSubmission,
        responses(
            (status = 200, body = NamespaceLifecycleQueuedResponse, description = "Event durably queued for block inclusion"),
            (status = 400, body = ApiError, description = "Malformed event, invalid signatures/validity, or invalid v2 action-specific fields"),
            (status = 403, body = ApiError, description = "Declared current controller does not own the namespace"),
            (status = 404, body = ApiError, description = "Namespace has no established controller"),
            (status = 409, body = ApiError, description = "Retired v1 event, forbidden v1 recovery, replay/stale nonce, or failed chain-state preflight"),
            (status = 500, body = ApiError, description = "Durable queue persistence failure"),
        ),
    )]
    pub async fn submit_namespace_lifecycle() {}

    /// Submit a delegation event through the action-specific route.
    ///
    /// The body must set `action` to `delegate`. V2 delegation permits only
    /// the time-bounded `publish` scope, does not replace the root controller,
    /// and must not carry authority proof/certificate fields. V1 is accepted
    /// only before CR-07 activation.
    #[utoipa::path(
        post,
        path = "/v1/publisher/namespaces/delegate",
        tag = "publishers",
        request_body = NamespaceLifecycleSubmission,
        responses(
            (status = 200, body = NamespaceLifecycleQueuedResponse, description = "Delegation durably queued for block inclusion"),
            (status = 400, body = ApiError, description = "Action mismatch, malformed delegation, or invalid controller signatures"),
            (status = 403, body = ApiError, description = "Declared current controller does not own the namespace"),
            (status = 404, body = ApiError, description = "Namespace has no established controller"),
            (status = 409, body = ApiError, description = "Retired v1 event, replay/stale nonce, or failed chain-state preflight"),
            (status = 500, body = ApiError, description = "Durable queue persistence failure"),
        ),
    )]
    pub async fn submit_namespace_delegate() {}

    /// Submit a root-controller transfer through the action-specific route.
    ///
    /// The body must set `action` to `transfer`. Current and next controllers
    /// sign the same domain-separated event. V2 transfer must not carry
    /// delegation or authority data. V1 is accepted only before CR-07
    /// activation.
    #[utoipa::path(
        post,
        path = "/v1/publisher/namespaces/transfer",
        tag = "publishers",
        request_body = NamespaceLifecycleSubmission,
        responses(
            (status = 200, body = NamespaceLifecycleQueuedResponse, description = "Transfer durably queued for block inclusion"),
            (status = 400, body = ApiError, description = "Action mismatch, malformed transfer, or invalid controller signatures"),
            (status = 403, body = ApiError, description = "Declared current controller does not own the namespace"),
            (status = 404, body = ApiError, description = "Namespace has no established controller"),
            (status = 409, body = ApiError, description = "Retired v1 event, replay/stale nonce, or failed chain-state preflight"),
            (status = 500, body = ApiError, description = "Durable queue persistence failure"),
        ),
    )]
    pub async fn submit_namespace_transfer() {}

    /// Submit an authority-certified recovery through the action-specific
    /// route.
    ///
    /// The body must set `action` to `recover` and must use v2. The successor
    /// signs the event; the old controller signature is empty. The included
    /// certificate must bind the same namespace, current/next Ed25519 and EVM
    /// controllers, nonce, validity, proof digest, chain/genesis, signed
    /// adapter policy, recent canonical block anchor, and validator-set quorum.
    #[utoipa::path(
        post,
        path = "/v1/publisher/namespaces/recover",
        tag = "publishers",
        request_body = NamespaceLifecycleSubmission,
        responses(
            (status = 200, body = NamespaceLifecycleQueuedResponse, description = "Recovery durably queued for block inclusion"),
            (status = 400, body = ApiError, description = "Action mismatch, malformed recovery, invalid successor signature, or missing/mismatched certificate"),
            (status = 403, body = ApiError, description = "Declared current controller conflicts with namespace state"),
            (status = 404, body = ApiError, description = "Namespace has no established controller"),
            (status = 409, body = ApiError, description = "V1 recovery, replay/stale nonce, invalid quorum/policy, or stale/conflicting canonical anchor"),
            (status = 500, body = ApiError, description = "Durable queue persistence failure"),
        ),
    )]
    pub async fn submit_namespace_recovery() {}

    /// P2P topology — peer list, role, and connection health.
    #[utoipa::path(
        get,
        path = "/v1/p2p/status",
        tag = "network",
        responses((status = 200, body = Object)),
    )]
    pub async fn p2p_status() {}

    /// Recent L1 anchor commits — chain-id, L1 block, Merkle root, L1 tx hash.
    #[utoipa::path(
        get,
        path = "/v1/bridge/anchors",
        tag = "bridge",
        params(("limit" = Option<usize>, Query, description = "Max anchors (default 50, max 500)")),
        responses((status = 200, body = [Object])),
    )]
    pub async fn bridge_anchors() {}

    /// Governance proposal list mirrored from Governance.sol via L1 eth_call.
    #[utoipa::path(
        get,
        path = "/v1/governance/proposals",
        tag = "governance",
        params(("limit" = Option<usize>, Query, description = "Max proposals (default 50, max 100)")),
        responses(
            (status = 200, description = "Proposal list, possibly empty when L1 is unconfigured", body = Object),
            (status = 503, description = "L1 RPC unavailable"),
        ),
    )]
    pub async fn governance_proposals() {}

    /// Submit a Governance.sol proposal as the configured CREG_GOVERNANCE_KEY signer.
    #[utoipa::path(
        post,
        path = "/v1/operator/governance/proposals",
        tag = "governance",
        security(("OperatorKey" = []), ("OperatorBearer" = [])),
        request_body(content = Object, description = "target address, hex calldata, description"),
        responses(
            (status = 200, description = "L1 submit transaction accepted", body = Object),
            (status = 400, description = "Invalid proposal payload or L1 revert", body = ApiError),
            (status = 401, description = "Missing or invalid operator credentials", body = ApiError),
            (status = 403, description = "CREG_GOVERNANCE_KEY is not a contract signer", body = ApiError),
            (status = 503, description = "Operator ACL, signer key, or L1 RPC unavailable", body = ApiError),
        ),
    )]
    pub async fn governance_submit() {}

    /// Vote on a pending Governance.sol proposal as CREG_GOVERNANCE_KEY.
    #[utoipa::path(
        post,
        path = "/v1/operator/governance/proposals/{id}/vote",
        tag = "governance",
        security(("OperatorKey" = []), ("OperatorBearer" = [])),
        params(("id" = u64, Path, description = "Proposal id")),
        request_body(content = Object, description = "{ \"approve\": true | false }"),
        responses(
            (status = 200, description = "L1 vote transaction accepted", body = Object),
            (status = 401, description = "Missing or invalid operator credentials", body = ApiError),
            (status = 403, description = "CREG_GOVERNANCE_KEY is not a contract signer", body = ApiError),
            (status = 404, description = "Proposal does not exist", body = ApiError),
            (status = 409, description = "Proposal already executed or cancelled", body = ApiError),
            (status = 503, description = "Operator ACL, signer key, or L1 RPC unavailable", body = ApiError),
        ),
    )]
    pub async fn governance_vote() {}

    /// Approval vote that triggers Governance.sol auto-execute at threshold.
    #[utoipa::path(
        post,
        path = "/v1/operator/governance/proposals/{id}/execute",
        tag = "governance",
        security(("OperatorKey" = []), ("OperatorBearer" = [])),
        params(("id" = u64, Path, description = "Proposal id")),
        responses(
            (status = 200, description = "L1 approval vote accepted; execution is automatic at threshold", body = Object),
            (status = 401, description = "Missing or invalid operator credentials", body = ApiError),
            (status = 403, description = "CREG_GOVERNANCE_KEY is not a contract signer", body = ApiError),
            (status = 404, description = "Proposal does not exist", body = ApiError),
            (status = 409, description = "Proposal already executed or cancelled", body = ApiError),
            (status = 503, description = "Operator ACL, signer key, or L1 RPC unavailable", body = ApiError),
        ),
    )]
    pub async fn governance_execute() {}

    /// Durable Prometheus time-series filtered to this node's signed network identity.
    #[utoipa::path(
        get,
        path = "/v1/public/metrics/history",
        tag = "system",
        params(("range" = Option<String>, Query, description = "Lookback window: 5m, 30m, 1h, 6h, 24h, or 7d (default 1h)")),
        responses(
            (status = 200, description = "Stored samples returned from the real Prometheus collector", body = Object),
            (status = 400, description = "Unsupported history range", body = ApiError),
            (status = 503, description = "Collector unavailable or history is warming", body = ApiError),
        ),
    )]
    pub async fn metrics_history() {}

    /// Detected reorgs — height, old/new tip, depth, timestamp.
    #[utoipa::path(
        get,
        path = "/v1/reorgs",
        tag = "system",
        responses((status = 200, body = [Object])),
    )]
    pub async fn reorgs() {}

    /// Top addresses by staked / native balance.
    #[utoipa::path(
        get,
        path = "/v1/richlist",
        tag = "addresses",
        params(("limit" = Option<usize>, Query, description = "Max entries (default 50, max 500)")),
        responses((status = 200, body = [Object])),
    )]
    pub async fn richlist() {}

    /// Submit a new package for inclusion — enters the pending pool and the validator pipeline.
    #[utoipa::path(
        post,
        path = "/v1/packages",
        tag = "packages",
        request_body(content = Object, description = "PublishRequest: canonical, ipfs_cid, payload_hash, publisher, signature"),
        responses(
            (status = 200, body = Object),
            (status = 400, body = ApiError),
        ),
    )]
    pub async fn submit_package() {}

    /// Revoke an existing package (operator / publisher only).
    #[utoipa::path(
        post,
        path = "/v1/packages/{canonical}/revoke",
        tag = "packages",
        params(("canonical" = String, Path, description = "Package canonical id")),
        request_body(content = Object, description = "{ reason: String, signature: String }"),
        responses(
            (status = 200, body = Object),
            (status = 400, body = ApiError),
        ),
        security(("OperatorKey" = [])),
    )]
    pub async fn revoke_package() {}

    /// Merkle proof for a package, verifiable against the current chain tip header.
    #[utoipa::path(
        get,
        path = "/v1/packages/{canonical}/proof",
        tag = "packages",
        params(("canonical" = String, Path, description = "Package canonical id")),
        responses(
            (status = 200, body = Object),
            (status = 404, body = ApiError),
        ),
    )]
    pub async fn get_proof() {}

    /// Gossip endpoint — accept a freshly-signed block from a peer.
    #[utoipa::path(
        post,
        path = "/v1/blocks/announce",
        tag = "blocks",
        request_body(content = Object, description = "Signed block payload"),
        responses(
            (status = 200, body = Object),
            (status = 400, body = ApiError),
        ),
    )]
    pub async fn receive_block_announcement() {}

    /// Publisher profile — package count, reputation, key rotation history.
    #[utoipa::path(
        get,
        path = "/v1/publishers/{pubkey}",
        tag = "addresses",
        params(("pubkey" = String, Path, description = "Publisher public key or canonical id")),
        responses(
            (status = 200, body = Object),
            (status = 404, body = ApiError),
        ),
    )]
    pub async fn get_publisher() {}

    /// Consensus vote intake — PBFT PREPARE/COMMIT vote from a validator.
    #[utoipa::path(
        post,
        path = "/v1/consensus/vote",
        tag = "consensus",
        request_body(content = Object, description = "SignedVote: consensus_subject, phase, voter, signature"),
        responses(
            (status = 200, body = Object),
            (status = 400, body = ApiError),
        ),
    )]
    pub async fn receive_vote() {}

    /// Rotate a publisher signing key — requires a signed rotation request from the old key.
    #[utoipa::path(
        post,
        path = "/v1/publishers/rotate-key",
        tag = "addresses",
        request_body(content = Object, description = "{ old_pubkey, new_pubkey, signature }"),
        responses(
            (status = 200, body = Object),
            (status = 400, body = ApiError),
        ),
    )]
    pub async fn rotate_publisher_key() {}

    /// Global smart search — classifies `q` as block height / hash / EVM address /
    /// package canonical / publisher and returns candidate matches.
    #[utoipa::path(
        get,
        path = "/v1/search",
        tag = "system",
        params(("q" = String, Query, description = "Query string (height, hash, address, canonical, or prefix)")),
        responses((status = 200, body = Object)),
    )]
    pub async fn search_handler() {}

    /// Submit an audit decision against a pending appeal (operator only).
    #[utoipa::path(
        post,
        path = "/v1/appeals/{id}/audit",
        tag = "system",
        params(("id" = String, Path, description = "Appeal id")),
        request_body(content = Object, description = "{ verdict, notes, signature }"),
        responses(
            (status = 200, body = Object),
            (status = 400, body = ApiError),
        ),
        security(("OperatorKey" = [])),
    )]
    pub async fn submit_audit() {}
}

// ─── Security schemes ─────────────────────────────────────────────────────────
//
// Most routes are public; operator endpoints gate on the `X-Operator-Key`
// header. Declared here so it's visible in Swagger UI.

struct SecurityAddon;
impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi
            .components
            .get_or_insert_with(utoipa::openapi::Components::default);
        components.add_security_scheme(
            "OperatorKey",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-Operator-Key"))),
        );
        components.add_security_scheme(
            "OperatorBearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

// ─── Root spec ────────────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Chain Registry Node API",
        version = "0.1.0",
        description = "Public REST surface of a Chain Registry node. Consumed by the web explorer, TUI explorer, and CLI.",
        license(name = "Apache-2.0"),
    ),
    servers(
        (url = "/", description = "Current host"),
    ),
    tags(
        (name = "system",       description = "Health, runtime configuration, search, metrics, reorgs, appeals"),
        (name = "blocks",       description = "Block list, lookup by height or hash, gossip intake"),
        (name = "transactions", description = "Transaction lookup and pending pool"),
        (name = "packages",     description = "Package publish records, proofs, revocation"),
        (name = "publishers",   description = "Publisher identity and authoritative namespace lifecycle"),
        (name = "validators",   description = "Validator identity registrations and profiles"),
        (name = "consensus",    description = "PBFT round state and vote intake"),
        (name = "network",      description = "Peer set and P2P topology"),
        (name = "bridge",       description = "L1 bridge anchor status and commit feed"),
        (name = "addresses",    description = "Per-address profiles, activity, publishers, rich list"),
        (name = "governance",   description = "On-chain governance proposals and votes"),
    ),
    paths(
        paths::health,
        paths::chain_stats,
        paths::runtime_config,
        paths::list_blocks,
        paths::get_block_by_height,
        paths::get_block_by_hash,
        paths::get_transaction,
        paths::list_packages,
        paths::get_package,
        paths::list_pending,
        paths::get_nodes,
        paths::bridge_status,
        paths::list_validator_registrations,
        paths::consensus_state,
        paths::get_address,
        paths::get_address_transactions,
        paths::get_validator_profile,
        paths::register_validator_identity,
        paths::list_validator_set_transitions,
        paths::submit_validator_set_transition,
        paths::attest_namespace_authority,
        paths::assemble_namespace_authority_certificate,
        paths::submit_namespace_lifecycle,
        paths::submit_namespace_delegate,
        paths::submit_namespace_transfer,
        paths::submit_namespace_recovery,
        paths::p2p_status,
        paths::bridge_anchors,
        paths::governance_proposals,
        paths::governance_submit,
        paths::governance_vote,
        paths::governance_execute,
        paths::metrics_history,
        paths::reorgs,
        paths::richlist,
        paths::submit_package,
        paths::revoke_package,
        paths::get_proof,
        paths::receive_block_announcement,
        paths::get_publisher,
        paths::receive_vote,
        paths::rotate_publisher_key,
        paths::search_handler,
        paths::submit_audit,
    ),
    components(schemas(
        ApiError,
        Health,
        ChainStats,
        RuntimeConfig,
        ValidatorIdentityInfo,
        ValidatorRegistration,
        BlockSummary,
        BlockDetail,
        TransactionSummary,
        TransactionDetail,
        PackageSummary,
        AnalysisBundleRefs,
        DeterministicRiskSummary,
        PackageList,
        PackageDetail,
        Pending,
        PendingList,
        NodeEntry,
        BridgeStatus,
        ConsensusVote,
        ConsensusRound,
        ConsensusValidator,
        ConsensusState,
        ValidatorSetMember,
        ValidatorSet,
        ValidatorSetTransitionSignature,
        ValidatorSetTransitionRequest,
        ValidatorSetTransitionAccepted,
        NamespaceOwnershipAuthority,
        NamespaceAuthorityPurpose,
        NamespaceAuthorityChallenge,
        NamespaceAuthorityProof,
        NamespaceAuthorityStatement,
        NamespaceAuthoritySignature,
        NamespaceAuthorityAttestationRequest,
        NamespaceAuthorityAttestationResponse,
        CertifiedNamespaceAuthority,
        NamespaceAuthorityCertificateResponse,
        NamespaceLifecycleAction,
        NamespaceLifecycleEventV1,
        NamespaceLifecycleEventV2,
        NamespaceLifecycleSubmission,
        NamespaceLifecycleQueuedResponse,
        AddressProfile,
        AddressTxRef,
        AddressTxList,
        ValidatorProfile,
    )),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;
