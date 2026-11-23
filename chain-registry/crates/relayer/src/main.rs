#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod store;

use alloy::{
    network::{ReceiptResponse, TransactionBuilder},
    primitives::{keccak256, Address, Bytes, B256, U256},
    providers::Provider,
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol,
    sol_types::{SolCall, SolValue},
};
use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use chrono::{NaiveDate, Utc};
use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};
use l1_tx::{
    AlloyL1Rpc, LedgerTx, SignerActor, SignerConfig, SignerHandle, SubmitError, SubmitRequest,
    TxState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fs, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use store::{
    LegacyJournalExport, OutboxRow, PgRelayerStore, RelayerStore, RequestStatusRecord, Reservation,
    ReserveError,
};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tracing::{error, info, warn};
use uuid::Uuid;
use zeroize::Zeroizing;

const EIP712_DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const SPONSORED_STAKE_INTENT_TYPE: &str = "SponsoredStakeIntent(address owner,address tokenContract,address stakingContract,uint8 action,uint256 amount,uint256 permitNonce,uint256 permitDeadline,uint256 relayerNonce,uint256 expiresAt)";
/// EIP-2612 permit typehash struct string. Used to recover the permit signer
/// off-chain so the relayer does not spend gas on a transaction whose permit
/// would revert on-chain.
const PERMIT_TYPE: &str =
    "Permit(address owner,address spender,uint256 value,uint256 nonce,uint256 deadline)";
const TOKEN_NAME: &str = "Chain Registry Token";
const TOKEN_VERSION: &str = "1";

sol!(
    #[sol(rpc)]
    interface IERC20PermitRead {
        function nonces(address owner) external view returns (uint256);
    }
);

sol!(
    #[sol(rpc)]
    interface IStakingSponsored {
        function stakeAsPublisherWithPermit(address publisher, uint256 amount, uint256 deadline, uint8 v, bytes32 r, bytes32 s) external;
        function applyToBeValidatorWithPermit(address validator, uint256 amount, uint256 deadline, uint8 v, bytes32 r, bytes32 s) external;
    }
);

#[derive(Clone)]
struct RelayerConfig {
    port: u16,
    /// `RELAYER_RPC_URL` then `RELAYER_RPC_FALLBACKS`, validated and
    /// de-duplicated (`common::l1_rpc`). List self-hosted endpoints first.
    rpc_endpoints: Vec<String>,
    policy_path: String,
    relayer_address: Address,
    active_chain_id: u64,
    /// Directory checked for legacy (sled-era) relayer state to migrate.
    data_dir: String,
    /// Number of trusted reverse proxies in front of the relayer (ECA-29).
    /// `0` (default) uses the TCP peer address only. With `N > 0`, the client
    /// is the `N`th `X-Forwarded-For` entry counted from the **right**, i.e.
    /// the address the outermost trusted proxy saw. Set via
    /// `RELAYER_TRUSTED_PROXY_HOPS`; legacy `RELAYER_TRUST_PROXY=true` means 1.
    trusted_proxy_hops: usize,
    /// Allowed CORS origins (exact match). Required outside testnet (`ECA-32`).
    allowed_origins: Vec<String>,
    /// Single-writer signer tuning (`crates/l1-tx`).
    signer: SignerConfig,
}

impl RelayerConfig {
    /// Read configuration and the hot key. The key is returned separately in
    /// zeroizing memory and handed to the signer actor; it is never stored in
    /// the config (`ECA-32`).
    async fn from_env() -> anyhow::Result<(Self, Zeroizing<String>)> {
        let secrets = chain_registry_secrets::SecretsProvider::from_env()?;
        let key = secrets
            .signing_key_secret(chain_registry_secrets::HotKeyRole::Relayer)
            .await?;
        let key = Zeroizing::new(key.expose_secret().to_string());
        secrets.warn_hot_key_if_env(
            "relayer",
            chain_registry_secrets::HotKeyRole::Relayer,
            &key,
            common::is_testnet_env(),
        );
        let relayer_address = key
            .parse::<PrivateKeySigner>()
            .map_err(|_| anyhow::anyhow!("RELAYER hot key is not a valid secp256k1 key"))?
            .address();

        let rpc_endpoints = common::l1_rpc::parse_rpc_endpoints(
            &env_string("RELAYER_RPC_URL", "http://localhost:8545"),
            &env_string("RELAYER_RPC_FALLBACKS", ""),
            "RELAYER_RPC_URL",
            "RELAYER_RPC_FALLBACKS",
        )?;
        let active_chain_id = AlloyL1Rpc::from_urls(&rpc_endpoints)?
            .provider()
            .get_chain_id()
            .await
            .map_err(|_| {
                anyhow::anyhow!("configured relayer RPC endpoints did not agree on a chain ID")
            })?;

        let allowed_origins: Vec<String> = env_string("RELAYER_ALLOWED_ORIGINS", "")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if allowed_origins.is_empty() && !common::is_testnet_env() {
            anyhow::bail!(
                "RELAYER_ALLOWED_ORIGINS must list the allowed browser origins outside testnet (ECA-32)"
            );
        }

        let defaults = SignerConfig::default();
        let gwei = 1_000_000_000u128;
        let signer = SignerConfig {
            confirmations: env_u64("RELAYER_CONFIRMATIONS", defaults.confirmations).max(1),
            fees: l1_tx::FeePolicy {
                max_fee_per_gas_cap: u128::from(env_u64(
                    "RELAYER_MAX_FEE_GWEI",
                    u64::try_from(defaults.fees.max_fee_per_gas_cap / gwei).unwrap_or(500),
                ))
                .saturating_mul(gwei),
                max_priority_fee_per_gas_cap: u128::from(env_u64(
                    "RELAYER_MAX_PRIORITY_FEE_GWEI",
                    u64::try_from(defaults.fees.max_priority_fee_per_gas_cap / gwei).unwrap_or(50),
                ))
                .saturating_mul(gwei),
                ..defaults.fees
            },
            ..defaults
        };

        Ok((
            Self {
                port: env_u16("RELAYER_PORT", 8083),
                rpc_endpoints,
                policy_path: env_string(
                    "RELAYER_POLICY_PATH",
                    "config/relayer-policy.example.json",
                ),
                relayer_address,
                active_chain_id,
                data_dir: env_string("RELAYER_DATA_DIR", "."),
                trusted_proxy_hops: std::env::var("RELAYER_TRUSTED_PROXY_HOPS")
                    .ok()
                    .and_then(|raw| raw.trim().parse::<usize>().ok())
                    .unwrap_or(usize::from(env_bool("RELAYER_TRUST_PROXY", false)))
                    .min(MAX_TRUSTED_PROXY_HOPS),
                allowed_origins,
                signer,
            },
            key,
        ))
    }
}

#[derive(Clone)]
struct AppState {
    config: RelayerConfig,
    policy: LoadedPolicy,
    /// Read-only L1 access (quorum for contract state, failover otherwise).
    l1: AlloyL1Rpc,
    store: Arc<dyn RelayerStore>,
    /// The only path that signs with the relayer key (`ECO-INV-07`). `None`
    /// only in tests that never reach a broadcast.
    signer: Option<SignerHandle>,
}

#[derive(Clone)]
struct LoadedPolicy {
    mode: String,
    signature: LoadedSignaturePolicy,
    replay_protection: LoadedReplayProtection,
    chain: LoadedChainPolicy,
}

#[derive(Clone)]
struct LoadedSignaturePolicy {
    scheme: String,
    domain_name: String,
    domain_version: String,
}

#[derive(Clone)]
struct LoadedReplayProtection {
    nonce_scope: String,
    max_expiry_seconds: u64,
}

#[derive(Clone)]
struct LoadedChainPolicy {
    id: u64,
    label: String,
    enabled: bool,
    daily_wallet_quota: u64,
    daily_ip_quota: u64,
    max_gas_per_request: u64,
    allow_contracts: Vec<Address>,
    actions: HashMap<SponsoredActionKind, LoadedActionPolicy>,
}

#[derive(Clone)]
struct LoadedActionPolicy {
    key: SponsoredActionKind,
    name: String,
    selector: String,
    max_amount_wei: U256,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum SponsoredActionKind {
    Publisher,
    Validator,
}

impl SponsoredActionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Publisher => "publisher",
            Self::Validator => "validator",
        }
    }

    fn as_code(self) -> u8 {
        match self {
            Self::Publisher => 0,
            Self::Validator => 1,
        }
    }

    fn from_code(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Publisher),
            1 => Some(Self::Validator),
            _ => None,
        }
    }

    fn from_key(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "publisher" => Some(Self::Publisher),
            "validator" => Some(Self::Validator),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPolicyFile {
    mode: String,
    chains: Vec<RawChainPolicy>,
    signature: RawSignaturePolicy,
    replay_protection: RawReplayProtection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSignaturePolicy {
    scheme: String,
    domain_name: String,
    domain_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReplayProtection {
    nonce_scope: String,
    max_expiry_seconds: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawChainPolicy {
    id: u64,
    label: String,
    enabled: bool,
    daily_wallet_quota: u64,
    daily_ip_quota: u64,
    max_gas_per_request: u64,
    allow_contracts: Vec<String>,
    allow_selectors: Vec<RawSelectorPolicy>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSelectorPolicy {
    key: Option<String>,
    name: String,
    selector: String,
    max_amount_wei: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyResponse {
    available: bool,
    relayer_address: String,
    active_chain_id: u64,
    signature: PublicSignaturePolicy,
    chains: Vec<PublicChainPolicy>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicSignaturePolicy {
    scheme: String,
    domain_name: String,
    domain_version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicChainPolicy {
    id: u64,
    label: String,
    enabled: bool,
    daily_wallet_quota: u64,
    daily_ip_quota: u64,
    max_gas_per_request: String,
    actions: Vec<PublicActionPolicy>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicActionPolicy {
    key: String,
    name: String,
    selector: String,
    max_amount_wei: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuoteRequest {
    owner: String,
    chain_id: u64,
    action: String,
    amount_wei: String,
    token_contract: String,
    staking_contract: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QuoteResponse {
    allowed: bool,
    reason: Option<String>,
    action: String,
    relayer_address: String,
    estimated_gas: String,
    estimated_fee_wei: String,
    permit_domain: TypedDataDomain,
    permit_message: PermitMessage,
    intent_domain: TypedDataDomain,
    intent_message: SponsoredStakeIntentMessage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SponsorRequest {
    action: String,
    permit_message: PermitMessage,
    intent_message: SponsoredStakeIntentMessage,
    permit_signature: String,
    intent_signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SponsorResponse {
    success: bool,
    request_id: String,
    status: String,
    tx_hash: Option<String>,
    message: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TypedDataDomain {
    name: String,
    version: String,
    chain_id: u64,
    verifying_contract: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermitMessage {
    owner: String,
    spender: String,
    value: String,
    nonce: String,
    deadline: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SponsoredStakeIntentMessage {
    owner: String,
    token_contract: String,
    staking_contract: String,
    action: u8,
    amount: String,
    permit_nonce: String,
    permit_deadline: String,
    relayer_nonce: String,
    expires_at: String,
}

#[derive(Clone)]
struct ValidatedActionRequest {
    action: SponsoredActionKind,
    owner: Address,
    token_contract: Address,
    staking_contract: Address,
    amount: U256,
    permit_nonce: U256,
    permit_deadline: U256,
    relayer_nonce: U256,
    expires_at: U256,
}

#[derive(Clone)]
struct ParsedSignatureParts {
    v: u8,
    r: B256,
    s: B256,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let (config, key) = RelayerConfig::from_env().await?;
    let policy = load_policy(&config.policy_path, config.active_chain_id)?;

    info!("╔════════════════════════════════════════════════════════╗");
    info!("║      Chain Registry Sponsored Transaction Relayer     ║");
    info!("╚════════════════════════════════════════════════════════╝");
    info!("  Chain ID:        {}", config.active_chain_id);
    info!("  Relayer address: {}", config.relayer_address);
    info!("  Policy:          {}", config.policy_path);
    info!("  Policy mode:     {}", policy.mode);
    info!(
        "  Nonce scope:     {}",
        policy.replay_protection.nonce_scope
    );
    // Endpoint URLs can embed provider API keys; log only their count.
    info!("  RPC endpoints:   {}", config.rpc_endpoints.len());

    // Durable state is mandatory: without PostgreSQL the relayer refuses to
    // start rather than sponsoring with nonces it cannot remember (ECA-33).
    let database_url = std::env::var("RELAYER_DATABASE_URL").map_err(|_| {
        anyhow::anyhow!("RELAYER_DATABASE_URL is required (PostgreSQL durable relayer state)")
    })?;
    let (pool, transport) = l1_tx::pg::connect_pool(
        &database_url,
        "RELAYER_DATABASE_URL",
        env_bool("RELAYER_PG_ALLOW_PLAINTEXT_PRIVATE_NETWORK", false),
        8,
    )
    .await?;
    drop(database_url);
    info!("  PostgreSQL:      {:?}", transport);
    l1_tx::pg::ensure_schema(&pool).await?;
    let store: Arc<dyn RelayerStore> = Arc::new(PgRelayerStore::open(pool.clone()).await?);
    migrate_legacy_state(store.as_ref(), &config.data_dir).await?;

    let l1 = AlloyL1Rpc::from_urls(&config.rpc_endpoints)?;
    let (signer, signer_task) = SignerActor::spawn(
        key,
        config.active_chain_id,
        Arc::new(l1.clone()),
        Arc::new(l1_tx::pg::PgNonceLedger::new(pool)),
        config.signer.clone(),
    )
    .await?;
    if signer.address() != config.relayer_address {
        anyhow::bail!("signer actor address does not match the configured relayer key");
    }

    let state = Arc::new(AppState {
        config: config.clone(),
        policy,
        l1,
        store,
        signer: Some(signer),
    });
    tokio::spawn(run_outbox_reconciler(Arc::clone(&state)));
    tokio::spawn(async move {
        if signer_task.await.is_err() {
            error!("relayer signer actor panicked");
        }
        error!("relayer signer actor stopped; sponsorship is unavailable until restart");
    });

    info!("  Data dir:        {}", config.data_dir);
    info!("  Trusted proxy hops: {}", config.trusted_proxy_hops);

    let cors = build_cors(&config.allowed_origins);

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/v1/relayer/policy", get(get_policy))
        .route("/v1/relayer/quote", post(quote_request))
        .route("/v1/relayer/sponsor", post(sponsor_request))
        .route("/v1/relayer/status/:request_id", get(get_status))
        .layer(cors)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    info!("Relayer listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let signer_running = state.signer.as_ref().is_some_and(SignerHandle::is_running);
    let payload = serde_json::json!({
        "status": if signer_running { "healthy" } else { "degraded" },
        "signer": if signer_running { "running" } else { "stopped" },
        "chainId": state.config.active_chain_id,
        "relayerAddress": state.config.relayer_address.to_string(),
        "policyChain": state.policy.chain.label,
    });
    let status = if signer_running {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(payload))
}

async fn get_policy(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let chain = &state.policy.chain;
    let response = PolicyResponse {
        available: chain.enabled,
        relayer_address: state.config.relayer_address.to_string(),
        active_chain_id: state.config.active_chain_id,
        signature: PublicSignaturePolicy {
            scheme: state.policy.signature.scheme.clone(),
            domain_name: state.policy.signature.domain_name.clone(),
            domain_version: state.policy.signature.domain_version.clone(),
        },
        chains: vec![PublicChainPolicy {
            id: chain.id,
            label: chain.label.clone(),
            enabled: chain.enabled,
            daily_wallet_quota: chain.daily_wallet_quota,
            daily_ip_quota: chain.daily_ip_quota,
            max_gas_per_request: chain.max_gas_per_request.to_string(),
            actions: chain
                .actions
                .values()
                .map(|action| PublicActionPolicy {
                    key: action.key.as_str().to_string(),
                    name: action.name.clone(),
                    selector: action.selector.clone(),
                    max_amount_wei: action.max_amount_wei.to_string(),
                })
                .collect(),
        }],
    };

    (StatusCode::OK, Json(response))
}

async fn quote_request(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<QuoteRequest>,
) -> impl IntoResponse {
    let client_ip = extract_client_ip(&headers, peer_addr, state.config.trusted_proxy_hops);

    match build_quote(&state, &request, &client_ip).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(reason) => {
            let fallback = QuoteResponse {
                allowed: false,
                reason: Some(reason),
                action: request.action,
                relayer_address: state.config.relayer_address.to_string(),
                estimated_gas: "0".to_string(),
                estimated_fee_wei: "0".to_string(),
                permit_domain: TypedDataDomain {
                    name: TOKEN_NAME.to_string(),
                    version: TOKEN_VERSION.to_string(),
                    chain_id: request.chain_id,
                    verifying_contract: request.token_contract.clone(),
                },
                permit_message: PermitMessage {
                    owner: request.owner.clone(),
                    spender: request.staking_contract.clone(),
                    value: request.amount_wei.clone(),
                    nonce: "0".to_string(),
                    deadline: "0".to_string(),
                },
                intent_domain: TypedDataDomain {
                    name: state.policy.signature.domain_name.clone(),
                    version: state.policy.signature.domain_version.clone(),
                    chain_id: request.chain_id,
                    verifying_contract: state.config.relayer_address.to_string(),
                },
                intent_message: SponsoredStakeIntentMessage {
                    owner: request.owner,
                    token_contract: request.token_contract,
                    staking_contract: request.staking_contract,
                    action: 255,
                    amount: request.amount_wei,
                    permit_nonce: "0".to_string(),
                    permit_deadline: "0".to_string(),
                    relayer_nonce: "0".to_string(),
                    expires_at: "0".to_string(),
                },
            };
            (StatusCode::OK, Json(fallback)).into_response()
        }
    }
}

async fn sponsor_request(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<SponsorRequest>,
) -> impl IntoResponse {
    let client_ip = extract_client_ip(&headers, peer_addr, state.config.trusted_proxy_hops);

    let prepared = match validate_sponsor_request(&state, &request) {
        Ok(prepared) => prepared,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(SponsorResponse {
                    success: false,
                    request_id: String::new(),
                    status: "rejected".to_string(),
                    tx_hash: None,
                    message,
                }),
            )
                .into_response();
        }
    };

    for (quota_key, limit) in [
        (
            wallet_quota_key(prepared.owner, prepared.action),
            state.policy.chain.daily_wallet_quota,
        ),
        (
            ip_quota_key(state.config.active_chain_id, &client_ip),
            state.policy.chain.daily_ip_quota,
        ),
    ] {
        if let Err(message) = check_quota(state.store.as_ref(), &quota_key, limit).await {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(SponsorResponse {
                    success: false,
                    request_id: String::new(),
                    status: "rejected".to_string(),
                    tx_hash: None,
                    message,
                }),
            )
                .into_response();
        }
    }

    let owner_key = owner_storage_key(prepared.owner);
    let expected_nonce = match state
        .store
        .owner_nonce(state.config.active_chain_id, &owner_key)
        .await
    {
        Ok(value) => value,
        Err(message) => {
            error!("Durable relayer nonce read failed: {}", message);
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(SponsorResponse {
                    success: false,
                    request_id: String::new(),
                    status: "unavailable".to_string(),
                    tx_hash: None,
                    message: "Durable relayer state is unavailable.".to_string(),
                }),
            )
                .into_response();
        }
    };
    let provided_nonce = match parse_u64(&request.intent_message.relayer_nonce, "relayer nonce") {
        Ok(value) => value,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(SponsorResponse {
                    success: false,
                    request_id: String::new(),
                    status: "rejected".to_string(),
                    tx_hash: None,
                    message,
                }),
            )
                .into_response();
        }
    };

    if provided_nonce != expected_nonce {
        return (
            StatusCode::CONFLICT,
            Json(SponsorResponse {
                success: false,
                request_id: String::new(),
                status: "rejected".to_string(),
                tx_hash: None,
                message: format!(
                    "Relayer nonce mismatch. Expected {}, got {}.",
                    expected_nonce, provided_nonce
                ),
            }),
        )
            .into_response();
    }

    if let Err(message) = verify_intent_signature(&state, &request, &prepared) {
        return (
            StatusCode::BAD_REQUEST,
            Json(SponsorResponse {
                success: false,
                request_id: String::new(),
                status: "rejected".to_string(),
                tx_hash: None,
                message,
            }),
        )
            .into_response();
    }

    let on_chain_nonce =
        match fetch_token_nonce(&state, prepared.token_contract, prepared.owner).await {
            Ok(value) => value,
            Err(err) => {
                error!(
                "Failed to fetch token nonce for sponsor request. Token: {}, Owner: {}. Error: {}",
                prepared.token_contract, prepared.owner, err
            );
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(SponsorResponse {
                        success: false,
                        request_id: String::new(),
                        status: "failed".to_string(),
                        tx_hash: None,
                        message: format!("Relayer failed to reach blockchain node: {}", err),
                    }),
                )
                    .into_response();
            }
        };
    if on_chain_nonce != prepared.permit_nonce {
        return (
            StatusCode::CONFLICT,
            Json(SponsorResponse {
                success: false,
                request_id: String::new(),
                status: "rejected".to_string(),
                tx_hash: None,
                message: format!(
                    "Permit nonce mismatch. Current on-chain nonce is {}, requested {}.",
                    on_chain_nonce, prepared.permit_nonce
                ),
            }),
        )
            .into_response();
    }

    let permit_signature = match parse_signature_parts(&request.permit_signature) {
        Ok(parts) => parts,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(SponsorResponse {
                    success: false,
                    request_id: String::new(),
                    status: "rejected".to_string(),
                    tx_hash: None,
                    message,
                }),
            )
                .into_response();
        }
    };

    // Recover the ERC-2612 permit signer off-chain and confirm it is the owner
    // BEFORE spending relayer gas. Without this, a malformed/forged permit only
    // fails inside the on-chain call — after the relayer has already paid.
    if let Err(message) = verify_permit_signature(&state, &request, &prepared) {
        return (
            StatusCode::BAD_REQUEST,
            Json(SponsorResponse {
                success: false,
                request_id: String::new(),
                status: "rejected".to_string(),
                tx_hash: None,
                message,
            }),
        )
            .into_response();
    }

    let transaction =
        match prepare_sponsored_transaction(&state, &prepared, &permit_signature).await {
            Ok(transaction) => transaction,
            Err(message) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(SponsorResponse {
                        success: false,
                        request_id: String::new(),
                        status: "rejected".to_string(),
                        tx_hash: None,
                        message,
                    }),
                )
                    .into_response();
            }
        };

    let Some(signer) = state.signer.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(SponsorResponse {
                success: false,
                request_id: String::new(),
                status: "unavailable".to_string(),
                tx_hash: None,
                message: "The relayer signer is not running; no transaction was submitted."
                    .to_string(),
            }),
        )
            .into_response();
    };

    let request_id = Uuid::new_v4().to_string();
    let signer_request_id = signer_request_id(&request_id);
    let now = Utc::now().to_rfc3339();
    let initial_record = RequestStatusRecord {
        request_id: request_id.clone(),
        status: "submitting".to_string(),
        action: prepared.action.as_str().to_string(),
        owner: prepared.owner.to_string(),
        chain_id: state.config.active_chain_id,
        tx_hash: None,
        block_number: None,
        message: format!(
            "Submitting sponsored {} transaction.",
            prepared.action.as_str()
        ),
        created_at: now.clone(),
        updated_at: now,
    };
    let reservation = Reservation {
        chain_id: state.config.active_chain_id,
        owner: owner_key,
        expected_owner_nonce: expected_nonce,
        wallet_quota_key: wallet_quota_key(prepared.owner, prepared.action),
        wallet_quota_limit: state.policy.chain.daily_wallet_quota,
        ip_quota_key: ip_quota_key(state.config.active_chain_id, &client_ip),
        ip_quota_limit: state.policy.chain.daily_ip_quota,
        day: utc_today(),
        record: initial_record.clone(),
        signer_request_id: signer_request_id.clone(),
    };
    if let Err(reserve_error) = state.store.reserve(&reservation).await {
        let (status, detail) = match reserve_error {
            ReserveError::QuotaReached => (
                StatusCode::TOO_MANY_REQUESTS,
                "Daily sponsorship quota has been reached.".to_string(),
            ),
            ReserveError::NonceConflict(actual) => (
                StatusCode::CONFLICT,
                format!("Relayer nonce advanced to {actual} while this request was being verified; request a new quote."),
            ),
            ReserveError::NonceExhausted => (
                StatusCode::CONFLICT,
                "Relayer nonce is exhausted for this wallet.".to_string(),
            ),
            ReserveError::Storage(message) => {
                error!("Failed to atomically reserve sponsored request: {}", message);
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Durable relayer state is unavailable; no transaction was submitted."
                        .to_string(),
                )
            }
        };
        return (
            status,
            Json(SponsorResponse {
                success: false,
                request_id: String::new(),
                status: "rejected".to_string(),
                tx_hash: None,
                message: detail,
            }),
        )
            .into_response();
    }

    // The signer actor allocates the relayer EOA nonce, writes the signed
    // transaction to the l1-tx ledger before broadcasting it, and follows it
    // to a final receipt (ECA-28, ECA-30).
    let submitted = signer
        .submit(SubmitRequest {
            request_id: signer_request_id,
            to: transaction.to,
            value: U256::ZERO,
            input: transaction.input,
            gas_limit: transaction.gas_limit,
        })
        .await;

    let mut record = initial_record;
    let (http_status, response) = match submitted {
        Ok(ticket) => {
            let tx_hash = ticket.tx_hash.to_string();
            record.status = "submitted".to_string();
            record.tx_hash = Some(tx_hash.clone());
            record.message = match ticket.broadcast {
                l1_tx::BroadcastStatus::Accepted => format!(
                    "Sponsored {} transaction submitted.",
                    prepared.action.as_str()
                ),
                l1_tx::BroadcastStatus::Uncertain => format!(
                    "Sponsored {} transaction signed; its broadcast is being retried.",
                    prepared.action.as_str()
                ),
            };
            (
                StatusCode::ACCEPTED,
                SponsorResponse {
                    success: true,
                    request_id: request_id.clone(),
                    status: "submitted".to_string(),
                    tx_hash: Some(tx_hash),
                    message: record.message.clone(),
                },
            )
        }
        Err(submit_error) => {
            let (status_code, friendly) = match &submit_error {
                SubmitError::Rejected(detail) => classify_send_error(detail),
                SubmitError::FeeAboveCap { .. } | SubmitError::Busy(_) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("{submit_error}. Try again later."),
                ),
                SubmitError::InvalidRequest(_) | SubmitError::Unavailable(_) => {
                    error!(
                        "Relayer signer refused request {}: {}",
                        request_id, submit_error
                    );
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "The relayer signer is unavailable.".to_string(),
                    )
                }
            };
            // Nothing is pending on chain for this request.
            record.status = "failed".to_string();
            record.message = format!("No transaction was sent: {friendly}");
            (
                status_code,
                SponsorResponse {
                    success: false,
                    request_id: request_id.clone(),
                    status: "failed".to_string(),
                    tx_hash: None,
                    message: record.message.clone(),
                },
            )
        }
    };

    if let Err(storage_error) = state.store.update_request(&record).await {
        // The outbox reconciler repairs the row from the l1-tx ledger.
        error!(
            "Sponsor request {} outcome was not recorded yet: {}",
            request_id, storage_error
        );
    }
    (http_status, Json(response)).into_response()
}

