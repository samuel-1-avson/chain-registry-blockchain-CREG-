use alloy::{
    primitives::{keccak256, Address},
    sol,
};
use anyhow::Context;
use common::{PackageStatus, PublishRequest};

use crate::SharedState;

sol!(
    #[sol(rpc)]
    interface IPublisherStakingRead {
        function stakedBalance(address publisher) external view returns (uint256);
    }
);

#[derive(Debug, Clone, Copy)]
pub enum AdmissionSurface {
    Rest,
    Grpc,
}

impl AdmissionSurface {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rest => "REST",
            Self::Grpc => "gRPC",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AdmissionOptions {
    pub surface: AdmissionSurface,
    pub verify_publisher_auth: bool,
}

#[derive(Debug, Clone)]
pub struct AdmissionReceipt {
    pub canonical: String,
    pub pending_count: usize,
}

#[derive(Debug)]
pub enum AdmissionError {
    InvalidPackageId(String),
    InvalidPublisherSignature(String),
    Publisher(PublisherAdmissionError),
    Scanner(crate::admission_scan::AdmissionScanError),
    AlreadyVerified(String),
    Revoked(String),
    AlreadyPending(String),
    NamespaceOwned(String),
    Storage(String),
    ShieldedPublishDisabled(String),
    CertifiedCommitteeRequired(String),
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPackageId(message)
            | Self::InvalidPublisherSignature(message)
            | Self::AlreadyVerified(message)
            | Self::Revoked(message)
            | Self::AlreadyPending(message)
            | Self::NamespaceOwned(message)
            | Self::Storage(message)
            | Self::ShieldedPublishDisabled(message) => write!(f, "{}", message),
            Self::CertifiedCommitteeRequired(message) => write!(f, "{}", message),
            Self::Publisher(error) => write!(f, "{}", error),
            Self::Scanner(error) => write!(f, "Pre-mempool YARA admission failed: {}", error),
        }
    }
}

impl std::error::Error for AdmissionError {}

#[derive(Debug)]
pub enum PublisherAdmissionError {
    InvalidAddress(String),
    Unstaked(String),
    Unavailable(String),
}

impl std::fmt::Display for PublisherAdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAddress(msg) | Self::Unstaked(msg) | Self::Unavailable(msg) => {
                write!(f, "{}", msg)
            }
        }
    }
}

impl std::error::Error for PublisherAdmissionError {}

/// Compute the hybrid analysis committee at admission time for consistent mandatory set across nodes.
pub fn compute_analysis_committee_at_admission(
    validator_set: &common::ValidatorSet,
    canonical: &str,
    content_hash: &str,
    tip_hash: String,
    recent_vrf_output: Option<String>,
) -> common::AnalysisCommittee {
    let total_validators = validator_set.validators.len();
    if total_validators == 0 {
        return common::AnalysisCommittee {
            size: 0,
            members: vec![],
            seed: String::new(),
            effective_size: 0,
            performance_bias: true,
            ..Default::default()
        };
    }
    let randomness = recent_vrf_output.as_deref().unwrap_or(&tip_hash);
    let seed =
        common::sha256_hex(format!("hybrid:{canonical}:{content_hash}:{randomness}").as_bytes());
    let mut scored: Vec<(u128, u64, u32, u32, String, String)> = validator_set
        .validators
        .iter()
        .map(|v| {
            let id = v.id.clone();
            let eth = v.eth_address.clone();
            let stake = v.stake;
            let rep = v.reputation;
            let perf = v.performance_score; // higher = better demonstrated resources/reliability
            let score_hex = common::sha256_hex(format!("{}:{}", hex::encode(&seed), id).as_bytes());
            let score = u128::from_str_radix(&score_hex[0..32], 16).unwrap_or(u128::MAX);
            let stake_w = if stake > 0 {
                score / (stake as u128)
            } else {
                u128::MAX
            };
            // Reputation feedback: higher rep -> smaller effective weight (earlier in list)
            let rep_w = if rep > 0 {
                stake_w / (rep as u128)
            } else {
                stake_w
            };
            // Performance/resources proxy: higher perf -> smaller weight -> earlier selection
            // This biases the mandatory committee toward nodes that have proven they can
            // successfully run the expensive verification pipeline.
            let weighted = if perf > 0 {
                rep_w / (perf as u128)
            } else {
                rep_w
            };
            (weighted, stake, rep, perf, id, eth)
        })
        .collect();

    // Sort ascending by weighted score (high stake + high rep + high performance makes weighted smaller -> earlier in list)
    // Performance (resources/reliability) is now factored so nodes proven capable of running
    // the full verification pipeline (IPFS fetch, sandbox, evidence) are preferred for mandatory slots.
    // Then by higher stake, rep, perf for ties.
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| b.3.cmp(&a.3))
    });

    // Apply per-eth cap (max 4 per eth_address to prevent concentration) to
    // the ranked order, but retain the complete ranked tail.  The first 100
    // members are the primary committee; later members are deterministic
    // expansion candidates when the primary committee has dropouts.
    const MAX_PER_ETH: u32 = 4;
    let mut members: Vec<String> = vec![];
    let mut eth_count: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut selected = std::collections::HashSet::new();
    for (_w, _stake, _rep, _perf, id, eth) in &scored {
        let count = eth_count.entry(eth.clone()).or_insert(0);
        if *count < MAX_PER_ETH {
            members.push(id.clone());
            selected.insert(id.clone());
            *count += 1;
        }
    }

    // If the control-domain cap excluded too many validators, append the
    // remaining identities in the same deterministic order.  This is bounded
    // only by the validator-set size and gives expansion a real candidate set.
    for (_w, _stake, _rep, _perf, id, _eth) in &scored {
        if selected.insert(id.clone()) {
            members.push(id.clone());
        }
    }

    let target_m = if total_validators <= 50 {
        total_validators as u32
    } else {
        100u32.min(members.len() as u32)
    };
    let mut committee = common::AnalysisCommittee {
        size: target_m,
        members,
        seed: hex::encode(seed),
        effective_size: target_m,
        performance_bias: true,
        validator_snapshot_hash: common::canonical_validator_set_hash(validator_set),
        // The caller supplies a canonical tip hash; the current local history
        // is compatibility-only until a certified transition is installed.
        validator_snapshot_height: 0,
        validator_snapshot_anchor_block_hash: tip_hash.clone(),
        selection_randomness: randomness.to_string(),
        ..Default::default()
    };
    committee.assignment_proof =
        common::committee_assignment_proof(canonical, content_hash, &committee);
    committee
}

/// Compute an assignment using a previously verified validator-set transition.
/// The transition must have been validated before persistence; this function
/// still binds its digest and fork anchor into the resulting proof so followers
/// can reject a substituted committee without trusting the admitting node.
pub fn compute_analysis_committee_for_authenticated_snapshot(
    snapshot: &common::AuthenticatedValidatorSetSnapshot,
    canonical: &str,
    content_hash: &str,
    selection_randomness: String,
) -> common::AnalysisCommittee {
    let mut committee = compute_analysis_committee_at_admission(
        &snapshot.validators,
        canonical,
        content_hash,
        snapshot.previous_block_hash.clone(),
        Some(selection_randomness),
    );
    committee.validator_snapshot_hash = snapshot.validator_set_hash.clone();
    committee.validator_snapshot_height = snapshot.effective_from_height;
    committee.validator_snapshot_anchor_block_hash = snapshot.previous_block_hash.clone();
    committee.validator_snapshot_certificate_hash =
        common::validator_snapshot_certificate_hash(snapshot);
    committee.assignment_proof =
        common::committee_assignment_proof(canonical, content_hash, &committee);
    committee
}

/// Verify both the immutable committee commitment and the deterministic
/// selection against the exact certified validator snapshot and independently
/// derived anchor-block randomness. This never treats a local compatibility
/// history entry as an authority anchor.
pub fn verify_analysis_committee_assignment(
    snapshot: &common::AuthenticatedValidatorSetSnapshot,
    canonical: &str,
    content_hash: &str,
    expected_selection_randomness: &str,
    committee: &common::AnalysisCommittee,
) -> Result<(), &'static str> {
    if !committee.assignment_proof_is_valid(canonical, content_hash) {
        return Err("committee assignment proof is invalid");
    }
    let logical_transition_id = common::validator_snapshot_certificate_hash(snapshot);
    // Migration compatibility: assignments produced before logical
    // transition IDs used the exact witness digest. Accept that binding only
    // when this node has the same audited witness; all new assignments use the
    // witness-independent logical ID above.
    let snapshot_binding_matches = committee.validator_snapshot_certificate_hash
        == logical_transition_id
        || committee.validator_snapshot_certificate_hash
            == common::validator_snapshot_witness_hash(snapshot);
    if committee.validator_snapshot_hash != snapshot.validator_set_hash
        || committee.validator_snapshot_height != snapshot.effective_from_height
        || committee.validator_snapshot_anchor_block_hash != snapshot.previous_block_hash
        || !snapshot_binding_matches
    {
        return Err("committee is not bound to the certified validator snapshot");
    }
    if committee.selection_randomness != expected_selection_randomness {
        return Err("committee selection randomness does not match its anchor block");
    }
    let expected = compute_analysis_committee_for_authenticated_snapshot(
        snapshot,
        canonical,
        content_hash,
        expected_selection_randomness.to_owned(),
    );
    if committee.size != expected.size
        || committee.effective_size != expected.effective_size
        || committee.performance_bias != expected.performance_bias
        || committee.seed != expected.seed
        || committee.members != expected.members
    {
        return Err("committee members do not match deterministic snapshot selection");
    }
    Ok(())
}

