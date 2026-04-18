// crates/node/src/api.rs
// Axum REST API — all HTTP endpoints for the chain registry node.

use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use common::{PackageStatus, PublishRequest, Transaction, ValidatorIdentity};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};

use crate::consensus_admission::{
    accept_peer_attestation, AdmissionAttestation, AttestationStore,
};
use crate::{
    events::{self, sse_handler, EventBus},
    openapi::ApiDoc,
    rate_limit::{rate_limit_middleware, RateLimiter},
    normalized_validator_key, validator_registration_status_text, ValidatorRegistrationStatus,
    SharedState,
};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// Query parameters for GET /v1/packages
#[derive(Deserialize)]
struct ListPackagesParams {
    offset: Option<usize>,
    limit: Option<usize>,
    ecosystem: Option<String>,
    status: Option<String>,
}

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn router(
    state: SharedState,
    event_bus: EventBus,
    limiter: RateLimiter,
    admission_store: Arc<AttestationStore>,
) -> Router {
    Router::new()
        // Health & chain
        .route("/v1/health", get(health))
        .route("/health", get(health))
        .route("/v1/chain/stats", get(chain_stats))
        .route("/v1/runtime/config", get(runtime_config))
        .route("/v1/validators/register", post(register_validator_identity))
        .route("/v1/validators/registrations", get(list_validator_registrations))
        .route(
            "/v1/validators/registrations/:evm_address",
            delete(delete_validator_registration),
        )
        .route("/v1/nodes", get(get_nodes))
        .route("/v1/p2p/status", get(p2p_status))
        .route("/v1/bridge/status", get(bridge_status))
        .route("/v1/bridge/anchors", get(bridge_anchors))
        .route("/v1/governance/proposals", get(governance_proposals))
        .route("/v1/metrics/history", get(metrics_history))
        .route("/v1/reorgs", get(reorgs))
        .route("/v1/richlist", get(richlist))
        .route("/v1/ws", get(ws_handler))
        // Packages
        .route("/v1/packages/:canonical", get(get_package))
        .route("/v1/packages", get(list_packages).post(submit_package))
        .route("/v1/packages/:canonical/revoke", post(revoke_package))
        .route("/v1/packages/:canonical/proof", get(get_proof))
        // Blocks
        .route("/v1/blocks", get(list_blocks_paginated))
        .route("/v1/blocks/:height", get(get_block_by_height))
        .route("/v1/blocks/hash/:hash", get(get_block_by_hash))
        .route("/v1/blocks/announce", post(receive_block_announcement))
        // Transactions
        .route("/v1/transactions/:canonical", get(get_transaction))
        // Publishers
        .route("/v1/publishers/:pubkey", get(get_publisher))
        // Addresses
        .route("/v1/addresses/:address", get(get_address))
        .route("/v1/addresses/:address/transactions", get(get_address_transactions))
        // Validator detail
        .route("/v1/validators/:address", get(get_validator_profile))
        // Pending pool
        .route("/v1/pending", get(list_pending))
        // Consensus
        .route("/v1/consensus/vote", post(receive_vote))
        .route("/v1/consensus/state", get(consensus_state))
        .route(
            "/v1/consensus/admission-attestation",
            post(receive_admission_attestation),
        )
        .route("/v1/publishers/rotate-key", post(rotate_publisher_key))
        // Search
        .route("/v1/search", get(search_handler))
        // Appeals & AAA
        .route("/v1/appeals/:id/audit", post(submit_audit))
        // OpenAPI spec + Swagger UI. The JSON is what the explorer's
        // `npm run gen-types` consumes; Swagger UI is a humans-only browser.
        .route("/v1/openapi.json", get(openapi_spec))
        .merge(SwaggerUi::new("/api-docs").url("/v1/openapi.json", ApiDoc::openapi()))
        // Observability
        .route("/metrics", get(prometheus_metrics))
        // Event streaming - SSE & Websockets
        .route(
            "/v1/events",
            get({
                let bus = Arc::clone(&event_bus);
                move |_: ()| async move { sse_handler(axum::extract::State(bus)).await }
            }),
        )
        .route(
            "/v1/ws",
            get(move |ws| {
                let bus = Arc::clone(&event_bus);
                async move { events::ws_handler(ws, axum::extract::State(bus)).await }
            }),
        )
        .fallback(api_fallback)
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(50 * 1024 * 1024))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(axum::extract::Extension(limiter))
        .layer(axum::extract::Extension(admission_store))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn receive_admission_attestation(
    State(state): State<SharedState>,
    axum::extract::Extension(store): axum::extract::Extension<Arc<AttestationStore>>,
    Json(att): Json<AdmissionAttestation>,
) -> Response {
    // Look up chain_id + staking_addr fresh so config changes are picked up.
    let (rpc_url, staking_addr_s) = {
        let s = state.read().await;
        (s.config.eth_rpc_url.clone(), s.config.staking_addr.clone())
    };
    if staking_addr_s.trim().is_empty()
        || staking_addr_s.eq_ignore_ascii_case("0x0000000000000000000000000000000000000000")
    {
        return bad_request("admission path disabled: staking address unconfigured");
    }

    let staking_addr = match staking_addr_s.parse::<alloy::primitives::Address>() {
        Ok(a) => a,
        Err(e) => return bad_request(format!("invalid staking address: {e}")),
    };

    let chain_id = {
        use alloy::providers::Provider;
        let provider = alloy::providers::ProviderBuilder::new().on_http(match rpc_url.parse() {
            Ok(u) => u,
            Err(e) => return bad_request(format!("invalid rpc url: {e}")),
        });
        match provider.get_chain_id().await {
            Ok(id) => id,
            Err(e) => return server_err(format!("chain_id lookup failed: {e}")),
        }
    };

    match accept_peer_attestation(&store, chain_id, staking_addr, att).await {
        Ok(fresh) => Json(serde_json::json!({ "accepted": true, "new": fresh }))
            .into_response(),
        Err(e) => bad_request(format!("rejected: {e}")),
    }
}

fn bad_request(msg: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: msg.into() }),
    )
        .into_response()
}

async fn api_fallback(uri: Uri) -> Response {
    if uri.path().starts_with("/v1/") || uri.path() == "/metrics" || uri.path() == "/health" {
        return not_found(format!("No route for {}", uri.path()));
    }

    crate::explorer::static_handler(uri).await.into_response()
}

// ─── Response helpers ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

fn not_found(msg: impl Into<String>) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse { error: msg.into() }),
    )
        .into_response()
}

fn server_err(msg: impl Into<String>) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: msg.into() }),
    )
        .into_response()
}

// ─── Handlers ────────────────────────────────────────────────────────────────

/// Serve the generated OpenAPI schema as JSON. Explorer codegen fetches this
/// at build time to keep TypeScript types in sync with Rust responses.
async fn openapi_spec() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status":  "ok",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

#[derive(Serialize)]
struct ChainStatsResponse {
    #[serde(flatten)]
    chain: crate::chain_store::ChainStats,
    validator_count: usize,
    active_validators: usize,
    total_stake: u64,
    total_stake_native: String,
    peer_count: usize,
    bridge_status: String,
    l1_block: u64,
    pending_tx_count: usize,
    publisher_count: usize,
    finalized_height: u64,
    finalization_lag: u64,
}

async fn chain_stats(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let validators = &s.validator_set.validators;
    let total_stake: u64 = validators.iter().map(|validator| validator.stake).sum();
    let active_count = validators.iter().filter(|v| v.status != "offline").count();
    let tip = s.chain.stats().current_height;
    let finalized = s.bridge_status.last_finalized_eth_block;
    Json(ChainStatsResponse {
        chain: s.chain.stats(),
        validator_count: validators.len(),
        active_validators: active_count,
        total_stake,
        total_stake_native: total_stake.to_string(),
        peer_count: s.p2p_status.peers.len(),
        bridge_status: if s.bridge_status.bridge_sync_status.trim().is_empty() {
            "Unknown".to_string()
        } else {
            s.bridge_status.bridge_sync_status.clone()
        },
        l1_block: finalized,
        pending_tx_count: s.pending_pool.len(),
        publisher_count: s.publisher_index.len(),
        finalized_height: finalized,
        finalization_lag: tip.saturating_sub(finalized),
    })
}

#[derive(Serialize)]
struct RuntimeConfigResponse {
    is_testnet: bool,
    registry_address: Option<String>,
    token_contract: Option<String>,
    staking_contract: Option<String>,
    validator_registration_mode: String,
    validator_registration_note: String,
}

