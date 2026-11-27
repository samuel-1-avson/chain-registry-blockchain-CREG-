//! Native CREG JSON-RPC 2.0 endpoint (`POST /rpc`, `POST /jsonrpc`).
//!
//! Every method is a read. Requests follow JSON-RPC 2.0 strictly:
//! - a call must be an object with `"jsonrpc": "2.0"` and a string `method`;
//!   `params`, if present, is an array or object; `id`, if present, is a
//!   string, number or null;
//! - a batch holds 1 to [`MAX_BATCH`] calls and is answered with an array in
//!   request order;
//! - a notification (a call without `id`) gets no reply, and a request made
//!   only of notifications is answered with HTTP 204.
//!
//! Rate limiting: the middleware charges each HTTP request once against the
//! JSON-RPC bucket, and [`handle`] charges the remaining members of a batch,
//! so a batch of N costs N calls.

use std::{
    net::{IpAddr, SocketAddr},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use axum::{
    body::Bytes,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{SecondsFormat, Utc};
use serde_json::{json, Value};

use crate::{chain_store::ChainStore, rate_limit::RateLimiter, SharedState};

const JSONRPC_VERSION: &str = "2.0";
/// Largest batch accepted in one request. Matches the Sepolia gateway's cap
/// (`explorer/rpc-allowlist.js`).
pub const MAX_BATCH: usize = 20;
/// Longest unknown method name echoed back in a "Method not found" error.
const MAX_METHOD_ECHO: usize = 64;
/// Sync lag, in blocks, that still counts as healthy (same as `/v1/health`).
const SYNC_HEALTHY_MAX_LAG: u64 = 10;

/// Methods with their own metrics series. Anything else is counted as `other`.
const METHODS: [&str; 5] = [
    "creg_chainId",
    "creg_blockNumber",
    "creg_getBlockByNumber",
    "creg_health",
    "creg_syncing",
];

struct MethodCounters {
    ok: AtomicU64,
    error: AtomicU64,
    micros: AtomicU64,
}

impl MethodCounters {
    const fn new() -> Self {
        Self {
            ok: AtomicU64::new(0),
            error: AtomicU64::new(0),
            micros: AtomicU64::new(0),
        }
    }
}

static METHOD_COUNTERS: [MethodCounters; METHODS.len() + 1] =
    [const { MethodCounters::new() }; METHODS.len() + 1];
static PARSE_ERRORS: AtomicU64 = AtomicU64::new(0);
static INVALID_REQUESTS: AtomicU64 = AtomicU64::new(0);
static BATCH_SIZE_ERRORS: AtomicU64 = AtomicU64::new(0);
static BATCHES: AtomicU64 = AtomicU64::new(0);
static NOTIFICATIONS: AtomicU64 = AtomicU64::new(0);

fn method_counters(method: &str) -> &'static MethodCounters {
    let index = METHODS
        .iter()
        .position(|known| *known == method)
        .unwrap_or(METHODS.len());
    &METHOD_COUNTERS[index]
}

fn metric_methods() -> impl Iterator<Item = (&'static str, &'static MethodCounters)> {
    // By value: the array is a `const`, so a borrowing iterator would point at
    // a temporary that does not outlive this function.
    METHODS
        .into_iter()
        .chain(std::iter::once("other"))
        .zip(METHOD_COUNTERS.iter())
}

/// Append the JSON-RPC series to the node's Prometheus text (`metrics::render`).
pub fn render_metrics(out: &mut String) {
    out.push_str("# HELP creg_rpc_requests_total JSON-RPC calls answered, by method and outcome\n");
    out.push_str("# TYPE creg_rpc_requests_total counter\n");
    for (method, counters) in metric_methods() {
        for (outcome, counter) in [("ok", &counters.ok), ("error", &counters.error)] {
            out.push_str(&format!(
                "creg_rpc_requests_total{{method=\"{method}\",outcome=\"{outcome}\"}} {}\n",
                counter.load(Ordering::Relaxed)
            ));
        }
    }
    out.push('\n');

    out.push_str(
        "# HELP creg_rpc_request_duration_seconds_total Time spent answering JSON-RPC calls, by method\n",
    );
    out.push_str("# TYPE creg_rpc_request_duration_seconds_total counter\n");
    for (method, counters) in metric_methods() {
        out.push_str(&format!(
            "creg_rpc_request_duration_seconds_total{{method=\"{method}\"}} {}\n",
            counters.micros.load(Ordering::Relaxed) as f64 / 1_000_000.0
        ));
    }
    out.push('\n');

    out.push_str(
        "# HELP creg_rpc_rejected_total JSON-RPC requests refused before any method ran, by reason\n",
    );
    out.push_str("# TYPE creg_rpc_rejected_total counter\n");
    for (reason, counter) in [
        ("parse_error", &PARSE_ERRORS),
        ("invalid_request", &INVALID_REQUESTS),
        ("batch_size", &BATCH_SIZE_ERRORS),
    ] {
        out.push_str(&format!(
            "creg_rpc_rejected_total{{reason=\"{reason}\"}} {}\n",
            counter.load(Ordering::Relaxed)
        ));
    }
    out.push('\n');

    for (name, help, counter) in [
        (
            "creg_rpc_batches_total",
            "JSON-RPC batch requests accepted",
            &BATCHES,
        ),
        (
            "creg_rpc_notifications_total",
            "JSON-RPC notifications received (not executed, never answered)",
            &NOTIFICATIONS,
        ),
    ] {
        out.push_str(&format!(
            "# HELP {name} {help}\n# TYPE {name} counter\n{name} {}\n\n",
            counter.load(Ordering::Relaxed)
        ));
    }
}

pub async fn handle(
    State(state): State<SharedState>,
    limiter: Option<Extension<RateLimiter>>,
    connect: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let payload = match serde_json::from_slice::<Value>(&body) {
        Ok(payload) => payload,
        Err(error) => {
            PARSE_ERRORS.fetch_add(1, Ordering::Relaxed);
            return reply(error_value(
                Value::Null,
                -32700,
                "Parse error",
                Some(json!(error.to_string())),
            ));
        }
    };

    let calls = match payload {
        Value::Array(calls) => calls,
        call => {
            return match handle_call(&state, call).await {
                Some(response) => reply(response),
                None => StatusCode::NO_CONTENT.into_response(),
            };
        }
    };

    if calls.is_empty() || calls.len() > MAX_BATCH {
        BATCH_SIZE_ERRORS.fetch_add(1, Ordering::Relaxed);
        return reply(error_value(
            Value::Null,
            -32600,
            "Invalid Request",
            Some(json!(format!(
                "a batch must contain 1 to {MAX_BATCH} calls"
            ))),
        ));
    }

    // The middleware charged this HTTP request as one call; charge the rest.
    if let Some(Extension(limiter)) = &limiter {
        let peer = connect
            .map(|ConnectInfo(addr)| addr.ip())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        let ip = crate::rate_limit::extract_client_ip(peer, &headers, limiter.trusted_proxies());
        if !limiter.charge_rpc(ip, calls.len() - 1) {
            return crate::rate_limit::json_rpc_rate_limited(limiter.window());
        }
    }

    BATCHES.fetch_add(1, Ordering::Relaxed);
    let mut responses = Vec::with_capacity(calls.len());
    for call in calls {
        if let Some(response) = handle_call(&state, call).await {
            responses.push(response);
        }
    }
    if responses.is_empty() {
        StatusCode::NO_CONTENT.into_response()
    } else {
        reply(Value::Array(responses))
    }
}

/// Validate and run one call. Returns `None` for a notification.
async fn handle_call(state: &SharedState, call: Value) -> Option<Value> {
    let Value::Object(mut call) = call else {
        return Some(invalid_request(
            Value::Null,
            "a JSON-RPC call must be an object",
        ));
    };

    let id = match call.remove("id") {
        None => None,
        Some(id @ (Value::Null | Value::String(_) | Value::Number(_))) => Some(id),
        Some(_) => {
            return Some(invalid_request(
                Value::Null,
                "id must be a string, a number or null",
            ))
        }
    };
    let reply_id = id.clone().unwrap_or(Value::Null);

    if call.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        return Some(invalid_request(reply_id, "jsonrpc must be \"2.0\""));
    }
    let Some(method) = call
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return Some(invalid_request(reply_id, "method must be a string"));
    };
    let params = match call.remove("params") {
        None => None,
        Some(params @ (Value::Array(_) | Value::Object(_))) => Some(params),
        Some(_) => {
            return Some(invalid_request(
                reply_id,
                "params must be an array or an object",
            ))
        }
    };

    // Every creg_* method is a read, so running a notification would only
    // spend work on an answer nobody asked for.
    if id.is_none() {
        NOTIFICATIONS.fetch_add(1, Ordering::Relaxed);
        return None;
    }

    let started = Instant::now();
    let outcome = execute(state, &method, params).await;
    let counters = method_counters(&method);
    let elapsed = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    counters.micros.fetch_add(elapsed, Ordering::Relaxed);

    Some(match outcome {
        Ok(result) => {
            counters.ok.fetch_add(1, Ordering::Relaxed);
            json!({
                "jsonrpc": JSONRPC_VERSION,
                "result": result,
                "id": reply_id,
            })
        }
        Err(error) => {
            counters.error.fetch_add(1, Ordering::Relaxed);
            error_value(reply_id, error.code, error.message, error.data)
        }
    })
}

