// crates/node/src/gossip.rs
// Peer-to-peer gossip layer.
//
// Responsibilities:
//   1. When this node produces or receives a PBFT vote, forward it to all peers.
//   2. When this node writes a new block, announce it to peers so they can sync.
//   3. When a peer announces a block we don't have, fetch and apply it.
//
// The gossip model is simple: every node fans out to every known peer.
// In a larger network this would be replaced with a structured gossip
// (epidemic broadcast), but for a registry with tens of nodes, full fan-out is fine.

// anyhow::Result is unused in this module
use common::Block;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Message sent to peers when we produce a PBFT vote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteGossip {
    pub block_hash: String,
    pub validator_id: String,
    /// Hex-encoded Ed25519 public key of the voting validator.
    pub validator_pubkey: String,
    pub phase: String, // "prepare" | "commit"
    pub approved: bool,
    pub reject_reason: Option<String>,
    /// Hex-encoded Ed25519 signature of "<block_hash>:<approved>".
    pub signature: String,
}

/// Message sent to peers when we write a new block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockAnnouncement {
    pub height: u64,
    pub block_hash: String,
    pub proposer: String,
}

pub struct Gossip {
    client: Client,
    peer_urls: Vec<String>,
}

impl Gossip {
    pub fn new(peer_urls: Vec<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, peer_urls }
    }

    // ── Vote fan-out ──────────────────────────────────────────────────────────

    /// Broadcast a PBFT vote to all known peers concurrently.
    /// Failures are logged but do not propagate — a peer being down is expected.
    pub async fn broadcast_vote(&self, vote: &VoteGossip) {
        let tasks: Vec<_> = self
            .peer_urls
            .iter()
            .map(|url| {
                let client = self.client.clone();
                let url = format!("{}/v1/consensus/vote", url.trim_end_matches('/'));
                let body = vote.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.post(&url).json(&body).send().await {
                        tracing::debug!("Vote gossip to {} failed: {}", url, e);
                    }
                })
            })
            .collect();

        futures::future::join_all(tasks).await;
    }

    // ── Block announcement ────────────────────────────────────────────────────

    /// Announce a new block height to all peers.
    /// Peers will fetch the full block via GET /v1/blocks/:height if they need it.
    pub async fn announce_block(&self, block: &Block) {
        let ann = BlockAnnouncement {
            height: block.header.height,
            block_hash: block.hash(),
            proposer: block.header.proposer_id.clone(),
        };

        let tasks: Vec<_> = self
            .peer_urls
            .iter()
            .map(|url| {
                let client = self.client.clone();
                let url = format!("{}/v1/blocks/announce", url.trim_end_matches('/'));
                let body = ann.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.post(&url).json(&body).send().await {
                        tracing::debug!("Block announce to {} failed: {}", url, e);
                    }
                })
            })
            .collect();

        futures::future::join_all(tasks).await;
    }

    // ── Block fetching ────────────────────────────────────────────────────────

    /// Fetch a specific block from the first peer that has it.
    pub async fn fetch_block(&self, height: u64) -> Option<Block> {
        for url in &self.peer_urls {
            let full_url = format!("{}/v1/blocks/{}", url.trim_end_matches('/'), height);
            match self.client.get(&full_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(block) = resp.json::<Block>().await {
                        tracing::debug!("Fetched block {} from {}", height, url);
                        return Some(block);
                    }
                }
                _ => continue,
            }
        }
        None
    }

    /// Fetch the chain tip height from the first reachable peer.
    pub async fn peer_tip_height(&self) -> Option<u64> {
        #[derive(Deserialize)]
        struct Stats {
            tip_height: u64,
        }

        for url in &self.peer_urls {
            let full_url = format!("{}/v1/chain/stats", url.trim_end_matches('/'));
            if let Ok(resp) = self.client.get(&full_url).send().await {
                if let Ok(stats) = resp.json::<Stats>().await {
                    return Some(stats.tip_height);
                }
            }
        }
        None
    }

    pub fn peer_count(&self) -> usize {
        self.peer_urls.len()
    }
}