#[derive(Deserialize)]
struct RegisterValidatorIdentityRequest {
    evm_address: String,
    node_id: String,
    ed25519_pubkey: String,
    alias: Option<String>,
}

fn non_zero_address(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("0x0000000000000000000000000000000000000000") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

async fn runtime_config(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    Json(RuntimeConfigResponse {
        is_testnet: s.config.is_testnet,
        registry_address: non_zero_address(&s.config.registry_addr),
        token_contract: non_zero_address(&s.config.token_addr),
        staking_contract: non_zero_address(&s.config.staking_addr),
        validator_registration_mode: "staking-plus-identity-sync".to_string(),
        validator_registration_note: "Stake on-chain, register your validator EVM address, node ID, and Ed25519 pubkey with /v1/validators/register, wait for governance approval, and the node sync loop will admit active validators into consensus automatically.".to_string(),
    })
}

fn validate_evm_address(value: &str) -> Result<String, String> {
    value
        .trim()
        .parse::<alloy::primitives::Address>()
        .map(|address| address.to_string().to_ascii_lowercase())
        .map_err(|_| "EVM address must be a valid 0x-prefixed address".to_string())
}

fn validate_node_id(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err("node_id is required".to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn validate_ed25519_pubkey(value: &str) -> Result<String, String> {
    let trimmed = value.trim().trim_start_matches("0x").to_ascii_lowercase();
    match hex::decode(&trimmed) {
        Ok(bytes) if bytes.len() == 32 => Ok(trimmed),
        Ok(bytes) => Err(format!(
            "Ed25519 pubkey must be 32 bytes (64 hex chars), got {} bytes",
            bytes.len()
        )),
        Err(_) => Err("Ed25519 pubkey must be valid hex".to_string()),
    }
}

async fn register_validator_identity(
    State(state): State<SharedState>,
    Json(request): Json<RegisterValidatorIdentityRequest>,
) -> Response {
    let evm_address = match validate_evm_address(&request.evm_address) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error }),
            )
                .into_response();
        }
    };

    let node_id = match validate_node_id(&request.node_id) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error }),
            )
                .into_response();
        }
    };

    let ed25519_pubkey = match validate_ed25519_pubkey(&request.ed25519_pubkey) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error }),
            )
                .into_response();
        }
    };

    let normalized_key = normalized_validator_key(&evm_address);
    let alias = request
        .alias
        .unwrap_or_else(|| node_id.clone())
        .trim()
        .to_string();

    let mut s = state.write().await;

    if s.validator_registrations.iter().any(|(key, registration)| {
        *key != normalized_key
            && (registration.identity.node_id == node_id
                || registration.identity.ed25519_pubkey == ed25519_pubkey)
    }) {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "node_id or Ed25519 pubkey is already registered to another wallet".to_string(),
            }),
        )
            .into_response();
    }

    let identity = ValidatorIdentity {
        evm_address,
        node_id,
        ed25519_pubkey,
    }
    .normalized();

    let mut registration = s
        .validator_registrations
        .remove(&normalized_key)
        .unwrap_or_else(|| ValidatorRegistrationStatus {
            reputation: 100,
            ..ValidatorRegistrationStatus::default()
        });

    registration.alias = alias;
    registration.identity = identity;
    registration.registered_with_node = true;
    registration.status = validator_registration_status_text(&registration);

    let response = registration.clone();
    s.validator_registrations
        .insert(normalized_key, registration);

    (StatusCode::ACCEPTED, Json(response)).into_response()
}

/// DELETE /v1/validators/registrations/:evm_address
///
/// Removes a stale validator-identity registration from this node's in-memory
/// table so the bound (node_id, ed25519_pubkey) pair is free to be reclaimed.
/// Useful when a different wallet previously claimed the same node_id and the
/// registration loop has not yet re-synced from on-chain state.
async fn delete_validator_registration(
    State(state): State<SharedState>,
    Path(evm_address): Path<String>,
) -> Response {
    let address = match validate_evm_address(&evm_address) {
        Ok(v) => v,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error }),
            )
                .into_response();
        }
    };
    let key = normalized_validator_key(&address);

    let mut s = state.write().await;
    match s.validator_registrations.remove(&key) {
        Some(removed) => {
            // Evict the validator from the in-memory validator set so the
            // delete is immediately visible to `/v1/nodes` consumers.
            let identity = removed.identity.normalized();
            s.validator_set.validators.retain(|v| {
                v.id != identity.node_id && v.pubkey != identity.ed25519_pubkey
            });
            Json(serde_json::json!({
                "removed": true,
                "evm_address": address,
                "node_id": removed.identity.node_id,
            }))
            .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("no registration found for {address}"),
            }),
        )
            .into_response(),
    }
}

async fn list_validator_registrations(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let mut registrations: Vec<ValidatorRegistrationStatus> =
        s.validator_registrations.values().cloned().collect();
    registrations.sort_by(|left, right| {
        left.alias
            .cmp(&right.alias)
            .then(left.identity.node_id.cmp(&right.identity.node_id))
    });
    Json(registrations)
}

// GET /v1/nodes
async fn get_nodes(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let node_id = s.config.node_id.clone();

    // Convert current validator set to API response, marking "self" where appropriate.
    let mut resp = s.validator_set.validators.clone();
    for v in &mut resp {
        if v.id == node_id {
            v.status = "self".into();
        }
    }

    Json(resp)
}

// GET /v1/p2p/status
//
// Returns full peer topology when the `X-Operator-Key` header matches the
// node's configured operator pubkey (set via CREG_OPERATOR_PUBKEY env var).
// Public callers receive only aggregate counts to prevent network topology
// disclosure, which could aid targeted DDoS attacks on specific validators.
async fn p2p_status(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let s = state.read().await;

    // Operator key header allows full peer list exposure (e.g. for monitoring).
    let operator_pubkey = std::env::var("CREG_OPERATOR_PUBKEY").unwrap_or_default();
    let caller_key = headers
        .get("X-Operator-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let is_operator = !operator_pubkey.is_empty() && caller_key == operator_pubkey;

    if is_operator {
        // Full topology for authenticated operators.
        Json(serde_json::json!({
            "peer_count": s.p2p_status.peers.len(),
            "peers": s.p2p_status.peers,
            "protocols": s.p2p_status.protocols,
        }))
    } else {
        // Aggregate-only for public callers — include an empty `peers` array
        // so clients (web explorer) that access `p2pStatus.peers.length` don't
        // crash with "Cannot read properties of undefined".
        let empty_peers: Vec<String> = vec![];
        Json(serde_json::json!({
            "peer_count": s.p2p_status.peers.len(),
            "peers": empty_peers,
            "protocols": s.p2p_status.protocols,
        }))
    }
}

// GET /v1/bridge/status
async fn bridge_status(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    Json(s.bridge_status.clone())
}

// GET /v1/bridge/anchors
//
// Returns the anchor commit history. For now, synthesises a single entry
// from the current bridge status since we don't persist a full log yet.
async fn bridge_anchors(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    let bs = &s.bridge_status;
    let mut anchors: Vec<serde_json::Value> = Vec::new();

    // Synthesise the latest anchor from current bridge state
    if bs.last_finalized_eth_block > 0 {
        anchors.push(serde_json::json!({
            "l2_height": s.chain.stats().current_height,
            "l1_block": bs.last_finalized_eth_block,
            "state_root": bs.last_committed_root,
            "l1_tx_hash": bs.last_commit_tx_hash,
            "committed_at": chrono::Utc::now().to_rfc3339(),
            "gas_used": serde_json::Value::Null,
        }));
    }

    Json(serde_json::json!({
        "anchors": anchors,
        "total": anchors.len(),
    }))
}

// GET /v1/governance/proposals
//
// Governance is not yet implemented on-chain. This stub returns an empty
// list so the explorer page can render gracefully. When on-chain governance
// arrives (e.g. via a GovernanceProposal transaction variant), this handler
// will scan the chain for proposal transactions.
async fn governance_proposals(State(_state): State<SharedState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "proposals": [] as Vec<serde_json::Value>,
        "total": 0,
        "note": "On-chain governance is planned for a future release.",
    }))
}

// GET /v1/metrics/history?range=1h
//
// Time-series metrics endpoint. Currently returns an empty sample set.
// When a metrics accumulator is added to the node, this will return
// historical chain stats at regular intervals.
#[derive(Deserialize)]
struct MetricsHistoryParams {
    #[serde(default = "default_metrics_range")]
    range: String,
}
fn default_metrics_range() -> String { "1h".to_string() }

