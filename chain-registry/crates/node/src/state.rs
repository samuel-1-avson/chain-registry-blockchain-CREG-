// crates/node/src/state.rs
// Shared NodeState and associated types.  Factored into its own module so
// that the library target (lib.rs) and binary target (main.rs) can both
// include it, enabling integration tests to construct and inspect the state.

use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};

use alloy::primitives::{Address, U256};
pub use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::RwLock;

use common::ValidatorIdentity;

use crate::{
    chain_store::ChainStore, config::NodeConfig, pending_pool::PendingPool,
    publisher_index::PublisherIndex,
};

pub use consensus::forced_inclusion::ForcedInclusionTracker;
pub use consensus::pbft::PbftEngine;

// ─── Validator registration ───────────────────────────────────────────────────

#[derive(Serialize, Clone, Debug, Default)]
pub struct ValidatorRegistrationStatus {
    pub alias: String,
    pub identity: ValidatorIdentity,
    pub registered_with_node: bool,
    pub applied_on_chain: bool,
    pub governance_approved: bool,
    pub admitted_to_consensus: bool,
    pub active: bool,
    pub staking_state: String,
    pub status: String,
    pub stake: u64,
    pub reputation: u32,
    pub last_error: Option<String>,
    pub last_synced_at: Option<String>,
}

pub fn normalized_validator_key(evm_address: &str) -> String {
    evm_address.trim().to_ascii_lowercase()
}

pub fn validator_registration_status_text(registration: &ValidatorRegistrationStatus) -> String {
    if registration.active {
        "active".to_string()
    } else if registration.admitted_to_consensus {
        "admitted-to-consensus".to_string()
    } else if registration.governance_approved {
        "governance-approved".to_string()
    } else if registration.applied_on_chain {
        "applied-on-chain".to_string()
    } else if registration.registered_with_node {
        "identity-registered".to_string()
    } else {
        "unregistered".to_string()
    }
}

// ─── Live status snapshots ─────────────────────────────────────────────────────

#[derive(Serialize, Clone, Default)]
pub struct P2PStatus {
    pub peers: Vec<String>,
    pub protocols: Vec<String>,
    /// Gossip vote messages rejected at validation boundary (invalid sig/key/evidence).
    pub gossip_votes_rejected_total: u64,
    /// PBFT Prepare/Commit gossip messages rejected at validation boundary.
    pub gossip_pbft_rejected_total: u64,
}