async fn execute(
    state: &SharedState,
    method: &str,
    params: Option<Value>,
) -> Result<Value, RpcError> {
    match method {
        "creg_chainId" => creg_chain_id(state).await.map(Value::String),
        "creg_blockNumber" => creg_block_number(state).await.map(Value::String),
        "creg_getBlockByNumber" => creg_get_block_by_number(state, params).await,
        "creg_health" => creg_health(state).await,
        "creg_syncing" => creg_syncing(state).await,
        _ => Err(RpcError {
            code: -32601,
            message: "Method not found",
            data: Some(json!(method
                .chars()
                .take(MAX_METHOD_ECHO)
                .collect::<String>())),
        }),
    }
}

/// A handle on the chain store that does not hold the node state lock while
/// RocksDB is read, so RPC reads never wait on, or delay, a block commit.
async fn chain_store(state: &SharedState) -> ChainStore {
    state.read().await.chain.clone()
}

async fn creg_chain_id(state: &SharedState) -> Result<String, RpcError> {
    let state = state.read().await;
    Ok(node_chain_id(&state.config))
}

async fn creg_block_number(state: &SharedState) -> Result<String, RpcError> {
    let height = chain_store(state)
        .await
        .tip_height()
        .map_err(internal_error)?;
    Ok(hex_quantity(height))
}

