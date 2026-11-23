//! PostgreSQL implementation of the nonce ledger, plus the connection policy
//! shared by the relayer and faucet (`ECA-32`: TLS or a local socket).

use crate::ledger::{LedgerTx, NonceLedger, SignerKey, TxAttempt, TxState, WriterLease};
use alloy::primitives::{Address, Bytes, B256, U256};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions, PgRow, PgSslMode},
    Connection, PgConnection, PgPool, Row,
};
use std::{str::FromStr, time::Duration};

/// Advisory lock serialising schema bootstrap across processes.
const SCHEMA_LOCK_ID: i64 = 0x4c31_7478_5f73_6368; // "L1tx_sch"
/// Current schema version of the ledger tables.
pub const SCHEMA_VERSION: i32 = 1;

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS l1_tx_schema (
    component TEXT PRIMARY KEY,
    version   INT  NOT NULL
);

CREATE TABLE IF NOT EXISTS l1_tx_ledger (
    chain_id         BIGINT        NOT NULL CHECK (chain_id > 0),
    signer           TEXT          NOT NULL CHECK (signer ~ '^0x[0-9a-f]{40}$'),
    request_id       TEXT          NOT NULL CHECK (length(request_id) BETWEEN 1 AND 128),
    nonce            BIGINT        NOT NULL CHECK (nonce >= 0),
    state            TEXT          NOT NULL CHECK (state IN ('signed','pending','included','confirmed','reverted','rejected','displaced')),
    tx_to            TEXT          NOT NULL CHECK (tx_to ~ '^0x[0-9a-f]{40}$'),
    tx_value         NUMERIC(78,0) NOT NULL CHECK (tx_value >= 0),
    tx_input         TEXT          NOT NULL,
    gas_limit        BIGINT        NOT NULL CHECK (gas_limit > 0),
    attempts         JSONB         NOT NULL DEFAULT '[]'::jsonb,
    included_tx_hash TEXT,
    block_number     BIGINT,
    last_error       TEXT,
    broadcast_at     TIMESTAMPTZ,
    created_at       TIMESTAMPTZ   NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ   NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, signer, request_id)
);

-- One live row per EOA nonce; a rejected row released its nonce.
CREATE UNIQUE INDEX IF NOT EXISTS l1_tx_ledger_live_nonce
    ON l1_tx_ledger (chain_id, signer, nonce) WHERE state <> 'rejected';

CREATE INDEX IF NOT EXISTS l1_tx_ledger_open
    ON l1_tx_ledger (chain_id, signer, nonce)
    WHERE state IN ('signed','pending','included');
"#;

/// Create or verify the ledger tables. Safe to call from several processes.
pub async fn ensure_schema(pool: &PgPool) -> Result<()> {
    let mut tx = pool.begin().await.context("l1-tx schema: begin")?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(SCHEMA_LOCK_ID)
        .execute(&mut *tx)
        .await
        .context("l1-tx schema: lock")?;
    sqlx::raw_sql(SCHEMA_SQL)
        .execute(&mut *tx)
        .await
        .context("l1-tx schema: create tables")?;
    let existing: Option<i32> =
        sqlx::query_scalar("SELECT version FROM l1_tx_schema WHERE component = 'l1_tx_ledger'")
            .fetch_optional(&mut *tx)
            .await
            .context("l1-tx schema: read version")?;
    match existing {
        None => {
            sqlx::query(
                "INSERT INTO l1_tx_schema (component, version) VALUES ('l1_tx_ledger', $1)",
            )
            .bind(SCHEMA_VERSION)
            .execute(&mut *tx)
            .await
            .context("l1-tx schema: record version")?;
        }
        Some(version) if version == SCHEMA_VERSION => {}
        Some(version) => bail!(
            "l1-tx ledger schema version {version} is not supported by this build (expects {SCHEMA_VERSION})"
        ),
    }
    tx.commit().await.context("l1-tx schema: commit")?;
    Ok(())
}

