//! Durable relayer state (ADR-019 decision #5, `ECA-33`): PostgreSQL in
//! production, an in-memory store for tests.
//!
//! Tables:
//! - `relayer_checkpoints`: the next expected sponsor (relayer) nonce per
//!   `(chain_id, owner)`. It only moves forward.
//! - `relayer_batches`: daily sponsorship batches, one counter per quota key
//!   (`wallet:<owner>:<action>` or `ip:<chain>:<bucket>`) and UTC day.
//! - `relayer_outbox`: one row per accepted sponsor request and the
//!   transaction sent for it. The L1 nonce, signing and receipt tracking live
//!   in the l1-tx ledger (`l1_tx_ledger`), keyed by `signer_request_id`.
//! - `relayer_meta`: schema version and one-time migration markers.
//!
//! A reservation (owner nonce check and advance, both quota increments and
//! the outbox insert) is one SERIALIZABLE transaction, retried on
//! serialization failures.

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::BTreeMap;
#[cfg(test)]
use std::{collections::HashMap, sync::Mutex};

const SCHEMA_LOCK_ID: i64 = 0x5265_6c61_795f_7363; // "Relay_sc"
pub const SCHEMA_VERSION: i32 = 1;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS relayer_meta (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS relayer_checkpoints (
    chain_id   BIGINT        NOT NULL CHECK (chain_id > 0),
    owner      TEXT          NOT NULL CHECK (owner ~ '^0x[0-9a-f]{40}$'),
    next_nonce NUMERIC(20,0) NOT NULL CHECK (next_nonce >= 0),
    updated_at TIMESTAMPTZ   NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, owner)
);

CREATE TABLE IF NOT EXISTS relayer_batches (
    quota_key  TEXT   NOT NULL CHECK (length(quota_key) BETWEEN 1 AND 200),
    day        DATE   NOT NULL,
    count      BIGINT NOT NULL CHECK (count >= 0),
    PRIMARY KEY (quota_key, day)
);

