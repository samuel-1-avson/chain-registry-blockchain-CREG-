// crates/node/src/chain_store.rs
// Persistent storage for the blockchain using RocksDB.
// Stores blocks by height and by hash, and a package index by canonical ID.
// Replaced sled for better write amplification, compaction, and snapshot support.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use common::{
    consistency_proof, inclusion_proof, transaction_hash, tree_root, Block, ChainRecord,
    ConsistencyProof, InclusionProof, PackageStatus, Transaction, TransparencyEvent, TreeHead,
    TRANSPARENCY_LOG_VERSION,
};
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use semver;
use sled;
use std::sync::Arc;
use std::{collections::HashSet, path::Path};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FinalizedWorkEnvelope {
    schema_version: u8,
    transaction: Transaction,
    enqueued_at: String,
    retry_count: u32,
    last_error: Option<String>,
    next_attempt_at: Option<String>,
}

const CF_BLOCKS_BY_HASH: &str = "blocks_by_hash";
const CF_BLOCKS_BY_HEIGHT: &str = "blocks_by_height";
const CF_PACKAGES: &str = "packages";
const CF_NAMESPACE_CONTROLLERS: &str = "namespace_controllers";
const CF_NAMESPACE_LIFECYCLE_NONCES: &str = "namespace_lifecycle_nonces";
const CF_APPLIED_NAMESPACE_LIFECYCLE_NONCES: &str = "applied_namespace_lifecycle_nonces";
const CF_PUBLISHER_ADMISSION_NONCES: &str = "publisher_admission_nonces";
const CF_FINALIZED_WORK: &str = "finalized_work";
const CF_FINALIZED_WORK_DEAD_LETTER: &str = "finalized_work_dead_letter";
const CF_TRANSPARENCY_EVENTS: &str = "transparency_events";
const CF_TRANSPARENCY_LEAVES: &str = "transparency_leaves";
const CF_TRANSPARENCY_HEADS: &str = "transparency_heads";
const MAX_FINALIZED_WORK_RETRIES: u32 = 5;

/// Operational view of the durable finalized-work queue.  Every field is
/// derived from the persisted queue, so a process restart cannot reset the
/// values that operators use to decide whether work is backing up.
#[derive(Debug, Clone, Default)]
pub struct FinalizedWorkStats {
    pub active_depth: u64,
    pub ready_depth: u64,
    pub deferred_depth: u64,
    pub retrying_depth: u64,
    pub retry_attempts: u64,
    pub oldest_age_seconds: Option<u64>,
}

/// Result of recording a failed production attempt.  `Missing` is distinct
/// from `Retrying`: forced-inclusion work can be legitimately absent from the
/// durable queue and must stay in the in-memory buffer rather than being
/// silently discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizedWorkFailureOutcome {
    Missing,
    Retrying,
    DeadLettered,
}

fn namespace_lifecycle_nonce_key(
    namespace: &str,
    current_controller_pubkey: &str,
    nonce: u64,
) -> Result<String> {
    common::validate_namespace_prefix(namespace)
        .map_err(|error| anyhow::anyhow!("invalid namespace lifecycle nonce: {}", error))?;
    let controller = current_controller_pubkey.trim().to_ascii_lowercase();
    if controller.len() != 64 || !controller.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("namespace lifecycle controller key must be hex Ed25519");
    }
    Ok(format!("{}\0{}\0{:020}", namespace, controller, nonce))
}

