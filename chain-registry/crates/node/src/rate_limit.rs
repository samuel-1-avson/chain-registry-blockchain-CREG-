#![allow(dead_code)]
// crates/node/src/rate_limit.rs
// In-memory sliding-window rate limiter for the REST and JSON-RPC API.
//
// Each request class has its own per-client bucket (defaults, per 60 s window):
//   - Package submissions (POST /v1/packages):           10
//   - Namespace-authority attestations:                    2
//   - Validator vote submissions:                         60
//   - JSON-RPC calls (POST /rpc, /jsonrpc):              600  (CREG_RATE_LIMIT_RPC)
//   - Other mutating routes (key rotation, registration):  30
//   - General API requests:                             3000
//
// A bucket holds the timestamps of recent requests; expired ones are dropped
// on access and by the background purge task. IPv6 clients are keyed by their
// /64. The client address comes from X-Forwarded-For only when the TCP peer
// matches CREG_TRUSTED_PROXIES (addresses or CIDR networks, `VNA-08`).

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_TRACKED_IPS: usize = 100_000;
/// Default JSON-RPC calls per client per window. Every native `creg_*` method
/// is a read, so this is far looser than the mutating-route limit.
const DEFAULT_RPC_LIMIT: u32 = 600;
/// JSON-RPC 2.0 error code for "limit exceeded" (EIP-1474), used by
/// Ethereum-style clients to recognise throttling.
const JSON_RPC_LIMIT_EXCEEDED: i64 = -32005;

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Max requests per window for general read endpoints.
    pub general_limit: u32,
    /// Max requests per window for mutating non-publish endpoints (`VNA-08`).
    pub mutate_limit: u32,
    /// Max submissions per window.
    pub publish_limit: u32,
    /// Max votes per window.
    pub vote_limit: u32,
    /// Max expensive namespace-authority verifier runs per window.
    pub authority_limit: u32,
    /// Max JSON-RPC calls per window. A batch costs one call per member.
    pub rpc_limit: u32,
    /// Sliding window duration.
    pub window: Duration,
    /// Proxies allowed to supply `X-Forwarded-For` (`VNA-08`).
    pub trusted_proxies: Vec<TrustedProxy>,
}

/// One `CREG_TRUSTED_PROXIES` entry: a single address or a CIDR network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustedProxy {
    network: IpAddr,
    prefix: u8,
}

impl TrustedProxy {
    /// Trust exactly one address.
    pub fn host(ip: IpAddr) -> Self {
        let ip = canonical_ip(ip);
        Self {
            network: ip,
            prefix: max_prefix(ip),
        }
    }

    /// Parse `10.0.0.5`, `172.18.0.0/16` or `fd00::/8`. Returns `None` for
    /// anything else, including a prefix longer than the address family allows.
    pub fn parse(raw: &str) -> Option<Self> {
        let (address, prefix) = match raw.trim().split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (raw.trim(), None),
        };
        let ip = canonical_ip(address.trim().parse::<IpAddr>().ok()?);
        let max = max_prefix(ip);
        let prefix = match prefix {
            Some(prefix) => prefix.trim().parse::<u8>().ok().filter(|p| *p <= max)?,
            None => max,
        };
        Some(Self {
            network: mask(ip, prefix),
            prefix,
        })
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let ip = canonical_ip(ip);
        self.network.is_ipv4() == ip.is_ipv4() && mask(ip, self.prefix) == self.network
    }
}

/// Treat an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) as the IPv4 address it carries.
fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

fn max_prefix(ip: IpAddr) -> u8 {
    if ip.is_ipv4() {
        32
    } else {
        128
    }
}

/// Zero every bit after the first `prefix` bits. `prefix` must not exceed the
/// family's width.
fn mask(ip: IpAddr, prefix: u8) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            let kept = if prefix == 0 {
                0
            } else {
                bits & (u32::MAX << (32 - u32::from(prefix)))
            };
            IpAddr::V4(Ipv4Addr::from(kept))
        }
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            let kept = if prefix == 0 {
                0
            } else {
                bits & (u128::MAX << (128 - u32::from(prefix)))
            };
            IpAddr::V6(Ipv6Addr::from(kept))
        }
    }
}

