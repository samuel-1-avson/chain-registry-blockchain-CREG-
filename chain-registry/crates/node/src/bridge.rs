// crates/node/src/bridge.rs
// Monitors PBFT consensus and finalizes records on the Ethereum Registry contract.

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, sleep, Duration};
use alloy::{
    network::EthereumWallet,
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use common::{Transaction, PackageStatus};
use sha2::Digest;
use crate::NodeState;

// ── Contract Binding ──────────────────────────────────────────────────────────
sol!(
    #[sol(rpc)]
    interface IRegistry {
        function latestStateRoot() external view returns (bytes32 _0);
        
        function finalizePackage(
            string calldata canonical,
            bytes[] calldata validatorSignatures
        ) external;

        function submitRollupBatch(
            bytes32 prevRoot,
            bytes32 nextRoot,
            uint256 txCount,
            bytes32 dataRoot,
            uint256[8] calldata proof,
            uint256[] calldata publicInputs
        ) external;
    }
);

pub async fn run(state: Arc<RwLock<NodeState>>) {
    let mut ticker = interval(Duration::from_secs(10));
    let mut last_processed_height = 0;

    tracing::info!("On-chain bridge started");

    // ── Wait for RPC to be available ──────────────────────────────────────────
    let mut rpc_ready = false;
    while !rpc_ready {
        let rpc_url = {
            let s = state.read().await;
            s.config.eth_rpc_url.clone()
        };

        match ProviderBuilder::new().on_http(rpc_url.parse().unwrap()).get_chain_id().await {
            Ok(id) => {
                tracing::info!("Connected to Ethereum RPC (Chain ID: {})", id);
                rpc_ready = true;
            }
            Err(_) => {
                tracing::warn!("Waiting for Ethereum RPC at {}...", rpc_url);
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
                tracing::warn!("Bridge RPC connection issue: {}. Retrying in 10s...", err_str);
            } else {
                tracing::error!("Bridge tick error: {}", e);
            }
        }
    }
}

async fn tick(
    state: Arc<RwLock<NodeState>>,
    last_height: &mut u64,
) -> anyhow::Result<()> {
    let (rpc_url, registry_addr, priv_key_opt, current_tip) = {
        let s = state.read().await;
        (
            s.config.eth_rpc_url.clone(),
            s.config.registry_addr.clone(),
            s.config.validator_privkey.clone(),
            s.chain.tip_height()?,
        )
    };

    if current_tip <= *last_height {
        return Ok(());
    }

    let priv_key = match priv_key_opt {
        Some(k) => k,
        None => return Ok(()), // Only validators with keys can bridge (or specifically authorized bridge nodes)
    };

    // ── Setup Ethereum Provider ───────────────────────────────────────────────
    let signer: PrivateKeySigner = priv_key.parse()?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(rpc_url.parse()?);

    let contract_addr = registry_addr.parse()?;
    let contract = IRegistry::new(contract_addr, &provider);

    // ── Rollup Batching ──────────────────────────────────────────────────────
    let mut batch_transactions = Vec::new();
    let prev_root = contract.latestStateRoot().call().await?._0;

    for h in (*last_height + 1)..=current_tip {
        let block = {
            let s = state.read().await;
            s.chain.get_block_by_height(h)?
        };

        if let Some(b) = block {
            for tx in &b.transactions {
                if let Transaction::Publish(record) = tx {
                    if record.status == PackageStatus::Verified {
                        batch_transactions.push(record.clone());
                    }
                }
            }
        }
        *last_height = h;
    }

    if !batch_transactions.is_empty() {
        tracing::info!("Preparing L2 Rollup Batch with {} transactions", batch_transactions.len());

        // Calculate Data Root (Merkle-style hash of the batch)
        let mut data_hasher = sha2::Sha256::new();
        for tx in &batch_transactions {
            data_hasher.update(tx.id.canonical().as_bytes());
            data_hasher.update(tx.content_hash.as_bytes());
        }
        let data_root: [u8; 32] = data_hasher.finalize().into();
        
        // Calculate Next State Root
        let mut state_hasher = sha2::Sha256::new();
        state_hasher.update(prev_root);
        state_hasher.update(data_root);
        let next_root: [u8; 32] = state_hasher.finalize().into();

        // Submit Rollup Batch to L1
        // (Mocking ZK proof and public inputs for this demo phase)
        let proof = [alloy::primitives::U256::from(0); 8];
        let mut public_inputs = Vec::new();
        // ZKVerifier expects at least one public input (vk.ic.length - 1)
        public_inputs.push(alloy::primitives::U256::from(0));

        let call = contract.submitRollupBatch(
            prev_root.into(),
            next_root.into(),
            alloy::primitives::U256::from(batch_transactions.len()),
            data_root.into(),
            proof,
            public_inputs
        );

        if let Err(e) = call.send().await {
            tracing::error!("Failed to submit Rollup Batch to L1: {}", e);
            let mut s = state.write().await;
            s.bridge_status.bridge_sync_status = format!("Rollup Error: {}", e);
        } else {
            tracing::info!("Successfully settled Rollup Batch on L1. New State Root: 0x{}", hex::encode(next_root));
            let eth_block = provider.get_block_number().await.unwrap_or(0);
            let mut s = state.write().await;
            s.bridge_status.bridge_sync_status = "L2 Scaled".into();
            s.bridge_status.last_finalized_eth_block = eth_block;
            s.bridge_status.current_state_root = format!("0x{}", hex::encode(next_root));
        }
    }

    Ok(())
}
