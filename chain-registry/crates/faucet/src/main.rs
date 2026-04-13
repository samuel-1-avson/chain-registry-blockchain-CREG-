// crates/faucet/src/main.rs
// Testnet Faucet Service - Distributes test tCREG tokens (REAL IMPLEMENTATION)
#![deny(clippy::unwrap_used)]

use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    sol,
};
use axum::{
    extract::{ConnectInfo, Json, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json as JsonResponse},
    routing::{get, post},
    Router,
};
use dashmap::DashMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, error};

sol!(
    #[sol(rpc)]
    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
        function balanceOf(address owner) external view returns (uint256);
    }
);

/// Faucet configuration
#[derive(Clone)]
struct FaucetConfig {
    /// Amount to distribute per request (in wei/tCREG smallest unit)
    drip_amount: u128,
    /// Amount of native ETH/testnet ETH to distribute per request (wei)
    native_drip_amount: u128,
    /// Cooldown between requests per address
    cooldown_secs: u64,
    /// Cooldown between requests per IP
    ip_cooldown_secs: u64,
    /// Maximum balance a single address can have (prevent hoarding)
    max_balance: u128,
    /// Maximum native balance a single address can have before gas drip stops
    native_max_balance: u128,
    /// Ethereum RPC URL
    rpc_url: String,
    /// Faucet private key (must have tokens to distribute)
    faucet_key: String,
    /// Test CREG token contract address
    token_contract: String,
    /// Faucet Ethereum address
    faucet_address: String,
}

impl FaucetConfig {
    fn from_env() -> Self {
        let faucet_key = std::env::var("FAUCET_PRIVATE_KEY")
            .expect("FAUCET_PRIVATE_KEY must be set");
        let faucet_address = std::env::var("FAUCET_ADDRESS")
            .expect("FAUCET_ADDRESS must be set");
        
        Self {
            drip_amount: env_u128("FAUCET_DRIP_AMOUNT", 1000_000_000_000_000_000_000), // 1000 tCREG
            native_drip_amount: env_u128("FAUCET_NATIVE_DRIP_AMOUNT", 100_000_000_000_000_000), // 0.1 ETH
            cooldown_secs: env_u64("FAUCET_COOLDOWN_SECS", 60),                        // 1 minute
            ip_cooldown_secs: env_u64("FAUCET_IP_COOLDOWN_SECS", 60),
            max_balance: env_u128("FAUCET_MAX_BALANCE", 10000_000_000_000_000_000_000), // 10k tCREG
            native_max_balance: env_u128("FAUCET_NATIVE_MAX_BALANCE", 1_000_000_000_000_000_000), // 1 ETH
            rpc_url: env_string("FAUCET_RPC_URL", "http://localhost:8545"),
            faucet_key,
            token_contract: std::env::var("FAUCET_TOKEN_CONTRACT")
                .expect("FAUCET_TOKEN_CONTRACT must be set"),
            faucet_address,
        }
    }
}

/// Rate limiter state
struct RateLimiter {
    /// Last request time per Ethereum address
    address_last_request: DashMap<String, Instant>,
    /// Last request time per IP
    ip_last_request: DashMap<String, Instant>,
}

struct CooldownRejection {
    message: String,
    retry_after_seconds: u64,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            address_last_request: DashMap::new(),
            ip_last_request: DashMap::new(),
        }
    }

    fn check_address(&self, address: &str, cooldown: Duration) -> Result<(), CooldownRejection> {
        let normalized = address.to_lowercase();
        if let Some(last) = self.address_last_request.get(&normalized) {
            let elapsed = last.elapsed();
            if elapsed < cooldown {
                let remaining = cooldown - elapsed;
                let retry_after_seconds = remaining.as_secs().max(1);
                return Err(CooldownRejection {
                    message: format!(
                        "Please wait {} seconds before requesting again",
                        retry_after_seconds
                    ),
                    retry_after_seconds,
                });
            }
        }
        Ok(())
    }

    fn check_ip(&self, ip: &str, cooldown: Duration) -> Result<(), CooldownRejection> {
        if let Some(last) = self.ip_last_request.get(ip) {
            let elapsed = last.elapsed();
            if elapsed < cooldown {
                let remaining = cooldown - elapsed;
                let retry_after_seconds = remaining.as_secs().max(1);
                return Err(CooldownRejection {
                    message: format!(
                        "IP rate limit: wait {} seconds",
                        retry_after_seconds
                    ),
                    retry_after_seconds,
                });
            }
        }
        Ok(())
    }

    fn record_request(&self, address: &str, ip: &str) {
        self.address_last_request
            .insert(address.to_lowercase(), Instant::now());
        self.ip_last_request.insert(ip.to_string(), Instant::now());
    }
}