/// Bucket key for a client: IPv4 as is, IPv6 by its /64. A single site is
/// normally handed at least a /64, so rotating addresses inside it must not
/// buy fresh buckets.
fn bucket_key(ip: IpAddr) -> IpAddr {
    let ip = canonical_ip(ip);
    if ip.is_ipv6() {
        mask(ip, 64)
    } else {
        ip
    }
}

/// Parse a comma-separated `CREG_TRUSTED_PROXIES` value. Invalid entries are
/// skipped with a warning, because silently dropping one would quietly put
/// every client behind that proxy into one shared bucket.
pub fn parse_trusted_proxies(raw: &str) -> Vec<TrustedProxy> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let parsed = TrustedProxy::parse(entry);
            if parsed.is_none() {
                tracing::warn!(
                    entry,
                    "ignoring invalid CREG_TRUSTED_PROXIES entry (expected an IP address or CIDR network)"
                );
            }
            parsed
        })
        .collect()
}

fn trusted_proxies_from_env() -> Vec<TrustedProxy> {
    parse_trusted_proxies(&std::env::var("CREG_TRUSTED_PROXIES").unwrap_or_default())
}

fn rpc_limit_from_env() -> u32 {
    match std::env::var("CREG_RATE_LIMIT_RPC") {
        Err(_) => DEFAULT_RPC_LIMIT,
        Ok(raw) => match raw.trim().parse::<u32>() {
            Ok(limit) if limit > 0 => limit,
            _ => {
                tracing::warn!(
                    value = raw.as_str(),
                    default = DEFAULT_RPC_LIMIT,
                    "CREG_RATE_LIMIT_RPC must be a positive integer; using the default"
                );
                DEFAULT_RPC_LIMIT
            }
        },
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            // The explorer polls several read-only endpoints concurrently and may
            // be opened in multiple tabs during local testnet use. Keep mutating
            // routes tight, but give the general read surface enough headroom to
            // avoid throttling the first-party UI and local tooling.
            general_limit: 3000,
            mutate_limit: 30,
            publish_limit: 10,
            vote_limit: 60,
            authority_limit: 2,
            rpc_limit: rpc_limit_from_env(),
            window: Duration::from_secs(60),
            trusted_proxies: trusted_proxies_from_env(),
        }
    }
}

/// Per-client sliding window bucket.
#[derive(Debug)]
struct Bucket {
    /// Timestamps of recent requests within the window.
    timestamps: Vec<Instant>,
}

impl Bucket {
    fn new() -> Self {
        Self {
            timestamps: Vec::new(),
        }
    }

    /// Drop timestamps older than the window, then take `weight` slots if they
    /// all fit. Either every slot is taken or none is.
    fn try_take(&mut self, window: Duration, limit: u32, weight: usize) -> bool {
        let now = Instant::now();
        if let Some(cutoff) = now.checked_sub(window) {
            self.timestamps.retain(|&t| t > cutoff);
        }
        if self.timestamps.len() + weight <= limit as usize {
            self.timestamps.extend(std::iter::repeat_n(now, weight));
            true
        } else {
            false
        }
    }
}

