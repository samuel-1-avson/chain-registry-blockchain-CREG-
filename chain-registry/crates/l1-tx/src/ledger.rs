//! Durable EOA nonce ledger: types, the [`NonceLedger`] trait and an
//! in-memory implementation for tests and single-process development.

use alloy::primitives::{Address, Bytes, B256, U256};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
};

/// One hot wallet on one chain. Exactly one writer may exist per key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SignerKey {
    pub chain_id: u64,
    pub address: Address,
}

impl SignerKey {
    pub fn new(chain_id: u64, address: Address) -> Self {
        Self { chain_id, address }
    }

    /// Lowercase `0x`-prefixed address, the ledger's storage form.
    pub fn address_hex(&self) -> String {
        format!("{:#x}", self.address)
    }

    /// Stable 64-bit identifier for PostgreSQL advisory locks.
    pub fn lock_id(&self) -> i64 {
        let digest = Sha256::digest(
            format!("creg-l1-signer-v1:{}:{}", self.chain_id, self.address_hex()).as_bytes(),
        );
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        i64::from_be_bytes(bytes)
    }
}

/// Lifecycle of one ledger entry. `rejected` is the only state that releases
/// its nonce; every other state keeps the nonce consumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxState {
    /// Signed and written ahead of the broadcast; the outcome is not yet known.
    Signed,
    /// Accepted by the primary endpoint (or already known to it).
    Pending,
    /// A receipt exists but the confirmation depth is not reached yet.
    Included,
    /// Mined successfully at the confirmation depth. Terminal.
    Confirmed,
    /// Mined at the confirmation depth with a failed status. Terminal.
    Reverted,
    /// Definitively refused before any acceptance; the nonce was released. Terminal.
    Rejected,
    /// The nonce was consumed by a transaction this ledger did not sign. Terminal.
    Displaced,
}

impl TxState {
    pub const ALL: [TxState; 7] = [
        Self::Signed,
        Self::Pending,
        Self::Included,
        Self::Confirmed,
        Self::Reverted,
        Self::Rejected,
        Self::Displaced,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Signed => "signed",
            Self::Pending => "pending",
            Self::Included => "included",
            Self::Confirmed => "confirmed",
            Self::Reverted => "reverted",
            Self::Rejected => "rejected",
            Self::Displaced => "displaced",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|state| state.as_str() == value)
            .ok_or_else(|| anyhow!("unknown l1-tx ledger state {value:?}"))
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Confirmed | Self::Reverted | Self::Rejected | Self::Displaced
        )
    }

    pub fn holds_nonce(self) -> bool {
        !matches!(self, Self::Rejected)
    }
}

/// One signed broadcast attempt for a nonce. Replacements append attempts;
/// any of them may be the one that is mined.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxAttempt {
    pub tx_hash: B256,
    #[serde(with = "u128_string")]
    pub max_fee_per_gas: u128,
    #[serde(with = "u128_string")]
    pub max_priority_fee_per_gas: u128,
    /// EIP-2718 encoded signed transaction, kept for idempotent rebroadcast.
    pub raw: Bytes,
}

mod u128_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// One ledger row: a caller request bound to one EOA nonce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerTx {
    pub key: SignerKey,
    /// Caller correlation id, unique per signer (idempotency key).
    pub request_id: String,
    pub nonce: u64,
    pub state: TxState,
    pub to: Address,
    pub value: U256,
    pub input: Bytes,
    pub gas_limit: u64,
    /// Broadcast attempts, oldest first. Never empty once written.
    pub attempts: Vec<TxAttempt>,
    pub included_tx_hash: Option<B256>,
    pub block_number: Option<u64>,
    pub last_error: Option<String>,
    pub broadcast_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl LedgerTx {
    pub fn latest_attempt(&self) -> Option<&TxAttempt> {
        self.attempts.last()
    }

    /// Hash of the attempt that was mined, or of the latest attempt.
    pub fn current_tx_hash(&self) -> Option<B256> {
        self.included_tx_hash
            .or_else(|| self.latest_attempt().map(|attempt| attempt.tx_hash))
    }
}

/// Proof that this process is the only writer for a signer key. Dropping it
/// releases the lease.
#[async_trait]
pub trait WriterLease: Send + Sync {
    /// Fails when the lease can no longer be guaranteed (for example the
    /// database session holding the lock was lost). The actor then stops
    /// signing.
    async fn check(&mut self) -> Result<()>;
}