/// Application state
struct AppState {
    config: FaucetConfig,
    rate_limiter: RateLimiter,
    /// Active PoW challenges keyed by challenge string.
    pow_challenges: DashMap<String, PowChallenge>,
    /// Faucet statistics
    stats: Mutex<FaucetStats>,
}

/// A proof-of-work challenge issued to clients.
#[derive(Clone)]
struct PowChallenge {
    difficulty: u8,
    created_at: Instant,
}

/// PoW difficulty — number of leading zero bits required in SHA-256(challenge || nonce).
/// 20 bits ≈ 1M hashes ≈ ~1 second on a modern browser.
const POW_DIFFICULTY: u8 = 20;
/// Challenge validity window.
const POW_TTL: Duration = Duration::from_secs(120);

#[derive(Default, Serialize)]
struct FaucetStats {
    total_drips: u64,
    total_distributed: String,
    total_native_distributed: String,
    unique_addresses: usize,
    last_drip: Option<DateTime<Utc>>,
}

/// Request to drip tokens
#[derive(Deserialize)]
struct DripRequest {
    address: String,
    /// The PoW challenge string returned by /api/challenge.
    challenge: Option<String>,
    /// The nonce the client found such that SHA256(challenge||nonce) has N leading zero bits.
    nonce: Option<String>,
}

/// PoW challenge response
#[derive(Serialize)]
struct ChallengeResponse {
    challenge: String,
    difficulty: u8,
    ttl_secs: u64,
}

/// Drip response
#[derive(Serialize)]
struct DripResponse {
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cooldown_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_amount: Option<String>,
}

impl DripResponse {
    fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            tx_hash: None,
            amount: None,
            retry_after_seconds: None,
            cooldown_seconds: None,
            token_tx_hash: None,
            native_tx_hash: None,
            token_amount: None,
            native_amount: None,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = FaucetConfig::from_env();

    info!("╔════════════════════════════════════════════════════════╗");
    info!("║        Chain Registry Testnet Faucet (REAL)            ║");
    info!("╚════════════════════════════════════════════════════════╝");
    info!(
        "  Drip amount: {} tCREG",
        config.drip_amount / 10_u128.pow(18)
    );
    if config.native_drip_amount > 0 {
        info!(
            "  Gas drip amount: {:.4} ETH",
            config.native_drip_amount as f64 / 10_f64.powi(18)
        );
    }
    info!("  Cooldown: {} seconds", config.cooldown_secs);
    info!("  Token contract: {}", config.token_contract);
    info!("  RPC: {}", config.rpc_url);
    info!("  Faucet address: {}", config.faucet_address);

    let state = Arc::new(AppState {
        config,
        rate_limiter: RateLimiter::new(),
        pow_challenges: DashMap::new(),
        stats: Mutex::new(FaucetStats::default()),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(index_page))
        .route("/favicon.ico", get(favicon))
        .route("/api/challenge", get(get_challenge))
        .route("/api/drip", post(handle_drip))
        .route("/api/stats", get(get_stats))
        .route("/api/balance/:address", get(get_balance))
        .route("/api/network", get(get_network_info))
        .route("/health", get(health_check))
        .layer(cors)
        .with_state(state);

    let port = env_u16("FAUCET_PORT", 8082);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // ── Optional TLS ──────────────────────────────────────────────────────────
    #[cfg(feature = "tls")]
    {
        let tls_cert = std::env::var("FAUCET_TLS_CERT").ok();
        let tls_key = std::env::var("FAUCET_TLS_KEY").ok();

        if let (Some(cert_path), Some(key_path)) = (tls_cert, tls_key) {
            use axum_server::tls_rustls::RustlsConfig;

            let tls_config =
                RustlsConfig::from_pem_file(&cert_path, &key_path)
                    .await
                    .expect("Failed to load TLS certificate/key");

            info!("Faucet listening on https://{}", addr);

            axum_server::bind_rustls(addr, tls_config)
                .serve(app.into_make_service())
                .await?;

            return Ok(());
        }
    }

    info!("Faucet listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;

    Ok(())
}

/// HTML faucet page
async fn index_page() -> impl IntoResponse {
    Html(include_str!("faucet.html"))
}

/// Small inline favicon so browsers do not fall back to a missing default asset.
async fn favicon() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>💧</text></svg>",
    )
}

