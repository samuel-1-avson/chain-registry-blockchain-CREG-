//! Deterministic CR-07 namespace-ownership state transitions.
//!
//! Upstream npm/PyPI traffic is deliberately absent from this module. Adapter
//! workers verify it before forming a certificate; proposal, sync, insertion,
//! reorg, and rebuild paths verify only chain-pinned data and signatures.

use anyhow::{Context, Result};
use common::crypto::{PublicKey, Signature, SignatureScheme, VerifyMode};
use common::{
    CertifiedNamespaceAuthority, NamespaceAuthorityPolicySpec, NamespaceAuthorityPurpose,
    NamespaceLifecycleAction, PublisherAuthorizationV1, Transaction,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::chain_store::ChainStore;

#[derive(Debug, Clone, Default)]
pub struct NamespaceConsensusConfig {
    pub chain_id: String,
    pub genesis_hash: String,
    pub staking_contract: String,
    pub activation_height: Option<u64>,
    pub authority_policies: Vec<NamespaceAuthorityPolicySpec>,
    pub policy_source_verified: bool,
    pub validator_set_bootstrap: Option<common::ValidatorSet>,
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
            validator_set_bootstrap: Some(config.validator_set.clone()),
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
    let mut in_block_published = HashMap::<String, String>::new();
    let mut publisher_nonces = HashSet::new();
    let mut lifecycle_nonces = HashSet::new();

    for (index, transaction) in block.transactions.iter().enumerate() {
        let outcome = match transaction {
            Transaction::Publish(record) if active => {
                let _ = record;
                Err(anyhow::anyhow!(
                    "legacy publish transaction after activation"
                ))
            }
            Transaction::Publish(record) => {
                in_block_published.insert(record.id.canonical(), record.publisher_pubkey.clone());
                Ok(())
            }
            Transaction::NamespaceLifecycle(_) if active => Err(anyhow::anyhow!(
                "legacy namespace lifecycle transaction after activation"
            )),
            Transaction::RotatePublisherKey { .. } if active => Err(anyhow::anyhow!(
                "legacy publisher-key rotation transaction after activation"
            )),
            Transaction::Revoke { .. } if active => Err(anyhow::anyhow!(
                "legacy revocation transaction after activation"
            )),
            Transaction::PublishAuthorizedV1 {
                record,
                authorization,
            } => {
                let result = validate_authorized_publish(
                    chain,
                    block,
                    config,
                    record,
                    authorization,
                    reference_time,
                    &mut loaded_controllers,
                    &mut publisher_nonces,
                    &mut effects,
                );
                if result.is_ok() {
                    in_block_published
                        .insert(record.id.canonical(), record.publisher_pubkey.clone());
                }
                result
            }
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
            Transaction::RevokeV2 { request, .. } => validate_revoke_v2(
                chain,
                block,
                config,
                active,
                request,
                &in_block_published,
                &loaded_controllers,
                &effects,
            ),
            _ => Ok(()),
        };
        outcome.with_context(|| format!("invalid CR-07 transaction at block index {}", index))?;
    }
    Ok(effects)
}

#[allow(clippy::too_many_arguments)]
fn validate_revoke_v2(
    chain: &ChainStore,
    block: &common::Block,
    config: &NamespaceConsensusConfig,
    active: bool,
    request: &common::SignedRevocationRequest,
    in_block_published: &HashMap<String, String>,
    loaded_controllers: &BTreeMap<String, Option<NamespaceControllerState>>,
    effects: &NamespaceBlockEffects,
) -> Result<()> {
    request.verify_signature()?;
    if active && request.signature_version != 1 {
        anyhow::bail!("legacy revocation signature version is retired after activation");
    }
    if request.signature_version == 1 {
        if !config.chain_id.is_empty() && request.chain_id != config.chain_id {
            anyhow::bail!("revocation targets another chain_id");
        }
        if !config.genesis_hash.is_empty() && request.genesis_hash != config.genesis_hash {
            anyhow::bail!("revocation targets another genesis_hash");
        }
    }

    if let Some(owner_pubkey) = in_block_published.get(&request.package_canonical) {
        if owner_pubkey.eq_ignore_ascii_case(&request.revoker_pubkey) {
            return Ok(());
        }
    } else if let Some(record) = chain.get_package(&request.package_canonical)? {
        if record
            .publisher_pubkey
            .eq_ignore_ascii_case(&request.revoker_pubkey)
        {
            return Ok(());
        }
    }

    if let Some((prefix, version)) = request.package_canonical.rsplit_once('@') {
        let legacy_ns = format!("{prefix}@");
        let auth_ns = prefix
            .split_once(':')
            .and_then(|(eco, name)| {
                let pkg = common::PackageId::new(eco, name, version);
                common::authoritative_package_namespace(&pkg).ok()
            })
            .unwrap_or_else(|| legacy_ns.clone());
        for ns in [&auth_ns, &legacy_ns] {
            if let Some(state) = effects.controllers.get(ns.as_str()) {
                if state
                    .root_ed25519_pubkey
                    .eq_ignore_ascii_case(&request.revoker_pubkey)
                {
                    return Ok(());
                }
            } else if let Some(Some(state)) = loaded_controllers.get(ns.as_str()) {
                if state
                    .root_ed25519_pubkey
                    .eq_ignore_ascii_case(&request.revoker_pubkey)
                {
                    return Ok(());
                }
            } else if let Some(state) = chain.namespace_controller_state(ns)? {
                if state
                    .root_ed25519_pubkey
                    .eq_ignore_ascii_case(&request.revoker_pubkey)
                {
                    return Ok(());
                }
            }
        }
        if let Some(legacy_owner) = chain.publisher_for_prefix(&legacy_ns) {
            if legacy_owner.eq_ignore_ascii_case(&request.revoker_pubkey) {
                return Ok(());
            }
        }
    }

    if let Some(bootstrap) = config.validator_set_bootstrap.as_ref() {
        let authority = crate::validator_set_history::verification_set_at(
            chain.data_dir(),
            block.header.height,
            bootstrap,
        )
        .map_err(anyhow::Error::msg)?;
        if authority.validators.iter().any(|v| {
            common::validator_status_is_active(&v.status)
                && v.pubkey.eq_ignore_ascii_case(&request.revoker_pubkey)
        }) {
            return Ok(());
        }
    }

    anyhow::bail!("revocation signer is not an authorized owner, controller, or active validator")
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
            // Before CR-07 activation, a publisher may establish a namespace
            // without upstream authority. Keep the signed v4 authorization in
            // the block and mark that controller as non-authoritative. After
            // activation, a first claim needs the quorum certificate.
            let certificate_digest = if config.is_active_at(block.header.height) {
                let certificate =
                    authorization
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
                certificate.digest()
            } else {
                if authorization.authority_certificate.is_some() {
                    anyhow::bail!("pre-activation claim must not assert upstream authority");
                }
                String::new()
            };
            let state = NamespaceControllerState {
                schema_version: 1,
                namespace: namespace.clone(),
                root_ed25519_pubkey: authorization.publisher_ed25519_pubkey.clone(),
                root_evm_address: authorization.publisher_address.clone(),
                authoritative: config.is_active_at(block.header.height),
                sequence: 0,
                last_lifecycle_nonce: 0,
                previous_event_hash: String::new(),
                authority_certificate_digest: certificate_digest,
                updated_at_height: block.header.height,
            };
            controllers.insert(namespace.clone(), Some(state.clone()));
            effects.controllers.insert(namespace, state);
        }
        Some(state) if state.root_ed25519_pubkey == authorization.publisher_ed25519_pubkey => {
            if config.is_active_at(block.header.height) && !state.authoritative {
                anyhow::bail!("pre-activation controller needs certified authority migration");
            }
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
        || reference_time < statement.authority_proof.issued_at.timestamp()
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
        let Ok(pubkey) = PublicKey::from_bytes(SignatureScheme::Ed25519, &pubkey) else {
            return false;
        };
        let Ok(signature) = Signature::from_bytes(SignatureScheme::Ed25519, &signature) else {
            return false;
        };
        pubkey
            .verify(message.as_bytes(), &signature, VerifyMode::Strict)
            .is_ok()
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    #[test]
    fn authority_certificate_cannot_be_consumed_before_its_proof_existed() {
        let proof_issued_at = 1_700_000_100;
        let genesis = common::Block::genesis();
        let certificate = CertifiedNamespaceAuthority {
            statement: common::NamespaceAuthorityStatement {
                schema_version: common::NAMESPACE_AUTHORITY_CERTIFICATE_SCHEMA_VERSION,
                chain_id: "creg-testnet-1".into(),
                genesis_hash: "1".repeat(64),
                purpose: NamespaceAuthorityPurpose::Claim,
                namespace: "npm:owned-package@".into(),
                current_controller_ed25519_pubkey: String::new(),
                current_controller_evm_address: String::new(),
                next_controller_ed25519_pubkey: "a".repeat(64),
                next_controller_evm_address: "0x1111111111111111111111111111111111111111".into(),
                authority_nonce: 1,
                authority_intent_digest: "b".repeat(64),
                authority_proof: common::NamespaceAuthorityProof {
                    schema_version: common::NAMESPACE_AUTHORITY_PROOF_SCHEMA_VERSION,
                    authority: common::OwnershipAuthority::NpmTrustedPublisher,
                    namespace: "npm:owned-package@".into(),
                    issuer: "https://token.actions.githubusercontent.com".into(),
                    subject: "repo:creg/owned-package:ref:refs/tags/v1.0.0".into(),
                    audience: "npm:registry.npmjs.org".into(),
                    evidence_uri: format!("creg-evidence:sha256:{}", "c".repeat(64)),
                    registry_evidence_uri:
                        "https://registry.npmjs.org/-/npm/v1/attestations/owned-package@1.0.0"
                            .into(),
                    evidence_sha256: "c".repeat(64),
                    challenge_sha256: "b".repeat(64),
                    artifact_digest_algorithm: "sha512".into(),
                    artifact_digest: "d".repeat(128),
                    controller_evm_address: "0x1111111111111111111111111111111111111111".into(),
                    controller_ed25519_pubkey: "a".repeat(64),
                    issued_at: DateTime::from_timestamp(proof_issued_at, 0)
                        .expect("valid proof issuance"),
                    expires_at: DateTime::from_timestamp(1_700_000_900, 0)
                        .expect("valid proof expiry"),
                },
                adapter_id: "creg-npm-authority".into(),
                adapter_version: "1.0.0".into(),
                verifier_sha256: "e".repeat(64),
                policy_sha256: "f".repeat(64),
                trust_root_sha256: "0".repeat(64),
                anchor_height: 0,
                anchor_block_hash: genesis.hash(),
                validator_set_hash: genesis.header.validator_set_hash,
                issued_at: 1_700_000_000,
                expires_at: 1_700_000_800,
            },
            validators: common::ValidatorSet::default(),
            signatures: Vec::new(),
        };
        assert!(
            certificate.statement.validate_structure().is_ok(),
            "the adversarial statement must be structurally valid"
        );
        let config = NamespaceConsensusConfig {
            chain_id: certificate.statement.chain_id.clone(),
            genesis_hash: certificate.statement.genesis_hash.clone(),
            authority_policies: vec![NamespaceAuthorityPolicySpec {
                authority: common::OwnershipAuthority::NpmTrustedPublisher,
                adapter_id: certificate.statement.adapter_id.clone(),
                adapter_version: certificate.statement.adapter_version.clone(),
                verifier_sha256: certificate.statement.verifier_sha256.clone(),
                policy_sha256: certificate.statement.policy_sha256.clone(),
                trust_root_sha256: certificate.statement.trust_root_sha256.clone(),
            }],
            ..NamespaceConsensusConfig::default()
        };
        let directory = tempfile::tempdir().expect("temporary chain");
        let chain = ChainStore::open(directory.path()).expect("open chain");

        let error = validate_authority_certificate(
            &chain,
            1,
            &config,
            &certificate,
            NamespaceAuthorityPurpose::Claim,
            proof_issued_at - 1,
        )
        .expect_err("proof must already exist at the consuming block's timestamp");
        assert_eq!(
            error.to_string(),
            "namespace authority certificate context is invalid"
        );
    }

    #[test]
    fn revoke_v2_allows_owner_and_active_validator_and_rejects_forged_unauthorized_or_legacy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let chain = ChainStore::open(dir.path()).expect("open chain");
        let genesis = chain
            .get_block_by_height(0)
            .expect("get genesis")
            .expect("genesis");

        let owner_key = common::crypto::SigningKey::from_ed25519_seed(&[11u8; 32]);
        let owner_pubkey = owner_key.public_key().to_hex();
        let validator_key = common::crypto::SigningKey::from_ed25519_seed(&[22u8; 32]);
        let validator_pubkey = validator_key.public_key().to_hex();
        let outsider_key = common::crypto::SigningKey::from_ed25519_seed(&[33u8; 32]);
        let outsider_pubkey = outsider_key.public_key().to_hex();

        let pkg = common::PackageId::new("npm", "revoke-v2-pkg", "1.0.0");
        let record = common::ChainRecord {
            id: pkg.clone(),
            content_hash: "a".repeat(64),
            ipfs_cid: "bafytestrevokepkg".into(),
            publisher_pubkey: owner_pubkey.clone(),
            block_hash: String::new(),
            published_at: DateTime::from_timestamp(1_700_000_005, 0).expect("ts"),
            status: common::PackageStatus::Verified,
            ..Default::default()
        };
        let b1_txs = vec![Transaction::Publish(record)];
        let b1 = common::Block {
            header: common::BlockHeader {
                height: 1,
                prev_hash: genesis.hash(),
                merkle_root: common::merkle_root(&b1_txs),
                proposer_id: "validator-1".into(),
                timestamp: genesis.header.timestamp + chrono::Duration::seconds(10),
                validator_set_hash: genesis.header.validator_set_hash.clone(),
                vrf_output: None,
                vrf_proof: None,
                view_number: 0,
                view_change_certificate: Vec::new(),
            },
            transactions: b1_txs,
            pbft_signatures: Vec::new(),
        };
        chain.insert_block(&b1).expect("insert block 1");

        let config = NamespaceConsensusConfig {
            chain_id: "creg-testnet-1".into(),
            genesis_hash: genesis.hash(),
            staking_contract: "0x1111111111111111111111111111111111111111".into(),
            activation_height: Some(2),
            authority_policies: vec![NamespaceAuthorityPolicySpec {
                authority: common::OwnershipAuthority::NpmTrustedPublisher,
                adapter_id: "creg-npm-authority".into(),
                adapter_version: "1.0.0".into(),
                verifier_sha256: "e".repeat(64),
                policy_sha256: "f".repeat(64),
                trust_root_sha256: "0".repeat(64),
            }],
            policy_source_verified: true,
            validator_set_bootstrap: Some(common::ValidatorSet::new(vec![common::Validator {
                id: "validator-1".into(),
                pubkey: validator_pubkey.clone(),
                status: "active".into(),
                stake: 100,
                ..Default::default()
            }])),
        };

        let make_block2 = |tx: Transaction| -> common::Block {
            let txs = vec![tx];
            common::Block {
                header: common::BlockHeader {
                    height: 2,
                    prev_hash: b1.hash(),
                    merkle_root: common::merkle_root(&txs),
                    proposer_id: "validator-1".into(),
                    timestamp: genesis.header.timestamp + chrono::Duration::seconds(20),
                    validator_set_hash: genesis.header.validator_set_hash.clone(),
                    vrf_output: None,
                    vrf_proof: None,
                    view_number: 0,
                    view_change_certificate: Vec::new(),
                },
                transactions: txs,
                pbft_signatures: Vec::new(),
            }
        };

        let sign_revoke = |key: &common::crypto::SigningKey, pubkey: &str, reason: &str| {
            let mut req = common::SignedRevocationRequest {
                chain_id: config.chain_id.clone(),
                genesis_hash: config.genesis_hash.clone(),
                package_canonical: pkg.canonical(),
                reason: reason.into(),
                revoker_pubkey: pubkey.to_string(),
                signature: String::new(),
                signature_version: 1,
            };
            req.signature = key.sign(req.signing_message().as_bytes()).to_hex();
            req
        };

        // 1. Package owner allowed
        let owner_req = sign_revoke(&owner_key, &owner_pubkey, "owner yanked release");
        validate_block(
            &chain,
            &make_block2(Transaction::RevokeV2 {
                request: owner_req.clone(),
                evidence_hash: "1".repeat(64),
                witness: None,
            }),
            &config,
        )
        .expect("package owner RevokeV2 should validate");

        // 2. Active validator allowed
        let val_req = sign_revoke(&validator_key, &validator_pubkey, "quorum malware finding");
        validate_block(
            &chain,
            &make_block2(Transaction::RevokeV2 {
                request: val_req,
                evidence_hash: "2".repeat(64),
                witness: None,
            }),
            &config,
        )
        .expect("active validator RevokeV2 should validate");

        // 3. Forged / tampered reason rejected
        let mut forged_req = owner_req;
        forged_req.reason = "tampered reason".into();
        assert!(validate_block(
            &chain,
            &make_block2(Transaction::RevokeV2 {
                request: forged_req,
                evidence_hash: "3".repeat(64),
                witness: None,
            }),
            &config,
        )
        .is_err());

        // 4. Unauthorized signer rejected
        let outsider_req = sign_revoke(&outsider_key, &outsider_pubkey, "unauthorized");
        assert!(validate_block(
            &chain,
            &make_block2(Transaction::RevokeV2 {
                request: outsider_req,
                evidence_hash: "4".repeat(64),
                witness: None,
            }),
            &config,
        )
        .is_err());

        // 5. Legacy Transaction::Revoke rejected post-activation
        let legacy_err = validate_block(
            &chain,
            &make_block2(Transaction::Revoke {
                package_canonical: pkg.canonical(),
                reason: "legacy".into(),
                revoked_by: "validator-1".into(),
                evidence_hash: "5".repeat(64),
            }),
            &config,
        )
        .expect_err("legacy Revoke must be rejected post-activation");
        assert!(
            format!("{legacy_err:#}").contains("legacy revocation transaction after activation")
        );
    }
}