async fn get_status(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> impl IntoResponse {
    match state.store.get_request(&request_id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(row.record)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Unknown relayer request id.",
                "requestId": request_id,
            })),
        )
            .into_response(),
        Err(error) => {
            error!("Failed to read relayer request {}: {}", request_id, error);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "Durable relayer status storage is unavailable.",
                    "requestId": request_id,
                })),
            )
                .into_response()
        }
    }
}

async fn build_quote(
    state: &AppState,
    request: &QuoteRequest,
    client_ip: &str,
) -> Result<QuoteResponse, String> {
    let action = SponsoredActionKind::from_key(&request.action)
        .ok_or_else(|| format!("Unsupported sponsored action: {}", request.action))?;
    let owner = parse_address(&request.owner, "owner")?;
    let token_contract = parse_address(&request.token_contract, "token contract")?;
    let staking_contract = parse_address(&request.staking_contract, "staking contract")?;
    let amount = parse_u256(&request.amount_wei, "amount")?;

    if request.chain_id != state.config.active_chain_id {
        return Err(format!(
            "This relayer is configured for chain {} and cannot sponsor chain {}.",
            state.config.active_chain_id, request.chain_id
        ));
    }

    let chain = &state.policy.chain;
    if !chain.enabled {
        return Err(format!(
            "Sponsored transactions are disabled for {}.",
            chain.label
        ));
    }
    ensure_contract_allowed(chain, token_contract, "token contract")?;
    ensure_contract_allowed(chain, staking_contract, "staking contract")?;

    let action_policy = chain.actions.get(&action).cloned().ok_or_else(|| {
        format!(
            "Policy does not allow sponsored {} actions.",
            action.as_str()
        )
    })?;

    if amount > action_policy.max_amount_wei {
        return Err(format!(
            "Requested amount exceeds the relayer policy cap for {}.",
            action.as_str()
        ));
    }

    check_quota(
        state.store.as_ref(),
        &wallet_quota_key(owner, action),
        chain.daily_wallet_quota,
    )
    .await?;
    check_quota(
        state.store.as_ref(),
        &ip_quota_key(state.config.active_chain_id, client_ip),
        chain.daily_ip_quota,
    )
    .await?;

    let permit_nonce = fetch_token_nonce(state, token_contract, owner).await?;
    let relayer_nonce = state
        .store
        .owner_nonce(state.config.active_chain_id, &owner_storage_key(owner))
        .await
        .map_err(|error| {
            error!("Durable relayer nonce read failed: {}", error);
            "Durable relayer state is unavailable.".to_string()
        })?;
    let permit_deadline = U256::from(
        (Utc::now().timestamp() as u64) + state.policy.replay_protection.max_expiry_seconds,
    );
    let expires_at = U256::from(
        (Utc::now().timestamp() as u64) + state.policy.replay_protection.max_expiry_seconds,
    );
    let gas_price = fetch_gas_price(state).await?;
    let estimated_fee = gas_price.saturating_mul(U256::from(chain.max_gas_per_request));

    Ok(QuoteResponse {
        allowed: true,
        reason: None,
        action: action.as_str().to_string(),
        relayer_address: state.config.relayer_address.to_string(),
        estimated_gas: chain.max_gas_per_request.to_string(),
        estimated_fee_wei: estimated_fee.to_string(),
        permit_domain: TypedDataDomain {
            name: TOKEN_NAME.to_string(),
            version: TOKEN_VERSION.to_string(),
            chain_id: state.config.active_chain_id,
            verifying_contract: token_contract.to_string(),
        },
        permit_message: PermitMessage {
            owner: owner.to_string(),
            spender: staking_contract.to_string(),
            value: amount.to_string(),
            nonce: permit_nonce.to_string(),
            deadline: permit_deadline.to_string(),
        },
        intent_domain: TypedDataDomain {
            name: state.policy.signature.domain_name.clone(),
            version: state.policy.signature.domain_version.clone(),
            chain_id: state.config.active_chain_id,
            verifying_contract: state.config.relayer_address.to_string(),
        },
        intent_message: SponsoredStakeIntentMessage {
            owner: owner.to_string(),
            token_contract: token_contract.to_string(),
            staking_contract: staking_contract.to_string(),
            action: action.as_code(),
            amount: amount.to_string(),
            permit_nonce: permit_nonce.to_string(),
            permit_deadline: permit_deadline.to_string(),
            relayer_nonce: relayer_nonce.to_string(),
            expires_at: expires_at.to_string(),
        },
    })
}