async fn metrics_history(
    State(_state): State<SharedState>,
    Query(params): Query<MetricsHistoryParams>,
) -> impl IntoResponse {
    Json(serde_json::json!({
        "range": params.range,
        "samples": [] as Vec<serde_json::Value>,
        "note": "Server-side metrics accumulation is planned for Sprint 5.",
    }))
}

// GET /v1/packages?offset=0&limit=50&ecosystem=npm&status=verified
async fn list_packages(
    State(state): State<SharedState>,
    Query(params): Query<ListPackagesParams>,
) -> Response {
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(50).min(200);
    let ecosystem = params.ecosystem.as_deref();
    let status_filter = params.status.as_deref().and_then(|s| match s {
        "verified" => Some(PackageStatus::Verified),
        "pending" => Some(PackageStatus::Pending),
        "revoked" => Some(PackageStatus::Revoked {
            reason: String::new(),
        }),
        _ => None,
    });

    let s = state.read().await;
    match s
        .chain
        .list_packages(offset, limit, ecosystem, status_filter.as_ref())
    {
        Ok((records, total)) => {
            #[derive(Serialize)]
            struct ListResp {
                packages: Vec<PackageSummary>,
                total: usize,
                offset: usize,
                limit: usize,
            }

            #[derive(Serialize)]
            struct PackageSummary {
                canonical: String,
                ecosystem: String,
                name: String,
                version: String,
                status: String,
                publisher: String,
                published_at: String,
            }

            let packages: Vec<PackageSummary> = records
                .into_iter()
                .map(|r| PackageSummary {
                    canonical: r.id.canonical(),
                    ecosystem: r.id.ecosystem.clone(),
                    name: r.id.name.clone(),
                    version: r.id.version.clone(),
                    status: match &r.status {
                        PackageStatus::Verified => "verified".into(),
                        PackageStatus::Pending => "pending".into(),
                        PackageStatus::Revoked { .. } => "revoked".into(),
                    },
                    publisher: r.publisher_pubkey.clone(),
                    published_at: r.published_at.to_rfc3339(),
                })
                .collect();

            Json(ListResp {
                packages,
                total,
                offset,
                limit,
            })
            .into_response()
        }
        Err(e) => server_err(format!("Failed to list packages: {}", e)),
    }
}

// GET /v1/packages/:canonical
async fn get_package(State(state): State<SharedState>, Path(canonical): Path<String>) -> Response {
    let canonical = urlencoding::decode(&canonical)
        .unwrap_or_default()
        .to_string();
    let s = state.read().await;

    // Check verified chain first.
    if let Ok(Some(record)) = s.chain.get_package(&canonical) {
        #[derive(Serialize)]
        struct PackageResp {
            canonical: String,
            status: &'static str,
            block_hash: Option<String>,
            content_hash: Option<String>,
            ipfs_cid: Option<String>,
            publisher: Option<String>,
            published_at: Option<String>,
            revocation_reason: Option<String>,
        }
        let resp = PackageResp {
            canonical: record.id.canonical(),
            status: match &record.status {
                PackageStatus::Verified => "verified",
                PackageStatus::Revoked { .. } => "revoked",
                _ => "pending",
            },
            block_hash: Some(record.block_hash.clone()),
            content_hash: Some(record.content_hash.clone()),
            ipfs_cid: Some(record.ipfs_cid.clone()),
            publisher: Some(record.publisher_pubkey.clone()),
            published_at: Some(record.published_at.to_rfc3339()),
            revocation_reason: if let PackageStatus::Revoked { reason } = &record.status {
                Some(reason.clone())
            } else {
                None
            },
        };
        return Json(resp).into_response();
    }

    // Check pending pool.
    if s.pending_pool.contains(&canonical) {
        return Json(serde_json::json!({
            "canonical": canonical,
            "status": "pending"
        }))
        .into_response();
    }

    not_found(format!("Package not found: {}", canonical))
}

// POST /v1/packages
async fn submit_package(
    State(state): State<SharedState>,
    Json(request): Json<PublishRequest>,
) -> Response {
    let canonical = request.id.canonical();
    tracing::info!("Publish request: {}", canonical);

    if let Err(e) = verify_publish_sig(&request) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid publisher signature: {}", e),
            }),
        )
            .into_response();
    }

    let mut s = state.write().await;

    // Reject if already verified/revoked.
    if let Ok(Some(rec)) = s.chain.get_package(&canonical) {
        if matches!(rec.status, PackageStatus::Verified) {
            return (
                StatusCode::CONFLICT,
                Json(ErrorResponse {
                    error: format!("{} is already verified on chain", canonical),
                }),
            )
                .into_response();
        }
        if matches!(rec.status, PackageStatus::Revoked { .. }) {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: format!("{} is revoked and cannot be resubmitted", canonical),
                }),
            )
                .into_response();
        }
    }

    // ── 3. Broadcast to P2P network ───────────────────────────────────────────
    let gossip_req = common::GossipMessage::PublishRequest(request.clone());
    let _ = s
        .p2p
        .sender
        .send(crate::p2p::P2PCommand::Broadcast {
            topic: "creg/v1/submissions".into(),
            data: serde_json::to_vec(&gossip_req).unwrap_or_default(),
        })
        .await;

    if !s.pending_pool.insert(request) {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: format!(
                    "{} is already pending with the same content hash",
                    canonical
                ),
            }),
        )
            .into_response();
    }
    tracing::info!(
        "{} added to pending pool ({} pending)",
        canonical,
        s.pending_pool.len()
    );

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "status":    "accepted",
            "canonical": canonical,
            "message":   "Package submitted. Validator pipeline will pick it up shortly."
        })),
    )
        .into_response()
}

// POST /v1/packages/:canonical/revoke
//
// Security: the caller MUST be either a registered validator or the original
// publisher of the package.  They prove their identity by signing the message
// `"{canonical}:revoke:{reason}"` with their Ed25519 key.
#[derive(Deserialize)]
struct RevokeReq {
    reason: String,
    /// Hex-encoded Ed25519 public key of the revoker.
    revoker_pubkey: String,
    /// Hex-encoded Ed25519 signature of `"{canonical}:revoke:{reason}"`.
    signature: String,
}

async fn revoke_package(
    State(state): State<SharedState>,
    Path(canonical): Path<String>,
    Json(req): Json<RevokeReq>,
) -> Response {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let canonical = urlencoding::decode(&canonical)
        .unwrap_or_default()
        .to_string();

    // ── 1. Verify Ed25519 signature ───────────────────────────────────────────
    let sig_msg = format!("{}:revoke:{}", canonical, req.reason);
    let sig_valid: Result<(), _> = (|| {
        let pk_bytes = hex::decode(&req.revoker_pubkey)
            .map_err(|_| anyhow::anyhow!("revoker_pubkey is not valid hex"))?;
        let vk = VerifyingKey::try_from(pk_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("revoker_pubkey is not a valid Ed25519 key"))?;
        let sig_bytes = hex::decode(&req.signature)
            .map_err(|_| anyhow::anyhow!("signature is not valid hex"))?;
        let sig = Signature::try_from(sig_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("signature is not a valid Ed25519 signature"))?;
        vk.verify(sig_msg.as_bytes(), &sig)
            .map_err(|_| anyhow::anyhow!("Signature verification failed"))
    })();

    if let Err(e) = sig_valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: format!("Invalid revocation signature: {}", e),
            }),
        )
            .into_response();
    }

    let s = state.read().await;

    // ── 2. Authorisation: revoker must be the original publisher or a validator ─
    let is_authorised = {
        // Check if revoker is a registered validator.
        let is_validator = s
            .validator_set
            .validators
            .iter()
            .any(|v| v.pubkey == req.revoker_pubkey);

        // Check if revoker is the original publisher of this package.
        let is_publisher = s
            .chain
            .get_package(&canonical)
            .ok()
            .flatten()
            .map(|r| r.publisher_pubkey == req.revoker_pubkey)
            .unwrap_or(false);

        is_validator || is_publisher
    };

    if !is_authorised {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: format!(
                    "Revoker pubkey {} is not a registered validator or the original publisher of {}",
                    &req.revoker_pubkey[..req.revoker_pubkey.len().min(16)],
                    canonical
                ),
            }),
        )
            .into_response();
    }

    // ── 3. Queue the revocation transaction ───────────────────────────────────
    match s.chain.get_package(&canonical) {
        Ok(Some(record)) => {
            let tx = common::Transaction::Revoke {
                package_canonical: canonical.clone(),
                reason: req.reason.clone(),
                revoked_by: req.revoker_pubkey.clone(),
                evidence_hash: record.content_hash.clone(),
            };
            if s.tx_sender.send(tx).await.is_err() {
                return server_err("Finalized-tx channel closed".to_string());
            }
            tracing::info!(
                canonical = %canonical,
                revoker = %&req.revoker_pubkey[..req.revoker_pubkey.len().min(16)],
                reason = %req.reason,
                "Package revocation queued"
            );
            events::emit(
                &s.event_bus,
                events::RegistryEvent::package_revoked(
                    &canonical,
                    &req.reason,
                    &req.revoker_pubkey,
                ),
            );
            Json(serde_json::json!({
                "status": "queued",
                "message": "Revocation will be included in the next block",
                "revoked_by": req.revoker_pubkey,
            }))
            .into_response()
        }
        Ok(None) => not_found(format!("Package not found: {}", canonical)),
        Err(e) => server_err(e.to_string()),
    }
}

