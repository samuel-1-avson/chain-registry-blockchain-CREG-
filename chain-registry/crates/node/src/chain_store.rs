// crates/node/src/chain_store.rs
// Persistent storage for the blockchain using RocksDB.
// Stores blocks by height and by hash, and a package index by canonical ID.
// Replaced sled for better write amplification, compaction, and snapshot support.
#![allow(dead_code)] // Reorg and diagnostic methods shared across node binary targets.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use common::{
    consistency_proof, inclusion_proof, transaction_hash, tree_root, Block, ChainRecord,
    ConsistencyProof, InclusionProof, PackageStatus, Transaction, TransparencyEvent, TreeHead,
    TRANSPARENCY_LOG_VERSION,
};
use rocksdb::{checkpoint::Checkpoint, ColumnFamilyDescriptor, Options, WriteOptions, DB};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FinalizedWorkEnvelope {
    schema_version: u8,
    transaction: Transaction,
    enqueued_at: String,
    /// Canonical tip when this node first durably accepted the work. Legacy
    /// v1 envelopes lack this field and are conservatively rehydrated at the
    /// current tip.
    #[serde(default)]
    submitted_at_height: Option<u64>,
    retry_count: u32,
    last_error: Option<String>,
    next_attempt_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReorgJournalEntry {
    height: u64,
    hash: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReorgJournal {
    schema_version: u8,
    common_ancestor: u64,
    ancestor_hash: String,
    old_branch: Vec<ReorgJournalEntry>,
    /// Exact pre-reorg authority history. `None` records that the history file
    /// did not exist, which is distinct from an empty but present history.
    validator_history: Option<Vec<u8>>,
}

const CF_BLOCKS_BY_HASH: &str = "blocks_by_hash";
const CF_BLOCKS_BY_HEIGHT: &str = "blocks_by_height";
const CF_PACKAGES: &str = "packages";
const CF_NAMESPACE_CONTROLLERS: &str = "namespace_controllers";
const CF_NAMESPACE_LIFECYCLE_NONCES: &str = "namespace_lifecycle_nonces";
const CF_APPLIED_NAMESPACE_LIFECYCLE_NONCES: &str = "applied_namespace_lifecycle_nonces";
const CF_APPLIED_PUBLISHER_AUTHORIZATION_NONCES: &str = "applied_publisher_authorization_nonces";
const CF_NAMESPACE_DELEGATIONS: &str = "namespace_delegations";
const CF_PUBLISHER_ADMISSION_NONCES: &str = "publisher_admission_nonces";
const CF_PUBLISHER_ROTATION: &str = "publisher_rotation";
const CF_FINALIZED_WORK: &str = "finalized_work";
const CF_FINALIZED_WORK_DEAD_LETTER: &str = "finalized_work_dead_letter";
const CF_TRANSPARENCY_EVENTS: &str = "transparency_events";
const CF_TRANSPARENCY_LEAVES: &str = "transparency_leaves";
const CF_TRANSPARENCY_HEADS: &str = "transparency_heads";
const CF_REORG_JOURNAL: &str = "reorg_journal";
const CF_PUBLISHER_INDEX: &str = "publisher_index";
const PUB_IDX_STATS_PREFIX: &str = "stats\0";
const PUB_IDX_PKG_PREFIX: &str = "pkg\0";
const PUB_IDX_TIP_KEY: &[u8] = b"meta\0indexed_tip";
const REORG_JOURNAL_KEY: &[u8] = b"active";
const REORG_FINALIZED_WORK_PREFIX: &[u8] = b"finalized_work\0";
const MAX_FINALIZED_WORK_RETRIES: u32 = 5;
const PBFT_JOURNAL_BINDING_KEY: &[u8] = b"meta/pbft-journal-binding-v1";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PublisherRotationIndex {
    nonce: u64,
    #[serde(default)]
    timestamp: Option<DateTime<Utc>>,
}

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

fn is_reorg_requeueable_finalized_work(transaction: &Transaction) -> bool {
    matches!(
        transaction,
        Transaction::Publish(_)
            | Transaction::PublishAuthorizedV1 { .. }
            | Transaction::Revoke { .. }
            | Transaction::RevokeV2 { .. }
            | Transaction::RotatePublisherKey { .. }
            | Transaction::NamespaceLifecycle(_)
            | Transaction::NamespaceLifecycleV2(_)
    )
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
    canonical_genesis: Arc<Block>,
    /// Parent directory of `chain.rocksdb`. Reorg checkpoints must be created
    /// on this filesystem because RocksDB uses hard links for immutable SSTs.
    data_dir: Arc<PathBuf>,
    /// Independent durable PBFT signing locks. This database is deliberately
    /// outside `chain.rocksdb` so reorg recovery cannot roll votes backward.
    pbft_safety_journal: Arc<OnceLock<Arc<crate::pbft_safety::PbftSigningJournal>>>,
    runtime_events: Arc<OnceLock<crate::events::RuntimeEventEmitter>>,
    namespace_consensus:
        Arc<std::sync::RwLock<crate::namespace_consensus::NamespaceConsensusConfig>>,
    /// RocksDB does not provide a conditional put through this wrapper. This
    /// lock makes check-and-record atomic for concurrent admissions handled by
    /// this node; the durable record then protects restarts. Cross-node replay
    /// prevention still requires the target canonical event log.
    publisher_admission_nonce_lock: Arc<std::sync::Mutex<()>>,
    /// Serializes append/root calculation. RocksDB batches make the event,
    /// leaf, and head durable together; this lock prevents two local writers
    /// from choosing the same sequence number.
    transparency_log_lock: Arc<std::sync::Mutex<()>>,
    /// Serializes every RocksDB mutation against reorg journal creation,
    /// adoption, recovery, and completion.
    ///
    /// Lock order is: caller-owned `NodeState` lock -> this store mutation
    /// lock -> validator-set-history lock. Code holding this lock must never
    /// acquire `NodeState`.
    store_mutation_lock: Arc<StoreMutationMutex>,
    read_only: bool,
    #[cfg(test)]
    reorg_begin_test_hook: Arc<Mutex<Option<ReorgBeginTestHook>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoreMutationOwner {
    Idle,
    Ordinary,
    Reorg,
}

#[derive(Debug)]
struct StoreMutationMutex {
    owner: Mutex<StoreMutationOwner>,
    available: Condvar,
}

impl Default for StoreMutationMutex {
    fn default() -> Self {
        Self {
            owner: Mutex::new(StoreMutationOwner::Idle),
            available: Condvar::new(),
        }
    }
}

struct StoreMutationGuard {
    mutex: Arc<StoreMutationMutex>,
    owner: StoreMutationOwner,
}

impl StoreMutationGuard {
    fn ordinary(mutex: &Arc<StoreMutationMutex>) -> Result<Self> {
        let mut owner = mutex
            .owner
            .lock()
            .map_err(|_| anyhow::anyhow!("store mutation lock is poisoned"))?;
        loop {
            match *owner {
                StoreMutationOwner::Idle => {
                    *owner = StoreMutationOwner::Ordinary;
                    return Ok(Self {
                        mutex: Arc::clone(mutex),
                        owner: StoreMutationOwner::Ordinary,
                    });
                }
                StoreMutationOwner::Ordinary => {
                    owner = mutex
                        .available
                        .wait(owner)
                        .map_err(|_| anyhow::anyhow!("store mutation lock is poisoned"))?;
                }
                StoreMutationOwner::Reorg => {
                    anyhow::bail!(
                        "chain is in reorg-recovery maintenance; canonical mutations are unavailable"
                    );
                }
            }
        }
    }

    fn reorg(mutex: &Arc<StoreMutationMutex>) -> Result<Self> {
        let mut owner = mutex
            .owner
            .lock()
            .map_err(|_| anyhow::anyhow!("store mutation lock is poisoned"))?;
        loop {
            match *owner {
                StoreMutationOwner::Idle => {
                    *owner = StoreMutationOwner::Reorg;
                    return Ok(Self {
                        mutex: Arc::clone(mutex),
                        owner: StoreMutationOwner::Reorg,
                    });
                }
                StoreMutationOwner::Ordinary => {
                    owner = mutex
                        .available
                        .wait(owner)
                        .map_err(|_| anyhow::anyhow!("store mutation lock is poisoned"))?;
                }
                StoreMutationOwner::Reorg => {
                    anyhow::bail!("another live reorg mutation guard is already active");
                }
            }
        }
    }
}

impl Drop for StoreMutationGuard {
    fn drop(&mut self) {
        let mut current = self
            .mutex
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert_eq!(*current, self.owner);
        if *current == self.owner {
            *current = StoreMutationOwner::Idle;
            self.mutex.available.notify_all();
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
struct ReorgBeginTestHook {
    lock_acquired: Arc<std::sync::Barrier>,
    resume: Arc<std::sync::Barrier>,
}

/// Capability authorizing the canonical writes that are part of one active
/// reorganization. Public mutation entry points reject all writes while the
/// durable journal exists; only code holding this DB-bound guard may apply or
/// commit the preflighted candidate branch.
pub(crate) struct ReorgMutationGuard {
    db: Arc<DB>,
    mutation: StoreMutationGuard,
}

fn finalized_work_transaction_summary(transaction: &Transaction) -> serde_json::Value {
    let (kind, canonical, publisher, nonce, expires_at, chain_id) = match transaction {
        Transaction::Publish(record) => (
            "publish",
            Some(record.id.canonical()),
            Some(record.publisher_pubkey.clone()),
            None,
            None,
            None,
        ),
        Transaction::PublishAuthorizedV1 {
            record,
            authorization,
            ..
        } => (
            "publish_authorized_v1",
            Some(record.id.canonical()),
            Some(authorization.publisher_address.clone()),
            Some(authorization.nonce.clone()),
            Some(authorization.expires_at),
            Some(authorization.chain_id.clone()),
        ),
        Transaction::Revoke {
            package_canonical, ..
        } => (
            "revoke",
            Some(package_canonical.clone()),
            None,
            None,
            None,
            None,
        ),
        Transaction::RevokeV2 { request, .. } => (
            "revoke_v2",
            Some(request.package_canonical.clone()),
            Some(request.revoker_pubkey.clone()),
            None,
            None,
            Some(request.chain_id.clone()),
        ),
        _ => ("other", None, None, None, None, None),
    };
    serde_json::json!({
        "type": kind,
        "canonical": canonical,
        "publisher": publisher,
        "publisher_nonce": nonce,
        "publisher_authorization_expires_at": expires_at,
        "authorization_chain_id": chain_id,
    })
}

impl ChainStore {
    /// Attach live notifications after the canonical startup replay completes.
    pub fn attach_event_bus(&self, bus: crate::events::EventBus) -> Result<()> {
        self.runtime_events
            .set(crate::events::RuntimeEventEmitter::new(bus))
            .map_err(|_| anyhow::anyhow!("runtime event bus was already attached"))
    }

    pub(crate) fn emit_registry_event(&self, event: crate::events::RegistryEvent) {
        if let Some(emitter) = self.runtime_events.get() {
            emitter.emit(event);
        }
    }

    pub(crate) fn emit_committed_events(&self, block: &Block) {
        let Some(emitter) = self.runtime_events.get() else {
            return;
        };
        if block.header.height == 0 || self.tip_height().ok() != Some(block.header.height) {
            return;
        }
        let hash = block.hash();
        match self.get_block_by_height(block.header.height) {
            Ok(Some(canonical)) if canonical.hash() == hash => {}
            _ => {
                tracing::warn!(
                    height = block.header.height,
                    "Skipped event for a block absent from the canonical store"
                );
                return;
            }
        }
        if !emitter.first_notification_for_commit(&hash) {
            return;
        }
        emitter.emit(
            crate::events::RegistryEvent::block_produced(
                block.header.height,
                &hash,
                block.transactions.len(),
            )
            .with_committed_block(block),
        );
        for transaction in &block.transactions {
            let event = if let Some(record) = transaction.published_record() {
                let validator_count = record
                    .validator_signatures
                    .iter()
                    .map(|signature| &signature.validator_id)
                    .collect::<HashSet<_>>()
                    .len();
                crate::events::RegistryEvent::package_verified(
                    &record.id.canonical(),
                    &hash,
                    validator_count,
                )
            } else if let Some((package_canonical, reason, revoked_by, _)) =
                transaction.revocation_target()
            {
                match self.get_package(package_canonical) {
                    Ok(Some(_)) => crate::events::RegistryEvent::package_revoked(
                        package_canonical,
                        reason,
                        revoked_by,
                    ),
                    Ok(None) => {
                        crate::events::RegistryEvent::package_rejected(package_canonical, reason)
                    }
                    Err(error) => {
                        tracing::warn!(canonical = %package_canonical, %error, "Could not classify committed package event");
                        continue;
                    }
                }
            } else {
                continue;
            };
            emitter.emit(event.with_committed_block(block));
        }
    }

    pub fn configure_namespace_consensus(
        &self,
        config: crate::namespace_consensus::NamespaceConsensusConfig,
    ) -> Result<()> {
        config.validate()?;
        {
            let mut current = self.namespace_consensus.write().map_err(|_| {
                anyhow::anyhow!("namespace consensus configuration lock is poisoned")
            })?;
            *current = config;
        }
        // `open` restores the old canonical durable state immediately when a
        // process died during reorg adoption. Rebuild consensus-derived
        // package state only after the signed namespace configuration exists.
        self.recover_incomplete_reorg()?;
        Ok(())
    }

    pub fn namespace_consensus_config(
        &self,
    ) -> Result<crate::namespace_consensus::NamespaceConsensusConfig> {
        self.namespace_consensus
            .read()
            .map(|config| config.clone())
            .map_err(|_| anyhow::anyhow!("namespace consensus configuration lock is poisoned"))
    }

    fn reorg_write_options() -> WriteOptions {
        let mut options = WriteOptions::default();
        options.set_sync(true);
        options
    }

    fn load_reorg_journal(&self) -> Result<Option<ReorgJournal>> {
        let cf = self
            .db
            .cf_handle(CF_REORG_JOURNAL)
            .context("cf reorg_journal")?;
        self.db
            .get_cf(&cf, REORG_JOURNAL_KEY)?
            .map(|bytes| serde_json::from_slice(&bytes).context("decode active reorg journal"))
            .transpose()
    }

    /// Acquire the store-wide ordinary mutation capability and check the
    /// durable maintenance latch while that capability is held. Keeping the
    /// returned guard alive through the RocksDB write closes the former
    /// check-then-write race with `begin_reorg_journal`.
    fn ensure_no_active_reorg(&self) -> Result<StoreMutationGuard> {
        let guard = StoreMutationGuard::ordinary(&self.store_mutation_lock)?;
        if self.load_reorg_journal()?.is_some() {
            anyhow::bail!(
                "chain is in reorg-recovery maintenance; canonical mutations are unavailable"
            );
        }
        Ok(guard)
    }

    fn try_active_reorg_guard(&self) -> Result<Option<ReorgMutationGuard>> {
        let mutation = StoreMutationGuard::reorg(&self.store_mutation_lock)?;
        if self.load_reorg_journal()?.is_none() {
            return Ok(None);
        }
        Ok(Some(ReorgMutationGuard {
            db: Arc::clone(&self.db),
            mutation,
        }))
    }

    fn validate_reorg_guard(&self, guard: &ReorgMutationGuard) -> Result<()> {
        if !Arc::ptr_eq(&self.db, &guard.db)
            || !Arc::ptr_eq(&self.store_mutation_lock, &guard.mutation.mutex)
            || guard.mutation.owner != StoreMutationOwner::Reorg
        {
            anyhow::bail!("reorg mutation guard belongs to a different chain database");
        }
        if self.load_reorg_journal()?.is_none() {
            anyhow::bail!("reorg mutation guard is no longer active");
        }
        Ok(())
    }

    fn load_reorg_finalized_work_snapshot(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let cf = self
            .db
            .cf_handle(CF_REORG_JOURNAL)
            .context("cf reorg_journal")?;
        let mut entries = Vec::new();
        for item in self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
            let (key, value) = item?;
            if key.as_ref() == REORG_JOURNAL_KEY {
                continue;
            }
            let Some(work_id) = key.as_ref().strip_prefix(REORG_FINALIZED_WORK_PREFIX) else {
                anyhow::bail!("reorg journal contains an unknown record");
            };
            let work_id =
                String::from_utf8(work_id.to_vec()).context("invalid reorg work identifier")?;
            let envelope = Self::decode_finalized_work(&value)?;
            if transaction_hash(&envelope.transaction) != work_id {
                anyhow::bail!("reorg journal finalized-work key does not match its transaction");
            }
            entries.push((work_id.into_bytes(), value.to_vec()));
        }
        Ok(entries)
    }

    /// Persist the old canonical branch before the first live reorg write.
    /// The sync WAL write makes process/power-loss recovery choose the prior
    /// fully committed branch unless adoption explicitly completes.
    pub(crate) fn begin_reorg_journal(
        &self,
        common_ancestor: u64,
        old_branch: &[Block],
    ) -> Result<ReorgMutationGuard> {
        // Acquire this before observing the journal or taking either durable
        // snapshot. Ordinary mutations retain the same store-wide capability
        // from their maintenance check through their write, so neither the
        // canonical branch nor finalized-work queue can change underneath
        // this journal.
        let mutation = StoreMutationGuard::reorg(&self.store_mutation_lock)?;
        #[cfg(test)]
        {
            let hook = self
                .reorg_begin_test_hook
                .lock()
                .map_err(|_| anyhow::anyhow!("reorg begin test hook lock is poisoned"))?
                .clone();
            if let Some(hook) = hook {
                hook.lock_acquired.wait();
                hook.resume.wait();
            }
        }
        if self.load_reorg_journal()?.is_some() {
            anyhow::bail!("another reorg journal is already active");
        }
        let reorg_cf = self
            .db
            .cf_handle(CF_REORG_JOURNAL)
            .context("cf reorg_journal")?;
        if self
            .db
            .iterator_cf(&reorg_cf, rocksdb::IteratorMode::Start)
            .next()
            .is_some()
        {
            anyhow::bail!("reorg journal contains orphaned records");
        }
        let ancestor = self
            .get_block_by_height(common_ancestor)?
            .ok_or_else(|| anyhow::anyhow!("reorg journal common ancestor is unavailable"))?;
        let ancestor_hash = ancestor.hash();
        let mut expected_height = common_ancestor.saturating_add(1);
        let mut expected_parent = ancestor_hash.clone();
        let mut entries = Vec::with_capacity(old_branch.len());
        for block in old_branch {
            if block.header.height != expected_height || block.header.prev_hash != expected_parent {
                anyhow::bail!("reorg journal old branch is not a contiguous canonical suffix");
            }
            let canonical = self.get_block_by_height(expected_height)?.ok_or_else(|| {
                anyhow::anyhow!("reorg journal old canonical block is unavailable")
            })?;
            if serde_json::to_value(&canonical)? != serde_json::to_value(block)? {
                anyhow::bail!("reorg journal old branch does not match canonical storage");
            }
            let hash = block.hash();
            entries.push(ReorgJournalEntry {
                height: expected_height,
                hash: hash.clone(),
            });
            expected_height = expected_height.saturating_add(1);
            expected_parent = hash;
        }
        if expected_height.saturating_sub(1) != self.tip_height()? {
            anyhow::bail!("reorg journal does not cover the complete canonical suffix");
        }

        crate::validator_set_history::with_reorg_snapshot(
            self.data_dir.as_path(),
            |validator_history| {
                let journal = ReorgJournal {
                    schema_version: 2,
                    common_ancestor,
                    ancestor_hash,
                    old_branch: entries,
                    validator_history,
                };
                let bytes = serde_json::to_vec(&journal).context("encode reorg journal")?;
                let work_cf = self
                    .db
                    .cf_handle(CF_FINALIZED_WORK)
                    .context("cf finalized_work")?;
                let mut batch = rocksdb::WriteBatch::default();
                batch.put_cf(&reorg_cf, REORG_JOURNAL_KEY, bytes);
                for item in self.db.iterator_cf(&work_cf, rocksdb::IteratorMode::Start) {
                    let (work_id, value) = item?;
                    let envelope = Self::decode_finalized_work(&value)?;
                    if transaction_hash(&envelope.transaction).as_bytes() != work_id.as_ref() {
                        anyhow::bail!("finalized-work key does not match its transaction");
                    }
                    let mut journal_key =
                        Vec::with_capacity(REORG_FINALIZED_WORK_PREFIX.len() + work_id.len());
                    journal_key.extend_from_slice(REORG_FINALIZED_WORK_PREFIX);
                    journal_key.extend_from_slice(&work_id);
                    batch.put_cf(&reorg_cf, journal_key, value);
                }
                self.db
                    .write_opt(batch, &Self::reorg_write_options())
                    .context(
                        "persist reorg journal, validator history, and finalized-work snapshot",
                    )?;
                Ok(ReorgMutationGuard {
                    db: Arc::clone(&self.db),
                    mutation,
                })
            },
        )
    }

    /// Restore the canonical height mapping, finalized-work queue, and
    /// validator authority history from an active journal. This is safe before
    /// namespace consensus configuration is available. Derived package state
    /// is rebuilt by `recover_incomplete_reorg` after configuration.
    fn restore_reorg_durable_state_if_needed(&self) -> Result<bool> {
        let Some(journal) = self.load_reorg_journal()? else {
            return Ok(false);
        };
        if journal.schema_version != 2 {
            anyhow::bail!("unsupported reorg journal schema version");
        }
        let ancestor = self
            .get_block_by_hash(&journal.ancestor_hash)?
            .ok_or_else(|| anyhow::anyhow!("reorg journal ancestor body is unavailable"))?;
        if ancestor.header.height != journal.common_ancestor
            || ancestor.hash() != journal.ancestor_hash
        {
            anyhow::bail!("reorg journal ancestor binding is invalid");
        }

        let mut expected_height = journal.common_ancestor.saturating_add(1);
        let mut expected_parent = journal.ancestor_hash.clone();
        for entry in &journal.old_branch {
            let block = self
                .get_block_by_hash(&entry.hash)?
                .ok_or_else(|| anyhow::anyhow!("reorg journal old block body is unavailable"))?;
            if entry.height != expected_height
                || block.header.height != entry.height
                || block.header.prev_hash != expected_parent
                || block.hash() != entry.hash
            {
                anyhow::bail!("reorg journal old branch binding is invalid");
            }
            expected_height = expected_height.saturating_add(1);
            expected_parent = entry.hash.clone();
        }

        let cf_height = self
            .db
            .cf_handle(CF_BLOCKS_BY_HEIGHT)
            .context("cf blocks_by_height")?;
        let cf_finalized_work = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let finalized_work_snapshot = self.load_reorg_finalized_work_snapshot()?;
        let height_keys: Vec<Box<[u8]>> = self
            .db
            .iterator_cf(&cf_height, rocksdb::IteratorMode::Start)
            .map(|item| item.map(|(key, _)| key))
            .collect::<std::result::Result<_, _>>()?;
        let mut batch = rocksdb::WriteBatch::default();
        for key in height_keys {
            let encoded: [u8; 8] = key
                .as_ref()
                .try_into()
                .map_err(|_| anyhow::anyhow!("canonical height key is malformed"))?;
            if u64::from_be_bytes(encoded) > journal.common_ancestor {
                batch.delete_cf(&cf_height, key);
            }
        }
        batch.put_cf(
            &cf_height,
            journal.common_ancestor.to_be_bytes(),
            journal.ancestor_hash.as_bytes(),
        );
        for entry in &journal.old_branch {
            batch.put_cf(
                &cf_height,
                entry.height.to_be_bytes(),
                entry.hash.as_bytes(),
            );
        }
        let finalized_work_keys: Vec<Box<[u8]>> = self
            .db
            .iterator_cf(&cf_finalized_work, rocksdb::IteratorMode::Start)
            .map(|item| item.map(|(key, _)| key))
            .collect::<std::result::Result<_, _>>()?;
        for key in finalized_work_keys {
            batch.delete_cf(&cf_finalized_work, key);
        }
        for (work_id, value) in finalized_work_snapshot {
            batch.put_cf(&cf_finalized_work, work_id, value);
        }
        self.db
            .write_opt(batch, &Self::reorg_write_options())
            .context("restore canonical height mapping and finalized work from reorg journal")?;
        crate::validator_set_history::restore_reorg_snapshot(
            self.data_dir.as_path(),
            journal.validator_history.as_deref(),
        )
        .context("restore validator-set history from reorg journal")?;
        tracing::warn!(
            ancestor = journal.common_ancestor,
            restored_tip = expected_height.saturating_sub(1),
            "Recovered prior canonical height mapping from incomplete reorg journal"
        );
        Ok(true)
    }

    /// Complete startup/live recovery of an interrupted reorg after the signed
    /// namespace configuration has been installed.
    pub(crate) fn recover_incomplete_reorg(&self) -> Result<bool> {
        let Some(guard) = self.try_active_reorg_guard()? else {
            return Ok(false);
        };
        self.recover_incomplete_reorg_with_guard(&guard)
    }

    /// Restore an interrupted live adoption while retaining its already-owned
    /// store mutation capability. Terminal adoption failures call this before
    /// their `ReorgMutationGuard` can be dropped; attempting to acquire a
    /// second exclusive guard would otherwise reject recovery and leave the
    /// durable journal latched.
    pub(crate) fn recover_incomplete_reorg_with_guard(
        &self,
        guard: &ReorgMutationGuard,
    ) -> Result<bool> {
        self.validate_reorg_guard(guard)?;
        self.restore_reorg_durable_state_if_needed()?;
        self.rebuild_package_index_for_reorg(guard)
            .context("rebuild derived state while recovering incomplete reorg")?;
        self.complete_reorg_journal(guard)?;
        Ok(true)
    }

    pub(crate) fn complete_reorg_journal(&self, guard: &ReorgMutationGuard) -> Result<()> {
        self.validate_reorg_guard(guard)?;
        let cf = self
            .db
            .cf_handle(CF_REORG_JOURNAL)
            .context("cf reorg_journal")?;
        let keys: Vec<Box<[u8]>> = self
            .db
            .iterator_cf(&cf, rocksdb::IteratorMode::Start)
            .map(|item| item.map(|(key, _)| key))
            .collect::<std::result::Result<_, _>>()?;
        let mut batch = rocksdb::WriteBatch::default();
        for key in keys {
            batch.delete_cf(&cf, key);
        }
        self.db
            .write_opt(batch, &Self::reorg_write_options())
            .context("clear committed reorg journal")?;
        Ok(())
    }

    pub fn open(data_dir: &Path) -> Result<Self> {
        Self::open_with_genesis(data_dir, Block::genesis())
    }

    pub fn open_phase_c(data_dir: &Path, spec_genesis_hash: &str) -> Result<Self> {
        Self::open_with_genesis(data_dir, Block::genesis_for_phase_c(spec_genesis_hash)?)
    }

    fn open_with_genesis(data_dir: &Path, genesis: Block) -> Result<Self> {
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
            ColumnFamilyDescriptor::new(
                CF_APPLIED_PUBLISHER_AUTHORIZATION_NONCES,
                Options::default(),
            ),
            ColumnFamilyDescriptor::new(CF_NAMESPACE_DELEGATIONS, Options::default()),
            ColumnFamilyDescriptor::new(CF_PUBLISHER_ADMISSION_NONCES, Options::default()),
            ColumnFamilyDescriptor::new(CF_PUBLISHER_ROTATION, Options::default()),
            ColumnFamilyDescriptor::new(CF_FINALIZED_WORK, Options::default()),
            ColumnFamilyDescriptor::new(CF_FINALIZED_WORK_DEAD_LETTER, Options::default()),
            ColumnFamilyDescriptor::new(CF_TRANSPARENCY_EVENTS, Options::default()),
            ColumnFamilyDescriptor::new(CF_TRANSPARENCY_LEAVES, Options::default()),
            ColumnFamilyDescriptor::new(CF_TRANSPARENCY_HEADS, Options::default()),
            ColumnFamilyDescriptor::new(CF_REORG_JOURNAL, Options::default()),
            ColumnFamilyDescriptor::new(CF_PUBLISHER_INDEX, Options::default()),
        ];

        let db = DB::open_cf_descriptors(&opts, &db_path, cfs).context("Failed to open RocksDB")?;

        let store = Self {
            db: Arc::new(db),
            canonical_genesis: Arc::new(genesis),
            data_dir: Arc::new(data_dir.to_path_buf()),
            pbft_safety_journal: Arc::new(OnceLock::new()),
            runtime_events: Arc::new(OnceLock::new()),
            namespace_consensus: Arc::new(std::sync::RwLock::new(Default::default())),
            publisher_admission_nonce_lock: Arc::new(std::sync::Mutex::new(())),
            transparency_log_lock: Arc::new(std::sync::Mutex::new(())),
            store_mutation_lock: Arc::new(StoreMutationMutex::default()),
            read_only: false,
            #[cfg(test)]
            reorg_begin_test_hook: Arc::new(Mutex::new(None)),
        };

        if let Some(guard) = store.try_active_reorg_guard()? {
            store.restore_reorg_durable_state_if_needed()?;
            drop(guard);
        }

        // Write the genesis block if the chain is empty.
        if store.tip_height()? == 0 && store.block_count() == 0 {
            store.insert_block(&store.canonical_genesis)?;
            tracing::info!("Chain initialised with genesis block");
        }
        let stored_genesis = store
            .get_block_by_height(0)?
            .context("canonical genesis block is missing")?;
        if serde_json::to_value(&stored_genesis)?
            != serde_json::to_value(&*store.canonical_genesis)?
        {
            anyhow::bail!(
                "chain storage belongs to another genesis; preserve its data and signing journal"
            );
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
        let existing_cfs: HashSet<String> = DB::list_cf(&opts, &db_path)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let mut cfs = vec![
            CF_BLOCKS_BY_HASH,
            CF_BLOCKS_BY_HEIGHT,
            CF_PACKAGES,
            CF_NAMESPACE_CONTROLLERS,
            CF_NAMESPACE_LIFECYCLE_NONCES,
            CF_APPLIED_NAMESPACE_LIFECYCLE_NONCES,
            CF_APPLIED_PUBLISHER_AUTHORIZATION_NONCES,
            CF_NAMESPACE_DELEGATIONS,
            CF_PUBLISHER_ADMISSION_NONCES,
            CF_PUBLISHER_ROTATION,
            CF_FINALIZED_WORK,
            CF_FINALIZED_WORK_DEAD_LETTER,
            CF_TRANSPARENCY_EVENTS,
            CF_TRANSPARENCY_LEAVES,
            CF_TRANSPARENCY_HEADS,
            CF_REORG_JOURNAL,
        ];
        if existing_cfs.contains(CF_PUBLISHER_INDEX) {
            cfs.push(CF_PUBLISHER_INDEX);
        }
        let db = DB::open_cf_for_read_only(&opts, &db_path, cfs, false)
            .context("Failed to open RocksDB in read-only mode")?;

        let mut store = Self {
            db: Arc::new(db),
            canonical_genesis: Arc::new(Block::genesis()),
            data_dir: Arc::new(data_dir.to_path_buf()),
            pbft_safety_journal: Arc::new(OnceLock::new()),
            runtime_events: Arc::new(OnceLock::new()),
            namespace_consensus: Arc::new(std::sync::RwLock::new(Default::default())),
            publisher_admission_nonce_lock: Arc::new(std::sync::Mutex::new(())),
            transparency_log_lock: Arc::new(std::sync::Mutex::new(())),
            store_mutation_lock: Arc::new(StoreMutationMutex::default()),
            read_only: true,
            #[cfg(test)]
            reorg_begin_test_hook: Arc::new(Mutex::new(None)),
        };
        if store.load_reorg_journal()?.is_some() {
            anyhow::bail!(
                "chain has an incomplete reorg journal; open it read-write to recover first"
            );
        }
        store.canonical_genesis = Arc::new(
            store
                .get_block_by_height(0)?
                .context("read-only chain has no canonical genesis")?,
        );
        Ok(store)
    }

    pub fn initialize_pbft_safety_journal(
        &self,
        genesis_hash: &str,
        validator_id: &str,
        validator_pubkey: &str,
    ) -> Result<()> {
        if self.pbft_safety_journal.get().is_some() {
            anyhow::bail!("PBFT safety journal was already initialized");
        }
        let binding = serde_json::to_vec(&(
            genesis_hash.trim_start_matches("0x").to_ascii_lowercase(),
            validator_id,
            validator_pubkey
                .trim_start_matches("0x")
                .to_ascii_lowercase(),
        ))?;
        let existing_binding = self.db.get(PBFT_JOURNAL_BINDING_KEY)?;
        if existing_binding
            .as_deref()
            .is_some_and(|bytes| bytes != binding.as_slice())
        {
            anyhow::bail!("chain storage is bound to another PBFT signing identity");
        }
        let existing_journal = self.data_dir.join("pbft-safety.rocksdb").exists();
        let journal = if existing_binding.is_some() || existing_journal {
            crate::pbft_safety::PbftSigningJournal::open_existing(
                &self.data_dir,
                genesis_hash,
                validator_id,
                validator_pubkey,
            )?
        } else {
            if self.tip_height()? != 0 || self.block_count() != 1 {
                anyhow::bail!("nonempty validator chain lacks its original signing journal; refusing to recreate locks");
            }
            crate::pbft_safety::PbftSigningJournal::open(
                &self.data_dir,
                genesis_hash,
                validator_id,
                validator_pubkey,
            )?
        };
        // No caller may sign until both durable anchors exist. A crash before
        // this write leaves an existing journal that is verified on restart.
        let mut options = WriteOptions::default();
        options.set_sync(true);
        self.db
            .put_opt(PBFT_JOURNAL_BINDING_KEY, binding, &options)?;
        self.pbft_safety_journal
            .set(Arc::new(journal))
            .map_err(|_| anyhow::anyhow!("PBFT safety journal initialization raced"))
    }

    pub fn pbft_safety_journal(&self) -> Result<Arc<crate::pbft_safety::PbftSigningJournal>> {
        self.pbft_safety_journal.get().cloned().context(
            "PBFT signing is blocked because its durable safety journal is not initialized",
        )
    }

    pub(crate) fn data_dir(&self) -> &Path {
        self.data_dir.as_path()
    }

    /// Load the persistent publisher index without O(N) block replay when
    /// already synchronized with the canonical chain tip; otherwise backfill
    /// missing heights incrementally or migrate a legacy database in place.
    pub fn load_or_sync_publisher_index(&self) -> Result<crate::publisher_index::PublisherIndex> {
        let tip = self.tip_height()?;
        let Some(cf_pub) = self.db.cf_handle(CF_PUBLISHER_INDEX) else {
            let mut index = crate::publisher_index::PublisherIndex::new();
            for h in 0..=tip {
                if let Some(block) = self.get_block_by_height(h)? {
                    index.apply_block(&block);
                }
            }
            return Ok(index);
        };

        let indexed_tip = self
            .db
            .get_cf(&cf_pub, PUB_IDX_TIP_KEY)?
            .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
            .map(u64::from_be_bytes);

        if let Some(t) = indexed_tip.filter(|t| *t <= tip) {
            let mut stats_map = HashMap::new();
            for item in self
                .db
                .prefix_iterator_cf(&cf_pub, PUB_IDX_STATS_PREFIX.as_bytes())
            {
                let (key_bytes, val_bytes) = item?;
                if !key_bytes.starts_with(PUB_IDX_STATS_PREFIX.as_bytes()) {
                    break;
                }
                let pubkey = String::from_utf8(key_bytes[PUB_IDX_STATS_PREFIX.len()..].to_vec())
                    .context("non-UTF-8 publisher stats key")?;
                let stats: crate::publisher_index::PublisherStats =
                    serde_json::from_slice(&val_bytes)?;
                stats_map.insert(pubkey, stats);
            }
            let mut canon_map = HashMap::new();
            for item in self
                .db
                .prefix_iterator_cf(&cf_pub, PUB_IDX_PKG_PREFIX.as_bytes())
            {
                let (key_bytes, val_bytes) = item?;
                if !key_bytes.starts_with(PUB_IDX_PKG_PREFIX.as_bytes()) {
                    break;
                }
                let canonical = String::from_utf8(key_bytes[PUB_IDX_PKG_PREFIX.len()..].to_vec())
                    .context("non-UTF-8 publisher package key")?;
                let pubkey = String::from_utf8(val_bytes.to_vec())
                    .context("non-UTF-8 publisher package mapping value")?;
                canon_map.insert(canonical, pubkey);
            }
            let mut index =
                crate::publisher_index::PublisherIndex::from_parts(stats_map, canon_map);
            if t < tip {
                for h in (t + 1)..=tip {
                    if let Some(block) = self.get_block_by_height(h)? {
                        index.apply_block(&block);
                    }
                }
                if !self.read_only {
                    self.write_publisher_index_to_db(&index, tip)?;
                }
            }
            return Ok(index);
        }

        let mut index = crate::publisher_index::PublisherIndex::new();
        for h in 0..=tip {
            if let Some(block) = self.get_block_by_height(h)? {
                index.apply_block(&block);
            }
        }
        if !self.read_only {
            self.write_publisher_index_to_db(&index, tip)?;
        }
        Ok(index)
    }

    fn write_publisher_index_to_db(
        &self,
        index: &crate::publisher_index::PublisherIndex,
        indexed_tip: u64,
    ) -> Result<()> {
        let _mutation = self.ensure_no_active_reorg()?;
        let cf_pub = self
            .db
            .cf_handle(CF_PUBLISHER_INDEX)
            .context("cf publisher_index")?;
        let mut batch = rocksdb::WriteBatch::default();
        let stale_keys: Vec<Box<[u8]>> = self
            .db
            .iterator_cf(&cf_pub, rocksdb::IteratorMode::Start)
            .filter_map(|item| item.ok().map(|(k, _)| k))
            .collect();
        for k in stale_keys {
            batch.delete_cf(&cf_pub, k);
        }
        for (pubkey, stats) in index.stats_entries() {
            let key = format!("{PUB_IDX_STATS_PREFIX}{pubkey}");
            batch.put_cf(&cf_pub, key.as_bytes(), &serde_json::to_vec(stats)?);
        }
        for (canonical, pubkey) in index.canonical_mappings() {
            let key = format!("{PUB_IDX_PKG_PREFIX}{canonical}");
            batch.put_cf(&cf_pub, key.as_bytes(), pubkey.as_bytes());
        }
        batch.put_cf(&cf_pub, PUB_IDX_TIP_KEY, indexed_tip.to_be_bytes());
        self.db.write_opt(batch, &Self::reorg_write_options())?;
        Ok(())
    }

    // ── Block operations ─────────────────────────────────────────────────────

    /// Insert a block without permitting an implicit canonical replacement.
    ///
    /// A different block at an occupied height is rejected by `insert_block`.
    /// Reorganizations must go through the full-branch checkpoint preflight
    /// and explicit rollback path in sync, so this outcome can only report an
    /// idempotent insert or a newly appended block.
    pub fn insert_block_with_outcome(&self, block: &Block) -> Result<BlockInsertOutcome> {
        let new_hash = block.hash();
        self.insert_block(block)?;
        Ok(BlockInsertOutcome {
            hash: new_hash,
            replaced_hash: None,
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
        let _mutation = self.ensure_no_active_reorg()?;
        self.rollback_to_height_inner(height)
    }

    pub(crate) fn rollback_to_height_for_reorg(
        &self,
        guard: &ReorgMutationGuard,
        height: u64,
    ) -> Result<Vec<String>> {
        self.validate_reorg_guard(guard)?;
        self.rollback_to_height_inner(height)
    }

    fn rollback_to_height_inner(&self, height: u64) -> Result<Vec<String>> {
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

    /// Validate a complete reorganization branch in an isolated RocksDB
    /// checkpoint before the live canonical height index is changed.
    ///
    /// The checkpoint is rewound to `common_ancestor` and its derived package,
    /// namespace-controller, delegation, and applied-nonce state is rebuilt
    /// from that prefix. Candidate blocks are then inserted sequentially so
    /// CR-07 validation observes the candidate branch rather than state left
    /// behind by the abandoned branch.
    pub(crate) fn preflight_reorg_branch(
        &self,
        common_ancestor: u64,
        candidate_blocks: &[Block],
    ) -> Result<()> {
        let canonical_tip = self.tip_height()?;
        if common_ancestor > canonical_tip {
            anyhow::bail!(
                "reorg common ancestor {} is above canonical tip {}",
                common_ancestor,
                canonical_tip
            );
        }
        if candidate_blocks.is_empty() {
            anyhow::bail!("reorg candidate branch is empty");
        }

        let ancestor = self
            .get_block_by_height(common_ancestor)?
            .ok_or_else(|| anyhow::anyhow!("reorg common ancestor block is unavailable"))?;
        let mut expected_height = common_ancestor.saturating_add(1);
        let mut expected_parent = ancestor.hash();
        for block in candidate_blocks {
            if let Some(journal) = self.pbft_safety_journal.get() {
                journal.validate_canonical_block(block)?;
            }
            if block.header.height != expected_height {
                anyhow::bail!(
                    "reorg candidate height {} does not continue expected height {}",
                    block.header.height,
                    expected_height
                );
            }
            if block.header.prev_hash != expected_parent {
                anyhow::bail!(
                    "reorg candidate block {} does not descend from the candidate prefix",
                    block.header.height
                );
            }
            expected_height = expected_height.saturating_add(1);
            expected_parent = block.hash();
        }

        let checkpoint_root = tempfile::Builder::new()
            .prefix("creg-reorg-preflight-")
            .tempdir_in(self.data_dir.as_ref())
            .context("create reorg preflight directory")?;
        let checkpoint_path = checkpoint_root.path().join("chain.rocksdb");
        Checkpoint::new(self.db.as_ref())
            .context("create RocksDB checkpoint handle")?
            .create_checkpoint(&checkpoint_path)
            .context("create RocksDB reorg checkpoint")?;

        let preflight = ChainStore::open_with_genesis(
            checkpoint_root.path(),
            (*self.canonical_genesis).clone(),
        )
        .context("open isolated reorg checkpoint")?;
        preflight.configure_namespace_consensus(self.namespace_consensus_config()?)?;
        let validation = (|| -> Result<()> {
            preflight.rollback_to_height(common_ancestor)?;
            preflight
                .rebuild_package_index()
                .context("rebuild common-ancestor namespace state in reorg checkpoint")?;
            for block in candidate_blocks {
                preflight.insert_block(block).with_context(|| {
                    format!(
                        "candidate branch failed CR-07 preflight at height {}",
                        block.header.height
                    )
                })?;
            }
            preflight
                .rebuild_package_index()
                .context("candidate branch failed deterministic rebuild preflight")?;
            if preflight.tip_height()? != candidate_blocks.last().expect("non-empty").header.height
                || preflight.tip_hash()? != expected_parent
            {
                anyhow::bail!("isolated reorg checkpoint did not reach the candidate tip");
            }
            Ok(())
        })();
        drop(preflight);
        if let Err(error) = checkpoint_root.close() {
            tracing::warn!(
                error = %error,
                "Could not remove completed RocksDB reorg checkpoint"
            );
        }
        validation
    }

    /// Rebuild the package index (CF_PACKAGES) by replaying every canonical
    /// block from genesis to the current tip. Required after a reorg rewind so
    /// records indexed from abandoned blocks don't linger.
    pub fn rebuild_package_index(&self) -> Result<()> {
        let _mutation = self.ensure_no_active_reorg()?;
        self.rebuild_package_index_inner(None)
    }

    pub(crate) fn rebuild_package_index_for_reorg(&self, guard: &ReorgMutationGuard) -> Result<()> {
        self.validate_reorg_guard(guard)?;
        self.rebuild_package_index_inner(Some(guard))
    }

    fn rebuild_package_index_inner(&self, reorg_guard: Option<&ReorgMutationGuard>) -> Result<()> {
        let cf_pkg = self.db.cf_handle(CF_PACKAGES).context("cf packages")?;
        let cf_namespace_controllers = self
            .db
            .cf_handle(CF_NAMESPACE_CONTROLLERS)
            .context("cf namespace_controllers")?;
        let cf_applied_namespace_lifecycle_nonces = self
            .db
            .cf_handle(CF_APPLIED_NAMESPACE_LIFECYCLE_NONCES)
            .context("cf applied_namespace_lifecycle_nonces")?;
        let cf_applied_publisher_authorization_nonces = self
            .db
            .cf_handle(CF_APPLIED_PUBLISHER_AUTHORIZATION_NONCES)
            .context("cf applied_publisher_authorization_nonces")?;
        let cf_namespace_delegations = self
            .db
            .cf_handle(CF_NAMESPACE_DELEGATIONS)
            .context("cf namespace_delegations")?;
        let cf_publisher_rotation = self
            .db
            .cf_handle(CF_PUBLISHER_ROTATION)
            .context("cf publisher_rotation")?;
        let cf_pub_index = self
            .db
            .cf_handle(CF_PUBLISHER_INDEX)
            .context("cf publisher_index")?;
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
        for cf in [
            &cf_applied_publisher_authorization_nonces,
            &cf_namespace_delegations,
            &cf_publisher_rotation,
            &cf_pub_index,
        ] {
            let keys: Vec<Box<[u8]>> = self
                .db
                .iterator_cf(cf, rocksdb::IteratorMode::Start)
                .filter_map(|item| item.ok().map(|(key, _)| key))
                .collect();
            for key in keys {
                batch.delete_cf(cf, key);
            }
        }
        self.db.write(batch)?;

        // Replay the canonical chain. insert_block re-indexes Publish /
        // Revoke / RotatePublisherKey transactions idempotently.
        let tip = self.tip_height()?;
        for h in 0..=tip {
            if let Some(block) = self.get_block_by_height(h)? {
                match reorg_guard {
                    Some(guard) => self.insert_block_for_reorg(guard, &block)?,
                    // Public rebuild holds the ordinary store mutation guard
                    // for the whole clear-and-replay operation.
                    None => self.insert_block_inner(&block)?,
                }
            }
        }
        tracing::info!("Package index rebuilt from canonical chain (tip {})", tip);
        Ok(())
    }

    pub fn insert_block(&self, block: &Block) -> Result<()> {
        let _mutation = self.ensure_no_active_reorg()?;
        self.insert_block_inner(block)
    }

    pub(crate) fn insert_block_for_reorg(
        &self,
        guard: &ReorgMutationGuard,
        block: &Block,
    ) -> Result<()> {
        self.validate_reorg_guard(guard)?;
        self.insert_block_inner(block)
    }

    fn insert_block_inner(&self, block: &Block) -> Result<()> {
        if let Some(journal) = self.pbft_safety_journal.get() {
            journal.validate_canonical_block(block)?;
        }
        let hash = block.hash();
        let bytes = serde_json::to_vec(block)?;
        // PBFT certificates attest the header hash but are not themselves
        // committed by it. Different honest quorum subsets or signature order
        // therefore describe the same canonical payload. Idempotence compares
        // the hash-bound header and transaction body, while preserving the
        // first accepted certificate encoding, including view-change witnesses.
        let mut identity_header = block.header.clone();
        identity_header.view_change_certificate.clear();
        let canonical_payload = serde_json::to_value((&identity_header, &block.transactions))?;
        let existing_at_height = self.get_block_by_height(block.header.height)?;
        if let Some(existing) = existing_at_height.as_ref() {
            let mut existing_header = existing.header.clone();
            existing_header.view_change_certificate.clear();
            if existing.hash() != hash
                || serde_json::to_value((&existing_header, &existing.transactions))?
                    != canonical_payload
            {
                anyhow::bail!(
                    "refusing direct replacement of canonical block at height {}; use the preflighted reorg path",
                    block.header.height
                );
            }
        }

        if block.header.height == 0 {
            if serde_json::to_value(block)? != serde_json::to_value(&*self.canonical_genesis)?
                || block.hash() != self.canonical_genesis.hash()
            {
                anyhow::bail!("height zero must contain the exact canonical genesis block");
            }
            if existing_at_height.is_none() && self.block_count() != 0 {
                anyhow::bail!("canonical genesis may only initialize an empty chain store");
            }
        } else {
            let expected_merkle_root = common::merkle_root(&block.transactions);
            if block.header.merkle_root != expected_merkle_root {
                anyhow::bail!("block merkle root does not commit to its transaction body");
            }

            let parent_height = block.header.height - 1;
            let parent = self.get_block_by_height(parent_height)?.ok_or_else(|| {
                anyhow::anyhow!("canonical parent block {parent_height} is missing")
            })?;
            let parent_hash = parent.hash();
            if block.header.prev_hash != parent_hash {
                anyhow::bail!(
                    "block {} does not descend from canonical parent {}",
                    block.header.height,
                    parent_height
                );
            }
            crate::block_time::validate_chain_timestamp(
                parent.header.timestamp,
                block.header.timestamp,
            )
            .with_context(|| {
                format!(
                    "block {} timestamp does not follow canonical parent {}",
                    block.header.height, parent_height
                )
            })?;
            if existing_at_height.is_none() {
                let expected_height = self.tip_height()?.saturating_add(1);
                if block.header.height != expected_height {
                    anyhow::bail!(
                        "new canonical block height {} does not append expected height {}",
                        block.header.height,
                        expected_height
                    );
                }
            }
        }
        let namespace_config = self.namespace_consensus_config()?;
        let namespace_effects =
            crate::namespace_consensus::validate_block(self, block, &namespace_config)?;
        let height_key = block.header.height.to_be_bytes();

        let cf_hash = self
            .db
            .cf_handle(CF_BLOCKS_BY_HASH)
            .context("cf blocks_by_hash")?;
        let body_already_stored =
            if let Some(existing_bytes) = self.db.get_cf(&cf_hash, hash.as_bytes())? {
                let existing_block: Block = serde_json::from_slice(&existing_bytes)
                    .context("decode block body stored under existing header hash")?;
                if existing_block.hash() != hash
                    || serde_json::to_value((&existing_block.header, &existing_block.transactions))?
                        != canonical_payload
                {
                    anyhow::bail!(
                        "refusing replacement of block body stored under existing header hash {}",
                        hash
                    );
                }
                true
            } else {
                false
            };
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
        let cf_applied_publisher_authorization_nonces = self
            .db
            .cf_handle(CF_APPLIED_PUBLISHER_AUTHORIZATION_NONCES)
            .context("cf applied_publisher_authorization_nonces")?;
        let cf_namespace_delegations = self
            .db
            .cf_handle(CF_NAMESPACE_DELEGATIONS)
            .context("cf namespace_delegations")?;
        let cf_finalized_work = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let cf_publisher_rotation = self
            .db
            .cf_handle(CF_PUBLISHER_ROTATION)
            .context("cf publisher_rotation")?;
        let cf_pub_index = self
            .db
            .cf_handle(CF_PUBLISHER_INDEX)
            .context("cf publisher_index")?;
        let existing_indexed_tip = self
            .db
            .get_cf(&cf_pub_index, PUB_IDX_TIP_KEY)?
            .and_then(|bytes| <[u8; 8]>::try_from(bytes.as_slice()).ok())
            .map(u64::from_be_bytes);
        let should_update_pub_index = existing_at_height.is_none()
            || existing_indexed_tip.is_none()
            || existing_indexed_tip.is_some_and(|t| block.header.height > t);

        // Use a WriteBatch for atomicity.
        let mut batch = rocksdb::WriteBatch::default();
        let mut lifecycle_nonces_in_block = HashSet::new();
        let mut pkg_overlay: HashMap<String, ChainRecord> = HashMap::new();
        let mut pub_stats_overlay: HashMap<String, crate::publisher_index::PublisherStats> =
            HashMap::new();
        let mut pub_canon_overlay: HashMap<String, String> = HashMap::new();
        let mut pub_stats_deleted: HashSet<String> = HashSet::new();
        let mut namespace_controllers_in_block: HashSet<String> = HashSet::new();

        // Preserve the first accepted encoding. A decoded/re-encoded block can
        // differ in JSON map order or carry a different valid PBFT quorum
        // subset/order; payload equality above makes that an idempotent replay
        // without allowing header or transaction replacement.
        if !body_already_stored {
            batch.put_cf(&cf_hash, hash.as_bytes(), &bytes);
        }
        batch.put_cf(&cf_height, height_key, hash.as_bytes());

        // Index every Publish, Revoke, Slash, and Rotate transaction atomically.
        for tx in &block.transactions {
            // A finalized block is the acknowledgement boundary for durable
            // work. Include this delete in the same RocksDB WriteBatch as the
            // block/index writes so a restart cannot lose work after ack.
            batch.delete_cf(&cf_finalized_work, transaction_hash(tx).as_bytes());
            if let Some(record) = tx.published_record() {
                // Update block_hash to the real finalized hash before persisting.
                let mut rec = record.clone();
                rec.block_hash = hash.clone();
                let canonical = rec.id.canonical();
                let rec_bytes = serde_json::to_vec(&rec)?;
                batch.put_cf(&cf_pkg, canonical.as_bytes(), &rec_bytes);
                if should_update_pub_index {
                    let pubkey = rec.publisher_pubkey.clone();
                    pub_canon_overlay.insert(canonical.clone(), pubkey.clone());
                    let mut stats = if let Some(s) = pub_stats_overlay.get(&pubkey) {
                        s.clone()
                    } else if !pub_stats_deleted.contains(&pubkey) {
                        let key = format!("{PUB_IDX_STATS_PREFIX}{pubkey}");
                        self.db
                            .get_cf(&cf_pub_index, key.as_bytes())?
                            .and_then(|b| serde_json::from_slice(&b).ok())
                            .unwrap_or_else(|| crate::publisher_index::PublisherStats {
                                pubkey: pubkey.clone(),
                                ..Default::default()
                            })
                    } else {
                        crate::publisher_index::PublisherStats {
                            pubkey: pubkey.clone(),
                            ..Default::default()
                        }
                    };
                    pub_stats_deleted.remove(&pubkey);
                    stats.total_packages += 1;
                    stats.verified_count += 1;
                    if stats.first_seen_at.is_none() {
                        stats.first_seen_at = Some(block.header.timestamp);
                    }
                    stats.first_seen_days = stats.account_age_days_at(block.header.timestamp);
                    pub_stats_overlay.insert(pubkey, stats);
                }
                if matches!(tx, common::Transaction::Publish(_)) {
                    let namespace = common::package_namespace_prefix(&rec.id);
                    if !namespace_controllers_in_block.contains(&namespace)
                        && self
                            .db
                            .get_cf(&cf_namespace_controllers, namespace.as_bytes())?
                            .is_none()
                    {
                        batch.put_cf(
                            &cf_namespace_controllers,
                            namespace.as_bytes(),
                            rec.publisher_pubkey.as_bytes(),
                        );
                        namespace_controllers_in_block.insert(namespace);
                    }
                }
                pkg_overlay.insert(canonical, rec);
            }
            if let Some((package_canonical, reason, _, _)) = tx.revocation_target() {
                let existing_rec = if let Some(rec) = pkg_overlay.get(package_canonical) {
                    Some(rec.clone())
                } else {
                    self.db
                        .get_cf(&cf_pkg, package_canonical.as_bytes())?
                        .and_then(|existing| serde_json::from_slice::<ChainRecord>(&existing).ok())
                };
                if let Some(mut rec) = existing_rec {
                    rec.status = PackageStatus::Revoked {
                        reason: reason.to_string(),
                    };
                    let updated = serde_json::to_vec(&rec)?;
                    batch.put_cf(&cf_pkg, package_canonical.as_bytes(), &updated);
                    pkg_overlay.insert(package_canonical.to_string(), rec);
                }
                if should_update_pub_index {
                    let mapped_pubkey = if let Some(pk) = pub_canon_overlay.get(package_canonical) {
                        Some(pk.clone())
                    } else {
                        let key = format!("{PUB_IDX_PKG_PREFIX}{package_canonical}");
                        self.db
                            .get_cf(&cf_pub_index, key.as_bytes())?
                            .and_then(|v| String::from_utf8(v.to_vec()).ok())
                    };
                    if let Some(pubkey) = mapped_pubkey {
                        let existing_stats = if let Some(s) = pub_stats_overlay.get(&pubkey) {
                            Some(s.clone())
                        } else if !pub_stats_deleted.contains(&pubkey) {
                            let key = format!("{PUB_IDX_STATS_PREFIX}{pubkey}");
                            self.db
                                .get_cf(&cf_pub_index, key.as_bytes())?
                                .and_then(|b| serde_json::from_slice(&b).ok())
                        } else {
                            None
                        };
                        if let Some(mut stats) = existing_stats {
                            stats.verified_count = stats.verified_count.saturating_sub(1);
                            stats.revoked_count += 1;
                            pub_stats_overlay.insert(pubkey, stats);
                        }
                    }
                }
            }
            if let common::Transaction::Slash {
                validator_id,
                amount,
                ..
            } = tx
            {
                if should_update_pub_index {
                    let existing_stats = if let Some(s) = pub_stats_overlay.get(validator_id) {
                        Some(s.clone())
                    } else if !pub_stats_deleted.contains(validator_id) {
                        let key = format!("{PUB_IDX_STATS_PREFIX}{validator_id}");
                        self.db
                            .get_cf(&cf_pub_index, key.as_bytes())?
                            .and_then(|b| serde_json::from_slice(&b).ok())
                    } else {
                        None
                    };
                    if let Some(mut stats) = existing_stats {
                        stats.stake_wei = stats.stake_wei.saturating_sub(u128::from(*amount));
                        pub_stats_overlay.insert(validator_id.clone(), stats);
                    }
                }
            }
            if let common::Transaction::RotatePublisherKey {
                canonical_prefix,
                old_pubkey,
                new_pubkey,
                nonce,
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
                    if let Ok(key_str) = std::str::from_utf8(&key_bytes) {
                        if pkg_overlay.contains_key(key_str) {
                            continue;
                        }
                        if let Ok(mut rec) = serde_json::from_slice::<ChainRecord>(&val_bytes) {
                            if rec.publisher_pubkey == *old_pubkey {
                                rec.publisher_pubkey = new_pubkey.clone();
                                if let Ok(updated) = serde_json::to_vec(&rec) {
                                    batch.put_cf(&cf_pkg, &*key_bytes, &updated);
                                }
                                pkg_overlay.insert(key_str.to_string(), rec);
                            }
                        }
                    }
                }
                for (canonical, rec) in pkg_overlay.iter_mut() {
                    if canonical.starts_with(canonical_prefix)
                        && rec.publisher_pubkey == *old_pubkey
                    {
                        rec.publisher_pubkey = new_pubkey.clone();
                        if let Ok(updated) = serde_json::to_vec(rec) {
                            batch.put_cf(&cf_pkg, canonical.as_bytes(), &updated);
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
                if should_update_pub_index {
                    let idx_prefix = format!("{PUB_IDX_PKG_PREFIX}{canonical_prefix}");
                    for item in self
                        .db
                        .prefix_iterator_cf(&cf_pub_index, idx_prefix.as_bytes())
                    {
                        let (key_bytes, val_bytes) = item?;
                        if !key_bytes.starts_with(idx_prefix.as_bytes()) {
                            break;
                        }
                        if let Ok(canonical) =
                            std::str::from_utf8(&key_bytes[PUB_IDX_PKG_PREFIX.len()..])
                        {
                            if !pub_canon_overlay.contains_key(canonical)
                                && val_bytes.as_ref() == old_pubkey.as_bytes()
                            {
                                pub_canon_overlay.insert(canonical.to_string(), new_pubkey.clone());
                            }
                        }
                    }
                    for (canonical, pubkey) in pub_canon_overlay.iter_mut() {
                        if canonical.starts_with(canonical_prefix) && pubkey == old_pubkey {
                            *pubkey = new_pubkey.clone();
                        }
                    }
                    let old_stats = if let Some(stats) = pub_stats_overlay.remove(old_pubkey) {
                        Some(stats)
                    } else if !pub_stats_deleted.contains(old_pubkey) {
                        let key = format!("{PUB_IDX_STATS_PREFIX}{old_pubkey}");
                        self.db
                            .get_cf(&cf_pub_index, key.as_bytes())?
                            .and_then(|b| serde_json::from_slice(&b).ok())
                    } else {
                        None
                    };
                    if let Some(mut stats) = old_stats {
                        pub_stats_deleted.insert(old_pubkey.clone());
                        pub_stats_deleted.remove(new_pubkey);
                        stats.pubkey = new_pubkey.clone();
                        pub_stats_overlay.insert(new_pubkey.clone(), stats);
                    }
                }
                let record = PublisherRotationIndex {
                    nonce: *nonce,
                    timestamp: Some(block.header.timestamp),
                };
                batch.put_cf(
                    &cf_publisher_rotation,
                    old_pubkey.as_bytes(),
                    &serde_json::to_vec(&record)?,
                );
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

        for (namespace, state) in namespace_effects.controllers {
            batch.put_cf(
                &cf_namespace_controllers,
                namespace.as_bytes(),
                serde_json::to_vec(&state)?,
            );
        }
        for (key, delegation) in namespace_effects.delegations {
            batch.put_cf(
                &cf_namespace_delegations,
                key.as_bytes(),
                serde_json::to_vec(&delegation)?,
            );
        }
        for key in namespace_effects.publisher_nonce_keys {
            batch.put_cf(
                &cf_applied_publisher_authorization_nonces,
                key.as_bytes(),
                b"",
            );
        }
        for key in namespace_effects.lifecycle_nonce_keys {
            batch.put_cf(&cf_applied_namespace_lifecycle_nonces, key.as_bytes(), b"");
        }
        if should_update_pub_index {
            for pubkey in pub_stats_deleted {
                let key = format!("{PUB_IDX_STATS_PREFIX}{pubkey}");
                batch.delete_cf(&cf_pub_index, key.as_bytes());
            }
            for (pubkey, stats) in pub_stats_overlay {
                let key = format!("{PUB_IDX_STATS_PREFIX}{pubkey}");
                batch.put_cf(&cf_pub_index, key.as_bytes(), &serde_json::to_vec(&stats)?);
            }
            for (canonical, pubkey) in pub_canon_overlay {
                let key = format!("{PUB_IDX_PKG_PREFIX}{canonical}");
                batch.put_cf(&cf_pub_index, key.as_bytes(), pubkey.as_bytes());
            }
            batch.put_cf(&cf_pub_index, PUB_IDX_TIP_KEY, height_key);
        }

        // Inclusion and queue acknowledgement share one synchronous commit.
        self.db.write_opt(batch, &Self::reorg_write_options())?;

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
        if offset > tip || limit == 0 {
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
        let _mutation = self.ensure_no_active_reorg()?;
        let cf = self.db.cf_handle(CF_PACKAGES).context("cf packages")?;
        let namespace_cf = self
            .db
            .cf_handle(CF_NAMESPACE_CONTROLLERS)
            .context("cf namespace_controllers")?;
        // Keep direct package-store users (including maintenance tooling) on
        // the same first-publisher ownership rule as block insertion.  Never
        // overwrite an established controller here; lifecycle events are the
        // only controller-change path.
        let namespace = common::package_namespace_prefix(&record.id);
        let establishes_controller = self
            .db
            .get_cf(&namespace_cf, namespace.as_bytes())?
            .is_none();
        if establishes_controller
            && self
                .namespace_consensus_config()?
                .activation_height
                .is_some()
        {
            anyhow::bail!(
                "direct package storage cannot establish a post-activation namespace controller"
            );
        }
        let bytes = serde_json::to_vec(record)?;
        self.db
            .put_cf(&cf, record.id.canonical().as_bytes(), &bytes)?;
        if establishes_controller {
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
                if let Ok(state) = serde_json::from_slice::<
                    crate::namespace_consensus::NamespaceControllerState,
                >(&controller)
                {
                    if !state.root_ed25519_pubkey.is_empty() {
                        return Some(state.root_ed25519_pubkey);
                    }
                }
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

    pub fn namespace_controller_state(
        &self,
        namespace: &str,
    ) -> Result<Option<crate::namespace_consensus::NamespaceControllerState>> {
        let cf = self
            .db
            .cf_handle(CF_NAMESPACE_CONTROLLERS)
            .context("cf namespace_controllers")?;
        let Some(bytes) = self.db.get_cf(&cf, namespace.as_bytes())? else {
            return Ok(None);
        };
        if let Ok(state) = serde_json::from_slice(&bytes) {
            return Ok(Some(state));
        }
        let controller = String::from_utf8(bytes.to_vec())
            .context("legacy namespace controller is not UTF-8")?;
        if controller.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(
            crate::namespace_consensus::NamespaceControllerState::legacy(namespace, &controller),
        ))
    }

    pub fn namespace_delegation(
        &self,
        key: &str,
    ) -> Result<Option<crate::namespace_consensus::NamespaceDelegationState>> {
        let cf = self
            .db
            .cf_handle(CF_NAMESPACE_DELEGATIONS)
            .context("cf namespace_delegations")?;
        self.db
            .get_cf(&cf, key.as_bytes())?
            .map(|bytes| serde_json::from_slice(&bytes).context("decode namespace delegation"))
            .transpose()
    }

    pub fn is_publisher_authorization_nonce_applied(&self, key: &str) -> Result<bool> {
        let cf = self
            .db
            .cf_handle(CF_APPLIED_PUBLISHER_AUTHORIZATION_NONCES)
            .context("cf applied_publisher_authorization_nonces")?;
        Ok(self.db.get_cf(&cf, key.as_bytes())?.is_some())
    }

    pub fn is_namespace_lifecycle_nonce_applied(&self, key: &str) -> Result<bool> {
        let cf = self
            .db
            .cf_handle(CF_APPLIED_NAMESPACE_LIFECYCLE_NONCES)
            .context("cf applied_namespace_lifecycle_nonces")?;
        Ok(self.db.get_cf(&cf, key.as_bytes())?.is_some())
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
        let _mutation = self.ensure_no_active_reorg()?;
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
        let _mutation = self.ensure_no_active_reorg()?;
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
        let _mutation = self.ensure_no_active_reorg()?;
        let work_id = transaction_hash(tx);
        let cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        if self.db.get_cf(&cf, work_id.as_bytes())?.is_some() {
            return Ok(false);
        }
        let dead_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK_DEAD_LETTER)
            .context("cf finalized_work_dead_letter")?;
        if self.db.get_cf(&dead_cf, work_id.as_bytes())?.is_some() {
            return Ok(false);
        }
        let envelope = FinalizedWorkEnvelope {
            schema_version: 2,
            transaction: tx.clone(),
            enqueued_at: chrono::Utc::now().to_rfc3339(),
            submitted_at_height: Some(self.tip_height()?),
            retry_count: 0,
            last_error: None,
            next_attempt_at: None,
        };
        let encoded = serde_json::to_vec(&envelope).context("serialize finalized work")?;
        self.db.put_cf_opt(
            &cf,
            work_id.as_bytes(),
            encoded,
            &Self::reorg_write_options(),
        )?;
        Ok(true)
    }

    /// Atomically reserve a lifecycle nonce and persist its finalized work.
    /// This prevents a storage failure from burning a nonce before the event is
    /// recoverable by the block producer after restart.
    pub fn enqueue_namespace_lifecycle_work(
        &self,
        tx: &Transaction,
        namespace: &str,
        current_controller_pubkey: &str,
        nonce: u64,
    ) -> Result<bool> {
        let _mutation = self.ensure_no_active_reorg()?;
        let nonce_key = namespace_lifecycle_nonce_key(namespace, current_controller_pubkey, nonce)?;
        let _guard = self
            .publisher_admission_nonce_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("publisher admission nonce lock is poisoned"))?;
        let nonce_cf = self
            .db
            .cf_handle(CF_NAMESPACE_LIFECYCLE_NONCES)
            .context("cf namespace_lifecycle_nonces")?;
        if self.db.get_cf(&nonce_cf, nonce_key.as_bytes())?.is_some() {
            return Ok(false);
        }

        let work_id = transaction_hash(tx);
        let work_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let envelope = FinalizedWorkEnvelope {
            schema_version: 2,
            transaction: tx.clone(),
            enqueued_at: chrono::Utc::now().to_rfc3339(),
            submitted_at_height: Some(self.tip_height()?),
            retry_count: 0,
            last_error: None,
            next_attempt_at: None,
        };
        let encoded = serde_json::to_vec(&envelope).context("serialize finalized work")?;
        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(&nonce_cf, nonce_key.as_bytes(), b"");
        batch.put_cf(&work_cf, work_id.as_bytes(), encoded);
        self.db.write_opt(batch, &Self::reorg_write_options())?;
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
                        submitted_at_height: None,
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

    /// Reconcile operational finalized work after a candidate branch has been
    /// fully inserted but before its reorg journal is committed.
    ///
    /// The pre-reorg queue is read from the synced journal snapshot, so retry
    /// metadata survives. Work included by the new canonical branch is
    /// removed, while eligible transactions found only on the abandoned
    /// branch are durably requeued. Governance/validator-set transactions are
    /// deliberately not synthesized into local work because this queue only
    /// accepts independently authenticated package and namespace operations.
    pub(crate) fn reconcile_finalized_work_after_reorg(
        &self,
        guard: &ReorgMutationGuard,
        abandoned_branch: &[Block],
        canonical_transaction_hashes: &HashSet<String>,
    ) -> Result<Vec<(Transaction, u64)>> {
        self.validate_reorg_guard(guard)?;

        let mut desired = BTreeMap::<String, Vec<u8>>::new();
        for (work_id, value) in self.load_reorg_finalized_work_snapshot()? {
            let work_id = String::from_utf8(work_id).context("invalid reorg work identifier")?;
            let envelope = Self::decode_finalized_work(&value)?;
            if transaction_hash(&envelope.transaction) != work_id {
                anyhow::bail!("reorg finalized-work snapshot is internally inconsistent");
            }
            if !canonical_transaction_hashes.contains(&work_id) {
                desired.insert(work_id, value);
            }
        }

        for block in abandoned_branch {
            for transaction in &block.transactions {
                if !is_reorg_requeueable_finalized_work(transaction) {
                    continue;
                }
                let work_id = transaction_hash(transaction);
                if canonical_transaction_hashes.contains(&work_id) {
                    continue;
                }
                let envelope = FinalizedWorkEnvelope {
                    schema_version: 2,
                    transaction: transaction.clone(),
                    // The original submission height/time predates inclusion
                    // but is unavailable after acknowledgement. The abandoned
                    // block timestamp is a deterministic conservative bound.
                    enqueued_at: block.header.timestamp.to_rfc3339(),
                    submitted_at_height: Some(block.header.height),
                    retry_count: 0,
                    last_error: None,
                    next_attempt_at: None,
                };
                desired
                    .entry(work_id)
                    .or_insert(serde_json::to_vec(&envelope)?);
            }
        }

        let cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let current_keys: Vec<Box<[u8]>> = self
            .db
            .iterator_cf(&cf, rocksdb::IteratorMode::Start)
            .map(|item| item.map(|(key, _)| key))
            .collect::<std::result::Result<_, _>>()?;
        let mut batch = rocksdb::WriteBatch::default();
        for key in current_keys {
            batch.delete_cf(&cf, key);
        }
        for (work_id, value) in &desired {
            batch.put_cf(&cf, work_id.as_bytes(), value);
        }
        self.db
            .write_opt(batch, &Self::reorg_write_options())
            .context("persist reconciled finalized work before committing reorg")?;

        let fallback_height = self.tip_height()?;
        desired
            .into_values()
            .map(|value| {
                let envelope = Self::decode_finalized_work(&value)?;
                Ok((
                    envelope.transaction,
                    envelope.submitted_at_height.unwrap_or(fallback_height),
                ))
            })
            .collect()
    }

    /// Return durable work with the original block-height deadline anchor used
    /// to rebuild the in-memory forced-inclusion tracker after restart.
    pub(crate) fn finalized_work_for_tracker(
        &self,
        fallback_height: u64,
    ) -> Result<Vec<(Transaction, u64)>> {
        let cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let mut work = Vec::new();
        for item in self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
            let (_, value) = item?;
            let envelope = Self::decode_finalized_work(&value)?;
            work.push((
                envelope.transaction,
                envelope.submitted_at_height.unwrap_or(fallback_height),
            ));
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
                if cfg!(feature = "phase-c-real-fleet")
                    && id != transaction_hash(&envelope.transaction)
                {
                    anyhow::bail!("finalized work key {id} does not match its Phase C transaction; preserve this store for recovery");
                }
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

    /// Best-effort notification and forced delivery obey the same durable
    /// retry deadline as hydration. Missing records remain valid for work
    /// received only through gossip. Explicit reorg requeue creates an active
    /// envelope and takes precedence over older dead-letter evidence.
    pub fn is_finalized_work_deliverable(&self, transaction: &Transaction) -> Result<bool> {
        let work_id = transaction_hash(transaction);
        let work_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        if let Some(bytes) = self.db.get_cf(&work_cf, work_id.as_bytes())? {
            let envelope = Self::decode_finalized_work(&bytes)?;
            if cfg!(feature = "phase-c-real-fleet")
                && work_id != transaction_hash(&envelope.transaction)
            {
                anyhow::bail!("finalized work payload does not match its Phase C key");
            }
            return Self::is_finalized_work_ready(&envelope, Utc::now());
        }
        let dead_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK_DEAD_LETTER)
            .context("cf finalized_work_dead_letter")?;
        Ok(self.db.get_cf(&dead_cf, work_id.as_bytes())?.is_none())
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
        let _mutation = self.ensure_no_active_reorg()?;
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

    pub fn transparency_event(&self, sequence: u64) -> Result<Option<TransparencyEvent>> {
        let cf = self
            .db
            .cf_handle(CF_TRANSPARENCY_EVENTS)
            .context("cf transparency_events")?;
        match self.db.get_cf(&cf, sequence.to_be_bytes())? {
            None => Ok(None),
            Some(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).context("decode transparency event")?,
            )),
        }
    }

    /// Latest event for an exact canonical subject. The matching tree head is
    /// still unsigned until independently witnessed; clients must not treat
    /// this lookup as install-eligible on its own.
    pub fn transparency_latest_event_for_subject(
        &self,
        log_id: &str,
        subject: &str,
    ) -> Result<Option<TransparencyEvent>> {
        let subject = subject.trim();
        if subject.is_empty() {
            anyhow::bail!("transparency subject is required");
        }
        let _head = self.transparency_tree_head(log_id)?;
        let cf = self
            .db
            .cf_handle(CF_TRANSPARENCY_EVENTS)
            .context("cf transparency_events")?;
        let mut latest = None;
        for item in self.db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
            let (_, value) = item?;
            let event: TransparencyEvent =
                serde_json::from_slice(&value).context("decode transparency event")?;
            if event.subject == subject {
                latest = Some(event);
            }
        }
        Ok(latest)
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

    /// Return a read-only, signature-redacted diagnostic view of active and
    /// dead-lettered finalized work. This is used by offline operator tooling;
    /// it never opens the store for mutation or returns publisher signatures.
    pub fn finalized_work_diagnostics(&self) -> Result<serde_json::Value> {
        let active_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let dead_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK_DEAD_LETTER)
            .context("cf finalized_work_dead_letter")?;

        let mut active = Vec::new();
        for item in self
            .db
            .iterator_cf(&active_cf, rocksdb::IteratorMode::Start)
        {
            let (key, value) = item?;
            let work_id = String::from_utf8(key.to_vec()).context("invalid finalized work key")?;
            let envelope = Self::decode_finalized_work(&value)?;
            active.push(serde_json::json!({
                "work_id": work_id,
                "enqueued_at": envelope.enqueued_at,
                "submitted_at_height": envelope.submitted_at_height,
                "retry_count": envelope.retry_count,
                "last_error": envelope.last_error,
                "next_attempt_at": envelope.next_attempt_at,
                "transaction": self.finalized_work_transaction_diagnostics(&envelope.transaction)?,
            }));
        }

        let mut dead_letters = Vec::new();
        for item in self.db.iterator_cf(&dead_cf, rocksdb::IteratorMode::Start) {
            let (key, value) = item?;
            let work_id = String::from_utf8(key.to_vec()).context("invalid dead-letter key")?;
            let record: serde_json::Value = serde_json::from_slice(&value)
                .context("decode finalized-work dead-letter record")?;
            let transaction = record
                .get("transaction")
                .cloned()
                .map(serde_json::from_value::<Transaction>)
                .transpose()
                .context("decode dead-letter transaction")?;
            dead_letters.push(serde_json::json!({
                "work_id": work_id,
                "reason": record.get("reason"),
                "terminal_reason": record.get("terminal_reason"),
                "dead_lettered_at": record.get("dead_lettered_at"),
                "enqueued_at": record.get("enqueued_at"),
                "retry_count": record.get("retry_count"),
                "last_error": record.get("last_error"),
                "source": record.get("source"),
                "transaction": transaction
                    .as_ref()
                    .map(|tx| self.finalized_work_transaction_diagnostics(tx))
                    .transpose()?,
            }));
        }

        Ok(serde_json::json!({
            "schema": "creg-finalized-work-diagnostics-v1",
            "mode": "read-only",
            "publisher_signatures_included": false,
            "active_count": active.len(),
            "dead_letter_count": dead_letters.len(),
            "active": active,
            "dead_letters": dead_letters,
        }))
    }

    fn finalized_work_transaction_diagnostics(
        &self,
        transaction: &Transaction,
    ) -> Result<serde_json::Value> {
        let mut summary = finalized_work_transaction_summary(transaction);
        if let Transaction::PublishAuthorizedV1 { authorization, .. } = transaction {
            let nonce_key = crate::namespace_consensus::publisher_authorization_nonce_key(
                &authorization.publisher_address,
                &authorization.nonce,
            );
            let now = Utc::now().timestamp();
            summary["authorization_nonce_applied"] =
                serde_json::json!(self.is_publisher_authorization_nonce_applied(&nonce_key)?);
            summary["authorization_expired_now"] =
                serde_json::json!(i128::from(authorization.expires_at) <= i128::from(now));
            summary["authorization_expires_at"] = serde_json::json!(authorization.expires_at);
        }
        Ok(summary)
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
        let _mutation = self.ensure_no_active_reorg()?;
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
        let _mutation = self.ensure_no_active_reorg()?;
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
        self.db.write_opt(batch, &Self::reorg_write_options())?;
        Ok(true)
    }

    /// Preserve an unprepared transaction that cannot be proposed again,
    /// including work delivered only through gossip or forced inclusion.
    /// Callers must not use this to change a quorum-prepared block.
    pub fn dead_letter_unprepared_transaction(
        &self,
        transaction: &Transaction,
        reason: &str,
    ) -> Result<()> {
        let _mutation = self.ensure_no_active_reorg()?;
        let work_id = transaction_hash(transaction);
        let work_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK)
            .context("cf finalized_work")?;
        let dead_cf = self
            .db
            .cf_handle(CF_FINALIZED_WORK_DEAD_LETTER)
            .context("cf finalized_work_dead_letter")?;
        let mut record = if let Some(bytes) = self.db.get_cf(&dead_cf, work_id.as_bytes())? {
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .context("invalid existing dead-letter record")?
        } else if let Some(bytes) = self.db.get_cf(&work_cf, work_id.as_bytes())? {
            let envelope = Self::decode_finalized_work(&bytes)?;
            serde_json::json!({
                "work_id": work_id,
                "reason": reason,
                "dead_lettered_at": Utc::now().to_rfc3339(),
                "enqueued_at": envelope.enqueued_at,
                "retry_count": envelope.retry_count,
                "last_error": envelope.last_error,
                "transaction": envelope.transaction,
            })
        } else {
            serde_json::json!({
                "work_id": work_id,
                "reason": reason,
                "dead_lettered_at": Utc::now().to_rfc3339(),
                "source": "ephemeral_or_forced",
                "transaction": transaction,
            })
        };
        let object = record
            .as_object_mut()
            .context("dead-letter record must be an object")?;
        object.insert("terminal_reason".into(), serde_json::json!(reason));
        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(&dead_cf, work_id.as_bytes(), serde_json::to_vec(&record)?);
        batch.delete_cf(&work_cf, work_id.as_bytes());
        self.db.write_opt(batch, &Self::reorg_write_options())?;
        Ok(())
    }

    /// Record a failed delivery and preserve the distinction between a queued
    /// retry and an item that was never durable work.
    pub fn record_finalized_work_failure(
        &self,
        work_id: &str,
        error: &str,
    ) -> Result<FinalizedWorkFailureOutcome> {
        let _mutation = self.ensure_no_active_reorg()?;
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
            self.db.write_opt(batch, &Self::reorg_write_options())?;
            return Ok(FinalizedWorkFailureOutcome::DeadLettered);
        }
        let delay_secs = (1u64 << envelope.retry_count.min(8)).min(300);
        envelope.next_attempt_at =
            Some((chrono::Utc::now() + chrono::Duration::seconds(delay_secs as i64)).to_rfc3339());
        self.db.put_cf_opt(
            &cf,
            work_id.as_bytes(),
            serde_json::to_vec(&envelope)?,
            &Self::reorg_write_options(),
        )?;
        Ok(FinalizedWorkFailureOutcome::Retrying)
    }

    /// Return the last-used rotation nonce for the given publisher pubkey.
    /// Returns `Some(0)` if no rotation has been recorded. `None` only if the
    /// chain tip cannot be read during a cold-index backfill.
    pub fn publisher_rotation_nonce(&self, pubkey: &str) -> Option<u64> {
        Some(self.publisher_rotation_index(pubkey)?.nonce)
    }

    /// Return the timestamp of the most recent key rotation by this pubkey.
    /// Used to enforce a cooldown period between rotations.
    pub fn publisher_last_rotation_time(
        &self,
        pubkey: &str,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        self.publisher_rotation_index(pubkey)?.timestamp
    }

    fn publisher_rotation_index(&self, pubkey: &str) -> Option<PublisherRotationIndex> {
        if let Some(cf) = self.db.cf_handle(CF_PUBLISHER_ROTATION) {
            if let Ok(Some(bytes)) = self.db.get_cf(&cf, pubkey.as_bytes()) {
                if let Ok(record) = serde_json::from_slice::<PublisherRotationIndex>(&bytes) {
                    return Some(record);
                }
            }
        }

        // Empty/new CF on an upgraded store is not "nonce 0". Scan once, then
        // persist so subsequent lookups are O(1).
        let tip = self.tip_height().ok()?;
        let mut found = None;
        for h in (0..=tip).rev() {
            if let Ok(Some(block)) = self.get_block_by_height(h) {
                for tx in &block.transactions {
                    if let common::Transaction::RotatePublisherKey {
                        old_pubkey, nonce, ..
                    } = tx
                    {
                        if old_pubkey == pubkey {
                            found = Some(PublisherRotationIndex {
                                nonce: *nonce,
                                timestamp: Some(block.header.timestamp),
                            });
                            break;
                        }
                    }
                }
            }
            if found.is_some() {
                break;
            }
        }
        let record = found.unwrap_or(PublisherRotationIndex {
            nonce: 0,
            timestamp: None,
        });
        if let Some(cf) = self.db.cf_handle(CF_PUBLISHER_ROTATION) {
            if let Ok(bytes) = serde_json::to_vec(&record) {
                let _ = self.db.put_cf(&cf, pubkey.as_bytes(), bytes);
            }
        }
        Some(record)
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

        #[allow(clippy::type_complexity)]
        let iter_box: Box<
            dyn Iterator<Item = Result<(Box<[u8]>, Box<[u8]>), rocksdb::Error>>,
        > = if let Some(eco) = ecosystem {
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
                let matches = matches!(
                    (st, &record.status),
                    (PackageStatus::Verified, PackageStatus::Verified)
                        | (PackageStatus::Pending, PackageStatus::Pending)
                        | (PackageStatus::Revoked { .. }, PackageStatus::Revoked { .. })
                );
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
    /// Retained for API compatibility. Direct canonical replacement is now
    /// rejected, so successful inserts always return `None`.
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
        merkle_root, AnalysisBundleRefs, Block, BlockHeader, BlockSignature, ChainRecord,
        DeterministicRiskSummary, NamespaceAuthorityPolicySpec, NamespaceLifecycleAction,
        NamespaceLifecycleEvent, NamespaceLifecycleEventV2, PackageId, Transaction, Validator,
        ValidatorSet,
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
            pgp_signature_sha256: String::new(),
            pgp_public_key_sha256: String::new(),
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
        static NEXT_TEST_BLOCK_SECOND: std::sync::atomic::AtomicI64 =
            std::sync::atomic::AtomicI64::new(0);
        let now_floor = Utc::now().timestamp().saturating_add(1);
        let mut observed = NEXT_TEST_BLOCK_SECOND.load(std::sync::atomic::Ordering::SeqCst);
        let timestamp = loop {
            let candidate = now_floor.max(observed.saturating_add(1));
            match NEXT_TEST_BLOCK_SECOND.compare_exchange(
                observed,
                candidate,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            ) {
                Ok(_) => {
                    break DateTime::from_timestamp(candidate, 0)
                        .expect("test block timestamp is representable");
                }
                Err(current) => observed = current,
            }
        };
        Block {
            header: BlockHeader {
                height,
                prev_hash: prev_hash.to_string(),
                merkle_root: merkle_root(&transactions),
                proposer_id: proposer.into(),
                timestamp,
                validator_set_hash: "test".into(),
                vrf_output: None,
                vrf_proof: None,
                view_number: 0,
                view_change_certificate: vec![],
                status_root: None,
            },
            transactions,
            pbft_signatures: vec![],
        }
    }

    fn validator_set(id: &str, key_byte: u8) -> ValidatorSet {
        ValidatorSet::new(vec![Validator {
            id: id.into(),
            alias: id.into(),
            pubkey: hex::encode([key_byte; 32]),
            status: "online".into(),
            ..Validator::default()
        }])
    }

    fn signed_block_certificate(
        block: &Block,
        signers: impl IntoIterator<Item = (String, SigningKey)>,
    ) -> Vec<BlockSignature> {
        let block_hash = block.hash();
        signers
            .into_iter()
            .map(|(validator_id, key)| BlockSignature {
                validator_id,
                pubkey: hex::encode(key.verifying_key().as_bytes()),
                signature: hex::encode(key.sign(block_hash.as_bytes()).to_bytes()),
                pq_signature: None,
            })
            .collect()
    }

    #[test]
    fn direct_same_height_replacement_is_rejected_without_mutating_canonical_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("tip hash");

        let signing_keys: Vec<_> = (1u8..=4)
            .map(|seed| SigningKey::from_bytes(&[seed; 32]))
            .collect();
        let mut block_a = block(1, &genesis_hash, "node-a", &["pkg-a"]);
        block_a.pbft_signatures = signed_block_certificate(
            &block_a,
            signing_keys
                .iter()
                .take(3)
                .enumerate()
                .map(|(index, key)| (format!("validator-{index}"), key.clone())),
        );
        let outcome_a = store.insert_block_with_outcome(&block_a).expect("insert a");
        assert!(outcome_a.replaced_hash.is_none(), "first insert is no fork");
        let cf_hash = store
            .db
            .cf_handle(CF_BLOCKS_BY_HASH)
            .expect("blocks-by-hash cf");
        let first_stored_bytes = store
            .db
            .get_cf(&cf_hash, block_a.hash().as_bytes())
            .expect("read first certificate encoding")
            .expect("first certificate stored");

        // Re-inserting the identical block is not a replacement.
        let outcome_same = store
            .insert_block_with_outcome(&block_a)
            .expect("re-insert");
        assert!(outcome_same.replaced_hash.is_none());

        // Signature order and the exact honest quorum subset are not committed
        // by the block hash. Both variants are idempotent, and neither may
        // overwrite the first accepted certificate/encoding.
        let mut reordered_certificate = block_a.clone();
        reordered_certificate.pbft_signatures.reverse();
        store
            .insert_block_with_outcome(&reordered_certificate)
            .expect("reordered valid certificate is the same canonical payload");
        let mut alternate_certificate = block_a.clone();
        alternate_certificate.pbft_signatures = signed_block_certificate(
            &alternate_certificate,
            signing_keys
                .iter()
                .enumerate()
                .skip(1)
                .map(|(index, key)| (format!("validator-{index}"), key.clone())),
        );
        store
            .insert_block_with_outcome(&alternate_certificate)
            .expect("alternate valid quorum subset is the same canonical payload");
        assert_eq!(
            store
                .db
                .get_cf(&cf_hash, block_a.hash().as_bytes())
                .expect("read retained certificate encoding")
                .expect("certificate remains stored"),
            first_stored_bytes,
            "certificate-only replay must retain the first stored bytes"
        );

        // Block hashes commit only to the header. A peer must not be able to
        // replace the stored transaction body while retaining that header hash.
        let mut same_hash_tamper = block_a.clone();
        same_hash_tamper.transactions =
            vec![Transaction::Publish(publish_record("same-hash-tamper"))];
        assert_eq!(same_hash_tamper.hash(), block_a.hash());
        store
            .insert_block_with_outcome(&same_hash_tamper)
            .expect_err("same-header body replacement must fail closed");
        assert!(store
            .get_package("npm:same-hash-tamper@1.0.0")
            .expect("read rejected same-hash package")
            .is_none());

        // A different block at the same height must go through the explicit,
        // full-branch reorg path rather than silently overwriting finality.
        let block_b = block(1, &genesis_hash, "node-b", &["pkg-b"]);
        let error = store
            .insert_block_with_outcome(&block_b)
            .expect_err("direct replacement must fail closed");
        assert!(
            error.to_string().contains("preflighted reorg path"),
            "unexpected error: {error}"
        );
        assert_eq!(store.tip_hash().expect("tip"), block_a.hash());
        assert!(store
            .get_package("npm:pkg-a@1.0.0")
            .expect("read canonical package")
            .is_some());
        assert!(store
            .get_package("npm:pkg-b@1.0.0")
            .expect("read rejected package")
            .is_none());
    }

    #[test]
    fn canonical_insert_rejects_gaps_wrong_parents_non_monotonic_time_and_stale_merkle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis = store
            .get_block_by_height(0)
            .expect("read genesis")
            .expect("genesis");
        let first = block(1, &genesis.hash(), "node-a", &["first"]);
        store.insert_block(&first).expect("insert first");

        let gap = block(3, &first.hash(), "node-a", &["gap"]);
        assert!(store
            .insert_block(&gap)
            .expect_err("height gap must fail closed")
            .to_string()
            .contains("canonical parent block 2 is missing"));

        let wrong_parent = block(2, &genesis.hash(), "node-a", &["wrong-parent"]);
        assert!(store
            .insert_block(&wrong_parent)
            .expect_err("wrong parent must fail closed")
            .to_string()
            .contains("does not descend from canonical parent"));

        let mut non_monotonic = block(2, &first.hash(), "node-a", &["non-monotonic"]);
        non_monotonic.header.timestamp = first.header.timestamp;
        let error = store
            .insert_block(&non_monotonic)
            .expect_err("non-monotonic timestamp must fail closed");
        assert!(
            format!("{error:#}").contains("strictly later"),
            "unexpected error: {error:#}"
        );

        let mut stale_merkle = block(2, &first.hash(), "node-a", &["original-body"]);
        stale_merkle.transactions = vec![Transaction::Publish(publish_record("tampered-body"))];
        assert!(store
            .insert_block(&stale_merkle)
            .expect_err("stale transaction commitment must fail closed")
            .to_string()
            .contains("merkle root"));

        let valid_child = block(2, &first.hash(), "node-a", &["valid"]);
        store
            .insert_block(&valid_child)
            .expect("strictly ordered child");
        store
            .insert_block(&valid_child)
            .expect("idempotent occupied-height replay still checks its parent");

        // Historical age is not a live-admission rule: a strictly ordered
        // 2024 chain remains valid when replayed years later.
        let historical_dir = tempfile::tempdir().expect("historical tempdir");
        let historical_store = ChainStore::open(historical_dir.path()).expect("historical store");
        let historical_genesis = historical_store
            .get_block_by_height(0)
            .expect("read historical genesis")
            .expect("historical genesis");
        let mut historical_one =
            block(1, &historical_genesis.hash(), "node-a", &["historical-one"]);
        historical_one.header.timestamp =
            historical_genesis.header.timestamp + chrono::Duration::seconds(1);
        historical_store
            .insert_block(&historical_one)
            .expect("historical first child");
        let mut historical_two = block(2, &historical_one.hash(), "node-a", &["historical-two"]);
        historical_two.header.timestamp =
            historical_one.header.timestamp + chrono::Duration::seconds(1);
        historical_store
            .insert_block(&historical_two)
            .expect("historical second child");
    }

    #[test]
    fn post_rollback_same_hash_body_replacement_is_rejected_but_semantic_replay_succeeds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let original = block(1, &genesis_hash, "node-a", &["original"]);
        store.insert_block(&original).expect("insert original");
        let original_encoding = serde_json::to_vec(&original).expect("encode original");
        let alternate_encoding =
            serde_json::to_vec_pretty(&original).expect("encode alternate whitespace");
        assert_ne!(
            alternate_encoding, original_encoding,
            "test requires two semantically equivalent JSON encodings"
        );
        let cf_hash = store
            .db
            .cf_handle(CF_BLOCKS_BY_HASH)
            .expect("blocks-by-hash cf");
        store
            .db
            .put_cf(&cf_hash, original.hash().as_bytes(), &alternate_encoding)
            .expect("install legacy-equivalent encoding");

        store.rollback_to_height(0).expect("rollback original");
        store
            .rebuild_package_index()
            .expect("rebuild rolled-back state");
        assert_eq!(store.tip_height().expect("tip"), 0);

        let mut same_hash_tamper = original.clone();
        same_hash_tamper.transactions =
            vec![Transaction::Publish(publish_record("post-rollback-tamper"))];
        assert_eq!(same_hash_tamper.hash(), original.hash());
        let error = store
            .insert_block(&same_hash_tamper)
            .expect_err("hash-indexed body replacement must fail after rollback");
        assert!(
            error.to_string().contains("merkle root"),
            "unexpected error: {error}"
        );
        assert_eq!(store.tip_height().expect("tip after rejection"), 0);
        assert_eq!(
            serde_json::to_value(
                store
                    .get_block_by_hash(&original.hash())
                    .expect("read original by hash")
                    .expect("original remains inspectable"),
            )
            .expect("serialize stored original"),
            serde_json::to_value(&original).expect("serialize expected original"),
        );
        assert!(store
            .get_package("npm:post-rollback-tamper@1.0.0")
            .expect("read rejected package")
            .is_none());

        // Re-adopting a semantically identical historical body remains
        // idempotent even when its persisted JSON map/field order differs.
        store
            .insert_block(&original)
            .expect("restore semantically identical historical block");
        assert_eq!(store.tip_hash().expect("restored tip"), original.hash());
        assert_eq!(
            store
                .db
                .get_cf(&cf_hash, original.hash().as_bytes())
                .expect("read original encoding")
                .expect("original encoding remains"),
            alternate_encoding,
            "idempotent replay must not rewrite the first accepted encoding"
        );
    }

    #[test]
    fn incomplete_reorg_journal_restores_old_branch_after_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let prefix = block(1, &genesis_hash, "prefix", &["prefix"]);
        store.insert_block(&prefix).expect("insert prefix");
        let old_tip = block(2, &prefix.hash(), "old", &["old"]);
        store.insert_block(&old_tip).expect("insert old tip");
        crate::validator_set_history::record(dir.path(), 0, &validator_set("old-set", 11))
            .expect("seed old validator history");
        let old_history = std::fs::read(dir.path().join("validator-set-history.json"))
            .expect("read old validator history");

        let reorg_guard = store
            .begin_reorg_journal(1, std::slice::from_ref(&old_tip))
            .expect("persist old branch");
        store
            .rollback_to_height_for_reorg(&reorg_guard, 1)
            .expect("rewind old branch");
        store
            .rebuild_package_index_for_reorg(&reorg_guard)
            .expect("rebuild common-prefix state");

        let candidate_two = block(2, &prefix.hash(), "candidate-two", &["candidate-two"]);
        store
            .insert_block_for_reorg(&reorg_guard, &candidate_two)
            .expect("insert candidate block two");
        let candidate_three = block(
            3,
            &candidate_two.hash(),
            "candidate-three",
            &["candidate-three"],
        );
        store
            .insert_block_for_reorg(&reorg_guard, &candidate_three)
            .expect("insert candidate block three");
        crate::validator_set_history::record(dir.path(), 3, &validator_set("candidate-set", 12))
            .expect("simulate candidate validator-history import");
        assert_ne!(
            std::fs::read(dir.path().join("validator-set-history.json"))
                .expect("read candidate validator history"),
            old_history
        );
        drop(reorg_guard);
        drop(store);

        // Opening the database restores the prior canonical height mapping
        // before any caller can observe an uncommitted candidate tip.
        let recovered = ChainStore::open(dir.path()).expect("reopen after interrupted reorg");
        assert_eq!(recovered.tip_hash().expect("restored tip"), old_tip.hash());
        recovered
            .configure_namespace_consensus(Default::default())
            .expect("rebuild recovered derived state");
        assert!(recovered
            .get_package("npm:old@1.0.0")
            .expect("read restored package")
            .is_some());
        assert!(recovered
            .get_package("npm:candidate-two@1.0.0")
            .expect("read abandoned candidate package")
            .is_none());
        assert!(recovered
            .get_package("npm:candidate-three@1.0.0")
            .expect("read abandoned candidate package")
            .is_none());
        assert!(recovered
            .load_reorg_journal()
            .expect("read cleared journal")
            .is_none());
        assert_eq!(
            std::fs::read(dir.path().join("validator-set-history.json"))
                .expect("read restored validator history"),
            old_history,
            "startup recovery must restore authority history with the old canonical branch"
        );
        drop(recovered);

        // Recovery is durable and does not replay after the journal is
        // cleared.
        let reopened = ChainStore::open(dir.path()).expect("reopen recovered store");
        reopened
            .configure_namespace_consensus(Default::default())
            .expect("configure recovered store");
        assert_eq!(
            reopened.tip_hash().expect("durable old tip"),
            old_tip.hash()
        );
    }

    #[test]
    fn reorg_begin_excludes_mutation_before_the_durable_journal_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let old_tip = block(1, &genesis_hash, "old", &["old-tip"]);
        store.insert_block(&old_tip).expect("insert old tip");

        let lock_acquired = Arc::new(std::sync::Barrier::new(2));
        let resume = Arc::new(std::sync::Barrier::new(2));
        *store.reorg_begin_test_hook.lock().expect("test hook lock") = Some(ReorgBeginTestHook {
            lock_acquired: Arc::clone(&lock_acquired),
            resume: Arc::clone(&resume),
        });

        let begin_store = store.clone();
        let begin_old_tip = old_tip.clone();
        let begin = std::thread::spawn(move || {
            begin_store.begin_reorg_journal(0, std::slice::from_ref(&begin_old_tip))
        });

        // The reorg owns the in-memory mutation lock, but its durable latch is
        // deliberately not written yet. This is the precise historical
        // check-then-write window.
        lock_acquired.wait();
        assert!(store
            .load_reorg_journal()
            .expect("journal status while begin is paused")
            .is_none());

        let queued = Transaction::Publish(publish_record("must-not-slip"));
        let mutation_store = store.clone();
        let (mutation_result_tx, mutation_result_rx) = std::sync::mpsc::channel();
        let mutation = std::thread::spawn(move || {
            mutation_result_tx
                .send(mutation_store.enqueue_finalized_work(&queued))
                .expect("send mutation result");
        });
        let mutation_result = mutation_result_rx.recv_timeout(std::time::Duration::from_secs(5));

        // Always release the paused begin thread before asserting so a failed
        // regression test cannot strand a worker.
        resume.wait();
        let reorg_guard = begin
            .join()
            .expect("join reorg begin")
            .expect("begin reorg");
        let mutation_result = match mutation_result {
            Ok(result) => result,
            Err(error) => {
                store
                    .complete_reorg_journal(&reorg_guard)
                    .expect("release timed-out reorg");
                drop(reorg_guard);
                mutation.join().expect("join timed-out ordinary mutation");
                panic!(
                    "ordinary mutation did not fail promptly while reorg owned the store: {error}"
                );
            }
        };
        mutation.join().expect("join ordinary mutation");
        let error = mutation_result
            .expect_err("ordinary mutation must not slip before journal persistence");
        assert!(error.to_string().contains("reorg-recovery maintenance"));
        assert!(store
            .pending_finalized_work()
            .expect("pending work")
            .is_empty());

        *store.reorg_begin_test_hook.lock().expect("test hook lock") = None;
        store
            .complete_reorg_journal(&reorg_guard)
            .expect("complete reorg");
    }

    #[test]
    fn completed_reorg_journal_preserves_candidate_after_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let prefix = block(1, &genesis_hash, "prefix", &["prefix"]);
        store.insert_block(&prefix).expect("insert prefix");
        let old_tip = block(2, &prefix.hash(), "old", &["old"]);
        store.insert_block(&old_tip).expect("insert old tip");
        crate::validator_set_history::record(dir.path(), 0, &validator_set("old-set", 21))
            .expect("seed old validator history");

        let reorg_guard = store
            .begin_reorg_journal(1, std::slice::from_ref(&old_tip))
            .expect("persist old branch");
        store
            .rollback_to_height_for_reorg(&reorg_guard, 1)
            .expect("rewind old branch");
        store
            .rebuild_package_index_for_reorg(&reorg_guard)
            .expect("rebuild common-prefix state");
        let candidate_two = block(2, &prefix.hash(), "candidate-two", &["candidate-two"]);
        store
            .insert_block_for_reorg(&reorg_guard, &candidate_two)
            .expect("insert candidate block two");
        let candidate_three = block(
            3,
            &candidate_two.hash(),
            "candidate-three",
            &["candidate-three"],
        );
        store
            .insert_block_for_reorg(&reorg_guard, &candidate_three)
            .expect("insert candidate block three");
        store
            .rebuild_package_index_for_reorg(&reorg_guard)
            .expect("rebuild candidate state");
        crate::validator_set_history::record(dir.path(), 3, &validator_set("candidate-set", 22))
            .expect("persist candidate validator history");
        store
            .complete_reorg_journal(&reorg_guard)
            .expect("commit candidate branch");
        drop(reorg_guard);
        drop(store);

        let reopened = ChainStore::open(dir.path()).expect("reopen committed store");
        reopened
            .configure_namespace_consensus(Default::default())
            .expect("configure committed store");
        assert_eq!(
            reopened.tip_hash().expect("committed candidate tip"),
            candidate_three.hash()
        );
        assert!(reopened
            .get_package("npm:old@1.0.0")
            .expect("read abandoned old package")
            .is_none());
        assert!(reopened
            .get_package("npm:candidate-two@1.0.0")
            .expect("read committed package")
            .is_some());
        assert!(reopened
            .get_package("npm:candidate-three@1.0.0")
            .expect("read committed package")
            .is_some());
        assert_eq!(
            crate::validator_set_history::set_at(dir.path(), 3)
                .expect("candidate validator history remains")
                .validators[0]
                .id,
            "candidate-set"
        );
    }

    #[test]
    fn failed_reorg_restore_latches_durable_mutation_maintenance() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let old_tip = block(1, &genesis_hash, "old", &["old"]);
        store.insert_block(&old_tip).expect("insert old tip");
        let reorg_guard = store
            .begin_reorg_journal(0, std::slice::from_ref(&old_tip))
            .expect("begin reorg");

        let mut journal = store
            .load_reorg_journal()
            .expect("read journal")
            .expect("active journal");
        journal.validator_history = Some(b"not-valid-history-json".to_vec());
        let journal_cf = store
            .db
            .cf_handle(CF_REORG_JOURNAL)
            .expect("reorg journal cf");
        store
            .db
            .put_cf(
                &journal_cf,
                REORG_JOURNAL_KEY,
                serde_json::to_vec(&journal).expect("encode corrupt fixture"),
            )
            .expect("install corrupt recovery fixture");

        store
            .rollback_to_height_for_reorg(&reorg_guard, 0)
            .expect("simulate interrupted rewind");
        drop(reorg_guard);
        assert!(store.recover_incomplete_reorg().is_err());
        assert!(store
            .load_reorg_journal()
            .expect("journal remains readable")
            .is_some());

        let queued = Transaction::Revoke {
            package_canonical: "npm:maintenance-gated@1.0.0".into(),
            reason: "fixture".into(),
            revoked_by: "validator-1".into(),
            evidence_hash: "fixture".into(),
        };
        assert!(store
            .enqueue_finalized_work(&queued)
            .expect_err("failed recovery must block queue mutation")
            .to_string()
            .contains("reorg-recovery maintenance"));
        assert!(store
            .insert_block(&block(1, &genesis_hash, "candidate", &["candidate"]))
            .expect_err("failed recovery must block canonical mutation")
            .to_string()
            .contains("reorg-recovery maintenance"));
    }

    #[test]
    fn reorg_preflight_rebuilds_branch_relative_namespace_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let config = crate::namespace_consensus::NamespaceConsensusConfig {
            chain_id: "creg-reorg-test".into(),
            genesis_hash: "a".repeat(64),
            staking_contract: format!("0x{}", "1".repeat(40)),
            activation_height: Some(2),
            authority_policies: vec![NamespaceAuthorityPolicySpec {
                authority: common::OwnershipAuthority::NpmTrustedPublisher,
                adapter_id: "test-adapter".into(),
                adapter_version: "1".into(),
                verifier_sha256: "d".repeat(64),
                policy_sha256: "b".repeat(64),
                trust_root_sha256: "c".repeat(64),
            }],
            policy_source_verified: true,
            validator_set_bootstrap: None,
            vote_v4_activation_height: None,
        };
        store
            .configure_namespace_consensus(config.clone())
            .expect("configure namespace consensus");

        let root = SigningKey::from_bytes(&[31; 32]);
        let abandoned_successor = SigningKey::from_bytes(&[32; 32]);
        let candidate_successor = SigningKey::from_bytes(&[33; 32]);
        let root_pubkey = hex::encode(root.verifying_key().as_bytes());
        let namespace = "npm:branch-owned@";
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let mut record = publish_record("branch-owned");
        record.publisher_pubkey = root_pubkey.clone();
        let prefix = block_with_transactions(
            1,
            &genesis_hash,
            "node-a",
            vec![Transaction::Publish(record)],
        );
        store
            .insert_block(&prefix)
            .expect("insert pre-activation claim");

        let timestamp = 1_800_000_000i64;
        let make_transfer = |next: &SigningKey, proposer: &str| {
            let mut event = NamespaceLifecycleEventV2 {
                schema_version: 2,
                chain_id: config.chain_id.clone(),
                genesis_hash: config.genesis_hash.clone(),
                action: NamespaceLifecycleAction::Transfer,
                namespace: namespace.into(),
                current_controller_ed25519_pubkey: root_pubkey.clone(),
                current_controller_evm_address: String::new(),
                next_controller_ed25519_pubkey: hex::encode(next.verifying_key().as_bytes()),
                next_controller_evm_address: format!("0x{}", "2".repeat(40)),
                delegation_scopes: vec![],
                delegation_expires_at: None,
                nonce: 1,
                issued_at: timestamp - 60,
                expires_at: timestamp + 60,
                current_controller_signature: String::new(),
                next_controller_signature: String::new(),
                authority_proof_digest: String::new(),
                authority_certificate: None,
            };
            let message = event.signing_message();
            event.current_controller_signature =
                hex::encode(root.sign(message.as_bytes()).to_bytes());
            event.next_controller_signature = hex::encode(next.sign(message.as_bytes()).to_bytes());
            let mut block = block_with_transactions(
                2,
                &prefix.hash(),
                proposer,
                vec![Transaction::NamespaceLifecycleV2(event)],
            );
            block.header.timestamp =
                DateTime::from_timestamp(timestamp, 0).expect("fixed timestamp");
            block
        };

        let abandoned = make_transfer(&abandoned_successor, "node-a");
        store
            .insert_block(&abandoned)
            .expect("insert abandoned branch");
        let abandoned_controller = hex::encode(abandoned_successor.verifying_key().as_bytes());
        assert_eq!(
            store.publisher_for_prefix(namespace),
            Some(abandoned_controller.clone())
        );

        let candidate = make_transfer(&candidate_successor, "node-b");
        store
            .preflight_reorg_branch(1, std::slice::from_ref(&candidate))
            .expect("candidate validates from common-ancestor state");

        // Preflight is isolated: the abandoned branch remains canonical until
        // sync explicitly adopts the fully validated candidate branch.
        assert_eq!(store.tip_hash().expect("tip"), abandoned.hash());
        assert_eq!(
            store.publisher_for_prefix(namespace),
            Some(abandoned_controller)
        );
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
    fn publisher_rotation_index_is_o1_and_survives_rebuild_and_cf_miss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("tip hash");
        let old_pubkey = "aa".repeat(32);
        let new_pubkey = "bb".repeat(32);
        let rotation_time = Utc::now();
        let rotation = Transaction::RotatePublisherKey {
            canonical_prefix: "npm:rotated-publisher".into(),
            old_pubkey: old_pubkey.clone(),
            new_pubkey,
            sig_from_old: String::new(),
            sig_from_new: String::new(),
            timestamp: rotation_time,
            nonce: 7,
        };
        let block1 = block_with_transactions(1, &genesis_hash, "node-a", vec![rotation]);
        store.insert_block(&block1).expect("insert rotation");

        assert_eq!(store.publisher_rotation_nonce(&old_pubkey), Some(7));
        assert_eq!(
            store.publisher_last_rotation_time(&old_pubkey),
            Some(block1.header.timestamp)
        );
        assert_eq!(
            store.publisher_rotation_nonce("ff".repeat(32).as_str()),
            Some(0)
        );
        assert!(store
            .publisher_last_rotation_time("ff".repeat(32).as_str())
            .is_none());

        // Simulate an upgraded DB whose new CF is empty: drop the indexed key
        // and confirm the first lookup scans once then refills.
        let cf = store
            .db
            .cf_handle(CF_PUBLISHER_ROTATION)
            .expect("publisher_rotation cf");
        store
            .db
            .delete_cf(&cf, old_pubkey.as_bytes())
            .expect("drop indexed rotation");
        assert_eq!(store.publisher_rotation_nonce(&old_pubkey), Some(7));
        assert_eq!(
            store.db.get_cf(&cf, old_pubkey.as_bytes()).expect("read"),
            Some(
                serde_json::to_vec(&PublisherRotationIndex {
                    nonce: 7,
                    timestamp: Some(block1.header.timestamp),
                })
                .expect("encode")
            )
        );

        store.rebuild_package_index().expect("rebuild");
        assert_eq!(store.publisher_rotation_nonce(&old_pubkey), Some(7));
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
    fn failed_reorg_restores_exact_finalized_work_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let old_branch = block(1, &genesis_hash, "old-proposer", &["old-canonical"]);
        store.insert_block(&old_branch).expect("insert old branch");
        crate::validator_set_history::record(dir.path(), 0, &validator_set("old-set", 41))
            .expect("seed old validator history");
        let old_history = std::fs::read(dir.path().join("validator-set-history.json"))
            .expect("read old validator history");

        let pending = Transaction::Revoke {
            package_canonical: "npm:pre-reorg-pending@1.0.0".into(),
            reason: "reorg-test".into(),
            revoked_by: "validator-1".into(),
            evidence_hash: "pending-evidence".into(),
        };
        let pending_id = transaction_hash(&pending);
        store
            .enqueue_finalized_work(&pending)
            .expect("enqueue pending");
        assert_eq!(
            store
                .record_finalized_work_failure(&pending_id, "pre-reorg retry")
                .expect("record retry"),
            FinalizedWorkFailureOutcome::Retrying
        );
        let reorg_guard = store
            .begin_reorg_journal(0, std::slice::from_ref(&old_branch))
            .expect("begin reorg");

        store
            .rollback_to_height_for_reorg(&reorg_guard, 0)
            .expect("rewind old branch");
        store
            .rebuild_package_index_for_reorg(&reorg_guard)
            .expect("rebuild ancestor");
        let candidate = block_with_transactions(
            1,
            &genesis_hash,
            "candidate-proposer",
            vec![pending.clone()],
        );
        assert!(store
            .insert_block(&candidate)
            .expect_err("active reorg journal must maintenance-gate canonical insertion")
            .to_string()
            .contains("reorg-recovery maintenance"));
        store
            .insert_block_for_reorg(&reorg_guard, &candidate)
            .expect("candidate acknowledges pending work");
        crate::validator_set_history::record(
            dir.path(),
            candidate.header.height,
            &validator_set("candidate-set", 42),
        )
        .expect("simulate candidate validator history before terminal fault");
        assert_ne!(
            std::fs::read(dir.path().join("validator-set-history.json"))
                .expect("read candidate validator history"),
            old_history
        );
        let post_journal_work = Transaction::Revoke {
            package_canonical: "npm:must-not-leak@1.0.0".into(),
            reason: "candidate-only".into(),
            revoked_by: "validator-2".into(),
            evidence_hash: "candidate-evidence".into(),
        };
        store
            .enqueue_finalized_work(&post_journal_work)
            .expect_err("active reorg journal must maintenance-gate independent work");

        assert!(store
            .recover_incomplete_reorg_with_guard(&reorg_guard)
            .expect("recover failed reorg using the live guard"));
        assert!(store
            .load_reorg_journal()
            .expect("read cleared reorg journal")
            .is_none());
        assert_eq!(
            std::fs::read(dir.path().join("validator-set-history.json"))
                .expect("read restored validator history"),
            old_history
        );
        drop(reorg_guard);
        assert_eq!(store.tip_hash().expect("restored tip"), old_branch.hash());
        let pending_after = store.pending_finalized_work().expect("restored work");
        assert_eq!(pending_after.len(), 1);
        assert_eq!(pending_after[0].0, pending_id);
        let stats = store.finalized_work_stats().expect("restored retry stats");
        assert_eq!(stats.retrying_depth, 1);
        assert_eq!(stats.retry_attempts, 1);
        let tracker_work = store
            .finalized_work_for_tracker(99)
            .expect("tracker recovery metadata");
        assert_eq!(tracker_work.len(), 1);
        assert_eq!(tracker_work[0].1, 1);
    }

    #[test]
    fn successful_reorg_requeues_only_abandoned_finalized_work() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ChainStore::open(dir.path()).expect("open");
        let genesis_hash = store.tip_hash().expect("genesis hash");
        let revoke = |name: &str| Transaction::Revoke {
            package_canonical: format!("npm:{name}@1.0.0"),
            reason: "reorg-test".into(),
            revoked_by: "validator-1".into(),
            evidence_hash: common::sha256_hex(name.as_bytes()),
        };
        let abandoned_only = revoke("abandoned-only");
        let shared = revoke("shared");
        let governance = Transaction::Slash {
            validator_id: "validator-2".into(),
            amount: 1,
            reason: "fixture".into(),
        };
        let old_branch = block_with_transactions(
            1,
            &genesis_hash,
            "old-proposer",
            vec![abandoned_only.clone(), shared.clone(), governance.clone()],
        );
        store.insert_block(&old_branch).expect("insert old branch");

        let pending_then_included = revoke("pending-then-included");
        store
            .enqueue_finalized_work(&pending_then_included)
            .expect("enqueue pre-reorg work");
        let reorg_guard = store
            .begin_reorg_journal(0, std::slice::from_ref(&old_branch))
            .expect("begin reorg");

        store
            .rollback_to_height_for_reorg(&reorg_guard, 0)
            .expect("rewind old branch");
        store
            .rebuild_package_index_for_reorg(&reorg_guard)
            .expect("rebuild ancestor");
        let candidate = block_with_transactions(
            1,
            &genesis_hash,
            "candidate-proposer",
            vec![shared.clone(), pending_then_included.clone()],
        );
        store
            .insert_block_for_reorg(&reorg_guard, &candidate)
            .expect("insert candidate");
        store
            .rebuild_package_index_for_reorg(&reorg_guard)
            .expect("rebuild candidate");
        let canonical_hashes: HashSet<String> = candidate
            .transactions
            .iter()
            .map(transaction_hash)
            .collect();
        let tracker_work = store
            .reconcile_finalized_work_after_reorg(
                &reorg_guard,
                std::slice::from_ref(&old_branch),
                &canonical_hashes,
            )
            .expect("reconcile finalized work");
        assert_eq!(tracker_work.len(), 1);
        assert_eq!(
            transaction_hash(&tracker_work[0].0),
            transaction_hash(&abandoned_only)
        );
        assert_eq!(tracker_work[0].1, old_branch.header.height);
        store
            .complete_reorg_journal(&reorg_guard)
            .expect("commit reorg");
        drop(reorg_guard);
        drop(store);

        let reopened = ChainStore::open(dir.path()).expect("reopen adopted branch");
        assert_eq!(
            reopened.tip_hash().expect("candidate tip"),
            candidate.hash()
        );
        let pending_after = reopened
            .pending_finalized_work()
            .expect("requeued abandoned work");
        assert_eq!(pending_after.len(), 1);
        assert_eq!(
            pending_after[0].0,
            transaction_hash(&abandoned_only),
            "newly canonical work must remain acknowledged"
        );
        assert_ne!(pending_after[0].0, transaction_hash(&shared));
        assert_ne!(pending_after[0].0, transaction_hash(&governance));
        assert_ne!(pending_after[0].0, transaction_hash(&pending_then_included));
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
        let latest = store
            .transparency_latest_event_for_subject(log_id, "npm:log-test@1.0.0")
            .expect("latest event")
            .expect("present");
        assert_eq!(latest.sequence, 1);
        assert_eq!(latest.kind, "revocation");
        assert_eq!(
            store
                .transparency_event(1)
                .expect("event by sequence")
                .expect("present")
                .kind,
            "revocation"
        );
    }

    #[test]
    fn publisher_index_hydrates_incrementally_and_supports_read_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pubkey = "11".repeat(32);
        let owner_key = common::crypto::SigningKey::from_ed25519_seed(&[11u8; 32]);
        let owner_pub = owner_key.public_key().to_hex();
        assert_eq!(owner_pub.len(), 64);

        {
            let store = ChainStore::open(dir.path()).expect("open rw");
            let genesis = store.get_block_by_height(0).unwrap().unwrap();
            let pkg1 = common::PackageId::new("npm", "idx-a", "1.0.0");
            let rec1 = ChainRecord {
                id: pkg1.clone(),
                content_hash: "a".repeat(64),
                ipfs_cid: "cid-a".into(),
                publisher_pubkey: owner_pub.clone(),
                published_at: DateTime::from_timestamp(1_700_000_005, 0).unwrap(),
                status: PackageStatus::Verified,
                ..Default::default()
            };
            let mut revoke_req = common::SignedRevocationRequest {
                chain_id: "creg-test".into(),
                genesis_hash: genesis.hash(),
                package_canonical: pkg1.canonical(),
                reason: "bad release".into(),
                revoker_pubkey: owner_pub.clone(),
                signature: String::new(),
                signature_version: 1,
            };
            revoke_req.signature = owner_key
                .sign(revoke_req.signing_message().as_bytes())
                .to_hex();
            let b1_txs = vec![Transaction::Publish(rec1)];
            let b1 = Block {
                header: common::BlockHeader {
                    height: 1,
                    prev_hash: genesis.hash(),
                    merkle_root: common::merkle_root(&b1_txs),
                    proposer_id: "v1".into(),
                    timestamp: genesis.header.timestamp + chrono::Duration::seconds(10),
                    validator_set_hash: genesis.header.validator_set_hash.clone(),
                    vrf_output: None,
                    vrf_proof: None,
                    view_number: 0,
                    view_change_certificate: Vec::new(),
                    status_root: None,
                },
                transactions: b1_txs,
                pbft_signatures: vec![],
            };
            store.insert_block(&b1).expect("insert b1");

            let b2_txs = vec![Transaction::RevokeV2 {
                request: revoke_req,
                evidence_hash: pubkey.clone(),
                witness: None,
                quorum_certificate: None,
            }];
            let b2 = Block {
                header: common::BlockHeader {
                    height: 2,
                    prev_hash: b1.hash(),
                    merkle_root: common::merkle_root(&b2_txs),
                    proposer_id: "v1".into(),
                    timestamp: genesis.header.timestamp + chrono::Duration::seconds(20),
                    validator_set_hash: genesis.header.validator_set_hash,
                    vrf_output: None,
                    vrf_proof: None,
                    view_number: 0,
                    view_change_certificate: Vec::new(),
                    status_root: None,
                },
                transactions: b2_txs,
                pbft_signatures: vec![],
            };
            store.insert_block(&b2).expect("insert b2");
            let cf_pub = store.db.cf_handle(CF_PUBLISHER_INDEX).unwrap();
            let tip_val = store.db.get_cf(&cf_pub, PUB_IDX_TIP_KEY).unwrap().unwrap();
            assert_eq!(u64::from_be_bytes(tip_val.try_into().unwrap()), 2);
        }

        // Read-only handle works and reads persisted publisher index at tip 2.
        {
            let ro = ChainStore::open_read_only(dir.path()).expect("open ro");
            let idx = ro.load_or_sync_publisher_index().expect("load ro index");
            let st = idx.get(&owner_pub).expect("stats present");
            assert_eq!(st.total_packages, 1);
            assert_eq!(st.verified_count, 0);
            assert_eq!(st.revoked_count, 1);
        }

        // Simulate restart with missing indexed_tip (legacy backfill) and verify recovery.
        {
            let store = ChainStore::open(dir.path()).expect("reopen rw");
            let cf_pub = store.db.cf_handle(CF_PUBLISHER_INDEX).unwrap();
            store.db.delete_cf(&cf_pub, PUB_IDX_TIP_KEY).unwrap();
            let idx = store.load_or_sync_publisher_index().expect("backfill");
            let st = idx.get(&owner_pub).expect("stats present after backfill");
            assert_eq!(st.total_packages, 1);
            assert_eq!(st.revoked_count, 1);
            let tip_val = store.db.get_cf(&cf_pub, PUB_IDX_TIP_KEY).unwrap().unwrap();
            assert_eq!(u64::from_be_bytes(tip_val.try_into().unwrap()), 2);
        }
    }
}