/// Lock a mutex, recovering gracefully if it was poisoned.
fn lock_buckets<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("Rate limiter mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

type Buckets = Arc<Mutex<HashMap<IpAddr, Bucket>>>;

/// Shared rate limit state (client key → bucket per endpoint class).
#[derive(Clone)]
pub struct RateLimiter {
    general: Buckets,
    mutate: Buckets,
    publish: Buckets,
    vote: Buckets,
    authority: Buckets,
    rpc: Buckets,
    config: RateLimitConfig,
}

impl RateLimiter {
    /// Proxies whose `X-Forwarded-For` is honoured (for [`extract_client_ip`]).
    pub fn trusted_proxies(&self) -> &[TrustedProxy] {
        &self.config.trusted_proxies
    }

    /// Length of the sliding window, used for `Retry-After`.
    pub fn window(&self) -> Duration {
        self.config.window
    }

    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            general: Arc::new(Mutex::new(HashMap::new())),
            mutate: Arc::new(Mutex::new(HashMap::new())),
            publish: Arc::new(Mutex::new(HashMap::new())),
            vote: Arc::new(Mutex::new(HashMap::new())),
            authority: Arc::new(Mutex::new(HashMap::new())),
            rpc: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    fn check(&self, buckets: &Buckets, ip: IpAddr, limit: u32, weight: usize) -> bool {
        let key = bucket_key(ip);
        let mut map = lock_buckets(buckets);
        if !map.contains_key(&key) && map.len() >= MAX_TRACKED_IPS {
            // Evict one bucket instead of refusing the new client. Refusing
            // would let anyone with enough source addresses lock every new
            // client out; evicting at worst restarts one client's window.
            // Expired buckets are removed by the purge task, so this stays
            // O(1) even under an address-rotation flood.
            let victim = map.keys().next().copied();
            if let Some(victim) = victim {
                map.remove(&victim);
            }
        }
        map.entry(key)
            .or_insert_with(Bucket::new)
            .try_take(self.config.window, limit, weight)
    }

    pub fn check_general(&self, ip: IpAddr) -> bool {
        self.check(&self.general, ip, self.config.general_limit, 1)
    }

    pub fn check_mutate(&self, ip: IpAddr) -> bool {
        self.check(&self.mutate, ip, self.config.mutate_limit, 1)
    }

    pub fn check_publish(&self, ip: IpAddr) -> bool {
        self.check(&self.publish, ip, self.config.publish_limit, 1)
    }

    pub fn check_vote(&self, ip: IpAddr) -> bool {
        self.check(&self.vote, ip, self.config.vote_limit, 1)
    }

    pub fn check_authority(&self, ip: IpAddr) -> bool {
        self.check(&self.authority, ip, self.config.authority_limit, 1)
    }

    /// One JSON-RPC HTTP request (charged by the middleware).
    pub fn check_rpc(&self, ip: IpAddr) -> bool {
        self.check(&self.rpc, ip, self.config.rpc_limit, 1)
    }

    /// Charge `calls` further JSON-RPC calls to `ip`: the members of a batch
    /// beyond the one the middleware already charged. All or nothing.
    pub fn charge_rpc(&self, ip: IpAddr, calls: usize) -> bool {
        self.check(&self.rpc, ip, self.config.rpc_limit, calls)
    }

    /// Purge expired entries from all buckets (call periodically).
    pub fn purge_expired(&self) {
        let now = Instant::now();
        let window = self.config.window;
        for buckets in [
            &self.general,
            &self.mutate,
            &self.publish,
            &self.vote,
            &self.authority,
            &self.rpc,
        ] {
            let mut map = lock_buckets(buckets);
            map.retain(|_, bucket| {
                bucket.timestamps.retain(|&t| now - t < window);
                !bucket.timestamps.is_empty()
            });
        }
    }
}

#[derive(Serialize)]
struct RateLimitError {
    error: &'static str,
    retry_after: u64,
}

/// HTTP 429 whose body is a JSON-RPC 2.0 error, so JSON-RPC clients can parse
/// it instead of failing on a REST-shaped body.
pub fn json_rpc_rate_limited(window: Duration) -> Response {
    let retry_after = window.as_secs().max(1);
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(axum::http::header::RETRY_AFTER, retry_after.to_string())],
        Json(serde_json::json!({
            "jsonrpc": "2.0",
            "error": {
                "code": JSON_RPC_LIMIT_EXCEEDED,
                "message": "Rate limit exceeded",
                "data": { "retry_after": retry_after },
            },
            "id": null,
        })),
    )
        .into_response()
}

/// POST package submission (legacy and grouped publisher routes).
fn is_publish_submit(path: &str, method: &axum::http::Method) -> bool {
    method == axum::http::Method::POST
        && (path == "/v1/packages" || path == "/v1/publisher/packages")
}

/// Validator consensus vote (legacy and grouped routes).
fn is_consensus_vote(path: &str, method: &axum::http::Method) -> bool {
    method == axum::http::Method::POST
        && (path == "/v1/consensus/vote" || path == "/v1/validator/consensus/vote")
}

fn is_authority_attestation(path: &str, method: &axum::http::Method) -> bool {
    method == axum::http::Method::POST && path == "/v1/validator/namespace-authority/attest"
}

/// Native JSON-RPC endpoint. Every `creg_*` method is a read, so it has its
/// own bucket rather than sharing the tight mutating-route one.
fn is_json_rpc_route(path: &str, method: &axum::http::Method) -> bool {
    method == axum::http::Method::POST && (path == "/rpc" || path == "/jsonrpc")
}

