//! Ethereum JSON-RPC access over independently configured endpoints.
//!
//! Shared by the node, faucet, relayer and IPFS pinner so every service that
//! reads Sepolia applies the same rule. Every request is routed by one of
//! three policies:
//!
//! * **Quorum** — state reads whose answers decide security-relevant outcomes
//!   (`eth_call`, `eth_getLogs`, `eth_getCode`, `eth_getStorageAt`,
//!   `eth_getBalance`, `eth_chainId`). The request is sent to every configured
//!   endpoint in parallel. A response is returned only when a strict majority
//!   of the *configured* endpoints returned the same normalised answer, so a
//!   provider that silently omits logs, serves pruned history as an empty
//!   result, or lags behind on contract state cannot decide the outcome on its
//!   own. Endpoints that fail still count toward the denominator: with three
//!   configured providers, two must agree. With one configured provider this
//!   is exactly that provider's answer, as before. With two, both must agree,
//!   so operators should configure one provider or at least three.
//! * **Failover** — other idempotent reads (block numbers, receipts, fee
//!   data). These tolerate small, honest differences between providers, so
//!   the first endpoint that answers without a transport error is used.
//! * **Primary only** — anything with possible side effects, such as
//!   transaction submission. A transport failure after a signed transaction is
//!   sent is ambiguous, and retrying it here could hide that ambiguity from
//!   the caller. The caller's durable transaction journal owns that recovery
//!   decision.
//!
//! A quorum read returns as soon as a strict majority agrees, without waiting
//! for the remaining endpoints, and fails as soon as no majority is possible.
//!
//! Endpoint health is remembered for the life of the process, per ordered
//! endpoint list ([`endpoint_health`]). Providers are often built per
//! operation, so health kept inside one provider would be forgotten at once.
//! An endpoint whose transport failed moves to the back of the failover order
//! for a cool-down (`CREG_L1_RPC_COOLDOWN_SECS`, default 15 s, 0 disables),
//! then leads again. Quorum reads still go to every endpoint, and primary-only
//! calls still go to the primary exactly once. [`endpoint_stats`] exposes
//! per-endpoint counters by position.
//!
//! Two entry points apply these policies:
//!
//! * [`build_provider_from_urls`] returns an alloy provider whose transport is
//!   [`FailoverTransport`]. Wrap it with a wallet (`on_provider`) to sign.
//! * [`routed_json_call`] applies the same policy to a caller-supplied
//!   per-endpoint request function, for services that speak raw JSON-RPC or
//!   an indexer adapter.
//!
//! Errors and logs identify endpoints only by position. Endpoint URLs often
//! embed provider API keys, so they are never logged or returned.

use alloy::{
    providers::{ProviderBuilder, RootProvider},
    rpc::{
        client::ClientBuilder,
        json_rpc::{RequestPacket, ResponsePacket, ResponsePayload},
    },
    transports::{
        http::{reqwest::Url, Client, Http},
        BoxTransport, Transport, TransportError, TransportErrorKind, TransportFut,
    },
};
use anyhow::{anyhow, bail, Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use serde_json::Value;
use std::{
    collections::HashMap,
    future::{poll_fn, Future},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, OnceLock,
    },
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant},
};
use tower::Service;

/// Upper bound on distinct endpoints, so a misconfiguration cannot fan one
/// read out to an unbounded number of providers.
pub const MAX_RPC_ENDPOINTS: usize = 8;

/// Fields that identify an Ethereum log. Providers add optional extras such
/// as `blockTimestamp`; those must not make honest providers disagree.
const CANONICAL_LOG_FIELDS: [&str; 9] = [
    "address",
    "blockHash",
    "blockNumber",
    "data",
    "logIndex",
    "removed",
    "topics",
    "transactionHash",
    "transactionIndex",
];

/// How one JSON-RPC request is dispatched across the configured endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadPolicy {
    Quorum,
    Failover,
    PrimaryOnly,
}

/// Number of matching answers needed to form a strict majority of the
/// configured endpoints. A majority of only the responders is insufficient:
/// if four providers are configured, two matching answers and two timeouts
/// are not a quorum.
pub fn configured_quorum_size(configured: usize) -> usize {
    if configured == 0 {
        0
    } else {
        configured / 2 + 1
    }
}

/// How long a failing endpoint stays at the back of the failover order.
pub const DEFAULT_ENDPOINT_COOLDOWN: Duration = Duration::from_secs(15);
const COOLDOWN_ENV: &str = "CREG_L1_RPC_COOLDOWN_SECS";
/// Upper bound on distinct endpoint lists whose health is remembered, so a
/// caller that builds endpoint lists dynamically cannot grow the registry
/// without limit. Lists beyond it get health that is not shared.
const MAX_TRACKED_ENDPOINT_LISTS: usize = 64;

/// Outcome counters for one configured endpoint. Endpoints are identified by
/// their 1-based position only: URLs often embed provider API keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EndpointStats {
    pub position: usize,
    /// Calls this endpoint answered without a transport error.
    pub successes: u64,
    /// Calls that failed at the transport level (connection, timeout, HTTP
    /// error). A JSON-RPC error answer is a success here.
    pub failures: u64,
    /// Times a failover read moved this endpoint to the back while it was
    /// cooling down.
    pub skips: u64,
    pub cooling_down: bool,
}

#[derive(Debug, Default)]
struct EndpointState {
    successes: AtomicU64,
    failures: AtomicU64,
    skips: AtomicU64,
    cooling_until: Mutex<Option<Instant>>,
}

