//! ADR-017 Increment 3c: enrol the ML-DSA-44 keys of validators that are
//! already certified members.
//!
//! A member proves possession of its key by registering again with a
//! `creg-pq-binding-v1` proof, which `api::apply_validator_registration`
//! verifies before storing the key. Every active validator then builds the
//! same next set (the certified set with each such member's `pq_pubkey`
//! filled in), signs it with the ordinary validator-set transition approval,
//! and installs it once `2f+1` approvals for that exact set arrive. Receivers,
//! including nodes on older builds, accept the result like any other
//! quorum-certified transition.
//!
//! Only members without a key are enrolled (rotation is out of scope), one
//! transition can enrol several members, and a quiet period lets
//! registrations settle so signers converge on one set per height. Admission
//! of a new validator takes precedence, because only one transition can be
//! certified per height.

use std::collections::{BTreeMap, HashMap, HashSet};

use alloy::primitives::Address;
use anyhow::Context;

use super::{
    now_ms, transition_quorum_required, AttestationStore, TransitionGossipMessage,
    TransitionProposalKey, TRANSITION_TOPIC,
};
use crate::state::SharedState;

/// Seconds the pending enrollment set must stay unchanged before this node
/// signs it (default 60).
pub(crate) const PQ_ENROLLMENT_QUIET_SECS_ENV: &str = "CREG_PQ_ENROLLMENT_QUIET_SECS";
const DEFAULT_QUIET_SECS: u64 = 60;

fn quiet_period_ms() -> i64 {
    let secs = std::env::var(PQ_ENROLLMENT_QUIET_SECS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_QUIET_SECS);
    i64::try_from(secs.saturating_mul(1_000)).unwrap_or(i64::MAX)
}

/// Active members of `authority` without an ML-DSA-44 key whose registration
/// carries a verified one for exactly their certified identity, keyed by
/// validator id. A key that would end up on two validators is skipped.
pub(crate) fn pending_pq_enrollments(
    authority: &common::ValidatorSet,
    registrations: &HashMap<String, crate::ValidatorRegistrationStatus>,
) -> BTreeMap<String, String> {
    let mut candidates = BTreeMap::new();
    for validator in authority.validators.iter().filter(|validator| {
        common::validator_status_is_active(&validator.status) && validator.pq_pubkey.is_none()
    }) {
        let Some(registration) =
            registrations.get(&crate::normalized_validator_key(&validator.eth_address))
        else {
            continue;
        };
        let Some(pq_pubkey) = registration.pq_pubkey.as_ref() else {
            continue;
        };
        let identity = registration.identity.normalized();
        if registration.registered_with_node
            && identity.node_id == validator.id
            && identity
                .ed25519_pubkey
                .eq_ignore_ascii_case(&validator.pubkey)
            && identity
                .evm_address
                .eq_ignore_ascii_case(&validator.eth_address)
        {
            candidates.insert(validator.id.clone(), pq_pubkey.clone());
        }
    }

    let mut uses: HashMap<&str, usize> = HashMap::new();
    for pq_pubkey in authority
        .validators
        .iter()
        .filter_map(|validator| validator.pq_pubkey.as_deref())
        .chain(candidates.values().map(String::as_str))
    {
        *uses.entry(pq_pubkey).or_default() += 1;
    }
    let shared: HashSet<String> = uses
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(pq_pubkey, _)| pq_pubkey.to_owned())
        .collect();
    candidates.retain(|_, pq_pubkey| !shared.contains(pq_pubkey));
    candidates
}

/// The certified set with each pending member's ML-DSA-44 key filled in.
pub(crate) fn enrollment_next_set(
    authority: &common::ValidatorSet,
    enrollments: &BTreeMap<String, String>,
) -> common::ValidatorSet {
    let mut next = authority.clone();
    for validator in &mut next.validators {
        if let Some(pq_pubkey) = enrollments.get(&validator.id) {
            validator.pq_pubkey = Some(pq_pubkey.clone());
        }
    }
    next
}