#[derive(Serialize, Clone, Default)]
pub struct BridgeStatus {
    pub last_finalized_eth_block: u64,
    pub registry_address: String,
    pub bridge_sync_status: String,
    pub current_state_root: String,
    /// L1 transaction hash of the most recent anchor commit (the governance
    /// vote/execute transaction that landed `submitRollupBatch`).
    pub last_anchor_tx_hash: Option<String>,
    /// RFC 3339 timestamp of the most recent anchor commit.
    pub last_anchor_at: Option<String>,
    /// The most recent L1 block reported as *finalized* by the RPC
    /// (`eth_getBlockByNumber("finalized")`), as opposed to
    /// `last_finalized_eth_block` which historically recorded the head.
    pub finalized_l1_block: Option<u64>,
    /// Honesty label for the anchoring trust model. The current Groth16
    /// batch circuit only proves the batch is non-empty — state roots are
    /// computed off-chain and trusted from the bridge operator — so this is
    /// a checkpoint attestation, not a validity proof.
    pub proof_mode: String,
    /// Number of anchor commits persisted in the local anchor journal.
    pub anchor_count: usize,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ValidatorSetSyncStatus {
    pub enabled: bool,
    pub mode: String,
    pub state: String,
    pub last_finalized_source_block: Option<u64>,
    pub cursor_block: Option<u64>,
    pub cursor_log_index: Option<u32>,
    pub cursor_block_hash: Option<String>,
    pub last_poll_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ReorgEvent {
    pub id: String,
    pub timestamp: String,
    pub depth: u64,
    pub abandoned_blocks: Vec<String>,
    pub new_tip: String,
}

/// Maximum number of reorg events retained in memory (and served by
/// `/v1/reorgs`). Oldest entries are dropped first.
pub const MAX_REORG_EVENTS: usize = 100;

#[derive(Clone, Debug)]
pub struct PackageRound {
    pub subject: String,
    pub first_vote_at: DateTime<Utc>,
    pub last_vote_at: DateTime<Utc>,
    signatures: Vec<common::ValidatorSignature>,
}

impl PackageRound {
    pub fn from_vote(subject: impl Into<String>, vote: common::ValidatorSignature) -> Self {
        let mut round = Self {
            subject: subject.into(),
            first_vote_at: Utc::now(),
            last_vote_at: Utc::now(),
            signatures: vec![vote],
        };
        round.refresh_timestamps();
        round
    }

    pub fn record_vote(&mut self, vote: common::ValidatorSignature) {
        let unique_key = package_round_vote_key(&vote);
        if let Some(existing) = self
            .signatures
            .iter_mut()
            .find(|existing| package_round_vote_key(existing) == unique_key)
        {
            *existing = vote;
        } else {
            self.signatures.push(vote);
        }
        self.refresh_timestamps();
    }

    pub fn signatures(&self) -> &[common::ValidatorSignature] {
        &self.signatures
    }

    pub fn vote_count(&self) -> usize {
        self.signatures.len()
    }

    fn refresh_timestamps(&mut self) {
        let Some(first_vote_at) = self
            .signatures
            .iter()
            .map(|signature| signature.signed_at.clone())
            .min()
        else {
            let now = Utc::now();
            self.first_vote_at = now;
            self.last_vote_at = now;
            return;
        };

        self.first_vote_at = first_vote_at;
        self.last_vote_at = self
            .signatures
            .iter()
            .map(|signature| signature.signed_at.clone())
            .max()
            .unwrap_or_else(Utc::now);
    }
}

fn package_round_vote_key(signature: &common::ValidatorSignature) -> String {
    if signature.validator_pubkey.is_empty() {
        format!("id:{}", signature.validator_id.to_ascii_lowercase())
    } else {
        format!("pubkey:{}", signature.validator_pubkey.to_ascii_lowercase())
    }
}

// ─── NodeState ─────────────────────────────────────────────────────────────────

/// Maximum number of tracked live package rounds in memory (VNA-28).
pub const MAX_PACKAGE_ROUNDS: usize = 4_096;
/// Maximum cached publisher stake balances in memory (VNA-28).
pub const MAX_PUBLISHER_STAKE_CACHE: usize = 10_000;
/// Maximum cached VRF proofs per epoch in memory.
pub const MAX_VRF_PROOFS_PER_EPOCH: usize = 2_048;

/// Background-refreshed health snapshot served lock-free by `GET /v1/health` (VNA-28).
#[derive(Serialize, Clone)]
pub struct NodeHealthSnapshot {
    pub node_id: String,
    pub is_validator: bool,
    pub version: String,
    pub validator_set_sync: ValidatorSetSyncStatus,
    pub sync_lag_blocks: u64,
    pub sync_max_peer_tip: u64,
    pub sandbox: validator::sandbox::SandboxStatus,
}

impl Default for NodeHealthSnapshot {
    fn default() -> Self {
        Self {
            node_id: String::new(),
            is_validator: false,
            version: env!("CARGO_PKG_VERSION").to_string(),
            validator_set_sync: ValidatorSetSyncStatus::default(),
            sync_lag_blocks: 0,
            sync_max_peer_tip: 0,
            sandbox: validator::sandbox::SandboxStatus {
                engine: "initializing".into(),
                isolated: false,
                dev_bypass: false,
                degraded: false,
                note: "Initializing sandbox probe".into(),
            },
        }
    }
}

/// Shared mutable state passed to every subsystem via `Arc<RwLock<_>>`.
pub struct NodeState {
    pub chain: ChainStore,
    pub pending_pool: PendingPool,
    pub publisher_index: PublisherIndex,
    pub validator_set_bootstrap: common::ValidatorSet,
    pub validator_set: common::ValidatorSet,
    /// Wait-free read snapshot of the active validator set for hot paths (P2P / verifiers).
    pub validator_set_snapshot: Arc<ArcSwap<common::ValidatorSet>>,
    /// Live package-consensus rounds keyed by the current consensus subject.
    pub package_rounds: HashMap<String, PackageRound>,

    // Committee info is primarily stored in PendingPool per canonical.
    // This map can cache the committee for fast lookup during voting windows.
    pub package_committees: HashMap<String, common::AnalysisCommittee>,
    pub config: NodeConfig,
    // Live metrics for the Explorer UI
    pub p2p_status: P2PStatus,
    pub bridge_status: BridgeStatus,
    /// Cached VRF proofs from other validators: validator_id → (output, proof).
    pub vrf_proofs: HashMap<String, (String, String)>,
    /// Decryption shares received from peers: canonical → Vec<KeyShare>.
    pub decryption_shares: HashMap<String, Vec<threshold_encryption::KeyShare>>,
    /// Validator registrations keyed by canonical EVM address.
    pub validator_registrations: HashMap<String, ValidatorRegistrationStatus>,
    pub validator_set_sync: ValidatorSetSyncStatus,
    /// View-change certificates accumulated from peers.
    /// Outer key: block_hash. Middle key: proposed new_view number.
    /// Inner set: validator IDs that have sent a certificate for this (block, view).
    ///
    /// A view-change is applied once ⌊n/3⌋+1 certificates are received,
    /// preventing a single Byzantine node from forcing a view-change.
    pub view_change_certs: HashMap<String, HashMap<u32, std::collections::HashSet<String>>>,
    pub reorgs: Vec<ReorgEvent>,
    /// The PBFT consensus engine managing block finalization.
    pub pbft_engine: PbftEngine,
    /// Tracks finalized transactions awaiting forced inclusion after delay.
    pub forced_inclusion_tracker: ForcedInclusionTracker,
    /// Blocks behind the highest known peer tip (updated by sync loop).
    pub sync_lag_blocks: u64,
    /// Highest chain tip observed from configured peers during sync.
    pub sync_max_peer_tip: u64,
    // Reward emission counters (for §9.12 incentives observability)
    /// Successful L1 reward emissions (distributeWorkRewards calls that succeeded)
    pub reward_emissions_succeeded: u64,
    /// Failed L1 reward emissions (including tx send/watch errors)
    pub reward_emissions_failed: u64,
    /// Reward jobs skipped because no finalized durable dispatcher is configured.
    pub reward_emissions_skipped: u64,
    // Live treasury / incentives values (populated via small L1 queries for monitoring)
    pub treasury_balance: u128,
    pub pending_rewards_estimate: u128,
    // More views for incentives monitoring (§9.12)
    pub total_rewards_distributed: u128,
    pub rewards_pool_balance: u128,
    pub last_incentives_refresh: Option<u64>,
    /// In-memory cache of publisher staked balances: Address -> (staked_balance, cached_at).
    pub publisher_stake_cache: HashMap<Address, (U256, Instant)>,
    /// Detection-only watch for validators signing conflicting PBFT votes
    /// (ADR-016). Disabled unless `CREG_EQUIVOCATION_DETECTION` is set.
    pub equivocation: crate::equivocation_monitor::EquivocationMonitor,
}

/// Default time-to-live for in-memory publisher stake cache entries.
pub const PUBLISHER_STAKE_CACHE_TTL: Duration = Duration::from_secs(300);

impl NodeState {
    /// Publish the current `validator_set` into the wait-free `ArcSwap` snapshot.
    pub fn sync_validator_set_snapshot(&self) {
        self.validator_set_snapshot
            .store(Arc::new(self.validator_set.clone()));
    }

    /// Replace `validator_set` and immediately publish the lock-free snapshot.
    pub fn set_validator_set(&mut self, next: common::ValidatorSet) {
        self.validator_set = next;
        self.sync_validator_set_snapshot();
    }

    /// Retrieve cached publisher stake if present and unexpired.
    pub fn get_cached_publisher_stake(&self, publisher: &Address) -> Option<U256> {
        let (staked, cached_at) = self.publisher_stake_cache.get(publisher)?;
        if cached_at.elapsed() <= PUBLISHER_STAKE_CACHE_TTL {
            Some(*staked)
        } else {
            None
        }
    }

    /// Insert or update publisher stake balance in the in-memory cache.
    pub fn set_cached_publisher_stake(&mut self, publisher: Address, staked: U256) {
        if !self.publisher_stake_cache.contains_key(&publisher)
            && self.publisher_stake_cache.len() >= MAX_PUBLISHER_STAKE_CACHE
        {
            self.publisher_stake_cache
                .retain(|_, (_, ts)| ts.elapsed() <= PUBLISHER_STAKE_CACHE_TTL);
            if self.publisher_stake_cache.len() >= MAX_PUBLISHER_STAKE_CACHE {
                if let Some(oldest_key) = self
                    .publisher_stake_cache
                    .iter()
                    .min_by_key(|(_, (_, ts))| *ts)
                    .map(|(k, _)| *k)
                {
                    self.publisher_stake_cache.remove(&oldest_key);
                }
            }
        }
        self.publisher_stake_cache
            .insert(publisher, (staked, Instant::now()));
    }

    /// Invalidate/evict cached publisher stake for a specific address.
    pub fn invalidate_publisher_stake(&mut self, publisher: &Address) {
        self.publisher_stake_cache.remove(publisher);
    }
    /// Record a chain reorganization event so it is visible via `/v1/reorgs`
    /// and the explorer Reorgs tab. Newest events are kept at the front.
    pub fn record_reorg(
        &mut self,
        depth: u64,
        abandoned_blocks: Vec<String>,
        new_tip: impl Into<String>,
    ) {
        let new_tip = new_tip.into();
        let event = ReorgEvent {
            id: format!("reorg-{}", uuid::Uuid::new_v4()),
            timestamp: Utc::now().to_rfc3339(),
            depth,
            abandoned_blocks,
            new_tip: new_tip.clone(),
        };
        tracing::warn!(
            depth = depth,
            new_tip = %new_tip,
            "Chain reorganization recorded ({} abandoned blocks)",
            event.abandoned_blocks.len()
        );
        self.reorgs.insert(0, event);
        self.reorgs.truncate(MAX_REORG_EVENTS);
    }

    pub fn record_package_vote(
        &mut self,
        subject: impl Into<String>,
        vote: common::ValidatorSignature,
    ) {
        let subject = subject.into();
        let repeated = self.package_rounds.get(&subject).is_some_and(|round| {
            round.signatures().iter().any(|previous| {
                previous.validator_id == vote.validator_id && previous.signature == vote.signature
            })
        });
        let event = crate::events::RegistryEvent::validator_voted(
            &vote.validator_id,
            &subject,
            matches!(&vote.vote, common::ValidatorVote::Approve),
        );
        if let Some(round) = self.package_rounds.get_mut(&subject) {
            round.record_vote(vote);
        } else {
            while self.package_rounds.len() >= MAX_PACKAGE_ROUNDS {
                let oldest = self
                    .package_rounds
                    .iter()
                    .min_by_key(|(_, r)| r.first_vote_at)
                    .map(|(k, _)| k.clone());
                match oldest {
                    Some(k) => {
                        self.package_rounds.remove(&k);
                        self.package_committees.remove(&k);
                    }
                    None => break,
                }
            }
            self.package_rounds
                .insert(subject.clone(), PackageRound::from_vote(subject, vote));
        }
        if !repeated {
            self.chain.emit_registry_event(event);
        }
    }

    pub fn package_round(&self, subject: &str) -> Option<&PackageRound> {
        self.package_rounds.get(subject)
    }

    pub fn clear_package_round(&mut self, subject: &str) {
        self.package_rounds.remove(subject);
    }

    /// Reconcile only rounds whose exact block is already canonically stored.
    /// Future or conflicting prepared values remain retained for recovery.
    pub(crate) fn reconcile_committed_pbft_rounds(&mut self) -> anyhow::Result<usize> {
        let tip = self.chain.tip_height()?;
        let mut reconciled = 0;
        for round in self.pbft_engine.observability_snapshot() {
            if round.block_height > tip || round.phase == "finalised" {
                continue;
            }
            let block = self
                .chain
                .get_block_by_height(round.block_height)?
                .ok_or_else(|| anyhow::anyhow!("canonical PBFT reconciliation block is missing"))?;
            if block.hash() != round.block_hash {
                if round.phase == "failed" && round.prepare_count < round.quorum {
                    continue;
                }
                anyhow::bail!(
                    "retained PBFT round conflicts with canonical height {}",
                    round.block_height
                );
            }
            let authority = crate::validator_set_history::verification_set_at(
                &self.config.data_dir,
                block.header.height,
                &self.validator_set_bootstrap,
            )
            .map_err(anyhow::Error::msg)?;
            crate::sync::verify_block_consensus_certificate(&block, &authority)?;
            if self.pbft_engine.observe_finalized_block(&block)? {
                reconciled += 1;
            }
        }
        self.pbft_engine.cleanup_stale_rounds();
        Ok(reconciled)
    }

    /// Observers do not run the validator pipeline's admission cleanup.
    /// Retire an admission only when its exact artifact and publisher appear
    /// in a certified block still mapped to the local canonical chain.
    pub(crate) fn reconcile_committed_pending_packages(&mut self) -> anyhow::Result<usize> {
        let mut retired = 0;
        for canonical in self.pending_pool.all_canonicals() {
            let Some(entry) = self.pending_pool.get(&canonical) else {
                continue;
            };
            let request = entry.request.clone();
            let Some(record) = self.chain.get_package(&canonical)? else {
                continue;
            };
            if record.status == common::PackageStatus::Pending
                || record.content_hash != request.content_hash
            {
                continue;
            }
            let Some(block) = self.chain.get_block_by_hash(&record.block_hash)? else {
                continue;
            };
            if self
                .chain
                .get_block_by_height(block.header.height)?
                .is_none_or(|stored| stored.hash() != block.hash())
            {
                continue;
            }
            let contains_admission =
                block
                    .transactions
                    .iter()
                    .any(|transaction| match transaction {
                        common::Transaction::Publish(published)
                        | common::Transaction::PublishAuthorizedV1 {
                            record: published, ..
                        } => {
                            published.id.canonical() == canonical
                                && published.content_hash == request.content_hash
                                && published.publisher_pubkey == request.publisher_pubkey
                        }
                        _ => false,
                    });
            if !contains_admission {
                continue;
            }
            let authority = crate::validator_set_history::verification_set_at(
                &self.config.data_dir,
                block.header.height,
                &self.validator_set_bootstrap,
            )
            .map_err(anyhow::Error::msg)?;
            crate::sync::verify_block_consensus_certificate(&block, &authority)?;
            if self.pending_pool.remove_durable(&canonical)?.is_some() {
                self.clear_package_round(&canonical);
                retired += 1;
                tracing::info!(canonical = %canonical, block_hash = %block.hash(),
                    "Retired pending admission already committed on the canonical chain");
            }
        }
        Ok(retired)
    }

    /// Apply bookkeeping and reconcile certified PBFT finality after persistence.
    pub fn on_block_committed(&mut self, block: &common::Block) {
        if let Err(error) = self.reconcile_committed_pending_packages() {
            tracing::error!(error = %error, "Canonical admission retirement failed; retry pending");
        }
        if let Err(error) = self.reconcile_committed_pbft_rounds() {
            tracing::error!(error = %error, "[PBFT] canonical round reconciliation failed; retained evidence unchanged");
        }
        let mut updated_validator_scores = false;
        let committed_secs = block.header.timestamp.timestamp().max(0) as u64;
        for tx in &block.transactions {
            if let Some(record) = tx.published_record() {
                if let Some(committee) = record.analysis_committee.as_ref() {
                    crate::package_admission::update_performance_scores(
                        &mut self.validator_set,
                        &record.validator_signatures,
                        committee,
                        true,
                        committed_secs,
                    );
                    updated_validator_scores = true;
                }
            }
        }
        if updated_validator_scores {
            self.sync_validator_set_snapshot();
        }
        let hashes = common::block_transaction_hashes(block);
        self.forced_inclusion_tracker.mark_included(&hashes);
        if let Some(evidence) = self.forced_inclusion_tracker.check_block(
            &block.header.proposer_id,
            block.header.height,
            &hashes,
        ) {
            tracing::warn!(
                proposer = %evidence.proposer_id,
                height = evidence.block_height,
                omitted = evidence.omitted_tx_hashes.len(),
                "[CENSORSHIP] Forced-inclusion transactions omitted from block"
            );
        }
        self.chain.emit_committed_events(block);
        self.equivocation.prune(block.header.height);
    }

    /// Detection only (ADR-016): remember a PBFT proposal header seen on the
    /// wire. Call before validation, so proposals PBFT rejects are kept too.
    pub fn observe_proposal_for_equivocation(&mut self, header: &common::BlockHeader) {
        if !self.equivocation.is_enabled() {
            return;
        }
        let tip_height = self.chain.tip_height().unwrap_or(0);
        let evidence = self.equivocation.observe_header(header, tip_height);
        report_equivocation(&evidence);
    }

    /// Detection only (ADR-016): record a PBFT vote that ingress validated.
    pub fn observe_vote_for_equivocation(
        &mut self,
        phase: consensus::slashing_relay::VotePhase,
        block_hash: &str,
        signature: &common::BlockSignature,
    ) {
        if !self.equivocation.is_enabled() {
            return;
        }
        let evidence = self.equivocation.observe_vote(phase, block_hash, signature);
        report_equivocation(&evidence);
    }
}

fn report_equivocation(evidence: &[consensus::slashing_relay::EquivocationEvidence]) {
    for evidence in evidence {
        tracing::error!(
            validator = %evidence.validator_id,
            phase = evidence.phase.as_str(),
            height = evidence.height,
            evidence_id = %evidence.evidence_id,
            "[EQUIVOCATION] validator signed two conflicting blocks; evidence recorded (detection only, no slash)"
        );
    }
}

pub type SharedState = Arc<RwLock<NodeState>>;

#[cfg(test)]
mod tests {
    use super::PackageRound;
    use chrono::Utc;
    use common::{AnalysisBundleRefs, ValidatorSignature, ValidatorVote};

    fn sig(id: &str, vote: ValidatorVote, model_version: &str) -> ValidatorSignature {
        ValidatorSignature {
            validator_id: id.to_string(),
            validator_pubkey: format!("{}-pubkey", id),
            signature: format!("{}-sig", id),
            vote,
            signed_at: Utc::now(),
            ml_model_version: model_version.to_string(),
            analysis_bundles: AnalysisBundleRefs::default(),
            is_mandatory: false,
            evidence_digest: String::new(),
            deterministic_risk: common::DeterministicRiskSummary::default(),
            ..Default::default()
        }
    }

    #[test]
    fn package_round_replaces_duplicate_validator_votes() {
        let mut round = PackageRound::from_vote(
            "npm:test@1.0.0",
            sig("validator-1", ValidatorVote::Approve, "creg-detect-v1.0.0"),
        );

        round.record_vote(sig(
            "validator-1",
            ValidatorVote::Reject {
                reason: "malicious".to_string(),
            },
            "creg-detect-v2.0.0",
        ));

        assert_eq!(round.subject, "npm:test@1.0.0");
        assert_eq!(round.vote_count(), 1);
        assert_eq!(round.signatures()[0].ml_model_version, "creg-detect-v2.0.0");
        assert!(matches!(
            round.signatures()[0].vote,
            ValidatorVote::Reject { .. }
        ));
    }
}
