//! Single-writer L1 transaction layer for CREG hot wallets (ADR-019 §5.2,
//! `ECA-28`, `ECA-30`, `ECO-INV-07`).
//!
//! Every hot key is owned by exactly one [`SignerActor`]: a Tokio task with a
//! single bounded `mpsc` queue per `(chain_id, signer_address)`. Callers hold a
//! cloneable [`SignerHandle`] and never sign themselves, so concurrent
//! requests cannot race on the EOA nonce.
//!
//! The actor keeps a durable EOA nonce ledger ([`NonceLedger`], PostgreSQL in
//! production via [`pg::PgNonceLedger`]):
//!
//! 1. allocate the next nonce in memory (the queue serialises allocation);
//! 2. sign the EIP-1559 transaction and write it to the ledger as `signed`
//!    *before* broadcasting (write-ahead), so a crash can always rebroadcast
//!    the exact bytes;
//! 3. broadcast to the primary endpoint only (`common::l1_rpc` policy) and
//!    record `pending`, or `rejected` (nonce released) on a definitive refusal;
//! 4. a reconciler on the same task follows every open transaction until a
//!    terminal state (`confirmed`, `reverted`, `displaced`): it checks receipts
//!    for every broadcast attempt, waits for a confirmation depth, handles
//!    reorgs, rebroadcasts dropped transactions and bumps EIP-1559 fees
//!    (replace-by-fee) on stuck ones. There is no give-up timeout.
//!
//! Across processes the PostgreSQL ledger takes a session advisory lock per
//! signer, so a second relayer or faucet process using the same key refuses
//! to start instead of racing.

pub mod fees;
pub mod ledger;
pub mod pg;
pub mod rpc;
pub mod signer;

pub use fees::{bump_fees, FeePolicy};
pub use ledger::{
    InMemoryNonceLedger, LedgerTx, NonceLedger, SignerKey, TxAttempt, TxState, WriterLease,
};
pub use rpc::{
    classify_rpc_error, AlloyL1Rpc, BroadcastOutcome, FeeEstimate, L1Rpc, ReceiptSummary,
};
pub use signer::{
    BroadcastStatus, SignerActor, SignerConfig, SignerHandle, SubmitError, SubmitRequest, TxTicket,
};