/// A key-enrollment transition keeps every member and field and only adds
/// `pq_pubkey` to members that had none, each one a key this node verified.
pub(crate) fn validate_pq_enrollment(
    current: &common::ValidatorSet,
    next: &common::ValidatorSet,
    verified: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        current.validators.len() == next.validators.len(),
        "a key enrollment must keep the validator set's membership"
    );
    let mut enrolled = 0usize;
    for (before, after) in current.validators.iter().zip(&next.validators) {
        let mut without_key_change = after.clone();
        without_key_change.pq_pubkey = before.pq_pubkey.clone();
        anyhow::ensure!(
            serde_json::to_value(&without_key_change)? == serde_json::to_value(before)?,
            "a key enrollment may only change ML-DSA-44 keys"
        );
        match (&before.pq_pubkey, &after.pq_pubkey) {
            (None, Some(pq_pubkey)) => {
                anyhow::ensure!(
                    verified.get(&after.id) == Some(pq_pubkey),
                    "validator {} would get an ML-DSA-44 key this node did not verify",
                    after.id
                );
                enrolled += 1;
            }
            (previous, proposed) => anyhow::ensure!(
                previous == proposed,
                "a key enrollment cannot replace or remove validator {}'s ML-DSA-44 key",
                after.id
            ),
        }
    }
    anyhow::ensure!(enrolled > 0, "a key enrollment must enrol at least one key");
    Ok(())
}

/// The approval gossip and store are keyed by one EVM address. Use the lowest
/// address among the enrolled members so every signer derives the same key,
/// and receivers find the registration they require for it.
fn proposal_applicant(
    authority: &common::ValidatorSet,
    enrollments: &BTreeMap<String, String>,
) -> anyhow::Result<String> {
    authority
        .validators
        .iter()
        .filter(|validator| enrollments.contains_key(&validator.id))
        .map(|validator| {
            validator
                .eth_address
                .parse::<Address>()
                .map(|address| format!("0x{address:040x}"))
        })
        .collect::<Result<Vec<_>, _>>()
        .context("an enrolled validator has an invalid EVM address")?
        .into_iter()
        .min()
        .context("no validator to enrol")
}

impl AttestationStore {
    /// True once `candidate_hash` has been the pending enrollment set for at
    /// least `quiet_ms`; a different candidate restarts the wait.
    async fn pq_enrollment_settled(&self, candidate_hash: &str, now: i64, quiet_ms: i64) -> bool {
        let mut g = self.inner.lock().await;
        match &g.pq_enrollment_watch {
            Some((hash, since)) if hash == candidate_hash => now.saturating_sub(*since) >= quiet_ms,
            _ => {
                g.pq_enrollment_watch = Some((candidate_hash.to_owned(), now));
                quiet_ms <= 0
            }
        }
    }

    async fn clear_pq_enrollment_watch(&self) {
        self.inner.lock().await.pq_enrollment_watch = None;
    }

    /// An applicant that is Active on L1 but not yet in the certified set is
    /// waiting for its admission transition.
    async fn admission_awaiting_transition(&self, authority: &common::ValidatorSet) -> bool {
        let g = self.inner.lock().await;
        g.cached_l1.applicants.iter().any(|(address, applicant)| {
            applicant.staking_state_code == 2
                && !authority
                    .validators
                    .iter()
                    .any(|validator| validator.eth_address.eq_ignore_ascii_case(address))
        })
    }
}

pub(super) async fn tick(
    state: &SharedState,
    store: &AttestationStore,
    p2p: &crate::p2p::P2PHandle,
) -> anyhow::Result<()> {
    tick_with_quiet_period(state, store, p2p, quiet_period_ms()).await
}

