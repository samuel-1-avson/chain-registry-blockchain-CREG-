//! Signer actor tests against a mock chain with a mempool, replacement rules
//! and mining. Includes `eco-adv-10` (ADR-019 §10).

use super::*;
use crate::{
    ledger::InMemoryNonceLedger,
    rpc::{FeeEstimate, ReceiptSummary},
};
use alloy::{
    consensus::{Transaction, TxEnvelope},
    eips::eip2718::Decodable2718,
};
use async_trait::async_trait;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
    time::Instant,
};

/// Anvil's first default development key (public test data).
const TEST_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const CHAIN_ID: u64 = 31_337;
const GWEI: u128 = 1_000_000_000;

#[derive(Clone)]
struct PoolTx {
    hash: B256,
    max_fee: u128,
    priority: u128,
}

#[derive(Default)]
struct MockState {
    mined_nonce: u64,
    pool: BTreeMap<u64, PoolTx>,
    receipts: HashMap<B256, (u64, bool)>,
    head: u64,
    fees: Option<FeeEstimate>,
    outcomes: Vec<BroadcastOutcome>,
    /// Make the next N broadcasts fail in transit without reaching the pool.
    drop_next: usize,
    /// Refuse every broadcast with this message.
    reject_with: Option<String>,
}

struct MockChain {
    state: Mutex<MockState>,
}

impl MockChain {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(MockState {
                fees: Some(FeeEstimate {
                    max_fee_per_gas: 20 * GWEI,
                    max_priority_fee_per_gas: GWEI,
                }),
                head: 100,
                ..MockState::default()
            }),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&mut MockState) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }

    /// Mine every contiguous pool transaction starting at the account nonce.
    fn mine_all(&self) {
        self.with(|state| {
            while let Some(tx) = state.pool.remove(&state.mined_nonce) {
                state.head += 1;
                let head = state.head;
                state.receipts.insert(tx.hash, (head, true));
                state.mined_nonce += 1;
            }
        });
    }

    fn advance(&self, blocks: u64) {
        self.with(|state| state.head += blocks);
    }

    fn outcomes(&self) -> Vec<BroadcastOutcome> {
        self.with(|state| state.outcomes.clone())
    }
}

#[async_trait]
impl L1Rpc for MockChain {
    async fn chain_id(&self) -> anyhow::Result<u64> {
        Ok(CHAIN_ID)
    }

    async fn transaction_count(&self, _address: Address, pending: bool) -> anyhow::Result<u64> {
        Ok(self.with(|state| {
            if !pending {
                return state.mined_nonce;
            }
            let mut next = state.mined_nonce;
            while state.pool.contains_key(&next) {
                next += 1;
            }
            next
        }))
    }

    async fn fee_estimate(&self) -> anyhow::Result<FeeEstimate> {
        self.with(|state| state.fees)
            .ok_or_else(|| anyhow!("fee oracle unavailable"))
    }

    async fn send_raw_transaction(&self, raw: &Bytes) -> BroadcastOutcome {
        let envelope = TxEnvelope::decode_2718(&mut raw.as_ref()).expect("valid signed tx");
        let nonce = envelope.nonce();
        let hash = *envelope.tx_hash();
        let max_fee = envelope.max_fee_per_gas();
        let priority = envelope.max_priority_fee_per_gas().unwrap_or_default();
        self.with(|state| {
            let outcome = if state.drop_next > 0 {
                state.drop_next -= 1;
                BroadcastOutcome::Uncertain("connection reset".to_string())
            } else if let Some(message) = state.reject_with.clone() {
                BroadcastOutcome::Rejected(message)
            } else if nonce < state.mined_nonce {
                BroadcastOutcome::NonceTooLow
            } else if let Some(existing) = state.pool.get(&nonce).cloned() {
                if existing.hash == hash {
                    BroadcastOutcome::AlreadyKnown
                } else if max_fee * 100 >= existing.max_fee * 110
                    && priority * 100 >= existing.priority * 110
                {
                    state.pool.insert(
                        nonce,
                        PoolTx {
                            hash,
                            max_fee,
                            priority,
                        },
                    );
                    BroadcastOutcome::Accepted(hash)
                } else {
                    BroadcastOutcome::Underpriced
                }
            } else {
                state.pool.insert(
                    nonce,
                    PoolTx {
                        hash,
                        max_fee,
                        priority,
                    },
                );
                BroadcastOutcome::Accepted(hash)
            };
            state.outcomes.push(outcome.clone());
            outcome
        })
    }

