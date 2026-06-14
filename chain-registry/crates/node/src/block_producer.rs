// crates/node/src/block_producer.rs
// Produces new blocks on a fixed interval from a durable finalized-tx buffer.

use crate::{finalized_tx, NodeState};
use chrono::Utc;
use common::{merkle_root, transaction_hash, Block, BlockHeader, Transaction};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

/// Snapshot gathered after the VRF proposer gate passes.
struct ProposerContext {
    tip_height: u64,
    prev_hash: String,
    node_id: String,
    privkey: Option<String>,
    our_pubkey: Option<String>,
    validator_set_hash: String,
    vrf_output: Option<String>,
    vrf_proof: Option<String>,
}

pub async fn run(
    state: Arc<RwLock<NodeState>>,
    rx: finalized_tx::FinalizedTxReceiver,
    p2p_handle: crate::p2p::P2PHandle,
) {
    let block_interval = {
        let s = state.read().await;
        s.config.block_interval_secs
    };

    let mut ticker = interval(Duration::from_secs(block_interval));
    // How long the chain tip may stall before the next-ranked proposer is
    // allowed to step in. Each elapsed window promotes one more fallback rank,
    // so a single offline proposer no longer halts block production.
    let fallback_window_secs = std::env::var("CREG_PROPOSER_FALLBACK_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(block_interval.saturating_mul(2).max(1));
    tracing::info!(
        "Block producer started (interval: {}s, proposer fallback window: {}s)",
        block_interval,
        fallback_window_secs
    );

    let mut last_seen_tip: u64 = {
        let s = state.read().await;
        s.chain.tip_height().unwrap_or(0)
    };
    let mut tip_unchanged_since = std::time::Instant::now();
    let mut pending_txs: Vec<Transaction> = Vec::new();

    loop {
        ticker.tick().await;

        // Track how long the tip has been stalled. Re-reading every tick means
        // blocks produced by peers reset the timer too, so fallback only
        // engages during a genuine production stall.
        let current_tip = {
            let s = state.read().await;
            s.chain.tip_height().unwrap_or(last_seen_tip)
        };
        if current_tip != last_seen_tip {
            last_seen_tip = current_tip;
            tip_unchanged_since = std::time::Instant::now();
        }
        let stall_secs = tip_unchanged_since.elapsed().as_secs();
        let allowed_fallback_rank = (stall_secs / fallback_window_secs) as usize;

        // Always move channel deliveries into the durable buffer; never drop
        // because this node is not the current VRF proposer.
        finalized_tx::recv_into_buffer(&rx, &mut pending_txs).await;
        if pending_txs.is_empty() {
            tracing::debug!("Block producer: no pending finalized transactions");
            continue;
        }

        let ctx = match prepare_proposer_context(
            Arc::clone(&state),
            &p2p_handle,
            allowed_fallback_rank,
        )
        .await
        {
            Ok(ctx) => ctx,
            Err(e) => {
                tracing::debug!(
                    pending = pending_txs.len(),
                    "Block producer: not proposer this tick — keeping {} buffered tx(s): {}",
                    pending_txs.len(),
                    e
                );
                continue;
            }
        };

        {
            let s = state.read().await;
            let height = s.chain.tip_height().unwrap_or(0);
            let forced: Vec<Transaction> = s
                .forced_inclusion_tracker
                .forced_transaction_payloads(height)
                .into_iter()
                .cloned()
                .collect();
            if !forced.is_empty() {
                let mut seen: HashSet<String> =
                    pending_txs.iter().map(transaction_hash).collect();
                for tx in forced.into_iter().rev() {
                    let hash = transaction_hash(&tx);
                    if seen.insert(hash) {
                        pending_txs.insert(0, tx);
                    }
                }
            }
        }

        let txs: Vec<Transaction> = pending_txs.drain(..).collect();
        finalized_tx::sync_pending_buffer_depth(pending_txs.len());
        let txs_backup = txs.clone();

        match produce_block(Arc::clone(&state), txs, p2p_handle.clone(), ctx).await {
            Ok(block) => {
                let bh = block.hash();
                tracing::info!(
                    "[PBFT] Proposer created block {} at height {} ({} tx) — starting round",
                    &bh[..bh.len().min(12)],
                    block.header.height,
                    block.transactions.len()
                );

                // Broadcast PbftPrePrepare to start the consensus round
                let msg = common::GossipMessage::PbftPrePrepare { block };
                match serde_json::to_vec(&msg) {
                    Ok(data) => {
                        let _ = p2p_handle
                            .sender
                            .send(crate::p2p::P2PCommand::Broadcast {
                                topic: "creg/v1/blocks".into(),
                                data,
                            })
                            .await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to serialize PbftPrePrepare gossip: {}", e);
                    }
                }
            }
            Err(e) => {
                tracing::error!("Block production failed: {}", e);
                finalized_tx::requeue_front(&mut pending_txs, txs_backup);
            }
        }
    }
}

/// Evaluate VRF proposer ranking and broadcast our proof when eligible.
async fn prepare_proposer_context(
    state: Arc<RwLock<NodeState>>,
    p2p: &crate::p2p::P2PHandle,
    allowed_fallback_rank: usize,
) -> anyhow::Result<ProposerContext> {
    let (tip_height, prev_hash, node_id, privkey, our_pubkey, validator_set_hash) = {
        let s = state.read().await;
        let tip_height = s.chain.tip_height()?;
        let prev_hash = s.chain.tip_hash()?;
        let node_id = s.config.node_id.clone();
        let privkey = s.config.validator_privkey.clone();
        let our_pubkey = s
            .validator_set
            .validators
            .iter()
            .find(|v| v.id == node_id)
            .map(|v| v.pubkey.clone());

        let mut sorted_ids: Vec<&str> = s
            .validator_set
            .validators
            .iter()
            .map(|v| v.id.as_str())
            .collect();
        sorted_ids.sort_unstable();
        let mut hasher = Sha256::new();
        for id in &sorted_ids {
            hasher.update(id.as_bytes());
            hasher.update(b"\0");
        }
        let validator_set_hash = hex::encode(hasher.finalize());

        (
            tip_height,
            prev_hash,
            node_id,
            privkey,
            our_pubkey,
            validator_set_hash,
        )
    };

    let epoch_seed = prev_hash.clone();

    let mut active: Vec<consensus::vrf::VrfValidator> = {
        let s = state.read().await;
        s.validator_set
            .validators
            .iter()
            .filter(|v| v.status == "online" || v.status == "self")
            .map(|v| {
                let (vrf_output, vrf_proof) = s
                    .vrf_proofs
                    .get(&v.id)
                    .cloned()
                    .map(|(o, p)| (Some(o), Some(p)))
                    .unwrap_or((None, None));
                consensus::vrf::VrfValidator {
                    id: v.id.clone(),
                    pubkey: v.pubkey.clone(),
                    vrf_output,
                    vrf_proof,
                }
            })
            .collect()
    };

    let (vrf_output, vrf_proof) = if !active.is_empty() {
        if let Some(ref privkey) = privkey {
            let (out, prf) = consensus::vrf::prove(epoch_seed.as_bytes(), privkey)?;
            for v in &mut active {
                if v.id == node_id {
                    v.vrf_output = Some(out.clone());
                    v.vrf_proof = Some(prf.clone());
                }
            }

            let gossip_msg = common::GossipMessage::VrfProof {
                validator_id: node_id.clone(),
                pubkey: our_pubkey.clone().unwrap_or_default(),
                epoch_seed: epoch_seed.clone(),
                output: out.clone(),
                proof: prf.clone(),
            };
            if let Ok(data) = serde_json::to_vec(&gossip_msg) {
                let _ = p2p
                    .sender
                    .send(crate::p2p::P2PCommand::Broadcast {
                        topic: "creg/v1/vrf-proofs".into(),
                        data,
                    })
                    .await;
            }

            let ranking = consensus::vrf::rank_proposers(&active, &epoch_seed);
            if ranking.is_empty() {
                anyhow::bail!("No active validators to select proposer");
            }
            let effective_rank = allowed_fallback_rank.min(ranking.len().saturating_sub(1));
            match ranking.iter().position(|id| id == &node_id) {
                Some(0) => {}
                Some(rank) if rank == effective_rank => {
                    tracing::warn!(
                        "Proposer fallback engaged: tip stalled, node {} stepping in as rank-{} proposer (primary appears offline)",
                        node_id,
                        rank
                    );
                }
                Some(rank) => {
                    anyhow::bail!(
                        "Node {} is proposer rank {} for this epoch; not its turn yet (allowed fallback rank {})",
                        node_id,
                        rank,
                        effective_rank
                    );
                }
                None => {
                    anyhow::bail!(
                        "Node {} is not in the active proposer set for this epoch",
                        node_id
                    );
                }
            }
            (Some(out), Some(prf))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    Ok(ProposerContext {
        tip_height,
        prev_hash,
        node_id,
        privkey,
        our_pubkey,
        validator_set_hash,
        vrf_output,
        vrf_proof,
    })
}

async fn produce_block(
    state: Arc<RwLock<NodeState>>,
    txs: Vec<Transaction>,
    p2p: crate::p2p::P2PHandle,
    ctx: ProposerContext,
) -> anyhow::Result<Block> {
    let ProposerContext {
        tip_height,
        prev_hash: _,
        node_id,
        privkey,
        our_pubkey: _,
        validator_set_hash,
        vrf_output,
        vrf_proof,
    } = ctx;

    let mut s = state.write().await;
    let header = BlockHeader {
        height: tip_height + 1,
        prev_hash: s.chain.tip_hash()?,
        merkle_root: merkle_root(&txs),
        proposer_id: node_id.clone(),
        timestamp: Utc::now(),
        validator_set_hash,
        vrf_output,
        vrf_proof,
    };

    let block = Block {
        header,
        transactions: txs,
        pbft_signatures: vec![],
    };

    let vs = s.validator_set.clone();
    s.pbft_engine.start_round(block.clone(), vs.into())?;

    let bh = block.hash();
    let mut prep_cmd = None;
    let mut commit_cmd = None;

    if let Some(ref privkey_hex) = privkey {
        if let Ok(pk_bytes) = hex::decode(privkey_hex) {
            if let Ok(sk) = ed25519_dalek::SigningKey::try_from(pk_bytes.as_slice()) {
                use ed25519_dalek::Signer;

                let prep_msg_str = consensus::pbft::pbft_signature_message("prepare", &bh);
                let prep_sig = hex::encode(sk.sign(prep_msg_str.as_bytes()).to_bytes());
                let pubkey = hex::encode(sk.verifying_key().as_bytes());

                let prep_sig_obj = common::BlockSignature {
                    validator_id: node_id.clone(),
                    pubkey: pubkey.clone(),
                    signature: prep_sig.clone(),
                };

                let prepare_quorum_reached = s
                    .pbft_engine
                    .prepare(&bh, &node_id, prep_sig_obj)
                    .unwrap_or(false);

                let prep_msg = common::GossipMessage::PbftPrepare {
                    block_hash: bh.clone(),
                    validator_id: node_id.clone(),
                    signature: prep_sig,
                };
                if let Ok(data) = serde_json::to_vec(&prep_msg) {
                    prep_cmd = Some(crate::p2p::P2PCommand::Broadcast {
                        topic: "creg/v1/blocks".into(),
                        data,
                    });
                }

                if prepare_quorum_reached {
                    let commit_msg_str = consensus::pbft::pbft_signature_message("commit", &bh);
                    let commit_sig = hex::encode(sk.sign(commit_msg_str.as_bytes()).to_bytes());

                    let commit_sig_obj = common::BlockSignature {
                        validator_id: node_id.clone(),
                        pubkey,
                        signature: commit_sig.clone(),
                    };

                    let commit_quorum_reached = s
                        .pbft_engine
                        .commit(&bh, &node_id, commit_sig_obj)
                        .unwrap_or(false);

                    let commit_msg = common::GossipMessage::PbftCommit {
                        block_hash: bh.clone(),
                        validator_id: node_id.clone(),
                        signature: commit_sig,
                    };
                    if let Ok(data) = serde_json::to_vec(&commit_msg) {
                        commit_cmd = Some(crate::p2p::P2PCommand::Broadcast {
                            topic: "creg/v1/blocks".into(),
                            data,
                        });
                    }

                    if commit_quorum_reached {
                        tracing::info!(
                            "[PBFT Proposer] Block {} finalised locally by proposer quorum",
                            &bh[..12]
                        );
                        if let Some(final_block) = s.pbft_engine.get_finalised_block(&bh) {
                            match s.chain.insert_block_with_outcome(&final_block) {
                                Ok(outcome) => {
                                    if let Some(replaced) = outcome.replaced_hash {
                                        s.record_reorg(1, vec![replaced], outcome.hash.clone());
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "[PBFT Proposer] Failed to insert finalised block: {}",
                                        e
                                    );
                                }
                            }
                            s.publisher_index.apply_block(&final_block);
                            s.on_block_committed(&final_block);
                        }
                    }
                }
            }
        }
    }

    s.vrf_proofs.clear();

    drop(s);

    if let Some(cmd) = prep_cmd {
        let _ = p2p.sender.send(cmd).await;
    }
    if let Some(cmd) = commit_cmd {
        let _ = p2p.sender.send(cmd).await;
    }

    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        chain_store::ChainStore,
        config::NodeConfig,
        finalized_tx,
        p2p::{P2PCommand, P2PHandle},
        pending_pool::PendingPool,
        publisher_index::PublisherIndex,
        state::PbftEngine,
        BridgeStatus, NodeState, P2PStatus,
    };
    use chrono::Utc;
    use common::{
        ChainRecord, PackageId, PackageStatus, Transaction, Validator, ValidatorSet,
    };
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use tempfile::TempDir;

    fn sample_publish_tx(name: &str) -> Transaction {
        Transaction::Publish(ChainRecord {
            id: PackageId::new("npm", name, "1.0.0"),
            content_hash: "abc".into(),
            ipfs_cid: "bafy".into(),
            publisher_pubkey: "pk".into(),
            block_hash: "0".repeat(64),
            published_at: Utc::now(),
            validator_signatures: vec![],
            status: PackageStatus::Verified,
            ..Default::default()
        })
    }

    async fn two_validator_state(
        primary_id: &str,
        secondary_id: &str,
    ) -> (Arc<RwLock<NodeState>>, TempDir, SigningKey, SigningKey) {
        let dir = TempDir::new().unwrap();
        let chain = ChainStore::open(dir.path()).unwrap();

        let primary_sk = SigningKey::generate(&mut OsRng);
        let secondary_sk = SigningKey::generate(&mut OsRng);

        let validators = vec![
            Validator {
                id: primary_id.into(),
                alias: "primary".into(),
                pubkey: hex::encode(primary_sk.verifying_key().as_bytes()),
                eth_address: "0x1111111111111111111111111111111111111111".into(),
                stake: 1000,
                reputation: 100,
                status: "online".into(),
            },
            Validator {
                id: secondary_id.into(),
                alias: "secondary".into(),
                pubkey: hex::encode(secondary_sk.verifying_key().as_bytes()),
                eth_address: "0x2222222222222222222222222222222222222222".into(),
                stake: 900,
                reputation: 100,
                status: "online".into(),
            },
        ];

        let state = Arc::new(RwLock::new(NodeState {
            chain,
            pending_pool: PendingPool::new(),
            publisher_index: PublisherIndex::new(),
            validator_set_bootstrap: ValidatorSet::default(),
            validator_set: ValidatorSet::new(validators),
            package_rounds: std::collections::HashMap::new(),
            config: NodeConfig {
                node_id: primary_id.into(),
                data_dir: dir.path().to_path_buf(),
                block_interval_secs: 1,
                ..NodeConfig::default()
            },
            p2p_status: P2PStatus::default(),
            bridge_status: BridgeStatus::default(),
            vrf_proofs: std::collections::HashMap::new(),
            decryption_shares: std::collections::HashMap::new(),
            validator_registrations: std::collections::HashMap::new(),
            validator_set_sync: crate::state::ValidatorSetSyncStatus::default(),
            view_change_certs: std::collections::HashMap::new(),
            reorgs: Vec::new(),
            pbft_engine: PbftEngine::new(),
            forced_inclusion_tracker: crate::state::ForcedInclusionTracker::new(),
            sync_lag_blocks: 0,
            sync_max_peer_tip: 0,
        }));

        (state, dir, primary_sk, secondary_sk)
    }

    fn noop_p2p() -> P2PHandle {
        let (sender, _rx) = tokio::sync::mpsc::channel::<P2PCommand>(1);
        P2PHandle { sender }
    }

    async fn proposer_ranking(
        state: &Arc<RwLock<NodeState>>,
    ) -> anyhow::Result<(String, String)> {
        let s = state.read().await;
        let epoch_seed = s.chain.tip_hash()?;
        let active: Vec<consensus::vrf::VrfValidator> = s
            .validator_set
            .validators
            .iter()
            .map(|v| consensus::vrf::VrfValidator {
                id: v.id.clone(),
                pubkey: v.pubkey.clone(),
                vrf_output: None,
                vrf_proof: None,
            })
            .collect();
        let ranking = consensus::vrf::rank_proposers(&active, &epoch_seed);
        anyhow::ensure!(
            ranking.len() >= 2,
            "expected two validators in proposer ranking"
        );
        Ok((ranking[0].clone(), ranking[1].clone()))
    }

    fn privkey_for_id(
        id: &str,
        primary_id: &str,
        primary_sk: &SigningKey,
        secondary_id: &str,
        secondary_sk: &SigningKey,
    ) -> String {
        if id == primary_id {
            hex::encode(primary_sk.to_bytes())
        } else if id == secondary_id {
            hex::encode(secondary_sk.to_bytes())
        } else {
            panic!("unexpected validator id {id}");
        }
    }

    #[tokio::test]
    async fn non_proposer_tick_keeps_buffered_transactions() {
        let (state, _dir, primary_sk, secondary_sk) =
            two_validator_state("validator-a", "validator-b").await;
        let (rank0, rank1) = proposer_ranking(&state).await.expect("proposer ranking");
        let (tx_sender, tx_receiver) = finalized_tx::channel();
        let p2p = noop_p2p();

        tx_sender.send(sample_publish_tx("buffered")).await.unwrap();

        let mut pending_txs = Vec::new();
        finalized_tx::recv_into_buffer(&tx_receiver, &mut pending_txs).await;
        assert_eq!(pending_txs.len(), 1);

        {
            let mut s = state.write().await;
            s.config.node_id = rank1.clone();
            s.config.validator_privkey = Some(privkey_for_id(
                &rank1,
                "validator-a",
                &primary_sk,
                "validator-b",
                &secondary_sk,
            ));
        }

        let gate = prepare_proposer_context(Arc::clone(&state), &p2p, 0).await;
        assert!(
            gate.is_err(),
            "rank-1 validator {rank1} should not propose on tick 0 (primary is {rank0})"
        );
        let err = gate.err().unwrap();
        assert!(
            err.to_string().contains("not its turn"),
            "unexpected error: {err}"
        );
        assert_eq!(pending_txs.len(), 1, "buffer must retain the transaction");
    }

    #[tokio::test]
    async fn proposer_consumes_buffer_and_advances_chain() {
        let (state, _dir, primary_sk, secondary_sk) =
            two_validator_state("validator-a", "validator-b").await;
        let (rank0, _rank1) = proposer_ranking(&state).await.expect("proposer ranking");
        let p2p = noop_p2p();

        {
            let mut s = state.write().await;
            s.config.node_id = rank0.clone();
            s.config.validator_privkey = Some(privkey_for_id(
                &rank0,
                "validator-a",
                &primary_sk,
                "validator-b",
                &secondary_sk,
            ));
        }

        let mut pending_txs = vec![sample_publish_tx("committed")];
        let ctx = prepare_proposer_context(Arc::clone(&state), &p2p, 0)
            .await
            .unwrap_or_else(|e| panic!("rank-0 validator {rank0} should be allowed to propose: {e}"));

        let txs: Vec<Transaction> = pending_txs.drain(..).collect();
        let block = produce_block(Arc::clone(&state), txs, p2p, ctx)
            .await
            .expect("produce_block should succeed for rank-0 proposer");
        assert_eq!(block.transactions.len(), 1);
        assert_eq!(block.header.height, 1);
        assert!(pending_txs.is_empty());
    }
}
