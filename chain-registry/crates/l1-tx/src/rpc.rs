//! The L1 JSON-RPC surface the signer needs, with an implementation over the
//! shared `common::l1_rpc` failover/quorum transport.

use alloy::{
    eips::BlockId,
    network::ReceiptResponse,
    primitives::{Address, Bytes, B256},
    providers::{Provider, RootProvider},
    transports::RpcError,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use common::l1_rpc::FailoverTransport;

/// EIP-1559 fee fields in wei.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeeEstimate {
    pub max_fee_per_gas: u128,
    pub max_priority_fee_per_gas: u128,
}

/// The parts of a receipt the reconciler needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiptSummary {
    pub tx_hash: B256,
    pub block_number: u64,
    pub success: bool,
}

/// Result of one `eth_sendRawTransaction`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BroadcastOutcome {
    /// The node accepted the transaction into its pool.
    Accepted(B256),
    /// The node already has this exact transaction.
    AlreadyKnown,
    /// The nonce is below the account's mined nonce.
    NonceTooLow,
    /// Another transaction holds this nonce in the pool with an equal or
    /// higher fee, or the fee is below the current base fee.
    Underpriced,
    /// Definitively refused (insufficient funds, invalid transaction, ...).
    Rejected(String),
    /// Transport failure: the transaction may or may not have been accepted.
    Uncertain(String),
}

/// L1 calls used by the signer actor.
#[async_trait]
pub trait L1Rpc: Send + Sync + 'static {
    async fn chain_id(&self) -> Result<u64>;
    /// Account nonce at `pending` (`true`) or `latest` (`false`).
    async fn transaction_count(&self, address: Address, pending: bool) -> Result<u64>;
    async fn fee_estimate(&self) -> Result<FeeEstimate>;
    async fn send_raw_transaction(&self, raw: &Bytes) -> BroadcastOutcome;
    async fn receipt(&self, tx_hash: B256) -> Result<Option<ReceiptSummary>>;
    async fn block_number(&self) -> Result<u64>;
}

/// Map a JSON-RPC error response from `eth_sendRawTransaction` to an outcome.
/// Messages differ between clients (geth, Nethermind, Erigon, Besu, Anvil), so
/// this matches the stable substrings they share.
pub fn classify_rpc_error(code: i64, message: &str) -> BroadcastOutcome {
    let lower = message.to_ascii_lowercase();
    let has = |needle: &str| lower.contains(needle);

    if has("already known")
        || has("known transaction")
        || has("alreadyknown")
        || has("already imported")
        || has("already in the mempool")
        || has("already exists")
    {
        return BroadcastOutcome::AlreadyKnown;
    }
    if has("nonce too low") || has("nonce is too low") || has("oldnonce") || has("nonce too small")
    {
        return BroadcastOutcome::NonceTooLow;
    }
    if has("underpriced")
        || has("replacement fee too low")
        || has("fee too low")
        || has("less than block base fee")
        || has("max fee per gas less than")
        || has("feecaptoolow")
    {
        return BroadcastOutcome::Underpriced;
    }
    let detail: String = message.chars().take(200).collect();
    BroadcastOutcome::Rejected(format!("RPC error {code}: {detail}"))
}

/// [`L1Rpc`] over a `common::l1_rpc` provider. Chain identity uses the
/// configured-endpoint quorum; nonce reads and submissions go to the primary
/// only; receipts, fees and block numbers fail over.
#[derive(Clone)]
pub struct AlloyL1Rpc {
    provider: RootProvider<FailoverTransport>,
}

impl AlloyL1Rpc {
    pub fn new(provider: RootProvider<FailoverTransport>) -> Self {
        Self { provider }
    }

    /// Build from an already validated endpoint list
    /// (`common::l1_rpc::parse_rpc_endpoints`), self-hosted endpoints first.
    pub fn from_urls(urls: &[String]) -> Result<Self> {
        Ok(Self::new(common::l1_rpc::build_provider_from_urls(urls)?))
    }