    async fn receipt(&self, tx_hash: B256) -> anyhow::Result<Option<ReceiptSummary>> {
        Ok(self.with(|state| {
            state
                .receipts
                .get(&tx_hash)
                .map(|(block_number, success)| ReceiptSummary {
                    tx_hash,
                    block_number: *block_number,
                    success: *success,
                })
        }))
    }

    async fn block_number(&self) -> anyhow::Result<u64> {
        Ok(self.with(|state| state.head))
    }
}

fn fast_config() -> SignerConfig {
    SignerConfig {
        confirmations: 2,
        reconcile_interval: Duration::from_millis(10),
        bump_after: Duration::from_secs(3600),
        max_open_transactions: 256,
        displaced_after_passes: 3,
        ..SignerConfig::default()
    }
}

async fn spawn(
    chain: &Arc<MockChain>,
    ledger: &InMemoryNonceLedger,
    config: SignerConfig,
) -> anyhow::Result<(SignerHandle, JoinHandle<()>)> {
    SignerActor::spawn(
        Zeroizing::new(TEST_KEY.to_string()),
        CHAIN_ID,
        Arc::clone(chain),
        Arc::new(ledger.clone()),
        config,
    )
    .await
}

fn request(id: &str) -> SubmitRequest {
    SubmitRequest {
        request_id: id.to_string(),
        to: Address::repeat_byte(0x42),
        value: U256::from(1u64),
        input: Bytes::from_static(&[0xde, 0xad]),
        gas_limit: 60_000,
    }
}

async fn wait_for_state(handle: &SignerHandle, request_id: &str, state: TxState) -> LedgerTx {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(row) = handle.status(request_id).await.unwrap() {
            if row.state == state {
                return row;
            }
        }
        assert!(
            Instant::now() < deadline,
            "{request_id} did not reach {}",
            state.as_str()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn eco_adv_10_concurrent_submissions_get_contiguous_nonces() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let (handle, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    let mut tasks = Vec::new();
    for index in 0..100 {
        let handle = handle.clone();
        // Half "sponsor" and half "drip" requests, as in eco-adv-10.
        let kind = if index % 2 == 0 { "sponsor" } else { "drip" };
        tasks.push(tokio::spawn(async move {
            handle.submit(request(&format!("{kind}-{index}"))).await
        }));
    }
    let mut nonces = Vec::new();
    for task in tasks {
        let ticket = task.await.unwrap().expect("every submission succeeds");
        assert_eq!(ticket.broadcast, BroadcastStatus::Accepted);
        nonces.push(ticket.nonce);
    }
    nonces.sort_unstable();
    assert_eq!(nonces, (0..100).collect::<Vec<u64>>());

    let outcomes = chain.outcomes();
    assert_eq!(outcomes.len(), 100);
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, BroadcastOutcome::Accepted(_))),
        "no 'nonce too low' or 'replacement underpriced' errors: {outcomes:?}"
    );
    assert_eq!(chain.with(|state| state.pool.len()), 100);

    chain.mine_all();
    chain.advance(2);
    for index in 0..100 {
        let kind = if index % 2 == 0 { "sponsor" } else { "drip" };
        wait_for_state(&handle, &format!("{kind}-{index}"), TxState::Confirmed).await;
    }
    assert!(ledger.open(handle.key()).await.unwrap().is_empty());
}

#[tokio::test]
async fn resubmitting_a_request_id_is_idempotent() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let (handle, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    let first = handle.submit(request("same")).await.unwrap();
    let second = handle.submit(request("same")).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(chain.outcomes().len(), 1);
}

