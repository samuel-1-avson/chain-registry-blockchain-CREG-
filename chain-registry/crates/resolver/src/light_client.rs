// crates/resolver/src/light_client.rs
// Proof-carrying light-client verification for the CLI resolver (ADR-019 §4.1).
//
// Authority derives exclusively from a binary-pinned, signed ChainSpec and
// quorum-certified validator-set transitions. Every security-relevant check
// calls the consensus crate's verification functions directly (ECO-INV-03).

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use common::crypto::PublicKey;
use common::status_tree::{hex32, split_canonical};
use common::{
    sha256_hex, AuthenticatedValidatorSetSnapshot, Block, BlockHeader, BlockSignature, ChainRecord,
    ChainSpec, ConsensusParams, CregFindingCategoryV1, DisplayText, PackageStatus,
    QuarantineOutcomeV1, StatusLeafStateV1, Transaction, TriggerWitnessV1, TrustVerdict, Validator,
    ValidatorSet, VerdictStatus,
};
use registry_view::{
    PackageStatusViewV1, ProofBundleV2, QuorumLabelV1, RevocationAuthorityV1, MAX_REASON_CHARS,
    PROOF_BUNDLE_SCHEMA,
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Maximum age accepted for a proven tip block (1 hour, ADR-019 Stage 4).
pub const DEFAULT_MAX_TIP_AGE_SECS: i64 = 3_600;
/// Maximum forward clock skew tolerated on a proven tip timestamp.
pub const DEFAULT_MAX_TIP_FUTURE_SKEW_SECS: i64 = 300;
/// Environment toggle matching the Stage 5 `--require-v4-quorum` option.
pub const REQUIRE_V4_QUORUM_ENV: &str = "CREG_REQUIRE_V4_QUORUM";

/// Runtime verification knobs for proof-carrying resolution (ADR-019 §4.1,
/// decision #7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyOptions {
    pub max_tip_age_secs: i64,
    pub max_tip_future_skew_secs: i64,
    /// When true, reject `legacy_v3_quorum` proofs even below hybrid activation.
    pub require_v4_quorum: bool,
}

impl Default for VerifyOptions {
    fn default() -> Self {
        Self {
            max_tip_age_secs: DEFAULT_MAX_TIP_AGE_SECS,
            max_tip_future_skew_secs: DEFAULT_MAX_TIP_FUTURE_SKEW_SECS,
            require_v4_quorum: false,
        }
    }
}

impl VerifyOptions {
    /// Load options using defaults and `CREG_REQUIRE_V4_QUORUM`.
    pub fn from_env() -> Self {
        Self {
            require_v4_quorum: crate::install_gate::env_flag(REQUIRE_V4_QUORUM_ENV),
            ..Self::default()
        }
    }
}

// ── Checkpoint ────────────────────────────────────────────────────────────────

/// A known-good (height, hash) pair hardcoded or loaded from config.
/// Light clients verify that the fetched chain reaches back to this checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub height: u64,
    pub hash: String,
}

impl Checkpoint {
    /// Genesis block hash for default-profile unit fixtures. Production callers
    /// derive checkpoints from the signed chain spec via [`Self::from_verified_spec`].
    pub fn genesis() -> Self {
        Self {
            height: 0,
            hash: Block::genesis().hash(),
        }
    }

    /// Derive the block-zero checkpoint from an already verified chain spec.
    pub fn from_verified_spec(spec: &ChainSpec) -> Result<Self> {
        Ok(Self {
            height: 0,
            hash: spec.canonical_genesis_block_for_client()?.hash(),
        })
    }
}

