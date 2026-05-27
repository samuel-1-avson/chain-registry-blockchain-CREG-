#![deny(clippy::unwrap_used)]

use alloy::{
    network::EthereumWallet,
    primitives::{keccak256, Address, B256, U256},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
    sol,
    sol_types::SolValue,
};
use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use dashmap::DashMap;
use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, fs, net::SocketAddr, sync::Arc, time::Duration};
use tokio::time::sleep;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info};
use uuid::Uuid;

const EIP712_DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const SPONSORED_STAKE_INTENT_TYPE: &str = "SponsoredStakeIntent(address owner,address tokenContract,address stakingContract,uint8 action,uint256 amount,uint256 permitNonce,uint256 permitDeadline,uint256 relayerNonce,uint256 expiresAt)";
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
}

impl RelayerConfig {
    async fn from_env(http_client: &reqwest::Client) -> anyhow::Result<Self> {
        let private_key =
            std::env::var("RELAYER_PRIVATE_KEY").expect("RELAYER_PRIVATE_KEY must be set");
        let signer: PrivateKeySigner = private_key.parse()?;
        let relayer_address = signer.address();

        let rpc_url = env_string("RELAYER_RPC_URL", "http://localhost:8545");
        let active_chain_id = fetch_chain_id(http_client, &rpc_url).await?;

        Ok(Self {
            port: env_u16("RELAYER_PORT", 8083),
            rpc_url,
            private_key,
            policy_path: env_string("RELAYER_POLICY_PATH", "config/relayer-policy.example.json"),
            relayer_address,
            active_chain_id,
        })
    }
}

#[derive(Clone)]
struct AppState {
    config: RelayerConfig,
    policy: LoadedPolicy,
    http_client: reqwest::Client,
    statuses: DashMap<String, RequestStatusRecord>,
    sponsor_nonces: DashMap<String, u64>,
    wallet_daily_counts: DashMap<String, DailyCounter>,
    ip_daily_counts: DashMap<String, DailyCounter>,
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

#[derive(Clone)]
struct DailyCounter {
    day: String,
    count: u64,
}

#[derive(Clone, Serialize)]
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
    let config = RelayerConfig::from_env(&http_client).await?;
    common::warn_hot_key_from_env(
        "relayer",
        "RELAYER_PRIVATE_KEY",
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

    let state = Arc::new(AppState {
        config: config.clone(),
        policy,
        http_client,
        statuses: DashMap::new(),
        sponsor_nonces: DashMap::new(),
        wallet_daily_counts: DashMap::new(),
        ip_daily_counts: DashMap::new(),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

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
    let client_ip = extract_client_ip(&headers, peer_addr);

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
    let client_ip = extract_client_ip(&headers, peer_addr);

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
        &state.wallet_daily_counts,
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
        &state.ip_daily_counts,
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
    let expected_nonce = current_nonce(&state.sponsor_nonces, &owner_nonce_key);
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

    state
        .sponsor_nonces
        .insert(owner_nonce_key, expected_nonce + 1);
    record_quota(
        &state.wallet_daily_counts,
        &wallet_quota_key(prepared.owner, prepared.action),
    );
    record_quota(
        &state.ip_daily_counts,
        &ip_quota_key(state.config.active_chain_id, &client_ip),
    );

    let request_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    state.statuses.insert(
        request_id.clone(),
        RequestStatusRecord {
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
        },
    );

    let tx_hash = match send_sponsored_transaction(&state, &prepared, &permit_signature).await {
        Ok(hash) => hash,
        Err(err) => {
            let (status_code, friendly) = classify_send_error(&err);
            update_status(
                &state,
                &request_id,
                "failed",
                &format!("Relayer send failed: {}", friendly),
                None,
                None,
            );
            return (
                status_code,
                Json(SponsorResponse {
                    success: false,
                    request_id,
                    status: "failed".to_string(),
                    tx_hash: None,
                    message: friendly,
                }),
            )
                .into_response();
        }
    };

    update_status(
        &state,
        &request_id,
        "submitted",
        &format!(
            "Sponsored {} transaction submitted.",
            prepared.action.as_str()
        ),
        Some(tx_hash.clone()),
        None,
    );

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
    match state.statuses.get(&request_id) {
        Some(status) => (StatusCode::OK, Json(status.clone())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "Unknown relayer request id.",
                "requestId": request_id,
            })),
        )
            .into_response(),
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
        &state.wallet_daily_counts,
        &wallet_quota_key(owner, action),
        chain.daily_wallet_quota,
    )?;
    check_quota(
        &state.ip_daily_counts,
        &ip_quota_key(state.config.active_chain_id, client_ip),
        chain.daily_ip_quota,
    )?;

