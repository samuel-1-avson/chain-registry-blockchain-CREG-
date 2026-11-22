// crates/consensus/src/pbft.rs
// Three-phase PBFT: PRE-PREPARE, PREPARE, COMMIT.
// Safety guarantee: the network is correct as long as fewer than ⌊n/3⌋
// validators are faulty or Byzantine.

use crate::ValidatorSet;
use anyhow::{bail, Result};
use common::{
    Block, BlockSignature, PreparedBlockHeader, PreparedCertificate, ReproposalProof,
    ViewChangeVote,
};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Default maximum time a round can stay in any single phase before it is
/// considered timed-out. Overridden via `CREG_PBFT_TIMEOUT` env var.
const DEFAULT_ROUND_PHASE_TIMEOUT_SECS: u64 = 30;

/// Default maximum number of view-change retries before a round is abandoned.
/// Overridden via `CREG_PBFT_MAX_VIEW_CHANGES` env var.
const DEFAULT_MAX_VIEW_CHANGES: u32 = 3;

/// Default age after which a terminal (Finalised / Failed) round is eligible
/// for garbage collection. Overridden via `CREG_PBFT_STALE_TTL` env var.
const DEFAULT_STALE_ROUND_TTL_SECS: u64 = 120;

/// Whether three-validator clusters may use a simple-majority quorum instead
/// of strict PBFT quorum. Disabled by default and intended for explicit local
/// bootstrap opt-in only.
const DEFAULT_ALLOW_SMALL_CLUSTER_QUORUM: bool = false;

/// Maximum nested proof recursion depth for view-change certificates.
const MAX_VIEW_CHANGE_PROOF_DEPTH: usize = 16;
/// Memory caps for tracked PBFT engine collections (VNA-23).
pub const MAX_TRACKED_ROUNDS: usize = 256;
pub const MAX_VIEW_CHANGE_BUCKETS: usize = 256;
pub const MAX_CERTIFIED_VIEWS: usize = 256;
pub const MAX_EQUIVOCATED_VIEW_VOTERS: usize = 4_096;

const PBFT_SIGNATURE_DOMAIN: &str = "creg-pbft-v1";

/// Canonical message signed by validators during PBFT block consensus.
pub fn pbft_signature_message(phase: &str, block_hash: &str) -> String {
    format!("{}:{}:{}", PBFT_SIGNATURE_DOMAIN, phase, block_hash)
}

/// Signed by the authorized replacement primary over an immutable prepared
/// block and the exact quorum certificate that permits its retransmission.
pub fn reproposal_signing_message(
    block_hash: &str,
    target_view: u32,
    leader_id: &str,
    certificate: &[ViewChangeVote],
) -> String {
    let bytes = common::consensus_json_bytes(&(
        "creg-pbft-reproposal-v1",
        block_hash,
        target_view,
        leader_id,
        certificate,
    ))
    .expect("PBFT reproposal payload must be serializable");
    common::sha256_hex(&bytes)
}

/// Verify an Ed25519 PBFT phase signature against a validator pubkey.
///
/// Verification is strict (`verify_strict`): it additionally rejects weak
/// (small-order) public keys and signature points, which plain `verify`
/// accepts. Honest signers never produce them, so a quorum of honest
/// validators is unaffected.
pub fn verify_pbft_phase_signature(
    phase: &str,
    block_hash: &str,
    pubkey_hex: &str,
    signature_hex: &str,
) -> Result<()> {
    use common::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};

    let expected_pubkey = pubkey_hex.trim_start_matches("0x").to_ascii_lowercase();
    let pubkey_bytes = hex::decode(&expected_pubkey)
        .map_err(|e| anyhow::anyhow!("Invalid validator pubkey hex for PBFT {}: {}", phase, e))?;
    let verifying_key = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid validator pubkey for PBFT {}: {}", phase, e))?;

    let signature_bytes = hex::decode(signature_hex.trim_start_matches("0x"))
        .map_err(|e| anyhow::anyhow!("Invalid PBFT {} signature hex: {}", phase, e))?;
    let signature = Signature::from_bytes(SignatureScheme::Ed25519, &signature_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid PBFT {} signature format: {}", phase, e))?;

    let message = pbft_signature_message(phase, block_hash);
    verifying_key
        .verify(message.as_bytes(), &signature, VerifyMode::Strict)
        .map_err(|e| anyhow::anyhow!("Invalid PBFT {} signature: {}", phase, e))
}

/// Configuration for PBFT consensus parameters.
/// All values have sensible defaults and can be overridden via environment
/// variables at startup.
#[derive(Debug, Clone)]
pub struct PbftConfig {
    pub round_phase_timeout: Duration,
    pub max_view_changes: u32,
    pub stale_round_ttl: Duration,
    pub allow_small_cluster_quorum: bool,
}

impl Default for PbftConfig {
    fn default() -> Self {
        Self {
            round_phase_timeout: Duration::from_secs(
                std::env::var("CREG_PBFT_TIMEOUT")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_ROUND_PHASE_TIMEOUT_SECS),
            ),
            max_view_changes: std::env::var("CREG_PBFT_MAX_VIEW_CHANGES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_VIEW_CHANGES),
            stale_round_ttl: Duration::from_secs(
                std::env::var("CREG_PBFT_STALE_TTL")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_STALE_ROUND_TTL_SECS),
            ),
            allow_small_cluster_quorum: env_flag(
                "CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM",
                DEFAULT_ALLOW_SMALL_CLUSTER_QUORUM,
            ),
        }
    }
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        })
        .unwrap_or(default)
}

/// Current phase of a PBFT round for a given block proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PbftPhase {
    PrePrepare,
    Prepare,
    Commit,
    Finalised,
    Failed,
}

impl PbftPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrePrepare => "pre_prepare",
            Self::Prepare => "prepare",
            Self::Commit => "commit",
            Self::Finalised => "finalised",
            Self::Failed => "failed",
        }
    }

    /// Numeric encoding for Prometheus gauges and alert rules.
    pub fn code(&self) -> i32 {
        match self {
            Self::PrePrepare => 1,
            Self::Prepare => 2,
            Self::Commit => 3,
            Self::Finalised => 4,
            Self::Failed => 5,
        }
    }
}

/// Snapshot of one PBFT block round for HTTP metrics and observability.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PbftRoundSnapshot {
    pub block_hash: String,
    pub block_height: u64,
    pub phase: String,
    pub phase_code: i32,
    pub phase_age_seconds: f64,
    pub prepare_count: usize,
    pub commit_count: usize,
    pub quorum: usize,
    pub validator_count: usize,
    pub view_number: u32,
    pub view_change_count: u32,
}

/// A view-change signal returned by `timeout_rounds()` so the block producer
/// or P2P layer can broadcast the certificate to peers.
#[derive(Debug, Clone)]
pub struct ViewChangeSignal {
    /// Unsigned local vote. The node signs it and feeds it through
    /// `receive_view_change` before gossiping.
    pub vote: ViewChangeVote,
}

/// One of our own votes eligible for rebroadcast, returned by
/// `own_votes_for_rebroadcast()`. Gossipsub delivery is best-effort: votes
/// sent while the mesh converges can be silently lost, stranding the round
/// at quorum-1 forever. Re-publishing our stored signatures heals that.
#[derive(Debug, Clone)]
pub struct OwnVoteForRebroadcast {
    /// Hash of the block whose round holds our vote.
    pub block_hash: String,
    /// Height of that block.
    pub block_height: u64,
    /// Parent hash for durable signing-lock lookup.
    pub prev_hash: String,
    /// View in which this block was proposed.
    pub view_number: u32,
    /// Current round phase.
    pub phase: PbftPhase,
    /// Our PREPARE signature, if we cast one.
    pub prepare_sig: Option<BlockSignature>,
    /// Our COMMIT signature, if we cast one.
    pub commit_sig: Option<BlockSignature>,
    /// How long the round has sat in the current phase.
    pub phase_age_seconds: f64,
    /// Phase timeout configured for the round.
    pub phase_timeout_seconds: f64,
}

/// State of a single PBFT consensus round.
pub struct PbftRound {
    pub block: Block,
    pub phase: PbftPhase,
    /// validator_id → their PREPARE message signature
    pub prepare_sigs: HashMap<String, BlockSignature>,
    /// validator_id → their COMMIT message signature
    pub commit_sigs: HashMap<String, BlockSignature>,
    pub validator_set: ValidatorSet,
    /// Wall-clock time the current phase was entered.
    pub phase_entered_at: Instant,
    /// Monotonically increasing view number (incremented on view-change).
    pub view_number: u32,
    /// How many view-changes have occurred for this round.
    pub view_change_count: u32,
    /// When the round was first created (for stale-round GC).
    pub created_at: Instant,
    /// Runtime configuration for timeouts and view-change limits.
    pub config: PbftConfig,
    /// Proof that this immutable block was carried into a later PBFT view.
    pub reproposal_proof: Option<ReproposalProof>,
}

impl PbftRound {
    pub fn new(block: Block, validator_set: ValidatorSet) -> Self {
        Self::with_config(block, validator_set, PbftConfig::default())
    }

    pub fn with_config(block: Block, validator_set: ValidatorSet, config: PbftConfig) -> Self {
        let now = Instant::now();
        let view_number = block.header.view_number;
        Self {
            block,
            phase: PbftPhase::PrePrepare,
            prepare_sigs: HashMap::new(),
            commit_sigs: HashMap::new(),
            validator_set,
            phase_entered_at: now,
            view_number,
            view_change_count: view_number,
            created_at: now,
            config,
            reproposal_proof: None,
        }
    }

    /// Quorum threshold.
    ///
    /// PBFT requires unanimity in a three-validator set to preserve the usual
    /// Byzantine tolerance guarantees. Local bootstrap can opt into a 2-of-3
    /// majority via `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=true`.
    pub fn quorum(&self) -> usize {
        quorum_threshold(
            self.validator_set.len(),
            self.config.allow_small_cluster_quorum,
        )
    }