CREATE TABLE IF NOT EXISTS relayer_outbox (
    request_id        TEXT        PRIMARY KEY,
    status            TEXT        NOT NULL,
    action            TEXT        NOT NULL,
    owner             TEXT        NOT NULL,
    chain_id          BIGINT      NOT NULL,
    signer_request_id TEXT,
    tx_hash           TEXT,
    block_number      TEXT,
    message           TEXT        NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS relayer_outbox_open
    ON relayer_outbox (status) WHERE status IN ('submitting', 'submitted', 'timed_out');
"#;

/// Outcome statuses a request can still move out of.
pub const OPEN_STATUSES: [&str; 3] = ["submitting", "submitted", "timed_out"];

/// Public status record (`GET /v1/relayer/status/:id`). The JSON shape is
/// unchanged from the sled-based releases.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestStatusRecord {
    pub request_id: String,
    pub status: String,
    pub action: String,
    pub owner: String,
    pub chain_id: u64,
    pub tx_hash: Option<String>,
    pub block_number: Option<String>,
    pub message: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Outbox row: the public record plus the l1-tx correlation id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxRow {
    pub record: RequestStatusRecord,
    pub signer_request_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Everything one sponsor request reserves atomically.
#[derive(Clone, Debug)]
pub struct Reservation {
    pub chain_id: u64,
    pub owner: String,
    pub expected_owner_nonce: u64,
    pub wallet_quota_key: String,
    pub wallet_quota_limit: u64,
    pub ip_quota_key: String,
    pub ip_quota_limit: u64,
    pub day: NaiveDate,
    pub record: RequestStatusRecord,
    pub signer_request_id: String,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReserveError {
    #[error("relayer nonce advanced to {0}")]
    NonceConflict(u64),
    #[error("daily sponsorship quota reached")]
    QuotaReached,
    #[error("relayer nonce is exhausted")]
    NonceExhausted,
    #[error("durable relayer state is unavailable: {0}")]
    Storage(String),
}

/// Export of a sled-era journal (`creg-relayer-sled-export`).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LegacyJournalExport {
    pub format: String,
    /// `"<chain_id>:<owner>" -> next nonce`.
    #[serde(default)]
    pub nonces: BTreeMap<String, u64>,
    /// Legacy quota keys (`"<owner>:<action>"` or `"<chain_id>:<ip>"`).
    #[serde(default)]
    pub quotas: BTreeMap<String, LegacyDailyCounter>,
    #[serde(default)]
    pub requests: Vec<RequestStatusRecord>,
}

pub const LEGACY_EXPORT_FORMAT: &str = "creg-relayer-journal-export-v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LegacyDailyCounter {
    pub day: String,
    pub count: u64,
}

/// Durable relayer state.
#[async_trait]
pub trait RelayerStore: Send + Sync + 'static {
    /// Next expected sponsor nonce for an owner (0 if never used).
    async fn owner_nonce(&self, chain_id: u64, owner: &str) -> Result<u64>;
    /// Current count for a quota key on `day`.
    async fn quota_count(&self, quota_key: &str, day: NaiveDate) -> Result<u64>;
    /// Atomically check and advance the owner nonce, increment both quotas
    /// and insert the outbox row.
    async fn reserve(&self, reservation: &Reservation) -> Result<(), ReserveError>;
    /// Update the outcome fields of an outbox row.
    async fn update_request(&self, record: &RequestStatusRecord) -> Result<()>;
    async fn get_request(&self, request_id: &str) -> Result<Option<OutboxRow>>;
    /// Outbox rows whose outcome is not final yet.
    async fn open_requests(&self) -> Result<Vec<OutboxRow>>;
    async fn meta_get(&self, key: &str) -> Result<Option<String>>;
    async fn meta_set(&self, key: &str, value: &str) -> Result<()>;
    /// One-shot import of a legacy journal. Conflicting nonces abort it.
    async fn import_legacy(&self, export: &LegacyJournalExport) -> Result<LegacyImportSummary>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LegacyImportSummary {
    pub nonces: usize,
    pub quotas: usize,
    pub requests: usize,
}

/// Parse a legacy `"<chain_id>:<owner>"` nonce key.
pub fn parse_legacy_nonce_key(key: &str) -> Result<(u64, String)> {
    let (chain, owner) = key
        .split_once(':')
        .ok_or_else(|| anyhow!("legacy nonce key {key:?} is not <chain>:<owner>"))?;
    let chain_id: u64 = chain
        .parse()
        .map_err(|_| anyhow!("legacy nonce key {key:?} has an invalid chain id"))?;
    let owner = normalize_owner(owner)?;
    Ok((chain_id, owner))
}

/// Lowercase `0x` + 40 hex digits.
pub fn normalize_owner(owner: &str) -> Result<String> {
    let lower = owner.trim().to_ascii_lowercase();
    let valid = lower.len() == 42
        && lower.starts_with("0x")
        && lower[2..].bytes().all(|byte| byte.is_ascii_hexdigit());
    if !valid {
        bail!("{owner:?} is not an Ethereum address");
    }
    Ok(lower)
}

/// Map a legacy quota key to the namespaced form used by `relayer_batches`.
pub fn legacy_quota_key(key: &str) -> String {
    if key.to_ascii_lowercase().starts_with("0x") {
        format!("wallet:{}", key.to_ascii_lowercase())
    } else {
        format!("ip:{key}")
    }
}

// ─── PostgreSQL ────────────────────────────────────────────────────────────

pub struct PgRelayerStore {
    pool: PgPool,
}

impl PgRelayerStore {
    /// Create or verify the relayer tables, then wrap the pool.
    pub async fn open(pool: PgPool) -> Result<Self> {
        let mut tx = pool.begin().await.context("relayer schema: begin")?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(SCHEMA_LOCK_ID)
            .execute(&mut *tx)
            .await
            .context("relayer schema: lock")?;
        sqlx::raw_sql(SCHEMA_SQL)
            .execute(&mut *tx)
            .await
            .context("relayer schema: create tables")?;
        let version: Option<String> =
            sqlx::query_scalar("SELECT value FROM relayer_meta WHERE key = 'schema_version'")
                .fetch_optional(&mut *tx)
                .await
                .context("relayer schema: read version")?;
        match version.as_deref() {
            None => {
                sqlx::query(
                    "INSERT INTO relayer_meta (key, value) VALUES ('schema_version', $1)",
                )
                .bind(SCHEMA_VERSION.to_string())
                .execute(&mut *tx)
                .await
                .context("relayer schema: record version")?;
            }
            Some(value) if value == SCHEMA_VERSION.to_string() => {}
            Some(value) => bail!(
                "relayer schema version {value} is not supported by this build (expects {SCHEMA_VERSION}); refusing to touch durable state"
            ),
        }
        tx.commit().await.context("relayer schema: commit")?;
        Ok(Self { pool })
    }
}

fn to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| anyhow!("{value} does not fit in BIGINT"))
}

