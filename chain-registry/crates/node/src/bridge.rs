// crates/node/src/bridge.rs
// Legacy research bridge. Production uses witnessed log checkpoints; this
// module exits unless explicitly enabled in a development environment.

use crate::NodeState;
use alloy::sol_types::SolCall;
use alloy::{
    network::EthereumWallet,
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use common::{PackageStatus, Transaction};
use sha2::Digest;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep, Duration};

// ── Contract Binding ──────────────────────────────────────────────────────────
sol!(
    #[sol(rpc)]
    interface IRegistry {
        function latestStateRoot() external view returns (bytes32 _0);

        // NOTE: this ABI is retained only for the development research path.
        // The batch function is retired in production because its legacy proof
        // does not establish a state transition or data availability.
        function submitRollupBatch(
            bytes32 prevRoot,
            bytes32 nextRoot,
            uint256 txCount,
            bytes32 dataRoot,
            uint256[8] calldata proof,
            uint256[] calldata publicInputs
        ) external;
    }

    #[sol(rpc)]
    interface IGovernance {
        function proposalCount() external view returns (uint256 _0);

        function threshold() external view returns (uint256 _0);

        function submit(
            address target,
            bytes calldata callData,
            string calldata description
        ) external returns (uint256 id);

        function vote(uint256 id, bool approve) external;
    }

    #[sol(rpc)]
    interface IValidatorRewards {
        function distributeDiscoveryBounty(
            bytes32 witnessDigest,
            string calldata packageCanonical,
            address prover,
            uint256 baseBounty
        ) external;
    }
);

/// Trust-model label attached to the development-only legacy path.
const PROOF_MODE: &str = "legacy-checkpoint-research";

/// Legacy checkpointing can run only in an explicitly marked development
/// environment. The old batch circuit does not prove a state transition or
/// data availability, so it is not an acceptable security boundary for alpha,
/// testnet, or production deployments.
fn legacy_checkpoint_research_enabled() -> bool {
    let enabled = std::env::var("CREG_ENABLE_LEGACY_CHECKPOINT_RESEARCH")
        .map(|v| v.trim().eq_ignore_ascii_case("true") || v.trim() == "1")
        .unwrap_or(false);
    let development = std::env::var("CREG_DEVELOPMENT")
        .map(|v| v.trim().eq_ignore_ascii_case("true") || v.trim() == "1")
        .unwrap_or(false);
    enabled && development
}

/// Whether the bridge should immediately vote `approve` on its own batch
/// proposal. Defaults to true (required for liveness while governance runs
/// threshold-1). Set CREG_BRIDGE_SELF_APPROVE=false once an independent
/// second signer approves batches, so the bridge key alone can no longer
/// both propose and execute.
fn bridge_self_approve_enabled() -> bool {
    std::env::var("CREG_BRIDGE_SELF_APPROVE")
        .map(|v| v.trim().eq_ignore_ascii_case("true") || v.trim() == "1")
        .unwrap_or(true)
}

/// Best-effort fetch of the L1 block number currently tagged `finalized`.
async fn fetch_finalized_l1_block<T, P>(provider: &P) -> Option<u64>
where
    T: alloy::transports::Transport + Clone,
    P: Provider<T>,
{
    let result: Result<serde_json::Value, _> = provider
        .client()
        .request("eth_getBlockByNumber", ("finalized", false))
        .await;
    match result {
        Ok(block) => block
            .get("number")
            .and_then(|n| n.as_str())
            .and_then(|hex_str| u64::from_str_radix(hex_str.trim_start_matches("0x"), 16).ok()),
        Err(_) => None,
    }
}

