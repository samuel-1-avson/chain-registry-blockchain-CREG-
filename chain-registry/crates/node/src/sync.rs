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

use crate::NodeState;
use common::{Transaction, ValidatorSet, ValidatorVote};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

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

async fn sync_once(state: Arc<RwLock<NodeState>>) -> anyhow::Result<()> {
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
            #[derive(serde::Deserialize)]
            struct Stats {
                tip_height: u64,
            }
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
        our_height,
        peer_height,
        peer_height - our_height
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
            None => {
                tracing::warn!("Could not fetch block {} from any peer", height);
                break;
            }
        };

        // Validate chain linkage.
        if block.header.prev_hash != prev_hash {
            tracing::error!(
                "Block {} has wrong prev_hash (expected {}, got {})",
                height,
                &prev_hash[..12],
                &block.header.prev_hash[..12]
            );
            break;
        }

        if block.header.height != height {
            tracing::error!(
                "Block claims height {} but we requested {}",
                block.header.height,
                height
            );
            break;
        }

        // Verify that every Publish transaction carries a PBFT quorum of valid
        // signatures against the current validator set. Without this check a
        // malicious peer could serve a syntactically-valid block whose payload
        // never reached consensus. (ISSUE-018)
        let validator_set = {
            let s = state.read().await;
            s.validator_set.clone()
        };
        if let Err(e) = verify_block_signatures(&block, &validator_set) {
            tracing::error!(
                "Block {} failed signature verification: {} — halting sync",
                height,
                e
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

/// Verify PBFT consensus signatures on every Publish transaction in a block.
///
/// Each `ChainRecord` in the block must carry at least `⌊2n/3⌋ + 1` valid
/// Ed25519 signatures from validators currently in the active set, where each
/// signature is over the canonical on-chain message `"<canonical>-<content_hash>"`
/// (the format produced by `validator_pipeline::sign`).
///
/// Notes and limitations:
///   * Uses the *current* validator set. Historical validator-set tracking is
///     a follow-up enhancement (ISSUE-050 in the roadmap). A node syncing
///     across a validator-set transition may legitimately see signatures from
///     validators not in the current set — those are simply ignored.
///   * Non-Publish transactions (Revoke, Slash, ValidatorJoin/Leave,
///     RotatePublisherKey) are intentionally not verified here; they are
///     governance-originated and validated at the state-transition layer.
///   * Single-validator deployments still require the one validator's signature.
fn verify_block_signatures(
    block: &common::Block,
    validator_set: &ValidatorSet,
) -> anyhow::Result<()> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    // Genesis has no signatures by design.
    if block.header.height == 0 {
        return Ok(());
    }

    let n = validator_set.validators.len();
    if n == 0 {
        anyhow::bail!("cannot verify block: local validator set is empty");
    }
    let quorum = (2 * n / 3) + 1;

    // Build a pubkey → validator_id lookup once so per-signature verification
    // is O(1) instead of O(n).
    let known: std::collections::HashMap<String, &common::Validator> = validator_set
        .validators
        .iter()
        .map(|v| (v.pubkey.to_ascii_lowercase(), v))
        .collect();

    for (tx_idx, tx) in block.transactions.iter().enumerate() {
        let record = match tx {
            Transaction::Publish(r) => r,
            _ => continue,
        };
        let canonical = record.id.canonical();
        let message = format!("{}-{}", canonical, record.content_hash);
        let msg_bytes = message.as_bytes();

        let mut approvals = 0usize;
        let mut seen = HashSet::new();

        for sig in &record.validator_signatures {
            if !matches!(sig.vote, ValidatorVote::Approve) {
                continue;
            }

            let pubkey_key = sig.validator_pubkey.to_ascii_lowercase();
            if !known.contains_key(&pubkey_key) {
                tracing::debug!(
                    "sync: tx {} carries signature from unknown validator pubkey {} — ignored",
                    tx_idx,
                    pubkey_key
                );
                continue;
            }
            if !seen.insert(pubkey_key.clone()) {
                // Duplicate signature from the same validator — count only once.
                continue;
            }

            let pubkey_bytes = match hex::decode(&sig.validator_pubkey) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let vk = match VerifyingKey::try_from(pubkey_bytes.as_slice()) {
                Ok(k) => k,
                Err(_) => continue,
            };
            let sig_bytes = match hex::decode(&sig.signature) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let ed_sig = match Signature::try_from(sig_bytes.as_slice()) {
                Ok(s) => s,
                Err(_) => continue,
            };

            if vk.verify(msg_bytes, &ed_sig).is_ok() {
                approvals += 1;
            }
        }

        if approvals < quorum {
            anyhow::bail!(
                "tx {} ({}): only {} valid signatures, need {} ({}/{} validators)",
                tx_idx,
                canonical,
                approvals,
                quorum,
                approvals,
                n
            );
        }
    }

    Ok(())
}