/// Node status, including what a client needs to tell an idle chain from a
/// stalled one. Blocks are produced only when finalized work exists, so a
/// large `tip_age_secs` alone means "idle"; with `pending_pool_size > 0` it
/// suggests a stall.
async fn creg_health(state: &SharedState) -> Result<Value, RpcError> {
    let (chain, chain_id, validator_set_sync, lag_blocks, max_peer_tip, peer_count, pending) = {
        let s = state.read().await;
        (
            s.chain.clone(),
            node_chain_id(&s.config),
            s.validator_set_sync.clone(),
            s.sync_lag_blocks,
            s.sync_max_peer_tip,
            s.p2p_status.peers.len(),
            s.pending_pool.len(),
        )
    };
    let height = chain.tip_height().map_err(internal_error)?;
    let tip_timestamp = chain
        .get_block_by_height(height)
        .map_err(internal_error)?
        .map(|block| block.header.timestamp);
    let tip_age_secs =
        tip_timestamp.map(|timestamp| crate::metrics::tip_age_seconds(timestamp, Utc::now()));

    Ok(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "chain_id": chain_id,
        "block_number": hex_quantity(height),
        "tip_timestamp": tip_timestamp
            .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)),
        "tip_age_secs": tip_age_secs,
        "pending_pool_size": pending,
        "peer_count": peer_count,
        "sync": {
            "lag_blocks": lag_blocks,
            "max_peer_tip": max_peer_tip,
            "healthy": lag_blocks <= SYNC_HEALTHY_MAX_LAG,
        },
        "validator_set_sync": validator_set_sync,
    }))
}

/// `false` when this node is caught up with its peers, otherwise its progress
/// (shaped like `eth_syncing`).
async fn creg_syncing(state: &SharedState) -> Result<Value, RpcError> {
    let (chain, lag_blocks, max_peer_tip) = {
        let s = state.read().await;
        (s.chain.clone(), s.sync_lag_blocks, s.sync_max_peer_tip)
    };
    if lag_blocks == 0 {
        return Ok(Value::Bool(false));
    }
    let height = chain.tip_height().map_err(internal_error)?;
    Ok(json!({
        "currentBlock": hex_quantity(height),
        "highestBlock": hex_quantity(max_peer_tip.max(height)),
        "lagBlocks": hex_quantity(lag_blocks),
    }))
}