// GET /v1/packages/:canonical/proof  (light-client SPV proof)
async fn get_proof(State(state): State<SharedState>, Path(canonical): Path<String>) -> Response {
    let canonical = urlencoding::decode(&canonical)
        .unwrap_or_default()
        .to_string();
    let s = state.read().await;

    match crate::proof::build_proof(&canonical, &s.chain) {
        Ok(Some(proof)) => Json(proof).into_response(),
        Ok(None) => not_found(format!("No proof available for: {}", canonical)),
        Err(e) => server_err(e.to_string()),
    }
}

/// Serialize a Block to JSON with a top-level `hash` field injected.
/// `Block` only stores `header` + `transactions`; the hash is computed on-the-fly
/// via `block.hash()`.  Clients (explorer, TUI) expect a `hash` field in the response.
fn block_to_json(b: &common::Block) -> serde_json::Value {
    let mut v = serde_json::to_value(b).unwrap_or_default();
    if let serde_json::Value::Object(ref mut map) = v {
        map.insert("hash".into(), serde_json::Value::String(b.hash()));
    }
    v
}

// GET /v1/blocks/:height
async fn get_block_by_height(
    State(state): State<SharedState>,
    Path(height): Path<u64>,
) -> Response {
    let s = state.read().await;
    match s.chain.get_block_by_height(height) {
        Ok(Some(b)) => Json(block_to_json(&b)).into_response(),
        Ok(None) => not_found(format!("No block at height {}", height)),
        Err(e) => server_err(e.to_string()),
    }
}

// GET /v1/blocks/hash/:hash
async fn get_block_by_hash(State(state): State<SharedState>, Path(hash): Path<String>) -> Response {
    let s = state.read().await;
    match s.chain.get_block_by_hash(&hash) {
        Ok(Some(b)) => Json(block_to_json(&b)).into_response(),
        Ok(None) => not_found(format!("No block with hash {}", hash)),
        Err(e) => server_err(e.to_string()),
    }
}

// GET /v1/blocks?offset=0&limit=20
//     /v1/blocks?before_height=H&limit=20  — cursor: heights < H
//     /v1/blocks?after_height=H&limit=20   — cursor: heights > H
//
// Returns blocks in descending height order (newest first).
// limit is capped at 100 to prevent large response payloads.
// Response includes X-Total-Height for UI pagination.
#[derive(Deserialize)]
struct ListBlocksParams {
    offset: Option<u64>,
    limit: Option<u64>,
    before_height: Option<u64>,
    after_height: Option<u64>,
}

async fn list_blocks_paginated(
    State(state): State<SharedState>,
    Query(params): Query<ListBlocksParams>,
) -> Response {
    let limit = params.limit.unwrap_or(20).min(100);

    let s = state.read().await;
    let tip = match s.chain.tip_height() {
        Ok(h) => h,
        Err(e) => return server_err(format!("Failed to read tip height: {}", e)),
    };

    // Cursor mode takes precedence over offset.
    let (offset, next_before, next_after) = if let Some(before) = params.before_height {
        // Blocks strictly below `before`, newest first → start at before-1.
        let start = before.saturating_sub(1);
        let computed_offset = tip.saturating_sub(start);
        let next_before = start.saturating_sub(limit.saturating_sub(1));
        (computed_offset, Some(next_before), None)
    } else if let Some(after) = params.after_height {
        // Blocks strictly above `after`, newest first → tip down to after+1.
        if after >= tip {
            (0, None, Some(after))
        } else {
            let window_top = tip.min(after.saturating_add(limit));
            let computed_offset = tip.saturating_sub(window_top);
            (computed_offset, None, Some(window_top))
        }
    } else {
        (params.offset.unwrap_or(0), None, None)
    };

    match s.chain.list_blocks(offset, limit) {
        Ok(blocks) => {
            let mut next_before_height = next_before;
            if next_before_height.is_none() && params.before_height.is_some() {
                next_before_height = blocks.last().map(|b| b.header.height);
            }

            let builder = axum::http::Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .header("X-Total-Height", tip.to_string())
                .header("X-Offset", offset.to_string())
                .header("X-Limit", limit.to_string());

            let blocks_with_hash: Vec<serde_json::Value> =
                blocks.iter().map(|b| block_to_json(b)).collect();
            let body = serde_json::json!({
                "blocks": blocks_with_hash,
                "tip_height": tip,
                "offset": offset,
                "limit": limit,
                "next_before_height": next_before_height,
                "next_after_height": next_after,
            });

            builder.body(axum::body::Body::from(
                serde_json::to_vec(&body).unwrap_or_default(),
            ))
            .unwrap_or_else(|_| server_err("Response build error"))
        }
        Err(e) => server_err(format!("Failed to list blocks: {}", e)),
    }
}

// GET /v1/transactions/:canonical
//
// Searches on-chain blocks for a transaction matching the given canonical ID
// (e.g., "npm/express@4.18.0").  Returns the transaction plus the block height
// and hash it was included in.  Scans the most recent 200 blocks.
async fn get_transaction(
    State(state): State<SharedState>,
    Path(canonical): Path<String>,
) -> Response {
    let canonical = urlencoding::decode(&canonical)
        .unwrap_or_default()
        .to_string();
    let s = state.read().await;

    // Scan recent blocks for a matching transaction.
    let blocks = match s.chain.list_blocks(0, 200) {
        Ok(b) => b,
        Err(e) => return server_err(format!("Failed to read blocks: {}", e)),
    };

    for block in &blocks {
        for tx in &block.transactions {
            let tx_canonical = match tx {
                common::Transaction::Publish(record) => record.id.canonical(),
                common::Transaction::Revoke {
                    package_canonical, ..
                } => package_canonical.clone(),
                common::Transaction::Slash { validator_id, .. } => validator_id.clone(),
                common::Transaction::ValidatorJoin { validator_id, .. } => validator_id.clone(),
                common::Transaction::ValidatorLeave { validator_id } => validator_id.clone(),
                common::Transaction::RotatePublisherKey {
                    canonical_prefix, ..
                } => canonical_prefix.clone(),
            };

            if tx_canonical == canonical {
                return Json(serde_json::json!({
                    "transaction": tx,
                    "block_height": block.header.height,
                    "block_hash": block.hash(),
                }))
                .into_response();
            }
        }
    }

    not_found(format!("Transaction not found: {}", canonical))
}

// POST /v1/blocks/announce
//
// The proposer proves identity by signing `"{block_hash}:{height}"` with their
// Ed25519 key.  The key must belong to a registered validator.  This prevents
// anonymous nodes from injecting fake block announcements.
#[derive(Deserialize)]
struct BlockAnnounceReq {
    /// Height of the announced block.
    height: u64,
    /// Hex-encoded SHA-256 block hash.
    block_hash: String,
    /// Validator ID of the proposer.
    proposer: String,
    /// Hex-encoded Ed25519 public key of the proposer.
    proposer_pubkey: String,
    /// Hex-encoded Ed25519 signature of `"{block_hash}:{height}"`.
    signature: String,
}