/// Health of one ordered endpoint list: outcome counters and the failover
/// cool-down. Shared through [`endpoint_health`].
#[derive(Debug)]
pub struct EndpointHealth {
    cooldown: Duration,
    endpoints: Vec<EndpointState>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl EndpointHealth {
    /// Health for `endpoints` endpoints. A zero `cooldown` disables
    /// reordering; the counters still run.
    pub fn new(endpoints: usize, cooldown: Duration) -> Self {
        Self {
            cooldown,
            endpoints: (0..endpoints).map(|_| EndpointState::default()).collect(),
        }
    }

    fn record(&self, index: usize, ok: bool) {
        let Some(state) = self.endpoints.get(index) else {
            return;
        };
        if ok {
            state.successes.fetch_add(1, Ordering::Relaxed);
            *lock(&state.cooling_until) = None;
        } else {
            state.failures.fetch_add(1, Ordering::Relaxed);
            if !self.cooldown.is_zero() {
                *lock(&state.cooling_until) = Instant::now().checked_add(self.cooldown);
            }
        }
    }

    fn is_cooling(&self, index: usize, now: Instant) -> bool {
        self.endpoints
            .get(index)
            .and_then(|state| *lock(&state.cooling_until))
            .is_some_and(|until| until > now)
    }

    /// Endpoint indexes to try for a failover read: configured order, with
    /// endpoints that are cooling down moved to the back (still in configured
    /// order) so they remain a last resort rather than being dropped.
    fn failover_order(&self) -> Vec<usize> {
        let now = Instant::now();
        let (cooling, ready): (Vec<usize>, Vec<usize>) =
            (0..self.endpoints.len()).partition(|&index| self.is_cooling(index, now));
        for &index in &cooling {
            self.endpoints[index].skips.fetch_add(1, Ordering::Relaxed);
        }
        ready.into_iter().chain(cooling).collect()
    }

    /// Counters for every endpoint, in configured order.
    pub fn stats(&self) -> Vec<EndpointStats> {
        let now = Instant::now();
        self.endpoints
            .iter()
            .enumerate()
            .map(|(index, state)| EndpointStats {
                position: index + 1,
                successes: state.successes.load(Ordering::Relaxed),
                failures: state.failures.load(Ordering::Relaxed),
                skips: state.skips.load(Ordering::Relaxed),
                cooling_down: self.is_cooling(index, now),
            })
            .collect()
    }
}

/// Keyed by the ordered endpoint list. The URLs live only in this map; they
/// are never logged or returned.
type HealthRegistry = Mutex<HashMap<Vec<String>, Arc<EndpointHealth>>>;

fn health_registry() -> &'static HealthRegistry {
    static REGISTRY: OnceLock<HealthRegistry> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn configured_cooldown() -> Duration {
    static COOLDOWN: OnceLock<Duration> = OnceLock::new();
    *COOLDOWN.get_or_init(|| match std::env::var(COOLDOWN_ENV) {
        Err(_) => DEFAULT_ENDPOINT_COOLDOWN,
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(seconds) => Duration::from_secs(seconds),
            Err(_) => {
                tracing::warn!(
                    "{COOLDOWN_ENV} must be a whole number of seconds; using the default"
                );
                DEFAULT_ENDPOINT_COOLDOWN
            }
        },
    })
}

/// The process-wide health for this exact ordered endpoint list, created on
/// first use.
pub fn endpoint_health(endpoints: &[String]) -> Arc<EndpointHealth> {
    let mut registry = lock(health_registry());
    if let Some(health) = registry.get(endpoints) {
        return Arc::clone(health);
    }
    let health = Arc::new(EndpointHealth::new(endpoints.len(), configured_cooldown()));
    if registry.len() < MAX_TRACKED_ENDPOINT_LISTS {
        registry.insert(endpoints.to_vec(), Arc::clone(&health));
    }
    health
}

/// Counters for an endpoint list that has been used in this process, by
/// position. `None` if the list was never used.
pub fn endpoint_stats(endpoints: &[String]) -> Option<Vec<EndpointStats>> {
    lock(health_registry())
        .get(endpoints)
        .map(|health| health.stats())
}

/// Answers collected so far by one quorum read.
struct QuorumTally<T> {
    configured: usize,
    answers: Vec<T>,
    keys: Vec<Option<Value>>,
    failed: usize,
}

impl<T> QuorumTally<T> {
    fn new(configured: usize) -> Self {
        Self {
            configured,
            answers: Vec::with_capacity(configured),
            keys: Vec::with_capacity(configured),
            failed: 0,
        }
    }

    /// Record an answer. Returns the winning answer once a strict majority of
    /// the configured endpoints agree.
    fn answer(&mut self, answer: T, key: Option<Value>) -> Option<T> {
        self.answers.push(answer);
        self.keys.push(key);
        let index = select_quorum_index(&self.keys, self.configured)?;
        Some(self.answers.swap_remove(index))
    }

    fn fail(&mut self) {
        self.failed += 1;
    }

    /// Whether the endpoints still outstanding could complete a majority.
    /// Unparseable answers never match, so they cannot help.
    fn still_possible(&self) -> bool {
        let outstanding = self
            .configured
            .saturating_sub(self.answers.len() + self.failed);
        let best = self
            .keys
            .iter()
            .flatten()
            .map(|key| {
                self.keys
                    .iter()
                    .filter(|other| other.as_ref() == Some(key))
                    .count()
            })
            .max()
            .unwrap_or(0);
        best + outstanding >= configured_quorum_size(self.configured)
    }

    fn responded(&self) -> usize {
        self.answers.len()
    }

    fn distinct(&self) -> usize {
        distinct_answer_count(&self.keys)
    }
}

/// An HTTP JSON-RPC transport that enforces a configured-endpoint quorum for
/// security-relevant state reads and fails over for other safe reads.
#[derive(Clone, Debug)]
pub struct FailoverTransport {
    upstreams: Vec<BoxTransport>,
    health: Arc<EndpointHealth>,
}

impl FailoverTransport {
    fn new(upstreams: Vec<BoxTransport>, health: Arc<EndpointHealth>) -> Self {
        Self { upstreams, health }
    }
}

impl Service<RequestPacket> for FailoverTransport {
    type Response = ResponsePacket;
    type Error = TransportError;
    type Future = TransportFut<'static>;

    fn poll_ready(
        &mut self,
        _cx: &mut TaskContext<'_>,
    ) -> Poll<std::result::Result<(), Self::Error>> {
        // Each HTTP upstream is polled immediately before dispatch in `call`.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: RequestPacket) -> Self::Future {
        let upstreams = self.upstreams.clone();
        let health = Arc::clone(&self.health);
        match request_policy(&request) {
            ReadPolicy::Quorum => Box::pin(quorum_call(upstreams, health, request)),
            ReadPolicy::Failover => Box::pin(failover_call(upstreams, health, request)),
            ReadPolicy::PrimaryOnly => Box::pin(primary_call(upstreams, health, request)),
        }
    }
}

