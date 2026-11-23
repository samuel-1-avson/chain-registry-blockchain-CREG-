//! The single-writer signer actor.

use crate::{
    fees::{bump_fees, FeePolicy},
    ledger::{LedgerTx, NonceLedger, SignerKey, TxAttempt, TxState, WriterLease},
    rpc::{BroadcastOutcome, FeeEstimate, L1Rpc},
};
use alloy::{
    eips::eip2718::Encodable2718,
    network::{EthereumWallet, TransactionBuilder},
    primitives::{Address, Bytes, B256, U256},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{error, info, warn};
use zeroize::Zeroizing;

/// Maximum length of a caller request id.
pub const MAX_REQUEST_ID_LEN: usize = 128;

/// Tuning for one signer actor.
#[derive(Clone, Debug)]
pub struct SignerConfig {
    /// Blocks on top of the inclusion block before a receipt is final.
    pub confirmations: u64,
    /// How often the reconciler runs.
    pub reconcile_interval: Duration,
    /// A pending transaction without a receipt for this long gets a fee bump.
    pub bump_after: Duration,
    pub fees: FeePolicy,
    /// Bounded command queue (`ECO-INV-08`).
    pub queue_capacity: usize,
    /// New submissions are refused while this many transactions are open.
    pub max_open_transactions: usize,
    /// Consecutive reconciler passes that must see the nonce consumed with no
    /// receipt for any attempt before a row is marked `displaced`.
    pub displaced_after_passes: u32,
}

impl Default for SignerConfig {
    fn default() -> Self {
        Self {
            confirmations: 3,
            reconcile_interval: Duration::from_secs(5),
            bump_after: Duration::from_secs(45),
            fees: FeePolicy::default(),
            queue_capacity: 256,
            max_open_transactions: 64,
            displaced_after_passes: 3,
        }
    }
}

/// A transaction to sign and send. `gas_limit` is mandatory: callers apply
/// their own estimate and policy ceiling before submitting.
#[derive(Clone, Debug)]
pub struct SubmitRequest {
    /// Idempotency key, unique per signer. Resubmitting the same id returns
    /// the existing ticket instead of signing again.
    pub request_id: String,
    pub to: Address,
    pub value: U256,
    pub input: Bytes,
    pub gas_limit: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BroadcastStatus {
    /// The primary endpoint accepted (or already had) the transaction.
    Accepted,
    /// The broadcast failed in transit; the reconciler will rebroadcast the
    /// same signed bytes.
    Uncertain,
}

/// Returned once a transaction is signed, durably recorded and broadcast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxTicket {
    pub request_id: String,
    pub nonce: u64,
    pub tx_hash: B256,
    pub broadcast: BroadcastStatus,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SubmitError {
    /// Refused before anything was accepted by the network; nothing is pending.
    #[error("transaction rejected: {0}")]
    Rejected(String),
    /// The network fee estimate is above the operator's ceiling.
    #[error("network fee {estimate_wei} wei/gas exceeds the configured cap {cap_wei} wei/gas")]
    FeeAboveCap { estimate_wei: u128, cap_wei: u128 },
    /// Too many transactions are still open; retry later.
    #[error("signer is busy: {0} transactions are still open")]
    Busy(usize),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// The ledger, the RPC or the actor itself is unavailable.
    #[error("signer unavailable: {0}")]
    Unavailable(String),
}

enum SignerCommand {
    Submit {
        request: SubmitRequest,
        reply: oneshot::Sender<Result<TxTicket, SubmitError>>,
    },
}

/// Cloneable handle to a running [`SignerActor`].
#[derive(Clone)]
pub struct SignerHandle {
    key: SignerKey,
    commands: mpsc::Sender<SignerCommand>,
    ledger: Arc<dyn NonceLedger>,
}

impl SignerHandle {
    pub fn key(&self) -> SignerKey {
        self.key
    }

    pub fn address(&self) -> Address {
        self.key.address
    }

    pub fn chain_id(&self) -> u64 {
        self.key.chain_id
    }

    /// Whether the actor task is still running.
    pub fn is_running(&self) -> bool {
        !self.commands.is_closed()
    }

    /// Queue a transaction. Requests are processed strictly one at a time.
    pub async fn submit(&self, request: SubmitRequest) -> Result<TxTicket, SubmitError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(SignerCommand::Submit { request, reply })
            .await
            .map_err(|_| SubmitError::Unavailable("signer actor has stopped".to_string()))?;
        response
            .await
            .map_err(|_| SubmitError::Unavailable("signer actor dropped the request".to_string()))?
    }

    /// Current ledger row for a request id.
    pub async fn status(&self, request_id: &str) -> Result<Option<LedgerTx>> {
        self.ledger.get(self.key, request_id).await
    }
}

/// Owns one hot key. Construct with [`SignerActor::spawn`].
pub struct SignerActor<R: L1Rpc> {
    key: SignerKey,
    wallet: EthereumWallet,
    rpc: Arc<R>,
    ledger: Arc<dyn NonceLedger>,
    lease: Box<dyn WriterLease>,
    config: SignerConfig,
    next_nonce: u64,
    displaced_strikes: HashMap<String, u32>,
}

impl<R: L1Rpc> SignerActor<R> {
    /// Load the key, take the writer lease, synchronise the nonce and start
    /// the actor task. `expected_chain_id` must match the quorum-read chain id.
    pub async fn spawn(
        private_key_hex: Zeroizing<String>,
        expected_chain_id: u64,
        rpc: Arc<R>,
        ledger: Arc<dyn NonceLedger>,
        config: SignerConfig,
    ) -> Result<(SignerHandle, JoinHandle<()>)> {
        let signer: PrivateKeySigner = private_key_hex
            .trim()
            .parse()
            .map_err(|_| anyhow!("hot wallet key is not a valid secp256k1 private key"))?;
        drop(private_key_hex);

        let chain_id = rpc.chain_id().await?;
        if chain_id != expected_chain_id {
            bail!("L1 RPC reports chain {chain_id}, expected {expected_chain_id}");
        }
        let key = SignerKey::new(chain_id, signer.address());
        let lease = ledger.acquire_writer(key).await.with_context(|| {
            format!("could not take the writer lease for {}", key.address_hex())
        })?;

        let ledger_next = ledger
            .highest_nonce(key)
            .await?
            .map(|nonce| nonce.saturating_add(1))
            .unwrap_or(0);
        let chain_pending = rpc.transaction_count(key.address, true).await?;
        if chain_pending > ledger_next {
            warn!(
                signer = %key.address_hex(),
                ledger_next,
                chain_pending,
                "signer account nonce advanced outside the l1-tx ledger; continuing from the chain's pending nonce"
            );
        }
        let next_nonce = ledger_next.max(chain_pending);
        info!(
            signer = %key.address_hex(),
            chain_id,
            next_nonce,
            "l1-tx signer actor started"
        );

        let (commands, receiver) = mpsc::channel(config.queue_capacity.max(1));
        let handle = SignerHandle {
            key,
            commands,
            ledger: Arc::clone(&ledger),
        };
        let actor = Self {
            key,
            wallet: EthereumWallet::from(signer),
            rpc,
            ledger,
            lease,
            config,
            next_nonce,
            displaced_strikes: HashMap::new(),
        };
        let task = tokio::spawn(actor.run(receiver));
        Ok((handle, task))
    }

    async fn run(mut self, mut receiver: mpsc::Receiver<SignerCommand>) {
        let mut ticker = tokio::time::interval(self.config.reconcile_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                command = receiver.recv() => match command {
                    Some(SignerCommand::Submit { request, reply }) => {
                        let result = self.handle_submit(request).await;
                        let _ = reply.send(result);
                    }
                    None => break,
                },
                _ = ticker.tick() => {
                    if let Err(error) = self.lease.check().await {
                        error!(
                            signer = %self.key.address_hex(),
                            "l1-tx writer lease lost ({error:#}); the signer stops to stay single-writer"
                        );
                        break;
                    }
                    if let Err(error) = self.reconcile().await {
                        warn!(signer = %self.key.address_hex(), "l1-tx reconcile pass failed: {error:#}");
                    }
                }
            }
        }
        receiver.close();
        while let Some(SignerCommand::Submit { reply, .. }) = receiver.recv().await {
            let _ = reply.send(Err(SubmitError::Unavailable(
                "signer actor stopped".to_string(),
            )));
        }
    }

    async fn handle_submit(&mut self, request: SubmitRequest) -> Result<TxTicket, SubmitError> {
        validate_request(&request)?;
        let unavailable = |error: anyhow::Error| SubmitError::Unavailable(format!("{error:#}"));

        if let Some(existing) = self
            .ledger
            .get(self.key, &request.request_id)
            .await
            .map_err(unavailable)?
        {
            return ticket_for_existing(&existing);
        }

        let open = self.ledger.open(self.key).await.map_err(unavailable)?.len();
        if open >= self.config.max_open_transactions {
            return Err(SubmitError::Busy(open));
        }

        let estimate = self.rpc.fee_estimate().await.map_err(unavailable)?;
        let fees = self
            .config
            .fees
            .initial(estimate)
            .ok_or(SubmitError::FeeAboveCap {
                estimate_wei: estimate.max_fee_per_gas,
                cap_wei: self.config.fees.max_fee_per_gas_cap,
            })?;

        let now = Utc::now();
        let mut row = LedgerTx {
            key: self.key,
            request_id: request.request_id.clone(),
            nonce: self.next_nonce,
            state: TxState::Signed,
            to: request.to,
            value: request.value,
            input: request.input.clone(),
            gas_limit: request.gas_limit,
            attempts: Vec::new(),
            included_tx_hash: None,
            block_number: None,
            last_error: None,
            broadcast_at: None,
            created_at: now,
            updated_at: now,
        };
        let mut inserted = false;

        // At most one resynchronisation: a nonce consumed outside the actor
        // (or an external transaction occupying it in the pool) moves the
        // request to the chain's pending nonce once.
        for round in 0..2 {
            let attempt = self.sign(&row, fees).await.map_err(unavailable)?;
            row.attempts = vec![attempt.clone()];
            row.state = TxState::Signed;
            row.updated_at = Utc::now();
            // Write-ahead: the signed bytes are durable before the broadcast.
            if inserted {
                self.ledger.update(&row).await.map_err(unavailable)?;
            } else {
                self.ledger.insert(&row).await.map_err(unavailable)?;
                inserted = true;
            }

            match self.rpc.send_raw_transaction(&attempt.raw).await {
                BroadcastOutcome::Accepted(_) | BroadcastOutcome::AlreadyKnown => {
                    row.state = TxState::Pending;
                    row.broadcast_at = Some(Utc::now());
                    row.updated_at = Utc::now();
                    self.next_nonce = row.nonce.saturating_add(1);
                    if let Err(error) = self.ledger.update(&row).await {
                        // The row stays `signed`; the reconciler will
                        // rebroadcast the same bytes and repair the state.
                        warn!(request_id = %row.request_id, "could not record the broadcast: {error:#}");
                    }
                    return Ok(ticket(&row, attempt.tx_hash, BroadcastStatus::Accepted));
                }
                BroadcastOutcome::Uncertain(detail) => {
                    // Keep the nonce consumed; the reconciler rebroadcasts.
                    self.next_nonce = row.nonce.saturating_add(1);
                    row.last_error = Some(detail);
                    row.updated_at = Utc::now();
                    if let Err(error) = self.ledger.update(&row).await {
                        warn!(request_id = %row.request_id, "could not record the uncertain broadcast: {error:#}");
                    }
                    return Ok(ticket(&row, attempt.tx_hash, BroadcastStatus::Uncertain));
                }
                BroadcastOutcome::NonceTooLow | BroadcastOutcome::Underpriced if round == 0 => {
                    let chain_pending = self
                        .rpc
                        .transaction_count(self.key.address, true)
                        .await
                        .map_err(unavailable)?;
                    warn!(
                        signer = %self.key.address_hex(),
                        nonce = row.nonce,
                        chain_pending,
                        "nonce in use outside the l1-tx ledger; resynchronising"
                    );
                    // Release the nonce before moving the row. The new nonce
                    // excludes this row, so a base-fee "underpriced" error
                    // retries the same nonce instead of leaving a gap.
                    row.state = TxState::Rejected;
                    row.last_error = Some("nonce in use outside the ledger".to_string());
                    self.ledger.update(&row).await.map_err(unavailable)?;
                    let highest = self
                        .ledger
                        .highest_nonce(self.key)
                        .await
                        .map_err(unavailable)?;
                    let ledger_next = highest.map(|n| n.saturating_add(1)).unwrap_or(0);
                    self.next_nonce = ledger_next.max(chain_pending);
                    row.nonce = self.next_nonce;
                    row.last_error = None;
                }
                refused => {
                    let message = match refused {
                        BroadcastOutcome::Rejected(detail) => detail,
                        BroadcastOutcome::NonceTooLow => {
                            "the signer nonce is already used on chain".to_string()
                        }
                        _ => {
                            "fee below the network minimum, or another transaction holds the nonce"
                                .to_string()
                        }
                    };
                    row.state = TxState::Rejected;
                    row.last_error = Some(message.clone());
                    row.updated_at = Utc::now();
                    self.ledger.update(&row).await.map_err(unavailable)?;
                    // Nothing was accepted, so the nonce stays free for the
                    // next request (`next_nonce` was not advanced).
                    return Err(SubmitError::Rejected(message));
                }
            }
        }
        Err(SubmitError::Unavailable(
            "could not obtain a free nonce after resynchronising".to_string(),
        ))
    }

    /// Sign `row` (its nonce, call and gas limit) with `fees`.
    async fn sign(&self, row: &LedgerTx, fees: FeeEstimate) -> Result<TxAttempt> {
        let request = TransactionRequest::default()
            .with_from(self.key.address)
            .with_chain_id(self.key.chain_id)
            .with_nonce(row.nonce)
            .with_to(row.to)
            .with_value(row.value)
            .with_input(row.input.clone())
            .with_gas_limit(row.gas_limit)
            .with_max_fee_per_gas(fees.max_fee_per_gas)
            .with_max_priority_fee_per_gas(fees.max_priority_fee_per_gas);
        let envelope = request
            .build(&self.wallet)
            .await
            .map_err(|error| anyhow!("failed to sign the L1 transaction: {error}"))?;
        let raw = Bytes::from(envelope.encoded_2718());
        Ok(TxAttempt {
            tx_hash: *envelope.tx_hash(),
            max_fee_per_gas: fees.max_fee_per_gas,
            max_priority_fee_per_gas: fees.max_priority_fee_per_gas,
            raw,
        })
    }

    /// One pass over every open row.
    async fn reconcile(&mut self) -> Result<()> {
        let open = self.ledger.open(self.key).await?;
        if open.is_empty() {
            self.displaced_strikes.clear();
            return Ok(());
        }
        let head = self.rpc.block_number().await?;
        let mined_nonce = self.rpc.transaction_count(self.key.address, false).await?;
        let mut fee_estimate = None;

        for mut row in open {
            if let Err(error) = self
                .reconcile_row(&mut row, head, mined_nonce, &mut fee_estimate)
                .await
            {
                warn!(request_id = %row.request_id, nonce = row.nonce, "reconcile failed: {error:#}");
            }
        }
        Ok(())
    }

    async fn reconcile_row(
        &mut self,
        row: &mut LedgerTx,
        head: u64,
        mined_nonce: u64,
        fee_estimate: &mut Option<FeeEstimate>,
    ) -> Result<()> {
        // 1. Is any attempt mined?
        let mut mined = None;
        for attempt in row.attempts.iter().rev() {
            if let Some(receipt) = self.rpc.receipt(attempt.tx_hash).await? {
                mined = Some(receipt);
                break;
            }
        }

        if let Some(receipt) = mined {
            self.displaced_strikes.remove(&row.request_id);
            let depth = head.saturating_add(1).saturating_sub(receipt.block_number);
            let state = if depth >= self.config.confirmations {
                if receipt.success {
                    TxState::Confirmed
                } else {
                    TxState::Reverted
                }
            } else {
                TxState::Included
            };
            if row.state != state
                || row.included_tx_hash != Some(receipt.tx_hash)
                || row.block_number != Some(receipt.block_number)
            {
                row.state = state;
                row.included_tx_hash = Some(receipt.tx_hash);
                row.block_number = Some(receipt.block_number);
                row.updated_at = Utc::now();
                self.ledger.update(row).await?;
                if state.is_terminal() {
                    info!(
                        request_id = %row.request_id,
                        nonce = row.nonce,
                        tx_hash = %receipt.tx_hash,
                        block = receipt.block_number,
                        state = state.as_str(),
                        "l1-tx transaction final"
                    );
                }
            }
            return Ok(());
        }

        // 2. No receipt. An `included` row lost its receipt: reorged out.
        if row.state == TxState::Included {
            warn!(request_id = %row.request_id, nonce = row.nonce, "receipt disappeared (reorg); tracking again");
            row.state = TxState::Pending;
            row.included_tx_hash = None;
            row.block_number = None;
            row.broadcast_at = None;
            row.updated_at = Utc::now();
            self.ledger.update(row).await?;
        }

        // 3. Nonce consumed without any of our receipts: displaced after a
        // grace period (receipts can lag across failover endpoints).
        if mined_nonce > row.nonce {
            let strikes = self
                .displaced_strikes
                .entry(row.request_id.clone())
                .or_insert(0);
            *strikes += 1;
            if *strikes >= self.config.displaced_after_passes {
                self.displaced_strikes.remove(&row.request_id);
                error!(
                    request_id = %row.request_id,
                    nonce = row.nonce,
                    "nonce consumed by a transaction this signer did not record; marking displaced"
                );
                row.state = TxState::Displaced;
                row.last_error =
                    Some("nonce consumed by a transaction outside the ledger".to_string());
                row.updated_at = Utc::now();
                self.ledger.update(row).await?;
            }
            return Ok(());
        }
        self.displaced_strikes.remove(&row.request_id);

        // 4. Still unmined. A row whose broadcast was never confirmed (or
        // that was reorged out) is rebroadcast as the same bytes; a row that
        // has sat in the pool for `bump_after` is replaced with higher fees.
        let bump_due = match row.broadcast_at {
            None => false,
            Some(at) => Utc::now()
                .signed_duration_since(at)
                .to_std()
                .map(|elapsed| elapsed >= self.config.bump_after)
                .unwrap_or(false),
        };
        if row.broadcast_at.is_some() && !bump_due {
            return Ok(());
        }

        let latest = row
            .latest_attempt()
            .cloned()
            .ok_or_else(|| anyhow!("ledger row {} has no signed attempt", row.request_id))?;
        let mut attempt = latest.clone();
        if bump_due && row.state == TxState::Pending {
            let current = match fee_estimate {
                Some(current) => *current,
                None => {
                    let current = self.rpc.fee_estimate().await?;
                    *fee_estimate = Some(current);
                    current
                }
            };
            let previous = FeeEstimate {
                max_fee_per_gas: latest.max_fee_per_gas,
                max_priority_fee_per_gas: latest.max_priority_fee_per_gas,
            };
            match bump_fees(previous, current, &self.config.fees) {
                Some(bumped) => {
                    attempt = self.sign(row, bumped).await?;
                    row.attempts.push(attempt.clone());
                    row.updated_at = Utc::now();
                    // Write-ahead before broadcasting the replacement.
                    self.ledger.update(row).await?;
                    info!(
                        request_id = %row.request_id,
                        nonce = row.nonce,
                        max_fee_per_gas = bumped.max_fee_per_gas,
                        "replacing a stuck transaction with higher fees"
                    );
                }
                None => warn!(
                    request_id = %row.request_id,
                    nonce = row.nonce,
                    "fee cap reached; rebroadcasting the last attempt"
                ),
            }
        }

        match self.rpc.send_raw_transaction(&attempt.raw).await {
            BroadcastOutcome::Accepted(_) | BroadcastOutcome::AlreadyKnown => {
                row.state = TxState::Pending;
                row.last_error = None;
            }
            BroadcastOutcome::Underpriced => {
                row.last_error = Some("replacement underpriced; will bump again".to_string());
            }
            BroadcastOutcome::NonceTooLow => {
                // Mined meanwhile (receipt not visible yet) or displaced; the
                // next pass decides.
            }
            BroadcastOutcome::Rejected(detail) | BroadcastOutcome::Uncertain(detail) => {
                // Never release a nonce that may already be in a pool; keep
                // retrying (for example after the wallet is funded).
                row.last_error = Some(detail);
            }
        }
        row.broadcast_at = Some(Utc::now());
        row.updated_at = Utc::now();
        self.ledger.update(row).await?;
        Ok(())
    }
}

