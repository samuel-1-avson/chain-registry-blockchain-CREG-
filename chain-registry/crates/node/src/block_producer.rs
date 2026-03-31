// crates/node/src/block_producer.rs
// Produces new blocks on a fixed interval by draining the finalized-tx channel.

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use common::{Block, BlockHeader, Transaction, merkle_root};
use chrono::Utc;
use crate::{NodeState, finalized_tx, gossip::Gossip};

pub async fn run(
    state:  Arc<RwLock<NodeState>>,
    rx:     finalized_tx::FinalizedTxReceiver,
) {
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
                    &bh[..bh.len().min(12)], block.header.height, block.transactions.len()
                );
                {
                    let mut s = state.write().await;
                    s.publisher_index.apply_block(&block);
                }
                
                // Announce block via P2P Gossipsub
                let ann = crate::gossip::BlockAnnouncement {
                    height:     block.header.height,
                    block_hash: block.hash(),
                    proposer:   block.header.proposer_id.clone(),
                };
                let p2p_handle = state.read().await.p2p.clone();
                let _ = p2p_handle.sender.send(crate::p2p::P2PCommand::Broadcast {
                    topic: "creg/v1/blocks".into(),
                    data: serde_json::to_vec(&ann).unwrap_or_default(),
                }).await;
            }
            Err(e) => tracing::error!("Block production failed: {}", e),
        }
    }
}

async fn produce_block(
    state: Arc<RwLock<NodeState>>,
    txs:   Vec<Transaction>,
) -> anyhow::Result<Block> {
    let s = state.write().await;
    let tip_height = s.chain.tip_height()?;
    let prev_hash  = s.chain.tip_hash()?;
    let node_id    = s.config.node_id.clone();

    let header = BlockHeader {
        height:             tip_height + 1,
        prev_hash,
        merkle_root:        merkle_root(&txs),
        proposer_id:        node_id,
        timestamp:          Utc::now(),
        validator_set_hash: "dev".to_string(),
    };

    let block = Block { header, transactions: txs };
    s.chain.insert_block(&block)?;
    Ok(block)
}
