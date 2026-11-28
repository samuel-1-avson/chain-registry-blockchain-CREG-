// crates/node/src/p2p.rs
// Real decentralized P2P layer using libp2p with rate limiting.

use anyhow::{Context, Result};
use futures::StreamExt;
use libp2p::{
    connection_limits, gossipsub, identify, identity, kad,
    multiaddr::Protocol,
    noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, Swarm,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::p2p_peer_score::{PeerAction, PeerScoreTracker};
use crate::p2p_rate_limit::{P2PRateLimitConfig, P2PRateLimiter};
use crate::p2p_subnet_filter::{SubnetConnectionFilter, DEFAULT_MAX_PEERS_PER_SUBNET};

pub const VIEW_CHANGE_TOPIC: &str = "creg/v1/view-change";

fn gossipsub_heartbeat_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(250)
    } else {
        Duration::from_secs(10)
    }
}

fn peer_id_from_multiaddr(addr: &Multiaddr) -> Option<PeerId> {
    addr.iter().find_map(|protocol| match protocol {
        Protocol::P2p(peer_id) => Some(peer_id),
        _ => None,
    })
}

pub fn parse_p2p_seed_multiaddrs(seeds: &[String]) -> (Vec<Multiaddr>, Vec<(String, String)>) {
    let mut dialable = Vec::new();
    let mut invalid = Vec::new();
    for seed in seeds {
        match seed.parse::<Multiaddr>() {
            Ok(addr) => dialable.push(addr),
            Err(error) => invalid.push((seed.clone(), error.to_string())),
        }
    }
    (dialable, invalid)
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("create P2P identity {}", path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("write P2P identity {}", path.display()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
            .with_context(|| format!("write P2P identity {}", path.display()))?;
        Ok(())
    }
}

pub(crate) fn load_or_create_p2p_identity(path: Option<&Path>) -> Result<identity::Keypair> {
    let Some(path) = path else {
        return Ok(identity::Keypair::generate_ed25519());
    };
    if path.exists() {
        let bytes =
            std::fs::read(path).with_context(|| format!("read P2P identity {}", path.display()))?;
        return identity::Keypair::from_protobuf_encoding(&bytes).map_err(|error| {
            anyhow::anyhow!("invalid P2P identity at {}: {error}", path.display())
        });
    }
    let keypair = identity::Keypair::generate_ed25519();
    let bytes = keypair
        .to_protobuf_encoding()
        .map_err(|error| anyhow::anyhow!("encode P2P identity: {error}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create P2P identity dir {}", parent.display()))?;
    }
    write_private_file(path, &bytes)?;
    Ok(keypair)
}

fn ingest_kademlia_address(
    swarm: &mut Swarm<Behaviour>,
    peer_id: PeerId,
    addr: Multiaddr,
    bootstrapped: bool,
) {
    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
    if !bootstrapped {
        if let Err(error) = swarm.behaviour_mut().kademlia.bootstrap() {
            tracing::debug!(%error, "Kademlia bootstrap deferred until a peer address is known");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoteValidationError {
    Malformed,
    UnknownValidator,
    MissingValidatorPubkey,
    ValidatorPubkeyMismatch,
    InvalidValidatorPubkeyHex,
    InvalidValidatorPubkey,
    MissingConsensusEvidence,
    InvalidSignatureHex,
    InvalidSignatureFormat,
    SignatureVerificationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PbftValidationError {
    UnknownValidator,
    MissingValidatorPubkey,
    SignatureVerificationFailed,
}

fn validate_pbft_phase_message(
    validator_set: &common::ValidatorSet,
    phase: &str,
    block_hash: &str,
    validator_id: &str,
    signature_hex: &str,
) -> Result<common::BlockSignature, PbftValidationError> {
    let validator = validator_set
        .validators
        .iter()
        .find(|validator| validator.id == validator_id)
        .ok_or(PbftValidationError::UnknownValidator)?;

    if validator.pubkey.is_empty() {
        return Err(PbftValidationError::MissingValidatorPubkey);
    }

    consensus::pbft::verify_pbft_phase_signature(
        phase,
        block_hash,
        &validator.pubkey,
        signature_hex,
    )
    .map_err(|_| PbftValidationError::SignatureVerificationFailed)?;

    Ok(common::BlockSignature {
        validator_id: validator_id.to_string(),
        pubkey: validator.pubkey.clone(),
        signature: signature_hex.to_string(),
        pq_signature: None,
    })
}

fn validate_pbft_pre_prepare_block(
    block: &common::Block,
    validator_set: &common::ValidatorSet,
    tip_height: u64,
    tip_hash: &str,
    tip_timestamp: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<()> {
    if block.header.view_number > 0 && !crate::block_producer::PBFT_VIEW_CHANGE_READY {
        anyhow::bail!("PBFT replacement leadership is disabled pending safety recovery");
    }
    if block.header.height != tip_height.saturating_add(1) {
        anyhow::bail!("PBFT proposal is not for the next canonical height");
    }
    if block.header.prev_hash != tip_hash {
        anyhow::bail!("PBFT proposal does not extend the canonical tip");
    }
    if block.header.validator_set_hash != common::canonical_validator_set_hash(validator_set) {
        anyhow::bail!("PBFT proposal commits to the wrong validator set");
    }
    if block.header.merkle_root != common::merkle_root(&block.transactions) {
        anyhow::bail!("PBFT proposal has an invalid transaction Merkle root");
    }
    if block.header.vrf_output.is_some() != block.header.vrf_proof.is_some() {
        anyhow::bail!("PBFT proposal must carry both VRF output and proof or neither");
    }
    crate::block_time::validate_live_timestamp(tip_timestamp, block.header.timestamp, now)?;
    Ok(())
}

fn report_pbft_validation_failure(
    phase: &str,
    validator_id: &str,
    block_hash: &str,
    error: PbftValidationError,
) {
    tracing::warn!(
        phase = phase,
        validator_id = validator_id,
        block_hash = block_hash,
        ?error,
        "P2P: rejected invalid PBFT gossip signature"
    );
}

/// Helper to sign a PBFT phase message (prepare/commit) using our key.
/// Returns (signature_hex, pubkey_hex). Logs on failure in caller.
fn sign_pbft_phase(
    sk: &common::crypto::SigningKey,
    phase: &str,
    block_hash: &str,
) -> (String, String) {
    let message = consensus::pbft::pbft_signature_message(phase, block_hash);
    let signature = sk.sign(message.as_bytes()).to_hex();
    let pubkey = sk.public_key().to_hex();
    (signature, pubkey)
}

async fn sign_and_record_commit(
    state: &mut crate::NodeState,
    block_hash: &str,
    validator_id: &str,
    signing_key: &common::crypto::SigningKey,
) -> anyhow::Result<(common::GossipMessage, bool)> {
    let (height, prev_hash, view_number) = state
        .pbft_engine
        .round_context(block_hash)
        .ok_or_else(|| anyhow::anyhow!("COMMIT has no PBFT round context"))?;
    let (prepared_block, certificate) = state
        .pbft_engine
        .prepared_certificate(block_hash)
        .ok_or_else(|| anyhow::anyhow!("COMMIT has no prepared certificate"))?;
    let authority = state
        .pbft_engine
        .round_validator_set(block_hash)
        .ok_or_else(|| anyhow::anyhow!("COMMIT has no validator authority"))?;
    let journal = state.chain.pbft_safety_journal()?;
    journal.record_prepared(
        &prepared_block,
        &certificate,
        &authority,
        state.pbft_engine.allow_small_cluster_quorum(),
    )?;
    let (signature, pubkey) = sign_pbft_phase(signing_key, "commit", block_hash);
    let vote = common::BlockSignature {
        validator_id: validator_id.to_owned(),
        pubkey,
        signature: signature.clone(),
        pq_signature: None,
    };
    let signing_message = consensus::pbft::pbft_signature_message("commit", block_hash);
    journal.record_decision(
        crate::pbft_safety::PbftVotePhase::Commit,
        height,
        &prev_hash,
        view_number,
        block_hash,
        &signing_message,
        &signature,
    )?;
    tracing::info!(
        event = "pbft_local_commit_durable",
        signing_path = "p2p_follower",
        node_id = %validator_id,
        chain_id = %state.config.chain_id,
        genesis_hash = %state.config.network_genesis_hash,
        height,
        parent_hash = %prev_hash,
        block_hash = %block_hash,
        view_number,
        "Local PBFT COMMIT durably persisted before consensus application or gossip"
    );
    #[cfg(feature = "phase-c-real-fleet")]
    crate::block_producer::pause_after_durable_commit_if_armed(
        state,
        &prepared_block,
        block_hash,
        view_number,
    )
    .await?;
    let finalized = state.pbft_engine.commit(block_hash, validator_id, vote)?;
    Ok((
        common::GossipMessage::PbftCommit {
            block_hash: block_hash.to_owned(),
            validator_id: validator_id.to_owned(),
            signature,
        },
        finalized,
    ))
}

fn apply_finalized_pbft_block(
    state: &mut crate::NodeState,
    block_hash: &str,
) -> anyhow::Result<()> {
    let block = state
        .pbft_engine
        .get_finalised_block(block_hash)
        .ok_or_else(|| anyhow::anyhow!("finalized PBFT round has no block"))?;
    let outcome = state.chain.insert_block_with_outcome(&block)?;
    if let Some(replaced) = outcome.replaced_hash {
        state.record_reorg(1, vec![replaced], outcome.hash.clone());
    }
    state.publisher_index.apply_block(&block);
    state.on_block_committed(&block);
    let data_dir = state.config.data_dir.clone();
    crate::intelligence::schedule_for_block(&block, data_dir);
    Ok(())
}

fn validate_vote_gossip_message(
    message: &[u8],
    validator_set: &common::ValidatorSet,
) -> Result<crate::gossip::VoteGossip, VoteValidationError> {
    use common::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};

    let vote = serde_json::from_slice::<crate::gossip::VoteGossip>(message)
        .map_err(|_| VoteValidationError::Malformed)?;

    let validator = validator_set
        .validators
        .iter()
        .find(|validator| validator.id == vote.validator_id)
        .ok_or(VoteValidationError::UnknownValidator)?;

    if validator.pubkey.is_empty() {
        return Err(VoteValidationError::MissingValidatorPubkey);
    }

    let pubkey_bytes = hex::decode(&vote.validator_pubkey)
        .map_err(|_| VoteValidationError::InvalidValidatorPubkeyHex)?;

    if vote.validator_pubkey != validator.pubkey {
        return Err(VoteValidationError::ValidatorPubkeyMismatch);
    }

    if !common::is_consensus_grade_vote(
        &vote.ml_model_version,
        &vote.analysis_bundles,
        &vote.evidence_digest,
    ) {
        return Err(VoteValidationError::MissingConsensusEvidence);
    }

    if !vote.approved
        && vote
            .reject_reason
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        // Treat reject without reason as non-grade to avoid silent fallbacks (addresses gossip signature/reason issues)
        return Err(VoteValidationError::MissingConsensusEvidence);
    }

    let verifying_key = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey_bytes)
        .map_err(|_| VoteValidationError::InvalidValidatorPubkey)?;

    let signature_bytes =
        hex::decode(&vote.signature).map_err(|_| VoteValidationError::InvalidSignatureHex)?;
    let signature = Signature::from_bytes(SignatureScheme::Ed25519, &signature_bytes)
        .map_err(|_| VoteValidationError::InvalidSignatureFormat)?;

    let scanner_profile =
        common::scanner_profile_digest(&vote.ml_model_version, &vote.analysis_bundles);
    let vote_metadata = crate::gossip::vote_metadata_digest(
        vote.is_mandatory,
        &vote.deterministic_risk,
        vote.reject_reason.as_deref(),
    );
    let message = if vote.publisher_authorization_digest.is_empty() {
        crate::gossip::canonical_vote_message(
            &vote.consensus_subject,
            &vote.content_hash,
            vote.approved,
            &vote.validator_pubkey,
            &scanner_profile,
            &vote.evidence_digest,
        )
    } else {
        crate::gossip::canonical_authorized_vote_message(
            &vote.consensus_subject,
            &vote.content_hash,
            vote.approved,
            &vote.validator_pubkey,
            &scanner_profile,
            &vote.evidence_digest,
            &vote.publisher_authorization_digest,
            &vote_metadata,
        )
    };

    verifying_key
        .verify(message.as_bytes(), &signature, VerifyMode::Strict)
        .map_err(|_| VoteValidationError::SignatureVerificationFailed)?;

    Ok(vote)
}

fn vote_gossip_to_signature(vote: &crate::gossip::VoteGossip) -> common::ValidatorSignature {
    common::ValidatorSignature {
        validator_id: vote.validator_id.clone(),
        validator_pubkey: vote.validator_pubkey.clone(),
        signature: vote.signature.clone(),
        vote: if vote.approved {
            common::ValidatorVote::Approve
        } else {
            common::ValidatorVote::Reject {
                reason: vote
                    .reject_reason
                    .clone()
                    .unwrap_or_else(|| "no reason provided".to_string()),
            }
        },
        signed_at: chrono::Utc::now(),
        ml_model_version: vote.ml_model_version.clone(),
        analysis_bundles: vote.analysis_bundles.clone(),
        evidence_digest: vote.evidence_digest.clone(),
        deterministic_risk: vote.deterministic_risk.clone(),
        is_mandatory: vote.is_mandatory,
        ..Default::default()
    }
}

async fn record_validated_vote_gossip(
    state: &crate::SharedState,
    vote: &crate::gossip::VoteGossip,
) {
    let mut state = state.write().await;
    state.record_package_vote(
        vote.consensus_subject.clone(),
        vote_gossip_to_signature(vote),
    );
    // The certificate signature and the v4 envelope are self-authenticating
    // and are verified when the certificate is assembled or the votes are
    // aggregated; here they only have to belong to the voter.
    if let Some(signature) = vote
        .committee_qc_signature
        .as_ref()
        .filter(|signature| signature.validator_id == vote.validator_id)
    {
        let domain = vote
            .committee_qc_domain
            .clone()
            .unwrap_or_else(|| crate::committee_qc::vote_qc_domain(vote.approved).to_string());
        state.record_package_qc_signature(
            &vote.consensus_subject,
            crate::committee_qc::QcVoteSignature {
                domain,
                content_hash: vote.content_hash.clone(),
                evidence_digest: vote.evidence_digest.clone(),
                signature: signature.clone(),
            },
        );
    }
    if let Some(envelope) = vote
        .vote_v4
        .as_ref()
        .filter(|envelope| envelope.validator_id == vote.validator_id)
    {
        state.record_package_v4_vote(&vote.consensus_subject, envelope.clone());
    }
}

fn report_validation_result(
    swarm: &mut Swarm<Behaviour>,
    message_id: &gossipsub::MessageId,
    propagation_source: &PeerId,
    acceptance: gossipsub::MessageAcceptance,
) {
    if !swarm
        .behaviour_mut()
        .gossipsub
        .report_message_validation_result(message_id, propagation_source, acceptance)
    {
        tracing::warn!(
            message_id = %message_id,
            peer = %propagation_source,
            "P2P: gossipsub message not found when reporting validation result"
        );
    }
}

pub(crate) fn gossipsub_message_id(topic: &str, data: &[u8]) -> gossipsub::MessageId {
    let mut preimage = Vec::with_capacity(topic.len() + 1 + data.len());
    preimage.extend_from_slice(topic.as_bytes());
    preimage.push(0u8);
    preimage.extend_from_slice(data);
    gossipsub::MessageId::from(common::sha256(&preimage).to_vec())
}

fn is_loopback_multiaddr(addr: &Multiaddr) -> bool {
    addr.iter().any(|proto| match proto {
        Protocol::Ip4(ip) => ip.is_loopback(),
        Protocol::Ip6(ip) => ip.is_loopback(),
        _ => false,
    })
}

fn handle_peer_action(swarm: &mut Swarm<Behaviour>, peer_id: PeerId, action: PeerAction) {
    if action == PeerAction::Blacklist {
        let _ = swarm.disconnect_peer_id(peer_id);
    }
}

fn requires_payload_validation_before_acceptance(_topic: &str) -> bool {
    true
}

fn transition_persistence_failure_acceptance(
    error: &anyhow::Error,
) -> gossipsub::MessageAcceptance {
    if error.chain().any(|cause| {
        cause
            .downcast_ref::<crate::validator_set_history::RecordAuthenticatedError>()
            .is_some_and(crate::validator_set_history::RecordAuthenticatedError::is_persistence)
            || cause.downcast_ref::<std::io::Error>().is_some()
    }) {
        // A receiver-side read, retention, serialization, or filesystem
        // failure is not evidence that the sender supplied an invalid
        // transition, so do not penalize the peer.
        gossipsub::MessageAcceptance::Ignore
    } else {
        // Certificate, continuity, replay, and same-height conflict failures
        // are attributable to the received payload.
        gossipsub::MessageAcceptance::Reject
    }
}

fn validate_and_persist_transition(
    data_dir: &std::path::Path,
    snapshot: common::AuthenticatedValidatorSetSnapshot,
    trusted_bootstrap: &common::ValidatorSet,
    tip_height: u64,
    tip_hash: &str,
    checkpoint: Option<&common::AuthenticatedValidatorSetSnapshot>,
) -> anyhow::Result<gossipsub::MessageAcceptance> {
    let mut is_configured_checkpoint = false;
    let mut exact_checkpoint_acceptance = gossipsub::MessageAcceptance::Accept;
    if let Some(checkpoint) = checkpoint {
        is_configured_checkpoint = common::validator_snapshot_certificate_hash(&snapshot)
            == common::validator_snapshot_certificate_hash(checkpoint);
        if is_configured_checkpoint
            && (common::validator_snapshot_witness_hash(&snapshot)
                != common::validator_snapshot_witness_hash(checkpoint)
                || snapshot.validate_structure().is_err())
        {
            // Logical checkpoint identity deliberately excludes its witness.
            // Always install and propagate only the independently configured
            // witness bytes. Ignore an alternate or structurally inconsistent
            // original so gossipsub cannot relay attacker-chosen checkpoint
            // bytes.
            exact_checkpoint_acceptance = gossipsub::MessageAcceptance::Ignore;
        }
        let descends_from_checkpoint = is_configured_checkpoint
            || crate::validator_set_history::authenticated_at_from_checkpoint_checked(
                data_dir,
                snapshot.effective_from_height.saturating_sub(1),
                checkpoint,
            )
            .map_err(crate::validator_set_history::RecordAuthenticatedError::persistence)?
            .is_some();
        if !descends_from_checkpoint {
            anyhow::bail!("validator-set transition is outside the configured checkpoint history");
        }
    }
    if snapshot.effective_from_height <= tip_height {
        if let Some(existing) =
            crate::validator_set_history::authenticated_at(data_dir, snapshot.effective_from_height)
        {
            if existing.effective_from_height == snapshot.effective_from_height
                && common::validator_snapshot_certificate_hash(&existing)
                    == common::validator_snapshot_certificate_hash(&snapshot)
            {
                return Ok(gossipsub::MessageAcceptance::Ignore);
            }
        }
    }
    crate::validator_set_history::validate_transition_at_tip(&snapshot, tip_height, tip_hash)?;
    if is_configured_checkpoint {
        crate::validator_set_history::record_configured_checkpoint(data_dir, &snapshot)?;
    } else {
        let previous_height = snapshot.effective_from_height.saturating_sub(1);
        let previous = match checkpoint {
            Some(checkpoint) => {
                crate::validator_set_history::authenticated_at_from_checkpoint_checked(
                    data_dir,
                    previous_height,
                    checkpoint,
                )
                .map_err(crate::validator_set_history::RecordAuthenticatedError::persistence)?
                .map(|certificate| certificate.validators)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "validator-set transition has no configured-checkpoint-descended prior authority"
                    )
                })?
            }
            None => crate::validator_set_history::authenticated_at_checked(
                data_dir,
                previous_height,
                trusted_bootstrap,
            )
            .map_err(crate::validator_set_history::RecordAuthenticatedError::persistence)?
            .map(|certificate| certificate.validators)
            // With no prior certificate this is the first transition and only
            // the deployment bootstrap set may authorize it.
            .unwrap_or_else(|| trusted_bootstrap.clone()),
        };
        crate::validator_set_history::record_authenticated(data_dir, snapshot, &previous)?;
    }
    Ok(exact_checkpoint_acceptance)
}