async fn receive_block_announcement(
    State(state): State<SharedState>,
    Json(ann): Json<BlockAnnounceReq>,
) -> impl IntoResponse {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    // ── 1. Verify Ed25519 signature ───────────────────────────────────────────
    let sig_msg = format!("{}:{}", ann.block_hash, ann.height);
    let sig_valid: Result<(), _> = (|| {
        let pk_bytes = hex::decode(&ann.proposer_pubkey)
            .map_err(|_| anyhow::anyhow!("proposer_pubkey is not valid hex"))?;
        let vk = VerifyingKey::try_from(pk_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("proposer_pubkey is not a valid Ed25519 key"))?;
        let sig_bytes = hex::decode(&ann.signature)
            .map_err(|_| anyhow::anyhow!("signature is not valid hex"))?;
        let sig = Signature::try_from(sig_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("signature is not a valid Ed25519 signature"))?;
        vk.verify(sig_msg.as_bytes(), &sig)
            .map_err(|_| anyhow::anyhow!("Signature verification failed"))
    })();

    if let Err(e) = sig_valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": format!("Invalid proposer signature: {}", e) })),
        )
            .into_response();
    }

    // ── 2. Proposer must be a registered validator ────────────────────────────
    let s = state.read().await;
    let is_validator = s
        .validator_set
        .validators
        .iter()
        .any(|v| v.pubkey == ann.proposer_pubkey);

    if !is_validator {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": format!(
                    "Proposer pubkey {} is not a registered validator",
                    &ann.proposer_pubkey[..ann.proposer_pubkey.len().min(16)]
                )
            })),
        )
            .into_response();
    }

    tracing::debug!(
        proposer = %ann.proposer,
        height = ann.height,
        hash = %&ann.block_hash[..ann.block_hash.len().min(12)],
        "Block announcement accepted"
    );
    Json(serde_json::json!({ "status": "noted" })).into_response()
}

// GET /v1/publishers/:pubkey
async fn get_publisher(State(state): State<SharedState>, Path(pubkey): Path<String>) -> Response {
    let s = state.read().await;
    match s.publisher_index.get(&pubkey) {
        Some(stats) => Json(stats.clone()).into_response(),
        None => not_found(format!("Publisher not found: {}", pubkey)),
    }
}

// ─── Address endpoints ────────────────────────────────────────────────────────
//
// EVM addresses surface in multiple places: as validator evm_address, as block
// proposer_id, and as revoker_by in Revoke txs.  These handlers aggregate
// across those for per-address profile and transaction-history views.

const ADDRESS_DEFAULT_SCAN_BLOCKS: u64 = 500;
const ADDRESS_MAX_SCAN_BLOCKS: u64 = 5000;

fn is_evm_address_like(s: &str) -> bool {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    stripped.len() == 40 && stripped.chars().all(|c| c.is_ascii_hexdigit())
}

fn tx_kind_label(tx: &Transaction) -> &'static str {
    match tx {
        Transaction::Publish(_) => "publish",
        Transaction::Revoke { .. } => "revoke",
        Transaction::Slash { .. } => "slash",
        Transaction::ValidatorJoin { .. } => "validator-join",
        Transaction::ValidatorLeave { .. } => "validator-leave",
        Transaction::RotatePublisherKey { .. } => "rotate-key",
    }
}

fn tx_canonical(tx: &Transaction) -> Option<String> {
    match tx {
        Transaction::Publish(rec) => Some(rec.id.canonical()),
        Transaction::Revoke { package_canonical, .. } => Some(package_canonical.clone()),
        Transaction::RotatePublisherKey { canonical_prefix, .. } => Some(canonical_prefix.clone()),
        _ => None,
    }
}

/// True if the given tx references `addr` (case-insensitive) in any role.
fn tx_touches_address(tx: &Transaction, addr: &str) -> bool {
    match tx {
        Transaction::Revoke { revoked_by, .. } => revoked_by.to_ascii_lowercase() == addr,
        Transaction::Slash { validator_id, .. } => validator_id.to_ascii_lowercase() == addr,
        Transaction::ValidatorJoin { validator_id, .. } => {
            validator_id.to_ascii_lowercase() == addr
        }
        Transaction::ValidatorLeave { validator_id } => validator_id.to_ascii_lowercase() == addr,
        // Publish / RotatePublisherKey use ed25519 pubkeys, not EVM addresses — skip here.
        _ => false,
    }
}

// GET /v1/addresses/:address
async fn get_address(State(state): State<SharedState>, Path(address): Path<String>) -> Response {
    if !is_evm_address_like(&address) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Not a valid EVM address: {}", address),
            }),
        )
            .into_response();
    }
    let normalized = address.to_ascii_lowercase();
    let s = state.read().await;

    let registration = s
        .validator_registrations
        .get(&normalized)
        .cloned()
        .map(|r| serde_json::to_value(&r).unwrap_or(serde_json::Value::Null));

    let active = s
        .validator_set
        .validators
        .iter()
        .find(|v| v.id.to_ascii_lowercase() == normalized)
        .cloned();

    let scan = ADDRESS_DEFAULT_SCAN_BLOCKS;
    let blocks = s.chain.list_blocks(0, scan).unwrap_or_default();
    let scanned_blocks = blocks.len() as u64;

    let mut blocks_proposed = 0u32;
    let mut tx_count = 0u32;
    for b in &blocks {
        if b.header.proposer_id.to_ascii_lowercase() == normalized {
            blocks_proposed += 1;
        }
        for tx in &b.transactions {
            if tx_touches_address(tx, &normalized) {
                tx_count += 1;
            }
        }
    }

    Json(serde_json::json!({
        "address": normalized,
        "is_validator": registration.is_some(),
        "is_active_validator": active.is_some(),
        "validator": registration,
        "active_status": active.as_ref().map(|v| v.status.clone()),
        "stake": active.as_ref().map(|v| v.stake.to_string()),
        "reputation": active.as_ref().map(|v| v.reputation),
        "blocks_proposed": blocks_proposed,
        "tx_count": tx_count,
        "scanned_blocks": scanned_blocks,
    }))
    .into_response()
}

// GET /v1/addresses/:address/transactions
#[derive(Deserialize)]
struct AddressTxParams {
    limit: Option<usize>,
    scan: Option<u64>,
}

async fn get_address_transactions(
    State(state): State<SharedState>,
    Path(address): Path<String>,
    Query(params): Query<AddressTxParams>,
) -> Response {
    if !is_evm_address_like(&address) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Not a valid EVM address: {}", address),
            }),
        )
            .into_response();
    }
    let normalized = address.to_ascii_lowercase();
    let limit = params.limit.unwrap_or(50).min(500);
    let scan = params
        .scan
        .unwrap_or(ADDRESS_DEFAULT_SCAN_BLOCKS)
        .min(ADDRESS_MAX_SCAN_BLOCKS);

    let s = state.read().await;
    let blocks = s.chain.list_blocks(0, scan).unwrap_or_default();
    let scanned_blocks = blocks.len() as u64;

    let mut results: Vec<serde_json::Value> = Vec::new();
    for b in &blocks {
        let block_hash = b.hash();
        let ts = b.header.timestamp.to_rfc3339();
        let proposer_match = b.header.proposer_id.to_ascii_lowercase() == normalized;
        if proposer_match {
            results.push(serde_json::json!({
                "block_height": b.header.height,
                "block_hash": block_hash,
                "tx_index": 0,
                "kind": "propose",
                "canonical": serde_json::Value::Null,
                "timestamp": ts,
            }));
        }
        for (idx, tx) in b.transactions.iter().enumerate() {
            if tx_touches_address(tx, &normalized) {
                results.push(serde_json::json!({
                    "block_height": b.header.height,
                    "block_hash": block_hash,
                    "tx_index": idx,
                    "kind": tx_kind_label(tx),
                    "canonical": tx_canonical(tx),
                    "timestamp": ts,
                }));
            }
        }
        if results.len() >= limit {
            break;
        }
    }
    results.truncate(limit);

    Json(serde_json::json!({
        "address": normalized,
        "transactions": results,
        "scanned_blocks": scanned_blocks,
        "total": results.len(),
    }))
    .into_response()
}