fn parse_rfc3339(text: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(text)
        .with_context(|| format!("invalid timestamp {text:?}"))?
        .with_timezone(&Utc))
}

fn outbox_from_row(row: &sqlx::postgres::PgRow) -> Result<OutboxRow> {
    let chain_id: i64 = row.try_get("chain_id")?;
    let created_at: DateTime<Utc> = row.try_get("created_at")?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at")?;
    Ok(OutboxRow {
        record: RequestStatusRecord {
            request_id: row.try_get("request_id")?,
            status: row.try_get("status")?,
            action: row.try_get("action")?,
            owner: row.try_get("owner")?,
            chain_id: u64::try_from(chain_id).context("negative chain id in outbox")?,
            tx_hash: row.try_get("tx_hash")?,
            block_number: row.try_get("block_number")?,
            message: row.try_get("message")?,
            created_at: created_at.to_rfc3339(),
            updated_at: updated_at.to_rfc3339(),
        },
        signer_request_id: row.try_get("signer_request_id")?,
        created_at,
    })
}

/// SQLSTATE for serialization failures and deadlocks: retry the transaction.
fn is_retryable(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|db| db.code())
        .is_some_and(|code| code == "40001" || code == "40P01")
}

enum Attempt {
    Done,
    Abort(ReserveError),
}

async fn increment_quota(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &str,
    day: NaiveDate,
    limit: u64,
) -> std::result::Result<bool, sqlx::Error> {
    // Insert or increment, but only while below the limit.
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    if limit <= 0 {
        return Ok(false);
    }
    let updated: Option<i64> = sqlx::query_scalar(
        "INSERT INTO relayer_batches (quota_key, day, count) VALUES ($1, $2, 1) \
         ON CONFLICT (quota_key, day) DO UPDATE SET count = relayer_batches.count + 1 \
         WHERE relayer_batches.count < $3 \
         RETURNING count",
    )
    .bind(key)
    .bind(day)
    .bind(limit)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(updated.is_some())
}

impl PgRelayerStore {
    async fn reserve_once(
        &self,
        reservation: &Reservation,
    ) -> std::result::Result<Attempt, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await?;

        let chain_id = i64::try_from(reservation.chain_id).unwrap_or(i64::MAX);
        let current: Option<String> = sqlx::query_scalar(
            "SELECT next_nonce::text FROM relayer_checkpoints \
             WHERE chain_id = $1 AND owner = $2 FOR UPDATE",
        )
        .bind(chain_id)
        .bind(&reservation.owner)
        .fetch_optional(&mut *tx)
        .await?;
        let actual = match current {
            Some(text) => text.parse::<u64>().unwrap_or(u64::MAX),
            None => 0,
        };
        if actual != reservation.expected_owner_nonce {
            return Ok(Attempt::Abort(ReserveError::NonceConflict(actual)));
        }
        let Some(next) = actual.checked_add(1) else {
            return Ok(Attempt::Abort(ReserveError::NonceExhausted));
        };

        if !increment_quota(
            &mut tx,
            &reservation.wallet_quota_key,
            reservation.day,
            reservation.wallet_quota_limit,
        )
        .await?
            || !increment_quota(
                &mut tx,
                &reservation.ip_quota_key,
                reservation.day,
                reservation.ip_quota_limit,
            )
            .await?
        {
            return Ok(Attempt::Abort(ReserveError::QuotaReached));
        }