fn validate_sponsor_request(
    state: &AppState,
    request: &SponsorRequest,
) -> Result<ValidatedActionRequest, String> {
    if !state.policy.chain.enabled {
        return Err("Sponsored transactions are disabled by relayer policy.".to_string());
    }

    let action = SponsoredActionKind::from_key(&request.action)
        .ok_or_else(|| format!("Unsupported sponsored action: {}", request.action))?;
    let intent_action = SponsoredActionKind::from_code(request.intent_message.action)
        .ok_or_else(|| "Intent message uses an unsupported action code.".to_string())?;
    if intent_action != action {
        return Err("Action mismatch between sponsor request and typed intent.".to_string());
    }

    let owner = parse_address(&request.intent_message.owner, "intent owner")?;
    let permit_owner = parse_address(&request.permit_message.owner, "permit owner")?;
    if owner != permit_owner {
        return Err("Permit owner and intent owner do not match.".to_string());
    }

    let token_contract = parse_address(&request.intent_message.token_contract, "token contract")?;
    let staking_contract =
        parse_address(&request.intent_message.staking_contract, "staking contract")?;
    let permit_spender = parse_address(&request.permit_message.spender, "permit spender")?;

    if permit_spender != staking_contract {
        return Err("Permit spender must match the staking contract.".to_string());
    }

    let amount = parse_u256(&request.intent_message.amount, "amount")?;
    if amount != parse_u256(&request.permit_message.value, "permit value")? {
        return Err("Permit value must match the sponsored amount.".to_string());
    }

    let permit_nonce = parse_u256(&request.intent_message.permit_nonce, "permit nonce")?;
    if permit_nonce != parse_u256(&request.permit_message.nonce, "permit nonce")? {
        return Err("Permit nonce mismatch between permit and intent message.".to_string());
    }

    let permit_deadline = parse_u256(&request.intent_message.permit_deadline, "permit deadline")?;
    if permit_deadline != parse_u256(&request.permit_message.deadline, "permit deadline")? {
        return Err("Permit deadline mismatch between permit and intent message.".to_string());
    }

    let relayer_nonce = parse_u256(&request.intent_message.relayer_nonce, "relayer nonce")?;
    let expires_at = parse_u256(&request.intent_message.expires_at, "intent expiry")?;
    let now = U256::from(Utc::now().timestamp() as u64);
    if expires_at <= now {
        return Err("Sponsored intent has already expired.".to_string());
    }
    if permit_deadline <= now {
        return Err("Permit has already expired.".to_string());
    }

    let max_future = now + U256::from(state.policy.replay_protection.max_expiry_seconds);
    if expires_at > max_future || permit_deadline > max_future {
        return Err("Requested expiry exceeds relayer policy limits.".to_string());
    }

    ensure_contract_allowed(&state.policy.chain, token_contract, "token contract")?;
    ensure_contract_allowed(&state.policy.chain, staking_contract, "staking contract")?;

    let action_policy = state
        .policy
        .chain
        .actions
        .get(&action)
        .cloned()
        .ok_or_else(|| {
            format!(
                "Policy does not allow sponsored {} actions.",
                action.as_str()
            )
        })?;
    if amount > action_policy.max_amount_wei {
        return Err(format!(
            "Requested amount exceeds the relayer policy cap for {}.",
            action.as_str()
        ));
    }

    Ok(ValidatedActionRequest {
        action,
        owner,
        token_contract,
        staking_contract,
        amount,
        permit_nonce,
        permit_deadline,
        relayer_nonce,
        expires_at,
    })
}

