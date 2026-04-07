// crates/node/src/chain_store.rs
// Persistent storage for the blockchain using RocksDB.
// Stores blocks by height and by hash, and a package index by canonical ID.
// Replaced sled for better write amplification, compaction, and snapshot support.

use anyhow::{Context, Result};
use common::{Block, ChainRecord, PackageStatus};
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use semver;
use sled;
use std::path::Path;
use std::sync::Arc;

const CF_BLOCKS_BY_HASH: &str = "blocks_by_hash";
const CF_BLOCKS_BY_HEIGHT: &str = "blocks_by_height";
const CF_PACKAGES: &str = "packages";

#[derive(Clone)]
pub struct ChainStore {
    db: Arc<DB>,
}

impl ChainStore {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;

        let db_path = data_dir.join("chain.rocksdb");

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        // Tuning for write-heavy workload (block insertion + package indexing).
        opts.set_write_buffer_size(64 * 1024 * 1024); // 64 MB memtable
        opts.set_max_write_buffer_number(3);
        opts.set_target_file_size_base(64 * 1024 * 1024);
        opts.set_level_compaction_dynamic_level_bytes(true);
        opts.set_max_background_jobs(4);

        let cf_opts = Options::default();
        let cfs = vec![
            ColumnFamilyDescriptor::new(CF_BLOCKS_BY_HASH, cf_opts.clone()),
            ColumnFamilyDescriptor::new(CF_BLOCKS_BY_HEIGHT, cf_opts.clone()),
            ColumnFamilyDescriptor::new(CF_PACKAGES, cf_opts),
        ];

        let db =
            DB::open_cf_descriptors(&opts, &db_path, cfs).context("Failed to open RocksDB")?;

        let store = Self { db: Arc::new(db) };

        // Write the genesis block if the chain is empty.
        if store.tip_height()? == 0 && store.block_count() == 0 {
            let genesis = Block::genesis();
            store.insert_block(&genesis)?;
            tracing::info!("Chain initialised with genesis block");
        }

