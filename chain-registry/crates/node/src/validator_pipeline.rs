// crates/node/src/validator_pipeline.rs
// Drives packages from pending pool through VRF → 3-stage validation →
// PBFT consensus → writes finalised Transaction to the channel.

use crate::{finalized_tx::FinalizedTxSender, NodeState};
use chrono::Utc;
use common::{ChainRecord, PackageStatus, PublishRequest, Transaction, ValidatorVote};
use consensus::{aggregate_evidence_votes, EvidenceVoteOutcome};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

const POLL_INTERVAL_SECS: u64 = 1;
const VOTE_POLL_MS: u64 = 500;

pub async fn run(
    state: Arc<RwLock<NodeState>>,
    tx_out: FinalizedTxSender,
    p2p_handle: crate::p2p::P2PHandle,
) {
    let mut ticker = interval(Duration::from_secs(POLL_INTERVAL_SECS));
    tracing::info!("Validator pipeline started");

    loop {
        ticker.tick().await;
        if let Err(e) = tick(Arc::clone(&state), &tx_out, p2p_handle.clone()).await {
            tracing::error!("Validator pipeline error: {}", e);
        }
    }
}

async fn tick(
    state: Arc<RwLock<NodeState>>,
    tx_out: &FinalizedTxSender,
    p2p_handle: crate::p2p::P2PHandle,
) -> anyhow::Result<()> {
    // Observer nodes sync validator-set state from L1 but do not run local
    // validation. Draining the pending pool here would remove submissions with
    // no chain record, so `GET /v1/packages/:canonical` would 404 immediately.
    if !state.read().await.config.is_validator {
        return Ok(());
    }

    let pending: Vec<PublishRequest> = {
        let mut s = state.write().await;
        s.pending_pool.ready_for_validation()
    };

    if pending.is_empty() {
        return Ok(());
    }
    tracing::info!("Pipeline processing {} package(s)", pending.len());

    let handles: Vec<_> = pending
        .into_iter()
        .map(|req| {
            let state = Arc::clone(&state);
            let sender = tx_out.clone();
            let p2p_handle = p2p_handle.clone();
            tokio::spawn(async move {
                process_package(state, req, sender, p2p_handle).await;
            })
        })
        .collect();

    for h in handles {
        if let Err(e) = h.await {
            tracing::error!("Package task panicked: {}", e);
        }
    }
    Ok(())
}

static PIPELINE_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| {
        let permits = std::env::var("CREG_MAX_CONCURRENT_VALIDATIONS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| (n.get() * 2).max(4))
                    .unwrap_or(16)
            });
        tracing::info!(
            "Initializing validator pipeline semaphore with {} concurrent permits",
            permits
        );
        Arc::new(tokio::sync::Semaphore::new(permits))
    });