async fn upstream_call(
    mut upstream: BoxTransport,
    request: RequestPacket,
) -> std::result::Result<ResponsePacket, TransportError> {
    poll_fn(|cx| upstream.poll_ready(cx)).await?;
    upstream.call(request).await
}

fn no_endpoint_error() -> TransportError {
    TransportErrorKind::custom_str("no Ethereum RPC endpoint is configured")
}

/// Try endpoints in failover order (healthy ones first, see
/// [`EndpointHealth::failover_order`]) until one answers.
async fn failover_call(
    upstreams: Vec<BoxTransport>,
    health: Arc<EndpointHealth>,
    request: RequestPacket,
) -> std::result::Result<ResponsePacket, TransportError> {
    let mut last_error = None;
    for index in health.failover_order() {
        let Some(upstream) = upstreams.get(index).cloned() else {
            continue;
        };
        match upstream_call(upstream, request.clone()).await {
            Ok(response) => {
                health.record(index, true);
                if index > 0 {
                    tracing::warn!(
                        endpoint_index = index + 1,
                        "Ethereum RPC read failed over to a configured endpoint"
                    );
                }
                return Ok(response);
            }
            Err(error) => {
                health.record(index, false);
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(no_endpoint_error))
}

/// Send a call with possible side effects to the primary exactly once,
/// whatever its health: a retry elsewhere could hide an ambiguous send.
async fn primary_call(
    upstreams: Vec<BoxTransport>,
    health: Arc<EndpointHealth>,
    request: RequestPacket,
) -> std::result::Result<ResponsePacket, TransportError> {
    let Some(primary) = upstreams.into_iter().next() else {
        return Err(no_endpoint_error());
    };
    let result = upstream_call(primary, request).await;
    health.record(0, result.is_ok());
    result
}

/// Send the request to every endpoint in parallel and return a response once
/// a strict majority of the configured endpoints agree on it. Endpoints still
/// outstanding at that point are no longer waited for. The read fails closed as
/// soon as no majority is possible.
async fn quorum_call(
    upstreams: Vec<BoxTransport>,
    health: Arc<EndpointHealth>,
    request: RequestPacket,
) -> std::result::Result<ResponsePacket, TransportError> {
    let configured = upstreams.len();
    if configured == 0 {
        return Err(no_endpoint_error());
    }

    let mut pending: FuturesUnordered<_> = upstreams
        .into_iter()
        .enumerate()
        .map(|(index, upstream)| {
            let request = request.clone();
            async move { (index, upstream_call(upstream, request).await) }
        })
        .collect();

    let mut tally = QuorumTally::new(configured);
    while let Some((index, result)) = pending.next().await {
        match result {
            Ok(packet) => {
                health.record(index, true);
                let key = packet_comparison_key(&packet);
                if let Some(winner) = tally.answer(packet, key) {
                    return Ok(winner);
                }
            }
            Err(_) => {
                health.record(index, false);
                tally.fail();
            }
        }
        if !tally.still_possible() {
            break;
        }
    }

    tracing::warn!(
        configured_endpoints = configured,
        responding_endpoints = tally.responded(),
        distinct_answers = tally.distinct(),
        "Ethereum RPC state read did not reach configured-endpoint quorum; failing closed"
    );
    Err(TransportErrorKind::custom_str(
        "configured Ethereum RPC endpoints did not reach read quorum",
    ))
}

/// Return the index of an answer whose comparison key is shared by at least
/// a strict majority of the configured endpoints. Unparseable answers
/// (`None`) never match anything. `keys` holds one entry per endpoint that
/// answered; endpoints that failed are simply absent.
pub fn select_quorum_index(keys: &[Option<Value>], configured: usize) -> Option<usize> {
    let required = configured_quorum_size(configured);
    if required == 0 {
        return None;
    }
    keys.iter().enumerate().find_map(|(index, candidate)| {
        let candidate = candidate.as_ref()?;
        let matching = keys
            .iter()
            .filter(|key| key.as_ref() == Some(candidate))
            .count();
        (matching >= required).then_some(index)
    })
}

fn distinct_answer_count(keys: &[Option<Value>]) -> usize {
    let mut distinct: Vec<&Value> = Vec::new();
    for key in keys.iter().flatten() {
        if !distinct.contains(&key) {
            distinct.push(key);
        }
    }
    distinct.len()
}

/// Normalised, provider-independent view of a response used only to compare
/// endpoints. The original packet is what gets returned to the caller.
fn packet_comparison_key(packet: &ResponsePacket) -> Option<Value> {
    match packet {
        ResponsePacket::Single(response) => payload_comparison_key(&response.payload),
        ResponsePacket::Batch(responses) => {
            let mut keyed = Vec::with_capacity(responses.len());
            for response in responses {
                let id = serde_json::to_value(&response.id).ok()?.to_string();
                keyed.push((id, payload_comparison_key(&response.payload)?));
            }
            keyed.sort_by(|left, right| left.0.cmp(&right.0));
            Some(Value::Array(
                keyed.into_iter().map(|(_, value)| value).collect(),
            ))
        }
    }
}

fn payload_comparison_key(payload: &ResponsePayload) -> Option<Value> {
    match payload {
        ResponsePayload::Success(raw) => serde_json::from_str::<Value>(raw.get())
            .ok()
            .map(normalize_rpc_value),
        // Error messages differ between providers; the JSON-RPC code is the
        // comparable part (for example, a consistent execution revert).
        ResponsePayload::Failure(error) => {
            Some(serde_json::json!({ "rpc_error_code": error.code }))
        }
    }
}

/// Make honest providers comparable: lowercase hex strings (some providers
/// return EIP-55 checksummed addresses) and reduce log objects to the fields
/// that identify them.
pub fn normalize_rpc_value(value: Value) -> Value {
    match value {
        Value::String(text) => normalize_hex_string(text),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize_rpc_value).collect()),
        Value::Object(map) if map.contains_key("topics") && map.contains_key("logIndex") => {
            let mut canonical = serde_json::Map::new();
            for field in CANONICAL_LOG_FIELDS {
                let value = match map.get(field) {
                    Some(value) => normalize_rpc_value(value.clone()),
                    None if field == "removed" => Value::Bool(false),
                    None => Value::Null,
                };
                canonical.insert(field.to_string(), value);
            }
            Value::Object(canonical)
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, normalize_rpc_value(value)))
                .collect(),
        ),
        other => other,
    }
}

