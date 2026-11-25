//! Confirmed ValidatorRewards event ingestion, with a separate L1 cursor.

use alloy::{
    primitives::{Address, B256, U256},
    rpc::types::Log,
    sol,
    sol_types::SolEvent,
};
use anyhow::{bail, ensure, Context, Result};
use common::l1_rpc::{parse_rpc_endpoints, quorum_json_call};
use registry_view::{SseEnvelopeV2, SseEventKindV2};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use std::{collections::BTreeMap, time::Duration};

use crate::apply::{insert_event, lock_cursor};

sol! {
    // ValidatorRewards.sol
    event DiscoveryBountyAwarded(
        bytes32 indexed witnessDigest,
        bytes32 indexed packageHash,
        address indexed prover,
        uint256 proverBonus,
        uint256 committeeShare
    );
}

const CONFIRMATIONS: u64 = 12;
const BLOCKS_PER_BATCH: u64 = 128;
const MAX_RPC_BYTES: usize = 8 * 1024 * 1024;

pub struct BountyLane {
    chain_id: i64,
    contract: Address,
    start_block: u64,
    endpoints: Vec<String>,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct L1Block {
    number: u64,
    hash: B256,
}

#[derive(Debug, PartialEq, Eq)]
struct Award {
    block: L1Block,
    tx_hash: B256,
    log_index: u64,
    witness_digest: B256,
    package_hash: B256,
    prover: Address,
    prover_bonus: U256,
    committee_share: U256,
}

fn decode(log: &Log, contract: Address) -> Result<Award> {
    ensure!(
        !log.removed && log.address() == contract,
        "bounty log is removed or from another contract"
    );
    let event = DiscoveryBountyAwarded::decode_log(&log.inner, true)?.data;
    Ok(Award {
        block: L1Block {
            number: log.block_number.context("bounty log lacks block number")?,
            hash: log.block_hash.context("bounty log lacks block hash")?,
        },
        tx_hash: log
            .transaction_hash
            .context("bounty log lacks transaction hash")?,
        log_index: log.log_index.context("bounty log lacks log index")?,
        witness_digest: event.witnessDigest,
        package_hash: event.packageHash,
        prover: event.prover,
        prover_bonus: event.proverBonus,
        committee_share: event.committeeShare,
    })
}

impl BountyLane {
    pub async fn from_env(pool: &PgPool) -> Result<Option<Self>> {
        let Ok(address) = std::env::var("CREG_INDEXER_REWARDS_ADDRESS") else {
            return Ok(None);
        };
        if address.trim().is_empty() {
            return Ok(None);
        }
        let contract: Address = address
            .parse()
            .context("invalid CREG_INDEXER_REWARDS_ADDRESS")?;
        ensure!(!contract.is_zero(), "rewards address cannot be zero");
        let start_block: u64 = std::env::var("CREG_INDEXER_L1_START_BLOCK")?.parse()?;
        let chain_id: i64 = std::env::var("CREG_INDEXER_L1_CHAIN_ID")?.parse()?;
        ensure!(chain_id > 0, "invalid CREG_INDEXER_L1_CHAIN_ID");
        let endpoints = parse_rpc_endpoints(
            &std::env::var("CREG_INDEXER_L1_RPC_URL")?,
            &std::env::var("CREG_INDEXER_L1_RPC_FALLBACK_URLS").unwrap_or_default(),
            "CREG_INDEXER_L1_RPC_URL",
            "CREG_INDEXER_L1_RPC_FALLBACK_URLS",
        )?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let lane = Self {
            chain_id,
            contract,
            start_block,
            endpoints,
            client,
        };
        sqlx::query(
            "INSERT INTO l1_bounty_cursor (l1_chain_id,rewards_address,start_block) VALUES ($1,$2,$3)
             ON CONFLICT (l1_chain_id) DO NOTHING",
        )
        .bind(chain_id).bind(format!("{contract:#x}")).bind(i64::try_from(start_block)?)
        .execute(pool).await?;
        let binding: (String, i64) = sqlx::query_as(
            "SELECT rewards_address,start_block FROM l1_bounty_cursor WHERE l1_chain_id = $1",
        )
        .bind(chain_id)
        .fetch_one(pool)
        .await?;
        ensure!(
            binding == (format!("{contract:#x}"), i64::try_from(start_block)?),
            "L1 cursor already belongs to a different contract or starting block"
        );
        Ok(Some(lane))
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value> {
        quorum_json_call(&self.endpoints, method, |endpoint| {
            let params = params.clone();
            async move {
                let mut response = self
                    .client
                    .post(endpoint)
                    .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
                    .send()
                    .await
                    .map_err(reqwest::Error::without_url)?
                    .error_for_status()
                    .map_err(reqwest::Error::without_url)?;
                let mut bytes = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(reqwest::Error::without_url)?
                {
                    ensure!(
                        bytes.len().saturating_add(chunk.len()) <= MAX_RPC_BYTES,
                        "L1 RPC response exceeds byte limit"
                    );
                    bytes.extend_from_slice(&chunk);
                }
                let result: Value = serde_json::from_slice(&bytes)?;
                ensure!(
                    result.get("id").and_then(Value::as_u64) == Some(1),
                    "invalid L1 RPC response id"
                );
                let result = result
                    .get("result")
                    .filter(|result| !result.is_null())
                    .with_context(|| format!("{method} returned no result"))?;
                if method == "eth_getBlockByNumber" {
                    // Compare only the number and hash, which identify the
                    // canonical block, across providers with different extras.
                    let block = L1Block {
                        number: quantity(result.get("number").context("L1 block lacks number")?)?,
                        hash: serde_json::from_value(
                            result.get("hash").context("L1 block lacks hash")?.clone(),
                        )?,
                    };
                    return Ok(serde_json::to_value(block)?);
                }
                Ok(result.clone())
            }
        })
        .await
    }

    async fn check_chain(&self) -> Result<()> {
        let chain = quantity(&self.rpc("eth_chainId", json!([])).await?)?;
        ensure!(
            i64::try_from(chain)? == self.chain_id,
            "L1 RPC chain ID differs from configured chain"
        );
        Ok(())
    }

    async fn block(&self, tag: &str) -> Result<L1Block> {
        Ok(serde_json::from_value(
            self.rpc("eth_getBlockByNumber", json!([tag, false]))
                .await?,
        )?)
    }

    async fn block_at(&self, number: u64) -> Result<L1Block> {
        let block = self.block(&format!("0x{number:x}")).await?;
        ensure!(
            block.number == number,
            "L1 RPC returned a different block number"
        );
        Ok(block)
    }

    async fn finalized(&self) -> Result<Option<L1Block>> {
        match self.block("finalized").await {
            Ok(block) => Ok(Some(block)),
            Err(_) => {
                let latest = self.block("latest").await?;
                let Some(confirmed) = latest.number.checked_sub(CONFIRMATIONS) else {
                    return Ok(None);
                };
                Ok(Some(self.block_at(confirmed).await?))
            }
        }
    }

    pub async fn tick(&self, pool: &PgPool) -> Result<()> {
        self.check_chain().await?;
        let previous: (Option<i64>, Option<String>) = sqlx::query_as(
            "SELECT last_block,last_block_hash FROM l1_bounty_cursor WHERE l1_chain_id = $1",
        )
        .bind(self.chain_id)
        .fetch_one(pool)
        .await?;
        let Some(finalized) = self.finalized().await? else {
            return Ok(());
        };
        let from = if let (Some(number), Some(hash)) = (&previous.0, &previous.1) {
            let number = u64::try_from(*number)?;
            if finalized.number < number {
                return Ok(());
            }
            let indexed = self.block_at(number).await?;
            ensure!(
                format!("{:#x}", indexed.hash) == *hash,
                "L1 bounty cursor hash conflict at {number}"
            );
            number.checked_add(1).context("L1 block number overflow")?
        } else {
            self.start_block
        };
        if from > finalized.number {
            return Ok(());
        }
        let to = finalized
            .number
            .min(from.saturating_add(BLOCKS_PER_BATCH - 1));
        let anchor = self.block_at(to).await?;
        ensure!(
            to != finalized.number || anchor == finalized,
            "L1 finalized boundary changed"
        );

        let logs: Vec<Log> = serde_json::from_value(
            self.rpc(
                "eth_getLogs",
                json!([{
                    "address": self.contract,
                    "fromBlock": format!("0x{from:x}"),
                    "toBlock": format!("0x{to:x}"),
                    "topics": [DiscoveryBountyAwarded::SIGNATURE_HASH],
                }]),
            )
            .await?,
        )?;
        let mut block_hashes = BTreeMap::from([(to, anchor.hash)]);
        let mut awards = Vec::with_capacity(logs.len());
        for log in &logs {
            let award = decode(log, self.contract)?;
            let number = award.block.number;
            ensure!(
                (from..=to).contains(&number),
                "L1 bounty log is outside the requested range"
            );
            let expected = match block_hashes.get(&number) {
                Some(hash) => *hash,
                None => {
                    let hash = self.block_at(number).await?.hash;
                    block_hashes.insert(number, hash);
                    hash
                }
            };
            ensure!(
                award.block.hash == expected,
                "bounty log belongs to a noncanonical L1 block"
            );
            awards.push(award);
        }
        ensure!(
            self.block_at(to).await? == anchor,
            "L1 anchor changed during log read"
        );
        awards.sort_by_key(|award| (award.block.number, award.log_index, award.tx_hash));
        self.store_batch(pool, &previous, anchor, &awards).await
    }

    async fn store_batch(
        &self,
        pool: &PgPool,
        previous: &(Option<i64>, Option<String>),
        anchor: L1Block,
        awards: &[Award],
    ) -> Result<()> {
        let mut tx = pool.begin().await?;
        let height = lock_cursor(&mut tx).await?;
        let actual: (Option<i64>, Option<String>) = sqlx::query_as(
            "SELECT last_block,last_block_hash FROM l1_bounty_cursor WHERE l1_chain_id = $1 FOR UPDATE",
        )
        .bind(self.chain_id).fetch_one(&mut *tx).await?;
        if height < 0 || actual != *previous {
            return Ok(());
        }
        let mut event_index: i64 = sqlx::query_scalar(
            "SELECT COALESCE(max(tx_index), -1) FROM index_events WHERE height = $1",
        )
        .bind(height)
        .fetch_one(&mut *tx)
        .await?;
        for award in awards {
            if insert_award(&mut tx, self.chain_id, award).await? {
                event_index += 1;
                let digest = format!("{:x}", award.witness_digest);
                let canonical: Option<String> = sqlx::query_scalar(
                    "SELECT canonical FROM trigger_witnesses WHERE witness_digest = $1",
                )
                .bind(&digest)
                .fetch_optional(&mut *tx)
                .await?;
                let event = SseEnvelopeV2::new(
                    SseEventKindV2::BountyAwarded,
                    u64::try_from(height)?,
                    u32::try_from(event_index)?,
                    canonical.as_deref(),
                    json!({
                        "l1_chain_id": self.chain_id,
                        "l1_block_number": award.block.number,
                        "l1_block_hash": format!("{:#x}", award.block.hash),
                        "l1_tx_hash": format!("{:#x}", award.tx_hash),
                        "log_index": award.log_index,
                        "witness_digest": digest,
                        "package_hash": format!("{:#x}", award.package_hash),
                        "prover_address": format!("{:#x}", award.prover),
                        "prover_bonus_wei": award.prover_bonus.to_string(),
                        "committee_share_wei": award.committee_share.to_string(),
                        "committee_share_status": "reserved",
                    }),
                );
                insert_event(&mut tx, &event).await?;
            }
        }
        sqlx::query("UPDATE l1_bounty_cursor SET last_block = $2, last_block_hash = $3 WHERE l1_chain_id = $1")
            .bind(self.chain_id).bind(i64::try_from(anchor.number)?).bind(format!("{:#x}", anchor.hash))
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn insert_award(db: &mut PgConnection, chain_id: i64, award: &Award) -> Result<bool> {
    let hash = format!("{:#x}", award.block.hash);
    let tx_hash = format!("{:#x}", award.tx_hash);
    let index = i64::try_from(award.log_index)?;
    let affected = sqlx::query(
        "INSERT INTO discovery_bounties (
             l1_chain_id,l1_block_number,l1_block_hash,l1_tx_hash,log_index,witness_digest,
             package_hash,prover_address,prover_bonus_wei,committee_share_wei,committee_share_status)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9::TEXT::NUMERIC(78,0),$10::TEXT::NUMERIC(78,0),'reserved')
         ON CONFLICT (l1_chain_id,l1_tx_hash,log_index) DO NOTHING",
    )
    .bind(chain_id).bind(i64::try_from(award.block.number)?).bind(&hash).bind(&tx_hash).bind(index)
    .bind(format!("{:x}", award.witness_digest)).bind(format!("{:#x}", award.package_hash))
    .bind(format!("{:#x}", award.prover)).bind(award.prover_bonus.to_string()).bind(award.committee_share.to_string())
    .execute(&mut *db).await?.rows_affected();
    if affected == 0 {
        let existing: String = sqlx::query_scalar(
            "SELECT l1_block_hash FROM discovery_bounties WHERE l1_chain_id = $1 AND l1_tx_hash = $2 AND log_index = $3",
        )
        .bind(chain_id).bind(&tx_hash).bind(index).fetch_one(db).await?;
        ensure!(existing == hash, "L1 bounty log hash conflict");
    }
    Ok(affected == 1)
}

fn quantity(value: &Value) -> Result<u64> {
    let Some(hex) = value.as_str().and_then(|value| value.strip_prefix("0x")) else {
        bail!("invalid L1 RPC quantity");
    };
    Ok(u64::from_str_radix(hex, 16)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_fixture() -> Value {
        let contract = Address::repeat_byte(0x11);
        let witness = B256::repeat_byte(0x22);
        let package = B256::repeat_byte(0x33);
        json!({
            "address": contract,
            "topics": [
                alloy::primitives::keccak256("DiscoveryBountyAwarded(bytes32,bytes32,address,uint256,uint256)"),
                witness, package, format!("0x{}{}", "00".repeat(12), "44".repeat(20)),
            ],
            "data": format!("0x80{}{:064x}", "00".repeat(31), 75),
            "blockNumber": "0x2a",
            "blockHash": B256::repeat_byte(0x55),
            "transactionHash": B256::repeat_byte(0x66),
            "transactionIndex": "0x1",
            "logIndex": "0x2",
            "removed": false,
        })
    }

    #[test]
    fn discovery_bounty_awarded_log_fixture() -> Result<()> {
        let contract = Address::repeat_byte(0x11);
        let witness = B256::repeat_byte(0x22);
        let package = B256::repeat_byte(0x33);
        let prover = Address::repeat_byte(0x44);
        let log: Log = serde_json::from_value(log_fixture())?;
        let award = decode(&log, contract)?;
        assert_eq!(
            award.block,
            L1Block {
                number: 42,
                hash: B256::repeat_byte(0x55)
            }
        );
        assert_eq!(award.tx_hash, B256::repeat_byte(0x66));
        assert_eq!(award.log_index, 2);
        assert_eq!(award.witness_digest, witness);
        assert_eq!(format!("{:x}", award.witness_digest), "22".repeat(32));
        assert_eq!(award.package_hash, package);
        assert_eq!(award.prover, prover);
        assert_eq!(award.prover_bonus, U256::ONE << 255_usize);
        assert_eq!(award.committee_share, U256::from(75));
        assert!(decode(&log, Address::ZERO).is_err());
        let mut removed = log;
        removed.removed = true;
        assert!(decode(&removed, contract).is_err());
        Ok(())
    }
    #[tokio::test]
    #[ignore = "requires a PostgreSQL test database"]
    async fn confirmed_bounty_round_trips_2_to_255_and_joins_later_witness() -> Result<()> {
        crate::test_db::isolate(|pool| async move {
            use std::sync::{atomic::AtomicBool, Arc};
            use axum::{routing::post, Json, Router};
            use registry_view::index::BountyLeaderboardV1;

            crate::MIGRATOR.run(&pool).await?;
            let genesis = common::Block::genesis();
            crate::apply::apply_block(&pool, &genesis).await?;
            let witness = common::TriggerWitnessV1 {
                schema_version: 1,
                package_canonical: "npm:fixture@1.0.0".into(),
                content_hash: "aa".repeat(32),
                manifest_sha256: "bb".repeat(32),
                sandbox_policy_bundle_id: "fixture-policy".into(),
                trigger_spec: common::TriggerSpecWireV1 { schema_version: 1, ..Default::default() },
                observed_violations: vec!["KB-MEMFD-EXEC".into()],
                graph_canonical_digest: "cc".repeat(32),
                witness_prover_id: "fixture-prover".into(),
                prover_key_id: "dd".repeat(32),
                prover_signature: "ee".repeat(64),
                prover_pq_signature: None,
            };
            let mut log = log_fixture();
            log["topics"][1] = json!(format!("0x{}", witness.canonical_digest()));
            log["topics"][2] = json!(alloy::primitives::keccak256(&witness.package_canonical));
            let contract = Address::repeat_byte(0x11);
            sqlx::query("INSERT INTO l1_bounty_cursor (l1_chain_id,rewards_address,start_block) VALUES (1,$1,42)")
                .bind(format!("{contract:#x}")).execute(&pool).await?;
            let rpc = Router::new().route("/", post(move |Json(request): Json<Value>| {
                let log = log.clone();
                async move {
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x1"),
                    "eth_getBlockByNumber" => match request["params"][0].as_str().unwrap() {
                        "finalized" => return Json(json!({"id":1,"jsonrpc":"2.0","error":{"code":-32602,"message":"unsupported"}})),
                        "latest" => json!({"number":"0x36","hash":B256::repeat_byte(0x77)}),
                        "0x2a" => json!({"number":"0x2a","hash":B256::repeat_byte(0x55)}),
                        tag => panic!("unexpected block tag {tag}"),
                    },
                    "eth_getLogs" => {
                        assert_eq!(request["params"][0]["fromBlock"], "0x2a");
                        assert_eq!(request["params"][0]["toBlock"], "0x2a");
                        json!([log])
                    }
                    method => panic!("unexpected method {method}"),
                };
                Json(json!({"id":1,"jsonrpc":"2.0","result":result}))
                }
            }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let endpoint = format!("http://{}", listener.local_addr()?);
            let rpc_task = tokio::spawn(async move { axum::serve(listener, rpc).await });
            let lane = BountyLane {
                chain_id: 1, contract, start_block: 42, endpoints: vec![endpoint],
                client: reqwest::Client::new(),
            };
            lane.tick(&pool).await?;
            lane.tick(&pool).await?;
            rpc_task.abort();

            let (amount, status, block_hash, canonical): (String, String, String, Option<String>) = sqlx::query_as(
                "SELECT prover_bonus_wei::TEXT,committee_share_status,l1_block_hash,canonical
                 FROM v_discovery_bounty_witnesses",
            ).fetch_one(&pool).await?;
            assert_eq!(amount, (U256::ONE << 255_usize).to_string());
            assert_eq!(status, "reserved");
            assert_eq!(block_hash, format!("{:#x}", B256::repeat_byte(0x55)));
            assert_eq!(canonical, None);
            let amount_type: (i32, i32) = sqlx::query_as(
                "SELECT numeric_precision,numeric_scale FROM information_schema.columns
                 WHERE table_schema = current_schema() AND table_name = 'discovery_bounties'
                   AND column_name = 'prover_bonus_wei'",
            ).fetch_one(&pool).await?;
            assert_eq!(amount_type, (78, 0));

            let mut header = genesis.header.clone();
            header.height = 1;
            header.prev_hash = genesis.hash();
            header.status_root = Some([1; 32]);
            let transaction = common::Transaction::RevokeV2 {
                request: common::SignedRevocationRequest {
                    chain_id: "fixture".into(),
                    genesis_hash: genesis.hash(),
                    package_canonical: witness.package_canonical.clone(),
                    reason: "fixture".into(),
                    revoker_pubkey: "fixture-key".into(),
                    signature: "fixture-signature".into(),
                    signature_version: 1,
                },
                evidence_hash: "ff".repeat(32),
                witness: Some(witness),
                quorum_certificate: None,
            };
            let transactions = vec![transaction];
            header.merkle_root = common::merkle_root(&transactions);
            crate::apply::apply_block(&pool, &common::Block {
                header, transactions, pbft_signatures: Vec::new(),
            }).await?;
            let api = crate::api::ApiState::new(pool.clone(), Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(true))).router();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let url = format!("http://{}/v1/index/bounties/leaderboard", listener.local_addr()?);
            let api_task = tokio::spawn(async move { axum::serve(listener, api).await });
            let leaderboard: BountyLeaderboardV1 = reqwest::get(url).await?.error_for_status()?.json().await?;
            api_task.abort();
            assert_eq!(leaderboard.entries.len(), 1);
            let leader = &leaderboard.entries[0];
            assert_eq!(leader.prover_bonus_wei, amount);
            assert_eq!(leader.committee_reserved_wei, "75");
            assert_eq!(leader.committee_distributed_wei, "0");
            assert_eq!(leader.matched_witnesses, 1);
            assert_eq!(leader.latest_award.canonical.as_ref().unwrap().as_str(), "npm:fixture@1.0.0");
            let events: Vec<String> = sqlx::query_scalar("SELECT envelope->>'kind' FROM index_events ORDER BY height,tx_index")
                .fetch_all(&pool).await?;
            assert_eq!(events, ["block_committed", "bounty_awarded", "package_revoked_v2", "block_committed"]);
            Ok(())
        }).await
    }
}
