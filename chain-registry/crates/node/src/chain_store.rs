// crates/node/src/chain_store.rs
// Persistent storage for the blockchain using sled.
// Stores blocks by height and by hash, and a package index by canonical ID.

use anyhow::{Context, Result};
use common::{Block, ChainRecord, PackageStatus};
use sled::{Db, Tree};
use std::path::Path;
use semver;

pub struct ChainStore {
    #[allow(dead_code)]
    db:            Db,
    blocks_by_hash: Tree,   // block_hash  → Block (JSON)
    blocks_by_height: Tree, // height (8 bytes BE) → block_hash
    packages:      Tree,    // canonical  → ChainRecord (JSON)
}

impl ChainStore {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let db = sled::open(data_dir.join("chain.db"))
            .context("Failed to open chain database")?;

        let blocks_by_hash   = db.open_tree("blocks_by_hash")?;
        let blocks_by_height = db.open_tree("blocks_by_height")?;
        let packages         = db.open_tree("packages")?;

        let store = Self { db, blocks_by_hash, blocks_by_height, packages };

        // Write the genesis block if the chain is empty.
        if store.tip_height()? == 0 && store.blocks_by_height.is_empty() {
            let genesis = Block::genesis();
            store.insert_block(&genesis)?;
            tracing::info!("Chain initialised with genesis block");
        }

        Ok(store)
    }

    // ── Block operations ─────────────────────────────────────────────────────

    pub fn insert_block(&self, block: &Block) -> Result<()> {
        let hash  = block.hash();
        let bytes = serde_json::to_vec(block)?;
        let height_key = block.header.height.to_be_bytes();

        self.blocks_by_hash.insert(hash.as_bytes(), bytes.as_slice())?;
        self.blocks_by_height.insert(height_key, hash.as_bytes())?;

        // Index every Publish transaction into the package tree.
        for tx in &block.transactions {
            if let common::Transaction::Publish(record) = tx {
                // Update block_hash to the real finalized hash before persisting.
                let mut rec = record.clone();
                rec.block_hash = hash.clone();
                let rec_bytes = serde_json::to_vec(&rec)?;
                self.packages.insert(rec.id.canonical().as_bytes(), rec_bytes)?;
            }
            if let common::Transaction::Revoke { package_canonical, reason, .. } = tx {
                if let Ok(Some(bytes)) = self.packages.get(package_canonical.as_bytes()) {
                    if let Ok(mut rec) = serde_json::from_slice::<ChainRecord>(&bytes) {
                        rec.status = PackageStatus::Revoked { reason: reason.clone() };
                        let updated = serde_json::to_vec(&rec)?;
                        self.packages.insert(package_canonical.as_bytes(), updated)?;
                    }
                }
            }
        }

        tracing::info!("Block {} inserted (height {})", &hash[..hash.len().min(12)], block.header.height);
        Ok(())
    }

    pub fn get_block_by_hash(&self, hash: &str) -> Result<Option<Block>> {
        match self.blocks_by_hash.get(hash.as_bytes())? {
            None    => Ok(None),
            Some(b) => Ok(Some(serde_json::from_slice(&b)?)),
        }
    }

    pub fn get_block_by_height(&self, height: u64) -> Result<Option<Block>> {
        match self.blocks_by_height.get(height.to_be_bytes())? {
            None => Ok(None),
            Some(hash_bytes) => {
                let hash = std::str::from_utf8(&hash_bytes)?;
                self.get_block_by_hash(hash)
            }
        }
    }

    pub fn tip_height(&self) -> Result<u64> {
        match self.blocks_by_height.last()? {
            None => Ok(0),
            Some((key, _)) => {
                let bytes: [u8; 8] = key.as_ref().try_into().unwrap_or([0; 8]);
                Ok(u64::from_be_bytes(bytes))
            }
        }
    }

    pub fn tip_hash(&self) -> Result<String> {
        let height = self.tip_height()?;
        match self.get_block_by_height(height)? {
            Some(b) => Ok(b.hash()),
            None    => Ok("0".repeat(64)),
        }
    }

    // ── Package index ─────────────────────────────────────────────────────────

    pub fn get_package(&self, canonical: &str) -> Result<Option<ChainRecord>> {
        match self.packages.get(canonical.as_bytes())? {
            None    => Ok(None),
            Some(b) => {
                let record: ChainRecord = serde_json::from_slice(&b)?;
                Ok(Some(record))
            }
        }
    }

    /// Mark a package as accessed and update metadata.
    pub fn mark_accessed(&self, canonical: &str) -> Result<()> {
        if let Some(mut record) = self.get_package(canonical)? {
            record.access_count += 1;
            record.last_accessed = Some(chrono::Utc::now());
            self.save_package(&record)?;
        }
        Ok(())
    }

    pub fn save_package(&self, record: &ChainRecord) -> Result<()> {
        let bytes = serde_json::to_vec(record)?;
        self.packages.insert(record.id.canonical().as_bytes(), bytes.as_slice())?;
        Ok(())
    }

    /// Find the latest verified version of a package in a given ecosystem.
    pub fn get_latest_version(&self, ecosystem: &str, name: &str) -> Result<Option<ChainRecord>> {
        let prefix = format!("{}:{}", ecosystem, name);
        let mut latest: Option<ChainRecord> = None;

        for item in self.packages.scan_prefix(prefix.as_bytes()) {
            let (_, bytes) = item?;
            let record: ChainRecord = serde_json::from_slice(&bytes)?;

            if record.status == PackageStatus::Verified {
                let is_newer = match &latest {
                    None => true,
                    Some(current) => {
                        // Parse as semver for correct ordering (e.g., 9.0.0 < 10.0.0).
                        // Fall back to string comparison if either version is non-semver.
                        let new_ver = semver::Version::parse(&record.id.version);
                        let cur_ver = semver::Version::parse(&current.id.version);
                        match (new_ver, cur_ver) {
                            (Ok(nv), Ok(cv)) => nv > cv,
                            _ => record.id.version > current.id.version,
                        }
                    }
                };
                if is_newer {
                    latest = Some(record);
                }
            }
        }
        Ok(latest)
    }

    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    // ── Chain stats ───────────────────────────────────────────────────────────

    pub fn stats(&self) -> ChainStats {
        ChainStats {
            tip_height:    self.tip_height().unwrap_or(0),
            tip_hash:      self.tip_hash().unwrap_or_default(),
            package_count: self.package_count(),
            block_count:   self.blocks_by_height.len(),
        }
    }
}

#[derive(serde::Serialize)]
pub struct ChainStats {
    pub tip_height:    u64,
    pub tip_hash:      String,
    pub package_count: usize,
    pub block_count:   usize,
}