/// Combined P2P behaviour.
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub identify: identify::Behaviour,
    pub connection_limits: connection_limits::Behaviour,
}

pub struct P2PNode {
    pub swarm: Swarm<Behaviour>,
    pub peer_id: PeerId,
    pub receiver: mpsc::Receiver<P2PCommand>,
    pub rate_limiter: P2PRateLimiter,
}

#[derive(Clone)]
pub struct P2PHandle {
    pub sender: mpsc::Sender<P2PCommand>,
}

pub enum P2PCommand {
    Broadcast { topic: String, data: Vec<u8> },
    Dial { addr: Multiaddr },
    IdentifyStorage { cid: String },
}

impl P2PNode {
    pub fn new(listen_addr: &str) -> Result<(Self, P2PHandle)> {
        Self::with_identity(listen_addr, None)
    }

    pub fn with_identity(
        listen_addr: &str,
        identity_path: Option<&Path>,
    ) -> Result<(Self, P2PHandle)> {
        let (sender, receiver) = mpsc::channel(100);
        let keypair = load_or_create_p2p_identity(identity_path)?;
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_dns()?
            .with_behaviour(|key| {
                // ── Gossipsub ────────────────────────────────────────────────
                let message_id_fn = |message: &gossipsub::Message| {
                    gossipsub_message_id(message.topic.as_str(), &message.data)
                };

                let gossipsub_config = gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(gossipsub_heartbeat_interval())
                    .validate_messages()
                    .validation_mode(gossipsub::ValidationMode::Strict)
                    .max_transmit_size(4 * 1024 * 1024)
                    .message_id_fn(message_id_fn)
                    .build()
                    .map_err(std::io::Error::other)?;

                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )?;

                // ── Kademlia ─────────────────────────────────────────────────
                let peer_id = key.public().to_peer_id();
                let store = kad::store::MemoryStore::new(peer_id);
                let mut kademlia = kad::Behaviour::new(peer_id, store);
                kademlia.set_mode(Some(kad::Mode::Server));

                // ── Identify ─────────────────────────────────────────────────
                let identify = identify::Behaviour::new(identify::Config::new(
                    "/creg/1.0.0".into(),
                    key.public(),
                ));

                let connection_limits = connection_limits::Behaviour::new(
                    connection_limits::ConnectionLimits::default()
                        .with_max_pending_incoming(Some(64))
                        .with_max_pending_outgoing(Some(64))
                        .with_max_established_incoming(Some(128))
                        .with_max_established_outgoing(Some(128))
                        .with_max_established_per_peer(Some(2)),
                );

                Ok(Behaviour {
                    gossipsub,
                    kademlia,
                    identify,
                    connection_limits,
                })
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        let peer_id = *swarm.local_peer_id();
        swarm.listen_on(listen_addr.parse()?)?;

        // Auto-ban stays off unless explicitly enabled: the limiter cannot
        // reliably recognize validators yet, and auto-ban escalated into
        // banning honest validators and stalling consensus on testnet.
        let rate_limit_config = P2PRateLimitConfig {
            auto_ban: matches!(
                std::env::var("CREG_P2P_AUTO_BAN")
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "1" | "true" | "yes" | "on"
            ),
            ..P2PRateLimitConfig::default()
        };
        let rate_limiter = P2PRateLimiter::new(rate_limit_config);

        Ok((
            Self {
                swarm,
                peer_id,
                receiver,
                rate_limiter,
            },
            P2PHandle { sender },
        ))
    }

    pub async fn run(
        mut self,
        state: crate::SharedState,
        _event_bus: crate::events::EventBus,
        admission_store: Arc<crate::consensus_admission::AttestationStore>,
    ) -> Result<()> {
        crate::p2p_rate_limit::spawn_purge_task(self.rate_limiter.clone());
        let validator_set_snapshot = state.read().await.validator_set_snapshot.clone();
        let mut peer_scores = PeerScoreTracker::default();
        let mut peer_validators: HashMap<PeerId, String> = HashMap::new();
        let max_per_subnet = std::env::var("CREG_P2P_MAX_PEERS_PER_SUBNET")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_PEERS_PER_SUBNET);
        let mut subnet_filter = SubnetConnectionFilter::new(max_per_subnet);
        let mut status_ticker = tokio::time::interval(Duration::from_secs(5));
        let votes_topic = gossipsub::IdentTopic::new("creg/v1/votes");
        let blocks_topic = gossipsub::IdentTopic::new("creg/v1/blocks");
        let submissions_topic = gossipsub::IdentTopic::new("creg/v1/submissions");
        let vrf_proofs_topic = gossipsub::IdentTopic::new("creg/v1/vrf-proofs");
        let validators_topic =
            gossipsub::IdentTopic::new(crate::validator_registry_gossip::REGISTRATION_TOPIC);
        let validator_set_transitions_topic = gossipsub::IdentTopic::new(
            crate::validator_registry_gossip::VALIDATOR_SET_TRANSITIONS_TOPIC,
        );
        let view_change_topic = gossipsub::IdentTopic::new(VIEW_CHANGE_TOPIC);
        let mut kad_bootstrapped = false;

        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&votes_topic)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&blocks_topic)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&submissions_topic)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&vrf_proofs_topic)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&validators_topic)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&validator_set_transitions_topic)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&view_change_topic)?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&gossipsub::IdentTopic::new(crate::revocation_gossip::TOPIC))?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&gossipsub::IdentTopic::new(
                crate::consensus_admission::TOPIC,
            ))?;
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&gossipsub::IdentTopic::new(
                crate::consensus_admission::TRANSITION_TOPIC,
            ))?;

        // Offload heavy IPFS download & YARA scanning for prechecked P2P submissions
        // off the single-threaded libp2p Swarm event loop (VNA-03).
        const MAX_P2P_SUBMISSION_QUEUE: usize = 64;
        const MAX_CONCURRENT_P2P_ADMISSIONS: usize = 4;
        let (sub_tx, mut sub_rx) =
            tokio::sync::mpsc::channel::<common::PublishRequest>(MAX_P2P_SUBMISSION_QUEUE);
        {
            let adm_state = Arc::clone(&state);
            let adm_sem = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_P2P_ADMISSIONS));
            tokio::spawn(async move {
                while let Some(req) = sub_rx.recv().await {
                    let Ok(permit) = Arc::clone(&adm_sem).acquire_owned().await else {
                        break;
                    };
                    let st = Arc::clone(&adm_state);
                    tokio::spawn(async move {
                        let _permit = permit;
                        let canonical = req.id.canonical();
                        match crate::package_admission::admit_publish_request(
                            &st,
                            req,
                            crate::package_admission::AdmissionOptions {
                                surface: crate::package_admission::AdmissionSurface::P2p,
                                verify_publisher_auth: true,
                            },
                        )
                        .await
                        {
                            Ok(_) => {
                                tracing::info!("P2P submission {} admitted to mempool", canonical)
                            }
                            Err(crate::package_admission::AdmissionError::AlreadyPending(_))
                            | Err(crate::package_admission::AdmissionError::AlreadyVerified(_)) => {
                                tracing::debug!("P2P submission {} already known", canonical);
                            }
                            Err(err) => {
                                tracing::warn!(
                                    "P2P submission {} rejected at admission: {}",
                                    canonical,
                                    err
                                );
                            }
                        }
                    });
                }
            });
        }

        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => match event {
                    SwarmEvent::Behaviour(BehaviourEvent::Kademlia(
                        kad::Event::OutboundQueryProgressed {
                            result: kad::QueryResult::Bootstrap(Ok(_)),
                            ..
                        },
                    )) => {
                        kad_bootstrapped = true;
                        tracing::info!("Kademlia bootstrap successful");
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Kademlia(
                        kad::Event::OutboundQueryProgressed { .. },
                    )) => {}
                    SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                        tracing::debug!(%peer_id, addrs = info.listen_addrs.len(), "Identify received");
                        for addr in info.listen_addrs {
                            ingest_kademlia_address(
                                &mut self.swarm,
                                peer_id,
                                addr,
                                kad_bootstrapped,
                            );
                        }
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
                        propagation_source: peer_id,
                        message_id: id,
                        message,
                    })) => {
                        tracing::debug!("Got Gossipsub message {} from {}", id, peer_id);

                        if !peer_scores.is_allowed(&peer_id) {
                            report_validation_result(
                                &mut self.swarm,
                                &id,
                                &peer_id,
                                gossipsub::MessageAcceptance::Ignore,
                            );
                            continue;
                        }

                        // Parse topic and check rate limits
                        let topic_str = message.topic.as_str();

                        // Retrieve active validators and their stakes lock-free
                        let vs_snapshot = validator_set_snapshot.load();
                        let active_validators: Vec<(String, u64)> = vs_snapshot
                            .validators
                            .iter()
                            .map(|v| (v.id.clone(), v.stake))
                            .collect();

                        // Apply rate limiting based on message type & authenticated identity
                        let validator_hint = peer_validators.get(&peer_id).map(String::as_str);
                        let allowed = if topic_str.contains("votes") {
                            self.rate_limiter.check_vote_for_validator(
                                peer_id,
                                validator_hint,
                                &active_validators,
                            )
                        } else if topic_str.contains("blocks") {
                            self.rate_limiter.check_block_for_validator(
                                peer_id,
                                validator_hint,
                                &active_validators,
                            )
                        } else {
                            self.rate_limiter.check_general_for_validator(
                                peer_id,
                                validator_hint,
                                &active_validators,
                            )
                        };

                        if !allowed {
                            report_validation_result(
                                &mut self.swarm,
                                &id,
                                &peer_id,
                                gossipsub::MessageAcceptance::Ignore,
                            );
                            let action = peer_scores.record_rate_limit_violation(&peer_id);
                            handle_peer_action(&mut self.swarm, peer_id, action);
                            tracing::warn!(
                                "P2P Rate limit: Dropping message {} from {} on topic {}",
                                id, peer_id, topic_str
                            );
                            continue;
                        }

                        let max_message_bytes: usize =
                            if topic_str == crate::consensus_admission::TOPIC {
                                crate::consensus_admission::MAX_GOSSIP_BYTES
                            } else if topic_str == crate::consensus_admission::TRANSITION_TOPIC {
                                crate::consensus_admission::MAX_TRANSITION_GOSSIP_BYTES
                            } else if topic_str.contains("blocks") || topic_str.contains("view-change") {
                                4 * 1024 * 1024
                            } else {
                                1024 * 1024
                            };
                        if message.data.len() > max_message_bytes {
                            report_validation_result(
                                &mut self.swarm,
                                &id,
                                &peer_id,
                                gossipsub::MessageAcceptance::Ignore,
                            );
                            tracing::warn!(
                                "P2P: Dropping oversized message {} from {} ({} bytes > {} limit)",
                                id, peer_id, message.data.len(), max_message_bytes
                            );
                            continue;
                        }
                        let _ = requires_payload_validation_before_acceptance(topic_str);

                        if topic_str == crate::consensus_admission::TOPIC {
                            let acceptance = match crate::consensus_admission::accept_gossip(
                                &admission_store, &state, &message.data,
                            ).await {
                                Ok(true) => gossipsub::MessageAcceptance::Accept,
                                Ok(false) => gossipsub::MessageAcceptance::Ignore,
                                Err(_) => gossipsub::MessageAcceptance::Reject,
                            };
                            report_validation_result(&mut self.swarm, &id, &peer_id, acceptance);
                            continue;
                        }
                        if topic_str == crate::consensus_admission::TRANSITION_TOPIC {
                            let acceptance = match crate::consensus_admission::accept_transition_gossip(
                                &admission_store, &state, &message.data,
                            ).await {
                                Ok(true) => gossipsub::MessageAcceptance::Accept,
                                Ok(false) => gossipsub::MessageAcceptance::Ignore,
                                Err(_) => gossipsub::MessageAcceptance::Reject,
                            };
                            report_validation_result(&mut self.swarm, &id, &peer_id, acceptance);
                            continue;
                        }

                        // Forward message to the node's internal event bus
                        if topic_str.contains("submissions") {
                            match serde_json::from_slice::<common::GossipMessage>(&message.data) {
                                Ok(common::GossipMessage::PublishRequest(req)) => {
                                    let canonical = req.id.canonical();
                                    match crate::package_admission::precheck_p2p_submission(&state, &req).await {
                                        crate::package_admission::P2pSubmissionPrecheck::Valid => {
                                            let acceptance = match sub_tx.try_send(req) {
                                                Ok(()) => {
                                                    peer_scores.record_valid_message(&peer_id);
                                                    gossipsub::MessageAcceptance::Accept
                                                }
                                                Err(_) => {
                                                    tracing::warn!("P2P: admission queue full, ignoring {}", canonical);
                                                    gossipsub::MessageAcceptance::Ignore
                                                }
                                            };
                                            report_validation_result(&mut self.swarm, &id, &peer_id, acceptance);
                                        }
                                        crate::package_admission::P2pSubmissionPrecheck::DuplicateIgnore => {
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Ignore,
                                            );
                                        }
                                        crate::package_admission::P2pSubmissionPrecheck::Reject(reason) => {
                                            tracing::warn!(peer = %peer_id, canonical = %canonical, reason = %reason, "P2P: rejecting invalid submission before acceptance");
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            let action = peer_scores.record_corrupted_payload(&peer_id);
                                            handle_peer_action(&mut self.swarm, peer_id, action);
                                        }
                                    }
                                }
                                _ => {
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        gossipsub::MessageAcceptance::Reject,
                                    );
                                    let action = peer_scores.record_corrupted_payload(&peer_id);
                                    handle_peer_action(&mut self.swarm, peer_id, action);
                                }
                            }
                            continue;
                        }

                        if topic_str.contains("vrf-proofs") {
                            match serde_json::from_slice::<common::GossipMessage>(&message.data) {
                                Ok(common::GossipMessage::VrfProof {
                                    validator_id,
                                    pubkey,
                                    epoch_seed,
                                    output,
                                    proof,
                                }) => {
                                    let member_matches = vs_snapshot.validators.iter().any(|v| {
                                        v.id == validator_id
                                            && common::validator_status_is_active(&v.status)
                                            && v.pubkey.eq_ignore_ascii_case(&pubkey)
                                    });
                                    if !member_matches {
                                        report_validation_result(
                                            &mut self.swarm,
                                            &id,
                                            &peer_id,
                                            gossipsub::MessageAcceptance::Reject,
                                        );
                                        let action = peer_scores.record_invalid_signature(&peer_id);
                                        handle_peer_action(&mut self.swarm, peer_id, action);
                                        continue;
                                    }
                                    let mut s = state.write().await;
                                    let current_seed = match s.chain.tip_hash() {
                                        Ok(h) => h,
                                        Err(_) => {
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Ignore,
                                            );
                                            continue;
                                        }
                                    };
                                    if epoch_seed != current_seed {
                                        report_validation_result(
                                            &mut self.swarm,
                                            &id,
                                            &peer_id,
                                            gossipsub::MessageAcceptance::Ignore,
                                        );
                                        continue;
                                    }
                                    if let Err(e) = consensus::vrf::verify(
                                        epoch_seed.as_bytes(),
                                        &pubkey,
                                        &output,
                                        &proof,
                                    ) {
                                        report_validation_result(
                                            &mut self.swarm,
                                            &id,
                                            &peer_id,
                                            gossipsub::MessageAcceptance::Reject,
                                        );
                                        let action = peer_scores.record_invalid_signature(&peer_id);
                                        handle_peer_action(&mut self.swarm, peer_id, action);
                                        tracing::debug!("Dropped invalid VRF proof from {}: {}", validator_id, e);
                                    } else {
                                        if s.vrf_proofs.contains_key(&validator_id)
                                            || s.vrf_proofs.len()
                                                < crate::state::MAX_VRF_PROOFS_PER_EPOCH
                                        {
                                            s.vrf_proofs
                                                .insert(validator_id.clone(), (output.clone(), proof.clone()));
                                        }
                                        if let Some(src) = message.source {
                                            peer_validators.insert(src, validator_id.clone());
                                        }
                                        peer_scores.record_valid_message(&peer_id);
                                        report_validation_result(
                                            &mut self.swarm,
                                            &id,
                                            &peer_id,
                                            gossipsub::MessageAcceptance::Accept,
                                        );
                                        tracing::debug!("Accepted VRF proof from {} for epoch {}", validator_id, &epoch_seed[..epoch_seed.len().min(12)]);
                                    }
                                }
                                _ => {
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        gossipsub::MessageAcceptance::Reject,
                                    );
                                    let action = peer_scores.record_corrupted_payload(&peer_id);
                                    handle_peer_action(&mut self.swarm, peer_id, action);
                                }
                            }
                            continue;
                        }

                        // ── Validator identity registrations ───────────────────
                        // A peer gossips a registration (with ownership proofs)
                        // so one POST propagates fleet-wide. We re-verify the
                        // proofs locally before applying — never trust the sender
                        // — and persist so a restart keeps the binding.
                        if topic_str.contains("validators") {
                            match serde_json::from_slice::<common::GossipMessage>(&message.data) {
                                Ok(common::GossipMessage::ValidatorRegistration {
                                    evm_address,
                                    node_id,
                                    ed25519_pubkey,
                                    alias,
                                    nonce,
                                    evm_signature,
                                    ed25519_signature,
                                    pq_pubkey,
                                    pq_binding_signature,
                                }) => {
                                    let proof = crate::validator_registry_gossip::RegistrationProof {
                                        evm_address,
                                        node_id,
                                        ed25519_pubkey,
                                        alias,
                                        nonce,
                                        evm_signature,
                                        ed25519_signature,
                                        pq_pubkey,
                                        pq_binding_signature,
                                    };
                                    match crate::api::apply_validator_registration(&state, &proof).await {
                                        Ok(status) => {
                                            let data_dir = { let s = state.read().await; s.config.data_dir.clone() };
                                            if let Err(e) = crate::validator_registry_gossip::persist(&data_dir, &proof) {
                                                tracing::warn!("Failed to persist gossiped validator registration: {}", e);
                                            }
                                            peer_scores.record_valid_message(&peer_id);
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Accept,
                                            );
                                            tracing::info!("Applied gossiped validator registration for {} ({})", proof.node_id, status.status);
                                        }
                                        Err((status, e)) => {
                                            if status.is_client_error() {
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Reject,
                                                );
                                                let action = peer_scores.record_invalid_signature(&peer_id);
                                                handle_peer_action(&mut self.swarm, peer_id, action);
                                            } else {
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Ignore,
                                                );
                                            }
                                            tracing::debug!("Dropped gossiped validator registration for {}: {}", proof.node_id, e);
                                        }
                                    }
                                }
                                _ => {
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        gossipsub::MessageAcceptance::Reject,
                                    );
                                    let action = peer_scores.record_corrupted_payload(&peer_id);
                                    handle_peer_action(&mut self.swarm, peer_id, action);
                                }
                            }
                            continue;
                        }
                        if topic_str.contains("validator-set-transitions") {
                            let snapshot = match serde_json::from_slice(&message.data) {
                                Ok(common::GossipMessage::ValidatorSetTransition { snapshot }) => {
                                    snapshot
                                }
                                Ok(_) | Err(_) => {
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        gossipsub::MessageAcceptance::Reject,
                                    );
                                    tracing::warn!(
                                        message_id = %id,
                                        peer = %peer_id,
                                        "P2P: rejected malformed validator-set transition gossip"
                                    );
                                    continue;
                                }
                            };

                            let snapshot_addresses: Vec<String> = snapshot
                                .validators
                                .validators
                                .iter()
                                .map(|v| v.eth_address.trim().to_ascii_lowercase())
                                .filter(|a| !a.is_empty())
                                .collect();
                            let persistence = {
                                // Serialize the canonical-tip check and
                                // synchronous history replacement with reorg
                                // adoption, which holds the state write guard
                                // through journal commit.
                                let s = state.read().await;
                                let checkpoint =
                                    match crate::validator_set_history::checkpoint_from_env() {
                                        Ok(checkpoint) => checkpoint,
                                        Err(error) => {
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Ignore,
                                            );
                                            tracing::error!("Dropped validator-set transition: invalid weak-subjectivity checkpoint: {error}");
                                            continue;
                                        }
                                    };
                                (|| -> anyhow::Result<gossipsub::MessageAcceptance> {
                                    let tip_height = s.chain.tip_height().map_err(
                                        crate::validator_set_history::RecordAuthenticatedError::persistence,
                                    )?;
                                    let tip_hash = s.chain.tip_hash().map_err(
                                        crate::validator_set_history::RecordAuthenticatedError::persistence,
                                    )?;
                                    validate_and_persist_transition(
                                        &s.config.data_dir,
                                        snapshot,
                                        &s.validator_set_bootstrap,
                                        tip_height,
                                        &tip_hash,
                                        checkpoint.as_ref(),
                                    )
                                })()
                            };
                            match persistence {
                                Ok(acceptance) => {
                                    if matches!(acceptance, gossipsub::MessageAcceptance::Accept) {
                                        peer_scores.record_valid_message(&peer_id);
                                        admission_store
                                            .record_transition_certified_batch(&snapshot_addresses)
                                            .await;
                                        {
                                            let mut s = state.write().await;
                                            if let Err(error) =
                                                s.refresh_runtime_validator_set_from_certified_history()
                                            {
                                                tracing::warn!(%error, "Failed to refresh runtime validator set after transition gossip");
                                            }
                                        }
                                        if let Err(error) =
                                            crate::consensus_admission::try_activate_candidate_validator(&state)
                                                .await
                                        {
                                            tracing::warn!(%error, "Candidate auto-activation check failed after transition gossip");
                                        }
                                    }
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        acceptance,
                                    );
                                }
                                Err(error) => {
                                    let acceptance =
                                        transition_persistence_failure_acceptance(&error);
                                    let is_reject =
                                        matches!(acceptance, gossipsub::MessageAcceptance::Reject);
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        acceptance,
                                    );
                                    if is_reject {
                                        let action = peer_scores.record_corrupted_payload(&peer_id);
                                        handle_peer_action(&mut self.swarm, peer_id, action);
                                    }
                                    tracing::debug!("Dropped validator-set transition gossip: {error}");
                                }
                            }
                            continue;
                        }

                        if topic_str == crate::revocation_gossip::TOPIC {
                            match serde_json::from_slice::<common::GossipMessage>(&message.data) {
                                Ok(common::GossipMessage::RevocationRequest { request, delivery_id })
                                    if delivery_id.len() <= 128 =>
                                {
                                    if let Err(error) = request.verify_signature() {
                                        tracing::warn!(peer = %peer_id, %error, "P2P: rejected revocation with invalid signature");
                                        report_validation_result(
                                            &mut self.swarm,
                                            &id,
                                            &peer_id,
                                            gossipsub::MessageAcceptance::Reject,
                                        );
                                        let action = peer_scores.record_invalid_signature(&peer_id);
                                        handle_peer_action(&mut self.swarm, peer_id, action);
                                    } else {
                                        match crate::revocation_gossip::admit(&state, &request).await {
                                            Ok(_) => {
                                                peer_scores.record_valid_message(&peer_id);
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Accept,
                                                );
                                            }
                                            Err(error) => {
                                                tracing::debug!(%error, "Revocation proof not admitted; sender will retry");
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Ignore,
                                                );
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        gossipsub::MessageAcceptance::Reject,
                                    );
                                    let action = peer_scores.record_corrupted_payload(&peer_id);
                                    handle_peer_action(&mut self.swarm, peer_id, action);
                                }
                            }
                            continue;
                        }

                        // ── PBFT view-change votes ──────────────────────────────
                        if topic_str.contains("view-change") {
                            if !crate::block_producer::PBFT_VIEW_CHANGE_READY {
                                report_validation_result(
                                    &mut self.swarm,
                                    &id,
                                    &peer_id,
                                    gossipsub::MessageAcceptance::Reject,
                                );
                                continue;
                            }
                            match serde_json::from_slice::<common::GossipMessage>(&message.data) {
                                Ok(common::GossipMessage::ViewChange { vote }) => {
                                    let (data_dir, bootstrap) = {
                                        let s = state.read().await;
                                        (s.config.data_dir.clone(), s.validator_set_bootstrap.clone())
                                    };
                                    let authority = match crate::validator_set_history::verification_set_at(
                                        &data_dir,
                                        vote.height,
                                        &bootstrap,
                                    ) {
                                        Ok(authority) => authority,
                                        Err(error) => {
                                            tracing::warn!(
                                                height = vote.height,
                                                error = %error,
                                                "Rejected view-change without authenticated validator authority"
                                            );
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            continue;
                                        }
                                    };
                                    let pbft_authority: consensus::ValidatorSet = authority.into();
                                    let (result, validator_id, new_view, height) = {
                                        let mut s = state.write().await;
                                        let validator_id = vote.validator_id.clone();
                                        let new_view = vote.new_view;
                                        let height = vote.height;
                                        let result = (|| -> anyhow::Result<_> {
                                            let tip_height = s.chain.tip_height()?;
                                            let tip_hash = s.chain.tip_hash()?;
                                            if vote.height != tip_height.saturating_add(1)
                                                || vote.prev_hash != tip_hash
                                            {
                                                anyhow::bail!("view-change vote is not for the next canonical height");
                                            }
                                            if s.config.is_validator {
                                                s.chain.pbft_safety_journal()?.record_received_view_change(
                                                    &vote,
                                                    &pbft_authority,
                                                    s.pbft_engine.allow_small_cluster_quorum(),
                                                )?;
                                            }
                                            s.pbft_engine.receive_view_change(vote, &pbft_authority)
                                        })();
                                        (result, validator_id, new_view, height)
                                    };
                                    match result {
                                        Ok(certificate) => {
                                            if let Some(src) = message.source {
                                                peer_validators.insert(src, validator_id.clone());
                                            }
                                            peer_scores.record_valid_message(&peer_id);
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Accept,
                                            );
                                            if let Some(certificate) = certificate {
                                                tracing::warn!(
                                                    height,
                                                    view = new_view,
                                                    validators = certificate.len(),
                                                    "[PBFT] Applied quorum-certified view change"
                                                );
                                            }
                                        }
                                        Err(error) => {
                                            tracing::warn!(
                                                height,
                                                new_view,
                                                validator_id = %validator_id,
                                                peer = %peer_id,
                                                error = %error,
                                                "Rejected invalid PBFT view-change vote"
                                            );
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            let action = peer_scores.record_invalid_signature(&peer_id);
                                            handle_peer_action(&mut self.swarm, peer_id, action);
                                        }
                                    }
                                }
                                _ => {
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        gossipsub::MessageAcceptance::Reject,
                                    );
                                    let action = peer_scores.record_corrupted_payload(&peer_id);
                                    handle_peer_action(&mut self.swarm, peer_id, action);
                                }
                            }
                            continue;
                        }

                        // Votes: gate propagation on application-level validation.
                        // gossipsub strict mode authenticates the relaying peer, but
                        // vote authenticity depends on the validator-set identity and
                        // Ed25519 signature inside the payload.
                        if topic_str.contains("votes") {
                            let validation =
                                validate_vote_gossip_message(&message.data, &vs_snapshot);

                            match validation {
                                Ok(vote) => {
                                    if let Some(src) = message.source {
                                        peer_validators.insert(src, vote.validator_id.clone());
                                    }
                                    peer_scores.record_valid_message(&peer_id);
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        gossipsub::MessageAcceptance::Accept,
                                    );

                                    record_validated_vote_gossip(&state, &vote).await;
                                }
                                Err(error) => {
                                    report_validation_result(
                                        &mut self.swarm,
                                        &id,
                                        &peer_id,
                                        gossipsub::MessageAcceptance::Reject,
                                    );
                                    let action = peer_scores.record_invalid_signature(&peer_id);
                                    handle_peer_action(&mut self.swarm, peer_id, action);
                                    {
                                        let mut s = state.write().await;
                                        s.p2p_status.gossip_votes_rejected_total =
                                            s.p2p_status.gossip_votes_rejected_total.saturating_add(1);
                                    }
                                    tracing::warn!(
                                        peer = %peer_id,
                                        reason = ?error,
                                        "P2P: rejecting gossip vote before mesh propagation"
                                    );
                                }
                            }
                            continue;
                        }

                        // ── PBFT Block Consensus ───────────────────────────────────────────────
                        if topic_str.contains("blocks") {
                            if let Ok(msg) = serde_json::from_slice::<common::GossipMessage>(&message.data) {
                                let reproposal = match &msg {
                                    common::GossipMessage::PbftRepropose {
                                        target_view,
                                        leader_id,
                                        view_change_certificate,
                                        leader_signature,
                                        ..
                                    } => Some((
                                        *target_view,
                                        leader_id.clone(),
                                        view_change_certificate.clone(),
                                        leader_signature.clone(),
                                    )),
                                    _ => None,
                                };
                                match msg {
                                    common::GossipMessage::PbftPrePrepare { block }
                                    | common::GossipMessage::PbftRepropose { block, .. } => {
                                        if reproposal.is_some()
                                            && !crate::block_producer::PBFT_VIEW_CHANGE_READY
                                        {
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            continue;
                                        }
                                        let mut s = state.write().await;
                                        // Detection only (ADR-016): keep the header before any
                                        // check below can reject the proposal, so a vote for a
                                        // conflicting proposal can still be proven.
                                        s.observe_proposal_for_equivocation(&block.header);
                                        let tip_height = s.chain.tip_height().unwrap_or(0);
                                        let tip_hash = s.chain.tip_hash().unwrap_or_default();
                                        let vs = match crate::validator_set_history::verification_set_at(
                                            &s.config.data_dir,
                                            block.header.height,
                                            &s.validator_set_bootstrap,
                                        ) {
                                            Ok(authority) => authority,
                                            Err(error) => {
                                                tracing::warn!(
                                                    peer = %peer_id,
                                                    height = block.header.height,
                                                    error = %error,
                                                    "Rejected PBFT pre-prepare because authenticated validator history is unavailable"
                                                );
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Reject,
                                                );
                                                continue;
                                            }
                                        };
                                        let Some(tip_timestamp) = s
                                            .chain
                                            .get_block_by_height(tip_height)
                                            .ok()
                                            .flatten()
                                            .map(|block| block.header.timestamp)
                                        else {
                                            tracing::warn!(
                                                peer = %peer_id,
                                                tip_height,
                                                "Rejected PBFT pre-prepare because the canonical tip timestamp is unavailable"
                                            );
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            continue;
                                        };
                                        let now = chrono::Utc::now();
                                        if reproposal.is_some()
                                            && block.header.timestamp > now + chrono::Duration::seconds(60)
                                        {
                                            tracing::warn!(peer = %peer_id, "Rejected prepared reproposal with a future block timestamp");
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            continue;
                                        }
                                        // A quorum-prepared immutable block may be older than
                                        // the live proposal window when the new leader sends it.
                                        let validation_now = if reproposal.is_some() {
                                            block.header.timestamp
                                        } else {
                                            now
                                        };
                                        if let Err(error) = validate_pbft_pre_prepare_block(
                                            &block,
                                            &vs,
                                            tip_height,
                                            &tip_hash,
                                            tip_timestamp,
                                            validation_now,
                                        ) {
                                            tracing::warn!(
                                                peer = %peer_id,
                                                error = %error,
                                                "Rejected invalid PBFT pre-prepare block"
                                            );
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            continue;
                                        }
                                        let namespace_config =
                                            crate::namespace_consensus::NamespaceConsensusConfig::from_node_config(&s.config);
                                        if let Err(error) = crate::sync::verify_block_signatures(&block, &vs) {
                                            tracing::warn!(peer = %peer_id, error = %error,
                                                "Rejected PBFT pre-prepare without valid package approvals");
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            continue;
                                        }
                                        if let Err(error) = crate::namespace_consensus::validate_block(
                                            &s.chain,
                                            &block,
                                            &namespace_config,
                                        ) {
                                            tracing::warn!(
                                                peer = %peer_id,
                                                error = %error,
                                                "Rejected PBFT pre-prepare with invalid namespace transition"
                                            );
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            continue;
                                        }
                                        // Vote lock: at most one proposal per height. A second,
                                        // different proposal at an already-open height is
                                        // rejected so this validator never votes twice and two
                                        // proposals can never both reach quorum.
                                        let incoming_hash = block.hash();
                                        if let Some((locked_hash, phase)) = s
                                            .pbft_engine
                                            .non_terminal_round_at_height(block.header.height)
                                        {
                                            if locked_hash != incoming_hash {
                                                tracing::warn!(
                                                    peer = %peer_id,
                                                    height = block.header.height,
                                                    phase = phase.as_str(),
                                                    locked = %locked_hash[..locked_hash.len().min(12)],
                                                    incoming = %incoming_hash[..incoming_hash.len().min(12)],
                                                    "Rejected second PBFT proposal at locked height"
                                                );
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Reject,
                                                );
                                                continue;
                                            }
                                            // Same proposal re-received (gossip duplicate): accept
                                            // but keep the existing round and its votes instead of
                                            // restarting it.
                                            let active_view = s
                                                .pbft_engine
                                                .round_context(&incoming_hash)
                                                .map(|(_, _, view)| view)
                                                .unwrap_or(block.header.view_number);
                                            if reproposal
                                                .as_ref()
                                                .is_none_or(|(target_view, _, _, _)| *target_view <= active_view)
                                            {
                                                peer_scores.record_valid_message(&peer_id);
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Accept,
                                                );
                                                continue;
                                            }
                                        }
                                        // Header validation and PBFT quorum use the
                                        // same authenticated authority snapshot.
                                        let start_result = (|| -> anyhow::Result<String> {
                                            if let Some((target_view, leader_id, certificate, leader_signature)) = &reproposal {
                                                let authority: consensus::ValidatorSet = vs.clone().into();
                                                if s.config.is_validator {
                                                    let journal = s.chain.pbft_safety_journal()?;
                                                    for vote in certificate {
                                                        journal.record_received_view_change(
                                                            vote,
                                                            &authority,
                                                            s.pbft_engine.allow_small_cluster_quorum(),
                                                        )?;
                                                    }
                                                }
                                                s.pbft_engine.repropose_prepared(
                                                    block.clone(),
                                                    authority,
                                                    *target_view,
                                                    leader_id,
                                                    certificate.clone(),
                                                    leader_signature,
                                                )
                                            } else {
                                                if block.header.view_number > 0 && s.config.is_validator {
                                                    let journal = s.chain.pbft_safety_journal()?;
                                                    let authority: consensus::ValidatorSet = vs.clone().into();
                                                    for vote in &block.header.view_change_certificate {
                                                        journal.record_received_view_change(
                                                            vote,
                                                            &authority,
                                                            s.pbft_engine.allow_small_cluster_quorum(),
                                                        )?;
                                                    }
                                                }
                                                s.pbft_engine.start_round(block.clone(), vs.clone().into())
                                            }
                                        })();
                                        if let Err(error) = start_result {
                                            tracing::warn!(
                                                peer = %peer_id,
                                                error = %error,
                                                "Rejected PBFT pre-prepare that failed consensus validation"
                                            );
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Reject,
                                            );
                                            let action = peer_scores.record_corrupted_payload(&peer_id);
                                            handle_peer_action(&mut self.swarm, peer_id, action);
                                        } else {
                                            peer_scores.record_valid_message(&peer_id);
                                            report_validation_result(
                                                &mut self.swarm,
                                                &id,
                                                &peer_id,
                                                gossipsub::MessageAcceptance::Accept,
                                            );
                                            // Sign the block and broadcast PREPARE (validators only; observers never sign)
                                            if s.config.is_validator {
                                                if let Some(our_id) = s.config.node_id.clone().into() {
                                                    if let Some(our_privkey_hex) = &s.config.validator_privkey {
                                                    let bh = block.hash();
                                                    if let Ok(pk_bytes) = hex::decode(our_privkey_hex) {
                                                        if let Ok(sk) = common::crypto::SigningKey::from_seed(common::crypto::SignatureScheme::Ed25519, &pk_bytes) {
                                                            let (signature, pubkey) = sign_pbft_phase(&sk, "prepare", &bh);
                                                            let sig_obj = common::BlockSignature { validator_id: our_id.clone(), pubkey: pubkey.clone(), signature: signature.clone(), pq_signature: None };
                                                            let signing_message = consensus::pbft::pbft_signature_message("prepare", &bh);
                                                            let durable = s.chain.pbft_safety_journal().and_then(|journal| {
                                                                journal.record_decision(
                                                                    crate::pbft_safety::PbftVotePhase::Prepare,
                                                                    block.header.height,
                                                                    &block.header.prev_hash,
                                                                    s.pbft_engine
                                                                        .round_context(&bh)
                                                                        .map(|(_, _, view)| view)
                                                                        .unwrap_or(block.header.view_number),
                                                                    &bh,
                                                                    &signing_message,
                                                                    &signature,
                                                                )
                                                            });
                                                            if let Err(error) = durable {
                                                                tracing::error!(block_hash = %bh, error = %error, "Refusing to broadcast PREPARE without a durable vote lock");
                                                            } else {
                                                                match s.pbft_engine.prepare(&bh, &our_id, sig_obj) {
                                                                    Ok(prepare_quorum) => {
                                                                        let prep_msg = common::GossipMessage::PbftPrepare { block_hash: bh.clone(), validator_id: our_id.clone(), signature };
                                                                        if let Ok(data) = serde_json::to_vec(&prep_msg) {
                                                                            let topic = libp2p::gossipsub::IdentTopic::new("creg/v1/blocks");
                                                                            let _ = self.swarm.behaviour_mut().gossipsub.publish(topic, data);
                                                                        }
                                                                        if prepare_quorum {
                                                                            match sign_and_record_commit(&mut s, &bh, &our_id, &sk).await {
                                                                                Ok((commit_msg, finalized)) => {
                                                                                    if finalized {
                                                                                        if let Err(error) = apply_finalized_pbft_block(&mut s, &bh) {
                                                                                            tracing::error!(block_hash = %bh, error = %error, "Failed to apply locally finalized PBFT block");
                                                                                        }
                                                                                    }
                                                                                    if let Ok(data) = serde_json::to_vec(&commit_msg) {
                                                                                        let topic = libp2p::gossipsub::IdentTopic::new("creg/v1/blocks");
                                                                                        let _ = self.swarm.behaviour_mut().gossipsub.publish(topic, data);
                                                                                    }
                                                                                }
                                                                                Err(error) => tracing::error!(block_hash = %bh, error = %error, "Refusing COMMIT after prepared reproposal"),
                                                                            }
                                                                        }
                                                                    }
                                                                    Err(error) => tracing::warn!(block_hash = %bh, error = %error, "Refusing to broadcast rejected local PBFT PREPARE"),
                                                                }
                                                            }
                                                        } else {
                                                            tracing::warn!("Failed to parse own validator key for Prepare sign on {}", bh);
                                                        }
                                                    } else {
                                                        tracing::warn!("Failed to hex-decode own validator key for Prepare sign on {}", bh);
                                                    }
                                                }
                                            }
                                        }
                                        }
                                    }
                                    common::GossipMessage::PbftPrepare { block_hash, validator_id, signature } => {
                                        let validation = match validate_pbft_phase_message(
                                            &vs_snapshot,
                                            "prepare",
                                            &block_hash,
                                            &validator_id,
                                            &signature,
                                        ) {
                                            Ok(sig) => Ok(sig),
                                            Err(PbftValidationError::UnknownValidator) => {
                                                let certified = {
                                                    let s = state.read().await;
                                                    s.chain.tip_height().ok().and_then(|h| h.checked_add(1)).and_then(|next_h| {
                                                        crate::validator_set_history::verification_set_at(
                                                            &s.config.data_dir,
                                                            next_h,
                                                            &s.validator_set_bootstrap,
                                                        )
                                                        .ok()
                                                    })
                                                };
                                                if let Some(authority) = certified {
                                                    let res = validate_pbft_phase_message(
                                                        &authority,
                                                        "prepare",
                                                        &block_hash,
                                                        &validator_id,
                                                        &signature,
                                                    );
                                                    if res.is_ok() {
                                                        let mut s = state.write().await;
                                                        let _ = s.refresh_runtime_validator_set_from_certified_history();
                                                    }
                                                    res
                                                } else {
                                                    Err(PbftValidationError::UnknownValidator)
                                                }
                                            }
                                            Err(other) => Err(other),
                                        };
                                        let sig = match validation {
                                            Ok(sig) => {
                                                if let Some(src) = message.source {
                                                    peer_validators.insert(src, validator_id.clone());
                                                }
                                                peer_scores.record_valid_message(&peer_id);
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Accept,
                                                );
                                                sig
                                            }
                                            Err(error) => {
                                                report_pbft_validation_failure(
                                                    "prepare",
                                                    &validator_id,
                                                    &block_hash,
                                                    error,
                                                );
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Reject,
                                                );
                                                let action = peer_scores.record_invalid_signature(&peer_id);
                                                handle_peer_action(&mut self.swarm, peer_id, action);
                                                {
                                                    let mut s = state.write().await;
                                                    s.p2p_status.gossip_pbft_rejected_total =
                                                        s.p2p_status.gossip_pbft_rejected_total
                                                            .saturating_add(1);
                                                }
                                                continue;
                                            }
                                        };
                                        let mut s = state.write().await;
                                        // Detection only (ADR-016), before the engine can
                                        // suppress a second vote from the same validator.
                                        s.observe_vote_for_equivocation(
                                            consensus::slashing_relay::VotePhase::Prepare,
                                            &block_hash,
                                            &sig,
                                        );
                                        if let Ok(true) = s.pbft_engine.prepare(&block_hash, &validator_id, sig) {
                                            if s.config.is_validator {
                                                if let Some(private_key_hex) = &s.config.validator_privkey {
                                                if let Ok(key_bytes) = hex::decode(private_key_hex) {
                                                    if let Ok(signing_key) = common::crypto::SigningKey::from_seed(common::crypto::SignatureScheme::Ed25519, &key_bytes) {
                                                        let our_id = s.config.node_id.clone();
                                                        match sign_and_record_commit(&mut s, &block_hash, &our_id, &signing_key).await {
                                                            Ok((commit_msg, finalized)) => {
                                                                if finalized {
                                                                    if let Err(error) = apply_finalized_pbft_block(&mut s, &block_hash) {
                                                                        tracing::error!(block_hash = %block_hash, error = %error, "Failed to apply locally finalized PBFT block");
                                                                    }
                                                                }
                                                                if let Ok(data) = serde_json::to_vec(&commit_msg) {
                                                                    let topic = libp2p::gossipsub::IdentTopic::new("creg/v1/blocks");
                                                                    let _ = self.swarm.behaviour_mut().gossipsub.publish(topic, data);
                                                                }
                                                            }
                                                            Err(error) => tracing::error!(block_hash = %block_hash, error = %error, "Refusing COMMIT without durable PBFT safety state"),
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        }
                                    }
                                    common::GossipMessage::PbftCommit { block_hash, validator_id, signature } => {
                                        let validation = match validate_pbft_phase_message(
                                            &vs_snapshot,
                                            "commit",
                                            &block_hash,
                                            &validator_id,
                                            &signature,
                                        ) {
                                            Ok(sig) => Ok(sig),
                                            Err(PbftValidationError::UnknownValidator) => {
                                                let certified = {
                                                    let s = state.read().await;
                                                    s.chain.tip_height().ok().and_then(|h| h.checked_add(1)).and_then(|next_h| {
                                                        crate::validator_set_history::verification_set_at(
                                                            &s.config.data_dir,
                                                            next_h,
                                                            &s.validator_set_bootstrap,
                                                        )
                                                        .ok()
                                                    })
                                                };
                                                if let Some(authority) = certified {
                                                    let res = validate_pbft_phase_message(
                                                        &authority,
                                                        "commit",
                                                        &block_hash,
                                                        &validator_id,
                                                        &signature,
                                                    );
                                                    if res.is_ok() {
                                                        let mut s = state.write().await;
                                                        let _ = s.refresh_runtime_validator_set_from_certified_history();
                                                    }
                                                    res
                                                } else {
                                                    Err(PbftValidationError::UnknownValidator)
                                                }
                                            }
                                            Err(other) => Err(other),
                                        };
                                        let sig = match validation {
                                            Ok(sig) => {
                                                if let Some(src) = message.source {
                                                    peer_validators.insert(src, validator_id.clone());
                                                }
                                                peer_scores.record_valid_message(&peer_id);
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Accept,
                                                );
                                                sig
                                            }
                                            Err(error) => {
                                                report_pbft_validation_failure(
                                                    "commit",
                                                    &validator_id,
                                                    &block_hash,
                                                    error,
                                                );
                                                report_validation_result(
                                                    &mut self.swarm,
                                                    &id,
                                                    &peer_id,
                                                    gossipsub::MessageAcceptance::Reject,
                                                );
                                                let action = peer_scores.record_invalid_signature(&peer_id);
                                                handle_peer_action(&mut self.swarm, peer_id, action);
                                                {
                                                    let mut s = state.write().await;
                                                    s.p2p_status.gossip_pbft_rejected_total =
                                                        s.p2p_status.gossip_pbft_rejected_total
                                                            .saturating_add(1);
                                                }
                                                continue;
                                            }
                                        };
                                        let mut s = state.write().await;
                                        // Detection only (ADR-016), before the engine can
                                        // suppress a second vote from the same validator.
                                        s.observe_vote_for_equivocation(
                                            consensus::slashing_relay::VotePhase::Commit,
                                            &block_hash,
                                            &sig,
                                        );
                                        if let Ok(true) = s.pbft_engine.commit(&block_hash, &validator_id, sig) {
                                            // Finalized!
                                            tracing::info!("[PBFT] Block {} finalised by quorum", &block_hash[..12]);
                                            if let Some(final_block) = s.pbft_engine.get_finalised_block(&block_hash) {
                                                let inserted = match s.chain.insert_block_with_outcome(&final_block) {
                                                    Ok(outcome) => {
                                                        if let Some(replaced) = outcome.replaced_hash {
                                                            s.record_reorg(1, vec![replaced], outcome.hash.clone());
                                                        }
                                                        true
                                                    }
                                                    Err(e) => {
                                                        tracing::error!("[PBFT] Failed to insert finalised block: {}", e);
                                                        false
                                                    }
                                                };
                                                if !inserted {
                                                    continue;
                                                }
                                                s.publisher_index.apply_block(&final_block);
                                                s.on_block_committed(&final_block);
                                                let data_dir = s.config.data_dir.clone();
                                                crate::intelligence::schedule_for_block(
                                                    &final_block,
                                                    data_dir,
                                                );
                                            }
                                        }
                                    }
                                    _ => {
                                        report_validation_result(
                                            &mut self.swarm,
                                            &id,
                                            &peer_id,
                                            gossipsub::MessageAcceptance::Reject,
                                        );
                                        let action = peer_scores.record_corrupted_payload(&peer_id);
                                        handle_peer_action(&mut self.swarm, peer_id, action);
                                    }
                                }
                            } else {
                                report_validation_result(
                                    &mut self.swarm,
                                    &id,
                                    &peer_id,
                                    gossipsub::MessageAcceptance::Reject,
                                );
                                let action = peer_scores.record_corrupted_payload(&peer_id);
                                handle_peer_action(&mut self.swarm, peer_id, action);
                            }
                            continue;
                        }

                        report_validation_result(
                            &mut self.swarm,
                            &id,
                            &peer_id,
                            gossipsub::MessageAcceptance::Ignore,
                        );
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        tracing::info!("P2P node listening on {}", address);
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        let remote_addr = endpoint.get_remote_address();
                        if !is_loopback_multiaddr(remote_addr)
                            && !subnet_filter.allow_and_record(
                                &peer_id,
                                remote_addr,
                                peer_validators.contains_key(&peer_id),
                            )
                        {
                            tracing::warn!(%peer_id, ?remote_addr, "P2P subnet cap exceeded; disconnecting peer");
                            let _ = self.swarm.disconnect_peer_id(peer_id);
                        } else {
                            tracing::info!("P2P Connection established with {} at {:?}", peer_id, endpoint);
                            ingest_kademlia_address(
                                &mut self.swarm,
                                peer_id,
                                remote_addr.clone(),
                                kad_bootstrapped,
                            );
                        }
                    }
                    SwarmEvent::ConnectionClosed { peer_id, num_established: 0, .. } => {
                        subnet_filter.record_disconnect(&peer_id);
                        peer_validators.remove(&peer_id);
                    }
                    SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                        tracing::error!("P2P Outgoing connection error to {:?}: {}", peer_id, error);
                    }
                    _ => {}
                },

                // ── Periodically update SharedState with peer list ────────────
                _ = status_ticker.tick() => {
                    peer_scores.prune_stale_peers(Duration::from_secs(3600));
                    let peers: Vec<String> = self.swarm.connected_peers()
                        .map(|p| p.to_string())
                        .collect();
                    let mut s = state.write().await;
                    s.p2p_status.peers = peers;
                    s.p2p_status.protocols = vec!["Identify".into(), "Ping".into(), "Kademlia".into()];
                }

                // ── Identify Storage Responsibility (Sharding) ───────────────
                command = self.receiver.recv() => {
                    if let Some(cmd) = command {
                        match cmd {
                            P2PCommand::Broadcast { topic, data } => {
                                let t = gossipsub::IdentTopic::new(topic);
                                match self.swarm.behaviour_mut().gossipsub.publish(t, data) {
                                    Ok(_) => {}
                                    Err(gossipsub::PublishError::Duplicate) => {
                                        tracing::debug!("P2P broadcast skipped duplicate message");
                                    }
                                    Err(e) => {
                                        tracing::error!("P2P broadcast failed: {}", e);
                                    }
                                }
                            }
                            P2PCommand::Dial { addr } => {
                                if let Some(peer_id) = peer_id_from_multiaddr(&addr) {
                                    ingest_kademlia_address(
                                        &mut self.swarm,
                                        peer_id,
                                        addr.clone(),
                                        kad_bootstrapped,
                                    );
                                    if peer_id == self.peer_id || self.swarm.is_connected(&peer_id) {
                                        tracing::trace!(
                                            %peer_id,
                                            %addr,
                                            "Skipping dial to self or already-connected seed peer"
                                        );
                                        continue;
                                    }
                                }
                                tracing::info!("P2P Dialing {}...", addr);
                                if let Err(e) = self.swarm.dial(addr) {
                                    tracing::error!("P2P dial failed: {}", e);
                                }
                            }
                            P2PCommand::IdentifyStorage { cid } => {
                                let is_responsible = self.is_responsible_for(&cid);
                                tracing::info!("Storage check for {}: Responsible={}", cid, is_responsible);
                                // Logic to trigger Pinning/Pruning would happen here
                            }
                        }
                    }
                }
            }
        }
    }

    /// Determines if this node is among the 'N' closest nodes to a CID.
    /// This is the core of our 'Masterless Sharding' for 500MB+ packages.
    ///
    /// Uses a Kademlia-style XOR distance over 8 bytes of the peer ID vs the
    /// SHA-256 of the CID.  The single-byte XOR used previously was biased
    /// (only 256 distinct distances) and collapsed entirely for small networks
    /// where collisions are common.
    pub fn is_responsible_for(&self, cid: &str) -> bool {
        use sha2::{Digest, Sha256};

        let local_bytes = self.peer_id.to_bytes();
        if local_bytes.len() < 8 {
            return false;
        }

        // Hash the CID to get a uniformly distributed key.
        let cid_hash = Sha256::digest(cid.as_bytes());

        // XOR-distance over the first 8 bytes, interpreted as a big-endian u64.
        // This gives 2^64 distinct distance values, matching Kademlia semantics.
        let mut local_arr = [0u8; 8];
        local_arr.copy_from_slice(&local_bytes[..8]);
        let mut cid_arr = [0u8; 8];
        cid_arr.copy_from_slice(&cid_hash[..8]);

        let distance = u64::from_be_bytes(local_arr) ^ u64::from_be_bytes(cid_arr);

        // Threshold: u64::MAX / 8 ≈ top-12.5% of the keyspace → ~7-10 nodes
        // in a 64-node network. Overridable via `CREG_SHARD_THRESHOLD_PCT` (1-100).
        let pct: u64 = std::env::var("CREG_SHARD_THRESHOLD_PCT")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(12)
            .clamp(1, 100);
        let threshold = u64::MAX / 100 * pct;
        distance < threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::RwLock;

    fn validator(id: &str, signing_key: &SigningKey) -> common::Validator {
        common::Validator {
            id: id.into(),
            alias: id.into(),
            pubkey: hex::encode(signing_key.verifying_key().as_bytes()),
            pq_pubkey: None,
            eth_address: String::new(),
            stake: 100,
            reputation: 100,
            status: "online".into(),
            performance_score: 500,
            contribution_count: 0,
        }
    }

    fn signed_transition(
        validator_set: &common::ValidatorSet,
        signing_keys: &[SigningKey],
        signer_indexes: &[usize],
        effective_from_height: u64,
        previous_block_hash: String,
    ) -> common::AuthenticatedValidatorSetSnapshot {
        let mut transition = common::AuthenticatedValidatorSetSnapshot {
            schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height,
            previous_block_hash,
            previous_validator_set_hash: common::canonical_validator_set_hash(validator_set),
            validators: validator_set.clone(),
            validator_set_hash: common::canonical_validator_set_hash(validator_set),
            transition_signatures: Vec::new(),
        };
        let message = common::validator_set_transition_message(&transition);
        for index in signer_indexes {
            let validator = &validator_set.validators[*index];
            let signing_key = &signing_keys[*index];
            transition
                .transition_signatures
                .push(common::ValidatorSetTransitionSignature {
                    validator_id: validator.id.clone(),
                    validator_pubkey: validator.pubkey.clone(),
                    signature: hex::encode(signing_key.sign(message.as_bytes()).to_bytes()),
                });
        }
        transition
    }

    fn signed_vote(
        validator_id: &str,
        signing_key: &SigningKey,
        validator_pubkey: String,
    ) -> crate::gossip::VoteGossip {
        let analysis_bundles = common::AnalysisBundleRefs {
            policy_bundle_id: "policy-v1".into(),
            feature_schema_id: "features-v1".into(),
            expert_bundle_id: "experts-v1".into(),
            embedding_model_id: "embeddings-v1".into(),
            index_epoch: "index-epoch-1".into(),
            threshold_profile_id: "thresholds-v1".into(),
            llm_prompt_profile_id: "llm-prompt-v1".into(),
            osv_snapshot_epoch: "osv-off".into(),
        };
        let evidence_digest = common::sha256_hex(b"p2p-test-evidence");
        let ml_model_version = "creg-detect-v1.0.0".to_string();
        let message = crate::gossip::canonical_vote_message(
            "npm/pkg@1.0.0",
            "deadbeef",
            true,
            &validator_pubkey,
            &common::scanner_profile_digest(&ml_model_version, &analysis_bundles),
            &evidence_digest,
        );

        crate::gossip::VoteGossip {
            consensus_subject: "npm/pkg@1.0.0".into(),
            content_hash: "deadbeef".into(),
            publisher_authorization_digest: String::new(),
            validator_id: validator_id.into(),
            validator_pubkey,
            ml_model_version,
            analysis_bundles,
            evidence_digest,
            deterministic_risk: common::DeterministicRiskSummary::default(),
            phase: "commit".into(),
            approved: true,
            reject_reason: None,
            signature: hex::encode(signing_key.sign(message.as_bytes()).to_bytes()),
            is_mandatory: true,
            committee_qc_signature: None,
            committee_qc_domain: None,
            vote_v4: None,
        }
    }

    async fn make_test_state(
        validator_set: common::ValidatorSet,
    ) -> anyhow::Result<crate::SharedState> {
        let tempdir = tempfile::tempdir()?;
        let data_dir = tempdir.keep();
        let chain = crate::chain_store::ChainStore::open(&data_dir)?;
        crate::validator_set_history::record(&data_dir, 0, &validator_set)?;

        Ok(Arc::new(RwLock::new(crate::NodeState {
            chain,
            pending_pool: crate::pending_pool::PendingPool::new(),
            publisher_index: crate::publisher_index::PublisherIndex::new(),
            validator_set_bootstrap: validator_set.clone(),
            validator_set_snapshot: Arc::new(crate::state::ArcSwap::from_pointee(
                validator_set.clone(),
            )),
            validator_set,
            package_rounds: HashMap::new(),
            package_committees: HashMap::new(),
            config: crate::config::NodeConfig {
                data_dir,
                ..Default::default()
            },
            p2p_status: crate::P2PStatus::default(),
            bridge_status: crate::BridgeStatus::default(),
            vrf_proofs: HashMap::new(),
            decryption_shares: HashMap::new(),
            validator_registrations: HashMap::new(),
            validator_set_sync: crate::state::ValidatorSetSyncStatus::default(),
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
        })))
    }

    #[test]
    fn validator_set_transition_acceptance_is_deferred_until_validation() {
        assert!(requires_payload_validation_before_acceptance(
            "creg/v1/validator-set-transitions"
        ));
        assert!(requires_payload_validation_before_acceptance(
            "creg/v1/votes"
        ));
        assert!(requires_payload_validation_before_acceptance(
            "creg/v1/submissions"
        ));
        assert!(requires_payload_validation_before_acceptance(
            VIEW_CHANGE_TOPIC
        ));
        assert!(requires_payload_validation_before_acceptance(
            "creg/v1/blocks"
        ));
        assert!(requires_payload_validation_before_acceptance(
            "creg/v1/vrf-proofs"
        ));
    }

    #[test]
    fn p2p_identity_persists_across_reloads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("identity.key");
        let first = load_or_create_p2p_identity(Some(&path)).expect("create identity");
        let second = load_or_create_p2p_identity(Some(&path)).expect("reload identity");
        assert_eq!(first.public().to_peer_id(), second.public().to_peer_id());
        assert!(path.exists());
    }

    #[test]
    fn parse_p2p_seed_multiaddrs_skips_invalid() {
        let (ok, bad) = parse_p2p_seed_multiaddrs(&[
            "/ip4/127.0.0.1/tcp/4001".into(),
            "not-a-multiaddr".into(),
        ]);
        assert_eq!(ok.len(), 1);
        assert_eq!(bad.len(), 1);
    }

    #[test]
    fn transition_payload_failures_reject_but_local_io_failures_do_not() {
        let conflicting =
            anyhow::anyhow!("conflicting validator-set transition at the same effective height");
        assert!(matches!(
            transition_persistence_failure_acceptance(&conflicting),
            gossipsub::MessageAcceptance::Reject
        ));

        let io_error = anyhow::Error::new(std::io::Error::other("disk unavailable"));
        assert!(matches!(
            transition_persistence_failure_acceptance(&io_error),
            gossipsub::MessageAcceptance::Ignore
        ));

        let retention_error = anyhow::Error::new(
            crate::validator_set_history::RecordAuthenticatedError::Persistence {
                source: anyhow::anyhow!("configured checkpoint retention failed"),
            },
        );
        assert!(matches!(
            transition_persistence_failure_acceptance(&retention_error),
            gossipsub::MessageAcceptance::Ignore
        ));
    }

    #[test]
    fn transition_receiver_rejects_stale_fork_conflict_checkpoint_and_invalid_quorum(
    ) -> anyhow::Result<()> {
        let signing_keys: Vec<_> = (1u8..=4)
            .map(|seed| SigningKey::from_bytes(&[seed; 32]))
            .collect();
        let validator_set = common::ValidatorSet::new(
            signing_keys
                .iter()
                .enumerate()
                .map(|(index, key)| validator(&format!("node-{}", index + 1), key))
                .collect(),
        );
        let dir = tempfile::tempdir()?;
        crate::validator_set_history::record(dir.path(), 0, &validator_set)?;
        let tip_hash = common::Block::genesis().hash();
        let valid = signed_transition(
            &validator_set,
            &signing_keys,
            &[0, 1, 2],
            1,
            tip_hash.clone(),
        );

        validate_and_persist_transition(
            dir.path(),
            valid.clone(),
            &validator_set,
            0,
            &tip_hash,
            None,
        )?;
        validate_and_persist_transition(
            dir.path(),
            valid.clone(),
            &validator_set,
            0,
            &tip_hash,
            None,
        )?; // exact idempotent replay

        let stale = signed_transition(
            &validator_set,
            &signing_keys,
            &[0, 1, 2],
            2,
            tip_hash.clone(),
        );
        assert!(validate_and_persist_transition(
            dir.path(),
            stale,
            &validator_set,
            0,
            &tip_hash,
            None,
        )
        .is_err());

        let wrong_fork =
            signed_transition(&validator_set, &signing_keys, &[0, 1, 2], 1, "f".repeat(64));
        assert!(validate_and_persist_transition(
            dir.path(),
            wrong_fork.clone(),
            &validator_set,
            0,
            &tip_hash,
            None,
        )
        .is_err());
        assert!(validate_and_persist_transition(
            dir.path(),
            valid.clone(),
            &validator_set,
            0,
            &tip_hash,
            Some(&wrong_fork),
        )
        .is_err());

        let insufficient_quorum =
            signed_transition(&validator_set, &signing_keys, &[0, 1], 1, tip_hash.clone());
        assert!(validate_and_persist_transition(
            dir.path(),
            insufficient_quorum,
            &validator_set,
            0,
            &tip_hash,
            None,
        )
        .is_err());

        let alternate_certificate = signed_transition(
            &validator_set,
            &signing_keys,
            &[1, 2, 3],
            1,
            tip_hash.clone(),
        );
        assert_eq!(
            common::validator_snapshot_certificate_hash(&alternate_certificate),
            common::validator_snapshot_certificate_hash(&valid)
        );
        assert_ne!(
            common::validator_snapshot_witness_hash(&alternate_certificate),
            common::validator_snapshot_witness_hash(&valid)
        );
        validate_and_persist_transition(
            dir.path(),
            alternate_certificate,
            &validator_set,
            0,
            &tip_hash,
            None,
        )?;

        let mut conflicting_transition = valid.clone();
        conflicting_transition.validators.validators[0].stake += 1;
        conflicting_transition.validator_set_hash =
            common::canonical_validator_set_hash(&conflicting_transition.validators);
        conflicting_transition.transition_signatures.clear();
        let conflicting_message = common::validator_set_transition_message(&conflicting_transition);
        for index in [0usize, 1, 2] {
            let validator = &validator_set.validators[index];
            conflicting_transition.transition_signatures.push(
                common::ValidatorSetTransitionSignature {
                    validator_id: validator.id.clone(),
                    validator_pubkey: validator.pubkey.clone(),
                    signature: hex::encode(
                        signing_keys[index]
                            .sign(conflicting_message.as_bytes())
                            .to_bytes(),
                    ),
                },
            );
        }
        assert!(validate_and_persist_transition(
            dir.path(),
            conflicting_transition,
            &validator_set,
            0,
            &tip_hash,
            None,
        )
        .is_err());

        let retained = crate::validator_set_history::authenticated_at(dir.path(), 1)
            .expect("original transition remains authoritative");
        assert_eq!(
            common::validator_snapshot_witness_hash(&retained),
            common::validator_snapshot_witness_hash(&valid),
            "alternate honest peer witness must not rewrite the first audit certificate"
        );

        let corrupt_dir = tempfile::tempdir()?;
        crate::validator_set_history::record(corrupt_dir.path(), 0, &validator_set)?;
        std::fs::write(
            corrupt_dir.path().join("validator-set-history.json"),
            b"{corrupt-history",
        )?;
        let local_error = validate_and_persist_transition(
            corrupt_dir.path(),
            valid,
            &validator_set,
            0,
            &tip_hash,
            None,
        )
        .expect_err("corrupt local history must fail closed");
        assert!(matches!(
            transition_persistence_failure_acceptance(&local_error),
            gossipsub::MessageAcceptance::Ignore
        ));
        Ok(())
    }

    #[tokio::test]
    async fn transition_receiver_installs_exact_configured_checkpoint_on_pruned_node(
    ) -> anyhow::Result<()> {
        let signing_keys: Vec<_> = (1u8..=4)
            .map(|seed| SigningKey::from_bytes(&[seed; 32]))
            .collect();
        let validator_set = common::ValidatorSet::new(
            signing_keys
                .iter()
                .enumerate()
                .map(|(index, key)| validator(&format!("node-{}", index + 1), key))
                .collect(),
        );
        let tip_hash = common::Block::genesis().hash();
        let checkpoint = signed_transition(
            &validator_set,
            &signing_keys,
            &[0, 1, 2],
            1,
            tip_hash.clone(),
        );
        let dir = tempfile::tempdir()?;
        let mut received_without_configured_witness = checkpoint.clone();
        received_without_configured_witness
            .transition_signatures
            .clear();

        // There is deliberately no prior-set history in this rebuilt node.
        // The arbitrary `previous` value proves the receiver takes the
        // configured trust-root path instead of quorum-validating against it.
        let acceptance = crate::validator_set_history::with_checkpoint_env_override(
            Some(serde_json::to_string(&checkpoint)?),
            async {
                validate_and_persist_transition(
                    dir.path(),
                    received_without_configured_witness,
                    &common::ValidatorSet::default(),
                    0,
                    &tip_hash,
                    Some(&checkpoint),
                )
            },
        )
        .await?;
        assert!(matches!(acceptance, gossipsub::MessageAcceptance::Ignore));

        let persisted = crate::validator_set_history::load_checked(dir.path())?;
        assert_eq!(persisted.len(), 1);
        assert_eq!(
            persisted[0]
                .certificate
                .as_ref()
                .map(common::validator_snapshot_certificate_hash),
            Some(common::validator_snapshot_certificate_hash(&checkpoint))
        );
        assert_eq!(
            persisted[0]
                .certificate
                .as_ref()
                .map(common::validator_snapshot_witness_hash),
            Some(common::validator_snapshot_witness_hash(&checkpoint)),
            "the configured witness must replace unverifiable received bytes"
        );
        Ok(())
    }

    #[test]
    fn validate_and_persist_transition_ignores_exact_replay_after_height_finalized(
    ) -> anyhow::Result<()> {
        let signing_keys: Vec<_> = (1u8..=4)
            .map(|seed| SigningKey::from_bytes(&[seed; 32]))
            .collect();
        let validator_set = common::ValidatorSet::new(
            signing_keys
                .iter()
                .enumerate()
                .map(|(index, key)| validator(&format!("node-{}", index + 1), key))
                .collect(),
        );
        let dir = tempfile::tempdir()?;
        crate::validator_set_history::record(dir.path(), 0, &validator_set)?;
        let tip_hash = common::Block::genesis().hash();
        let valid = signed_transition(
            &validator_set,
            &signing_keys,
            &[0, 1, 2],
            1,
            tip_hash.clone(),
        );
        validate_and_persist_transition(
            dir.path(),
            valid.clone(),
            &validator_set,
            0,
            &tip_hash,
            None,
        )?;

        let block_1_hash = "b".repeat(64);
        let replay = validate_and_persist_transition(
            dir.path(),
            valid.clone(),
            &validator_set,
            1,
            &block_1_hash,
            None,
        )?;
        assert!(matches!(replay, gossipsub::MessageAcceptance::Ignore));

        let mut conflicting = valid;
        conflicting.validators.validators[0].stake += 1;
        conflicting.validator_set_hash =
            common::canonical_validator_set_hash(&conflicting.validators);
        conflicting.transition_signatures.clear();
        let conflicting_message = common::validator_set_transition_message(&conflicting);
        for index in [0usize, 1, 2] {
            let v = &validator_set.validators[index];
            conflicting
                .transition_signatures
                .push(common::ValidatorSetTransitionSignature {
                    validator_id: v.id.clone(),
                    validator_pubkey: v.pubkey.clone(),
                    signature: hex::encode(
                        signing_keys[index]
                            .sign(conflicting_message.as_bytes())
                            .to_bytes(),
                    ),
                });
        }
        assert!(validate_and_persist_transition(
            dir.path(),
            conflicting,
            &validator_set,
            1,
            &block_1_hash,
            None,
        )
        .is_err());

        let unpersisted_past =
            signed_transition(&validator_set, &signing_keys, &[0, 1, 2], 1, tip_hash);
        let empty_dir = tempfile::tempdir()?;
        crate::validator_set_history::record(empty_dir.path(), 0, &validator_set)?;
        assert!(validate_and_persist_transition(
            empty_dir.path(),
            unpersisted_past,
            &validator_set,
            1,
            &block_1_hash,
            None,
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn validate_vote_gossip_accepts_known_validator() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-1", &signing_key)]);
        let vote = signed_vote(
            "node-1",
            &signing_key,
            hex::encode(signing_key.verifying_key().as_bytes()),
        );

        let result =
            validate_vote_gossip_message(&serde_json::to_vec(&vote).unwrap(), &validator_set)
                .unwrap();

        assert_eq!(result.validator_id, "node-1");
    }

    #[test]
    fn validate_vote_gossip_rejects_malformed_payload() {
        let validator_set = common::ValidatorSet::default();

        let error = validate_vote_gossip_message(br#"not-json"#, &validator_set).unwrap_err();

        assert_eq!(error, VoteValidationError::Malformed);
    }

    #[test]
    fn validate_vote_gossip_rejects_invalid_signature() {
        let validator_key = SigningKey::from_bytes(&[7u8; 32]);
        let attacker_key = SigningKey::from_bytes(&[8u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-1", &validator_key)]);
        let vote = signed_vote(
            "node-1",
            &attacker_key,
            hex::encode(validator_key.verifying_key().as_bytes()),
        );

        let error =
            validate_vote_gossip_message(&serde_json::to_vec(&vote).unwrap(), &validator_set)
                .unwrap_err();

        assert_eq!(error, VoteValidationError::SignatureVerificationFailed);
    }

    #[test]
    fn validate_vote_gossip_rejects_unknown_validator() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-1", &signing_key)]);
        let vote = signed_vote(
            "unknown-node",
            &signing_key,
            hex::encode(signing_key.verifying_key().as_bytes()),
        );

        let error =
            validate_vote_gossip_message(&serde_json::to_vec(&vote).unwrap(), &validator_set)
                .unwrap_err();

        assert_eq!(error, VoteValidationError::UnknownValidator);
    }

    #[test]
    fn validate_vote_gossip_rejects_invalid_pubkey_hex() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-1", &signing_key)]);
        let vote = signed_vote("node-1", &signing_key, "not-valid-hex".into());

        let error =
            validate_vote_gossip_message(&serde_json::to_vec(&vote).unwrap(), &validator_set)
                .unwrap_err();

        assert_eq!(error, VoteValidationError::InvalidValidatorPubkeyHex);
    }

    fn pbft_phase_signature(phase: &str, block_hash: &str, signing_key: &SigningKey) -> String {
        let message = consensus::pbft::pbft_signature_message(phase, block_hash);
        hex::encode(signing_key.sign(message.as_bytes()).to_bytes())
    }

    #[test]
    fn validate_pbft_prepare_accepts_domain_separated_signature() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-pbft", &signing_key)]);
        let block_hash = "abc123blockhash";
        let signature = pbft_phase_signature("prepare", block_hash, &signing_key);

        let result = validate_pbft_phase_message(
            &validator_set,
            "prepare",
            block_hash,
            "node-pbft",
            &signature,
        )
        .unwrap();

        assert_eq!(result.validator_id, "node-pbft");
        assert_eq!(result.signature, signature);
    }

    #[test]
    fn validate_pbft_pre_prepare_binds_tip_height_merkle_and_validator_authority() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-pbft", &signing_key)]);
        let tip_hash = common::Block::genesis().hash();
        let now = chrono::DateTime::from_timestamp(1_900_000_000, 0).unwrap();
        let tip_timestamp = chrono::DateTime::from_timestamp(1_899_999_999, 0).unwrap();
        let block = common::Block {
            header: common::BlockHeader {
                height: 1,
                prev_hash: tip_hash.clone(),
                merkle_root: common::merkle_root(&[]),
                proposer_id: "node-pbft".into(),
                timestamp: now,
                validator_set_hash: common::canonical_validator_set_hash(&validator_set),
                vrf_output: None,
                vrf_proof: None,
                view_number: 0,
                view_change_certificate: vec![],
                status_root: None,
            },
            transactions: Vec::new(),
            pbft_signatures: Vec::new(),
        };
        validate_pbft_pre_prepare_block(&block, &validator_set, 0, &tip_hash, tip_timestamp, now)
            .expect("canonical next-height proposal is structurally valid");

        let mut wrong_fork = block.clone();
        wrong_fork.header.prev_hash = "f".repeat(64);
        assert!(validate_pbft_pre_prepare_block(
            &wrong_fork,
            &validator_set,
            0,
            &tip_hash,
            tip_timestamp,
            now,
        )
        .is_err());

        let mut wrong_authority = block.clone();
        wrong_authority.header.validator_set_hash = "e".repeat(64);
        assert!(validate_pbft_pre_prepare_block(
            &wrong_authority,
            &validator_set,
            0,
            &tip_hash,
            tip_timestamp,
            now,
        )
        .is_err());

        let mut wrong_merkle = block;
        wrong_merkle.header.merkle_root = "d".repeat(64);
        assert!(validate_pbft_pre_prepare_block(
            &wrong_merkle,
            &validator_set,
            0,
            &tip_hash,
            tip_timestamp,
            now,
        )
        .is_err());
    }

    #[test]
    fn validate_pbft_pre_prepare_rejects_backdated_and_future_timestamps() {
        let signing_key = SigningKey::from_bytes(&[10u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-time", &signing_key)]);
        let tip_hash = common::Block::genesis().hash();
        let now = chrono::DateTime::from_timestamp(1_900_000_000, 0).unwrap();
        let tip_timestamp = chrono::DateTime::from_timestamp(1_899_999_800, 0).unwrap();
        let mut block = common::Block {
            header: common::BlockHeader {
                height: 1,
                prev_hash: tip_hash.clone(),
                merkle_root: common::merkle_root(&[]),
                proposer_id: "node-time".into(),
                timestamp: now,
                validator_set_hash: common::canonical_validator_set_hash(&validator_set),
                vrf_output: None,
                vrf_proof: None,
                view_number: 0,
                view_change_certificate: vec![],
                status_root: None,
            },
            transactions: Vec::new(),
            pbft_signatures: Vec::new(),
        };

        block.header.timestamp = chrono::DateTime::from_timestamp(now.timestamp() - 61, 0).unwrap();
        assert!(
            validate_pbft_pre_prepare_block(
                &block,
                &validator_set,
                0,
                &tip_hash,
                tip_timestamp,
                now,
            )
            .is_err(),
            "a monotonic but stale proposal must not revive expired authorizations"
        );

        block.header.timestamp = chrono::DateTime::from_timestamp(now.timestamp() + 61, 0).unwrap();
        assert!(
            validate_pbft_pre_prepare_block(
                &block,
                &validator_set,
                0,
                &tip_hash,
                tip_timestamp,
                now,
            )
            .is_err(),
            "a far-future proposal must not poison subsequent chain time"
        );

        block.header.timestamp = tip_timestamp;
        assert!(
            validate_pbft_pre_prepare_block(
                &block,
                &validator_set,
                0,
                &tip_hash,
                tip_timestamp,
                now,
            )
            .is_err(),
            "a child timestamp must be strictly later than its parent"
        );
    }

    #[test]
    fn validate_pbft_prepare_rejects_wrong_signature_domain() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-pbft", &signing_key)]);
        let block_hash = "abc123blockhash";
        // Signed for "commit" but presented as "prepare".
        let wrong_domain_sig = pbft_phase_signature("commit", block_hash, &signing_key);

        let error = validate_pbft_phase_message(
            &validator_set,
            "prepare",
            block_hash,
            "node-pbft",
            &wrong_domain_sig,
        )
        .unwrap_err();

        assert_eq!(error, PbftValidationError::SignatureVerificationFailed);
    }

    #[test]
    fn validate_pbft_commit_rejects_unknown_validator() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-pbft", &signing_key)]);
        let signature = pbft_phase_signature("commit", "block-1", &signing_key);

        let error = validate_pbft_phase_message(
            &validator_set,
            "commit",
            "block-1",
            "missing-validator",
            &signature,
        )
        .unwrap_err();

        assert_eq!(error, PbftValidationError::UnknownValidator);
    }

    #[test]
    fn validate_pbft_prepare_rejects_invalid_signature_hex() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-pbft", &signing_key)]);

        let error =
            validate_pbft_phase_message(&validator_set, "prepare", "block-1", "node-pbft", "zzzz")
                .unwrap_err();

        assert_eq!(error, PbftValidationError::SignatureVerificationFailed);
    }

    #[tokio::test]
    async fn validated_gossip_vote_updates_package_round_state() -> anyhow::Result<()> {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-1", &signing_key)]);
        let vote = signed_vote(
            "node-1",
            &signing_key,
            hex::encode(signing_key.verifying_key().as_bytes()),
        );
        let state = make_test_state(validator_set).await?;

        record_validated_vote_gossip(&state, &vote).await;

        let state = state.read().await;
        let round = state
            .package_round(&vote.consensus_subject)
            .expect("validated gossip vote should update package round state");

        assert_eq!(round.vote_count(), 1);
        assert_eq!(round.signatures()[0].validator_id, "node-1");
        assert_eq!(round.signatures()[0].ml_model_version, "creg-detect-v1.0.0");

        Ok(())
    }

    fn reserve_listen_addr() -> String {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .expect("should reserve a loopback port for the p2p test");
        let port = listener
            .local_addr()
            .expect("reserved listener should have a local address")
            .port();
        drop(listener);
        format!("/ip4/127.0.0.1/tcp/{}", port)
    }

    #[tokio::test]
    async fn admission_gossip_delivers_a_validated_staking_wallet_signature() -> anyhow::Result<()>
    {
        use crate::consensus_admission::{
            gossip_payload, sign_attestation, AttestationStore, TOPIC,
        };
        use alloy::primitives::{Address, U256};
        use alloy::signers::local::PrivateKeySigner;

        let wallet_seed = "01".repeat(32);
        let wallet: PrivateKeySigner = wallet_seed.parse()?;
        let staking = Address::repeat_byte(0x42);
        let applicant = Address::repeat_byte(0x12);
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let mut signer = validator("node-1", &signing_key);
        signer.eth_address = format!("0x{:040x}", wallet.address());
        let validators = common::ValidatorSet::new(vec![signer]);
        let state_a = make_test_state(validators.clone()).await?;
        let state_b = make_test_state(validators).await?;
        {
            let mut receiver = state_b.write().await;
            receiver.config.staking_addr = format!("0x{:040x}", staking);
            receiver.validator_registrations.insert(
                format!("0x{:040x}", applicant),
                crate::state::ValidatorRegistrationStatus {
                    registered_with_node: true,
                    staking_state: "pending".into(),
                    ..Default::default()
                },
            );
        }
        let listen_a = reserve_listen_addr();
        let listen_b = reserve_listen_addr();
        let (node_a, handle_a) = P2PNode::new(&listen_a)?;
        let (node_b, _handle_b) = P2PNode::new(&listen_b)?;
        let store = AttestationStore::for_chain(Some(31337));
        let task_a = tokio::spawn(node_a.run(
            state_a,
            crate::events::new_event_bus(),
            AttestationStore::new(),
        ));
        let task_b =
            tokio::spawn(node_b.run(state_b, crate::events::new_event_bus(), store.clone()));
        tokio::time::sleep(Duration::from_millis(200)).await;
        handle_a
            .sender
            .send(P2PCommand::Dial {
                addr: listen_b.parse()?,
            })
            .await?;
        let attestation = sign_attestation(
            &wallet_seed,
            31337,
            staking,
            applicant,
            U256::from(100u64),
            U256::from(42u64),
            U256::from(1u64),
            U256::from(4u64),
        )
        .await?;
        let result = tokio::time::timeout(Duration::from_secs(6), async {
            while !store.contains(&attestation).await {
                handle_a
                    .sender
                    .send(P2PCommand::Broadcast {
                        topic: TOPIC.into(),
                        data: gossip_payload(attestation.clone())?,
                    })
                    .await?;
                tokio::time::sleep(Duration::from_millis(350)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await;
        task_a.abort();
        task_b.abort();
        result??;
        Ok(())
    }

    #[tokio::test]
    async fn gossipsub_vote_broadcast_updates_peer_round_state() -> anyhow::Result<()> {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let validator_set = common::ValidatorSet::new(vec![validator("node-1", &signing_key)]);
        let vote = signed_vote(
            "node-1",
            &signing_key,
            hex::encode(signing_key.verifying_key().as_bytes()),
        );

        let listen_addr_a = reserve_listen_addr();
        let listen_addr_b = reserve_listen_addr();
        let (node_a, handle_a) = P2PNode::new(&listen_addr_a)?;
        let (node_b, _handle_b) = P2PNode::new(&listen_addr_b)?;
        let state_a = make_test_state(validator_set.clone()).await?;
        let state_b = make_test_state(validator_set).await?;
        let event_bus_a = crate::events::new_event_bus();
        let event_bus_b = crate::events::new_event_bus();

        let task_a = tokio::spawn(node_a.run(
            state_a,
            event_bus_a,
            crate::consensus_admission::AttestationStore::new(),
        ));
        let task_b = tokio::spawn(node_b.run(
            state_b.clone(),
            event_bus_b,
            crate::consensus_admission::AttestationStore::new(),
        ));

        tokio::time::sleep(Duration::from_millis(200)).await;
        handle_a
            .sender
            .send(P2PCommand::Dial {
                addr: listen_addr_b.parse()?,
            })
            .await?;

        tokio::time::sleep(Duration::from_secs(1)).await;
        handle_a
            .sender
            .send(P2PCommand::Broadcast {
                topic: "creg/v1/votes".into(),
                data: serde_json::to_vec(&vote)?,
            })
            .await?;

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let maybe_round = {
                    let state = state_b.read().await;
                    state.package_round(&vote.consensus_subject).cloned()
                };

                if let Some(round) = maybe_round {
                    assert_eq!(round.vote_count(), 1);
                    assert_eq!(round.signatures()[0].validator_id, "node-1");
                    break;
                }

                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("gossipsub vote should reach the peer within the timeout");

        task_a.abort();
        task_b.abort();
        let _ = task_a.await;
        let _ = task_b.await;

        Ok(())
    }

    #[tokio::test]
    async fn gossipsub_signed_transition_is_independently_persisted_by_peer() -> anyhow::Result<()>
    {
        let signing_keys: Vec<_> = (1u8..=4)
            .map(|seed| SigningKey::from_bytes(&[seed; 32]))
            .collect();
        let validator_set = common::ValidatorSet::new(
            signing_keys
                .iter()
                .enumerate()
                .map(|(index, key)| validator(&format!("node-{}", index + 1), key))
                .collect(),
        );
        let state_a = make_test_state(validator_set.clone()).await?;
        let state_b = make_test_state(validator_set.clone()).await?;
        let anchor_hash = { state_a.read().await.chain.tip_hash()? };
        let transition =
            signed_transition(&validator_set, &signing_keys, &[0, 1, 2], 1, anchor_hash);

        let listen_addr_a = reserve_listen_addr();
        let listen_addr_b = reserve_listen_addr();
        let (node_a, handle_a) = P2PNode::new(&listen_addr_a)?;
        let (node_b, _handle_b) = P2PNode::new(&listen_addr_b)?;
        let event_bus_a = crate::events::new_event_bus();
        let event_bus_b = crate::events::new_event_bus();
        let receiver_data_dir = state_b.read().await.config.data_dir.clone();

        let task_a = tokio::spawn(node_a.run(
            state_a,
            event_bus_a,
            crate::consensus_admission::AttestationStore::new(),
        ));
        let task_b = tokio::spawn(node_b.run(
            state_b,
            event_bus_b,
            crate::consensus_admission::AttestationStore::new(),
        ));

        tokio::time::sleep(Duration::from_millis(200)).await;
        handle_a
            .sender
            .send(P2PCommand::Dial {
                addr: listen_addr_b.parse()?,
            })
            .await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        handle_a
            .sender
            .send(P2PCommand::Broadcast {
                topic: crate::validator_registry_gossip::VALIDATOR_SET_TRANSITIONS_TOPIC.into(),
                data: serde_json::to_vec(&common::GossipMessage::ValidatorSetTransition {
                    snapshot: transition.clone(),
                })?,
            })
            .await?;

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(received) =
                    crate::validator_set_history::authenticated_at(&receiver_data_dir, 1)
                {
                    assert_eq!(
                        common::validator_snapshot_certificate_hash(&received),
                        common::validator_snapshot_certificate_hash(&transition)
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("signed transition should reach and be persisted by the peer");

        task_a.abort();
        task_b.abort();
        let _ = task_a.await;
        let _ = task_b.await;

        Ok(())
    }

    #[test]
    fn gossipsub_message_id_separates_topics_and_is_collision_resistant() {
        let id_votes = gossipsub_message_id("creg/v1/votes", b"payload");
        let id_blocks = gossipsub_message_id("creg/v1/blocks", b"payload");
        let id_other = gossipsub_message_id("creg/v1/votes", b"payload-2");
        assert_ne!(id_votes, id_blocks);
        assert_ne!(id_votes, id_other);
        assert_eq!(id_votes.0.len(), 32);
    }

    #[test]
    fn rate_limiter_grants_authenticated_validator_multiplier() {
        let cfg = P2PRateLimitConfig {
            vote_limit: 1,
            ..Default::default()
        };
        let limiter = P2PRateLimiter::new(cfg);
        let validators = vec![("val-1".to_string(), 100)];
        let anon_peer = PeerId::random();
        assert!(limiter.check_vote_for_validator(anon_peer, None, &validators));
        assert!(!limiter.check_vote_for_validator(anon_peer, None, &validators));

        let val_peer = PeerId::random();
        for _ in 0..5 {
            assert!(limiter.check_vote_for_validator(val_peer, Some("val-1"), &validators));
        }
        assert!(!limiter.check_vote_for_validator(val_peer, Some("val-1"), &validators));
    }
}
