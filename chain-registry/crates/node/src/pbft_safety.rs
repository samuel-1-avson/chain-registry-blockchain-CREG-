//! Durable local PBFT signing decisions.
//!
//! This database is separate from the chain store so a chain reorganization
//! or checkpoint restore cannot roll a validator's vote lock backward.

use anyhow::{bail, Context, Result};
use common::{Block, PreparedCertificate, ViewChangeVote};
use ed25519_dalek::{Signature, VerifyingKey};
use rocksdb::{Options, WriteOptions, DB};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Mutex};

const IDENTITY_KEY: &[u8] = b"meta/identity";
const DECISION_PREFIX: &[u8] = b"decision/";
const LOCAL_VIEW_LOCK_PREFIX: &[u8] = b"local-view-lock/";
const PREPARED_PREFIX: &[u8] = b"prepared/";
const RECEIVED_VIEW_CHANGE_PREFIX: &[u8] = b"received-view-change/";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct JournalIdentity {
    schema_version: u8,
    genesis_hash: String,
    validator_id: String,
    validator_pubkey: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PbftVotePhase {
    Prepare,
    Commit,
    ViewChange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistedDecision {
    schema_version: u8,
    identity: JournalIdentity,
    height: u64,
    prev_hash: String,
    view_number: u32,
    phase: PbftVotePhase,
    value_hash: String,
    signing_message: String,
    signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedPrepared {
    schema_version: u8,
    identity: JournalIdentity,
    block_hash: String,
    block: Block,
    certificate: PreparedCertificate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedViewChange {
    schema_version: u8,
    identity: JournalIdentity,
    vote: ViewChangeVote,
}

pub struct PbftSigningJournal {
    db: DB,
    identity: JournalIdentity,
    write_lock: Mutex<()>,
}

impl PbftSigningJournal {
    pub fn open(
        data_dir: &Path,
        genesis_hash: &str,
        validator_id: &str,
        validator_pubkey: &str,
    ) -> Result<Self> {
        Self::open_with_creation_policy(
            data_dir,
            genesis_hash,
            validator_id,
            validator_pubkey,
            true,
        )
    }

    pub fn open_existing(
        data_dir: &Path,
        genesis_hash: &str,
        validator_id: &str,
        validator_pubkey: &str,
    ) -> Result<Self> {
        Self::open_with_creation_policy(
            data_dir,
            genesis_hash,
            validator_id,
            validator_pubkey,
            false,
        )
    }

    fn open_with_creation_policy(
        data_dir: &Path,
        genesis_hash: &str,
        validator_id: &str,
        validator_pubkey: &str,
        allow_create: bool,
    ) -> Result<Self> {
        if allow_create {
            std::fs::create_dir_all(data_dir).context("create PBFT safety data directory")?;
        }
        let path = data_dir.join("pbft-safety.rocksdb");
        if !allow_create && !path.join("CURRENT").is_file() {
            bail!("bound PBFT signing journal is missing; recover its original durable state before signing");
        }
        let mut options = Options::default();
        options.create_if_missing(allow_create);
        let db = DB::open(&options, path).context("open durable PBFT safety database")?;
        let identity = JournalIdentity {
            schema_version: 1,
            genesis_hash: normalize_hash(genesis_hash, "genesis hash")?,
            validator_id: validator_id.to_owned(),
            validator_pubkey: normalize_hex(validator_pubkey),
        };
        if identity.validator_id.is_empty() {
            bail!("PBFT safety identity has an empty validator ID");
        }
        verify_pubkey(&identity.validator_pubkey)?;

        match db.get(IDENTITY_KEY).context("read PBFT safety identity")? {
            Some(bytes) => {
                let persisted: JournalIdentity =
                    serde_json::from_slice(&bytes).context("decode PBFT safety identity")?;
                if persisted != identity {
                    bail!(
                        "PBFT safety database belongs to a different genesis or validator identity"
                    );
                }
            }
            None => {
                if !allow_create {
                    bail!("bound PBFT signing journal has no identity anchor");
                }
                if db.iterator(rocksdb::IteratorMode::Start).next().is_some() {
                    bail!("PBFT safety database has records but no identity anchor");
                }
                let bytes = serde_json::to_vec(&identity)?;
                let mut options = WriteOptions::default();
                options.set_sync(true);
                db.put_opt(IDENTITY_KEY, bytes, &options)
                    .context("persist PBFT safety identity")?;
            }
        }

        let journal = Self {
            db,
            identity,
            write_lock: Mutex::new(()),
        };
        journal.validate_recovered_decisions()?;
        Ok(journal)
    }

    /// Record a locally signed PBFT message durably before it is broadcast.
    /// PREPARE locks are per view; COMMIT locks span all views at one
    /// height/parent; view-change locks are per target view.
    pub fn record_decision(
        &self,
        phase: PbftVotePhase,
        height: u64,
        prev_hash: &str,
        view_number: u32,
        value_hash: &str,
        signing_message: &str,
        signature: &str,
    ) -> Result<()> {
        if phase == PbftVotePhase::ViewChange {
            bail!("local view changes require atomic full-vote persistence");
        }
        self.record_decision_inner(
            phase,
            height,
            prev_hash,
            view_number,
            value_hash,
            signing_message,
            signature,
            None,
        )
    }

    /// Persist the signing lock, view floor and complete recoverable vote in
    /// one synchronous write before applying or broadcasting the local vote.
    pub fn record_local_view_change(
        &self,
        vote: &ViewChangeVote,
        validator_set: &consensus::ValidatorSet,
        allow_small_cluster_quorum: bool,
    ) -> Result<()> {
        if vote.validator_id != self.identity.validator_id {
            bail!("local view-change vote belongs to another validator");
        }
        consensus::pbft::verify_view_change_vote(vote, validator_set, allow_small_cluster_quorum)?;
        let message = vote.signing_message();
        self.record_decision_inner(
            PbftVotePhase::ViewChange,
            vote.height,
            &vote.prev_hash,
            vote.new_view,
            &common::sha256_hex(message.as_bytes()),
            &message,
            &vote.signature,
            Some(vote),
        )
    }

    fn record_decision_inner(
        &self,
        phase: PbftVotePhase,
        height: u64,
        prev_hash: &str,
        view_number: u32,
        value_hash: &str,
        signing_message: &str,
        signature: &str,
        local_vote: Option<&ViewChangeVote>,
    ) -> Result<()> {
        if height == 0 {
            bail!("PBFT signing decision has invalid height zero");
        }
        let prev_hash = normalize_hash(prev_hash, "PBFT parent hash")?;
        let value_hash = normalize_hash(value_hash, "PBFT value hash")?;
        validate_signing_payload(phase, view_number, &value_hash, signing_message)?;
        verify_signature(&self.identity.validator_pubkey, signing_message, signature)?;

        let decision = PersistedDecision {
            schema_version: 1,
            identity: self.identity.clone(),
            height,
            prev_hash: prev_hash.clone(),
            view_number,
            phase,
            value_hash,
            signing_message: signing_message.to_owned(),
            signature: normalize_hex(signature),
        };
        let key = decision_key(&decision);
        let bytes = serde_json::to_vec(&decision)?;
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT safety write lock was poisoned"))?;
        if matches!(phase, PbftVotePhase::Prepare | PbftVotePhase::Commit) {
            self.validate_commit_lock(height, &prev_hash, &decision.value_hash)?;
        }
        if let Some(vote) = local_vote {
            if let Some(bytes) = self.db.get(prepared_key(height, &prev_hash))? {
                let prepared: PersistedPrepared = serde_json::from_slice(&bytes)?;
                let reported = vote
                    .prepared
                    .as_ref()
                    .context("local view change omitted the durable prepared value")?;
                if reported.prepared_view < prepared.certificate.prepared_view
                    || (reported.prepared_view == prepared.certificate.prepared_view
                        && reported.block().hash() != prepared.block_hash)
                {
                    bail!("local view change conflicts with the durable prepared value");
                }
            }
        }
        let local_view_key = local_view_key(height, &prev_hash);
        let local_view_lock = self
            .db
            .get(&local_view_key)
            .context("read durable local PBFT view lock")?
            .map(|bytes| {
                let raw: &[u8] = bytes.as_ref();
                let bytes: [u8; 4] = raw
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("durable local PBFT view lock is malformed"))?;
                Ok::<u32, anyhow::Error>(u32::from_be_bytes(bytes))
            })
            .transpose()?
            .unwrap_or(0);
        if phase == PbftVotePhase::Prepare {
            let commit_key = commit_key(height, &prev_hash);
            if let Some(committed) = self
                .db
                .get(&commit_key)
                .context("read durable COMMIT lock")?
            {
                let committed: PersistedDecision =
                    serde_json::from_slice(&committed).context("decode durable COMMIT lock")?;
                if committed.value_hash != decision.value_hash {
                    bail!("refusing PREPARE for a value different from the durable COMMIT lock");
                }
            }
        }
        match phase {
            PbftVotePhase::Prepare if view_number < local_view_lock => {
                bail!(
                    "refusing PREPARE in stale view {view_number}; local view lock is {local_view_lock}"
                );
            }
            PbftVotePhase::ViewChange if view_number < local_view_lock => {
                bail!(
                    "refusing view change to stale view {view_number}; local view lock is {local_view_lock}"
                );
            }
            PbftVotePhase::Commit
                if view_number < local_view_lock && self.db.get(&key)?.is_none() =>
            {
                bail!("refusing a new COMMIT after leaving PBFT view {view_number}; local view lock is {local_view_lock}");
            }
            _ => {}
        }
        if let Some(previous) = self.db.get(&key).context("read PBFT vote lock")? {
            let previous: PersistedDecision =
                serde_json::from_slice(&previous).context("decode existing PBFT vote lock")?;
            if !same_decision(&previous, &decision) {
                bail!("refusing conflicting durable PBFT signing decision");
            }
            if local_vote.is_none() {
                return Ok(());
            }
        }
        let mut options = WriteOptions::default();
        options.set_sync(true);
        let mut batch = rocksdb::WriteBatch::default();
        batch.put(&key, bytes);
        if let Some(vote) = local_vote {
            let vote_key = received_view_change_key(vote);
            if let Some(bytes) = self.db.get(&vote_key)? {
                let previous: PersistedViewChange = serde_json::from_slice(&bytes)?;
                if previous.vote.signing_message() != signing_message
                    || previous.vote.signature != vote.signature
                {
                    bail!("local view-change recovery payload conflicts with its signing lock");
                }
            }
            batch.put(
                vote_key,
                serde_json::to_vec(&PersistedViewChange {
                    schema_version: 1,
                    identity: self.identity.clone(),
                    vote: vote.clone(),
                })?,
            );
        }
        if phase == PbftVotePhase::ViewChange && view_number > local_view_lock {
            batch.put(local_view_key, view_number.to_be_bytes());
        }
        self.db
            .write_opt(batch, &options)
            .context("sync durable PBFT signing decision")?;
        Ok(())
    }

    pub fn genesis_hash(&self) -> &str {
        &self.identity.genesis_hash
    }

    pub fn validator_id(&self) -> &str {
        &self.identity.validator_id
    }

    pub fn validator_pubkey(&self) -> &str {
        &self.identity.validator_pubkey
    }

    /// A chain rollback cannot authorize a different COMMIT at the same height,
    /// even when the candidate fork changes the parent.
    fn validate_commit_lock(&self, height: u64, prev_hash: &str, block_hash: &str) -> Result<()> {
        let prefix = format!("decision/commit/{height:020}/");
        for entry in self.db.prefix_iterator(prefix.as_bytes()) {
            let (key, bytes) = entry.context("read height-wide durable COMMIT lock")?;
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            let decision: PersistedDecision = serde_json::from_slice(&bytes)?;
            if decision.prev_hash != prev_hash || decision.value_hash != block_hash {
                bail!("candidate conflicts with a durable COMMIT at height {height}");
            }
        }
        Ok(())
    }

    pub fn validate_canonical_block(&self, block: &Block) -> Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT safety write lock was poisoned"))?;
        self.validate_commit_lock(block.header.height, &block.header.prev_hash, &block.hash())
    }

    /// Queue cleanup must not retire work needed by a durable prepared value,
    /// even if its in-memory round timed out or is awaiting a certified view.
    pub fn has_prepared_or_commit_lock(&self, height: u64, prev_hash: &str) -> Result<bool> {
        Ok(self.db.get(prepared_key(height, prev_hash))?.is_some()
            || self.db.get(commit_key(height, prev_hash))?.is_some())
    }

    /// Recover signed local COMMIT votes so an active prepared round retains
    /// its vote and the existing rebroadcast worker can resend it after a
    /// restart. A COMMIT without its full prepared value is rejected at startup.
    pub fn recovered_commits(&self) -> Result<Vec<(u64, String, String, String)>> {
        let prefix = b"decision/commit/";
        let mut commits = Vec::new();
        for entry in self.db.prefix_iterator(prefix) {
            let (key, value) = entry.context("scan durable COMMIT decisions")?;
            if !key.starts_with(prefix) {
                break;
            }
            let decision: PersistedDecision =
                serde_json::from_slice(&value).context("decode recovered COMMIT decision")?;
            if decision.phase != PbftVotePhase::Commit
                || decision.identity != self.identity
                || key.as_ref() != decision_key(&decision).as_slice()
            {
                bail!("recovered COMMIT decision has a mismatched identity or key");
            }
            commits.push((
                decision.height,
                decision.prev_hash,
                decision.value_hash,
                decision.signature,
            ));
        }
        Ok(commits)
    }

    /// Persist the complete prepared value and its quorum proof before this
    /// validator signs COMMIT. A restart can then recover the exact block
    /// hash, transaction payload and PREPARE signatures for a view change.
    pub fn record_prepared(
        &self,
        block: &Block,
        certificate: &PreparedCertificate,
        validator_set: &consensus::ValidatorSet,
        allow_small_cluster_quorum: bool,
    ) -> Result<()> {
        let header = &certificate.header;
        if header.height == 0 {
            bail!("cannot journal a prepared genesis block");
        }
        let next_view = certificate
            .prepared_view
            .checked_add(1)
            .context("prepared PBFT view has no successor")?;
        let vote_context = ViewChangeVote {
            height: header.height,
            prev_hash: header.prev_hash.clone(),
            new_view: next_view,
            validator_id: self.identity.validator_id.clone(),
            prepared: None,
            signature: String::new(),
        };
        consensus::pbft::verify_prepared_certificate(
            &vote_context,
            certificate,
            validator_set,
            allow_small_cluster_quorum,
        )?;
        let block_hash = certificate.block().hash();
        if block.hash() != block_hash
            || block.header.height != header.height
            || block.header.prev_hash != header.prev_hash
            || common::merkle_root(&block.transactions) != header.merkle_root
        {
            bail!("prepared certificate does not reconstruct the supplied block");
        }
        let record = PersistedPrepared {
            schema_version: 1,
            identity: self.identity.clone(),
            block_hash,
            block: block.clone(),
            certificate: certificate.clone(),
        };
        let key = prepared_key(header.height, &header.prev_hash);
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT safety write lock was poisoned"))?;
        self.validate_commit_lock(header.height, &header.prev_hash, &record.block_hash)?;
        if let Some(committed) = self
            .db
            .get(commit_key(header.height, &header.prev_hash))
            .context("read durable COMMIT lock before prepared value")?
        {
            let committed: PersistedDecision =
                serde_json::from_slice(&committed).context("decode durable COMMIT lock")?;
            if committed.value_hash != record.block_hash {
                bail!("refusing prepared value different from the durable COMMIT lock");
            }
        }
        if let Some(previous) = self.db.get(&key).context("read durable prepared value")? {
            let previous: PersistedPrepared =
                serde_json::from_slice(&previous).context("decode durable prepared value")?;
            if previous.certificate.prepared_view > certificate.prepared_view {
                bail!("refusing to replace a higher-view durable prepared value");
            }
            if previous.certificate.prepared_view == certificate.prepared_view
                && previous.block_hash != record.block_hash
            {
                bail!("refusing a conflicting prepared value in the same PBFT view");
            }
        }
        let mut options = WriteOptions::default();
        options.set_sync(true);
        self.db
            .put_opt(key, serde_json::to_vec(&record)?, &options)
            .context("sync durable PBFT prepared value")?;
        Ok(())
    }

    /// Return prepared certificates for verification against the authenticated
    /// validator history and restoration into the consensus engine at startup.
    pub fn recovered_prepared(&self) -> Result<Vec<(Block, PreparedCertificate)>> {
        let mut recovered = Vec::new();
        for entry in self.db.prefix_iterator(PREPARED_PREFIX) {
            let (key, value) = entry.context("scan durable prepared values")?;
            if !key.starts_with(PREPARED_PREFIX) {
                break;
            }
            let record: PersistedPrepared =
                serde_json::from_slice(&value).context("decode recovered prepared value")?;
            validate_prepared_record(&self.identity, &key, &record)?;
            recovered.push((record.block, record.certificate));
        }
        Ok(recovered)
    }

    /// Store each verified signed input before it is applied to the in-memory
    /// view-change collector. Replaying these inputs restores a learned quorum
    /// certificate even when this validator did not cast the view-change vote.
    pub fn record_received_view_change(
        &self,
        vote: &ViewChangeVote,
        validator_set: &consensus::ValidatorSet,
        allow_small_cluster_quorum: bool,
    ) -> Result<()> {
        consensus::pbft::verify_view_change_vote(vote, validator_set, allow_small_cluster_quorum)?;
        let record = PersistedViewChange {
            schema_version: 1,
            identity: self.identity.clone(),
            vote: vote.clone(),
        };
        let key = received_view_change_key(vote);
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("PBFT safety write lock was poisoned"))?;
        if let Some(previous) = self
            .db
            .get(&key)
            .context("read received view-change vote")?
        {
            let previous: PersistedViewChange =
                serde_json::from_slice(&previous).context("decode received view-change vote")?;
            if previous.vote.signing_message() != vote.signing_message()
                || previous.vote.signature != vote.signature
            {
                bail!("refusing equivocated durable view-change vote");
            }
            return Ok(());
        }
        let mut options = WriteOptions::default();
        options.set_sync(true);
        self.db
            .put_opt(key, serde_json::to_vec(&record)?, &options)
            .context("sync received view-change vote")?;
        Ok(())
    }

    pub fn recovered_view_change_votes(&self) -> Result<Vec<ViewChangeVote>> {
        let mut votes = Vec::new();
        for entry in self.db.prefix_iterator(RECEIVED_VIEW_CHANGE_PREFIX) {
            let (key, value) = entry.context("scan received view-change votes")?;
            if !key.starts_with(RECEIVED_VIEW_CHANGE_PREFIX) {
                break;
            }
            let record: PersistedViewChange =
                serde_json::from_slice(&value).context("decode recovered view-change vote")?;
            if record.schema_version != 1
                || record.identity != self.identity
                || key.as_ref() != received_view_change_key(&record.vote).as_slice()
            {
                bail!("recovered view-change vote has a mismatched identity or key");
            }
            votes.push(record.vote);
        }
        Ok(votes)
    }

    fn validate_recovered_decisions(&self) -> Result<()> {
        let mut expected_local_view_locks = std::collections::HashMap::<Vec<u8>, u32>::new();
        for entry in self.db.iterator(rocksdb::IteratorMode::Start) {
            let (key, value) = entry.context("scan durable PBFT decisions")?;
            if !key.starts_with(DECISION_PREFIX) {
                continue;
            }
            let decision: PersistedDecision =
                serde_json::from_slice(&value).context("decode recovered PBFT decision")?;
            if decision.schema_version != 1 || decision.identity != self.identity {
                bail!("recovered PBFT decision has a mismatched schema or identity");
            }
            if key.as_ref() != decision_key(&decision).as_slice() {
                bail!("recovered PBFT decision key does not match its contents");
            }
            normalize_hash(&decision.prev_hash, "recovered PBFT parent hash")?;
            normalize_hash(&decision.value_hash, "recovered PBFT value hash")?;
            validate_signing_payload(
                decision.phase,
                decision.view_number,
                &decision.value_hash,
                &decision.signing_message,
            )?;
            verify_signature(
                &self.identity.validator_pubkey,
                &decision.signing_message,
                &decision.signature,
            )?;
            if decision.phase == PbftVotePhase::ViewChange {
                let vote_key = [
                    RECEIVED_VIEW_CHANGE_PREFIX,
                    format!(
                        "{:020}/{}/{:010}/{}",
                        decision.height,
                        decision.prev_hash,
                        decision.view_number,
                        self.identity.validator_id
                    )
                    .as_bytes(),
                ]
                .concat();
                let payload = self.db.get(vote_key)?.context(
                    "durable local view-change decision lacks its original full vote; signing is blocked")?;
                let recovered: PersistedViewChange = serde_json::from_slice(&payload)?;
                if recovered.vote.signing_message() != decision.signing_message
                    || recovered.vote.signature != decision.signature
                {
                    bail!("durable view-change payload does not match its signing decision");
                }
                let key = local_view_key(decision.height, &decision.prev_hash);
                expected_local_view_locks
                    .entry(key)
                    .and_modify(|current| *current = (*current).max(decision.view_number))
                    .or_insert(decision.view_number);
            }
        }
        let mut actual_local_view_locks = std::collections::HashMap::<Vec<u8>, u32>::new();
        for entry in self.db.iterator(rocksdb::IteratorMode::Start) {
            let (key, value) = entry.context("scan durable PBFT local view locks")?;
            if !key.starts_with(LOCAL_VIEW_LOCK_PREFIX) {
                continue;
            }
            let raw: &[u8] = value.as_ref();
            let bytes: [u8; 4] = raw
                .try_into()
                .map_err(|_| anyhow::anyhow!("durable local PBFT view lock is malformed"))?;
            actual_local_view_locks.insert(key.to_vec(), u32::from_be_bytes(bytes));
        }
        if actual_local_view_locks != expected_local_view_locks {
            bail!("durable local PBFT view locks do not match recovered view-change decisions");
        }
        self.recovered_prepared()?;
        self.recovered_view_change_votes()?;
        Ok(())
    }
}

fn received_view_change_key(vote: &ViewChangeVote) -> Vec<u8> {
    [
        RECEIVED_VIEW_CHANGE_PREFIX,
        format!(
            "{:020}/{}/{:010}/{}",
            vote.height, vote.prev_hash, vote.new_view, vote.validator_id
        )
        .as_bytes(),
    ]
    .concat()
}

fn prepared_key(height: u64, prev_hash: &str) -> Vec<u8> {
    [
        PREPARED_PREFIX,
        format!("{:020}/{}", height, prev_hash).as_bytes(),
    ]
    .concat()
}

fn commit_key(height: u64, prev_hash: &str) -> Vec<u8> {
    [
        DECISION_PREFIX,
        format!("commit/{:020}/{}", height, prev_hash).as_bytes(),
    ]
    .concat()
}

fn validate_prepared_record(
    identity: &JournalIdentity,
    key: &[u8],
    record: &PersistedPrepared,
) -> Result<()> {
    if record.schema_version != 1 || &record.identity != identity {
        bail!("recovered prepared value has a mismatched schema or identity");
    }
    let header = &record.certificate.header;
    if header.height == 0 || key != prepared_key(header.height, &header.prev_hash) {
        bail!("recovered prepared value key does not match its context");
    }
    normalize_hash(&header.prev_hash, "recovered prepared parent hash")?;
    if record.block_hash != record.certificate.block().hash()
        || record.block_hash != record.block.hash()
    {
        bail!("recovered prepared value has a mismatched block hash");
    }
    if record.block.header.height != header.height
        || record.block.header.prev_hash != header.prev_hash
        || record.block.header.merkle_root != header.merkle_root
        || common::merkle_root(&record.block.transactions) != header.merkle_root
    {
        bail!("recovered prepared value does not match its full block");
    }
    if common::merkle_root(&record.certificate.transactions) != header.merkle_root {
        bail!("recovered prepared value has an invalid transaction root");
    }
    Ok(())
}

fn validate_signing_payload(
    phase: PbftVotePhase,
    view_number: u32,
    value_hash: &str,
    signing_message: &str,
) -> Result<()> {
    let expected_message = match phase {
        PbftVotePhase::Prepare => Some(consensus::pbft::pbft_signature_message(
            "prepare", value_hash,
        )),
        PbftVotePhase::Commit => Some(consensus::pbft::pbft_signature_message(
            "commit", value_hash,
        )),
        PbftVotePhase::ViewChange => None,
    };
    if expected_message
        .as_deref()
        .is_some_and(|expected| expected != signing_message)
    {
        bail!("PBFT phase signing message does not match its recorded value hash");
    }
    if phase == PbftVotePhase::ViewChange {
        if view_number == 0 {
            bail!("view-change signing decision has target view zero");
        }
        if common::sha256_hex(signing_message.as_bytes()) != value_hash {
            bail!("view-change signing message does not match its recorded digest");
        }
    }
    Ok(())
}

fn decision_key(decision: &PersistedDecision) -> Vec<u8> {
    let lock_scope = match decision.phase {
        PbftVotePhase::Prepare => format!(
            "prepare/{:020}/{}/{:010}",
            decision.height, decision.prev_hash, decision.view_number
        ),
        PbftVotePhase::Commit => format!("commit/{:020}/{}", decision.height, decision.prev_hash),
        PbftVotePhase::ViewChange => format!(
            "view-change/{:020}/{}/{:010}",
            decision.height, decision.prev_hash, decision.view_number
        ),
    };
    [DECISION_PREFIX, lock_scope.as_bytes()].concat()
}

fn local_view_key(height: u64, prev_hash: &str) -> Vec<u8> {
    let mut key = LOCAL_VIEW_LOCK_PREFIX.to_vec();
    key.extend_from_slice(format!("{:020}/{}", height, prev_hash).as_bytes());
    key
}

fn same_decision(previous: &PersistedDecision, next: &PersistedDecision) -> bool {
    if previous.phase == PbftVotePhase::Commit && next.phase == PbftVotePhase::Commit {
        previous.schema_version == next.schema_version
            && previous.identity == next.identity
            && previous.height == next.height
            && previous.prev_hash == next.prev_hash
            && previous.phase == next.phase
            && previous.value_hash == next.value_hash
            && previous.signing_message == next.signing_message
            && previous.signature == next.signature
    } else {
        previous == next
    }
}

fn normalize_hex(value: &str) -> String {
    value.trim_start_matches("0x").to_ascii_lowercase()
}

fn normalize_hash(value: &str, name: &str) -> Result<String> {
    let normalized = normalize_hex(value);
    if normalized.len() != 64 || hex::decode(&normalized).is_err() {
        bail!("{name} must be a 32-byte hex value");
    }
    Ok(normalized)
}

fn verify_pubkey(pubkey: &str) -> Result<()> {
    let bytes = hex::decode(pubkey).context("decode PBFT validator public key")?;
    VerifyingKey::try_from(bytes.as_slice()).context("parse PBFT validator public key")?;
    Ok(())
}

fn verify_signature(pubkey: &str, message: &str, signature: &str) -> Result<()> {
    let bytes = hex::decode(normalize_hex(signature)).context("decode PBFT signature")?;
    let signature = Signature::try_from(bytes.as_slice()).context("parse PBFT signature")?;
    let key_bytes = hex::decode(pubkey).context("decode PBFT validator public key")?;
    let key =
        VerifyingKey::try_from(key_bytes.as_slice()).context("parse PBFT validator public key")?;
    key.verify_strict(message.as_bytes(), &signature)
        .context("verify PBFT signing decision")
}

#[cfg(test)]
mod tests {
    use super::{PbftSigningJournal, PbftVotePhase};
    use common::ViewChangeVote;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::tempdir;

    fn identity(key: &SigningKey) -> (String, String) {
        (
            "validator-1".to_owned(),
            hex::encode(key.verifying_key().as_bytes()),
        )
    }

    #[test]
    fn durable_vote_lock_survives_reopen_and_rejects_conflicts() {
        let dir = tempdir().unwrap();
        let key = SigningKey::from_bytes(&[41; 32]);
        let (validator_id, pubkey) = identity(&key);
        let journal =
            PbftSigningJournal::open(dir.path(), &"aa".repeat(32), &validator_id, &pubkey).unwrap();
        let parent = "bb".repeat(32);
        let value = "cc".repeat(32);
        let message = consensus::pbft::pbft_signature_message("prepare", &value);
        let signature = hex::encode(key.sign(message.as_bytes()).to_bytes());
        journal
            .record_decision(
                PbftVotePhase::Prepare,
                7,
                &parent,
                2,
                &value,
                &message,
                &signature,
            )
            .unwrap();
        drop(journal);

        let recovered =
            PbftSigningJournal::open(dir.path(), &"aa".repeat(32), &validator_id, &pubkey).unwrap();
        let conflicting_value = "dd".repeat(32);
        let conflicting_message =
            consensus::pbft::pbft_signature_message("prepare", &conflicting_value);
        assert!(recovered
            .record_decision(
                PbftVotePhase::Prepare,
                7,
                &parent,
                2,
                &conflicting_value,
                &conflicting_message,
                &hex::encode(key.sign(conflicting_message.as_bytes()).to_bytes()),
            )
            .is_err());
        assert!(recovered
            .record_decision(
                PbftVotePhase::Prepare,
                7,
                &parent,
                2,
                &value,
                &message,
                &signature,
            )
            .is_ok());
    }

    #[test]
    fn commit_lock_spans_views_and_identity_mismatch_fails_closed() {
        let dir = tempdir().unwrap();
        let key = SigningKey::from_bytes(&[42; 32]);
        let (validator_id, pubkey) = identity(&key);
        let journal =
            PbftSigningJournal::open(dir.path(), &"aa".repeat(32), &validator_id, &pubkey).unwrap();
        let parent = "bb".repeat(32);
        let value = "cc".repeat(32);
        let message = consensus::pbft::pbft_signature_message("commit", &value);
        let signature = hex::encode(key.sign(message.as_bytes()).to_bytes());
        journal
            .record_decision(
                PbftVotePhase::Commit,
                7,
                &parent,
                0,
                &value,
                &message,
                &signature,
            )
            .unwrap();
        journal
            .record_decision(
                PbftVotePhase::Commit,
                7,
                &parent,
                1,
                &value,
                &message,
                &signature,
            )
            .unwrap();
        let conflicting_value = "dd".repeat(32);
        let conflicting_message =
            consensus::pbft::pbft_signature_message("commit", &conflicting_value);
        assert!(journal
            .record_decision(
                PbftVotePhase::Commit,
                7,
                &parent,
                1,
                &conflicting_value,
                &conflicting_message,
                &hex::encode(key.sign(conflicting_message.as_bytes()).to_bytes()),
            )
            .is_err());
        drop(journal);
        assert!(
            PbftSigningJournal::open(dir.path(), &"ee".repeat(32), &validator_id, &pubkey,)
                .is_err()
        );
    }

    #[test]
    fn local_view_lock_survives_restart_and_blocks_stale_prepare() {
        let dir = tempdir().unwrap();
        let key = SigningKey::from_bytes(&[43; 32]);
        let (validator_id, pubkey) = identity(&key);
        let journal =
            PbftSigningJournal::open(dir.path(), &"aa".repeat(32), &validator_id, &pubkey).unwrap();
        let parent = "bb".repeat(32);
        let authority = common::ValidatorSet::new(vec![common::Validator {
            id: validator_id.clone(),
            pubkey: pubkey.clone(),
            stake: 100,
            reputation: 100,
            status: "active".into(),
            ..Default::default()
        }])
        .into();
        let mut vote = ViewChangeVote {
            height: 7,
            prev_hash: parent.clone(),
            new_view: 2,
            validator_id: validator_id.clone(),
            prepared: None,
            signature: String::new(),
        };
        vote.signature = hex::encode(key.sign(vote.signing_message().as_bytes()).to_bytes());
        journal
            .record_local_view_change(&vote, &authority, false)
            .unwrap();
        drop(journal);

        let recovered =
            PbftSigningJournal::open(dir.path(), &"aa".repeat(32), &validator_id, &pubkey).unwrap();
        let stale_value = "cc".repeat(32);
        let stale_message = consensus::pbft::pbft_signature_message("prepare", &stale_value);
        assert!(recovered
            .record_decision(
                PbftVotePhase::Prepare,
                7,
                &parent,
                1,
                &stale_value,
                &stale_message,
                &hex::encode(key.sign(stale_message.as_bytes()).to_bytes()),
            )
            .is_err());

        let current_value = "dd".repeat(32);
        let current_message = consensus::pbft::pbft_signature_message("prepare", &current_value);
        assert!(recovered
            .record_decision(
                PbftVotePhase::Prepare,
                7,
                &parent,
                2,
                &current_value,
                &current_message,
                &hex::encode(key.sign(current_message.as_bytes()).to_bytes()),
            )
            .is_ok());
    }
}