async fn creg_get_block_by_number(
    state: &SharedState,
    params: Option<Value>,
) -> Result<Value, RpcError> {
    let height = parse_block_number(params)?;
    let chain = chain_store(state).await;
    let resolved_height = match height {
        BlockNumberParam::Latest => chain.tip_height().map_err(internal_error)?,
        BlockNumberParam::Number(height) => height,
    };

    match chain
        .get_block_by_height(resolved_height)
        .map_err(internal_error)?
    {
        Some(block) => Ok(block_to_json(&block)),
        None => Ok(Value::Null),
    }
}

#[derive(Debug, Clone, Copy)]
enum BlockNumberParam {
    Latest,
    Number(u64),
}

fn parse_block_number(params: Option<Value>) -> Result<BlockNumberParam, RpcError> {
    let Some(Value::Array(params)) = params else {
        return Err(invalid_params(
            "creg_getBlockByNumber expects params array: [height | \"latest\"]",
        ));
    };
    let Some(first) = params.first() else {
        return Err(invalid_params(
            "creg_getBlockByNumber requires a block number parameter",
        ));
    };

    match first {
        Value::String(value) if value.eq_ignore_ascii_case("latest") => {
            Ok(BlockNumberParam::Latest)
        }
        Value::String(value) if value.starts_with("0x") => u64::from_str_radix(&value[2..], 16)
            .map(BlockNumberParam::Number)
            .map_err(|_| invalid_params("block number hex quantity is invalid")),
        Value::Number(value) => value
            .as_u64()
            .map(BlockNumberParam::Number)
            .ok_or_else(|| invalid_params("block number must be a non-negative integer")),
        _ => Err(invalid_params(
            "block number must be a non-negative integer, hex quantity, or \"latest\"",
        )),
    }
}

/// Every block in the chain store is the genesis block or carried a PBFT
/// commit quorum when it was stored, so `finalized` is always true (the REST
/// block endpoints say the same, see `api::block_to_json`).
fn block_to_json(block: &common::Block) -> Value {
    let mut value = serde_json::to_value(block).unwrap_or(Value::Null);
    if let Value::Object(ref mut map) = value {
        map.insert("hash".into(), Value::String(block.hash()));
        map.insert("finalized".into(), Value::Bool(true));
    }
    value
}

fn node_chain_id(config: &crate::config::NodeConfig) -> String {
    if !config.chain_id.trim().is_empty() {
        config.chain_id.clone()
    } else if config.is_testnet {
        "creg-testnet-1".to_string()
    } else {
        "creg-mainnet-1".to_string()
    }
}

fn hex_quantity(value: u64) -> String {
    format!("0x{value:x}")
}

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: &'static str,
    data: Option<Value>,
}

fn invalid_params(message: impl Into<String>) -> RpcError {
    RpcError {
        code: -32602,
        message: "Invalid params",
        data: Some(json!(message.into())),
    }
}

fn internal_error(error: impl std::fmt::Display) -> RpcError {
    RpcError {
        code: -32603,
        message: "Internal error",
        data: Some(json!(error.to_string())),
    }
}

fn invalid_request(id: Value, detail: &str) -> Value {
    INVALID_REQUESTS.fetch_add(1, Ordering::Relaxed);
    error_value(id, -32600, "Invalid Request", Some(json!(detail)))
}

fn reply(value: Value) -> Response {
    Json(value).into_response()
}

