// crates/faucet/src/main.rs
// Testnet Faucet Service - Distributes test tCREG tokens (REAL IMPLEMENTATION)

use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::ProviderBuilder,
    signers::local::PrivateKeySigner,
    sol,
};
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json as JsonResponse},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
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
    /// Cooldown between requests per address
    cooldown_secs: u64,
    /// Cooldown between requests per IP
    ip_cooldown_secs: u64,
    /// Maximum balance a single address can have (prevent hoarding)
    max_balance: u128,
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
            cooldown_secs: env_u64("FAUCET_COOLDOWN_SECS", 60),                        // 1 minute
            ip_cooldown_secs: env_u64("FAUCET_IP_COOLDOWN_SECS", 60),
            max_balance: env_u128("FAUCET_MAX_BALANCE", 10000_000_000_000_000_000_000), // 10k tCREG
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

impl RateLimiter {
    fn new() -> Self {
        Self {
            address_last_request: DashMap::new(),
            ip_last_request: DashMap::new(),
        }
    }

    fn check_address(&self, address: &str, cooldown: Duration) -> Result<(), String> {
        if let Some(last) = self.address_last_request.get(address) {
            let elapsed = last.elapsed();
            if elapsed < cooldown {
                let remaining = cooldown - elapsed;
                return Err(format!(
                    "Please wait {} seconds before requesting again",
                    remaining.as_secs()
                ));
            }
        }
        Ok(())
    }

    fn check_ip(&self, ip: &str, cooldown: Duration) -> Result<(), String> {
        if let Some(last) = self.ip_last_request.get(ip) {
            let elapsed = last.elapsed();
            if elapsed < cooldown {
                let remaining = cooldown - elapsed;
                return Err(format!(
                    "IP rate limit: wait {} seconds",
                    remaining.as_secs()
                ));
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
    /// Faucet statistics
    stats: Mutex<FaucetStats>,
}

#[derive(Default, Serialize)]
struct FaucetStats {
    total_drips: u64,
    total_distributed: String,
    unique_addresses: usize,
    last_drip: Option<DateTime<Utc>>,
}

/// Request to drip tokens
#[derive(Deserialize)]
struct DripRequest {
    address: String,
    /// Optional: human verification token (future use)
    #[allow(dead_code)]
    captcha: Option<String>,
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
    info!("  Cooldown: {} seconds", config.cooldown_secs);
    info!("  Token contract: {}", config.token_contract);
    info!("  RPC: {}", config.rpc_url);
    info!("  Faucet address: {}", config.faucet_address);

    let state = Arc::new(AppState {
        config,
        rate_limiter: RateLimiter::new(),
        stats: Mutex::new(FaucetStats::default()),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(index_page))
        .route("/api/drip", post(handle_drip))
        .route("/api/stats", get(get_stats))
        .route("/api/balance/:address", get(get_balance))
        .route("/health", get(health_check))
        .layer(cors)
        .with_state(state);

    let port = env_u16("FAUCET_PORT", 8081);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    info!("Faucet listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// HTML faucet page
async fn index_page() -> impl IntoResponse {
    Html(include_str!("faucet.html"))
}

fn parse_address(value: &str, field_name: &str) -> Result<Address, String> {
    value
        .parse::<Address>()
        .map_err(|e| format!("Invalid {}: {}", field_name, e))
}

async fn execute_transfer(config: &FaucetConfig, to_address: &str) -> Result<String, String> {
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

async fn get_real_balance(config: &FaucetConfig, address: &str) -> Result<u128, String> {
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
    let holder = parse_address(address, "holder address")?;
    let contract = IERC20::new(token_address, &provider);
    let balance = contract
        .balanceOf(holder)
        .call()
        .await
        .map_err(|e| format!("Balance check failed: {}", e))?
        ._0;

    balance
        .to_string()
        .parse::<u128>()
        .map_err(|e| format!("Failed to parse balance: {}", e))
}

/// Handle drip request
async fn handle_drip(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DripRequest>,
) -> impl IntoResponse {
    let address = request.address.to_lowercase();

    // Validate address format
    if !address.starts_with("0x") || address.len() != 42 {
        return (
            StatusCode::BAD_REQUEST,
            JsonResponse(DripResponse {
                success: false,
                message: "Invalid Ethereum address format".to_string(),
                tx_hash: None,
                amount: None,
            }),
        );
    }

    // Get client IP (in production, extract from headers)
    let client_ip = "0.0.0.0".to_string();

    // Check rate limits
    let cooldown = Duration::from_secs(state.config.cooldown_secs);
    if let Err(msg) = state.rate_limiter.check_address(&address, cooldown) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            JsonResponse(DripResponse {
                success: false,
                message: msg,
                tx_hash: None,
                amount: None,
            }),
        );
    }

    let ip_cooldown = Duration::from_secs(state.config.ip_cooldown_secs);
    if let Err(msg) = state.rate_limiter.check_ip(&client_ip, ip_cooldown) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            JsonResponse(DripResponse {
                success: false,
                message: msg,
                tx_hash: None,
                amount: None,
            }),
        );
    }

    // Check current balance
    match get_real_balance(&state.config, &address).await {
        Ok(balance) => {
            if balance >= state.config.max_balance {
                return (
                    StatusCode::BAD_REQUEST,
                    JsonResponse(DripResponse {
                        success: false,
                        message: format!(
                            "Address already has maximum allowed balance ({} tCREG)",
                            balance / 10_u128.pow(18)
                        ),
                        tx_hash: None,
                        amount: None,
                    }),
                );
            }
        }
        Err(e) => {
            error!("Failed to check balance: {}", e);
            // Continue anyway, the transfer will fail if there's a real issue
        }
    }

    // Execute real token transfer
    match execute_transfer(&state.config, &address).await {
        Ok(tx_hash) => {
            state.rate_limiter.record_request(&address, &client_ip);

            // Update stats
            let mut stats = state.stats.lock().await;
            stats.total_drips += 1;
            stats.unique_addresses = state.rate_limiter.address_last_request.len();
            stats.total_distributed = format!(
                "{}",
                (stats.total_drips as u128 * state.config.drip_amount) / 10_u128.pow(18)
            );
            stats.last_drip = Some(Utc::now());
            drop(stats);

            info!(
                "Dripped {} tCREG to {} (tx: {})",
                state.config.drip_amount / 10_u128.pow(18),
                address,
                tx_hash
            );

            (
                StatusCode::OK,
                JsonResponse(DripResponse {
                    success: true,
                    message: "Tokens sent successfully!".to_string(),
                    tx_hash: Some(tx_hash),
                    amount: Some(format!("{}", state.config.drip_amount / 10_u128.pow(18))),
                }),
            )
        }
        Err(e) => {
            error!("Transfer failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(DripResponse {
                    success: false,
                    message: format!("Transfer failed: {}", e),
                    tx_hash: None,
                    amount: None,
                }),
            )
        }
    }
}

/// Get faucet statistics
async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let stats = state.stats.lock().await;
    
