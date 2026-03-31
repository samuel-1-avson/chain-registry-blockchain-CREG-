// crates/node/src/api.rs
// Axum REST API — all HTTP endpoints for the chain registry node.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use common::{PackageStatus, PublishRequest};
use serde::{Deserialize, Serialize};
use tower_http::{cors::CorsLayer, trace::TraceLayer, limit::RequestBodyLimitLayer};
use std::sync::Arc;

use crate::{SharedState, events::{EventBus, sse_handler}, rate_limit::{RateLimiter, rate_limit_middleware}};
use crate::events;

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn router(state: SharedState, event_bus: EventBus, limiter: RateLimiter) -> Router {
    Router::new()
        // Health & chain
        .route("/v1/health",                          get(health))
        .route("/v1/chain/stats",                     get(chain_stats))
        .route("/v1/nodes",                           get(get_nodes))
        .route("/v1/p2p/status",                      get(p2p_status))
        .route("/v1/bridge/status",                   get(bridge_status))
        // Packages
        .route("/v1/packages/:canonical",             get(get_package))
        .route("/v1/packages",                        post(submit_package))
        .route("/v1/packages/:canonical/revoke",      post(revoke_package))
        .route("/v1/packages/:canonical/proof",       get(get_proof))
        // Blocks
        .route("/v1/blocks/:height",                  get(get_block_by_height))
        .route("/v1/blocks/hash/:hash",               get(get_block_by_hash))
        .route("/v1/blocks/announce",                 post(receive_block_announcement))
        // Publishers
        .route("/v1/publishers/:pubkey",              get(get_publisher))
        // Pending pool
        .route("/v1/pending",                         get(list_pending))
        // Consensus
        .route("/v1/consensus/vote",                  post(receive_vote))
        // Appeals & AAA
        .route("/v1/appeals/:id/audit",               post(submit_audit))
        // Observability
        .route("/metrics",                            get(prometheus_metrics))
        // Event streaming - SSE & Websockets
        .route("/v1/events",                          get({
            let bus = Arc::clone(&event_bus);
            move |_: ()| async move { sse_handler(axum::extract::State(bus)).await }
        }))
        .route("/v1/ws",                              get(move |ws| {
            let bus = Arc::clone(&event_bus);
            async move { events::ws_handler(ws, axum::extract::State(bus)).await }
        }))
        .fallback(crate::explorer::static_handler)
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(50 * 1024 * 1024))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(axum::extract::Extension(limiter))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ─── Response helpers ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ErrorResponse { error: String }

fn not_found(msg: impl Into<String>) -> Response {
    (StatusCode::NOT_FOUND, Json(ErrorResponse { error: msg.into() })).into_response()
}

fn server_err(msg: impl Into<String>) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: msg.into() })).into_response()
}

// ─── Handlers ────────────────────────────────────────────────────────────────

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status":  "ok",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn chain_stats(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    Json(s.chain.stats())
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
async fn p2p_status(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    Json(s.p2p_status.clone())
}

// GET /v1/bridge/status
async fn bridge_status(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    Json(s.bridge_status.clone())
}

// GET /v1/packages/:canonical
async fn get_package(
    State(state): State<SharedState>,
    Path(canonical): Path<String>,
) -> Response {
    let canonical = urlencoding::decode(&canonical)
        .unwrap_or_default().to_string();
    let s = state.read().await;

    // Check verified chain first.
    if let Ok(Some(record)) = s.chain.get_package(&canonical) {
        #[derive(Serialize)]
        struct PackageResp {
            canonical:         String,
            status:            &'static str,
            block_hash:        Option<String>,
            content_hash:      Option<String>,
            ipfs_cid:          Option<String>,
            publisher:         Option<String>,
            published_at:      Option<String>,
            revocation_reason: Option<String>,
        }
        let resp = PackageResp {
            canonical: record.id.canonical(),
            status: match &record.status {
                PackageStatus::Verified       => "verified",
                PackageStatus::Revoked { .. } => "revoked",
                _                             => "pending",
            },
            block_hash:        Some(record.block_hash.clone()),
            content_hash:      Some(record.content_hash.clone()),
            ipfs_cid:          Some(record.ipfs_cid.clone()),
            publisher:         Some(record.publisher_pubkey.clone()),
            published_at:      Some(record.published_at.to_rfc3339()),
            revocation_reason: if let PackageStatus::Revoked { reason } = &record.status {
                Some(reason.clone())
            } else { None },
        };
        return Json(resp).into_response();
    }

    // Check pending pool.
    if s.pending_pool.contains(&canonical) {
        return Json(serde_json::json!({
            "canonical": canonical,
            "status": "pending"
        })).into_response();
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
        return (StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: format!("Invalid publisher signature: {}", e),
        })).into_response();
    }

    let mut s = state.write().await;

    // Reject if already verified/revoked.
    if let Ok(Some(rec)) = s.chain.get_package(&canonical) {
        if matches!(rec.status, PackageStatus::Verified) {
            return (StatusCode::CONFLICT, Json(ErrorResponse {
                error: format!("{} is already verified on chain", canonical),
            })).into_response();
        }
        if matches!(rec.status, PackageStatus::Revoked { .. }) {
            return (StatusCode::FORBIDDEN, Json(ErrorResponse {
                error: format!("{} is revoked and cannot be resubmitted", canonical),
            })).into_response();
        }
    }

    // ── 3. Broadcast to P2P network ───────────────────────────────────────────
    let gossip_req = common::GossipMessage::PublishRequest(request.clone());
    let _ = s.p2p.sender.send(crate::p2p::P2PCommand::Broadcast {
        topic: "creg/v1/submissions".into(),
        data: serde_json::to_vec(&gossip_req).unwrap_or_default(),
    }).await;

    if !s.pending_pool.insert(request) {
        return (StatusCode::CONFLICT, Json(ErrorResponse {
            error: format!("{} is already pending with the same content hash", canonical),
        })).into_response();
    }
    tracing::info!("{} added to pending pool ({} pending)", canonical, s.pending_pool.len());

    (StatusCode::ACCEPTED, Json(serde_json::json!({
        "status":    "accepted",
        "canonical": canonical,
        "message":   "Package submitted. Validator pipeline will pick it up shortly."
    }))).into_response()
}