        Ok(store)
    }

    /// Migrate data from a legacy sled database into this RocksDB store.
    /// Call once at startup if the sled directory still exists.
    pub fn migrate_from_sled(&self, sled_dir: &Path) -> Result<u64> {
        let sled_db_path = sled_dir.join("chain.db");
        if !sled_db_path.exists() {
            return Ok(0);
        }
        tracing::info!("Migrating data from sled → RocksDB ...");

        let sled_db = sled::open(&sled_db_path).context("open legacy sled DB")?;
        let mut migrated = 0u64;

        // Migrate blocks_by_hash
        if let Ok(tree) = sled_db.open_tree("blocks_by_hash") {
            let cf = self
                .db
                .cf_handle(CF_BLOCKS_BY_HASH)
                .context("cf blocks_by_hash")?;
            for item in tree.iter() {
                let (k, v) = item?;
                self.db.put_cf(&cf, &k, &v)?;
                migrated += 1;
            }
        }

        // Migrate blocks_by_height
        if let Ok(tree) = sled_db.open_tree("blocks_by_height") {
            let cf = self
                .db
                .cf_handle(CF_BLOCKS_BY_HEIGHT)
                .context("cf blocks_by_height")?;
            for item in tree.iter() {
                let (k, v) = item?;
                self.db.put_cf(&cf, &k, &v)?;
                migrated += 1;
            }
        }

        // Migrate packages
        if let Ok(tree) = sled_db.open_tree("packages") {
            let cf = self
                .db
                .cf_handle(CF_PACKAGES)
                .context("cf packages")?;
            for item in tree.iter() {
                let (k, v) = item?;
                self.db.put_cf(&cf, &k, &v)?;
                migrated += 1;
            }
        }

        tracing::info!("Migrated {} records from sled → RocksDB", migrated);
        Ok(migrated)
    }

    // ── Block operations ─────────────────────────────────────────────────────

    pub fn insert_block(&self, block: &Block) -> Result<()> {
        let hash = block.hash();
        let bytes = serde_json::to_vec(block)?;
        let height_key = block.header.height.to_be_bytes();

        let cf_hash = self
            .db
            .cf_handle(CF_BLOCKS_BY_HASH)
            .context("cf blocks_by_hash")?;
        let cf_height = self
            .db
            .cf_handle(CF_BLOCKS_BY_HEIGHT)
            .context("cf blocks_by_height")?;
        let cf_pkg = self
            .db
            .cf_handle(CF_PACKAGES)
            .context("cf packages")?;

        // Use a WriteBatch for atomicity.
        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(&cf_hash, hash.as_bytes(), &bytes);
        batch.put_cf(&cf_height, height_key, hash.as_bytes());

        // Index every Publish transaction into the package tree.
        for tx in &block.transactions {
            if let common::Transaction::Publish(record) = tx {
                // Update block_hash to the real finalized hash before persisting.
                let mut rec = record.clone();
                rec.block_hash = hash.clone();
                let rec_bytes = serde_json::to_vec(&rec)?;
                batch.put_cf(&cf_pkg, rec.id.canonical().as_bytes(), &rec_bytes);
            }
            if let common::Transaction::Revoke {
                package_canonical,
                reason,
                ..
            } = tx
            {
                if let Some(existing) = self.db.get_cf(&cf_pkg, package_canonical.as_bytes())? {
                    if let Ok(mut rec) = serde_json::from_slice::<ChainRecord>(&existing) {
                        rec.status = PackageStatus::Revoked {
                            reason: reason.clone(),
                        };
                        let updated = serde_json::to_vec(&rec)?;
                        batch.put_cf(&cf_pkg, package_canonical.as_bytes(), &updated);
                    }
                }
            }
            if let common::Transaction::RotatePublisherKey {
                canonical_prefix,
                old_pubkey,
                new_pubkey,
                ..
            } = tx
            {
                let prefix = canonical_prefix.as_bytes();
                let iter = self.db.prefix_iterator_cf(&cf_pkg, prefix);
                for item in iter {
                    let (key_bytes, val_bytes) = item?;
                    if !key_bytes.starts_with(prefix) {
                        break;
                    }
                    if let Ok(mut rec) = serde_json::from_slice::<ChainRecord>(&val_bytes) {
                        if rec.publisher_pubkey == *old_pubkey {
                            rec.publisher_pubkey = new_pubkey.clone();
                            if let Ok(updated) = serde_json::to_vec(&rec) {
                                batch.put_cf(&cf_pkg, &*key_bytes, &updated);
                            }
                        }
                    }
                }
            }
        }

        self.db.write(batch)?;

        tracing::info!(
            "Block {} inserted (height {})",
            &hash[..hash.len().min(12)],
            block.header.height
        );
        Ok(())
    }

    pub fn get_block_by_hash(&self, hash: &str) -> Result<Option<Block>> {
        let cf = self
            .db
            .cf_handle(CF_BLOCKS_BY_HASH)
            .context("cf blocks_by_hash")?;
        match self.db.get_cf(&cf, hash.as_bytes())? {
            None => Ok(None),
            Some(b) => Ok(Some(serde_json::from_slice(&b)?)),
        }
    }

    pub fn get_block_by_height(&self, height: u64) -> Result<Option<Block>> {
        let cf = self
            .db
            .cf_handle(CF_BLOCKS_BY_HEIGHT)
            .context("cf blocks_by_height")?;
        match self.db.get_cf(&cf, height.to_be_bytes())? {
            None => Ok(None),
            Some(hash_bytes) => {
                let hash = std::str::from_utf8(&hash_bytes)?;
                self.get_block_by_hash(hash)
            }
        }
    }

    pub fn tip_height(&self) -> Result<u64> {
        let cf = self
            .db
            .cf_handle(CF_BLOCKS_BY_HEIGHT)
            .context("cf blocks_by_height")?;
        let mut iter = self.db.raw_iterator_cf(&cf);
        iter.seek_to_last();
        if iter.valid() {
            if let Some(key) = iter.key() {
                let bytes: [u8; 8] = key.try_into().unwrap_or([0; 8]);
                return Ok(u64::from_be_bytes(bytes));
            }
        }
        Ok(0)
    }

    pub fn tip_hash(&self) -> Result<String> {
        let height = self.tip_height()?;
        match self.get_block_by_height(height)? {
            Some(b) => Ok(b.hash()),
            None => Ok("0".repeat(64)),
        }
    }

    // ── Package index ─────────────────────────────────────────────────────────

    pub fn get_package(&self, canonical: &str) -> Result<Option<ChainRecord>> {
        let cf = self.db.cf_handle(CF_PACKAGES).context("cf packages")?;
        match self.db.get_cf(&cf, canonical.as_bytes())? {
            None => Ok(None),
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
        let cf = self.db.cf_handle(CF_PACKAGES).context("cf packages")?;
        let bytes = serde_json::to_vec(record)?;
        self.db
            .put_cf(&cf, record.id.canonical().as_bytes(), &bytes)?;
        Ok(())
    }

    /// Find the latest verified version of a package in a given ecosystem.
    pub fn get_latest_version(&self, ecosystem: &str, name: &str) -> Result<Option<ChainRecord>> {
        let cf = self.db.cf_handle(CF_PACKAGES).context("cf packages")?;
        let prefix = format!("{}:{}", ecosystem, name);
        let mut latest: Option<ChainRecord> = None;

        for item in self.db.prefix_iterator_cf(&cf, prefix.as_bytes()) {
            let (key, bytes) = item?;
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
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
        let cf = match self.db.cf_handle(CF_PACKAGES) {
            Some(cf) => cf,
            None => return 0,
        };
        let mut count = 0usize;
        let mut iter = self.db.raw_iterator_cf(&cf);
        iter.seek_to_first();
        while iter.valid() {
            count += 1;
            iter.next();
        }
        count
    }

    /// Check whether `publisher_pubkey` owns at least one package with the given prefix.
    pub fn has_publisher_for_prefix(&self, prefix: &str, publisher_pubkey: &str) -> bool {
        let cf = match self.db.cf_handle(CF_PACKAGES) {
            Some(cf) => cf,
            None => return false,
        };
        for item in self.db.prefix_iterator_cf(&cf, prefix.as_bytes()) {
            let Ok((key, bytes)) = item else { continue };
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            if let Ok(record) = serde_json::from_slice::<ChainRecord>(&bytes) {
                if record.publisher_pubkey == publisher_pubkey {
                    return true;
                }
            }
        }
        false
    }

    /// Return the last-used rotation nonce for the given publisher pubkey.
    /// Returns 0 if no rotation has been recorded.
    pub fn publisher_rotation_nonce(&self, pubkey: &str) -> Option<u64> {
        // Scan all blocks in reverse for the most recent RotatePublisherKey
        // transaction matching this pubkey.  A dedicated CF would be more
        // efficient at scale, but this is correct for now.
        let tip = self.tip_height().ok()?;
        for h in (0..=tip).rev() {
            if let Ok(Some(block)) = self.get_block_by_height(h) {
                for tx in &block.transactions {
                    if let common::Transaction::RotatePublisherKey {
                        old_pubkey,
                        nonce,
                        ..
                    } = tx
                    {
                        if old_pubkey == pubkey {
                            return Some(*nonce);
                        }
                    }
                }
            }
        }
        Some(0)
    }

    /// Return the timestamp of the most recent key rotation by this pubkey.
    /// Used to enforce a cooldown period between rotations.
    pub fn publisher_last_rotation_time(
        &self,
        pubkey: &str,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let tip = self.tip_height().ok()?;
        for h in (0..=tip).rev() {
            if let Ok(Some(block)) = self.get_block_by_height(h) {
                for tx in &block.transactions {
                    if let common::Transaction::RotatePublisherKey { old_pubkey, .. } = tx {
                        if old_pubkey == pubkey {
                            return Some(block.timestamp);
                        }
                    }
                }
            }
        }
        None
    }

    // ── Chain stats ───────────────────────────────────────────────────────────

    /// List packages with pagination and optional filters.
    ///
    /// Returns `(records, total_matching)` where `total_matching` is the count
    /// of all records that pass the filters (before offset/limit).
    pub fn list_packages(
        &self,
        offset: usize,
        limit: usize,
        ecosystem: Option<&str>,
        status: Option<&PackageStatus>,
    ) -> Result<(Vec<ChainRecord>, usize)> {
        let cf = self.db.cf_handle(CF_PACKAGES).context("cf packages")?;
        let mut matching = Vec::new();

        let iter_box: Box<dyn Iterator<Item = Result<(Box<[u8]>, Box<[u8]>), rocksdb::Error>>> =
            if let Some(eco) = ecosystem {
                Box::new(
                    self.db
                        .prefix_iterator_cf(&cf, format!("{}:", eco).as_bytes()),
                )
            } else {
                Box::new(
                    self.db
                        .iterator_cf(&cf, rocksdb::IteratorMode::Start),
                )
            };

        for item in iter_box {
            let (key, bytes) = item?;
            if let Some(eco) = ecosystem {
                let prefix = format!("{}:", eco);
                if !key.starts_with(prefix.as_bytes()) {
                    break;
                }
            }
            let record: ChainRecord = serde_json::from_slice(&bytes)?;

            if let Some(st) = status {
                let matches = match (st, &record.status) {
                    (PackageStatus::Verified, PackageStatus::Verified) => true,
                    (PackageStatus::Pending, PackageStatus::Pending) => true,
                    (PackageStatus::Revoked { .. }, PackageStatus::Revoked { .. }) => true,
                    _ => false,
                };
                if !matches {
                    continue;
                }
            }

            matching.push(record);
        }

        let total = matching.len();
        let page: Vec<ChainRecord> = matching.into_iter().skip(offset).take(limit).collect();
        Ok((page, total))
    }

    fn block_count(&self) -> usize {
        let cf = match self.db.cf_handle(CF_BLOCKS_BY_HEIGHT) {
            Some(cf) => cf,
            None => return 0,
        };
        let mut count = 0usize;
        let mut iter = self.db.raw_iterator_cf(&cf);
        iter.seek_to_first();
        while iter.valid() {
            count += 1;
            iter.next();
        }
        count
    }

    pub fn stats(&self) -> ChainStats {
        ChainStats {
            tip_height: self.tip_height().unwrap_or(0),
            tip_hash: self.tip_hash().unwrap_or_default(),
            package_count: self.package_count(),
            block_count: self.block_count(),
        }
    }
}

#[derive(serde::Serialize)]
pub struct ChainStats {
    pub tip_height: u64,
    pub tip_hash: String,
    pub package_count: usize,
    pub block_count: usize,
}