async fn process_package(
    state: Arc<RwLock<NodeState>>,
    req: PublishRequest,
    tx_out: FinalizedTxSender,
    p2p_handle: crate::p2p::P2PHandle,
) {
    let _permit = match PIPELINE_SEMAPHORE.acquire().await {
        Ok(permit) => permit,
        Err(e) => {
            tracing::error!("Failed to acquire pipeline semaphore permit: {}", e);
            return;
        }
    };

    let canonical = req.id.canonical();
    tracing::info!("Processing {}", canonical);

    let ipfs_url = {
        let s = state.read().await;
        s.config.ipfs_url.clone()
    };

    // ── Fetch tarball from IPFS ───────────────────────────────────────────────
    let mut tarball = match fetch_from_ipfs(&req.ipfs_cid, &ipfs_url).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("IPFS fetch failed for {}: {}", canonical, e);
            cleanup(&state, &canonical).await;
            return;
        }
    };

    if tarball.is_empty() {
        tracing::error!("Empty tarball received for {} — rejecting", canonical);
        cleanup(&state, &canonical).await;
        return;
    }

    // ── 2.5. Decrypt if shielded ──────────────────────────────────────────────
    if req.shielded {
        let Some(bundle) = &req.key_bundle else {
            tracing::warn!(
                canonical = %canonical,
                "Dropping shielded submission {} with missing key_bundle (VNA-01)",
                canonical
            );
            cleanup(&state, &canonical).await;
            return;
        };
        tracing::info!("Decrypting shielded package: {}", canonical);
        match decrypt_shielded(&tarball, bundle, &state).await {
            Ok(decrypted) => {
                tarball = decrypted;
            }
            Err(e) => {
                tracing::error!("Decryption failed for {}: {}", canonical, e);
                cleanup(&state, &canonical).await;
                return;
            }
        }
    }

    // ── Verify content hash (never auto-emit Transaction::Revoke on pre-vote mismatch, VNA-01) ──
    let actual = common::sha256_hex(&tarball);
    if let Some(reason) = precheck_fetched_tarball_drop_reason(
        req.shielded,
        req.key_bundle.is_some(),
        &req.content_hash,
        &actual,
    ) {
        tracing::warn!(
            canonical = %canonical,
            declared = %req.content_hash,
            actual = %actual,
            "Dropping unverified submission {}: {} (no automatic Transaction::Revoke)",
            canonical,
            reason
        );
        cleanup(&state, &canonical).await;
        return;
    }

    let (
        is_validator,
        node_id,
        privkey_opt,
        prev_manifest,
        reputation_snapshot,
        assigned_validator_count,
        admitted_committee,
        chain_id,
        genesis_hash,
    ) = {
        let s = state.read().await;
        let prev = s
            .chain
            .get_latest_version(&req.id.ecosystem, &req.id.name)
            .ok()
            .flatten();
        let tip = s.chain.tip_height().unwrap_or(0);
        let tip_timestamp = s
            .chain
            .get_block_by_height(tip)
            .ok()
            .flatten()
            .map(|b| b.header.timestamp)
            .unwrap_or_else(|| common::Block::genesis().header.timestamp);
        let rep_snapshot = s.publisher_index.get(&req.publisher_pubkey).map(|stats| {
            validator::reputation::PublisherReputationSnapshot {
                total_packages: stats.total_packages,
                verified_count: stats.verified_count,
                revoked_count: stats.revoked_count,
                stake_wei: stats.stake_wei,
                account_age_days: stats.account_age_days_at(tip_timestamp),
                revoked_pgp_fps: Vec::new(),
            }
        });
        let genesis_hash = if !s.config.network_genesis_hash.is_empty() {
            s.config.network_genesis_hash.clone()
        } else {
            s.chain
                .get_block_by_height(0)
                .ok()
                .flatten()
                .map(|b| b.hash())
                .unwrap_or_default()
        };
        (
            s.config.is_validator,
            s.config.node_id.clone(),
            s.config.validator_privkey.clone(),
            // Retrieve the previous version's manifest for diff analysis.
            // Returns None for the first publish of a package.
            prev.and_then(|r| r.manifest),
            rep_snapshot,
            s.validator_set.validators.len(),
            s.pending_pool.get_committee(&canonical).cloned(),
            s.config.chain_id.clone(),
            genesis_hash,
        )
    };

    // === CANONICAL COMMITTEE ASSIGNMENT ===
    // Admission persists the assignment. Do not recompute a second sampling
    // predicate here: that previously allowed a validator to be mandatory in
    // the recorded committee but ineligible for the expensive analysis.
    let committee = match admitted_committee {
        Some(committee) => committee,
        None => {
            compute_hybrid_committee(
                &state,
                &canonical,
                &req.content_hash,
                assigned_validator_count,
            )
            .await
        }
    };
    // A committee that claims a transition certificate must be reproducible
    // from that exact persisted certificate. Do not fall back to the mutable
    // current validator set: doing so would make fork/replay substitution
    // possible after a rotation.
    if !committee.validator_snapshot_certificate_hash.is_empty() {
        let verified = {
            let s = state.read().await;
            let expected_selection_randomness =
                s.chain
                    .get_block_by_hash(&committee.validator_snapshot_anchor_block_hash)
                    .ok()
                    .flatten()
                    .filter(|block| block.hash() == committee.validator_snapshot_anchor_block_hash)
                    .map(|block| {
                        block.header.vrf_output.unwrap_or_else(|| {
                            committee.validator_snapshot_anchor_block_hash.clone()
                        })
                    });
            expected_selection_randomness
                .as_deref()
                .and_then(|expected_selection_randomness| {
                    crate::validator_set_history::committee_authority_at(
                        &s.config.data_dir,
                        committee.validator_snapshot_height,
                        &s.validator_set_bootstrap,
                    )
                    .ok()
                    .flatten()
                    .filter(|snapshot| {
                        crate::package_admission::verify_analysis_committee_assignment(
                            snapshot,
                            &canonical,
                            &req.content_hash,
                            expected_selection_randomness,
                            &committee,
                        )
                        .is_ok()
                    })
                })
                .is_some()
        };
        if !verified {
            tracing::error!(
                canonical = %canonical,
                "Rejecting package with an invalid certified committee assignment"
            );
            cleanup(&state, &canonical).await;
            return;
        }
    }
    {
        let mut sw = state.write().await;
        sw.pending_pool
            .set_analysis_committee(&canonical, committee.clone());
        sw.package_committees
            .insert(canonical.clone(), committee.clone());
    }

    let is_mandatory = committee
        .members
        .iter()
        .take(committee.effective_size.max(committee.size) as usize)
        .any(|member| member == &node_id);

    // === LARGE-SCALE ANALYSIS SAMPLING (critical for 10k+ validators) ===
    // We only run the expensive full pipeline (IPFS fetch already done + sandbox etc.)
    // on a subset of validators. The rest will rely on evidence votes gossiped by
    // the selected analyzers.
    let should_analyze = is_validator && is_mandatory;

    let (
        vote,
        pgp_fingerprint,
        findings,
        analysis_bundles,
        evidence_digest,
        deterministic_risk,
        ml_model_version,
    ) = if should_analyze {
        if let Some(privkey) = privkey_opt.as_ref() {
            tracing::info!(
                "[Consensus] Running FULL analysis for {} (set size: {}, this node selected)",
                canonical,
                assigned_validator_count
            );
            match validator::validate_package_with_reputation(
                &req,
                &tarball,
                privkey,
                prev_manifest.as_ref(),
                None,
                reputation_snapshot.as_ref(),
            )
            .await
            {
                Ok(res) => {
                    let evidence_digest = res.deterministic_risk.evidence_digest.clone();
                    let deterministic_risk = res.deterministic_risk.to_common_summary();
                    let ml_model_version = if res.ml_model_version.trim().is_empty() {
                        ml_validator::DeepScanner::default().model_version()
                    } else {
                        res.ml_model_version.clone()
                    };
                    (
                        res.vote,
                        res.pgp_fingerprint,
                        res.findings,
                        res.analysis_bundles.to_refs(),
                        evidence_digest,
                        deterministic_risk,
                        ml_model_version,
                    )
                }
                Err(e) => {
                    tracing::error!("Validation error for {}: {}", canonical, e);
                    cleanup(&state, &canonical).await;
                    return;
                }
            }
        } else {
            tracing::error!(
                "[Consensus] Validator node missing private key — cannot analyze {}",
                canonical
            );
            cleanup(&state, &canonical).await;
            return;
        }
    } else {
        // Non-validator nodes should NOT cast votes — they observe consensus
        // results from validators but do not participate in the vote.
        tracing::info!(
            "[Consensus] Node is NOT a validator — not participating in consensus for {}",
            canonical
        );
        cleanup(&state, &canonical).await;
        return;
    };

    // Sampling guard moved earlier; the expensive validate_package is now guarded by `should_analyze`.

    // ── Generate our own signature (validators only) ──────────────────────────
    // Non-validators skipped consensus steps already; guard here defensively.
    let privkey_str = match privkey_opt.as_ref() {
        Some(k) => k,
        None => {
            tracing::warn!("No validator key — skipping signing for {}", canonical);
            cleanup(&state, &canonical).await;
            return;
        }
    };

    let publisher_authorization_digest = if req.publisher_authorization_version == 4 {
        common::PublisherAuthorizationV1::from_publish_request(&req)
            .authorization_digest(&req.id, &req.content_hash)
    } else {
        String::new()
    };

    let (signing_key, validator_pubkey) = {
        use common::crypto::SigningKey;
        let key_bytes = match hex::decode(privkey_str) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("Invalid validator key hex for {}: {}", canonical, e);
                cleanup(&state, &canonical).await;
                return;
            }
        };
        let key_arr: [u8; 32] = match key_bytes.try_into() {
            Ok(a) => a,
            Err(_) => {
                tracing::error!("Validator key must be 32 bytes for {}", canonical);
                cleanup(&state, &canonical).await;
                return;
            }
        };
        let signing_key = SigningKey::from_ed25519_seed(&key_arr);
        let validator_pubkey = signing_key.public_key().to_hex();
        (signing_key, validator_pubkey)
    };

    let our_sig = {
        let approved = matches!(vote, ValidatorVote::Approve);
        let scanner_profile_digest =
            common::scanner_profile_digest(&ml_model_version, &analysis_bundles);
        let reject_reason = match &vote {
            ValidatorVote::Approve => None,
            ValidatorVote::Reject { reason } => Some(reason.as_str()),
        };
        let vote_metadata_digest =
            crate::gossip::vote_metadata_digest(should_analyze, &deterministic_risk, reject_reason);

        // Use the same domain-separated message format for both local records
        // and gossiped votes so synced blocks can verify every validator vote
        // against one canonical payload.
        let msg = if publisher_authorization_digest.is_empty() {
            crate::gossip::canonical_vote_message(
                &canonical,
                &req.content_hash,
                approved,
                &validator_pubkey,
                &scanner_profile_digest,
                &evidence_digest,
            )
        } else {
            crate::gossip::canonical_authorized_vote_message(
                &canonical,
                &req.content_hash,
                approved,
                &validator_pubkey,
                &scanner_profile_digest,
                &evidence_digest,
                &publisher_authorization_digest,
                &vote_metadata_digest,
            )
        };
        let signature = signing_key.sign(msg.as_bytes());

        common::ValidatorSignature {
            validator_id: node_id.clone(),
            validator_pubkey: validator_pubkey.clone(),
            signature: signature.to_hex(),
            vote: vote.clone(),
            signed_at: Utc::now(),
            ml_model_version,
            analysis_bundles: analysis_bundles.clone(),
            evidence_digest: evidence_digest.clone(),
            deterministic_risk: deterministic_risk.clone(),
            is_mandatory: should_analyze, // in hybrid: true only for selected mandatory nodes
            ..Default::default()
        }
    };

    // Store our own vote locally
    {
        let mut sw = state.write().await;
        sw.record_package_vote(canonical.clone(), our_sig.clone());
    }

    // Gossip our vote to peers via P2P Gossipsub
    // Only mandatory (or voluntary) analyzers produce and gossip a vote in hybrid model.
    if should_analyze {
        let (approved, reject_reason) = match &vote {
            ValidatorVote::Approve => (true, None),
            ValidatorVote::Reject { reason } => (false, Some(reason.clone())),
        };

        // Reuse the signature produced above — same canonical vote message format.
        let gossip_sig = our_sig.signature.clone();

        let gossip_vote = crate::gossip::VoteGossip {
            consensus_subject: canonical.clone(),
            content_hash: req.content_hash.clone(),
            publisher_authorization_digest: publisher_authorization_digest.clone(),
            validator_id: node_id.clone(),
            validator_pubkey: our_sig.validator_pubkey.clone(),
            ml_model_version: our_sig.ml_model_version.clone(),
            analysis_bundles: analysis_bundles.clone(),
            evidence_digest: evidence_digest.clone(),
            deterministic_risk: deterministic_risk.clone(),
            phase: "commit".into(),
            approved,
            reject_reason,
            signature: gossip_sig,
            is_mandatory: our_sig.is_mandatory,
        };

        let gossip_bytes = match serde_json::to_vec(&gossip_vote) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::error!("Failed to serialize gossip vote for {}: {}", canonical, e);
                cleanup(&state, &canonical).await;
                return;
            }
        };

        if p2p_handle
            .sender
            .send(crate::p2p::P2PCommand::Broadcast {
                topic: "creg/v1/votes".into(),
                data: gossip_bytes,
            })
            .await
            .is_err()
        {
            tracing::warn!(
                "P2P broadcast channel closed while gossiping vote for {}",
                canonical
            );
        }
    } else {
        // Non-analyzers skip gossip; they rely on committee evidence.
    }

    // ── WAIT FOR QUORUM OUTCOME ───────────────────────────────────────────────
    let (assigned_validator_count, vote_timeout_secs) = {
        let s = state.read().await;
        (
            s.validator_set.validators.len(),
            s.config.vote_timeout_secs.max(1),
        )
    };
    let mut consensus_outcome = None;

    // Get committee for hybrid quorum
    let mut committee = {
        let sr = state.read().await;
        sr.package_committees
            .get(&canonical)
            .cloned()
            .unwrap_or_default()
    };

    let max_iterations = vote_timeout_secs
        .saturating_mul(1000)
        .div_ceil(VOTE_POLL_MS);
    for i in 0..max_iterations {
        {
            let sr = state.read().await;
            if let Some(round) = sr.package_round(&canonical) {
                // Use hybrid quorum logic (mandatory core first, volunteers for liveness)
                if let Some(outcome) = aggregate_evidence_votes(round.signatures(), &committee) {
                    consensus_outcome = Some(outcome);
                    break;
                }
            }
        }

        // Hybrid expansion: after primary timeout, expand eligible committee
        if i > (vote_timeout_secs * 1000 / VOTE_POLL_MS) / 2 {
            let mut sw = state.write().await;
            let expanded_size = sw.package_committees.get_mut(&canonical).and_then(|c| {
                if c.effective_size < (c.members.len() as u32).min(256) {
                    let expand_by =
                        32u32.min((c.members.len() as u32).saturating_sub(c.effective_size));
                    c.effective_size += expand_by;
                    Some(c.effective_size)
                } else {
                    None
                }
            });
            if let Some(new_size) = expanded_size {
                committee.effective_size = new_size;
                sw.pending_pool
                    .update_effective_committee_size(&canonical, new_size);
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(VOTE_POLL_MS)).await;
    }

    let Some(consensus_outcome) = consensus_outcome else {
        let detail = {
            let s = state.read().await;
            s.package_round(&canonical)
                .map(|round| summarize_vote_round(round.signatures(), assigned_validator_count))
                .unwrap_or_else(|| "no votes recorded".to_string())
        };
        tracing::error!(
            "Consensus timeout for package {} after {}s (quorum needs {} of {} validators): {}",
            canonical,
            vote_timeout_secs,
            (assigned_validator_count * 2 / 3) + 1,
            assigned_validator_count,
            detail
        );
        cleanup(&state, &canonical).await;
        return;
    };

    match (&vote, &consensus_outcome) {
        (ValidatorVote::Approve, EvidenceVoteOutcome::Rejected { reason }) => {
            tracing::warn!(
                "{} local validator approved, but consensus rejected: {}",
                canonical,
                reason
            );
        }
        (ValidatorVote::Reject { reason }, EvidenceVoteOutcome::Verified(_)) => {
            tracing::warn!(
                "{} local validator rejected ({}), but consensus verified",
                canonical,
                reason
            );
        }
        _ => {}
    }

    // ── Write finalised transaction ───────────────────────────────────────────
    let (tx, outcome_label) = match consensus_outcome {
        EvidenceVoteOutcome::Verified(final_sigs) => {
            let mandatory_contributed = final_sigs.iter().filter(|s| s.is_mandatory).count() as u32;

            // Snapshot pre-finality performance scores without mutating state
            // before canonical block commit; deterministic updates run in
            // NodeState::on_block_committed.
            let mut committee_for_record = committee.clone();
            {
                let s = state.read().await;
                for mid in &committee.members {
                    if let Some(v) = s.validator_set.validators.iter().find(|vv| &vv.id == mid) {
                        committee_for_record
                            .member_performance_at_finalization
                            .insert(mid.clone(), v.performance_score);
                    }
                }
            }

            let record = ChainRecord {
                id: req.id.clone(),
                content_hash: req.content_hash.clone(),
                ipfs_cid: req.ipfs_cid.clone(),
                publisher_pubkey: req.publisher_pubkey.clone(),
                publisher_pubkeys: req.publisher_pubkeys.clone(),
                block_hash: "pending".into(),
                published_at: Utc::now(),
                validator_signatures: final_sigs.clone(),
                status: PackageStatus::Verified,
                shielded: req.shielded,
                key_bundle: req.key_bundle.clone(),
                pgp_fingerprint,
                pgp_signature_sha256: req
                    .pgp_signature
                    .as_ref()
                    .map(|value| common::sha256_hex(value.as_bytes()))
                    .unwrap_or_default(),
                pgp_public_key_sha256: req
                    .pgp_public_key
                    .as_ref()
                    .map(|value| common::sha256_hex(value.as_bytes()))
                    .unwrap_or_default(),
                findings,
                analysis_bundles,
                evidence_digest,
                deterministic_risk,
                access_count: 0,
                last_accessed: None,
                manifest: Some(req.manifest.clone()),
                threshold: req.threshold,
                // Full committee data now captured in the immutable on-chain record
                analysis_committee_size: committee.size,
                analysis_selection_seed: committee.seed.clone(),
                analysis_effective_size: committee.effective_size,
                analysis_mandatory_contributed: mandatory_contributed,
                analysis_committee: Some(committee_for_record),
                ..Default::default()
            };
            // This path runs before canonical block finality. Reward sends are
            // held until a durable, governance-authorized dispatcher can bind
            // them to the finalized record and recover their receipts.
            {
                let mut s = state.write().await;
                s.reward_emissions_skipped = s.reward_emissions_skipped.saturating_add(1);
            }
            tracing::info!(
                target: "reward_emission",
                canonical = %canonical,
                "Reward dispatch skipped pending canonical finality and an authorized durable dispatcher"
            );

            let transaction = if req.publisher_authorization_version == 4 {
                let authorization = common::PublisherAuthorizationV1::from_publish_request(&req);
                if let Err(error) = authorization.validate_record_binding(&record) {
                    tracing::error!(
                        canonical = %canonical,
                        "Refusing to finalize invalid publisher v4 authorization: {}",
                        error
                    );
                    cleanup(&state, &canonical).await;
                    return;
                }
                Transaction::PublishAuthorizedV1 {
                    record,
                    authorization,
                }
            } else {
                Transaction::Publish(record)
            };
            (transaction, "VERIFIED")
        }
        EvidenceVoteOutcome::Rejected { reason } => {
            let mut request = common::SignedRevocationRequest {
                chain_id,
                genesis_hash,
                package_canonical: canonical.clone(),
                reason,
                revoker_pubkey: validator_pubkey,
                signature: String::new(),
                signature_version: 1,
            };
            request.signature = signing_key
                .sign(request.signing_message().as_bytes())
                .to_hex();
            (
                common::Transaction::RevokeV2 {
                    request,
                    evidence_hash: evidence_digest,
                    witness: None,
                },
                "REJECTED",
            )
        }
    };

    if let Err(error) = crate::finalized_tx::persist_and_notify(&state, &tx_out, tx.clone()).await {
        tracing::error!(
            "Finalized work persisted but could not notify block producer for {}: {}",
            canonical,
            error
        );
    } else {
        tracing::info!("{} → {}", canonical, outcome_label);
        let mut s = state.write().await;
        let height = s.chain.tip_height().unwrap_or(0);
        s.forced_inclusion_tracker.submit(tx, height);
    }

    cleanup(&state, &canonical).await;
}