fn is_mutating_sensitive_route(path: &str, method: &axum::http::Method) -> bool {
    if method != axum::http::Method::POST && method != axum::http::Method::DELETE {
        return false;
    }
    path == "/v1/publisher/rotate-key"
        || path == "/v1/publishers/rotate-key"
        || path == "/v1/validator/register"
        || path == "/v1/validators/register"
        || path == "/v1/validator/set-transitions"
        || path.starts_with("/v1/publisher/namespaces/")
        || path.starts_with("/v1/publishers/namespaces/")
        || path.starts_with("/v1/validator/registrations/")
        || path.starts_with("/v1/validators/registrations/")
        || path.ends_with("/revoke")
}

/// Extracts the effective client IP: honors the right-most `X-Forwarded-For`
/// entry that is not itself a trusted proxy, ONLY when `peer_ip` is a trusted
/// proxy (`VNA-08`).
pub fn extract_client_ip(
    peer_ip: IpAddr,
    headers: &axum::http::HeaderMap,
    trusted_proxies: &[TrustedProxy],
) -> IpAddr {
    let is_trusted = |ip: IpAddr| trusted_proxies.iter().any(|proxy| proxy.contains(ip));
    if !is_trusted(peer_ip) {
        return peer_ip;
    }
    let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) else {
        return peer_ip;
    };
    for part in xff.split(',').rev() {
        if let Ok(candidate) = part.trim().parse::<IpAddr>() {
            if !is_trusted(candidate) {
                return candidate;
            }
        }
    }
    peer_ip
}

/// Axum middleware that applies rate limiting based on the request path.
pub async fn rate_limit_middleware(
    limiter: axum::extract::Extension<RateLimiter>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let peer_ip: IpAddr = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let ip = extract_client_ip(peer_ip, req.headers(), &limiter.config.trusted_proxies);

    let path = req.uri().path();
    let method = req.method();
    let json_rpc = is_json_rpc_route(path, method);

    let allowed = if is_authority_attestation(path, method) {
        limiter.check_authority(ip)
    } else if is_publish_submit(path, method) {
        limiter.check_publish(ip)
    } else if is_consensus_vote(path, method) {
        limiter.check_vote(ip)
    } else if json_rpc {
        limiter.check_rpc(ip)
    } else if is_mutating_sensitive_route(path, method) {
        limiter.check_mutate(ip)
    } else {
        limiter.check_general(ip)
    };

    if !allowed {
        if json_rpc {
            return json_rpc_rate_limited(limiter.config.window);
        }
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("Retry-After", "60"), ("X-RateLimit-Window", "60")],
            Json(RateLimitError {
                error: "Rate limit exceeded",
                retry_after: 60,
            }),
        )
            .into_response();
    }

    next.run(req).await
}

