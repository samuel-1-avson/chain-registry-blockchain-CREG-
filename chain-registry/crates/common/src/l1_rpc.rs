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
use serde_json::Value;
use std::{
    future::{poll_fn, Future},
    task::{Context as TaskContext, Poll},
    time::Duration,
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

/// An HTTP JSON-RPC transport that enforces a configured-endpoint quorum for
/// security-relevant state reads and fails over for other safe reads.
#[derive(Clone, Debug)]
pub struct FailoverTransport {
    upstreams: Vec<BoxTransport>,
}

impl FailoverTransport {
    fn new(upstreams: Vec<BoxTransport>) -> Self {
        Self { upstreams }
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
        match request_policy(&request) {
            ReadPolicy::Quorum => Box::pin(quorum_call(upstreams, request)),
            ReadPolicy::Failover => Box::pin(sequential_call(upstreams, request, true)),
            ReadPolicy::PrimaryOnly => Box::pin(sequential_call(upstreams, request, false)),
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

/// Try endpoints in configured order. Without failover only the primary is
/// attempted.
async fn sequential_call(
    upstreams: Vec<BoxTransport>,
    request: RequestPacket,
    can_fail_over: bool,
) -> std::result::Result<ResponsePacket, TransportError> {
    let mut last_error = None;
    let last_attempt = if can_fail_over { upstreams.len() } else { 1 };

    for (index, upstream) in upstreams.into_iter().take(last_attempt).enumerate() {
        match upstream_call(upstream, request.clone()).await {
            Ok(response) => {
                if index > 0 {
                    tracing::warn!(
                        endpoint_index = index + 1,
                        "Ethereum RPC read failed over to a configured endpoint"
                    );
                }
                return Ok(response);
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        TransportErrorKind::custom_str("no Ethereum RPC endpoint is configured")
    }))
}

/// Send the request to every endpoint in parallel and return a response only
/// when a strict majority of the configured endpoints agree on it.
async fn quorum_call(
    upstreams: Vec<BoxTransport>,
    request: RequestPacket,
) -> std::result::Result<ResponsePacket, TransportError> {
    let configured = upstreams.len();
    if configured == 0 {
        return Err(TransportErrorKind::custom_str(
            "no Ethereum RPC endpoint is configured",
        ));
    }

    let results = futures::future::join_all(
        upstreams
            .into_iter()
            .map(|upstream| upstream_call(upstream, request.clone())),
    )
    .await;

    let mut packets = Vec::with_capacity(configured);
    let mut keys = Vec::with_capacity(configured);
    for packet in results.into_iter().flatten() {
        keys.push(packet_comparison_key(&packet));
        packets.push(packet);
    }

    match select_quorum_index(&keys, configured) {
        Some(index) => Ok(packets.swap_remove(index)),
        None => {
            tracing::warn!(
                configured_endpoints = configured,
                responding_endpoints = packets.len(),
                distinct_answers = distinct_answer_count(&keys),
                "Ethereum RPC state read did not reach configured-endpoint quorum; failing closed"
            );
            Err(TransportErrorKind::custom_str(
                "configured Ethereum RPC endpoints did not reach read quorum",
            ))
        }
    }
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
    let client = ClientBuilder::default().transport(FailoverTransport::new(upstreams), false);
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

async fn quorum_json_call<F, Fut>(endpoints: &[String], method: &str, call: F) -> Result<Value>
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    let configured = endpoints.len();
    let results = futures::future::join_all(endpoints.iter().cloned().map(&call)).await;
    let mut answers = Vec::with_capacity(configured);
    let mut keys = Vec::with_capacity(configured);
    for answer in results.into_iter().flatten() {
        keys.push(Some(normalize_rpc_value(answer.clone())));
        answers.push(answer);
    }
    match select_quorum_index(&keys, configured) {
        Some(index) => Ok(answers.swap_remove(index)),
        None => {
            let distinct = distinct_answer_count(&keys);
            tracing::warn!(
                method,
                configured_endpoints = configured,
                responding_endpoints = answers.len(),
                distinct_answers = distinct,
                "Ethereum RPC state read did not reach configured-endpoint quorum; failing closed"
            );
            bail!(
                "{method} did not reach read quorum: {} of {configured} configured endpoints answered with {distinct} distinct answers, {} matching answers required",
                answers.len(),
                configured_quorum_size(configured)
            )
        }
    }
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
    let attempts = if can_fail_over { endpoints.len() } else { 1 };
    let mut failed = Vec::new();
    for (index, endpoint) in endpoints.iter().take(attempts).enumerate() {
        match call(endpoint.clone()).await {
            Ok(answer) => {
                if index > 0 {
                    tracing::warn!(
                        method,
                        endpoint_index = index + 1,
                        "Ethereum RPC read failed over to a configured endpoint"
                    );
                }
                return Ok(answer);
            }
            Err(_) => failed.push((index + 1).to_string()),
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
        let app = axum::Router::new().route(
            "/",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let result = result.clone();
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
}
