#![allow(dead_code)]
// crates/node/src/rate_limit.rs
// In-memory sliding-window rate limiter for the REST API.
//
// Limits per remote IP address:
//   - Package submissions (POST /v1/packages):   10 per 60 seconds
//   - General API requests:                    3000 per 60 seconds
//   - Validator vote submissions:               60 per 60 seconds
//
// Uses a token-bucket algorithm per IP stored in a dashmap.
// Expired buckets are periodically purged to prevent memory growth.

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
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_TRACKED_IPS: usize = 100_000;

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
    /// Sliding window duration.
    pub window: Duration,
    /// Explicit proxy IPs allowed to supply `X-Forwarded-For` (`VNA-08`).
    pub trusted_proxies: Vec<IpAddr>,
}

fn trusted_proxies_from_env() -> Vec<IpAddr> {
    std::env::var("CREG_TRUSTED_PROXIES")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<IpAddr>().ok())
        .collect()
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
            window: Duration::from_secs(60),
            trusted_proxies: trusted_proxies_from_env(),
        }
    }
}

/// Per-IP sliding window bucket.
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

    /// Remove timestamps older than the window, then check count.
    fn is_allowed(&mut self, window: Duration, limit: u32) -> bool {
        let now = Instant::now();
        let cutoff = now - window;
        self.timestamps.retain(|&t| t > cutoff);
        if self.timestamps.len() < limit as usize {
            self.timestamps.push(now);
            true
        } else {
            false
        }
    }
}

/// Lock a mutex, recovering gracefully if it was poisoned.
fn lock_buckets<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("Rate limiter mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

/// Shared rate limit state (IP → bucket per endpoint class).
#[derive(Clone)]
pub struct RateLimiter {
    general: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    mutate: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    publish: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    vote: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    authority: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            general: Arc::new(Mutex::new(HashMap::new())),
            mutate: Arc::new(Mutex::new(HashMap::new())),
            publish: Arc::new(Mutex::new(HashMap::new())),
            vote: Arc::new(Mutex::new(HashMap::new())),
            authority: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    fn check(&self, buckets: &Arc<Mutex<HashMap<IpAddr, Bucket>>>, ip: IpAddr, limit: u32) -> bool {
        let mut map = lock_buckets(buckets);
        if !map.contains_key(&ip) && map.len() >= MAX_TRACKED_IPS {
            let now = Instant::now();
            let window = self.config.window;
            map.retain(|_, bucket| {
                bucket.timestamps.retain(|&t| now - t < window);
                !bucket.timestamps.is_empty()
            });
            if map.len() >= MAX_TRACKED_IPS {
                return false;
            }
        }
        let bucket = map.entry(ip).or_insert_with(Bucket::new);
        bucket.is_allowed(self.config.window, limit)
    }

    pub fn check_general(&self, ip: IpAddr) -> bool {
        self.check(&self.general, ip, self.config.general_limit)
    }

    pub fn check_mutate(&self, ip: IpAddr) -> bool {
        self.check(&self.mutate, ip, self.config.mutate_limit)
    }

    pub fn check_publish(&self, ip: IpAddr) -> bool {
        self.check(&self.publish, ip, self.config.publish_limit)
    }

    pub fn check_vote(&self, ip: IpAddr) -> bool {
        self.check(&self.vote, ip, self.config.vote_limit)
    }

    pub fn check_authority(&self, ip: IpAddr) -> bool {
        self.check(&self.authority, ip, self.config.authority_limit)
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

fn is_mutating_sensitive_route(path: &str, method: &axum::http::Method) -> bool {
    if method != axum::http::Method::POST && method != axum::http::Method::DELETE {
        return false;
    }
    path == "/rpc"
        || path == "/jsonrpc"
        || path == "/v1/publisher/rotate-key"
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

/// Extracts the effective client IP: honors right-most non-trusted `X-Forwarded-For`
/// entry ONLY when `peer_ip` belongs to `trusted_proxies` (`VNA-08`).
pub fn extract_client_ip(
    peer_ip: IpAddr,
    headers: &axum::http::HeaderMap,
    trusted_proxies: &[IpAddr],
) -> IpAddr {
    if !trusted_proxies.contains(&peer_ip) {
        return peer_ip;
    }
    let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) else {
        return peer_ip;
    };
    for part in xff.split(',').rev() {
        if let Ok(candidate) = part.trim().parse::<IpAddr>() {
            if !trusted_proxies.contains(&candidate) {
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

    let allowed = if is_authority_attestation(path, method) {
        limiter.check_authority(ip)
    } else if is_publish_submit(path, method) {
        limiter.check_publish(ip)
    } else if is_consensus_vote(path, method) {
        limiter.check_vote(ip)
    } else if is_mutating_sensitive_route(path, method) {
        limiter.check_mutate(ip)
    } else {
        limiter.check_general(ip)
    };

    if !allowed {
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
        assert!(is_mutating_sensitive_route("/rpc", &Method::POST));
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
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.9, 10.0.0.5"),
        );

        // Untrusted peer cannot spoof X-Forwarded-For
        assert_eq!(
            extract_client_ip(direct_peer, &headers, &[trusted_proxy]),
            direct_peer
        );
        // Trusted proxy reveals real client IP
        assert_eq!(
            extract_client_ip(trusted_proxy, &headers, &[trusted_proxy]),
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9))
        );
    }
}