        sqlx::query(
            "INSERT INTO relayer_checkpoints (chain_id, owner, next_nonce, updated_at) \
             VALUES ($1, $2, $3::numeric, now()) \
             ON CONFLICT (chain_id, owner) DO UPDATE SET next_nonce = EXCLUDED.next_nonce, \
             updated_at = now()",
        )
        .bind(chain_id)
        .bind(&reservation.owner)
        .bind(next.to_string())
        .execute(&mut *tx)
        .await?;

        let record = &reservation.record;
        let created = parse_rfc3339(&record.created_at).unwrap_or_else(|_| Utc::now());
        let inserted = sqlx::query(
            "INSERT INTO relayer_outbox (request_id, status, action, owner, chain_id, \
             signer_request_id, tx_hash, block_number, message, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10) \
             ON CONFLICT (request_id) DO NOTHING",
        )
        .bind(&record.request_id)
        .bind(&record.status)
        .bind(&record.action)
        .bind(&record.owner)
        .bind(chain_id)
        .bind(&reservation.signer_request_id)
        .bind(&record.tx_hash)
        .bind(&record.block_number)
        .bind(&record.message)
        .bind(created)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Ok(Attempt::Abort(ReserveError::Storage(
                "request identifier collision".to_string(),
            )));
        }

        tx.commit().await?;
        Ok(Attempt::Done)
    }
}