fn summarize_vote_round(
    signatures: &[common::ValidatorSignature],
    assigned_validator_count: usize,
) -> String {
    let quorum = (assigned_validator_count * 2 / 3) + 1;
    let mut degraded = 0usize;
    let mut approve_profiles: HashMap<String, usize> = HashMap::new();
    let mut reject_count = 0usize;

    for sig in signatures {
        if !common::is_consensus_grade_vote(
            &sig.ml_model_version,
            &sig.analysis_bundles,
            &sig.evidence_digest,
        ) {
            degraded += 1;
            continue;
        }
        let profile_key = format!(
            "{}:{}",
            common::scanner_profile_digest(&sig.ml_model_version, &sig.analysis_bundles),
            &sig.evidence_digest[..sig.evidence_digest.len().min(12)]
        );
        match &sig.vote {
            ValidatorVote::Approve => {
                *approve_profiles.entry(profile_key).or_default() += 1;
            }
            ValidatorVote::Reject { .. } => reject_count += 1,
        }
    }

    let best_profile = approve_profiles
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(profile, count)| format!("{profile}={count}"))
        .unwrap_or_else(|| "none".to_string());

    format!(
        "votes={} degraded={} rejects={} quorum={} top_approve_profile={}",
        signatures.len(),
        degraded,
        reject_count,
        quorum,
        best_profile
    )
}