fn parse_address(value: &str, field_name: &str) -> Result<Address, String> {
    value
        .parse::<Address>()
        .map_err(|e| format!("Invalid {}: {}", field_name, e))
}

async fn execute_token_transfer(config: &FaucetConfig, to_address: &str) -> Result<String, String> {
    let signer: PrivateKeySigner = config
        .faucet_key
        .parse()
        .map_err(|e| format!("Invalid faucet private key: {}", e))?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(
            config
                .rpc_url
                .parse()
                .map_err(|e| format!("Invalid faucet RPC URL: {}", e))?,
        );

    let token_address = parse_address(&config.token_contract, "token contract")?;
    let recipient = parse_address(to_address, "recipient address")?;
    let contract = IERC20::new(token_address, &provider);

    let pending_tx = contract
        .transfer(recipient, U256::from(config.drip_amount))
        .send()
        .await
        .map_err(|e| format!("Transfer failed: {}", e))?;
    let tx_hash = pending_tx.tx_hash().to_string();

    pending_tx
        .watch()
        .await
        .map_err(|e| format!("Transfer confirmation failed: {}", e))?;

    Ok(tx_hash)
}

async fn execute_native_transfer(config: &FaucetConfig, to_address: &str) -> Result<String, String> {
    let signer: PrivateKeySigner = config
        .faucet_key
        .parse()
        .map_err(|e| format!("Invalid faucet private key: {}", e))?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(
            config
                .rpc_url
                .parse()
                .map_err(|e| format!("Invalid faucet RPC URL: {}", e))?,
        );

    let recipient = parse_address(to_address, "recipient address")?;
    let tx = TransactionRequest::default()
        .to(recipient)
        .value(U256::from(config.native_drip_amount));

    let pending_tx = provider
        .send_transaction(tx)
        .await
        .map_err(|e| format!("Native ETH transfer failed: {}", e))?;
    let tx_hash = pending_tx.tx_hash().to_string();

    pending_tx
        .watch()
        .await
        .map_err(|e| format!("Native ETH confirmation failed: {}", e))?;

    Ok(tx_hash)
}

async fn get_token_balance(config: &FaucetConfig, address: &str) -> Result<u128, String> {
    let holder = parse_address(address, "holder address")?;
    let holder_hex = holder.to_string().trim_start_matches("0x").to_ascii_lowercase();
    let call_data = format!("0x70a08231{:0>64}", holder_hex);

    let response: serde_json::Value = reqwest::Client::new()
        .post(&config.rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [
                {
                    "to": config.token_contract,
                    "data": call_data,
                },
                "latest"
            ],
            "id": 1
        }))
        .send()
        .await
        .map_err(|e| format!("Balance check failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Balance response decode failed: {}", e))?;

    if let Some(err) = response.get("error") {
        return Err(format!("Balance check failed: {}", err));
    }

    let result = response
        .get("result")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "Balance check failed: missing result".to_string())?;

    u128::from_str_radix(result.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Failed to parse balance: {}", e))
}

async fn get_native_balance(config: &FaucetConfig, address: &str) -> Result<u128, String> {
    let response: serde_json::Value = reqwest::Client::new()
        .post(&config.rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getBalance",
            "params": [address, "latest"],
            "id": 1
        }))
        .send()
        .await
        .map_err(|e| format!("Native balance check failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Native balance response decode failed: {}", e))?;

    if let Some(err) = response.get("error") {
        return Err(format!("Native balance check failed: {}", err));
    }

    let result = response
        .get("result")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "Native balance check failed: missing result".to_string())?;

    u128::from_str_radix(result.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Failed to parse native balance: {}", e))
}

