//! Zone 1 Unprivileged HTTP DMZ Gateway (`creg-gateway`, `ADR-018` Increment 6).
//!
//! Compile-time `INV-01`: this binary crate has ZERO dependencies on consensus,
//! archive unpacking, sandboxes, Vault/secp256k1 secrets, or LLM providers.
//! Every request crosses an explicit authenticated `zone-ipc` frame into Zone 2.

use anyhow::{Context, Result};
use axum::body::{to_bytes, Body};
use axum::extract::{ConnectInfo, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use zone_ipc::{
    decode_b64, encode_b64, enforce_zone_forbidden_env, ZoneEndpoint, ZoneId, ZoneIpcClient,
    ZoneIpcToken, ZoneRequest, ZoneResponse, GATEWAY_FORBIDDEN_ENV, ZONE_IPC_TOKEN_ENV,
};

pub const PUBLIC_BODY_LIMIT_BYTES: usize = 1024 * 1024;
pub const ADMIN_BODY_LIMIT_BYTES: usize = 256 * 1024;
pub const DEFAULT_RATE_LIMIT_MAX_REQUESTS: usize = 120;
pub const DEFAULT_RATE_LIMIT_WINDOW_SECS: u64 = 60;

#[derive(Clone)]
pub struct IpRateLimiter {
    max_requests: usize,
    window: Duration,
    buckets: Arc<Mutex<HashMap<IpAddr, VecDeque<Instant>>>>,
}

impl IpRateLimiter {
    pub fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests: max_requests.max(1),
            window,
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn allow(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut guard = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let q = guard.entry(ip).or_default();
        while let Some(&front) = q.front() {
            if now.saturating_duration_since(front) >= self.window {
                q.pop_front();
            } else {
                break;
            }
        }
        if q.len() >= self.max_requests {
            return false;
        }
        q.push_back(now);
        true
    }
}

#[derive(Clone)]
pub struct GatewayState {
    pub ipc_client: ZoneIpcClient,
    pub admin_token: String,
    pub rate_limiter: IpRateLimiter,
    pub next_request_id: Arc<AtomicU64>,
}

impl GatewayState {
    pub fn new(
        ipc_client: ZoneIpcClient,
        admin_token: impl Into<String>,
        rate_limiter: IpRateLimiter,
    ) -> Self {
        Self {
            ipc_client,
            admin_token: admin_token.into(),
            rate_limiter,
            next_request_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn next_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }
}

fn client_ip_from_request(req: &Request<Body>) -> IpAddr {
    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip();
    }
    if let Some(forwarded) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(first) = forwarded.split(',').next() {
            if let Ok(ip) = first.trim().parse::<IpAddr>() {
                return ip;
            }
        }
    }
    IpAddr::from([127, 0, 0, 1])
}

async fn rate_limit_middleware(
    State(state): State<Arc<GatewayState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = client_ip_from_request(&req);
    if !state.rate_limiter.allow(ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": "rate_limit_exceeded",
                "message": "Too many requests from client IP"
            })),
        )
            .into_response();
    }
    next.run(req).await
}

async fn admin_bearer_auth_middleware(
    State(state): State<Arc<GatewayState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let expected = state.admin_token.as_bytes();
    if expected.is_empty() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "admin_token_unconfigured" })),
        )
            .into_response();
    }
    let supplied = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::as_bytes);

    let authorized = matches!(supplied, Some(tok) if tok.len() == expected.len() && bool::from(tok.ct_eq(expected)));
    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response();
    }
    next.run(req).await
}

async fn forward_public_http(
    State(state): State<Arc<GatewayState>>,
    req: Request<Body>,
) -> Response {
    forward_http_over_ipc(state, req, false, PUBLIC_BODY_LIMIT_BYTES).await
}

async fn forward_admin_http(
    State(state): State<Arc<GatewayState>>,
    req: Request<Body>,
) -> Response {
    forward_http_over_ipc(state, req, true, ADMIN_BODY_LIMIT_BYTES).await
}

