// crates/node/src/bridge.rs
// Monitors PBFT consensus and finalizes records on the Ethereum Registry contract.

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

    #[sol(rpc)]
    interface IGovernance {
        function proposalCount() external view returns (uint256 _0);

        function submit(
            address target,
            bytes calldata callData,
            string calldata description
        ) external returns (uint256 id);

        function vote(uint256 id, bool approve) external;
    }
);

pub async fn run(state: Arc<RwLock<NodeState>>) {
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

        let parsed_url = match rpc_url.parse() {
            Ok(url) => url,
            Err(e) => {
                tracing::error!(
                    "Invalid CREG_ETH_RPC URL {:?}: {} — retrying in 5s",
                    rpc_url,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        match ProviderBuilder::new()
            .on_http(parsed_url)
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
    let (rpc_url, registry_addr, governance_addr, priv_key_opt, current_tip) = {
        let s = state.read().await;
        (
            s.config.eth_rpc_url.clone(),
            s.config.registry_addr.clone(),
            s.config.governance_addr.clone(),
            s.config.bridge_privkey.clone(),
            s.chain.tip_height()?,
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
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(rpc_url.parse()?);

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
                if let Transaction::Publish(record) = tx {
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

    // Generate a Groth16 ZK proof for the batch state transition using the
    // dedicated BatchStateTransitionCircuit.
    //
    // Public inputs (6 Fr elements encoded as U256):
    //   [prev_root_lo, prev_root_hi, data_root_lo, data_root_hi,
    //    next_root_lo, next_root_hi]
    //
    // The circuit proves: tx_count ≥ 1 (non-empty batch). The on-chain
    // ZKVerifier checks the proof against these public inputs, and
    // submitRollupBatch checks prevRoot == latestStateRoot().
    let (proof, public_inputs) = {
        use zk_validator::{BatchInputs, BatchStateTransitionValidator};

        let batch_inputs = BatchInputs::new(
            prev_root.into(),
            data_root,
            next_root,
            batch_transactions.len() as u64,
        );

        // Proof generation is CPU-bound; run in a blocking worker thread so
        // we don't block the async runtime.
        let batch_inputs_clone = batch_inputs.clone();
        let proof_result = tokio::task::spawn_blocking(move || {
            let validator = BatchStateTransitionValidator::new()?;
            validator.generate_proof(&batch_inputs_clone)
        })
        .await
        .map_err(|e| anyhow::anyhow!("proof task panicked: {}", e))?;

        match proof_result {
            Ok(p) => {
                // Convert proof chunks → [U256; 8]
                let chunks = BatchStateTransitionValidator::proof_to_chunks(&p)
                    .map_err(|e| anyhow::anyhow!("proof serialization failed: {}", e))?;
                let mut arr = [alloy::primitives::U256::from(0u64); 8];
                for (i, chunk) in chunks.into_iter().enumerate() {
                    arr[i] = alloy::primitives::U256::from_be_bytes(chunk);
                }
                // Convert public-input byte chunks → Vec<U256>
                let pi: Vec<alloy::primitives::U256> = batch_inputs
                    .public_inputs_bytes()
                    .into_iter()
                    .map(alloy::primitives::U256::from_be_bytes)
                    .collect();
                (arr, pi)
            }
            Err(e) => {
                // Fail closed — never submit without a valid proof.
                tracing::error!(
                    "Batch ZK proof generation failed \
                     (prev_root={}, next_root={}, tx_count={}): {}. \
                     Refusing to submit batch.",
                    hex::encode(prev_root),
                    hex::encode(next_root),
                    batch_transactions.len(),
                    e
                );
                anyhow::bail!(
                    "refusing to submit rollup batch without a valid ZK proof: {}",
                    e
                );
            }
        }
    };

    let proposal_id = governance_contract.proposalCount().call().await?._0;
    let call_data = IRegistry::submitRollupBatchCall {
        prevRoot: prev_root.into(),
        nextRoot: next_root.into(),
        txCount: alloy::primitives::U256::from(batch_transactions.len()),
        dataRoot: data_root.into(),
        proof,
        publicInputs: public_inputs,
    }
    .abi_encode();

    let submit_result: anyhow::Result<()> = async {
        governance_contract
            .submit(
                contract_addr,
                call_data.into(),
                format!("Submit rollup batch {}", proposal_id),
            )
            .send()
            .await?
            .watch()
            .await?;

        governance_contract
            .vote(proposal_id, true)
            .send()
            .await?
            .watch()
            .await?;

        Ok(())
    }
    .await;

    match submit_result {
        Ok(()) => {
            tracing::info!(
                "Successfully settled Rollup Batch on L1 via governance proposal {}. New State Root: 0x{}",
                proposal_id,
                hex::encode(next_root)
            );
            let eth_block = provider.get_block_number().await.unwrap_or(0);
            *last_height = new_last_height;
            let mut s = state.write().await;
            s.bridge_status.bridge_sync_status = "L2 Scaled".into();
            s.bridge_status.last_finalized_eth_block = eth_block;
            s.bridge_status.current_state_root = format!("0x{}", hex::encode(next_root));
        }
        Err(e) => {
            tracing::error!("Failed to submit Rollup Batch to L1 via governance: {}", e);
            let mut s = state.write().await;
            s.bridge_status.bridge_sync_status = format!("Rollup Error: {}", e);
            return Err(e);
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