#[tokio::test]
async fn uncertain_broadcast_is_rebroadcast_with_the_same_bytes() {
    let chain = MockChain::new();
    chain.with(|state| state.drop_next = 1);
    let ledger = InMemoryNonceLedger::new();
    let (handle, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    let ticket = handle.submit(request("flaky")).await.unwrap();
    assert_eq!(ticket.broadcast, BroadcastStatus::Uncertain);
    assert_eq!(ticket.nonce, 0);

    // The nonce stays consumed while the outcome is unknown.
    let next = handle.submit(request("after")).await.unwrap();
    assert_eq!(next.nonce, 1);

    let row = wait_for_state(&handle, "flaky", TxState::Pending).await;
    assert_eq!(row.attempts.len(), 1, "same bytes, no re-signing");
    assert_eq!(chain.with(|state| state.pool[&0].hash), ticket.tx_hash);
}

#[tokio::test]
async fn definitive_rejection_releases_the_nonce() {
    let chain = MockChain::new();
    chain.with(|state| {
        state.reject_with = Some("insufficient funds for gas * price + value".to_string())
    });
    let ledger = InMemoryNonceLedger::new();
    let (handle, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    let error = handle.submit(request("broke")).await.unwrap_err();
    assert!(matches!(error, SubmitError::Rejected(ref m) if m.contains("insufficient funds")));
    assert_eq!(
        handle.status("broke").await.unwrap().unwrap().state,
        TxState::Rejected
    );

    chain.with(|state| state.reject_with = None);
    let ticket = handle.submit(request("funded")).await.unwrap();
    assert_eq!(
        ticket.nonce, 0,
        "the rejected request did not consume a nonce"
    );
}

#[tokio::test]
async fn fee_above_cap_is_refused_before_signing() {
    let chain = MockChain::new();
    chain.with(|state| {
        state.fees = Some(FeeEstimate {
            max_fee_per_gas: 900 * GWEI,
            max_priority_fee_per_gas: GWEI,
        })
    });
    let ledger = InMemoryNonceLedger::new();
    let (handle, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    let error = handle.submit(request("spike")).await.unwrap_err();
    assert!(matches!(error, SubmitError::FeeAboveCap { .. }));
    assert!(handle.status("spike").await.unwrap().is_none());
    assert!(chain.outcomes().is_empty());
}

#[tokio::test]
async fn invalid_requests_are_refused() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let (handle, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    let mut bad_id = request("x");
    bad_id.request_id = "has space".to_string();
    assert!(matches!(
        handle.submit(bad_id).await,
        Err(SubmitError::InvalidRequest(_))
    ));
    let mut no_gas = request("y");
    no_gas.gas_limit = 0;
    assert!(matches!(
        handle.submit(no_gas).await,
        Err(SubmitError::InvalidRequest(_))
    ));
}

#[tokio::test]
async fn stuck_transaction_is_replaced_with_higher_fees() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let config = SignerConfig {
        bump_after: Duration::from_millis(30),
        ..fast_config()
    };
    let (handle, _task) = spawn(&chain, &ledger, config).await.unwrap();

    handle.submit(request("stuck")).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let row = loop {
        let row = handle.status("stuck").await.unwrap().unwrap();
        if row.attempts.len() >= 2 {
            break row;
        }
        assert!(Instant::now() < deadline, "no replacement was broadcast");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let (first, second) = (&row.attempts[0], &row.attempts[1]);
    assert_eq!(row.nonce, 0);
    assert!(second.max_fee_per_gas * 100 >= first.max_fee_per_gas * 110);
    assert!(second.max_priority_fee_per_gas * 100 >= first.max_priority_fee_per_gas * 110);
    assert!(
        !chain
            .outcomes()
            .iter()
            .any(|outcome| matches!(outcome, BroadcastOutcome::Underpriced)),
        "replacements must clear the 10% rule"
    );

    // Whatever attempt is in the pool when the block is built gets mined.
    chain.mine_all();
    chain.advance(2);
    let done = wait_for_state(&handle, "stuck", TxState::Confirmed).await;
    let mined = done.included_tx_hash.unwrap();
    assert!(done.attempts.iter().any(|attempt| attempt.tx_hash == mined));
}

#[tokio::test]
async fn an_older_attempt_being_mined_is_recognised() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let config = SignerConfig {
        bump_after: Duration::from_millis(30),
        ..fast_config()
    };
    let (handle, _task) = spawn(&chain, &ledger, config).await.unwrap();

    let first = handle.submit(request("raced")).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while handle
        .status("raced")
        .await
        .unwrap()
        .unwrap()
        .attempts
        .len()
        < 2
    {
        assert!(Instant::now() < deadline, "no replacement was broadcast");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // A builder had already picked the original transaction.
    chain.with(|state| {
        state.pool.clear();
        state.head += 1;
        let head = state.head;
        state.receipts.insert(first.tx_hash, (head, true));
        state.mined_nonce = 1;
    });
    chain.advance(2);
    let done = wait_for_state(&handle, "raced", TxState::Confirmed).await;
    assert_eq!(done.included_tx_hash, Some(first.tx_hash));
}

#[tokio::test]
async fn reverted_receipts_are_final() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let (handle, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    let ticket = handle.submit(request("revert")).await.unwrap();
    chain.with(|state| {
        state.pool.clear();
        state.head += 1;
        let head = state.head;
        state.receipts.insert(ticket.tx_hash, (head, false));
        state.mined_nonce = 1;
    });
    chain.advance(2);
    wait_for_state(&handle, "revert", TxState::Reverted).await;
}

#[tokio::test]
async fn a_reorged_receipt_returns_the_transaction_to_pending() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let config = SignerConfig {
        confirmations: 5,
        ..fast_config()
    };
    let (handle, _task) = spawn(&chain, &ledger, config).await.unwrap();

    let ticket = handle.submit(request("reorg")).await.unwrap();
    chain.mine_all();
    wait_for_state(&handle, "reorg", TxState::Included).await;

    // The block is reorged out and the transaction is back in the pool.
    chain.with(|state| {
        state.receipts.remove(&ticket.tx_hash);
        state.mined_nonce = 0;
        state.head -= 1;
    });
    wait_for_state(&handle, "reorg", TxState::Pending).await;
    assert_eq!(chain.with(|state| state.pool[&0].hash), ticket.tx_hash);

    chain.mine_all();
    chain.advance(5);
    wait_for_state(&handle, "reorg", TxState::Confirmed).await;
}

#[tokio::test]
async fn a_nonce_consumed_elsewhere_is_marked_displaced() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let (handle, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    handle.submit(request("victim")).await.unwrap();
    // Someone else's transaction with the same nonce was mined.
    chain.with(|state| {
        state.pool.clear();
        state.mined_nonce = 1;
        state.head += 1;
    });
    let row = wait_for_state(&handle, "victim", TxState::Displaced).await;
    assert!(row.last_error.unwrap().contains("outside the ledger"));
}

#[tokio::test]
async fn startup_and_runtime_resynchronise_with_external_nonce_use() {
    let chain = MockChain::new();
    chain.with(|state| state.mined_nonce = 5);
    let ledger = InMemoryNonceLedger::new();
    let (handle, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    let ticket = handle.submit(request("after-external")).await.unwrap();
    assert_eq!(ticket.nonce, 5);

    // An out-of-band transaction consumed nonces 6 and 7 while running.
    chain.with(|state| {
        state.pool.clear();
        state
            .receipts
            .insert(ticket.tx_hash, (state.head + 1, true));
        state.mined_nonce = 8;
    });
    let ticket = handle.submit(request("resynced")).await.unwrap();
    assert_eq!(ticket.nonce, 8);
    assert_eq!(ticket.broadcast, BroadcastStatus::Accepted);
}

#[tokio::test]
async fn only_one_actor_may_own_a_key_and_a_restart_resumes() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let (handle, task) = spawn(&chain, &ledger, fast_config()).await.unwrap();

    assert!(
        spawn(&chain, &ledger, fast_config()).await.is_err(),
        "a second writer for the same key must be refused"
    );

    handle.submit(request("a")).await.unwrap();
    handle.submit(request("b")).await.unwrap();
    task.abort();
    let _ = task.await;
    assert!(!handle.is_running());
    assert!(matches!(
        handle.submit(request("c")).await,
        Err(SubmitError::Unavailable(_))
    ));

    // A restarted actor continues after the ledger's highest nonce and
    // finishes the open transactions it inherited.
    let (restarted, _task) = spawn(&chain, &ledger, fast_config()).await.unwrap();
    let ticket = restarted.submit(request("c")).await.unwrap();
    assert_eq!(ticket.nonce, 2);
    chain.mine_all();
    chain.advance(2);
    for id in ["a", "b", "c"] {
        wait_for_state(&restarted, id, TxState::Confirmed).await;
    }
}

#[tokio::test]
async fn the_signer_refuses_a_chain_id_mismatch() {
    let chain = MockChain::new();
    let ledger = InMemoryNonceLedger::new();
    let result = SignerActor::spawn(
        Zeroizing::new(TEST_KEY.to_string()),
        1,
        Arc::clone(&chain),
        Arc::new(ledger),
        fast_config(),
    )
    .await;
    assert!(result.is_err());
}