fn verify_intent_signature(
    state: &AppState,
    request: &SponsorRequest,
    prepared: &ValidatedActionRequest,
) -> Result<(), String> {
    let digest = hash_sponsored_intent(
        &state.policy.signature.domain_name,
        &state.policy.signature.domain_version,
        state.config.active_chain_id,
        state.config.relayer_address,
        prepared,
    );

    let recovered = recover_address_from_signature(&digest, &request.intent_signature)?;
    if recovered != prepared.owner {
        return Err(format!(
            "Intent signature recovered {}, expected {}.",
            recovered, prepared.owner
        ));
    }

    Ok(())
}

/// EIP-712 digest of the ERC-2612 `Permit` typed-data over the token domain.
/// Mirrors `hash_sponsored_intent` but for the permit struct so the relayer
/// can recover the permit signer locally.
fn hash_permit(chain_id: u64, token_contract: Address, prepared: &ValidatedActionRequest) -> B256 {
    let domain_type_hash = keccak256(EIP712_DOMAIN_TYPE.as_bytes());
    let domain_separator = keccak256(
        (
            domain_type_hash,
            keccak256(TOKEN_NAME.as_bytes()),
            keccak256(TOKEN_VERSION.as_bytes()),
            U256::from(chain_id),
            token_contract,
        )
            .abi_encode(),
    );
    let permit_type_hash = keccak256(PERMIT_TYPE.as_bytes());
    let struct_hash = keccak256(
        (
            permit_type_hash,
            prepared.owner,
            prepared.staking_contract,
            prepared.amount,
            prepared.permit_nonce,
            prepared.permit_deadline,
        )
            .abi_encode(),
    );

    let mut bytes = Vec::with_capacity(66);
    bytes.extend_from_slice(&[0x19, 0x01]);
    bytes.extend_from_slice(domain_separator.as_slice());
    bytes.extend_from_slice(struct_hash.as_slice());
    keccak256(bytes)
}