/// Spawn a background task that purges expired rate limit entries every minute.
pub fn spawn_purge_task(limiter: RateLimiter) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            limiter.purge_expired();
            tracing::debug!("Rate limiter: purged expired entries");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn allows_requests_within_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            general_limit: 5,
            ..Default::default()
        });
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        for _ in 0..5 {
            assert!(limiter.check_general(ip));
        }
    }

    #[test]
    fn blocks_requests_over_limit() {
        let limiter = RateLimiter::new(RateLimitConfig {
            general_limit: 3,
            ..Default::default()
        });
        let ip = IpAddr::V4(Ipv4Addr::new(5, 6, 7, 8));
        for _ in 0..3 {
            limiter.check_general(ip);
        }
        assert!(!limiter.check_general(ip), "4th request should be blocked");
    }

    #[test]
    fn different_ips_have_independent_limits() {
        let limiter = RateLimiter::new(RateLimitConfig {
            general_limit: 1,
            ..Default::default()
        });
        let ip1 = IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(2, 2, 2, 2));

        assert!(limiter.check_general(ip1));
        assert!(!limiter.check_general(ip1)); // ip1 exhausted
        assert!(limiter.check_general(ip2)); // ip2 still fresh
    }

    #[test]
    fn publish_and_general_limits_are_independent() {
        let limiter = RateLimiter::new(RateLimitConfig {
            general_limit: 100,
            publish_limit: 1,
            ..Default::default()
        });
        let ip = IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9));

        assert!(limiter.check_publish(ip));
        assert!(!limiter.check_publish(ip)); // publish exhausted
        assert!(limiter.check_general(ip)); // general still has room
    }

    #[test]
    fn publish_path_detection_covers_grouped_route() {
        use axum::http::Method;
        assert!(is_publish_submit("/v1/publisher/packages", &Method::POST));
        assert!(is_publish_submit("/v1/packages", &Method::POST));
        assert!(!is_publish_submit("/v1/publisher/packages", &Method::GET));
    }

    #[test]
    fn vote_path_detection_covers_grouped_route() {
        use axum::http::Method;
        assert!(is_consensus_vote(
            "/v1/validator/consensus/vote",
            &Method::POST
        ));
        assert!(is_consensus_vote("/v1/consensus/vote", &Method::POST));
        assert!(!is_consensus_vote(
            "/v1/validator/consensus/vote",
            &Method::GET
        ));
    }

    #[test]
    fn authority_attestation_has_a_dedicated_tight_bucket() {
        use axum::http::Method;
        let limiter = RateLimiter::new(RateLimitConfig {
            authority_limit: 1,
            ..Default::default()
        });
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert!(is_authority_attestation(
            "/v1/validator/namespace-authority/attest",
            &Method::POST
        ));
        assert!(limiter.check_authority(ip));
        assert!(!limiter.check_authority(ip));
        assert!(limiter.check_general(ip));
    }

    #[test]
    fn sensitive_mutating_routes_and_trusted_proxy_xff_work_safely() {
        use axum::http::{HeaderMap, HeaderValue, Method};

        assert!(is_mutating_sensitive_route(
            "/v1/publisher/rotate-key",
            &Method::POST
        ));
        assert!(is_mutating_sensitive_route(
            "/v1/publisher/packages/npm:a@1.0.0/revoke",
            &Method::POST
        ));
        assert!(is_mutating_sensitive_route(
            "/v1/validator/register",
            &Method::POST
        ));
        assert!(!is_mutating_sensitive_route(
            "/v1/public/health",
            &Method::GET
        ));

        let direct_peer = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 10));
        let trusted_proxy = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
        let trusted = [TrustedProxy::host(trusted_proxy)];
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.9, 10.0.0.5"),
        );

        // Untrusted peer cannot spoof X-Forwarded-For
        assert_eq!(
            extract_client_ip(direct_peer, &headers, &trusted),
            direct_peer
        );
        // Trusted proxy reveals real client IP
        assert_eq!(
            extract_client_ip(trusted_proxy, &headers, &trusted),
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))
        );
    }

    #[test]
    fn json_rpc_routes_have_their_own_bucket() {
        use axum::http::Method;
        assert!(is_json_rpc_route("/rpc", &Method::POST));
        assert!(is_json_rpc_route("/jsonrpc", &Method::POST));
        assert!(!is_json_rpc_route("/rpc", &Method::GET));
        assert!(!is_mutating_sensitive_route("/rpc", &Method::POST));
        assert!(!is_mutating_sensitive_route("/jsonrpc", &Method::POST));

        let limiter = RateLimiter::new(RateLimitConfig {
            rpc_limit: 2,
            mutate_limit: 1,
            ..Default::default()
        });
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        assert!(limiter.check_rpc(ip));
        assert!(limiter.check_rpc(ip));
        assert!(!limiter.check_rpc(ip), "third call exceeds rpc_limit 2");
        assert!(limiter.check_mutate(ip), "mutating bucket is untouched");
    }

    #[test]
    fn default_rpc_limit_is_far_above_the_mutating_limit() {
        let config = RateLimitConfig::default();
        if std::env::var("CREG_RATE_LIMIT_RPC").is_err() {
            assert_eq!(config.rpc_limit, DEFAULT_RPC_LIMIT);
        }
        assert!(config.rpc_limit > config.mutate_limit);
    }

    #[test]
    fn batch_charges_are_all_or_nothing() {
        let limiter = RateLimiter::new(RateLimitConfig {
            rpc_limit: 3,
            ..Default::default()
        });
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2));
        assert!(limiter.charge_rpc(ip, 2));
        assert!(!limiter.charge_rpc(ip, 2), "2 + 2 exceeds the limit of 3");
        assert!(limiter.check_rpc(ip), "the refused charge took no slots");
        assert!(!limiter.check_rpc(ip));
    }

    #[tokio::test]
    async fn json_rpc_429_is_a_json_rpc_error() -> anyhow::Result<()> {
        let response = json_rpc_rate_limited(Duration::from_secs(60));
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("60")
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
        let json: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["error"]["code"], JSON_RPC_LIMIT_EXCEEDED);
        assert_eq!(json["error"]["data"]["retry_after"], 60);
        assert!(json["id"].is_null());
        Ok(())
    }

    #[test]
    fn trusted_proxy_cidrs_parse_and_match() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();

        let net = TrustedProxy::parse("172.18.0.0/16").unwrap();
        assert!(net.contains(ip("172.18.3.4")));
        assert!(!net.contains(ip("172.19.0.1")));
        assert!(
            !net.contains(ip("2001:db8::1")),
            "an IPv4 network never matches IPv6"
        );

        let host = TrustedProxy::parse("10.0.0.5").unwrap();
        assert!(host.contains(ip("10.0.0.5")));
        assert!(!host.contains(ip("10.0.0.6")));
        assert!(
            host.contains(ip("::ffff:10.0.0.5")),
            "IPv4-mapped IPv6 matches its IPv4 address"
        );

        let v6 = TrustedProxy::parse("fd00::/8").unwrap();
        assert!(v6.contains(ip("fd12::1")));
        assert!(!v6.contains(ip("fe80::1")));
        assert!(!v6.contains(ip("10.0.0.1")));

        // A host-bits-set network is normalised to its network address.
        assert_eq!(
            TrustedProxy::parse("172.18.9.9/16"),
            TrustedProxy::parse("172.18.0.0/16")
        );
        assert!(TrustedProxy::parse("0.0.0.0/0")
            .unwrap()
            .contains(ip("203.0.113.1")));

        assert_eq!(TrustedProxy::parse("10.0.0.0/33"), None);
        assert_eq!(TrustedProxy::parse("fd00::/129"), None);
        assert_eq!(TrustedProxy::parse("nonsense"), None);
        assert_eq!(TrustedProxy::parse("10.0.0.0/x"), None);
    }

    #[test]
    fn trusted_proxy_list_skips_invalid_entries() {
        let parsed = parse_trusted_proxies(" 10.0.0.5, 172.18.0.0/16 , bogus,, fd00::/8");
        assert_eq!(parsed.len(), 3);
        assert!(parse_trusted_proxies("").is_empty());
    }

    #[test]
    fn xff_is_honoured_from_a_trusted_cidr() {
        use axum::http::{HeaderMap, HeaderValue};
        let trusted = parse_trusted_proxies("172.18.0.0/16");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.9, 172.18.0.3"),
        );
        let caddy = IpAddr::V4(Ipv4Addr::new(172, 18, 0, 3));
        assert_eq!(
            extract_client_ip(caddy, &headers, &trusted),
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))
        );
        let outsider = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1));
        assert_eq!(extract_client_ip(outsider, &headers, &trusted), outsider);
    }

    #[test]
    fn ipv6_clients_share_a_bucket_per_64() {
        let limiter = RateLimiter::new(RateLimitConfig {
            general_limit: 1,
            ..Default::default()
        });
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(limiter.check_general(ip("2001:db8:1:2::1")));
        assert!(
            !limiter.check_general(ip("2001:db8:1:2:ffff::9")),
            "same /64 shares the bucket"
        );
        assert!(
            limiter.check_general(ip("2001:db8:1:3::1")),
            "another /64 has its own"
        );
        assert!(limiter.check_general(ip("192.0.2.7")));
        assert!(
            !limiter.check_general(ip("::ffff:192.0.2.7")),
            "IPv4-mapped IPv6 shares the IPv4 bucket"
        );
    }

    #[test]
    fn a_full_table_evicts_instead_of_refusing_new_clients() {
        let limiter = RateLimiter::new(RateLimitConfig {
            general_limit: 10,
            ..Default::default()
        });
        for n in 0..MAX_TRACKED_IPS as u32 {
            assert!(limiter.check_general(IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + n))));
        }
        let newcomer = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 200));
        assert!(limiter.check_general(newcomer));
        assert_eq!(lock_buckets(&limiter.general).len(), MAX_TRACKED_IPS);
    }
}