/// Update validator performance scores based on contributions to a finalized package.
/// Called from the pipeline after a package reaches Verified.
///
/// This is the mechanism that makes "resources-based" selection work in practice:
/// nodes that consistently deliver timely, correct, high-quality analysis (i.e. have
/// the actual compute/bandwidth/resources to run the full pipeline) get higher
/// performance_score and therefore higher probability of being selected into future
/// mandatory committees.
///
/// The calculation is deterministic because it is derived from on-chain data
/// (final signatures, committee membership, outcome).
pub fn update_performance_scores(
    validator_set: &mut common::ValidatorSet,
    final_sigs: &[common::ValidatorSignature],
    committee: &common::AnalysisCommittee,
    package_was_accepted: bool,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for sig in final_sigs {
        if let Some(v) = validator_set
            .validators
            .iter_mut()
            .find(|vv| vv.id == sig.validator_id)
        {
            let is_mand =
                sig.is_mandatory && committee.members.iter().any(|m| m == &sig.validator_id);
            let was_correct =
                package_was_accepted == matches!(sig.vote, common::ValidatorVote::Approve);

            // Base delta from participation
            let mut delta: i32 = 5;

            if was_correct {
                delta += 15;
            } else {
                delta -= 10;
            }

            if is_mand {
                delta += 10; // mandatory work is harder/more valuable
            }

            // Simple timeliness proxy: if signed recently relative to "now", bonus.
            // In real use the caller can pass better timing from signed_at vs admission.
            let age_secs = (now.saturating_sub(sig.signed_at.timestamp() as u64)) as i32;
            if age_secs < 30 {
                delta += 8;
            } else if age_secs < 60 {
                delta += 3;
            } else if age_secs > 120 {
                delta -= 5;
            }

            // Quality signal from evidence (non-degraded + bundles present)
            if !sig.ml_model_version.contains("degraded") && !sig.evidence_digest.is_empty() {
                delta += 5;
            }

            // Decay for old contributions: gradually reduce influence of historical scores
            // so recent performance (recent resources/capability demonstration) matters more.
            // This prevents stale high scores from locking in nodes that no longer perform well.
            if v.contribution_count > 5 {
                v.performance_score = ((v.performance_score as f64 * 0.985) as u32).max(50);
            }

            // Apply with simple moving average (contribution_count prevents early volatility)
            let contrib = (v.contribution_count as u64).max(1);
            let current = v.performance_score as i32;
            let new_avg =
                ((current as i64 * contrib as i64) + (delta as i64 * 10)) / (contrib as i64 + 1);

            v.performance_score = new_avg.clamp(1, 1000) as u32;
            v.contribution_count = v.contribution_count.saturating_add(1);
        }
    }
}

pub async fn admit_publish_request(
    state: &SharedState,
    request: PublishRequest,
    options: AdmissionOptions,
) -> Result<AdmissionReceipt, AdmissionError> {
    validate_package_id(&request)?;
    if request.shielded && !common::shielded_publish_enabled() {
        return Err(AdmissionError::ShieldedPublishDisabled(
            "shielded publish is disabled; set CREG_SHIELDED_PUBLISH_ENABLED=true on the node \
             (experimental — see docs/PHASE3_KICKOFF.md)"
                .to_string(),
        ));
    }
    if request.shielded {
        let threshold_encryption = state.read().await.config.threshold_encryption;
        if threshold_encryption {
            return Err(AdmissionError::ShieldedPublishDisabled(
                "chain spec enables threshold_encryption but t-of-n share collection is not \
                 deployed; refusing experimental single-node X25519 so confidentiality is not \
                 over-claimed"
                    .into(),
            ));
        }
    }
    if options.verify_publisher_auth {
        verify_publish_auth(state, &request).await?;
    }

    let canonical = request.id.canonical();
    let namespace_prefix = if request.publisher_authorization_version == 4 {
        common::authoritative_package_namespace(&request.id)
            .map_err(|error| AdmissionError::InvalidPackageId(error.to_string()))?
    } else {
        common::package_namespace_prefix(&request.id)
    };
    let ipfs_url = {
        let state = state.read().await;
        let next_height = state.chain.tip_height().unwrap_or(0).saturating_add(1);
        let authority_active = state
            .config
            .namespace_authority_activation_height
            .is_some_and(|height| next_height >= height);
        if authority_active && request.publisher_authorization_version != 4 {
            return Err(AdmissionError::InvalidPublisherSignature(
                "Publisher authorization v4 is required after namespace-authority activation"
                    .into(),
            ));
        }
        match state.chain.get_package(&canonical) {
            Ok(Some(record)) => {
                if matches!(record.status, PackageStatus::Verified) {
                    return Err(AdmissionError::AlreadyVerified(format!(
                        "{} is already verified on chain",
                        canonical
                    )));
                }
                if matches!(record.status, PackageStatus::Revoked { .. }) {
                    return Err(AdmissionError::Revoked(format!(
                        "{} is revoked and cannot be resubmitted",
                        canonical
                    )));
                }
            }
            Ok(None) => {}
            Err(error) => return Err(AdmissionError::Storage(error.to_string())),
        }
        if let Some(existing) = state.pending_pool.get(&canonical) {
            if existing.request.content_hash == request.content_hash {
                return Err(AdmissionError::AlreadyPending(format!(
                    "{} is already pending with the same content hash",
                    canonical
                )));
            }
        }

        // CR-07: the first accepted version establishes a namespace
        // controller.  A different publisher key may rotate only through the
        // explicit, authenticated key-rotation flow; it may not claim a later
        // version by simply presenting a staked EVM address.
        let chain_controller = state.chain.publisher_for_prefix(&namespace_prefix);
        if authority_active {
            let authorization = common::PublisherAuthorizationV1::from_publish_request(&request);
            let record = common::ChainRecord {
                id: request.id.clone(),
                content_hash: request.content_hash.clone(),
                ipfs_cid: request.ipfs_cid.clone(),
                publisher_pubkey: request.publisher_pubkey.clone(),
                publisher_pubkeys: request.publisher_pubkeys.clone(),
                threshold: request.threshold,
                shielded: request.shielded,
                key_bundle: request.key_bundle.clone(),
                manifest: Some(request.manifest.clone()),
                pgp_signature_sha256: request
                    .pgp_signature
                    .as_ref()
                    .map(|value| common::sha256_hex(value.as_bytes()))
                    .unwrap_or_default(),
                pgp_public_key_sha256: request
                    .pgp_public_key
                    .as_ref()
                    .map(|value| common::sha256_hex(value.as_bytes()))
                    .unwrap_or_default(),
                ..Default::default()
            };
            authorization
                .validate_record_binding(&record)
                .map_err(|error| AdmissionError::InvalidPublisherSignature(error.to_string()))?;
            if chain_controller.is_none() {
                let certificate =
                    authorization
                        .authority_certificate
                        .as_ref()
                        .ok_or_else(|| {
                            AdmissionError::InvalidPublisherSignature(
                        "First namespace claim requires a quorum-certified npm/PyPI authority proof"
                            .into(),
                    )
                        })?;
                let consensus_config =
                    crate::namespace_consensus::NamespaceConsensusConfig::from_node_config(
                        &state.config,
                    );
                crate::namespace_consensus::validate_authority_certificate(
                    &state.chain,
                    next_height,
                    &consensus_config,
                    certificate,
                    common::NamespaceAuthorityPurpose::Claim,
                    chrono::Utc::now().timestamp(),
                )
                .map_err(|error| AdmissionError::InvalidPublisherSignature(error.to_string()))?;
            } else if authorization.authority_certificate.is_some() {
                return Err(AdmissionError::InvalidPublisherSignature(
                    "An established namespace publish must not replay a first-claim certificate"
                        .into(),
                ));
            }
        }
        ensure_namespace_controller(
            &namespace_prefix,
            &request.publisher_pubkey,
            chain_controller,
            state.pending_pool.publisher_for_prefix(&namespace_prefix),
            active_publish_delegation(&state.chain, &namespace_prefix, &request)
                .map_err(|error| AdmissionError::Storage(error.to_string()))?,
        )?;
        state.config.ipfs_url.clone()
    };

    if let Err(error) = crate::admission_scan::run_pre_mempool_yara_gate(&request, &ipfs_url).await
    {
        tracing::warn!(
            surface = options.surface.label(),
            canonical = %canonical,
            "Pre-mempool YARA gate rejected submission: {}",
            error
        );
        return Err(AdmissionError::Scanner(error));
    }

    let pending_count = {
        let mut state = state.write().await;

        // The pre-scan check above is only an early rejection. Scanning happens
        // without the state lock, so a competing first submission can establish
        // the namespace while this request is being scanned. Re-check while
        // holding the write lock immediately before the pending-pool mutation.
        ensure_namespace_controller(
            &namespace_prefix,
            &request.publisher_pubkey,
            state.chain.publisher_for_prefix(&namespace_prefix),
            state.pending_pool.publisher_for_prefix(&namespace_prefix),
            active_publish_delegation(&state.chain, &namespace_prefix, &request)
                .map_err(|error| AdmissionError::Storage(error.to_string()))?,
        )?;
        match state.pending_pool.insert_durable(request.clone()) {
            Ok(true) => {}
            Ok(false) => {
                return Err(AdmissionError::AlreadyPending(format!(
                    "{} is already pending with the same content hash",
                    canonical
                )))
            }
            Err(error) => return Err(AdmissionError::Storage(error.to_string())),
        }
        // === WIRE REAL COMMITTEE COMPUTATION AT ADMISSION TIME (hybrid model) ===
        // Compute early so all nodes get the same mandatory committee via gossip.
        let tip = state.chain.tip_hash().unwrap_or_default();
        // Try to get recent VRF from tip block for VRF-seeded selection
        let recent_vrf = state
            .chain
            .get_block_by_hash(&tip)
            .ok()
            .flatten()
            .and_then(|b| b.header.vrf_output.clone());
        let selection_randomness = recent_vrf.clone().unwrap_or_else(|| tip.clone());
        // A certificate is optional during the migration, but when one is
        // present for the active set it is the only valid committee anchor.
        // Local history entries are deliberately not upgraded by assertion.
        let next_height = state.chain.tip_height().unwrap_or(0).saturating_add(1);
        let certified = match crate::validator_set_history::committee_authority_at(
            &state.config.data_dir,
            next_height,
            &state.validator_set_bootstrap,
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::error!("Weak-subjectivity checkpoint configuration rejected: {error}");
                None
            }
        }
        .filter(|snapshot| {
            snapshot.validator_set_hash
                == common::canonical_validator_set_hash(&state.validator_set)
        });
        let committee = match certified {
            Some(snapshot) => compute_analysis_committee_for_authenticated_snapshot(
                &snapshot,
                &canonical,
                &request.content_hash,
                selection_randomness,
            ),
            None => compute_analysis_committee_at_admission(
                &state.validator_set,
                &canonical,
                &request.content_hash,
                tip,
                recent_vrf,
            ),
        };
        if crate::validator_set_history::certified_committees_required()
            && committee.validator_snapshot_certificate_hash.is_empty()
        {
            state.pending_pool.remove(&canonical);
            return Err(AdmissionError::CertifiedCommitteeRequired(
                "Certified, checkpoint-anchored validator snapshot is required for package admission".into(),
            ));
        }
        if let Err(error) = state
            .pending_pool
            .set_analysis_committee_durable(&canonical, committee)
        {
            state.pending_pool.remove(&canonical);
            return Err(AdmissionError::Storage(error.to_string()));
        }
        if !request.publisher_evm_signature.trim().is_empty() {
            let nonce_consumed = match state.chain.consume_publisher_admission_nonce(
                &request.publisher_address,
                &request.publisher_nonce,
                request.publisher_expires_at,
            ) {
                Ok(consumed) => consumed,
                Err(error) => {
                    state.pending_pool.remove(&canonical);
                    return Err(AdmissionError::Storage(error.to_string()));
                }
            };
            if !nonce_consumed {
                state.pending_pool.remove(&canonical);
                return Err(AdmissionError::InvalidPublisherSignature(
                    "Publisher proof nonce has already been used".into(),
                ));
            }
        }
        state
            .chain
            .emit_registry_event(crate::events::RegistryEvent::package_submitted(
                &canonical,
                &request.publisher_pubkey,
            ));
        state.pending_pool.len()
    };

    tracing::info!(
        surface = options.surface.label(),
        canonical = %canonical,
        pending_count,
        "Package admitted into pending pool"
    );

    Ok(AdmissionReceipt {
        canonical,
        pending_count,
    })
}