    // ── Phase 1: PRE-PREPARE ─────────────────────────────────────────────────
    /// The primary (proposer) broadcasts the block. Other validators verify
    /// the block hash and move to PREPARE.
    pub fn pre_prepare(&mut self, proposer_id: &str) -> Result<String> {
        if self.phase != PbftPhase::PrePrepare {
            bail!("Not in PRE-PREPARE phase");
        }
        if !self.validator_set.is_member(proposer_id) {
            bail!("Proposer {} is not in the validator set", proposer_id);
        }
        // If the block includes a VRF proof, verify it. Proposer selection itself
        // remains deterministic until VRF proof propagation is synchronized.
        if let (Some(ref proof), Some(ref output)) =
            (&self.block.header.vrf_proof, &self.block.header.vrf_output)
        {
            let validator = self
                .validator_set
                .validators
                .get(proposer_id)
                .ok_or_else(|| {
                    anyhow::anyhow!("Proposer {} not found in validator set", proposer_id)
                })?;
            let epoch_seed = &self.block.header.prev_hash;
            crate::vrf::verify(epoch_seed.as_bytes(), &validator.pubkey, output, proof).map_err(
                |e| {
                    anyhow::anyhow!(
                        "VRF verification failed for proposer {}: {}",
                        proposer_id,
                        e
                    )
                },
            )?;
        }

        let prepared = verify_view_change_certificate(
            &self.block.header,
            &self.validator_set,
            self.config.allow_small_cluster_quorum,
        )?;
        if let Some(prepared) = prepared {
            // A PBFT COMMIT signs the exact block hash, including the original
            // proposer, timestamp, and view. Carrying only the same transaction
            // root into a new header would let two different hashes finalise
            // at one height. Until the proposal envelope can authorize a new
            // primary while preserving the prepared block hash, fail closed.
            if self.block.hash() != prepared_block_hash(&prepared.header)? {
                bail!("view-change proposer changed the highest prepared block hash");
            }
        }
        let selected = selected_proposer_for_view(
            &self.validator_set,
            &self.block.header.prev_hash,
            self.block.header.view_number,
        )?;
        if &selected != proposer_id {
            bail!(
                "Proposer {} is not the selected proposer for view {} (expected {})",
                proposer_id,
                self.block.header.view_number,
                selected
            );
        }

        // Broadcast the block hash — validators use this as the message digest.
        let block_hash = self.block.hash();
        tracing::info!(
            "[PBFT] PRE-PREPARE: block {} from {} in view {}",
            &block_hash[..12],
            proposer_id,
            self.block.header.view_number
        );
        self.phase = PbftPhase::Prepare;
        self.phase_entered_at = Instant::now();
        Ok(block_hash)
    }

    // ── Phase 2: PREPARE ─────────────────────────────────────────────────────
    /// A validator casts its PREPARE vote (approve or reject) over the block hash.
    pub fn receive_prepare(&mut self, validator_id: &str, sig: BlockSignature) -> Result<bool> {
        if self.phase != PbftPhase::Prepare && self.phase != PbftPhase::Commit {
            bail!("Not in PREPARE phase");
        }
        self.verify_block_signature("prepare", validator_id, &sig)?;
        self.prepare_sigs.insert(validator_id.to_string(), sig);
        if self.phase == PbftPhase::Commit {
            return Ok(true);
        }
        tracing::debug!(
            "[PBFT] PREPARE: {}/{} votes",
            self.prepare_sigs.len(),
            self.quorum()
        );

        if self.prepare_sigs.len() >= self.quorum() {
            self.phase = PbftPhase::Commit;
            self.phase_entered_at = Instant::now();
            tracing::info!("[PBFT] PREPARE quorum reached — moving to COMMIT");
            return Ok(true); // caller should now broadcast COMMIT
        }
        Ok(false)
    }

    // ── Phase 3: COMMIT ──────────────────────────────────────────────────────
    /// A validator sends its COMMIT signature. Once quorum is reached the
    /// block is finalised and can be written to the chain.
    pub fn receive_commit(&mut self, validator_id: &str, sig: BlockSignature) -> Result<bool> {
        if self.phase != PbftPhase::Commit {
            bail!("Not in COMMIT phase");
        }
        self.verify_block_signature("commit", validator_id, &sig)?;
        self.commit_sigs.insert(validator_id.to_string(), sig);
        tracing::debug!(
            "[PBFT] COMMIT: {}/{} votes",
            self.commit_sigs.len(),
            self.quorum()
        );

        if self.commit_sigs.len() >= self.quorum() {
            let approvals = self.commit_sigs.len();

            if approvals >= self.quorum() {
                self.phase = PbftPhase::Finalised;
                tracing::info!(
                    "[PBFT] FINALISED block {} ({} approvals / {} commits)",
                    &self.block.hash()[..12],
                    approvals,
                    self.commit_sigs.len()
                );
                return Ok(true);
            } else {
                self.phase = PbftPhase::Failed;
                tracing::warn!(
                    "[PBFT] FAILED — insufficient approvals ({}/{})",
                    approvals,
                    self.quorum()
                );
                return Ok(false);
            }
        }
        Ok(false)
    }

    /// Pipelined fast-path helper: If prepare quorum is met and the caller provides
    /// an auto-commit signature, applies commit and checks if the block reaches immediate finality.
    pub fn try_fast_commit(
        &mut self,
        validator_id: &str,
        prepare_sig: BlockSignature,
        commit_sig: BlockSignature,
    ) -> Result<bool> {
        let reached_prepare_quorum = self.receive_prepare(validator_id, prepare_sig)?;
        if reached_prepare_quorum || self.phase == PbftPhase::Commit {
            return self.receive_commit(validator_id, commit_sig);
        }
        Ok(false)
    }

    fn verify_block_signature(
        &self,
        phase: &str,
        validator_id: &str,
        sig: &BlockSignature,
    ) -> Result<()> {
        if sig.validator_id != validator_id {
            bail!(
                "PBFT {} signature validator mismatch: envelope={} signature={}",
                phase,
                validator_id,
                sig.validator_id
            );
        }

        let validator = self
            .validator_set
            .validators
            .get(validator_id)
            .ok_or_else(|| {
                anyhow::anyhow!("Validator {} is not in the validator set", validator_id)
            })?;
        if !validator.is_active {
            bail!("Validator {} is not active", validator_id);
        }

        let expected_pubkey = validator
            .pubkey
            .trim_start_matches("0x")
            .to_ascii_lowercase();
        let supplied_pubkey = sig.pubkey.trim_start_matches("0x").to_ascii_lowercase();
        if supplied_pubkey != expected_pubkey {
            bail!(
                "PBFT {} signature pubkey mismatch for validator {}",
                phase,
                validator_id
            );
        }

        verify_pbft_phase_signature(phase, &self.block.hash(), &expected_pubkey, &sig.signature)
    }

    /// Returns the finalised signatures to embed in the ChainRecord.
    pub fn finalised_signatures(&self) -> Vec<BlockSignature> {
        self.commit_sigs.values().cloned().collect()
    }

    /// Returns `true` when the current (non-terminal) phase has exceeded
    /// the configured round phase timeout.
    pub fn is_phase_timed_out(&self) -> bool {
        match self.phase {
            PbftPhase::Finalised | PbftPhase::Failed => false,
            _ => self.phase_entered_at.elapsed() > self.config.round_phase_timeout,
        }
    }

    /// Whether this round is in a terminal state (Finalised or Failed).
    pub fn is_terminal(&self) -> bool {
        matches!(self.phase, PbftPhase::Finalised | PbftPhase::Failed)
    }
}

/// Top-level engine managing multiple concurrent PBFT rounds (one per pending block).
pub struct PbftEngine {
    rounds: HashMap<String, PbftRound>, // block_hash → round
    config: PbftConfig,
    view_change_votes: HashMap<(u64, String, u32), HashMap<String, ViewChangeVote>>,
    equivocated_view_change_voters: HashSet<(u64, String, u32, String)>,
    certified_views: HashMap<(u64, String), Vec<ViewChangeVote>>,
}

fn quorum_threshold(validator_count: usize, allow_small_cluster_quorum: bool) -> usize {
    match validator_count {
        0 => 0,
        3 if allow_small_cluster_quorum => 2,
        n => (2 * n / 3) + 1,
    }
}

fn selected_proposer_for_view(
    validator_set: &ValidatorSet,
    prev_hash: &str,
    view_number: u32,
) -> Result<String> {
    let active: Vec<crate::vrf::VrfValidator> = validator_set
        .validators
        .values()
        .filter(|validator| validator.is_active)
        .map(|validator| crate::vrf::VrfValidator {
            id: validator.id.clone(),
            pubkey: validator.pubkey.clone(),
            vrf_output: None,
            vrf_proof: None,
        })
        .collect();
    let ranking = crate::vrf::rank_proposers(&active, prev_hash);
    if ranking.is_empty() {
        bail!("No active validators to select proposer");
    }
    Ok(ranking[(view_number as usize) % ranking.len()].clone())
}

fn prepared_block_hash(header: &PreparedBlockHeader) -> Result<String> {
    let bytes = serde_json::to_vec(header)?;
    Ok(common::sha256_hex(&bytes))
}

fn verify_message_signature(pubkey_hex: &str, message: &str, signature_hex: &str) -> Result<()> {
    use common::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};

    let pubkey = hex::decode(pubkey_hex.trim_start_matches("0x"))?;
    let key = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey)?;
    let signature = hex::decode(signature_hex.trim_start_matches("0x"))?;
    let signature = Signature::from_bytes(SignatureScheme::Ed25519, &signature)?;
    key.verify(message.as_bytes(), &signature, VerifyMode::Strict)?;
    Ok(())
}

pub fn verify_prepared_certificate(
    vote: &ViewChangeVote,
    certificate: &PreparedCertificate,
    validator_set: &ValidatorSet,
    allow_small_cluster_quorum: bool,
) -> Result<()> {
    verify_prepared_certificate_with_depth(
        vote,
        certificate,
        validator_set,
        allow_small_cluster_quorum,
        0,
    )
}