#[derive(Clone)]
pub struct ChainStore {
    db: Arc<DB>,
    /// RocksDB does not provide a conditional put through this wrapper. This
    /// lock makes check-and-record atomic for concurrent admissions handled by
    /// this node; the durable record then protects restarts. Cross-node replay
    /// prevention still requires the target canonical event log.
    publisher_admission_nonce_lock: Arc<std::sync::Mutex<()>>,
    /// Serializes append/root calculation. RocksDB batches make the event,
    /// leaf, and head durable together; this lock prevents two local writers
    /// from choosing the same sequence number.
    transparency_log_lock: Arc<std::sync::Mutex<()>>,
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
            ColumnFamilyDescriptor::new(CF_NAMESPACE_CONTROLLERS, Options::default()),
            ColumnFamilyDescriptor::new(CF_NAMESPACE_LIFECYCLE_NONCES, Options::default()),
            ColumnFamilyDescriptor::new(CF_APPLIED_NAMESPACE_LIFECYCLE_NONCES, Options::default()),
            ColumnFamilyDescriptor::new(CF_PUBLISHER_ADMISSION_NONCES, Options::default()),
            ColumnFamilyDescriptor::new(CF_FINALIZED_WORK, Options::default()),
            ColumnFamilyDescriptor::new(CF_FINALIZED_WORK_DEAD_LETTER, Options::default()),
            ColumnFamilyDescriptor::new(CF_TRANSPARENCY_EVENTS, Options::default()),
            ColumnFamilyDescriptor::new(CF_TRANSPARENCY_LEAVES, Options::default()),
            ColumnFamilyDescriptor::new(CF_TRANSPARENCY_HEADS, Options::default()),
        ];

        let db = DB::open_cf_descriptors(&opts, &db_path, cfs).context("Failed to open RocksDB")?;

        let store = Self {
            db: Arc::new(db),
            publisher_admission_nonce_lock: Arc::new(std::sync::Mutex::new(())),
            transparency_log_lock: Arc::new(std::sync::Mutex::new(())),
        };

        // Write the genesis block if the chain is empty.
        if store.tip_height()? == 0 && store.block_count() == 0 {
            let genesis = Block::genesis();
            store.insert_block(&genesis)?;
            tracing::info!("Chain initialised with genesis block");
        }

        Ok(store)
    }

    pub fn open_read_only(data_dir: &Path) -> Result<Self> {
        let db_path = data_dir.join("chain.rocksdb");
        if !db_path.exists() {
            anyhow::bail!(
                "Chain database not found at {}. Start a node with CREG_DATA_DIR={} first.",
                db_path.display(),
                data_dir.display()
            );
        }

        let opts = Options::default();
        let cfs = vec![
            CF_BLOCKS_BY_HASH,
            CF_BLOCKS_BY_HEIGHT,
            CF_PACKAGES,
            CF_NAMESPACE_CONTROLLERS,
            CF_NAMESPACE_LIFECYCLE_NONCES,
            CF_APPLIED_NAMESPACE_LIFECYCLE_NONCES,
            CF_PUBLISHER_ADMISSION_NONCES,
            CF_FINALIZED_WORK,
            CF_FINALIZED_WORK_DEAD_LETTER,
            CF_TRANSPARENCY_EVENTS,
            CF_TRANSPARENCY_LEAVES,
            CF_TRANSPARENCY_HEADS,
        ];
        let db = DB::open_cf_for_read_only(&opts, &db_path, cfs, false)
            .context("Failed to open RocksDB in read-only mode")?;

        Ok(Self {
            db: Arc::new(db),
            publisher_admission_nonce_lock: Arc::new(std::sync::Mutex::new(())),
            transparency_log_lock: Arc::new(std::sync::Mutex::new(())),
        })
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
            let cf = self.db.cf_handle(CF_PACKAGES).context("cf packages")?;
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

    /// Insert a block, detecting whether it replaces a different block that
    /// was already finalized at the same height (i.e. a fork / reorg).
    ///
    /// Returns the outcome so callers can record a `ReorgEvent` when a
    /// replacement happened instead of silently overwriting history.
    pub fn insert_block_with_outcome(&self, block: &Block) -> Result<BlockInsertOutcome> {
        let new_hash = block.hash();
        let replaced_hash = match self.get_block_by_height(block.header.height)? {
            Some(existing) => {
                let existing_hash = existing.hash();
                if existing_hash != new_hash {
                    Some(existing_hash)
                } else {
                    None
                }
            }
            None => None,
        };
        self.insert_block(block)?;
        Ok(BlockInsertOutcome {
            hash: new_hash,
            replaced_hash,
        })
    }

    /// Remove the height index for every block above `height`, returning the
    /// hashes of the abandoned blocks (highest first). The blocks themselves
    /// remain retrievable by hash so abandoned forks stay inspectable in the
    /// explorer; only the canonical height → hash mapping is rewound.
    ///
    /// Callers MUST rebuild derived state (package index, publisher index)
    /// afterwards via `rebuild_package_index` + `PublisherIndex::rebuild_from_chain`.
    pub fn rollback_to_height(&self, height: u64) -> Result<Vec<String>> {
        let tip = self.tip_height()?;
        if tip <= height {
            return Ok(vec![]);
        }
        let cf_height = self
            .db
            .cf_handle(CF_BLOCKS_BY_HEIGHT)
            .context("cf blocks_by_height")?;

        let mut abandoned = Vec::new();
        let mut batch = rocksdb::WriteBatch::default();
        for h in ((height + 1)..=tip).rev() {
            if let Some(hash_bytes) = self.db.get_cf(&cf_height, h.to_be_bytes())? {
                abandoned.push(String::from_utf8_lossy(&hash_bytes).to_string());
            }
            batch.delete_cf(&cf_height, h.to_be_bytes());
        }
        self.db.write(batch)?;
        tracing::warn!(
            "Chain rolled back from height {} to {} ({} blocks abandoned)",
            tip,
            height,
            abandoned.len()
        );
        Ok(abandoned)
    }

    /// Rebuild the package index (CF_PACKAGES) by replaying every canonical
    /// block from genesis to the current tip. Required after a reorg rewind so
    /// records indexed from abandoned blocks don't linger.
    pub fn rebuild_package_index(&self) -> Result<()> {
        let cf_pkg = self.db.cf_handle(CF_PACKAGES).context("cf packages")?;
        let cf_namespace_controllers = self
            .db
            .cf_handle(CF_NAMESPACE_CONTROLLERS)
            .context("cf namespace_controllers")?;
        let cf_applied_namespace_lifecycle_nonces = self
            .db
            .cf_handle(CF_APPLIED_NAMESPACE_LIFECYCLE_NONCES)
            .context("cf applied_namespace_lifecycle_nonces")?;
        let cf_finalized_work = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;

        // Clear the existing index.
        let keys: Vec<Box<[u8]>> = self
            .db
            .iterator_cf(&cf_pkg, rocksdb::IteratorMode::Start)
            .filter_map(|item| item.ok().map(|(k, _)| k))
            .collect();
        let mut batch = rocksdb::WriteBatch::default();
        for key in keys {
            batch.delete_cf(&cf_pkg, key);
        }
        let namespace_keys: Vec<Box<[u8]>> = self
            .db
            .iterator_cf(&cf_namespace_controllers, rocksdb::IteratorMode::Start)
            .filter_map(|item| item.ok().map(|(k, _)| k))
            .collect();
        for key in namespace_keys {
            batch.delete_cf(&cf_namespace_controllers, key);
        }
        let applied_nonce_keys: Vec<Box<[u8]>> = self
            .db
            .iterator_cf(
                &cf_applied_namespace_lifecycle_nonces,
                rocksdb::IteratorMode::Start,
            )
            .filter_map(|item| item.ok().map(|(k, _)| k))
            .collect();
        for key in applied_nonce_keys {
            batch.delete_cf(&cf_applied_namespace_lifecycle_nonces, key);
        }
        self.db.write(batch)?;

        // Replay the canonical chain. insert_block re-indexes Publish /
        // Revoke / RotatePublisherKey transactions idempotently.
        let tip = self.tip_height()?;
        for h in 0..=tip {
            if let Some(block) = self.get_block_by_height(h)? {
                self.insert_block(&block)?;
            }
        }
        tracing::info!("Package index rebuilt from canonical chain (tip {})", tip);
        Ok(())
    }

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
        let cf_pkg = self.db.cf_handle(CF_PACKAGES).context("cf packages")?;
        let cf_namespace_controllers = self
            .db
            .cf_handle(CF_NAMESPACE_CONTROLLERS)
            .context("cf namespace_controllers")?;
        let cf_applied_namespace_lifecycle_nonces = self
            .db
            .cf_handle(CF_APPLIED_NAMESPACE_LIFECYCLE_NONCES)
            .context("cf applied_namespace_lifecycle_nonces")?;
        let cf_finalized_work = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;

        // Use a WriteBatch for atomicity.
        let mut batch = rocksdb::WriteBatch::default();
        let mut lifecycle_nonces_in_block = HashSet::new();
        batch.put_cf(&cf_hash, hash.as_bytes(), &bytes);
        batch.put_cf(&cf_height, height_key, hash.as_bytes());

        // Index every Publish transaction into the package tree.
        for tx in &block.transactions {
            // A finalized block is the acknowledgement boundary for durable
            // work. Include this delete in the same RocksDB WriteBatch as the
            // block/index writes so a restart cannot lose work after ack.
            batch.delete_cf(&cf_finalized_work, transaction_hash(tx).as_bytes());
            if let common::Transaction::Publish(record) = tx {
                // Update block_hash to the real finalized hash before persisting.
                let mut rec = record.clone();
                rec.block_hash = hash.clone();
                let rec_bytes = serde_json::to_vec(&rec)?;
                batch.put_cf(&cf_pkg, rec.id.canonical().as_bytes(), &rec_bytes);
                let namespace = common::package_namespace_prefix(&rec.id);
                if self
                    .db
                    .get_cf(&cf_namespace_controllers, namespace.as_bytes())?
                    .is_none()
                {
                    batch.put_cf(
                        &cf_namespace_controllers,
                        namespace.as_bytes(),
                        rec.publisher_pubkey.as_bytes(),
                    );
                }
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
                if self
                    .db
                    .get_cf(&cf_namespace_controllers, canonical_prefix.as_bytes())?
                    .as_deref()
                    .is_some_and(|controller| controller == old_pubkey.as_bytes())
                {
                    batch.put_cf(
                        &cf_namespace_controllers,
                        canonical_prefix.as_bytes(),
                        new_pubkey.as_bytes(),
                    );
                }
            }
            if let common::Transaction::NamespaceLifecycle(event) = tx {
                // Recovery is deliberately fail-closed until an ecosystem
                // authority adapter can verify its proof.  This check is also
                // needed when rebuilding or receiving a block: HTTP admission
                // is not a consensus boundary.  Delegate and transfer remain
                // independently replay-verifiable using both signatures.
                let nonce_key = namespace_lifecycle_nonce_key(
                    &event.namespace,
                    &event.current_controller_ed25519_pubkey,
                    event.nonce,
                );
                if !matches!(event.action, common::NamespaceLifecycleAction::Recover)
                    && event.is_valid_at(block.header.timestamp)
                    && event.verify_controller_signatures().is_ok()
                    && nonce_key
                        .as_ref()
                        .is_ok_and(|key| lifecycle_nonces_in_block.insert(key.clone()))
                    && nonce_key.as_ref().is_ok_and(|key| {
                        matches!(
                            self.db
                                .get_cf(&cf_applied_namespace_lifecycle_nonces, key.as_bytes()),
                            Ok(None)
                        )
                    })
                    && self
                        .db
                        .get_cf(&cf_namespace_controllers, event.namespace.as_bytes())?
                        .as_deref()
                        .is_some_and(|controller| {
                            controller == event.current_controller_ed25519_pubkey.as_bytes()
                        })
                {
                    let nonce_key = nonce_key.expect("validated above");
                    batch.put_cf(
                        &cf_namespace_controllers,
                        event.namespace.as_bytes(),
                        event.next_controller_ed25519_pubkey.as_bytes(),
                    );
                    batch.put_cf(
                        &cf_applied_namespace_lifecycle_nonces,
                        nonce_key.as_bytes(),
                        b"",
                    );
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

    /// Return up to `limit` blocks in descending height order starting from
    /// `tip_height - offset`.  Used for paginated API responses.
    pub fn list_blocks(&self, offset: u64, limit: u64) -> Result<Vec<Block>> {
        let tip = self.tip_height()?;
        if tip == 0 {
            return Ok(vec![]);
        }
        let start_height = tip.saturating_sub(offset);
        let mut blocks = Vec::with_capacity(limit as usize);
        let mut height = start_height;
        while blocks.len() < limit as usize {
            match self.get_block_by_height(height)? {
                Some(b) => blocks.push(b),
                None => break,
            }
            if height == 0 {
                break;
            }
            height -= 1;
        }
        Ok(blocks)
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
        let namespace_cf = self
            .db
            .cf_handle(CF_NAMESPACE_CONTROLLERS)
            .context("cf namespace_controllers")?;
        let bytes = serde_json::to_vec(record)?;
        self.db
            .put_cf(&cf, record.id.canonical().as_bytes(), &bytes)?;
        // Keep direct package-store users (including maintenance tooling) on
        // the same first-publisher ownership rule as block insertion.  Never
        // overwrite an established controller here; lifecycle events are the
        // only controller-change path.
        let namespace = common::package_namespace_prefix(&record.id);
        if self
            .db
            .get_cf(&namespace_cf, namespace.as_bytes())?
            .is_none()
        {
            self.db.put_cf(
                &namespace_cf,
                namespace.as_bytes(),
                record.publisher_pubkey.as_bytes(),
            )?;
        }
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

    /// Return the first non-empty publisher key observed for a package
    /// namespace.  Callers use this as an ownership guard before admitting a
    /// new version; the canonical event history remains authoritative.
    pub fn publisher_for_prefix(&self, prefix: &str) -> Option<String> {
        if let Some(cf) = self.db.cf_handle(CF_NAMESPACE_CONTROLLERS) {
            if let Ok(Some(controller)) = self.db.get_cf(&cf, prefix.as_bytes()) {
                let controller = String::from_utf8_lossy(&controller).trim().to_string();
                if !controller.is_empty() {
                    return Some(controller);
                }
            }
        }
        let cf = self.db.cf_handle(CF_PACKAGES)?;
        for item in self.db.prefix_iterator_cf(&cf, prefix.as_bytes()) {
            let Ok((key, bytes)) = item else { continue };
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            if let Ok(record) = serde_json::from_slice::<ChainRecord>(&bytes) {
                if !record.publisher_pubkey.trim().is_empty() {
                    return Some(record.publisher_pubkey);
                }
            }
        }
        None
    }

    /// Atomically consume a namespace-lifecycle nonce. The namespace and
    /// current controller are part of the key so independent namespaces may
    /// use the same monotonic sequence while a replay for one controller is
    /// rejected across restarts.
    pub fn consume_namespace_lifecycle_nonce(
        &self,
        namespace: &str,
        current_controller_pubkey: &str,
        nonce: u64,
    ) -> Result<bool> {
        let key = namespace_lifecycle_nonce_key(namespace, current_controller_pubkey, nonce)?;
        let _guard = self
            .publisher_admission_nonce_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("publisher admission nonce lock is poisoned"))?;
        let cf = self
            .db
            .cf_handle(CF_NAMESPACE_LIFECYCLE_NONCES)
            .context("cf namespace_lifecycle_nonces")?;
        if self.db.get_cf(&cf, key.as_bytes())?.is_some() {
            return Ok(false);
        }
        self.db.put_cf(&cf, key.as_bytes(), b"")?;
        Ok(true)
    }

    /// Atomically record a publisher-admission nonce for this node.
    ///
    /// The payload already binds the nonce to the publisher address, chain,
    /// staking contract, package subject, and expiry. This method makes that
    /// nonce single-use across process restarts. It intentionally stores only
    /// the expiry timestamp, never a signature or package contents.
    ///
    /// `true` means the nonce was newly consumed; `false` means it was already
    /// consumed. Callers must reject the latter rather than treating the
    /// duplicated request as idempotent, because a signed package admission is
    /// security-sensitive.
    pub fn consume_publisher_admission_nonce(
        &self,
        publisher_address: &str,
        nonce: &str,
        expires_at: u64,
    ) -> Result<bool> {
        let publisher = common::canonical_publisher_address(publisher_address);
        let nonce = nonce.trim();
        if publisher.is_empty() {
            anyhow::bail!("publisher address is required when consuming an admission nonce");
        }
        if nonce.is_empty() || nonce.len() > 128 {
            anyhow::bail!("publisher admission nonce must contain 1 to 128 characters");
        }

        let mut key = publisher.into_bytes();
        key.push(0);
        key.extend_from_slice(nonce.as_bytes());

        let _guard = self
            .publisher_admission_nonce_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("publisher admission nonce lock is poisoned"))?;
        let cf = self
            .db
            .cf_handle(CF_PUBLISHER_ADMISSION_NONCES)
            .context("cf publisher_admission_nonces")?;
        if self.db.get_cf(&cf, &key)?.is_some() {
            return Ok(false);
        }

        self.db.put_cf(&cf, key, expires_at.to_be_bytes())?;
        Ok(true)
    }

    /// Persist a finalized transaction before it is handed to the in-memory
    /// block-producer notification channel. The transaction hash is both the
    /// idempotency key and the durable work identifier, so a retry cannot add a
    /// second logical item.
    pub fn enqueue_finalized_work(&self, tx: &Transaction) -> Result<bool> {
        let work_id = transaction_hash(tx);
        let cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        if self.db.get_cf(&cf, work_id.as_bytes())?.is_some() {
            return Ok(false);
        }
        let envelope = FinalizedWorkEnvelope {
            schema_version: 1,
            transaction: tx.clone(),
            enqueued_at: chrono::Utc::now().to_rfc3339(),
            retry_count: 0,
            last_error: None,
            next_attempt_at: None,
        };
        let encoded = serde_json::to_vec(&envelope).context("serialize finalized work")?;
        self.db.put_cf(&cf, work_id.as_bytes(), encoded)?;
        Ok(true)
    }

    fn decode_finalized_work(value: &[u8]) -> Result<FinalizedWorkEnvelope> {
        serde_json::from_slice::<FinalizedWorkEnvelope>(value)
            // Pre-envelope queue entries are retained during migration.  When
            // they fail next, record_finalized_work_failure upgrades them to
            // the current envelope instead of losing retry accountability.
            .or_else(|_| {
                serde_json::from_slice::<Transaction>(value).map(|transaction| {
                    FinalizedWorkEnvelope {
                        schema_version: 1,
                        transaction,
                        enqueued_at: Utc::now().to_rfc3339(),
                        retry_count: 0,
                        last_error: None,
                        next_attempt_at: None,
                    }
                })
            })
            .context("decode finalized work")
    }

    fn is_finalized_work_ready(
        envelope: &FinalizedWorkEnvelope,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        match envelope.next_attempt_at.as_deref() {
            None => Ok(true),
            Some(value) => Ok(DateTime::parse_from_rfc3339(value)
                .context("invalid finalized work next_attempt_at")?
                .with_timezone(&Utc)
                <= now),
        }
    }

    /// Return all persisted work in deterministic RocksDB key order. Entries
    /// remain until acknowledged; production hydration must use
    /// `pending_ready_finalized_work` so a retry delay is honored.
    pub fn pending_finalized_work(&self) -> Result<Vec<(String, Transaction)>> {
        let cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let mut work = Vec::new();
        for item in self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
            let (key, value) = item?;
            let id = String::from_utf8(key.to_vec()).context("invalid finalized work key")?;
            work.push((id, Self::decode_finalized_work(&value)?.transaction));
        }
        Ok(work)
    }

    fn pending_ready_finalized_work_at(
        &self,
        limit: usize,
        now: DateTime<Utc>,
    ) -> Result<Vec<(String, Transaction)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let mut work = Vec::with_capacity(limit);
        for item in self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
            let (key, value) = item?;
            let envelope = Self::decode_finalized_work(&value)?;
            if Self::is_finalized_work_ready(&envelope, now)? {
                let id = String::from_utf8(key.to_vec()).context("invalid finalized work key")?;
                work.push((id, envelope.transaction));
                if work.len() == limit {
                    break;
                }
            }
        }
        Ok(work)
    }

    /// Return at most `limit` work items whose retry deadline has elapsed.
    /// This is the only producer-hydration API and enforces both backoff and
    /// an in-memory capacity bound across restarts.
    pub fn pending_ready_finalized_work(&self, limit: usize) -> Result<Vec<(String, Transaction)>> {
        self.pending_ready_finalized_work_at(limit, Utc::now())
    }

    fn transparency_leaves(&self) -> Result<Vec<String>> {
        let cf = self
            .db
            .cf_handle(CF_TRANSPARENCY_LEAVES)
            .context("cf transparency_leaves")?;
        let mut leaves = Vec::new();
        for item in self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
            let (_, value) = item?;
            let leaf = String::from_utf8(value.to_vec()).context("invalid transparency leaf")?;
            // Validate every persisted hash before producing a proof or root.
            let decoded = hex::decode(&leaf).context("invalid transparency leaf encoding")?;
            if decoded.len() != 32 {
                anyhow::bail!("transparency leaf must be 32 bytes");
            }
            leaves.push(leaf);
        }
        Ok(leaves)
    }

    /// Append an immutable, versioned event and persist its resulting tree head
    /// atomically. Callers must label imports from the legacy chain explicitly;
    /// this method never upgrades them into an install-safe statement.
    pub fn append_transparency_event(
        &self,
        log_id: &str,
        kind: &str,
        subject: &str,
        payload_hash: &str,
        source: &str,
        occurred_at: &str,
    ) -> Result<TreeHead> {
        if log_id.trim().is_empty()
            || kind.trim().is_empty()
            || subject.trim().is_empty()
            || source.trim().is_empty()
        {
            anyhow::bail!("transparency event log_id, kind, subject, and source are required");
        }
        let _guard = self
            .transparency_log_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("transparency log lock poisoned"))?;
        let mut leaves = self.transparency_leaves()?;
        let event = TransparencyEvent {
            version: TRANSPARENCY_LOG_VERSION,
            sequence: leaves.len() as u64,
            kind: kind.to_string(),
            subject: subject.to_string(),
            payload_hash: payload_hash.to_string(),
            source: source.to_string(),
            occurred_at: occurred_at.to_string(),
        };
        let leaf = event.leaf_hash();
        leaves.push(leaf.clone());
        let head = TreeHead {
            version: TRANSPARENCY_LOG_VERSION,
            log_id: log_id.to_string(),
            tree_size: leaves.len() as u64,
            root_hash: tree_root(&leaves)?,
            issued_at: Utc::now().to_rfc3339(),
        };
        let events_cf = self
            .db
            .cf_handle(CF_TRANSPARENCY_EVENTS)
            .context("cf transparency_events")?;
        let leaves_cf = self
            .db
            .cf_handle(CF_TRANSPARENCY_LEAVES)
            .context("cf transparency_leaves")?;
        let heads_cf = self
            .db
            .cf_handle(CF_TRANSPARENCY_HEADS)
            .context("cf transparency_heads")?;
        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(
            &events_cf,
            event.sequence.to_be_bytes(),
            serde_json::to_vec(&event)?,
        );
        batch.put_cf(&leaves_cf, event.sequence.to_be_bytes(), leaf.as_bytes());
        batch.put_cf(
            &heads_cf,
            head.tree_size.to_be_bytes(),
            serde_json::to_vec(&head)?,
        );
        self.db.write(batch)?;
        Ok(head)
    }

    pub fn transparency_tree_head(&self, log_id: &str) -> Result<Option<TreeHead>> {
        let leaves = self.transparency_leaves()?;
        if leaves.is_empty() {
            return Ok(None);
        }
        let heads_cf = self
            .db
            .cf_handle(CF_TRANSPARENCY_HEADS)
            .context("cf transparency_heads")?;
        let Some(bytes) = self
            .db
            .get_cf(&heads_cf, (leaves.len() as u64).to_be_bytes())?
        else {
            anyhow::bail!("transparency tree head missing for persisted leaves");
        };
        let head: TreeHead =
            serde_json::from_slice(&bytes).context("decode transparency tree head")?;
        if head.log_id != log_id
            || head.tree_size != leaves.len() as u64
            || head.root_hash != tree_root(&leaves)?
        {
            anyhow::bail!("persisted transparency tree head does not match canonical leaves");
        }
        Ok(Some(head))
    }

    pub fn transparency_inclusion_proof(
        &self,
        log_id: &str,
        sequence: u64,
    ) -> Result<InclusionProof> {
        let leaves = self.transparency_leaves()?;
        let proof = inclusion_proof(log_id, &leaves, sequence as usize)?;
        if !proof.verify()? {
            anyhow::bail!("generated invalid transparency inclusion proof");
        }
        Ok(proof)
    }

    pub fn transparency_consistency_proof(
        &self,
        log_id: &str,
        first_tree_size: u64,
    ) -> Result<ConsistencyProof> {
        let leaves = self.transparency_leaves()?;
        let proof = consistency_proof(log_id, &leaves, first_tree_size as usize)?;
        if !proof.verify()? {
            anyhow::bail!("generated invalid transparency consistency proof");
        }
        Ok(proof)
    }

    pub fn finalized_work_stats(&self) -> Result<FinalizedWorkStats> {
        let cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let now = Utc::now();
        let mut stats = FinalizedWorkStats::default();
        for item in self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
            let (_, value) = item?;
            let envelope = Self::decode_finalized_work(&value)?;
            stats.active_depth += 1;
            if Self::is_finalized_work_ready(&envelope, now)? {
                stats.ready_depth += 1;
            } else {
                stats.deferred_depth += 1;
            }
            if envelope.retry_count > 0 {
                stats.retrying_depth += 1;
                stats.retry_attempts += u64::from(envelope.retry_count);
            }
            let enqueued_at = DateTime::parse_from_rfc3339(&envelope.enqueued_at)
                .context("invalid finalized work enqueued_at")?
                .with_timezone(&Utc);
            let age = now.signed_duration_since(enqueued_at).num_seconds().max(0) as u64;
            stats.oldest_age_seconds = Some(stats.oldest_age_seconds.unwrap_or(0).max(age));
        }
        Ok(stats)
    }

    pub fn finalized_work_depths(&self) -> Result<(u64, u64)> {
        let active = self.finalized_work_stats()?.active_depth;
        let dead_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK_DEAD_LETTER)
            .context("cf finalized_work_dead_letter")?;
        let dead = self
            .db
            .iterator_cf(&dead_cf, rocksdb::IteratorMode::Start)
            .count() as u64;
        Ok((active, dead))
    }

    /// Acknowledge only after the transaction has been included in durable
    /// finalized block history. Failed production leaves the work item intact.
    pub fn acknowledge_finalized_work(&self, work_id: &str) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        self.db.delete_cf(&cf, work_id.as_bytes())?;
        Ok(())
    }

    /// Move unrecoverable work to a durable dead-letter record. This never
    /// discards the original transaction or its failure reason.
    pub fn dead_letter_finalized_work(&self, work_id: &str, reason: &str) -> Result<bool> {
        let work_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let dead_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK_DEAD_LETTER)
            .context("cf finalized_work_dead_letter")?;
        let Some(tx) = self.db.get_cf(&work_cf, work_id.as_bytes())? else {
            return Ok(false);
        };
        let envelope = Self::decode_finalized_work(&tx)?;
        let record = serde_json::json!({
            "work_id": work_id,
            "reason": reason,
            "dead_lettered_at": chrono::Utc::now().to_rfc3339(),
            "enqueued_at": envelope.enqueued_at,
            "retry_count": envelope.retry_count,
            "last_error": envelope.last_error,
            "transaction": envelope.transaction,
        });
        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(&dead_cf, work_id.as_bytes(), serde_json::to_vec(&record)?);
        batch.delete_cf(&work_cf, work_id.as_bytes());
        self.db.write(batch)?;
        Ok(true)
    }

    /// Record a failed delivery and preserve the distinction between a queued
    /// retry and an item that was never durable work.
    pub fn record_finalized_work_failure(
        &self,
        work_id: &str,
        error: &str,
    ) -> Result<FinalizedWorkFailureOutcome> {
        let cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let Some(bytes) = self.db.get_cf(&cf, work_id.as_bytes())? else {
            return Ok(FinalizedWorkFailureOutcome::Missing);
        };
        let mut envelope = Self::decode_finalized_work(&bytes)?;
        envelope.retry_count = envelope.retry_count.saturating_add(1);
        envelope.last_error = Some(error.to_string());
        if envelope.retry_count > MAX_FINALIZED_WORK_RETRIES {
            // The final failed attempt has not been written to the active
            // envelope yet. Write this in-memory state directly into the DLQ
            // record in the same batch that removes active work, otherwise
            // operators would see an off-by-one retry count.
            let dead_cf = self
                .db
                .cf_handle(CF_FINALIZED_WORK_DEAD_LETTER)
                .context("cf finalized_work_dead_letter")?;
            let record = serde_json::json!({
                "work_id": work_id,
                "reason": error,
                "dead_lettered_at": Utc::now().to_rfc3339(),
                "enqueued_at": envelope.enqueued_at,
                "retry_count": envelope.retry_count,
                "last_error": envelope.last_error,
                "transaction": envelope.transaction,
            });
            let mut batch = rocksdb::WriteBatch::default();
            batch.put_cf(&dead_cf, work_id.as_bytes(), serde_json::to_vec(&record)?);
            batch.delete_cf(&cf, work_id.as_bytes());
            self.db.write(batch)?;
            return Ok(FinalizedWorkFailureOutcome::DeadLettered);
        }
        let delay_secs = (1u64 << envelope.retry_count.min(8)).min(300);
        envelope.next_attempt_at =
            Some((chrono::Utc::now() + chrono::Duration::seconds(delay_secs as i64)).to_rfc3339());
        self.db
            .put_cf(&cf, work_id.as_bytes(), serde_json::to_vec(&envelope)?)?;
        Ok(FinalizedWorkFailureOutcome::Retrying)
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
                        old_pubkey, nonce, ..
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
                            return Some(block.header.timestamp);
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
                Box::new(self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start))
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