fn validate_request(request: &SubmitRequest) -> Result<(), SubmitError> {
    let id = &request.request_id;
    if id.is_empty() || id.len() > MAX_REQUEST_ID_LEN {
        return Err(SubmitError::InvalidRequest(format!(
            "request id must be 1..={MAX_REQUEST_ID_LEN} bytes"
        )));
    }
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
    {
        return Err(SubmitError::InvalidRequest(
            "request id may contain only ASCII letters, digits, '-', '_', ':' and '.'".to_string(),
        ));
    }
    if request.gas_limit == 0 {
        return Err(SubmitError::InvalidRequest(
            "gas limit must be positive".to_string(),
        ));
    }
    Ok(())
}

fn ticket(row: &LedgerTx, tx_hash: B256, broadcast: BroadcastStatus) -> TxTicket {
    TxTicket {
        request_id: row.request_id.clone(),
        nonce: row.nonce,
        tx_hash,
        broadcast,
    }
}

fn ticket_for_existing(row: &LedgerTx) -> Result<TxTicket, SubmitError> {
    if row.state == TxState::Rejected {
        return Err(SubmitError::Rejected(
            row.last_error
                .clone()
                .unwrap_or_else(|| "previously rejected".to_string()),
        ));
    }
    let tx_hash = row
        .current_tx_hash()
        .ok_or_else(|| SubmitError::Unavailable("ledger row has no attempt".to_string()))?;
    let broadcast = if row.state == TxState::Signed {
        BroadcastStatus::Uncertain
    } else {
        BroadcastStatus::Accepted
    };
    Ok(ticket(row, tx_hash, broadcast))
}

#[cfg(test)]
mod tests;