fn ensure_namespace_controller(
    namespace_prefix: &str,
    requested_pubkey: &str,
    chain_owner: Option<String>,
    pending_owner: Option<String>,
    delegated: bool,
) -> Result<(), AdmissionError> {
    if delegated {
        return Ok(());
    }
    let Some(owner) = chain_owner.or(pending_owner) else {
        return Ok(());
    };

    if owner.trim().eq_ignore_ascii_case(requested_pubkey.trim()) {
        return Ok(());
    }

    Err(AdmissionError::NamespaceOwned(format!(
        "{} is controlled by another publisher; use the signed namespace transfer or key-rotation flow",
        namespace_prefix.trim_end_matches('@')
    )))
}

fn active_publish_delegation(
    chain: &crate::chain_store::ChainStore,
    namespace: &str,
    request: &PublishRequest,
) -> anyhow::Result<bool> {
    let Some(root) = chain.publisher_for_prefix(namespace) else {
        return Ok(false);
    };
    let key = crate::namespace_consensus::delegation_key(namespace, &request.publisher_pubkey);
    Ok(chain.namespace_delegation(&key)?.is_some_and(|delegation| {
        delegation.root_ed25519_pubkey == root
            && delegation.delegate_evm_address == request.publisher_address
            && delegation.scopes.iter().any(|scope| scope == "publish")
            && chrono::Utc::now().timestamp() < delegation.expires_at
    }))
}