pub async fn run(state: Arc<RwLock<NodeState>>) {
    if !legacy_checkpoint_research_enabled() {
        let mut s = state.write().await;
        s.bridge_status.bridge_sync_status =
            "Legacy checkpointing disabled by Phase 0 safety freeze".into();
        s.bridge_status.proof_mode = "disabled".into();
        tracing::warn!(
            "Legacy checkpointing is disabled. It may run only with both \
             CREG_DEVELOPMENT=true and CREG_ENABLE_LEGACY_CHECKPOINT_RESEARCH=true."
        );
        return;
    }

    let mut ticker = interval(Duration::from_secs(10));
    let mut last_processed_height = 0;

    tracing::info!("On-chain bridge started");

    {
        let (bridge_key, is_testnet) = {
            let s = state.read().await;
            (s.config.bridge_privkey.clone(), s.config.is_testnet)
        };
        if let Some(ref key) = bridge_key {
            if let Ok(secrets) = chain_registry_secrets::SecretsProvider::from_env() {
                secrets.warn_hot_key_if_env(
                    "bridge",
                    chain_registry_secrets::HotKeyRole::Bridge,
                    key,
                    is_testnet,
                );
            }
        }
    }

    // ── Wait for RPC to be available ──────────────────────────────────────────
    let mut rpc_ready = false;
    while !rpc_ready {
        let rpc_url = {
            let s = state.read().await;
            s.config.eth_rpc_url.clone()
        };

        let provider = match crate::rpc_failover::build_l1_provider(&rpc_url) {
            Ok(provider) => provider,
            Err(error) => {
                tracing::error!("Invalid Ethereum RPC configuration: {error} — retrying in 5s");
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        match provider.get_chain_id().await {
            Ok(id) => {
                tracing::info!("Connected to Ethereum RPC (Chain ID: {})", id);
                rpc_ready = true;
            }
            Err(_) => {
                tracing::warn!("Waiting for a configured Ethereum RPC endpoint...");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }

    loop {
        ticker.tick().await;

        if let Err(e) = tick(Arc::clone(&state), &mut last_processed_height).await {
            // Check if it's a connection error to reduce noise
            let err_str = e.to_string();
            if err_str.contains("error sending request") || err_str.contains("connection refused") {
                tracing::warn!(
                    "Bridge RPC connection issue: {}. Retrying in 10s...",
                    err_str
                );
            } else {
                tracing::error!("Bridge tick error: {}", e);
            }
        }
    }
}

async fn tick(state: Arc<RwLock<NodeState>>, last_height: &mut u64) -> anyhow::Result<()> {
    let (rpc_url, registry_addr, governance_addr, priv_key_opt, current_tip, data_dir) = {
        let s = state.read().await;
        (
            s.config.eth_rpc_url.clone(),
            s.config.registry_addr.clone(),
            s.config.governance_addr.clone(),
            s.config.bridge_privkey.clone(),
            s.chain.tip_height()?,
            s.config.data_dir.clone(),
        )
    };

    if current_tip <= *last_height {
        return Ok(());
    }

    let priv_key = match priv_key_opt {
        Some(k) => k,
        None => {
            let mut s = state.write().await;
            s.bridge_status.bridge_sync_status =
                "Bridge disabled: CREG_BRIDGE_KEY not configured".into();
            return Ok(());
        }
    };

    if governance_addr == "0x0000000000000000000000000000000000000000" {
        let mut s = state.write().await;
        s.bridge_status.bridge_sync_status =
            "Bridge disabled: CREG_GOVERNANCE_ADDR not configured".into();
        return Ok(());
    }

    // ── Setup Ethereum Provider ───────────────────────────────────────────────
    let signer: PrivateKeySigner = priv_key.parse()?;
    let wallet = EthereumWallet::from(signer);
    let base_provider = crate::rpc_failover::build_l1_provider(&rpc_url)?;
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_provider(base_provider);

    let contract_addr = registry_addr.parse()?;
    let contract = IRegistry::new(contract_addr, &provider);
    let governance_contract = IGovernance::new(governance_addr.parse()?, &provider);

    // ── Rollup Batching ──────────────────────────────────────────────────────
    let mut batch_transactions = Vec::new();
    let prev_root = contract.latestStateRoot().call().await?._0;
    let mut new_last_height = *last_height;

    for h in (*last_height + 1)..=current_tip {
        let block = {
            let s = state.read().await;
            s.chain.get_block_by_height(h)?
        };

        if let Some(b) = block {
            for tx in &b.transactions {
                if let Some(record) = tx.published_record() {
                    if record.status == PackageStatus::Verified {
                        batch_transactions.push(record.clone());
                    }
                }
            }
        }
        new_last_height = h;
    }

    if batch_transactions.is_empty() {
        *last_height = new_last_height;
        return Ok(());
    }

    tracing::info!(
        "Preparing L2 Rollup Batch with {} transactions",
        batch_transactions.len()
    );

    // Calculate Data Root using a binary Merkle tree over the batch.
    // Each leaf is SHA-256(canonical || content_hash). If the leaf count is odd,
    // the last leaf is duplicated before pairing.
    let leaves: Vec<[u8; 32]> = batch_transactions
        .iter()
        .map(|tx| {
            let mut h = sha2::Sha256::new();
            h.update(tx.id.canonical().as_bytes());
            h.update(tx.content_hash.as_bytes());
            h.finalize().into()
        })
        .collect();

    let data_root = merkle_root(&leaves);

    // Calculate Next State Root = SHA-256(prev_root || data_root)
    let mut state_hasher = sha2::Sha256::new();
    state_hasher.update(prev_root);
    state_hasher.update(data_root);
    let next_root: [u8; 32] = state_hasher.finalize().into();

    // Legacy Groth16 checkpointing circuits are retired (ADR-018 / VNA-22).
    // Refuse rollup batch submission before generating any unverified proof.
    anyhow::bail!("legacy ZK rollup batching is retired (ADR-018 / VNA-22)");
}

/// Compute a binary Merkle root over the given leaf hashes.
///
/// - If the list is empty, returns the all-zeros hash.
/// - If the leaf count is odd, the last leaf is duplicated before pairing.
/// - Internal nodes are `SHA-256(left || right)`.
fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    let mut current: Vec<[u8; 32]> = leaves.to_vec();
    while current.len() > 1 {
        if current.len() % 2 != 0 {
            // Duplicate last leaf for odd length (standard Merkle padding).
            if let Some(last) = current.last().copied() {
                current.push(last);
            }
        }
        let mut next = Vec::with_capacity(current.len() / 2);
        for pair in current.chunks(2) {
            let mut h = sha2::Sha256::new();
            h.update(pair[0]);
            h.update(pair[1]);
            next.push(h.finalize().into());
        }
        current = next;
    }
    current[0]
}

#[cfg(test)]
mod tests {
    use super::IValidatorRewards;
    use alloy::primitives::{keccak256, Address, FixedBytes, U256};
    use alloy::sol_types::SolCall;

    #[test]
    fn discovery_bounty_abi_selector_and_roundtrip() {
        let expected_sig = keccak256("distributeDiscoveryBounty(bytes32,string,address,uint256)");
        assert_eq!(
            IValidatorRewards::distributeDiscoveryBountyCall::SELECTOR,
            &expected_sig.0[..4]
        );

        let digest = FixedBytes::<32>::from([0xAB; 32]);
        let prover = Address::from([0x11; 20]);
        let call = IValidatorRewards::distributeDiscoveryBountyCall {
            witnessDigest: digest,
            packageCanonical: "npm:adv-ci-timebomb-canary@1.0.0".to_string(),
            prover,
            baseBounty: U256::from(1_000_000u64),
        };
        let encoded = call.abi_encode();
        assert_eq!(&encoded[..4], &expected_sig.0[..4]);

        let decoded = IValidatorRewards::distributeDiscoveryBountyCall::abi_decode(&encoded, true)
            .expect("ABI decode distributeDiscoveryBounty");
        assert_eq!(decoded.witnessDigest, digest);
        assert_eq!(decoded.packageCanonical, "npm:adv-ci-timebomb-canary@1.0.0");
        assert_eq!(decoded.prover, prover);
        assert_eq!(decoded.baseBounty, U256::from(1_000_000u64));
    }
}