/// Result of `insert_block_with_outcome`.
#[derive(Clone, Debug)]
pub struct BlockInsertOutcome {
    /// Hash of the inserted block.
    pub hash: String,
    /// Hash of a *different* block that previously occupied the same height,
    /// if any. `Some(_)` means a fork replaced finalized history and the
    /// caller should record a reorg event.
    pub replaced_hash: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ChainStats {
    pub tip_height: u64,
    pub tip_hash: String,
    pub package_count: usize,
    pub block_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use common::{
        merkle_root, AnalysisBundleRefs, Block, BlockHeader, ChainRecord, DeterministicRiskSummary,
        NamespaceLifecycleAction, NamespaceLifecycleEvent, PackageId, Transaction,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;

    fn publish_record(name: &str) -> ChainRecord {
        ChainRecord {
            id: PackageId::new("npm", name, "1.0.0"),
            content_hash: common::sha256_hex(name.as_bytes()),
            ipfs_cid: "bafytestcid".into(),
            publisher_pubkey: "publisher-pubkey".into(),
            block_hash: String::new(),
            published_at: Utc::now(),
            validator_signatures: vec![],
            status: PackageStatus::Verified,
            shielded: false,
            key_bundle: None,
            pgp_fingerprint: None,
            findings: vec![],
            analysis_bundles: AnalysisBundleRefs::default(),
            evidence_digest: String::new(),
            deterministic_risk: DeterministicRiskSummary::default(),
            access_count: 0,
            last_accessed: None,
            threshold: 0,
            publisher_pubkeys: vec![],
            manifest: None,
            analysis_committee_size: 0,
            analysis_selection_seed: String::new(),
            analysis_effective_size: 0,
            analysis_mandatory_contributed: 0,
            analysis_committee: None,
        }
    }

    fn block(height: u64, prev_hash: &str, proposer: &str, packages: &[&str]) -> Block {
        let transactions: Vec<Transaction> = packages
            .iter()
            .map(|name| Transaction::Publish(publish_record(name)))
            .collect();
        block_with_transactions(height, prev_hash, proposer, transactions)
    }

    fn block_with_transactions(
        height: u64,
        prev_hash: &str,
        proposer: &str,
        transactions: Vec<Transaction>,
    ) -> Block {
        Block {
            header: BlockHeader {
                height,
                prev_hash: prev_hash.to_string(),
                merkle_root: merkle_root(&transactions),
                proposer_id: proposer.into(),
                timestamp: Utc::now(),
                validator_set_hash: "test".into(),
                vrf_output: None,
                vrf_proof: None,
            },
            transactions,
            pbft_signatures: vec![],
        }
    }

    #[test]
    fn insert_block_with_outcome_detects_replacement() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("tip hash");

        let block_a = block(1, &genesis_hash, "node-a", &["pkg-a"]);
        let outcome_a = store.insert_block_with_outcome(&block_a).expect("insert a");
        assert!(outcome_a.replaced_hash.is_none(), "first insert is no fork");

        // Re-inserting the identical block is not a replacement.
        let outcome_same = store
            .insert_block_with_outcome(&block_a)
            .expect("re-insert");
        assert!(outcome_same.replaced_hash.is_none());

        // A different block at the same height is a fork replacement.
        let block_b = block(1, &genesis_hash, "node-b", &["pkg-b"]);
        let outcome_b = store.insert_block_with_outcome(&block_b).expect("insert b");
        assert_eq!(outcome_b.replaced_hash, Some(block_a.hash()));
        assert_eq!(store.tip_hash().expect("tip"), block_b.hash());
    }

    #[test]
    fn rollback_and_rebuild_drop_abandoned_packages() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("tip hash");

        let block1 = block(1, &genesis_hash, "node-a", &["kept"]);
        store.insert_block(&block1).expect("insert 1");
        let block2 = block(2, &block1.hash(), "node-a", &["abandoned"]);
        store.insert_block(&block2).expect("insert 2");

        assert!(store
            .get_package("npm:abandoned@1.0.0")
            .expect("get")
            .is_some());

        // Rewind to height 1: block 2 is abandoned but stays readable by hash.
        let abandoned = store.rollback_to_height(1).expect("rollback");
        assert_eq!(abandoned, vec![block2.hash()]);
        assert_eq!(store.tip_height().expect("tip"), 1);
        assert!(store
            .get_block_by_hash(&block2.hash())
            .expect("by hash")
            .is_some());

        // Rebuilding the package index removes records from abandoned blocks.
        store.rebuild_package_index().expect("rebuild");
        assert!(store.get_package("npm:kept@1.0.0").expect("get").is_some());
        assert!(store
            .get_package("npm:abandoned@1.0.0")
            .expect("get")
            .is_none());
    }

