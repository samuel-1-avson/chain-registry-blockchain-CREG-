#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use alloy::{
    network::{EthereumWallet, TransactionBuilder},
    primitives::{keccak256, Address, B256, U256},
    providers::{Provider, ProviderBuilder, RootProvider},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolValue,
};
use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use common::l1_rpc::FailoverTransport;
use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sled::transaction::ConflictableTransactionError;
use std::{collections::HashMap, fs, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::time::sleep;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tracing::{error, info};
use uuid::Uuid;

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
    rpc_url: String,
    private_key: String,
    policy_path: String,
    relayer_address: Address,
    active_chain_id: u64,
    /// Directory for relayer-local persistence (sponsor nonce journal).
    data_dir: String,
    /// Only honour `X-Forwarded-For` / `X-Real-IP` when true. Enable only when
    /// the relayer sits behind a trusted reverse proxy that sets these
    /// headers; otherwise clients can spoof their IP to bypass per-IP quotas.
    trust_proxy: bool,
    /// Allowed CORS origins (exact match). Empty → allow any origin (dev only).
    allowed_origins: Vec<String>,
}

impl RelayerConfig {
    async fn from_env(
        _http_client: &reqwest::Client,
    ) -> anyhow::Result<(Self, chain_registry_secrets::SecretsProvider)> {
        let secrets = chain_registry_secrets::SecretsProvider::from_env()?;
        let private_key = secrets
            .secp256k1_signing_key_hex(chain_registry_secrets::HotKeyRole::Relayer)
            .await?;
        let signer: PrivateKeySigner = private_key.parse()?;
        let relayer_address = signer.address();

        let rpc_url = env_string("RELAYER_RPC_URL", "http://localhost:8545");
        let active_chain_id = fetch_chain_id(&rpc_url).await?;

        let allowed_origins: Vec<String> = env_string("RELAYER_ALLOWED_ORIGINS", "")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok((
            Self {
                port: env_u16("RELAYER_PORT", 8083),
                rpc_url,
                private_key,
                policy_path: env_string(
                    "RELAYER_POLICY_PATH",
                    "config/relayer-policy.example.json",
                ),
                relayer_address,
                active_chain_id,
                data_dir: env_string("RELAYER_DATA_DIR", "."),
                trust_proxy: env_bool("RELAYER_TRUST_PROXY", false),
                allowed_origins,
            },
            secrets,
        ))
    }
}