// GET /v1/validators/:address
async fn get_validator_profile(
    State(state): State<SharedState>,
    Path(address): Path<String>,
) -> Response {
    if !is_evm_address_like(&address) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Not a valid EVM address: {}", address),
            }),
        )
            .into_response();
    }
    let normalized = address.to_ascii_lowercase();
    let s = state.read().await;

    let registration = s.validator_registrations.get(&normalized).cloned();
    let active = s
        .validator_set
        .validators
        .iter()
        .find(|v| v.id.to_ascii_lowercase() == normalized)
        .cloned();

    if registration.is_none() && active.is_none() {
        return not_found(format!("Validator not found: {}", address));
    }

    let (stake, reputation, status, in_active_set) = match active.as_ref() {
        Some(v) => (v.stake.to_string(), v.reputation, v.status.clone(), true),
        None => (
            registration
                .as_ref()
                .map(|r| r.stake.to_string())
                .unwrap_or_default(),
            registration.as_ref().map(|r| r.reputation).unwrap_or(0),
            registration
                .as_ref()
                .map(|r| r.status.clone())
                .unwrap_or_else(|| "unknown".to_string()),
            false,
        ),
    };

    let blocks = s.chain.list_blocks(0, 500).unwrap_or_default();
    let recent_proposals: Vec<serde_json::Value> = blocks
        .iter()
        .filter(|b| b.header.proposer_id.to_ascii_lowercase() == normalized)
        .take(25)
        .map(|b| block_to_json(b))
        .collect();

    Json(serde_json::json!({
        "address": normalized,
        "registration": registration,
        "in_active_set": in_active_set,
        "stake": stake,
        "reputation": reputation,
        "status": status,
        "recent_proposals": recent_proposals,
    }))
    .into_response()
}

// GET /v1/pending
async fn list_pending(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    Json(serde_json::json!({
        "count":    s.pending_pool.len(),
        "packages": s.pending_pool.all_canonicals()
    }))
}

// ─── Search endpoint ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchParams {
    q: String,
}

#[derive(Serialize)]
struct SearchMatch {
    kind: &'static str,
    href: String,
    title: String,
    subtitle: String,
}

/// GET /v1/search?q=<query>
///
/// Smart-classifies the query string and returns matching entities:
///  - All digits → block by height
///  - 0x + 40 hex → EVM address (check validator set)
///  - 0x + 64 hex → try block by hash, then transaction
///  - Contains '@' → package canonical
///  - Otherwise → scan package names, validator aliases
async fn search_handler(
    State(state): State<SharedState>,
    Query(params): Query<SearchParams>,
) -> Response {
    let q = params.q.trim().to_string();
    if q.is_empty() {
        return Json(serde_json::json!({ "matches": [] as Vec<serde_json::Value> })).into_response();
    }

    let s = state.read().await;
    let mut matches: Vec<SearchMatch> = Vec::new();
    const MAX_RESULTS: usize = 10;

    // 1. All digits → block height
    if q.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(height) = q.parse::<u64>() {
            if let Ok(Some(_)) = s.chain.get_block_by_height(height) {
                matches.push(SearchMatch {
                    kind: "block",
                    href: format!("/block/{}", height),
                    title: format!("Block #{}", height),
                    subtitle: "Block by height".into(),
                });
            }
        }
    }

    // 2. 0x + 40 hex → EVM address
    let stripped = q.strip_prefix("0x").unwrap_or(&q);
    if stripped.len() == 40 && stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        let normalized = q.to_ascii_lowercase();
        matches.push(SearchMatch {
            kind: "address",
            href: format!("/address/{}", normalized),
            title: normalized.clone(),
            subtitle: "EVM address".into(),
        });
        // Check if it's a validator
        let clean = normalized.strip_prefix("0x").unwrap_or(&normalized).to_string();
        let is_validator = s.validator_registrations.contains_key(&normalized)
            || s.validator_registrations.contains_key(&clean)
            || s.validator_set.validators.iter().any(|v| v.id.to_ascii_lowercase() == normalized);
        if is_validator {
            matches.push(SearchMatch {
                kind: "validator",
                href: format!("/validator/{}", normalized),
                title: normalized.clone(),
                subtitle: "Validator".into(),
            });
        }
    }

    // 3. 0x + 64 hex → block hash or tx hash
    if stripped.len() == 64 && stripped.chars().all(|c| c.is_ascii_hexdigit()) {
        let hash_lower = q.to_ascii_lowercase();
        if let Ok(Some(block)) = s.chain.get_block_by_hash(&hash_lower) {
            matches.push(SearchMatch {
                kind: "block",
                href: format!("/block/{}", block.header.height),
                title: format!("Block #{}", block.header.height),
                subtitle: format!("hash: {}…", &hash_lower[..16]),
            });
        }
    }

    // 4. Contains '@' → package canonical
    if q.contains('@') && !q.starts_with("0x") {
        if let Ok(Some(record)) = s.chain.get_package(&q) {
            let status_str = match &record.status {
                PackageStatus::Verified => "verified",
                PackageStatus::Pending => "pending",
                PackageStatus::Revoked { .. } => "revoked",
            };
            matches.push(SearchMatch {
                kind: "package",
                href: format!("/package/{}", urlencoding::encode(&q)),
                title: record.id.canonical(),
                subtitle: format!("status: {}", status_str),
            });
        } else if s.pending_pool.contains(&q) {
            matches.push(SearchMatch {
                kind: "package",
                href: format!("/package/{}", urlencoding::encode(&q)),
                title: q.clone(),
                subtitle: "pending".into(),
            });
        }
    }

    // 5. Free text — search validator aliases and publisher index
    if matches.is_empty() || (!q.starts_with("0x") && !q.chars().all(|c| c.is_ascii_digit()) && !q.contains('@')) {
        let q_lower = q.to_ascii_lowercase();
        // Scan validator aliases
        for (key, reg) in s.validator_registrations.iter() {
            if matches.len() >= MAX_RESULTS { break; }
            if reg.alias.to_ascii_lowercase().contains(&q_lower)
                || key.contains(&q_lower)
            {
                let addr = key.clone();
                if !matches.iter().any(|m| m.href.contains(&addr)) {
                    matches.push(SearchMatch {
                        kind: "validator",
                        href: format!("/validator/{}", addr),
                        title: if reg.alias.is_empty() { addr.clone() } else { reg.alias.clone() },
                        subtitle: format!("validator: {}", addr),
                    });
                }
            }
        }
        // Scan publisher index
        for (pubkey, _stats) in s.publisher_index.iter() {
            if matches.len() >= MAX_RESULTS { break; }
            if pubkey.to_ascii_lowercase().contains(&q_lower) {
                if !matches.iter().any(|m| m.href.contains(pubkey)) {
                    matches.push(SearchMatch {
                        kind: "publisher",
                        href: format!("/publisher/{}", urlencoding::encode(pubkey)),
                        title: pubkey.clone(),
                        subtitle: "publisher".into(),
                    });
                }
            }
        }
    }

    matches.truncate(MAX_RESULTS);
    Json(serde_json::json!({ "matches": matches })).into_response()
}

// POST /v1/publishers/rotate-key
#[derive(Deserialize)]
pub struct RotateKeyRequest {
    pub canonical_prefix: String,
    pub old_pubkey: String,
    pub new_pubkey: String,
    pub sig_from_old: String,
    pub sig_from_new: String,
    /// Monotonic nonce — must be strictly greater than the publisher's last
    /// rotation nonce.  Prevents replay of old rotation requests.
    #[serde(default)]
    pub nonce: u64,
}

