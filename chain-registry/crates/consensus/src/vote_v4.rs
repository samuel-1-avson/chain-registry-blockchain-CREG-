//! `creg-vote-v4` Consensus Rules, Height-Gated Hybrid PQ PBFT Verification,
//! View-Change Deduplication, Witness Replay Guarding, and Quantized Class C Quorum
//! (`ADR-017` + `ADR-018` Increment 6).

use anyhow::{bail, Context, Result};
use common::crypto::{
    PublicKey, Signature, SignatureScheme, SigningKey, VerifyMode, ED25519_SIGNATURE_LEN,
    MLDSA44_SIGNATURE_LEN,
};
use common::vote_v4::MLDSA44_SIGNATURE_HEX_LEN;
use common::{
    sha256_hex, validator_status_is_active, BlockSignature, ConsensusParams, PackageStatus,
    PreparedBlockHeader, PreparedCertificate, ReproposalProof, SemanticDispositionV4, Transaction,
    TriggerWitnessV1, Validator, ViewChangeVote, VoteV4Envelope, CLASS_C_QUARANTINE_TTL_SECS,
    MAX_WITNESS_REPLAYS_PER_PACKAGE,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Whether post-activation Hybrid Ed25519 + ML-DSA-44 consensus verification
/// is enforced at `height`.
pub fn pbft_hybrid_required(params: &ConsensusParams, height: u64) -> bool {
    params.vote_v4_active_at(height)
}

/// Sign a PBFT phase message (`"prepare"` / `"commit"`) at `height`.
/// Pre-activation heights emit classical Ed25519 (`pq_signature: None`);
/// activation height and above require `HybridEd25519MlDsa44V1`.
pub fn sign_pbft_phase_at_height(
    phase: &str,
    block_hash: &str,
    validator_id: &str,
    height: u64,
    params: &ConsensusParams,
    signing_key: &SigningKey,
) -> Result<BlockSignature> {
    let require_hybrid = params.vote_v4_active_at(height);
    if require_hybrid && signing_key.scheme() != SignatureScheme::HybridEd25519MlDsa44V1 {
        bail!(
            "post-activation PBFT {} signature at height {} requires HybridEd25519MlDsa44V1",
            phase,
            height
        );
    }
    if !require_hybrid && signing_key.scheme() != SignatureScheme::Ed25519 {
        bail!(
            "pre-activation PBFT {} signature at height {} must use classical Ed25519",
            phase,
            height
        );
    }
    let pk = signing_key.public_key();
    let ed_pubkey_hex = match pk.hybrid_parts() {
        Some((ed, _)) => hex::encode(ed),
        None => pk.to_hex(),
    };
    let msg = crate::pbft::pbft_signature_message(phase, block_hash);
    let sig = signing_key.sign(msg.as_bytes());
    Ok(BlockSignature {
        validator_id: validator_id.to_string(),
        pubkey: ed_pubkey_hex,
        signature: hex::encode(sig.ed25519_bytes()),
        pq_signature: sig.mldsa44_bytes().map(hex::encode),
    })
}

/// Verify a PBFT phase signature (`"prepare"` / `"commit"`) at `height`.
/// Pre-activation rejects any attached `pq_signature`; activation height and
/// above non-separably require both Ed25519 and FIPS 204 ML-DSA-44 halves
/// bound to `validator`'s identity.
pub fn verify_pbft_phase_signature_at_height(
    phase: &str,
    block_hash: &str,
    height: u64,
    params: &ConsensusParams,
    validator: &Validator,
    sig: &BlockSignature,
) -> Result<()> {
    if !validator_status_is_active(&validator.status) {
        bail!("validator {} is not active", validator.id);
    }
    if sig.validator_id != validator.id {
        bail!(
            "PBFT {} signature validator mismatch: expected {}, got {}",
            phase,
            validator.id,
            sig.validator_id
        );
    }
    let expected_ed = validator
        .pubkey
        .trim_start_matches("0x")
        .to_ascii_lowercase();
    let supplied_ed = sig.pubkey.trim_start_matches("0x").to_ascii_lowercase();
    if expected_ed != supplied_ed {
        bail!(
            "PBFT {} signature pubkey mismatch for validator {}",
            phase,
            validator.id
        );
    }

    let require_hybrid = params.vote_v4_active_at(height);
    let pk: PublicKey = validator.verifying_key(require_hybrid)?;
    let ed_bytes = hex::decode(sig.signature.trim_start_matches("0x"))
        .with_context(|| format!("invalid PBFT {phase} Ed25519 signature hex"))?;
    if ed_bytes.len() != ED25519_SIGNATURE_LEN {
        bail!(
            "invalid PBFT {} Ed25519 signature length: expected {}, got {}",
            phase,
            ED25519_SIGNATURE_LEN,
            ed_bytes.len()
        );
    }

    let parsed_sig = match (require_hybrid, &sig.pq_signature) {
        (false, Some(_)) => bail!(
            "pre-activation PBFT {} signature at height {} must not carry pq_signature",
            phase,
            height
        ),
        (false, None) => Signature::from_bytes(SignatureScheme::Ed25519, &ed_bytes)?,
        (true, None) => bail!(
            "post-activation PBFT {} signature at height {} requires pq_signature",
            phase,
            height
        ),
        (true, Some(pq_hex)) => {
            if pq_hex.len() != MLDSA44_SIGNATURE_HEX_LEN
                || !pq_hex
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                bail!(
                    "post-activation PBFT {} pq_signature must be {} lowercase hex chars",
                    phase,
                    MLDSA44_SIGNATURE_HEX_LEN
                );
            }
            let pq_bytes = hex::decode(pq_hex)
                .with_context(|| format!("invalid PBFT {phase} ML-DSA-44 signature hex"))?;
            if pq_bytes.len() != MLDSA44_SIGNATURE_LEN {
                bail!(
                    "invalid PBFT {} ML-DSA-44 signature length: expected {}, got {}",
                    phase,
                    MLDSA44_SIGNATURE_LEN,
                    pq_bytes.len()
                );
            }
            Signature::hybrid_from_parts(&ed_bytes, &pq_bytes)?
        }
    };

    let msg = crate::pbft::pbft_signature_message(phase, block_hash);
    pk.verify(msg.as_bytes(), &parsed_sig, VerifyMode::Strict)
        .with_context(|| format!("PBFT {phase} hybrid verification failed at height {height}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeduplicatedPreparedPayload {
    pub header: PreparedBlockHeader,
    pub prepared_view: u32,
    pub original_view_change_certificate: Vec<ViewChangeVote>,
    pub reproposal_proof: Option<Box<ReproposalProof>>,
    pub transactions: Vec<Transaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactViewChangeVote {
    pub height: u64,
    pub prev_hash: String,
    pub new_view: u32,
    pub validator_id: String,
    pub prepared_payload_idx: Option<usize>,
    pub prepare_sig_indices: Vec<usize>,
    pub signature: String,
}

/// Wire-deduplicated view-change certificate (`ADR-017` §5).
/// Extracts shared prepared block payloads and repeated `BlockSignature`s into
/// indexed tables so a 50-validator hybrid PQ view-change stays far below 4 MiB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeduplicatedViewChangeCertificate {
    pub prepared_payloads: Vec<DeduplicatedPreparedPayload>,
    pub prepare_signature_pool: Vec<BlockSignature>,
    pub votes: Vec<CompactViewChangeVote>,
}

impl DeduplicatedViewChangeCertificate {
    pub fn wire_bytes(&self) -> Result<usize> {
        Ok(serde_json::to_vec(self)
            .context("serialize DeduplicatedViewChangeCertificate")?
            .len())
    }

    pub fn reconstruct(&self) -> Result<Vec<ViewChangeVote>> {
        let mut out = Vec::with_capacity(self.votes.len());
        for compact in &self.votes {
            let prepared = match compact.prepared_payload_idx {
                Some(payload_idx) => {
                    let payload = self.prepared_payloads.get(payload_idx).ok_or_else(|| {
                        anyhow::anyhow!("prepared payload index {} out of bounds", payload_idx)
                    })?;
                    let mut prepare_signatures =
                        Vec::with_capacity(compact.prepare_sig_indices.len());
                    for &sig_idx in &compact.prepare_sig_indices {
                        let sig = self.prepare_signature_pool.get(sig_idx).ok_or_else(|| {
                            anyhow::anyhow!("prepare signature index {} out of bounds", sig_idx)
                        })?;
                        prepare_signatures.push(sig.clone());
                    }
                    Some(PreparedCertificate {
                        header: payload.header.clone(),
                        prepared_view: payload.prepared_view,
                        original_view_change_certificate: payload
                            .original_view_change_certificate
                            .clone(),
                        reproposal_proof: payload.reproposal_proof.clone(),
                        transactions: payload.transactions.clone(),
                        prepare_signatures,
                    })
                }
                None => {
                    if !compact.prepare_sig_indices.is_empty() {
                        bail!("compact view-change vote has sig indices without prepared payload");
                    }
                    None
                }
            };

            out.push(ViewChangeVote {
                height: compact.height,
                prev_hash: compact.prev_hash.clone(),
                new_view: compact.new_view,
                validator_id: compact.validator_id.clone(),
                prepared,
                signature: compact.signature.clone(),
            });
        }
        Ok(out)
    }
}

pub fn deduplicate_view_change_certificate(
    votes: &[ViewChangeVote],
) -> Result<DeduplicatedViewChangeCertificate> {
    let mut payload_index_by_digest: HashMap<String, usize> = HashMap::new();
    let mut sig_index_by_digest: HashMap<String, usize> = HashMap::new();
    let mut prepared_payloads: Vec<DeduplicatedPreparedPayload> = Vec::new();
    let mut prepare_signature_pool: Vec<BlockSignature> = Vec::new();
    let mut compact_votes = Vec::with_capacity(votes.len());

    for vote in votes {
        let (prepared_payload_idx, prepare_sig_indices) = match &vote.prepared {
            Some(cert) => {
                let payload = DeduplicatedPreparedPayload {
                    header: cert.header.clone(),
                    prepared_view: cert.prepared_view,
                    original_view_change_certificate: cert.original_view_change_certificate.clone(),
                    reproposal_proof: cert.reproposal_proof.clone(),
                    transactions: cert.transactions.clone(),
                };
                let payload_bytes = serde_json::to_vec(&payload)
                    .context("serialize DeduplicatedPreparedPayload")?;
                let payload_digest = sha256_hex(&payload_bytes);
                let payload_idx = if let Some(&idx) = payload_index_by_digest.get(&payload_digest) {
                    idx
                } else {
                    let idx = prepared_payloads.len();
                    prepared_payloads.push(payload);
                    payload_index_by_digest.insert(payload_digest, idx);
                    idx
                };

                let mut sig_indices = Vec::with_capacity(cert.prepare_signatures.len());
                for sig in &cert.prepare_signatures {
                    let sig_bytes = serde_json::to_vec(sig).context("serialize BlockSignature")?;
                    let sig_digest = sha256_hex(&sig_bytes);
                    let sig_idx = if let Some(&idx) = sig_index_by_digest.get(&sig_digest) {
                        idx
                    } else {
                        let idx = prepare_signature_pool.len();
                        prepare_signature_pool.push(sig.clone());
                        sig_index_by_digest.insert(sig_digest, idx);
                        idx
                    };
                    sig_indices.push(sig_idx);
                }
                (Some(payload_idx), sig_indices)
            }
            None => (None, Vec::new()),
        };

        compact_votes.push(CompactViewChangeVote {
            height: vote.height,
            prev_hash: vote.prev_hash.clone(),
            new_view: vote.new_view,
            validator_id: vote.validator_id.clone(),
            prepared_payload_idx,
            prepare_sig_indices,
            signature: vote.signature.clone(),
        });
    }

    Ok(DeduplicatedViewChangeCertificate {
        prepared_payloads,
        prepare_signature_pool,
        votes: compact_votes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessPrecheckDecision {
    RunReplay {
        witness_digest: String,
    },
    CachedResult {
        witness_digest: String,
        reproduced: bool,
    },
}

/// Deterministic DoS & fraud guard placed strictly ahead of Tier 1/2/3 sandbox
/// witness replay (`INV-04`, ADR-018 Increment 6).
///
/// 1. Validates wire budget & authenticates `witness_prover_id` + signature at `height`
///    BEFORE consuming any replay budget.
/// 2. Deduplicates identical `canonical_digest` witnesses for the same package.
/// 3. Caps distinct sandbox replays at `MAX_WITNESS_REPLAYS_PER_PACKAGE` (2).
/// 4. Tracks bogus/non-reproducing witness penalties per prover validator.
#[derive(Debug, Default)]
pub struct WitnessReplayGuard {
    replay_counts: HashMap<(String, String), usize>,
    cached_outcomes: HashMap<(String, String, String), bool>,
    bogus_penalties: HashMap<String, u32>,
}

impl WitnessReplayGuard {
    pub fn new() -> Self {
        Self::default()
    }

    fn pkg_key(pkg: &str, content_hash: &str) -> (String, String) {
        (
            pkg.trim().to_string(),
            content_hash.trim().to_ascii_lowercase(),
        )
    }

    fn record_bogus(&mut self, validator_id: &str) {
        let trimmed = validator_id.trim();
        if !trimmed.is_empty() {
            let entry = self.bogus_penalties.entry(trimmed.to_string()).or_insert(0);
            *entry = entry.saturating_add(1);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn preflight_and_reserve(
        &mut self,
        witness: &TriggerWitnessV1,
        expected_package: &str,
        expected_content_hash: &str,
        expected_policy_bundle_id: Option<&str>,
        validators: &[Validator],
        height: u64,
        params: &ConsensusParams,
    ) -> Result<WitnessPrecheckDecision> {
        if witness.package_canonical.trim() != expected_package.trim()
            || !witness
                .content_hash
                .trim()
                .eq_ignore_ascii_case(expected_content_hash.trim())
        {
            self.record_bogus(&witness.witness_prover_id);
            bail!(
                "TriggerWitnessV1 target mismatch: expected {}@{}, got {}@{}",
                expected_package,
                expected_content_hash,
                witness.package_canonical,
                witness.content_hash
            );
        }
        if let Some(expected_bundle) = expected_policy_bundle_id {
            if witness.sandbox_policy_bundle_id.trim() != expected_bundle.trim() {
                self.record_bogus(&witness.witness_prover_id);
                bail!(
                    "TriggerWitnessV1 policy bundle mismatch: expected {}, got {}",
                    expected_bundle,
                    witness.sandbox_policy_bundle_id
                );
            }
        }

        let Some(prover) = validators
            .iter()
            .find(|v| v.id == witness.witness_prover_id && validator_status_is_active(&v.status))
        else {
            self.record_bogus(&witness.witness_prover_id);
            bail!(
                "TriggerWitnessV1 prover '{}' is not an active validator",
                witness.witness_prover_id
            );
        };

        let require_hybrid = params.vote_v4_active_at(height);
        let pk = match prover.verifying_key(require_hybrid) {
            Ok(pk) => pk,
            Err(err) => {
                self.record_bogus(&witness.witness_prover_id);
                return Err(err);
            }
        };
        if let Err(err) = witness.verify(&pk, require_hybrid) {
            self.record_bogus(&witness.witness_prover_id);
            return Err(err);
        }

        let (pkg, hash) = Self::pkg_key(expected_package, expected_content_hash);
        let witness_digest = witness.canonical_digest();
        let cache_key = (pkg.clone(), hash.clone(), witness_digest.clone());
        if let Some(&reproduced) = self.cached_outcomes.get(&cache_key) {
            return Ok(WitnessPrecheckDecision::CachedResult {
                witness_digest,
                reproduced,
            });
        }

        let used = self
            .replay_counts
            .get(&(pkg.clone(), hash.clone()))
            .copied()
            .unwrap_or(0);
        if used >= MAX_WITNESS_REPLAYS_PER_PACKAGE {
            bail!(
                "witness replay budget exhausted ({}/{}) for {}@{}",
                used,
                MAX_WITNESS_REPLAYS_PER_PACKAGE,
                pkg,
                hash
            );
        }
        self.replay_counts.insert((pkg, hash), used + 1);
        Ok(WitnessPrecheckDecision::RunReplay { witness_digest })
    }

    pub fn record_replay_outcome(&mut self, witness: &TriggerWitnessV1, reproduced: bool) {
        let (pkg, hash) = Self::pkg_key(&witness.package_canonical, &witness.content_hash);
        let witness_digest = witness.canonical_digest();
        self.cached_outcomes
            .insert((pkg, hash, witness_digest), reproduced);
        if !reproduced {
            self.record_bogus(&witness.witness_prover_id);
        }
    }

    pub fn replays_used(&self, package_canonical: &str, content_hash: &str) -> usize {
        self.replay_counts
            .get(&Self::pkg_key(package_canonical, content_hash))
            .copied()
            .unwrap_or(0)
    }

    pub fn bogus_count(&self, validator_id: &str) -> u32 {
        self.bogus_penalties
            .get(validator_id.trim())
            .copied()
            .unwrap_or(0)
    }

    pub fn should_slash_for_bogus_witness(&self, validator_id: &str) -> bool {
        self.bogus_count(validator_id) > 0
    }
}

#[derive(Debug, Clone)]
pub struct VoteV4QuorumConfig {
    pub assigned_count: usize,
    pub allow_small_cluster_quorum: bool,
    pub min_model_families: usize,
    pub quarantine_ttl_secs: i64,
    pub require_semantic_vote_for_approval: bool,
}

impl Default for VoteV4QuorumConfig {
    fn default() -> Self {
        Self {
            assigned_count: 4,
            allow_small_cluster_quorum: false,
            min_model_families: 2,
            quarantine_ttl_secs: CLASS_C_QUARANTINE_TTL_SECS,
            require_semantic_vote_for_approval: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoteV4Outcome {
    Pending {
        valid_votes: usize,
        approvals: usize,
        rejections: usize,
        quarantine_reviews: usize,
        abstentions: usize,
        quorum: usize,
    },
    Approved {
        approving_validators: Vec<String>,
        distinct_model_families: BTreeSet<String>,
    },
    Quarantined {
        reason: String,
        expires_at_unix: i64,
        approvals: usize,
        quarantine_reviews: usize,
        distinct_model_families: BTreeSet<String>,
    },
    Rejected {
        reason: String,
        witness_digest: Option<String>,
        deterministic_floor: bool,
    },
}

fn vote_quorum_threshold(assigned_count: usize, allow_small_cluster_quorum: bool) -> usize {
    match assigned_count {
        0 => 0,
        3 if allow_small_cluster_quorum => 2,
        n => (2 * n / 3) + 1,
    }
}

/// Aggregate quantized `creg-vote-v4` votes under `ADR-018` precedence rules:
/// - Verified Class B `TriggerWitnessV1` deterministically escalates to `Rejected`,
///   overriding even unanimous oblivious `Approve` votes (`INV-02`, `INV-04`).
/// - Class A deterministic rejections cannot be overridden by Class C approvals (`INV-03`).
/// - Quantized Class C approvals must satisfy both quorum and model-family diversity;
///   single-family or abstaining rounds fail closed into `Quarantined` (72h TTL).
#[allow(clippy::too_many_arguments)]
pub fn aggregate_vote_v4_quorum(
    votes: &[VoteV4Envelope],
    expected_package: &str,
    expected_content_hash: &str,
    validators: &[Validator],
    height: u64,
    params: &ConsensusParams,
    config: &VoteV4QuorumConfig,
    verified_witness_digests: &HashSet<String>,
    now_unix: i64,
) -> Result<VoteV4Outcome> {
    let effective_assigned = config.assigned_count.max(1);
    let quorum = vote_quorum_threshold(effective_assigned, config.allow_small_cluster_quorum);
    let require_hybrid = params.vote_v4_active_at(height);
    let active_validators: HashMap<&str, &Validator> = validators
        .iter()
        .filter(|v| validator_status_is_active(&v.status))
        .map(|v| (v.id.as_str(), v))
        .collect();

    let mut accepted: HashMap<String, &VoteV4Envelope> = HashMap::new();
    let mut equivocated: HashSet<String> = HashSet::new();

    for vote in votes {
        if equivocated.contains(&vote.validator_id) {
            continue;
        }
        let Some(validator) = active_validators.get(vote.validator_id.as_str()).copied() else {
            continue;
        };
        if vote.package_canonical.trim() != expected_package.trim()
            || !vote
                .content_hash
                .trim()
                .eq_ignore_ascii_case(expected_content_hash.trim())
        {
            continue;
        }
        if vote.verify(validator, require_hybrid).is_err() {
            continue;
        }
        if let Some(prev) = accepted.get(&vote.validator_id) {
            if prev.canonical_vote_v4_message() != vote.canonical_vote_v4_message() {
                accepted.remove(&vote.validator_id);
                equivocated.insert(vote.validator_id.clone());
            }
            continue;
        }
        accepted.insert(vote.validator_id.clone(), vote);
    }

    let mut sorted_votes: Vec<&VoteV4Envelope> = accepted.into_values().collect();
    sorted_votes.sort_by(|a, b| a.validator_id.cmp(&b.validator_id));

    // Precedence 1: Any peer-verified reproducing Class B `TriggerWitnessV1`
    // immediately converges to Rejected, overriding oblivious Approve votes.
    for vote in &sorted_votes {
        if let Some(ref witness) = vote.witness {
            if let Some(prover) = active_validators
                .get(witness.witness_prover_id.as_str())
                .copied()
            {
                if let Ok(prover_pk) = prover.verifying_key(require_hybrid) {
                    if witness.verify(&prover_pk, require_hybrid).is_ok()
                        && verified_witness_digests.contains(&witness.canonical_digest())
                    {
                        return Ok(VoteV4Outcome::Rejected {
                            reason: format!(
                                "verified reproducing TriggerWitnessV1 ({}) by {}",
                                witness.observed_violations.join(","),
                                witness.witness_prover_id
                            ),
                            witness_digest: Some(witness.canonical_digest()),
                            deterministic_floor: true,
                        });
                    }
                }
            }
        }
    }

    let mut approving_validators = Vec::new();
    let mut distinct_model_families = BTreeSet::new();
    let mut rejections = 0usize;
    let mut class_a_rejects = 0usize;
    let mut quarantine_reviews = 0usize;
    let mut abstentions = 0usize;
    let mut first_reject_reason: Option<String> = None;

    for vote in &sorted_votes {
        let degraded = vote.ml_model_version.trim().starts_with("degraded-")
            || vote.scanner_profile_digest.trim().is_empty()
            || vote.evidence_digest.trim().is_empty();
        if degraded {
            abstentions += 1;
            continue;
        }

        let has_class_a = vote.deterministic_risk.critical_findings > 0
            || vote
                .deterministic_risk
                .disposition
                .eq_ignore_ascii_case("reject");

        if !vote.approved || has_class_a {
            let is_quarantine_hold = !has_class_a
                && vote
                    .semantic_vote
                    .as_ref()
                    .is_some_and(|sv| sv.disposition == SemanticDispositionV4::QuarantineReview);
            if is_quarantine_hold {
                quarantine_reviews += 1;
            } else {
                rejections += 1;
                if has_class_a {
                    class_a_rejects += 1;
                }
                if first_reject_reason.is_none() {
                    first_reject_reason = Some(
                        vote.reject_reason
                            .clone()
                            .unwrap_or_else(|| "consensus vote rejected package".to_string()),
                    );
                }
            }
            continue;
        }

        match &vote.semantic_vote {
            Some(sv) if sv.disposition == SemanticDispositionV4::Reject => {
                rejections += 1;
                if first_reject_reason.is_none() {
                    first_reject_reason = Some(
                        vote.reject_reason
                            .clone()
                            .unwrap_or_else(|| "semantic vote rejected package".to_string()),
                    );
                }
            }
            Some(sv) if sv.disposition == SemanticDispositionV4::QuarantineReview => {
                quarantine_reviews += 1;
            }
            Some(sv) => {
                approving_validators.push(vote.validator_id.clone());
                distinct_model_families.insert(sv.model_family.trim().to_ascii_lowercase());
            }
            None => {
                if config.require_semantic_vote_for_approval || config.min_model_families > 0 {
                    abstentions += 1;
                } else {
                    approving_validators.push(vote.validator_id.clone());
                }
            }
        }
    }

    if rejections >= quorum {
        return Ok(VoteV4Outcome::Rejected {
            reason: first_reject_reason.unwrap_or_else(|| "quorum rejected package".to_string()),
            witness_digest: None,
            deterministic_floor: class_a_rejects >= quorum,
        });
    }

    let approvals = approving_validators.len();
    let total_valid = sorted_votes.len();
    let remaining_unvoted = effective_assigned.saturating_sub(total_valid);
    let max_possible_approvals = approvals + remaining_unvoted;
    let ttl = config.quarantine_ttl_secs.max(1);

    if approvals >= quorum {
        if config.min_model_families > 0
            && distinct_model_families.len() < config.min_model_families
        {
            return Ok(VoteV4Outcome::Quarantined {
                reason: format!(
                    "Class C approval quorum ({}/{}) lacks required model-family diversity ({}/{} distinct families)",
                    approvals,
                    quorum,
                    distinct_model_families.len(),
                    config.min_model_families
                ),
                expires_at_unix: now_unix.saturating_add(ttl),
                approvals,
                quarantine_reviews,
                distinct_model_families,
            });
        }
        return Ok(VoteV4Outcome::Approved {
            approving_validators,
            distinct_model_families,
        });
    }

    if max_possible_approvals < quorum {
        if quarantine_reviews > 0 || abstentions > 0 {
            return Ok(VoteV4Outcome::Quarantined {
                reason: format!(
                    "Class C approval quorum unreachable ({}/{} approvals, {} quarantine_review, {} abstentions, {} rejections)",
                    approvals, quorum, quarantine_reviews, abstentions, rejections
                ),
                expires_at_unix: now_unix.saturating_add(ttl),
                approvals,
                quarantine_reviews,
                distinct_model_families,
            });
        }
        return Ok(VoteV4Outcome::Rejected {
            reason: first_reject_reason
                .unwrap_or_else(|| "approval quorum unreachable".to_string()),
            witness_digest: None,
            deterministic_floor: class_a_rejects > 0,
        });
    }

    Ok(VoteV4Outcome::Pending {
        valid_votes: total_valid,
        approvals,
        rejections,
        quarantine_reviews,
        abstentions,
        quorum,
    })
}

/// Verify a committee quorum certificate under the consensus rules in force
/// at `height` (ADR-019 §3.2, `ECA-02`, `ECO-INV-03`). `authority` must be
/// the validator set authenticated at `qc.authority_height`. Hybrid
/// signatures are required exactly when `vote_v4_active_at(height)`.
/// Returns the number of distinct valid committee signers, which is at
/// least `2m/3 + 1` of the committee's `m` signers on success.
pub fn verify_committee_qc(
    qc: &common::CommitteeQuorumCertificateV1,
    expected_domain: &str,
    committee: &common::AnalysisCommittee,
    authority: &[Validator],
    params: &ConsensusParams,
    height: u64,
) -> Result<usize> {
    if qc.authority_height > height {
        bail!(
            "committee QC authority height {} is above the block height {}",
            qc.authority_height,
            height
        );
    }
    qc.verify_quorum(
        expected_domain,
        committee,
        authority,
        params.vote_v4_active_at(height),
    )
}

/// Fail-closed 72h timeout handler for `PackageStatus::Quarantined` (`ADR-018`).
pub fn resolve_quarantine_timeout(status: &PackageStatus, now_unix: i64) -> Option<PackageStatus> {
    match status {
        PackageStatus::Quarantined {
            reason,
            expires_at_unix,
        } if now_unix >= *expires_at_unix => Some(PackageStatus::Revoked {
            reason: format!(
                "Class C quarantine TTL expired without resolution (fail-closed): {}",
                reason
            ),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{AnalysisBundleRefs, DeterministicRiskSummary, SemanticVoteV4, TriggerSpecWireV1};
    use std::collections::BTreeMap;

    fn make_validator_pair(id: &str, seed_byte: u8) -> (Validator, SigningKey, SigningKey) {
        let ed_seed = [seed_byte; 32];
        let pq_seed = [seed_byte.wrapping_add(50); 32];
        let ed_sk = SigningKey::from_ed25519_seed(&ed_seed);
        let hybrid_sk = SigningKey::hybrid_from_seeds(&ed_seed, &pq_seed);
        let (_, pq_pk) = hybrid_sk.public_key().hybrid_parts().unwrap();
        let val = Validator {
            id: id.into(),
            alias: id.into(),
            pubkey: ed_sk.public_key().to_hex(),
            pq_pubkey: Some(hex::encode(pq_pk)),
            eth_address: format!("0x{:040x}", seed_byte),
            stake: 1_000,
            reputation: 100,
            status: "active".into(),
            ..Default::default()
        };
        (val, ed_sk, hybrid_sk)
    }

    fn test_params(activation_height: Option<u64>) -> ConsensusParams {
        ConsensusParams {
            consensus_encoding: Some("phase-c-v1".into()),
            block_time_seconds: 5,
            vote_timeout_ms: 5_000,
            quorum_percentage: 67,
            min_validator_stake_wei: "0".into(),
            min_publisher_stake_wei: "0".into(),
            unbonding_period_seconds: 604_800,
            slash_penalty_low_bp: 500,
            slash_penalty_medium_bp: 2500,
            slash_penalty_critical_bp: 10000,
            max_validators: 50,
            namespace_authority_activation_height: None,
            namespace_authority_policies: vec![],
            hybrid_signature_activation_height: activation_height,
        }
    }

    #[test]
    fn committee_qc_follows_the_activation_height_and_authority_bound() {
        let members: Vec<_> = (0..4)
            .map(|i| make_validator_pair(&format!("v{i}"), 60 + i as u8))
            .collect();
        let authority: Vec<Validator> = members.iter().map(|(v, _, _)| v.clone()).collect();
        let committee = common::AnalysisCommittee {
            size: 4,
            effective_size: 4,
            members: authority.iter().map(|v| v.id.clone()).collect(),
            seed: "seed".into(),
            ..Default::default()
        };
        let params = test_params(Some(100));
        let base = common::CommitteeQuorumCertificateV1::new(
            common::REVOKE_QC_DOMAIN,
            "npm/x@1.0.0",
            &"aa".repeat(32),
            &"bb".repeat(32),
            &committee,
            90,
        );

        let mut classical = base.clone();
        for (validator, ed_sk, _) in members.iter().take(3) {
            classical.add_signature(classical.sign_member(&validator.id, ed_sk));
        }
        assert_eq!(
            verify_committee_qc(
                &classical,
                common::REVOKE_QC_DOMAIN,
                &committee,
                &authority,
                &params,
                99
            )
            .unwrap(),
            3
        );
        // At activation, Ed25519-only certificates no longer count.
        assert!(verify_committee_qc(
            &classical,
            common::REVOKE_QC_DOMAIN,
            &committee,
            &authority,
            &params,
            100
        )
        .is_err());

        let mut hybrid = base.clone();
        for (validator, _, hyb_sk) in members.iter().take(3) {
            hybrid.add_signature(hybrid.sign_member(&validator.id, hyb_sk));
        }
        assert_eq!(
            verify_committee_qc(
                &hybrid,
                common::REVOKE_QC_DOMAIN,
                &committee,
                &authority,
                &params,
                100
            )
            .unwrap(),
            3
        );
        // The authority height cannot be in the future of the block.
        assert!(verify_committee_qc(
            &classical,
            common::REVOKE_QC_DOMAIN,
            &committee,
            &authority,
            &params,
            80
        )
        .is_err());
    }

    #[test]
    fn height_boundary_pbft_phase_sig_non_separable() {
        let (val, ed_sk, hyb_sk) = make_validator_pair("v1", 3);
        let params = test_params(Some(100));
        let hash = "ab".repeat(32);

        let pre_sig = sign_pbft_phase_at_height("prepare", &hash, &val.id, 99, &params, &ed_sk)
            .expect("sign pre-activation");
        assert!(pre_sig.pq_signature.is_none());
        assert!(verify_pbft_phase_signature_at_height(
            "prepare", &hash, 99, &params, &val, &pre_sig
        )
        .is_ok());
        assert!(verify_pbft_phase_signature_at_height(
            "prepare", &hash, 100, &params, &val, &pre_sig
        )
        .is_err());

        let post_sig = sign_pbft_phase_at_height("commit", &hash, &val.id, 100, &params, &hyb_sk)
            .expect("sign post-activation");
        assert!(post_sig.pq_signature.is_some());
        assert!(verify_pbft_phase_signature_at_height(
            "commit", &hash, 100, &params, &val, &post_sig
        )
        .is_ok());
        assert!(verify_pbft_phase_signature_at_height(
            "commit", &hash, 99, &params, &val, &post_sig
        )
        .is_err());

        // Stripping PQ half at height 100 fails both post-activation and classical checks.
        let mut stripped = post_sig.clone();
        stripped.pq_signature = None;
        assert!(verify_pbft_phase_signature_at_height(
            "commit", &hash, 100, &params, &val, &stripped
        )
        .is_err());
        assert!(verify_pbft_phase_signature_at_height(
            "commit", &hash, 99, &params, &val, &stripped
        )
        .is_err());
    }

    #[test]
    fn quarantine_timeout_transitions_fail_closed_to_revoked() {
        let q = PackageStatus::Quarantined {
            reason: "single model family".into(),
            expires_at_unix: 1_000 + CLASS_C_QUARANTINE_TTL_SECS,
        };
        assert_eq!(resolve_quarantine_timeout(&q, 1_000 + 3600), None);
        let resolved = resolve_quarantine_timeout(&q, 1_000 + CLASS_C_QUARANTINE_TTL_SECS);
        assert!(matches!(resolved, Some(PackageStatus::Revoked { .. })));
    }

    #[test]
    fn witness_replay_guard_enforces_auth_before_budget() {
        let (v1, _, hyb_1) = make_validator_pair("v1", 1);
        let (v2, _, hyb_2) = make_validator_pair("v2", 2);
        let validators = vec![v1.clone(), v2.clone()];
        let params = test_params(Some(10));

        let mut env = BTreeMap::new();
        env.insert("CI".into(), "true".into());
        let mut w = TriggerWitnessV1 {
            schema_version: 1,
            package_canonical: "npm:pkg@1.0.0".into(),
            content_hash: "aa".repeat(32),
            manifest_sha256: "bb".repeat(32),
            sandbox_policy_bundle_id: "bundle-v1".into(),
            trigger_spec: TriggerSpecWireV1 {
                schema_version: 1,
                env_overrides: env,
                clock_offset_secs: 0,
                entry_function: None,
                args: Vec::new(),
            },
            observed_violations: vec!["KB-CANARY-EXFIL".into()],
            graph_canonical_digest: "cc".repeat(32),
            witness_prover_id: v1.id.clone(),
            prover_key_id: hyb_1.key_id(),
            prover_signature: String::new(),
            prover_pq_signature: None,
        };
        // Unsigned/invalid witness fails BEFORE consuming replay budget.
        let mut guard = WitnessReplayGuard::new();
        assert!(guard
            .preflight_and_reserve(
                &w,
                "npm:pkg@1.0.0",
                &"aa".repeat(32),
                Some("bundle-v1"),
                &validators,
                10,
                &params,
            )
            .is_err());
        assert_eq!(guard.replays_used("npm:pkg@1.0.0", &"aa".repeat(32)), 0);
        assert_eq!(guard.bogus_count(&v1.id), 1);

        // Signed witness #1 consumes slot 1 and caches result.
        w.sign_with(&hyb_1).unwrap();
        let d1 = guard
            .preflight_and_reserve(
                &w,
                "npm:pkg@1.0.0",
                &"aa".repeat(32),
                Some("bundle-v1"),
                &validators,
                10,
                &params,
            )
            .unwrap();
        assert!(matches!(d1, WitnessPrecheckDecision::RunReplay { .. }));
        guard.record_replay_outcome(&w, true);
        let d1_cached = guard
            .preflight_and_reserve(
                &w,
                "npm:pkg@1.0.0",
                &"aa".repeat(32),
                Some("bundle-v1"),
                &validators,
                10,
                &params,
            )
            .unwrap();
        assert_eq!(
            d1_cached,
            WitnessPrecheckDecision::CachedResult {
                witness_digest: w.canonical_digest(),
                reproduced: true
            }
        );

        // Signed witness #2 consumes slot 2, fails reproduction -> slash flag set;
        // #3 is capped at MAX_WITNESS_REPLAYS_PER_PACKAGE (2).
        w.witness_prover_id = v2.id.clone();
        w.trigger_spec.clock_offset_secs = 60;
        w.sign_with(&hyb_2).unwrap();
        assert!(guard
            .preflight_and_reserve(
                &w,
                "npm:pkg@1.0.0",
                &"aa".repeat(32),
                Some("bundle-v1"),
                &validators,
                10,
                &params,
            )
            .is_ok());
        guard.record_replay_outcome(&w, false);
        assert!(guard.should_slash_for_bogus_witness(&v2.id));

        w.trigger_spec.clock_offset_secs = 120;
        w.sign_with(&hyb_2).unwrap();
        assert!(guard
            .preflight_and_reserve(
                &w,
                "npm:pkg@1.0.0",
                &"aa".repeat(32),
                Some("bundle-v1"),
                &validators,
                10,
                &params,
            )
            .is_err());
    }

    #[test]
    fn aggregate_vote_v4_enforces_class_c_diversity() {
        let (v1, _, sk1) = make_validator_pair("v1", 11);
        let (v2, _, sk2) = make_validator_pair("v2", 12);
        let (v3, _, sk3) = make_validator_pair("v3", 13);
        let (v4, _, _sk4) = make_validator_pair("v4", 14);
        let validators = vec![v1.clone(), v2.clone(), v3.clone(), v4];
        let params = test_params(Some(5));

        let mk_vote = |id: &str, sk: &SigningKey, family: &str| {
            let mut env = VoteV4Envelope {
                package_canonical: "npm:clean@1.0.0".into(),
                content_hash: "aa".repeat(32),
                approved: true,
                reject_reason: None,
                validator_id: id.into(),
                voter_key_id: sk.key_id(),
                scanner_profile_digest: "bb".repeat(32),
                evidence_digest: "cc".repeat(32),
                publisher_authorization_digest: "dd".repeat(32),
                vote_metadata_digest: "ee".repeat(32),
                ml_model_version: "creg-static-v1".into(),
                analysis_bundles: AnalysisBundleRefs::default(),
                deterministic_risk: DeterministicRiskSummary::default(),
                is_mandatory: true,
                witness: None,
                semantic_vote: Some(SemanticVoteV4 {
                    disposition: SemanticDispositionV4::Approve,
                    model_family: family.into(),
                    finding_digests: Vec::new(),
                }),
                signature: String::new(),
                pq_signature: None,
            };
            env.sign_with(sk).unwrap();
            env
        };

        let same_family = vec![
            mk_vote(&v1.id, &sk1, "openai"),
            mk_vote(&v2.id, &sk2, "openai"),
            mk_vote(&v3.id, &sk3, "openai"),
        ];
        let res = aggregate_vote_v4_quorum(
            &same_family,
            "npm:clean@1.0.0",
            &"aa".repeat(32),
            &validators,
            5,
            &params,
            &VoteV4QuorumConfig::default(),
            &HashSet::new(),
            10_000,
        )
        .unwrap();
        assert!(matches!(res, VoteV4Outcome::Quarantined { .. }));

        let diverse = vec![
            mk_vote(&v1.id, &sk1, "gemini"),
            mk_vote(&v2.id, &sk2, "claude"),
            mk_vote(&v3.id, &sk3, "llama"),
        ];
        let res_ok = aggregate_vote_v4_quorum(
            &diverse,
            "npm:clean@1.0.0",
            &"aa".repeat(32),
            &validators,
            5,
            &params,
            &VoteV4QuorumConfig::default(),
            &HashSet::new(),
            10_000,
        )
        .unwrap();
        assert!(matches!(res_ok, VoteV4Outcome::Approved { .. }));
    }
}