async fn cleanup(state: &Arc<RwLock<NodeState>>, canonical: &str) {
    let mut s = state.write().await;
    s.pending_pool.remove(canonical);
    s.clear_package_round(canonical);
}

/// Maximum IPFS payload size (512 MB). Packages larger than this are rejected
/// to prevent OOM attacks via malicious CIDs.
const MAX_IPFS_PAYLOAD_BYTES: u64 = 512 * 1024 * 1024;

/// Timeout for IPFS fetch operations (5 minutes).
const IPFS_FETCH_TIMEOUT_SECS: u64 = 300;

fn precheck_fetched_tarball_drop_reason(
    shielded: bool,
    has_key_bundle: bool,
    declared_hash: &str,
    actual_hash: &str,
) -> Option<&'static str> {
    if shielded && !has_key_bundle {
        return Some("shielded submission missing key_bundle");
    }
    if !declared_hash.eq_ignore_ascii_case(actual_hash) {
        return Some("content hash mismatch");
    }
    None
}

async fn fetch_from_ipfs(cid: &str, ipfs_url: &str) -> anyhow::Result<Vec<u8>> {
    crate::ipfs_fetch::fetch_ipfs_bounded(
        cid,
        ipfs_url,
        MAX_IPFS_PAYLOAD_BYTES,
        IPFS_FETCH_TIMEOUT_SECS,
    )
    .await
    .map_err(Into::into)
}