/// Issue a proof-of-work challenge. Client must find a nonce such that
/// SHA-256(challenge || nonce) has `difficulty` leading zero bits.
async fn get_challenge(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let challenge = hex::encode(bytes);

    // Prune expired challenges periodically.
    state.pow_challenges.retain(|_, v| v.created_at.elapsed() < POW_TTL);

    state.pow_challenges.insert(
        challenge.clone(),
        PowChallenge {
            difficulty: POW_DIFFICULTY,
            created_at: Instant::now(),
        },
    );

    (
        StatusCode::OK,
        JsonResponse(ChallengeResponse {
            challenge,
            difficulty: POW_DIFFICULTY,
            ttl_secs: POW_TTL.as_secs(),
        }),
    )
}

/// Verify proof-of-work: SHA-256(challenge || nonce) must have `difficulty` leading zero bits.
fn verify_pow(challenge: &str, nonce: &str, difficulty: u8) -> bool {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(challenge.as_bytes());
    hasher.update(nonce.as_bytes());
    let hash = hasher.finalize();

    // Count leading zero bits.
    let mut leading_zeros = 0u8;
    for byte in hash.iter() {
        if *byte == 0 {
            leading_zeros += 8;
        } else {
            leading_zeros += byte.leading_zeros() as u8;
            break;
        }
        if leading_zeros >= difficulty {
            break;
        }
    }
    leading_zeros >= difficulty
}

