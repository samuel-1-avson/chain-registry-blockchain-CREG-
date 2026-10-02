//! Read-only failover for independently configured Ethereum JSON-RPC endpoints.
//!
//! Transaction submission is deliberately sent only once. A transport failure
//! after a signed transaction is sent is ambiguous, and retrying it here could
//! hide that ambiguity from the caller. The durable transaction journal owns
//! that recovery decision.

use alloy::{
    providers::{ProviderBuilder, RootProvider},
    rpc::{client::ClientBuilder, json_rpc::RequestPacket},
    transports::{
        http::{Client, Http},
        BoxTransport, Transport, TransportError, TransportErrorKind, TransportFut,
    },
};
use anyhow::{anyhow, Context, Result};
use std::{
    future::poll_fn,
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tower::Service;

const MAX_RPC_ENDPOINTS: usize = 8;

/// An HTTP JSON-RPC transport that retries safe reads against the configured
/// endpoint list when an upstream transport fails.
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
    type Response = alloy::rpc::json_rpc::ResponsePacket;
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
        let can_fail_over = request_is_read_only(&request);
        let upstreams = self.upstreams.clone();

        Box::pin(async move {
            let mut last_error = None;
            let last_attempt = if can_fail_over { upstreams.len() } else { 1 };

            for (index, mut upstream) in upstreams.into_iter().take(last_attempt).enumerate() {
                if let Err(error) = poll_fn(|cx| upstream.poll_ready(cx)).await {
                    last_error = Some(error);
                    continue;
                }

                match upstream.call(request.clone()).await {
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
        })
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

/// Construct the common L1 provider used by node subsystems. Only JSON-RPC
/// reads are retried; calls with possible side effects use the primary once.
pub fn build_l1_provider(primary_url: &str) -> Result<RootProvider<FailoverTransport>> {
    let urls = configured_rpc_urls(primary_url)?;
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

fn request_is_read_only(packet: &RequestPacket) -> bool {
    match packet {
        RequestPacket::Single(request) => method_is_read_only(request.method()),
        RequestPacket::Batch(requests) => {
            !requests.is_empty()
                && requests
                    .iter()
                    .all(|request| method_is_read_only(request.method()))
        }
    }
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
