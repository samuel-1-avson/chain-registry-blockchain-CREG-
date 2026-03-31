// crates/node/src/sync.rs
// Chain synchronisation — brings a lagging node up to the network tip.
//
// On startup, and periodically during operation, this module:
//   1. Asks peers for their chain tip height.
//   2. If we are behind, fetches each missing block in order.
//   3. Validates each block's prev_hash linkage before inserting.
//   4. Applies each block to the publisher index.
//
// This is a simple linear sync. In a production network with thousands of
// blocks, a state-snapshot sync (download a snapshot + apply delta) would
// be more efficient, but for a registry where each block contains a handful
// of package verification transactions, linear sync is perfectly adequate.

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use crate::NodeState;

/// Sync interval — check for new blocks from peers every 10 seconds.
const SYNC_INTERVAL_SECS: u64 = 10;

pub async fn run(state: Arc<RwLock<NodeState>>) {
    // Initial sync at startup.
    if let Err(e) = sync_once(Arc::clone(&state)).await {
        tracing::warn!("Initial chain sync failed: {}", e);
    }

    let mut ticker = interval(Duration::from_secs(SYNC_INTERVAL_SECS));
    tracing::info!("Chain sync running (interval: {}s)", SYNC_INTERVAL_SECS);

    loop {
        ticker.tick().await;
        if let Err(e) = sync_once(Arc::clone(&state)).await {
            tracing::debug!("Chain sync tick failed: {}", e);
        }
    }
}

async fn sync_once(
    state: Arc<RwLock<NodeState>>,
) -> anyhow::Result<()> {
    let (our_height, peer_urls) = {
        let s = state.read().await;
        (s.chain.tip_height()?, s.config.peers.clone())
    };

    // Ask peers for their tip.
    let mut peer_height = 0;
    let client = reqwest::Client::new();
    
    for url in &peer_urls {
        let full_url = format!("{}/v1/chain/stats", url.trim_end_matches('/'));
        if let Ok(resp) = client.get(&full_url).send().await {
            #[derive(serde::Deserialize)] struct Stats { tip_height: u64 }
            if let Ok(stats) = resp.json::<Stats>().await {
                peer_height = peer_height.max(stats.tip_height);
            }
        }
    }

    if peer_height <= our_height {
        tracing::debug!("Chain is up to date (height {})", our_height);
        return Ok(());
    }

    tracing::info!(
        "Chain sync: our height={} peer height={} — fetching {} blocks",
        our_height, peer_height, peer_height - our_height
    );

    // Fetch each missing block in order and validate the chain linkage.
    let mut prev_hash = {
        let s = state.read().await;
        s.chain.tip_hash()?
    };

    for height in (our_height + 1)..=peer_height {
        let mut block = None;
        for url in &peer_urls {
            let full_url = format!("{}/v1/blocks/{}", url.trim_end_matches('/'), height);
            if let Ok(resp) = client.get(&full_url).send().await {
                if let Ok(b) = resp.json::<common::Block>().await {
                    block = Some(b);
                    break;
                }
            }
        }

        let block = match block {
            Some(b) => b,
            None    => {
                tracing::warn!("Could not fetch block {} from any peer", height);
                break;
            }
        };

        // Validate chain linkage.
        if block.header.prev_hash != prev_hash {
            tracing::error!(
                "Block {} has wrong prev_hash (expected {}, got {})",
                height, &prev_hash[..12], &block.header.prev_hash[..12]
            );
            break;
        }

        if block.header.height != height {
            tracing::error!(
                "Block claims height {} but we requested {}",
                block.header.height, height
            );
            break;
        }

        prev_hash = block.hash();

        // Insert and index the block.
        {
            let mut s = state.write().await;
            s.chain.insert_block(&block)?;
            s.publisher_index.apply_block(&block);
        }

        tracing::info!("Synced block {} ({})", height, &prev_hash[..12]);
    }

    Ok(())
}