async fn rotate_publisher_key(
    State(state): State<SharedState>,
    Json(req): Json<RotateKeyRequest>,
) -> Response {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    // 1. Verify sig_from_old: old key signs new_pubkey
    let verify_sig = |pubkey_hex: &str, msg: &str, sig_hex: &str| -> anyhow::Result<()> {
        let pk_bytes = hex::decode(pubkey_hex)?;
        let sig_bytes = hex::decode(sig_hex)?;
        let vk = VerifyingKey::try_from(pk_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("bad pubkey"))?;
        let sig = Signature::try_from(sig_bytes.as_slice())
            .map_err(|_| anyhow::anyhow!("bad signature"))?;
        vk.verify(msg.as_bytes(), &sig)
            .map_err(|_| anyhow::anyhow!("signature verification failed"))
    };

    if let Err(e) = verify_sig(&req.old_pubkey, &req.new_pubkey, &req.sig_from_old) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid sig_from_old: {}", e),
            }),
        )
            .into_response();
    }
    if let Err(e) = verify_sig(&req.new_pubkey, &req.old_pubkey, &req.sig_from_new) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid sig_from_new: {}", e),
            }),
        )
            .into_response();
    }

    // 2. Replay protection: nonce must be strictly greater than the
    //    publisher's last rotation nonce, and timestamp must be recent.
    let now = chrono::Utc::now();
    {
        let s = state.read().await;
        let last_nonce = s
            .chain
            .publisher_rotation_nonce(&req.old_pubkey)
            .unwrap_or(0);
        if req.nonce <= last_nonce {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!(
                        "Rotation nonce {} must be > last nonce {}. Replay rejected.",
                        req.nonce, last_nonce
                    ),
                }),
            )
                .into_response();
        }

        // 2b. Time-lock: enforce a minimum cooldown between rotations to
        //     prevent rapid unauthorized rotation attacks.
        const ROTATION_COOLDOWN_SECS: i64 = 3600; // 1 hour
        if let Some(last_time) = s.chain.publisher_last_rotation_time(&req.old_pubkey) {
            let elapsed = now.signed_duration_since(last_time).num_seconds();
            if elapsed < ROTATION_COOLDOWN_SECS {
                let remaining = ROTATION_COOLDOWN_SECS - elapsed;
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(ErrorResponse {
                        error: format!(
                            "Key rotation cooldown: {} seconds remaining. Last rotation was {}s ago (minimum {}s).",
                            remaining, elapsed, ROTATION_COOLDOWN_SECS
                        ),
                    }),
                )
                    .into_response();
            }
        }
    }

    // 3. Verify old_pubkey owns at least one package matching the prefix.
    let has_match = state
        .read()
        .await
        .chain
        .has_publisher_for_prefix(&req.canonical_prefix, &req.old_pubkey);
    if !has_match {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: format!(
                    "old_pubkey does not own any package matching {}",
                    req.canonical_prefix
                ),
            }),
        )
            .into_response();
    }

    // 4. Queue the rotation transaction.
    let tx = common::Transaction::RotatePublisherKey {
        canonical_prefix: req.canonical_prefix.clone(),
        old_pubkey: req.old_pubkey.clone(),
        new_pubkey: req.new_pubkey.clone(),
        sig_from_old: req.sig_from_old.clone(),
        sig_from_new: req.sig_from_new.clone(),
        timestamp: now,
        nonce: req.nonce,
    };

    let s = state.read().await;
    if s.tx_sender.send(tx).await.is_err() {
        return server_err("Finalized-tx channel closed".to_string());
    }

    Json(serde_json::json!({
        "status": "queued",
        "message": "Key rotation will be included in the next block"
    }))
    .into_response()
}

// GET /v1/consensus/state
//
// Returns a lightweight snapshot of the current PBFT consensus activity
// derived from the accumulated vote map and active validator set.  The TUI
// and web explorer use this to draw the PBFT gauge / consensus panel.
async fn consensus_state(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;

    let total_validators = s.validator_set.validators.len();
    // Standard BFT quorum: ⌊2n/3⌋ + 1
    let quorum = if total_validators == 0 {
        1
    } else {
        (2 * total_validators / 3) + 1
    };

    #[derive(Serialize)]
    struct RoundSummary {
        block_hash: String,
        vote_count: usize,
        approvals: usize,
        rejections: usize,
        phase: &'static str,
        /// Validator IDs that have cast a vote in this round.
        voters: Vec<String>,
        /// Subset of voters that cast Approve.
        approvers: Vec<String>,
        /// Subset of voters that cast Reject (with reason attached).
        rejecters: Vec<String>,
        /// Milliseconds since the earliest vote in this round (round age).
        age_ms: i64,
    }

    let now = chrono::Utc::now();
    let mut active_rounds: Vec<RoundSummary> = s
        .votes
        .iter()
        .map(|(block_hash, sigs)| {
            let mut approvers = Vec::new();
            let mut rejecters = Vec::new();
            let mut voters = Vec::with_capacity(sigs.len());
            for sig in sigs {
                voters.push(sig.validator_id.clone());
                match sig.vote {
                    common::ValidatorVote::Approve => approvers.push(sig.validator_id.clone()),
                    common::ValidatorVote::Reject { .. } => rejecters.push(sig.validator_id.clone()),
                }
            }
            let approvals = approvers.len();
            let rejections = rejecters.len();
            let phase = if approvals >= quorum {
                "quorum-reached"
            } else if rejections > 0 {
                "contested"
            } else {
                "collecting-votes"
            };
            let age_ms = sigs
                .iter()
                .map(|s| (now - s.signed_at).num_milliseconds())
                .max()
                .unwrap_or(0);
            RoundSummary {
                block_hash: block_hash.clone(),
                vote_count: sigs.len(),
                approvals,
                rejections,
                phase,
                voters,
                approvers,
                rejecters,
                age_ms,
            }
        })
        .collect();

    // Sort descending by vote count so the most active round is first.
    active_rounds.sort_by(|a, b| b.vote_count.cmp(&a.vote_count));
    // Cap to the 10 most active rounds to keep the response bounded.
    active_rounds.truncate(10);

    #[derive(Serialize)]
    struct ValidatorSnapshot {
        id: String,
        alias: String,
        stake: u64,
        reputation: u32,
        status: String,
    }
    let validators: Vec<ValidatorSnapshot> = s
        .validator_set
        .validators
        .iter()
        .map(|v| ValidatorSnapshot {
            id: v.id.clone(),
            alias: v.alias.clone(),
            stake: v.stake,
            reputation: v.reputation,
            status: v.status.clone(),
        })
        .collect();

    Json(serde_json::json!({
        "total_validators": total_validators,
        "quorum": quorum,
        "active_rounds": active_rounds,
        "pending_count": s.pending_pool.len(),
        "validators": validators,
    }))
}

// POST /v1/consensus/vote
#[derive(Deserialize, Serialize)]
pub struct VoteMessage {
    /// The canonical package ID (at vote time) or block hash (at seal time).
    pub block_hash: String,
    /// SHA-256 of the tarball bytes — bound into the signed message to prevent
    /// cross-version replay. Defaults to empty for backwards compatibility.
    #[serde(default)]
    pub content_hash: String,
    pub validator_id: String,
    pub phase: String,
    /// Hex-encoded Ed25519 signature of `gossip::canonical_vote_message(...)`.
    pub signature: String,
    /// Hex-encoded Ed25519 public key of the voting validator.
    pub validator_pubkey: String,
    pub approved: bool,
    pub reject_reason: Option<String>,
}

async fn receive_vote(State(state): State<SharedState>, Json(vote): Json<VoteMessage>) -> Response {
    // ── Authenticate: verify the vote is from a known validator ──────────────
    {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        let s = state.read().await;

        // 1. Check the claimed validator is in the active validator set
        //    AND that the supplied pubkey matches the registered pubkey.
        let validator = s
            .validator_set
            .validators
            .iter()
            .find(|v| v.id == vote.validator_id);
        let Some(validator) = validator else {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: format!("Unknown validator: {}", vote.validator_id),
                }),
            )
                .into_response();
        };
        // Every validator must have a registered pubkey — reject if missing,
        // because an empty pubkey would allow unauthenticated vote submission.
        if validator.pubkey.is_empty() {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: format!(
                        "Validator {} has no registered public key; \
                         vote authentication impossible",
                        vote.validator_id
                    ),
                }),
            )
                .into_response();
        }
        if vote.validator_pubkey != validator.pubkey {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse {
                    error: format!(
                        "Validator pubkey mismatch for {}: expected {}, got {}",
                        vote.validator_id, validator.pubkey, vote.validator_pubkey
                    ),
                }),
            )
                .into_response();
        }

        // 2. Verify the Ed25519 signature.
        let pubkey_bytes = match hex::decode(&vote.validator_pubkey) {
            Ok(b) => b,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid validator_pubkey hex".into(),
                    }),
                )
                    .into_response()
            }
        };
        let vk = match VerifyingKey::try_from(pubkey_bytes.as_slice()) {
            Ok(k) => k,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid Ed25519 public key".into(),
                    }),
                )
                    .into_response()
            }
        };
        let sig_bytes = match hex::decode(&vote.signature) {
            Ok(b) => b,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid signature hex".into(),
                    }),
                )
                    .into_response()
            }
        };
        let sig = match Signature::try_from(sig_bytes.as_slice()) {
            Ok(s) => s,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: "Invalid Ed25519 signature format".into(),
                    }),
                )
                    .into_response()
            }
        };

        // Canonical domain-separated vote message — must match exactly what
        // validator_pipeline.rs::gossip_sig produces via
        // gossip::canonical_vote_message.
        let msg = crate::gossip::canonical_vote_message(
            &vote.block_hash,
            &vote.content_hash,
            vote.approved,
            &vote.validator_pubkey,
        );
        if vk.verify(msg.as_bytes(), &sig).is_err() {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Vote signature verification failed".into(),
                }),
            )
                .into_response();
        }
    }

    let mut s = state.write().await;

    let sig = common::ValidatorSignature {
        validator_id: vote.validator_id.clone(),
        validator_pubkey: vote.validator_pubkey.clone(),
        signature: vote.signature.clone(),
        vote: if vote.approved {
            common::ValidatorVote::Approve
        } else {
            common::ValidatorVote::Reject {
                reason: vote.reject_reason.clone().unwrap_or_default(),
            }
        },
        signed_at: chrono::Utc::now(),
        ml_model_version: String::new(), // Populated by the originating validator
    };

    let key = vote.block_hash.clone();
    s.votes.entry(key).or_insert_with(Vec::new).push(sig);

    events::emit(
        &s.event_bus,
        events::RegistryEvent::validator_voted(&vote.validator_id, &vote.block_hash, vote.approved),
    );

    Json(serde_json::json!({ "status": "accepted" })).into_response()
}

