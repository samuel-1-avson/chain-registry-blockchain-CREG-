// crates/node/src/consensus_admission.rs
//
// Mechanical-consensus validator admission.
//
// Every configured active validator checks the staged transition and staking
// rules for the pending applicant. If all rules pass, the validator signs an
// EIP-712 attestation binding (applicant, stake, nonce, ruleSetVersion,
// validatorSetEpoch) and broadcasts it to peers. Any node that has collected attestations from
// ≥ 2/3 of the active set may submit the aggregated calldata to Staking's
// `approveByConsensus`. The plan designates a single automatic submitter.
//
// There is no privileged approver. No single key can admit or block anyone.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use alloy::{
    network::EthereumWallet,
    primitives::{keccak256, Address, Bytes, B256, U256},
    providers::{Provider, ProviderBuilder},
    signers::{local::PrivateKeySigner, Signer},
    sol,
    sol_types::SolValue,
};
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, time::interval};

use crate::state::SharedState;
use anyhow::Context;
use chain_registry_secrets::{HotKeyRole, SecretsProvider};
use common::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};

pub(crate) const TOPIC: &str = "creg/v1/admission-attestations";
pub(crate) const TRANSITION_TOPIC: &str = "creg/v1/admission-transitions";
pub(crate) const MAX_GOSSIP_BYTES: usize = 2_048;
pub(crate) const MAX_TRANSITION_GOSSIP_BYTES: usize = 4_096;

const ADMISSION_STATE_FILE: &str = "validator-admission-state.json";
const DEFAULT_ADMISSION_POLL_SECS: u64 = 3;
const SUBMITTER_STAGGER_SECS: i64 = 30;
const PENDING_TX_GRACE_SECS: i64 = 120;
const MIN_SUBMIT_BALANCE_WEI: u128 = 10_000_000_000_000; // 0.00001 ETH

// ─── Contract interface (read + submit) ───────────────────────────────────────

sol!(
    #[sol(rpc)]
    interface IStakingAdmission {
        function validators(address)
            external
            view
            returns (
                uint256 stake,
                uint8 state,
                uint256 unbondingAt,
                uint256 slashCount,
                uint256 ejectedAt,
                uint256 appliedAt
            );

        function RULE_SET_VERSION() external view returns (uint256);
        function APPLICATION_TIMEOUT() external view returns (uint256);
        function minValidatorStake() external view returns (uint256);
        function activeValidatorCount() external view returns (uint256);
        function validatorSetEpoch() external view returns (uint256);

        function consensusNonceUsed(address applicant, uint256 nonce)
            external view returns (bool);

        function approveByConsensus(
            address applicant,
            uint256 nonce,
            address[] calldata signers,
            bytes[]   calldata sigs
        ) external;
    }
);

// ─── EIP-712 digest ──────────────────────────────────────────────────────────
//
// Must stay byte-identical to Staking.consensusMessageHash. Any divergence
// breaks signature recovery on-chain.

const DOMAIN_NAME: &str = "Chain Registry Validator Admission";
const DOMAIN_VERSION: &str = "1";
const STRUCT_TYPE: &str = "ValidatorAdmission(address applicant,uint256 stake,uint256 nonce,uint256 ruleSetVersion,uint256 validatorSetEpoch)";
const DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

fn domain_separator(chain_id: u64, staking_addr: Address) -> B256 {
    let type_hash = keccak256(DOMAIN_TYPE.as_bytes());
    let name_hash = keccak256(DOMAIN_NAME.as_bytes());
    let version_hash = keccak256(DOMAIN_VERSION.as_bytes());
    let encoded = (
        type_hash,
        name_hash,
        version_hash,
        U256::from(chain_id),
        staking_addr,
    )
        .abi_encode();
    keccak256(&encoded)
}

fn struct_hash(
    applicant: Address,
    stake: U256,
    nonce: U256,
    rule_set_version: U256,
    validator_set_epoch: U256,
) -> B256 {
    let type_hash = keccak256(STRUCT_TYPE.as_bytes());
    let encoded = (
        type_hash,
        applicant,
        stake,
        nonce,
        rule_set_version,
        validator_set_epoch,
    )
        .abi_encode();
    keccak256(&encoded)
}

fn admission_digest(
    chain_id: u64,
    staking_addr: Address,
    applicant: Address,
    stake: U256,
    nonce: U256,
    rule_set_version: U256,
    validator_set_epoch: U256,
) -> B256 {
    let dsep = domain_separator(chain_id, staking_addr);
    let shash = struct_hash(
        applicant,
        stake,
        nonce,
        rule_set_version,
        validator_set_epoch,
    );
    let mut buf = Vec::with_capacity(66);
    buf.extend_from_slice(b"\x19\x01");
    buf.extend_from_slice(dsep.as_slice());
    buf.extend_from_slice(shash.as_slice());
    keccak256(&buf)
}

// ─── On-wire attestation ──────────────────────────────────────────────────────

/// Attestation broadcast by an active validator for a Pending applicant.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdmissionAttestation {
    /// Applicant's EVM address (lowercased, 0x-prefixed).
    pub applicant: String,
    /// Applicant's staked amount in wei (hex, 0x-prefixed).
    pub stake: String,
    /// Unique nonce for this application — the on-chain `appliedAt` timestamp.
    pub nonce: String,
    /// Rule-set version the signer evaluated against. Rejected if != on-chain.
    pub rule_set_version: String,
    /// Exact on-chain active-set membership epoch used for this signature.
    pub validator_set_epoch: String,
    /// Signer's EVM address.
    pub signer: String,
    /// 65-byte ECDSA signature (r||s||v), hex-encoded with 0x prefix.
    pub signature: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GossipAttestation {
    attestation: AdmissionAttestation,
    delivery_id: uuid::Uuid,
}

