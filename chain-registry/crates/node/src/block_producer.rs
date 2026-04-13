// crates/node/src/block_producer.rs
// Produces new blocks on a fixed interval by draining the finalized-tx channel.

use crate::{finalized_tx, gossip::Gossip, NodeState};
use chrono::Utc;
use common::{merkle_root, Block, BlockHeader, Transaction};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

pub async fn run(state: Arc<RwLock<NodeState>>, rx: finalized_tx::FinalizedTxReceiver) {
    let block_interval = {
        let s = state.read().await;
        s.config.block_interval_secs
    };

    let mut ticker = interval(Duration::from_secs(block_interval));
    tracing::info!("Block producer started (interval: {}s)", block_interval);

    loop {
        ticker.tick().await;

        // Drain everything the validator pipeline has finalised since last tick.
        let txs: Vec<Transaction> = finalized_tx::drain(&rx).await;
        if txs.is_empty() {
            tracing::debug!("Block producer: no new transactions");
            continue;
        }

        match produce_block(Arc::clone(&state), txs).await {
            Ok(block) => {
                let bh = block.hash();
                tracing::info!(
                    "Block {} produced at height {} ({} tx)",
                    &bh[..bh.len().min(12)],
                    block.header.height,
                    block.transactions.len()
                );
                {
                    let mut s = state.write().await;
                    s.publisher_index.apply_block(&block);
                }

                // Announce block via P2P Gossipsub
                let ann = crate::gossip::BlockAnnouncement {
                    height: block.header.height,
                    block_hash: block.hash(),
                    proposer: block.header.proposer_id.clone(),
                };
                let p2p_handle = state.read().await.p2p.clone();
                let _ = p2p_handle
                    .sender
                    .send(crate::p2p::P2PCommand::Broadcast {
                        topic: "creg/v1/blocks".into(),
                        data: serde_json::to_vec(&ann).unwrap_or_default(),
                    })
                    .await;
            }
            Err(e) => tracing::error!("Block production failed: {}", e),
        }
    }
}

async fn produce_block(
    state: Arc<RwLock<NodeState>>,
    txs: Vec<Transaction>,
) -> anyhow::Result<Block> {
    // ── Read-only snapshot of state needed for VRF selection ────────────────
    let (tip_height, prev_hash, node_id, privkey, our_pubkey, p2p, validator_set_hash) = {
        let s = state.read().await;
        let tip_height = s.chain.tip_height()?;
        let prev_hash = s.chain.tip_hash()?;
        let node_id = s.config.node_id.clone();
        let privkey = s.config.validator_privkey.clone();
        let our_pubkey = s
            .validator_set
            .validators
            .iter()
            .find(|v| v.id == node_id)
            .map(|v| v.pubkey.clone());
        let p2p = s.p2p.clone();

        // Compute a deterministic hash of the validator set so light clients
        // and bridge code can detect membership changes between blocks.
        // Input: sorted validator IDs concatenated with NUL separators.
        let mut sorted_ids: Vec<&str> = s
            .validator_set
            .validators
            .iter()
            .map(|v| v.id.as_str())
            .collect();
        sorted_ids.sort_unstable();
        let mut hasher = Sha256::new();
        for id in &sorted_ids {
            hasher.update(id.as_bytes());
            hasher.update(b"\0");
        }
        let validator_set_hash = hex::encode(hasher.finalize());

        (tip_height, prev_hash, node_id, privkey, our_pubkey, p2p, validator_set_hash)
    };

    let epoch_seed = prev_hash.clone();

    // Build active set, injecting any cached VRF proofs from peers.
    let mut active: Vec<consensus::vrf::VrfValidator> = {
        let s = state.read().await;
        s.validator_set
            .validators
            .iter()
            .filter(|v| v.status == "online" || v.status == "self")
            .map(|v| {
                let (vrf_output, vrf_proof) = s
                    .vrf_proofs
                    .get(&v.id)
                    .cloned()
                    .map(|(o, p)| (Some(o), Some(p)))
                    .unwrap_or((None, None));
                consensus::vrf::VrfValidator {
                    id: v.id.clone(),
                    pubkey: v.pubkey.clone(),
                    vrf_output,
                    vrf_proof,
                }
            })
            .collect()
    };

    let (vrf_output, vrf_proof) = if !active.is_empty() {
        if let Some(ref privkey) = privkey {
            let (out, prf) = consensus::vrf::prove(epoch_seed.as_bytes(), privkey)?;
            // Inject our own proof into the active set.
            for v in &mut active {
                if v.id == node_id {
                    v.vrf_output = Some(out.clone());
                    v.vrf_proof = Some(prf.clone());
                }
            }

            // Broadcast our VRF proof so peers can include it in their selection.
            let gossip_msg = common::GossipMessage::VrfProof {
                validator_id: node_id.clone(),
                pubkey: our_pubkey.unwrap_or_default(),
                epoch_seed: epoch_seed.clone(),
                output: out.clone(),
                proof: prf.clone(),
            };
            let _ = p2p
                .sender
                .send(crate::p2p::P2PCommand::Broadcast {
                    topic: "creg/v1/vrf-proofs".into(),
                    data: serde_json::to_vec(&gossip_msg).unwrap_or_default(),
                })
                .await;

            let selected_proposer = consensus::vrf::select_proposer(&active, &epoch_seed)
                .ok_or_else(|| anyhow::anyhow!("No active validators to select proposer"))?;
            if node_id != selected_proposer {
                anyhow::bail!(
                    "Node {} is not the selected proposer for this epoch (expected {})",
                    node_id,
                    selected_proposer
                );
            }
            (Some(out), Some(prf))
        } else {
            (None, None)
        }
    } else {
        // Dev/test fallback when no validator set is configured.
        (None, None)
    };

    // ── Write the new block ────────────────────────────────────────────────
    let mut s = state.write().await;
    let header = BlockHeader {
        height: tip_height + 1,
        prev_hash,
        merkle_root: merkle_root(&txs),
        proposer_id: node_id,
        timestamp: Utc::now(),
        validator_set_hash,
        vrf_output,
        vrf_proof,
    };

    let block = Block {
        header,
        transactions: txs,
    };
    s.chain.insert_block(&block)?;
    // Proofs are epoch-specific (seed = prev_hash); clear cache for next round.
    s.vrf_proofs.clear();
    Ok(block)
}
