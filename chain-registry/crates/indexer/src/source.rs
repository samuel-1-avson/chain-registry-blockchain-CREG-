//! Canonical chain sources for the standalone indexer.

use anyhow::{bail, ensure, Context, Result};
use common::{AuthenticatedValidatorSetSnapshot, Block, ChainSpec};
use resolver::light_client::{
    load_verified_chain_spec, verify_header, verify_pbft_consensus_with_params, AuthorityTimeline,
    Checkpoint,
};
use rocksdb::{Options, DB};
use serde::{de::DeserializeOwned, Deserialize};
use sqlx::PgPool;
use std::{path::Path, time::Duration};

// Keys shared with the node's ChainStore on-disk format. Open only the
// families needed for the canonical block reader.
const BLOCKS_BY_HEIGHT: &str = "blocks_by_height";
const BLOCKS_BY_HASH: &str = "blocks_by_hash";
const REORG_JOURNAL: &str = "reorg_journal";
const MAX_PUBLIC_RESPONSE: usize = 64 * 1024 * 1024;

pub struct Local {
    db: DB,
}

impl Local {
    pub fn open(data_dir: &Path, secondary_dir: &Path) -> Result<Self> {
        let primary = data_dir.join("chain.rocksdb");
        let canonical_primary = primary
            .canonicalize()
            .context("locate primary chain database")?;
        std::fs::create_dir_all(secondary_dir).context("create secondary directory")?;
        let canonical_secondary = secondary_dir.canonicalize()?;
        ensure!(
            !canonical_secondary.starts_with(&canonical_primary)
                && !canonical_primary.starts_with(&canonical_secondary),
            "RocksDB primary and secondary directories must be separate"
        );
        let mut options = Options::default();
        // Required by RocksDB secondary mode to keep primary SST files open
        // even after compaction unlinks them.
        options.set_max_open_files(-1);
        let mut families = vec![BLOCKS_BY_HEIGHT, BLOCKS_BY_HASH];
        if DB::list_cf(&options, &primary)?
            .iter()
            .any(|cf| cf == REORG_JOURNAL)
        {
            families.push(REORG_JOURNAL);
        }
        // RocksDB appends forward slashes to these paths. Windows verbatim
        // paths returned by canonicalize() reject that mixed-separator form.
        let db = DB::open_cf_as_secondary(&options, primary.as_path(), secondary_dir, families)
            .context("open RocksDB secondary")?;
        Ok(Self { db })
    }

    pub fn catch_up(&self) -> Result<u64> {
        self.db.try_catch_up_with_primary()?;
        if let Some(journal) = self.db.cf_handle(REORG_JOURNAL) {
            ensure!(
                self.db.get_cf(&journal, b"active")?.is_none(),
                "primary has an in-flight reorganization"
            );
        }
        let heights = self
            .db
            .cf_handle(BLOCKS_BY_HEIGHT)
            .context("height column family")?;
        let mut iter = self.db.raw_iterator_cf(&heights);
        iter.seek_to_last();
        iter.status()?;
        let key = iter.key().context("primary lacks a canonical genesis")?;
        Ok(u64::from_be_bytes(key.try_into()?))
    }

    pub fn block(&self, height: u64) -> Result<Block> {
        let heights = self
            .db
            .cf_handle(BLOCKS_BY_HEIGHT)
            .context("height column family")?;
        let blocks = self
            .db
            .cf_handle(BLOCKS_BY_HASH)
            .context("block column family")?;
        let hash = self
            .db
            .get_cf(&heights, height.to_be_bytes())?
            .context("missing canonical height")?;
        let bytes = self
            .db
            .get_cf(&blocks, &hash)?
            .context("missing canonical block body")?;
        let block: Block = serde_json::from_slice(&bytes)?;
        verify_header(&block.header, std::str::from_utf8(&hash)?, height)?;
        Ok(block)
    }
}

pub struct Remote {
    base: String,
    client: reqwest::Client,
    spec: ChainSpec,
    genesis: Checkpoint,
}

#[derive(Deserialize)]
struct PublicTip {
    tip_height: u64,
}

#[derive(Deserialize)]
struct PublicBlock {
    hash: String,
    #[serde(flatten)]
    block: Block,
}

pub struct RemoteAuthority {
    timeline: AuthorityTimeline,
    transitions: Vec<AuthenticatedValidatorSetSnapshot>,
}