    #[test]
    fn publisher_admission_nonce_is_single_use_and_survives_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let publisher = "0x1111111111111111111111111111111111111111";
        {
            let store = ChainStore::open(dir.path()).expect("open");
            assert!(store
                .consume_publisher_admission_nonce(publisher, "nonce-1", 1_800_000_000)
                .expect("consume"));
            assert!(!store
                .consume_publisher_admission_nonce(publisher, "nonce-1", 1_800_000_000)
                .expect("duplicate"));
        }

        let reopened = ChainStore::open(dir.path()).expect("reopen");
        assert!(!reopened
            .consume_publisher_admission_nonce(publisher, "nonce-1", 1_800_000_000)
            .expect("duplicate after restart"));
        assert!(reopened
            .consume_publisher_admission_nonce(publisher, "nonce-2", 1_800_000_000)
            .expect("distinct nonce"));
    }

    #[test]
    fn namespace_lifecycle_nonce_is_single_use_and_survives_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let namespace = "npm:lifecycle-nonce@";
        let controller = "a".repeat(64);
        {
            let store = ChainStore::open(dir.path()).expect("open");
            assert!(store
                .consume_namespace_lifecycle_nonce(namespace, &controller, 7)
                .expect("consume"));
            assert!(!store
                .consume_namespace_lifecycle_nonce(namespace, &controller, 7)
                .expect("duplicate"));
        }
        let reopened = ChainStore::open(dir.path()).expect("reopen");
        assert!(!reopened
            .consume_namespace_lifecycle_nonce(namespace, &controller, 7)
            .expect("duplicate after restart"));
        assert!(reopened
            .consume_namespace_lifecycle_nonce(namespace, &controller, 8)
            .expect("distinct nonce"));
    }

    #[test]
    fn namespace_lifecycle_controller_survives_restart_and_rebuild() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = SigningKey::generate(&mut OsRng);
        let next = SigningKey::generate(&mut OsRng);
        let current_pubkey = hex::encode(current.verifying_key().as_bytes());
        let next_pubkey = hex::encode(next.verifying_key().as_bytes());
        let namespace = "npm:lifecycle-package@";
        let now = Utc::now();

        let mut event = NamespaceLifecycleEvent {
            schema_version: common::OWNERSHIP_SCHEMA_VERSION,
            action: NamespaceLifecycleAction::Transfer,
            namespace: namespace.into(),
            current_controller_ed25519_pubkey: current_pubkey.clone(),
            next_controller_ed25519_pubkey: next_pubkey.clone(),
            nonce: 1,
            issued_at: now,
            expires_at: now + Duration::hours(1),
            current_controller_signature: String::new(),
            next_controller_signature: String::new(),
            authority_proof_digest: String::new(),
        };
        let message = event.signing_message();
        event.current_controller_signature =
            hex::encode(current.sign(message.as_bytes()).to_bytes());
        event.next_controller_signature = hex::encode(next.sign(message.as_bytes()).to_bytes());

        {
            let store = ChainStore::open(dir.path()).expect("open");
            let genesis_hash = store.tip_hash().expect("genesis hash");
            let mut record = publish_record("lifecycle-package");
            record.publisher_pubkey = current_pubkey.clone();
            let publish = block_with_transactions(
                1,
                &genesis_hash,
                "node-a",
                vec![Transaction::Publish(record)],
            );
            store.insert_block(&publish).expect("publish block");
            assert_eq!(store.publisher_for_prefix(namespace), Some(current_pubkey));

            let transfer = block_with_transactions(
                2,
                &publish.hash(),
                "node-a",
                vec![Transaction::NamespaceLifecycle(event)],
            );
            store.insert_block(&transfer).expect("transfer block");
            assert_eq!(
                store.publisher_for_prefix(namespace),
                Some(next_pubkey.clone())
            );
            store.rebuild_package_index().expect("rebuild");
            assert_eq!(
                store.publisher_for_prefix(namespace),
                Some(next_pubkey.clone())
            );
        }

        let reopened = ChainStore::open(dir.path()).expect("reopen");
        assert_eq!(reopened.publisher_for_prefix(namespace), Some(next_pubkey));
    }

    #[test]
    fn recovery_event_never_changes_a_controller_without_an_authority_adapter() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = SigningKey::generate(&mut OsRng);
        let next = SigningKey::generate(&mut OsRng);
        let current_pubkey = hex::encode(current.verifying_key().as_bytes());
        let next_pubkey = hex::encode(next.verifying_key().as_bytes());
        let namespace = "npm:recovery-package@";
        let now = Utc::now();
        let mut event = NamespaceLifecycleEvent {
            schema_version: common::OWNERSHIP_SCHEMA_VERSION,
            action: NamespaceLifecycleAction::Recover,
            namespace: namespace.into(),
            current_controller_ed25519_pubkey: current_pubkey.clone(),
            next_controller_ed25519_pubkey: next_pubkey,
            nonce: 1,
            issued_at: now,
            expires_at: now + Duration::hours(1),
            current_controller_signature: String::new(),
            next_controller_signature: String::new(),
            authority_proof_digest: "a".repeat(64),
        };
        event.next_controller_signature =
            hex::encode(next.sign(event.signing_message().as_bytes()).to_bytes());

        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let mut record = publish_record("recovery-package");
        record.publisher_pubkey = current_pubkey.clone();
        let publish = block_with_transactions(
            1,
            &genesis_hash,
            "node-a",
            vec![Transaction::Publish(record)],
        );
        store.insert_block(&publish).expect("publish block");
        let recovery = block_with_transactions(
            2,
            &publish.hash(),
            "node-a",
            vec![Transaction::NamespaceLifecycle(event)],
        );
        store
            .insert_block(&recovery)
            .expect("recovery block retained");

        assert_eq!(store.publisher_for_prefix(namespace), Some(current_pubkey));
    }

    #[test]
    fn expired_lifecycle_event_never_changes_a_controller_during_block_replay() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = SigningKey::generate(&mut OsRng);
        let next = SigningKey::generate(&mut OsRng);
        let current_pubkey = hex::encode(current.verifying_key().as_bytes());
        let next_pubkey = hex::encode(next.verifying_key().as_bytes());
        let namespace = "npm:expired-lifecycle-package@";
        let now = Utc::now();
        let mut event = NamespaceLifecycleEvent {
            schema_version: common::OWNERSHIP_SCHEMA_VERSION,
            action: NamespaceLifecycleAction::Transfer,
            namespace: namespace.into(),
            current_controller_ed25519_pubkey: current_pubkey.clone(),
            next_controller_ed25519_pubkey: next_pubkey,
            nonce: 1,
            issued_at: now - Duration::hours(2),
            expires_at: now - Duration::hours(1),
            current_controller_signature: String::new(),
            next_controller_signature: String::new(),
            authority_proof_digest: String::new(),
        };
        let message = event.signing_message();
        event.current_controller_signature =
            hex::encode(current.sign(message.as_bytes()).to_bytes());
        event.next_controller_signature = hex::encode(next.sign(message.as_bytes()).to_bytes());

        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let mut record = publish_record("expired-lifecycle-package");
        record.publisher_pubkey = current_pubkey.clone();
        let publish = block_with_transactions(
            1,
            &genesis_hash,
            "node-a",
            vec![Transaction::Publish(record)],
        );
        store.insert_block(&publish).expect("publish block");
        let expired = block_with_transactions(
            2,
            &publish.hash(),
            "node-a",
            vec![Transaction::NamespaceLifecycle(event)],
        );
        store
            .insert_block(&expired)
            .expect("expired block retained");

        assert_eq!(store.publisher_for_prefix(namespace), Some(current_pubkey));
    }

    #[test]
    fn duplicate_lifecycle_nonce_in_one_block_is_applied_once_and_rebuild_is_consistent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = SigningKey::generate(&mut OsRng);
        let first_next = SigningKey::generate(&mut OsRng);
        let conflicting_next = SigningKey::generate(&mut OsRng);
        let current_pubkey = hex::encode(current.verifying_key().as_bytes());
        let first_next_pubkey = hex::encode(first_next.verifying_key().as_bytes());
        let conflicting_next_pubkey = hex::encode(conflicting_next.verifying_key().as_bytes());
        let namespace = "npm:duplicate-lifecycle-package@";
        let now = Utc::now();
        let mut first = NamespaceLifecycleEvent {
            schema_version: common::OWNERSHIP_SCHEMA_VERSION,
            action: NamespaceLifecycleAction::Transfer,
            namespace: namespace.into(),
            current_controller_ed25519_pubkey: current_pubkey.clone(),
            next_controller_ed25519_pubkey: first_next_pubkey.clone(),
            nonce: 1,
            issued_at: now,
            expires_at: now + Duration::hours(1),
            current_controller_signature: String::new(),
            next_controller_signature: String::new(),
            authority_proof_digest: String::new(),
        };
        let first_message = first.signing_message();
        first.current_controller_signature =
            hex::encode(current.sign(first_message.as_bytes()).to_bytes());
        first.next_controller_signature =
            hex::encode(first_next.sign(first_message.as_bytes()).to_bytes());
        let mut conflicting = NamespaceLifecycleEvent {
            next_controller_ed25519_pubkey: conflicting_next_pubkey,
            ..first.clone()
        };
        let conflicting_message = conflicting.signing_message();
        conflicting.current_controller_signature =
            hex::encode(current.sign(conflicting_message.as_bytes()).to_bytes());
        conflicting.next_controller_signature = hex::encode(
            conflicting_next
                .sign(conflicting_message.as_bytes())
                .to_bytes(),
        );

        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let mut record = publish_record("duplicate-lifecycle-package");
        record.publisher_pubkey = current_pubkey;
        let publish = block_with_transactions(
            1,
            &genesis_hash,
            "node-a",
            vec![Transaction::Publish(record)],
        );
        store.insert_block(&publish).expect("publish block");
        let lifecycle_block = block_with_transactions(
            2,
            &publish.hash(),
            "node-a",
            vec![
                Transaction::NamespaceLifecycle(first),
                Transaction::NamespaceLifecycle(conflicting),
            ],
        );
        store
            .insert_block(&lifecycle_block)
            .expect("lifecycle block");
        assert_eq!(
            store.publisher_for_prefix(namespace),
            Some(first_next_pubkey.clone())
        );

        store.rebuild_package_index().expect("rebuild");
        assert_eq!(
            store.publisher_for_prefix(namespace),
            Some(first_next_pubkey)
        );
    }

    #[test]
    fn finalized_work_is_idempotent_and_survives_restart_until_acknowledged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tx = Transaction::Revoke {
            package_canonical: "npm:durable-queue@1.0.0".into(),
            reason: "test".into(),
            revoked_by: "validator-1".into(),
            evidence_hash: "evidence".into(),
        };
        let work_id = common::transaction_hash(&tx);
        {
            let store = ChainStore::open(dir.path()).expect("open");
            assert!(store.enqueue_finalized_work(&tx).expect("enqueue"));
            assert!(!store.enqueue_finalized_work(&tx).expect("idempotent"));
        }

        let reopened = ChainStore::open(dir.path()).expect("reopen");
        let pending = reopened.pending_finalized_work().expect("load work");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, work_id);
        reopened
            .acknowledge_finalized_work(&work_id)
            .expect("acknowledge");
        assert!(reopened.pending_finalized_work().expect("empty").is_empty());
    }

    #[test]
    fn finalized_work_retry_is_deferred_then_promoted_to_dead_letter() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let tx = Transaction::Revoke {
            package_canonical: "npm:retryable@1.0.0".into(),
            reason: "production failed".into(),
            revoked_by: "validator-1".into(),
            evidence_hash: "evidence".into(),
        };
        let work_id = transaction_hash(&tx);
        store.enqueue_finalized_work(&tx).expect("enqueue");

        assert_eq!(
            store
                .record_finalized_work_failure(&work_id, "first failure")
                .expect("record first failure"),
            FinalizedWorkFailureOutcome::Retrying
        );
        // The first retry has a two-second backoff and must not be picked up
        // by an immediate producer tick.
        assert!(store
            .pending_ready_finalized_work_at(10, Utc::now())
            .expect("ready work")
            .is_empty());
        assert_eq!(
            store
                .pending_ready_finalized_work_at(10, Utc::now() + chrono::Duration::seconds(3))
                .expect("future ready work")
                .len(),
            1
        );

        for attempt in 2..=MAX_FINALIZED_WORK_RETRIES {
            assert_eq!(
                store
                    .record_finalized_work_failure(&work_id, &format!("failure {attempt}"))
                    .expect("record retry"),
                FinalizedWorkFailureOutcome::Retrying
            );
        }
        assert_eq!(
            store
                .record_finalized_work_failure(&work_id, "retry budget exhausted")
                .expect("promote dead letter"),
            FinalizedWorkFailureOutcome::DeadLettered
        );

        let (active, dead) = store.finalized_work_depths().expect("depths");
        assert_eq!(active, 0);
        assert_eq!(dead, 1);
        let dead_cf = store
            .db
            .cf_handle(CF_FINALIZED_WORK_DEAD_LETTER)
            .expect("dead letter cf");
        let record = store
            .db
            .get_cf(&dead_cf, work_id.as_bytes())
            .expect("read dead letter")
            .expect("dead letter record");
        let value: serde_json::Value = serde_json::from_slice(&record).expect("decode record");
        assert_eq!(value["retry_count"], MAX_FINALIZED_WORK_RETRIES + 1);
        let transaction: Transaction = serde_json::from_value(value["transaction"].clone())
            .expect("dead letter preserves a deserializable transaction");
        assert!(matches!(
            transaction,
            Transaction::Revoke { package_canonical, .. }
                if package_canonical == "npm:retryable@1.0.0"
        ));
    }

    #[test]
    fn finalized_work_survives_restart_and_only_block_insertion_acknowledges_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = Transaction::Revoke {
            package_canonical: "npm:no-drop-first@1.0.0".into(),
            reason: "test".into(),
            revoked_by: "validator-1".into(),
            evidence_hash: "evidence-1".into(),
        };
        let second = Transaction::Revoke {
            package_canonical: "npm:no-drop-second@1.0.0".into(),
            reason: "test".into(),
            revoked_by: "validator-1".into(),
            evidence_hash: "evidence-2".into(),
        };
        let first_id = transaction_hash(&first);
        let second_id = transaction_hash(&second);

        {
            let store = ChainStore::open(dir.path()).expect("open");
            store.enqueue_finalized_work(&first).expect("enqueue first");
            store
                .enqueue_finalized_work(&second)
                .expect("enqueue second");
        } // Simulated process crash/restart boundary: no in-memory ack exists.

        let store = ChainStore::open(dir.path()).expect("reopen");
        assert_eq!(
            store
                .pending_ready_finalized_work(1)
                .expect("bounded hydration")
                .len(),
            1,
            "restart hydration must obey its requested capacity"
        );
        assert_eq!(store.finalized_work_stats().expect("stats").active_depth, 2);

        let block = block_with_transactions(
            1,
            &store.tip_hash().expect("tip hash"),
            "node-a",
            vec![first.clone()],
        );
        store.insert_block(&block).expect("insert canonical block");
        drop(store);

        let reopened = ChainStore::open(dir.path()).expect("reopen after block");
        let pending = reopened.pending_finalized_work().expect("pending work");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, second_id);
        assert_ne!(pending[0].0, first_id);
    }

    #[test]
    fn transparency_log_persists_and_proves_append_only_history() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_id = "creg-test-log";
        let first_head = {
            let store = ChainStore::open(dir.path()).expect("open");
            store
                .append_transparency_event(
                    log_id,
                    "package-finalized",
                    "npm:log-test@1.0.0",
                    &common::sha256_hex(b"first payload"),
                    "test-fixture",
                    "2026-07-14T00:00:00Z",
                )
                .expect("append first")
        };

        let store = ChainStore::open(dir.path()).expect("restart");
        let second_head = store
            .append_transparency_event(
                log_id,
                "revocation",
                "npm:log-test@1.0.0",
                &common::sha256_hex(b"second payload"),
                "test-fixture",
                "2026-07-14T00:01:00Z",
            )
            .expect("append second");
        assert_eq!(first_head.tree_size, 1);
        assert_eq!(second_head.tree_size, 2);
        assert_ne!(first_head.root_hash, second_head.root_hash);
        assert_eq!(
            store
                .transparency_tree_head(log_id)
                .expect("latest head")
                .expect("head")
                .root_hash,
            second_head.root_hash
        );
        assert!(store
            .transparency_inclusion_proof(log_id, 0)
            .expect("inclusion proof")
            .verify()
            .expect("verify inclusion"));
        assert!(store
            .transparency_consistency_proof(log_id, 1)
            .expect("consistency proof")
            .verify()
            .expect("verify consistency"));
    }
}
