// crates/node/src/block_producer.rs
// Produces new blocks on a fixed interval from a bounded/backpressured
// finalized-tx buffer. Durable queue migration remains a release gate.

use crate::{chain_store::FinalizedWorkFailureOutcome, finalized_tx, NodeState};
use anyhow::Context;
use chrono::Utc;
use common::{merkle_root, transaction_hash, Block, BlockHeader, PackageStatus, Transaction};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

/// Replacement leadership is enabled unconditionally (ADR-018 Increment 3).
pub(crate) const PBFT_VIEW_CHANGE_READY: bool = true;

/// Snapshot gathered after the VRF proposer gate passes.
struct ProposerContext {
    tip_height: u64,
    prev_hash: String,
    node_id: String,
    privkey: Option<chain_registry_secrets::SecretString>,
    our_pubkey: Option<String>,
    validator_set_hash: String,
    package_authority: common::ValidatorSet,
    pbft_authority: consensus::ValidatorSet,
    view_number: u32,
    view_change_certificate: Vec<common::ViewChangeVote>,
    prepared_transactions: Option<Vec<Transaction>>,
    reproposal: Option<common::GossipMessage>,
    vrf_output: Option<String>,
    vrf_proof: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "stale proposer context: captured tip {captured_height}:{captured_hash}, current tip {current_height}:{current_hash}"
)]
struct StaleProposerContext {
    captured_height: u64,
    captured_hash: String,
    current_height: u64,
    current_hash: String,
}

#[derive(Debug, thiserror::Error)]
#[error("PBFT safety gate: {0}")]
struct PbftSafetyGate(String);

/// A narrowly scoped crash-capture arm for a real Phase C testnet revocation.
/// It is compiled only into the Phase C candidate and is inert unless all four
/// target fields are explicitly supplied at process start.
#[cfg(feature = "phase-c-real-fleet")]
#[derive(Debug)]
struct CommitCaptureArm {
    node_id: String,
    chain_id: String,
    genesis_hash: String,
    height: u64,
    package_canonical: String,
    pause_ms: u64,
}

#[cfg(feature = "phase-c-real-fleet")]
fn capture_env(name: &str) -> anyhow::Result<Option<String>> {
    match std::env::var_os(name) {
        Some(value) => value
            .into_string()
            .map(Some)
            .map_err(|_| anyhow::anyhow!("{name} must contain valid UTF-8")),
        None => Ok(None),
    }
}

#[cfg(feature = "phase-c-real-fleet")]
fn read_commit_capture_arm() -> anyhow::Result<Option<CommitCaptureArm>> {
    let node_id = capture_env("CREG_PHASE_C_COMMIT_CAPTURE_NODE_ID")?;
    let chain_id = capture_env("CREG_PHASE_C_COMMIT_CAPTURE_CHAIN_ID")?;
    let genesis_hash = capture_env("CREG_PHASE_C_COMMIT_CAPTURE_GENESIS_HASH")?;
    let height = capture_env("CREG_PHASE_C_COMMIT_CAPTURE_HEIGHT")?;
    let package_canonical = capture_env("CREG_PHASE_C_COMMIT_CAPTURE_PACKAGE_CANONICAL")?;
    let pause_ms = capture_env("CREG_PHASE_C_COMMIT_CAPTURE_PAUSE_MS")?;
    let configured = [
        node_id.is_some(),
        chain_id.is_some(),
        genesis_hash.is_some(),
        height.is_some(),
        package_canonical.is_some(),
        pause_ms.is_some(),
    ];
    let configured_count = configured.iter().filter(|configured| **configured).count();
    if configured_count == 0 {
        return Ok(None);
    }
    anyhow::ensure!(
        configured_count == configured.len(),
        "durable COMMIT capture requires node ID, chain ID, genesis hash, height, package canonical ID, and pause duration"
    );

    let node_id = node_id.context("capture node ID is required")?;
    let chain_id = chain_id.context("capture chain ID is required")?;
    let genesis_hash = genesis_hash.context("capture genesis hash is required")?;
    let height = height
        .context("capture height is required")?
        .parse::<u64>()
        .context("capture height must be an unsigned integer")?;
    let package_canonical = package_canonical.context("capture package ID is required")?;
    let pause_ms = pause_ms
        .context("capture pause duration is required")?
        .parse::<u64>()
        .context("capture pause duration must be an unsigned integer")?;

    anyhow::ensure!(
        !node_id.trim().is_empty(),
        "capture node ID cannot be empty"
    );
    anyhow::ensure!(
        !chain_id.trim().is_empty(),
        "capture chain ID cannot be empty"
    );
    anyhow::ensure!(
        !genesis_hash.trim().is_empty(),
        "capture genesis hash cannot be empty"
    );
    anyhow::ensure!(height > 0, "capture height must be greater than zero");
    anyhow::ensure!(
        !package_canonical.trim().is_empty(),
        "capture package ID cannot be empty"
    );
    anyhow::ensure!(
        (1..=30_000).contains(&pause_ms),
        "capture pause duration must be between 1 and 30000 milliseconds"
    );

    Ok(Some(CommitCaptureArm {
        node_id,
        chain_id,
        genesis_hash,
        height,
        package_canonical,
        pause_ms,
    }))
}

#[cfg(feature = "phase-c-real-fleet")]
pub(crate) fn validate_commit_capture_arm(
    config: &crate::config::NodeConfig,
) -> anyhow::Result<()> {
    if let Some(arm) = read_commit_capture_arm()? {
        anyhow::ensure!(
            config.is_testnet && config.is_validator,
            "durable COMMIT crash capture is allowed only on a testnet validator"
        );
        anyhow::ensure!(
            config.node_id == arm.node_id,
            "durable COMMIT capture target does not match this validator identity"
        );
        anyhow::ensure!(
            config.chain_id == arm.chain_id && config.network_genesis_hash == arm.genesis_hash,
            "durable COMMIT capture target does not match this signed chain identity"
        );
    }
    Ok(())
}

#[cfg(feature = "phase-c-real-fleet")]
pub(crate) async fn pause_after_durable_commit_if_armed(
    state: &NodeState,
    block: &Block,
    block_hash: &str,
    view_number: u32,
) -> anyhow::Result<()> {
    let Some(arm) = read_commit_capture_arm()? else {
        return Ok(());
    };
    if state.config.node_id != arm.node_id
        || state.config.chain_id != arm.chain_id
        || state.config.network_genesis_hash != arm.genesis_hash
        || block.header.height != arm.height
        || !block.transactions.iter().any(|transaction| {
            matches!(
                transaction.revocation_target(),
                Some((package_canonical, _, _, _)) if package_canonical == arm.package_canonical
            )
        })
    {
        return Ok(());
    }

    tracing::warn!(
        event = "phase_c_durable_commit_capture_paused",
        node_id = %arm.node_id,
        chain_id = %arm.chain_id,
        genesis_hash = %arm.genesis_hash,
        height = block.header.height,
        parent_hash = %block.header.prev_hash,
        block_hash = %block_hash,
        view_number,
        package_canonical = %arm.package_canonical,
        pause_ms = arm.pause_ms,
        "Durable local COMMIT is held before in-memory application and peer broadcast"
    );
    tokio::time::sleep(Duration::from_millis(arm.pause_ms)).await;
    tracing::warn!(
        event = "phase_c_durable_commit_capture_resumed",
        node_id = %arm.node_id,
        height = block.header.height,
        block_hash = %block_hash,
        "Durable COMMIT capture pause elapsed"
    );
    Ok(())
}

fn ensure_proposer_context_is_current(
    captured_height: u64,
    captured_hash: &str,
    current_height: u64,
    current_hash: &str,
) -> Result<(), StaleProposerContext> {
    if captured_height != current_height || captured_hash != current_hash {
        return Err(StaleProposerContext {
            captured_height,
            captured_hash: captured_hash.to_owned(),
            current_height,
            current_hash: current_hash.to_owned(),
        });
    }
    Ok(())
}

