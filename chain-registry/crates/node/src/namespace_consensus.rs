//! Deterministic CR-07 namespace-ownership state transitions.
//!
//! Upstream npm/PyPI traffic is deliberately absent from this module. Adapter
//! workers verify it before forming a certificate; proposal, sync, insertion,
//! reorg, and rebuild paths verify only chain-pinned data and signatures.

use anyhow::{Context, Result};
use common::{
    CertifiedNamespaceAuthority, NamespaceAuthorityPolicySpec, NamespaceAuthorityPurpose,
    NamespaceLifecycleAction, PublisherAuthorizationV1, Transaction,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::chain_store::ChainStore;

#[derive(Debug, Clone, Default)]
pub struct NamespaceConsensusConfig {
    pub chain_id: String,
    pub genesis_hash: String,
    pub staking_contract: String,
    pub activation_height: Option<u64>,
    pub authority_policies: Vec<NamespaceAuthorityPolicySpec>,
    pub policy_source_verified: bool,
}

impl NamespaceConsensusConfig {
    pub fn from_node_config(config: &crate::config::NodeConfig) -> Self {
        Self {
            chain_id: config.chain_id.clone(),
            genesis_hash: config.network_genesis_hash.clone(),
            staking_contract: common::canonical_publisher_address(&config.staking_addr),
            activation_height: config.namespace_authority_activation_height,
            authority_policies: config.namespace_authority_policies.clone(),
            policy_source_verified: config.namespace_authority_policy_source_verified,
        }
    }

    pub fn is_active_at(&self, height: u64) -> bool {
        self.activation_height
            .is_some_and(|activation| height >= activation)
    }

    pub fn validate(&self) -> Result<()> {
        if self.activation_height.is_none() {
            return Ok(());
        }
        if !common::is_canonical_single_line(&self.chain_id)
            || self.genesis_hash.len() != 64
            || self.genesis_hash != self.genesis_hash.to_ascii_lowercase()
            || !self
                .genesis_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.staking_contract.len() != 42
            || self.authority_policies.is_empty()
            || !self.policy_source_verified
        {
            anyhow::bail!("authoritative namespace consensus configuration is incomplete");
        }
        for policy in &self.authority_policies {
            if !common::is_canonical_single_line(&policy.adapter_id)
                || !common::is_canonical_single_line(&policy.adapter_version)
                || !is_lower_sha256(&policy.verifier_sha256)
                || !is_lower_sha256(&policy.policy_sha256)
                || !is_lower_sha256(&policy.trust_root_sha256)
            {
                anyhow::bail!("namespace authority policy pin is not canonical");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamespaceControllerState {
    pub schema_version: u16,
    pub namespace: String,
    pub root_ed25519_pubkey: String,
    #[serde(default)]
    pub root_evm_address: String,
    pub authoritative: bool,
    pub sequence: u64,
    pub last_lifecycle_nonce: u64,
    #[serde(default)]
    pub previous_event_hash: String,
    #[serde(default)]
    pub authority_certificate_digest: String,
    pub updated_at_height: u64,
}

impl NamespaceControllerState {
    pub fn legacy(namespace: &str, controller: &str) -> Self {
        Self {
            schema_version: 1,
            namespace: namespace.to_string(),
            root_ed25519_pubkey: controller.trim().to_ascii_lowercase(),
            root_evm_address: String::new(),
            authoritative: false,
            sequence: 0,
            last_lifecycle_nonce: 0,
            previous_event_hash: String::new(),
            authority_certificate_digest: String::new(),
            updated_at_height: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NamespaceDelegationState {
    pub schema_version: u16,
    pub namespace: String,
    pub root_ed25519_pubkey: String,
    pub delegate_ed25519_pubkey: String,
    pub delegate_evm_address: String,
    pub scopes: Vec<String>,
    pub expires_at: i64,
    pub lifecycle_event_hash: String,
    pub updated_at_height: u64,
}

#[derive(Debug, Clone, Default)]
pub struct NamespaceBlockEffects {
    pub controllers: BTreeMap<String, NamespaceControllerState>,
    pub delegations: BTreeMap<String, NamespaceDelegationState>,
    pub publisher_nonce_keys: BTreeSet<String>,
    pub lifecycle_nonce_keys: BTreeSet<String>,
}

pub fn publisher_authorization_nonce_key(address: &str, nonce: &str) -> String {
    format!(
        "{}\0{}",
        common::canonical_publisher_address(address),
        nonce.trim()
    )
}

pub fn lifecycle_nonce_key(namespace: &str, current_controller: &str, nonce: u64) -> String {
    format!(
        "{}\0{}\0{:020}",
        namespace,
        current_controller.trim().to_ascii_lowercase(),
        nonce
    )
}

pub fn delegation_key(namespace: &str, delegate: &str) -> String {
    format!("{}\0{}", namespace, delegate.trim().to_ascii_lowercase())
}

/// Validate every CR-07 transition in a block before any canonical or derived
/// RocksDB write is staged. Returned effects are the only namespace state that
/// insertion should persist.
pub fn validate_block(
    chain: &ChainStore,
    block: &common::Block,
    config: &NamespaceConsensusConfig,
) -> Result<NamespaceBlockEffects> {
    config.validate()?;
    let active = config.is_active_at(block.header.height);
    if active && block.header.timestamp.timestamp_subsec_nanos() != 0 {
        anyhow::bail!("post-activation block timestamp must use whole seconds");
    }
    let reference_time = block.header.timestamp.timestamp();
    let mut effects = NamespaceBlockEffects::default();
    let mut loaded_controllers = BTreeMap::<String, Option<NamespaceControllerState>>::new();
    let mut publisher_nonces = HashSet::new();
    let mut lifecycle_nonces = HashSet::new();

    for (index, transaction) in block.transactions.iter().enumerate() {
        let outcome = match transaction {
            Transaction::Publish(_) if active => Err(anyhow::anyhow!(
                "legacy publish transaction after activation"
            )),
            Transaction::NamespaceLifecycle(_) if active => Err(anyhow::anyhow!(
                "legacy namespace lifecycle transaction after activation"
            )),
            Transaction::RotatePublisherKey { .. } if active => Err(anyhow::anyhow!(
                "legacy publisher-key rotation transaction after activation"
            )),
            Transaction::PublishAuthorizedV1 {
                record,
                authorization,
            } => validate_authorized_publish(
                chain,
                block,
                config,
                record,
                authorization,
                reference_time,
                &mut loaded_controllers,
                &mut publisher_nonces,
                &mut effects,
            ),
            Transaction::NamespaceLifecycleV2(event) => validate_lifecycle_v2(
                chain,
                block,
                config,
                transaction,
                event,
                reference_time,
                &mut loaded_controllers,
                &mut lifecycle_nonces,
                &mut effects,
            ),
            _ => Ok(()),
        };
        outcome.with_context(|| format!("invalid CR-07 transaction at block index {}", index))?;
    }
    Ok(effects)
}

#[allow(clippy::too_many_arguments)]
fn validate_authorized_publish(
    chain: &ChainStore,
    block: &common::Block,
    config: &NamespaceConsensusConfig,
    record: &common::ChainRecord,
    authorization: &PublisherAuthorizationV1,
    reference_time: i64,
    controllers: &mut BTreeMap<String, Option<NamespaceControllerState>>,
    block_nonces: &mut HashSet<String>,
    effects: &mut NamespaceBlockEffects,
) -> Result<()> {
    authorization
        .validate_record_binding(record)
        .map_err(|error| anyhow::anyhow!(error))?;
    if authorization.chain_id != config.chain_id
        || authorization.genesis_hash != config.genesis_hash
        || authorization.staking_contract != config.staking_contract
    {
        anyhow::bail!("publisher authorization targets another network");
    }
    if authorization.issued_at > reference_time || reference_time >= authorization.expires_at as i64
    {
        anyhow::bail!("publisher authorization is not live at block time");
    }
    verify_publisher_signatures(record, authorization)?;

    let nonce_key =
        publisher_authorization_nonce_key(&authorization.publisher_address, &authorization.nonce);
    if !block_nonces.insert(nonce_key.clone())
        || chain.is_publisher_authorization_nonce_applied(&nonce_key)?
    {
        anyhow::bail!("publisher authorization nonce has already been applied");
    }

    let namespace = common::authoritative_package_namespace(&record.id)
        .map_err(|error| anyhow::anyhow!(error))?;
    let current = controller_for(chain, controllers, &namespace)?;
    match current {
        None => {
            let certificate = authorization
                .authority_certificate
                .as_ref()
                .ok_or_else(|| {
                    anyhow::anyhow!("first namespace claim lacks authority certificate")
                })?;
            validate_authority_certificate(
                chain,
                block.header.height,
                config,
                certificate,
                NamespaceAuthorityPurpose::Claim,
                reference_time,
            )?;
            let state = NamespaceControllerState {
                schema_version: 1,
                namespace: namespace.clone(),
                root_ed25519_pubkey: authorization.publisher_ed25519_pubkey.clone(),
                root_evm_address: authorization.publisher_address.clone(),
                authoritative: true,
                sequence: 0,
                last_lifecycle_nonce: 0,
                previous_event_hash: String::new(),
                authority_certificate_digest: certificate.digest(),
                updated_at_height: block.header.height,
            };
            controllers.insert(namespace.clone(), Some(state.clone()));
            effects.controllers.insert(namespace, state);
        }
        Some(state) if state.root_ed25519_pubkey == authorization.publisher_ed25519_pubkey => {
            if authorization.authority_certificate.is_some() {
                anyhow::bail!("established controller publish must not replay a claim certificate");
            }
        }
        Some(state) => {
            if authorization.authority_certificate.is_some() {
                anyhow::bail!("delegated publish must not carry a first-claim certificate");
            }
            let key = delegation_key(&namespace, &authorization.publisher_ed25519_pubkey);
            let delegation = effects
                .delegations
                .get(&key)
                .cloned()
                .or(chain.namespace_delegation(&key)?);
            let Some(delegation) = delegation else {
                anyhow::bail!("publisher is not the namespace controller or an active delegate");
            };
            if delegation.root_ed25519_pubkey != state.root_ed25519_pubkey
                || delegation.delegate_evm_address != authorization.publisher_address
                || !delegation.scopes.iter().any(|scope| scope == "publish")
                || reference_time >= delegation.expires_at
            {
                anyhow::bail!(
                    "namespace delegation is expired or no longer rooted in the controller"
                );
            }
        }
    }
    effects.publisher_nonce_keys.insert(nonce_key);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_lifecycle_v2(
    chain: &ChainStore,
    block: &common::Block,
    config: &NamespaceConsensusConfig,
    transaction: &Transaction,
    event: &common::NamespaceLifecycleEventV2,
    reference_time: i64,
    controllers: &mut BTreeMap<String, Option<NamespaceControllerState>>,
    block_nonces: &mut HashSet<String>,
    effects: &mut NamespaceBlockEffects,
) -> Result<()> {
    event
        .verify_controller_signatures()
        .map_err(|error| anyhow::anyhow!(error))?;
    if event.chain_id != config.chain_id || event.genesis_hash != config.genesis_hash {
        anyhow::bail!("namespace lifecycle event targets another network");
    }
    if !event.is_valid_at(reference_time) {
        anyhow::bail!("namespace lifecycle event is not live at block time");
    }
    let Some(mut state) = controller_for(chain, controllers, &event.namespace)? else {
        anyhow::bail!("namespace lifecycle event targets an unclaimed namespace");
    };
    if state.root_ed25519_pubkey != event.current_controller_ed25519_pubkey
        || state.root_evm_address != event.current_controller_evm_address
    {
        anyhow::bail!("namespace lifecycle current controller conflicts with chain state");
    }
    if event.nonce <= state.last_lifecycle_nonce {
        anyhow::bail!("namespace lifecycle nonce is stale");
    }
    let nonce_key = lifecycle_nonce_key(
        &event.namespace,
        &event.current_controller_ed25519_pubkey,
        event.nonce,
    );
    if !block_nonces.insert(nonce_key.clone())
        || chain.is_namespace_lifecycle_nonce_applied(&nonce_key)?
    {
        anyhow::bail!("namespace lifecycle nonce has already been applied");
    }

    let event_hash = common::transaction_hash(transaction);
    state.sequence = state.sequence.saturating_add(1);
    state.last_lifecycle_nonce = event.nonce;
    state.previous_event_hash = event_hash.clone();
    state.updated_at_height = block.header.height;
    match event.action {
        NamespaceLifecycleAction::Delegate => {
            let delegation = NamespaceDelegationState {
                schema_version: 1,
                namespace: event.namespace.clone(),
                root_ed25519_pubkey: state.root_ed25519_pubkey.clone(),
                delegate_ed25519_pubkey: event.next_controller_ed25519_pubkey.clone(),
                delegate_evm_address: event.next_controller_evm_address.clone(),
                scopes: event.delegation_scopes.clone(),
                expires_at: event
                    .delegation_expires_at
                    .expect("validated delegation expiry"),
                lifecycle_event_hash: event_hash,
                updated_at_height: block.header.height,
            };
            effects.delegations.insert(
                delegation_key(&event.namespace, &event.next_controller_ed25519_pubkey),
                delegation,
            );
        }
        NamespaceLifecycleAction::Transfer => {
            state.root_ed25519_pubkey = event.next_controller_ed25519_pubkey.clone();
            state.root_evm_address = event.next_controller_evm_address.clone();
        }
        NamespaceLifecycleAction::Recover => {
            let certificate = event
                .authority_certificate
                .as_ref()
                .expect("validated recovery certificate");
            validate_authority_certificate(
                chain,
                block.header.height,
                config,
                certificate,
                NamespaceAuthorityPurpose::Recover,
                reference_time,
            )?;
            state.root_ed25519_pubkey = event.next_controller_ed25519_pubkey.clone();
            state.root_evm_address = event.next_controller_evm_address.clone();
            state.authoritative = true;
            state.authority_certificate_digest = certificate.digest();
        }
    }
    controllers.insert(event.namespace.clone(), Some(state.clone()));
    effects.controllers.insert(event.namespace.clone(), state);
    effects.lifecycle_nonce_keys.insert(nonce_key);
    Ok(())
}

fn controller_for(
    chain: &ChainStore,
    overlay: &mut BTreeMap<String, Option<NamespaceControllerState>>,
    namespace: &str,
) -> Result<Option<NamespaceControllerState>> {
    if let Some(value) = overlay.get(namespace) {
        return Ok(value.clone());
    }
    let value = chain.namespace_controller_state(namespace)?;
    overlay.insert(namespace.to_string(), value.clone());
    Ok(value)
}

pub(crate) fn validate_authority_certificate(
    chain: &ChainStore,
    target_height: u64,
    config: &NamespaceConsensusConfig,
    certificate: &CertifiedNamespaceAuthority,
    purpose: NamespaceAuthorityPurpose,
    reference_time: i64,
) -> Result<()> {
    let statement = &certificate.statement;
    if statement.purpose != purpose
        || statement.chain_id != config.chain_id
        || statement.genesis_hash != config.genesis_hash
        || statement.issued_at > reference_time
        || reference_time >= statement.expires_at
        || statement.anchor_height >= target_height
        || target_height.saturating_sub(statement.anchor_height)
            > common::MAX_NAMESPACE_AUTHORITY_ANCHOR_LAG_BLOCKS
    {
        anyhow::bail!("namespace authority certificate context is invalid");
    }
    let policy_allowed = config.authority_policies.iter().any(|policy| {
        policy.authority == statement.authority_proof.authority
            && policy.adapter_id == statement.adapter_id
            && policy.adapter_version == statement.adapter_version
            && policy.verifier_sha256 == statement.verifier_sha256
            && policy.policy_sha256 == statement.policy_sha256
            && policy.trust_root_sha256 == statement.trust_root_sha256
    });
    if !policy_allowed {
        anyhow::bail!("namespace authority certificate uses an unpinned adapter policy");
    }
    let anchor = chain
        .get_block_by_height(statement.anchor_height)?
        .ok_or_else(|| anyhow::anyhow!("namespace authority certificate anchor is missing"))?;
    if anchor.hash() != statement.anchor_block_hash
        || anchor.header.validator_set_hash != statement.validator_set_hash
    {
        anyhow::bail!("namespace authority certificate anchor is not canonical");
    }
    certificate
        .validate_quorum_against_hash(&anchor.header.validator_set_hash)
        .map_err(|error| anyhow::anyhow!(error))?;
    Ok(())
}

fn verify_publisher_signatures(
    record: &common::ChainRecord,
    authorization: &PublisherAuthorizationV1,
) -> Result<()> {
    let message = authorization.signing_message(&record.id, &record.content_hash);
    let verify = |pubkey: &str, signature: &str| -> bool {
        let Ok(pubkey) = hex::decode(pubkey) else {
            return false;
        };
        let Ok(signature) = hex::decode(signature) else {
            return false;
        };
        let Ok(pubkey) = VerifyingKey::try_from(pubkey.as_slice()) else {
            return false;
        };
        let Ok(signature) = Signature::try_from(signature.as_slice()) else {
            return false;
        };
        pubkey.verify(message.as_bytes(), &signature).is_ok()
    };

    if authorization.publisher_pubkeys.is_empty() {
        if !verify(
            &authorization.publisher_ed25519_pubkey,
            &authorization.publisher_ed25519_signature,
        ) {
            anyhow::bail!("publisher Ed25519 signature verification failed");
        }
    } else {
        let mut valid = 0usize;
        let mut primary_signed = false;
        for (pubkey, signature) in authorization
            .publisher_pubkeys
            .iter()
            .zip(&authorization.signatures)
        {
            if !signature.is_empty() && verify(pubkey, signature) {
                valid += 1;
                primary_signed |= pubkey == &authorization.publisher_ed25519_pubkey;
            }
        }
        if valid < authorization.threshold || !primary_signed {
            anyhow::bail!("publisher multisig threshold or controller signature is missing");
        }
    }
    let recovered = crate::package_admission::recover_evm_personal_signer(
        &message,
        &authorization.publisher_evm_signature,
    )?;
    if recovered != authorization.publisher_address {
        anyhow::bail!("publisher EVM signature does not match its retained address");
    }
    Ok(())
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value == value.to_ascii_lowercase()
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