fn verify_prepared_certificate_with_depth(
    vote: &ViewChangeVote,
    certificate: &PreparedCertificate,
    validator_set: &ValidatorSet,
    allow_small_cluster_quorum: bool,
    depth: usize,
) -> Result<()> {
    if depth >= MAX_VIEW_CHANGE_PROOF_DEPTH {
        bail!("prepared certificate recursion depth exceeded");
    }
    let header = &certificate.header;
    if header.height != vote.height || header.prev_hash != vote.prev_hash {
        bail!("prepared certificate is for a different height or parent");
    }
    if certificate.prepared_view < header.view_number || certificate.prepared_view >= vote.new_view
    {
        bail!("prepared certificate view must be lower than the target view");
    }
    if common::merkle_root(&certificate.transactions) != header.merkle_root {
        bail!("prepared certificate transaction root is invalid");
    }
    let block_hash = prepared_block_hash(header)?;
    let original = certificate.block();
    if original.hash() != block_hash {
        bail!("prepared certificate does not reconstruct its original block");
    }
    let original_prepared = verify_view_change_certificate_with_depth(
        &original.header,
        validator_set,
        allow_small_cluster_quorum,
        depth + 1,
    )?;
    if original_prepared
        .as_ref()
        .is_some_and(|earlier| earlier.block().hash() != block_hash)
    {
        bail!("original view proof does not preserve the prepared block hash");
    }
    let selected =
        selected_proposer_for_view(validator_set, &header.prev_hash, header.view_number)?;
    if header.proposer_id != selected {
        bail!("prepared block has an unauthorized original proposer");
    }
    if certificate.prepared_view > header.view_number {
        let proof = certificate.reproposal_proof.as_deref().ok_or_else(|| {
            anyhow::anyhow!("later prepared view lacks a signed reproposal proof")
        })?;
        if proof.target_view != certificate.prepared_view {
            bail!("reproposal proof targets a different prepared view");
        }
        verify_reproposal_proof_with_depth(
            &original,
            proof,
            validator_set,
            allow_small_cluster_quorum,
            depth + 1,
        )?;
    } else if certificate.reproposal_proof.is_some() {
        bail!("original-view prepared certificate has an unexpected reproposal proof");
    }
    let quorum = quorum_threshold(validator_set.len(), allow_small_cluster_quorum);
    let mut seen = HashSet::new();
    for signature in &certificate.prepare_signatures {
        if !seen.insert(signature.validator_id.clone()) {
            bail!("prepared certificate contains a duplicate validator identity");
        }
        let validator = validator_set
            .validators
            .get(&signature.validator_id)
            .filter(|validator| validator.is_active)
            .ok_or_else(|| anyhow::anyhow!("prepared certificate names an inactive validator"))?;
        if !validator
            .pubkey
            .trim_start_matches("0x")
            .eq_ignore_ascii_case(signature.pubkey.trim_start_matches("0x"))
        {
            bail!("prepared certificate public key does not match validator authority");
        }
        verify_pbft_phase_signature(
            "prepare",
            &block_hash,
            &validator.pubkey,
            &signature.signature,
        )?;
    }
    if seen.len() < quorum {
        bail!(
            "prepared certificate has {}/{} PREPARE signatures",
            seen.len(),
            quorum
        );
    }
    Ok(())
}

pub fn verify_view_change_vote(
    vote: &ViewChangeVote,
    validator_set: &ValidatorSet,
    allow_small_cluster_quorum: bool,
) -> Result<()> {
    verify_view_change_vote_with_depth(vote, validator_set, allow_small_cluster_quorum, 0)
}

fn verify_view_change_vote_with_depth(
    vote: &ViewChangeVote,
    validator_set: &ValidatorSet,
    allow_small_cluster_quorum: bool,
    depth: usize,
) -> Result<()> {
    if depth >= MAX_VIEW_CHANGE_PROOF_DEPTH {
        bail!("view-change vote recursion depth exceeded");
    }
    if vote.new_view == 0 || vote.prev_hash.is_empty() || vote.height == 0 {
        bail!("view-change vote has an invalid height, parent, or view");
    }
    let validator = validator_set
        .validators
        .get(&vote.validator_id)
        .filter(|validator| validator.is_active)
        .ok_or_else(|| anyhow::anyhow!("view-change vote is from an inactive validator"))?;
    verify_message_signature(&validator.pubkey, &vote.signing_message(), &vote.signature)
        .map_err(|error| anyhow::anyhow!("invalid view-change signature: {error}"))?;
    if let Some(prepared) = &vote.prepared {
        verify_prepared_certificate_with_depth(
            vote,
            prepared,
            validator_set,
            allow_small_cluster_quorum,
            depth + 1,
        )?;
    }
    Ok(())
}

/// Verify a quorum-certified view change and return the highest prepared value
/// that the replacement proposer must carry forward.
pub fn verify_view_change_certificate(
    header: &common::BlockHeader,
    validator_set: &ValidatorSet,
    allow_small_cluster_quorum: bool,
) -> Result<Option<PreparedCertificate>> {
    verify_view_change_certificate_with_depth(header, validator_set, allow_small_cluster_quorum, 0)
}

fn verify_view_change_certificate_with_depth(
    header: &common::BlockHeader,
    validator_set: &ValidatorSet,
    allow_small_cluster_quorum: bool,
    depth: usize,
) -> Result<Option<PreparedCertificate>> {
    if depth >= MAX_VIEW_CHANGE_PROOF_DEPTH {
        bail!("view-change certificate recursion depth exceeded");
    }
    if header.view_number == 0 {
        if !header.view_change_certificate.is_empty() {
            bail!("view zero must not carry a view-change certificate");
        }
        return Ok(None);
    }

    let quorum = quorum_threshold(validator_set.len(), allow_small_cluster_quorum);
    let mut seen = HashSet::new();
    let mut highest: Option<PreparedCertificate> = None;
    let mut highest_hash: Option<String> = None;
    for vote in &header.view_change_certificate {
        if vote.height != header.height
            || vote.prev_hash != header.prev_hash
            || vote.new_view != header.view_number
        {
            bail!("view-change certificate contains a vote for another view context");
        }
        if !seen.insert(vote.validator_id.clone()) {
            bail!("view-change certificate contains a duplicate validator identity");
        }
        verify_view_change_vote_with_depth(
            vote,
            validator_set,
            allow_small_cluster_quorum,
            depth + 1,
        )?;

        if let Some(prepared) = &vote.prepared {
            let prepared_hash = prepared_block_hash(&prepared.header)?;
            match highest.as_ref() {
                None => {
                    highest_hash = Some(prepared_hash);
                    highest = Some(prepared.clone());
                }
                Some(current) if prepared.prepared_view > current.prepared_view => {
                    highest_hash = Some(prepared_hash);
                    highest = Some(prepared.clone());
                }
                Some(current) if prepared.prepared_view == current.prepared_view => {
                    if highest_hash.as_deref() != Some(prepared_hash.as_str()) {
                        bail!("conflicting prepared values reported for the same PBFT view");
                    }
                }
                Some(_) => {}
            }
        }
    }
    if seen.len() < quorum {
        bail!(
            "view-change certificate has {}/{} votes",
            seen.len(),
            quorum
        );
    }
    Ok(highest)
}

pub fn verify_reproposal_proof(
    block: &Block,
    proof: &ReproposalProof,
    validator_set: &ValidatorSet,
    allow_small_cluster_quorum: bool,
) -> Result<()> {
    verify_reproposal_proof_with_depth(block, proof, validator_set, allow_small_cluster_quorum, 0)
}

fn verify_reproposal_proof_with_depth(
    block: &Block,
    proof: &ReproposalProof,
    validator_set: &ValidatorSet,
    allow_small_cluster_quorum: bool,
    depth: usize,
) -> Result<()> {
    if depth >= MAX_VIEW_CHANGE_PROOF_DEPTH {
        bail!("reproposal proof recursion depth exceeded");
    }
    if proof.target_view <= block.header.view_number {
        bail!("reproposal proof does not advance the original block view");
    }
    let mut check_header = block.header.clone();
    check_header.view_number = proof.target_view;
    check_header.view_change_certificate = proof.view_change_certificate.clone();
    let highest = verify_view_change_certificate_with_depth(
        &check_header,
        validator_set,
        allow_small_cluster_quorum,
        depth + 1,
    )?
    .ok_or_else(|| anyhow::anyhow!("reproposal proof has no prepared value"))?;
    let hash = block.hash();
    if highest.block().hash() != hash {
        bail!("reproposal proof changed the highest prepared block hash");
    }
    let selected =
        selected_proposer_for_view(validator_set, &block.header.prev_hash, proof.target_view)?;
    if proof.leader_id != selected {
        bail!("reproposal proof has an unauthorized replacement primary");
    }
    let leader = validator_set
        .validators
        .get(&proof.leader_id)
        .filter(|validator| validator.is_active)
        .ok_or_else(|| anyhow::anyhow!("replacement primary is not active"))?;
    verify_message_signature(
        &leader.pubkey,
        &reproposal_signing_message(
            &hash,
            proof.target_view,
            &proof.leader_id,
            &proof.view_change_certificate,
        ),
        &proof.leader_signature,
    )
}

impl PbftEngine {
    pub fn allow_small_cluster_quorum(&self) -> bool {
        self.config.allow_small_cluster_quorum
    }

    fn evict_rounds_if_needed(&mut self) {
        while self.rounds.len() >= MAX_TRACKED_ROUNDS {
            let candidate = self
                .rounds
                .iter()
                .filter(|(_, r)| r.is_terminal())
                .min_by_key(|(_, r)| (r.block.header.height, r.created_at))
                .map(|(h, _)| h.clone())
                .or_else(|| {
                    self.rounds
                        .iter()
                        .min_by_key(|(_, r)| (r.block.header.height, r.created_at))
                        .map(|(h, _)| h.clone())
                });
            let Some(evict_hash) = candidate else {
                break;
            };
            self.rounds.remove(&evict_hash);
        }
    }