/// Load the trust root independently of a node's proof response. The binary
/// pins the signer. Environment configuration selects the signed network.
pub async fn load_verified_chain_spec() -> Result<ChainSpec> {
    let pinned = option_env!("CREG_SPEC_SIGNING_PUBKEY")
        .filter(|key| !key.trim().is_empty())
        .context("resolver has no binary-pinned CREG_SPEC_SIGNING_PUBKEY")?;
    let (raw, signature) = if let Ok(path) = std::env::var("CREG_CHAIN_SPEC_PATH") {
        let signature_path =
            std::env::var("CREG_CHAIN_SPEC_SIG_PATH").unwrap_or_else(|_| format!("{path}.sig"));
        (
            std::fs::read_to_string(&path).context("read configured chain spec")?,
            std::fs::read_to_string(signature_path).context("read chain-spec signature")?,
        )
    } else {
        let url = std::env::var("CREG_CHAIN_SPEC_URL")
            .context("proof verification requires CREG_CHAIN_SPEC_URL or CREG_CHAIN_SPEC_PATH")?;
        let spec_url = reqwest::Url::parse(&url)?;
        if spec_url.scheme() != "https" {
            bail!("chain-spec URL must use HTTPS");
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let raw = client
            .get(spec_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let spec: ChainSpec = serde_json::from_str(&raw)?;
        let signature_url = reqwest::Url::parse(&spec.signing.detached_signature_url)?;
        if signature_url.scheme() != "https" {
            bail!("chain-spec signature URL must use HTTPS");
        }
        let signature = client
            .get(signature_url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        (raw, signature)
    };
    let spec: ChainSpec = serde_json::from_str(&raw)?;
    if !spec
        .signing
        .signing_key_pubkey_hex
        .trim_start_matches("0x")
        .eq_ignore_ascii_case(pinned.trim_start_matches("0x"))
    {
        bail!("chain-spec signer differs from the binary pin");
    }
    spec.verify_signature(signature.trim(), pinned)?;
    spec.validate_consensus_identity_for_client()?;
    if let Ok(expected) = std::env::var("CREG_CHAIN_ID") {
        if spec.chain_id != expected {
            bail!("signed chain ID differs from configured pin");
        }
    }
    if let Ok(expected) = std::env::var("CREG_GENESIS_HASH") {
        if spec.genesis_hash.trim_start_matches("0x") != expected.trim_start_matches("0x") {
            bail!("signed genesis differs from configured pin");
        }
    }
    Ok(spec)
}

/// Derive the default block-zero checkpoint from the verified signed chain spec.
pub async fn default_checkpoint() -> Result<Checkpoint> {
    Checkpoint::from_verified_spec(&load_verified_chain_spec().await?)
}

// ── Merkle inclusion proof ────────────────────────────────────────────────────

/// A Merkle inclusion proof: a path of sibling hashes from the transaction
/// leaf up to the Merkle root stored in the block header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    /// Hash of the transaction being proven.
    pub tx_hash: String,
    /// Sibling hashes from leaf to root, with side indicators.
    pub path: Vec<MerkleStep>,
    /// The Merkle root the path should produce (must match block header).
    pub expected_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleStep {
    pub sibling_hash: String,
    pub is_right: bool, // true = sibling is on the right side
}

impl MerkleProof {
    /// Verify the proof: walk the path and check the computed root
    /// matches `expected_root`.
    pub fn verify(&self) -> bool {
        let mut current = self.tx_hash.clone();

        for step in &self.path {
            current = if step.is_right {
                sha256_hex(format!("{}{}", current, step.sibling_hash).as_bytes())
            } else {
                sha256_hex(format!("{}{}", step.sibling_hash, current).as_bytes())
            };
        }

        current == self.expected_root
    }
}

/// Canonical SHA-256 commitment over `validators` matching node header hashes.
pub fn compute_validator_set_hash(validators: &[Validator]) -> String {
    common::canonical_validator_set_hash(&ValidatorSet::new(validators.to_vec()))
}

/// Verify that `block_header` carries a PBFT commit quorum using
/// [`consensus::vote_v4::verify_pbft_phase_signature_at_height`], tolerating
/// individual malformed/bad signatures while counting distinct active signers.
pub fn verify_pbft_consensus_with_params(
    block_hash: &str,
    block_header: &BlockHeader,
    active_validators: &[Validator],
    pbft_signatures: &[BlockSignature],
    params: &ConsensusParams,
) -> Result<usize> {
    let computed_hash = compute_validator_set_hash(active_validators);
    if computed_hash != block_header.validator_set_hash {
        bail!(
            "Validator set hash mismatch: header has {}, computed {}",
            block_header.validator_set_hash,
            computed_hash
        );
    }

    common::validate_validator_identities(&ValidatorSet::new(active_validators.to_vec()))
        .map_err(anyhow::Error::msg)?;
    let active_by_id: HashMap<&str, &Validator> = active_validators
        .iter()
        .filter(|v| common::validator_status_is_active(&v.status))
        .map(|v| (v.id.as_str(), v))
        .collect();
    let n = active_by_id.len();
    if n == 0 {
        bail!("Active validator set is empty");
    }
    if params.consensus_encoding.is_some() && n < 4 {
        bail!("Phase C requires at least four active validators");
    }

    let quorum = (2 * n / 3) + 1;
    let mut seen_validators = HashSet::new();

    for sig in pbft_signatures {
        if seen_validators.contains(sig.validator_id.as_str()) {
            continue;
        }
        let Some(validator) = active_by_id.get(sig.validator_id.as_str()).copied() else {
            continue;
        };
        if consensus::vote_v4::verify_pbft_phase_signature_at_height(
            "commit",
            block_hash,
            block_header.height,
            params,
            validator,
            sig,
        )
        .is_err()
        {
            continue;
        }
        seen_validators.insert(validator.id.as_str());
    }

    let approvals = seen_validators.len();
    if approvals < quorum {
        bail!(
            "Insufficient PBFT consensus signatures: got {} valid, need {} (quorum for validator set size {})",
            approvals,
            quorum,
            n
        );
    }

    Ok(approvals)
}

/// Pre-activation wrapper for [`verify_pbft_consensus_with_params`].
pub fn verify_pbft_consensus(
    block_hash: &str,
    block_header: &BlockHeader,
    active_validators: &[Validator],
    pbft_signatures: &[BlockSignature],
) -> Result<()> {
    let pre_activation = ConsensusParams {
        consensus_encoding: None,
        block_time_seconds: 5,
        vote_timeout_ms: 120_000,
        quorum_percentage: 67,
        min_validator_stake_wei: "0".into(),
        min_publisher_stake_wei: "0".into(),
        unbonding_period_seconds: 0,
        slash_penalty_low_bp: 0,
        slash_penalty_medium_bp: 0,
        slash_penalty_critical_bp: 0,
        max_validators: u32::try_from(active_validators.len().max(1)).unwrap_or(u32::MAX),
        namespace_authority_activation_height: None,
        namespace_authority_policies: vec![],
        hybrid_signature_activation_height: None,
        vote_v4_quorum: None,
    };
    verify_pbft_consensus_with_params(
        block_hash,
        block_header,
        active_validators,
        pbft_signatures,
        &pre_activation,
    )
    .map(|_| ())
}

// ── Block header verification ─────────────────────────────────────────────────

fn hash_prefix(value: &str) -> &str {
    &value[..value.len().min(12)]
}

/// Verify that a block header is self-consistent:
/// - The hash of the header bytes equals the claimed block hash.
/// - The height equals `expected_height`.
pub fn verify_header(
    header: &BlockHeader,
    expected_hash: &str,
    expected_height: u64,
) -> Result<()> {
    let computed = Block {
        header: header.clone(),
        transactions: Vec::new(),
        pbft_signatures: Vec::new(),
    }
    .hash();

    if computed != expected_hash {
        bail!(
            "Block header hash mismatch at height {}: expected {} got {}",
            expected_height,
            hash_prefix(expected_hash),
            hash_prefix(&computed)
        );
    }

    if header.height != expected_height {
        bail!(
            "Block height mismatch: expected {} got {}",
            expected_height,
            header.height
        );
    }

    Ok(())
}

// ── Chain verification ────────────────────────────────────────────────────────

/// Verify a chain of headers from `checkpoint` up to `tip`.
/// Returns an error if any link in the chain is broken.
pub fn verify_header_chain(
    headers: &[(BlockHeader, String)], // (header, hash) pairs in ascending height order
    checkpoint: &Checkpoint,
) -> Result<()> {
    if headers.is_empty() {
        return Ok(());
    }

    let (first_header, first_hash) = &headers[0];
    verify_header(first_header, first_hash, checkpoint.height + 1)?;
    if first_header.prev_hash != checkpoint.hash {
        bail!(
            "Chain break at height {}: expected prev={} got={}",
            first_header.height,
            hash_prefix(&checkpoint.hash),
            hash_prefix(&first_header.prev_hash)
        );
    }

    for window in headers.windows(2) {
        let (prev_header, prev_hash) = &window[0];
        let (next_header, next_hash) = &window[1];

        verify_header(next_header, next_hash, prev_header.height + 1)?;

        if next_header.prev_hash != *prev_hash {
            bail!(
                "Chain break at height {}: expected prev={} got={}",
                next_header.height,
                hash_prefix(prev_hash),
                hash_prefix(&next_header.prev_hash)
            );
        }
    }

    Ok(())
}

// ── Legacy light-client response (kept for node::proof wire serialization) ────

/// Response from the legacy light-client endpoint (`GET /v1/packages/:canonical/proof`).
#[derive(Debug, Serialize, Deserialize)]
pub struct LightClientResponse {
    pub status: String,
    #[serde(default)]
    pub package_status: String,
    pub block_hash: String,
    pub block_header: BlockHeader,
    pub proof: MerkleProof,
    pub header_chain: Vec<(BlockHeader, String)>,
    #[serde(default)]
    pub pbft_signatures: Vec<BlockSignature>,
    #[serde(default)]
    pub active_validators: Vec<Validator>,
    #[serde(default)]
    pub proven_transaction: Option<Transaction>,
    #[serde(default)]
    pub validator_set_history: Vec<AuthenticatedValidatorSetSnapshot>,
}

// ── Authenticated authority timeline & ProofBundleV2 verifier ─────────────────

/// Quorum-certified authority timeline anchored to the signed chain spec.
#[derive(Debug, Clone)]
pub struct AuthorityTimeline {
    epochs: Vec<(u64, ValidatorSet)>,
}

impl AuthorityTimeline {
    /// Authenticate `history` sequentially starting from `spec`'s bootstrap
    /// validator set up to `tip_height`.
    pub fn from_spec_and_history(
        spec: &ChainSpec,
        history: &[AuthenticatedValidatorSetSnapshot],
        tip_height: u64,
    ) -> Result<Self> {
        spec.validate_consensus_identity_for_client()?;
        let genesis_hash = spec.canonical_genesis_block_for_client()?.hash();
        let mut current = spec.to_runtime_validator_set();
        let mut epochs = vec![(0, current.clone())];
        let mut last_height = 0u64;
        let phase_c = spec.consensus_params.consensus_encoding.is_some();

        for transition in history {
            let height = transition.effective_from_height;
            if height <= last_height || height > tip_height {
                bail!("authority history is unordered or extends past tip height {tip_height}");
            }
            if height == 1
                && !transition
                    .previous_block_hash
                    .eq_ignore_ascii_case(&genesis_hash)
            {
                bail!("authority transition at height 1 does not anchor to signed genesis");
            }
            transition
                .validate_against(&current)
                .map_err(anyhow::Error::msg)?;
            if phase_c
                && transition
                    .validators
                    .validators
                    .iter()
                    .filter(|v| common::validator_status_is_active(&v.status))
                    .count()
                    < 4
            {
                bail!("Phase C authority transition requires at least four active validators");
            }
            current = transition.validators.clone();
            epochs.push((height, current.clone()));
            last_height = height;
        }

        Ok(Self { epochs })
    }

    /// Return the active authority set governing `height`.
    pub fn set_at(&self, height: u64) -> &ValidatorSet {
        self.epochs
            .iter()
            .rfind(|(effective_from, _)| *effective_from <= height)
            .map(|(_, set)| set)
            .expect("timeline always contains genesis height 0")
    }
}

/// Cryptographically proven package status extracted from a `ProofBundleV2`.
#[derive(Debug, Clone)]
pub struct VerifiedProofBundle {
    pub canonical: String,
    pub tip_height: u64,
    pub tip_block_hash: String,
    pub tip_timestamp_unix: i64,
    pub publication_height: u64,
    pub publication_block_hash: String,
    pub record: ChainRecord,
    pub status: VerdictStatus,
    pub status_view: PackageStatusViewV1,
    pub quorum: QuorumLabelV1,
    pub certified: bool,
    pub status_height: u64,
    pub status_proven_by_smt: bool,
}

fn verify_tip_freshness(
    tip_timestamp_unix: i64,
    now: DateTime<Utc>,
    options: &VerifyOptions,
) -> Result<()> {
    let age = now.timestamp().saturating_sub(tip_timestamp_unix);
    if age > options.max_tip_age_secs {
        bail!(
            "proven tip block is stale ({age}s old > max {}s)",
            options.max_tip_age_secs
        );
    }
    if age < -options.max_tip_future_skew_secs {
        bail!(
            "proven tip block timestamp is too far in the future ({}s ahead > max {}s)",
            -age,
            options.max_tip_future_skew_secs
        );
    }
    Ok(())
}

fn verify_checkpoint_anchor(
    checkpoint: Option<&Checkpoint>,
    genesis_hash: &str,
    bundle: &ProofBundleV2,
) -> Result<()> {
    if bundle.tip_height == 1 && bundle.tip_header.prev_hash != genesis_hash {
        bail!("tip block at height 1 does not chain from signed genesis");
    }
    if bundle.publication_height == 1 && bundle.publication_header.prev_hash != genesis_hash {
        bail!("publication block at height 1 does not chain from signed genesis");
    }
    if bundle.publication_height + 1 == bundle.tip_height
        && bundle.tip_header.prev_hash != bundle.publication_block_hash
    {
        bail!("tip header does not chain directly from publication block");
    }
    for transition in &bundle.authority_history {
        if transition.effective_from_height == bundle.publication_height + 1
            && !transition
                .previous_block_hash
                .eq_ignore_ascii_case(&bundle.publication_block_hash)
        {
            bail!("authority transition after publication block belongs to a different fork");
        }
    }

    let Some(cp) = checkpoint else {
        return Ok(());
    };
    if cp.height > bundle.tip_height {
        bail!(
            "checkpoint height {} exceeds proven tip height {}",
            cp.height,
            bundle.tip_height
        );
    }
    if cp.height == 0 && cp.hash != genesis_hash {
        bail!("checkpoint at genesis does not match the verified chain spec");
    }
    if cp.height == bundle.tip_height && cp.hash != bundle.tip_block_hash {
        bail!("checkpoint at tip height does not match proven tip hash");
    }
    if cp.height == bundle.publication_height && cp.hash != bundle.publication_block_hash {
        bail!("checkpoint at publication height does not match proven publication hash");
    }
    for transition in &bundle.authority_history {
        if transition.effective_from_height == cp.height + 1
            && !transition
                .previous_block_hash
                .eq_ignore_ascii_case(&cp.hash)
        {
            bail!("checkpoint conflicts with authority transition fork anchor");
        }
    }
    Ok(())
}

fn verify_publication_approval(
    tx: &Transaction,
    canonical: &str,
    record: &ChainRecord,
    timeline: &AuthorityTimeline,
    params: &ConsensusParams,
    publication_height: u64,
    require_v4_quorum: bool,
) -> Result<QuorumLabelV1> {
    match tx {
        Transaction::PublishAuthorizedV1 {
            committee_qc: Some(qc),
            ..
        } => {
            if qc.authority_height > publication_height {
                bail!("publication approval QC authority height exceeds publication height");
            }
            consensus::vote_v4::verify_committee_qc_for_record(
                qc,
                common::APPROVAL_QC_DOMAIN,
                canonical,
                &record.content_hash,
                Some(record),
                &timeline.set_at(qc.authority_height).validators,
                &timeline.set_at(publication_height).validators,
                params.hybrid_signature_activation_height,
                publication_height,
            )?;
            if !qc
                .evidence_digest
                .trim()
                .eq_ignore_ascii_case(record.evidence_digest.trim())
            {
                bail!("approval QC does not commit to record evidence digest");
            }
            Ok(QuorumLabelV1::V4Quorum)
        }
        Transaction::Publish(_) | Transaction::PublishAuthorizedV1 { .. } => {
            if params.vote_v4_active_at(publication_height) {
                bail!("publication at activated height {publication_height} lacks committee approval QC");
            }
            if require_v4_quorum {
                bail!("publication carries legacy_v3_quorum and require_v4_quorum is enabled");
            }
            Ok(QuorumLabelV1::LegacyV3Quorum)
        }
        _ => bail!("publication transaction is not a publish record"),
    }
}

fn verify_trigger_witness(
    witness: &TriggerWitnessV1,
    canonical: &str,
    content_hash: &str,
    authority: &[Validator],
    require_hybrid: bool,
) -> Result<()> {
    if witness.package_canonical.trim() != canonical.trim() {
        bail!("trigger witness targets another package");
    }
    if !witness
        .content_hash
        .trim()
        .eq_ignore_ascii_case(content_hash.trim())
    {
        bail!("trigger witness targets another content hash");
    }
    let prover = authority
        .iter()
        .find(|validator| {
            validator.id == witness.witness_prover_id.trim()
                && common::validator_status_is_active(&validator.status)
        })
        .context("trigger witness prover is not an active validator at status height")?;
    let public_key: PublicKey = prover.verifying_key(require_hybrid)?;
    witness.verify(&public_key, require_hybrid)
}

pub struct VerifiedRevocationTx {
    pub certified: bool,
    pub authority: RevocationAuthorityV1,
    pub reason: String,
    pub witness_digest: Option<String>,
    pub certificate_digest: Option<String>,
}

/// Independently authenticate a revocation status transaction (`ECO-INV-03`,
/// `ECO-INV-04`, `eco-adv-05`).
///
/// Post-activation `RevokeV2` without a committee QC is certified ONLY when
/// signed by the package publisher (`record.publisher_pubkey`). Any non-owner
/// signer without a committee QC (including a single validator) yields
/// `certified = false`, and fails closed if `leaf_certified == Some(true)`.
#[allow(clippy::too_many_arguments)]
pub fn verify_revocation_transaction(
    tx: &Transaction,
    leaf_certified: Option<bool>,
    canonical: &str,
    content_hash: &str,
    record: Option<&ChainRecord>,
    timeline: &AuthorityTimeline,
    params: &ConsensusParams,
    status_height: u64,
) -> Result<VerifiedRevocationTx> {
    let active = params.vote_v4_active_at(status_height);
    match tx {
        Transaction::RevokeV2 {
            request,
            evidence_hash,
            witness,
            quorum_certificate,
        } => {
            if request.package_canonical.trim() != canonical.trim() {
                bail!("revocation transaction targets another package");
            }
            request.verify_signature()?;
            if active && request.signature_version != 1 {
                bail!("legacy revocation signature version is retired after activation");
            }
            let status_validators = &timeline.set_at(status_height).validators;
            if let Some(w) = witness {
                verify_trigger_witness(w, canonical, content_hash, status_validators, active)?;
            }
            let witness_digest = witness.as_ref().map(|w| w.canonical_digest());

            if let Some(qc) = quorum_certificate {
                if qc.authority_height > status_height {
                    bail!("revocation QC authority height exceeds status height");
                }
                consensus::vote_v4::verify_committee_qc_for_record(
                    qc,
                    common::REVOKE_QC_DOMAIN,
                    canonical,
                    content_hash,
                    record,
                    &timeline.set_at(qc.authority_height).validators,
                    status_validators,
                    params.hybrid_signature_activation_height,
                    status_height,
                )?;
                if !qc
                    .evidence_digest
                    .trim()
                    .eq_ignore_ascii_case(evidence_hash.trim())
                {
                    bail!("revocation QC does not commit to the evidence hash");
                }
                let qc_witness = qc
                    .witness_digest
                    .as_deref()
                    .map(|d| d.trim().to_ascii_lowercase());
                if let Some(ref digest) = witness_digest {
                    if qc_witness.as_deref() != Some(digest.as_str()) {
                        bail!("revocation QC does not commit to the carried witness");
                    }
                } else if qc_witness.is_some() {
                    bail!("revocation QC names a witness the transaction does not carry");
                }
                return Ok(VerifiedRevocationTx {
                    certified: true,
                    authority: RevocationAuthorityV1::CommitteeQuorum,
                    reason: request.reason.clone(),
                    witness_digest,
                    certificate_digest: Some(qc.digest()),
                });
            }

            let is_owner = record.is_some_and(|rec| {
                rec.publisher_pubkey
                    .trim()
                    .eq_ignore_ascii_case(request.revoker_pubkey.trim())
            });
            if is_owner {
                return Ok(VerifiedRevocationTx {
                    certified: true,
                    authority: RevocationAuthorityV1::Owner,
                    reason: request.reason.clone(),
                    witness_digest,
                    certificate_digest: None,
                });
            }

            if active && leaf_certified == Some(true) {
                bail!("post-activation RevokeV2 without committee QC or owner signature cannot be certified offline");
            }
            Ok(VerifiedRevocationTx {
                certified: false,
                authority: RevocationAuthorityV1::LegacySingleValidator,
                reason: request.reason.clone(),
                witness_digest,
                certificate_digest: None,
            })
        }
        Transaction::QuarantineResolutionV1 {
            package_canonical,
            content_hash: tx_hash,
            outcome: QuarantineOutcomeV1::Reject,
            quorum_certificate,
        } => {
            if !active {
                bail!("QuarantineResolutionV1 is invalid before hybrid activation");
            }
            if package_canonical.trim() != canonical.trim() {
                bail!("quarantine rejection targets another package");
            }
            if !tx_hash.trim().eq_ignore_ascii_case(content_hash.trim()) {
                bail!("quarantine rejection targets another content hash");
            }
            if quorum_certificate.decision != Some(QuarantineOutcomeV1::Reject) {
                bail!("quarantine resolution QC does not commit to Reject");
            }
            if quorum_certificate.authority_height > status_height {
                bail!("quarantine resolution QC authority height exceeds status height");
            }
            consensus::vote_v4::verify_committee_qc_for_record(
                quorum_certificate,
                common::QUARANTINE_RESOLUTION_QC_DOMAIN,
                canonical,
                content_hash,
                record,
                &timeline
                    .set_at(quorum_certificate.authority_height)
                    .validators,
                &timeline.set_at(status_height).validators,
                params.hybrid_signature_activation_height,
                status_height,
            )?;
            Ok(VerifiedRevocationTx {
                certified: true,
                authority: RevocationAuthorityV1::CommitteeQuorum,
                reason: "rejected by Class C committee review".to_string(),
                witness_digest: quorum_certificate.witness_digest.clone(),
                certificate_digest: Some(quorum_certificate.digest()),
            })
        }
        Transaction::Revoke {
            package_canonical,
            reason,
            ..
        } => {
            if package_canonical.trim() != canonical.trim() {
                bail!("legacy revocation targets another package");
            }
            if active {
                bail!("legacy unsigned revocation is invalid after hybrid activation");
            }
            Ok(VerifiedRevocationTx {
                certified: false,
                authority: RevocationAuthorityV1::LegacySingleValidator,
                reason: reason.clone(),
                witness_digest: None,
                certificate_digest: None,
            })
        }
        _ => bail!("status transaction is not a valid revocation transaction"),
    }
}

fn category_reason(code: CregFindingCategoryV1) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{code:?}"))
}

fn status_block_hash_for(height: u64, bundle: &ProofBundleV2) -> String {
    if height == bundle.tip_height {
        bundle.tip_block_hash.clone()
    } else if height == bundle.publication_height {
        bundle.publication_block_hash.clone()
    } else {
        String::new()
    }
}

struct VerifiedStatusState {
    status: VerdictStatus,
    status_view: PackageStatusViewV1,
    certified: bool,
    status_height: u64,
    status_proven_by_smt: bool,
}

#[allow(clippy::too_many_arguments)]
fn verify_post_activation_status(
    canonical: &str,
    package: &str,
    version: &str,
    bundle: &ProofBundleV2,
    record: &ChainRecord,
    timeline: &AuthorityTimeline,
    params: &ConsensusParams,
    now: DateTime<Utc>,
) -> Result<VerifiedStatusState> {
    let tip_root = bundle
        .tip_header
        .status_root
        .context("post-activation tip header omitted status_root")?;
    let declared_root = bundle
        .status_root
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("post-activation proof bundle omitted status_root"))
        .and_then(|text| hex32::parse(text).map_err(anyhow::Error::msg))?;
    if declared_root != tip_root {
        bail!("proof bundle status_root does not match proven tip header");
    }
    let leaf = bundle
        .status_leaf
        .as_ref()
        .context("post-activation proof bundle omitted status_leaf")?;
    let smt_proof = bundle
        .status_proof
        .as_ref()
        .context("post-activation proof bundle omitted status_proof")?;
    if !smt_proof.verify_for_encoding(
        &tip_root,
        package,
        version,
        Some(leaf),
        params.consensus_encoding.as_deref(),
    ) {
        bail!("status SMT proof failed against proven tip status_root");
    }
    if !leaf
        .content_hash
        .trim()
        .eq_ignore_ascii_case(record.content_hash.trim())
    {
        bail!("status leaf content_hash does not match proven publication record");
    }
    if leaf.height < bundle.publication_height || leaf.height > bundle.tip_height {
        bail!("status leaf height is outside [publication_height, tip_height]");
    }

    let status_validators = &timeline.set_at(leaf.height).validators;
    match leaf.status {
        StatusLeafStateV1::Verified => {
            if leaf.height == bundle.publication_height {
                if !matches!(record.status, PackageStatus::Verified) {
                    bail!("publication record status is not Verified");
                }
            } else {
                let Some(Transaction::QuarantineResolutionV1 {
                    package_canonical,
                    content_hash,
                    outcome: QuarantineOutcomeV1::Release,
                    quorum_certificate,
                }) = &bundle.status_transaction
                else {
                    bail!("post-publication Verified status requires a verified QuarantineResolutionV1(Release)");
                };
                if package_canonical.trim() != canonical.trim() {
                    bail!("quarantine release targets another package");
                }
                if !content_hash
                    .trim()
                    .eq_ignore_ascii_case(record.content_hash.trim())
                {
                    bail!("quarantine release targets another content hash");
                }
                if quorum_certificate.decision != Some(QuarantineOutcomeV1::Release) {
                    bail!("quarantine release QC does not commit to Release");
                }
                if quorum_certificate.authority_height > leaf.height {
                    bail!("quarantine release QC authority height exceeds leaf height");
                }
                consensus::vote_v4::verify_committee_qc_for_record(
                    quorum_certificate,
                    common::QUARANTINE_RESOLUTION_QC_DOMAIN,
                    canonical,
                    &record.content_hash,
                    Some(record),
                    &timeline
                        .set_at(quorum_certificate.authority_height)
                        .validators,
                    status_validators,
                    params.hybrid_signature_activation_height,
                    leaf.height,
                )?;
            }
            let status_view = PackageStatusViewV1::Verified {
                height: leaf.height,
                block_hash: status_block_hash_for(leaf.height, bundle),
            };
            let status = VerdictStatus::Verified {
                block_hash: bundle.publication_block_hash.clone(),
                content_hash: record.content_hash.clone(),
                ipfs_cid: record.ipfs_cid.clone(),
                findings: record.findings.clone(),
            };
            Ok(VerifiedStatusState {
                status,
                status_view,
                certified: true,
                status_height: leaf.height,
                status_proven_by_smt: true,
            })
        }
        StatusLeafStateV1::Quarantined {
            expires_at_unix,
            reason_code,
        } => {
            let Some(Transaction::QuarantineV1 {
                record: q_record,
                reason_code: tx_reason,
                quorum_certificate,
            }) = &bundle.status_transaction
            else {
                bail!("post-activation Quarantined status requires QuarantineV1 transaction");
            };
            if q_record.id.canonical() != canonical {
                bail!("quarantine transaction targets another package");
            }
            if !q_record
                .content_hash
                .trim()
                .eq_ignore_ascii_case(record.content_hash.trim())
            {
                bail!("quarantine transaction targets another content hash");
            }
            if *tx_reason != reason_code {
                bail!("quarantine transaction reason_code disagrees with status leaf");
            }
            if quorum_certificate.authority_height > leaf.height {
                bail!("quarantine QC authority height exceeds leaf height");
            }
            consensus::vote_v4::verify_committee_qc_for_record(
                quorum_certificate,
                common::QUARANTINE_QC_DOMAIN,
                canonical,
                &record.content_hash,
                Some(record),
                &timeline
                    .set_at(quorum_certificate.authority_height)
                    .validators,
                status_validators,
                params.hybrid_signature_activation_height,
                leaf.height,
            )?;
            let status = VerdictStatus::Quarantined {
                reason: category_reason(reason_code),
                content_hash: record.content_hash.clone(),
                expires_at_unix: Some(expires_at_unix),
            };
            let status_view = PackageStatusViewV1::Quarantined {
                height: leaf.height,
                expires_at_unix,
                reason_code,
                committee: None,
            }
            .at(now.timestamp());
            Ok(VerifiedStatusState {
                status,
                status_view,
                certified: true,
                status_height: leaf.height,
                status_proven_by_smt: true,
            })
        }
        StatusLeafStateV1::Revoked { certified } => {
            let (final_certified, authority, reason, witness_digest, certificate_digest) =
                match &bundle.status_transaction {
                    Some(tx) => {
                        let verified = verify_revocation_transaction(
                            tx,
                            Some(certified),
                            canonical,
                            &record.content_hash,
                            Some(record),
                            timeline,
                            params,
                            leaf.height,
                        )?;
                        (
                            verified.certified,
                            verified.authority,
                            verified.reason,
                            verified.witness_digest,
                            verified.certificate_digest,
                        )
                    }
                    None => {
                        if !certified {
                            (
                                false,
                                RevocationAuthorityV1::LegacySingleValidator,
                                "uncertified revocation".to_string(),
                                None,
                                None,
                            )
                        } else {
                            (
                                true,
                                RevocationAuthorityV1::QuarantineExpiry,
                                "quarantine expired without resolution".to_string(),
                                None,
                                None,
                            )
                        }
                    }
                };
            Ok(VerifiedStatusState {
                status: VerdictStatus::Revoked {
                    reason: reason.clone(),
                    findings: vec![],
                },
                status_view: PackageStatusViewV1::Revoked {
                    height: leaf.height,
                    authority,
                    reason: DisplayText::bounded(&reason, MAX_REASON_CHARS),
                    witness_digest,
                    certificate_digest,
                },
                certified: final_certified,
                status_height: leaf.height,
                status_proven_by_smt: true,
            })
        }
    }
}

fn verify_pre_activation_status(
    canonical: &str,
    bundle: &ProofBundleV2,
    record: &ChainRecord,
    timeline: &AuthorityTimeline,
    params: &ConsensusParams,
) -> Result<VerifiedStatusState> {
    if bundle.tip_header.status_root.is_some()
        || bundle.status_root.is_some()
        || bundle.status_leaf.is_some()
        || bundle.status_proof.is_some()
    {
        bail!("pre-activation proof bundle unexpectedly carries status_root / status_proof");
    }

    if let Some(tx) = &bundle.status_transaction {
        let verified = verify_revocation_transaction(
            tx,
            None,
            canonical,
            &record.content_hash,
            Some(record),
            timeline,
            params,
            bundle.tip_height,
        )?;
        let reason = verified.reason;
        return Ok(VerifiedStatusState {
            status: VerdictStatus::Revoked {
                reason: reason.clone(),
                findings: vec![],
            },
            status_view: PackageStatusViewV1::Revoked {
                height: bundle.tip_height,
                authority: verified.authority,
                reason: DisplayText::bounded(&reason, MAX_REASON_CHARS),
                witness_digest: verified.witness_digest,
                certificate_digest: verified.certificate_digest,
            },
            certified: verified.certified,
            status_height: bundle.tip_height,
            status_proven_by_smt: false,
        });
    }

    match &record.status {
        PackageStatus::Verified => Ok(VerifiedStatusState {
            status: VerdictStatus::Verified {
                block_hash: bundle.publication_block_hash.clone(),
                content_hash: record.content_hash.clone(),
                ipfs_cid: record.ipfs_cid.clone(),
                findings: record.findings.clone(),
            },
            status_view: PackageStatusViewV1::Verified {
                height: bundle.publication_height,
                block_hash: bundle.publication_block_hash.clone(),
            },
            certified: true,
            status_height: bundle.publication_height,
            status_proven_by_smt: false,
        }),
        PackageStatus::Pending => Ok(VerifiedStatusState {
            status: VerdictStatus::Unverified,
            status_view: PackageStatusViewV1::Pending,
            certified: false,
            status_height: bundle.publication_height,
            status_proven_by_smt: false,
        }),
        PackageStatus::Quarantined {
            reason,
            expires_at_unix,
        } => Ok(VerifiedStatusState {
            status: VerdictStatus::Quarantined {
                reason: reason.clone(),
                content_hash: record.content_hash.clone(),
                expires_at_unix: Some(*expires_at_unix),
            },
            status_view: PackageStatusViewV1::Quarantined {
                height: bundle.publication_height,
                expires_at_unix: *expires_at_unix,
                reason_code: CregFindingCategoryV1::Other,
                committee: None,
            },
            certified: false,
            status_height: bundle.publication_height,
            status_proven_by_smt: false,
        }),
        PackageStatus::Revoked { reason } => Ok(VerifiedStatusState {
            status: VerdictStatus::Revoked {
                reason: reason.clone(),
                findings: vec![],
            },
            status_view: PackageStatusViewV1::Revoked {
                height: bundle.publication_height,
                authority: RevocationAuthorityV1::LegacySingleValidator,
                reason: DisplayText::bounded(reason, MAX_REASON_CHARS),
                witness_digest: None,
                certificate_digest: None,
            },
            certified: false,
            status_height: bundle.publication_height,
            status_proven_by_smt: false,
        }),
    }
}

/// Cryptographically verify a [`ProofBundleV2`] offline against `spec`
/// (ADR-019 §4.1, `ECO-INV-01`, `ECO-INV-03`).
pub fn verify_proof_bundle(
    canonical: &str,
    bundle: &ProofBundleV2,
    spec: &ChainSpec,
    checkpoint: Option<&Checkpoint>,
    options: &VerifyOptions,
    now: DateTime<Utc>,
) -> Result<VerifiedProofBundle> {
    if bundle.schema != PROOF_BUNDLE_SCHEMA {
        bail!("unexpected proof bundle schema {:?}", bundle.schema);
    }
    if bundle.canonical.as_str() != canonical {
        bail!(
            "proof bundle targets {:?}, expected {:?}",
            bundle.canonical.as_str(),
            canonical
        );
    }
    if bundle.tip_height == 0 || bundle.publication_height == 0 {
        bail!("proof bundle block heights must be at least 1");
    }
    if bundle.publication_height > bundle.tip_height {
        bail!("proof bundle publication_height exceeds tip_height");
    }
    let (package, version) =
        split_canonical(canonical).context("canonical package ID cannot be split")?;

    verify_header(
        &bundle.tip_header,
        &bundle.tip_block_hash,
        bundle.tip_height,
    )?;
    if bundle.tip_header.timestamp.timestamp() != bundle.tip_timestamp_unix {
        bail!("proof bundle tip_timestamp_unix disagrees with tip header");
    }
    verify_tip_freshness(bundle.tip_timestamp_unix, now, options)?;

    let timeline = AuthorityTimeline::from_spec_and_history(
        spec,
        &bundle.authority_history,
        bundle.tip_height,
    )?;
    let genesis_hash = spec.canonical_genesis_block_for_client()?.hash();
    verify_checkpoint_anchor(checkpoint, &genesis_hash, bundle)?;

    verify_pbft_consensus_with_params(
        &bundle.tip_block_hash,
        &bundle.tip_header,
        &timeline.set_at(bundle.tip_height).validators,
        &bundle.tip_commit_signatures,
        &spec.consensus_params,
    )?;

    verify_header(
        &bundle.publication_header,
        &bundle.publication_block_hash,
        bundle.publication_height,
    )?;
    if bundle.publication_height == bundle.tip_height
        && bundle.publication_block_hash != bundle.tip_block_hash
    {
        bail!("same-height publication block hash disagrees with tip block hash");
    }
    verify_pbft_consensus_with_params(
        &bundle.publication_block_hash,
        &bundle.publication_header,
        &timeline.set_at(bundle.publication_height).validators,
        &bundle.publication_commit_signatures,
        &spec.consensus_params,
    )?;

    let encoding = spec.consensus_params.consensus_encoding.as_deref();
    let computed_tx_hash =
        common::transaction_hash_for_encoding(&bundle.publication_transaction, encoding)?;
    if computed_tx_hash != bundle.publication_tx_hash {
        bail!("publication transaction does not match publication_tx_hash");
    }
    let merkle_proof = MerkleProof {
        tx_hash: bundle.publication_tx_hash.clone(),
        path: bundle
            .publication_merkle_path
            .iter()
            .map(|step| MerkleStep {
                sibling_hash: step.sibling_hash.clone(),
                is_right: step.is_right,
            })
            .collect(),
        expected_root: bundle.publication_header.merkle_root.clone(),
    };
    if !merkle_proof.verify() {
        bail!("publication Merkle inclusion proof failed for {canonical}");
    }

    let record = bundle
        .publication_transaction
        .published_record()
        .context("publication transaction is not a package publication")?
        .clone();
    if record.id.canonical() != canonical {
        bail!("publication transaction belongs to a different package");
    }

    let derived_quorum = verify_publication_approval(
        &bundle.publication_transaction,
        canonical,
        &record,
        &timeline,
        &spec.consensus_params,
        bundle.publication_height,
        options.require_v4_quorum,
    )?;
    if bundle.quorum != derived_quorum {
        bail!("proof bundle quorum label does not match proven publication transaction");
    }

    let status_state = if spec.consensus_params.vote_v4_active_at(bundle.tip_height) {
        verify_post_activation_status(
            canonical,
            package,
            version,
            bundle,
            &record,
            &timeline,
            &spec.consensus_params,
            now,
        )?
    } else {
        verify_pre_activation_status(
            canonical,
            bundle,
            &record,
            &timeline,
            &spec.consensus_params,
        )?
    };

    Ok(VerifiedProofBundle {
        canonical: canonical.to_string(),
        tip_height: bundle.tip_height,
        tip_block_hash: bundle.tip_block_hash.clone(),
        tip_timestamp_unix: bundle.tip_timestamp_unix,
        publication_height: bundle.publication_height,
        publication_block_hash: bundle.publication_block_hash.clone(),
        record,
        status: status_state.status,
        status_view: status_state.status_view,
        quorum: derived_quorum,
        certified: status_state.certified,
        status_height: status_state.status_height,
        status_proven_by_smt: status_state.status_proven_by_smt,
    })
}

/// Fetch `ProofBundleV2` from `GET /v1/public/v4/packages/:name/:version/proof-bundle`.
pub async fn fetch_proof_bundle(canonical: &str, node_url: &str) -> Result<Option<ProofBundleV2>> {
    let (package, version) =
        split_canonical(canonical).context("canonical package ID cannot be split")?;
    let url = format!(
        "{}/v1/public/v4/packages/{}/{}/proof-bundle",
        node_url.trim_end_matches('/'),
        urlencoding::encode(package),
        urlencoding::encode(version),
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()?;
    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .context("Failed to reach chain node for v4 proof bundle")?;

    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }

    let bundle: ProofBundleV2 = response
        .error_for_status()
        .context("Node returned an error for v4 proof bundle")?
        .json()
        .await
        .context("Invalid JSON from v4 proof-bundle endpoint")?;
    Ok(Some(bundle))
}

/// Fetch and cryptographically verify a package proof bundle with explicit
/// [`VerifyOptions`].
pub async fn verify_package_with_options(
    canonical: &str,
    node_url: &str,
    checkpoint: Option<&Checkpoint>,
    options: &VerifyOptions,
) -> Result<Option<VerifiedProofBundle>> {
    let spec = load_verified_chain_spec().await?;
    let Some(bundle) = fetch_proof_bundle(canonical, node_url).await? else {
        return Ok(None);
    };
    let verified = verify_proof_bundle(canonical, &bundle, &spec, checkpoint, options, Utc::now())?;
    Ok(Some(verified))
}

/// Fetch and cryptographically verify a package verdict using `ProofBundleV2`.
/// Returns `true` ONLY when the proven current status is `Verified`.
pub async fn verify_package(
    canonical: &str,
    node_url: &str,
    checkpoint: &Checkpoint,
) -> Result<bool> {
    Ok(verify_package_with_options(
        canonical,
        node_url,
        Some(checkpoint),
        &VerifyOptions::from_env(),
    )
    .await?
    .is_some_and(|verified| verified.status.is_safe()))
}

/// Verify a REST [`TrustVerdict`] against the node's [`ProofBundleV2`] and return
/// both the proof-corrected verdict and the verified proof metadata.
pub async fn verify_verdict_detailed(
    mut verdict: TrustVerdict,
    node_url: &str,
    checkpoint: Option<&Checkpoint>,
    options: &VerifyOptions,
) -> Result<(TrustVerdict, Option<VerifiedProofBundle>)> {
    let canonical = verdict.package.canonical();
    let verified_result =
        verify_package_with_options(&canonical, node_url, checkpoint, options).await;

    match verified_result {
        Ok(Some(verified)) => {
            verdict.status = verified.status.clone();
            verdict.publisher = Some(verified.record.publisher_pubkey.clone());
            verdict.deterministic_risk = Some(verified.record.deterministic_risk.clone());
            tracing::info!(
                "Proof-carrying verification completed for {} (tip {}, status_height {})",
                canonical,
                verified.tip_height,
                verified.status_height
            );
            Ok((verdict, Some(verified)))
        }
        Ok(None) => {
            if verdict.status.is_safe() {
                bail!("node claimed Verified for {canonical} without a v4 proof bundle");
            }
            Ok((verdict, None))
        }
        Err(err) => {
            // Preserve an explicit REST block so a proof failure cannot clear a
            // quarantine or revocation into break-glass installability.
            if verdict.status.is_blocked() {
                tracing::warn!(
                    "v4 proof verification failed for blocked REST verdict {} ({}); keeping block",
                    canonical,
                    err
                );
                Ok((verdict, None))
            } else {
                Err(err)
            }
        }
    }
}

/// Replace endpoint/cache data with cryptographically proven publication and
/// current-status fields from [`ProofBundleV2`].
pub async fn verify_verdict_with_options(
    verdict: TrustVerdict,
    node_url: &str,
    checkpoint: Option<&Checkpoint>,
    options: &VerifyOptions,
) -> Result<TrustVerdict> {
    Ok(
        verify_verdict_detailed(verdict, node_url, checkpoint, options)
            .await?
            .0,
    )
}

/// [`verify_verdict_with_options`] with default environment options.
pub async fn verify_verdict(
    verdict: TrustVerdict,
    node_url: &str,
    checkpoint: Option<&Checkpoint>,
) -> Result<TrustVerdict> {
    verify_verdict_with_options(verdict, node_url, checkpoint, &VerifyOptions::from_env()).await
}

/// Build a Merkle inclusion proof for a transaction at `tx_index` within `block`.
pub fn build_merkle_proof(block: &Block, tx_index: usize) -> Result<MerkleProof> {
    let n = block.transactions.len();
    if tx_index >= n {
        bail!(
            "tx_index {} out of range (block has {} transactions)",
            tx_index,
            n
        );
    }

    let leaves: Vec<String> = block
        .transactions
        .iter()
        .map(common::transaction_hash)
        .collect();

    let tx_hash = leaves[tx_index].clone();
    let expected_root = block.header.merkle_root.clone();

    let mut path = Vec::new();
    let mut idx = tx_index;
    let mut level = leaves.clone();

    while level.len() > 1 {
        if !level.len().is_multiple_of(2) {
            level.push(
                level
                    .last()
                    .ok_or_else(|| anyhow::anyhow!("Empty merkle level during proof construction"))?
                    .clone(),
            );
        }

        let sibling_idx = if idx.is_multiple_of(2) {
            idx + 1
        } else {
            idx - 1
        };
        path.push(MerkleStep {
            sibling_hash: level[sibling_idx].clone(),
            is_right: idx.is_multiple_of(2),
        });

        level = level
            .chunks(2)
            .map(|pair| sha256_hex(format!("{}{}", pair[0], pair[1]).as_bytes()))
            .collect();
        idx /= 2;
    }

    Ok(MerkleProof {
        tx_hash,
        path,
        expected_root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkle_proof_verifies() {
        use common::{PackageId, PackageStatus};

        let txs: Vec<Transaction> = (0..4)
            .map(|i| {
                Transaction::Publish(ChainRecord {
                    id: PackageId::new("npm", format!("pkg-{}", i), "1.0.0"),
                    content_hash: sha256_hex(format!("pkg-{}", i).as_bytes()),
                    ipfs_cid: format!("bafytest{}", i),
                    publisher_pubkey: "pub".into(),
                    block_hash: "0".repeat(64),
                    published_at: Utc::now(),
                    validator_signatures: vec![],
                    status: PackageStatus::Verified,
                    ..Default::default()
                })
            })
            .collect();

        let block = Block {
            header: BlockHeader {
                height: 1,
                prev_hash: "0".repeat(64),
                merkle_root: common::merkle_root(&txs),
                proposer_id: "test".into(),
                timestamp: Utc::now(),
                validator_set_hash: "dev".into(),
                vrf_output: None,
                vrf_proof: None,
                view_number: 0,
                view_change_certificate: vec![],
                status_root: None,
            },
            transactions: txs,
            pbft_signatures: vec![],
        };

        let proof = build_merkle_proof(&block, 2).unwrap();
        assert!(proof.verify(), "Merkle proof should verify");

        let mut bad_proof = proof.clone();
        bad_proof.tx_hash = sha256_hex(b"tampered");
        assert!(!bad_proof.verify(), "Tampered proof should fail");
    }

    #[test]
    fn header_chain_verifies() {
        let genesis = Checkpoint::genesis();
        verify_header_chain(&[], &genesis).unwrap();
    }

    #[test]
    fn header_chain_verifies_single_header() {
        let genesis = Checkpoint::genesis();
        let block = Block {
            header: BlockHeader {
                height: 1,
                prev_hash: genesis.hash.clone(),
                merkle_root: sha256_hex(b"leaf"),
                proposer_id: "validator-1".into(),
                timestamp: Utc::now(),
                validator_set_hash: "dev".into(),
                vrf_output: None,
                vrf_proof: None,
                view_number: 0,
                view_change_certificate: vec![],
                status_root: None,
            },
            transactions: vec![],
            pbft_signatures: vec![],
        };

        verify_header_chain(&[(block.header.clone(), block.hash())], &genesis).unwrap();
    }

    #[test]
    fn header_chain_rejects_tampered_first_hash() {
        let genesis = Checkpoint::genesis();
        let header = BlockHeader {
            height: 1,
            prev_hash: genesis.hash.clone(),
            merkle_root: sha256_hex(b"leaf"),
            proposer_id: "validator-1".into(),
            timestamp: Utc::now(),
            validator_set_hash: "dev".into(),
            vrf_output: None,
            vrf_proof: None,
            view_number: 0,
            view_change_certificate: vec![],
            status_root: None,
        };

        assert!(verify_header_chain(&[(header, "bad-hash".into())], &genesis).is_err());
    }

    #[test]
    fn checkpoint_genesis_hash_stable() {
        let c1 = Checkpoint::genesis();
        let c2 = Checkpoint::genesis();
        assert_eq!(c1.hash, c2.hash, "Genesis hash must be deterministic");
    }

    #[test]
    fn test_pbft_consensus_verification() {
        use ed25519_dalek::{Signer, SigningKey};

        let keys: Vec<SigningKey> = (0..3)
            .map(|i| SigningKey::from_bytes(&[i as u8 + 1; 32]))
            .collect();

        let validators: Vec<Validator> = keys
            .iter()
            .enumerate()
            .map(|(i, key)| Validator {
                id: format!("val-{}", i),
                alias: format!("val-{}", i),
                pubkey: hex::encode(key.verifying_key().as_bytes()),
                pq_pubkey: None,
                eth_address: String::new(),
                stake: 100,
                reputation: 100,
                status: "active".into(),
                performance_score: 500,
                contribution_count: 0,
            })
            .collect();

        let val_set_hash = compute_validator_set_hash(&validators);

        let mut block_header = BlockHeader {
            height: 1,
            prev_hash: "0".repeat(64),
            merkle_root: "0".repeat(64),
            proposer_id: "val-0".into(),
            timestamp: Utc::now(),
            validator_set_hash: val_set_hash,
            vrf_output: None,
            vrf_proof: None,
            view_number: 0,
            view_change_certificate: vec![],
            status_root: None,
        };

        let block_hash =
            "abc123abc123abc123abc123abc123abc123abc123abc123abc123abc1231234".to_string();
        let message = consensus::pbft::pbft_signature_message("commit", &block_hash);

        let pbft_signatures: Vec<BlockSignature> = keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                let signature = key.sign(message.as_bytes());
                BlockSignature {
                    validator_id: format!("val-{}", i),
                    pubkey: hex::encode(key.verifying_key().as_bytes()),
                    signature: hex::encode(signature.to_bytes()),
                    pq_signature: None,
                }
            })
            .collect();

        // Standard quorum of 3 validators is (2*3/3)+1 = 3.
        assert!(
            verify_pbft_consensus(&block_hash, &block_header, &validators, &pbft_signatures)
                .is_ok()
        );
        assert!(verify_pbft_consensus(
            &block_hash,
            &block_header,
            &validators,
            &pbft_signatures[..2]
        )
        .is_err());

        block_header.validator_set_hash = "wrong-hash".to_string();
        assert!(
            verify_pbft_consensus(&block_hash, &block_header, &validators, &pbft_signatures)
                .is_err()
        );
    }

    struct ProofFixture {
        spec_signing_key: common::crypto::SigningKey,
        spec: ChainSpec,
        publisher_key: common::crypto::SigningKey,
        ed_seeds: Vec<[u8; 32]>,
        hybrid_keys: Vec<common::crypto::SigningKey>,
        validators: Vec<Validator>,
    }

    impl ProofFixture {
        fn new(consensus_encoding: Option<&str>, activation_height: Option<u64>) -> Self {
            use common::chain_spec::{
                Bootnode, FeatureFlags, L1Config, Network, Phase, Services, Signing, Support,
                ValidatorSetSpec, ValidatorSpecEntry, CURRENT_SPEC_VERSION,
            };
            use std::collections::BTreeMap;

            let spec_signing_key = common::crypto::SigningKey::from_ed25519_seed(&[77u8; 32]);
            let publisher_key = common::crypto::SigningKey::from_ed25519_seed(&[88u8; 32]);
            let mut ed_seeds = Vec::new();
            let mut hybrid_keys = Vec::new();
            let mut validators = Vec::new();
            let mut spec_entries = Vec::new();

            for i in 0..4u8 {
                let ed_seed = [i + 10; 32];
                let pq_seed = [i + 50; 32];
                let hybrid = common::crypto::SigningKey::hybrid_from_seeds(&ed_seed, &pq_seed);
                let ed_only = common::crypto::SigningKey::from_ed25519_seed(&ed_seed);
                let pk_bytes = hybrid.public_key().to_bytes();
                let ed_hex = ed_only.public_key().to_hex();
                let pq_hex = hex::encode(&pk_bytes[common::crypto::ED25519_PUBLIC_KEY_LEN..]);
                let id = format!("val-{i}");
                let eth_address = format!("0x{:040x}", i + 1);
                validators.push(Validator {
                    id: id.clone(),
                    alias: id.clone(),
                    pubkey: ed_hex.clone(),
                    eth_address: eth_address.clone(),
                    stake: 100,
                    reputation: 100,
                    status: "active".into(),
                    performance_score: 500,
                    contribution_count: 0,
                    pq_pubkey: Some(pq_hex.clone()),
                });
                let pq_binding_signature =
                    common::PqKeyBindingV1::sign_with(id.clone(), "creg-stage4c-test", &hybrid)
                        .expect("sign PQ key binding")
                        .compact_signature();
                spec_entries.push(ValidatorSpecEntry {
                    id,
                    alias: format!("val-{i}"),
                    pubkey: ed_hex,
                    eth_address,
                    stake: 100,
                    reputation: 100,
                    status: "active".into(),
                    pq_pubkey: Some(pq_hex),
                    pq_binding_signature: Some(pq_binding_signature),
                });
                ed_seeds.push(ed_seed);
                hybrid_keys.push(hybrid);
            }

            let mut contracts = BTreeMap::new();
            contracts.insert(
                "registry".into(),
                "0xD8a5a9b31c3C0232E196d518E89Fd8bF83AcAd43".into(),
            );
            let mut spec = ChainSpec {
                spec_version: CURRENT_SPEC_VERSION,
                chain_id: "creg-stage4c-test".into(),
                network: Network::Testnet,
                phase: Phase::Alpha,
                genesis_time: "2026-05-01T00:00:00Z".into(),
                genesis_hash: "0x00".into(),
                consensus_params: ConsensusParams {
                    consensus_encoding: consensus_encoding.map(str::to_string),
                    block_time_seconds: 5,
                    vote_timeout_ms: 120_000,
                    quorum_percentage: 67,
                    min_validator_stake_wei: "100".into(),
                    min_publisher_stake_wei: "10".into(),
                    unbonding_period_seconds: 86_400,
                    slash_penalty_low_bp: 200,
                    slash_penalty_medium_bp: 1_000,
                    slash_penalty_critical_bp: 3_000,
                    max_validators: 50,
                    namespace_authority_activation_height: None,
                    namespace_authority_policies: vec![],
                    hybrid_signature_activation_height: activation_height,
                    vote_v4_quorum: None,
                },
                feature_flags: FeatureFlags {
                    zk_validation: false,
                    ml_validation: true,
                    wasm_sandbox: true,
                    private_registries: true,
                    cross_chain: false,
                    insurance: false,
                    threshold_encryption: false,
                },
                l1: L1Config {
                    name: "sepolia".into(),
                    chain_id: 11_155_111,
                    block_explorer: "https://sepolia.etherscan.io".into(),
                    min_finality_blocks: 6,
                },
                contracts,
                bootnodes: vec![Bootnode {
                    id: "boot-0".into(),
                    operator: "test".into(),
                    region: "eu".into(),
                    multiaddr: "/ip4/127.0.0.1/tcp/9000".into(),
                }],
                validator_set: ValidatorSetSpec {
                    version: 1,
                    last_updated: "2026-05-01T00:00:00Z".into(),
                    epoch_block_height: 0,
                    validators: spec_entries,
                },
                services: Services {
                    ipfs_gateway: "https://ipfs.example".into(),
                    ipfs_api: "https://ipfs-api.example".into(),
                    faucet: "https://faucet.example".into(),
                    explorer: "https://explorer.example".into(),
                    metrics: "https://metrics.example".into(),
                },
                support: Support::default(),
                signing: Signing {
                    signature_algorithm: common::crypto::SignatureScheme::Ed25519.id().to_string(),
                    signing_key_pubkey_hex: spec_signing_key.public_key().to_hex(),
                    detached_signature_url: "https://spec.example/chain-spec.json.sig".into(),
                },
            };
            spec.genesis_hash = spec.compute_genesis_hash().unwrap();

            Self {
                spec_signing_key,
                spec,
                publisher_key,
                ed_seeds,
                hybrid_keys,
                validators,
            }
        }

        fn encoding(&self) -> Option<&str> {
            self.spec.consensus_params.consensus_encoding.as_deref()
        }

        fn genesis_hash(&self) -> String {
            self.spec
                .canonical_genesis_block_for_client()
                .unwrap()
                .hash()
        }

        fn sign_spec(&self) -> String {
            let jcs = common::chain_spec::to_jcs_canonical_json(&self.spec).unwrap();
            let msg = format!(
                "{}|{}",
                common::chain_spec::SPEC_SIGNATURE_DOMAIN,
                String::from_utf8(jcs).unwrap()
            );
            hex::encode(self.spec_signing_key.sign(msg.as_bytes()).ed25519_bytes())
        }

        fn record(&self, canonical: &str, status: PackageStatus) -> ChainRecord {
            let (pkg, version) = split_canonical(canonical).unwrap();
            let (eco, name) = pkg.split_once(':').unwrap();
            ChainRecord {
                id: common::PackageId::new(eco, name, version),
                content_hash: sha256_hex(canonical.as_bytes()),
                ipfs_cid: "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi".into(),
                publisher_pubkey: self.publisher_key.public_key().to_hex(),
                block_hash: "0".repeat(64),
                published_at: Utc::now(),
                validator_signatures: vec![],
                status,
                evidence_digest: sha256_hex(format!("ev:{canonical}").as_bytes()),
                ..Default::default()
            }
        }

        fn authorization(&self, record: &ChainRecord) -> common::PublisherAuthorizationV1 {
            common::PublisherAuthorizationV1 {
                schema_version: common::PUBLISHER_AUTHORIZATION_SCHEMA_VERSION,
                chain_id: self.spec.chain_id.clone(),
                genesis_hash: self.spec.genesis_hash.clone(),
                staking_contract: "0x0000000000000000000000000000000000000001".into(),
                publisher_address: "0x0000000000000000000000000000000000000002".into(),
                publisher_ed25519_pubkey: record.publisher_pubkey.clone(),
                publisher_ed25519_signature: "00".repeat(64),
                publisher_evm_signature: "00".repeat(65),
                nonce: "1".into(),
                issued_at: 1_700_000_000,
                expires_at: 1_700_086_400,
                source_hash: record.content_hash.clone(),
                manifest_hash: record.content_hash.clone(),
                ipfs_cid: record.ipfs_cid.clone(),
                shielded: false,
                key_bundle_sha256: String::new(),
                pgp_signature_sha256: String::new(),
                pgp_public_key_sha256: String::new(),
                ownership_claim: "publisher".into(),
                threshold: 0,
                publisher_pubkeys: vec![],
                signatures: vec![],
                ownership_proof: None,
                authority_certificate: None,
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn sign_qc(
            &self,
            domain: &str,
            canonical: &str,
            content_hash: &str,
            evidence_digest: &str,
            authority_height: u64,
            height: u64,
            decision: Option<QuarantineOutcomeV1>,
            witness_digest: Option<&str>,
        ) -> common::CommitteeQuorumCertificateV1 {
            let hybrid = self.spec.consensus_params.vote_v4_active_at(height);
            let committee = consensus::vote_v4::full_set_committee(&self.validators);
            let mut qc = common::CommitteeQuorumCertificateV1::new(
                domain,
                canonical,
                content_hash,
                evidence_digest,
                &committee,
                authority_height,
            );
            qc.decision = decision;
            qc.witness_digest = witness_digest.map(str::to_string);
            for i in 0..3 {
                let sig = if hybrid {
                    qc.sign_member(&self.validators[i].id, &self.hybrid_keys[i])
                } else {
                    let ed = common::crypto::SigningKey::from_ed25519_seed(&self.ed_seeds[i]);
                    qc.sign_member(&self.validators[i].id, &ed)
                };
                qc.add_signature(sig);
            }
            qc
        }

        fn publish_authorized_tx(&self, record: ChainRecord, height: u64) -> Transaction {
            let canonical = record.id.canonical();
            let qc = self.sign_qc(
                common::APPROVAL_QC_DOMAIN,
                &canonical,
                &record.content_hash,
                &record.evidence_digest,
                height,
                height,
                None,
                None,
            );
            let authorization = self.authorization(&record);
            Transaction::PublishAuthorizedV1 {
                record,
                authorization,
                committee_qc: Some(qc),
            }
        }

        fn sign_revoke_v2(
            &self,
            signer_key: &common::crypto::SigningKey,
            canonical: &str,
            reason: &str,
            evidence_hash: &str,
            qc: Option<common::CommitteeQuorumCertificateV1>,
        ) -> Transaction {
            let mut request = common::SignedRevocationRequest {
                chain_id: self.spec.chain_id.clone(),
                genesis_hash: self.spec.genesis_hash.clone(),
                package_canonical: canonical.to_string(),
                reason: reason.to_string(),
                revoker_pubkey: signer_key.public_key().to_hex(),
                signature: String::new(),
                signature_version: 1,
            };
            request.signature = hex::encode(
                signer_key
                    .sign(request.signing_message().as_bytes())
                    .ed25519_bytes(),
            );
            Transaction::RevokeV2 {
                request,
                evidence_hash: evidence_hash.to_string(),
                witness: None,
                quorum_certificate: qc,
            }
        }

        fn sign_commit_sigs_for(
            &self,
            block_hash: &str,
            validators: &[Validator],
            ed_seeds: &[[u8; 32]],
            hybrid_keys: &[common::crypto::SigningKey],
            hybrid: bool,
        ) -> Vec<BlockSignature> {
            let msg = consensus::pbft::pbft_signature_message("commit", block_hash);
            validators
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    if hybrid {
                        let sig = hybrid_keys[i].sign(msg.as_bytes());
                        BlockSignature {
                            validator_id: v.id.clone(),
                            pubkey: v.pubkey.clone(),
                            signature: hex::encode(sig.ed25519_bytes()),
                            pq_signature: sig.mldsa44_bytes().map(hex::encode),
                        }
                    } else {
                        let ed = common::crypto::SigningKey::from_ed25519_seed(&ed_seeds[i]);
                        let sig = ed.sign(msg.as_bytes());
                        BlockSignature {
                            validator_id: v.id.clone(),
                            pubkey: v.pubkey.clone(),
                            signature: hex::encode(sig.ed25519_bytes()),
                            pq_signature: None,
                        }
                    }
                })
                .collect()
        }

        fn merkle_for_txs(
            &self,
            txs: &[Transaction],
            tx_index: usize,
        ) -> (String, String, Vec<registry_view::MerkleStepV1>) {
            let leaves: Vec<String> = txs
                .iter()
                .map(|tx| common::transaction_hash_for_encoding(tx, self.encoding()).unwrap())
                .collect();
            let tx_hash = leaves[tx_index].clone();
            let mut path = Vec::new();
            let mut idx = tx_index;
            let mut level = leaves;
            while level.len() > 1 {
                if !level.len().is_multiple_of(2) {
                    let last = level.last().unwrap().clone();
                    level.push(last);
                }
                let sibling_idx = if idx.is_multiple_of(2) {
                    idx + 1
                } else {
                    idx - 1
                };
                path.push(registry_view::MerkleStepV1 {
                    sibling_hash: level[sibling_idx].clone(),
                    is_right: idx.is_multiple_of(2),
                });
                level = level
                    .chunks(2)
                    .map(|pair| sha256_hex(format!("{}{}", pair[0], pair[1]).as_bytes()))
                    .collect();
                idx /= 2;
            }
            (level[0].clone(), tx_hash, path)
        }

        fn block_header(
            &self,
            height: u64,
            prev_hash: String,
            merkle_root: String,
            ts: DateTime<Utc>,
            status_root: Option<[u8; 32]>,
            validators: &[Validator],
        ) -> (BlockHeader, String) {
            let header = BlockHeader {
                height,
                prev_hash,
                merkle_root,
                proposer_id: validators[0].id.clone(),
                timestamp: ts,
                validator_set_hash: compute_validator_set_hash(validators),
                vrf_output: None,
                vrf_proof: None,
                view_number: 0,
                view_change_certificate: vec![],
                status_root,
            };
            let hash = Block {
                header: header.clone(),
                transactions: vec![],
                pbft_signatures: vec![],
            }
            .hash();
            (header, hash)
        }
    }

    fn build_single_block_verified_bundle(
        fx: &ProofFixture,
        canonical: &str,
        tx: Transaction,
        quorum: QuorumLabelV1,
        tip_ts: DateTime<Utc>,
    ) -> ProofBundleV2 {
        let (pkg, ver) = split_canonical(canonical).unwrap();
        let rec = tx.published_record().unwrap().clone();
        let (merkle_root, tx_hash, path) = fx.merkle_for_txs(std::slice::from_ref(&tx), 0);
        let active = fx.spec.consensus_params.vote_v4_active_at(1);
        let (status_root, status_leaf, status_proof) = if active {
            let mut tree = common::StatusTree::new();
            let leaf = common::StatusLeafV1 {
                package: pkg.to_string(),
                version: ver.to_string(),
                content_hash: rec.content_hash,
                status: StatusLeafStateV1::Verified,
                height: 1,
            };
            tree.upsert(leaf.clone());
            let root = tree.root_for_encoding(fx.encoding()).unwrap();
            let proof = tree.prove_for_encoding(pkg, ver, fx.encoding()).unwrap();
            (Some(root), Some(leaf), Some(proof))
        } else {
            (None, None, None)
        };
        let (header, hash) = fx.block_header(
            1,
            fx.genesis_hash(),
            merkle_root,
            tip_ts,
            status_root,
            &fx.validators,
        );
        let sigs =
            fx.sign_commit_sigs_for(&hash, &fx.validators, &fx.ed_seeds, &fx.hybrid_keys, active);
        ProofBundleV2 {
            schema: PROOF_BUNDLE_SCHEMA.to_string(),
            canonical: DisplayText::bounded(canonical, 512),
            tip_height: 1,
            tip_block_hash: hash.clone(),
            tip_timestamp_unix: tip_ts.timestamp(),
            tip_header: header.clone(),
            tip_commit_signatures: sigs.clone(),
            authority_history: vec![],
            status_root: ProofBundleV2::status_root_from(status_root.as_ref()),
            status_leaf,
            status_proof,
            status_transaction: None,
            publication_height: 1,
            publication_block_hash: hash,
            publication_header: header,
            publication_tx_index: 0,
            publication_tx_hash: tx_hash,
            publication_transaction: tx,
            publication_merkle_path: path,
            publication_commit_signatures: sigs,
            quorum,
        }
    }

    #[test]
    fn standalone_phase_c_proof_bundle_verifies_without_compile_feature() {
        let fx = ProofFixture::new(Some(common::PHASE_C_CONSENSUS_ENCODING), Some(1));
        assert!(fx
            .spec
            .verify_signature(&fx.sign_spec(), &fx.spec_signing_key.public_key().to_hex())
            .is_ok());
        let now = Utc::now();
        let canonical = "npm:@scope/left-pad@1.0.0";
        let rec = fx.record(canonical, PackageStatus::Verified);
        let tx = fx.publish_authorized_tx(rec, 1);
        let bundle =
            build_single_block_verified_bundle(&fx, canonical, tx, QuorumLabelV1::V4Quorum, now);
        let cp = Checkpoint::from_verified_spec(&fx.spec).unwrap();

        let verified = verify_proof_bundle(
            canonical,
            &bundle,
            &fx.spec,
            Some(&cp),
            &VerifyOptions::default(),
            now,
        )
        .expect("Phase C proof bundle should verify even without phase-c compile feature");
        assert!(verified.status.is_safe());
        assert_eq!(verified.quorum, QuorumLabelV1::V4Quorum);
        assert!(verified.certified);
        assert!(verified.status_proven_by_smt);
    }

    #[test]
    fn pre_activation_legacy_v3_quorum_honors_require_v4_quorum_option() {
        let fx = ProofFixture::new(Some(common::PHASE_C_CONSENSUS_ENCODING), Some(10));
        let now = Utc::now();
        let canonical = "npm:legacy-pkg@1.0.0";
        let rec = fx.record(canonical, PackageStatus::Verified);
        let bundle = build_single_block_verified_bundle(
            &fx,
            canonical,
            Transaction::Publish(rec),
            QuorumLabelV1::LegacyV3Quorum,
            now,
        );

        let accepted = verify_proof_bundle(
            canonical,
            &bundle,
            &fx.spec,
            None,
            &VerifyOptions {
                require_v4_quorum: false,
                ..VerifyOptions::default()
            },
            now,
        )
        .expect("decision #7 accepts legacy_v3_quorum below activation");
        assert_eq!(accepted.quorum, QuorumLabelV1::LegacyV3Quorum);
        assert!(!accepted.status_proven_by_smt);

        let rejected = verify_proof_bundle(
            canonical,
            &bundle,
            &fx.spec,
            None,
            &VerifyOptions {
                require_v4_quorum: true,
                ..VerifyOptions::default()
            },
            now,
        )
        .expect_err("require_v4_quorum must reject legacy_v3_quorum even below activation");
        assert!(rejected.to_string().contains("require_v4_quorum"));
    }

    #[test]
    fn tip_freshness_enforces_3600s_staleness_and_future_skew_bounds() {
        let fx = ProofFixture::new(Some(common::PHASE_C_CONSENSUS_ENCODING), Some(1));
        let now = Utc::now();
        let canonical = "npm:fresh-pkg@1.0.0";
        let rec = fx.record(canonical, PackageStatus::Verified);
        let tx = fx.publish_authorized_tx(rec, 1);

        let bundle_ok = build_single_block_verified_bundle(
            &fx,
            canonical,
            tx.clone(),
            QuorumLabelV1::V4Quorum,
            now - chrono::Duration::seconds(DEFAULT_MAX_TIP_AGE_SECS - 1),
        );
        assert!(verify_proof_bundle(
            canonical,
            &bundle_ok,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .is_ok());

        let bundle_stale = build_single_block_verified_bundle(
            &fx,
            canonical,
            tx.clone(),
            QuorumLabelV1::V4Quorum,
            now - chrono::Duration::seconds(DEFAULT_MAX_TIP_AGE_SECS + 1),
        );
        assert!(verify_proof_bundle(
            canonical,
            &bundle_stale,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .unwrap_err()
        .to_string()
        .contains("stale"));

        let bundle_future = build_single_block_verified_bundle(
            &fx,
            canonical,
            tx,
            QuorumLabelV1::V4Quorum,
            now + chrono::Duration::seconds(DEFAULT_MAX_TIP_FUTURE_SKEW_SECS + 1),
        );
        assert!(verify_proof_bundle(
            canonical,
            &bundle_future,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .unwrap_err()
        .to_string()
        .contains("future"));
    }

    #[test]
    fn authority_history_transition_updates_signing_authority_and_rejects_forgery() {
        let fx = ProofFixture::new(Some(common::PHASE_C_CONSENSUS_ENCODING), Some(1));
        let now = Utc::now();
        let canonical = "npm:rotated@1.0.0";
        let rec = fx.record(canonical, PackageStatus::Verified);
        let pub_tx = fx.publish_authorized_tx(rec.clone(), 1);
        let (pub_merkle, pub_tx_hash, pub_path) =
            fx.merkle_for_txs(std::slice::from_ref(&pub_tx), 0);
        let (pub_header, pub_hash) =
            fx.block_header(1, fx.genesis_hash(), pub_merkle, now, None, &fx.validators);
        let pub_sigs = fx.sign_commit_sigs_for(
            &pub_hash,
            &fx.validators,
            &fx.ed_seeds,
            &fx.hybrid_keys,
            true,
        );

        let mut rotated_validators = Vec::new();
        let mut rotated_ed_seeds = Vec::new();
        let mut rotated_hybrid_keys = Vec::new();
        for i in 0..4u8 {
            let ed_seed = [i + 120; 32];
            let pq_seed = [i + 160; 32];
            let hybrid = common::crypto::SigningKey::hybrid_from_seeds(&ed_seed, &pq_seed);
            let ed_only = common::crypto::SigningKey::from_ed25519_seed(&ed_seed);
            let pk_bytes = hybrid.public_key().to_bytes();
            rotated_validators.push(Validator {
                id: format!("rot-{i}"),
                alias: format!("rot-{i}"),
                pubkey: ed_only.public_key().to_hex(),
                eth_address: format!("0x{:040x}", i + 100),
                stake: 200,
                reputation: 100,
                status: "active".into(),
                performance_score: 500,
                contribution_count: 0,
                pq_pubkey: Some(hex::encode(
                    &pk_bytes[common::crypto::ED25519_PUBLIC_KEY_LEN..],
                )),
            });
            rotated_ed_seeds.push(ed_seed);
            rotated_hybrid_keys.push(hybrid);
        }
        let bootstrap_set = ValidatorSet::new(fx.validators.clone());
        let rotated_set = ValidatorSet::new(rotated_validators.clone());
        let mut transition = AuthenticatedValidatorSetSnapshot {
            schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height: 2,
            previous_block_hash: pub_hash.clone(),
            previous_validator_set_hash: common::canonical_validator_set_hash(&bootstrap_set),
            validators: rotated_set,
            validator_set_hash: common::canonical_validator_set_hash(&ValidatorSet::new(
                rotated_validators.clone(),
            )),
            transition_signatures: vec![],
        };
        let msg = common::validator_set_transition_message(&transition);
        for i in 0..3 {
            let ed = common::crypto::SigningKey::from_ed25519_seed(&fx.ed_seeds[i]);
            transition
                .transition_signatures
                .push(common::ValidatorSetTransitionSignature {
                    validator_id: fx.validators[i].id.clone(),
                    validator_pubkey: fx.validators[i].pubkey.clone(),
                    signature: hex::encode(ed.sign(msg.as_bytes()).ed25519_bytes()),
                });
        }

        let (pkg, ver) = split_canonical(canonical).unwrap();
        let mut tree = common::StatusTree::new();
        let leaf = common::StatusLeafV1 {
            package: pkg.to_string(),
            version: ver.to_string(),
            content_hash: rec.content_hash.clone(),
            status: StatusLeafStateV1::Verified,
            height: 1,
        };
        tree.upsert(leaf.clone());
        let root = tree.root_for_encoding(fx.encoding()).unwrap();
        let proof = tree.prove_for_encoding(pkg, ver, fx.encoding()).unwrap();
        let (tip_header, tip_hash) = fx.block_header(
            2,
            pub_hash.clone(),
            sha256_hex(b"empty"),
            now,
            Some(root),
            &rotated_validators,
        );
        let tip_sigs = fx.sign_commit_sigs_for(
            &tip_hash,
            &rotated_validators,
            &rotated_ed_seeds,
            &rotated_hybrid_keys,
            true,
        );

        let bundle = ProofBundleV2 {
            schema: PROOF_BUNDLE_SCHEMA.to_string(),
            canonical: DisplayText::bounded(canonical, 512),
            tip_height: 2,
            tip_block_hash: tip_hash,
            tip_timestamp_unix: now.timestamp(),
            tip_header,
            tip_commit_signatures: tip_sigs,
            authority_history: vec![transition],
            status_root: ProofBundleV2::status_root_from(Some(&root)),
            status_leaf: Some(leaf),
            status_proof: Some(proof),
            status_transaction: None,
            publication_height: 1,
            publication_block_hash: pub_hash,
            publication_header: pub_header,
            publication_tx_index: 0,
            publication_tx_hash: pub_tx_hash,
            publication_transaction: pub_tx,
            publication_merkle_path: pub_path,
            publication_commit_signatures: pub_sigs,
            quorum: QuorumLabelV1::V4Quorum,
        };

        assert!(verify_proof_bundle(
            canonical,
            &bundle,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .is_ok());

        let mut forged = bundle.clone();
        forged.authority_history[0].transition_signatures[0].signature = "00".repeat(64);
        assert!(verify_proof_bundle(
            canonical,
            &forged,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .is_err());
    }

    #[test]
    fn eco_adv_04_post_activation_ed25519_only_commit_qc_is_rejected() {
        let fx = ProofFixture::new(Some(common::PHASE_C_CONSENSUS_ENCODING), Some(1));
        let now = Utc::now();
        let canonical = "npm:hybrid-only@1.0.0";
        let rec = fx.record(canonical, PackageStatus::Verified);
        let tx = fx.publish_authorized_tx(rec, 1);
        let mut bundle =
            build_single_block_verified_bundle(&fx, canonical, tx, QuorumLabelV1::V4Quorum, now);

        for sig in &mut bundle.tip_commit_signatures {
            sig.pq_signature = None;
        }
        assert!(verify_proof_bundle(
            canonical,
            &bundle,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .is_err());

        let mut tolerant = build_single_block_verified_bundle(
            &fx,
            canonical,
            fx.publish_authorized_tx(fx.record(canonical, PackageStatus::Verified), 1),
            QuorumLabelV1::V4Quorum,
            now,
        );
        let mut forged_first = tolerant.tip_commit_signatures[3].clone();
        forged_first.pq_signature = None;
        tolerant.tip_commit_signatures.insert(0, forged_first);
        assert!(verify_proof_bundle(
            canonical,
            &tolerant,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .is_ok());
    }

    #[test]
    fn quarantine_qc_expiry_and_certified_release_verify() {
        let fx = ProofFixture::new(Some(common::PHASE_C_CONSENSUS_ENCODING), Some(1));
        let start = Utc::now();
        let canonical = "npm:quarantined-pkg@1.0.0";
        let rec = fx.record(canonical, PackageStatus::Verified);
        let pub_tx = fx.publish_authorized_tx(rec.clone(), 1);
        let (pub_merkle, pub_tx_hash, pub_path) =
            fx.merkle_for_txs(std::slice::from_ref(&pub_tx), 0);
        let (pub_header, pub_hash) = fx.block_header(
            1,
            fx.genesis_hash(),
            pub_merkle,
            start,
            None,
            &fx.validators,
        );
        let pub_sigs = fx.sign_commit_sigs_for(
            &pub_hash,
            &fx.validators,
            &fx.ed_seeds,
            &fx.hybrid_keys,
            true,
        );

        let mut held_rec = rec.clone();
        held_rec.status = PackageStatus::Pending;
        let hold_qc = fx.sign_qc(
            common::QUARANTINE_QC_DOMAIN,
            canonical,
            &rec.content_hash,
            &rec.evidence_digest,
            2,
            2,
            None,
            None,
        );
        let q_tx = Transaction::QuarantineV1 {
            record: held_rec,
            reason_code: CregFindingCategoryV1::CredentialExfiltration,
            quorum_certificate: hold_qc,
        };
        let deadline = start.timestamp() + 3_000;
        let (pkg, ver) = split_canonical(canonical).unwrap();
        let mut tree = common::StatusTree::new();
        let q_leaf = common::StatusLeafV1 {
            package: pkg.to_string(),
            version: ver.to_string(),
            content_hash: rec.content_hash.clone(),
            status: StatusLeafStateV1::Quarantined {
                expires_at_unix: deadline,
                reason_code: CregFindingCategoryV1::CredentialExfiltration,
            },
            height: 2,
        };
        tree.upsert(q_leaf.clone());
        let q_root = tree.root_for_encoding(fx.encoding()).unwrap();
        let q_proof = tree.prove_for_encoding(pkg, ver, fx.encoding()).unwrap();
        let (q_header, q_hash) = fx.block_header(
            2,
            pub_hash.clone(),
            sha256_hex(b"q"),
            start,
            Some(q_root),
            &fx.validators,
        );
        let q_sigs =
            fx.sign_commit_sigs_for(&q_hash, &fx.validators, &fx.ed_seeds, &fx.hybrid_keys, true);

        let q_bundle = ProofBundleV2 {
            schema: PROOF_BUNDLE_SCHEMA.to_string(),
            canonical: DisplayText::bounded(canonical, 512),
            tip_height: 2,
            tip_block_hash: q_hash,
            tip_timestamp_unix: start.timestamp(),
            tip_header: q_header,
            tip_commit_signatures: q_sigs,
            authority_history: vec![],
            status_root: ProofBundleV2::status_root_from(Some(&q_root)),
            status_leaf: Some(q_leaf),
            status_proof: Some(q_proof),
            status_transaction: Some(q_tx),
            publication_height: 1,
            publication_block_hash: pub_hash.clone(),
            publication_header: pub_header.clone(),
            publication_tx_index: 0,
            publication_tx_hash: pub_tx_hash.clone(),
            publication_transaction: pub_tx.clone(),
            publication_merkle_path: pub_path.clone(),
            publication_commit_signatures: pub_sigs.clone(),
            quorum: QuorumLabelV1::V4Quorum,
        };

        let verified_q = verify_proof_bundle(
            canonical,
            &q_bundle,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            start + chrono::Duration::seconds(100),
        )
        .expect("valid quarantine bundle must verify");
        assert!(matches!(
            verified_q.status,
            VerdictStatus::Quarantined {
                expires_at_unix: Some(d),
                ..
            } if d == deadline
        ));
        assert!(matches!(
            verified_q.status_view,
            PackageStatusViewV1::Quarantined { .. }
        ));

        let past_deadline_now = DateTime::<Utc>::from_timestamp(deadline + 1, 0).unwrap();
        let expired_q = verify_proof_bundle(
            canonical,
            &q_bundle,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            past_deadline_now,
        )
        .expect("expired quarantine still verifies");
        assert!(matches!(
            expired_q.status_view,
            PackageStatusViewV1::Revoked {
                authority: RevocationAuthorityV1::QuarantineExpiry,
                ..
            }
        ));

        let mut forged_q = q_bundle;
        if let Some(Transaction::QuarantineV1 {
            ref mut quorum_certificate,
            ..
        }) = forged_q.status_transaction
        {
            for sig in &mut quorum_certificate.signatures {
                sig.signature = "00".repeat(64);
            }
        }
        assert!(verify_proof_bundle(
            canonical,
            &forged_q,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            start
        )
        .is_err());

        let release_qc = fx.sign_qc(
            common::QUARANTINE_RESOLUTION_QC_DOMAIN,
            canonical,
            &rec.content_hash,
            &rec.evidence_digest,
            3,
            3,
            Some(QuarantineOutcomeV1::Release),
            None,
        );
        let release_tx = Transaction::QuarantineResolutionV1 {
            package_canonical: canonical.to_string(),
            content_hash: rec.content_hash.clone(),
            outcome: QuarantineOutcomeV1::Release,
            quorum_certificate: release_qc,
        };
        let mut release_tree = common::StatusTree::new();
        let rel_leaf = common::StatusLeafV1 {
            package: pkg.to_string(),
            version: ver.to_string(),
            content_hash: rec.content_hash,
            status: StatusLeafStateV1::Verified,
            height: 3,
        };
        release_tree.upsert(rel_leaf.clone());
        let rel_root = release_tree.root_for_encoding(fx.encoding()).unwrap();
        let rel_proof = release_tree
            .prove_for_encoding(pkg, ver, fx.encoding())
            .unwrap();
        let (rel_header, rel_hash) = fx.block_header(
            3,
            "11".repeat(32),
            sha256_hex(b"r"),
            start,
            Some(rel_root),
            &fx.validators,
        );
        let rel_sigs = fx.sign_commit_sigs_for(
            &rel_hash,
            &fx.validators,
            &fx.ed_seeds,
            &fx.hybrid_keys,
            true,
        );
        let release_bundle = ProofBundleV2 {
            schema: PROOF_BUNDLE_SCHEMA.to_string(),
            canonical: DisplayText::bounded(canonical, 512),
            tip_height: 3,
            tip_block_hash: rel_hash,
            tip_timestamp_unix: start.timestamp(),
            tip_header: rel_header,
            tip_commit_signatures: rel_sigs,
            authority_history: vec![],
            status_root: ProofBundleV2::status_root_from(Some(&rel_root)),
            status_leaf: Some(rel_leaf),
            status_proof: Some(rel_proof),
            status_transaction: Some(release_tx),
            publication_height: 1,
            publication_block_hash: pub_hash,
            publication_header: pub_header,
            publication_tx_index: 0,
            publication_tx_hash: pub_tx_hash,
            publication_transaction: pub_tx,
            publication_merkle_path: pub_path,
            publication_commit_signatures: pub_sigs,
            quorum: QuorumLabelV1::V4Quorum,
        };
        let verified_rel = verify_proof_bundle(
            canonical,
            &release_bundle,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            start,
        )
        .expect("certified release should verify");
        assert!(verified_rel.status.is_safe());
        assert!(verified_rel.certified);
    }

    #[test]
    fn eco_adv_05_single_validator_revoke_v2_without_qc_post_activation_is_rejected_and_uncertified(
    ) {
        let fx = ProofFixture::new(Some(common::PHASE_C_CONSENSUS_ENCODING), Some(1));
        let now = Utc::now();
        let canonical = "npm:rev-target@1.0.0";
        let rec = fx.record(canonical, PackageStatus::Verified);
        let pub_tx = fx.publish_authorized_tx(rec.clone(), 1);
        let (pub_merkle, pub_tx_hash, pub_path) =
            fx.merkle_for_txs(std::slice::from_ref(&pub_tx), 0);
        let (pub_header, pub_hash) =
            fx.block_header(1, fx.genesis_hash(), pub_merkle, now, None, &fx.validators);
        let pub_sigs = fx.sign_commit_sigs_for(
            &pub_hash,
            &fx.validators,
            &fx.ed_seeds,
            &fx.hybrid_keys,
            true,
        );
        let (pkg, ver) = split_canonical(canonical).unwrap();

        let make_rev_bundle = |certified_leaf: bool, rev_tx: Transaction| {
            let mut tree = common::StatusTree::new();
            let leaf = common::StatusLeafV1 {
                package: pkg.to_string(),
                version: ver.to_string(),
                content_hash: rec.content_hash.clone(),
                status: StatusLeafStateV1::Revoked {
                    certified: certified_leaf,
                },
                height: 2,
            };
            tree.upsert(leaf.clone());
            let root = tree.root_for_encoding(fx.encoding()).unwrap();
            let proof = tree.prove_for_encoding(pkg, ver, fx.encoding()).unwrap();
            let (tip_header, tip_hash) = fx.block_header(
                2,
                pub_hash.clone(),
                sha256_hex(b"rev"),
                now,
                Some(root),
                &fx.validators,
            );
            let tip_sigs = fx.sign_commit_sigs_for(
                &tip_hash,
                &fx.validators,
                &fx.ed_seeds,
                &fx.hybrid_keys,
                true,
            );
            ProofBundleV2 {
                schema: PROOF_BUNDLE_SCHEMA.to_string(),
                canonical: DisplayText::bounded(canonical, 512),
                tip_height: 2,
                tip_block_hash: tip_hash,
                tip_timestamp_unix: now.timestamp(),
                tip_header,
                tip_commit_signatures: tip_sigs,
                authority_history: vec![],
                status_root: ProofBundleV2::status_root_from(Some(&root)),
                status_leaf: Some(leaf),
                status_proof: Some(proof),
                status_transaction: Some(rev_tx),
                publication_height: 1,
                publication_block_hash: pub_hash.clone(),
                publication_header: pub_header.clone(),
                publication_tx_index: 0,
                publication_tx_hash: pub_tx_hash.clone(),
                publication_transaction: pub_tx.clone(),
                publication_merkle_path: pub_path.clone(),
                publication_commit_signatures: pub_sigs.clone(),
                quorum: QuorumLabelV1::V4Quorum,
            }
        };

        let single_val_key = common::crypto::SigningKey::from_ed25519_seed(&fx.ed_seeds[0]);
        let single_val_tx = fx.sign_revoke_v2(
            &single_val_key,
            canonical,
            "single validator malicious yank",
            &rec.evidence_digest,
            None,
        );
        let err_bundle = make_rev_bundle(true, single_val_tx.clone());
        assert!(verify_proof_bundle(
            canonical,
            &err_bundle,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .is_err());

        let uncertified_bundle = make_rev_bundle(false, single_val_tx);
        let verified_uncert = verify_proof_bundle(
            canonical,
            &uncertified_bundle,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now,
        )
        .expect("uncertified leaf classifies cleanly as non-sticky");
        assert!(!verified_uncert.certified);

        let owner_tx = fx.sign_revoke_v2(
            &fx.publisher_key,
            canonical,
            "owner deprecation",
            &rec.evidence_digest,
            None,
        );
        let owner_bundle = make_rev_bundle(true, owner_tx);
        let verified_owner = verify_proof_bundle(
            canonical,
            &owner_bundle,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now,
        )
        .expect("owner self-revocation is certified");
        assert!(verified_owner.certified);
        assert!(matches!(
            verified_owner.status_view,
            PackageStatusViewV1::Revoked {
                authority: RevocationAuthorityV1::Owner,
                ..
            }
        ));

        let rev_qc = fx.sign_qc(
            common::REVOKE_QC_DOMAIN,
            canonical,
            &rec.content_hash,
            &rec.evidence_digest,
            2,
            2,
            None,
            None,
        );
        let qc_tx = fx.sign_revoke_v2(
            &single_val_key,
            canonical,
            "committee confirmed malware",
            &rec.evidence_digest,
            Some(rev_qc),
        );
        let qc_bundle = make_rev_bundle(true, qc_tx);
        let verified_qc = verify_proof_bundle(
            canonical,
            &qc_bundle,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now,
        )
        .expect("committee QC revocation is certified");
        assert!(verified_qc.certified);
        assert!(matches!(
            verified_qc.status_view,
            PackageStatusViewV1::Revoked {
                authority: RevocationAuthorityV1::CommitteeQuorum,
                ..
            }
        ));

        let lying_rest = TrustVerdict {
            package: rec.id,
            status: VerdictStatus::Verified {
                block_hash: pub_hash,
                content_hash: rec.content_hash,
                ipfs_cid: rec.ipfs_cid,
                findings: vec![],
            },
            resolved_at: now,
            source: common::VerdictSource::Chain {
                node_url: "https://node.example".into(),
            },
            deterministic_risk: None,
            evidence_phase: None,
            block_phase: None,
            chain_status: None,
            publisher: None,
        };
        let joined =
            crate::install_gate::reconcile_verdict_with_proof(&lying_rest, Ok(&verified_qc), now);
        let decision = crate::install_gate::evaluate_lattice_at(canonical, &joined, true, now);
        assert!(matches!(
            decision,
            crate::install_gate::InstallDecision::Block {
                code: "revoked_certified",
                ..
            }
        ));
    }

    #[test]
    fn status_proof_tampering_or_omission_post_activation_fails() {
        let fx = ProofFixture::new(Some(common::PHASE_C_CONSENSUS_ENCODING), Some(1));
        let now = Utc::now();
        let canonical = "npm:tampered-smt@1.0.0";
        let rec = fx.record(canonical, PackageStatus::Verified);
        let tx = fx.publish_authorized_tx(rec, 1);
        let bundle =
            build_single_block_verified_bundle(&fx, canonical, tx, QuorumLabelV1::V4Quorum, now);

        let mut missing_proof = bundle.clone();
        missing_proof.status_proof = None;
        assert!(verify_proof_bundle(
            canonical,
            &missing_proof,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .is_err());

        let mut tampered_leaf = bundle;
        tampered_leaf.status_leaf.as_mut().unwrap().status =
            StatusLeafStateV1::Revoked { certified: true };
        assert!(verify_proof_bundle(
            canonical,
            &tampered_leaf,
            &fx.spec,
            None,
            &VerifyOptions::default(),
            now
        )
        .is_err());
    }
}