    let permit_nonce = fetch_token_nonce(state, token_contract, owner).await?;
    let relayer_nonce = current_nonce(
        &state.sponsor_nonces,
        &sponsor_nonce_key(state.config.active_chain_id, owner),
    );
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

async fn send_sponsored_transaction(
    state: &AppState,
    prepared: &ValidatedActionRequest,
    permit_signature: &ParsedSignatureParts,
) -> Result<String, String> {
    let signer: PrivateKeySigner = state
        .config
        .private_key
        .parse()
        .map_err(|e| format!("Invalid relayer private key: {}", e))?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(
            state
                .config
                .rpc_url
                .parse()
                .map_err(|e| format!("Invalid relayer RPC URL: {}", e))?,
        );

    let contract = IStakingSponsored::new(prepared.staking_contract, &provider);
    let deadline = prepared.permit_deadline;

    let pending_tx = match prepared.action {
        SponsoredActionKind::Publisher => contract
            .stakeAsPublisherWithPermit(
                prepared.owner,
                prepared.amount,
                deadline,
                permit_signature.v,
                permit_signature.r,
                permit_signature.s,
            )
            .send()
            .await
            .map_err(|e| format!("Failed to submit sponsored publisher stake: {}", e))?,
        SponsoredActionKind::Validator => contract
            .applyToBeValidatorWithPermit(
                prepared.owner,
                prepared.amount,
                deadline,
                permit_signature.v,
                permit_signature.r,
                permit_signature.s,
            )
            .send()
            .await
            .map_err(|e| format!("Failed to submit sponsored validator application: {}", e))?,
    };

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
                update_status(
                    &state,
                    &request_id,
                    status,
                    message,
                    Some(tx_hash.clone()),
                    receipt.block_number.clone(),
                );
                return;
            }
            Ok(None) => sleep(Duration::from_secs(2)).await,
            Err(err) => {
                error!("Receipt polling failed for {}: {}", request_id, err);
                sleep(Duration::from_secs(2)).await;
            }
        }
    }

    update_status(
        &state,
        &request_id,
        "timed_out",
        "Timed out while waiting for the sponsored transaction receipt.",
        Some(tx_hash),
        None,
    );
}

fn update_status(
    state: &AppState,
    request_id: &str,
    status: &str,
    message: &str,
    tx_hash: Option<String>,
    block_number: Option<String>,
) {
    if let Some(mut entry) = state.statuses.get_mut(request_id) {
        entry.status = status.to_string();
        entry.message = message.to_string();
        if tx_hash.is_some() {
            entry.tx_hash = tx_hash;
        }
        if block_number.is_some() {
            entry.block_number = block_number;
        }
        entry.updated_at = Utc::now().to_rfc3339();
    }
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

async fn fetch_chain_id(http_client: &reqwest::Client, rpc_url: &str) -> anyhow::Result<u64> {
    let response: RpcEnvelope<String> =
        rpc_call(http_client, rpc_url, "eth_chainId", serde_json::json!([])).await?;

    let result = response
        .result
        .ok_or_else(|| anyhow::anyhow!("Missing chainId result from RPC"))?;
    parse_hex_u64(&result).map_err(anyhow::Error::msg)
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
    let provider = ProviderBuilder::new().on_http(
        state
            .config
            .rpc_url
            .parse()
            .map_err(|e| format!("Invalid relayer RPC URL: {}", e))?,
    );
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

fn current_nonce(store: &DashMap<String, u64>, key: &str) -> u64 {
    store.get(key).map(|entry| *entry).unwrap_or(0)
}

fn check_quota(store: &DashMap<String, DailyCounter>, key: &str, limit: u64) -> Result<(), String> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    if let Some(counter) = store.get(key) {
        if counter.day == today && counter.count >= limit {
            return Err(format!("Daily quota reached for {}.", key));
        }
    }
    Ok(())
}

fn record_quota(store: &DashMap<String, DailyCounter>, key: &str) {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let next_count = store
        .get(key)
        .map(|entry| {
            if entry.day == today {
                entry.count + 1
            } else {
                1
            }
        })
        .unwrap_or(1);
    store.insert(
        key.to_string(),
        DailyCounter {
            day: today,
            count: next_count,
        },
    );
}

fn extract_client_ip(headers: &HeaderMap, peer_addr: SocketAddr) -> String {
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
}