#[derive(Clone)]
struct AppState {
    config: RelayerConfig,
    policy: LoadedPolicy,
    http_client: reqwest::Client,
    journal: sled::Tree,
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

#[derive(Clone, Deserialize, Serialize)]
struct DailyCounter {
    day: String,
    count: u64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestStatusRecord {
    request_id: String,
    status: String,
    action: String,
    owner: String,
    chain_id: u64,
    tx_hash: Option<String>,
    block_number: Option<String>,
    message: String,
    created_at: String,
    updated_at: String,
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

#[derive(Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
    error: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcReceipt {
    block_number: Option<String>,
    status: Option<String>,
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

    let http_client = reqwest::Client::new();
    let (config, secrets) = RelayerConfig::from_env(&http_client).await?;
    secrets.warn_hot_key_if_env(
        "relayer",
        chain_registry_secrets::HotKeyRole::Relayer,
        &config.private_key,
        common::is_testnet_env(),
    );
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
    info!("  RPC:             {}", config.rpc_url);

    let journal = open_durable_journal(&config.data_dir)?;
    let state = Arc::new(AppState {
        config: config.clone(),
        policy,
        http_client,
        journal,
    });

    let restored = load_request_statuses(&state.journal)?;
    let restored_count = restored.len();
    for mut record in restored {
        if record.status == "submitting" && record.tx_hash.is_none() {
            record.status = "reconciliation_required".to_string();
            record.message = "Relayer restarted before the broadcast result was durably recorded. This request is held for transaction reconciliation; its nonce and quota remain reserved.".to_string();
            record.updated_at = Utc::now().to_rfc3339();
            persist_status_record(&state.journal, &record).map_err(anyhow::Error::msg)?;
        } else if matches!(record.status.as_str(), "submitted" | "timed_out") {
            if let Some(tx_hash) = record.tx_hash.clone() {
                tokio::spawn(watch_request_receipt(
                    Arc::clone(&state),
                    record.request_id.clone(),
                    tx_hash,
                ));
            }
        }
    }
    info!("  Data dir:        {}", config.data_dir);
    info!("  Trust proxy hdr: {}", config.trust_proxy);
    info!("  Restored nonces: {}", restored_count);

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
    let payload = serde_json::json!({
        "status": "healthy",
        "chainId": state.config.active_chain_id,
        "relayerAddress": state.config.relayer_address.to_string(),
        "policyChain": state.policy.chain.label,
    });
    (StatusCode::OK, Json(payload))
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
    let client_ip = extract_client_ip(&headers, peer_addr, state.config.trust_proxy);

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
    let client_ip = extract_client_ip(&headers, peer_addr, state.config.trust_proxy);

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

    if let Err(message) = check_quota(
        &state.journal,
        &wallet_quota_key(prepared.owner, prepared.action),
        state.policy.chain.daily_wallet_quota,
    ) {
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
    if let Err(message) = check_quota(
        &state.journal,
        &ip_quota_key(state.config.active_chain_id, &client_ip),
        state.policy.chain.daily_ip_quota,
    ) {
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

    let owner_nonce_key = sponsor_nonce_key(state.config.active_chain_id, prepared.owner);
    let expected_nonce = match current_nonce(&state.journal, &owner_nonce_key) {
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

    let request_id = Uuid::new_v4().to_string();
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
    let wallet_key = wallet_quota_key(prepared.owner, prepared.action);
    let ip_key = ip_quota_key(state.config.active_chain_id, &client_ip);
    if let Err(message) = reserve_sponsorship(
        &state.journal,
        &owner_nonce_key,
        expected_nonce,
        &wallet_key,
        state.policy.chain.daily_wallet_quota,
        &ip_key,
        state.policy.chain.daily_ip_quota,
        &initial_record,
    ) {
        let status = if message == "QUOTA_REACHED" {
            StatusCode::TOO_MANY_REQUESTS
        } else if message.starts_with("NONCE_CONFLICT:") {
            StatusCode::CONFLICT
        } else {
            error!(
                "Failed to atomically reserve sponsored request: {}",
                message
            );
            StatusCode::SERVICE_UNAVAILABLE
        };
        let detail = if let Some(actual) = message.strip_prefix("NONCE_CONFLICT:") {
            format!("Relayer nonce advanced to {} while this request was being verified; request a new quote.", actual)
        } else if message == "QUOTA_REACHED" {
            "Daily sponsorship quota has been reached.".to_string()
        } else {
            "Durable relayer state is unavailable; no transaction was submitted.".to_string()
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

    let tx_hash = match send_sponsored_transaction(&state, transaction).await {
        Ok(hash) => hash,
        Err(err) => {
            let (status_code, friendly) = classify_send_error(&err);
            if let Err(storage_error) = update_status(
                &state,
                &request_id,
                "reconciliation_required",
                &format!(
                    "Broadcast result is uncertain after the RPC error: {}",
                    friendly
                ),
                None,
                None,
            ) {
                error!(
                    "Could not persist ambiguous sponsor result {}: {}",
                    request_id, storage_error
                );
            }
            return (
                status_code,
                Json(SponsorResponse {
                    success: false,
                    request_id,
                    status: "reconciliation_required".to_string(),
                    tx_hash: None,
                    message: format!("{} The reserved nonce will not be reused until this request is reconciled.", friendly),
                }),
            )
                .into_response();
        }
    };

    if let Err(storage_error) = update_status(
        &state,
        &request_id,
        "submitted",
        &format!(
            "Sponsored {} transaction submitted.",
            prepared.action.as_str()
        ),
        Some(tx_hash.clone()),
        None,
    ) {
        error!(
            "Transaction {} was submitted but its durable status update failed: {}",
            tx_hash, storage_error
        );
        return (
            StatusCode::ACCEPTED,
            Json(SponsorResponse {
                success: true,
                request_id,
                status: "submitted_reconciliation_required".to_string(),
                tx_hash: Some(tx_hash),
                message: "Transaction was submitted. Durable status storage needs operator reconciliation; use the returned transaction hash.".to_string(),
            }),
        )
            .into_response();
    }

    tokio::spawn(watch_request_receipt(
        Arc::clone(&state),
        request_id.clone(),
        tx_hash.clone(),
    ));

    (
        StatusCode::ACCEPTED,
        Json(SponsorResponse {
            success: true,
            request_id,
            status: "submitted".to_string(),
            tx_hash: Some(tx_hash),
            message: format!(
                "Sponsored {} transaction submitted.",
                prepared.action.as_str()
            ),
        }),
    )
        .into_response()
}

async fn get_status(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> impl IntoResponse {
    match state.journal.get(request_storage_key(&request_id)) {
        Ok(Some(bytes)) => match serde_json::from_slice::<RequestStatusRecord>(&bytes) {
            Ok(status) => (StatusCode::OK, Json(status)).into_response(),
            Err(error) => {
                error!(
                    "Stored relayer request {} is unreadable: {}",
                    request_id, error
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "Stored relayer request state is unreadable.",
                        "requestId": request_id,
                    })),
                )
                    .into_response()
            }
        },
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
        &state.journal,
        &wallet_quota_key(owner, action),
        chain.daily_wallet_quota,
    )?;
    check_quota(
        &state.journal,
        &ip_quota_key(state.config.active_chain_id, client_ip),
        chain.daily_ip_quota,
    )?;

    let permit_nonce = fetch_token_nonce(state, token_contract, owner).await?;
    let relayer_nonce = current_nonce(
        &state.journal,
        &sponsor_nonce_key(state.config.active_chain_id, owner),
    )?;
    let permit_deadline = U256::from(
        (Utc::now().timestamp() as u64) + state.policy.replay_protection.max_expiry_seconds,
    );
    let expires_at = U256::from(
        (Utc::now().timestamp() as u64) + state.policy.replay_protection.max_expiry_seconds,
    );
    let gas_price = fetch_gas_price(&state.http_client, &state.config.rpc_url).await?;
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

fn request_storage_key(request_id: &str) -> Vec<u8> {
    format!("request:{}", request_id).into_bytes()
}

fn current_nonce(journal: &sled::Tree, key: &str) -> Result<u64, String> {
    let storage_key = format!("nonce:{}", key);
    match journal
        .get(storage_key.as_bytes())
        .map_err(|error| error.to_string())?
    {
        Some(value) => {
            let bytes: [u8; 8] = value
                .as_ref()
                .try_into()
                .map_err(|_| "Stored relayer nonce has an invalid encoding.".to_string())?;
            Ok(u64::from_be_bytes(bytes))
        }
        None => Ok(0),
    }
}

fn check_quota(journal: &sled::Tree, key: &str, limit: u64) -> Result<(), String> {
    let storage_key = format!("quota:{}", key);
    let Some(bytes) = journal
        .get(storage_key.as_bytes())
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let counter: DailyCounter = serde_json::from_slice(&bytes)
        .map_err(|_| "Stored relayer quota has an invalid encoding.".to_string())?;
    let today = Utc::now().format("%Y-%m-%d").to_string();
    if counter.day == today && counter.count >= limit {
        return Err(format!("Daily quota reached for {}.", key));
    }
    Ok(())
}

fn persist_status_record(journal: &sled::Tree, record: &RequestStatusRecord) -> Result<(), String> {
    let bytes = serde_json::to_vec(record).map_err(|error| error.to_string())?;
    journal
        .insert(request_storage_key(&record.request_id), bytes)
        .map_err(|error| error.to_string())?;
    journal.flush().map_err(|error| error.to_string())?;
    Ok(())
}

fn load_request_statuses(journal: &sled::Tree) -> anyhow::Result<Vec<RequestStatusRecord>> {
    let mut records = Vec::new();
    for entry in journal.scan_prefix(b"request:") {
        let (_, bytes) = entry?;
        records.push(serde_json::from_slice::<RequestStatusRecord>(&bytes)?);
    }
    Ok(records)
}

fn open_durable_journal(data_dir: &str) -> anyhow::Result<sled::Tree> {
    let directory = PathBuf::from(data_dir);
    fs::create_dir_all(&directory)?;
    let db = sled::open(directory.join("relayer-state.db"))?;
    let journal = db.open_tree("state")?;

    match journal.get(b"meta:schema")? {
        Some(version) if version.as_ref() == b"1" => {}
        Some(_) => {
            anyhow::bail!("Unsupported relayer journal schema; refusing to reset durable state.")
        }
        None => {
            journal.insert(b"meta:schema", b"1")?;
            journal.flush()?;
        }
    }

    // Migrate the former nonce file once. An unreadable or conflicting legacy
    // journal stops startup instead of silently restoring nonce zero.
    if journal.get(b"meta:legacy-nonces-migrated")?.is_none() {
        let legacy_path = directory.join("sponsor-nonces.json");
        if legacy_path.exists() {
            let bytes = fs::read(&legacy_path)?;
            let legacy: HashMap<String, u64> = serde_json::from_slice(&bytes)?;
            for (key, value) in legacy {
                let storage_key = format!("nonce:{}", key);
                let encoded = value.to_be_bytes();
                match journal.get(storage_key.as_bytes())? {
                    Some(existing) if existing.as_ref() != encoded.as_slice() => {
                        anyhow::bail!("Legacy relayer nonce conflicts with the durable journal; operator review is required.");
                    }
                    Some(_) => {}
                    None => {
                        journal.insert(storage_key.as_bytes(), encoded.as_slice())?;
                    }
                }
            }
        }
        journal.insert(b"meta:legacy-nonces-migrated", b"1")?;
        journal.flush()?;
    }
    Ok(journal)
}

#[allow(clippy::too_many_arguments)]
fn reserve_sponsorship(
    journal: &sled::Tree,
    owner_nonce_key: &str,
    expected_nonce: u64,
    wallet_quota_key: &str,
    wallet_quota_limit: u64,
    ip_quota_key: &str,
    ip_quota_limit: u64,
    initial_record: &RequestStatusRecord,
) -> Result<(), String> {
    let nonce_key = format!("nonce:{}", owner_nonce_key).into_bytes();
    let wallet_key = format!("quota:{}", wallet_quota_key).into_bytes();
    let ip_key = format!("quota:{}", ip_quota_key).into_bytes();
    let request_key = request_storage_key(&initial_record.request_id);
    let request_bytes = serde_json::to_vec(initial_record).map_err(|error| error.to_string())?;
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let next_nonce = expected_nonce
        .checked_add(1)
        .ok_or_else(|| "Relayer nonce is exhausted.".to_string())?;

    let transaction: Result<(), sled::transaction::TransactionError<String>> =
        journal.transaction(|tree| {
            if tree.get(&request_key)?.is_some() {
                return Err(ConflictableTransactionError::Abort(
                    "Request identifier collision.".to_string(),
                ));
            }

            let actual_nonce = match tree.get(&nonce_key)? {
                Some(value) => {
                    let bytes: [u8; 8] = value.as_ref().try_into().map_err(|_| {
                        ConflictableTransactionError::Abort(
                            "Stored relayer nonce has an invalid encoding.".to_string(),
                        )
                    })?;
                    u64::from_be_bytes(bytes)
                }
                None => 0,
            };
            if actual_nonce != expected_nonce {
                return Err(ConflictableTransactionError::Abort(format!(
                    "NONCE_CONFLICT:{}",
                    actual_nonce
                )));
            }

            let read_counter = |key: &[u8], limit: u64| {
                let count = match tree.get(key)? {
                    Some(bytes) => {
                        let counter: DailyCounter =
                            serde_json::from_slice(&bytes).map_err(|_| {
                                ConflictableTransactionError::Abort(
                                    "Stored relayer quota has an invalid encoding.".to_string(),
                                )
                            })?;
                        if counter.day == today && counter.count >= limit {
                            return Err(ConflictableTransactionError::Abort(
                                "QUOTA_REACHED".to_string(),
                            ));
                        }
                        if counter.day == today {
                            counter.count
                        } else {
                            0
                        }
                    }
                    None => 0,
                };
                Ok(count)
            };
            let wallet_count = read_counter(&wallet_key, wallet_quota_limit)?;
            let ip_count = read_counter(&ip_key, ip_quota_limit)?;
            let wallet_next = serde_json::to_vec(&DailyCounter {
                day: today.clone(),
                count: wallet_count.saturating_add(1),
            })
            .map_err(|error| ConflictableTransactionError::Abort(error.to_string()))?;
            let ip_next = serde_json::to_vec(&DailyCounter {
                day: today.clone(),
                count: ip_count.saturating_add(1),
            })
            .map_err(|error| ConflictableTransactionError::Abort(error.to_string()))?;

            tree.insert(nonce_key.as_slice(), next_nonce.to_be_bytes().as_slice())?;
            tree.insert(wallet_key.as_slice(), wallet_next)?;
            tree.insert(ip_key.as_slice(), ip_next)?;
            tree.insert(request_key.as_slice(), request_bytes.as_slice())?;
            Ok(())
        });
    transaction.map_err(|error| error.to_string())?;
    journal.flush().map_err(|error| error.to_string())?;
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

async fn prepare_sponsored_transaction(
    state: &AppState,
    prepared: &ValidatedActionRequest,
    permit_signature: &ParsedSignatureParts,
) -> Result<TransactionRequest, String> {
    let provider = l1_read_provider(&state.config.rpc_url)
        .map_err(|_| "Relayer RPC configuration is invalid.".to_string())?;

    let contract = IStakingSponsored::new(prepared.staking_contract, &provider);
    let deadline = prepared.permit_deadline;

    let mut transaction = match prepared.action {
        SponsoredActionKind::Publisher => contract
            .stakeAsPublisherWithPermit(
                prepared.owner,
                prepared.amount,
                deadline,
                permit_signature.v,
                permit_signature.r,
                permit_signature.s,
            )
            .from(state.config.relayer_address)
            .into_transaction_request(),
        SponsoredActionKind::Validator => contract
            .applyToBeValidatorWithPermit(
                prepared.owner,
                prepared.amount,
                deadline,
                permit_signature.v,
                permit_signature.r,
                permit_signature.s,
            )
            .from(state.config.relayer_address)
            .into_transaction_request(),
    };

    let estimated_gas = provider.estimate_gas(&transaction).await.map_err(|_| {
        "Unable to estimate sponsored transaction gas; no transaction was submitted.".to_string()
    })?;
    validate_sponsored_gas(estimated_gas, state.policy.chain.max_gas_per_request)?;
    // Keep the checked calldata/sender and an explicit policy ceiling so the
    // signing provider cannot replace the estimate with an uncapped gas limit.
    transaction.set_gas_limit(state.policy.chain.max_gas_per_request);
    Ok(transaction)
}

async fn send_sponsored_transaction(
    state: &AppState,
    transaction: TransactionRequest,
) -> Result<String, String> {
    let signer: PrivateKeySigner = state
        .config
        .private_key
        .parse()
        .map_err(|_| "Relayer signing configuration is invalid.".to_string())?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(
            state
                .config
                .rpc_url
                .parse()
                .map_err(|_| "Relayer RPC configuration is invalid.".to_string())?,
        );
    let pending_tx = provider.send_transaction(transaction).await.map_err(|_| {
        "Failed to submit sponsored transaction; its broadcast result requires reconciliation."
            .to_string()
    })?;

    Ok(pending_tx.tx_hash().to_string())
}

async fn watch_request_receipt(state: Arc<AppState>, request_id: String, tx_hash: String) {
    for _ in 0..90 {
        match fetch_transaction_receipt(&state.http_client, &state.config.rpc_url, &tx_hash).await {
            Ok(Some(receipt)) => {
                let status = if receipt.status.as_deref() == Some("0x1") {
                    "confirmed"
                } else {
                    "failed"
                };
                let message = if status == "confirmed" {
                    "Sponsored transaction confirmed."
                } else {
                    "Sponsored transaction reverted on-chain."
                };
                if let Err(error) = update_status(
                    &state,
                    &request_id,
                    status,
                    message,
                    Some(tx_hash.clone()),
                    receipt.block_number.clone(),
                ) {
                    error!(
                        "Failed to persist receipt status for {}: {}",
                        request_id, error
                    );
                }
                return;
            }
            Ok(None) => sleep(Duration::from_secs(2)).await,
            Err(err) => {
                error!("Receipt polling failed for {}: {}", request_id, err);
                sleep(Duration::from_secs(2)).await;
            }
        }
    }

    if let Err(error) = update_status(
        &state,
        &request_id,
        "timed_out",
        "Timed out while waiting for the sponsored transaction receipt.",
        Some(tx_hash),
        None,
    ) {
        error!(
            "Failed to persist timeout status for {}: {}",
            request_id, error
        );
    }
}

fn update_status(
    state: &AppState,
    request_id: &str,
    status: &str,
    message: &str,
    tx_hash: Option<String>,
    block_number: Option<String>,
) -> Result<(), String> {
    let key = request_storage_key(request_id);
    let bytes = state
        .journal
        .get(&key)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("request {} is missing from durable storage", request_id))?;
    let mut record: RequestStatusRecord = serde_json::from_slice(&bytes)
        .map_err(|error| format!("request {} is unreadable: {}", request_id, error))?;
    record.status = status.to_string();
    record.message = message.to_string();
    if tx_hash.is_some() {
        record.tx_hash = tx_hash;
    }
    if block_number.is_some() {
        record.block_number = block_number;
    }
    record.updated_at = Utc::now().to_rfc3339();
    persist_status_record(&state.journal, &record)
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

/// Read-only L1 provider over `RELAYER_RPC_URL` plus the comma-separated
/// `RELAYER_RPC_FALLBACKS`. The chain ID (which binds the EIP-712 domain) and
/// contract reads need a strict majority of the configured endpoints to
/// agree; gas estimates fail over. Signed submissions do not use this
/// provider: they go to the primary once (`send_sponsored_transaction`).
fn l1_read_provider(rpc_url: &str) -> anyhow::Result<RootProvider<FailoverTransport>> {
    let urls = common::l1_rpc::parse_rpc_endpoints(
        rpc_url,
        &std::env::var("RELAYER_RPC_FALLBACKS").unwrap_or_default(),
        "RELAYER_RPC_URL",
        "RELAYER_RPC_FALLBACKS",
    )?;
    common::l1_rpc::build_provider_from_urls(&urls)
}

async fn fetch_chain_id(rpc_url: &str) -> anyhow::Result<u64> {
    l1_read_provider(rpc_url)?
        .get_chain_id()
        .await
        .map_err(|_| {
            anyhow::anyhow!("configured relayer RPC endpoints did not agree on a chain ID")
        })
}

async fn fetch_gas_price(http_client: &reqwest::Client, rpc_url: &str) -> Result<U256, String> {
    let response: RpcEnvelope<String> =
        rpc_call(http_client, rpc_url, "eth_gasPrice", serde_json::json!([]))
            .await
            .map_err(|e| format!("Failed to fetch gas price: {}", e))?;

    let result = response.result.ok_or_else(|| {
        response
            .error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Missing gas price result".to_string())
    })?;
    U256::from_str_radix(result.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Failed to parse gas price: {}", e))
}

async fn fetch_transaction_receipt(
    http_client: &reqwest::Client,
    rpc_url: &str,
    tx_hash: &str,
) -> Result<Option<RpcReceipt>, String> {
    let response: RpcEnvelope<RpcReceipt> = rpc_call(
        http_client,
        rpc_url,
        "eth_getTransactionReceipt",
        serde_json::json!([tx_hash]),
    )
    .await
    .map_err(|e| format!("Failed to fetch transaction receipt: {}", e))?;

    Ok(response.result)
}

async fn fetch_token_nonce(
    state: &AppState,
    token_contract: Address,
    owner: Address,
) -> Result<U256, String> {
    let provider = l1_read_provider(&state.config.rpc_url)
        .map_err(|e| format!("Invalid relayer RPC configuration: {e}"))?;
    let contract = IERC20PermitRead::new(token_contract, &provider);
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

async fn rpc_call<T: for<'de> Deserialize<'de>>(
    http_client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: Value,
) -> anyhow::Result<T> {
    Ok(http_client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1,
        }))
        .send()
        .await?
        .json::<T>()
        .await?)
}

fn wallet_quota_key(owner: Address, action: SponsoredActionKind) -> String {
    format!("{}:{}", owner, action.as_str())
}

fn ip_quota_key(chain_id: u64, ip: &str) -> String {
    format!("{}:{}", chain_id, ip)
}

fn sponsor_nonce_key(chain_id: u64, owner: Address) -> String {
    format!("{}:{}", chain_id, owner)
}

/// Resolve the client IP used for per-IP quotas. Proxy headers
/// (`X-Forwarded-For`, `X-Real-IP`) are only honoured when `trust_proxy` is
/// set, because otherwise any client can spoof them to evade the IP quota.
/// When not trusting proxies we always use the real TCP peer address.
fn extract_client_ip(headers: &HeaderMap, peer_addr: SocketAddr, trust_proxy: bool) -> String {
    if !trust_proxy {
        return peer_addr.ip().to_string();
    }
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(|value| value.trim().to_string())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(|value| value.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| peer_addr.ip().to_string())
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

#[allow(dead_code)]
fn parse_hex_u64(value: &str) -> Result<u64, String> {
    u64::from_str_radix(value.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Failed to parse hex u64: {}", e))
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

    #[tokio::test]
    async fn disabled_policy_rejects_direct_sponsor_without_reserving_state() {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let state = Arc::new(AppState {
            config: RelayerConfig {
                port: 8083,
                rpc_url: "invalid-rpc-configuration".to_string(),
                private_key: String::new(),
                policy_path: String::new(),
                relayer_address: Address::ZERO,
                active_chain_id: 11155111,
                data_dir: String::new(),
                trust_proxy: false,
                allowed_origins: Vec::new(),
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
            http_client: reqwest::Client::new(),
            journal: db.open_tree("state").unwrap(),
        });
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
            assert!(state.journal.is_empty());
        }
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