    fn evict_view_change_state_if_needed(&mut self) {
        while self.view_change_votes.len() >= MAX_VIEW_CHANGE_BUCKETS {
            let Some(lowest_key) = self
                .view_change_votes
                .keys()
                .min_by_key(|(h, _, v)| (*h, *v))
                .cloned()
            else {
                break;
            };
            self.view_change_votes.remove(&lowest_key);
        }
        while self.certified_views.len() >= MAX_CERTIFIED_VIEWS {
            let Some(lowest_ctx) = self.certified_views.keys().min_by_key(|(h, _)| *h).cloned()
            else {
                break;
            };
            self.certified_views.remove(&lowest_ctx);
        }
        while self.equivocated_view_change_voters.len() >= MAX_EQUIVOCATED_VIEW_VOTERS {
            let Some(lowest_voter) = self
                .equivocated_view_change_voters
                .iter()
                .min_by_key(|(h, _, v, _)| (*h, *v))
                .cloned()
            else {
                break;
            };
            self.equivocated_view_change_voters.remove(&lowest_voter);
        }
    }

    fn prune_view_change_below(&mut self, floor_height: u64) {
        self.view_change_votes
            .retain(|(h, _, _), _| *h >= floor_height);
        self.certified_views.retain(|(h, _), _| *h >= floor_height);
        self.equivocated_view_change_voters
            .retain(|(h, _, _, _)| *h >= floor_height);
    }

    /// Capture the full quorum proof before a local COMMIT can be signed.
    pub fn prepared_certificate(&self, block_hash: &str) -> Option<(Block, PreparedCertificate)> {
        let round = self.rounds.get(block_hash)?;
        if round.prepare_sigs.len() < round.quorum() {
            return None;
        }
        let mut prepare_signatures: Vec<_> = round.prepare_sigs.values().cloned().collect();
        prepare_signatures.sort_by(|a, b| a.validator_id.cmp(&b.validator_id));
        Some((
            round.block.clone(),
            PreparedCertificate {
                header: PreparedBlockHeader::from(&round.block.header),
                prepared_view: round.view_number,
                original_view_change_certificate: round
                    .block
                    .header
                    .view_change_certificate
                    .clone(),
                reproposal_proof: round.reproposal_proof.clone().map(Box::new),
                transactions: round.block.transactions.clone(),
                prepare_signatures,
            },
        ))
    }

    pub fn round_validator_set(&self, block_hash: &str) -> Option<ValidatorSet> {
        self.rounds
            .get(block_hash)
            .map(|round| round.validator_set.clone())
    }