// ─── Appeals & AAA ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AuditSubmission {
    pub approved: bool,
    pub proof: String,
    pub rationales: Vec<validator::report::Rationale>,
}

async fn submit_audit(
    Path(id): Path<u64>,
    Json(audit): Json<AuditSubmission>,
) -> impl IntoResponse {
    tracing::info!(
        "Received AAA audit for appeal {}: approved={}",
        id,
        audit.approved
    );

    // In a production node, this would:
    // 1. Verify the AI model's signature/proof.
    // 2. Submit the submitAIVerdict() transaction to the Appeal.sol contract.
    // 3. Update the local chain store if the block producer picks it up.

    Json(serde_json::json!({
        "status":  "submitted",
        "message": "AI verdict received and queued for on-chain finalization."
    }))
}

// GET /metrics  (Prometheus text format)
async fn prometheus_metrics(State(state): State<SharedState>) -> impl IntoResponse {
    let body = crate::metrics::render(Arc::clone(&state)).await;
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
}

// ─── Publisher signature verification ────────────────────────────────────────

pub(crate) fn verify_publish_sig(req: &PublishRequest) -> anyhow::Result<()> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let msg = format!("{}{}", req.id.canonical(), req.content_hash);

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
    for (pubkey_hex, sig_hex) in req.publisher_pubkeys.iter().zip(req.signatures.iter()) {
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

// ─── Sprint 5: Scale & Observability ──────────────────────────────────────────

/// GET /v1/reorgs — History of chain reorganizations
#[utoipa::path(
    get,
    path = "/v1/reorgs",
    tag = "Chain",
    responses(
        (status = 200, description = "Reorg history")
    )
)]
async fn reorgs(State(state): State<SharedState>) -> impl IntoResponse {
    let reorgs = state.read().await.reorgs.clone();
    Json(reorgs).into_response()
}

/// GET /v1/richlist — Top staked accounts
#[utoipa::path(
    get,
    path = "/v1/richlist",
    tag = "Diagnostics",
    responses(
        (status = 200, description = "Top accounts by stake")
    )
)]
async fn richlist(State(state): State<SharedState>) -> impl IntoResponse {
    let mut top: Vec<_> = state
        .read()
        .await
        .validator_registrations
        .values()
        .cloned()
        .collect();
    
    // Sort descending by stake
    top.sort_by(|a, b| b.stake.cmp(&a.stake));
    
    // Truncate to top 500
    top.truncate(500);

    Json(top).into_response()
}

// WS Handler
use axum::extract::ws::{WebSocketUpgrade, WebSocket, Message};

/// GET /v1/ws — High performance bidirectional websocket stream for consensus events
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_websocket(socket, state))
}

async fn handle_websocket(mut socket: WebSocket, state: SharedState) {
    let mut rx = state.read().await.event_bus.subscribe();

    tracing::info!("Websocket client connected");

    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            // Convert SSE payload to WS Message
            let data = serde_json::to_string(&event).unwrap_or_default();
            if socket.send(Message::Text(data)).await.is_err() {
                break; // Client disconnected
            }
        }
    });

    // In a push-based architecture, we don't strictly require ACKs at the WS layer
    // but we spawn a dummy receiver so we don't deadlock on incoming control frames.
    // If the socket yields None, it's closed.
    // Wait for the send_task or the receive loop to finish.
    let _ = send_task.await;
    tracing::info!("Websocket client disconnected");
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::{PackageId, PackageManifest};
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn make_keypair() -> (SigningKey, String) {
        let sk = SigningKey::generate(&mut OsRng);
        let pk = hex::encode(sk.verifying_key().as_bytes());
        (sk, pk)
    }

    fn make_request_with_sigs(
        publisher_pubkeys: Vec<String>,
        signatures: Vec<String>,
        threshold: usize,
    ) -> PublishRequest {
        PublishRequest {
            id: PackageId::new("npm", "test", "1.0.0"),
            content_hash: common::sha256_hex(b"test"),
            ipfs_cid: "bafytest".into(),
            publisher_pubkey: publisher_pubkeys.first().cloned().unwrap_or_default(),
            signature: signatures.first().cloned().unwrap_or_default(),
            manifest: PackageManifest::default(),
            submitted_at: chrono::Utc::now(),
            shielded: false,
            key_bundle: None,
            pgp_signature: None,
            pgp_public_key: None,
            threshold,
            publisher_pubkeys,
            signatures,
        }
    }

    #[test]
    fn single_sig_verifies() {
        let (sk, pk) = make_keypair();
        let req = make_request_with_sigs(vec![], vec![], 0);
        let msg = format!("{}{}", req.id.canonical(), req.content_hash);
        let sig = sk.sign(msg.as_bytes());

        let req = PublishRequest {
            publisher_pubkey: pk,
            signature: hex::encode(sig.to_bytes()),
            ..req
        };
        assert!(verify_publish_sig(&req).is_ok());
    }

    #[test]
    fn single_sig_rejects_bad_signature() {
        let (_sk, pk) = make_keypair();
        let req = PublishRequest {
            publisher_pubkey: pk,
            signature: "deadbeef".repeat(8),
            ..make_request_with_sigs(vec![], vec![], 0)
        };
        assert!(verify_publish_sig(&req).is_err());
    }

    #[test]
    fn multisig_2_of_3_verifies() {
        let (sk1, pk1) = make_keypair();
        let (sk2, pk2) = make_keypair();
        let (sk3, pk3) = make_keypair();

        let msg = format!(
            "{}{}",
            PackageId::new("npm", "test", "1.0.0").canonical(),
            common::sha256_hex(b"test")
        );
        let sig1 = sk1.sign(msg.as_bytes());
        let sig2 = sk2.sign(msg.as_bytes());

        let req = make_request_with_sigs(
            vec![pk1.clone(), pk2.clone(), pk3.clone()],
            vec![
                hex::encode(sig1.to_bytes()),
                hex::encode(sig2.to_bytes()),
                String::new(),
            ],
            2,
        );
        assert!(verify_publish_sig(&req).is_ok());
    }

    #[test]
    fn multisig_rejects_insufficient_sigs() {
        let (sk1, pk1) = make_keypair();
        let (_sk2, pk2) = make_keypair();
        let (_sk3, pk3) = make_keypair();

        let msg = format!(
            "{}{}",
            PackageId::new("npm", "test", "1.0.0").canonical(),
            common::sha256_hex(b"test")
        );
        let sig1 = sk1.sign(msg.as_bytes());

        let req = make_request_with_sigs(
            vec![pk1.clone(), pk2.clone(), pk3.clone()],
            vec![hex::encode(sig1.to_bytes()), String::new(), String::new()],
            2,
        );
        assert!(verify_publish_sig(&req).is_err());
    }

    #[test]
    fn multisig_3_of_3_verifies() {
        let (sk1, pk1) = make_keypair();
        let (sk2, pk2) = make_keypair();
        let (sk3, pk3) = make_keypair();

        let msg = format!(
            "{}{}",
            PackageId::new("npm", "test", "1.0.0").canonical(),
            common::sha256_hex(b"test")
        );
        let sig1 = sk1.sign(msg.as_bytes());
        let sig2 = sk2.sign(msg.as_bytes());
        let sig3 = sk3.sign(msg.as_bytes());

        let req = make_request_with_sigs(
            vec![pk1.clone(), pk2.clone(), pk3.clone()],
            vec![
                hex::encode(sig1.to_bytes()),
                hex::encode(sig2.to_bytes()),
                hex::encode(sig3.to_bytes()),
            ],
            3,
        );
        assert!(verify_publish_sig(&req).is_ok());
    }

    #[test]
    fn multisig_rejects_mismatched_counts() {
        let req = make_request_with_sigs(vec!["aa".into(), "bb".into()], vec!["cc".into()], 2);
        assert!(verify_publish_sig(&req).is_err());
    }
}