/// Durable storage for the EOA nonce ledger.
#[async_trait]
pub trait NonceLedger: Send + Sync + 'static {
    /// Take the exclusive writer lease for `key`, failing if another holder
    /// (in this or another process) has it.
    async fn acquire_writer(&self, key: SignerKey) -> Result<Box<dyn WriterLease>>;

    /// Highest nonce held by a non-rejected row, if any.
    async fn highest_nonce(&self, key: SignerKey) -> Result<Option<u64>>;

    /// Insert a new row. Fails on a duplicate `request_id` or a live nonce.
    async fn insert(&self, tx: &LedgerTx) -> Result<()>;

    /// Replace the row with the same `(key, request_id)`.
    async fn update(&self, tx: &LedgerTx) -> Result<()>;

    async fn get(&self, key: SignerKey, request_id: &str) -> Result<Option<LedgerTx>>;

    /// Non-terminal rows for `key`, ordered by nonce.
    async fn open(&self, key: SignerKey) -> Result<Vec<LedgerTx>>;
}

/// Process-local ledger for tests and development. Not durable.
#[derive(Clone, Default)]
pub struct InMemoryNonceLedger {
    inner: Arc<Mutex<InMemoryState>>,
}

#[derive(Default)]
struct InMemoryState {
    rows: BTreeMap<(SignerKey, String), LedgerTx>,
    writers: HashSet<SignerKey>,
}

impl InMemoryNonceLedger {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, InMemoryState>> {
        self.inner
            .lock()
            .map_err(|_| anyhow!("in-memory nonce ledger lock poisoned"))
    }

    /// Every row for `key` (any state), ordered by nonce. Test helper.
    pub fn all(&self, key: SignerKey) -> Result<Vec<LedgerTx>> {
        let state = self.lock()?;
        let mut rows: Vec<LedgerTx> = state
            .rows
            .values()
            .filter(|row| row.key == key)
            .cloned()
            .collect();
        rows.sort_by_key(|row| (row.nonce, row.created_at));
        Ok(rows)
    }
}

struct InMemoryLease {
    ledger: InMemoryNonceLedger,
    key: SignerKey,
}

#[async_trait]
impl WriterLease for InMemoryLease {
    async fn check(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Drop for InMemoryLease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.ledger.inner.lock() {
            state.writers.remove(&self.key);
        }
    }
}

#[async_trait]
impl NonceLedger for InMemoryNonceLedger {
    async fn acquire_writer(&self, key: SignerKey) -> Result<Box<dyn WriterLease>> {
        let mut state = self.lock()?;
        if !state.writers.insert(key) {
            bail!(
                "another signer already owns {} on chain {}",
                key.address_hex(),
                key.chain_id
            );
        }
        Ok(Box::new(InMemoryLease {
            ledger: self.clone(),
            key,
        }))
    }

    async fn highest_nonce(&self, key: SignerKey) -> Result<Option<u64>> {
        let state = self.lock()?;
        Ok(state
            .rows
            .values()
            .filter(|row| row.key == key && row.state.holds_nonce())
            .map(|row| row.nonce)
            .max())
    }

    async fn insert(&self, tx: &LedgerTx) -> Result<()> {
        let mut state = self.lock()?;
        let id = (tx.key, tx.request_id.clone());
        if state.rows.contains_key(&id) {
            bail!("duplicate l1-tx request id {}", tx.request_id);
        }
        if tx.state.holds_nonce()
            && state
                .rows
                .values()
                .any(|row| row.key == tx.key && row.nonce == tx.nonce && row.state.holds_nonce())
        {
            bail!("nonce {} is already held by another ledger row", tx.nonce);
        }
        state.rows.insert(id, tx.clone());
        Ok(())
    }

    async fn update(&self, tx: &LedgerTx) -> Result<()> {
        let mut state = self.lock()?;
        if tx.state.holds_nonce()
            && state.rows.values().any(|row| {
                row.key == tx.key
                    && row.request_id != tx.request_id
                    && row.nonce == tx.nonce
                    && row.state.holds_nonce()
            })
        {
            bail!("nonce {} is already held by another ledger row", tx.nonce);
        }
        let row = state
            .rows
            .get_mut(&(tx.key, tx.request_id.clone()))
            .ok_or_else(|| anyhow!("unknown l1-tx request id {}", tx.request_id))?;
        *row = tx.clone();
        Ok(())
    }