#[async_trait]
impl RelayerStore for PgRelayerStore {
    async fn owner_nonce(&self, chain_id: u64, owner: &str) -> Result<u64> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT next_nonce::text FROM relayer_checkpoints WHERE chain_id = $1 AND owner = $2",
        )
        .bind(to_i64(chain_id)?)
        .bind(owner)
        .fetch_optional(&self.pool)
        .await
        .context("relayer: owner nonce read failed")?;
        match value {
            Some(text) => text
                .parse()
                .map_err(|_| anyhow!("stored relayer nonce is not a u64")),
            None => Ok(0),
        }
    }

    async fn quota_count(&self, quota_key: &str, day: NaiveDate) -> Result<u64> {
        let count: Option<i64> = sqlx::query_scalar(
            "SELECT count FROM relayer_batches WHERE quota_key = $1 AND day = $2",
        )
        .bind(quota_key)
        .bind(day)
        .fetch_optional(&self.pool)
        .await
        .context("relayer: quota read failed")?;
        Ok(count
            .map(|count| u64::try_from(count).unwrap_or(0))
            .unwrap_or(0))
    }

    async fn reserve(&self, reservation: &Reservation) -> Result<(), ReserveError> {
        for _ in 0..5 {
            match self.reserve_once(reservation).await {
                Ok(Attempt::Done) => return Ok(()),
                Ok(Attempt::Abort(error)) => return Err(error),
                Err(error) if is_retryable(&error) => continue,
                Err(error) => return Err(ReserveError::Storage(error.to_string())),
            }
        }
        Err(ReserveError::Storage(
            "too many concurrent reservations; retry".to_string(),
        ))
    }

    async fn update_request(&self, record: &RequestStatusRecord) -> Result<()> {
        let result = sqlx::query(
            "UPDATE relayer_outbox SET status = $2, tx_hash = $3, block_number = $4, \
             message = $5, updated_at = now() WHERE request_id = $1",
        )
        .bind(&record.request_id)
        .bind(&record.status)
        .bind(&record.tx_hash)
        .bind(&record.block_number)
        .bind(&record.message)
        .execute(&self.pool)
        .await
        .context("relayer: outbox update failed")?;
        if result.rows_affected() != 1 {
            bail!(
                "request {} is missing from durable storage",
                record.request_id
            );
        }
        Ok(())
    }

    async fn get_request(&self, request_id: &str) -> Result<Option<OutboxRow>> {
        let row = sqlx::query("SELECT * FROM relayer_outbox WHERE request_id = $1")
            .bind(request_id)
            .fetch_optional(&self.pool)
            .await
            .context("relayer: outbox read failed")?;
        row.as_ref().map(outbox_from_row).transpose()
    }

    async fn open_requests(&self) -> Result<Vec<OutboxRow>> {
        let rows = sqlx::query(
            "SELECT * FROM relayer_outbox WHERE status = ANY($1) ORDER BY created_at LIMIT 1000",
        )
        .bind(
            OPEN_STATUSES
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )
        .fetch_all(&self.pool)
        .await
        .context("relayer: open outbox query failed")?;
        rows.iter().map(outbox_from_row).collect()
    }

    async fn meta_get(&self, key: &str) -> Result<Option<String>> {
        sqlx::query_scalar("SELECT value FROM relayer_meta WHERE key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .context("relayer: meta read failed")
    }

    async fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO relayer_meta (key, value, updated_at) VALUES ($1, $2, now()) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await
        .context("relayer: meta write failed")?;
        Ok(())
    }

    async fn import_legacy(&self, export: &LegacyJournalExport) -> Result<LegacyImportSummary> {
        if export.format != LEGACY_EXPORT_FORMAT {
            bail!(
                "unsupported relayer journal export format {:?}",
                export.format
            );
        }
        let mut tx = self.pool.begin().await?;
        let mut summary = LegacyImportSummary::default();
        for (key, value) in &export.nonces {
            let (chain_id, owner) = parse_legacy_nonce_key(key)?;
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT next_nonce::text FROM relayer_checkpoints WHERE chain_id = $1 AND owner = $2",
            )
            .bind(to_i64(chain_id)?)
            .bind(&owner)
            .fetch_optional(&mut *tx)
            .await?;
            match existing {
                Some(text) if text != value.to_string() => bail!(
                    "legacy relayer nonce for {key} conflicts with the database; operator review is required"
                ),
                Some(_) => {}
                None => {
                    sqlx::query(
                        "INSERT INTO relayer_checkpoints (chain_id, owner, next_nonce) \
                         VALUES ($1, $2, $3::numeric)",
                    )
                    .bind(to_i64(chain_id)?)
                    .bind(&owner)
                    .bind(value.to_string())
                    .execute(&mut *tx)
                    .await?;
                    summary.nonces += 1;
                }
            }
        }
        for (key, counter) in &export.quotas {
            let Ok(day) = NaiveDate::parse_from_str(&counter.day, "%Y-%m-%d") else {
                continue;
            };
            let count = i64::try_from(counter.count).unwrap_or(i64::MAX);
            sqlx::query(
                "INSERT INTO relayer_batches (quota_key, day, count) VALUES ($1, $2, $3) \
                 ON CONFLICT (quota_key, day) DO UPDATE SET \
                 count = GREATEST(relayer_batches.count, EXCLUDED.count)",
            )
            .bind(legacy_quota_key(key))
            .bind(day)
            .bind(count)
            .execute(&mut *tx)
            .await?;
            summary.quotas += 1;
        }
        for record in &export.requests {
            let created = parse_rfc3339(&record.created_at).unwrap_or_else(|_| Utc::now());
            let updated = parse_rfc3339(&record.updated_at).unwrap_or(created);
            let result = sqlx::query(
                "INSERT INTO relayer_outbox (request_id, status, action, owner, chain_id, \
                 signer_request_id, tx_hash, block_number, message, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, NULL, $6, $7, $8, $9, $10) \
                 ON CONFLICT (request_id) DO NOTHING",
            )
            .bind(&record.request_id)
            .bind(&record.status)
            .bind(&record.action)
            .bind(&record.owner)
            .bind(to_i64(record.chain_id)?)
            .bind(&record.tx_hash)
            .bind(&record.block_number)
            .bind(&record.message)
            .bind(created)
            .bind(updated)
            .execute(&mut *tx)
            .await?;
            summary.requests += usize::try_from(result.rows_affected()).unwrap_or(0);
        }
        tx.commit().await?;
        Ok(summary)
    }
}

// ─── In-memory (tests) ─────────────────────────────────────────────────────

#[cfg(test)]
#[derive(Default)]
pub struct MemoryRelayerStore {
    state: Mutex<MemoryState>,
}