/// Recover the ERC-2612 permit signer and confirm it equals the owner, so the
/// relayer never spends gas on a permit that would revert on-chain.
fn verify_permit_signature(
    state: &AppState,
    request: &SponsorRequest,
    prepared: &ValidatedActionRequest,
) -> Result<(), String> {
    let digest = hash_permit(
        state.config.active_chain_id,
        prepared.token_contract,
        prepared,
    );
    let recovered = recover_address_from_signature(&digest, &request.permit_signature)?;
    if recovered != prepared.owner {
        return Err(format!(
            "Permit signature recovered {}, expected owner {}.",
            recovered, prepared.owner
        ));
    }
    Ok(())
}

/// Build the CORS layer. With no configured origins we keep `Any` (dev) but
/// warn; with an allowlist we restrict `Access-Control-Allow-Origin` to it.
fn build_cors(allowed_origins: &[String]) -> CorsLayer {
    let base = CorsLayer::new().allow_methods(Any).allow_headers(Any);
    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|o| o.parse::<HeaderValue>().ok())
        .collect();
    if origins.is_empty() {
        info!("  CORS:            any origin (set RELAYER_ALLOWED_ORIGINS in production)");
        base.allow_origin(Any)
    } else {
        info!("  CORS:            {} allowed origin(s)", origins.len());
        base.allow_origin(AllowOrigin::list(origins))
    }
}

async fn check_quota(store: &dyn RelayerStore, key: &str, limit: u64) -> Result<(), String> {
    let count = store.quota_count(key, utc_today()).await.map_err(|error| {
        error!("Relayer quota read failed: {}", error);
        "Durable relayer state is unavailable.".to_string()
    })?;
    if count >= limit {
        return Err("Daily sponsorship quota has been reached.".to_string());
    }
    Ok(())
}

/// Meta keys recording one-time migrations from the sled-era relayer.
const META_LEGACY_SLED: &str = "legacy_sled_journal";
const META_LEGACY_NONCE_FILE: &str = "legacy_nonce_file";

/// Bring sled-era state into PostgreSQL exactly once (ADR-019 §5.3).
///
/// - `<data_dir>/sponsor-nonces.json` (the pre-sled nonce file) is imported
///   with the same conflict rule as before.
/// - A `<data_dir>/relayer-state.db` sled journal cannot be read by this
///   build. Export it with `creg-relayer-sled-export` and set
///   `RELAYER_LEGACY_JOURNAL_EXPORT=<file>`, or, after review, set
///   `RELAYER_LEGACY_SLED_DISCARD=true`. Until one of them is done the
///   relayer refuses to start, so remembered owner nonces are never reset.
async fn migrate_legacy_state(store: &dyn RelayerStore, data_dir: &str) -> anyhow::Result<()> {
    let directory = PathBuf::from(data_dir);

    if store.meta_get(META_LEGACY_NONCE_FILE).await?.is_none() {
        let legacy_path = directory.join("sponsor-nonces.json");
        if legacy_path.exists() {
            let bytes = fs::read(&legacy_path)?;
            let nonces: std::collections::BTreeMap<String, u64> = serde_json::from_slice(&bytes)?;
            let export = LegacyJournalExport {
                format: store::LEGACY_EXPORT_FORMAT.to_string(),
                nonces,
                ..LegacyJournalExport::default()
            };
            let summary = store.import_legacy(&export).await?;
            info!("  Imported {} legacy sponsor nonces", summary.nonces);
        }
        store.meta_set(META_LEGACY_NONCE_FILE, "done").await?;
    }

    if store.meta_get(META_LEGACY_SLED).await?.is_some() {
        return Ok(());
    }
    if let Ok(path) = std::env::var("RELAYER_LEGACY_JOURNAL_EXPORT") {
        let bytes = fs::read(&path).map_err(|error| {
            anyhow::anyhow!("cannot read RELAYER_LEGACY_JOURNAL_EXPORT: {error}")
        })?;
        let export: LegacyJournalExport = serde_json::from_slice(&bytes)?;
        let summary = store.import_legacy(&export).await?;
        store.meta_set(META_LEGACY_SLED, "imported").await?;
        info!(
            "  Imported legacy relayer journal: {} nonces, {} quota windows, {} requests",
            summary.nonces, summary.quotas, summary.requests
        );
        return Ok(());
    }
    if directory.join("relayer-state.db").exists() {
        if env_bool("RELAYER_LEGACY_SLED_DISCARD", false) {
            warn!("Discarding the legacy sled relayer journal by operator request");
            store.meta_set(META_LEGACY_SLED, "discarded").await?;
            return Ok(());
        }
        anyhow::bail!(
            "a legacy sled relayer journal exists in RELAYER_DATA_DIR; export it with \
             creg-relayer-sled-export and set RELAYER_LEGACY_JOURNAL_EXPORT, or set \
             RELAYER_LEGACY_SLED_DISCARD=true after review"
        );
    }
    store.meta_set(META_LEGACY_SLED, "none").await?;
    Ok(())
}

fn validate_sponsored_gas(estimated_gas: u64, maximum_gas: u64) -> Result<(), String> {
    if estimated_gas == 0 || maximum_gas == 0 {
        return Err("Sponsored transaction gas estimate or policy limit is invalid.".to_string());
    }
    if estimated_gas > maximum_gas {
        return Err("Sponsored transaction exceeds the relayer gas policy limit.".to_string());
    }
    Ok(())
}

/// A checked sponsored call, ready for the signer actor.
struct PreparedCall {
    to: Address,
    input: Bytes,
    gas_limit: u64,
}

async fn prepare_sponsored_transaction(
    state: &AppState,
    prepared: &ValidatedActionRequest,
    permit_signature: &ParsedSignatureParts,
) -> Result<PreparedCall, String> {
    let input: Bytes = match prepared.action {
        SponsoredActionKind::Publisher => IStakingSponsored::stakeAsPublisherWithPermitCall {
            publisher: prepared.owner,
            amount: prepared.amount,
            deadline: prepared.permit_deadline,
            v: permit_signature.v,
            r: permit_signature.r,
            s: permit_signature.s,
        }
        .abi_encode()
        .into(),
        SponsoredActionKind::Validator => IStakingSponsored::applyToBeValidatorWithPermitCall {
            validator: prepared.owner,
            amount: prepared.amount,
            deadline: prepared.permit_deadline,
            v: permit_signature.v,
            r: permit_signature.r,
            s: permit_signature.s,
        }
        .abi_encode()
        .into(),
    };
    let estimate_request = TransactionRequest::default()
        .with_from(state.config.relayer_address)
        .with_to(prepared.staking_contract)
        .with_input(input.clone());

    let estimated_gas = state
        .l1
        .provider()
        .estimate_gas(&estimate_request)
        .await
        .map_err(|_| {
            "Unable to estimate sponsored transaction gas; no transaction was submitted."
                .to_string()
        })?;
    validate_sponsored_gas(estimated_gas, state.policy.chain.max_gas_per_request)?;
    // Sign with the policy ceiling as the gas limit, never an uncapped
    // estimate: the checked calldata and sender stay exactly as estimated.
    Ok(PreparedCall {
        to: prepared.staking_contract,
        input,
        gas_limit: state.policy.chain.max_gas_per_request,
    })
}

/// How often the outbox follows open requests.
const OUTBOX_RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
/// A `submitting` row with no l1-tx ledger entry after this long was never
/// signed (the relayer stopped between reservation and signing).
const UNSIGNED_REQUEST_GRACE: Duration = Duration::from_secs(120);

/// Follow open outbox rows until their transaction is final (ECA-30): there
/// is no give-up timeout. Sent transactions are tracked through the l1-tx
/// ledger; legacy rows imported from the sled era are tracked by receipt.
async fn run_outbox_reconciler(state: Arc<AppState>) {
    let mut ticker = tokio::time::interval(OUTBOX_RECONCILE_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if let Err(error) = reconcile_outbox_once(&state).await {
            warn!("relayer outbox reconcile pass failed: {error:#}");
        }
    }
}

async fn reconcile_outbox_once(state: &AppState) -> anyhow::Result<()> {
    for row in state.store.open_requests().await? {
        let update = match (&row.signer_request_id, &state.signer) {
            (Some(signer_request_id), Some(signer)) => {
                let ledger_row = signer.status(signer_request_id).await?;
                outbox_update_from_ledger(&row, ledger_row.as_ref(), Utc::now())
            }
            (None, _) => legacy_receipt_update(state, &row).await?,
            (Some(_), None) => None,
        };
        if let Some(record) = update {
            state.store.update_request(&record).await?;
        }
    }
    Ok(())
}