    async fn get(&self, key: SignerKey, request_id: &str) -> Result<Option<LedgerTx>> {
        let state = self.lock()?;
        Ok(state.rows.get(&(key, request_id.to_string())).cloned())
    }

    async fn open(&self, key: SignerKey) -> Result<Vec<LedgerTx>> {
        let state = self.lock()?;
        let mut rows: Vec<LedgerTx> = state
            .rows
            .values()
            .filter(|row| row.key == key && !row.state.is_terminal())
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.nonce);
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: SignerKey, request_id: &str, nonce: u64, state: TxState) -> LedgerTx {
        let now = Utc::now();
        LedgerTx {
            key,
            request_id: request_id.to_string(),
            nonce,
            state,
            to: Address::ZERO,
            value: U256::ZERO,
            input: Bytes::new(),
            gas_limit: 21_000,
            attempts: vec![TxAttempt {
                tx_hash: B256::repeat_byte(nonce as u8),
                max_fee_per_gas: 2,
                max_priority_fee_per_gas: 1,
                raw: Bytes::from_static(&[0x02]),
            }],
            included_tx_hash: None,
            block_number: None,
            last_error: None,
            broadcast_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn state_names_round_trip() {
        for state in TxState::ALL {
            assert_eq!(TxState::parse(state.as_str()).unwrap(), state);
        }
        assert!(TxState::parse("mined").is_err());
        assert!(!TxState::Rejected.holds_nonce());
        assert!(TxState::Displaced.is_terminal());
        assert!(!TxState::Included.is_terminal());
    }

    #[test]
    fn lock_ids_differ_per_chain_and_address() {
        let a = SignerKey::new(1, Address::repeat_byte(1));
        let b = SignerKey::new(2, Address::repeat_byte(1));
        let c = SignerKey::new(1, Address::repeat_byte(2));
        assert_ne!(a.lock_id(), b.lock_id());
        assert_ne!(a.lock_id(), c.lock_id());
        assert_eq!(
            a.lock_id(),
            SignerKey::new(1, Address::repeat_byte(1)).lock_id()
        );
        assert_eq!(a.address_hex(), format!("0x{}", "01".repeat(20)));
    }

    #[test]
    fn attempts_serialize_fees_as_strings() {
        let attempt = TxAttempt {
            tx_hash: B256::ZERO,
            max_fee_per_gas: u128::MAX,
            max_priority_fee_per_gas: 7,
            raw: Bytes::from_static(&[0x02, 0xff]),
        };
        let json = serde_json::to_value(&attempt).unwrap();
        assert_eq!(json["max_fee_per_gas"], u128::MAX.to_string());
        let back: TxAttempt = serde_json::from_value(json).unwrap();
        assert_eq!(back, attempt);
    }

    #[tokio::test]
    async fn in_memory_ledger_enforces_single_writer_and_live_nonces() {
        let ledger = InMemoryNonceLedger::new();
        let key = SignerKey::new(11_155_111, Address::repeat_byte(9));

        let lease = ledger.acquire_writer(key).await.unwrap();
        assert!(ledger.acquire_writer(key).await.is_err());
        drop(lease);
        let _lease = ledger.acquire_writer(key).await.unwrap();

        ledger
            .insert(&row(key, "a", 0, TxState::Pending))
            .await
            .unwrap();
        assert!(ledger
            .insert(&row(key, "a", 1, TxState::Pending))
            .await
            .is_err());
        assert!(ledger
            .insert(&row(key, "b", 0, TxState::Signed))
            .await
            .is_err());

        // A rejected row releases its nonce.
        let mut rejected = row(key, "c", 1, TxState::Signed);
        ledger.insert(&rejected).await.unwrap();
        rejected.state = TxState::Rejected;
        ledger.update(&rejected).await.unwrap();
        ledger
            .insert(&row(key, "d", 1, TxState::Signed))
            .await
            .unwrap();

        assert_eq!(ledger.highest_nonce(key).await.unwrap(), Some(1));
        let open: Vec<u64> = ledger
            .open(key)
            .await
            .unwrap()
            .iter()
            .map(|r| r.nonce)
            .collect();
        assert_eq!(open, vec![0, 1]);
    }
}
