use alloy::{
    primitives::{keccak256, Address},
    providers::ProviderBuilder,
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
    let seed = common::sha256_hex(format!("hybrid:{canonical}:{content_hash}:{randomness}").as_bytes());
    let mut scored: Vec<(u128, u64, u32, u32, String, String)> = validator_set.validators.iter().map(|v| {
        let id = v.id.clone();
        let eth = v.eth_address.clone();
        let stake = v.stake;
        let rep = v.reputation;
        let perf = v.performance_score;  // higher = better demonstrated resources/reliability
        let score_hex = common::sha256_hex(format!("{}:{}", hex::encode(&seed), id).as_bytes());
        let score = u128::from_str_radix(&score_hex[0..32], 16).unwrap_or(u128::MAX);
        let stake_w = if stake > 0 { score / (stake as u128) } else { u128::MAX };
        // Reputation feedback: higher rep -> smaller effective weight (earlier in list)
        let rep_w = if rep > 0 { stake_w / (rep as u128) } else { stake_w };
        // Performance/resources proxy: higher perf -> smaller weight -> earlier selection
        // This biases the mandatory committee toward nodes that have proven they can
        // successfully run the expensive verification pipeline.
        let weighted = if perf > 0 { rep_w / (perf as u128) } else { rep_w };
        (weighted, stake, rep, perf, id, eth)
    }).collect();

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
    for (_w, _stake, _rep, _perf, id, eth) in &scored {
        let count = eth_count.entry(eth.clone()).or_insert(0);
        if *count < MAX_PER_ETH {
            members.push(id.clone());
            *count += 1;
        }
    }

    // If the control-domain cap excluded too many validators, append the
    // remaining identities in the same deterministic order.  This is bounded
    // only by the validator-set size and gives expansion a real candidate set.
    for (_w, _stake, _rep, _perf, id, _eth) in &scored {
        if !members.contains(id) {
            members.push(id.clone());
        }
    }

    let target_m = if total_validators <= 50 {
        total_validators as u32
    } else {
        100u32.min(members.len() as u32)
    };
    common::AnalysisCommittee {
        size: target_m,
        members,
        seed: hex::encode(seed),
        effective_size: target_m,
        performance_bias: true,
        ..Default::default()
    }
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
        if let Some(v) = validator_set.validators.iter_mut().find(|vv| vv.id == sig.validator_id) {
            let is_mand = sig.is_mandatory && committee.members.iter().any(|m| m == &sig.validator_id);
            let was_correct = package_was_accepted == matches!(sig.vote, common::ValidatorVote::Approve);

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
            let new_avg = ((current as i64 * contrib as i64) + (delta as i64 * 10)) / (contrib as i64 + 1);

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
    if options.verify_publisher_auth {
        verify_publish_auth(state, &request).await?;
    }

    let canonical = request.id.canonical();
    let namespace_prefix = format!("{}:{}@", request.id.ecosystem, request.id.name);
    let ipfs_url = {
        let state = state.read().await;
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
        let chain_owner = state.chain.publisher_for_prefix(&namespace_prefix);
        let pending_owner = state.pending_pool.publisher_for_prefix(&namespace_prefix);
        let owner = chain_owner.or(pending_owner);
        if let Some(owner) = owner {
            let requested = request.publisher_pubkey.trim();
            if !requested.is_empty() && owner != requested {
                return Err(AdmissionError::NamespaceOwned(format!(
                    "{} is controlled by another publisher; use the signed namespace transfer or key-rotation flow",
                    namespace_prefix.trim_end_matches('@')
                )));
            }
        }
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
        if !state.pending_pool.insert(request.clone()) {
            return Err(AdmissionError::AlreadyPending(format!(
                "{} is already pending with the same content hash",
                canonical
            )));
        }

        // === WIRE REAL COMMITTEE COMPUTATION AT ADMISSION TIME (hybrid model) ===
        // Compute early so all nodes get the same mandatory committee via gossip.
        let tip = state.chain.tip_hash().unwrap_or_default();
        // Try to get recent VRF from tip block for VRF-seeded selection
        let recent_vrf = state.chain.get_block_by_hash(&tip)
            .ok()
            .flatten()
            .and_then(|b| b.header.vrf_output.clone());
        let committee = compute_analysis_committee_at_admission(
            &state.validator_set,
            &canonical,
            &request.content_hash,
            tip,
            recent_vrf,
        );
        state.pending_pool.set_analysis_committee(&canonical, committee);
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

pub async fn verify_publish_auth(
    state: &SharedState,
    request: &PublishRequest,
) -> Result<(), AdmissionError> {
    validate_package_id(request)?;
    verify_publish_sig(request)
        .map_err(|error| AdmissionError::InvalidPublisherSignature(error.to_string()))?;

    let require_dual = std::env::var("CREG_REQUIRE_PUBLISHER_DUAL_PROOF")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or_else(|_| {
            std::env::var("CREG_PRODUCTION")
                .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
        });

    if request.publisher_evm_signature.trim().is_empty() {
        if require_dual {
            return Err(AdmissionError::InvalidPublisherSignature(
                "Publisher EVM signature is required; legacy Ed25519-only admission is disabled"
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
        .map_err(AdmissionError::Publisher)
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
    Ok(())
}

pub(crate) fn verify_publish_sig(req: &PublishRequest) -> anyhow::Result<()> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    if req.publisher_address.trim().is_empty() {
        anyhow::bail!("Publisher EVM address is required for runtime admission");
    }

    let msg = publisher_signature_message(req)?;

    // Single-signature fallback.
    if req.publisher_pubkeys.is_empty() {
        let pubkey_bytes = hex::decode(&req.publisher_pubkey)?;
        let sig_bytes = hex::decode(&req.signature)?;
        let vk = VerifyingKey::try_from(pubkey_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("Invalid Ed25519 public key"))?;
        let sig = Signature::try_from(sig_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("Invalid Ed25519 signature"))?;
        return vk
            .verify(msg.as_bytes(), &sig)
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
    let mut seen_pubkeys = std::collections::HashSet::new();
    for (pubkey_hex, sig_hex) in req.publisher_pubkeys.iter().zip(req.signatures.iter()) {
        let normalized_pubkey = pubkey_hex.trim().to_ascii_lowercase();
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
        let vk = match VerifyingKey::try_from(pk_bytes.as_slice()) {
            Ok(k) => k,
            Err(_) => continue,
        };
        let sig = match Signature::try_from(sig_bytes.as_slice()) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if vk.verify(msg.as_bytes(), &sig).is_ok() {
            valid += 1;
        }
    }

    if valid >= threshold {
        Ok(())
    } else {
        anyhow::bail!(
            "Multi-sig verification failed: only {}/{} valid signatures (need {})",
            valid,
            req.publisher_pubkeys.len(),
            threshold
        )
    }
}

fn publisher_signature_message(req: &PublishRequest) -> anyhow::Result<String> {
    if !req.publisher_nonce.trim().is_empty()
        && req.publisher_expires_at > 0
        && !req.manifest_hash.trim().is_empty()
        && !req.publisher_chain_id.trim().is_empty()
        && !req.publisher_staking_contract.trim().is_empty()
    {
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
    let (chain_id, staking_contract) = {
        let state = state.read().await;
        (state.config.chain_id.clone(), state.config.staking_addr.clone())
    };
    if chain_id.trim().is_empty() || staking_contract.trim().is_empty() {
        anyhow::bail!(
            "Publisher dual-key proof cannot be verified without CREG_CHAIN_ID and CREG_STAKING_ADDR"
        );
    }
    if request.publisher_chain_id != chain_id {
        anyhow::bail!("Publisher proof chain ID does not match this node");
    }
    if common::canonical_publisher_address(&request.publisher_staking_contract)
        != common::canonical_publisher_address(&staking_contract)
    {
        anyhow::bail!("Publisher proof staking contract does not match this node");
    }
    if request.publisher_nonce.trim().is_empty() {
        anyhow::bail!("Publisher proof nonce is required");
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

fn recover_evm_personal_signer(message: &str, signature_hex: &str) -> anyhow::Result<String> {
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
    let signature = K256Signature::try_from(&bytes[..64])
        .context("Invalid EVM signature r/s values")?;
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.as_bytes().len());
    let mut payload = prefix.into_bytes();
    payload.extend_from_slice(message.as_bytes());
    let digest = keccak256(payload);
    let verifying_key = VerifyingKey::recover_from_prehash(
        digest.as_slice(),
        &signature,
        recovery_id,
    )
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

    let provider = ProviderBuilder::new().on_http(rpc_url.parse().map_err(|_| {
        PublisherAdmissionError::Unavailable(
            "Publisher stake enforcement unavailable: CREG_ETH_RPC is invalid".into(),
        )
    })?);
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
            treasury_balance: 0,
            pending_rewards_estimate: 0,
            total_rewards_distributed: 0,
            rewards_pool_balance: 0,
            last_incentives_refresh: None,
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
        assert!(committee.is_some(), "committee should be computed at admission time");
        assert_eq!(committee.unwrap().size, 1, "for 1 validator test, committee size should be 1");
        Ok(())
    }

    #[test]
    fn committee_selection_uses_vrf_seed_when_provided() {
        let mut vs = common::ValidatorSet::default();
        vs.validators.push(common::Validator { id: "v1".into(), alias: "v1".into(), pubkey: "p1".into(), eth_address: "0x1".into(), stake: 100, reputation: 50, status: "active".into(), performance_score: 500, contribution_count: 0 });
        vs.validators.push(common::Validator { id: "v2".into(), alias: "v2".into(), pubkey: "p2".into(), eth_address: "0x2".into(), stake: 100, reputation: 50, status: "active".into(), performance_score: 500, contribution_count: 0 });
        let c1 = compute_analysis_committee_at_admission(&vs, "npm:t@1", "h1", "tip1".into(), None);
        let c2 = compute_analysis_committee_at_admission(&vs, "npm:t@1", "h1", "tip1".into(), Some("vrf123".into()));
        assert_ne!(c1.seed, c2.seed, "VRF seed should change the selection seed");
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
}