fn restore_stale_context_transactions(
    pending_txs: &mut Vec<Transaction>,
    mut attempted_txs: Vec<Transaction>,
) {
    attempted_txs.append(pending_txs);
    *pending_txs = attempted_txs;
    finalized_tx::sync_pending_buffer_depth(pending_txs.len());
}

fn ensure_unprepared_context_is_current(
    s: &NodeState,
    ctx: &ProposerContext,
) -> anyhow::Result<()> {
    ensure_proposer_context_is_current(
        ctx.tip_height,
        &ctx.prev_hash,
        s.chain.tip_height()?,
        &s.chain.tip_hash()?,
    )?;
    let height = ctx.tip_height.saturating_add(1);
    let certified = s.pbft_engine.certified_view(height, &ctx.prev_hash);
    let current_view = certified.as_ref().map(|(view, _, _)| *view).unwrap_or(0);
    if current_view != ctx.view_number
        || certified
            .as_ref()
            .is_some_and(|(_, _, prepared)| prepared.is_some())
        || s.pbft_engine.non_terminal_round_at_height(height).is_some()
        || s.chain
            .pbft_safety_journal()?
            .has_prepared_or_commit_lock(height, &ctx.prev_hash)?
    {
        return Err(PbftSafetyGate(
            "unprepared queue selection raced a view or a protected PBFT value".into(),
        )
        .into());
    }
    Ok(())
}