/// Handle drip request
async fn handle_drip(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<DripRequest>,
) -> impl IntoResponse {
    let address = request.address.to_lowercase();

    // ── PoW validation ────────────────────────────────────────────────────────
    let pow_enabled = std::env::var("FAUCET_POW_DISABLED").unwrap_or_default() != "true";
    if pow_enabled {
        let challenge = match &request.challenge {
            Some(c) => c.clone(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    JsonResponse(DripResponse::error("Missing proof-of-work challenge. Call GET /api/challenge first.")),
                );
            }
        };

        let nonce = match &request.nonce {
            Some(n) => n.clone(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    JsonResponse(DripResponse::error("Missing proof-of-work nonce.")),
                );
            }
        };

        // Look up and consume the challenge (single-use).
        let pow_entry = state.pow_challenges.remove(&challenge);
        match pow_entry {
            Some((_, pc)) if pc.created_at.elapsed() < POW_TTL => {
                if !verify_pow(&challenge, &nonce, pc.difficulty) {
                    return (
                        StatusCode::BAD_REQUEST,
                        JsonResponse(DripResponse::error("Invalid proof-of-work solution.")),
                    );
                }
            }
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    JsonResponse(DripResponse::error("Unknown or expired challenge. Request a new one.")),
                );
            }
        }
    }

    // Validate address format
    if !address.starts_with("0x") || address.len() != 42 {
        return (
            StatusCode::BAD_REQUEST,
            JsonResponse(DripResponse::error("Invalid Ethereum address format")),
        );
    }

    // Extract client IP from X-Forwarded-For / X-Real-IP headers, fallback to socket peer
    let client_ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| peer_addr.ip().to_string());

    // Check rate limits
    let cooldown = Duration::from_secs(state.config.cooldown_secs);
    if let Err(rejection) = state.rate_limiter.check_address(&address, cooldown) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            JsonResponse(DripResponse {
                retry_after_seconds: Some(rejection.retry_after_seconds),
                cooldown_seconds: Some(state.config.cooldown_secs),
                ..DripResponse::error(rejection.message)
            }),
        );
    }

    let ip_cooldown = Duration::from_secs(state.config.ip_cooldown_secs);
    if let Err(rejection) = state.rate_limiter.check_ip(&client_ip, ip_cooldown) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            JsonResponse(DripResponse {
                retry_after_seconds: Some(rejection.retry_after_seconds),
                cooldown_seconds: Some(state.config.ip_cooldown_secs),
                ..DripResponse::error(rejection.message)
            }),
        );
    }

    let token_balance = get_token_balance(&state.config, &address).await.ok();
    let native_balance = if state.config.native_drip_amount > 0 {
        get_native_balance(&state.config, &address).await.ok()
    } else {
        None
    };

    let should_send_token = match token_balance {
        Some(balance) => balance < state.config.max_balance,
        None => true,
    };
    let should_send_native = if state.config.native_drip_amount == 0 {
        false
    } else {
        match native_balance {
            Some(balance) => balance < state.config.native_max_balance,
            None => true,
        }
    };

    if !should_send_token && !should_send_native {
        let token_msg = token_balance
            .map(|balance| format!("{} tCREG", balance / 10_u128.pow(18)))
            .unwrap_or_else(|| "sufficient tCREG".to_string());
        let native_msg = native_balance
            .map(|balance| format!("{:.4} ETH", balance as f64 / 10_f64.powi(18)))
            .unwrap_or_else(|| "sufficient testnet ETH".to_string());
        return (
            StatusCode::BAD_REQUEST,
            JsonResponse(DripResponse {
                success: false,
                message: format!(
                    "Address already has enough test funds for now ({}, {}).",
                    token_msg,
                    native_msg
                ),
                tx_hash: None,
                amount: None,
                retry_after_seconds: None,
                cooldown_seconds: None,
                token_tx_hash: None,
                native_tx_hash: None,
                token_amount: None,
                native_amount: None,
            }),
        );
    }

    let mut token_tx_hash = None;
    let mut native_tx_hash = None;
    let mut parts = Vec::new();
    let mut failures = Vec::new();

    if should_send_native {
        match execute_native_transfer(&state.config, &address).await {
            Ok(tx_hash) => {
                parts.push(format!(
                    "{:.4} ETH for gas",
                    state.config.native_drip_amount as f64 / 10_f64.powi(18)
                ));
                native_tx_hash = Some(tx_hash);
            }
            Err(err) => {
                error!("Native gas drip failed: {}", err);
                failures.push(format!("native ETH: {}", err));
            }
        }
    }

    if should_send_token {
        match execute_token_transfer(&state.config, &address).await {
            Ok(tx_hash) => {
                parts.push(format!("{} tCREG", state.config.drip_amount / 10_u128.pow(18)));
                token_tx_hash = Some(tx_hash);
            }
            Err(err) => {
                error!("Token drip failed: {}", err);
                failures.push(format!("tCREG: {}", err));
            }
        }
    }

    if token_tx_hash.is_some() || native_tx_hash.is_some() {
            state.rate_limiter.record_request(&address, &client_ip);

            // Update stats
            let mut stats = state.stats.lock().await;
            stats.total_drips += 1;
            stats.unique_addresses = state.rate_limiter.address_last_request.len();
            let current_token_total = stats.total_distributed.parse::<u128>().unwrap_or_default();
            let current_native_total = stats.total_native_distributed.parse::<u128>().unwrap_or_default();
            stats.total_distributed = (current_token_total
                + if token_tx_hash.is_some() { state.config.drip_amount } else { 0 })
                .to_string();
            stats.total_native_distributed = (current_native_total
                + if native_tx_hash.is_some() { state.config.native_drip_amount } else { 0 })
                .to_string();
            stats.last_drip = Some(Utc::now());
            drop(stats);

            info!(
                "Dripped {} to {}{}{}",
                parts.join(" + "),
                address,
                token_tx_hash
                    .as_ref()
                    .map(|tx| format!(" (token tx: {})", tx))
                    .unwrap_or_default(),
                native_tx_hash
                    .as_ref()
                    .map(|tx| format!(" (gas tx: {})", tx))
                    .unwrap_or_default()
            );

            (
                StatusCode::OK,
                JsonResponse(DripResponse {
                    success: true,
                    message: if failures.is_empty() {
                        format!("Sent {}.", parts.join(" + "))
                    } else {
                        format!("Sent {}. Partial issue: {}", parts.join(" + "), failures.join("; "))
                    },
                    tx_hash: token_tx_hash.clone().or_else(|| native_tx_hash.clone()),
                    amount: Some(parts.join(" + ")),
                    retry_after_seconds: None,
                    cooldown_seconds: Some(state.config.cooldown_secs),
                    token_tx_hash,
                    native_tx_hash,
                    token_amount: if should_send_token {
                        Some(format!("{}", state.config.drip_amount / 10_u128.pow(18)))
                    } else {
                        None
                    },
                    native_amount: if should_send_native {
                        Some(format!("{:.4}", state.config.native_drip_amount as f64 / 10_f64.powi(18)))
                    } else {
                        None
                    },
                }),
            )
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(DripResponse {
                    success: false,
                    message: if failures.is_empty() {
                        "Faucet transfer failed for an unknown reason.".to_string()
                    } else {
                        format!("Faucet transfer failed: {}", failures.join("; "))
                    },
                    tx_hash: None,
                    amount: None,
                    retry_after_seconds: None,
                    cooldown_seconds: None,
                    token_tx_hash: None,
                    native_tx_hash: None,
                    token_amount: None,
                    native_amount: None,
                }),
            )
        }
}

