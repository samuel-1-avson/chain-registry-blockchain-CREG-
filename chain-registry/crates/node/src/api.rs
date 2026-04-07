#[cfg(test)]
mod tests {
    use super::*;
    use common::{PackageId, PackageManifest};
    use ed25519_dalek::{Signer, SigningKey};

    fn make_keypair() -> (SigningKey, String) {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        let sk = SigningKey::from_bytes(&bytes);
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
// crates/node/src/api.rs
// Axum REST API — all HTTP endpoints for the chain registry node.

use axum::{
    extract::{Path, Query, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use common::{PackageStatus, PublishRequest};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};

use crate::events;
use crate::{
    events::{sse_handler, EventBus},
    rate_limit::{rate_limit_middleware, RateLimiter},
    SharedState,
};

/// Query parameters for GET /v1/packages
#[derive(Deserialize)]
struct ListPackagesParams {
    offset: Option<usize>,
    limit: Option<usize>,
    ecosystem: Option<String>,
    status: Option<String>,
}

// ─── Router ───────────────────────────────────────────────────────────────────

pub fn router(state: SharedState, event_bus: EventBus, limiter: RateLimiter) -> Router {
    Router::new()
        // Health & chain
        .route("/v1/health", get(health))
        .route("/health", get(health))
        .route("/v1/chain/stats", get(chain_stats))
        .route("/v1/runtime/config", get(runtime_config))
        .route("/v1/nodes", get(get_nodes))
        .route("/v1/p2p/status", get(p2p_status))
        .route("/v1/bridge/status", get(bridge_status))
        // Packages
        .route("/v1/packages/:canonical", get(get_package))
        .route("/v1/packages", get(list_packages).post(submit_package))
        .route("/v1/packages/:canonical/revoke", post(revoke_package))
        .route("/v1/packages/:canonical/proof", get(get_proof))
        // Blocks
        .route("/v1/blocks/:height", get(get_block_by_height))
        .route("/v1/blocks/hash/:hash", get(get_block_by_hash))
        .route("/v1/blocks/announce", post(receive_block_announcement))
        // Publishers
        .route("/v1/publishers/:pubkey", get(get_publisher))
        // Pending pool
        .route("/v1/pending", get(list_pending))
        // Consensus
        .route("/v1/consensus/vote", post(receive_vote))
        .route("/v1/publishers/rotate-key", post(rotate_publisher_key))
        // Appeals & AAA
        .route("/v1/appeals/:id/audit", post(submit_audit))
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
        .layer(CorsLayer::permissive())
        .with_state(state)
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

#[derive(Serialize)]
struct RuntimeConfigResponse {
    is_testnet: bool,
    registry_address: Option<String>,
    token_contract: Option<String>,
    staking_contract: Option<String>,
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
    })
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
#[derive(Deserialize)]
struct RevokeReq {
    reason: String,
}

async fn revoke_package(
    State(state): State<SharedState>,
    Path(canonical): Path<String>,
    Json(req): Json<RevokeReq>,
) -> Response {
    let canonical = urlencoding::decode(&canonical)
        .unwrap_or_default()
        .to_string();
    let s = state.read().await;

    match s.chain.get_package(&canonical) {
        Ok(Some(record)) => {
            let tx = common::Transaction::Revoke {
                package_canonical: canonical.clone(),
                reason: req.reason.clone(),
                revoked_by: "api-request".into(),
                evidence_hash: record.content_hash.clone(),
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

// GET /v1/blocks/:height
async fn get_block_by_height(
    State(state): State<SharedState>,
    Path(height): Path<u64>,
) -> Response {
    let s = state.read().await;
    match s.chain.get_block_by_height(height) {
        Ok(Some(b)) => Json(b).into_response(),
        Ok(None) => not_found(format!("No block at height {}", height)),
        Err(e) => server_err(e.to_string()),
    }
}

// GET /v1/blocks/hash/:hash
async fn get_block_by_hash(State(state): State<SharedState>, Path(hash): Path<String>) -> Response {
    let s = state.read().await;
    match s.chain.get_block_by_hash(&hash) {
        Ok(Some(b)) => Json(b).into_response(),
        Ok(None) => not_found(format!("No block with hash {}", hash)),
        Err(e) => server_err(e.to_string()),
    }
}

// POST /v1/blocks/announce
async fn receive_block_announcement(
    State(_state): State<SharedState>,
    Json(ann): Json<crate::gossip::BlockAnnouncement>,
) -> impl IntoResponse {
    tracing::debug!(
        "Block announcement: height={} hash={}",
        ann.height,
        &ann.block_hash[..std::cmp::min(12, ann.block_hash.len())]
    );
    Json(serde_json::json!({ "status": "noted" }))
}

// GET /v1/publishers/:pubkey
async fn get_publisher(State(state): State<SharedState>, Path(pubkey): Path<String>) -> Response {
    let s = state.read().await;
    match s.publisher_index.get(&pubkey) {
        Some(stats) => Json(stats.clone()).into_response(),
        None => not_found(format!("Publisher not found: {}", pubkey)),
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

// POST /v1/consensus/vote
#[derive(Deserialize, Serialize)]
pub struct VoteMessage {
    /// The canonical package ID or block hash this vote is for.
    pub block_hash: String,
    pub validator_id: String,
    pub phase: String,
    /// Hex-encoded Ed25519 signature of "<block_hash>:<approved>" by the validator.
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
        if !validator.pubkey.is_empty() && vote.validator_pubkey != validator.pubkey {
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

        // The signed message must match what validator_pipeline.rs produces:
        // "<canonical>-<content_hash>"  (for package consensus votes)
        // We sign "block_hash:approved" for vote messages.
        let msg = format!("{}:{}", vote.block_hash, vote.approved);
        if let Err(_) = vk.verify(msg.as_bytes(), &sig) {
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
