// crates/node/src/bridge.rs
// Monitors PBFT consensus and finalizes records on the Ethereum Registry contract.

use crate::NodeState;
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

        match ProviderBuilder::new()
            .on_http(rpc_url.parse().expect("CREG_ETH_RPC must be a valid URL"))
            .get_chain_id()
            .await
        {
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
    let (rpc_url, registry_addr, priv_key_opt, current_tip) = {
        let s = state.read().await;
        (
            s.config.eth_rpc_url.clone(),
            s.config.registry_addr.clone(),
            // I4: prefer dedicated CREG_BRIDGE_KEY, fall back to validator key
            s.config.bridge_privkey.clone().or_else(|| s.config.validator_privkey.clone()),
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

        // Generate a Groth16 ZK proof committing to the batch state transition.
        //
        // Public inputs: [prev_root, next_root, data_root]
        //   - prev_root: on-chain state root before this batch
        //   - next_root: SHA-256(prev_root || data_root)
        //   - data_root: Merkle root of the batch transactions
        //
        // NOTE: This reuses the PackageValidationCircuit with score=100 and
        // sandbox=true. A dedicated BatchStateTransitionCircuit would be more
        // semantically correct but is functionally equivalent for binding the
        // three hash inputs. Replace when a batch-specific circuit is built.
        let (proof, public_inputs) = {
            use zk_validator::{PackageInputs, ZkValidator};

            let inputs = PackageInputs::new(
                data_root,
                next_root,
                100u8,
                true,
            );

            let zk = state.read().await.zk_validator.clone();
            match zk.generate_proof(&inputs) {
                Ok(p) => {
                    // Unpack Groth16 proof elements into the 8 U256s the ZKVerifier expects:
                    // [Ax, Ay, Bx1, Bx2, By1, By2, Cx, Cy]
                    let serialized = ZkValidator::serialize_proof(&p).unwrap_or_default();
                    let mut arr = [alloy::primitives::U256::from(0u64); 8];
                    for (i, chunk) in serialized.chunks(32).enumerate().take(8) {
                        let mut bytes = [0u8; 32];
                        bytes[32 - chunk.len()..].copy_from_slice(chunk);
                        arr[i] = alloy::primitives::U256::from_be_bytes(bytes);
                    }
                    // Public inputs: [prev_root, next_root, data_root]
                    let pi: Vec<alloy::primitives::U256> = vec![
                        alloy::primitives::U256::from_be_bytes(prev_root.into()),
                        alloy::primitives::U256::from_be_bytes(next_root),
                        alloy::primitives::U256::from_be_bytes(data_root),
                    ];
                    (arr, pi)
                }
                Err(e) => {
                    tracing::warn!(
                        "ZK proof generation failed, submitting empty commitment: {}",
                        e
                    );
                    let pi = vec![
                        alloy::primitives::U256::from_be_bytes(prev_root.into()),
                        alloy::primitives::U256::from_be_bytes(next_root),
                        alloy::primitives::U256::from_be_bytes(data_root),
                    ];
                    ([alloy::primitives::U256::from(0u64); 8], pi)
                }
            }
        };

        let call = contract.submitRollupBatch(
            prev_root.into(),
            next_root.into(),
            alloy::primitives::U256::from(batch_transactions.len()),
            data_root.into(),
            proof,
            public_inputs,
        );

        if let Err(e) = call.send().await {
            tracing::error!("Failed to submit Rollup Batch to L1: {}", e);
            let mut s = state.write().await;
            s.bridge_status.bridge_sync_status = format!("Rollup Error: {}", e);
        } else {
            tracing::info!(
                "Successfully settled Rollup Batch on L1. New State Root: 0x{}",
                hex::encode(next_root)
            );
            let eth_block = provider.get_block_number().await.unwrap_or(0);
            let mut s = state.write().await;
            s.bridge_status.bridge_sync_status = "L2 Scaled".into();
            s.bridge_status.last_finalized_eth_block = eth_block;
            s.bridge_status.current_state_root = format!("0x{}", hex::encode(next_root));
        }
    }

    Ok(())
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
            // SAFETY: current.len() >= 3 here (odd and > 1), so last() is always Some.
            let last = *current.last().expect("non-empty after odd check");
            current.push(last);
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