fn normalize_hex_string(text: String) -> Value {
    if text.starts_with("0x") || text.starts_with("0X") {
        Value::String(text.to_ascii_lowercase())
    } else {
        Value::String(text)
    }
}

/// Parse a primary URL and a comma-separated fallback list into an ordered,
/// validated endpoint list. Endpoints on a host already listed are dropped,
/// so one provider cannot be counted twice toward a quorum. The labels name
/// the configuration keys in error messages; URLs never appear in errors.
pub fn parse_rpc_endpoints(
    primary_url: &str,
    fallback_urls: &str,
    primary_label: &str,
    fallback_label: &str,
) -> Result<Vec<String>> {
    let mut raw_urls = vec![primary_url.trim().to_string()];
    raw_urls.extend(
        fallback_urls
            .split(',')
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(ToOwned::to_owned),
    );

    let mut urls = Vec::new();
    let mut hosts = Vec::new();
    for (index, raw_url) in raw_urls.into_iter().enumerate() {
        if raw_url.is_empty() {
            if index == 0 {
                bail!("{primary_label} is empty");
            }
            continue;
        }
        let parsed = Url::parse(&raw_url).with_context(|| {
            if index == 0 {
                format!("{primary_label} is not a valid URL")
            } else {
                format!("{fallback_label} entry {index} is not a valid URL")
            }
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            bail!("Ethereum RPC endpoints must use HTTP or HTTPS");
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow!("Ethereum RPC endpoint has no hostname"))?
            .to_ascii_lowercase();
        if hosts.contains(&host) {
            continue;
        }
        hosts.push(host);
        urls.push(parsed.to_string());
        if urls.len() > MAX_RPC_ENDPOINTS {
            bail!("at most {MAX_RPC_ENDPOINTS} distinct Ethereum RPC endpoints may be configured");
        }
    }

    if urls.is_empty() {
        bail!("no Ethereum RPC endpoints are configured");
    }
    Ok(urls)
}

/// Build a provider over an already validated, de-duplicated endpoint list
/// (see [`parse_rpc_endpoints`]). Security-relevant state reads require a
/// configured-endpoint quorum, other reads fail over, and calls with possible
/// side effects use the primary once.
pub fn build_provider_from_urls(urls: &[String]) -> Result<RootProvider<FailoverTransport>> {
    if urls.is_empty() {
        bail!("no Ethereum RPC endpoints are configured");
    }
    let http_client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build bounded Ethereum RPC HTTP client")?;
    let upstreams = urls
        .iter()
        .map(|url| {
            let parsed = Url::parse(url).context("validated Ethereum RPC URL changed")?;
            let transport: BoxTransport =
                Http::<Client>::with_client(http_client.clone(), parsed).boxed();
            Ok(transport)
        })
        .collect::<Result<Vec<_>>>()?;
    let transport = FailoverTransport::new(upstreams, endpoint_health(urls));
    let client = ClientBuilder::default().transport(transport, false);
    Ok(ProviderBuilder::new().on_client(client))
}

/// Apply the read policy for `method` to a caller-supplied request function.
///
/// `call` performs `method` against one endpoint URL and returns the JSON-RPC
/// `result`. With one endpoint its answer or error is returned unchanged.
/// With several, quorum methods are sent to every endpoint and the answer
/// shared by a strict majority of the configured endpoints is returned;
/// other read-only methods fail over in order; anything else uses the
/// primary once. Per-endpoint errors are not propagated in the multi-endpoint
/// case because they can contain endpoint URLs.
pub async fn routed_json_call<F, Fut>(endpoints: &[String], method: &str, call: F) -> Result<Value>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    match endpoints {
        [] => bail!("no Ethereum RPC endpoint is configured"),
        [only] => call(only.clone()).await,
        _ => match policy_for_methods(&[method]) {
            ReadPolicy::Quorum => quorum_json_call(endpoints, method, call).await,
            ReadPolicy::Failover => sequential_json_call(endpoints, method, call, true).await,
            ReadPolicy::PrimaryOnly => sequential_json_call(endpoints, method, call, false).await,
        },
    }
}

/// Require a strict majority of matching answers even for reads that normally
/// use failover, such as block headers that anchor a finalized log range.
/// Returns as soon as a majority agrees and fails as soon as none is possible.
pub async fn quorum_json_call<F, Fut>(endpoints: &[String], method: &str, call: F) -> Result<Value>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let configured = endpoints.len();
    let health = endpoint_health(endpoints);
    let mut pending: FuturesUnordered<_> = endpoints
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, endpoint)| {
            let answer = call(endpoint);
            async move { (index, answer.await) }
        })
        .collect();

    let mut tally = QuorumTally::new(configured);
    while let Some((index, result)) = pending.next().await {
        match result {
            Ok(answer) => {
                health.record(index, true);
                let key = Some(normalize_rpc_value(answer.clone()));
                if let Some(winner) = tally.answer(answer, key) {
                    return Ok(winner);
                }
            }
            Err(_) => {
                health.record(index, false);
                tally.fail();
            }
        }
        if !tally.still_possible() {
            break;
        }
    }

    let distinct = tally.distinct();
    tracing::warn!(
        method,
        configured_endpoints = configured,
        responding_endpoints = tally.responded(),
        distinct_answers = distinct,
        "Ethereum RPC state read did not reach configured-endpoint quorum; failing closed"
    );
    bail!(
        "{method} did not reach read quorum: {} of {configured} configured endpoints answered with {distinct} distinct answers, {} matching answers required",
        tally.responded(),
        configured_quorum_size(configured)
    )
}

async fn sequential_json_call<F, Fut>(
    endpoints: &[String],
    method: &str,
    call: F,
    can_fail_over: bool,
) -> Result<Value>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let health = endpoint_health(endpoints);
    // Calls with possible side effects go to the primary once, whatever its
    // health; failover reads lead with the endpoints that are not cooling down.
    let order = if can_fail_over {
        health.failover_order()
    } else {
        vec![0]
    };
    let mut failed = Vec::new();
    for index in order {
        let Some(endpoint) = endpoints.get(index) else {
            continue;
        };
        match call(endpoint.clone()).await {
            Ok(answer) => {
                health.record(index, true);
                if index > 0 {
                    tracing::warn!(
                        method,
                        endpoint_index = index + 1,
                        "Ethereum RPC read failed over to a configured endpoint"
                    );
                }
                return Ok(answer);
            }
            Err(_) => {
                health.record(index, false);
                failed.push((index + 1).to_string());
            }
        }
    }
    bail!(
        "{method} failed on configured Ethereum RPC endpoint(s) {}",
        failed.join(", ")
    )
}