    // Get real faucet balance
    let faucet_balance = get_real_balance(&state.config, &state.config.faucet_address)
        .await
        .unwrap_or(0);
    
    JsonResponse(serde_json::json!({
        "drip_amount": state.config.drip_amount.to_string(),
        "cooldown_seconds": state.config.cooldown_secs,
        "token_contract": state.config.token_contract,
        "faucet_address": state.config.faucet_address,
        "faucet_balance": faucet_balance.to_string(),
        "faucet_balance_formatted": format!("{:.2}", faucet_balance as f64 / 10_f64.powi(18)),
        "stats": *stats,
    }))
}

/// Get balance for address (REAL)
async fn get_balance(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(address): axum::extract::Path<String>,
) -> impl IntoResponse {
    match get_real_balance(&state.config, &address).await {
        Ok(balance) => {
            JsonResponse(serde_json::json!({
                "address": address,
                "balance": balance.to_string(),
                "balance_formatted": format!("{:.2}", balance as f64 / 10_f64.powi(18)),
            }))
        }
        Err(e) => {
            JsonResponse(serde_json::json!({
                "address": address,
                "error": e,
            }))
        }
    }
}

/// Health check
async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match get_real_balance(&state.config, &state.config.faucet_address).await {
        Ok(faucet_balance) => (
            StatusCode::OK,
            JsonResponse(serde_json::json!({
                "status": "healthy",
                "faucet": "online",
                "mode": "real",
                "faucet_balance": faucet_balance.to_string(),
            })),
        ),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            JsonResponse(serde_json::json!({
                "status": "degraded",
                "faucet": "offline",
                "mode": "real",
                "error": err,
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
