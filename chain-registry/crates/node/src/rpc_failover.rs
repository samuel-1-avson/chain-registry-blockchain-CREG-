//! Node binding for the shared Ethereum JSON-RPC transport.
//!
//! Endpoints come from `CREG_ETH_RPC` (primary) and the comma-separated
//! `CREG_ETH_RPC_FALLBACKS`. Security-relevant state reads require a strict
//! majority of the configured endpoints to agree, other reads fail over, and
//! calls with possible side effects use the primary once. The policy and its
//! tests live in `common::l1_rpc`, which the faucet, relayer and IPFS pinner
//! share.

use alloy::providers::RootProvider;
use anyhow::Result;

pub use common::l1_rpc::FailoverTransport;

/// Return the ordered primary and fallback URLs, with invalid URLs and
/// duplicate hosts rejected before the provider starts.
pub fn configured_rpc_urls(primary_url: &str) -> Result<Vec<String>> {
    common::l1_rpc::parse_rpc_endpoints(
        primary_url,
        &std::env::var("CREG_ETH_RPC_FALLBACKS").unwrap_or_default(),
        "CREG_ETH_RPC",
        "CREG_ETH_RPC_FALLBACKS",
    )
}

/// Construct the common L1 provider used by node subsystems.
pub fn build_l1_provider(primary_url: &str) -> Result<RootProvider<FailoverTransport>> {
    let urls = configured_rpc_urls(primary_url)?;
    common::l1_rpc::build_provider_from_urls(&urls)
}