/// New outbox record for a row given its l1-tx ledger entry, or `None` when
/// nothing changed.
fn outbox_update_from_ledger(
    row: &OutboxRow,
    ledger_row: Option<&LedgerTx>,
    now: chrono::DateTime<Utc>,
) -> Option<RequestStatusRecord> {
    let mut record = row.record.clone();
    match ledger_row {
        None => {
            let age = now.signed_duration_since(row.created_at).to_std().ok()?;
            if record.status != "submitting" || age < UNSIGNED_REQUEST_GRACE {
                return None;
            }
            record.status = "failed".to_string();
            record.message = "No transaction was sent: the relayer stopped before signing. \
                The relayer nonce for this wallet moved on; request a new quote."
                .to_string();
        }
        Some(ledger_row) => {
            record.tx_hash = ledger_row.current_tx_hash().map(|hash| hash.to_string());
            match ledger_row.state {
                TxState::Signed | TxState::Pending | TxState::Included => {
                    record.status = "submitted".to_string();
                    if row.record.status == "submitting" {
                        record.message =
                            format!("Sponsored {} transaction submitted.", record.action);
                    }
                }
                TxState::Confirmed => {
                    record.status = "confirmed".to_string();
                    record.block_number =
                        ledger_row.block_number.map(|block| format!("{block:#x}"));
                    record.message = "Sponsored transaction confirmed.".to_string();
                }
                TxState::Reverted => {
                    record.status = "failed".to_string();
                    record.block_number =
                        ledger_row.block_number.map(|block| format!("{block:#x}"));
                    record.message = "Sponsored transaction reverted on-chain.".to_string();
                }
                TxState::Rejected => {
                    record.status = "failed".to_string();
                    record.message = format!(
                        "No transaction was sent: {}",
                        ledger_row
                            .last_error
                            .as_deref()
                            .unwrap_or("refused by the L1 node")
                    );
                }
                TxState::Displaced => {
                    record.status = "failed".to_string();
                    record.message = "The relayer transaction was displaced by another transaction with the same nonce."
                        .to_string();
                }
            }
        }
    }
    let changed = record.status != row.record.status
        || record.tx_hash != row.record.tx_hash
        || record.block_number != row.record.block_number
        || record.message != row.record.message;
    changed.then_some(record)
}

/// Rows imported from the sled era carry only a transaction hash.
async fn legacy_receipt_update(
    state: &AppState,
    row: &OutboxRow,
) -> anyhow::Result<Option<RequestStatusRecord>> {
    let Some(tx_hash) = row.record.tx_hash.as_deref() else {
        return Ok(None);
    };
    let Ok(hash) = tx_hash.parse::<B256>() else {
        return Ok(None);
    };
    let Some(receipt) = state.l1.provider().get_transaction_receipt(hash).await? else {
        return Ok(None);
    };
    let Some(block) = receipt.block_number() else {
        return Ok(None);
    };
    let head = state.l1.provider().get_block_number().await?;
    if head.saturating_add(1).saturating_sub(block) < state.config.signer.confirmations {
        return Ok(None);
    }
    let mut record = row.record.clone();
    record.block_number = Some(format!("{block:#x}"));
    if receipt.status() {
        record.status = "confirmed".to_string();
        record.message = "Sponsored transaction confirmed.".to_string();
    } else {
        record.status = "failed".to_string();
        record.message = "Sponsored transaction reverted on-chain.".to_string();
    }
    Ok(Some(record))
}

fn load_policy(policy_path: &str, active_chain_id: u64) -> anyhow::Result<LoadedPolicy> {
    let raw_contents = fs::read_to_string(policy_path)?;
    let raw_json: Value = serde_json::from_str(&raw_contents)?;
    let substituted = substitute_env_value(raw_json);
    let raw_policy: RawPolicyFile = serde_json::from_value(substituted)?;

    let raw_chain = raw_policy
        .chains
        .into_iter()
        .find(|chain| chain.id == active_chain_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No relayer policy configured for active chain {}",
                active_chain_id
            )
        })?;

    let mut actions = HashMap::new();
    for selector in raw_chain.allow_selectors {
        let Some(key) = selector
            .key
            .as_deref()
            .and_then(SponsoredActionKind::from_key)
            .or_else(|| infer_action_kind(&selector.name))
        else {
            continue;
        };

        actions.insert(
            key,
            LoadedActionPolicy {
                key,
                name: selector.name,
                selector: selector.selector,
                max_amount_wei: selector.max_amount_wei.parse::<U256>()?,
            },
        );
    }

    let allow_contracts = raw_chain
        .allow_contracts
        .into_iter()
        .filter_map(|address| address.parse::<Address>().ok())
        .collect::<Vec<_>>();

    Ok(LoadedPolicy {
        mode: raw_policy.mode,
        signature: LoadedSignaturePolicy {
            scheme: raw_policy.signature.scheme,
            domain_name: raw_policy.signature.domain_name,
            domain_version: raw_policy.signature.domain_version,
        },
        replay_protection: LoadedReplayProtection {
            nonce_scope: raw_policy.replay_protection.nonce_scope,
            max_expiry_seconds: raw_policy.replay_protection.max_expiry_seconds,
        },
        chain: LoadedChainPolicy {
            id: raw_chain.id,
            label: raw_chain.label,
            enabled: raw_chain.enabled,
            daily_wallet_quota: raw_chain.daily_wallet_quota,
            daily_ip_quota: raw_chain.daily_ip_quota,
            max_gas_per_request: raw_chain.max_gas_per_request,
            allow_contracts,
            actions,
        },
    })
}

fn infer_action_kind(name: &str) -> Option<SponsoredActionKind> {
    let lowered = name.to_ascii_lowercase();
    if lowered.contains("stakeaspublisherwithpermit") {
        Some(SponsoredActionKind::Publisher)
    } else if lowered.contains("applytobevalidatorwithpermit") {
        Some(SponsoredActionKind::Validator)
    } else {
        None
    }
}

fn substitute_env_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(substitute_env_placeholders(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(substitute_env_value).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, substitute_env_value(value)))
                .collect(),
        ),
        other => other,
    }
}

fn substitute_env_placeholders(input: &str) -> String {
    let mut result = String::new();
    let bytes = input.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        if cursor + 1 < bytes.len() && bytes[cursor] == b'$' && bytes[cursor + 1] == b'{' {
            if let Some(end) = input[cursor + 2..].find('}') {
                let name = &input[cursor + 2..cursor + 2 + end];
                let replacement = std::env::var(name).unwrap_or_else(|_| format!("${{{}}}", name));
                result.push_str(&replacement);
                cursor += end + 3;
                continue;
            }
        }
        result.push(bytes[cursor] as char);
        cursor += 1;
    }

    result
}

fn ensure_contract_allowed(
    chain: &LoadedChainPolicy,
    contract: Address,
    label: &str,
) -> Result<(), String> {
    if !chain.allow_contracts.contains(&contract) {
        return Err(format!(
            "{} is not allowlisted by the relayer policy.",
            label
        ));
    }
    Ok(())
}

fn hash_sponsored_intent(
    domain_name: &str,
    domain_version: &str,
    chain_id: u64,
    relayer_address: Address,
    request: &ValidatedActionRequest,
) -> B256 {
    let domain_type_hash = keccak256(EIP712_DOMAIN_TYPE.as_bytes());
    let domain_separator = keccak256(
        (
            domain_type_hash,
            keccak256(domain_name.as_bytes()),
            keccak256(domain_version.as_bytes()),
            U256::from(chain_id),
            relayer_address,
        )
            .abi_encode(),
    );
    let intent_type_hash = keccak256(SPONSORED_STAKE_INTENT_TYPE.as_bytes());
    let struct_hash = keccak256(
        (
            intent_type_hash,
            request.owner,
            request.token_contract,
            request.staking_contract,
            U256::from(request.action.as_code()),
            request.amount,
            request.permit_nonce,
            request.permit_deadline,
            request.relayer_nonce,
            request.expires_at,
        )
            .abi_encode(),
    );

    let mut bytes = Vec::with_capacity(66);
    bytes.extend_from_slice(&[0x19, 0x01]);
    bytes.extend_from_slice(domain_separator.as_slice());
    bytes.extend_from_slice(struct_hash.as_slice());
    keccak256(bytes)
}

fn recover_address_from_signature(digest: &B256, signature_hex: &str) -> Result<Address, String> {
    let bytes = parse_signature_bytes(signature_hex)?;
    let recovery_id = normalize_recovery_id(bytes[64])?;
    let signature = K256Signature::try_from(&bytes[..64])
        .map_err(|e| format!("Invalid ECDSA signature: {}", e))?;
    let verifying_key =
        VerifyingKey::recover_from_prehash(digest.as_slice(), &signature, recovery_id)
            .map_err(|e| format!("Failed to recover signer: {}", e))?;
    let uncompressed = verifying_key.to_encoded_point(false);
    let pubkey = uncompressed.as_bytes();
    let hashed = keccak256(&pubkey[1..]);
    Ok(Address::from_slice(&hashed.as_slice()[12..]))
}

fn parse_signature_parts(signature_hex: &str) -> Result<ParsedSignatureParts, String> {
    let bytes = parse_signature_bytes(signature_hex)?;
    Ok(ParsedSignatureParts {
        v: match bytes[64] {
            27 | 28 => bytes[64],
            other => other.saturating_add(27),
        },
        r: B256::from_slice(&bytes[0..32]),
        s: B256::from_slice(&bytes[32..64]),
    })
}