#[cfg(test)]
#[derive(Default)]
struct MemoryState {
    checkpoints: HashMap<(u64, String), u64>,
    batches: HashMap<(String, NaiveDate), u64>,
    outbox: BTreeMap<String, OutboxRow>,
    meta: HashMap<String, String>,
}

#[cfg(test)]
impl MemoryRelayerStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MemoryState>> {
        self.state
            .lock()
            .map_err(|_| anyhow!("in-memory relayer store poisoned"))
    }

    /// True when nothing has been reserved or recorded. Test helper.
    pub fn is_empty(&self) -> bool {
        self.lock()
            .map(|state| {
                state.checkpoints.is_empty() && state.batches.is_empty() && state.outbox.is_empty()
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
#[async_trait]
impl RelayerStore for MemoryRelayerStore {
    async fn owner_nonce(&self, chain_id: u64, owner: &str) -> Result<u64> {
        Ok(self
            .lock()?
            .checkpoints
            .get(&(chain_id, owner.to_string()))
            .copied()
            .unwrap_or(0))
    }

    async fn quota_count(&self, quota_key: &str, day: NaiveDate) -> Result<u64> {
        Ok(self
            .lock()?
            .batches
            .get(&(quota_key.to_string(), day))
            .copied()
            .unwrap_or(0))
    }

    async fn reserve(&self, reservation: &Reservation) -> Result<(), ReserveError> {
        let mut state = self
            .lock()
            .map_err(|error| ReserveError::Storage(error.to_string()))?;
        let nonce_key = (reservation.chain_id, reservation.owner.clone());
        let actual = state.checkpoints.get(&nonce_key).copied().unwrap_or(0);
        if actual != reservation.expected_owner_nonce {
            return Err(ReserveError::NonceConflict(actual));
        }
        let next = actual.checked_add(1).ok_or(ReserveError::NonceExhausted)?;
        let wallet = (reservation.wallet_quota_key.clone(), reservation.day);
        let ip = (reservation.ip_quota_key.clone(), reservation.day);
        let wallet_count = state.batches.get(&wallet).copied().unwrap_or(0);
        let ip_count = state.batches.get(&ip).copied().unwrap_or(0);
        if wallet_count >= reservation.wallet_quota_limit || ip_count >= reservation.ip_quota_limit
        {
            return Err(ReserveError::QuotaReached);
        }
        if state.outbox.contains_key(&reservation.record.request_id) {
            return Err(ReserveError::Storage(
                "request identifier collision".to_string(),
            ));
        }
        state.checkpoints.insert(nonce_key, next);
        state.batches.insert(wallet, wallet_count + 1);
        state.batches.insert(ip, ip_count + 1);
        state.outbox.insert(
            reservation.record.request_id.clone(),
            OutboxRow {
                record: reservation.record.clone(),
                signer_request_id: Some(reservation.signer_request_id.clone()),
                created_at: parse_rfc3339(&reservation.record.created_at)
                    .unwrap_or_else(|_| Utc::now()),
            },
        );
        Ok(())
    }

    async fn update_request(&self, record: &RequestStatusRecord) -> Result<()> {
        let mut state = self.lock()?;
        let row = state
            .outbox
            .get_mut(&record.request_id)
            .ok_or_else(|| anyhow!("request {} is missing", record.request_id))?;
        row.record = RequestStatusRecord {
            created_at: row.record.created_at.clone(),
            updated_at: Utc::now().to_rfc3339(),
            ..record.clone()
        };
        Ok(())
    }

    async fn get_request(&self, request_id: &str) -> Result<Option<OutboxRow>> {
        Ok(self.lock()?.outbox.get(request_id).cloned())
    }

    async fn open_requests(&self) -> Result<Vec<OutboxRow>> {
        Ok(self
            .lock()?
            .outbox
            .values()
            .filter(|row| OPEN_STATUSES.contains(&row.record.status.as_str()))
            .cloned()
            .collect())
    }

    async fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.lock()?.meta.get(key).cloned())
    }

    async fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.lock()?.meta.insert(key.to_string(), value.to_string());
        Ok(())
    }

    async fn import_legacy(&self, export: &LegacyJournalExport) -> Result<LegacyImportSummary> {
        if export.format != LEGACY_EXPORT_FORMAT {
            bail!(
                "unsupported relayer journal export format {:?}",
                export.format
            );
        }
        let mut state = self.lock()?;
        let mut summary = LegacyImportSummary::default();
        for (key, value) in &export.nonces {
            let id = parse_legacy_nonce_key(key)?;
            match state.checkpoints.get(&id) {
                Some(existing) if existing != value => bail!(
                    "legacy relayer nonce for {key} conflicts with the database; operator review is required"
                ),
                Some(_) => {}
                None => {
                    state.checkpoints.insert(id, *value);
                    summary.nonces += 1;
                }
            }
        }
        for (key, counter) in &export.quotas {
            if let Ok(day) = NaiveDate::parse_from_str(&counter.day, "%Y-%m-%d") {
                let slot = state
                    .batches
                    .entry((legacy_quota_key(key), day))
                    .or_insert(0);
                *slot = (*slot).max(counter.count);
                summary.quotas += 1;
            }
        }
        for record in &export.requests {
            if !state.outbox.contains_key(&record.request_id) {
                state.outbox.insert(
                    record.request_id.clone(),
                    OutboxRow {
                        record: record.clone(),
                        signer_request_id: None,
                        created_at: parse_rfc3339(&record.created_at)
                            .unwrap_or_else(|_| Utc::now()),
                    },
                );
                summary.requests += 1;
            }
        }
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reservation(id: &str, expected: u64) -> Reservation {
        let now = Utc::now().to_rfc3339();
        Reservation {
            chain_id: 11_155_111,
            owner: format!("0x{}", "ab".repeat(20)),
            expected_owner_nonce: expected,
            wallet_quota_key: "wallet:owner:publisher".to_string(),
            wallet_quota_limit: 2,
            ip_quota_key: "ip:11155111:198.51.100.7".to_string(),
            ip_quota_limit: 10,
            day: NaiveDate::from_ymd_opt(2026, 11, 23).unwrap(),
            record: RequestStatusRecord {
                request_id: id.to_string(),
                status: "submitting".to_string(),
                action: "publisher".to_string(),
                owner: format!("0x{}", "ab".repeat(20)),
                chain_id: 11_155_111,
                tx_hash: None,
                block_number: None,
                message: "Submitting.".to_string(),
                created_at: now.clone(),
                updated_at: now,
            },
            signer_request_id: format!("relayer:{id}"),
        }
    }

    async fn exercise_store(store: &dyn RelayerStore) {
        let owner = format!("0x{}", "ab".repeat(20));
        let day = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap();

        store.reserve(&reservation("r1", 0)).await.unwrap();
        assert_eq!(store.owner_nonce(11_155_111, &owner).await.unwrap(), 1);
        assert_eq!(
            store.reserve(&reservation("r2", 0)).await,
            Err(ReserveError::NonceConflict(1))
        );
        store.reserve(&reservation("r2", 1)).await.unwrap();
        assert_eq!(
            store.reserve(&reservation("r3", 2)).await,
            Err(ReserveError::QuotaReached),
            "wallet quota of 2 per day"
        );
        // A refused reservation changes nothing.
        assert_eq!(store.owner_nonce(11_155_111, &owner).await.unwrap(), 2);
        assert_eq!(
            store
                .quota_count("wallet:owner:publisher", day)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            store
                .quota_count("ip:11155111:198.51.100.7", day)
                .await
                .unwrap(),
            2
        );
        assert!(store.get_request("r3").await.unwrap().is_none());

        let mut row = store.get_request("r1").await.unwrap().unwrap();
        assert_eq!(row.signer_request_id.as_deref(), Some("relayer:r1"));
        row.record.status = "confirmed".to_string();
        row.record.tx_hash = Some("0x01".to_string());
        store.update_request(&row.record).await.unwrap();
        let open: Vec<String> = store
            .open_requests()
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.record.request_id)
            .collect();
        assert_eq!(open, vec!["r2".to_string()]);

        assert_eq!(store.meta_get("legacy").await.unwrap(), None);
        store.meta_set("legacy", "imported").await.unwrap();
        assert_eq!(
            store.meta_get("legacy").await.unwrap().as_deref(),
            Some("imported")
        );

        let export = LegacyJournalExport {
            format: LEGACY_EXPORT_FORMAT.to_string(),
            nonces: BTreeMap::from([
                (format!("11155111:{owner}"), 2),
                (format!("11155111:0x{}", "cd".repeat(20)), 9),
            ]),
            quotas: BTreeMap::from([(
                format!("0x{}:validator", "cd".repeat(20)),
                LegacyDailyCounter {
                    day: "2026-11-22".to_string(),
                    count: 3,
                },
            )]),
            requests: vec![RequestStatusRecord {
                request_id: "legacy-1".to_string(),
                status: "timed_out".to_string(),
                action: "validator".to_string(),
                owner: format!("0x{}", "cd".repeat(20)),
                chain_id: 11_155_111,
                tx_hash: Some(format!("0x{}", "11".repeat(32))),
                block_number: None,
                message: "Timed out.".to_string(),
                created_at: "2026-11-22T10:00:00+00:00".to_string(),
                updated_at: "2026-11-22T10:05:00+00:00".to_string(),
            }],
        };
        let summary = store.import_legacy(&export).await.unwrap();
        assert_eq!(summary.nonces, 1, "the matching nonce is not re-inserted");
        assert_eq!(summary.requests, 1);
        assert_eq!(
            store
                .owner_nonce(11_155_111, &format!("0x{}", "cd".repeat(20)))
                .await
                .unwrap(),
            9
        );
        let legacy = store.get_request("legacy-1").await.unwrap().unwrap();
        assert_eq!(legacy.signer_request_id, None);

        let mut conflicting = export.clone();
        conflicting.nonces = BTreeMap::from([(format!("11155111:{owner}"), 99)]);
        assert!(store.import_legacy(&conflicting).await.is_err());

        let mut wrong = export;
        wrong.format = "something-else".to_string();
        assert!(store.import_legacy(&wrong).await.is_err());
    }

    #[tokio::test]
    async fn memory_store_semantics() {
        exercise_store(&MemoryRelayerStore::new()).await;
    }

    /// Same semantics against PostgreSQL when `CREG_TEST_DATABASE_URL` is set.
    #[tokio::test]
    async fn pg_store_semantics() {
        let Ok(url) = std::env::var("CREG_TEST_DATABASE_URL") else {
            eprintln!("skipping: CREG_TEST_DATABASE_URL is not set");
            return;
        };
        let (pool, _) = l1_tx::pg::connect_pool(&url, "CREG_TEST_DATABASE_URL", true, 4)
            .await
            .unwrap();
        sqlx::raw_sql(
            "DROP TABLE IF EXISTS relayer_outbox, relayer_batches, relayer_checkpoints, relayer_meta",
        )
        .execute(&pool)
        .await
        .unwrap();
        let store = PgRelayerStore::open(pool.clone()).await.unwrap();
        // Reopening an existing schema is a no-op.
        let store_again = PgRelayerStore::open(pool).await.unwrap();
        drop(store_again);
        exercise_store(&store).await;
    }

    #[test]
    fn legacy_keys_are_parsed_and_namespaced() {
        let (chain, owner) =
            parse_legacy_nonce_key(&format!("11155111:0x{}", "AB".repeat(20))).unwrap();
        assert_eq!(chain, 11_155_111);
        assert_eq!(owner, format!("0x{}", "ab".repeat(20)));
        assert!(parse_legacy_nonce_key("garbage").is_err());
        assert!(parse_legacy_nonce_key("1:not-an-address").is_err());
        assert_eq!(
            legacy_quota_key("0xAbC:publisher"),
            "wallet:0xabc:publisher"
        );
        assert_eq!(legacy_quota_key("11155111:1.2.3.4"), "ip:11155111:1.2.3.4");
    }
}