/// Get faucet statistics
async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let stats = state.stats.lock().await;
    
    // Get real faucet balance
    let faucet_balance = get_token_balance(&state.config, &state.config.faucet_address)
        .await
        .unwrap_or(0);
    let faucet_native_balance = get_native_balance(&state.config, &state.config.faucet_address)
        .await
        .unwrap_or(0);
    
    JsonResponse(serde_json::json!({
        "drip_amount": state.config.drip_amount.to_string(),
        "native_drip_amount": state.config.native_drip_amount.to_string(),
        "cooldown_seconds": state.config.cooldown_secs,
        "max_balance": state.config.max_balance.to_string(),
        "native_max_balance": state.config.native_max_balance.to_string(),
        "token_contract": state.config.token_contract,
        "faucet_address": state.config.faucet_address,
        "faucet_balance": faucet_balance.to_string(),
        "faucet_balance_formatted": format!("{:.2}", faucet_balance as f64 / 10_f64.powi(18)),
        "faucet_native_balance": faucet_native_balance.to_string(),
        "faucet_native_balance_formatted": format!("{:.4}", faucet_native_balance as f64 / 10_f64.powi(18)),
        "stats": *stats,
    }))
}

/// Get balance for address (REAL)
async fn get_balance(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(address): axum::extract::Path<String>,
) -> impl IntoResponse {
    let token_balance = get_token_balance(&state.config, &address).await;
    let native_balance = get_native_balance(&state.config, &address).await;

    match (token_balance, native_balance) {
        (Ok(balance), Ok(native)) => {
            JsonResponse(serde_json::json!({
                "address": address,
                "balance": balance.to_string(),
                "balance_formatted": format!("{:.2}", balance as f64 / 10_f64.powi(18)),
                "token_balance": balance.to_string(),
                "token_balance_formatted": format!("{:.2}", balance as f64 / 10_f64.powi(18)),
                "native_balance": native.to_string(),
                "native_balance_formatted": format!("{:.4}", native as f64 / 10_f64.powi(18)),
            }))
        }
        (token_result, native_result) => {
            JsonResponse(serde_json::json!({
                "address": address,
                "error": format!(
                    "token={}, native={}",
                    token_result.err().unwrap_or_else(|| "ok".to_string()),
                    native_result.err().unwrap_or_else(|| "ok".to_string())
                ),
            }))
        }
    }
}

/// Network configuration info for wallet setup and next-step guidance
async fn get_network_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let explorer_url = env_string("FAUCET_EXPLORER_URL", "http://localhost:3000");
    let rpc_url = env_string("FAUCET_PUBLIC_RPC_URL", "http://localhost:8545");
    let chain_id = env_u64("FAUCET_CHAIN_ID", 31337);

    JsonResponse(serde_json::json!({
        "chain_id": chain_id,
        "rpc_url": rpc_url,
        "token_contract": state.config.token_contract,
        "explorer_url": explorer_url,
        "chain_name": "CREG Testnet (Anvil)",
        "currency": "ETH",
        "token_symbol": "tCREG",
        "native_currency_symbol": "ETH",
        "gas_note": "Gas on EVM testnets is paid in the native testnet ETH for that chain, not in ERC-20 tokens.",
    }))
}

/// Health check
async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match (
        get_token_balance(&state.config, &state.config.faucet_address).await,
        get_native_balance(&state.config, &state.config.faucet_address).await,
    ) {
        (Ok(faucet_balance), Ok(faucet_native_balance)) => (
            StatusCode::OK,
            JsonResponse(serde_json::json!({
                "status": "healthy",
                "faucet": "online",
                "mode": "real",
                "faucet_balance": faucet_balance.to_string(),
                "faucet_native_balance": faucet_native_balance.to_string(),
            })),
        ),
        (token_result, native_result) => (
            StatusCode::SERVICE_UNAVAILABLE,
            JsonResponse(serde_json::json!({
                "status": "degraded",
                "faucet": "offline",
                "mode": "real",
                "error": format!(
                    "token={}, native={}",
                    token_result.err().unwrap_or_else(|| "ok".to_string()),
                    native_result.err().unwrap_or_else(|| "ok".to_string())
                ),
            })),
        ),
    }
}

// Helper functions
fn env_string(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u128(key: &str, default: u128) -> u128 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