/// Compute the hybrid analysis committee for a package.
/// Mandatory core of ~100 (or scaled for small sets) using deterministic hash selection.
/// Returns AnalysisCommittee with members ordered for selection and expansion.
/// This is the core of the hybrid mandatory + voluntary model.
async fn compute_hybrid_committee(
    state: &Arc<RwLock<NodeState>>,
    canonical: &str,
    content_hash: &str,
    _total_validators: usize,
) -> common::AnalysisCommittee {
    let s = state.read().await;
    let tip = s.chain.tip_hash().unwrap_or_default();
    let recent_vrf = s
        .chain
        .get_block_by_hash(&tip)
        .ok()
        .flatten()
        .and_then(|b| b.header.vrf_output.clone());
    // Delegate to the canonical admission-time implementation (which now includes stake-weighting + cap + VRF seed)
    crate::package_admission::compute_analysis_committee_at_admission(
        &s.validator_set,
        canonical,
        content_hash,
        tip,
        recent_vrf,
    )
}

/// Decrypt a shielded package produced by `creg publish --shield`.
///
/// Matches the on-wire format emitted by `cli/src/publish.rs::encrypt_for_validators`:
///
///   * **Tarball** (uploaded to IPFS): `nonce (12) || aes256_gcm_ciphertext`.
///   * **Key bundle** (stored on-chain in `PublishRequest.key_bundle`):
///       - `"plain:<aes_key_hex>:<nonce_hex>"` — dev fallback when no
///         validator-set X25519 pubkey is configured.
///       - `"ecies:<eph_pub_hex>:<wrap_nonce_hex>:<encrypted_bundle_hex>"` —
///         X25519 ECDH to the validator-set pubkey, HKDF-SHA256-derived wrap
///         key, AES-256-GCM wrap of the raw `"<aes_key_hex>:<nonce_hex>"` tuple.
///
/// When the chain spec enables `threshold_encryption`, this path **fails closed**
/// instead of falling back to single-node X25519 (ISSUE-014). A future t-of-n
/// upgrade needs share-collection gossip plus `crates/threshold-encryption`.
async fn decrypt_shielded(
    data: &[u8],
    bundle: &str,
    state: &crate::SharedState,
) -> anyhow::Result<Vec<u8>> {
    let threshold_encryption = state.read().await.config.threshold_encryption;
    let plaintext = decrypt_shielded_with_policy(threshold_encryption, data, bundle)?;
    tracing::info!(
        "Shielded package decrypted successfully: {} bytes",
        plaintext.len()
    );
    Ok(plaintext)
}