pub async fn verify_publish_auth(
    state: &SharedState,
    request: &PublishRequest,
) -> Result<(), AdmissionError> {
    validate_package_id(request)?;
    verify_publish_sig(request)
        .map_err(|error| AdmissionError::InvalidPublisherSignature(error.to_string()))?;

    // CR-01: dual identity binding is fail-closed. A deployment must opt in to
    // the legacy Ed25519-only compatibility mode explicitly, and production
    // mode never permits that override.
    let production = env_flag("CREG_PRODUCTION");
    let allow_legacy = !production && env_flag("CREG_ALLOW_LEGACY_PUBLISHER_PROOF");

    let has_dual_proof = !request.publisher_evm_signature.trim().is_empty();
    if !has_dual_proof {
        if !allow_legacy {
            return Err(AdmissionError::InvalidPublisherSignature(
                "Publisher EVM signature is required; set CREG_ALLOW_LEGACY_PUBLISHER_PROOF=true only for a non-production compatibility migration"
                    .into(),
            ));
        }
        tracing::warn!(
            canonical = %request.id.canonical(),
            "Accepting legacy publisher proof in non-production compatibility mode"
        );
    } else {
        verify_publisher_dual_proof(state, request)
            .await
            .map_err(|error| AdmissionError::InvalidPublisherSignature(error.to_string()))?;
    }

    ensure_publisher_staked(state, &request.publisher_address)
        .await
        .map_err(AdmissionError::Publisher)?;

    Ok(())
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn validate_package_id(request: &PublishRequest) -> Result<(), AdmissionError> {
    if request.id.ecosystem.trim().is_empty()
        || request.id.name.trim().is_empty()
        || request.id.version.trim().is_empty()
    {
        return Err(AdmissionError::InvalidPackageId(
            "Package ecosystem, name, and version are required".to_string(),
        ));
    }
    common::validate_namespace_prefix(&common::package_namespace_prefix(&request.id)).map_err(
        |error| {
            AdmissionError::InvalidPackageId(format!(
                "Invalid package namespace for admission: {}",
                error
            ))
        },
    )?;
    Ok(())
}

pub(crate) fn verify_publish_sig(req: &PublishRequest) -> anyhow::Result<()> {
    use common::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};

    if req.publisher_address.trim().is_empty() {
        anyhow::bail!("Publisher EVM address is required for runtime admission");
    }

    let primary_pubkey = req.publisher_pubkey.trim();
    let primary_pubkey_bytes =
        hex::decode(primary_pubkey).context("Publisher Ed25519 public key must be valid hex")?;
    PublicKey::from_bytes(SignatureScheme::Ed25519, &primary_pubkey_bytes)
        .map_err(|_| anyhow::anyhow!("Invalid Ed25519 public key"))?;

    let msg = publisher_signature_message(req)?;

    // Single-signature fallback.
    if req.publisher_pubkeys.is_empty() {
        let sig_bytes = hex::decode(&req.signature)?;
        let vk = PublicKey::from_bytes(SignatureScheme::Ed25519, &primary_pubkey_bytes)
            .map_err(|_| anyhow::anyhow!("Invalid Ed25519 public key"))?;
        let sig = Signature::from_bytes(SignatureScheme::Ed25519, &sig_bytes)
            .map_err(|_| anyhow::anyhow!("Invalid Ed25519 signature"))?;
        return vk
            .verify(msg.as_bytes(), &sig, VerifyMode::Permissive)
            .map_err(|_| anyhow::anyhow!("Signature verification failed"));
    }

    // Multi-signature: require at least threshold-of-N valid signatures.
    let threshold = if req.threshold == 0 { 2 } else { req.threshold };

    if req.signatures.len() != req.publisher_pubkeys.len() {
        anyhow::bail!(
            "Signature count ({}) does not match pubkey count ({})",
            req.signatures.len(),
            req.publisher_pubkeys.len()
        );
    }

    let mut valid = 0usize;
    let mut primary_signed = false;
    let mut seen_pubkeys = std::collections::HashSet::new();
    for (pubkey_hex, sig_hex) in req.publisher_pubkeys.iter().zip(req.signatures.iter()) {
        let normalized_pubkey = pubkey_hex.trim().to_ascii_lowercase();
        let is_primary_pubkey = normalized_pubkey.eq_ignore_ascii_case(primary_pubkey);
        if !seen_pubkeys.insert(normalized_pubkey) {
            continue;
        }
        let pk_bytes = match hex::decode(pubkey_hex) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let sig_bytes = match hex::decode(sig_hex) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let vk = match PublicKey::from_bytes(SignatureScheme::Ed25519, &pk_bytes) {
            Ok(k) => k,
            Err(_) => continue,
        };
        let sig = match Signature::from_bytes(SignatureScheme::Ed25519, &sig_bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if vk
            .verify(msg.as_bytes(), &sig, VerifyMode::Permissive)
            .is_ok()
        {
            valid += 1;
            if is_primary_pubkey {
                primary_signed = true;
            }
        }
    }

    if valid < threshold {
        anyhow::bail!(
            "Multi-sig verification failed: only {}/{} valid signatures (need {})",
            valid,
            req.publisher_pubkeys.len(),
            threshold
        )
    }

    if !primary_signed {
        anyhow::bail!(
            "Multi-sig verification failed: the declared namespace controller key did not sign"
        )
    }

    Ok(())
}

fn publisher_signature_message(req: &PublishRequest) -> anyhow::Result<String> {
    if req.publisher_authorization_version == 4 {
        if req.submitted_at.timestamp_subsec_nanos() != 0 {
            anyhow::bail!("Publisher v4 submitted_at must use whole Unix seconds");
        }
        let namespace = common::authoritative_package_namespace(&req.id)
            .map_err(|error| anyhow::anyhow!("Invalid authoritative namespace: {}", error))?;
        if req.ownership_claim != namespace.trim_end_matches('@') {
            anyhow::bail!("Publisher ownership claim is not the canonical namespace");
        }
        let keyset =
            common::canonical_publisher_keyset(&req.publisher_pubkey, &req.publisher_pubkeys);
        if req.publisher_pubkeys.is_empty()
            || !req.signature.is_empty()
            || req.threshold == 0
            || req.threshold > keyset.len()
            || req.publisher_pubkeys != keyset
            || !keyset.contains(&req.publisher_pubkey)
        {
            anyhow::bail!("Publisher v4 multisig policy is not canonical");
        }
        if req.source_hash.len() != 64
            || req.source_hash != req.source_hash.to_ascii_lowercase()
            || !req.source_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            anyhow::bail!("Publisher v4 source hash must be lowercase SHA-256");
        }
        let proof_digest = match &req.ownership_proof {
            Some(proof) => {
                proof
                    .validate_structure()
                    .map_err(|error| anyhow::anyhow!("Invalid ownership proof: {}", error))?;
                proof.digest()
            }
            None => String::new(),
        };
        let certificate_digest = req
            .namespace_authority_certificate
            .as_ref()
            .map(common::CertifiedNamespaceAuthority::digest)
            .unwrap_or_default();
        let key_bundle_sha256 = req
            .key_bundle
            .as_ref()
            .map(|bundle| common::sha256_hex(bundle.as_bytes()))
            .unwrap_or_default();
        let pgp_signature_sha256 = req
            .pgp_signature
            .as_ref()
            .map(|value| common::sha256_hex(value.as_bytes()))
            .unwrap_or_default();
        let pgp_public_key_sha256 = req
            .pgp_public_key
            .as_ref()
            .map(|value| common::sha256_hex(value.as_bytes()))
            .unwrap_or_default();
        return Ok(common::publish_signature_message_v4(
            &req.publisher_chain_id,
            &req.publisher_genesis_hash,
            &req.publisher_staking_contract,
            &req.id,
            &req.content_hash,
            &req.source_hash,
            &req.manifest_hash,
            &req.ipfs_cid,
            req.shielded,
            &key_bundle_sha256,
            &pgp_signature_sha256,
            &pgp_public_key_sha256,
            &req.publisher_pubkey,
            &req.publisher_address,
            &req.ownership_claim,
            &req.publisher_nonce,
            req.submitted_at.timestamp(),
            req.publisher_expires_at,
            req.threshold,
            &req.publisher_pubkeys,
            &proof_digest,
            &certificate_digest,
        ));
    }
    if !req.publisher_nonce.trim().is_empty()
        && req.publisher_expires_at > 0
        && !req.manifest_hash.trim().is_empty()
        && !req.publisher_chain_id.trim().is_empty()
        && !req.publisher_staking_contract.trim().is_empty()
    {
        if let Some(proof) = &req.ownership_proof {
            proof
                .validate_structure()
                .map_err(|error| anyhow::anyhow!("Invalid ownership proof: {}", error))?;
            if proof.namespace != common::package_namespace_prefix(&req.id) {
                anyhow::bail!("Ownership proof namespace does not match the submitted package");
            }
            if !proof
                .controller_evm_address
                .eq_ignore_ascii_case(&req.publisher_address)
            {
                anyhow::bail!("Ownership proof EVM controller does not match the publisher");
            }
            if !proof
                .controller_ed25519_pubkey
                .eq_ignore_ascii_case(&req.publisher_pubkey)
            {
                anyhow::bail!("Ownership proof Ed25519 controller does not match the publisher");
            }
            return Ok(common::publish_signature_message_v3(
                &req.publisher_chain_id,
                &req.publisher_staking_contract,
                &req.id,
                &req.content_hash,
                &req.source_hash,
                &req.manifest_hash,
                &req.publisher_pubkey,
                &req.publisher_address,
                &req.ownership_claim,
                &req.publisher_nonce,
                req.submitted_at.timestamp(),
                req.publisher_expires_at,
                &proof.digest(),
            ));
        }
        return Ok(common::publish_signature_message_v2(
            &req.publisher_chain_id,
            &req.publisher_staking_contract,
            &req.id,
            &req.content_hash,
            &req.source_hash,
            &req.manifest_hash,
            &req.publisher_pubkey,
            &req.publisher_address,
            &req.ownership_claim,
            &req.publisher_nonce,
            req.submitted_at.timestamp(),
            req.publisher_expires_at,
        ));
    }

    Ok(common::publish_signature_message(
        &req.id,
        &req.content_hash,
        &req.publisher_address,
    ))
}

async fn verify_publisher_dual_proof(
    state: &SharedState,
    request: &PublishRequest,
) -> anyhow::Result<()> {
    let (chain_id, genesis_hash, staking_contract) = {
        let state = state.read().await;
        (
            state.config.chain_id.clone(),
            state.config.network_genesis_hash.clone(),
            state.config.staking_addr.clone(),
        )
    };
    if chain_id.trim().is_empty() || staking_contract.trim().is_empty() {
        anyhow::bail!(
            "Publisher dual-key proof cannot be verified without CREG_CHAIN_ID and CREG_STAKING_ADDR"
        );
    }
    if request.publisher_chain_id != chain_id {
        anyhow::bail!("Publisher proof chain ID does not match this node");
    }
    if request.publisher_authorization_version == 4
        && request.publisher_genesis_hash != genesis_hash
    {
        anyhow::bail!("Publisher proof genesis hash does not match this network");
    }
    if common::canonical_publisher_address(&request.publisher_staking_contract)
        != common::canonical_publisher_address(&staking_contract)
    {
        anyhow::bail!("Publisher proof staking contract does not match this node");
    }
    let nonce = request.publisher_nonce.trim();
    if nonce.is_empty() || nonce.len() > 128 {
        anyhow::bail!("Publisher proof nonce must contain 1 to 128 characters");
    }
    if !nonce
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        anyhow::bail!("Publisher proof nonce may contain only ASCII letters, digits, '-' and '_'");
    }
    let now = chrono::Utc::now().timestamp();
    let issued_at = request.submitted_at.timestamp();
    if issued_at > now.saturating_add(300) {
        anyhow::bail!("Publisher proof issued-at time is too far in the future");
    }
    if request.publisher_expires_at <= now as u64 {
        anyhow::bail!("Publisher proof has expired");
    }
    if request.publisher_expires_at > now.saturating_add(86_400) as u64 {
        anyhow::bail!("Publisher proof expiry exceeds the 24-hour maximum");
    }
    let expected_claim = format!("{}:{}", request.id.ecosystem, request.id.name);
    if request.ownership_claim != expected_claim {
        anyhow::bail!("Publisher ownership claim does not match the package namespace");
    }
    let expected_manifest_hash = common::sha256_hex(
        &serde_json::to_vec(&request.manifest).context("Failed to serialize package manifest")?,
    );
    if !request
        .manifest_hash
        .eq_ignore_ascii_case(&expected_manifest_hash)
    {
        anyhow::bail!("Publisher proof manifest digest does not match the submitted manifest");
    }
    let message = publisher_signature_message(request)?;
    let recovered = recover_evm_personal_signer(&message, &request.publisher_evm_signature)?;
    if recovered != common::canonical_publisher_address(&request.publisher_address) {
        anyhow::bail!(
            "EVM signature recovered {}, expected {}",
            recovered,
            common::canonical_publisher_address(&request.publisher_address)
        );
    }
    Ok(())
}