    pub fn provider(&self) -> &RootProvider<FailoverTransport> {
        &self.provider
    }
}

#[async_trait]
impl L1Rpc for AlloyL1Rpc {
    async fn chain_id(&self) -> Result<u64> {
        self.provider
            .get_chain_id()
            .await
            .context("configured L1 RPC endpoints did not agree on a chain id")
    }

    async fn transaction_count(&self, address: Address, pending: bool) -> Result<u64> {
        let call = self.provider.get_transaction_count(address);
        let call = if pending {
            call.block_id(BlockId::pending())
        } else {
            call.block_id(BlockId::latest())
        };
        call.await
            .context("failed to read the signer account nonce")
    }

    async fn fee_estimate(&self) -> Result<FeeEstimate> {
        let estimate = self
            .provider
            .estimate_eip1559_fees(None)
            .await
            .context("failed to estimate EIP-1559 fees")?;
        Ok(FeeEstimate {
            max_fee_per_gas: estimate.max_fee_per_gas,
            max_priority_fee_per_gas: estimate.max_priority_fee_per_gas,
        })
    }

    async fn send_raw_transaction(&self, raw: &Bytes) -> BroadcastOutcome {
        let result: Result<B256, _> = self
            .provider
            .client()
            .request("eth_sendRawTransaction", (raw.clone(),))
            .await;
        match result {
            Ok(hash) => BroadcastOutcome::Accepted(hash),
            Err(RpcError::ErrorResp(payload)) => classify_rpc_error(payload.code, &payload.message),
            // Transport errors, timeouts, unparseable responses: the node may
            // have accepted the transaction before the failure.
            Err(_) => BroadcastOutcome::Uncertain(
                "transport error while broadcasting; outcome unknown".to_string(),
            ),
        }
    }

    async fn receipt(&self, tx_hash: B256) -> Result<Option<ReceiptSummary>> {
        let receipt = self
            .provider
            .get_transaction_receipt(tx_hash)
            .await
            .context("failed to read a transaction receipt")?;
        Ok(receipt.and_then(|receipt| {
            receipt.block_number().map(|block_number| ReceiptSummary {
                tx_hash,
                block_number,
                success: receipt.status(),
            })
        }))
    }

    async fn block_number(&self) -> Result<u64> {
        self.provider
            .get_block_number()
            .await
            .context("failed to read the L1 block number")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_client_messages() {
        assert_eq!(
            classify_rpc_error(-32000, "already known"),
            BroadcastOutcome::AlreadyKnown
        );
        assert_eq!(
            classify_rpc_error(-32010, "AlreadyKnown"),
            BroadcastOutcome::AlreadyKnown
        );
        assert_eq!(
            classify_rpc_error(-32000, "nonce too low: next nonce 5, tx nonce 4"),
            BroadcastOutcome::NonceTooLow
        );
        assert_eq!(
            classify_rpc_error(-32010, "OldNonce"),
            BroadcastOutcome::NonceTooLow
        );
        assert_eq!(
            classify_rpc_error(-32000, "replacement transaction underpriced"),
            BroadcastOutcome::Underpriced
        );
        assert_eq!(
            classify_rpc_error(-32000, "max fee per gas less than block base fee"),
            BroadcastOutcome::Underpriced
        );
        assert!(matches!(
            classify_rpc_error(-32000, "insufficient funds for gas * price + value"),
            BroadcastOutcome::Rejected(_)
        ));
        assert!(matches!(
            classify_rpc_error(-32602, "invalid chain id"),
            BroadcastOutcome::Rejected(_)
        ));
    }

    #[test]
    fn rejection_detail_is_bounded() {
        let long = "x".repeat(10_000);
        match classify_rpc_error(-1, &long) {
            BroadcastOutcome::Rejected(detail) => assert!(detail.len() < 300),
            other => panic!("unexpected outcome {other:?}"),
        }
    }
}