pub(crate) fn gossip_payload(attestation: AdmissionAttestation) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&GossipAttestation {
        attestation,
        delivery_id: uuid::Uuid::new_v4(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalPlan {
    applicant: Address,
    submitter: Address,
    application_nonce: u64,
    validator_set_epoch: u64,
    transition: common::AuthenticatedValidatorSetSnapshot,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
struct AttestationKey {
    applicant: String,
    nonce: String,
    validator_set_epoch: String,
}

impl AdmissionAttestation {
    fn key(&self) -> AttestationKey {
        AttestationKey {
            applicant: self.applicant.to_ascii_lowercase(),
            nonce: self.nonce.to_ascii_lowercase(),
            validator_set_epoch: self.validator_set_epoch.to_ascii_lowercase(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
struct TransitionProposalKey {
    applicant: String,
    effective_from_height: u64,
    previous_block_hash: String,
    validator_set_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TransitionGossipMessage {
    TransitionApproval {
        applicant: String,
        effective_from_height: u64,
        previous_block_hash: String,
        previous_validator_set_hash: String,
        validator_set_hash: String,
        signature: common::ValidatorSetTransitionSignature,
        delivery_id: uuid::Uuid,
    },
    AdmissionTxNotice {
        applicant: String,
        nonce: String,
        validator_set_epoch: String,
        tx_hash: String,
        status: String,
        confirmed_block: Option<u64>,
        submitter: String,
        validator_id: String,
        validator_pubkey: String,
        signature: String,
        delivery_id: uuid::Uuid,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct L1TxRecord {
    pub applicant: String,
    pub nonce: String,
    pub validator_set_epoch: String,
    pub tx_hash: String,
    pub status: String,
    pub submitter: String,
    pub submitted_at_ms: i64,
    pub confirmed_at_ms: Option<i64>,
    pub confirmed_block: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StageTimings {
    pub started_at_ms: Option<i64>,
    pub l1_quorum_reached_at_ms: Option<i64>,
    pub l1_tx_submitted_at_ms: Option<i64>,
    pub l1_confirmed_at_ms: Option<i64>,
    pub transition_certified_at_ms: Option<i64>,
    pub first_vote_observed_at_ms: Option<i64>,
    pub signature_collection_ms: Option<u64>,
    pub l1_confirmation_ms: Option<u64>,
    pub voting_activation_ms: Option<u64>,
    pub first_finalized_vote_ms: Option<u64>,
}

impl StageTimings {
    fn recompute_durations(&mut self) {
        self.signature_collection_ms = match (self.started_at_ms, self.l1_quorum_reached_at_ms) {
            (Some(start), Some(end)) if end >= start => Some((end - start) as u64),
            _ => None,
        };
        self.l1_confirmation_ms = match (self.l1_tx_submitted_at_ms, self.l1_confirmed_at_ms) {
            (Some(start), Some(end)) if end >= start => Some((end - start) as u64),
            _ => None,
        };
        self.voting_activation_ms =
            match (self.l1_confirmed_at_ms, self.transition_certified_at_ms) {
                (Some(start), Some(end)) if end >= start => Some((end - start) as u64),
                _ => None,
            };
        self.first_finalized_vote_ms =
            match (self.transition_certified_at_ms, self.first_vote_observed_at_ms) {
                (Some(start), Some(end)) if end >= start => Some((end - start) as u64),
                _ => None,
            };
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct LocalAdmissionSignerStatus {
    pub checked: bool,
    pub configured: bool,
    pub address: Option<String>,
    pub matches_active_wallet: bool,
    pub active_on_l1: bool,
    pub has_gas: bool,
    pub balance_wei: Option<String>,
    pub issue: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct CachedL1Snapshot {
    fetched_at_ms: i64,
    active_validators: u64,
    required_l1_approvals: u64,
    validator_set_epoch: Option<u64>,
    rule_set_version: Option<u64>,
    latest_l1_block: Option<u64>,
    min_stake_wei: Option<U256>,
    application_timeout_secs: Option<u64>,
    rpc_error: Option<String>,
    applicants: HashMap<String, CachedL1Applicant>,
}

#[derive(Clone, Debug, Default)]
struct CachedL1Applicant {
    staking_state_code: u8,
    staking_state: String,
    stake_wei: U256,
    stake_creg: u64,
    applied_at: u64,
    ejected_at: u64,
    expires_at: Option<u64>,
    nonce_used: bool,
    active_l1_signers: Vec<String>,
    head_applicant: Option<String>,
    queue_position: Option<usize>,
    l1_issue: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkAdmissionSummary {
    pub chain_id: String,
    pub tip_height: u64,
    pub finalized_height: u64,
    pub active_validators: u64,
    pub required_l1_approvals: u64,
    pub required_transition_approvals: u64,
    pub validator_set_epoch: Option<u64>,
    pub rpc_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalNodeAdmissionSummary {
    pub node_id: String,
    pub is_validator: bool,
    pub admission_auto_activate: bool,
    pub admission_signer_configured: bool,
    pub admission_signer_address: Option<String>,
    pub admission_signer_matches_active_wallet: bool,
    pub admission_signer_active_on_l1: bool,
    pub admission_signer_has_gas: bool,
    pub admission_signer_issue: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdmissionProgressView {
    pub applicant: String,
    pub node_id: String,
    pub ed25519_pubkey: String,
    pub stage: String,
    pub stage_reason: String,
    pub staking_state: String,
    pub stake_creg: u64,
    pub application_nonce: Option<u64>,
    pub expires_at: Option<String>,
    pub l1_approvals_collected: u64,
    pub l1_approvals_required: u64,
    pub l1_signers: Vec<String>,
    pub transition_approvals_collected: u64,
    pub transition_approvals_required: u64,
    pub transition_signers: Vec<String>,
    pub missing_signer_setup: Vec<String>,
    pub l1_tx_hash: Option<String>,
    pub l1_tx_status: String,
    pub l1_confirmed_block: Option<u64>,
    pub voting_activation_height: Option<u64>,
    pub first_finalized_vote_height: Option<u64>,
    pub timings: StageTimings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdmissionStatusReport {
    pub local_node: LocalNodeAdmissionSummary,
    pub network: NetworkAdmissionSummary,
    pub applicants: Vec<AdmissionProgressView>,
}

// ─── Store ────────────────────────────────────────────────────────────────────

const MAX_ATTESTATION_KEYS: usize = 256;
const MAX_SIGNERS_PER_KEY: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedTransitionEntry {
    key: TransitionProposalKey,
    signatures: Vec<common::ValidatorSetTransitionSignature>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct PersistedAdmissionState {
    #[serde(default)]
    attestations: Vec<AdmissionAttestation>,
    #[serde(default)]
    transition_approvals: Vec<PersistedTransitionEntry>,
    #[serde(default)]
    signed_transition_heights: HashMap<u64, String>,
    #[serde(default)]
    tx_records: HashMap<String, L1TxRecord>,
    #[serde(default)]
    timings: HashMap<String, StageTimings>,
    #[serde(default)]
    first_finalized_votes: HashMap<String, u64>,
}

#[derive(Default)]
struct StoreData {
    attestations: HashMap<AttestationKey, HashMap<String, AdmissionAttestation>>,
    transition_approvals:
        HashMap<TransitionProposalKey, HashMap<String, common::ValidatorSetTransitionSignature>>,
    signed_transition_heights: HashMap<u64, String>,
    tx_records: HashMap<String, L1TxRecord>,
    timings: HashMap<String, StageTimings>,
    first_finalized_votes: HashMap<String, u64>,
    local_signer_status: LocalAdmissionSignerStatus,
    cached_l1: CachedL1Snapshot,
    signer_override: Option<Option<chain_registry_secrets::SecretString>>,
    finality_lag_override: Option<u64>,
}

#[derive(Default)]
pub struct AttestationStore {
    inner: Mutex<StoreData>,
    signed_l1_chain_id: Option<u64>,
    state_path: Option<PathBuf>,
}

impl AttestationStore {
    #[cfg(test)]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn for_chain(signed_l1_chain_id: Option<u64>) -> Arc<Self> {
        Self::for_chain_in_dir(signed_l1_chain_id, None)
    }

    pub fn for_chain_in_dir(
        signed_l1_chain_id: Option<u64>,
        data_dir: Option<PathBuf>,
    ) -> Arc<Self> {
        let state_path = data_dir.map(|dir| dir.join(ADMISSION_STATE_FILE));
        let mut data = StoreData::default();
        if let Some(path) = state_path.as_ref() {
            if let Ok(bytes) = std::fs::read(path) {
                match serde_json::from_slice::<PersistedAdmissionState>(&bytes) {
                    Ok(persisted) => {
                        for att in persisted.attestations {
                            Self::insert_att_into(&mut data.attestations, att);
                        }
                        for entry in persisted.transition_approvals {
                            let map = data.transition_approvals.entry(entry.key).or_default();
                            for sig in entry.signatures {
                                if map.len() < MAX_SIGNERS_PER_KEY {
                                    map.insert(sig.validator_id.clone(), sig);
                                }
                            }
                        }
                        data.signed_transition_heights = persisted.signed_transition_heights;
                        data.tx_records = persisted.tx_records;
                        data.timings = persisted.timings;
                        data.first_finalized_votes = persisted.first_finalized_votes;
                    }
                    Err(error) => {
                        tracing::warn!(
                            path = %path.display(),
                            %error,
                            "Could not parse persisted admission state; starting fresh"
                        );
                    }
                }
            }
        }
        Arc::new(Self {
            inner: Mutex::new(data),
            signed_l1_chain_id,
            state_path,
        })
    }

    fn insert_att_into(
        map: &mut HashMap<AttestationKey, HashMap<String, AdmissionAttestation>>,
        att: AdmissionAttestation,
    ) -> bool {
        let key = att.key();
        if !map.contains_key(&key) && map.len() >= MAX_ATTESTATION_KEYS {
            let evict_key = map
                .keys()
                .min_by(|a, b| {
                    a.validator_set_epoch
                        .cmp(&b.validator_set_epoch)
                        .then_with(|| a.nonce.cmp(&b.nonce))
                        .then_with(|| a.applicant.cmp(&b.applicant))
                })
                .cloned();
            if let Some(evict) = evict_key {
                map.remove(&evict);
            }
        }
        let signer = att.signer.to_ascii_lowercase();
        let by_signer = map.entry(key).or_default();
        if !by_signer.contains_key(&signer) && by_signer.len() >= MAX_SIGNERS_PER_KEY {
            return false;
        }
        by_signer.insert(signer, att).is_none()
    }

    fn persist_locked(&self, data: &StoreData) {
        let Some(path) = self.state_path.as_ref() else {
            return;
        };
        let persisted = PersistedAdmissionState {
            attestations: data
                .attestations
                .values()
                .flat_map(|m| m.values().cloned())
                .collect(),
            transition_approvals: data
                .transition_approvals
                .iter()
                .map(|(k, m)| PersistedTransitionEntry {
                    key: k.clone(),
                    signatures: m.values().cloned().collect(),
                })
                .collect(),
            signed_transition_heights: data.signed_transition_heights.clone(),
            tx_records: data.tx_records.clone(),
            timings: data.timings.clone(),
            first_finalized_votes: data.first_finalized_votes.clone(),
        };
        match serde_json::to_vec_pretty(&persisted) {
            Ok(bytes) => {
                if let Err(error) = crate::validator_set_history::atomic_write_bytes(path, &bytes) {
                    tracing::warn!(path = %path.display(), %error, "Failed to persist admission state");
                }
            }
            Err(error) => tracing::warn!(%error, "Failed to serialize admission state"),
        }
    }

    pub async fn insert(&self, att: AdmissionAttestation) -> bool {
        let mut g = self.inner.lock().await;
        let inserted = Self::insert_att_into(&mut g.attestations, att);
        if inserted {
            self.persist_locked(&g);
        }
        inserted
    }

    async fn snapshot(&self, key: &AttestationKey) -> Vec<AdmissionAttestation> {
        let g = self.inner.lock().await;
        g.attestations
            .get(key)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) async fn contains(&self, attestation: &AdmissionAttestation) -> bool {
        self.snapshot(&attestation.key()).await.iter().any(|seen| {
            seen.signer == attestation.signer && seen.signature == attestation.signature
        })
    }

    pub async fn drop_applicant(&self, applicant: &str) {
        let mut g = self.inner.lock().await;
        let needle = applicant.to_ascii_lowercase();
        let before_att = g.attestations.len();
        let before_tr = g.transition_approvals.len();
        g.attestations.retain(|k, _| k.applicant != needle);
        g.transition_approvals.retain(|k, _| k.applicant != needle);
        if g.attestations.len() != before_att || g.transition_approvals.len() != before_tr {
            self.persist_locked(&g);
        }
    }

    async fn insert_transition_approval(
        &self,
        key: TransitionProposalKey,
        sig: common::ValidatorSetTransitionSignature,
    ) -> bool {
        let mut g = self.inner.lock().await;
        if !g.transition_approvals.contains_key(&key)
            && g.transition_approvals.len() >= MAX_ATTESTATION_KEYS
        {
            if let Some(evict) = g
                .transition_approvals
                .keys()
                .min_by_key(|k| k.effective_from_height)
                .cloned()
            {
                g.transition_approvals.remove(&evict);
            }
        }
        let by_validator = g.transition_approvals.entry(key).or_default();
        if !by_validator.contains_key(&sig.validator_id)
            && by_validator.len() >= MAX_SIGNERS_PER_KEY
        {
            return false;
        }
        let inserted = by_validator.insert(sig.validator_id.clone(), sig).is_none();
        if inserted {
            self.persist_locked(&g);
        }
        inserted
    }

    async fn snapshot_transition_approvals(
        &self,
        key: &TransitionProposalKey,
    ) -> Vec<common::ValidatorSetTransitionSignature> {
        let g = self.inner.lock().await;
        g.transition_approvals
            .get(key)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    async fn prune_stale_transition_approvals(&self, tip_height: u64) {
        let mut g = self.inner.lock().await;
        let before = g.transition_approvals.len();
        g.transition_approvals
            .retain(|k, _| k.effective_from_height > tip_height);
        if g.transition_approvals.len() != before {
            self.persist_locked(&g);
        }
    }

    async fn check_and_record_signed_transition_height(
        &self,
        height: u64,
        validator_set_hash: &str,
    ) -> bool {
        let mut g = self.inner.lock().await;
        if let Some(existing) = g.signed_transition_heights.get(&height) {
            return existing.eq_ignore_ascii_case(validator_set_hash);
        }
        g.signed_transition_heights
            .insert(height, validator_set_hash.to_ascii_lowercase());
        self.persist_locked(&g);
        true
    }

    async fn tx_record(&self, applicant: &str, nonce: U256, epoch: U256) -> Option<L1TxRecord> {
        let key = tx_record_key(applicant, nonce, epoch);
        let g = self.inner.lock().await;
        g.tx_records.get(&key).cloned()
    }

    async fn latest_confirmed_tx_record(&self, applicant: &str, nonce: U256) -> Option<L1TxRecord> {
        let applicant_lc = applicant.to_ascii_lowercase();
        let nonce_hex = format!("0x{:x}", nonce);
        let g = self.inner.lock().await;
        g.tx_records
            .values()
            .filter(|rec| {
                rec.applicant.eq_ignore_ascii_case(&applicant_lc)
                    && rec.nonce.eq_ignore_ascii_case(&nonce_hex)
                    && rec.status == "confirmed"
            })
            .max_by_key(|rec| rec.confirmed_block.unwrap_or(0))
            .cloned()
    }

    pub(crate) async fn record_transition_certified_batch(&self, applicants: &[String]) {
        if applicants.is_empty() {
            return;
        }
        let now = now_ms();
        let mut g = self.inner.lock().await;
        let mut changed = false;
        for addr in applicants {
            let key = addr.trim().to_ascii_lowercase();
            if let Some(entry) = g.timings.get_mut(&key) {
                if entry.transition_certified_at_ms.is_none() {
                    entry.transition_certified_at_ms = Some(now);
                    entry.recompute_durations();
                    changed = true;
                }
            }
        }
        if changed {
            self.persist_locked(&g);
        }
    }

    async fn upsert_tx_record(&self, record: L1TxRecord) {
        let key = format!(
            "{}:{}:{}",
            record.applicant.to_ascii_lowercase(),
            record.nonce.to_ascii_lowercase(),
            record.validator_set_epoch.to_ascii_lowercase()
        );
        let mut g = self.inner.lock().await;
        let should_replace = match g.tx_records.get(&key) {
            Some(existing) if existing.status == "confirmed" && record.status != "confirmed" => {
                false
            }
            _ => true,
        };
        if should_replace {
            g.tx_records.insert(key, record);
            self.persist_locked(&g);
        }
    }

    async fn update_timings(
        &self,
        applicant: &str,
        update: impl FnOnce(&mut StageTimings),
    ) {
        let key = applicant.to_ascii_lowercase();
        let mut g = self.inner.lock().await;
        let entry = g.timings.entry(key).or_default();
        let before = serde_json::to_vec(entry).ok();
        update(entry);
        entry.recompute_durations();
        if serde_json::to_vec(entry).ok() != before {
            self.persist_locked(&g);
        }
    }

    async fn record_first_vote(&self, applicant: &str, height: u64) {
        let key = applicant.to_ascii_lowercase();
        let mut g = self.inner.lock().await;
        let changed = match g.first_finalized_votes.get(&key) {
            Some(&prev) if prev <= height => false,
            _ => {
                g.first_finalized_votes.insert(key, height);
                true
            }
        };
        if changed {
            let entry = g.timings.entry(applicant.to_ascii_lowercase()).or_default();
            if entry.first_vote_observed_at_ms.is_none() {
                entry.first_vote_observed_at_ms = Some(now_ms());
                entry.recompute_durations();
            }
            self.persist_locked(&g);
        }
    }

    async fn set_local_signer_status(&self, status: LocalAdmissionSignerStatus) {
        let mut g = self.inner.lock().await;
        g.local_signer_status = status;
    }

    async fn set_cached_l1(&self, snapshot: CachedL1Snapshot) {
        let mut g = self.inner.lock().await;
        g.cached_l1 = snapshot;
    }

    pub async fn set_admission_signer_override(&self, signer: Option<Option<String>>) {
        let mut g = self.inner.lock().await;
        g.signer_override = signer.map(|opt| opt.map(Into::into));
    }

    pub async fn set_finality_lag_override(&self, lag: Option<u64>) {
        let mut g = self.inner.lock().await;
        g.finality_lag_override = lag;
    }

    #[cfg(test)]
    async fn len(&self) -> usize {
        self.inner.lock().await.attestations.len()
    }
}

fn tx_record_key(applicant: &str, nonce: U256, epoch: U256) -> String {
    format!(
        "{}:0x{:x}:0x{:x}",
        applicant.to_ascii_lowercase(),
        nonce,
        epoch
    )
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ─── Signer ──────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn sign_attestation(
    priv_key: &str,
    chain_id: u64,
    staking_addr: Address,
    applicant: Address,
    stake: U256,
    nonce: U256,
    rule_set_version: U256,
    validator_set_epoch: U256,
) -> anyhow::Result<AdmissionAttestation> {
    let signer: PrivateKeySigner = priv_key.parse()?;
    let signer_addr = signer.address();
    let digest = admission_digest(
        chain_id,
        staking_addr,
        applicant,
        stake,
        nonce,
        rule_set_version,
        validator_set_epoch,
    );
    let sig = signer.sign_hash(&digest).await?;
    let sig_bytes: [u8; 65] = sig.as_bytes();
    Ok(AdmissionAttestation {
        applicant: format!("0x{:040x}", applicant),
        stake: format!("0x{:x}", stake),
        nonce: format!("0x{:x}", nonce),
        rule_set_version: format!("0x{:x}", rule_set_version),
        validator_set_epoch: format!("0x{:x}", validator_set_epoch),
        signer: format!("0x{:040x}", signer_addr),
        signature: format!("0x{}", hex::encode(sig_bytes)),
    })
}

// ─── Verifier ────────────────────────────────────────────────────────────────

pub fn verify_attestation(
    att: &AdmissionAttestation,
    chain_id: u64,
    staking_addr: Address,
) -> anyhow::Result<bool> {
    let applicant: Address = att.applicant.parse()?;
    let stake = parse_u256(&att.stake)?;
    let nonce = parse_u256(&att.nonce)?;
    let rsv = parse_u256(&att.rule_set_version)?;
    let validator_set_epoch = parse_u256(&att.validator_set_epoch)?;
    let claimed_signer: Address = att.signer.parse()?;

    let digest = admission_digest(
        chain_id,
        staking_addr,
        applicant,
        stake,
        nonce,
        rsv,
        validator_set_epoch,
    );

    let raw = hex_decode(&att.signature)?;
    if raw.len() != 65 {
        return Ok(false);
    }
    let sig = alloy::primitives::PrimitiveSignature::try_from(raw.as_slice())?;
    if sig.normalize_s().is_some() {
        return Ok(false);
    }
    let recovered = sig.recover_address_from_prehash(&digest)?;
    Ok(recovered == claimed_signer)
}

/// Verify both the signature and its exact active-set membership epoch.
///
/// Callers must obtain `expected_validator_set_epoch` from the configured
/// Staking contract. Treating a missing lookup as a default epoch would reopen
/// same-size-set replay, so lookup failures are propagated instead.
pub fn verify_attestation_for_epoch(
    att: &AdmissionAttestation,
    chain_id: u64,
    staking_addr: Address,
    expected_validator_set_epoch: U256,
) -> anyhow::Result<bool> {
    if parse_u256(&att.validator_set_epoch)? != expected_validator_set_epoch {
        return Ok(false);
    }
    verify_attestation(att, chain_id, staking_addr)
}

fn parse_u256(s: &str) -> anyhow::Result<U256> {
    let trimmed = s.trim_start_matches("0x").trim_start_matches("0X");
    U256::from_str_radix(trimmed, 16).map_err(|e| anyhow::anyhow!("bad u256: {e}"))
}

fn hex_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    let trimmed = s.trim_start_matches("0x").trim_start_matches("0X");
    Ok(hex::decode(trimmed)?)
}

// ─── Main loop ────────────────────────────────────────────────────────────────

fn admission_poll_interval() -> Duration {
    let secs = std::env::var("CREG_ADMISSION_POLL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_ADMISSION_POLL_SECS);
    Duration::from_secs(secs)
}

fn admission_finality_lag(is_testnet: bool) -> u64 {
    let default_lag = if is_testnet { 2 } else { 6 };
    std::env::var("CREG_VALIDATOR_SET_FINALITY_LAG")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default_lag)
}

fn designated_submitter_override() -> Option<Address> {
    std::env::var("CREG_VALIDATOR_ADMISSION_SUBMITTER")
        .ok()
        .and_then(|v| v.trim().parse::<Address>().ok())
}

fn wei_to_creg_u64(stake: U256) -> u64 {
    let unit = U256::from(10u64).pow(U256::from(18u64));
    (stake / unit).try_into().unwrap_or(u64::MAX)
}

fn staking_state_label(state: u8) -> &'static str {
    match state {
        0 => "none",
        1 => "pending",
        2 => "active",
        3 => "unbonding",
        4 => "withdrawn",
        5 => "rejected",
        6 => "expired",
        _ => "unknown",
    }
}

fn l1_quorum_required(active_count: u64) -> u64 {
    if active_count == 0 {
        0
    } else {
        (active_count * 2).div_ceil(3)
    }
}

fn transition_quorum_required(member_count: usize) -> usize {
    if member_count == 0 {
        0
    } else {
        (2 * member_count) / 3 + 1
    }
}

pub async fn run(state: SharedState, store: Arc<AttestationStore>, p2p: crate::p2p::P2PHandle) {
    run_with_interval(state, store, p2p, admission_poll_interval()).await;
}

pub async fn run_with_interval(
    state: SharedState,
    store: Arc<AttestationStore>,
    p2p: crate::p2p::P2PHandle,
    poll_interval: Duration,
) {
    let mut ticker = interval(poll_interval);
    loop {
        ticker.tick().await;
        if let Err(error) = run_tick(&state, &store, &p2p).await {
            tracing::warn!(%error, "Validator admission tick will retry");
        }
    }
}

pub async fn run_tick(
    state: &SharedState,
    store: &Arc<AttestationStore>,
    p2p: &crate::p2p::P2PHandle,
) -> anyhow::Result<()> {
    let legacy_plan = AdmissionSettings::load_plan_only().await?;
    if legacy_plan.is_some() {
        let settings = match AdmissionSettings::load().await {
            Ok(Some(settings)) => settings,
            Ok(None) => return Ok(()),
            Err(error) => return Err(error),
        };
        return tick_legacy_plan(state, store, p2p, &settings).await;
    }

    if let Err(error) = try_activate_candidate_validator(state).await {
        tracing::warn!(%error, "Candidate auto-activation check failed");
    }

    auto_tick(state, store, p2p).await
}

struct AdmissionSettings {
    key: chain_registry_secrets::SecretString,
    address: Address,
    plan: Option<ApprovalPlan>,
}

impl AdmissionSettings {
    async fn load_plan_only() -> anyhow::Result<Option<ApprovalPlan>> {
        let Some(path) = std::env::var_os("CREG_VALIDATOR_ADMISSION_PLAN") else {
            return Ok(None);
        };
        let bytes = tokio::fs::read(path)
            .await
            .context("could not read the admission plan")?;
        Ok(Some(
            serde_json::from_slice(&bytes).context("invalid admission plan JSON")?,
        ))
    }

    async fn load() -> anyhow::Result<Option<Self>> {
        let secrets =
            SecretsProvider::from_env().context("could not initialize admission key custody")?;
        let plan_path = std::env::var_os("CREG_VALIDATOR_ADMISSION_PLAN");
        let Some(key) = secrets
            .try_signing_key_secret(HotKeyRole::ValidatorAdmission)
            .await
            .context("could not load the validator admission signer")?
        else {
            anyhow::ensure!(
                plan_path.is_none(),
                "an admission plan requires an admission signer"
            );
            return Ok(None);
        };
        let signer: PrivateKeySigner = key
            .expose_secret()
            .parse()
            .context("invalid admission signer")?;
        let plan = if let Some(path) = plan_path {
            let bytes = tokio::fs::read(path)
                .await
                .context("could not read the admission plan")?;
            Some(serde_json::from_slice(&bytes).context("invalid admission plan JSON")?)
        } else {
            None
        };
        Ok(Some(Self {
            key,
            address: signer.address(),
            plan,
        }))
    }
}

async fn load_optional_admission_signer() -> (
    Option<(chain_registry_secrets::SecretString, Address)>,
    Option<String>,
) {
    let secrets = match SecretsProvider::from_env() {
        Ok(secrets) => secrets,
        Err(error) => {
            return (
                None,
                Some(format!("could not initialize admission key custody: {error}")),
            );
        }
    };
    match secrets
        .try_signing_key_secret(HotKeyRole::ValidatorAdmission)
        .await
    {
        Ok(Some(key)) => match key.expose_secret().parse::<PrivateKeySigner>() {
            Ok(signer) => (Some((key, signer.address())), None),
            Err(error) => (None, Some(format!("invalid admission signer key: {error}"))),
        },
        Ok(None) => (
            None,
            Some(
                "Validator admission signer (CREG_VALIDATOR_ADMISSION_KEY / Vault validator-admission role) is not configured"
                    .to_string(),
            ),
        ),
        Err(error) => (
            None,
            Some(format!("could not load validator admission signer: {error}")),
        ),
    }
}

impl ApprovalPlan {
    fn validate_transition(
        &self,
        current: &common::ValidatorSet,
        identity: &common::ValidatorIdentity,
        tip_height: u64,
        tip_hash: &str,
    ) -> anyhow::Result<()> {
        validate_single_addition_transition(
            &self.transition,
            current,
            identity,
            self.applicant,
            tip_height,
            tip_hash,
        )?;
        anyhow::ensure!(
            active_signer(current, self.submitter),
            "the designated submitter must be an existing active validator"
        );
        Ok(())
    }
}

fn validate_single_addition_transition(
    transition: &common::AuthenticatedValidatorSetSnapshot,
    current: &common::ValidatorSet,
    identity: &common::ValidatorIdentity,
    applicant: Address,
    tip_height: u64,
    tip_hash: &str,
) -> anyhow::Result<()> {
    crate::validator_set_history::validate_transition_at_tip(transition, tip_height, tip_hash)?;
    transition
        .validate_against(current)
        .map_err(anyhow::Error::msg)?;
    let next = &transition.validators.validators;
    let candidate = next
        .iter()
        .find(|validator| {
            validator.id == identity.node_id
                && validator
                    .pubkey
                    .eq_ignore_ascii_case(&identity.ed25519_pubkey)
                && validator.eth_address.parse::<Address>().ok() == Some(applicant)
                && common::validator_status_is_active(&validator.status)
        })
        .context("the certified transition does not add the registered applicant")?;
    let previous = common::ValidatorSet::new(
        next.iter()
            .filter(|v| v.id != candidate.id)
            .cloned()
            .collect(),
    );
    anyhow::ensure!(
        common::canonical_validator_set_hash(&previous)
            == common::canonical_validator_set_hash(current),
        "the admission transition must add exactly one validator"
    );
    Ok(())
}

fn active_signer(set: &common::ValidatorSet, address: Address) -> bool {
    set.validators.iter().any(|validator| {
        common::validator_status_is_active(&validator.status)
            && validator.eth_address.parse::<Address>().ok() == Some(address)
    })
}

fn matches_node_wallet(set: &common::ValidatorSet, node_id: &str, address: Address) -> bool {
    set.validators.iter().any(|validator| {
        validator.id == node_id
            && common::validator_status_is_active(&validator.status)
            && validator.eth_address.parse::<Address>().ok() == Some(address)
    })
}

fn candidate_in_authority(
    authority: &common::ValidatorSet,
    identity: &common::ValidatorIdentity,
) -> Result<bool, String> {
    let norm = identity.normalized();
    for validator in &authority.validators {
        let same_id = validator.id == norm.node_id;
        let same_pk = validator.pubkey.eq_ignore_ascii_case(&norm.ed25519_pubkey);
        let same_evm = validator.eth_address.eq_ignore_ascii_case(&norm.evm_address);
        if same_id && same_pk && same_evm {
            return Ok(common::validator_status_is_active(&validator.status));
        }
        if same_id || same_pk || same_evm {
            return Err(format!(
                "Registration conflicts with existing validator {} ({})",
                validator.id, validator.eth_address
            ));
        }
    }
    Ok(false)
}

fn build_next_validator_set_for_candidate(
    current: &common::ValidatorSet,
    registration: &crate::ValidatorRegistrationStatus,
    stake_creg: u64,
) -> common::ValidatorSet {
    let identity = registration.identity.normalized();
    let alias = if registration.alias.trim().is_empty() {
        identity.node_id.clone()
    } else {
        registration.alias.trim().to_string()
    };
    let mut validators = current.validators.clone();
    validators.push(common::Validator {
        id: identity.node_id,
        alias,
        pubkey: identity.ed25519_pubkey,
        eth_address: identity.evm_address,
        stake: stake_creg.max(100),
        reputation: 100,
        status: "active".to_string(),
        performance_score: 500,
        contribution_count: 0,
        pq_pubkey: None,
    });
    validators.sort_by(|left, right| {
        left.id.cmp(&right.id).then_with(|| {
            left.pubkey
                .to_ascii_lowercase()
                .cmp(&right.pubkey.to_ascii_lowercase())
        })
    });
    common::ValidatorSet::new(validators)
}

fn ed25519_signing_key_from_hex(raw: &str) -> anyhow::Result<common::crypto::SigningKey> {
    let trimmed = raw.trim().trim_start_matches("0x");
    let bytes = hex::decode(trimmed).context("validator Ed25519 key is not valid hex")?;
    common::crypto::SigningKey::from_seed(common::crypto::SignatureScheme::Ed25519, &bytes)
        .map_err(|error| anyhow::anyhow!("invalid validator Ed25519 key: {error}"))
}

fn tx_notice_message(
    applicant: &str,
    nonce: &str,
    epoch: &str,
    tx_hash: &str,
    status: &str,
    confirmed_block: Option<u64>,
) -> String {
    format!(
        "creg-admission-tx-v1\napplicant:{}\nnonce:{}\nepoch:{}\ntx_hash:{}\nstatus:{}\nconfirmed_block:{}",
        applicant.to_ascii_lowercase(),
        nonce.to_ascii_lowercase(),
        epoch.to_ascii_lowercase(),
        tx_hash.to_ascii_lowercase(),
        status.to_ascii_lowercase(),
        confirmed_block.map(|b| b.to_string()).unwrap_or_default()
    )
}

/// Automatically activate a synced candidate node from observer mode into an
/// active PBFT validator once its certified transition is anchored to the local
/// canonical chain.
pub async fn try_activate_candidate_validator(state: &SharedState) -> anyhow::Result<bool> {
    let mut s = state.write().await;
    if !s.config.admission_auto_activate || s.config.is_validator {
        return Ok(false);
    }
    let Some(privkey_secret) = s.config.validator_privkey.as_ref() else {
        return Ok(false);
    };
    let signing_key = ed25519_signing_key_from_hex(privkey_secret.expose_secret())?;
    let validator_pubkey = signing_key.public_key().to_hex();
    let tip_height = s.chain.tip_height()?;
    let next_height = tip_height.checked_add(1).context("invalid tip height")?;
    let membership = crate::validator_set_history::require_phase_c_signing_membership(
        &s.config.data_dir,
        next_height,
        &s.validator_set_bootstrap,
        &s.config.node_id,
        &validator_pubkey,
        |height| Ok(s.chain.get_block_by_height(height)?.map(|b| b.hash())),
    );
    if membership.is_err() {
        return Ok(false);
    }

    let activation_height = crate::validator_set_history::load_checked(&s.config.data_dir)?
        .into_iter()
        .filter_map(|entry| entry.certificate)
        .find(|cert| {
            cert.validators.validators.iter().any(|validator| {
                validator.id == s.config.node_id
                    && validator.pubkey.eq_ignore_ascii_case(&validator_pubkey)
                    && common::validator_status_is_active(&validator.status)
            })
        })
        .map(|cert| cert.effective_from_height);

    let genesis = s
        .chain
        .get_block_by_height(0)?
        .context("canonical genesis block is unavailable during candidate activation")?;
    s.chain.initialize_pbft_safety_journal_at_height(
        &genesis.hash(),
        &s.config.node_id,
        &validator_pubkey,
        activation_height,
    )?;
    s.refresh_runtime_validator_set_from_certified_history()?;
    s.config.is_validator = true;
    tracing::info!(
        node_id = %s.config.node_id,
        activation_height = ?activation_height,
        tip_height,
        "Candidate validator certified and activated for PBFT voting"
    );
    Ok(true)
}

async fn auto_tick(
    state: &SharedState,
    store: &Arc<AttestationStore>,
    p2p: &crate::p2p::P2PHandle,
) -> anyhow::Result<()> {
    let (
        is_validator,
        node_id,
        data_dir,
        bootstrap,
        rpc_url,
        staking_addr_str,
        validator_privkey,
        is_testnet,
        registrations,
        tip_height,
        _tip_hash,
    ) = {
        let s = state.read().await;
        (
            s.config.is_validator,
            s.config.node_id.clone(),
            s.config.data_dir.clone(),
            s.validator_set_bootstrap.clone(),
            s.config.eth_rpc_url.clone(),
            s.config.staking_addr.clone(),
            s.config.validator_privkey.clone(),
            s.config.is_testnet,
            s.validator_registrations.clone(),
            s.chain.tip_height()?,
            s.chain.tip_hash()?,
        )
    };

    store.prune_stale_transition_approvals(tip_height).await;

    let authority = crate::validator_set_history::verification_set_at(
        &data_dir,
        tip_height.saturating_add(1),
        &bootstrap,
    )
    .map_err(anyhow::Error::msg)?;

    let staking_addr = match crate::config::parse_staking_contract_address(&staking_addr_str) {
        Ok(Some(addr)) => addr,
        Ok(None) => return Ok(()),
        Err(error) => anyhow::bail!("{error}"),
    };

    let (signer_override, finality_lag_override) = {
        let g = store.inner.lock().await;
        (g.signer_override.clone(), g.finality_lag_override)
    };
    let (signer_pair, signer_load_issue) = if is_validator {
        match signer_override {
            Some(Some(key)) => match key.expose_secret().parse::<PrivateKeySigner>() {
                Ok(signer) => (Some((key, signer.address())), None),
                Err(error) => (None, Some(format!("invalid admission signer key: {error}"))),
            },
            Some(None) => (
                None,
                Some(
                    "Validator admission signer (CREG_VALIDATOR_ADMISSION_KEY / Vault validator-admission role) is not configured"
                        .to_string(),
                ),
            ),
            None => load_optional_admission_signer().await,
        }
    } else {
        (None, None)
    };

    let mut local_signer_status = LocalAdmissionSignerStatus {
        checked: is_validator,
        configured: signer_pair.is_some(),
        address: signer_pair
            .as_ref()
            .map(|(_, addr)| format!("0x{:040x}", addr)),
        matches_active_wallet: false,
        active_on_l1: false,
        has_gas: false,
        balance_wei: None,
        issue: signer_load_issue.clone(),
    };

    let expected_chain_id = store.signed_l1_chain_id;
    let provider = match crate::rpc_failover::build_l1_provider(&rpc_url) {
        Ok(provider) => Some(provider),
        Err(error) => {
            let mut cached = CachedL1Snapshot {
                fetched_at_ms: now_ms(),
                active_validators: authority
                    .validators
                    .iter()
                    .filter(|v| common::validator_status_is_active(&v.status))
                    .count() as u64,
                required_l1_approvals: l1_quorum_required(
                    authority
                        .validators
                        .iter()
                        .filter(|v| common::validator_status_is_active(&v.status))
                        .count() as u64,
                ),
                rpc_error: Some(format!("L1 RPC client unavailable: {error}")),
                ..Default::default()
            };
            cached.fetched_at_ms = now_ms();
            store.set_cached_l1(cached).await;
            if is_validator && local_signer_status.issue.is_none() {
                local_signer_status.issue = Some(format!("L1 RPC client unavailable: {error}"));
            }
            store.set_local_signer_status(local_signer_status.clone()).await;
            None
        }
    };

    let Some(provider) = provider else {
        return Ok(());
    };

    let rpc_chain_id = match provider.get_chain_id().await {
        Ok(id) => id,
        Err(error) => {
            let mut cached = CachedL1Snapshot::default();
            cached.fetched_at_ms = now_ms();
            cached.rpc_error = Some(format!("L1 RPC chain_id query failed: {error}"));
            store.set_cached_l1(cached).await;
            if is_validator && local_signer_status.issue.is_none() {
                local_signer_status.issue =
                    Some(format!("L1 RPC chain_id query failed: {error}"));
            }
            store.set_local_signer_status(local_signer_status).await;
            return Ok(());
        }
    };

    if let Some(expected) = expected_chain_id {
        if rpc_chain_id != expected {
            let msg = format!(
                "L1 RPC chain ID {rpc_chain_id} does not match signed chain spec L1 chain ID {expected}"
            );
            let mut cached = CachedL1Snapshot::default();
            cached.fetched_at_ms = now_ms();
            cached.rpc_error = Some(msg.clone());
            store.set_cached_l1(cached).await;
            if is_validator {
                local_signer_status.issue = Some(msg);
            }
            store.set_local_signer_status(local_signer_status).await;
            return Ok(());
        }
    }

    let effective_chain_id = expected_chain_id.unwrap_or(rpc_chain_id);
    let contract = IStakingAdmission::new(staking_addr, &provider);

    if let Some((_, signer_addr)) = signer_pair.as_ref() {
        local_signer_status.matches_active_wallet =
            matches_node_wallet(&authority, &node_id, *signer_addr);
        if !local_signer_status.matches_active_wallet {
            local_signer_status.issue = Some(format!(
                "Admission signer 0x{:040x} does not match {node_id}'s active staking wallet in certified history",
                signer_addr
            ));
        } else {
            match contract.validators(*signer_addr).call().await {
                Ok(entry) => {
                    local_signer_status.active_on_l1 = entry.state == 2u8;
                    if !local_signer_status.active_on_l1 {
                        local_signer_status.issue = Some(format!(
                            "Admission signer 0x{:040x} is {} on L1 (expected active)",
                            signer_addr,
                            staking_state_label(entry.state)
                        ));
                    }
                }
                Err(error) => {
                    local_signer_status.issue =
                        Some(format!("Failed reading signer staking state: {error}"));
                }
            }
            if let Ok(balance) = provider.get_balance(*signer_addr).await {
                local_signer_status.balance_wei = Some(balance.to_string());
                local_signer_status.has_gas = balance >= U256::from(MIN_SUBMIT_BALANCE_WEI);
            }
        }
    }
    store
        .set_local_signer_status(local_signer_status.clone())
        .await;

    let active_count = contract.activeValidatorCount().call().await?._0;
    let active_count_u64: u64 = active_count.try_into().unwrap_or(0);
    let required_l1 = l1_quorum_required(active_count_u64);
    let signing_epoch = contract.validatorSetEpoch().call().await?._0;
    let signing_epoch_u64: u64 = signing_epoch.try_into().unwrap_or(0);
    let rule_set_version = contract.RULE_SET_VERSION().call().await?._0;
    let rule_set_version_u64: u64 = rule_set_version.try_into().unwrap_or(1);
    let min_stake = contract.minValidatorStake().call().await?._0;
    let app_timeout = contract.APPLICATION_TIMEOUT().call().await?._0;
    let latest_l1_block = provider.get_block_number().await.ok();
    let finality_lag = finality_lag_override.unwrap_or_else(|| admission_finality_lag(is_testnet));
    let now_secs = U256::from(chrono::Utc::now().timestamp().max(0) as u64);

    let mut cached_l1 = CachedL1Snapshot {
        fetched_at_ms: now_ms(),
        active_validators: active_count_u64,
        required_l1_approvals: required_l1,
        validator_set_epoch: Some(signing_epoch_u64),
        rule_set_version: Some(rule_set_version_u64),
        latest_l1_block,
        min_stake_wei: Some(min_stake),
        application_timeout_secs: Some(app_timeout.try_into().unwrap_or(0)),
        rpc_error: None,
        applicants: HashMap::new(),
    };

    // Query L1 state for all registered applicants and scan first finalized votes
    // for already-certified candidates.
    struct CandidateInfo {
        applicant: Address,
        applicant_key: String,
        registration: crate::ValidatorRegistrationStatus,
        l1_state: u8,
        stake_wei: U256,
        applied_at: u64,
    }
    let mut phase2_candidates: Vec<CandidateInfo> = Vec::new();
    let mut phase1_candidates: Vec<CandidateInfo> = Vec::new();

    for (key, registration) in &registrations {
        let identity = registration.identity.normalized();
        if !registration.registered_with_node || !identity.is_complete() {
            continue;
        }
        let Ok(applicant) = identity.evm_address.parse::<Address>() else {
            continue;
        };
        let applicant_key = format!("0x{:040x}", applicant);

        // If already in certified authority, scan chain for first finalized vote
        // and ensure L1 snapshot is up-to-date.
        match candidate_in_authority(&authority, &identity) {
            Ok(true) => {
                if let Ok(Some(activation_height)) =
                    crate::validator_set_history::certified_activation_height_for(
                        &data_dir,
                        &identity.node_id,
                        &identity.ed25519_pubkey,
                        &applicant_key,
                    )
                {
                    let existing_vote = {
                        let g = store.inner.lock().await;
                        g.first_finalized_votes.get(&applicant_key).copied()
                    };
                    if existing_vote.is_none() {
                        if let Some(vote_h) = find_first_finalized_vote_height(
                            state,
                            &identity.node_id,
                            &identity.ed25519_pubkey,
                            activation_height,
                            tip_height,
                        )
                        .await
                        {
                            store.record_first_vote(&applicant_key, vote_h).await;
                        }
                    }
                }
                if let Ok(v) = contract.validators(applicant).call().await {
                    cached_l1.applicants.insert(
                        key.clone(),
                        CachedL1Applicant {
                            staking_state_code: v.state,
                            staking_state: staking_state_label(v.state).to_string(),
                            stake_wei: v.stake,
                            stake_creg: wei_to_creg_u64(v.stake),
                            applied_at: v.appliedAt.try_into().unwrap_or(0),
                            ejected_at: v.ejectedAt.try_into().unwrap_or(0),
                            expires_at: None,
                            nonce_used: true,
                            active_l1_signers: Vec::new(),
                            head_applicant: None,
                            queue_position: None,
                            l1_issue: None,
                        },
                    );
                }
                continue;
            }
            Err(conflict) => {
                cached_l1.applicants.insert(
                    key.clone(),
                    CachedL1Applicant {
                        l1_issue: Some(conflict),
                        ..Default::default()
                    },
                );
                continue;
            }
            Ok(false) => {}
        }

        let v = match contract.validators(applicant).call().await {
            Ok(v) => v,
            Err(error) => {
                cached_l1.applicants.insert(
                    key.clone(),
                    CachedL1Applicant {
                        l1_issue: Some(format!("Failed reading applicant L1 staking state: {error}")),
                        ..Default::default()
                    },
                );
                continue;
            }
        };

        let applied_at: u64 = v.appliedAt.try_into().unwrap_or(0);
        let ejected_at: u64 = v.ejectedAt.try_into().unwrap_or(0);
        let expires_at = (v.state == 1u8 && applied_at > 0)
            .then(|| applied_at.saturating_add(app_timeout.try_into().unwrap_or(0)));
        let nonce_used = if v.state == 1u8 {
            contract
                .consensusNonceUsed(applicant, v.appliedAt)
                .call()
                .await
                .map(|r| r._0)
                .unwrap_or(false)
        } else {
            false
        };

        let mut l1_issue = None;
        let mut active_l1_signers = Vec::new();

        if v.state == 1u8 {
            let snapshot_key = AttestationKey {
                applicant: applicant_key.clone(),
                nonce: format!("0x{:x}", v.appliedAt),
                validator_set_epoch: format!("0x{:x}", signing_epoch),
            };
            let stored_atts = store.snapshot(&snapshot_key).await;
            for att in stored_atts {
                let matches_context = att
                    .applicant
                    .parse::<Address>()
                    .map(|value| value == applicant)
                    .unwrap_or(false)
                    && parse_u256(&att.stake)
                        .map(|value| value == v.stake)
                        .unwrap_or(false)
                    && parse_u256(&att.nonce)
                        .map(|value| value == v.appliedAt)
                        .unwrap_or(false)
                    && parse_u256(&att.rule_set_version)
                        .map(|value| value == rule_set_version)
                        .unwrap_or(false);
                if matches_context
                    && verify_attestation_for_epoch(
                        &att,
                        effective_chain_id,
                        staking_addr,
                        signing_epoch,
                    )
                    .unwrap_or(false)
                {
                    if let Ok(signer_addr) = att.signer.parse::<Address>() {
                        if let Ok(info) = contract.validators(signer_addr).call().await {
                            if info.state == 2u8 {
                                active_l1_signers.push(format!("0x{:040x}", signer_addr));
                            }
                        }
                    }
                }
            }
            active_l1_signers.sort();
            active_l1_signers.dedup();
        }

        if v.state == 2u8 {
            // Check L1 finality lag if we observed the confirmation block
            let tx_rec = store
                .latest_confirmed_tx_record(&applicant_key, v.appliedAt)
                .await;
            let waiting_finality = match (tx_rec.and_then(|r| r.confirmed_block), latest_l1_block) {
                (Some(conf_block), Some(latest))
                    if latest < conf_block.saturating_add(finality_lag) =>
                {
                    l1_issue = Some(format!(
                        "Sepolia approval confirmed at L1 block #{conf_block}; waiting for finality lag ({}/{finality_lag} blocks)",
                        latest.saturating_sub(conf_block)
                    ));
                    true
                }
                _ => false,
            };
            if !waiting_finality {
                phase2_candidates.push(CandidateInfo {
                    applicant,
                    applicant_key: applicant_key.clone(),
                    registration: registration.clone(),
                    l1_state: v.state,
                    stake_wei: v.stake,
                    applied_at,
                });
            }
        } else if v.state == 1u8 {
            if v.stake < min_stake {
                l1_issue = Some(format!(
                    "Applicant stake ({} tCREG) is below the required minimum ({} tCREG)",
                    wei_to_creg_u64(v.stake),
                    wei_to_creg_u64(min_stake)
                ));
            } else if v.ejectedAt > U256::ZERO && v.appliedAt <= v.ejectedAt {
                l1_issue = Some("Applicant is still inside post-slash restake cooldown".to_string());
            } else if now_secs >= v.appliedAt + app_timeout {
                l1_issue = Some("Pending Sepolia application expired after the 7-day window".to_string());
                store.drop_applicant(&applicant_key).await;
            } else if nonce_used {
                l1_issue = Some("Application nonce was already consumed on Sepolia".to_string());
            } else {
                phase1_candidates.push(CandidateInfo {
                    applicant,
                    applicant_key: applicant_key.clone(),
                    registration: registration.clone(),
                    l1_state: v.state,
                    stake_wei: v.stake,
                    applied_at,
                });
            }
        } else {
            l1_issue = Some(match v.state {
                0 => "Applicant has not staked on Sepolia yet".to_string(),
                5 => "Sepolia application was rejected".to_string(),
                6 => "Sepolia application expired and stake was refunded".to_string(),
                3 => "Applicant stake is unbonding".to_string(),
                4 => "Applicant stake was withdrawn".to_string(),
                _ => "Applicant has an unknown Sepolia staking state".to_string(),
            });
        }

        cached_l1.applicants.insert(
            key.clone(),
            CachedL1Applicant {
                staking_state_code: v.state,
                staking_state: staking_state_label(v.state).to_string(),
                stake_wei: v.stake,
                stake_creg: wei_to_creg_u64(v.stake),
                applied_at,
                ejected_at,
                expires_at,
                nonce_used,
                active_l1_signers,
                head_applicant: None,
                queue_position: None,
                l1_issue,
            },
        );
    }

    phase2_candidates.sort_by(|a, b| {
        a.applied_at
            .cmp(&b.applied_at)
            .then_with(|| a.applicant_key.cmp(&b.applicant_key))
    });
    phase1_candidates.sort_by(|a, b| {
        a.applied_at
            .cmp(&b.applied_at)
            .then_with(|| a.applicant_key.cmp(&b.applicant_key))
    });

    let mut ordered_queue: Vec<&CandidateInfo> = Vec::new();
    ordered_queue.extend(phase2_candidates.iter());
    ordered_queue.extend(phase1_candidates.iter());

    let head_applicant_key = ordered_queue.first().map(|c| c.applicant_key.clone());
    for (idx, candidate) in ordered_queue.iter().enumerate() {
        if let Some(entry) = cached_l1.applicants.get_mut(&candidate.applicant_key) {
            entry.queue_position = Some(idx);
            entry.head_applicant = head_applicant_key.clone();
        }
    }

    store.set_cached_l1(cached_l1).await;

    if !is_validator {
        return Ok(());
    }

    let Some(head) = ordered_queue.into_iter().next() else {
        return Ok(());
    };

    // Ensure timing start is recorded as soon as an eligible candidate is picked up.
    store
        .update_timings(&head.applicant_key, |t| {
            if t.started_at_ms.is_none() {
                t.started_at_ms = Some(now_ms());
            }
        })
        .await;

    if head.l1_state == 1u8 {
        // ── Phase 1: EIP-712 attestations & Sepolia approveByConsensus ──────
        let Some((ref admission_key, admission_addr)) = signer_pair else {
            return Ok(());
        };
        if !local_signer_status.matches_active_wallet || !local_signer_status.active_on_l1 {
            return Ok(());
        }

        let nonce = U256::from(head.applied_at);
        let stake = head.stake_wei;
        let att = sign_attestation(
            admission_key.expose_secret(),
            effective_chain_id,
            staking_addr,
            head.applicant,
            stake,
            nonce,
            rule_set_version,
            signing_epoch,
        )
        .await?;
        if store.insert(att.clone()).await {
            tracing::info!(
                applicant = %head.applicant_key,
                %signing_epoch,
                "Signed validator admission attestation"
            );
        }
        p2p.sender
            .send(crate::p2p::P2PCommand::Broadcast {
                topic: TOPIC.to_owned(),
                data: gossip_payload(att)?,
            })
            .await
            .context("admission gossip channel is unavailable")?;

        let snapshot_key = AttestationKey {
            applicant: head.applicant_key.clone(),
            nonce: format!("0x{:x}", nonce),
            validator_set_epoch: format!("0x{:x}", signing_epoch),
        };
        let atts = store.snapshot(&snapshot_key).await;

        if contract.validatorSetEpoch().call().await?._0 != signing_epoch {
            return Ok(());
        }

        let verified: Vec<AdmissionAttestation> = atts
            .into_iter()
            .filter(|a| {
                let matches_context = a
                    .applicant
                    .parse::<Address>()
                    .map(|value| value == head.applicant)
                    .unwrap_or(false)
                    && parse_u256(&a.stake)
                        .map(|value| value == stake)
                        .unwrap_or(false)
                    && parse_u256(&a.nonce)
                        .map(|value| value == nonce)
                        .unwrap_or(false)
                    && parse_u256(&a.rule_set_version)
                        .map(|value| value == rule_set_version)
                        .unwrap_or(false);
                matches_context
                    && verify_attestation_for_epoch(
                        a,
                        effective_chain_id,
                        staking_addr,
                        signing_epoch,
                    )
                    .unwrap_or(false)
            })
            .collect();

        let mut filtered: Vec<(Address, AdmissionAttestation)> = Vec::with_capacity(verified.len());
        for a in verified {
            let signer_addr: Address = a.signer.parse()?;
            let info = contract.validators(signer_addr).call().await?;
            if info.state == 2u8 {
                filtered.push((signer_addr, a));
            }
        }
        filtered.sort_by_key(|(addr, _)| *addr);
        filtered.dedup_by_key(|(addr, _)| *addr);

        let live_active_count = contract.activeValidatorCount().call().await?._0;
        if contract.validatorSetEpoch().call().await?._0 != signing_epoch {
            return Ok(());
        }
        if live_active_count.is_zero()
            || U256::from(filtered.len() as u64) * U256::from(3u64)
                < live_active_count * U256::from(2u64)
        {
            return Ok(());
        }

        store
            .update_timings(&head.applicant_key, |t| {
                if t.l1_quorum_reached_at_ms.is_none() {
                    t.l1_quorum_reached_at_ms = Some(now_ms());
                }
            })
            .await;

        // Check existing tx record to avoid duplicate L1 submissions across restarts or peers.
        if let Some(existing_tx) = store
            .tx_record(&head.applicant_key, nonce, signing_epoch)
            .await
        {
            if existing_tx.status == "submitted" {
                if let Ok(tx_hash) = existing_tx.tx_hash.parse::<alloy::primitives::TxHash>() {
                    if let Ok(Some(receipt)) = provider.get_transaction_receipt(tx_hash).await {
                        let status = if receipt.status() {
                            "confirmed"
                        } else {
                            "reverted"
                        };
                        let confirmed_block = receipt.block_number;
                        let mut updated = existing_tx.clone();
                        updated.status = status.to_string();
                        updated.confirmed_block = confirmed_block;
                        if receipt.status() {
                            updated.confirmed_at_ms = Some(now_ms());
                        } else {
                            updated.last_error = Some("transaction reverted on-chain".to_string());
                        }
                        store.upsert_tx_record(updated.clone()).await;
                        if receipt.status() {
                            store
                                .update_timings(&head.applicant_key, |t| {
                                    if t.l1_confirmed_at_ms.is_none() {
                                        t.l1_confirmed_at_ms = updated.confirmed_at_ms;
                                    }
                                })
                                .await;
                        }
                        if let Some(ref priv_hex) = validator_privkey {
                            let _ = broadcast_tx_notice(
                                p2p,
                                &node_id,
                                priv_hex.expose_secret(),
                                &updated,
                            )
                            .await;
                        }
                        return Ok(());
                    }
                }
                let elapsed_secs = (now_ms() - existing_tx.submitted_at_ms) / 1000;
                if elapsed_secs < PENDING_TX_GRACE_SECS {
                    return Ok(());
                }
            } else if existing_tx.status == "confirmed" {
                return Ok(());
            }
        }

        // Only wallets with gas can submit; deterministic rank + stagger prevents duplicate tx races.
        if !local_signer_status.has_gas {
            return Ok(());
        }
        if let Some(override_addr) = designated_submitter_override() {
            if admission_addr != override_addr {
                return Ok(());
            }
        } else {
            let mut funded_signers = Vec::new();
            for (addr, _) in &filtered {
                if let Ok(bal) = provider.get_balance(*addr).await {
                    if bal >= U256::from(MIN_SUBMIT_BALANCE_WEI) {
                        funded_signers.push(*addr);
                    }
                }
            }
            funded_signers.sort();
            let Some(rank) = funded_signers.iter().position(|addr| *addr == admission_addr) else {
                return Ok(());
            };
            if rank > 0 {
                let quorum_at = {
                    let g = store.inner.lock().await;
                    g.timings
                        .get(&head.applicant_key)
                        .and_then(|t| t.l1_quorum_reached_at_ms)
                        .unwrap_or_else(now_ms)
                };
                let waited_secs = (now_ms() - quorum_at) / 1000;
                if waited_secs < (rank as i64) * SUBMITTER_STAGGER_SECS {
                    return Ok(());
                }
            }
        }

        // Re-verify live L1 state right before simulation/submission.
        let latest_v = contract.validators(head.applicant).call().await?;
        if latest_v.state != 1u8
            || contract
                .consensusNonceUsed(head.applicant, nonce)
                .call()
                .await?
                ._0
            || contract.validatorSetEpoch().call().await?._0 != signing_epoch
        {
            return Ok(());
        }

        submit_consensus_approval_tracked(
            &rpc_url,
            admission_key.expose_secret(),
            admission_addr,
            staking_addr,
            head.applicant,
            nonce,
            signing_epoch,
            &filtered,
            store,
            p2p,
            &node_id,
            validator_privkey.as_ref().map(|s| s.expose_secret()),
        )
        .await?;
        return Ok(());
    }

    if head.l1_state == 2u8 {
        // ── Phase 2: Quorum-certified CREG validator-set transition ──────────
        let Some(ref privkey_secret) = validator_privkey else {
            return Ok(());
        };
        let ed_sk = ed25519_signing_key_from_hex(privkey_secret.expose_secret())?;
        let ed_pubkey_hex = ed_sk.public_key().to_hex();

        let (fresh_tip_height, fresh_tip_hash, fresh_authority) = {
            let s = state.read().await;
            let h = s.chain.tip_height()?;
            let hash = s.chain.tip_hash()?;
            let auth = crate::validator_set_history::verification_set_at(
                &s.config.data_dir,
                h.saturating_add(1),
                &s.validator_set_bootstrap,
            )
            .map_err(anyhow::Error::msg)?;
            (h, hash, auth)
        };

        let is_member = fresh_authority.validators.iter().any(|validator| {
            validator.id == node_id
                && validator.pubkey.eq_ignore_ascii_case(&ed_pubkey_hex)
                && common::validator_status_is_active(&validator.status)
        });
        if !is_member {
            return Ok(());
        }
        if candidate_in_authority(&fresh_authority, &head.registration.identity).unwrap_or(true) {
            return Ok(());
        }

        let effective_from_height = fresh_tip_height
            .checked_add(1)
            .context("invalid chain tip height")?;
        let next_set = build_next_validator_set_for_candidate(
            &fresh_authority,
            &head.registration,
            wei_to_creg_u64(head.stake_wei),
        );
        let prev_set_hash = common::canonical_validator_set_hash(&fresh_authority);
        let next_set_hash = common::canonical_validator_set_hash(&next_set);

        // Equivocation guard: never sign two distinct validator sets for the same height.
        if !store
            .check_and_record_signed_transition_height(effective_from_height, &next_set_hash)
            .await
        {
            tracing::warn!(
                effective_from_height,
                applicant = %head.applicant_key,
                "Refusing to sign conflicting validator-set transition at height; waiting for tip to advance"
            );
            return Ok(());
        }

        let unsigned = common::AuthenticatedValidatorSetSnapshot {
            schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height,
            previous_block_hash: fresh_tip_hash.clone(),
            previous_validator_set_hash: prev_set_hash.clone(),
            validators: next_set.clone(),
            validator_set_hash: next_set_hash.clone(),
            transition_signatures: Vec::new(),
        };
        let message = common::validator_set_transition_message(&unsigned);
        let signature = common::ValidatorSetTransitionSignature {
            validator_id: node_id.clone(),
            validator_pubkey: ed_pubkey_hex,
            signature: ed_sk.sign(message.as_bytes()).to_hex(),
        };

        let proposal_key = TransitionProposalKey {
            applicant: head.applicant_key.clone(),
            effective_from_height,
            previous_block_hash: fresh_tip_hash.clone(),
            validator_set_hash: next_set_hash.clone(),
        };
        if store
            .insert_transition_approval(proposal_key.clone(), signature.clone())
            .await
        {
            tracing::info!(
                applicant = %head.applicant_key,
                effective_from_height,
                "Signed validator-set transition approval"
            );
        }

        let gossip_msg = TransitionGossipMessage::TransitionApproval {
            applicant: head.applicant_key.clone(),
            effective_from_height,
            previous_block_hash: fresh_tip_hash.clone(),
            previous_validator_set_hash: prev_set_hash.clone(),
            validator_set_hash: next_set_hash.clone(),
            signature,
            delivery_id: uuid::Uuid::new_v4(),
        };
        let gossip_bytes = serde_json::to_vec(&gossip_msg)?;
        p2p.sender
            .send(crate::p2p::P2PCommand::Broadcast {
                topic: TRANSITION_TOPIC.to_owned(),
                data: gossip_bytes,
            })
            .await
            .context("transition approval gossip channel is unavailable")?;

        let mut collected = store.snapshot_transition_approvals(&proposal_key).await;
        collected.sort_by(|a, b| a.validator_id.cmp(&b.validator_id));
        collected.dedup_by(|a, b| a.validator_id == b.validator_id);

        let required_transition = transition_quorum_required(fresh_authority.validators.len());
        if collected.len() < required_transition {
            return Ok(());
        }

        let mut snapshot = unsigned;
        snapshot.transition_signatures = collected;
        validate_single_addition_transition(
            &snapshot,
            &fresh_authority,
            &head.registration.identity,
            head.applicant,
            fresh_tip_height,
            &fresh_tip_hash,
        )?;

        {
            let mut s = state.write().await;
            let locked_tip_height = s.chain.tip_height()?;
            let locked_tip_hash = s.chain.tip_hash()?;
            crate::validator_set_history::validate_transition_at_tip(
                &snapshot,
                locked_tip_height,
                &locked_tip_hash,
            )?;
            crate::validator_set_history::record_authenticated(
                &s.config.data_dir,
                snapshot.clone(),
                &fresh_authority,
            )?;
            s.refresh_runtime_validator_set_from_certified_history()?;
        }

        store
            .update_timings(&head.applicant_key, |t| {
                if t.transition_certified_at_ms.is_none() {
                    t.transition_certified_at_ms = Some(now_ms());
                }
            })
            .await;
        store.drop_applicant(&head.applicant_key).await;

        let transition_data = serde_json::to_vec(&common::GossipMessage::ValidatorSetTransition {
            snapshot,
        })?;
        if let Err(error) = p2p
            .sender
            .send(crate::p2p::P2PCommand::Broadcast {
                topic: crate::validator_registry_gossip::VALIDATOR_SET_TRANSITIONS_TOPIC
                    .to_owned(),
                data: transition_data,
            })
            .await
        {
            tracing::warn!(%error, "Failed to broadcast certified validator-set transition");
        }
        let _ = try_activate_candidate_validator(state).await;
        tracing::info!(
            applicant = %head.applicant_key,
            effective_from_height,
            "Certified validator-set transition installed and broadcast"
        );
    }

    Ok(())
}

async fn find_first_finalized_vote_height(
    state: &SharedState,
    node_id: &str,
    ed25519_pubkey: &str,
    from_height: u64,
    tip_height: u64,
) -> Option<u64> {
    if from_height > tip_height {
        return None;
    }
    let s = state.read().await;
    for height in from_height..=tip_height {
        if let Ok(Some(block)) = s.chain.get_block_by_height(height) {
            if block.pbft_signatures.iter().any(|sig| {
                sig.validator_id == node_id && sig.pubkey.eq_ignore_ascii_case(ed25519_pubkey)
            }) {
                return Some(height);
            }
        }
    }
    None
}

async fn broadcast_tx_notice(
    p2p: &crate::p2p::P2PHandle,
    validator_id: &str,
    validator_priv_hex: &str,
    record: &L1TxRecord,
) -> anyhow::Result<()> {
    let sk = ed25519_signing_key_from_hex(validator_priv_hex)?;
    let pubkey = sk.public_key().to_hex();
    let msg = tx_notice_message(
        &record.applicant,
        &record.nonce,
        &record.validator_set_epoch,
        &record.tx_hash,
        &record.status,
        record.confirmed_block,
    );
    let signature = sk.sign(msg.as_bytes()).to_hex();
    let payload = TransitionGossipMessage::AdmissionTxNotice {
        applicant: record.applicant.clone(),
        nonce: record.nonce.clone(),
        validator_set_epoch: record.validator_set_epoch.clone(),
        tx_hash: record.tx_hash.clone(),
        status: record.status.clone(),
        confirmed_block: record.confirmed_block,
        submitter: record.submitter.clone(),
        validator_id: validator_id.to_owned(),
        validator_pubkey: pubkey,
        signature,
        delivery_id: uuid::Uuid::new_v4(),
    };
    p2p.sender
        .send(crate::p2p::P2PCommand::Broadcast {
            topic: TRANSITION_TOPIC.to_owned(),
            data: serde_json::to_vec(&payload)?,
        })
        .await
        .context("admission tx notice broadcast failed")?;
    Ok(())
}

async fn check_plan(
    state: &SharedState,
    plan: &ApprovalPlan,
    signer: Address,
) -> anyhow::Result<()> {
    let s = state.read().await;
    let applicant_key = format!("0x{:040x}", plan.applicant);
    let registration = s
        .validator_registrations
        .get(&applicant_key)
        .filter(|record| record.registered_with_node && record.identity.normalized().is_complete())
        .context("waiting for verified applicant registration")?;
    anyhow::ensure!(
        registration.identity.evm_address.parse::<Address>()? == plan.applicant,
        "the applicant does not match the registered staking wallet"
    );
    let tip_height = s.chain.tip_height()?;
    let tip_hash = s.chain.tip_hash()?;
    let authority = crate::validator_set_history::verification_set_at(
        &s.config.data_dir,
        tip_height
            .checked_add(1)
            .context("invalid chain tip height")?,
        &s.validator_set_bootstrap,
    )
    .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        matches_node_wallet(&authority, &s.config.node_id, signer),
        "the admission signer does not match this validator in certified chain history"
    );
    plan.validate_transition(&authority, &registration.identity, tip_height, &tip_hash)
}

struct TickCtx {
    rpc_url: String,
    staking_addr: Address,
    chain_id: u64,
}

async fn tick_legacy_plan(
    state: &SharedState,
    store: &AttestationStore,
    p2p: &crate::p2p::P2PHandle,
    settings: &AdmissionSettings,
) -> anyhow::Result<()> {
    if !state.read().await.config.is_validator {
        return Ok(());
    }
    let chain_id = store
        .signed_l1_chain_id
        .context("admission requires a signed chain specification")?;
    let ctx = {
        let s = state.read().await;
        anyhow::ensure!(
            matches_node_wallet(&s.validator_set, &s.config.node_id, settings.address),
            "the admission signer does not match this validator's active staking wallet"
        );
        TickCtx {
            rpc_url: s.config.eth_rpc_url.clone(),
            staking_addr: s.config.staking_addr.parse()?,
            chain_id,
        }
    };
    let provider = crate::rpc_failover::build_l1_provider(&ctx.rpc_url)
        .context("could not build the admission RPC client")?;
    anyhow::ensure!(
        provider
            .get_chain_id()
            .await
            .context("could not verify the admission RPC chain ID")?
            == ctx.chain_id,
        "admission RPC does not match the signed L1 chain ID"
    );
    let contract = IStakingAdmission::new(ctx.staking_addr, &provider);
    let entry = contract
        .validators(settings.address)
        .call()
        .await
        .context("could not read the admission signer's staking status")?;
    anyhow::ensure!(entry.state == 2u8, "the admission signer is inactive on L1");

    let Some(plan) = settings.plan.as_ref() else {
        return Ok(());
    };
    process_applicant(state, store, p2p, settings, plan, &ctx)
        .await
        .context("could not process the staged validator admission")
}

async fn process_applicant(
    state: &SharedState,
    store: &AttestationStore,
    p2p: &crate::p2p::P2PHandle,
    settings: &AdmissionSettings,
    plan: &ApprovalPlan,
    ctx: &TickCtx,
) -> anyhow::Result<()> {
    let applicant = plan.applicant;
    let applicant_key = format!("0x{:040x}", applicant);
    let provider = crate::rpc_failover::build_l1_provider(&ctx.rpc_url)?;
    let contract = IStakingAdmission::new(ctx.staking_addr, &provider);

    let v = contract.validators(applicant).call().await?;

    if v.state != 1u8 {
        store.drop_applicant(&applicant_key).await;
        return Ok(());
    }
    if v.stake < contract.minValidatorStake().call().await?._0 {
        return Ok(());
    }
    check_plan(state, plan, settings.address).await?;
    if v.ejectedAt > U256::ZERO && v.appliedAt <= v.ejectedAt {
        return Ok(());
    }
    let now = U256::from(chrono::Utc::now().timestamp() as u64);
    if now >= v.appliedAt + contract.APPLICATION_TIMEOUT().call().await?._0 {
        store.drop_applicant(&applicant_key).await;
        return Ok(());
    }

    let nonce = v.appliedAt;
    let stake = v.stake;
    anyhow::ensure!(
        nonce == U256::from(plan.application_nonce),
        "the current application nonce differs from the staged plan"
    );

    if contract
        .consensusNonceUsed(applicant, nonce)
        .call()
        .await?
        ._0
    {
        store.drop_applicant(&applicant_key).await;
        return Ok(());
    }

    let signing_epoch = contract.validatorSetEpoch().call().await?._0;
    anyhow::ensure!(
        signing_epoch == U256::from(plan.validator_set_epoch),
        "the current L1 validator epoch differs from the staged plan"
    );

    let rule_set_version = contract.RULE_SET_VERSION().call().await?._0;
    let att = sign_attestation(
        settings.key.expose_secret(),
        ctx.chain_id,
        ctx.staking_addr,
        applicant,
        stake,
        nonce,
        rule_set_version,
        signing_epoch,
    )
    .await?;
    if store.insert(att.clone()).await {
        tracing::info!(%applicant, %signing_epoch, "Signed validator admission attestation");
    }
    let data = gossip_payload(att)?;
    p2p.sender
        .send(crate::p2p::P2PCommand::Broadcast {
            topic: TOPIC.to_owned(),
            data,
        })
        .await
        .context("admission gossip channel is unavailable")?;
    if settings.address != plan.submitter {
        return Ok(());
    }
    let snapshot_key = AttestationKey {
        applicant: applicant_key.to_ascii_lowercase(),
        nonce: format!("0x{:x}", nonce),
        validator_set_epoch: format!("0x{:x}", signing_epoch),
    };
    let atts = store.snapshot(&snapshot_key).await;

    if contract.validatorSetEpoch().call().await?._0 != signing_epoch {
        return Ok(());
    }

    let verified: Vec<AdmissionAttestation> = atts
        .into_iter()
        .filter(|a| {
            let matches_context = a
                .applicant
                .parse::<Address>()
                .map(|value| value == applicant)
                .unwrap_or(false)
                && parse_u256(&a.stake)
                    .map(|value| value == stake)
                    .unwrap_or(false)
                && parse_u256(&a.nonce)
                    .map(|value| value == nonce)
                    .unwrap_or(false)
                && parse_u256(&a.rule_set_version)
                    .map(|value| value == rule_set_version)
                    .unwrap_or(false);
            matches_context
                && verify_attestation_for_epoch(a, ctx.chain_id, ctx.staking_addr, signing_epoch)
                    .unwrap_or(false)
        })
        .collect();

    let mut filtered: Vec<(Address, AdmissionAttestation)> = Vec::with_capacity(verified.len());
    for a in verified {
        let signer_addr: Address = a.signer.parse()?;
        let info = contract.validators(signer_addr).call().await?;
        if info.state == 2u8 {
            filtered.push((signer_addr, a));
        }
    }

    filtered.sort_by_key(|(addr, _)| *addr);
    filtered.dedup_by_key(|(addr, _)| *addr);

    let active_count = contract.activeValidatorCount().call().await?._0;
    if contract.validatorSetEpoch().call().await?._0 != signing_epoch {
        return Ok(());
    }
    if active_count.is_zero()
        || U256::from(filtered.len() as u64) * U256::from(3u64) < active_count * U256::from(2u64)
    {
        return Ok(());
    }

    check_plan(state, plan, settings.address).await?;
    submit_consensus_approval(
        &ctx.rpc_url,
        settings.key.expose_secret(),
        ctx.staking_addr,
        applicant,
        nonce,
        &filtered,
    )
    .await?;
    store.drop_applicant(&applicant_key).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn submit_consensus_approval_tracked(
    rpc_url: &str,
    priv_key: &str,
    submitter_addr: Address,
    staking_addr: Address,
    applicant: Address,
    nonce: U256,
    signing_epoch: U256,
    filtered: &[(Address, AdmissionAttestation)],
    store: &AttestationStore,
    p2p: &crate::p2p::P2PHandle,
    validator_id: &str,
    validator_priv_hex: Option<&str>,
) -> anyhow::Result<()> {
    if priv_key.is_empty() {
        anyhow::bail!("no admission signer available for submitter");
    }
    let signer: PrivateKeySigner = priv_key.parse()?;
    let wallet = EthereumWallet::from(signer);
    let base_provider = crate::rpc_failover::build_l1_provider(rpc_url)?;
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_provider(base_provider);
    let contract = IStakingAdmission::new(staking_addr, &provider);

    let signers: Vec<Address> = filtered.iter().map(|(a, _)| *a).collect();
    let sigs: Vec<Bytes> = filtered
        .iter()
        .map(|(_, a)| Bytes::from(hex_decode(&a.signature).unwrap_or_default()))
        .collect();

    let applicant_key = format!("0x{:040x}", applicant);
    let nonce_hex = format!("0x{:x}", nonce);
    let epoch_hex = format!("0x{:x}", signing_epoch);
    let submitter_hex = format!("0x{:040x}", submitter_addr);

    tracing::info!(
        "admission: simulating & submitting approveByConsensus for {} (nonce={}, signers={})",
        applicant,
        nonce,
        signers.len()
    );

    let call = contract.approveByConsensus(applicant, nonce, signers, sigs);
    call.call().await?;
    let pending = call.send().await?;
    let tx_hash_hex = format!("{}", pending.tx_hash());
    let submitted_at = now_ms();

    let submitted_rec = L1TxRecord {
        applicant: applicant_key.clone(),
        nonce: nonce_hex.clone(),
        validator_set_epoch: epoch_hex.clone(),
        tx_hash: tx_hash_hex.clone(),
        status: "submitted".to_string(),
        submitter: submitter_hex.clone(),
        submitted_at_ms: submitted_at,
        confirmed_at_ms: None,
        confirmed_block: None,
        last_error: None,
    };
    store.upsert_tx_record(submitted_rec.clone()).await;
    store
        .update_timings(&applicant_key, |t| {
            if t.l1_tx_submitted_at_ms.is_none() {
                t.l1_tx_submitted_at_ms = Some(submitted_at);
            }
        })
        .await;
    if let Some(priv_hex) = validator_priv_hex {
        let _ = broadcast_tx_notice(p2p, validator_id, priv_hex, &submitted_rec).await;
    }

    let receipt = pending.get_receipt().await?;
    let confirmed_at = now_ms();
    let status = if receipt.status() {
        "confirmed"
    } else {
        "reverted"
    };
    let confirmed_rec = L1TxRecord {
        applicant: applicant_key.clone(),
        nonce: nonce_hex,
        validator_set_epoch: epoch_hex,
        tx_hash: tx_hash_hex,
        status: status.to_string(),
        submitter: submitter_hex,
        submitted_at_ms: submitted_at,
        confirmed_at_ms: receipt.status().then_some(confirmed_at),
        confirmed_block: receipt.block_number,
        last_error: (!receipt.status()).then_some("approveByConsensus reverted".to_string()),
    };
    store.upsert_tx_record(confirmed_rec.clone()).await;
    if receipt.status() {
        store
            .update_timings(&applicant_key, |t| {
                if t.l1_confirmed_at_ms.is_none() {
                    t.l1_confirmed_at_ms = Some(confirmed_at);
                }
            })
            .await;
    }
    if let Some(priv_hex) = validator_priv_hex {
        let _ = broadcast_tx_notice(p2p, validator_id, priv_hex, &confirmed_rec).await;
    }
    anyhow::ensure!(receipt.status(), "validator admission transaction reverted");
    tracing::info!(
        "admission: approveByConsensus mined (tx={}, block={})",
        receipt.transaction_hash,
        receipt.block_number.unwrap_or_default()
    );
    Ok(())
}

async fn submit_consensus_approval(
    rpc_url: &str,
    priv_key: &str,
    staking_addr: Address,
    applicant: Address,
    nonce: U256,
    filtered: &[(Address, AdmissionAttestation)],
) -> anyhow::Result<()> {
    if priv_key.is_empty() {
        anyhow::bail!("no admission signer available for submitter");
    }
    let signer: PrivateKeySigner = priv_key.parse()?;
    let wallet = EthereumWallet::from(signer);
    let base_provider = crate::rpc_failover::build_l1_provider(rpc_url)?;
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_provider(base_provider);
    let contract = IStakingAdmission::new(staking_addr, &provider);

    let signers: Vec<Address> = filtered.iter().map(|(a, _)| *a).collect();
    let sigs: Vec<Bytes> = filtered
        .iter()
        .map(|(_, a)| {
            let raw = hex_decode(&a.signature).unwrap_or_default();
            Bytes::from(raw)
        })
        .collect();

    tracing::info!(
        "admission: submitting approveByConsensus for {} (nonce={}, signers={})",
        applicant,
        nonce,
        signers.len()
    );

    let call = contract.approveByConsensus(applicant, nonce, signers, sigs);
    call.call().await?;
    let pending = call.send().await?;
    let receipt = pending.get_receipt().await?;
    anyhow::ensure!(receipt.status(), "validator admission transaction reverted");
    tracing::info!(
        "admission: approveByConsensus mined (tx={}, block={})",
        receipt.transaction_hash,
        receipt.block_number.unwrap_or_default()
    );
    Ok(())
}

pub async fn accept_gossip(
    store: &AttestationStore,
    state: &SharedState,
    data: &[u8],
) -> anyhow::Result<bool> {
    let Some(chain_id) = store.signed_l1_chain_id else {
        return Ok(false);
    };
    let GossipAttestation { attestation, .. } = serde_json::from_slice(data)?;
    let applicant: Address = attestation.applicant.parse()?;
    let signer: Address = attestation.signer.parse()?;
    let staking_addr = {
        let s = state.read().await;
        let tip_height = s.chain.tip_height().unwrap_or(0);
        let authority = crate::validator_set_history::verification_set_at(
            &s.config.data_dir,
            tip_height.saturating_add(1),
            &s.validator_set_bootstrap,
        )
        .unwrap_or_else(|_| s.validator_set.clone());
        let is_active = active_signer(&authority, signer) || active_signer(&s.validator_set, signer);
        if !is_active
            || !s
                .validator_registrations
                .get(&format!("0x{:040x}", applicant))
                .is_some_and(|r| {
                    r.registered_with_node
                        && (r.staking_state.is_empty() || r.staking_state == "pending")
                })
        {
            return Ok(false);
        }
        s.config.staking_addr.parse()?
    };
    anyhow::ensure!(
        verify_attestation(&attestation, chain_id, staking_addr)?,
        "invalid admission signature"
    );
    store.insert(attestation).await;
    Ok(true)
}

pub(crate) async fn accept_transition_gossip(
    store: &AttestationStore,
    state: &SharedState,
    data: &[u8],
) -> anyhow::Result<bool> {
    let msg: TransitionGossipMessage = serde_json::from_slice(data)?;
    match msg {
        TransitionGossipMessage::TransitionApproval {
            applicant,
            effective_from_height,
            previous_block_hash,
            previous_validator_set_hash,
            validator_set_hash,
            signature,
            ..
        } => {
            let applicant_addr: Address = applicant.parse()?;
            let applicant_key = format!("0x{:040x}", applicant_addr);
            let (tip_height, tip_hash, authority, registration_ok) = {
                let s = state.read().await;
                let tip_height = s.chain.tip_height()?;
                let tip_hash = s.chain.tip_hash()?;
                let authority = crate::validator_set_history::verification_set_at(
                    &s.config.data_dir,
                    tip_height.saturating_add(1),
                    &s.validator_set_bootstrap,
                )
                .map_err(anyhow::Error::msg)?;
                let reg_ok = s
                    .validator_registrations
                    .get(&applicant_key)
                    .is_some_and(|r| r.registered_with_node && r.identity.normalized().is_complete());
                (tip_height, tip_hash, authority, reg_ok)
            };

            if !registration_ok
                || effective_from_height != tip_height.saturating_add(1)
                || previous_block_hash != tip_hash
                || previous_validator_set_hash != common::canonical_validator_set_hash(&authority)
            {
                return Ok(false);
            }

            let signer = authority
                .validators
                .iter()
                .find(|v| {
                    v.id == signature.validator_id
                        && common::validator_status_is_active(&v.status)
                })
                .context("transition approval signer is not an active validator")?;
            anyhow::ensure!(
                signer.pubkey.eq_ignore_ascii_case(&signature.validator_pubkey),
                "transition approval signer pubkey mismatch"
            );

            let placeholder = common::AuthenticatedValidatorSetSnapshot {
                schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
                effective_from_height,
                previous_block_hash: previous_block_hash.clone(),
                previous_validator_set_hash: previous_validator_set_hash.clone(),
                validators: common::ValidatorSet::default(),
                validator_set_hash: validator_set_hash.clone(),
                transition_signatures: Vec::new(),
            };
            let message = common::validator_set_transition_message(&placeholder);
            let pubkey_bytes = hex::decode(&signer.pubkey)?;
            let sig_bytes = hex::decode(signature.signature.trim().trim_start_matches("0x"))?;
            let pk = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey_bytes)
                .map_err(|e| anyhow::anyhow!("invalid transition signer pubkey: {e}"))?;
            let sig = Signature::from_bytes(SignatureScheme::Ed25519, &sig_bytes)
                .map_err(|e| anyhow::anyhow!("invalid transition signature: {e}"))?;
            pk.verify(message.as_bytes(), &sig, VerifyMode::Strict)
                .map_err(|e| anyhow::anyhow!("transition signature verification failed: {e}"))?;

            let key = TransitionProposalKey {
                applicant: applicant_key,
                effective_from_height,
                previous_block_hash,
                validator_set_hash,
            };
            store.insert_transition_approval(key, signature).await;
            Ok(true)
        }
        TransitionGossipMessage::AdmissionTxNotice {
            applicant,
            nonce,
            validator_set_epoch,
            tx_hash,
            status,
            confirmed_block,
            submitter,
            validator_id,
            validator_pubkey,
            signature,
            ..
        } => {
            if !matches!(status.as_str(), "submitted" | "confirmed" | "reverted") {
                anyhow::bail!("unsupported admission tx notice status");
            }
            let authority = {
                let s = state.read().await;
                let tip_height = s.chain.tip_height()?;
                crate::validator_set_history::verification_set_at(
                    &s.config.data_dir,
                    tip_height.saturating_add(1),
                    &s.validator_set_bootstrap,
                )
                .map_err(anyhow::Error::msg)?
            };
            let signer = authority
                .validators
                .iter()
                .find(|v| v.id == validator_id && common::validator_status_is_active(&v.status))
                .context("tx notice signer is not an active validator")?;
            anyhow::ensure!(
                signer.pubkey.eq_ignore_ascii_case(&validator_pubkey),
                "tx notice pubkey mismatch"
            );
            let msg = tx_notice_message(
                &applicant,
                &nonce,
                &validator_set_epoch,
                &tx_hash,
                &status,
                confirmed_block,
            );
            let pk = PublicKey::from_bytes(SignatureScheme::Ed25519, &hex::decode(&signer.pubkey)?)
                .map_err(|e| anyhow::anyhow!("invalid tx notice pubkey: {e}"))?;
            let sig = Signature::from_bytes(
                SignatureScheme::Ed25519,
                &hex::decode(signature.trim().trim_start_matches("0x"))?,
            )
            .map_err(|e| anyhow::anyhow!("invalid tx notice sig: {e}"))?;
            pk.verify(msg.as_bytes(), &sig, VerifyMode::Strict)
                .map_err(|e| anyhow::anyhow!("tx notice verification failed: {e}"))?;

            let now = now_ms();
            let applicant_key = applicant.to_ascii_lowercase();
            store
                .upsert_tx_record(L1TxRecord {
                    applicant: applicant_key.clone(),
                    nonce,
                    validator_set_epoch,
                    tx_hash,
                    status: status.clone(),
                    submitter: submitter.to_ascii_lowercase(),
                    submitted_at_ms: now,
                    confirmed_at_ms: (status == "confirmed").then_some(now),
                    confirmed_block,
                    last_error: None,
                })
                .await;
            store
                .update_timings(&applicant_key, |t| {
                    if t.l1_tx_submitted_at_ms.is_none() {
                        t.l1_tx_submitted_at_ms = Some(now);
                    }
                    if status == "confirmed" && t.l1_confirmed_at_ms.is_none() {
                        t.l1_confirmed_at_ms = Some(now);
                    }
                })
                .await;
            Ok(true)
        }
    }
}

pub async fn build_admission_status_report(
    state: &SharedState,
    store: &AttestationStore,
    applicant_filter: Option<&str>,
    node_id_filter: Option<&str>,
) -> AdmissionStatusReport {
    let (
        chain_id,
        node_id,
        is_validator,
        admission_auto_activate,
        data_dir,
        bootstrap,
        registrations,
        tip_height,
        tip_hash,
        finalized_height,
        validator_set,
    ) = {
        let s = state.read().await;
        let tip_height = s.chain.tip_height().unwrap_or(0);
        let tip_hash = s.chain.tip_hash().unwrap_or_default();
        (
            s.config.chain_id.clone(),
            s.config.node_id.clone(),
            s.config.is_validator,
            s.config.admission_auto_activate,
            s.config.data_dir.clone(),
            s.validator_set_bootstrap.clone(),
            s.validator_registrations.clone(),
            tip_height,
            tip_hash,
            tip_height,
            s.validator_set.clone(),
        )
    };

    let authority = crate::validator_set_history::verification_set_at(
        &data_dir,
        tip_height.saturating_add(1),
        &bootstrap,
    )
    .unwrap_or(validator_set);
    let active_in_authority = authority
        .validators
        .iter()
        .filter(|v| common::validator_status_is_active(&v.status))
        .count();
    let required_transition = transition_quorum_required(active_in_authority) as u64;

    let store_snapshot = {
        let g = store.inner.lock().await;
        (
            g.local_signer_status.clone(),
            g.cached_l1.clone(),
            g.attestations.clone(),
            g.transition_approvals.clone(),
            g.tx_records.clone(),
            g.timings.clone(),
            g.first_finalized_votes.clone(),
        )
    };
    let (
        local_signer,
        cached_l1,
        attestations,
        transition_approvals,
        tx_records,
        timings_map,
        cached_votes,
    ) = store_snapshot;

    let active_validators = if cached_l1.active_validators > 0 {
        cached_l1.active_validators
    } else {
        active_in_authority as u64
    };
    let required_l1 = if cached_l1.required_l1_approvals > 0 {
        cached_l1.required_l1_approvals
    } else {
        l1_quorum_required(active_validators)
    };

    let applicant_filter_norm = applicant_filter
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_ascii_lowercase);
    let node_id_filter_norm = node_id_filter
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);

    let mut applicants = Vec::new();
    let mut keys: Vec<String> = registrations.keys().cloned().collect();
    keys.sort();

    for key in keys {
        let Some(reg) = registrations.get(&key) else {
            continue;
        };
        let identity = reg.identity.normalized();
        let applicant_addr = identity.evm_address.clone();
        if let Some(ref filter) = applicant_filter_norm {
            if &applicant_addr != filter {
                continue;
            }
        }
        if let Some(ref filter) = node_id_filter_norm {
            if &identity.node_id != filter {
                continue;
            }
        }

        let l1_info = cached_l1.applicants.get(&key);
        let staking_state = l1_info
            .map(|c| c.staking_state.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if reg.staking_state.is_empty() {
                    "unknown".to_string()
                } else {
                    reg.staking_state.clone()
                }
            });
        let stake_creg = l1_info.map(|c| c.stake_creg).unwrap_or(reg.stake);
        let application_nonce = l1_info.map(|c| c.applied_at).filter(|&n| n > 0);
        let expires_at = l1_info
            .and_then(|c| c.expires_at)
            .and_then(|ts| chrono::DateTime::<chrono::Utc>::from_timestamp(ts as i64, 0))
            .map(|dt| dt.to_rfc3339());

        // Find latest matching tx_record for this applicant
        let tx_rec = tx_records
            .values()
            .filter(|r| r.applicant.eq_ignore_ascii_case(&applicant_addr))
            .max_by_key(|r| r.submitted_at_ms)
            .cloned();

        // Count EIP-712 approvals for current epoch & nonce (or latest matching key)
        let mut l1_signers: Vec<String> = l1_info
            .map(|c| c.active_l1_signers.clone())
            .unwrap_or_default();
        if l1_signers.is_empty() {
            if let (Some(nonce), Some(epoch)) = (application_nonce, cached_l1.validator_set_epoch) {
                let k = AttestationKey {
                    applicant: applicant_addr.clone(),
                    nonce: format!("0x{:x}", nonce),
                    validator_set_epoch: format!("0x{:x}", epoch),
                };
                if let Some(m) = attestations.get(&k) {
                    l1_signers = m.keys().cloned().collect();
                    l1_signers.sort();
                }
            }
        }

        let mut transition_signers = Vec::new();
        let next_height = tip_height.saturating_add(1);
        for (pk, sigs) in &transition_approvals {
            if pk.applicant.eq_ignore_ascii_case(&applicant_addr)
                && pk.effective_from_height == next_height
                && pk.previous_block_hash == tip_hash
            {
                transition_signers = sigs.keys().cloned().collect();
                transition_signers.sort();
                break;
            }
        }

        let voting_activation_height = if identity.is_complete() {
            crate::validator_set_history::certified_activation_height_for(
                &data_dir,
                &identity.node_id,
                &identity.ed25519_pubkey,
                &applicant_addr,
            )
            .ok()
            .flatten()
        } else {
            None
        };

        let mut first_finalized_vote_height = cached_votes.get(&applicant_addr).copied();
        if first_finalized_vote_height.is_none() {
            if let Some(act_h) = voting_activation_height {
                first_finalized_vote_height = find_first_finalized_vote_height(
                    state,
                    &identity.node_id,
                    &identity.ed25519_pubkey,
                    act_h,
                    tip_height,
                )
                .await;
                if let Some(vote_h) = first_finalized_vote_height {
                    store.record_first_vote(&applicant_addr, vote_h).await;
                }
            }
        }

        let mut missing_signer_setup = Vec::new();
        if local_signer.checked {
            if !local_signer.configured {
                missing_signer_setup.push(
                    local_signer
                        .issue
                        .clone()
                        .unwrap_or_else(|| "Local admission signer is not configured".to_string()),
                );
            } else if !local_signer.matches_active_wallet || !local_signer.active_on_l1 {
                if let Some(ref issue) = local_signer.issue {
                    missing_signer_setup.push(issue.clone());
                }
            }
        }

        let (l1_tx_hash, l1_tx_status, l1_confirmed_block) = match tx_rec.as_ref() {
            Some(rec) => (
                Some(rec.tx_hash.clone()),
                rec.status.clone(),
                rec.confirmed_block,
            ),
            None => (
                None,
                if staking_state == "active" {
                    "confirmed_on_chain".to_string()
                } else {
                    "not_submitted".to_string()
                },
                None,
            ),
        };

        let in_certified_set = candidate_in_authority(&authority, &identity).unwrap_or(false);
        let (stage, stage_reason) = if let Some(vote_h) = first_finalized_vote_height {
            (
                "active_voting".to_string(),
                format!(
                    "Certified at block #{}; first finalized vote recorded at block #{vote_h}",
                    voting_activation_height.unwrap_or(vote_h)
                ),
            )
        } else if in_certified_set {
            (
                "activated_awaiting_first_vote".to_string(),
                format!(
                    "Certified in R7 voting set at block #{}; waiting for next finalized block containing this validator's PBFT vote",
                    voting_activation_height.unwrap_or(next_height)
                ),
            )
        } else if let Some(err) = cached_l1.rpc_error.as_ref() {
            (
                "blocked_l1_rpc".to_string(),
                format!("Waiting for healthy Sepolia RPC: {err}"),
            )
        } else if let Some(issue) = l1_info.and_then(|c| c.l1_issue.as_ref()) {
            let st = if staking_state == "active" {
                "waiting_l1_finality"
            } else if staking_state == "expired" || staking_state == "rejected" {
                "terminal"
            } else {
                "blocked"
            };
            (st.to_string(), issue.clone())
        } else if l1_info.and_then(|c| c.queue_position).is_some_and(|p| p > 0) {
            (
                "queued".to_string(),
                format!(
                    "Queued behind earlier applicant {}",
                    l1_info
                        .and_then(|c| c.head_applicant.as_deref())
                        .unwrap_or("unknown")
                ),
            )
        } else if staking_state == "active" {
            (
                "collecting_transition_quorum".to_string(),
                format!(
                    "Approved on Sepolia; collecting Ed25519 voting-set transition signatures for block #{next_height} ({}/{required_transition})",
                    transition_signers.len()
                ),
            )
        } else if staking_state == "pending" {
            if (l1_signers.len() as u64) >= required_l1 && required_l1 > 0 {
                if l1_tx_status == "submitted" {
                    (
                        "sepolia_tx_submitted".to_string(),
                        format!(
                            "Quorum reached ({}/{required_l1}); waiting for Sepolia confirmation of {}",
                            l1_signers.len(),
                            l1_tx_hash.as_deref().unwrap_or("pending tx")
                        ),
                    )
                } else if local_signer.checked && local_signer.configured && !local_signer.has_gas {
                    missing_signer_setup.push(
                        "Local validator wallet has insufficient Sepolia ETH gas to submit approveByConsensus"
                            .to_string(),
                    );
                    (
                        "awaiting_sepolia_submission".to_string(),
                        format!(
                            "Quorum reached ({}/{required_l1}); waiting for a funded validator wallet to submit approveByConsensus",
                            l1_signers.len()
                        ),
                    )
                } else {
                    (
                        "awaiting_sepolia_submission".to_string(),
                        format!(
                            "Quorum reached ({}/{required_l1}); simulating and submitting approveByConsensus to Sepolia",
                            l1_signers.len()
                        ),
                    )
                }
            } else if !missing_signer_setup.is_empty() && l1_signers.is_empty() {
                (
                    "blocked_signer_setup".to_string(),
                    format!(
                        "Awaiting validator approvals (0/{required_l1}); {}",
                        missing_signer_setup[0]
                    ),
                )
            } else {
                (
                    "collecting_approvals".to_string(),
                    format!(
                        "Collecting validator EIP-712 approvals ({}/{required_l1} collected); missing {} more active validator signature(s)",
                        l1_signers.len(),
                        required_l1.saturating_sub(l1_signers.len() as u64)
                    ),
                )
            }
        } else {
            (
                "awaiting_stake".to_string(),
                "Registered identity is waiting for Sepolia stake confirmation".to_string(),
            )
        };

        let timings = timings_map.get(&applicant_addr).cloned().unwrap_or_default();
        applicants.push(AdmissionProgressView {
            applicant: applicant_addr,
            node_id: identity.node_id,
            ed25519_pubkey: identity.ed25519_pubkey,
            stage,
            stage_reason,
            staking_state: staking_state.clone(),
            stake_creg,
            application_nonce,
            expires_at,
            l1_approvals_collected: if staking_state == "active" {
                required_l1
            } else {
                l1_signers.len() as u64
            },
            l1_approvals_required: required_l1,
            l1_signers,
            transition_approvals_collected: if in_certified_set {
                required_transition
            } else {
                transition_signers.len() as u64
            },
            transition_approvals_required: required_transition,
            transition_signers,
            missing_signer_setup,
            l1_tx_hash,
            l1_tx_status,
            l1_confirmed_block,
            voting_activation_height,
            first_finalized_vote_height,
            timings,
        });
    }

    AdmissionStatusReport {
        local_node: LocalNodeAdmissionSummary {
            node_id,
            is_validator,
            admission_auto_activate,
            admission_signer_configured: local_signer.configured,
            admission_signer_address: local_signer.address,
            admission_signer_matches_active_wallet: local_signer.matches_active_wallet,
            admission_signer_active_on_l1: local_signer.active_on_l1,
            admission_signer_has_gas: local_signer.has_gas,
            admission_signer_issue: local_signer.issue,
        },
        network: NetworkAdmissionSummary {
            chain_id,
            tip_height,
            finalized_height,
            active_validators,
            required_l1_approvals: required_l1,
            required_transition_approvals: required_transition,
            validator_set_epoch: cached_l1.validator_set_epoch,
            rpc_error: cached_l1.rpc_error,
        },
        applicants,
    }
}

pub async fn accept_peer_attestation(
    store: &AttestationStore,
    rpc_url: &str,
    staking_addr: Address,
    att: AdmissionAttestation,
) -> anyhow::Result<bool> {
    let provider = crate::rpc_failover::build_l1_provider(rpc_url)?;
    let contract = IStakingAdmission::new(staking_addr, &provider);
    let chain_id = provider.get_chain_id().await?;
    let current_epoch = contract.validatorSetEpoch().call().await?._0;

    if !verify_attestation_for_epoch(&att, chain_id, staking_addr, current_epoch)? {
        anyhow::bail!(
            "signature is invalid or validator-set epoch is not the current on-chain epoch"
        );
    }
    Ok(store.insert(att).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    fn member(id: &str, key: &SigningKey, address: Address) -> common::Validator {
        common::Validator {
            id: id.into(),
            alias: id.into(),
            pubkey: hex::encode(key.verifying_key().as_bytes()),
            eth_address: format!("0x{:040x}", address),
            stake: 100,
            reputation: 100,
            status: "active".into(),
            performance_score: 500,
            contribution_count: 0,
            pq_pubkey: None,
        }
    }

    #[test]
    fn auto_approval_requires_a_quorum_signed_single_addition_at_the_current_tip() {
        let keys = (1..=5)
            .map(|seed| SigningKey::from_bytes(&[seed; 32]))
            .collect::<Vec<_>>();
        let validators = keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                member(
                    &format!("validator-{i}"),
                    key,
                    Address::repeat_byte(i as u8 + 1),
                )
            })
            .collect::<Vec<_>>();
        let old = common::ValidatorSet::new(validators[..4].to_vec());
        let candidate = &validators[4];
        let identity = common::ValidatorIdentity {
            evm_address: candidate.eth_address.clone(),
            node_id: candidate.id.clone(),
            ed25519_pubkey: candidate.pubkey.clone(),
        };
        let next = common::ValidatorSet::new(validators.clone());
        let hash = "aa".repeat(32);
        let mut plan = ApprovalPlan {
            applicant: candidate.eth_address.parse().unwrap(),
            submitter: validators[0].eth_address.parse().unwrap(),
            application_nonce: 42,
            validator_set_epoch: 4,
            transition: common::AuthenticatedValidatorSetSnapshot {
                schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
                effective_from_height: 22,
                previous_block_hash: hash.clone(),
                previous_validator_set_hash: common::canonical_validator_set_hash(&old),
                validator_set_hash: common::canonical_validator_set_hash(&next),
                validators: next,
                transition_signatures: Vec::new(),
            },
        };
        let resign = |snapshot: &mut common::AuthenticatedValidatorSetSnapshot| {
            snapshot.validator_set_hash =
                common::canonical_validator_set_hash(&snapshot.validators);
            let message = common::validator_set_transition_message(snapshot);
            snapshot.transition_signatures = validators[..3]
                .iter()
                .zip(keys.iter())
                .map(|(v, key)| common::ValidatorSetTransitionSignature {
                    validator_id: v.id.clone(),
                    validator_pubkey: v.pubkey.clone(),
                    signature: hex::encode(key.sign(message.as_bytes()).to_bytes()),
                })
                .collect();
        };
        assert!(plan
            .validate_transition(&old, &identity, 21, &hash)
            .is_err());
        resign(&mut plan.transition);
        plan.validate_transition(&old, &identity, 21, &hash)
            .unwrap();
        assert!(plan
            .validate_transition(&old, &identity, 22, &hash)
            .is_err());
        assert!(plan
            .validate_transition(&old, &identity, 21, &"bb".repeat(32))
            .is_err());
        let mut wrong_identity = identity.clone();
        wrong_identity.ed25519_pubkey = "cc".repeat(32);
        assert!(plan
            .validate_transition(&old, &wrong_identity, 21, &hash)
            .is_err());
        plan.transition.validators.validators[0].stake += 1;
        resign(&mut plan.transition);
        assert!(plan
            .validate_transition(&old, &identity, 21, &hash)
            .is_err());
    }

    /// Fixed test vectors so that any divergence from the contract's digest
    /// shows up as a failing Rust test before it can bite us on-chain.
    #[test]
    fn domain_separator_matches_eip712_hash() {
        let staking: Address = "0xBabE000000000000000000000000000000000BaD"
            .parse()
            .unwrap();
        let dsep = domain_separator(1337, staking);
        // Re-derive via the same formula; this asserts the encoding path stays stable.
        let type_hash = keccak256(DOMAIN_TYPE.as_bytes());
        let name_hash = keccak256(DOMAIN_NAME.as_bytes());
        let version_hash = keccak256(DOMAIN_VERSION.as_bytes());
        let expected = keccak256(
            (
                type_hash,
                name_hash,
                version_hash,
                U256::from(1337u64),
                staking,
            )
                .abi_encode(),
        );
        assert_eq!(dsep, expected);
    }

    #[tokio::test]
    async fn sign_then_verify_roundtrip() {
        // 32-byte hex seed = known private key from foundry's default set.
        let pk = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let signer: PrivateKeySigner = pk.parse().unwrap();
        let signer_addr = signer.address();

        let staking: Address = "0x000000000000000000000000000000000000dEaD"
            .parse()
            .unwrap();
        let applicant: Address = "0x00000000000000000000000000000000000CAFEE"
            .parse()
            .unwrap();

        let att = sign_attestation(
            pk,
            31337,
            staking,
            applicant,
            U256::from(123u64),
            U256::from(1u64),
            U256::from(1u64),
            U256::from(7u64),
        )
        .await
        .expect("sign");

        assert!(att
            .signer
            .eq_ignore_ascii_case(&format!("0x{:040x}", signer_addr)));
        assert!(verify_attestation(&att, 31337, staking).unwrap());
        assert!(verify_attestation_for_epoch(&att, 31337, staking, U256::from(7u64)).unwrap());
        assert!(
            !verify_attestation_for_epoch(&att, 31337, staking, U256::from(8u64)).unwrap(),
            "a cryptographically valid stale-set attestation must fail closed"
        );

        // Flipping the chain_id must break recovery — the digest changes.
        assert!(!verify_attestation(&att, 1, staking).unwrap());

        let mut tampered = att.clone();
        tampered.validator_set_epoch = "0x8".to_owned();
        assert!(
            !verify_attestation(&tampered, 31337, staking).unwrap(),
            "changing the signed epoch must break recovery"
        );
    }

    #[tokio::test]
    async fn store_dedups_by_signer() {
        let store = AttestationStore::new();
        let pk = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let staking: Address = "0x000000000000000000000000000000000000dEaD"
            .parse()
            .unwrap();
        let applicant: Address = "0x00000000000000000000000000000000000CAFEE"
            .parse()
            .unwrap();

        let att = sign_attestation(
            pk,
            31337,
            staking,
            applicant,
            U256::from(123u64),
            U256::from(1u64),
            U256::from(1u64),
            U256::from(7u64),
        )
        .await
        .unwrap();

        assert!(store.insert(att.clone()).await);
        assert!(
            !store.insert(att.clone()).await,
            "second insert must be no-op"
        );

        let snap = store.snapshot(&att.key()).await;
        assert_eq!(snap.len(), 1);
    }

    #[tokio::test]
    async fn store_separates_same_signer_across_validator_set_epochs() {
        let store = AttestationStore::new();
        let pk = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let staking: Address = "0x000000000000000000000000000000000000dEaD"
            .parse()
            .unwrap();
        let applicant: Address = "0x00000000000000000000000000000000000CAFEE"
            .parse()
            .unwrap();

        let epoch_7 = sign_attestation(
            pk,
            31337,
            staking,
            applicant,
            U256::from(123u64),
            U256::from(1u64),
            U256::from(1u64),
            U256::from(7u64),
        )
        .await
        .unwrap();
        let epoch_8 = sign_attestation(
            pk,
            31337,
            staking,
            applicant,
            U256::from(123u64),
            U256::from(1u64),
            U256::from(1u64),
            U256::from(8u64),
        )
        .await
        .unwrap();

        assert!(store.insert(epoch_7.clone()).await);
        assert!(store.insert(epoch_8.clone()).await);
        assert_eq!(store.snapshot(&epoch_7.key()).await.len(), 1);
        assert_eq!(store.snapshot(&epoch_8.key()).await.len(), 1);
    }

    #[tokio::test]
    async fn store_caps_keys_and_signers_per_key() {
        let store = AttestationStore::new();
        for i in 0..(MAX_ATTESTATION_KEYS + 10) {
            let att = AdmissionAttestation {
                signer: format!("0x{:040x}", i + 1),
                applicant: format!("0x{:040x}", i + 1),
                stake: "0x1".into(),
                nonce: format!("0x{:x}", i + 1),
                rule_set_version: "0x1".into(),
                validator_set_epoch: "0x1".into(),
                signature: "00".repeat(65),
            };
            assert!(store.insert(att).await);
        }
        assert_eq!(store.len().await, MAX_ATTESTATION_KEYS);

        let shared_store = AttestationStore::new();
        for i in 0..MAX_SIGNERS_PER_KEY {
            let att = AdmissionAttestation {
                signer: format!("0x{:040x}", i + 1),
                applicant: "0x00000000000000000000000000000000000cafee".into(),
                stake: "0x1".into(),
                nonce: "0x1".into(),
                rule_set_version: "0x1".into(),
                validator_set_epoch: "0x1".into(),
                signature: "00".repeat(65),
            };
            assert!(shared_store.insert(att).await);
        }
        let extra = AdmissionAttestation {
            signer: format!("0x{:040x}", MAX_SIGNERS_PER_KEY + 1),
            applicant: "0x00000000000000000000000000000000000cafee".into(),
            stake: "0x1".into(),
            nonce: "0x1".into(),
            rule_set_version: "0x1".into(),
            validator_set_epoch: "0x1".into(),
            signature: "00".repeat(65),
        };
        assert!(!shared_store.insert(extra).await);
    }

    #[tokio::test]
    async fn durable_state_persists_and_reloads_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let pk = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let staking: Address = "0x000000000000000000000000000000000000dEaD"
            .parse()
            .unwrap();
        let applicant: Address = "0x00000000000000000000000000000000000CAFEE"
            .parse()
            .unwrap();
        let applicant_hex = format!("0x{:040x}", applicant);

        let att = sign_attestation(
            pk,
            31337,
            staking,
            applicant,
            U256::from(100u64),
            U256::from(42u64),
            U256::from(1u64),
            U256::from(4u64),
        )
        .await
        .unwrap();
        let att_key = att.key();

        let pkey = TransitionProposalKey {
            applicant: applicant_hex.clone(),
            effective_from_height: 22,
            previous_block_hash: "aa".repeat(32),
            validator_set_hash: "bb".repeat(32),
        };
        let tsig = common::ValidatorSetTransitionSignature {
            validator_id: "validator-1".into(),
            validator_pubkey: "cc".repeat(32),
            signature: "dd".repeat(64),
        };

        {
            let store = AttestationStore::for_chain_in_dir(Some(31337), Some(dir.path().to_path_buf()));
            assert!(store.insert(att.clone()).await);
            assert!(store.insert_transition_approval(pkey.clone(), tsig.clone()).await);
            store
                .upsert_tx_record(L1TxRecord {
                    applicant: applicant_hex.clone(),
                    nonce: "0x2a".into(),
                    validator_set_epoch: "0x4".into(),
                    tx_hash: "0x1234".into(),
                    status: "confirmed".into(),
                    submitter: "0x1111111111111111111111111111111111111111".into(),
                    submitted_at_ms: 1000,
                    confirmed_at_ms: Some(2500),
                    confirmed_block: Some(99),
                    last_error: None,
                })
                .await;
            store
                .update_timings(&applicant_hex, |t| {
                    t.started_at_ms = Some(500);
                    t.l1_quorum_reached_at_ms = Some(900);
                    t.l1_tx_submitted_at_ms = Some(1000);
                    t.l1_confirmed_at_ms = Some(2500);
                    t.transition_certified_at_ms = Some(3000);
                })
                .await;
            store.record_first_vote(&applicant_hex, 23).await;
        }

        let reloaded =
            AttestationStore::for_chain_in_dir(Some(31337), Some(dir.path().to_path_buf()));
        assert_eq!(reloaded.snapshot(&att_key).await.len(), 1);
        assert!(
            !reloaded.insert(att).await,
            "reloaded store must still dedup already persisted attestation"
        );
        assert_eq!(reloaded.snapshot_transition_approvals(&pkey).await.len(), 1);
        let tx = reloaded
            .latest_confirmed_tx_record(&applicant_hex, U256::from(42u64))
            .await
            .expect("confirmed tx record must reload");
        assert_eq!(tx.confirmed_block, Some(99));
        let g = reloaded.inner.lock().await;
        assert_eq!(g.first_finalized_votes.get(&applicant_hex).copied(), Some(23));
        let timings = g.timings.get(&applicant_hex).unwrap();
        assert_eq!(timings.signature_collection_ms, Some(400));
        assert_eq!(timings.l1_confirmation_ms, Some(1500));
        assert_eq!(timings.voting_activation_ms, Some(500));
    }

    #[tokio::test]
    async fn equivocation_guard_blocks_conflicting_transition_at_same_height_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttestationStore::for_chain_in_dir(Some(31337), Some(dir.path().to_path_buf()));
        assert!(
            store
                .check_and_record_signed_transition_height(22, &"aa".repeat(32))
                .await
        );
        assert!(
            store
                .check_and_record_signed_transition_height(22, &"aa".repeat(32))
                .await,
            "idempotent re-sign of identical hash at height 22 must be allowed"
        );
        assert!(
            !store
                .check_and_record_signed_transition_height(22, &"bb".repeat(32))
                .await,
            "distinct validator_set_hash at height 22 must be blocked"
        );
        drop(store);

        let reloaded =
            AttestationStore::for_chain_in_dir(Some(31337), Some(dir.path().to_path_buf()));
        assert!(
            !reloaded
                .check_and_record_signed_transition_height(22, &"bb".repeat(32))
                .await,
            "equivocation guard must survive restart"
        );
        assert!(
            reloaded
                .check_and_record_signed_transition_height(23, &"bb".repeat(32))
                .await,
            "advancing to height 23 allows signing next transition"
        );
    }

    #[tokio::test]
    async fn moving_chain_tip_prunes_stale_transition_approvals() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttestationStore::for_chain_in_dir(Some(31337), Some(dir.path().to_path_buf()));
        let stale_key = TransitionProposalKey {
            applicant: "0x0000000000000000000000000000000000000001".into(),
            effective_from_height: 21,
            previous_block_hash: "aa".repeat(32),
            validator_set_hash: "bb".repeat(32),
        };
        let next_key = TransitionProposalKey {
            applicant: "0x0000000000000000000000000000000000000001".into(),
            effective_from_height: 22,
            previous_block_hash: "cc".repeat(32),
            validator_set_hash: "dd".repeat(32),
        };
        let sig = common::ValidatorSetTransitionSignature {
            validator_id: "validator-1".into(),
            validator_pubkey: "ee".repeat(32),
            signature: "ff".repeat(64),
        };
        assert!(store.insert_transition_approval(stale_key.clone(), sig.clone()).await);
        assert!(store.insert_transition_approval(next_key.clone(), sig).await);
        store.prune_stale_transition_approvals(21).await;
        assert!(store.snapshot_transition_approvals(&stale_key).await.is_empty());
        assert_eq!(store.snapshot_transition_approvals(&next_key).await.len(), 1);
    }

    #[tokio::test]
    async fn observer_stays_non_validator_until_certified_transition_then_activates() {
        let dir = tempfile::tempdir().unwrap();
        let keys: Vec<_> = (1u8..=5)
            .map(|seed| SigningKey::from_bytes(&[seed; 32]))
            .collect();
        let all_validators: Vec<_> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                member(
                    &format!("validator-{}", i + 1),
                    k,
                    Address::repeat_byte((i + 1) as u8),
                )
            })
            .collect();
        let bootstrap = common::ValidatorSet::new(all_validators[..4].to_vec());
        let candidate = &all_validators[4];
        let candidate_priv = hex::encode(keys[4].to_bytes());

        let chain = crate::chain_store::ChainStore::open(dir.path()).unwrap();
        let genesis_hash = chain.tip_hash().unwrap();
        let genesis_ts = chain.get_block_by_height(0).unwrap().unwrap().header.timestamp;
        let block1 = common::Block {
            header: common::BlockHeader {
                height: 1,
                prev_hash: genesis_hash.clone(),
                merkle_root: common::merkle_root(&[]),
                proposer_id: "validator-1".into(),
                timestamp: genesis_ts + chrono::Duration::seconds(5),
                validator_set_hash: common::canonical_validator_set_hash(&bootstrap),
                vrf_output: None,
                vrf_proof: None,
                view_number: 0,
                view_change_certificate: Vec::new(),
                status_root: None,
            },
            transactions: Vec::new(),
            pbft_signatures: Vec::new(),
        };
        chain.insert_block(&block1).unwrap();
        crate::validator_set_history::record(dir.path(), 0, &bootstrap).unwrap();

        let state: SharedState = Arc::new(tokio::sync::RwLock::new(crate::NodeState {
            chain,
            pending_pool: crate::pending_pool::PendingPool::new(),
            publisher_index: crate::publisher_index::PublisherIndex::new(),
            validator_set_bootstrap: bootstrap.clone(),
            validator_set_snapshot: Arc::new(crate::state::ArcSwap::from_pointee(bootstrap.clone())),
            validator_set: bootstrap.clone(),
            package_rounds: HashMap::new(),
            package_committees: HashMap::new(),
            config: crate::config::NodeConfig {
                node_id: candidate.id.clone(),
                data_dir: dir.path().to_path_buf(),
                is_validator: false,
                admission_auto_activate: true,
                validator_privkey: Some(candidate_priv.into()),
                ..Default::default()
            },
            p2p_status: crate::P2PStatus::default(),
            bridge_status: crate::BridgeStatus::default(),
            vrf_proofs: HashMap::new(),
            decryption_shares: HashMap::new(),
            validator_registrations: HashMap::new(),
            validator_set_sync: crate::ValidatorSetSyncStatus::default(),
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

        // Before certified transition, candidate stays strictly in observer mode.
        assert!(!try_activate_candidate_validator(&state).await.unwrap());
        {
            let s = state.read().await;
            assert!(!s.config.is_validator);
            assert!(s.chain.pbft_safety_journal().is_err());
        }

        // Certify 5-validator set at effective height 2 anchored to block #1.
        let next_set = common::ValidatorSet::new(all_validators.clone());
        let mut snapshot = common::AuthenticatedValidatorSetSnapshot {
            schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height: 2,
            previous_block_hash: block1.hash(),
            previous_validator_set_hash: common::canonical_validator_set_hash(&bootstrap),
            validator_set_hash: common::canonical_validator_set_hash(&next_set),
            validators: next_set,
            transition_signatures: Vec::new(),
        };
        let msg = common::validator_set_transition_message(&snapshot);
        snapshot.transition_signatures = all_validators[..3]
            .iter()
            .zip(keys.iter())
            .map(|(v, sk)| common::ValidatorSetTransitionSignature {
                validator_id: v.id.clone(),
                validator_pubkey: v.pubkey.clone(),
                signature: hex::encode(sk.sign(msg.as_bytes()).to_bytes()),
            })
            .collect();
        crate::validator_set_history::record_authenticated(dir.path(), snapshot, &bootstrap)
            .unwrap();

        assert!(try_activate_candidate_validator(&state).await.unwrap());
        let s = state.read().await;
        assert!(s.config.is_validator);
        assert!(s.chain.pbft_safety_journal().is_ok());
        assert_eq!(s.validator_set.validators.len(), 5);
    }
}