fn parse_signature_bytes(signature_hex: &str) -> Result<[u8; 65], String> {
    let normalized = signature_hex.trim().trim_start_matches("0x");
    let decoded = hex::decode(normalized).map_err(|e| format!("Invalid hex signature: {}", e))?;
    if decoded.len() != 65 {
        return Err("Signatures must be 65-byte hex strings.".to_string());
    }
    let mut bytes = [0u8; 65];
    bytes.copy_from_slice(&decoded);
    Ok(bytes)
}

fn normalize_recovery_id(v: u8) -> Result<RecoveryId, String> {
    let normalized = match v {
        0 | 1 => v,
        27 | 28 => v - 27,
        _ => return Err(format!("Unsupported recovery id: {}", v)),
    };
    RecoveryId::from_byte(normalized).ok_or_else(|| format!("Invalid recovery id: {}", normalized))
}

async fn fetch_gas_price(state: &AppState) -> Result<U256, String> {
    state
        .l1
        .provider()
        .get_gas_price()
        .await
        .map(U256::from)
        .map_err(|_| "Failed to fetch gas price from the configured L1 endpoints.".to_string())
}

async fn fetch_token_nonce(
    state: &AppState,
    token_contract: Address,
    owner: Address,
) -> Result<U256, String> {
    // `eth_call` is a quorum read across the configured endpoints.
    let contract = IERC20PermitRead::new(token_contract, state.l1.provider());
    contract
        .nonces(owner)
        .call()
        .await
        .map(|response| response._0)
        .map_err(|e| {
            let msg = format!(
                "Failed to read token permit nonce from {}: {}",
                token_contract, e
            );
            error!("{}", msg);
            msg
        })
}

/// Storage form of an owner address (lowercase hex).
fn owner_storage_key(owner: Address) -> String {
    format!("{owner:#x}")
}

fn wallet_quota_key(owner: Address, action: SponsoredActionKind) -> String {
    format!("wallet:{}:{}", owner_storage_key(owner), action.as_str())
}

fn ip_quota_key(chain_id: u64, ip: &str) -> String {
    format!("ip:{}:{}", chain_id, ip)
}

/// l1-tx correlation id for a sponsor request.
fn signer_request_id(request_id: &str) -> String {
    format!("relayer:{request_id}")
}

fn utc_today() -> NaiveDate {
    Utc::now().date_naive()
}
/// Upper bound for `RELAYER_TRUSTED_PROXY_HOPS`.
const MAX_TRUSTED_PROXY_HOPS: usize = 8;

/// Resolve the per-IP quota bucket for a request (ECA-29).
///
/// With `trusted_proxy_hops == 0` the TCP peer address is used. Otherwise the
/// client is the `trusted_proxy_hops`-th `X-Forwarded-For` entry counted from
/// the **right**: every trusted proxy appends the address it received the
/// request from, so the entries to the left of that position are
/// client-supplied and spoofable. With exactly one hop, `X-Real-IP` (set by
/// that proxy from `$remote_addr`) is accepted when `X-Forwarded-For` is
/// absent. Anything that is not a valid IP address falls back to the peer
/// address, so attacker-chosen strings never become quota keys. IPv6 clients
/// are bucketed by `/64`.
fn extract_client_ip(
    headers: &HeaderMap,
    peer_addr: SocketAddr,
    trusted_proxy_hops: usize,
) -> String {
    let peer = peer_addr.ip();
    if trusted_proxy_hops == 0 {
        return quota_bucket(peer);
    }
    let forwarded: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    let from_forwarded = forwarded
        .len()
        .checked_sub(trusted_proxy_hops)
        .and_then(|index| forwarded.get(index))
        .and_then(|entry| entry.parse::<std::net::IpAddr>().ok());
    let from_real_ip = || {
        if trusted_proxy_hops != 1 || !forwarded.is_empty() {
            return None;
        }
        headers
            .get("x-real-ip")?
            .to_str()
            .ok()?
            .trim()
            .parse::<std::net::IpAddr>()
            .ok()
    };
    quota_bucket(from_forwarded.or_else(from_real_ip).unwrap_or(peer))
}

/// Quota key for an address: IPv4 as-is (IPv4-mapped IPv6 is unwrapped),
/// IPv6 truncated to its `/64` so one host cannot rotate through a prefix.
fn quota_bucket(ip: std::net::IpAddr) -> String {
    match ip {
        std::net::IpAddr::V4(v4) => v4.to_string(),
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
    }
}

fn parse_address(value: &str, field_name: &str) -> Result<Address, String> {
    value
        .parse::<Address>()
        .map_err(|e| format!("Invalid {}: {}", field_name, e))
}

fn parse_u256(value: &str, field_name: &str) -> Result<U256, String> {
    value
        .parse::<U256>()
        .map_err(|e| format!("Invalid {}: {}", field_name, e))
}

fn parse_u64(value: &str, field_name: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|e| format!("Invalid {}: {}", field_name, e))
}

fn classify_send_error(err: &str) -> (StatusCode, String) {
    let lower = err.to_lowercase();
    let revert_reason = extract_revert_reason(err);
    if lower.contains("erc20: insufficient balance") {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Insufficient tCREG balance to stake the requested amount. Visit the faucet to mint testnet tokens first.".to_string(),
        );
    }
    if lower.contains("erc20: insufficient allowance")
        || lower.contains("erc20permit: invalid signature")
        || lower.contains("permit:")
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "Permit could not be applied on-chain: {}",
                revert_reason.unwrap_or_else(|| err.to_string())
            ),
        );
    }
    if lower.contains("execution reverted") {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "On-chain transaction reverted: {}",
                revert_reason.unwrap_or_else(|| err.to_string())
            ),
        );
    }
    (StatusCode::BAD_GATEWAY, err.to_string())
}