// POST /v1/packages/:canonical/revoke
#[derive(Deserialize)]
struct RevokeReq { reason: String }

async fn revoke_package(
    State(state): State<SharedState>,
    Path(canonical): Path<String>,
    Json(req): Json<RevokeReq>,
) -> Response {
    let canonical = urlencoding::decode(&canonical)
        .unwrap_or_default().to_string();
    let s = state.read().await;

    match s.chain.get_package(&canonical) {
        Ok(Some(record)) => {
            let tx = common::Transaction::Revoke {
                package_canonical: canonical.clone(),
                reason:            req.reason.clone(),
                revoked_by:        "api-request".into(),
                evidence_hash:     record.content_hash.clone(),
            };
            // Send directly to finalized-tx channel so the block producer picks it up.
            if s.tx_sender.send(tx).await.is_err() {
                return server_err("Finalized-tx channel closed".to_string());
            }
            events::emit(
                &s.event_bus,
                events::RegistryEvent::package_revoked(&canonical, &req.reason, "api-request"),
            );
            Json(serde_json::json!({
                "status": "queued",
                "message": "Revocation will be included in the next block"
            })).into_response()
        }
        Ok(None)  => not_found(format!("Package not found: {}", canonical)),
        Err(e)    => server_err(e.to_string()),
    }
}

// GET /v1/packages/:canonical/proof  (light-client SPV proof)
async fn get_proof(
    State(state): State<SharedState>,
    Path(canonical): Path<String>,
) -> Response {
    let canonical = urlencoding::decode(&canonical)
        .unwrap_or_default().to_string();
    let s = state.read().await;

    match crate::proof::build_proof(&canonical, &s.chain) {
        Ok(Some(proof)) => Json(proof).into_response(),
        Ok(None)        => not_found(format!("No proof available for: {}", canonical)),
        Err(e)          => server_err(e.to_string()),
    }
}

// GET /v1/blocks/:height
async fn get_block_by_height(
    State(state): State<SharedState>,
    Path(height): Path<u64>,
) -> Response {
    let s = state.read().await;
    match s.chain.get_block_by_height(height) {
        Ok(Some(b)) => Json(b).into_response(),
        Ok(None)    => not_found(format!("No block at height {}", height)),
        Err(e)      => server_err(e.to_string()),
    }
}

// GET /v1/blocks/hash/:hash
async fn get_block_by_hash(
    State(state): State<SharedState>,
    Path(hash): Path<String>,
) -> Response {
    let s = state.read().await;
    match s.chain.get_block_by_hash(&hash) {
        Ok(Some(b)) => Json(b).into_response(),
        Ok(None)    => not_found(format!("No block with hash {}", hash)),
        Err(e)      => server_err(e.to_string()),
    }
}

// POST /v1/blocks/announce
async fn receive_block_announcement(
    State(_state): State<SharedState>,
    Json(ann): Json<crate::gossip::BlockAnnouncement>,
) -> impl IntoResponse {
    tracing::debug!(
        "Block announcement: height={} hash={}",
        ann.height, &ann.block_hash[..std::cmp::min(12, ann.block_hash.len())]
    );
    Json(serde_json::json!({ "status": "noted" }))
}

// GET /v1/publishers/:pubkey
async fn get_publisher(
    State(state): State<SharedState>,
    Path(pubkey): Path<String>,
) -> Response {
    let s = state.read().await;
    match s.publisher_index.get(&pubkey) {
        Some(stats) => Json(stats.clone()).into_response(),
        None        => not_found(format!("Publisher not found: {}", pubkey)),
    }
}

// GET /v1/pending
async fn list_pending(State(state): State<SharedState>) -> impl IntoResponse {
    let s = state.read().await;
    Json(serde_json::json!({
        "count":    s.pending_pool.len(),
        "packages": s.pending_pool.all_canonicals()
    }))
}