/// Retire transactions that cannot be proposed again on the canonical chain.
/// This runs on every validator, including backups, but only when no live or
/// durable PBFT value protects the next height. The dead-letter write happens
/// before the in-memory and forced-inclusion copies are removed.
fn retire_ineligible_unprepared_work(
    s: &mut NodeState,
    pending_txs: &mut Vec<Transaction>,
) -> anyhow::Result<usize> {
    // Observers never sign, so they have no PBFT signing journal and no
    // prepared or commit lock to protect. Without this guard every observer
    // failed at the journal lookup below on every tick and logged the same
    // "durable safety journal is not initialized" warning each block interval
    // (136,649 times on one observer). It also never retired anything, so
    // returning early keeps observer behavior unchanged.
    if !s.config.is_validator {
        return Ok(0);
    }
    let tip_height = s.chain.tip_height()?;
    let prev_hash = s.chain.tip_hash()?;
    let height = tip_height.saturating_add(1);
    let certified = s.pbft_engine.certified_view(height, &prev_hash);
    if certified
        .as_ref()
        .is_some_and(|(_, _, prepared)| prepared.is_some())
        || s.pbft_engine.non_terminal_round_at_height(height).is_some()
        || s.chain
            .pbft_safety_journal()?
            .has_prepared_or_commit_lock(height, &prev_hash)?
    {
        return Ok(0);
    }

    let parent = s
        .chain
        .get_block_by_height(tip_height)?
        .context("canonical parent block is unavailable during terminal queue cleanup")?;
    let timestamp = crate::block_time::next_live_timestamp(parent.header.timestamp, Utc::now())?;
    let now = timestamp.timestamp();
    let mut seen = HashSet::new();
    pending_txs.retain(|tx| seen.insert(transaction_hash(tx)));
    let mut candidates = pending_txs.clone();
    for tx in s
        .forced_inclusion_tracker
        .forced_transaction_payloads(tip_height)
    {
        if seen.insert(transaction_hash(tx)) {
            candidates.push(tx.clone());
        }
    }

    let mut ineligible = Vec::new();
    for tx in &candidates {
        let reason = match tx {
            Transaction::PublishAuthorizedV1 { authorization, .. } => {
                let nonce_key = crate::namespace_consensus::publisher_authorization_nonce_key(
                    &authorization.publisher_address,
                    &authorization.nonce,
                );
                if s.chain
                    .is_publisher_authorization_nonce_applied(&nonce_key)?
                {
                    Some("publisher authorization nonce is already committed on this chain")
                } else if i128::from(authorization.expires_at) <= i128::from(now) {
                    Some("publisher authorization expired before an unprepared proposal")
                } else {
                    None
                }
            }
            Transaction::Revoke { .. } | Transaction::RevokeV2 { .. } => {
                let package_canonical = tx
                    .revocation_target()
                    .map(|(canonical, _, _, _)| canonical)
                    .unwrap_or_default();
                match s.chain.get_package(package_canonical)? {
                    Some(record) if matches!(record.status, PackageStatus::Revoked { .. }) => {
                        Some("revocation target is already revoked on this chain")
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(reason) = reason {
            ineligible.push((tx.clone(), reason));
        }
    }

    let mut retired = HashSet::new();
    for (tx, reason) in ineligible {
        let work_id = transaction_hash(&tx);
        // Persist first. On a storage error the buffer remains available and
        // the producer defers; no transaction is silently discarded.
        if let Err(error) = s.chain.dead_letter_unprepared_transaction(&tx, reason) {
            pending_txs.retain(|tx| !retired.contains(&transaction_hash(tx)));
            finalized_tx::sync_pending_buffer_depth(pending_txs.len());
            return Err(error);
        }
        s.forced_inclusion_tracker.retire_ineligible(&work_id);
        retired.insert(work_id.clone());
        tracing::warn!(work_id = %work_id, reason, "Ineligible unprepared work preserved in dead letter");
    }
    pending_txs.retain(|tx| !retired.contains(&transaction_hash(tx)));
    finalized_tx::sync_pending_buffer_depth(pending_txs.len());
    Ok(retired.len())
}

/// Select a valid new proposal while retaining competing nonce payloads until
/// canonical inclusion makes their authorization permanently ineligible.
async fn prepare_unprepared_batch(
    state: &Arc<RwLock<NodeState>>,
    pending_txs: &mut Vec<Transaction>,
    ctx: &ProposerContext,
) -> anyhow::Result<Vec<Transaction>> {
    let mut s = state.write().await;
    ensure_unprepared_context_is_current(&s, ctx)?;
    retire_ineligible_unprepared_work(&mut s, pending_txs)?;
    let parent = s
        .chain
        .get_block_by_height(ctx.tip_height)?
        .context("canonical parent block is unavailable during queue selection")?;
    let timestamp = crate::block_time::next_live_timestamp(parent.header.timestamp, Utc::now())?;

    let namespace_config =
        crate::namespace_consensus::NamespaceConsensusConfig::from_node_config(&s.config);
    let mut proposal = Block {
        header: BlockHeader {
            height: ctx.tip_height.saturating_add(1),
            prev_hash: ctx.prev_hash.clone(),
            merkle_root: merkle_root(&[]),
            proposer_id: ctx.node_id.clone(),
            timestamp,
            validator_set_hash: ctx.validator_set_hash.clone(),
            vrf_output: ctx.vrf_output.clone(),
            vrf_proof: ctx.vrf_proof.clone(),
            view_number: ctx.view_number,
            view_change_certificate: ctx.view_change_certificate.clone(),
        },
        transactions: Vec::new(),
        pbft_signatures: Vec::new(),
    };
    let mut selected = HashSet::new();
    for tx in pending_txs.iter() {
        if !s.chain.is_finalized_work_deliverable(tx)? {
            continue;
        }
        proposal.transactions.push(tx.clone());
        let validation =
            crate::namespace_consensus::validate_block(&s.chain, &proposal, &namespace_config)
                .map(|_| ())
                .and_then(|_| {
                    // Namespace validation needs the selected prefix. Package
                    // approval validation only needs this record and authority.
                    let record_block = Block {
                        header: proposal.header.clone(),
                        transactions: vec![tx.clone()],
                        pbft_signatures: Vec::new(),
                    };
                    crate::sync::verify_block_signatures(&record_block, &ctx.package_authority)
                });
        if let Err(error) = validation {
            proposal.transactions.pop();
            tracing::debug!(work_id = %transaction_hash(tx), error = %format!("{error:#}"),
                "Deferred unprepared work; retaining its original payload and authorization");
            continue;
        }
        selected.insert(transaction_hash(tx));
    }
    pending_txs.retain(|tx| !selected.contains(&transaction_hash(tx)));
    finalized_tx::sync_pending_buffer_depth(pending_txs.len());
    Ok(proposal.transactions)
}

async fn drive_pbft_view_changes(state: Arc<RwLock<NodeState>>, p2p_handle: crate::p2p::P2PHandle) {
    let mut ticker = interval(Duration::from_secs(5));
    let timeout = std::env::var("CREG_PBFT_TIMEOUT")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(30);
    let mut last_tip = {
        let s = state.read().await;
        (
            s.chain.tip_height().unwrap_or(0),
            s.chain.tip_hash().unwrap_or_default(),
        )
    };
    let mut tip_stalled_since = std::time::Instant::now();
    let mut last_view_change_at = tip_stalled_since;
    loop {
        ticker.tick().await;
        let (tip, node_id, privkey, data_dir, bootstrap, active_round, certified_view) = {
            let s = state.read().await;
            let tip_height = s.chain.tip_height().unwrap_or(0);
            let tip_hash = s.chain.tip_hash().unwrap_or_default();
            let certified_view = s
                .pbft_engine
                .certified_view(tip_height.saturating_add(1), &tip_hash)
                .is_some();
            (
                (tip_height, tip_hash),
                s.config.node_id.clone(),
                s.config.validator_privkey.clone(),
                s.config.data_dir.clone(),
                s.validator_set_bootstrap.clone(),
                s.pbft_engine
                    .non_terminal_round_at_height(tip_height.saturating_add(1))
                    .is_some(),
                certified_view,
            )
        };
        if tip != last_tip {
            last_tip = tip.clone();
            tip_stalled_since = std::time::Instant::now();
            last_view_change_at = tip_stalled_since;
        }
        // A quiet chain has no stalled proposal. Start the timeout clock when
        // finalized work or a live PBFT round first appears, so an idle node
        // cannot consume all views before the next real package arrives.
        if finalized_tx::pending_buffer_depth() == 0 && !active_round && !certified_view {
            tip_stalled_since = std::time::Instant::now();
            last_view_change_at = tip_stalled_since;
            continue;
        }
        if tip_stalled_since.elapsed() < Duration::from_secs(timeout)
            || last_view_change_at.elapsed() < Duration::from_secs(timeout)
        {
            continue;
        }
        let Some(privkey_hex) = privkey else { continue };
        let height = tip.0.saturating_add(1);
        let authority = match crate::validator_set_history::verification_set_at(
            &data_dir, height, &bootstrap,
        ) {
            Ok(authority) => authority,
            Err(error) => {
                tracing::warn!(height, error = %error, "Cannot start view change without authenticated validator authority");
                last_view_change_at = std::time::Instant::now();
                continue;
            }
        };
        let pbft_authority: consensus::ValidatorSet = authority.clone().into();
        if !authority.validators.iter().any(|validator| {
            validator.id == node_id && common::validator_status_is_active(&validator.status)
        }) {
            last_view_change_at = std::time::Instant::now();
            continue;
        }
        let Ok(pk_bytes) = hex::decode(privkey_hex.trim_start_matches("0x")) else {
            tracing::warn!("Skipping view-change broadcast: validator key is not valid hex");
            last_view_change_at = std::time::Instant::now();
            continue;
        };
        let Ok(sk) = common::crypto::SigningKey::from_seed(
            common::crypto::SignatureScheme::Ed25519,
            &pk_bytes,
        ) else {
            tracing::warn!("Skipping view-change broadcast: validator key is not 32 bytes");
            last_view_change_at = std::time::Instant::now();
            continue;
        };
        let local_vote = {
            let mut s = state.write().await;
            if s.chain.tip_height().ok() != Some(tip.0)
                || s.chain.tip_hash().ok().as_deref() != Some(tip.1.as_str())
            {
                continue;
            }
            let _ = s.pbft_engine.cleanup_stale_rounds();
            let Some(signal) = s
                .pbft_engine
                .request_view_change_for_tip(height, &tip.1, &node_id)
            else {
                last_view_change_at = std::time::Instant::now();
                continue;
            };
            let mut vote = signal.vote;
            let message = vote.signing_message();
            vote.signature = sk.sign(message.as_bytes()).to_hex();
            let result = s
                .chain
                .pbft_safety_journal()
                .and_then(|journal| {
                    journal.record_local_view_change(
                        &vote,
                        &pbft_authority,
                        s.pbft_engine.allow_small_cluster_quorum(),
                    )
                })
                .and_then(|_| {
                    s.pbft_engine
                        .receive_view_change(vote.clone(), &pbft_authority)
                });
            (vote, result)
        };
        let (vote, result) = local_vote;
        match result {
            Ok(Some(certificate)) => tracing::warn!(
                height,
                view = vote.new_view,
                votes = certificate.len(),
                "[PBFT] Local timeout vote completed a certified view change"
            ),
            Ok(None) => tracing::info!(
                height,
                view = vote.new_view,
                "[PBFT] Broadcast local view-change vote; waiting for quorum"
            ),
            Err(error) => {
                tracing::warn!(height, error = %error, "Rejected local view-change vote");
                last_view_change_at = std::time::Instant::now();
                continue;
            }
        }
        let gossip = common::GossipMessage::ViewChange { vote };
        if let Ok(data) = serde_json::to_vec(&gossip) {
            let _ = p2p_handle
                .sender
                .send(crate::p2p::P2PCommand::Broadcast {
                    topic: crate::p2p::VIEW_CHANGE_TOPIC.into(),
                    data,
                })
                .await;
        }
        last_view_change_at = std::time::Instant::now();
    }
}

/// Re-publish our own PBFT votes for stuck rounds. Gossipsub delivery is
/// best-effort: votes sent while the mesh converges can be silently lost,
/// stranding the round below quorum forever. Only rounds whose phase age
/// already exceeds the phase timeout are retried, so healthy operation sends
/// no extra traffic; each retry carries just our highest-phase vote.
async fn drive_pbft_vote_rebroadcast(
    state: Arc<RwLock<NodeState>>,
    p2p_handle: crate::p2p::P2PHandle,
) {
    let mut ticker = interval(Duration::from_secs(10));
    loop {
        ticker.tick().await;
        let (votes, node_id) = {
            let s = state.read().await;
            let votes = s.pbft_engine.own_votes_for_rebroadcast(&s.config.node_id);
            (votes, s.config.node_id.clone())
        };
        for vote in votes {
            if vote.phase_age_seconds <= vote.phase_timeout_seconds {
                continue;
            }
            let (phase, signature) = if let Some(commit) = vote.commit_sig.as_ref() {
                (crate::pbft_safety::PbftVotePhase::Commit, &commit.signature)
            } else if let Some(prepare) = vote.prepare_sig.as_ref() {
                (
                    crate::pbft_safety::PbftVotePhase::Prepare,
                    &prepare.signature,
                )
            } else {
                continue;
            };
            let phase_name = match phase {
                crate::pbft_safety::PbftVotePhase::Prepare => "prepare",
                crate::pbft_safety::PbftVotePhase::Commit => "commit",
                crate::pbft_safety::PbftVotePhase::ViewChange => continue,
            };
            let signing_message =
                consensus::pbft::pbft_signature_message(phase_name, &vote.block_hash);
            let durable = {
                let s = state.read().await;
                s.chain.pbft_safety_journal().and_then(|journal| {
                    journal.record_decision(
                        phase,
                        vote.block_height,
                        &vote.prev_hash,
                        vote.view_number,
                        &vote.block_hash,
                        &signing_message,
                        signature,
                    )
                })
            };
            if let Err(error) = durable {
                tracing::error!(block_hash = %vote.block_hash, error = %error, "PBFT vote rebroadcast suppressed because durable lock failed");
                continue;
            }
            let gossip = if phase == crate::pbft_safety::PbftVotePhase::Commit {
                common::GossipMessage::PbftCommit {
                    block_hash: vote.block_hash.clone(),
                    validator_id: node_id.clone(),
                    signature: signature.clone(),
                }
            } else {
                common::GossipMessage::PbftPrepare {
                    block_hash: vote.block_hash.clone(),
                    validator_id: node_id.clone(),
                    signature: signature.clone(),
                }
            };
            if let Ok(data) = serde_json::to_vec(&gossip) {
                tracing::info!(
                    "[PBFT] Rebroadcasting own vote for stuck round {} at height {}",
                    &vote.block_hash[..vote.block_hash.len().min(12)],
                    vote.block_height
                );
                let _ = p2p_handle
                    .sender
                    .send(crate::p2p::P2PCommand::Broadcast {
                        topic: "creg/v1/blocks".into(),
                        data,
                    })
                    .await;
            }
        }
    }
}

pub async fn run(
    state: Arc<RwLock<NodeState>>,
    rx: finalized_tx::FinalizedTxReceiver,
    p2p_handle: crate::p2p::P2PHandle,
) {
    let block_interval = {
        let s = state.read().await;
        s.config.block_interval_secs
    };

    let mut ticker = interval(Duration::from_secs(block_interval));
    tracing::info!(
        "Block producer started (interval: {}s; PBFT view-change quorum required for backup leadership)",
        block_interval
    );

    if PBFT_VIEW_CHANGE_READY {
        let timeout_state = Arc::clone(&state);
        let timeout_p2p = p2p_handle.clone();
        tokio::spawn(drive_pbft_view_changes(timeout_state, timeout_p2p));
    } else {
        tracing::warn!("PBFT replacement leadership disabled pending durable safety recovery");
    }

    {
        let rebroadcast_state = Arc::clone(&state);
        let rebroadcast_p2p = p2p_handle.clone();
        tokio::spawn(drive_pbft_vote_rebroadcast(
            rebroadcast_state,
            rebroadcast_p2p,
        ));
    }

    let mut pending_txs = Vec::new();
    hydrate_ready_work(&state, &mut pending_txs).await;

    loop {
        ticker.tick().await;

        // Always move channel deliveries into the backpressured buffer; never
        // drop because this node is not the current proposer.
        finalized_tx::recv_into_buffer(&rx, &mut pending_txs).await;
        // Refill only the available buffer capacity from durable storage.  This
        // is repeated every tick so a restart never materializes the entire
        // queue in memory, and work delayed by retry backoff becomes eligible
        // without a process restart.
        hydrate_ready_work(&state, &mut pending_txs).await;
        // Retire terminal authorized publishes before the proposer gate so
        // backup validators do not hold invalid durable work indefinitely.
        // The helper leaves the queue alone while a prepared/live PBFT value
        // or durable signing lock protects the next height.
        let retired = {
            let mut s = state.write().await;
            retire_ineligible_unprepared_work(&mut s, &mut pending_txs)
        };
        match retired {
            Ok(count) if count > 0 => {
                tracing::info!(
                    count,
                    "Retired terminal unprepared work before proposer selection"
                );
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    "Deferred terminal queue cleanup without discarding work: {error:#}"
                );
                continue;
            }
        }
        let (certified_view, prepared_view_blocked) = {
            let s = state.read().await;
            let next_height = s.chain.tip_height().unwrap_or(0).saturating_add(1);
            let prev_hash = s.chain.tip_hash().unwrap_or_default();
            let view = s.pbft_engine.certified_view(next_height, &prev_hash);
            let prepared = view
                .as_ref()
                .is_some_and(|(_, _, prepared)| prepared.is_some());
            (view.is_some(), prepared)
        };
        if prepared_view_blocked && !PBFT_VIEW_CHANGE_READY {
            // The current block format changes the hash when a replacement
            // proposer changes the view/header. Keep durable work available
            // until exact-block reproposal is implemented.
            tracing::debug!(
                "Block producer: prepared view is safety-blocked; keeping finalized work buffered"
            );
            continue;
        }
        // The replacement proposer may not hold the old primary's finalized
        // work locally. A certified view must still allow a prepared value to
        // be carried forward, or an empty block when nothing was prepared.
        if pending_txs.is_empty() && !certified_view {
            tracing::debug!("Block producer: no pending finalized transactions");
            continue;
        }

        // Proposal guard: never start a second proposal while a round for the
        // next height is still live. Without this the producer mints a fresh,
        // different block every tick until one seals, and peers can finalize
        // different proposals at the same height.
        {
            let s = state.read().await;
            let next_height = s.chain.tip_height().unwrap_or(0).saturating_add(1);
            if let Some((hash, phase)) = s.pbft_engine.non_terminal_round_at_height(next_height) {
                let parent = s.chain.tip_hash().unwrap_or_default();
                let authorized_reproposal = PBFT_VIEW_CHANGE_READY
                    && s.pbft_engine
                        .certified_view(next_height, &parent)
                        .is_some_and(|(view, _, prepared)| {
                            prepared.is_some_and(|prepared| {
                                prepared.block().hash() == hash
                                    && view > prepared.prepared_view
                                    && s.pbft_engine
                                        .round_context(&hash)
                                        .is_some_and(|(_, _, current_view)| current_view < view)
                            })
                        });
                tracing::debug!(
                    "Block producer: round {} already live for height {} ({})",
                    &hash[..hash.len().min(12)],
                    next_height,
                    phase.as_str()
                );
                if !authorized_reproposal {
                    continue;
                }
            }
        }

        let ctx = match prepare_proposer_context(Arc::clone(&state), &p2p_handle, 0).await {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::debug!(
                    pending = pending_txs.len(),
                    "Block producer: not proposer this tick — keeping {} buffered tx(s): {}",
                    pending_txs.len(),
                    e
                );
                continue;
            }
        };

        let reproposal = ctx.reproposal.clone();

        let prepared_transactions = ctx.prepared_transactions.clone();
        let carrying_prepared_value = prepared_transactions.is_some();
        if prepared_transactions.is_none() {
            let s = state.read().await;
            let height = s.chain.tip_height().unwrap_or(0);
            let forced: anyhow::Result<Vec<Transaction>> = s
                .forced_inclusion_tracker
                .forced_transaction_payloads(height)
                .into_iter()
                .filter_map(|tx| match s.chain.is_finalized_work_deliverable(tx) {
                    Ok(true) => Some(Ok(tx.clone())),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                })
                .collect();
            let forced = match forced {
                Ok(forced) => forced,
                Err(error) => {
                    tracing::error!("Could not read forced-work delivery eligibility: {error:#}");
                    continue;
                }
            };
            if !forced.is_empty() {
                let mut seen: HashSet<String> = pending_txs.iter().map(transaction_hash).collect();
                for tx in forced.into_iter().rev() {
                    let hash = transaction_hash(&tx);
                    if seen.insert(hash) {
                        pending_txs.insert(0, tx);
                    }
                }
            }
            drop(s);
        }

        let txs: Vec<Transaction> = if let Some(prepared) = prepared_transactions {
            let included: HashSet<String> = prepared.iter().map(transaction_hash).collect();
            pending_txs.retain(|tx| !included.contains(&transaction_hash(tx)));
            prepared
        } else {
            match prepare_unprepared_batch(&state, &mut pending_txs, &ctx).await {
                Ok(selected) => selected,
                Err(error) => {
                    tracing::warn!(
                        "Deferred unprepared selection without discarding work: {error:#}"
                    );
                    continue;
                }
            }
        };
        // A certified empty view still needs its proposal to progress.
        if txs.is_empty() && !certified_view {
            continue;
        }
        finalized_tx::sync_pending_buffer_depth(pending_txs.len());
        let txs_backup = txs.clone();

        match produce_block(Arc::clone(&state), txs, p2p_handle.clone(), ctx).await {
            Ok(block) => {
                let bh = block.hash();
                tracing::info!(
                    "[PBFT] Proposer created block {} at height {} ({} tx) — starting round",
                    &bh[..bh.len().min(12)],
                    block.header.height,
                    block.transactions.len()
                );

                // Broadcast PbftPrePrepare to start the consensus round
                let msg = reproposal.unwrap_or(common::GossipMessage::PbftPrePrepare { block });
                match serde_json::to_vec(&msg) {
                    Ok(data) => {
                        let _ = p2p_handle
                            .sender
                            .send(crate::p2p::P2PCommand::Broadcast {
                                topic: "creg/v1/blocks".into(),
                                data,
                            })
                            .await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to serialize PbftPrePrepare gossip: {}", e);
                    }
                }
            }
            Err(e) => {
                if carrying_prepared_value
                    || e.downcast_ref::<StaleProposerContext>().is_some()
                    || e.downcast_ref::<PbftSafetyGate>().is_some()
                {
                    tracing::warn!(
                        pending = txs_backup.len(),
                        carrying_prepared_value,
                        error = %format!("{e:#}"),
                        "Block producer deferred; retaining work without consuming its retry budget"
                    );
                    restore_stale_context_transactions(&mut pending_txs, txs_backup);
                    continue;
                }
                tracing::error!(
                    tx_count = txs_backup.len(),
                    "Block production failed: {e:#}"
                );
                let chain = { state.read().await.chain.clone() };
                let mut retryable = Vec::new();
                for tx in txs_backup {
                    let work_id = transaction_hash(&tx);
                    match chain.record_finalized_work_failure(&work_id, &e.to_string()) {
                        Ok(FinalizedWorkFailureOutcome::DeadLettered) => {
                            state
                                .write()
                                .await
                                .forced_inclusion_tracker
                                .retire_ineligible(&work_id);
                            tracing::error!("Finalized work {} moved to dead letter", work_id)
                        }
                        // The durable record now carries a future retry
                        // deadline. Do not requeue it in memory or the
                        // backoff would be bypassed on the next tick.
                        Ok(FinalizedWorkFailureOutcome::Retrying) => {}
                        Ok(FinalizedWorkFailureOutcome::Missing) => {
                            tracing::warn!(
                                "Failed transaction {} was not durable work; retaining it in memory",
                                work_id
                            );
                            retryable.push(tx);
                        }
                        Err(record_error) => {
                            tracing::error!(
                                "Failed to record durable-work retry for {}: {}",
                                work_id,
                                record_error
                            );
                            retryable.push(tx);
                        }
                    }
                }
                finalized_tx::requeue_front(&mut pending_txs, retryable);
            }
        }
    }
}

/// Refill the producer buffer from durable work without exceeding its
/// backpressure cap.  Hash de-duplication covers the normal case where a
/// persisted item was also delivered over the best-effort notification
/// channel before the producer observed it.
async fn hydrate_ready_work(state: &Arc<RwLock<NodeState>>, buffer: &mut Vec<Transaction>) {
    let capacity = finalized_tx::PENDING_BLOCK_TX_BUFFER_CAP.saturating_sub(buffer.len());
    if capacity == 0 {
        finalized_tx::sync_pending_buffer_depth(buffer.len());
        return;
    }
    let work = {
        let s = state.read().await;
        s.chain.pending_ready_finalized_work(capacity)
    };
    match work {
        Ok(work) => {
            let mut seen: HashSet<String> = buffer.iter().map(transaction_hash).collect();
            for (_, tx) in work {
                if seen.insert(transaction_hash(&tx)) {
                    buffer.push(tx);
                }
            }
        }
        Err(error) => tracing::error!("Failed to hydrate durable finalized work: {}", error),
    }
    finalized_tx::sync_pending_buffer_depth(buffer.len());
}

/// Evaluate the authenticated PBFT primary and broadcast our proof when eligible.
/// Lower-ranked validators keep pending work buffered until a certified
/// view-change path authorizes a different leader.
async fn prepare_proposer_context(
    state: Arc<RwLock<NodeState>>,
    p2p: &crate::p2p::P2PHandle,
    _allowed_fallback_rank: usize,
) -> anyhow::Result<ProposerContext> {
    let (tip_height, prev_hash, node_id, privkey, data_dir, bootstrap, vrf_proofs, certified_view) = {
        let s = state.read().await;
        let tip_height = s.chain.tip_height()?;
        let prev_hash = s.chain.tip_hash()?;
        let node_id = s.config.node_id.clone();
        let privkey = s.config.validator_privkey.clone();
        let certified_view = s
            .pbft_engine
            .certified_view(tip_height.saturating_add(1), &prev_hash);

        (
            tip_height,
            prev_hash,
            node_id,
            privkey,
            s.config.data_dir.clone(),
            s.validator_set_bootstrap.clone(),
            s.vrf_proofs.clone(),
            certified_view,
        )
    };

    // Use the authenticated authority for this height. Computing the header
    // hash from the live set would commit node-local performance and liveness
    // values that may differ between validators and observers.
    let authority = crate::validator_set_history::verification_set_at(
        &data_dir,
        tip_height.saturating_add(1),
        &bootstrap,
    )
    .map_err(anyhow::Error::msg)?;
    let pbft_authority = authority.clone().into();
    let (view_number, view_change_certificate, prepared_certificate) =
        certified_view.unwrap_or((0, Vec::new(), None));
    let prepared_transactions = prepared_certificate
        .as_ref()
        .map(|prepared| prepared.transactions.clone());
    if view_number > 0 && !PBFT_VIEW_CHANGE_READY {
        anyhow::bail!("PBFT replacement leadership is disabled pending safety recovery");
    }
    let our_pubkey = authority
        .validators
        .iter()
        .find(|validator| validator.id == node_id)
        .map(|validator| validator.pubkey.clone());
    let validator_set_hash = common::canonical_validator_set_hash(&authority);

    let epoch_seed = prev_hash.clone();

    let mut active: Vec<consensus::vrf::VrfValidator> = authority
        .validators
        .iter()
        .filter(|validator| common::validator_status_is_active(&validator.status))
        .map(|validator| {
            let (vrf_output, vrf_proof) = vrf_proofs
                .get(&validator.id)
                .cloned()
                .map(|(output, proof)| (Some(output), Some(proof)))
                .unwrap_or((None, None));
            consensus::vrf::VrfValidator {
                id: validator.id.clone(),
                pubkey: validator.pubkey.clone(),
                vrf_output,
                vrf_proof,
            }
        })
        .collect();

    let (vrf_output, vrf_proof) = if !active.is_empty() {
        if let Some(ref privkey) = privkey {
            let (out, prf) = consensus::vrf::prove(epoch_seed.as_bytes(), privkey)?;
            for v in &mut active {
                if v.id == node_id {
                    v.vrf_output = Some(out.clone());
                    v.vrf_proof = Some(prf.clone());
                }
            }

            if let Some(pubkey) = our_pubkey.clone() {
                let gossip_msg = common::GossipMessage::VrfProof {
                    validator_id: node_id.clone(),
                    pubkey,
                    epoch_seed: epoch_seed.clone(),
                    output: out.clone(),
                    proof: prf.clone(),
                };
                if let Ok(data) = serde_json::to_vec(&gossip_msg) {
                    let _ = p2p
                        .sender
                        .send(crate::p2p::P2PCommand::Broadcast {
                            topic: "creg/v1/vrf-proofs".into(),
                            data,
                        })
                        .await;
                }
            } else {
                tracing::warn!(
                    validator_id = %node_id,
                    "Skipping VRF proof gossip: validator pubkey missing from active set"
                );
            }

            let ranking = consensus::vrf::rank_proposers(&active, &epoch_seed);
            if ranking.is_empty() {
                anyhow::bail!("No active validators to select proposer");
            }
            let authorized_rank = (view_number as usize) % ranking.len();
            match ranking.iter().position(|id| id == &node_id) {
                Some(rank) if rank == authorized_rank => {}
                Some(rank) => {
                    anyhow::bail!(
                        "Node {} is proposer rank {} but certified PBFT view {} authorizes rank {} ({})",
                        node_id,
                        rank,
                        view_number,
                        authorized_rank,
                        ranking[authorized_rank]
                    );
                }
                None => {
                    anyhow::bail!(
                        "Node {} is not in the active proposer set for this epoch",
                        node_id
                    );
                }
            }
            (Some(out), Some(prf))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let reproposal = if let Some(prepared) = &prepared_certificate {
        let block = prepared.block();
        let private_key_hex = privkey
            .as_deref()
            .context("prepared reproposal requires the replacement validator key")?;
        let key_bytes = hex::decode(private_key_hex.trim_start_matches("0x"))?;
        let signing_key = common::crypto::SigningKey::from_seed(
            common::crypto::SignatureScheme::Ed25519,
            &key_bytes,
        )?;
        let signing_message = consensus::pbft::reproposal_signing_message(
            &block.hash(),
            view_number,
            &node_id,
            &view_change_certificate,
        );
        Some(common::GossipMessage::PbftRepropose {
            block,
            target_view: view_number,
            leader_id: node_id.clone(),
            view_change_certificate: view_change_certificate.clone(),
            leader_signature: signing_key.sign(signing_message.as_bytes()).to_hex(),
        })
    } else {
        None
    };

    Ok(ProposerContext {
        tip_height,
        prev_hash,
        node_id,
        privkey,
        our_pubkey,
        validator_set_hash,
        package_authority: authority,
        pbft_authority,
        view_number,
        view_change_certificate,
        prepared_transactions,
        reproposal,
        vrf_output,
        vrf_proof,
    })
}

async fn produce_block(
    state: Arc<RwLock<NodeState>>,
    txs: Vec<Transaction>,
    p2p: crate::p2p::P2PHandle,
    ctx: ProposerContext,
) -> anyhow::Result<Block> {
    let mut s = state.write().await;
    if ctx.reproposal.is_none() {
        ensure_unprepared_context_is_current(&s, &ctx)?;
    }
    let ProposerContext {
        tip_height,
        prev_hash,
        node_id,
        privkey,
        our_pubkey: _,
        validator_set_hash,
        package_authority,
        pbft_authority,
        view_number,
        view_change_certificate,
        prepared_transactions: _,
        reproposal,
        vrf_output,
        vrf_proof,
    } = ctx;

    let current_tip_height = s.chain.tip_height()?;
    let current_tip_hash = s.chain.tip_hash()?;
    ensure_proposer_context_is_current(
        tip_height,
        &prev_hash,
        current_tip_height,
        &current_tip_hash,
    )?;
    let parent = s
        .chain
        .get_block_by_height(tip_height)?
        .ok_or_else(|| anyhow::anyhow!("canonical parent block is unavailable"))?;
    let block = if let Some(common::GossipMessage::PbftRepropose { block, .. }) = &reproposal {
        if block.header.height != tip_height.saturating_add(1)
            || block.header.prev_hash != prev_hash
            || block.header.timestamp <= parent.header.timestamp
            || block.header.merkle_root != merkle_root(&txs)
        {
            return Err(PbftSafetyGate(
                "prepared reproposal does not extend the current tip with the exact buffered value"
                    .into(),
            )
            .into());
        }
        block.clone()
    } else {
        let timestamp = crate::block_time::next_live_timestamp(parent.header.timestamp, Utc::now())
            .context("refusing to propose a block with an invalid live timestamp")?;
        let header = BlockHeader {
            height: tip_height + 1,
            prev_hash,
            merkle_root: merkle_root(&txs),
            proposer_id: node_id.clone(),
            timestamp,
            validator_set_hash,
            vrf_output,
            vrf_proof,
            view_number,
            view_change_certificate,
        };
        Block {
            header,
            transactions: txs,
            pbft_signatures: vec![],
        }
    };

    let namespace_config =
        crate::namespace_consensus::NamespaceConsensusConfig::from_node_config(&s.config);
    crate::namespace_consensus::validate_block(&s.chain, &block, &namespace_config)
        .context("refusing to propose a block with an invalid namespace transition")?;
    crate::sync::verify_block_signatures(&block, &package_authority)
        .context("refusing to propose a block without valid package approvals")?;

    // Use the same authenticated authority used to compute the header hash.
    match &reproposal {
        Some(common::GossipMessage::PbftRepropose {
            target_view,
            leader_id,
            view_change_certificate,
            leader_signature,
            ..
        }) => s.pbft_engine.repropose_prepared(
            block.clone(),
            pbft_authority,
            *target_view,
            leader_id,
            view_change_certificate.clone(),
            leader_signature,
        ),
        _ => s.pbft_engine.start_round(block.clone(), pbft_authority),
    }
    .map_err(|error| PbftSafetyGate(error.to_string()))?;

    let bh = block.hash();
    let mut prep_cmd = None;
    let mut commit_cmd = None;

    if let Some(ref privkey_hex) = privkey {
        if let Ok(pk_bytes) = hex::decode(privkey_hex) {
            if let Ok(sk) = common::crypto::SigningKey::from_seed(
                common::crypto::SignatureScheme::Ed25519,
                &pk_bytes,
            ) {
                let prep_msg_str = consensus::pbft::pbft_signature_message("prepare", &bh);
                let prep_sig = sk.sign(prep_msg_str.as_bytes()).to_hex();
                let pubkey = sk.public_key().to_hex();

                let prep_sig_obj = common::BlockSignature {
                    validator_id: node_id.clone(),
                    pubkey: pubkey.clone(),
                    signature: prep_sig.clone(),
                };

                s.chain
                    .pbft_safety_journal()
                    .and_then(|journal| {
                        journal.record_decision(
                            crate::pbft_safety::PbftVotePhase::Prepare,
                            block.header.height,
                            &block.header.prev_hash,
                            view_number,
                            &bh,
                            &prep_msg_str,
                            &prep_sig,
                        )
                    })
                    .map_err(|error| {
                        PbftSafetyGate(format!("local PREPARE could not be persisted: {error}"))
                    })?;
                let prepare_quorum_reached = s
                    .pbft_engine
                    .prepare(&bh, &node_id, prep_sig_obj)
                    .map_err(|error| PbftSafetyGate(format!("local PREPARE rejected: {error}")))?;

                let prep_msg = common::GossipMessage::PbftPrepare {
                    block_hash: bh.clone(),
                    validator_id: node_id.clone(),
                    signature: prep_sig,
                };
                if let Ok(data) = serde_json::to_vec(&prep_msg) {
                    prep_cmd = Some(crate::p2p::P2PCommand::Broadcast {
                        topic: "creg/v1/blocks".into(),
                        data,
                    });
                }

                if prepare_quorum_reached {
                    let (prepared_block, certificate) =
                        s.pbft_engine.prepared_certificate(&bh).ok_or_else(|| {
                            PbftSafetyGate("PREPARE quorum has no recoverable certificate".into())
                        })?;
                    let authority = s.pbft_engine.round_validator_set(&bh).ok_or_else(|| {
                        PbftSafetyGate("PREPARE quorum has no validator authority".into())
                    })?;
                    s.chain
                        .pbft_safety_journal()
                        .and_then(|journal| {
                            journal.record_prepared(
                                &prepared_block,
                                &certificate,
                                &authority,
                                s.pbft_engine.allow_small_cluster_quorum(),
                            )
                        })
                        .map_err(|error| {
                            PbftSafetyGate(format!(
                                "prepared value could not be persisted: {error}"
                            ))
                        })?;
                    let commit_msg_str = consensus::pbft::pbft_signature_message("commit", &bh);
                    let commit_sig = sk.sign(commit_msg_str.as_bytes()).to_hex();

                    let commit_sig_obj = common::BlockSignature {
                        validator_id: node_id.clone(),
                        pubkey,
                        signature: commit_sig.clone(),
                    };

                    s.chain
                        .pbft_safety_journal()
                        .and_then(|journal| {
                            journal.record_decision(
                                crate::pbft_safety::PbftVotePhase::Commit,
                                block.header.height,
                                &block.header.prev_hash,
                                view_number,
                                &bh,
                                &commit_msg_str,
                                &commit_sig,
                            )
                        })
                        .map_err(|error| {
                            PbftSafetyGate(format!("local COMMIT could not be persisted: {error}"))
                        })?;
                    tracing::info!(
                        event = "pbft_local_commit_durable",
                        node_id = %node_id,
                        chain_id = %s.config.chain_id,
                        genesis_hash = %s.config.network_genesis_hash,
                        height = block.header.height,
                        parent_hash = %block.header.prev_hash,
                        block_hash = %bh,
                        view_number,
                        "Local PBFT COMMIT durably persisted before consensus application or gossip"
                    );
                    #[cfg(feature = "phase-c-real-fleet")]
                    pause_after_durable_commit_if_armed(&s, &block, &bh, view_number).await?;

                    let commit_quorum_reached = s
                        .pbft_engine
                        .commit(&bh, &node_id, commit_sig_obj)
                        .map_err(|error| {
                            PbftSafetyGate(format!("local COMMIT rejected: {error}"))
                        })?;

                    let commit_msg = common::GossipMessage::PbftCommit {
                        block_hash: bh.clone(),
                        validator_id: node_id.clone(),
                        signature: commit_sig,
                    };
                    if let Ok(data) = serde_json::to_vec(&commit_msg) {
                        commit_cmd = Some(crate::p2p::P2PCommand::Broadcast {
                            topic: "creg/v1/blocks".into(),
                            data,
                        });
                    }

                    if commit_quorum_reached {
                        tracing::info!(
                            "[PBFT Proposer] Block {} finalised locally by proposer quorum",
                            &bh[..12]
                        );
                        if let Some(final_block) = s.pbft_engine.get_finalised_block(&bh) {
                            let inserted = match s.chain.insert_block_with_outcome(&final_block) {
                                Ok(outcome) => {
                                    if let Some(replaced) = outcome.replaced_hash {
                                        s.record_reorg(1, vec![replaced], outcome.hash.clone());
                                    }
                                    true
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "[PBFT Proposer] Failed to insert finalised block: {}",
                                        e
                                    );
                                    false
                                }
                            };
                            if inserted {
                                s.publisher_index.apply_block(&final_block);
                                s.on_block_committed(&final_block);
                            }
                        }
                    }
                }
            }
        }
    }

    s.vrf_proofs.clear();

    drop(s);

    if let Some(cmd) = prep_cmd {
        let _ = p2p.sender.send(cmd).await;
    }
    if let Some(cmd) = commit_cmd {
        let _ = p2p.sender.send(cmd).await;
    }

    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chain_store::ChainStore,
        config::NodeConfig,
        finalized_tx,
        p2p::{P2PCommand, P2PHandle},
        pending_pool::PendingPool,
        publisher_index::PublisherIndex,
        state::PbftEngine,
        BridgeStatus, NodeState, P2PStatus,
    };
    use chrono::Utc;
    use common::{
        AnalysisBundleRefs, ChainRecord, DeterministicRiskSummary, PackageId, PackageStatus,
        Transaction, Validator, ValidatorSet, ValidatorSignature, ValidatorVote,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    use tempfile::TempDir;

    fn sample_publish_tx(name: &str) -> Transaction {
        Transaction::Publish(ChainRecord {
            id: PackageId::new("npm", name, "1.0.0"),
            content_hash: "abc".into(),
            ipfs_cid: "bafy".into(),
            publisher_pubkey: "pk".into(),
            block_hash: "0".repeat(64),
            published_at: Utc::now(),
            validator_signatures: vec![],
            status: PackageStatus::Verified,
            ..Default::default()
        })
    }

    fn signed_publish_tx(name: &str, validators: &[(&str, &SigningKey)]) -> Transaction {
        let Transaction::Publish(mut record) = sample_publish_tx(name) else {
            unreachable!("sample publish helper returns a publish transaction");
        };
        record.content_hash = common::sha256_hex(b"proposer-test-package-bytes");
        let canonical = record.id.canonical();
        let content_hash = record.content_hash.clone();
        let model_version = "creg-detect-v1.0.0".to_string();
        let analysis_bundles = AnalysisBundleRefs {
            policy_bundle_id: "policy-v1".into(),
            feature_schema_id: "features-v1".into(),
            expert_bundle_id: "experts-v1".into(),
            embedding_model_id: "embeddings-v1".into(),
            index_epoch: "index-epoch-1".into(),
            threshold_profile_id: "thresholds-v1".into(),
            llm_prompt_profile_id: "llm-prompt-v1".into(),
            osv_snapshot_epoch: "osv-off".into(),
        };
        record.validator_signatures = validators
            .iter()
            .map(|(validator_id, signing_key)| {
                let validator_pubkey = hex::encode(signing_key.verifying_key().as_bytes());
                let evidence_digest = common::sha256_hex(
                    format!("{canonical}:{content_hash}:proposer-test-evidence").as_bytes(),
                );
                let scanner_profile =
                    common::scanner_profile_digest(&model_version, &analysis_bundles);
                let message = crate::gossip::canonical_vote_message(
                    &canonical,
                    &content_hash,
                    true,
                    &validator_pubkey,
                    &scanner_profile,
                    &evidence_digest,
                );
                ValidatorSignature {
                    validator_id: (*validator_id).into(),
                    validator_pubkey,
                    signature: hex::encode(signing_key.sign(message.as_bytes()).to_bytes()),
                    vote: ValidatorVote::Approve,
                    signed_at: Utc::now(),
                    ml_model_version: model_version.clone(),
                    analysis_bundles: analysis_bundles.clone(),
                    evidence_digest,
                    deterministic_risk: DeterministicRiskSummary::default(),
                    is_mandatory: true,
                }
            })
            .collect();
        Transaction::Publish(record)
    }

    async fn two_validator_state(
        primary_id: &str,
        secondary_id: &str,
    ) -> (Arc<RwLock<NodeState>>, TempDir, SigningKey, SigningKey) {
        let dir = TempDir::new().unwrap();
        let chain = ChainStore::open(dir.path()).unwrap();

        let primary_sk = SigningKey::generate(&mut OsRng);
        let secondary_sk = SigningKey::generate(&mut OsRng);

        let validators = vec![
            Validator {
                id: primary_id.into(),
                alias: "primary".into(),
                pubkey: hex::encode(primary_sk.verifying_key().as_bytes()),
                eth_address: "0x1111111111111111111111111111111111111111".into(),
                stake: 1000,
                reputation: 100,
                status: "online".into(),
                performance_score: 500,
                contribution_count: 0,
            },
            Validator {
                id: secondary_id.into(),
                alias: "secondary".into(),
                pubkey: hex::encode(secondary_sk.verifying_key().as_bytes()),
                eth_address: "0x2222222222222222222222222222222222222222".into(),
                stake: 900,
                reputation: 100,
                status: "online".into(),
                performance_score: 500,
                contribution_count: 0,
            },
        ];

        let state = Arc::new(RwLock::new(NodeState {
            chain,
            pending_pool: PendingPool::new(),
            publisher_index: PublisherIndex::new(),
            validator_set_bootstrap: ValidatorSet::new(validators.clone()),
            validator_set_snapshot: Arc::new(crate::state::ArcSwap::from_pointee(
                ValidatorSet::new(validators.clone()),
            )),
            validator_set: ValidatorSet::new(validators),
            package_rounds: std::collections::HashMap::new(),
            package_committees: std::collections::HashMap::new(),
            config: NodeConfig {
                node_id: primary_id.into(),
                data_dir: dir.path().to_path_buf(),
                block_interval_secs: 1,
                ..NodeConfig::default()
            },
            p2p_status: P2PStatus::default(),
            bridge_status: BridgeStatus::default(),
            vrf_proofs: std::collections::HashMap::new(),
            decryption_shares: std::collections::HashMap::new(),
            validator_registrations: std::collections::HashMap::new(),
            validator_set_sync: crate::state::ValidatorSetSyncStatus::default(),
            view_change_certs: std::collections::HashMap::new(),
            reorgs: Vec::new(),
            pbft_engine: PbftEngine::new(),
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
            publisher_stake_cache: std::collections::HashMap::new(),
            equivocation: crate::equivocation_monitor::EquivocationMonitor::disabled(),
        }));

        (state, dir, primary_sk, secondary_sk)
    }

    fn noop_p2p() -> P2PHandle {
        let (sender, _rx) = tokio::sync::mpsc::channel::<P2PCommand>(1);
        P2PHandle { sender }
    }

    async fn proposer_ranking(state: &Arc<RwLock<NodeState>>) -> anyhow::Result<(String, String)> {
        let s = state.read().await;
        let epoch_seed = s.chain.tip_hash()?;
        let active: Vec<consensus::vrf::VrfValidator> = s
            .validator_set
            .validators
            .iter()
            .map(|v| consensus::vrf::VrfValidator {
                id: v.id.clone(),
                pubkey: v.pubkey.clone(),
                vrf_output: None,
                vrf_proof: None,
            })
            .collect();
        let ranking = consensus::vrf::rank_proposers(&active, &epoch_seed);
        anyhow::ensure!(
            ranking.len() >= 2,
            "expected two validators in proposer ranking"
        );
        Ok((ranking[0].clone(), ranking[1].clone()))
    }

    fn privkey_for_id(
        id: &str,
        primary_id: &str,
        primary_sk: &SigningKey,
        secondary_id: &str,
        secondary_sk: &SigningKey,
    ) -> String {
        if id == primary_id {
            hex::encode(primary_sk.to_bytes())
        } else if id == secondary_id {
            hex::encode(secondary_sk.to_bytes())
        } else {
            panic!("unexpected validator id {id}");
        }
    }

    #[test]
    fn proposer_context_requires_the_exact_captured_tip() {
        assert!(ensure_proposer_context_is_current(7, "tip-a", 7, "tip-a").is_ok());

        let advanced = ensure_proposer_context_is_current(7, "tip-a", 8, "tip-b")
            .expect_err("an advanced tip must make the captured context stale");
        assert_eq!(advanced.captured_height, 7);
        assert_eq!(advanced.current_height, 8);

        let competing_hash = ensure_proposer_context_is_current(7, "tip-a", 7, "tip-b")
            .expect_err("a competing tip hash must make the captured context stale");
        assert_eq!(competing_hash.captured_hash, "tip-a");
        assert_eq!(competing_hash.current_hash, "tip-b");
    }

    #[test]
    fn stale_context_retry_restores_every_attempted_transaction_in_order() {
        let attempted = vec![sample_publish_tx("first"), sample_publish_tx("second")];
        let later = sample_publish_tx("later");
        let mut pending = vec![later.clone()];
        let expected_hashes = attempted
            .iter()
            .chain(std::iter::once(&later))
            .map(transaction_hash)
            .collect::<Vec<_>>();

        restore_stale_context_transactions(&mut pending, attempted);

        assert_eq!(
            pending.iter().map(transaction_hash).collect::<Vec<_>>(),
            expected_hashes
        );
    }

    #[tokio::test]
    async fn observer_cleanup_is_a_no_op_without_a_signing_journal() {
        let (state, _dir, _primary_sk, _secondary_sk) =
            two_validator_state("validator-a", "validator-b").await;
        let mut pending_txs = vec![sample_publish_tx("buffered")];

        let mut s = state.write().await;
        s.config.is_validator = false;
        // Observers never initialize a signing journal.
        assert!(s.chain.pbft_safety_journal().is_err());

        let retired = retire_ineligible_unprepared_work(&mut s, &mut pending_txs)
            .expect("an observer must not fail the cleanup tick");
        assert_eq!(retired, 0);
        assert_eq!(
            pending_txs.len(),
            1,
            "observer cleanup must keep its buffer"
        );
    }

    #[tokio::test]
    async fn validator_cleanup_still_fails_closed_without_a_signing_journal() {
        let (state, _dir, _primary_sk, _secondary_sk) =
            two_validator_state("validator-a", "validator-b").await;
        let mut pending_txs = vec![sample_publish_tx("buffered")];

        let mut s = state.write().await;
        s.config.is_validator = true;
        assert!(s.chain.pbft_safety_journal().is_err());

        let error = retire_ineligible_unprepared_work(&mut s, &mut pending_txs)
            .expect_err("a validator without a journal must not retire work");
        assert!(
            error.to_string().contains("PBFT signing is blocked"),
            "unexpected error: {error}"
        );
        assert_eq!(pending_txs.len(), 1, "failed cleanup must keep the buffer");
    }

    #[tokio::test]
    async fn non_proposer_tick_keeps_buffered_transactions() {
        let (state, _dir, primary_sk, secondary_sk) =
            two_validator_state("validator-a", "validator-b").await;
        let (rank0, rank1) = proposer_ranking(&state).await.expect("proposer ranking");
        let (tx_sender, tx_receiver) = finalized_tx::channel();
        let p2p = noop_p2p();

        tx_sender.send(sample_publish_tx("buffered")).await.unwrap();

        let mut pending_txs = Vec::new();
        finalized_tx::recv_into_buffer(&tx_receiver, &mut pending_txs).await;
        assert_eq!(pending_txs.len(), 1);

        {
            let mut s = state.write().await;
            s.config.node_id = rank1.clone();
            s.config.validator_privkey = Some(
                privkey_for_id(
                    &rank1,
                    "validator-a",
                    &primary_sk,
                    "validator-b",
                    &secondary_sk,
                )
                .into(),
            );
        }

        let gate = prepare_proposer_context(Arc::clone(&state), &p2p, 0).await;
        assert!(
            gate.is_err(),
            "rank-1 validator {rank1} should not propose on tick 0 (primary is {rank0})"
        );
        let err = gate.err().unwrap();
        assert!(
            err.to_string().contains("authorizes rank"),
            "unexpected error: {err}"
        );
        assert_eq!(pending_txs.len(), 1, "buffer must retain the transaction");
    }

    #[tokio::test]
    async fn proposer_consumes_buffer_and_advances_chain() {
        let (state, _dir, primary_sk, secondary_sk) =
            two_validator_state("validator-a", "validator-b").await;
        let (rank0, _rank1) = proposer_ranking(&state).await.expect("proposer ranking");
        let p2p = noop_p2p();

        {
            let mut s = state.write().await;
            s.config.node_id = rank0.clone();
            s.config.validator_privkey = Some(
                privkey_for_id(
                    &rank0,
                    "validator-a",
                    &primary_sk,
                    "validator-b",
                    &secondary_sk,
                )
                .into(),
            );
            let pubkey = s
                .validator_set
                .validators
                .iter()
                .find(|validator| validator.id == rank0)
                .expect("ranked proposer must belong to the test validator set")
                .pubkey
                .clone();
            let genesis_hash = s.chain.tip_hash().expect("test chain has genesis");
            s.chain
                .initialize_pbft_safety_journal(&genesis_hash, &rank0, &pubkey)
                .expect("test proposer safety journal initializes before signing");
        }

        let mut pending_txs = vec![signed_publish_tx(
            "committed",
            &[("validator-a", &primary_sk), ("validator-b", &secondary_sk)],
        )];
        let ctx = prepare_proposer_context(Arc::clone(&state), &p2p, 0)
            .await
            .unwrap_or_else(|e| {
                panic!("rank-0 validator {rank0} should be allowed to propose: {e}")
            });

        let txs: Vec<Transaction> = std::mem::take(&mut pending_txs);
        let block = produce_block(Arc::clone(&state), txs, p2p, ctx)
            .await
            .expect("produce_block should succeed for rank-0 proposer");
        assert_eq!(block.transactions.len(), 1);
        assert_eq!(block.header.height, 1);
        let genesis = common::Block::genesis();
        assert!(block.header.timestamp > genesis.header.timestamp);
        assert_eq!(block.header.timestamp.timestamp_subsec_nanos(), 0);
        assert!(pending_txs.is_empty());
    }
}