fn decrypt_shielded_with_policy(
    threshold_encryption: bool,
    data: &[u8],
    bundle: &str,
) -> anyhow::Result<Vec<u8>> {
    if threshold_encryption {
        anyhow::bail!(
            "chain spec enables threshold_encryption but validator share collection is not active; \
             refusing single-node X25519 fallback"
        );
    }
    common::decrypt_shielded_package(data, bundle)
}

// Note: The Shamir secret-sharing threshold-decryption path (decrypt_share,
// broadcast_decryption_share, collect_decryption_shares, reconstruct_key,
// decrypt_with_key) is not wired. Shielded packages on Sepolia use experimental
// single-node X25519 only while `feature_flags.threshold_encryption` is false.
// Enabling that flag without share distribution fails closed.

#[cfg(test)]
mod shielded_tests {
    /// `decrypt_shielded` delegates to `common::decrypt_shielded_package`; exercise the same wire format the CLI publishes.
    #[tokio::test]
    async fn decrypt_shielded_matches_common_plain_round_trip() {
        let plaintext = b"shielded e2e plain path";
        let (wire, bundle) = common::encrypt_shielded_package(plaintext, None).unwrap();
        let got = common::decrypt_shielded_package(&wire, &bundle).unwrap();
        assert_eq!(got, plaintext);
    }

