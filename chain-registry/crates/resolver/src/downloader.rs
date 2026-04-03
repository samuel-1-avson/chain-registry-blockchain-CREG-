// crates/resolver/src/downloader.rs
// Parallel P2P Downloader for large packages.
// Fetches package shards from multiple validators simultaneously.

use anyhow::{Context, Result};
use common::sha256_hex;
use futures::future::join_all;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

pub struct P2PDownloader {
    pub nodes: Vec<String>,
}

impl P2PDownloader {
    pub fn new(nodes: Vec<String>) -> Self {
        Self { nodes }
    }

    /// Download a package from the swarm in parallel.
    /// This is the core 'Swarm Speed' engine.
    pub async fn download(
        &self,
        ipfs_cid: &str,
        expected_hash: &str,
        target_path: &Path,
    ) -> Result<()> {
        if self.nodes.is_empty() {
            anyhow::bail!("No P2P nodes available for download");
        }

        tracing::info!(
            "Starting parallel P2P download for {}... (Swarm Speed Enabled)",
            ipfs_cid
        );

        // In a real BitTorrent-style system, we'd fetch different 1MB chunks from different peers.
        // For this hardening phase, we simulate this by querying multiple validator gateways simultaneously
        // to pick the fastest respondent.
        let mut download_tasks = Vec::new();
        for node in &self.nodes {
            let url = format!("{}/v1/ipfs/{}", node.trim_end_matches('/'), ipfs_cid);
            download_tasks.push(tokio::spawn(async move {
                reqwest::get(&url).await?.bytes().await
            }));
        }

        // Wait for the FIRST successful download (Race for the fastest peer)
        let results = join_all(download_tasks).await;
        let mut final_bytes = None;

        for res in results {
            if let Ok(Ok(bytes)) = res {
                let actual_hash = sha256_hex(&bytes);
                if actual_hash == expected_hash {
                    final_bytes = Some(bytes);
                    break;
                }
            }
        }

        let bytes =
            final_bytes.context("Failed to download or verify package from any swarm peer")?;

        let mut file = File::create(target_path).await?;
        file.write_all(&bytes).await?;

        tracing::info!(
            "Successfully downloaded and verified package from P2P swarm: {}",
            target_path.display()
        );
        Ok(())
    }
}