async fn tick_with_quiet_period(
    state: &SharedState,
    store: &AttestationStore,
    p2p: &crate::p2p::P2PHandle,
    quiet_ms: i64,
) -> anyhow::Result<()> {
    let (
        node_id,
        signing_keys,
        hybrid_activation_height,
        data_dir,
        bootstrap,
        registrations,
        tip_height,
        tip_hash,
    ) = {
        let s = state.read().await;
        if !s.config.is_validator {
            return Ok(());
        }
        let Some(signing_keys) = s
            .config
            .validator_signing_keys()
            .map_err(anyhow::Error::msg)?
        else {
            return Ok(());
        };
        (
            s.config.node_id.clone(),
            signing_keys,
            s.config.vote_v4_activation_height,
            s.config.data_dir.clone(),
            s.validator_set_bootstrap.clone(),
            s.validator_registrations.clone(),
            s.chain.tip_height()?,
            s.chain.tip_hash()?,
        )
    };
    let effective_from_height = tip_height
        .checked_add(1)
        .context("invalid chain tip height")?;
    let authority = crate::validator_set_history::verification_set_at(
        &data_dir,
        effective_from_height,
        &bootstrap,
    )
    .map_err(anyhow::Error::msg)?;

    let signer_pubkey = signing_keys.pubkey_hex();
    if !authority.validators.iter().any(|validator| {
        validator.id == node_id
            && validator.pubkey.eq_ignore_ascii_case(&signer_pubkey)
            && common::validator_status_is_active(&validator.status)
    }) {
        return Ok(());
    }

    let enrollments = pending_pq_enrollments(&authority, &registrations);
    if enrollments.is_empty() {
        store.clear_pq_enrollment_watch().await;
        return Ok(());
    }
    if store.admission_awaiting_transition(&authority).await {
        return Ok(());
    }
    let next_set = enrollment_next_set(&authority, &enrollments);
    validate_pq_enrollment(&authority, &next_set, &enrollments)?;
    common::validate_validator_identities(&next_set).map_err(anyhow::Error::msg)?;
    let next_set_hash = common::canonical_validator_set_hash(&next_set);
    if !store
        .pq_enrollment_settled(&next_set_hash, now_ms(), quiet_ms)
        .await
    {
        return Ok(());
    }

    // Never sign two distinct validator sets for the same height.
    if !store
        .check_and_record_signed_transition_height(effective_from_height, &next_set_hash)
        .await
    {
        tracing::debug!(
            effective_from_height,
            "Another validator-set transition was signed at this height; key enrollment waits for the next block"
        );
        return Ok(());
    }

    let previous_set_hash = common::canonical_validator_set_hash(&authority);
    let unsigned = common::AuthenticatedValidatorSetSnapshot {
        schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
        effective_from_height,
        previous_block_hash: tip_hash.clone(),
        previous_validator_set_hash: previous_set_hash.clone(),
        validators: next_set,
        validator_set_hash: next_set_hash.clone(),
        transition_signatures: Vec::new(),
    };
    let message = common::validator_set_transition_message(&unsigned);
    let (signature_hex, pq_signature) = signing_keys.sign_at(
        hybrid_activation_height,
        effective_from_height,
        message.as_bytes(),
    )?;
    let signature = common::ValidatorSetTransitionSignature {
        validator_id: node_id.clone(),
        validator_pubkey: signer_pubkey,
        signature: signature_hex,
        pq_signature,
    };
    let applicant = proposal_applicant(&authority, &enrollments)?;
    let proposal_key = TransitionProposalKey {
        applicant: applicant.clone(),
        effective_from_height,
        previous_block_hash: tip_hash.clone(),
        validator_set_hash: next_set_hash.clone(),
    };
    if store
        .insert_transition_approval(proposal_key.clone(), signature.clone())
        .await
    {
        tracing::info!(
            effective_from_height,
            validators = ?enrollments.keys().collect::<Vec<_>>(),
            "Signed ML-DSA-44 key enrollment"
        );
    }
    let approval = TransitionGossipMessage::TransitionApproval {
        applicant,
        effective_from_height,
        previous_block_hash: tip_hash.clone(),
        previous_validator_set_hash: previous_set_hash,
        validator_set_hash: next_set_hash,
        signature,
        delivery_id: uuid::Uuid::new_v4(),
    };
    p2p.sender
        .send(crate::p2p::P2PCommand::Broadcast {
            topic: TRANSITION_TOPIC.to_owned(),
            data: serde_json::to_vec(&approval)?,
        })
        .await
        .context("transition approval gossip channel is unavailable")?;

    let mut collected = store.snapshot_transition_approvals(&proposal_key).await;
    collected.sort_by(|a, b| a.validator_id.cmp(&b.validator_id));
    collected.dedup_by(|a, b| a.validator_id == b.validator_id);
    if collected.len() < transition_quorum_required(authority.validators.len()) {
        return Ok(());
    }

    let mut snapshot = unsigned;
    snapshot.transition_signatures = collected;
    validate_pq_enrollment(&authority, &snapshot.validators, &enrollments)?;
    {
        let mut s = state.write().await;
        crate::validator_set_history::validate_transition_at_tip(
            &snapshot,
            s.chain.tip_height()?,
            &s.chain.tip_hash()?,
        )?;
        crate::validator_set_history::record_authenticated(
            &s.config.data_dir,
            snapshot.clone(),
            &authority,
            hybrid_activation_height,
        )?;
        s.refresh_runtime_validator_set_from_certified_history()?;
    }
    store.clear_pq_enrollment_watch().await;

    let transition =
        serde_json::to_vec(&common::GossipMessage::ValidatorSetTransition { snapshot })?;
    if let Err(error) = p2p
        .sender
        .send(crate::p2p::P2PCommand::Broadcast {
            topic: crate::validator_registry_gossip::VALIDATOR_SET_TRANSITIONS_TOPIC.to_owned(),
            data: transition,
        })
        .await
    {
        tracing::warn!(%error, "Failed to broadcast the certified ML-DSA-44 key enrollment");
    }
    tracing::info!(
        effective_from_height,
        validators = ?enrollments.keys().collect::<Vec<_>>(),
        "Certified ML-DSA-44 key enrollment installed and broadcast"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::crypto::SigningKey;
    use std::sync::Arc;

    fn pq(byte: u8) -> String {
        format!("{byte:02x}").repeat(common::crypto::MLDSA44_PUBLIC_KEY_LEN)
    }

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_ed25519_seed(&[seed; 32])
    }

    fn member(index: u8) -> common::Validator {
        common::Validator {
            id: format!("validator-{index}"),
            alias: format!("validator-{index}"),
            pubkey: key(index).public_key().to_hex(),
            eth_address: format!("0x{:040x}", Address::repeat_byte(index)),
            stake: 100,
            reputation: 100,
            status: "active".into(),
            ..Default::default()
        }
    }

    fn registration(
        validator: &common::Validator,
        pq_pubkey: Option<String>,
    ) -> crate::ValidatorRegistrationStatus {
        crate::ValidatorRegistrationStatus {
            identity: common::ValidatorIdentity {
                evm_address: validator.eth_address.clone(),
                node_id: validator.id.clone(),
                ed25519_pubkey: validator.pubkey.clone(),
            },
            pq_pubkey,
            registered_with_node: true,
            ..Default::default()
        }
    }

    fn registrations(
        entries: impl IntoIterator<Item = crate::ValidatorRegistrationStatus>,
    ) -> HashMap<String, crate::ValidatorRegistrationStatus> {
        entries
            .into_iter()
            .map(|entry| (entry.identity.evm_address.to_ascii_lowercase(), entry))
            .collect()
    }

    #[test]
    fn only_members_with_a_verified_key_for_their_exact_identity_are_enrolled() {
        let mut set = common::ValidatorSet::new((1..=5).map(member).collect());
        set.validators[4].pq_pubkey = Some(pq(0x55));
        let mut stale = registration(&set.validators[2], Some(pq(0x33)));
        stale.identity.node_id = "another-node".into();
        let mut unregistered = registration(&set.validators[3], Some(pq(0x44)));
        unregistered.registered_with_node = false;
        let found = pending_pq_enrollments(
            &set,
            &registrations([
                registration(&set.validators[0], Some(pq(0x11))),
                registration(&set.validators[1], None),
                stale,
                unregistered,
                // Already enrolled: rotation is out of scope.
                registration(&set.validators[4], Some(pq(0x66))),
            ]),
        );
        assert_eq!(
            found,
            BTreeMap::from([("validator-1".to_owned(), pq(0x11))])
        );

        // A key claimed by two members, or already held by another member,
        // is never enrolled.
        let found = pending_pq_enrollments(
            &set,
            &registrations([
                registration(&set.validators[0], Some(pq(0x77))),
                registration(&set.validators[1], Some(pq(0x77))),
                registration(&set.validators[2], Some(pq(0x55))),
                registration(&set.validators[3], Some(pq(0x44))),
            ]),
        );
        assert_eq!(
            found,
            BTreeMap::from([("validator-4".to_owned(), pq(0x44))])
        );
    }

    #[test]
    fn an_enrollment_transition_only_adds_verified_keys() {
        let mut current = common::ValidatorSet::new((1..=4).map(member).collect());
        current.validators[3].pq_pubkey = Some(pq(0x44));
        let verified = BTreeMap::from([
            ("validator-1".to_owned(), pq(0x11)),
            ("validator-2".to_owned(), pq(0x22)),
        ]);
        let next = enrollment_next_set(&current, &verified);
        validate_pq_enrollment(&current, &next, &verified).unwrap();
        assert_ne!(
            common::canonical_validator_set_hash(&next),
            common::canonical_validator_set_hash(&current)
        );

        let rejects = |next: &common::ValidatorSet| {
            validate_pq_enrollment(&current, next, &verified).is_err()
        };
        assert!(rejects(&current), "nothing enrolled");
        let mut unverified = next.clone();
        unverified.validators[2].pq_pubkey = Some(pq(0x33));
        assert!(rejects(&unverified));
        let mut wrong_key = next.clone();
        wrong_key.validators[0].pq_pubkey = Some(pq(0x99));
        assert!(rejects(&wrong_key));
        let mut replaced = next.clone();
        replaced.validators[3].pq_pubkey = Some(pq(0x45));
        assert!(rejects(&replaced));
        let mut removed = next.clone();
        removed.validators[3].pq_pubkey = None;
        assert!(rejects(&removed));
        let mut restaked = next.clone();
        restaked.validators[1].stake += 1;
        assert!(rejects(&restaked));
        let mut shrunk = next.clone();
        shrunk.validators.pop();
        assert!(rejects(&shrunk));
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        state: SharedState,
        store: Arc<AttestationStore>,
        p2p: crate::p2p::P2PHandle,
        rx: tokio::sync::mpsc::Receiver<crate::p2p::P2PCommand>,
        bootstrap: common::ValidatorSet,
        tip_hash: String,
    }

    fn fixture(registrations: HashMap<String, crate::ValidatorRegistrationStatus>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let bootstrap = common::ValidatorSet::new((1..=4).map(member).collect());
        let chain = crate::chain_store::ChainStore::open(dir.path()).unwrap();
        let genesis_hash = chain.tip_hash().unwrap();
        let genesis_ts = chain
            .get_block_by_height(0)
            .unwrap()
            .unwrap()
            .header
            .timestamp;
        let block1 = common::Block {
            header: common::BlockHeader {
                height: 1,
                prev_hash: genesis_hash,
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
            validator_set_snapshot: Arc::new(crate::state::ArcSwap::from_pointee(
                bootstrap.clone(),
            )),
            validator_set: bootstrap.clone(),
            package_rounds: HashMap::new(),
            package_committees: HashMap::new(),
            config: crate::config::NodeConfig {
                node_id: "validator-1".into(),
                data_dir: dir.path().to_path_buf(),
                is_validator: true,
                validator_privkey: Some(hex::encode([1u8; 32]).into()),
                ..Default::default()
            },
            p2p_status: crate::P2PStatus::default(),
            bridge_status: crate::BridgeStatus::default(),
            vrf_proofs: HashMap::new(),
            decryption_shares: HashMap::new(),
            validator_registrations: registrations,
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
        }));
        let (sender, rx) = tokio::sync::mpsc::channel(32);
        Fixture {
            _dir: dir,
            state,
            store: AttestationStore::new(),
            p2p: crate::p2p::P2PHandle { sender },
            rx,
            bootstrap,
            tip_hash: block1.hash(),
        }
    }

    fn approval(
        next: &common::ValidatorSet,
        previous: &common::ValidatorSet,
        tip_hash: &str,
        signer: u8,
    ) -> common::ValidatorSetTransitionSignature {
        let unsigned = common::AuthenticatedValidatorSetSnapshot {
            schema_version: common::VALIDATOR_SNAPSHOT_SCHEMA_VERSION,
            effective_from_height: 2,
            previous_block_hash: tip_hash.to_owned(),
            previous_validator_set_hash: common::canonical_validator_set_hash(previous),
            validators: next.clone(),
            validator_set_hash: common::canonical_validator_set_hash(next),
            transition_signatures: Vec::new(),
        };
        let message = common::validator_set_transition_message(&unsigned);
        common::ValidatorSetTransitionSignature {
            validator_id: format!("validator-{signer}"),
            validator_pubkey: key(signer).public_key().to_hex(),
            signature: key(signer).sign(message.as_bytes()).to_hex(),
            pq_signature: None,
        }
    }

    fn broadcast_topics(
        rx: &mut tokio::sync::mpsc::Receiver<crate::p2p::P2PCommand>,
    ) -> Vec<String> {
        let mut topics = Vec::new();
        while let Ok(command) = rx.try_recv() {
            if let crate::p2p::P2PCommand::Broadcast { topic, .. } = command {
                topics.push(topic);
            }
        }
        topics
    }

    #[tokio::test]
    async fn quorum_of_approvals_installs_the_key_enrollment() {
        let members: Vec<_> = (1..=4).map(member).collect();
        let mut f = fixture(registrations([
            registration(&members[1], Some(pq(0x22))),
            registration(&members[2], Some(pq(0x33))),
        ]));
        let expected = BTreeMap::from([
            ("validator-2".to_owned(), pq(0x22)),
            ("validator-3".to_owned(), pq(0x33)),
        ]);
        let next = enrollment_next_set(&f.bootstrap, &expected);

        // A quiet period that has not elapsed signs nothing.
        tick_with_quiet_period(&f.state, &f.store, &f.p2p, 60_000)
            .await
            .unwrap();
        assert!(broadcast_topics(&mut f.rx).is_empty());

        // This node signs and gossips its approval; one of three is not quorum.
        tick_with_quiet_period(&f.state, &f.store, &f.p2p, 0)
            .await
            .unwrap();
        assert_eq!(
            broadcast_topics(&mut f.rx),
            vec![TRANSITION_TOPIC.to_owned()]
        );
        let key = TransitionProposalKey {
            applicant: format!("0x{:040x}", Address::repeat_byte(2)),
            effective_from_height: 2,
            previous_block_hash: f.tip_hash.clone(),
            validator_set_hash: common::canonical_validator_set_hash(&next),
        };
        assert_eq!(f.store.snapshot_transition_approvals(&key).await.len(), 1);

        // Approvals from two more members complete the 3-of-4 quorum.
        for signer in [2, 3] {
            f.store
                .insert_transition_approval(
                    key.clone(),
                    approval(&next, &f.bootstrap, &f.tip_hash, signer),
                )
                .await;
        }
        tick_with_quiet_period(&f.state, &f.store, &f.p2p, 0)
            .await
            .unwrap();
        assert_eq!(
            broadcast_topics(&mut f.rx),
            vec![
                TRANSITION_TOPIC.to_owned(),
                crate::validator_registry_gossip::VALIDATOR_SET_TRANSITIONS_TOPIC.to_owned()
            ]
        );

        let data_dir = f.state.read().await.config.data_dir.clone();
        let certified =
            crate::validator_set_history::verification_set_at(&data_dir, 2, &f.bootstrap).unwrap();
        assert_eq!(
            common::canonical_validator_set_hash(&certified),
            common::canonical_validator_set_hash(&next)
        );
        let runtime = f.state.read().await.validator_set.clone();
        let runtime_keys: BTreeMap<_, _> = runtime
            .validators
            .iter()
            .filter_map(|v| v.pq_pubkey.clone().map(|pq| (v.id.clone(), pq)))
            .collect();
        assert_eq!(runtime_keys, expected);

        // Nothing is left to enrol afterwards.
        tick_with_quiet_period(&f.state, &f.store, &f.p2p, 0)
            .await
            .unwrap();
        assert!(broadcast_topics(&mut f.rx).is_empty());
    }

    #[tokio::test]
    async fn a_pending_admission_takes_precedence_over_key_enrollment() {
        let members: Vec<_> = (1..=4).map(member).collect();
        let mut f = fixture(registrations([registration(&members[1], Some(pq(0x22)))]));
        f.store.inner.lock().await.cached_l1.applicants.insert(
            format!("0x{:040x}", Address::repeat_byte(9)),
            super::super::CachedL1Applicant {
                staking_state_code: 2,
                ..Default::default()
            },
        );
        tick_with_quiet_period(&f.state, &f.store, &f.p2p, 0)
            .await
            .unwrap();
        assert!(broadcast_topics(&mut f.rx).is_empty());
    }

    #[tokio::test]
    async fn an_observer_or_non_member_signs_nothing() {
        let members: Vec<_> = (1..=4).map(member).collect();
        let mut f = fixture(registrations([registration(&members[1], Some(pq(0x22)))]));
        f.state.write().await.config.is_validator = false;
        tick_with_quiet_period(&f.state, &f.store, &f.p2p, 0)
            .await
            .unwrap();
        assert!(broadcast_topics(&mut f.rx).is_empty());

        let mut f = fixture(registrations([registration(&members[1], Some(pq(0x22)))]));
        f.state.write().await.config.validator_privkey = Some(hex::encode([9u8; 32]).into());
        tick_with_quiet_period(&f.state, &f.store, &f.p2p, 0)
            .await
            .unwrap();
        assert!(broadcast_topics(&mut f.rx).is_empty());
    }
}