async fn forward_http_over_ipc(
    state: Arc<GatewayState>,
    req: Request<Body>,
    is_admin: bool,
    max_body_bytes: usize,
) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| q.to_string());
    let body_bytes = match to_bytes(req.into_body(), max_body_bytes).await {
        Ok(b) => b,
        Err(err) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({
                    "error": "payload_too_large",
                    "message": err.to_string()
                })),
            )
                .into_response();
        }
    };
    let body_b64 = encode_b64(&body_bytes);
    let ipc_req = if is_admin {
        ZoneRequest::ForwardAdminHttp {
            method,
            path,
            query,
            body_b64,
        }
    } else {
        ZoneRequest::ForwardPublicHttp {
            method,
            path,
            query,
            body_b64,
        }
    };

    match state.ipc_client.call(state.next_id(), &ipc_req).await {
        Ok(ZoneResponse::HttpReply {
            status,
            content_type,
            body_b64,
        }) => {
            let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            match decode_b64(&body_b64) {
                Ok(raw) => Response::builder()
                    .status(status_code)
                    .header(CONTENT_TYPE, content_type)
                    .body(Body::from(raw))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
                Err(err) => (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({
                        "error": "invalid_upstream_encoding",
                        "message": err.to_string()
                    })),
                )
                    .into_response(),
            }
        }
        Ok(ZoneResponse::Error { code, message }) => {
            let status_code = StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_GATEWAY);
            (
                status_code,
                Json(serde_json::json!({ "error": "zone_error", "message": message })),
            )
                .into_response()
        }
        Ok(_) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "unexpected_zone_response" })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": "consensus_zone_unreachable",
                "message": err.to_string()
            })),
        )
            .into_response(),
    }
}