impl Remote {
    pub async fn connect(url: &str) -> Result<Self> {
        let parsed = reqwest::Url::parse(url)?;
        ensure!(
            matches!(parsed.scheme(), "http" | "https")
                && parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.query().is_none()
                && parsed.fragment().is_none(),
            "invalid CREG_INDEXER_REMOTE_URL"
        );
        let spec = load_verified_chain_spec().await?;
        let genesis = Checkpoint::from_verified_spec(&spec)?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            base: url.trim_end_matches('/').into(),
            client,
            spec,
            genesis,
        })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let mut response = self
            .client
            .get(format!("{}{path}", self.base))
            .send()
            .await?
            .error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_PUBLIC_RESPONSE,
                "public chain response exceeds byte limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn tip(&self) -> Result<u64> {
        Ok(self
            .get::<PublicTip>("/v1/public/blocks?limit=0")
            .await?
            .tip_height)
    }

    pub async fn authority(
        &self,
        pool: &PgPool,
        indexed: i64,
        through: u64,
    ) -> Result<RemoteAuthority> {
        let mut transitions: Vec<AuthenticatedValidatorSetSnapshot> =
            self.get("/v1/public/validator-set-transitions").await?;
        transitions.retain(|transition| transition.effective_from_height <= through);
        let timeline = AuthorityTimeline::from_spec_and_history(&self.spec, &transitions, through)?;
        // A certificate also commits to the preceding block hash. Check
        // already indexed transition parents when resuming after a restart.
        for transition in &transitions {
            let parent = i64::try_from(transition.effective_from_height - 1)?;
            if parent > indexed {
                break;
            }
            let hash: String = sqlx::query_scalar("SELECT hash FROM blocks WHERE height = $1")
                .bind(parent)
                .fetch_one(pool)
                .await?;
            ensure!(
                transition.previous_block_hash == hash,
                "authority transition conflicts with indexed chain"
            );
        }
        Ok(RemoteAuthority {
            timeline,
            transitions,
        })
    }

    pub async fn block(&self, height: u64, authority: &RemoteAuthority) -> Result<Block> {
        let PublicBlock { hash, block } = self.get(&format!("/v1/public/blocks/{height}")).await?;
        verify_header(&block.header, &hash, height)?;
        if height == 0 {
            ensure!(
                hash == self.genesis.hash && block.transactions.is_empty(),
                "remote genesis differs from signed spec"
            );
            return Ok(block);
        }
        let params = &self.spec.consensus_params;
        verify_body(&block, params.consensus_encoding.as_deref())?;
        ensure!(
            block.header.status_root.is_some() == params.vote_v4_active_at(height),
            "block status root does not match signed activation rules"
        );
        ensure!(
            params.vote_v4_active_at(height)
                || !block
                    .transactions
                    .iter()
                    .any(common::Transaction::requires_vote_v4_activation),
            "block carries v4 transactions before signed activation"
        );
        if let Ok(index) = authority
            .transitions
            .binary_search_by_key(&height, |entry| entry.effective_from_height)
        {
            ensure!(
                authority.transitions[index].previous_block_hash == block.header.prev_hash,
                "authority transition does not anchor to block parent"
            );
        }
        verify_pbft_consensus_with_params(
            &hash,
            &block.header,
            &authority.timeline.set_at(height).validators,
            &block.pbft_signatures,
            params,
        )?;
        Ok(block)
    }
}

fn verify_body(block: &Block, encoding: Option<&str>) -> Result<()> {
    let root = common::merkle_root_for_encoding(&block.transactions, encoding)?;
    if root != block.header.merkle_root {
        bail!(
            "transaction Merkle root mismatch at height {}",
            block.header.height
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::Transaction;

    #[test]
    fn secondary_reads_primary_updates_without_reopening() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut options = Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);
        let primary = DB::open_cf(
            &options,
            dir.path().join("chain.rocksdb"),
            [BLOCKS_BY_HASH, BLOCKS_BY_HEIGHT, REORG_JOURNAL],
        )?;
        let by_hash = primary.cf_handle(BLOCKS_BY_HASH).unwrap();
        let by_height = primary.cf_handle(BLOCKS_BY_HEIGHT).unwrap();
        let write = |block: &Block| -> Result<()> {
            let mut batch = rocksdb::WriteBatch::default();
            batch.put_cf(&by_hash, block.hash(), serde_json::to_vec(block)?);
            batch.put_cf(&by_height, block.header.height.to_be_bytes(), block.hash());
            primary.write(batch)?;
            Ok(())
        };
        let mut block = Block::genesis();
        write(&block)?;
        let reader = Local::open(dir.path(), &dir.path().join("secondary"))?;
        assert_eq!(reader.catch_up()?, 0);
        block.header.prev_hash = block.hash();
        block.header.height = 1;
        write(&block)?;
        assert_eq!(reader.catch_up()?, 1);
        assert_eq!(reader.block(1)?.hash(), block.hash());
        primary.put_cf(
            &primary.cf_handle(REORG_JOURNAL).unwrap(),
            b"active",
            b"fixture",
        )?;
        assert!(reader.catch_up().is_err());
        Ok(())
    }

    #[test]
    fn remote_body_must_match_signed_merkle_commitment() -> Result<()> {
        let mut block = Block::genesis();
        block.transactions.push(Transaction::ValidatorLeave {
            validator_id: "v".into(),
        });
        block.header.merkle_root = common::merkle_root_for_encoding(&block.transactions, None)?;
        verify_body(&block, None)?;
        block.transactions.clear();
        assert!(verify_body(&block, None).is_err());
        Ok(())
    }
}