    /// Reconstruct a prepared round after restart from its durable full block
    /// and PREPARE quorum. The normal proposal and signature checks still run.
    pub fn restore_prepared(
        &mut self,
        block: Block,
        certificate: PreparedCertificate,
        validator_set: ValidatorSet,
    ) -> Result<()> {
        let block_hash = block.hash();
        if certificate.block().hash() != block_hash {
            bail!("recovered prepared certificate names a different block hash");
        }
        let next_view = certificate
            .prepared_view
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("recovered prepared view has no successor"))?;
        let vote_context = ViewChangeVote {
            height: certificate.header.height,
            prev_hash: certificate.header.prev_hash.clone(),
            new_view: next_view,
            validator_id: String::new(),
            prepared: None,
            signature: String::new(),
        };
        verify_prepared_certificate(
            &vote_context,
            &certificate,
            &validator_set,
            self.config.allow_small_cluster_quorum,
        )?;
        let quorum = quorum_threshold(validator_set.len(), self.config.allow_small_cluster_quorum);
        let prepared_view = certificate.prepared_view;
        let reproposal_proof = certificate.reproposal_proof.as_deref().cloned();
        self.start_round(block, validator_set)?;
        // PREPARE replay must use the certified view rather than the immutable
        // original proposal header's view.
        if let Some(round) = self.rounds.get_mut(&block_hash) {
            round.view_number = prepared_view;
            round.reproposal_proof = reproposal_proof;
        }
        for signature in certificate.prepare_signatures.into_iter().take(quorum) {
            let validator_id = signature.validator_id.clone();
            self.prepare(&block_hash, &validator_id, signature)?;
        }
        if self
            .rounds
            .get(&block_hash)
            .is_none_or(|round| round.phase != PbftPhase::Commit)
        {
            bail!("recovered prepared round did not enter COMMIT phase");
        }
        Ok(())
    }

    pub fn new() -> Self {
        Self {
            rounds: HashMap::new(),
            config: PbftConfig::default(),
            view_change_votes: HashMap::new(),
            equivocated_view_change_voters: HashSet::new(),
            certified_views: HashMap::new(),
        }
    }

    pub fn with_config(config: PbftConfig) -> Self {
        Self {
            rounds: HashMap::new(),
            config,
            view_change_votes: HashMap::new(),
            equivocated_view_change_voters: HashSet::new(),
            certified_views: HashMap::new(),
        }
    }

    pub fn start_round(&mut self, block: Block, vs: ValidatorSet) -> Result<String> {
        let context = (block.header.height, block.header.prev_hash.clone());
        let proposal_view = block.header.view_number;
        let proposal_certificate = block.header.view_change_certificate.clone();
        let hash = block.hash();
        if self.rounds.contains_key(&hash) {
            // A duplicate proposal must never replace a round and erase the
            // PREPARE/COMMIT signatures already collected for this hash.
            return Ok(hash);
        }
        if self.rounds.iter().any(|(other_hash, other)| {
            other_hash != &hash
                && other.block.header.height == block.header.height
                && other.block.header.prev_hash == block.header.prev_hash
                && other.block.header.view_number == proposal_view
        }) {
            bail!("conflicting PBFT proposal already exists in this view");
        }
        if let Some(certified) = self.certified_views.get(&context) {
            if let Some(vote) = certified.first() {
                if block.header.view_number < vote.new_view {
                    bail!(
                        "proposal view {} is stale; view {} is already certified",
                        block.header.view_number,
                        vote.new_view
                    );
                }
            }
        }
        let mut round = PbftRound::with_config(block, vs, self.config.clone());
        let proposer = round.block.header.proposer_id.clone();
        round.pre_prepare(&proposer)?;
        if proposal_view > 0 {
            let current_view = self
                .certified_views
                .get(&context)
                .and_then(|votes| votes.first().map(|vote| vote.new_view))
                .unwrap_or(0);
            if proposal_view > current_view {
                self.evict_view_change_state_if_needed();
                self.certified_views.insert(context, proposal_certificate);
            }
        }
        self.evict_rounds_if_needed();
        self.rounds.insert(hash.clone(), round);
        Ok(hash)
    }

    /// Authorize retransmission of the exact block that a prior view
    /// prepared. The new primary signs an envelope; it cannot change the
    /// hash-bound original proposer, timestamp, view, or transaction root.
    pub fn repropose_prepared(
        &mut self,
        block: Block,
        validator_set: ValidatorSet,
        target_view: u32,
        leader_id: &str,
        certificate: Vec<ViewChangeVote>,
        leader_signature: &str,
    ) -> Result<String> {
        if target_view <= block.header.view_number {
            bail!("reproposal must advance the original block view");
        }
        let hash = block.hash();
        let mut certificate_header = block.header.clone();
        certificate_header.view_number = target_view;
        certificate_header.view_change_certificate = certificate.clone();
        let highest = verify_view_change_certificate(
            &certificate_header,
            &validator_set,
            self.config.allow_small_cluster_quorum,
        )?
        .ok_or_else(|| anyhow::anyhow!("reproposal certificate has no prepared value"))?;
        if highest.block().hash() != hash {
            bail!("reproposal changed the highest prepared block hash");
        }
        let selected =
            selected_proposer_for_view(&validator_set, &block.header.prev_hash, target_view)?;
        if leader_id != selected {
            bail!("reproposal is not signed by the selected replacement primary");
        }
        let leader = validator_set
            .validators
            .get(leader_id)
            .filter(|validator| validator.is_active)
            .ok_or_else(|| anyhow::anyhow!("replacement primary is not active"))?;
        verify_message_signature(
            &leader.pubkey,
            &reproposal_signing_message(&hash, target_view, leader_id, &certificate),
            leader_signature,
        )?;
        let proof = ReproposalProof {
            target_view,
            leader_id: leader_id.to_owned(),
            view_change_certificate: certificate.clone(),
            leader_signature: leader_signature.to_owned(),
        };
        let context = (block.header.height, block.header.prev_hash.clone());
        if self
            .certified_views
            .get(&context)
            .and_then(|votes| votes.first())
            .is_some_and(|vote| vote.new_view > target_view)
        {
            bail!("reproposal targets a stale certified PBFT view");
        }
        let mut candidate =
            PbftRound::with_config(block.clone(), validator_set, self.config.clone());
        candidate.pre_prepare(&block.header.proposer_id)?;
        if let Some(round) = self.rounds.get_mut(&hash) {
            if round.phase == PbftPhase::Finalised {
                return Ok(hash);
            }
            if round.view_number > target_view {
                bail!("reproposal targets a stale active PBFT view");
            }
            if round.view_number < target_view || round.phase == PbftPhase::Failed {
                round.view_number = target_view;
                round.phase = PbftPhase::Prepare;
                round.phase_entered_at = Instant::now();
                round.reproposal_proof = Some(proof.clone());
            }
            for signature in &highest.prepare_signatures {
                round
                    .prepare_sigs
                    .insert(signature.validator_id.clone(), signature.clone());
            }
        } else {
            candidate.view_number = target_view;
            candidate.reproposal_proof = Some(proof);
            for signature in &highest.prepare_signatures {
                candidate
                    .prepare_sigs
                    .insert(signature.validator_id.clone(), signature.clone());
            }
            self.evict_rounds_if_needed();
            self.rounds.insert(hash.clone(), candidate);
        }
        self.evict_view_change_state_if_needed();
        self.certified_views.insert(context, certificate);
        Ok(hash)
    }

    pub fn prepare(&mut self, block_hash: &str, vid: &str, sig: BlockSignature) -> Result<bool> {
        let round = self
            .rounds
            .get(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        if self.rounds.iter().any(|(other_hash, other)| {
            other_hash != block_hash
                && other.block.header.height == round.block.header.height
                && other.block.header.prev_hash == round.block.header.prev_hash
                && other.view_number == round.view_number
                && other.prepare_sigs.contains_key(vid)
        }) {
            bail!("validator already PREPARED another block in this PBFT view");
        }
        let round = self
            .rounds
            .get_mut(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        round.receive_prepare(vid, sig)
    }

    pub fn commit(&mut self, block_hash: &str, vid: &str, sig: BlockSignature) -> Result<bool> {
        let round = self
            .rounds
            .get(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        if self.rounds.iter().any(|(other_hash, other)| {
            other_hash != block_hash
                && other.block.header.height == round.block.header.height
                && other.block.header.prev_hash == round.block.header.prev_hash
                && other.commit_sigs.contains_key(vid)
        }) {
            bail!("validator already COMMITTED another block at this height");
        }
        let round = self
            .rounds
            .get_mut(block_hash)
            .ok_or_else(|| anyhow::anyhow!("No active round for block {}", block_hash))?;
        round.receive_commit(vid, sig)
    }

    /// Pipelined fast commit directly on active round
    pub fn try_fast_commit(
        &mut self,
        block_hash: &str,
        vid: &str,
        prepare_sig: BlockSignature,
        commit_sig: BlockSignature,
    ) -> Result<bool> {
        let reached_prepare_quorum = self.prepare(block_hash, vid, prepare_sig)?;
        if reached_prepare_quorum
            || self
                .rounds
                .get(block_hash)
                .is_some_and(|round| round.phase == PbftPhase::Commit)
        {
            return self.commit(block_hash, vid, commit_sig);
        }
        Ok(false)
    }

    pub fn finalised_sigs(&self, block_hash: &str) -> Vec<BlockSignature> {
        self.rounds
            .get(block_hash)
            .map(|r| r.finalised_signatures())
            .unwrap_or_default()
    }

    /// Reconcile a round after the caller has persisted and authenticated the
    /// same canonical block, for example through chain sync. A timeout or a
    /// different block at the same height never releases this round's locks.
    pub fn observe_finalized_block(&mut self, block: &Block) -> Result<bool> {
        let hash = block.hash();
        let Some(round) = self.rounds.get_mut(&hash) else {
            return Ok(false);
        };
        if round.phase == PbftPhase::Finalised {
            return Ok(false);
        }
        if block.header.merkle_root != common::merkle_root(&block.transactions)
            || round.block.header.merkle_root != common::merkle_root(&round.block.transactions)
        {
            bail!("canonical PBFT reconciliation requires the hash-bound transaction body");
        }
        let mut commits = HashMap::new();
        for signature in &block.pbft_signatures {
            round.verify_block_signature("commit", &signature.validator_id, signature)?;
            if commits
                .insert(signature.validator_id.clone(), signature.clone())
                .is_some()
            {
                bail!("canonical PBFT reconciliation rejects duplicate COMMIT signers");
            }
        }
        if commits.len() < round.quorum() {
            bail!("canonical PBFT reconciliation requires a full COMMIT quorum");
        }
        // Retain PREPARE evidence and the immutable proposal. Only the matching
        // round receives the independently verified canonical certificate.
        round.commit_sigs = commits;
        round.block.pbft_signatures = block.pbft_signatures.clone();
        round.phase = PbftPhase::Finalised;
        round.phase_entered_at = Instant::now();
        self.prune_view_change_below(block.header.height.saturating_sub(64));
        tracing::info!(height = block.header.height, block_hash = %hash,
            "[PBFT] reconciled round with certified canonical finality");
        Ok(true)
    }

    pub fn get_finalised_block(&self, block_hash: &str) -> Option<Block> {
        let round = self.rounds.get(block_hash)?;
        if round.phase == PbftPhase::Finalised {
            let mut final_block = round.block.clone();
            final_block.pbft_signatures = round.finalised_signatures();
            Some(final_block)
        } else {
            None
        }
    }

    /// Consensus context required to persist a local signing decision.
    pub fn round_context(&self, block_hash: &str) -> Option<(u64, String, u32)> {
        self.rounds.get(block_hash).map(|round| {
            (
                round.block.header.height,
                round.block.header.prev_hash.clone(),
                round.view_number,
            )
        })
    }

    /// Create view-change votes for timed-out rounds. A timeout only creates a
    /// signed vote; it does not change the active view. The view advances only
    /// after `receive_view_change` assembles a PBFT quorum.
    pub fn timeout_rounds(&mut self, local_validator_id: &str) -> Vec<ViewChangeSignal> {
        let timed_out: Vec<(u64, String)> = self
            .rounds
            .iter()
            .filter(|(_, r)| r.is_phase_timed_out())
            .map(|(_, round)| {
                (
                    round.block.header.height,
                    round.block.header.prev_hash.clone(),
                )
            })
            .collect();

        let mut signals = Vec::new();
        for (height, prev_hash) in timed_out {
            if let Some(vote) = self.make_view_change_vote(height, &prev_hash, local_validator_id) {
                signals.push(ViewChangeSignal { vote });
            }
        }
        signals
    }

    /// Create the local vote for a stalled chain tip even when this validator
    /// never received a proposal. This is how the backup can join a quorum
    /// when the primary stopped before gossiping PRE-PREPARE.
    pub fn request_view_change_for_tip(
        &mut self,
        height: u64,
        prev_hash: &str,
        local_validator_id: &str,
    ) -> Option<ViewChangeSignal> {
        self.make_view_change_vote(height, prev_hash, local_validator_id)
            .map(|vote| ViewChangeSignal { vote })
    }

    fn make_view_change_vote(
        &mut self,
        height: u64,
        prev_hash: &str,
        local_validator_id: &str,
    ) -> Option<ViewChangeVote> {
        if self.rounds.values().any(|round| {
            round.block.header.height == height
                && round.block.header.prev_hash == prev_hash
                && !round.validator_set.is_member(local_validator_id)
        }) {
            return None;
        }
        if self.rounds.values().any(|round| {
            round.block.header.height == height
                && round.block.header.prev_hash == prev_hash
                && round.phase == PbftPhase::Finalised
        }) {
            return None;
        }
        let certified_view = self
            .certified_views
            .get(&(height, prev_hash.to_string()))
            .and_then(|votes| votes.first().map(|vote| vote.new_view))
            .unwrap_or(0);
        let active_view = self
            .rounds
            .values()
            .filter(|round| {
                round.block.header.height == height && round.block.header.prev_hash == prev_hash
            })
            .map(|round| round.view_number)
            .max()
            .unwrap_or(0);
        let current_view = certified_view.max(active_view);
        let pending_view = self
            .view_change_votes
            .keys()
            .filter(|(vote_height, vote_parent, vote_view)| {
                *vote_height == height && vote_parent == prev_hash && *vote_view > current_view
            })
            .map(|(_, _, vote_view)| *vote_view)
            .max();
        let new_view = pending_view.unwrap_or(current_view.checked_add(1)?);
        if new_view > self.config.max_view_changes {
            return None;
        }
        let key = (height, prev_hash.to_string(), new_view);
        if let Some(existing) = self
            .view_change_votes
            .get(&key)
            .and_then(|votes| votes.get(local_validator_id))
        {
            return Some(existing.clone());
        }

        let prepared = self
            .rounds
            .values()
            .filter(|round| {
                round.block.header.height == height
                    && round.block.header.prev_hash == prev_hash
                    && round.prepare_sigs.len() >= round.quorum()
            })
            .max_by_key(|round| round.view_number)
            .map(|round| {
                let mut prepare_signatures: Vec<_> = round.prepare_sigs.values().cloned().collect();
                prepare_signatures.sort_by(|a, b| a.validator_id.cmp(&b.validator_id));
                PreparedCertificate {
                    header: PreparedBlockHeader::from(&round.block.header),
                    prepared_view: round.view_number,
                    original_view_change_certificate: round
                        .block
                        .header
                        .view_change_certificate
                        .clone(),
                    reproposal_proof: round.reproposal_proof.clone().map(Box::new),
                    transactions: round.block.transactions.clone(),
                    prepare_signatures,
                }
            });

        Some(ViewChangeVote {
            height,
            prev_hash: prev_hash.to_string(),
            new_view,
            validator_id: local_validator_id.to_string(),
            prepared,
            signature: String::new(),
        })
    }

    /// Validate and record one signed view-change vote. Returns the complete
    /// quorum certificate once it is available; a local timeout alone never
    /// advances the view.
    pub fn receive_view_change(
        &mut self,
        vote: ViewChangeVote,
        validator_set: &ValidatorSet,
    ) -> Result<Option<Vec<ViewChangeVote>>> {
        verify_view_change_vote(&vote, validator_set, self.config.allow_small_cluster_quorum)?;
        if self.rounds.values().any(|round| {
            round.block.header.height == vote.height
                && round.block.header.prev_hash == vote.prev_hash
                && round.phase == PbftPhase::Finalised
        }) {
            bail!("cannot change view after this height is finalised locally");
        }
        let key = (vote.height, vote.prev_hash.clone(), vote.new_view);
        let voter_key = (
            vote.height,
            vote.prev_hash.clone(),
            vote.new_view,
            vote.validator_id.clone(),
        );
        if self.equivocated_view_change_voters.contains(&voter_key) {
            bail!("validator already equivocated in this view-change round");
        }
        self.evict_view_change_state_if_needed();
        let votes = self.view_change_votes.entry(key.clone()).or_default();
        if let Some(existing) = votes.get(&vote.validator_id) {
            if existing.signing_message() != vote.signing_message() {
                votes.remove(&vote.validator_id);
                self.evict_view_change_state_if_needed();
                self.equivocated_view_change_voters.insert(voter_key);
                bail!("validator equivocated in this view-change round");
            }
        } else {
            votes.insert(vote.validator_id.clone(), vote.clone());
        }

        let quorum = quorum_threshold(validator_set.len(), self.config.allow_small_cluster_quorum);
        if votes.len() < quorum {
            return Ok(None);
        }
        let mut certificate: Vec<_> = votes.values().cloned().collect();
        certificate.sort_by(|a, b| a.validator_id.cmp(&b.validator_id));
        let mut check_header = common::Block::genesis().header;
        check_header.height = vote.height;
        check_header.prev_hash = vote.prev_hash.clone();
        check_header.view_number = vote.new_view;
        check_header.view_change_certificate = certificate.clone();
        verify_view_change_certificate(
            &check_header,
            validator_set,
            self.config.allow_small_cluster_quorum,
        )?;

        let context = (vote.height, vote.prev_hash.clone());
        let should_replace = self
            .certified_views
            .get(&context)
            .and_then(|existing| existing.first())
            .map(|existing| existing.new_view < vote.new_view)
            .unwrap_or(true);
        if should_replace {
            self.certified_views.insert(context, certificate.clone());
            for round in self.rounds.values_mut().filter(|round| {
                round.block.header.height == vote.height
                    && round.block.header.prev_hash == vote.prev_hash
                    && round.view_number < vote.new_view
                    && !matches!(round.phase, PbftPhase::Finalised)
            }) {
                round.phase = PbftPhase::Failed;
            }
            tracing::warn!(
                height = vote.height,
                view = vote.new_view,
                votes = certificate.len(),
                quorum,
                "[PBFT] quorum-certified view change applied"
            );
        }
        Ok(Some(certificate))
    }

    /// Current highest certified view for a height and parent. View zero needs
    /// no certificate; `None` means no view-change certificate exists yet.
    pub fn certified_view(
        &self,
        height: u64,
        prev_hash: &str,
    ) -> Option<(u32, Vec<ViewChangeVote>, Option<PreparedCertificate>)> {
        let votes = self
            .certified_views
            .get(&(height, prev_hash.to_string()))?
            .clone();
        let view_number = votes.first()?.new_view;
        let mut best: Option<PreparedCertificate> = None;
        for vote in &votes {
            if let Some(prepared) = &vote.prepared {
                if best
                    .as_ref()
                    .is_none_or(|current| prepared.prepared_view > current.prepared_view)
                {
                    best = Some(prepared.clone());
                }
            }
        }
        Some((view_number, votes, best))
    }

    /// Remove rounds that have been in a terminal state (Finalised / Failed)
    /// for longer than the configured stale round TTL. Returns the number of
    /// rounds removed.
    pub fn cleanup_stale_rounds(&mut self) -> usize {
        let ttl = self.config.stale_round_ttl;
        let finalised_contexts: HashSet<(u64, String)> = self
            .rounds
            .values()
            .filter(|round| round.phase == PbftPhase::Finalised)
            .map(|round| {
                (
                    round.block.header.height,
                    round.block.header.prev_hash.clone(),
                )
            })
            .collect();
        let stale: Vec<String> = self
            .rounds
            .iter()
            .filter(|(_, r)| {
                r.is_terminal()
                    && r.created_at.elapsed() > ttl
                    // A failed earlier view may still carry the highest
                    // prepared value needed by a later view change. Retain it
                    // until this height has actually finalised.
                    && (r.phase != PbftPhase::Failed
                        || r.prepare_sigs.len() < r.quorum()
                        || finalised_contexts
                            .contains(&(r.block.header.height, r.block.header.prev_hash.clone())))
            })
            .map(|(h, _)| h.clone())
            .collect();
        let count = stale.len();
        for hash in stale {
            self.rounds.remove(&hash);
        }
        if count > 0 {
            tracing::info!("[PBFT] Cleaned up {} stale rounds", count);
        }
        if let Some(highest_finalized) = finalised_contexts.iter().map(|(h, _)| *h).max() {
            self.prune_view_change_below(highest_finalized.saturating_sub(64));
        }
        count
    }

    /// Number of currently-tracked rounds (for metrics / observability).
    pub fn active_round_count(&self) -> usize {
        self.rounds.len()
    }

    /// Whether `canonical` appears in a non-terminal PBFT round (block proposed).
    pub fn canonical_in_non_terminal_round(&self, canonical: &str) -> bool {
        self.rounds.values().any(|round| {
            !round.is_terminal()
                && round
                    .block
                    .transactions
                    .iter()
                    .any(|tx| transaction_package_canonical(tx).as_deref() == Some(canonical))
        })
    }

    /// Non-terminal round at `height`, if any, as (block_hash, phase).
    /// Used by node callers to avoid replacing an active proposal. Terminal
    /// rounds are omitted here; separate engine vote guards still apply while
    /// their records remain in memory.
    pub fn non_terminal_round_at_height(&self, height: u64) -> Option<(String, PbftPhase)> {
        self.rounds
            .iter()
            .find(|(_, round)| !round.is_terminal() && round.block.header.height == height)
            .map(|(hash, round)| (hash.clone(), round.phase.clone()))
    }

    /// Own votes eligible for rebroadcast: non-terminal rounds in Prepare or
    /// Commit phase where `validator_id` already voted. Callers should only
    /// rebroadcast rounds whose phase age exceeds the phase timeout (stuck
    /// rounds whose votes were likely lost), to avoid steady-state spam.
    pub fn own_votes_for_rebroadcast(&self, validator_id: &str) -> Vec<OwnVoteForRebroadcast> {
        self.rounds
            .iter()
            .filter(|(_, round)| {
                !round.is_terminal()
                    && matches!(round.phase, PbftPhase::Prepare | PbftPhase::Commit)
            })
            .filter_map(|(hash, round)| {
                let prepare_sig = round.prepare_sigs.get(validator_id).cloned();
                let commit_sig = round.commit_sigs.get(validator_id).cloned();
                if prepare_sig.is_none() && commit_sig.is_none() {
                    return None;
                }
                Some(OwnVoteForRebroadcast {
                    block_hash: hash.clone(),
                    block_height: round.block.header.height,
                    prev_hash: round.block.header.prev_hash.clone(),
                    view_number: round.view_number,
                    phase: round.phase.clone(),
                    prepare_sig,
                    commit_sig,
                    phase_age_seconds: round.phase_entered_at.elapsed().as_secs_f64(),
                    phase_timeout_seconds: round.config.round_phase_timeout.as_secs_f64(),
                })
            })
            .collect()
    }

    /// Observability snapshot of all tracked PBFT block rounds.
    pub fn observability_snapshot(&self) -> Vec<PbftRoundSnapshot> {
        let mut rounds: Vec<PbftRoundSnapshot> = self
            .rounds
            .iter()
            .map(|(block_hash, round)| PbftRoundSnapshot {
                block_hash: block_hash.clone(),
                block_height: round.block.header.height,
                phase: round.phase.as_str().to_string(),
                phase_code: round.phase.code(),
                phase_age_seconds: round.phase_entered_at.elapsed().as_secs_f64(),
                prepare_count: round.prepare_sigs.len(),
                commit_count: round.commit_sigs.len(),
                quorum: round.quorum(),
                validator_count: round.validator_set.len(),
                view_number: round.view_number,
                view_change_count: round.view_change_count,
            })
            .collect();
        rounds.sort_by(|a, b| {
            a.block_height
                .cmp(&b.block_height)
                .then_with(|| a.block_hash.cmp(&b.block_hash))
        });
        rounds
    }

    /// Maximum phase age across non-terminal rounds (0 when none).
    pub fn max_non_terminal_phase_age_seconds(&self) -> f64 {
        self.rounds
            .values()
            .filter(|round| !round.is_terminal())
            .map(|round| round.phase_entered_at.elapsed().as_secs_f64())
            .fold(0.0, f64::max)
    }
}

fn transaction_package_canonical(tx: &common::Transaction) -> Option<String> {
    match tx {
        common::Transaction::Publish(record)
        | common::Transaction::PublishAuthorizedV1 { record, .. } => Some(record.id.canonical()),
        common::Transaction::Revoke {
            package_canonical, ..
        } => Some(package_canonical.clone()),
        common::Transaction::RevokeV2 { request, .. } => Some(request.package_canonical.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        pbft_signature_message, quorum_threshold, selected_proposer_for_view, PbftPhase, PbftRound,
        PreparedBlockHeader, PreparedCertificate,
    };
    use crate::validator_set::{ValidatorInfo, ValidatorSet};
    use common::{Block, BlockSignature};
    use ed25519_dalek::{Signer, SigningKey};

    fn validator_set(id: &str, signing_key: &SigningKey) -> ValidatorSet {
        let mut set = ValidatorSet::new();
        set.add(ValidatorInfo {
            id: id.into(),
            pubkey: hex::encode(signing_key.verifying_key().as_bytes()),
            eth_address: None,
            stake: 100,
            reputation: 100,
            is_active: true,
        });
        set
    }

    fn test_block(proposer_id: &str) -> Block {
        let mut block = Block::genesis();
        block.header.height = 1;
        block.header.prev_hash = "11".repeat(32);
        block.header.proposer_id = proposer_id.into();
        block
    }

    fn signed_block_vote(
        validator_id: &str,
        signing_key: &SigningKey,
        phase: &str,
        block_hash: &str,
    ) -> BlockSignature {
        let message = pbft_signature_message(phase, block_hash);
        BlockSignature {
            validator_id: validator_id.into(),
            pubkey: hex::encode(signing_key.verifying_key().as_bytes()),
            signature: hex::encode(signing_key.sign(message.as_bytes()).to_bytes()),
            pq_signature: None,
        }
    }

    #[test]
    fn small_cluster_quorum_is_explicitly_gated() {
        assert_eq!(quorum_threshold(0, false), 0);
        assert_eq!(quorum_threshold(1, false), 1);
        assert_eq!(quorum_threshold(2, false), 2);
        assert_eq!(quorum_threshold(3, false), 3);
        assert_eq!(quorum_threshold(3, true), 2);
    }

    #[test]
    fn larger_validator_sets_use_pbft_quorum_regardless_of_flag() {
        assert_eq!(quorum_threshold(4, false), 3);
        assert_eq!(quorum_threshold(4, true), 3);
        assert_eq!(quorum_threshold(5, false), 4);
        assert_eq!(quorum_threshold(7, true), 5);
    }

    #[test]
    fn prepare_requires_valid_phase_bound_signature() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let block = test_block("node-1");
        let block_hash = block.hash();
        let mut round = PbftRound::new(block, validator_set("node-1", &signing_key));

        round.pre_prepare("node-1").unwrap();
        let sig = signed_block_vote("node-1", &signing_key, "prepare", &block_hash);

        assert!(round.receive_prepare("node-1", sig).unwrap());
        assert_eq!(round.phase, PbftPhase::Commit);
    }

    #[test]
    fn prepare_rejects_forged_signature() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let attacker_key = SigningKey::from_bytes(&[8u8; 32]);
        let block = test_block("node-1");
        let block_hash = block.hash();
        let mut round = PbftRound::new(block, validator_set("node-1", &signing_key));

        round.pre_prepare("node-1").unwrap();
        let mut sig = signed_block_vote("node-1", &attacker_key, "prepare", &block_hash);
        sig.pubkey = hex::encode(signing_key.verifying_key().as_bytes());

        assert!(round.receive_prepare("node-1", sig).is_err());
        assert!(round.prepare_sigs.is_empty());
        assert_eq!(round.phase, PbftPhase::Prepare);
    }

    #[test]
    fn commit_requires_active_member_signature() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let outsider_key = SigningKey::from_bytes(&[9u8; 32]);
        let block = test_block("node-1");
        let block_hash = block.hash();
        let mut round = PbftRound::new(block, validator_set("node-1", &signing_key));

        round.pre_prepare("node-1").unwrap();
        let prepare = signed_block_vote("node-1", &signing_key, "prepare", &block_hash);
        assert!(round.receive_prepare("node-1", prepare).unwrap());

        let outsider = signed_block_vote("node-2", &outsider_key, "commit", &block_hash);
        assert!(round.receive_commit("node-2", outsider).is_err());
        assert!(round.commit_sigs.is_empty());
        assert_eq!(round.phase, PbftPhase::Commit);
    }

    #[test]
    fn commit_rejects_prepare_signature_replay() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let block = test_block("node-1");
        let block_hash = block.hash();
        let mut round = PbftRound::new(block, validator_set("node-1", &signing_key));

        round.pre_prepare("node-1").unwrap();
        let prepare = signed_block_vote("node-1", &signing_key, "prepare", &block_hash);
        assert!(round.receive_prepare("node-1", prepare.clone()).unwrap());

        assert!(round.receive_commit("node-1", prepare).is_err());
        assert!(round.commit_sigs.is_empty());
        assert_eq!(round.phase, PbftPhase::Commit);
    }

    #[test]
    fn observability_snapshot_reports_active_round() {
        use super::PbftEngine;

        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let block = test_block("node-1");
        let mut engine = PbftEngine::new();
        engine
            .start_round(block, validator_set("node-1", &signing_key))
            .unwrap();

        let rounds = engine.observability_snapshot();
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0].phase, "prepare");
        assert_eq!(rounds[0].phase_code, 2);
        assert!(engine.max_non_terminal_phase_age_seconds() >= 0.0);
    }

    fn four_validator_set() -> ValidatorSet {
        let mut set = ValidatorSet::new();
        for i in 0..4u8 {
            let key = SigningKey::from_bytes(&[i + 1; 32]);
            set.add(ValidatorInfo {
                id: format!("v{i}"),
                pubkey: hex::encode(key.verifying_key().as_bytes()),
                eth_address: None,
                stake: 100,
                reputation: 100,
                is_active: true,
            });
        }
        set
    }

    fn signed_view_change_vote(
        mut vote: common::ViewChangeVote,
        signing_key: &SigningKey,
    ) -> common::ViewChangeVote {
        vote.signature = hex::encode(
            signing_key
                .sign(vote.signing_message().as_bytes())
                .to_bytes(),
        );
        vote
    }

    fn two_validator_set() -> (ValidatorSet, Vec<SigningKey>) {
        let keys: Vec<_> = (1u8..=2)
            .map(|seed| SigningKey::from_bytes(&[seed; 32]))
            .collect();
        let mut set = ValidatorSet::new();
        for (index, key) in keys.iter().enumerate() {
            set.add(ValidatorInfo {
                id: format!("v{index}"),
                pubkey: hex::encode(key.verifying_key().as_bytes()),
                eth_address: None,
                stake: 100,
                reputation: 100,
                is_active: true,
            });
        }
        (set, keys)
    }

    fn selected_proposer(vs: &ValidatorSet, prev_hash: &str) -> String {
        let active: Vec<crate::vrf::VrfValidator> = vs
            .validators
            .values()
            .filter(|v| v.is_active)
            .map(|v| crate::vrf::VrfValidator {
                id: v.id.clone(),
                pubkey: v.pubkey.clone(),
                vrf_output: None,
                vrf_proof: None,
            })
            .collect();
        crate::vrf::select_proposer_deterministic(&active, prev_hash)
            .expect("four-validator set has a proposer")
    }

    #[test]
    fn timeout_rounds_broadcasts_without_unilateral_view_change() {
        use super::{PbftConfig, PbftEngine};
        use std::time::Duration;

        let vs = four_validator_set();
        let keys: Vec<_> = (0..4u8)
            .map(|seed| SigningKey::from_bytes(&[seed + 1; 32]))
            .collect();
        let proposer = selected_proposer(&vs, &"11".repeat(32));
        let mut engine = PbftEngine::with_config(PbftConfig {
            round_phase_timeout: Duration::from_millis(1),
            max_view_changes: 3,
            stale_round_ttl: Duration::from_secs(120),
            allow_small_cluster_quorum: false,
        });
        engine
            .start_round(test_block(&proposer), vs.clone())
            .unwrap();
        std::thread::sleep(Duration::from_millis(5));

        let signals = engine.timeout_rounds("v0");
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].vote.new_view, 1);
        assert_eq!(engine.observability_snapshot()[0].view_number, 0);

        let first_vote = signed_view_change_vote(signals[0].vote.clone(), &keys[0]);
        assert!(engine
            .receive_view_change(first_vote.clone(), &vs)
            .unwrap()
            .is_none());
        assert!(engine.certified_view(1, &"11".repeat(32)).is_none());

        let mut completed = None;
        for index in 1..3 {
            let mut vote = first_vote.clone();
            vote.validator_id = format!("v{index}");
            vote.signature.clear();
            let vote = signed_view_change_vote(vote, &keys[index]);
            completed = engine.receive_view_change(vote, &vs).unwrap();
        }
        assert_eq!(completed.unwrap().len(), 3);
        assert_eq!(
            engine
                .certified_view(1, &"11".repeat(32))
                .map(|(view, _, _)| view),
            Some(1)
        );
        assert_eq!(engine.observability_snapshot()[0].phase, "failed");
    }

    #[test]
    fn certified_view_change_authorizes_the_rotated_proposer() {
        let (vs, keys) = two_validator_set();
        let prev_hash = "22".repeat(32);
        let mut votes = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            let vote = common::ViewChangeVote {
                height: 1,
                prev_hash: prev_hash.clone(),
                new_view: 1,
                validator_id: format!("v{index}"),
                prepared: None,
                signature: String::new(),
            };
            votes.push(signed_view_change_vote(vote, key));
        }

        let primary = selected_proposer_for_view(&vs, &prev_hash, 0).unwrap();
        let replacement = selected_proposer_for_view(&vs, &prev_hash, 1).unwrap();
        assert_ne!(primary, replacement);

        let mut block = test_block(&replacement);
        block.header.prev_hash = prev_hash.clone();
        block.header.view_number = 1;
        block.header.view_change_certificate = votes.clone();
        let mut round = PbftRound::new(block.clone(), vs.clone());
        round.pre_prepare(&replacement).unwrap();

        block.header.proposer_id = primary;
        let mut unauthorized = PbftRound::new(block, vs);
        assert!(unauthorized
            .pre_prepare(&unauthorized.block.header.proposer_id.clone())
            .is_err());

        let mut incomplete = common::Block::genesis();
        incomplete.header.height = 1;
        incomplete.header.prev_hash = prev_hash;
        incomplete.header.proposer_id = replacement;
        incomplete.header.view_number = 1;
        incomplete.header.view_change_certificate = votes.into_iter().take(1).collect();
        let mut round = PbftRound::new(incomplete, two_validator_set().0);
        assert!(round
            .pre_prepare(&round.block.header.proposer_id.clone())
            .is_err());
    }

    #[test]
    fn view_change_rejects_a_different_hash_even_with_the_same_transactions() {
        let (vs, keys) = two_validator_set();
        let prev_hash = "33".repeat(32);
        let transactions = vec![common::Transaction::Revoke {
            package_canonical: "npm:prepared@1.0.0".into(),
            reason: "prepared value".into(),
            revoked_by: "test".into(),
            evidence_hash: "evidence".into(),
        }];
        let primary = selected_proposer_for_view(&vs, &prev_hash, 0).unwrap();
        let mut prepared_block = test_block(&primary);
        prepared_block.header.prev_hash = prev_hash.clone();
        prepared_block.header.merkle_root = common::merkle_root(&transactions);
        prepared_block.transactions = transactions.clone();
        let prepared_hash = prepared_block.hash();
        let prepare_signatures: Vec<_> = keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                signed_block_vote(&format!("v{index}"), key, "prepare", &prepared_hash)
            })
            .collect();
        let prepared = PreparedCertificate {
            header: PreparedBlockHeader::from(&prepared_block.header),
            prepared_view: prepared_block.header.view_number,
            original_view_change_certificate: prepared_block.header.view_change_certificate.clone(),
            reproposal_proof: None,
            transactions: transactions.clone(),
            prepare_signatures,
        };
        let mut votes = Vec::new();
        for (index, key) in keys.iter().enumerate() {
            let vote = common::ViewChangeVote {
                height: 1,
                prev_hash: prev_hash.clone(),
                new_view: 1,
                validator_id: format!("v{index}"),
                prepared: Some(prepared.clone()),
                signature: String::new(),
            };
            votes.push(signed_view_change_vote(vote, key));
        }

        let replacement = selected_proposer_for_view(&vs, &prev_hash, 1).unwrap();
        let mut candidate = test_block(&replacement);
        candidate.header.prev_hash = prev_hash.clone();
        candidate.header.view_number = 1;
        candidate.header.merkle_root = common::merkle_root(&transactions);
        candidate.header.view_change_certificate = votes.clone();
        candidate.transactions = transactions;
        let mut same_transactions = PbftRound::new(candidate.clone(), vs.clone());
        assert!(same_transactions
            .pre_prepare(&replacement)
            .unwrap_err()
            .to_string()
            .contains("prepared block hash"));

        candidate.transactions.clear();
        candidate.header.merkle_root = common::merkle_root(&candidate.transactions);
        let mut conflicting = PbftRound::new(candidate, vs);
        assert!(conflicting.pre_prepare(&replacement).is_err());
    }

    #[test]
    fn timeout_rounds_ignores_non_members() {
        use super::{PbftConfig, PbftEngine};
        use std::time::Duration;

        let vs = four_validator_set();
        let proposer = selected_proposer(&vs, &"11".repeat(32));
        let mut engine = PbftEngine::with_config(PbftConfig {
            round_phase_timeout: Duration::from_millis(1),
            max_view_changes: 3,
            stale_round_ttl: Duration::from_secs(120),
            allow_small_cluster_quorum: false,
        });
        engine.start_round(test_block(&proposer), vs).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(engine.timeout_rounds("outsider").is_empty());
    }

    #[test]
    fn record_view_change_rejects_non_members() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let vs = validator_set("node-1", &signing_key);
        let vote = common::ViewChangeVote {
            height: 1,
            prev_hash: "11".repeat(32),
            new_view: 1,
            validator_id: "outsider".into(),
            prepared: None,
            signature: String::new(),
        };
        assert!(super::verify_view_change_vote(&vote, &vs, false).is_err());
    }

    #[test]
    fn test_pipelined_fast_commit() {
        use super::PbftEngine;

        let k0 = SigningKey::from_bytes(&[1u8; 32]);
        let k1 = SigningKey::from_bytes(&[2u8; 32]);
        let k2 = SigningKey::from_bytes(&[3u8; 32]);
        let k3 = SigningKey::from_bytes(&[4u8; 32]);

        let mut vs = ValidatorSet::new();
        for (vid, k) in [("v0", &k0), ("v1", &k1), ("v2", &k2), ("v3", &k3)] {
            vs.add(ValidatorInfo {
                id: vid.into(),
                pubkey: hex::encode(k.verifying_key().as_bytes()),
                eth_address: None,
                stake: 100,
                reputation: 100,
                is_active: true,
            });
        }

        let proposer = selected_proposer(&vs, &"11".repeat(32));
        let mut engine = PbftEngine::new();
        let hash = engine.start_round(test_block(&proposer), vs).unwrap();

        // v0 prepares
        engine
            .prepare(&hash, "v0", signed_block_vote("v0", &k0, "prepare", &hash))
            .unwrap();
        // v1 prepares
        engine
            .prepare(&hash, "v1", signed_block_vote("v1", &k1, "prepare", &hash))
            .unwrap();

        // v2 uses try_fast_commit (providing both prepare and commit together)
        let fast_res = engine
            .try_fast_commit(
                &hash,
                "v2",
                signed_block_vote("v2", &k2, "prepare", &hash),
                signed_block_vote("v2", &k2, "commit", &hash),
            )
            .unwrap();

        // Reached prepare quorum and recorded commit, but needs 3 commits for finality
        assert!(!fast_res);

        // v0 and v1 commit
        engine
            .commit(&hash, "v0", signed_block_vote("v0", &k0, "commit", &hash))
            .unwrap();
        let finalised = engine
            .commit(&hash, "v1", signed_block_vote("v1", &k1, "commit", &hash))
            .unwrap();
        assert!(finalised, "Block should be finalised after 3 commits");
    }

    #[test]
    fn live_round_at_height_is_visible_for_proposal_guard() {
        use super::PbftEngine;

        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let block = test_block("node-1");
        let block_hash = block.hash();
        let mut engine = PbftEngine::new();
        assert!(engine.non_terminal_round_at_height(1).is_none());
        engine
            .start_round(block, validator_set("node-1", &signing_key))
            .unwrap();

        let found = engine
            .non_terminal_round_at_height(1)
            .expect("live round at height 1");
        assert_eq!(found.0, block_hash);
        assert_eq!(found.1, PbftPhase::Prepare);
        assert!(engine.non_terminal_round_at_height(2).is_none());
    }

    #[test]
    fn duplicate_proposal_keeps_votes_and_conflicting_same_view_is_rejected() {
        use super::PbftEngine;

        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let authority = validator_set("node-1", &signing_key);
        let block = test_block("node-1");
        let block_hash = block.hash();
        let mut engine = PbftEngine::new();
        engine
            .start_round(block.clone(), authority.clone())
            .unwrap();
        assert!(engine
            .prepare(
                &block_hash,
                "node-1",
                signed_block_vote("node-1", &signing_key, "prepare", &block_hash),
            )
            .unwrap());

        assert_eq!(
            engine
                .start_round(block.clone(), authority.clone())
                .unwrap(),
            block_hash
        );
        assert_eq!(engine.observability_snapshot()[0].phase, "commit");

        let mut conflicting = block;
        conflicting.header.timestamp += chrono::Duration::seconds(1);
        assert_ne!(conflicting.hash(), block_hash);
        assert!(engine.start_round(conflicting, authority).is_err());
    }

    #[test]
    fn terminal_rounds_do_not_block_new_proposals() {
        use super::PbftEngine;

        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let block = test_block("node-1");
        let block_hash = block.hash();
        let mut engine = PbftEngine::new();
        engine
            .start_round(block, validator_set("node-1", &signing_key))
            .unwrap();

        // Single-validator set: own prepare reaches quorum, own commit finalises.
        let prepare = signed_block_vote("node-1", &signing_key, "prepare", &block_hash);
        assert!(engine.prepare(&block_hash, "node-1", prepare).unwrap());
        let commit = signed_block_vote("node-1", &signing_key, "commit", &block_hash);
        assert!(engine.commit(&block_hash, "node-1", commit).unwrap());
        assert!(engine.non_terminal_round_at_height(1).is_none());
    }

    #[test]
    fn own_votes_for_rebroadcast_collects_only_own_live_votes() {
        use super::PbftEngine;

        let k0 = SigningKey::from_bytes(&[7u8; 32]);
        let k1 = SigningKey::from_bytes(&[8u8; 32]);
        let mut vs = validator_set("node-1", &k0);
        vs.add(ValidatorInfo {
            id: "node-2".into(),
            pubkey: hex::encode(k1.verifying_key().as_bytes()),
            eth_address: None,
            stake: 100,
            reputation: 100,
            is_active: true,
        });
        let proposer = selected_proposer(&vs, &"11".repeat(32));
        let block = test_block(&proposer);
        let block_hash = block.hash();
        let mut engine = PbftEngine::new();
        engine.start_round(block, vs).unwrap();

        // Quorum is 2: one vote keeps the round live in Prepare.
        let prepare = signed_block_vote("node-1", &k0, "prepare", &block_hash);
        assert!(!engine.prepare(&block_hash, "node-1", prepare).unwrap());

        let ours = engine.own_votes_for_rebroadcast("node-1");
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].block_hash, block_hash);
        assert_eq!(ours[0].block_height, 1);
        assert_eq!(ours[0].phase, PbftPhase::Prepare);
        assert!(ours[0].prepare_sig.is_some());
        assert!(ours[0].commit_sig.is_none());
        assert!(ours[0].phase_timeout_seconds > 0.0);
        assert!(engine.own_votes_for_rebroadcast("node-2").is_empty());

        // Second voter reaches prepare quorum; our commit is then included too.
        let prepare2 = signed_block_vote("node-2", &k1, "prepare", &block_hash);
        assert!(engine.prepare(&block_hash, "node-2", prepare2).unwrap());
        let commit = signed_block_vote("node-1", &k0, "commit", &block_hash);
        assert!(!engine.commit(&block_hash, "node-1", commit).unwrap());
        let ours = engine.own_votes_for_rebroadcast("node-1");
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].phase, PbftPhase::Commit);
        assert!(ours[0].commit_sig.is_some());

        // Finalised rounds are excluded.
        let commit2 = signed_block_vote("node-2", &k1, "commit", &block_hash);
        assert!(engine.commit(&block_hash, "node-2", commit2).unwrap());
        assert!(engine.own_votes_for_rebroadcast("node-1").is_empty());
    }

    #[test]
    fn pbft_engine_bounds_tracked_collections() {
        use super::{PbftEngine, MAX_CERTIFIED_VIEWS, MAX_TRACKED_ROUNDS, MAX_VIEW_CHANGE_BUCKETS};

        let (vs, keys) = two_validator_set();
        let mut engine = PbftEngine::new();
        for height in 1..=(MAX_TRACKED_ROUNDS as u64 + 25) {
            let prev_hash = format!("{height:064x}");
            let proposer = selected_proposer(&vs, &prev_hash);
            let mut block = test_block(&proposer);
            block.header.height = height;
            block.header.prev_hash = prev_hash;
            engine.start_round(block, vs.clone()).unwrap();
            assert!(engine.active_round_count() <= MAX_TRACKED_ROUNDS);
        }
        assert_eq!(engine.active_round_count(), MAX_TRACKED_ROUNDS);

        for height in 1..=(MAX_VIEW_CHANGE_BUCKETS as u64 + 16) {
            let prev_hash = format!("{height:064x}");
            for (index, key) in keys.iter().enumerate() {
                let vote = signed_view_change_vote(
                    common::ViewChangeVote {
                        height,
                        prev_hash: prev_hash.clone(),
                        new_view: 1,
                        validator_id: format!("v{index}"),
                        prepared: None,
                        signature: String::new(),
                    },
                    key,
                );
                let _ = engine.receive_view_change(vote, &vs).unwrap();
            }
            assert!(engine.view_change_votes.len() <= MAX_VIEW_CHANGE_BUCKETS);
            assert!(engine.certified_views.len() <= MAX_CERTIFIED_VIEWS);
        }
    }
}