fn request_policy(packet: &RequestPacket) -> ReadPolicy {
    let methods: Vec<&str> = match packet {
        RequestPacket::Single(request) => vec![request.method()],
        RequestPacket::Batch(requests) => requests.iter().map(|request| request.method()).collect(),
    };
    policy_for_methods(&methods)
}

/// A batch is only as permissive as its strictest member: any method with
/// possible side effects sends the whole batch to the primary once, and any
/// security-relevant read puts the whole batch under quorum.
fn policy_for_methods(methods: &[&str]) -> ReadPolicy {
    if methods.is_empty() || !methods.iter().all(|method| method_is_read_only(method)) {
        ReadPolicy::PrimaryOnly
    } else if methods.iter().any(|method| method_requires_quorum(method)) {
        ReadPolicy::Quorum
    } else {
        ReadPolicy::Failover
    }
}

/// State reads where one wrong or incomplete provider could change a
/// security decision: stake and admission checks, validator-set events,
/// contract code, balances, and chain identity.
pub fn method_requires_quorum(method: &str) -> bool {
    matches!(
        method,
        "eth_call"
            | "eth_getLogs"
            | "eth_getCode"
            | "eth_getStorageAt"
            | "eth_getBalance"
            | "eth_chainId"
    )
}

/// Methods without side effects that may be retried on another endpoint.
/// `eth_getTransactionCount` is deliberately absent: nonce selection belongs
/// to the primary that will receive the transaction.
pub fn method_is_read_only(method: &str) -> bool {
    matches!(
        method,
        "eth_blockNumber"
            | "eth_call"
            | "eth_chainId"
            | "eth_estimateGas"
            | "eth_feeHistory"
            | "eth_gasPrice"
            | "eth_maxPriorityFeePerGas"
            | "eth_syncing"
            | "eth_getBalance"
            | "eth_getBlockByHash"
            | "eth_getBlockByNumber"
            | "eth_getBlockTransactionCountByHash"
            | "eth_getBlockTransactionCountByNumber"
            | "eth_getCode"
            | "eth_getLogs"
            | "eth_getStorageAt"
            | "eth_getTransactionByBlockHashAndIndex"
            | "eth_getTransactionByBlockNumberAndIndex"
            | "eth_getTransactionByHash"
            | "eth_getTransactionReceipt"
            | "eth_getUncleByBlockHashAndIndex"
            | "eth_getUncleByBlockNumberAndIndex"
            | "eth_getUncleCountByBlockHash"
            | "eth_getUncleCountByBlockNumber"
            | "net_listening"
            | "net_peerCount"
            | "net_version"
            | "web3_clientVersion"
            | "web3_sha3"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{primitives::Address, providers::Provider};
    use serde_json::json;
    use std::collections::HashMap;

    fn log(log_index: &str, extra: Option<(&str, Value)>) -> Value {
        let mut value = json!({
            "address": "0x61c00EBe7C46f0bcE63942F7E15Bcc434DFC9B7C",
            "blockHash": "0xAB",
            "blockNumber": "0xb3c568",
            "data": "0x",
            "logIndex": log_index,
            "topics": ["0xDDF2"],
            "transactionHash": "0x01",
            "transactionIndex": "0x0"
        });
        if let Some((key, extra_value)) = extra {
            value[key] = extra_value;
        }
        value
    }

    #[test]
    fn quorum_size_is_a_strict_majority_of_configured_endpoints() {
        assert_eq!(configured_quorum_size(0), 0);
        assert_eq!(configured_quorum_size(1), 1);
        assert_eq!(configured_quorum_size(2), 2);
        assert_eq!(configured_quorum_size(3), 2);
        assert_eq!(configured_quorum_size(4), 3);
    }

    #[test]
    fn policy_routes_security_reads_to_quorum() {
        assert_eq!(policy_for_methods(&["eth_call"]), ReadPolicy::Quorum);
        assert_eq!(policy_for_methods(&["eth_getLogs"]), ReadPolicy::Quorum);
        assert_eq!(policy_for_methods(&["eth_chainId"]), ReadPolicy::Quorum);
        assert_eq!(
            policy_for_methods(&["eth_blockNumber"]),
            ReadPolicy::Failover
        );
        assert_eq!(
            policy_for_methods(&["eth_getTransactionReceipt"]),
            ReadPolicy::Failover
        );
        assert_eq!(
            policy_for_methods(&["eth_sendRawTransaction"]),
            ReadPolicy::PrimaryOnly
        );
        assert_eq!(
            policy_for_methods(&["eth_getTransactionCount"]),
            ReadPolicy::PrimaryOnly
        );
    }

    #[test]
    fn batch_policy_follows_its_strictest_member() {
        assert_eq!(
            policy_for_methods(&["eth_blockNumber", "eth_call"]),
            ReadPolicy::Quorum
        );
        assert_eq!(
            policy_for_methods(&["eth_call", "eth_sendRawTransaction"]),
            ReadPolicy::PrimaryOnly
        );
        assert_eq!(policy_for_methods(&[]), ReadPolicy::PrimaryOnly);
    }

    #[test]
    fn log_normalisation_ignores_provider_extras_and_case() {
        let extra = Some(("blockTimestamp", json!("0x66f00000")));
        let with_timestamp = normalize_rpc_value(json!([log("0x1", extra)]));
        let lowercase_plain = normalize_rpc_value(json!([{
            "address": "0x61c00ebe7c46f0bce63942f7e15bcc434dfc9b7c",
            "blockHash": "0xab",
            "blockNumber": "0xb3c568",
            "data": "0x",
            "logIndex": "0x1",
            "removed": false,
            "topics": ["0xddf2"],
            "transactionHash": "0x01",
            "transactionIndex": "0x0"
        }]));
        assert_eq!(with_timestamp, lowercase_plain);
    }

    #[test]
    fn log_normalisation_keeps_identity_fields() {
        let first = normalize_rpc_value(json!([log("0x1", None)]));
        let second = normalize_rpc_value(json!([log("0x2", None)]));
        assert_ne!(first, second);
    }

    #[test]
    fn quorum_rejects_a_lone_empty_log_answer() {
        // The 2026-10 failure mode: one provider returns no staking logs
        // while two others return the real events.
        let empty = Some(normalize_rpc_value(json!([])));
        let full = Some(normalize_rpc_value(json!([log("0x1", None)])));
        let extra = Some(("blockTimestamp", json!("0x1")));
        let full_with_extra = Some(normalize_rpc_value(json!([log("0x1", extra)])));

        let keys = [empty.clone(), full.clone(), full_with_extra];
        let index = select_quorum_index(&keys, 3).expect("two providers agree");
        assert_eq!(keys[index], full);

        // An empty answer from the primary plus one silent provider is not a
        // quorum of three.
        assert_eq!(select_quorum_index(&[empty.clone(), None], 3), None);
        // Two disagreeing answers out of three configured is not a quorum.
        assert_eq!(select_quorum_index(&[empty, full], 3), None);
    }

    #[test]
    fn quorum_counts_unavailable_endpoints_in_the_denominator() {
        let answer = Some(json!("0x01"));
        assert_eq!(
            select_quorum_index(std::slice::from_ref(&answer), 1),
            Some(0)
        );
        assert_eq!(select_quorum_index(std::slice::from_ref(&answer), 2), None);
        assert_eq!(
            select_quorum_index(&[answer.clone(), answer.clone()], 3),
            Some(0)
        );
        assert_eq!(select_quorum_index(&[answer.clone(), answer], 4), None);
        assert_eq!(select_quorum_index(&[], 0), None);
    }

    #[test]
    fn unparseable_responses_never_form_a_quorum() {
        assert_eq!(select_quorum_index(&[None, None, None], 3), None);
        assert_eq!(
            distinct_answer_count(&[None, Some(json!(1)), Some(json!(1))]),
            1
        );
    }

    #[test]
    fn endpoint_parsing_drops_repeated_hosts_and_keeps_order() {
        let urls = parse_rpc_endpoints(
            "https://a.example/v1/key",
            " https://b.example/rpc , https://A.example/other,,https://c.example ",
            "PRIMARY",
            "FALLBACKS",
        )
        .expect("valid endpoints");
        assert_eq!(
            urls,
            [
                "https://a.example/v1/key",
                "https://b.example/rpc",
                "https://c.example/"
            ]
        );
        assert_eq!(
            parse_rpc_endpoints("http://127.0.0.1:8545", "", "PRIMARY", "FALLBACKS")
                .expect("primary only")
                .len(),
            1
        );
    }

    #[test]
    fn endpoint_parsing_names_the_setting_without_echoing_urls() {
        let empty = parse_rpc_endpoints(" ", "", "FAUCET_RPC_URL", "FAUCET_RPC_FALLBACKS")
            .expect_err("empty primary");
        assert_eq!(empty.to_string(), "FAUCET_RPC_URL is empty");

        let invalid = parse_rpc_endpoints(
            "https://a.example",
            "not a url secret-key",
            "FAUCET_RPC_URL",
            "FAUCET_RPC_FALLBACKS",
        )
        .expect_err("invalid fallback");
        assert_eq!(
            invalid.to_string(),
            "FAUCET_RPC_FALLBACKS entry 1 is not a valid URL"
        );
        assert!(!format!("{invalid:#}").contains("secret-key"));

        assert!(parse_rpc_endpoints("ws://a.example", "", "P", "F").is_err());
    }

    #[test]
    fn endpoint_parsing_bounds_the_endpoint_count() {
        let fallbacks = (1..=MAX_RPC_ENDPOINTS)
            .map(|index| format!("https://fallback{index}.example"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_rpc_endpoints("https://primary.example", &fallbacks, "P", "F").is_err());
    }

    /// Endpoints listed in `answers` return their scripted result; any other
    /// endpoint fails with an error that names it.
    async fn scripted_call(answers: HashMap<String, Value>, endpoint: String) -> Result<Value> {
        answers
            .get(&endpoint)
            .cloned()
            .ok_or_else(|| anyhow!("endpoint {endpoint} unavailable"))
    }

    fn endpoints(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[tokio::test]
    async fn routed_call_with_one_endpoint_keeps_its_answer_and_error() {
        let answers = HashMap::from([("one".to_string(), json!("0x1"))]);
        let result = routed_json_call(&endpoints(&["one"]), "eth_getLogs", |endpoint| {
            scripted_call(answers.clone(), endpoint)
        })
        .await
        .expect("single endpoint answer");
        assert_eq!(result, json!("0x1"));

        let error = routed_json_call(&endpoints(&["missing"]), "eth_getLogs", |endpoint| {
            scripted_call(HashMap::new(), endpoint)
        })
        .await
        .expect_err("single endpoint error");
        assert_eq!(error.to_string(), "endpoint missing unavailable");
    }

    #[tokio::test]
    async fn routed_call_outvotes_a_provider_that_returns_no_logs() {
        let full = json!([log("0x1", None)]);
        let answers = HashMap::from([
            ("pruned".to_string(), json!([])),
            ("full".to_string(), full.clone()),
            (
                "full-with-extras".to_string(),
                json!([log("0x1", Some(("blockTimestamp", json!("0x1"))))]),
            ),
        ]);
        let result = routed_json_call(
            &endpoints(&["pruned", "full", "full-with-extras"]),
            "eth_getLogs",
            |endpoint| scripted_call(answers.clone(), endpoint),
        )
        .await
        .expect("two of three agree");
        assert_eq!(result, full);
    }

    #[tokio::test]
    async fn routed_call_fails_closed_without_quorum() {
        let answers = HashMap::from([
            ("pruned".to_string(), json!([])),
            ("full".to_string(), json!([log("0x1", None)])),
        ]);
        let error = routed_json_call(
            &endpoints(&["pruned", "full", "offline"]),
            "eth_getLogs",
            |endpoint| scripted_call(answers.clone(), endpoint),
        )
        .await
        .expect_err("no majority");
        let message = error.to_string();
        assert!(message.contains("did not reach read quorum"), "{message}");
        assert!(!message.contains("offline"), "{message}");
    }

    #[tokio::test]
    async fn routed_call_fails_over_for_non_security_reads() {
        let answers = HashMap::from([("second".to_string(), json!("0x10"))]);
        let result = routed_json_call(
            &endpoints(&["first", "second"]),
            "eth_getBlockByNumber",
            |endpoint| scripted_call(answers.clone(), endpoint),
        )
        .await
        .expect("failover");
        assert_eq!(result, json!("0x10"));
    }

    #[tokio::test]
    async fn routed_call_sends_side_effects_to_the_primary_only() {
        let answers = HashMap::from([("second".to_string(), json!("0xhash"))]);
        let error = routed_json_call(
            &endpoints(&["first", "second"]),
            "eth_sendRawTransaction",
            |endpoint| scripted_call(answers.clone(), endpoint),
        )
        .await
        .expect_err("primary only");
        assert_eq!(
            error.to_string(),
            "eth_sendRawTransaction failed on configured Ethereum RPC endpoint(s) 1"
        );
    }

    /// Start a JSON-RPC stub that answers every request with `result`, or with
    /// HTTP 503 when `result` is `None`.
    async fn spawn_rpc_stub(result: Option<Value>) -> String {
        spawn_counting_rpc_stub(result).await.0
    }

    /// [`spawn_rpc_stub`], also returning how many requests it received.
    async fn spawn_counting_rpc_stub(result: Option<Value>) -> (String, Arc<AtomicU64>) {
        let hits = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&hits);
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let result = result.clone();
                counter.fetch_add(1, Ordering::Relaxed);
                async move {
                    let id = body.get("id").cloned().unwrap_or(Value::Null);
                    match result {
                        Some(value) => (
                            axum::http::StatusCode::OK,
                            axum::Json(json!({ "jsonrpc": "2.0", "id": id, "result": value })),
                        ),
                        None => (
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                            axum::Json(json!({ "error": "unavailable" })),
                        ),
                    }
                }
            }),
        );
        (serve_stub(app).await, hits)
    }

    /// Start a JSON-RPC stub that accepts requests and never answers.
    async fn spawn_hanging_rpc_stub() -> String {
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(|| async {
                std::future::pending::<()>().await;
                ""
            }),
        );
        serve_stub(app).await
    }

    async fn serve_stub(app: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind RPC stub");
        let address = listener.local_addr().expect("RPC stub address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{address}/")
    }

    #[tokio::test]
    async fn provider_returns_the_majority_chain_id() {
        let urls = [
            spawn_rpc_stub(Some(json!("0x1"))).await,
            spawn_rpc_stub(Some(json!("0xaa36a7"))).await,
            spawn_rpc_stub(Some(json!("0xaa36a7"))).await,
        ];
        let provider = build_provider_from_urls(&urls).expect("provider");
        assert_eq!(provider.get_chain_id().await.expect("quorum"), 11_155_111);
    }

    #[tokio::test]
    async fn provider_fails_closed_without_quorum() {
        let urls = [
            spawn_rpc_stub(Some(json!("0x1"))).await,
            spawn_rpc_stub(Some(json!("0xaa36a7"))).await,
            spawn_rpc_stub(None).await,
        ];
        let provider = build_provider_from_urls(&urls).expect("provider");
        assert!(provider.get_chain_id().await.is_err());
    }

    #[tokio::test]
    async fn provider_requires_quorum_for_contract_code() {
        // A provider that reports no bytecode must not hide a deployed
        // contract, and one that reports bytecode must not invent one.
        let urls = [
            spawn_rpc_stub(Some(json!("0x"))).await,
            spawn_rpc_stub(Some(json!("0x6080"))).await,
            spawn_rpc_stub(Some(json!("0x6080"))).await,
        ];
        let provider = build_provider_from_urls(&urls).expect("provider");
        let code = provider
            .get_code_at(Address::ZERO)
            .await
            .expect("two of three agree");
        assert_eq!(code.to_vec(), vec![0x60_u8, 0x80]);

        let split = [
            spawn_rpc_stub(Some(json!("0x"))).await,
            spawn_rpc_stub(Some(json!("0x6080"))).await,
            spawn_rpc_stub(None).await,
        ];
        let provider = build_provider_from_urls(&split).expect("provider");
        assert!(provider.get_code_at(Address::ZERO).await.is_err());
    }

    #[tokio::test]
    async fn non_security_reads_still_fail_over() {
        let urls = [
            spawn_rpc_stub(None).await,
            spawn_rpc_stub(Some(json!("0x10"))).await,
        ];
        let provider = build_provider_from_urls(&urls).expect("provider");
        assert_eq!(provider.get_block_number().await.expect("failover"), 16);
    }

    // ── Phase 3: early quorum and endpoint health ─────────────────────────────
    // Routed-call tests use endpoint names no other test uses, because health
    // is shared per endpoint list for the whole process.

    #[tokio::test]
    async fn quorum_returns_without_waiting_for_a_hung_endpoint() {
        let answer = json!([log("0x1", None)]);
        let answers = HashMap::from([
            ("agree-a".to_string(), answer.clone()),
            ("agree-b".to_string(), answer.clone()),
        ]);
        let list = endpoints(&["agree-a", "hung-c", "agree-b"]);
        let read = routed_json_call(&list, "eth_getLogs", |endpoint| {
            let answers = answers.clone();
            async move {
                if endpoint == "hung-c" {
                    std::future::pending::<()>().await;
                }
                scripted_call(answers, endpoint).await
            }
        });
        let result = tokio::time::timeout(Duration::from_secs(5), read)
            .await
            .expect("a majority answered, so the hung endpoint must not be awaited")
            .expect("two of three agree");
        assert_eq!(result, answer);
    }

    #[tokio::test]
    async fn quorum_fails_as_soon_as_no_majority_is_possible() {
        // Four configured endpoints need three matching answers. Two disagree
        // and one is down, so the hung fourth cannot complete a majority.
        let answers = HashMap::from([
            ("early-empty".to_string(), json!([])),
            ("early-full".to_string(), json!([log("0x1", None)])),
        ]);
        let list = endpoints(&["early-empty", "early-full", "early-down", "early-hung"]);
        let read = routed_json_call(&list, "eth_getLogs", |endpoint| {
            let answers = answers.clone();
            async move {
                if endpoint == "early-hung" {
                    std::future::pending::<()>().await;
                }
                scripted_call(answers, endpoint).await
            }
        });
        let error = tokio::time::timeout(Duration::from_secs(5), read)
            .await
            .expect("fails without waiting for the hung endpoint")
            .expect_err("no majority is possible");
        assert!(
            error.to_string().contains("did not reach read quorum"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_failing_endpoint_is_skipped_during_its_cool_down() {
        let calls_to_dead = Arc::new(AtomicU64::new(0));
        let answers = HashMap::from([("cool-live".to_string(), json!("0x10"))]);
        let list = endpoints(&["cool-dead", "cool-live"]);
        for _ in 0..3 {
            let result = routed_json_call(&list, "eth_blockNumber", |endpoint| {
                let answers = answers.clone();
                let calls = Arc::clone(&calls_to_dead);
                async move {
                    if endpoint == "cool-dead" {
                        calls.fetch_add(1, Ordering::Relaxed);
                    }
                    scripted_call(answers, endpoint).await
                }
            })
            .await
            .expect("failover answer");
            assert_eq!(result, json!("0x10"));
        }
        assert_eq!(
            calls_to_dead.load(Ordering::Relaxed),
            1,
            "the dead primary is tried once, then skipped while it cools down"
        );
        let stats = endpoint_stats(&list).expect("the list was used");
        assert_eq!(stats[0].failures, 1);
        assert_eq!(stats[0].skips, 2);
        assert!(stats[0].cooling_down);
        assert_eq!(stats[1].successes, 3);
    }

    #[test]
    fn a_cooled_endpoint_leads_again_after_its_cool_down() {
        let health = EndpointHealth::new(3, Duration::from_millis(50));
        assert_eq!(health.failover_order(), vec![0, 1, 2]);
        health.record(0, false);
        assert_eq!(health.failover_order(), vec![1, 2, 0]);
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(health.failover_order(), vec![0, 1, 2]);

        health.record(1, false);
        health.record(1, true);
        assert_eq!(
            health.failover_order(),
            vec![0, 1, 2],
            "a success clears the cool-down"
        );

        let disabled = EndpointHealth::new(2, Duration::ZERO);
        disabled.record(0, false);
        assert_eq!(disabled.failover_order(), vec![0, 1]);
        assert_eq!(disabled.stats()[0].failures, 1);
    }

    #[test]
    fn endpoints_that_are_all_cooling_are_still_tried_in_order() {
        let health = EndpointHealth::new(2, Duration::from_secs(60));
        health.record(0, false);
        health.record(1, false);
        assert_eq!(health.failover_order(), vec![0, 1]);
    }

    #[tokio::test]
    async fn side_effects_go_to_a_cooling_primary_and_nowhere_else() {
        let list = endpoints(&["send-primary", "send-other"]);
        let calls_to_other = Arc::new(AtomicU64::new(0));
        let call = |endpoint: String| {
            let calls = Arc::clone(&calls_to_other);
            async move {
                if endpoint == "send-other" {
                    calls.fetch_add(1, Ordering::Relaxed);
                }
                scripted_call(HashMap::new(), endpoint).await
            }
        };

        // A failed read puts the primary into its cool-down.
        assert!(routed_json_call(&list, "eth_blockNumber", call)
            .await
            .is_err());
        assert!(endpoint_stats(&list).expect("the list was used")[0].cooling_down);

        let before = calls_to_other.load(Ordering::Relaxed);
        let error = routed_json_call(&list, "eth_sendRawTransaction", call)
            .await
            .expect_err("the primary is down");
        assert_eq!(
            error.to_string(),
            "eth_sendRawTransaction failed on configured Ethereum RPC endpoint(s) 1"
        );
        assert_eq!(
            calls_to_other.load(Ordering::Relaxed),
            before,
            "a call with side effects never fails over"
        );
    }

    #[tokio::test]
    async fn endpoint_counters_never_carry_urls() {
        let list = endpoints(&[
            "https://rpc.example/v3/secret-key-123",
            "https://backup.example/?apikey=secret-key-456",
        ]);
        let answers = HashMap::from([(list[1].clone(), json!("0x1"))]);
        routed_json_call(&list, "eth_blockNumber", |endpoint| {
            scripted_call(answers.clone(), endpoint)
        })
        .await
        .expect("failover");
        let stats = endpoint_stats(&list).expect("the list was used");
        assert!(!format!("{stats:?}").contains("secret-key"));
        assert_eq!(
            stats.iter().map(|s| s.position).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(endpoint_stats(&endpoints(&["never-used"])), None);
    }

    #[tokio::test]
    async fn provider_skips_a_dead_primary_while_it_cools_down() {
        let (dead, dead_hits) = spawn_counting_rpc_stub(None).await;
        let (live, live_hits) = spawn_counting_rpc_stub(Some(json!("0x10"))).await;
        let provider = build_provider_from_urls(&[dead, live]).expect("provider");
        for _ in 0..3 {
            assert_eq!(provider.get_block_number().await.expect("failover"), 16);
        }
        assert_eq!(dead_hits.load(Ordering::Relaxed), 1);
        assert_eq!(live_hits.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn providers_built_from_the_same_list_share_health() {
        // The node builds a provider per operation, so health must outlive one.
        let (dead, dead_hits) = spawn_counting_rpc_stub(None).await;
        let (live, _) = spawn_counting_rpc_stub(Some(json!("0x10"))).await;
        let urls = [dead, live];
        for _ in 0..3 {
            let provider = build_provider_from_urls(&urls).expect("provider");
            assert_eq!(provider.get_block_number().await.expect("failover"), 16);
        }
        assert_eq!(dead_hits.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn provider_quorum_does_not_wait_for_a_hung_endpoint() {
        let urls = [
            spawn_rpc_stub(Some(json!("0xaa36a7"))).await,
            spawn_hanging_rpc_stub().await,
            spawn_rpc_stub(Some(json!("0xaa36a7"))).await,
        ];
        let provider = build_provider_from_urls(&urls).expect("provider");
        // The HTTP timeout is 15 s, so finishing well inside 5 s shows the
        // hung endpoint was not waited for.
        let chain_id = tokio::time::timeout(Duration::from_secs(5), async {
            provider.get_chain_id().await
        })
        .await
        .expect("quorum without the hung endpoint")
        .expect("two of three agree");
        assert_eq!(chain_id, 11_155_111);
    }
}