/// How a service may reach PostgreSQL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PgTransport {
    /// `sslmode=require|verify-ca|verify-full`.
    Tls,
    /// UNIX domain socket.
    UnixSocket,
    /// Loopback TCP (same host).
    Loopback,
    /// Plaintext TCP on a private container network, explicitly allowed.
    PlaintextPrivateNetwork,
}

/// Parse a PostgreSQL URL and enforce the transport policy (`ECA-32`):
/// TLS, a UNIX socket or loopback, unless `allow_plaintext_private_network`
/// is set for a single-host private container network. Errors name
/// `url_label` and never echo the URL (it carries a password).
pub fn connect_options(
    url: &str,
    url_label: &str,
    allow_plaintext_private_network: bool,
) -> Result<(PgConnectOptions, PgTransport)> {
    let options = PgConnectOptions::from_str(url.trim())
        .map_err(|_| anyhow!("{url_label} is not a valid PostgreSQL URL"))?;
    let transport = if matches!(
        options.get_ssl_mode(),
        PgSslMode::Require | PgSslMode::VerifyCa | PgSslMode::VerifyFull
    ) {
        PgTransport::Tls
    } else if options.get_socket().is_some() || options.get_host().starts_with('/') {
        PgTransport::UnixSocket
    } else if is_loopback_host(options.get_host()) {
        PgTransport::Loopback
    } else if allow_plaintext_private_network {
        PgTransport::PlaintextPrivateNetwork
    } else {
        bail!(
            "{url_label} must use TLS (sslmode=require, verify-ca or verify-full), a UNIX socket, \
             or a loopback host; plaintext TCP to another host is allowed only on a private \
             single-host container network with the matching *_ALLOW_PLAINTEXT_PRIVATE_NETWORK=true"
        );
    };
    // `prefer` may silently fall back to plaintext; pin the decision.
    let options = match transport {
        PgTransport::Tls => options,
        _ => options.ssl_mode(PgSslMode::Disable),
    };
    Ok((options, transport))
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// Build a bounded pool with the transport policy applied.
pub async fn connect_pool(
    url: &str,
    url_label: &str,
    allow_plaintext_private_network: bool,
    max_connections: u32,
) -> Result<(PgPool, PgTransport)> {
    let (options, transport) = connect_options(url, url_label, allow_plaintext_private_network)?;
    let pool = PgPoolOptions::new()
        .max_connections(max_connections.max(2))
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await
        .map_err(|error| anyhow!("could not connect to PostgreSQL ({url_label}): {error}"))?;
    Ok((pool, transport))
}

/// Durable ledger in PostgreSQL. Writer leases are session-level advisory
/// locks held on a connection detached from the pool for the lease lifetime.
#[derive(Clone)]
pub struct PgNonceLedger {
    pool: PgPool,
}

impl PgNonceLedger {
    /// Wrap a pool. Call [`ensure_schema`] first.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

struct PgWriterLease {
    connection: PgConnection,
}

#[async_trait]
impl WriterLease for PgWriterLease {
    async fn check(&mut self) -> Result<()> {
        // The session lock lives exactly as long as this connection.
        self.connection
            .ping()
            .await
            .context("the PostgreSQL session holding the signer lock was lost")
    }
}

fn to_i64(value: u64, what: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| anyhow!("{what} {value} does not fit in BIGINT"))
}

fn parse_address(text: &str, what: &str) -> Result<Address> {
    text.parse()
        .map_err(|_| anyhow!("ledger column {what} holds an invalid address"))
}

fn row_to_tx(row: &PgRow) -> Result<LedgerTx> {
    let chain_id: i64 = row.try_get("chain_id")?;
    let signer: String = row.try_get("signer")?;
    let nonce: i64 = row.try_get("nonce")?;
    let state: String = row.try_get("state")?;
    let to: String = row.try_get("tx_to")?;
    let value: String = row.try_get("tx_value")?;
    let input: String = row.try_get("tx_input")?;
    let gas_limit: i64 = row.try_get("gas_limit")?;
    let attempts: String = row.try_get("attempts")?;
    let included: Option<String> = row.try_get("included_tx_hash")?;
    let block_number: Option<i64> = row.try_get("block_number")?;

    Ok(LedgerTx {
        key: SignerKey::new(
            u64::try_from(chain_id).context("negative chain id in ledger")?,
            parse_address(&signer, "signer")?,
        ),
        request_id: row.try_get("request_id")?,
        nonce: u64::try_from(nonce).context("negative nonce in ledger")?,
        state: TxState::parse(&state)?,
        to: parse_address(&to, "tx_to")?,
        value: U256::from_str_radix(&value, 10).context("invalid tx_value in ledger")?,
        input: input
            .parse::<Bytes>()
            .map_err(|_| anyhow!("invalid tx_input in ledger"))?,
        gas_limit: u64::try_from(gas_limit).context("invalid gas_limit in ledger")?,
        attempts: serde_json::from_str::<Vec<TxAttempt>>(&attempts)
            .context("invalid attempts in ledger")?,
        included_tx_hash: included
            .map(|hash| {
                hash.parse::<B256>()
                    .map_err(|_| anyhow!("invalid included_tx_hash in ledger"))
            })
            .transpose()?,
        block_number: block_number
            .map(|block| u64::try_from(block).context("negative block number in ledger"))
            .transpose()?,
        last_error: row.try_get("last_error")?,
        broadcast_at: row.try_get::<Option<DateTime<Utc>>, _>("broadcast_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

const SELECT_COLUMNS: &str = "chain_id, signer, request_id, nonce, state, tx_to, \
    tx_value::text AS tx_value, tx_input, gas_limit, attempts::text AS attempts, \
    included_tx_hash, block_number, last_error, broadcast_at, created_at, updated_at";

#[async_trait]
impl NonceLedger for PgNonceLedger {
    async fn acquire_writer(&self, key: SignerKey) -> Result<Box<dyn WriterLease>> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .context("l1-tx: no PostgreSQL connection for the writer lease")?
            .detach();
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(key.lock_id())
            .fetch_one(&mut connection)
            .await
            .context("l1-tx: advisory lock query failed")?;
        if !acquired {
            bail!(
                "another process already owns the hot key {} on chain {} (single-writer, ECO-INV-07)",
                key.address_hex(),
                key.chain_id
            );
        }
        Ok(Box::new(PgWriterLease { connection }))
    }

    async fn highest_nonce(&self, key: SignerKey) -> Result<Option<u64>> {
        let highest: Option<i64> = sqlx::query_scalar(
            "SELECT max(nonce) FROM l1_tx_ledger \
             WHERE chain_id = $1 AND signer = $2 AND state <> 'rejected'",
        )
        .bind(to_i64(key.chain_id, "chain id")?)
        .bind(key.address_hex())
        .fetch_one(&self.pool)
        .await
        .context("l1-tx: highest nonce query failed")?;
        highest
            .map(|nonce| u64::try_from(nonce).context("negative nonce in ledger"))
            .transpose()
    }

    async fn insert(&self, tx: &LedgerTx) -> Result<()> {
        sqlx::query(
            "INSERT INTO l1_tx_ledger (chain_id, signer, request_id, nonce, state, tx_to, \
             tx_value, tx_input, gas_limit, attempts, included_tx_hash, block_number, \
             last_error, broadcast_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::numeric, $8, $9, $10::jsonb, $11, $12, \
             $13, $14, $15, $16)",
        )
        .bind(to_i64(tx.key.chain_id, "chain id")?)
        .bind(tx.key.address_hex())
        .bind(&tx.request_id)
        .bind(to_i64(tx.nonce, "nonce")?)
        .bind(tx.state.as_str())
        .bind(format!("{:#x}", tx.to))
        .bind(tx.value.to_string())
        .bind(tx.input.to_string())
        .bind(to_i64(tx.gas_limit, "gas limit")?)
        .bind(serde_json::to_string(&tx.attempts)?)
        .bind(tx.included_tx_hash.map(|hash| hash.to_string()))
        .bind(
            tx.block_number
                .map(|block| to_i64(block, "block number"))
                .transpose()?,
        )
        .bind(&tx.last_error)
        .bind(tx.broadcast_at)
        .bind(tx.created_at)
        .bind(tx.updated_at)
        .execute(&self.pool)
        .await
        .context("l1-tx: ledger insert failed")?;
        Ok(())
    }

    async fn update(&self, tx: &LedgerTx) -> Result<()> {
        let result = sqlx::query(
            "UPDATE l1_tx_ledger SET nonce = $4, state = $5, attempts = $6::jsonb, \
             included_tx_hash = $7, block_number = $8, last_error = $9, broadcast_at = $10, \
             updated_at = $11 \
             WHERE chain_id = $1 AND signer = $2 AND request_id = $3",
        )
        .bind(to_i64(tx.key.chain_id, "chain id")?)
        .bind(tx.key.address_hex())
        .bind(&tx.request_id)
        .bind(to_i64(tx.nonce, "nonce")?)
        .bind(tx.state.as_str())
        .bind(serde_json::to_string(&tx.attempts)?)
        .bind(tx.included_tx_hash.map(|hash| hash.to_string()))
        .bind(
            tx.block_number
                .map(|block| to_i64(block, "block number"))
                .transpose()?,
        )
        .bind(&tx.last_error)
        .bind(tx.broadcast_at)
        .bind(tx.updated_at)
        .execute(&self.pool)
        .await
        .context("l1-tx: ledger update failed")?;
        if result.rows_affected() != 1 {
            bail!("l1-tx: unknown ledger request id {}", tx.request_id);
        }
        Ok(())
    }

    async fn get(&self, key: SignerKey, request_id: &str) -> Result<Option<LedgerTx>> {
        let row = sqlx::query(&format!(
            "SELECT {SELECT_COLUMNS} FROM l1_tx_ledger \
             WHERE chain_id = $1 AND signer = $2 AND request_id = $3"
        ))
        .bind(to_i64(key.chain_id, "chain id")?)
        .bind(key.address_hex())
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await
        .context("l1-tx: ledger read failed")?;
        row.as_ref().map(row_to_tx).transpose()
    }

    async fn open(&self, key: SignerKey) -> Result<Vec<LedgerTx>> {
        let rows = sqlx::query(&format!(
            "SELECT {SELECT_COLUMNS} FROM l1_tx_ledger \
             WHERE chain_id = $1 AND signer = $2 AND state IN ('signed','pending','included') \
             ORDER BY nonce"
        ))
        .bind(to_i64(key.chain_id, "chain id")?)
        .bind(key.address_hex())
        .fetch_all(&self.pool)
        .await
        .context("l1-tx: open ledger rows query failed")?;
        rows.iter().map(row_to_tx).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_policy_requires_tls_off_host() {
        let (_, transport) = connect_options(
            "postgres://u:p@db.example:5432/x?sslmode=require",
            "URL",
            false,
        )
        .unwrap();
        assert_eq!(transport, PgTransport::Tls);

        let (_, transport) =
            connect_options("postgres://u:p@127.0.0.1:5432/x", "URL", false).unwrap();
        assert_eq!(transport, PgTransport::Loopback);
        let (_, transport) = connect_options("postgres://u:p@localhost/x", "URL", false).unwrap();
        assert_eq!(transport, PgTransport::Loopback);

        let error = connect_options(
            "postgres://u:secret@faucet-db:5432/x",
            "FAUCET_PG_URL",
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("FAUCET_PG_URL"));
        assert!(!error.contains("secret"), "the password must not be echoed");

        // `prefer` (the default) is not TLS.
        assert!(connect_options(
            "postgres://u:p@faucet-db:5432/x?sslmode=prefer",
            "URL",
            false
        )
        .is_err());

        let (options, transport) =
            connect_options("postgres://u:p@faucet-db:5432/x", "URL", true).unwrap();
        assert_eq!(transport, PgTransport::PlaintextPrivateNetwork);
        assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
    }

    #[test]
    fn invalid_urls_are_not_echoed() {
        let error = connect_options("not a url with hunter2", "RELAYER_DATABASE_URL", true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("RELAYER_DATABASE_URL"));
        assert!(!error.contains("hunter2"));
    }

    /// Runs against a real PostgreSQL when `CREG_TEST_DATABASE_URL` is set
    /// (for example a throwaway `postgres:16-alpine` container).
    #[tokio::test]
    async fn pg_ledger_round_trip_and_single_writer() {
        let Ok(url) = std::env::var("CREG_TEST_DATABASE_URL") else {
            eprintln!("skipping: CREG_TEST_DATABASE_URL is not set");
            return;
        };
        let (pool, _) = connect_pool(&url, "CREG_TEST_DATABASE_URL", true, 4)
            .await
            .unwrap();
        ensure_schema(&pool).await.unwrap();
        ensure_schema(&pool).await.unwrap();
        let ledger = PgNonceLedger::new(pool.clone());

        let address = Address::repeat_byte(rand_byte());
        let key = SignerKey::new(11_155_111, address);
        let lease = ledger.acquire_writer(key).await.unwrap();
        assert!(ledger.acquire_writer(key).await.is_err());

        let now = Utc::now();
        let mut row = LedgerTx {
            key,
            request_id: "pg-test-1".to_string(),
            nonce: 7,
            state: TxState::Signed,
            to: Address::repeat_byte(0x11),
            value: U256::MAX,
            input: Bytes::from_static(&[0xca, 0xfe]),
            gas_limit: 90_000,
            attempts: vec![TxAttempt {
                tx_hash: B256::repeat_byte(0xab),
                max_fee_per_gas: u128::MAX,
                max_priority_fee_per_gas: 3,
                raw: Bytes::from_static(&[0x02, 0x01]),
            }],
            included_tx_hash: None,
            block_number: None,
            last_error: None,
            broadcast_at: None,
            created_at: now,
            updated_at: now,
        };
        ledger.insert(&row).await.unwrap();
        assert!(ledger.insert(&row).await.is_err(), "duplicate request id");

        let mut clash = row.clone();
        clash.request_id = "pg-test-2".to_string();
        assert!(ledger.insert(&clash).await.is_err(), "live nonce clash");

        row.state = TxState::Included;
        row.included_tx_hash = Some(B256::repeat_byte(0xab));
        row.block_number = Some(123);
        row.broadcast_at = Some(now);
        ledger.update(&row).await.unwrap();

        let back = ledger.get(key, "pg-test-1").await.unwrap().unwrap();
        assert_eq!(back.value, U256::MAX);
        assert_eq!(back.attempts, row.attempts);
        assert_eq!(back.state, TxState::Included);
        assert_eq!(back.block_number, Some(123));
        assert_eq!(ledger.highest_nonce(key).await.unwrap(), Some(7));
        assert_eq!(ledger.open(key).await.unwrap().len(), 1);

        drop(lease);
        // The detached session closes on drop, releasing the lock.
        let mut reacquired = None;
        for _ in 0..50 {
            if let Ok(lease) = ledger.acquire_writer(key).await {
                reacquired = Some(lease);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(reacquired.is_some(), "lease released after drop");

        sqlx::query("DELETE FROM l1_tx_ledger WHERE signer = $1")
            .bind(key.address_hex())
            .execute(&pool)
            .await
            .unwrap();
    }

    fn rand_byte() -> u8 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos())
            .unwrap_or(7);
        (nanos % 250) as u8 + 3
    }
}
