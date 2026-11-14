//! Ethereum JSON-RPC transport over independently configured endpoints.
//!
//! Every request is routed by one of three policies:
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
//!   the caller. The durable transaction journal owns that recovery decision.

use alloy::{
    providers::{ProviderBuilder, RootProvider},
    rpc::{
        client::ClientBuilder,
        json_rpc::{RequestPacket, ResponsePacket, ResponsePayload},
    },
    transports::{
        http::{Client, Http},
        BoxTransport, Transport, TransportError, TransportErrorKind, TransportFut,
    },
};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::{
    future::poll_fn,
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tower::Service;

const MAX_RPC_ENDPOINTS: usize = 8;

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

/// Return the index of a response whose comparison key is shared by at least
/// a strict majority of the configured endpoints. Unparseable responses
/// (`None`) never match anything.
fn select_quorum_index(keys: &[Option<Value>], configured: usize) -> Option<usize> {
    let required = crate::l1_quorum::configured_quorum_size(configured);
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
fn normalize_rpc_value(value: Value) -> Value {
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

/// Return the ordered primary and fallback URLs, with invalid URLs and
/// duplicate hosts rejected before the provider starts.
pub fn configured_rpc_urls(primary_url: &str) -> Result<Vec<String>> {
    let mut raw_urls = vec![primary_url.trim().to_string()];
    raw_urls.extend(
        std::env::var("CREG_ETH_RPC_FALLBACKS")
            .unwrap_or_default()
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
                return Err(anyhow!("CREG_ETH_RPC is empty"));
            }
            continue;
        }
        let parsed = reqwest::Url::parse(&raw_url).with_context(|| {
            if index == 0 {
                "CREG_ETH_RPC is not a valid URL".to_string()
            } else {
                format!("CREG_ETH_RPC_FALLBACKS entry {} is not a valid URL", index)
            }
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(anyhow!("Ethereum RPC endpoints must use HTTP or HTTPS"));
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
            return Err(anyhow!(
                "at most {MAX_RPC_ENDPOINTS} distinct Ethereum RPC endpoints may be configured"
            ));
        }
    }

    if urls.is_empty() {
        return Err(anyhow!("no Ethereum RPC endpoints are configured"));
    }
    Ok(urls)
}

/// Construct the common L1 provider used by node subsystems. Security-relevant
/// state reads require a configured-endpoint quorum, other reads fail over,
/// and calls with possible side effects use the primary once.
pub fn build_l1_provider(primary_url: &str) -> Result<RootProvider<FailoverTransport>> {
    let urls = configured_rpc_urls(primary_url)?;
    build_l1_provider_from_urls(&urls)
}

/// Build the provider from an already validated, de-duplicated endpoint list.
fn build_l1_provider_from_urls(urls: &[String]) -> Result<RootProvider<FailoverTransport>> {
    if urls.is_empty() {
        return Err(anyhow!("no Ethereum RPC endpoints are configured"));
    }
    let http_client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .context("failed to build bounded Ethereum RPC HTTP client")?;
    let upstreams = urls
        .iter()
        .map(|url| {
            let parsed = reqwest::Url::parse(url).context("validated Ethereum RPC URL changed")?;
            let transport: BoxTransport =
                Http::<Client>::with_client(http_client.clone(), parsed).boxed();
            Ok(transport)
        })
        .collect::<Result<Vec<_>>>()?;
    let client = ClientBuilder::default().transport(FailoverTransport::new(upstreams), false);
    Ok(ProviderBuilder::new().on_client(client))
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
/// contract code, and chain identity.
fn method_requires_quorum(method: &str) -> bool {
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

fn method_is_read_only(method: &str) -> bool {
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
    use alloy::providers::Provider;
    use serde_json::json;

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
        assert_eq!(select_quorum_index(&[answer.clone()], 1), Some(0));
        assert_eq!(select_quorum_index(&[answer.clone()], 2), None);
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
        let provider = build_l1_provider_from_urls(&urls).expect("provider");
        assert_eq!(provider.get_chain_id().await.expect("quorum"), 11_155_111);
    }

    #[tokio::test]
    async fn provider_fails_closed_without_quorum() {
        let urls = [
            spawn_rpc_stub(Some(json!("0x1"))).await,
            spawn_rpc_stub(Some(json!("0xaa36a7"))).await,
            spawn_rpc_stub(None).await,
        ];
        let provider = build_l1_provider_from_urls(&urls).expect("provider");
        assert!(provider.get_chain_id().await.is_err());
    }

    #[tokio::test]
    async fn non_security_reads_still_fail_over() {
        let urls = [
            spawn_rpc_stub(None).await,
            spawn_rpc_stub(Some(json!("0x10"))).await,
        ];
        let provider = build_l1_provider_from_urls(&urls).expect("provider");
        assert_eq!(provider.get_block_number().await.expect("failover"), 16);
    }
}