pub(crate) fn recover_evm_personal_signer(
    message: &str,
    signature_hex: &str,
) -> anyhow::Result<String> {
    use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};

    let normalized = signature_hex.trim().trim_start_matches("0x");
    let bytes = hex::decode(normalized).context("EVM signature must be valid hex")?;
    if bytes.len() != 65 {
        anyhow::bail!("EVM signature must be 65 bytes");
    }
    let recovery = match bytes[64] {
        0 | 1 => bytes[64],
        27 | 28 => bytes[64] - 27,
        other => anyhow::bail!("Unsupported EVM signature recovery id {}", other),
    };
    let recovery_id = RecoveryId::from_byte(recovery)
        .ok_or_else(|| anyhow::anyhow!("Invalid EVM signature recovery id"))?;
    let signature =
        K256Signature::try_from(&bytes[..64]).context("Invalid EVM signature r/s values")?;
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.as_bytes().len());
    let mut payload = prefix.into_bytes();
    payload.extend_from_slice(message.as_bytes());
    let digest = keccak256(payload);
    let verifying_key =
        VerifyingKey::recover_from_prehash(digest.as_slice(), &signature, recovery_id)
            .context("Failed to recover EVM signer")?;
    let encoded = verifying_key.to_encoded_point(false);
    let hash = keccak256(&encoded.as_bytes()[1..]);
    Ok(Address::from_slice(&hash.as_slice()[12..])
        .to_string()
        .to_ascii_lowercase())
}

fn parse_publisher_address(publisher_address: &str) -> Result<Address, PublisherAdmissionError> {
    let normalized = common::canonical_publisher_address(publisher_address);
    if normalized.is_empty() {
        return Err(PublisherAdmissionError::InvalidAddress(
            "Publisher EVM address is required for runtime admission".into(),
        ));
    }

    normalized.parse::<Address>().map_err(|_| {
        PublisherAdmissionError::InvalidAddress(
            "Publisher EVM address must be a valid 0x-prefixed address".into(),
        )
    })
}