fn error_value(id: Value, code: i64, message: &'static str, data: Option<Value>) -> Value {
    let mut error = serde_json::Map::new();
    error.insert("code".into(), json!(code));
    error.insert("message".into(), json!(message));
    if let Some(data) = data {
        error.insert("data".into(), data);
    }

    json!({
        "jsonrpc": JSONRPC_VERSION,
        "error": Value::Object(error),
        "id": id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chain_store::ChainStore,
        config::NodeConfig,
        pending_pool::PendingPool,
        publisher_index::PublisherIndex,
        rate_limit::RateLimitConfig,
        state::{
            BridgeStatus, NodeState, P2PStatus, ValidatorRegistrationStatus, ValidatorSetSyncStatus,
        },
    };
    use std::{collections::HashMap, sync::Arc};
    use tempfile::TempDir;
    use tokio::sync::RwLock;

    async fn test_state() -> anyhow::Result<(SharedState, TempDir)> {
        let tempdir = tempfile::tempdir()?;
        let chain = ChainStore::open(tempdir.path())?;
        let state = Arc::new(RwLock::new(NodeState {
            chain,
            pending_pool: PendingPool::new(),
            publisher_index: PublisherIndex::new(),
            validator_set_bootstrap: common::ValidatorSet::default(),
            validator_set: common::ValidatorSet::default(),
            validator_set_snapshot: Arc::new(crate::state::ArcSwap::from_pointee(
                common::ValidatorSet::default(),
            )),
            package_rounds: HashMap::new(),
            package_committees: HashMap::new(),
            config: NodeConfig {
                chain_id: "creg-testnet-1".into(),
                data_dir: tempdir.path().to_path_buf(),
                is_testnet: true,
                ..Default::default()
            },
            p2p_status: P2PStatus::default(),
            bridge_status: BridgeStatus::default(),
            vrf_proofs: HashMap::new(),
            decryption_shares: HashMap::new(),
            validator_registrations: HashMap::<String, ValidatorRegistrationStatus>::new(),
            validator_set_sync: ValidatorSetSyncStatus::default(),
            view_change_certs: HashMap::new(),
            reorgs: Vec::new(),
            pbft_engine: crate::state::PbftEngine::new(),
            forced_inclusion_tracker: crate::state::ForcedInclusionTracker::new(),
            sync_lag_blocks: 0,
            sync_max_peer_tip: 0,
            reward_emissions_succeeded: 0,
            reward_emissions_failed: 0,
            reward_emissions_skipped: 0,
            treasury_balance: 0,
            pending_rewards_estimate: 0,
            total_rewards_distributed: 0,
            rewards_pool_balance: 0,
            last_incentives_refresh: None,
            publisher_stake_cache: HashMap::new(),
            equivocation: crate::equivocation_monitor::EquivocationMonitor::disabled(),
        }));

        Ok((state, tempdir))
    }

    /// One call with an `id`, answered without going through HTTP.
    async fn call(state: &SharedState, body: Value) -> Value {
        handle_call(state, body)
            .await
            .expect("a call with an id gets a reply")
    }

    /// A raw HTTP body through [`handle`], with an optional rate limiter.
    async fn post_with(
        state: &SharedState,
        limiter: Option<RateLimiter>,
        body: &str,
    ) -> anyhow::Result<(StatusCode, Option<Value>)> {
        let response = handle(
            State(Arc::clone(state)),
            limiter.map(Extension),
            None,
            HeaderMap::new(),
            Bytes::from(body.to_owned()),
        )
        .await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
        let json = if bytes.is_empty() {
            None
        } else {
            Some(serde_json::from_slice(&bytes)?)
        };
        Ok((status, json))
    }

    async fn post(state: &SharedState, body: &str) -> anyhow::Result<(StatusCode, Option<Value>)> {
        post_with(state, None, body).await
    }

    #[tokio::test]
    async fn creg_block_number_returns_tip_as_hex_quantity() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let response = call(
            &state,
            json!({
                "jsonrpc": "2.0",
                "method": "creg_blockNumber",
                "params": [],
                "id": 1,
            }),
        )
        .await;

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["result"], "0x0");
        assert_eq!(response["id"], 1);
        Ok(())
    }

    #[tokio::test]
    async fn creg_get_block_by_number_returns_genesis_block() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let response = call(
            &state,
            json!({
                "jsonrpc": "2.0",
                "method": "creg_getBlockByNumber",
                "params": ["0x0"],
                "id": "block-0",
            }),
        )
        .await;

        assert_eq!(response["result"]["header"]["height"], 0);
        assert_eq!(response["result"]["finalized"], true);
        assert!(response["result"]["hash"].as_str().is_some());
        assert_eq!(response["id"], "block-0");
        Ok(())
    }

    #[tokio::test]
    async fn unknown_method_returns_json_rpc_error() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let response = call(
            &state,
            json!({
                "jsonrpc": "2.0",
                "method": "creg_missingMethod",
                "id": 99,
            }),
        )
        .await;

        assert_eq!(response["error"]["code"], -32601);
        assert_eq!(response["error"]["data"], "creg_missingMethod");
        assert_eq!(response["id"], 99);
        Ok(())
    }

    #[tokio::test]
    async fn unknown_method_echo_is_truncated() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let long_name = "x".repeat(500);
        let response = call(
            &state,
            json!({ "jsonrpc": "2.0", "method": long_name, "id": 1 }),
        )
        .await;
        assert_eq!(
            response["error"]["data"].as_str().map(str::len),
            Some(MAX_METHOD_ECHO)
        );
        Ok(())
    }

    #[tokio::test]
    async fn malformed_block_params_return_invalid_params() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let response = call(
            &state,
            json!({
                "jsonrpc": "2.0",
                "method": "creg_getBlockByNumber",
                "params": ["nope"],
                "id": 7,
            }),
        )
        .await;

        assert_eq!(response["error"]["code"], -32602);
        assert_eq!(response["id"], 7);
        Ok(())
    }

    #[tokio::test]
    async fn parse_error_has_null_id() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let (status, body) = post(&state, "{\"jsonrpc\":").await?;
        assert_eq!(status, StatusCode::OK);
        let body = body.expect("parse errors are answered");
        assert_eq!(body["error"]["code"], -32700);
        assert!(body["id"].is_null());
        Ok(())
    }

    #[tokio::test]
    async fn batch_is_answered_in_request_order() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let (status, body) = post(
            &state,
            r#"[{"jsonrpc":"2.0","method":"creg_chainId","id":1},
                {"jsonrpc":"2.0","method":"creg_blockNumber","params":[],"id":"two"}]"#,
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        let body = body.expect("a batch with ids is answered");
        let items = body.as_array().expect("a batch is answered with an array");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["id"], 1);
        assert_eq!(items[0]["result"], "creg-testnet-1");
        assert_eq!(items[1]["id"], "two");
        assert_eq!(items[1]["result"], "0x0");
        Ok(())
    }

    #[tokio::test]
    async fn notifications_get_no_reply() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;

        let (status, body) =
            post(&state, r#"{"jsonrpc":"2.0","method":"creg_blockNumber"}"#).await?;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_none());

        let (status, body) = post(
            &state,
            r#"[{"jsonrpc":"2.0","method":"creg_blockNumber","id":1},
                {"jsonrpc":"2.0","method":"creg_chainId"}]"#,
        )
        .await?;
        assert_eq!(status, StatusCode::OK);
        let body = body.expect("the call with an id is answered");
        assert_eq!(body.as_array().map(Vec::len), Some(1));
        assert_eq!(body[0]["id"], 1);

        let (status, body) = post(
            &state,
            r#"[{"jsonrpc":"2.0","method":"creg_blockNumber"},
                {"jsonrpc":"2.0","method":"creg_chainId"}]"#,
        )
        .await?;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(body.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn empty_and_oversized_batches_are_invalid() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;

        let (_, body) = post(&state, "[]").await?;
        let body = body.expect("an empty batch is answered");
        assert_eq!(body["error"]["code"], -32600);
        assert!(body["id"].is_null());

        let oversized: Vec<Value> = (0..=MAX_BATCH)
            .map(|id| json!({ "jsonrpc": "2.0", "method": "creg_chainId", "id": id }))
            .collect();
        let (_, body) = post(&state, &serde_json::to_string(&oversized)?).await?;
        let body = body.expect("an oversized batch is answered");
        assert_eq!(body["error"]["code"], -32600);
        Ok(())
    }

    #[tokio::test]
    async fn positional_array_is_not_a_request() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let (_, body) = post(&state, r#"["2.0","creg_chainId"]"#).await?;
        let body = body.expect("invalid batch members are answered");
        let items = body.as_array().expect("answered as a batch");
        assert_eq!(items.len(), 2);
        for item in items {
            assert_eq!(item["error"]["code"], -32600);
            assert!(item.get("result").is_none());
        }
        Ok(())
    }

    #[tokio::test]
    async fn requests_must_be_well_formed() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;

        let missing_version = call(&state, json!({ "method": "creg_blockNumber", "id": 1 })).await;
        assert_eq!(missing_version["error"]["code"], -32600);
        assert_eq!(missing_version["id"], 1);

        let wrong_version = call(
            &state,
            json!({ "jsonrpc": "1.0", "method": "creg_blockNumber", "id": 2 }),
        )
        .await;
        assert_eq!(wrong_version["error"]["code"], -32600);

        let object_id = call(
            &state,
            json!({ "jsonrpc": "2.0", "method": "creg_blockNumber", "id": {"x": 1} }),
        )
        .await;
        assert_eq!(object_id["error"]["code"], -32600);
        assert!(object_id["id"].is_null());

        let string_params = call(
            &state,
            json!({ "jsonrpc": "2.0", "method": "creg_blockNumber", "params": "x", "id": 3 }),
        )
        .await;
        assert_eq!(string_params["error"]["code"], -32600);
        assert_eq!(string_params["id"], 3);

        let numeric_method = call(&state, json!({ "jsonrpc": "2.0", "method": 5, "id": 4 })).await;
        assert_eq!(numeric_method["error"]["code"], -32600);

        let null_id = call(
            &state,
            json!({ "jsonrpc": "2.0", "method": "creg_blockNumber", "id": null }),
        )
        .await;
        assert_eq!(
            null_id["result"], "0x0",
            "a null id is a request, not a notification"
        );
        assert!(null_id["id"].is_null());
        Ok(())
    }

    #[tokio::test]
    async fn creg_health_reports_tip_age_and_pending_work() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let response = call(
            &state,
            json!({ "jsonrpc": "2.0", "method": "creg_health", "id": 1 }),
        )
        .await;
        let health = &response["result"];
        assert_eq!(health["status"], "ok");
        assert_eq!(health["block_number"], "0x0");
        assert!(health["tip_timestamp"].as_str().is_some());
        assert!(health["tip_age_secs"].as_i64().is_some_and(|age| age >= 0));
        assert_eq!(health["pending_pool_size"], 0);
        assert_eq!(health["peer_count"], 0);
        assert_eq!(health["sync"]["lag_blocks"], 0);
        assert_eq!(health["sync"]["healthy"], true);
        Ok(())
    }

    #[tokio::test]
    async fn creg_syncing_reports_lag_against_peers() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let request = json!({ "jsonrpc": "2.0", "method": "creg_syncing", "id": 1 });

        let caught_up = call(&state, request.clone()).await;
        assert_eq!(caught_up["result"], false);

        {
            let mut s = state.write().await;
            s.sync_lag_blocks = 5;
            s.sync_max_peer_tip = 5;
        }
        let behind = call(&state, request).await;
        assert_eq!(behind["result"]["currentBlock"], "0x0");
        assert_eq!(behind["result"]["highestBlock"], "0x5");
        assert_eq!(behind["result"]["lagBlocks"], "0x5");
        Ok(())
    }

    #[tokio::test]
    async fn batch_members_are_charged_to_the_rpc_bucket() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        let limiter = RateLimiter::new(RateLimitConfig {
            rpc_limit: 3,
            ..Default::default()
        });
        let batch = r#"[{"jsonrpc":"2.0","method":"creg_chainId","id":1},
                        {"jsonrpc":"2.0","method":"creg_chainId","id":2},
                        {"jsonrpc":"2.0","method":"creg_chainId","id":3}]"#;

        // The middleware is not in this path, so each batch charges only its
        // two extra members: 2 of 3, then 4 of 3.
        let (status, _) = post_with(&state, Some(limiter.clone()), batch).await?;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = post_with(&state, Some(limiter), batch).await?;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        let body = body.expect("a throttled batch is answered");
        assert_eq!(body["error"]["code"], -32005);
        Ok(())
    }

    #[tokio::test]
    async fn rpc_metrics_are_rendered() -> anyhow::Result<()> {
        let (state, _tempdir) = test_state().await?;
        call(
            &state,
            json!({ "jsonrpc": "2.0", "method": "creg_chainId", "id": 1 }),
        )
        .await;

        let mut out = String::new();
        render_metrics(&mut out);
        let line = out
            .lines()
            .find(|line| {
                line.starts_with("creg_rpc_requests_total{method=\"creg_chainId\",outcome=\"ok\"}")
            })
            .expect("creg_chainId ok series is present");
        let count: u64 = line.rsplit(' ').next().unwrap_or("0").parse()?;
        assert!(count >= 1);
        assert_eq!(
            out.matches("# TYPE creg_rpc_requests_total counter")
                .count(),
            1
        );
        assert!(out.contains("creg_rpc_requests_total{method=\"other\",outcome=\"error\"}"));
        assert!(out.contains("creg_rpc_rejected_total{reason=\"batch_size\"}"));
        Ok(())
    }
}