// POST /v1/consensus/vote
#[derive(Deserialize, Serialize)]
pub struct VoteMessage {
    /// The canonical package ID or block hash this vote is for.
    pub block_hash:     String,
    pub validator_id:   String,
    pub phase:          String,
    /// Hex-encoded Ed25519 signature of "<block_hash>:<approved>" by the validator.
    pub signature:      String,
    /// Hex-encoded Ed25519 public key of the voting validator.
    pub validator_pubkey: String,
    pub approved:       bool,
    pub reject_reason:  Option<String>,
}

async fn receive_vote(
    State(state): State<SharedState>,
    Json(vote): Json<VoteMessage>,
) -> Response {
    // ── Authenticate: verify the vote is from a known validator ──────────────
    {
        use ed25519_dalek::{VerifyingKey, Signature, Verifier};

        let s = state.read().await;

        // 1. Check the claimed validator is in the active validator set.
        let is_known = s.validator_set.validators.iter()
            .any(|v| v.id == vote.validator_id);
        if !is_known {
            return (StatusCode::FORBIDDEN, Json(ErrorResponse {
                error: format!("Unknown validator: {}", vote.validator_id),
            })).into_response();
        }

        // 2. Verify the Ed25519 signature.
        let pubkey_bytes = match hex::decode(&vote.validator_pubkey) {
            Ok(b) => b,
            Err(_) => return (StatusCode::BAD_REQUEST, Json(ErrorResponse {
                error: "Invalid validator_pubkey hex".into(),
            })).into_response(),
        };
        let vk = match VerifyingKey::try_from(pubkey_bytes.as_slice()) {
            Ok(k) => k,
            Err(_) => return (StatusCode::BAD_REQUEST, Json(ErrorResponse {
                error: "Invalid Ed25519 public key".into(),
            })).into_response(),
        };
        let sig_bytes = match hex::decode(&vote.signature) {
            Ok(b) => b,
            Err(_) => return (StatusCode::BAD_REQUEST, Json(ErrorResponse {
                error: "Invalid signature hex".into(),
            })).into_response(),
        };
        let sig = match Signature::try_from(sig_bytes.as_slice()) {
            Ok(s) => s,
            Err(_) => return (StatusCode::BAD_REQUEST, Json(ErrorResponse {
                error: "Invalid Ed25519 signature format".into(),
            })).into_response(),
        };

        // The signed message must match what validator_pipeline.rs produces:
        // "<canonical>-<content_hash>"  (for package consensus votes)
        // We sign "block_hash:approved" for vote messages.
        let msg = format!("{}:{}", vote.block_hash, vote.approved);
        if let Err(_) = vk.verify(msg.as_bytes(), &sig) {
            return (StatusCode::UNAUTHORIZED, Json(ErrorResponse {
                error: "Vote signature verification failed".into(),
            })).into_response();
        }
    }

    let mut s = state.write().await;

    let sig = common::ValidatorSignature {
        validator_id:     vote.validator_id.clone(),
        validator_pubkey: vote.validator_pubkey.clone(),
        signature:        vote.signature.clone(),
        vote: if vote.approved {
            common::ValidatorVote::Approve
        } else {
            common::ValidatorVote::Reject { reason: vote.reject_reason.clone().unwrap_or_default() }
        },
        signed_at: chrono::Utc::now(),
    };

    let key = vote.block_hash.clone();
    s.votes.entry(key).or_insert_with(Vec::new).push(sig);

    events::emit(&s.event_bus, events::RegistryEvent::validator_voted(
        &vote.validator_id, &vote.block_hash, vote.approved,
    ));

    Json(serde_json::json!({ "status": "accepted" })).into_response()
}

// ─── Appeals & AAA ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AuditSubmission {
    pub approved:   bool,
    pub proof:      String,
    pub rationales: Vec<validator::report::Rationale>,
}

async fn submit_audit(
    Path(id): Path<u64>,
    Json(audit): Json<AuditSubmission>,
) -> impl IntoResponse {
    tracing::info!("Received AAA audit for appeal {}: approved={}", id, audit.approved);
    
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
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

// ─── Publisher signature verification ────────────────────────────────────────

fn verify_publish_sig(req: &PublishRequest) -> anyhow::Result<()> {
    use ed25519_dalek::{VerifyingKey, Signature, Verifier};
    let pubkey_bytes = hex::decode(&req.publisher_pubkey)?;
    let sig_bytes    = hex::decode(&req.signature)?;
    let vk = VerifyingKey::try_from(pubkey_bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("Invalid Ed25519 public key"))?;
    let sig = Signature::try_from(sig_bytes.as_slice())
        .map_err(|_| anyhow::anyhow!("Invalid Ed25519 signature"))?;
    let msg = format!("{}{}", req.id.canonical(), req.content_hash);
    vk.verify(msg.as_bytes(), &sig)
        .map_err(|_| anyhow::anyhow!("Signature verification failed"))
}