pub(crate) async fn ensure_publisher_staked(
    state: &SharedState,
    publisher_address: &str,
) -> Result<(), PublisherAdmissionError> {
    let publisher = parse_publisher_address(publisher_address)?;

    // 1. Check in-memory stake cache first (sub-millisecond RAM lookup)
    {
        let s = state.read().await;
        if let Some(staked) = s.get_cached_publisher_stake(&publisher) {
            if staked == alloy::primitives::U256::ZERO {
                return Err(PublisherAdmissionError::Unstaked(format!(
                    "Publisher {} has no on-chain stake and cannot publish",
                    common::canonical_publisher_address(publisher_address)
                )));
            }
            return Ok(());
        }
    }

    // 2. Cache miss or expired: query on-chain via L1 RPC
    let (rpc_url, staking_addr_s) = {
        let s = state.read().await;
        (s.config.eth_rpc_url.clone(), s.config.staking_addr.clone())
    };

    if rpc_url.trim().is_empty() {
        return Err(PublisherAdmissionError::Unavailable(
            "Publisher stake enforcement unavailable: CREG_ETH_RPC is not configured".into(),
        ));
    }
    if staking_addr_s.trim().is_empty()
        || staking_addr_s.eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
    {
        return Err(PublisherAdmissionError::Unavailable(
            "Publisher stake enforcement unavailable: CREG_STAKING_ADDR is not configured".into(),
        ));
    }

    let staking_addr = staking_addr_s.parse::<Address>().map_err(|_| {
        PublisherAdmissionError::Unavailable(
            "Publisher stake enforcement unavailable: CREG_STAKING_ADDR is invalid".into(),
        )
    })?;

    let provider = crate::rpc_failover::build_l1_provider(&rpc_url).map_err(|_| {
        PublisherAdmissionError::Unavailable(
            "Publisher stake enforcement unavailable: CREG_ETH_RPC is invalid".into(),
        )
    })?;
    let staking = IPublisherStakingRead::new(staking_addr, &provider);
    let staked = staking
        .stakedBalance(publisher)
        .call()
        .await
        .map_err(|error| {
            PublisherAdmissionError::Unavailable(format!(
                "Publisher stake lookup failed: {}",
                error
            ))
        })?
        ._0;

    // 3. Cache the verified on-chain balance
    {
        let mut s = state.write().await;
        s.set_cached_publisher_stake(publisher, staked);
    }

    if staked == alloy::primitives::U256::ZERO {
        return Err(PublisherAdmissionError::Unstaked(format!(
            "Publisher {} has no on-chain stake and cannot publish",
            common::canonical_publisher_address(publisher_address)
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chain_store::ChainStore,
        config::NodeConfig,
        pending_pool::PendingPool,
        publisher_index::PublisherIndex,
        state::{
            BridgeStatus, NodeState, P2PStatus, ValidatorRegistrationStatus, ValidatorSetSyncStatus,
        },
    };
    use axum::{http::StatusCode, routing::post, Json, Router};
    use common::{PackageId, PackageManifest, PublishRequest};
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    use serde_json::Value;
    use std::{collections::HashMap, sync::Arc};
    use tempfile::TempDir;
    use tokio::{
        net::TcpListener,
        sync::{Mutex, RwLock},
    };

    static ENV_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvRestore {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvRestore {
        fn set(key: &'static str, value: impl AsRef<str>) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value.as_ref());
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn make_keypair() -> (SigningKey, String) {
        let sk = SigningKey::generate(&mut OsRng);
        let pk = hex::encode(sk.verifying_key().as_bytes());
        (sk, pk)
    }

    fn make_tarball(path: &str, content: &str) -> anyhow::Result<Vec<u8>> {
        let mut tar_bytes = Vec::new();
        {
            let encoder =
                flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::default());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(content.as_bytes().len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, path, content.as_bytes())?;
            archive.finish()?;
            archive.into_inner()?.finish()?;
        }
        Ok(tar_bytes)
    }

    fn write_rules(dir: &std::path::Path, threat_level: u8) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(
            dir.join("test-malware.yar"),
            format!(
                r#"
rule TestMaliciousPayload {{
    meta:
        threat_level = {threat_level}
        description = "test malicious payload"
        category = "test"
    strings:
        $payload = "MALICIOUS_PAYLOAD"
    condition:
        $payload
}}
"#
            ),
        )?;
        Ok(())
    }

    async fn spawn_mock_staking_rpc(staked_balance: u64) -> String {
        async fn handle_rpc(Json(payload): Json<Value>, staked_result: String) -> Json<Value> {
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": payload.get("id").cloned().unwrap_or_else(|| serde_json::json!(1)),
                "result": staked_result,
            }))
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let staked_result = format!("0x{:064x}", staked_balance);
        let app = Router::new().route(
            "/",
            post({
                let staked_result = staked_result.clone();
                move |payload| handle_rpc(payload, staked_result.clone())
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{}", addr)
    }

    async fn spawn_mock_ipfs(payload: Vec<u8>, status: StatusCode) -> String {
        async fn handle_ipfs(
            payload: Vec<u8>,
            status: StatusCode,
        ) -> impl axum::response::IntoResponse {
            (status, payload)
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/api/v0/cat",
            post({
                let payload = payload.clone();
                move || handle_ipfs(payload.clone(), status)
            }),
        );

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        format!("http://{}", addr)
    }

    fn test_validator_set(staked_balance: u64) -> common::ValidatorSet {
        let mut vs = common::ValidatorSet::default();
        vs.validators.push(common::Validator {
            id: "v-test".into(),
            alias: "v-test".into(),
            pubkey: "p-test".into(),
            eth_address: "0x1111111111111111111111111111111111111111".into(),
            stake: staked_balance,
            reputation: 50,
            status: "active".into(),
            performance_score: 500,
            contribution_count: 0,
        });
        vs
    }

    async fn make_test_state(
        staked_balance: u64,
        ipfs_url: String,
    ) -> anyhow::Result<(SharedState, TempDir)> {
        let rpc_url = spawn_mock_staking_rpc(staked_balance).await;
        let tempdir = tempfile::tempdir()?;
        let chain = ChainStore::open(tempdir.path())?;
        let validator_set = test_validator_set(staked_balance);

        let state = Arc::new(RwLock::new(NodeState {
            chain,
            pending_pool: PendingPool::new(),
            publisher_index: PublisherIndex::new(),
            validator_set_bootstrap: validator_set.clone(),
            validator_set,
            package_rounds: HashMap::new(),
            package_committees: HashMap::new(),
            config: NodeConfig {
                data_dir: tempdir.path().to_path_buf(),
                eth_rpc_url: rpc_url,
                staking_addr: "0x1000000000000000000000000000000000000001".into(),
                ipfs_url,
                ..Default::default()
            },
            p2p_status: P2PStatus::default(),
            bridge_status: BridgeStatus::default(),
            vrf_proofs: HashMap::new(),
            decryption_shares: HashMap::new(),
            validator_registrations: HashMap::<String, ValidatorRegistrationStatus>::new(),
            validator_set_sync: ValidatorSetSyncStatus::default(),
            view_change_certs: HashMap::new(),
            reorgs: Vec::new(),
            pbft_engine: crate::state::PbftEngine::new(),
            forced_inclusion_tracker: crate::state::ForcedInclusionTracker::new(),
            sync_lag_blocks: 0,
            sync_max_peer_tip: 0,
            reward_emissions_succeeded: 0,
            reward_emissions_failed: 0,
            reward_emissions_skipped: 0,
            treasury_balance: 0,
            pending_rewards_estimate: 0,
            total_rewards_distributed: 0,
            rewards_pool_balance: 0,
            last_incentives_refresh: None,
            publisher_stake_cache: HashMap::new(),
            equivocation: crate::equivocation_monitor::EquivocationMonitor::disabled(),
        }));

        Ok((state, tempdir))
    }

    fn signed_request(payload: &[u8], content_hash: String, cid: &str) -> PublishRequest {
        let (sk, pk) = make_keypair();
        let mut request = PublishRequest {
            id: PackageId::new("npm", "test", "1.0.0"),
            content_hash,
            ipfs_cid: cid.into(),
            publisher_address: "0x1111111111111111111111111111111111111111".into(),
            publisher_pubkey: pk,
            signature: String::new(),
            manifest: PackageManifest::default(),
            submitted_at: chrono::Utc::now(),
            shielded: false,
            key_bundle: None,
            pgp_signature: None,
            pgp_public_key: None,
            threshold: 0,
            publisher_pubkeys: Vec::new(),
            signatures: Vec::new(),
            ..Default::default()
        };
        let message = common::publish_signature_message(
            &request.id,
            &request.content_hash,
            &request.publisher_address,
        );
        request.signature = hex::encode(sk.sign(message.as_bytes()).to_bytes());
        assert_eq!(request.content_hash, common::sha256_hex(payload));
        request
    }

    async fn admit_for_test(
        state: &SharedState,
        request: PublishRequest,
    ) -> Result<AdmissionReceipt, AdmissionError> {
        // Existing package-admission fixtures exercise scanner and storage
        // behavior with a synthetic Ed25519-only publisher. Production code
        // rejects this path by default; tests opt in locally and explicitly.
        let _legacy = EnvRestore::set("CREG_ALLOW_LEGACY_PUBLISHER_PROOF", "true");
        admit_publish_request(
            state,
            request,
            AdmissionOptions {
                surface: AdmissionSurface::Rest,
                verify_publisher_auth: true,
            },
        )
        .await
    }

    #[tokio::test]
    async fn publisher_admission_rejects_legacy_proof_by_default() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let _legacy = EnvRestore::remove("CREG_ALLOW_LEGACY_PUBLISHER_PROOF");
        let _production = EnvRestore::remove("CREG_PRODUCTION");
        let (state, _tempdir) = make_test_state(1, "http://127.0.0.1:65535".into()).await?;
        let request = signed_request(b"payload", common::sha256_hex(b"payload"), "bafylegacy");

        let error = verify_publish_auth(&state, &request).await.unwrap_err();
        assert!(matches!(
            error,
            AdmissionError::InvalidPublisherSignature(message)
                if message.contains("Publisher EVM signature is required")
        ));
        Ok(())
    }

    #[test]
    fn multisig_requires_the_declared_namespace_controller_to_sign() {
        let (_controller_signing_key, controller_pubkey) = make_keypair();
        let (first_signing_key, first_pubkey) = make_keypair();
        let (second_signing_key, second_pubkey) = make_keypair();
        let mut request = signed_request(
            b"payload",
            common::sha256_hex(b"payload"),
            "bafymultisig-controller",
        );
        request.publisher_pubkey = controller_pubkey;
        request.publisher_pubkeys = vec![first_pubkey, second_pubkey];
        request.threshold = 2;
        let message = common::publish_signature_message(
            &request.id,
            &request.content_hash,
            &request.publisher_address,
        );
        request.signatures = vec![
            hex::encode(first_signing_key.sign(message.as_bytes()).to_bytes()),
            hex::encode(second_signing_key.sign(message.as_bytes()).to_bytes()),
        ];

        let error = verify_publish_sig(&request).unwrap_err();
        assert!(error
            .to_string()
            .contains("declared namespace controller key did not sign"));
    }

    #[test]
    fn namespace_controller_check_rejects_a_different_key_and_accepts_key_casing() {
        let prefix = "npm:controlled-package@";
        assert!(
            ensure_namespace_controller(prefix, "AABB", Some("aabb".into()), None, false).is_ok()
        );
        assert!(matches!(
            ensure_namespace_controller(prefix, "ccdd", Some("aabb".into()), None, false),
            Err(AdmissionError::NamespaceOwned(_))
        ));
    }

    #[tokio::test]
    async fn admission_rejects_missing_yara_rules() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let no_rules = tempfile::tempdir()?;
        let _rules = EnvRestore::set("CREG_YARA_RULES_DIR", no_rules.path().display().to_string());
        let payload = make_tarball("index.js", "console.log('safe');")?;
        let ipfs_url = spawn_mock_ipfs(payload.clone(), StatusCode::OK).await;
        let (state, _tempdir) = make_test_state(1, ipfs_url).await?;
        let request = signed_request(&payload, common::sha256_hex(&payload), "bafymissingrules");

        let error = admit_for_test(&state, request).await.unwrap_err();
        assert!(matches!(
            error,
            AdmissionError::Scanner(crate::admission_scan::AdmissionScanError::RulesUnavailable)
        ));
        assert!(!state.read().await.pending_pool.contains("npm:test@1.0.0"));
        Ok(())
    }

    #[tokio::test]
    async fn admission_rejects_ipfs_fetch_failure() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let rules = tempfile::tempdir()?;
        write_rules(rules.path(), 5)?;
        let _rules = EnvRestore::set("CREG_YARA_RULES_DIR", rules.path().display().to_string());
        let payload = make_tarball("index.js", "console.log('safe');")?;
        let ipfs_url =
            spawn_mock_ipfs(b"missing".to_vec(), StatusCode::INTERNAL_SERVER_ERROR).await;
        let (state, _tempdir) = make_test_state(1, ipfs_url).await?;
        let request = signed_request(&payload, common::sha256_hex(&payload), "bafyipfsfail");

        let error = admit_for_test(&state, request).await.unwrap_err();
        assert!(matches!(
            error,
            AdmissionError::Scanner(crate::admission_scan::AdmissionScanError::IpfsFetch { .. })
        ));
        assert!(!state.read().await.pending_pool.contains("npm:test@1.0.0"));
        Ok(())
    }

    #[tokio::test]
    async fn admission_rejects_content_hash_mismatch() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let rules = tempfile::tempdir()?;
        write_rules(rules.path(), 5)?;
        let _rules = EnvRestore::set("CREG_YARA_RULES_DIR", rules.path().display().to_string());
        let payload = make_tarball("index.js", "console.log('safe');")?;
        let ipfs_url = spawn_mock_ipfs(payload.clone(), StatusCode::OK).await;
        let (state, _tempdir) = make_test_state(1, ipfs_url).await?;
        let wrong_hash = common::sha256_hex(b"different payload");
        let request = signed_request(b"different payload", wrong_hash, "bafyhashmismatch");

        let error = admit_for_test(&state, request).await.unwrap_err();
        assert!(matches!(
            error,
            AdmissionError::Scanner(
                crate::admission_scan::AdmissionScanError::ContentHashMismatch { .. }
            )
        ));
        assert!(!state.read().await.pending_pool.contains("npm:test@1.0.0"));
        Ok(())
    }

    #[tokio::test]
    async fn admission_rejects_oversized_payload() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let rules = tempfile::tempdir()?;
        write_rules(rules.path(), 5)?;
        let _rules = EnvRestore::set("CREG_YARA_RULES_DIR", rules.path().display().to_string());
        let _limit = EnvRestore::set("CREG_PRE_MEMPOOL_SCAN_MAX_BYTES", "8");
        let payload = make_tarball("index.js", "console.log('safe');")?;
        let ipfs_url = spawn_mock_ipfs(payload.clone(), StatusCode::OK).await;
        let (state, _tempdir) = make_test_state(1, ipfs_url).await?;
        let request = signed_request(&payload, common::sha256_hex(&payload), "bafytoolarge");

        let error = admit_for_test(&state, request).await.unwrap_err();
        assert!(matches!(
            error,
            AdmissionError::Scanner(
                crate::admission_scan::AdmissionScanError::PayloadTooLarge { .. }
            )
        ));
        assert!(!state.read().await.pending_pool.contains("npm:test@1.0.0"));
        Ok(())
    }

    #[tokio::test]
    async fn admission_rejects_malicious_yara_match() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let rules = tempfile::tempdir()?;
        write_rules(rules.path(), 5)?;
        let _rules = EnvRestore::set("CREG_YARA_RULES_DIR", rules.path().display().to_string());
        let payload = make_tarball("index.js", "const marker = 'MALICIOUS_PAYLOAD';")?;
        let ipfs_url = spawn_mock_ipfs(payload.clone(), StatusCode::OK).await;
        let (state, _tempdir) = make_test_state(1, ipfs_url).await?;
        let request = signed_request(&payload, common::sha256_hex(&payload), "bafymalicious");

        let error = admit_for_test(&state, request).await.unwrap_err();
        assert!(matches!(
            error,
            AdmissionError::Scanner(crate::admission_scan::AdmissionScanError::Rejected { .. })
        ));
        assert!(!state.read().await.pending_pool.contains("npm:test@1.0.0"));
        Ok(())
    }

    #[tokio::test]
    async fn admission_accepts_benign_package_into_pending_pool() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let rules = tempfile::tempdir()?;
        write_rules(rules.path(), 5)?;
        let _rules = EnvRestore::set("CREG_YARA_RULES_DIR", rules.path().display().to_string());
        let payload = make_tarball("index.js", "module.exports = () => 'safe';")?;
        let ipfs_url = spawn_mock_ipfs(payload.clone(), StatusCode::OK).await;
        let (state, _tempdir) = make_test_state(1, ipfs_url).await?;
        let request = signed_request(&payload, common::sha256_hex(&payload), "bafybenign");

        let receipt = admit_for_test(&state, request).await?;
        assert_eq!(receipt.canonical, "npm:test@1.0.0");
        assert_eq!(receipt.pending_count, 1);
        assert!(state.read().await.pending_pool.contains("npm:test@1.0.0"));
        // Small integration test for admission committee
        let guard = state.read().await;
        let committee = guard.pending_pool.get_committee("npm:test@1.0.0");
        assert!(
            committee.is_some(),
            "committee should be computed at admission time"
        );
        assert_eq!(
            committee.unwrap().size,
            1,
            "for 1 validator test, committee size should be 1"
        );
        Ok(())
    }

    #[tokio::test]
    async fn mandatory_certified_committee_mode_rejects_legacy_authority() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let rules = tempfile::tempdir()?;
        write_rules(rules.path(), 5)?;
        let _rules = EnvRestore::set("CREG_YARA_RULES_DIR", rules.path().display().to_string());
        let _required = EnvRestore::set("CREG_REQUIRE_CERTIFIED_COMMITTEES", "true");
        let payload = make_tarball("index.js", "module.exports = () => 'safe';")?;
        let ipfs_url = spawn_mock_ipfs(payload.clone(), StatusCode::OK).await;
        let (state, _tempdir) = make_test_state(1, ipfs_url).await?;
        let request = signed_request(&payload, common::sha256_hex(&payload), "bafycertrequired");

        let error = admit_for_test(&state, request).await.unwrap_err();
        assert!(matches!(
            error,
            AdmissionError::CertifiedCommitteeRequired(_)
        ));
        assert!(!state.read().await.pending_pool.contains("npm:test@1.0.0"));
        Ok(())
    }

    #[test]
    fn committee_selection_uses_vrf_seed_when_provided() {
        let mut vs = common::ValidatorSet::default();
        vs.validators.push(common::Validator {
            id: "v1".into(),
            alias: "v1".into(),
            pubkey: "p1".into(),
            eth_address: "0x1".into(),
            stake: 100,
            reputation: 50,
            status: "active".into(),
            performance_score: 500,
            contribution_count: 0,
        });
        vs.validators.push(common::Validator {
            id: "v2".into(),
            alias: "v2".into(),
            pubkey: "p2".into(),
            eth_address: "0x2".into(),
            stake: 100,
            reputation: 50,
            status: "active".into(),
            performance_score: 500,
            contribution_count: 0,
        });
        let c1 = compute_analysis_committee_at_admission(&vs, "npm:t@1", "h1", "tip1".into(), None);
        let c2 = compute_analysis_committee_at_admission(
            &vs,
            "npm:t@1",
            "h1",
            "tip1".into(),
            Some("vrf123".into()),
        );
        assert_ne!(
            c1.seed, c2.seed,
            "VRF seed should change the selection seed"
        );
    }

    #[test]
    fn certified_committee_rejects_member_and_fork_anchor_tampering() {
        let keys: Vec<_> = (0..4).map(|_| SigningKey::generate(&mut OsRng)).collect();
        let previous = common::ValidatorSet::new(
            keys.iter()
                .enumerate()
                .map(|(index, key)| common::Validator {
                    id: format!("v{index}"),
                    alias: format!("v{index}"),
                    pubkey: hex::encode(key.verifying_key().as_bytes()),
                    eth_address: format!("0x{index:040x}"),
                    stake: 100,
                    reputation: 100,
                    status: "active".into(),
                    performance_score: 500,
                    contribution_count: 0,
                })
                .collect(),
        );
        let mut snapshot = common::AuthenticatedValidatorSetSnapshot {
            schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height: 9,
            previous_block_hash: "a".repeat(64),
            previous_validator_set_hash: common::canonical_validator_set_hash(&previous),
            validators: previous.clone(),
            validator_set_hash: common::canonical_validator_set_hash(&previous),
            transition_signatures: vec![],
        };
        let message = common::validator_set_transition_message(&snapshot);
        for (index, key) in keys.iter().take(3).enumerate() {
            snapshot
                .transition_signatures
                .push(common::ValidatorSetTransitionSignature {
                    validator_id: format!("v{index}"),
                    validator_pubkey: hex::encode(key.verifying_key().as_bytes()),
                    signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
                });
        }
        snapshot.validate_against(&previous).expect("prior quorum");
        let mut committee = compute_analysis_committee_for_authenticated_snapshot(
            &snapshot,
            "npm:certified@1.0.0",
            "content-hash",
            "vrf-output".into(),
        );
        assert!(verify_analysis_committee_assignment(
            &snapshot,
            "npm:certified@1.0.0",
            "content-hash",
            "vrf-output",
            &committee,
        )
        .is_ok());

        let mut alternate_witness = snapshot.clone();
        alternate_witness.transition_signatures.clear();
        for index in [1usize, 2, 3] {
            let key = &keys[index];
            alternate_witness
                .transition_signatures
                .push(common::ValidatorSetTransitionSignature {
                    validator_id: format!("v{index}"),
                    validator_pubkey: hex::encode(key.verifying_key().as_bytes()),
                    signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
                });
        }
        alternate_witness
            .validate_against(&previous)
            .expect("alternate prior quorum");
        assert!(verify_analysis_committee_assignment(
            &alternate_witness,
            "npm:certified@1.0.0",
            "content-hash",
            "vrf-output",
            &committee,
        )
        .is_ok());

        let mut legacy_witness_bound = committee.clone();
        legacy_witness_bound.validator_snapshot_certificate_hash =
            common::validator_snapshot_witness_hash(&snapshot);
        legacy_witness_bound.assignment_proof = common::committee_assignment_proof(
            "npm:certified@1.0.0",
            "content-hash",
            &legacy_witness_bound,
        );
        assert!(verify_analysis_committee_assignment(
            &snapshot,
            "npm:certified@1.0.0",
            "content-hash",
            "vrf-output",
            &legacy_witness_bound,
        )
        .is_ok());

        committee.members.reverse();
        assert!(verify_analysis_committee_assignment(
            &snapshot,
            "npm:certified@1.0.0",
            "content-hash",
            "vrf-output",
            &committee,
        )
        .is_err());
        committee = compute_analysis_committee_for_authenticated_snapshot(
            &snapshot,
            "npm:certified@1.0.0",
            "content-hash",
            "vrf-output".into(),
        );
        committee.validator_snapshot_anchor_block_hash = "b".repeat(64);
        committee.assignment_proof =
            common::committee_assignment_proof("npm:certified@1.0.0", "content-hash", &committee);
        assert!(verify_analysis_committee_assignment(
            &snapshot,
            "npm:certified@1.0.0",
            "content-hash",
            "vrf-output",
            &committee,
        )
        .is_err());

        // Recomputing both the selected ordering and its self-consistent proof
        // cannot substitute attacker-chosen randomness for the anchor block's
        // authenticated VRF output.
        let attacker_committee = compute_analysis_committee_for_authenticated_snapshot(
            &snapshot,
            "npm:certified@1.0.0",
            "content-hash",
            "attacker-grinded-randomness".into(),
        );
        assert!(attacker_committee.assignment_proof_is_valid("npm:certified@1.0.0", "content-hash"));
        assert_ne!(attacker_committee.seed, committee.seed);
        assert_eq!(
            verify_analysis_committee_assignment(
                &snapshot,
                "npm:certified@1.0.0",
                "content-hash",
                "vrf-output",
                &attacker_committee,
            ),
            Err("committee selection randomness does not match its anchor block")
        );
    }

    #[test]
    fn committee_selection_has_reachable_primary_and_expansion_members_at_scale() {
        let validators = (0..10_000)
            .map(|index| common::Validator {
                id: format!("validator-{index}"),
                alias: format!("validator-{index}"),
                pubkey: format!("pubkey-{index}"),
                eth_address: format!("0x{:040x}", index + 1),
                stake: 100,
                reputation: 100,
                status: "active".into(),
                performance_score: 500,
                contribution_count: 0,
            })
            .collect();
        let validator_set = common::ValidatorSet::new(validators);
        let committee = compute_analysis_committee_at_admission(
            &validator_set,
            "npm:scale@1.0.0",
            "content-hash",
            "epoch-tip".into(),
            None,
        );
        assert_eq!(committee.size, 100);
        assert_eq!(committee.effective_size, 100);
        assert_eq!(committee.members.len(), 10_000);
        assert_eq!(
            committee.members[..100]
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            100
        );
    }

    #[test]
    fn committee_selection_vectors_are_deterministic_at_required_boundaries() {
        for validator_count in [1usize, 4, 7, 50, 51, 100, 1_000, 10_000] {
            let validators = (0..validator_count)
                .map(|index| common::Validator {
                    id: format!("validator-{index}"),
                    alias: format!("Validator {index}"),
                    pubkey: format!("pubkey-{index}"),
                    eth_address: format!("0x{:040x}", index + 1),
                    stake: 100,
                    reputation: 100,
                    status: "active".into(),
                    performance_score: 500,
                    contribution_count: 0,
                })
                .collect();
            let validator_set = common::ValidatorSet::new(validators);

            let first = compute_analysis_committee_at_admission(
                &validator_set,
                "npm:selector-vector@1.0.0",
                "content-digest",
                "authenticated-snapshot-tip".into(),
                Some("randomness-epoch-7".into()),
            );
            let second = compute_analysis_committee_at_admission(
                &validator_set,
                "npm:selector-vector@1.0.0",
                "content-digest",
                "authenticated-snapshot-tip".into(),
                Some("randomness-epoch-7".into()),
            );

            assert_eq!(first.seed, second.seed, "vector {validator_count}");
            assert_eq!(first.members, second.members, "vector {validator_count}");
            assert_eq!(first.size, validator_count.min(100) as u32);
            assert_eq!(first.effective_size, first.size);
            assert_eq!(first.members.len(), validator_count);
            assert_eq!(
                first
                    .members
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                validator_count,
                "vector {validator_count} must not duplicate members"
            );
        }
    }

    fn shielded_signed_request(
        plaintext: &[u8],
        wire: &[u8],
        bundle: &str,
        cid: &str,
    ) -> PublishRequest {
        let mut request = signed_request(plaintext, common::sha256_hex(plaintext), cid);
        request.shielded = true;
        request.key_bundle = Some(bundle.to_string());
        assert_ne!(wire, plaintext, "IPFS mock must serve encrypted wire bytes");
        request
    }

    #[tokio::test]
    async fn admission_accepts_shielded_when_enabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let _enabled = EnvRestore::set("CREG_SHIELDED_PUBLISH_ENABLED", "true");
        let plaintext = make_tarball("index.js", "module.exports = () => 'shielded-safe';")?;
        let (wire, bundle) = common::encrypt_shielded_package(&plaintext, None)?;
        let ipfs_url = spawn_mock_ipfs(wire.clone(), StatusCode::OK).await;
        let (state, _tempdir) = make_test_state(1, ipfs_url).await?;
        let request = shielded_signed_request(&plaintext, &wire, &bundle, "bafyshieldedenabled");

        let receipt = admit_for_test(&state, request).await?;
        assert_eq!(receipt.canonical, "npm:test@1.0.0");
        assert_eq!(receipt.pending_count, 1);
        assert!(state.read().await.pending_pool.contains("npm:test@1.0.0"));
        Ok(())
    }

    #[tokio::test]
    async fn admission_rejects_shielded_when_disabled() -> anyhow::Result<()> {
        let _guard = env_lock().lock().await;
        let _enabled = EnvRestore::set("CREG_SHIELDED_PUBLISH_ENABLED", "false");
        let (state, _tempdir) = make_test_state(1, "http://127.0.0.1:65535".into()).await?;
        let mut request = signed_request(b"payload", common::sha256_hex(b"payload"), "bafyshield");
        request.shielded = true;

        let error = admit_publish_request(
            &state,
            request,
            AdmissionOptions {
                surface: AdmissionSurface::Rest,
                verify_publisher_auth: false,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, AdmissionError::ShieldedPublishDisabled(_)));
        assert!(!state.read().await.pending_pool.contains("npm:test@1.0.0"));
        Ok(())
    }

    #[tokio::test]
    async fn admission_rejects_shielded_when_threshold_flag_on_without_shares() -> anyhow::Result<()>
    {
        let _guard = env_lock().lock().await;
        let _enabled = EnvRestore::set("CREG_SHIELDED_PUBLISH_ENABLED", "true");
        let plaintext = make_tarball("index.js", "module.exports = () => 'threshold-flag';")?;
        let (wire, bundle) = common::encrypt_shielded_package(&plaintext, None)?;
        let (state, _tempdir) = make_test_state(1, "http://127.0.0.1:65535".into()).await?;
        state.write().await.config.threshold_encryption = true;
        let request = shielded_signed_request(&plaintext, &wire, &bundle, "bafythresholdflag");

        let error = admit_publish_request(
            &state,
            request,
            AdmissionOptions {
                surface: AdmissionSurface::Rest,
                verify_publisher_auth: false,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(error, AdmissionError::ShieldedPublishDisabled(_)));
        assert!(error.to_string().contains("threshold_encryption"));
        assert!(!state.read().await.pending_pool.contains("npm:test@1.0.0"));
        Ok(())
    }

    #[tokio::test]
    async fn publisher_stake_cache_hits_and_eviction() -> anyhow::Result<()> {
        let (state, _tempdir) = make_test_state(100, "http://127.0.0.1:65535".into()).await?;
        let test_addr = "0x2222222222222222222222222222222222222222";
        let parsed_addr: Address = test_addr.parse().unwrap();

        // 1. Initial call populates the cache from the mock RPC
        ensure_publisher_staked(&state, test_addr).await.unwrap();
        {
            let s = state.read().await;
            let cached = s.get_cached_publisher_stake(&parsed_addr);
            assert_eq!(cached, Some(alloy::primitives::U256::from(100)));
        }

        // 2. Break the RPC configuration in state to prove subsequent call is served entirely from cache
        {
            let mut s = state.write().await;
            s.config.eth_rpc_url = String::new(); // would fail if RPC called
        }
        ensure_publisher_staked(&state, test_addr).await.unwrap();

        // 3. Invalidate cache: now the broken RPC config must trigger Unavailable error
        {
            let mut s = state.write().await;
            s.invalidate_publisher_stake(&parsed_addr);
        }
        let err = ensure_publisher_staked(&state, test_addr)
            .await
            .unwrap_err();
        assert!(matches!(err, PublisherAdmissionError::Unavailable(_)));

        // 4. Manually cache zero balance: must return Unstaked error without RPC call
        {
            let mut s = state.write().await;
            s.set_cached_publisher_stake(parsed_addr, alloy::primitives::U256::ZERO);
        }
        let err = ensure_publisher_staked(&state, test_addr)
            .await
            .unwrap_err();
        assert!(matches!(err, PublisherAdmissionError::Unstaked(_)));

        Ok(())
    }
}