/// Public Zone 1 router (DMZ). Explicitly mounts only `/v1/public/*` and
/// `/v1/publisher/*` with strict 1 MiB body limit and per-IP rate limiting.
pub fn public_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .route("/v1/public/health", get(forward_public_http))
        .route("/v1/public/*path", any(forward_public_http))
        .route("/v1/publisher/*path", any(forward_public_http))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            rate_limit_middleware,
        ))
        .layer(RequestBodyLimitLayer::new(PUBLIC_BODY_LIMIT_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Dedicated Zone 1 loopback admin router. Requires constant-time Bearer token
/// matching `CREG_GATEWAY_ADMIN_TOKEN`.
pub fn admin_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .route("/metrics", get(forward_admin_http))
        .route("/v1/operator/*path", any(forward_admin_http))
        .route("/v1/internal/*path", any(forward_admin_http))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            admin_bearer_auth_middleware,
        ))
        .layer(RequestBodyLimitLayer::new(ADMIN_BODY_LIMIT_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Runtime INV-01 check: Zone 1 refuses startup if any validator key, Vault
    // secret, or LLM API key was passed in the process environment.
    enforce_zone_forbidden_env(ZoneId::Gateway, GATEWAY_FORBIDDEN_ENV)?;

    let ipc_token = ZoneIpcToken::from_env(ZONE_IPC_TOKEN_ENV)?;
    let consensus_addr =
        std::env::var("CREG_CONSENSUS_IPC_ADDR").unwrap_or_else(|_| "127.0.0.1:19002".to_string());
    let endpoint = ZoneEndpoint::parse(&consensus_addr)?;

    let public_listen: SocketAddr = std::env::var("CREG_GATEWAY_PUBLIC_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()
        .context("invalid CREG_GATEWAY_PUBLIC_LISTEN")?;
    let admin_listen: SocketAddr = std::env::var("CREG_GATEWAY_ADMIN_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:18080".to_string())
        .parse()
        .context("invalid CREG_GATEWAY_ADMIN_LISTEN")?;
    if !admin_listen.ip().is_loopback()
        && !std::env::var("CREG_GATEWAY_ALLOW_NONLOOPBACK_ADMIN")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    {
        anyhow::bail!(
            "CREG_GATEWAY_ADMIN_LISTEN '{}' must bind loopback unless explicitly opted in",
            admin_listen
        );
    }

    let admin_token = std::env::var("CREG_GATEWAY_ADMIN_TOKEN")
        .unwrap_or_else(|_| ZoneIpcToken::generate().to_hex());

    let client = ZoneIpcClient::new(ZoneId::Gateway, ZoneId::Consensus, endpoint, ipc_token);
    let rate_limit_max = std::env::var("CREG_GATEWAY_RATE_LIMIT_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RATE_LIMIT_MAX_REQUESTS);
    let state = Arc::new(GatewayState::new(
        client,
        admin_token,
        IpRateLimiter::new(
            rate_limit_max,
            Duration::from_secs(DEFAULT_RATE_LIMIT_WINDOW_SECS),
        ),
    ));

    let public_app = public_router(Arc::clone(&state));
    let admin_app = admin_router(state);

    let public_listener = TcpListener::bind(public_listen)
        .await
        .with_context(|| format!("bind gateway public DMZ listener {public_listen}"))?;
    let admin_listener = TcpListener::bind(admin_listen)
        .await
        .with_context(|| format!("bind gateway loopback admin listener {admin_listen}"))?;

    tracing::info!(
        public = %public_listen,
        admin = %admin_listen,
        consensus_ipc = %consensus_addr,
        "Zone 1 creg-gateway online (INV-01 enforced)"
    );

    tokio::try_join!(
        async {
            axum::serve(
                public_listener,
                public_app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .context("gateway public server failed")
        },
        async {
            axum::serve(admin_listener, admin_app)
                .await
                .context("gateway admin server failed")
        }
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Method;
    use tower::ServiceExt;
    use zone_ipc::{check_forbidden_env_map, ZoneIpcServer};

    async fn spawn_mock_consensus_ipc(token: ZoneIpcToken) -> ZoneEndpoint {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let server = ZoneIpcServer::bind(ZoneId::Consensus, ZoneEndpoint::Tcp(addr), token)
            .await
            .unwrap();
        let ep = server.local_endpoint().unwrap();
        tokio::spawn(server.serve(|_sender, req| {
            Box::pin(async move {
                match req {
                    ZoneRequest::ForwardPublicHttp { method, path, .. } => {
                        let body = serde_json::json!({
                            "zone": "consensus",
                            "channel": "public",
                            "method": method,
                            "path": path,
                        })
                        .to_string();
                        ZoneResponse::HttpReply {
                            status: 200,
                            content_type: "application/json".into(),
                            body_b64: encode_b64(body.as_bytes()),
                        }
                    }
                    ZoneRequest::ForwardAdminHttp { method, path, .. } => {
                        let body = serde_json::json!({
                            "zone": "consensus",
                            "channel": "admin",
                            "method": method,
                            "path": path,
                        })
                        .to_string();
                        ZoneResponse::HttpReply {
                            status: 200,
                            content_type: "application/json".into(),
                            body_b64: encode_b64(body.as_bytes()),
                        }
                    }
                    _ => ZoneResponse::Error {
                        code: 400,
                        message: "unexpected op".into(),
                    },
                }
            })
        }));
        ep
    }

    #[test]
    fn inv_01_env_guard_blocks_forbidden_secrets() {
        let clean = [("CREG_ZONE_IPC_TOKEN", "00".repeat(32))];
        assert!(check_forbidden_env_map(
            ZoneId::Gateway,
            &[(clean[0].0, clean[0].1.as_str())],
            GATEWAY_FORBIDDEN_ENV
        )
        .is_ok());

        for &secret_var in GATEWAY_FORBIDDEN_ENV {
            let poisoned = [(secret_var, "secret-value")];
            assert!(
                check_forbidden_env_map(ZoneId::Gateway, &poisoned, GATEWAY_FORBIDDEN_ENV).is_err(),
                "gateway must fail closed when {secret_var} is set"
            );
        }
    }

    #[tokio::test]
    async fn public_dmz_enforces_limits_and_hides_admin_routes() {
        let token = ZoneIpcToken::generate();
        let ep = spawn_mock_consensus_ipc(token.clone()).await;
        let client = ZoneIpcClient::new(ZoneId::Gateway, ZoneId::Consensus, ep, token);
        let state = Arc::new(GatewayState::new(
            client,
            "super-secret-admin-token",
            IpRateLimiter::new(2, Duration::from_secs(60)),
        ));
        let app = public_router(state);

        // Public health forwards cleanly over Zone IPC.
        let health_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/public/health")
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health_resp.status(), StatusCode::OK);

        // Admin/internal endpoints MUST NOT exist on the public DMZ router (404).
        let metrics_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/metrics")
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(metrics_resp.status(), StatusCode::NOT_FOUND);

        // 3rd request within window triggers per-IP rate limiting (429).
        let limited_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/public/packages")
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(limited_resp.status(), StatusCode::TOO_MANY_REQUESTS);

        // Request exceeding 1 MiB body limit is rejected (413).
        let oversized = vec![b'x'; PUBLIC_BODY_LIMIT_BYTES + 1];
        let payload_resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/publisher/packages")
                    .header("x-forwarded-for", "203.0.113.11")
                    .body(Body::from(oversized))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(payload_resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn admin_router_requires_constant_time_bearer_and_forwards_ipc() {
        let token = ZoneIpcToken::generate();
        let ep = spawn_mock_consensus_ipc(token.clone()).await;
        let client = ZoneIpcClient::new(ZoneId::Gateway, ZoneId::Consensus, ep, token);
        let state = Arc::new(GatewayState::new(
            client,
            "admin-token-42",
            IpRateLimiter::new(20, Duration::from_secs(60)),
        ));
        let app = admin_router(state);

        // Missing auth -> 401.
        let unauth = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        // Wrong token -> 401.
        let wrong = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/operator/nodes")
                    .header(AUTHORIZATION, "Bearer admin-token-99")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

        // Exact token -> 200 forwarded to Zone 2 consensus.
        let ok = app
            .oneshot(
                Request::builder()
                    .uri("/v1/operator/nodes")
                    .header(AUTHORIZATION, "Bearer admin-token-42")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
        let raw = to_bytes(ok.into_body(), 16 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(json["zone"], "consensus");
        assert_eq!(json["channel"], "admin");
        assert_eq!(json["path"], "/v1/operator/nodes");
    }
}