fn extract_revert_reason(err: &str) -> Option<String> {
    let marker = "execution reverted: ";
    let start = err.find(marker)? + marker.len();
    let tail = &err[start..];
    let end = tail.find(", data:").unwrap_or(tail.len());
    Some(tail[..end].trim().to_string())
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn env_string(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            let v = value.trim().to_ascii_lowercase();
            v == "true" || v == "1" || v == "yes"
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::MemoryRelayerStore;

    #[test]
    fn env_placeholder_substitution_replaces_known_values() {
        std::env::set_var("TEST_RELAYER_TOKEN", "0x1234");
        assert_eq!(
            substitute_env_placeholders("prefix-${TEST_RELAYER_TOKEN}-suffix"),
            "prefix-0x1234-suffix"
        );
    }

    #[test]
    fn infer_action_kind_maps_sponsored_helpers() {
        assert_eq!(
            infer_action_kind(
                "stakeAsPublisherWithPermit(address,uint256,uint256,uint8,bytes32,bytes32)"
            ),
            Some(SponsoredActionKind::Publisher)
        );
        assert_eq!(
            infer_action_kind(
                "applyToBeValidatorWithPermit(address,uint256,uint256,uint8,bytes32,bytes32)"
            ),
            Some(SponsoredActionKind::Validator)
        );
    }

    fn test_state(store: Arc<MemoryRelayerStore>) -> Arc<AppState> {
        Arc::new(AppState {
            config: RelayerConfig {
                port: 8083,
                rpc_endpoints: vec!["http://127.0.0.1:9/".to_string()],
                policy_path: String::new(),
                relayer_address: Address::ZERO,
                active_chain_id: 11155111,
                data_dir: String::new(),
                trusted_proxy_hops: 0,
                allowed_origins: Vec::new(),
                signer: SignerConfig::default(),
            },
            policy: LoadedPolicy {
                mode: "testnet-only".to_string(),
                signature: LoadedSignaturePolicy {
                    scheme: "eip712".to_string(),
                    domain_name: String::new(),
                    domain_version: "1".to_string(),
                },
                replay_protection: LoadedReplayProtection {
                    nonce_scope: "wallet-and-chain".to_string(),
                    max_expiry_seconds: 300,
                },
                chain: LoadedChainPolicy {
                    id: 11155111,
                    label: "private-policy-label".to_string(),
                    enabled: false,
                    daily_wallet_quota: 10,
                    daily_ip_quota: 40,
                    max_gas_per_request: 500000,
                    allow_contracts: Vec::new(),
                    actions: HashMap::new(),
                },
            },
            l1: AlloyL1Rpc::from_urls(&["http://127.0.0.1:9/".to_string()]).unwrap(),
            store,
            signer: None,
        })
    }

    #[tokio::test]
    async fn disabled_policy_rejects_direct_sponsor_without_reserving_state() {
        let store = Arc::new(MemoryRelayerStore::new());
        let state = test_state(Arc::clone(&store));
        for action in ["publisher", "validator"] {
            let request = serde_json::from_value(serde_json::json!({
                "action": action,
                "permitMessage": {
                    "owner": "", "spender": "", "value": "", "nonce": "", "deadline": ""
                },
                "intentMessage": {
                    "owner": "", "tokenContract": "", "stakingContract": "", "action": 0,
                    "amount": "", "permitNonce": "", "permitDeadline": "", "relayerNonce": "", "expiresAt": ""
                },
                "permitSignature": "", "intentSignature": ""
            }))
            .unwrap();
            let response = sponsor_request(
                State(Arc::clone(&state)),
                ConnectInfo("127.0.0.1:12345".parse().unwrap()),
                HeaderMap::new(),
                Json(request),
            )
            .await
            .into_response();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let payload: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(payload["status"], "rejected");
            assert_eq!(payload["requestId"], "");
            assert_eq!(
                payload["message"],
                "Sponsored transactions are disabled by relayer policy."
            );
            assert!(store.is_empty());
        }
    }

    #[tokio::test]
    async fn health_reports_a_missing_signer_as_degraded() {
        let state = test_state(Arc::new(MemoryRelayerStore::new()));
        let response = health_check(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn legacy_sled_journal_blocks_startup_until_exported_or_discarded() {
        let dir = std::env::temp_dir().join(format!("creg-relayer-legacy-{}", Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("relayer-state.db")).unwrap();
        std::fs::write(
            dir.join("sponsor-nonces.json"),
            format!(r#"{{"11155111:0x{}": 4}}"#, "Ab".repeat(20)),
        )
        .unwrap();
        let data_dir = dir.to_string_lossy().to_string();

        let store = MemoryRelayerStore::new();
        let error = migrate_legacy_state(&store, &data_dir)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("creg-relayer-sled-export"), "{error}");
        // The pre-sled nonce file was imported before refusing.
        assert_eq!(
            store
                .owner_nonce(11155111, &format!("0x{}", "ab".repeat(20)))
                .await
                .unwrap(),
            4
        );

        let export_path = dir.join("export.json");
        std::fs::write(
            &export_path,
            serde_json::to_vec(&LegacyJournalExport {
                format: store::LEGACY_EXPORT_FORMAT.to_string(),
                ..LegacyJournalExport::default()
            })
            .unwrap(),
        )
        .unwrap();
        std::env::set_var("RELAYER_LEGACY_JOURNAL_EXPORT", &export_path);
        let imported = migrate_legacy_state(&store, &data_dir).await;
        std::env::remove_var("RELAYER_LEGACY_JOURNAL_EXPORT");
        imported.unwrap();
        assert_eq!(
            store.meta_get(META_LEGACY_SLED).await.unwrap().as_deref(),
            Some("imported")
        );
        // Once resolved, the sled directory no longer blocks startup.
        migrate_legacy_state(&store, &data_dir).await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    fn outbox_row(status: &str, created_at: chrono::DateTime<Utc>) -> OutboxRow {
        OutboxRow {
            record: RequestStatusRecord {
                request_id: "r".to_string(),
                status: status.to_string(),
                action: "publisher".to_string(),
                owner: format!("0x{}", "ab".repeat(20)),
                chain_id: 11155111,
                tx_hash: None,
                block_number: None,
                message: "Submitting sponsored publisher transaction.".to_string(),
                created_at: created_at.to_rfc3339(),
                updated_at: created_at.to_rfc3339(),
            },
            signer_request_id: Some("relayer:r".to_string()),
            created_at,
        }
    }

    fn ledger_row(state: TxState) -> LedgerTx {
        let now = Utc::now();
        LedgerTx {
            key: l1_tx::SignerKey::new(11155111, Address::repeat_byte(1)),
            request_id: "relayer:r".to_string(),
            nonce: 3,
            state,
            to: Address::repeat_byte(2),
            value: U256::ZERO,
            input: Bytes::new(),
            gas_limit: 500_000,
            attempts: vec![l1_tx::TxAttempt {
                tx_hash: B256::repeat_byte(0xaa),
                max_fee_per_gas: 2,
                max_priority_fee_per_gas: 1,
                raw: Bytes::from_static(&[0x02]),
            }],
            included_tx_hash: None,
            block_number: Some(0x10),
            last_error: Some("insufficient funds".to_string()),
            broadcast_at: Some(now),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn outbox_follows_the_ledger_to_a_final_state() {
        let now = Utc::now();
        let row = outbox_row("submitting", now);

        let pending = outbox_update_from_ledger(&row, Some(&ledger_row(TxState::Pending)), now)
            .expect("submitting -> submitted");
        assert_eq!(pending.status, "submitted");
        assert_eq!(pending.tx_hash, Some(B256::repeat_byte(0xaa).to_string()));

        let confirmed =
            outbox_update_from_ledger(&row, Some(&ledger_row(TxState::Confirmed)), now).unwrap();
        assert_eq!(confirmed.status, "confirmed");
        assert_eq!(confirmed.block_number.as_deref(), Some("0x10"));

        for (state, needle) in [
            (TxState::Reverted, "reverted"),
            (TxState::Rejected, "insufficient funds"),
            (TxState::Displaced, "displaced"),
        ] {
            let failed = outbox_update_from_ledger(&row, Some(&ledger_row(state)), now).unwrap();
            assert_eq!(failed.status, "failed");
            assert!(failed.message.contains(needle), "{}", failed.message);
        }

        // Unchanged rows produce no write.
        let mut submitted = row.clone();
        submitted.record = pending;
        assert!(
            outbox_update_from_ledger(&submitted, Some(&ledger_row(TxState::Pending)), now)
                .is_none()
        );
    }

    #[test]
    fn unsigned_requests_fail_only_after_the_grace_period() {
        let now = Utc::now();
        let fresh = outbox_row("submitting", now);
        assert!(outbox_update_from_ledger(&fresh, None, now).is_none());

        let stale = outbox_row("submitting", now - chrono::Duration::minutes(10));
        let failed = outbox_update_from_ledger(&stale, None, now).unwrap();
        assert_eq!(failed.status, "failed");
        assert!(failed.message.contains("before signing"));
    }

    #[test]
    fn quota_and_owner_keys_are_namespaced_and_case_insensitive() {
        let owner: Address = format!("0x{}", "Ab".repeat(20)).parse().unwrap();
        assert_eq!(owner_storage_key(owner), format!("0x{}", "ab".repeat(20)));
        assert_eq!(
            wallet_quota_key(owner, SponsoredActionKind::Validator),
            format!("wallet:0x{}:validator", "ab".repeat(20))
        );
        assert_eq!(ip_quota_key(1, "2001:db8::/64"), "ip:1:2001:db8::/64");
        assert_eq!(signer_request_id("abc"), "relayer:abc");
    }

    fn xff(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append("x-forwarded-for", value.parse().unwrap());
        }
        headers
    }

    /// ECA-29: the leftmost (client-supplied) XFF entry is never trusted.
    #[test]
    fn client_ip_is_taken_from_the_right_by_trusted_hops() {
        let peer: SocketAddr = "10.0.0.9:443".parse().unwrap();
        let spoofed = xff(&["6.6.6.6, 203.0.113.7"]);

        assert_eq!(extract_client_ip(&spoofed, peer, 0), "10.0.0.9");
        assert_eq!(extract_client_ip(&spoofed, peer, 1), "203.0.113.7");
        assert_eq!(extract_client_ip(&spoofed, peer, 2), "6.6.6.6");
        // Fewer entries than trusted hops: the chain is incomplete, use peer.
        assert_eq!(extract_client_ip(&spoofed, peer, 3), "10.0.0.9");
        // Multiple header lines are one list.
        let split = xff(&["6.6.6.6", "203.0.113.7"]);
        assert_eq!(extract_client_ip(&split, peer, 1), "203.0.113.7");
    }

    #[test]
    fn garbage_forwarding_values_never_become_quota_keys() {
        let peer: SocketAddr = "10.0.0.9:443".parse().unwrap();
        for value in ["not-an-ip", "1.2.3.4.5", "' OR 1=1 --", "::zz"] {
            assert_eq!(extract_client_ip(&xff(&[value]), peer, 1), "10.0.0.9");
        }
        let mut real_ip = HeaderMap::new();
        real_ip.insert("x-real-ip", "198.51.100.4".parse().unwrap());
        assert_eq!(extract_client_ip(&real_ip, peer, 1), "198.51.100.4");
        assert_eq!(extract_client_ip(&real_ip, peer, 2), "10.0.0.9");
        assert_eq!(extract_client_ip(&real_ip, peer, 0), "10.0.0.9");
    }

    #[test]
    fn ipv6_clients_share_a_64_bucket() {
        let peer: SocketAddr = "[2001:db8:1:2:aaaa::1]:443".parse().unwrap();
        assert_eq!(
            extract_client_ip(&HeaderMap::new(), peer, 0),
            "2001:db8:1:2::/64"
        );
        let rotated = xff(&["2001:db8:1:2:ffff:ffff:ffff:fffe"]);
        assert_eq!(extract_client_ip(&rotated, peer, 1), "2001:db8:1:2::/64");
        let mapped: SocketAddr = "[::ffff:192.0.2.1]:443".parse().unwrap();
        assert_eq!(extract_client_ip(&HeaderMap::new(), mapped, 0), "192.0.2.1");
    }

    #[test]
    fn sponsored_gas_policy_accepts_boundary_and_rejects_excess_or_invalid_values() {
        assert!(validate_sponsored_gas(499999, 500000).is_ok());
        assert!(validate_sponsored_gas(500000, 500000).is_ok());
        assert_eq!(
            validate_sponsored_gas(500001, 500000).unwrap_err(),
            "Sponsored transaction exceeds the relayer gas policy limit."
        );
        assert!(validate_sponsored_gas(0, 500000).is_err());
        assert!(validate_sponsored_gas(21000, 0).is_err());
    }
}