    #[tokio::test]
    async fn decrypt_shielded_matches_common_ecies_round_trip() {
        let secret = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
        let public = x25519_dalek::PublicKey::from(&secret);
        std::env::set_var(
            "CREG_VALIDATOR_PRIVKEY_X25519",
            hex::encode(secret.to_bytes()),
        );

        let plaintext = b"shielded e2e ecies path";
        let (wire, bundle) =
            common::encrypt_shielded_package(plaintext, Some(public.as_bytes())).unwrap();
        let got = common::decrypt_shielded_package(&wire, &bundle).unwrap();
        assert_eq!(got, plaintext);

        std::env::remove_var("CREG_VALIDATOR_PRIVKEY_X25519");
    }

    #[test]
    fn threshold_flag_refuses_x25519_fallback() {
        let plaintext = b"must not decrypt when threshold flag is on";
        let (wire, bundle) = common::encrypt_shielded_package(plaintext, None).unwrap();
        let err = super::decrypt_shielded_with_policy(true, &wire, &bundle)
            .expect_err("threshold mode must fail closed without share collection");
        assert!(err.to_string().contains("refusing single-node X25519"));

        let got = super::decrypt_shielded_with_policy(false, &wire, &bundle).unwrap();
        assert_eq!(got, plaintext);
    }

    #[test]
    fn mismatch_and_missing_key_bundle_drop_without_revocation() {
        assert_eq!(
            super::precheck_fetched_tarball_drop_reason(true, false, "aa", "aa"),
            Some("shielded submission missing key_bundle")
        );
        assert_eq!(
            super::precheck_fetched_tarball_drop_reason(false, false, "aa", "bb"),
            Some("content hash mismatch")
        );
        assert_eq!(
            super::precheck_fetched_tarball_drop_reason(false, false, "AA", "aa"),
            None
        );
    }
}
