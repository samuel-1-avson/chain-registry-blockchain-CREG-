// crates/faucet/src/main.rs
// Testnet Faucet Service - Distributes test tCREG tokens (REAL IMPLEMENTATION)
#![deny(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
    signers::Signer,
    sol,
};
use anyhow::Context;
use axum::{
    extract::{ConnectInfo, Json, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json as JsonResponse},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use ipnet::IpNet;
use metrics::{counter, gauge};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_postgres::NoTls;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

sol!(
    #[sol(rpc)]
    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
        function balanceOf(address owner) external view returns (uint256);
    }
);

/// Faucet configuration
#[derive(Clone)]
struct FaucetConfig {
    /// Amount to distribute per request (in wei/tCREG smallest unit)
    drip_amount: u128,
    /// Amount of native ETH/testnet ETH to distribute per request (wei)
    native_drip_amount: u128,
    /// Cooldown between requests per address
    cooldown_secs: u64,
    /// Cooldown between requests per IP
    ip_cooldown_secs: u64,
    /// Maximum balance a single address can have (prevent hoarding)
    max_balance: u128,
    /// Maximum native balance a single address can have before gas drip stops
    native_max_balance: u128,
    /// Ethereum RPC URL
    rpc_url: String,
    /// Initial R7 treasury holder, governance reserve, and staking contract.
    /// Public supply reporting is unavailable until these are configured.
    treasury_address: String,
    governance_address: String,
    staking_address: String,
    reconciliation_rpc_url: String,
    deployment_receipt_rpc_url: String,
    reconciliation_indexer_url: String,
    /// Faucet private key (must have tokens to distribute)
    faucet_key: String,
    /// Test CREG token contract address
    token_contract: String,
    chain_id: u64,
    token_deployment_tx_hash: String,
    token_deployment_block: Option<u64>,
    reconciliation_confirmations: u64,
    /// Faucet Ethereum address
    faucet_address: String,
    /// Networks allowed to supply client IP headers. Configure only the edge
    /// proxy's actual source ranges; untrusted request headers are ignored.
    trusted_proxy_cidrs: Vec<IpNet>,
    /// Governance-approved tCREG allocation available to this faucet instance.
    /// These values are deliberately unset until policy is approved.
    token_allocation_wei: Option<u128>,
    token_daily_cap_wei: Option<u128>,
    token_address_lifetime_cap_wei: Option<u128>,
    token_hot_wallet_float_wei: Option<u128>,
    /// Explicit rollout gate. Configuring budgets must not open public claims.
    token_distribution_open: bool,
}

impl FaucetConfig {
    async fn from_env() -> anyhow::Result<(Self, chain_registry_secrets::SecretsProvider)> {
        let secrets = chain_registry_secrets::SecretsProvider::from_env()?;
        let faucet_key = secrets
            .secp256k1_signing_key_hex(chain_registry_secrets::HotKeyRole::Faucet)
            .await?;
        let faucet_address = std::env::var("FAUCET_ADDRESS").expect("FAUCET_ADDRESS must be set");
        let signer: PrivateKeySigner = faucet_key
            .parse()
            .map_err(|err| anyhow::anyhow!("Invalid configured faucet signing key: {err}"))?;
        let configured_address: Address = faucet_address
            .parse()
            .map_err(|err| anyhow::anyhow!("Invalid FAUCET_ADDRESS: {err}"))?;
        if signer.address() != configured_address {
            anyhow::bail!("FAUCET_ADDRESS does not match the configured faucet signing key");
        }

        Ok((
            Self {
                drip_amount: env_u128("FAUCET_DRIP_AMOUNT", 1000_000_000_000_000_000_000), // 1000 tCREG
                native_drip_amount: env_u128("FAUCET_NATIVE_DRIP_AMOUNT", 100_000_000_000_000_000), // 0.1 ETH
                cooldown_secs: env_u64("FAUCET_COOLDOWN_SECS", 60), // 1 minute
                ip_cooldown_secs: env_u64("FAUCET_IP_COOLDOWN_SECS", 60),
                max_balance: env_u128("FAUCET_MAX_BALANCE", 10000_000_000_000_000_000_000), // 10k tCREG
                native_max_balance: env_u128(
                    "FAUCET_NATIVE_MAX_BALANCE",
                    1_000_000_000_000_000_000,
                ), // 1 ETH
                rpc_url: env_string("FAUCET_RPC_URL", "http://localhost:8545"),
                treasury_address: env_string("FAUCET_TREASURY_ADDRESS", ""),
                governance_address: env_string("FAUCET_GOVERNANCE_ADDRESS", ""),
                staking_address: env_string("FAUCET_STAKING_ADDRESS", ""),
                reconciliation_rpc_url: env_string("FAUCET_RECONCILIATION_RPC_URL", ""),
                deployment_receipt_rpc_url: env_string("FAUCET_DEPLOYMENT_RECEIPT_RPC_URL", ""),
                reconciliation_indexer_url: env_string(
                    "FAUCET_RECONCILIATION_INDEXER_URL",
                    "https://api.routescan.io/v2/network/testnet/evm/11155111/etherscan/api",
                ),
                faucet_key,
                token_contract: std::env::var("FAUCET_TOKEN_CONTRACT")
                    .expect("FAUCET_TOKEN_CONTRACT must be set"),
                chain_id: env_u64("FAUCET_CHAIN_ID", 31337),
                token_deployment_tx_hash: env_string("FAUCET_TOKEN_DEPLOYMENT_TX_HASH", ""),
                token_deployment_block: env_optional_u64("FAUCET_TOKEN_DEPLOYMENT_BLOCK")?,
                reconciliation_confirmations: env_u64("FAUCET_RECONCILIATION_CONFIRMATIONS", 64),
                faucet_address,
                trusted_proxy_cidrs: env_ipnets("FAUCET_TRUSTED_PROXY_CIDRS")?,
                token_allocation_wei: env_optional_u128("FAUCET_TOKEN_ALLOCATION_WEI")?,
                token_daily_cap_wei: env_optional_u128("FAUCET_TOKEN_DAILY_CAP_WEI")?,
                token_address_lifetime_cap_wei: env_optional_u128(
                    "FAUCET_TOKEN_ADDRESS_LIFETIME_CAP_WEI",
                )?,
                token_hot_wallet_float_wei: env_optional_u128("FAUCET_TOKEN_HOT_WALLET_FLOAT_WEI")?,
                token_distribution_open: env_bool("FAUCET_TOKEN_DISTRIBUTION_OPEN", false)?,
            },
            secrets,
        ))
    }
}

impl FaucetConfig {
    fn token_claim_policy_ready(&self) -> bool {
        match (
            self.token_allocation_wei,
            self.token_daily_cap_wei,
            self.token_address_lifetime_cap_wei,
            self.token_hot_wallet_float_wei,
        ) {
            (Some(allocation), Some(daily), Some(address), Some(float)) => {
                self.drip_amount > 0
                    && allocation >= daily
                    && allocation >= float
                    && daily >= self.drip_amount
                    && address >= self.drip_amount
                    && float >= self.drip_amount
            }
            _ => false,
        }
    }

    fn history_rpc_url(&self) -> &str {
        if self.reconciliation_rpc_url.trim().is_empty() {
            &self.rpc_url
        } else {
            &self.reconciliation_rpc_url
        }
    }
}

struct CooldownRejection {
    message: String,
    retry_after_seconds: u64,
    cooldown_seconds: u64,
}

#[derive(Clone)]
struct ChainTransfer {
    tx_hash: String,
    log_index: i64,
    block_number: i64,
    block_hash: String,
    from_address: String,
    to_address: String,
    amount_wei: String,
    block_time: DateTime<Utc>,
}

// ── Postgres-backed persistent rate limiter ───────────────────────────────────

/// Atomic, durable address and IP cooldown claims. PostgreSQL is required so a
/// restart or a second faucet instance cannot bypass a cooldown.
#[derive(Clone)]
struct PersistentRateLimiter {
    pg: Arc<Mutex<tokio_postgres::Client>>,
}

impl PersistentRateLimiter {
    async fn new(pg_url: &str) -> anyhow::Result<Self> {
        if pg_url.trim().is_empty() {
            anyhow::bail!("FAUCET_PG_URL is required; refusing to start with volatile cooldowns");
        }

        let (mut client, connection) = tokio_postgres::connect(pg_url, NoTls)
            .await
            .context("connect to durable faucet cooldown database")?;
        tokio::spawn(async move {
            if let Err(err) = connection.await {
                error!("Postgres rate-limiter connection error: {}", err);
            }
        });

        client
            .execute(
                "CREATE TABLE IF NOT EXISTS faucet_rate_limits (\
                  key TEXT PRIMARY KEY, \
                  last_request_unix BIGINT NOT NULL\
                )",
                &[],
            )
            .await
            .context("initialize durable faucet cooldown table")?;

        client
            .execute(
                "CREATE TABLE IF NOT EXISTS faucet_claim_ledger (\
                  id BIGSERIAL PRIMARY KEY, \
                  recipient TEXT NOT NULL, \
                  token_amount_wei NUMERIC(39, 0) NOT NULL, \
                  native_amount_wei NUMERIC(39, 0) NOT NULL, \
                  token_status TEXT NOT NULL CHECK (token_status IN ('not_requested', 'reserved', 'submitted', 'confirmed', 'failed')), \
                  native_status TEXT NOT NULL CHECK (native_status IN ('not_requested', 'reserved', 'submitted', 'confirmed', 'failed')), \
                  token_tx_hash TEXT UNIQUE, \
                  native_tx_hash TEXT UNIQUE, \
                  created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(), \
                  token_confirmed_at TIMESTAMPTZ, \
                  native_confirmed_at TIMESTAMPTZ\
                )",
                &[],
            )
            .await
            .context("initialize durable faucet claim ledger")?;

        client
            .batch_execute(
                "ALTER TABLE faucet_claim_ledger ADD COLUMN IF NOT EXISTS token_block_number BIGINT;\
                 ALTER TABLE faucet_claim_ledger ADD COLUMN IF NOT EXISTS token_log_index BIGINT;\
                 ALTER TABLE faucet_claim_ledger ADD COLUMN IF NOT EXISTS native_block_number BIGINT;\
                 ALTER TABLE faucet_claim_ledger ADD COLUMN IF NOT EXISTS native_log_index BIGINT;\
                 ALTER TABLE faucet_claim_ledger DROP CONSTRAINT IF EXISTS faucet_claim_ledger_token_status_check;\
                 ALTER TABLE faucet_claim_ledger DROP CONSTRAINT IF EXISTS faucet_claim_ledger_native_status_check;\
                 DO $$ BEGIN \
                   IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'faucet_claim_ledger_token_status_check') THEN \
                     ALTER TABLE faucet_claim_ledger ADD CONSTRAINT faucet_claim_ledger_token_status_check \
                       CHECK (token_status IN ('not_requested', 'reserved', 'submitted', 'confirmed', 'failed'));\
                   END IF;\
                   IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'faucet_claim_ledger_native_status_check') THEN \
                     ALTER TABLE faucet_claim_ledger ADD CONSTRAINT faucet_claim_ledger_native_status_check \
                       CHECK (native_status IN ('not_requested', 'reserved', 'submitted', 'confirmed', 'failed'));\
                   END IF;\
                 END $$;\
                 CREATE TABLE IF NOT EXISTS faucet_token_transfers (\
                   chain_id BIGINT NOT NULL, token_address TEXT NOT NULL, tx_hash TEXT NOT NULL, log_index BIGINT NOT NULL,\
                   block_number BIGINT NOT NULL, block_hash TEXT NOT NULL,\
                   from_address TEXT NOT NULL, to_address TEXT NOT NULL,\
                   amount_wei NUMERIC(39, 0) NOT NULL, block_time TIMESTAMPTZ NOT NULL,\
                   PRIMARY KEY (chain_id, token_address, tx_hash, log_index)\
                 );\
                 CREATE TABLE IF NOT EXISTS faucet_funding_ledger (\
                   chain_id BIGINT NOT NULL, token_address TEXT NOT NULL, tx_hash TEXT NOT NULL, log_index BIGINT NOT NULL,\
                   block_number BIGINT NOT NULL, sender TEXT NOT NULL,\
                   amount_wei NUMERIC(39, 0) NOT NULL, block_time TIMESTAMPTZ NOT NULL,\
                   classification TEXT NOT NULL DEFAULT 'historical_unallocated',\
                   PRIMARY KEY (chain_id, token_address, tx_hash, log_index)\
                 );\
                 ALTER TABLE faucet_token_transfers ADD COLUMN IF NOT EXISTS token_address TEXT NOT NULL DEFAULT '';\
                 ALTER TABLE faucet_funding_ledger ADD COLUMN IF NOT EXISTS token_address TEXT NOT NULL DEFAULT '';\
                 CREATE TABLE IF NOT EXISTS faucet_reconciliation_state (\
                   chain_id BIGINT NOT NULL, token_address TEXT NOT NULL,\
                   deployment_tx_hash TEXT NOT NULL, deployment_block BIGINT NOT NULL,\
                   reconciled_through_block BIGINT NOT NULL, reconciled_through_hash TEXT NOT NULL,\
                   faucet_balance_wei NUMERIC(39, 0) NOT NULL, total_supply_wei NUMERIC(39, 0) NOT NULL,\
                   updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),\
                   PRIMARY KEY (chain_id, token_address)\
                 );",
            )
            .await
            .context("migrate durable faucet reconciliation schema")?;

        info!("Postgres durable faucet rate-limit and claim-ledger tables ready");
        Ok(Self {
            pg: Arc::new(Mutex::new(client)),
        })
    }

    /// Atomically claim both cooldown keys before sending funds. Advisory locks
    /// serialize claims across replicas, including when a key has no row yet.
    async fn claim(
        &self,
        address: &str,
        ip: &str,
        address_cooldown: Duration,
        ip_cooldown: Duration,
        token_amount_wei: u128,
        native_amount_wei: u128,
        token_allocation_wei: Option<u128>,
        token_daily_cap_wei: Option<u128>,
        token_address_lifetime_cap_wei: Option<u128>,
    ) -> anyhow::Result<Result<i64, CooldownRejection>> {
        let addr_key = format!("addr:{}", address.to_lowercase());
        let ip_key = format!("ip:{}", ip);
        let mut claims = vec![
            (addr_key, address_cooldown, "Please wait"),
            (ip_key, ip_cooldown, "IP rate limit: wait"),
        ];
        claims.sort_by(|left, right| left.0.cmp(&right.0));

        let mut client = self.pg.lock().await;
        let tx = client
            .transaction()
            .await
            .context("begin durable faucet claim")?;
        let mut lock_keys = claims
            .iter()
            .map(|(key, _, _)| key.to_string())
            .collect::<Vec<_>>();
        if token_amount_wei > 0 {
            lock_keys.extend([
                "budget:token:allocation".to_string(),
                "budget:token:daily".to_string(),
                format!("budget:token:address:{}", address.to_lowercase()),
            ]);
        }
        lock_keys.sort();
        lock_keys.dedup();
        for key in &lock_keys {
            tx.query_one(
                "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
                &[key],
            )
            .await
            .context("lock faucet cooldown or budget key")?;
        }

        let now_unix: i64 = tx
            .query_one(
                "SELECT floor(extract(epoch FROM clock_timestamp()))::bigint",
                &[],
            )
            .await
            .context("read cooldown database clock")?
            .get(0);

        for (key, cooldown, message_prefix) in &claims {
            if let Some(row) = tx
                .query_opt(
                    "SELECT last_request_unix FROM faucet_rate_limits WHERE key = $1",
                    &[key],
                )
                .await
                .context("read faucet cooldown")?
            {
                let last_unix: i64 = row.get(0);
                let elapsed = now_unix.saturating_sub(last_unix).max(0) as u64;
                let cooldown_secs = cooldown.as_secs();
                if elapsed < cooldown_secs {
                    let remaining = cooldown_secs - elapsed;
                    tx.rollback()
                        .await
                        .context("release rejected faucet cooldown claim")?;
                    return Ok(Err(CooldownRejection {
                        message: format!("{} {} seconds", message_prefix, remaining),
                        retry_after_seconds: remaining,
                        cooldown_seconds: cooldown_secs,
                    }));
                }
            }
        }

        if token_amount_wei > 0 {
            let unresolved = tx
                .query_opt(
                    "SELECT id FROM faucet_claim_ledger \
                     WHERE recipient = $1 AND token_status IN ('reserved', 'submitted') \
                     ORDER BY id LIMIT 1",
                    &[&address.to_lowercase()],
                )
                .await
                .context("check for unresolved recipient token claim")?;
            if unresolved.is_some() {
                tx.rollback()
                    .await
                    .context("release claim blocked by unresolved prior token transfer")?;
                return Ok(Err(CooldownRejection {
                    message: "A previous token claim for this address is still being reconciled. No additional token transfer was sent.".to_string(),
                    retry_after_seconds: 3600,
                    cooldown_seconds: 3600,
                }));
            }

            let (Some(allocation_cap), Some(daily_cap), Some(address_cap)) = (
                token_allocation_wei,
                token_daily_cap_wei,
                token_address_lifetime_cap_wei,
            ) else {
                tx.rollback()
                    .await
                    .context("release claim with incomplete faucet token policy")?;
                return Ok(Err(CooldownRejection {
                    message: "Token faucet policy is not fully configured; no funds were sent."
                        .to_string(),
                    retry_after_seconds: 3600,
                    cooldown_seconds: 3600,
                }));
            };

            let allocation_used = tx
                .query_one(
                    "SELECT COALESCE(SUM(token_amount_wei), 0)::text \
                     FROM faucet_claim_ledger \
                     WHERE token_status IN ('reserved', 'submitted', 'confirmed')",
                    &[],
                )
                .await
                .context("read durable faucet allocation usage")?
                .get::<_, String>(0)
                .parse::<u128>()
                .context("parse durable faucet allocation usage")?;
            let daily_used = tx
                .query_one(
                    "SELECT COALESCE(SUM(token_amount_wei), 0)::text \
                     FROM faucet_claim_ledger \
                     WHERE token_status IN ('reserved', 'submitted', 'confirmed') \
                       AND created_at >= (date_trunc('day', now() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC')",
                    &[],
                )
                .await
                .context("read durable daily faucet usage")?
                .get::<_, String>(0)
                .parse::<u128>()
                .context("parse durable daily faucet usage")?;
            let address_used = tx
                .query_one(
                    "SELECT COALESCE(SUM(token_amount_wei), 0)::text \
                     FROM faucet_claim_ledger \
                     WHERE recipient = $1 AND token_status IN ('reserved', 'submitted', 'confirmed')",
                    &[&address.to_lowercase()],
                )
                .await
                .context("read durable recipient faucet usage")?
                .get::<_, String>(0)
                .parse::<u128>()
                .context("parse durable recipient faucet usage")?;

            let within_allocation = allocation_used
                .checked_add(token_amount_wei)
                .is_some_and(|used| used <= allocation_cap);
            let within_daily_cap = daily_used
                .checked_add(token_amount_wei)
                .is_some_and(|used| used <= daily_cap);
            let within_address_cap = address_used
                .checked_add(token_amount_wei)
                .is_some_and(|used| used <= address_cap);
            if !within_allocation || !within_daily_cap || !within_address_cap {
                tx.rollback()
                    .await
                    .context("release claim that exceeds faucet token budget")?;
                let message = if !within_allocation {
                    "The approved faucet token allocation is exhausted."
                } else if !within_daily_cap {
                    "The faucet daily tCREG distribution cap has been reached."
                } else {
                    "This address has reached its lifetime tCREG faucet cap."
                };
                return Ok(Err(CooldownRejection {
                    message: message.to_string(),
                    retry_after_seconds: 86_400,
                    cooldown_seconds: 86_400,
                }));
            }
        }

        let token_status = if token_amount_wei > 0 {
            "reserved"
        } else {
            "not_requested"
        };
        let native_status = if native_amount_wei > 0 {
            "reserved"
        } else {
            "not_requested"
        };
        let token_amount_text = token_amount_wei.to_string();
        let native_amount_text = native_amount_wei.to_string();
        let claim_id: i64 = tx
            .query_one(
                "INSERT INTO faucet_claim_ledger \
                   (recipient, token_amount_wei, native_amount_wei, token_status, native_status) \
                 VALUES ($1, $2::text::numeric, $3::text::numeric, $4, $5) RETURNING id",
                &[
                    &address.to_lowercase(),
                    &token_amount_text,
                    &native_amount_text,
                    &token_status,
                    &native_status,
                ],
            )
            .await
            .context("reserve durable faucet claim")?
            .get(0);

        for (key, _, _) in &claims {
            tx.execute(
                "INSERT INTO faucet_rate_limits (key, last_request_unix) VALUES ($1, $2) \
                 ON CONFLICT (key) DO UPDATE SET last_request_unix = EXCLUDED.last_request_unix",
                &[key, &now_unix],
            )
            .await
            .context("persist faucet cooldown claim")?;
        }
        tx.commit().await.context("commit durable faucet claim")?;
        Ok(Ok(claim_id))
    }

    async fn mark_token_submitted(&self, claim_id: i64, tx_hash: &str) -> anyhow::Result<()> {
        self.pg
            .lock()
            .await
            .execute(
                "UPDATE faucet_claim_ledger \
                 SET token_status = 'submitted', token_tx_hash = $2 \
                 WHERE id = $1 AND token_status = 'reserved'",
                &[&claim_id, &tx_hash],
            )
            .await
            .context("journal submitted tCREG faucet transfer")?;
        Ok(())
    }

    async fn confirm_token_transfer(&self, claim_id: i64, tx_hash: &str) -> anyhow::Result<()> {
        self.pg
            .lock()
            .await
            .execute(
                "UPDATE faucet_claim_ledger \
                 SET token_status = 'confirmed', token_tx_hash = $2, token_confirmed_at = clock_timestamp() \
                 WHERE id = $1 AND token_status IN ('reserved', 'submitted') AND (token_tx_hash IS NULL OR token_tx_hash = $2)",
                &[&claim_id, &tx_hash],
            )
            .await
            .context("record confirmed tCREG faucet transfer")?;
        Ok(())
    }

    async fn mark_native_submitted(&self, claim_id: i64, tx_hash: &str) -> anyhow::Result<()> {
        self.pg
            .lock()
            .await
            .execute(
                "UPDATE faucet_claim_ledger \
                 SET native_status = 'submitted', native_tx_hash = $2 \
                 WHERE id = $1 AND native_status = 'reserved'",
                &[&claim_id, &tx_hash],
            )
            .await
            .context("journal submitted native faucet transfer")?;
        Ok(())
    }

    async fn confirm_native_transfer(&self, claim_id: i64, tx_hash: &str) -> anyhow::Result<()> {
        self.pg
            .lock()
            .await
            .execute(
                "UPDATE faucet_claim_ledger \
                 SET native_status = 'confirmed', native_tx_hash = $2, native_confirmed_at = clock_timestamp() \
                 WHERE id = $1 AND native_status IN ('reserved', 'submitted') AND (native_tx_hash IS NULL OR native_tx_hash = $2)",
                &[&claim_id, &tx_hash],
            )
            .await
            .context("record confirmed native faucet transfer")?;
        Ok(())
    }

    async fn mark_transfer_failed(
        &self,
        claim_id: i64,
        is_token: bool,
        tx_hash: &str,
    ) -> anyhow::Result<()> {
        let column = if is_token { "token" } else { "native" };
        let status_column = format!("{column}_status");
        let hash_column = format!("{column}_tx_hash");
        let sql = format!(
            "UPDATE faucet_claim_ledger SET {status_column} = 'failed' \
             WHERE id = $1 AND {status_column} IN ('reserved', 'submitted') \
               AND ({hash_column} IS NULL OR {hash_column} = $2)"
        );
        self.pg
            .lock()
            .await
            .execute(&sql, &[&claim_id, &tx_hash])
            .await
            .context("record reverted faucet transfer")?;
        Ok(())
    }

    async fn token_claims_have_no_unresolved_rows(&self) -> anyhow::Result<bool> {
        let row = self
            .pg
            .lock()
            .await
            .query_one(
                "SELECT NOT EXISTS (SELECT 1 FROM faucet_claim_ledger \
                   WHERE token_status IN ('reserved', 'submitted') OR native_status IN ('reserved', 'submitted'))",
                &[],
            )
            .await
            .context("check unresolved token claim ledger rows")?;
        Ok(row.get(0))
    }

    async fn reconciliation_cursor(
        &self,
        chain_id: i64,
        token_address: &str,
        deployment_tx_hash: &str,
        deployment_block: i64,
    ) -> anyhow::Result<Option<(i64, String)>> {
        let row = self
            .pg
            .lock()
            .await
            .query_opt(
                "SELECT reconciled_through_block, reconciled_through_hash \
                 FROM faucet_reconciliation_state \
                 WHERE chain_id = $1 AND token_address = $2 \
                   AND deployment_tx_hash = $3 AND deployment_block = $4",
                &[
                    &chain_id,
                    &token_address,
                    &deployment_tx_hash,
                    &deployment_block,
                ],
            )
            .await
            .context("read faucet reconciliation cursor")?;
        Ok(row.map(|row| (row.get(0), row.get(1))))
    }

    async fn reconciled_supply_snapshot(
        &self,
        chain_id: i64,
        token_address: &str,
    ) -> anyhow::Result<Option<(i64, String, u128, u128, String)>> {
        let row = self
            .pg
            .lock()
            .await
            .query_opt(
                "SELECT reconciled_through_block, reconciled_through_hash, \
                   faucet_balance_wei::text, total_supply_wei::text, updated_at::text \
                 FROM faucet_reconciliation_state WHERE chain_id = $1 AND token_address = $2",
                &[&chain_id, &token_address],
            )
            .await
            .context("read verified faucet supply snapshot")?;
        row.map(|row| {
            let balance: String = row.get(2);
            let supply: String = row.get(3);
            Ok((
                row.get(0),
                row.get(1),
                balance
                    .parse::<u128>()
                    .context("invalid stored faucet balance")?,
                supply
                    .parse::<u128>()
                    .context("invalid stored total supply")?,
                row.get(4),
            ))
        })
        .transpose()
    }

    async fn apply_reconciled_transfers(
        &self,
        chain_id: i64,
        token_address: &str,
        deployment_tx_hash: &str,
        deployment_block: i64,
        through_block: i64,
        through_hash: &str,
        faucet_balance_wei: u128,
        total_supply_wei: u128,
        faucet_address: &str,
        transfers: &[ChainTransfer],
    ) -> anyhow::Result<bool> {
        let mut client = self.pg.lock().await;
        let tx = client
            .transaction()
            .await
            .context("begin atomic on-chain faucet history import")?;
        for transfer in transfers {
            tx.execute(
                "INSERT INTO faucet_token_transfers \
                   (chain_id, token_address, tx_hash, log_index, block_number, block_hash, from_address, to_address, amount_wei, block_time) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::text::numeric, $10) \
                 ON CONFLICT (chain_id, token_address, tx_hash, log_index) DO UPDATE SET \
                   block_number = EXCLUDED.block_number, block_hash = EXCLUDED.block_hash, \
                   from_address = EXCLUDED.from_address, to_address = EXCLUDED.to_address, \
                   amount_wei = EXCLUDED.amount_wei, block_time = EXCLUDED.block_time",
                &[
                    &chain_id,
                    &token_address,
                    &transfer.tx_hash,
                    &transfer.log_index,
                    &transfer.block_number,
                    &transfer.block_hash,
                    &transfer.from_address,
                    &transfer.to_address,
                    &transfer.amount_wei,
                    &transfer.block_time,
                ],
            )
            .await
            .context("persist canonical faucet token transfer event")?;

            if transfer.to_address == faucet_address {
                tx.execute(
                "INSERT INTO faucet_funding_ledger \
                       (chain_id, token_address, tx_hash, log_index, block_number, sender, amount_wei, block_time) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7::text::numeric, $8) \
                     ON CONFLICT (chain_id, token_address, tx_hash, log_index) DO UPDATE SET \
                       block_number = EXCLUDED.block_number, sender = EXCLUDED.sender, \
                       amount_wei = EXCLUDED.amount_wei, block_time = EXCLUDED.block_time",
                    &[
                        &chain_id,
                        &token_address,
                        &transfer.tx_hash,
                        &transfer.log_index,
                        &transfer.block_number,
                        &transfer.from_address,
                        &transfer.amount_wei,
                        &transfer.block_time,
                    ],
                )
                .await
                .context("record historical faucet funding as unallocated")?;
            }

            if transfer.from_address == faucet_address {
                if let Some(existing) = tx
                    .query_opt(
                        "SELECT recipient, token_amount_wei::text FROM faucet_claim_ledger WHERE token_tx_hash = $1",
                        &[&transfer.tx_hash],
                    )
                    .await
                    .context("check imported transfer against existing claim")?
                {
                    let recipient: String = existing.get(0);
                    let amount: String = existing.get(1);
                    if recipient != transfer.to_address || amount != transfer.amount_wei {
                        anyhow::bail!(
                            "claim ledger transaction {} conflicts with its canonical Transfer event",
                            transfer.tx_hash
                        );
                    }
                    tx.execute(
                        "UPDATE faucet_claim_ledger SET token_status = 'confirmed', \
                           token_confirmed_at = $2, token_block_number = $3, token_log_index = $4 \
                         WHERE token_tx_hash = $1",
                        &[
                            &transfer.tx_hash,
                            &transfer.block_time,
                            &transfer.block_number,
                            &transfer.log_index,
                        ],
                    )
                    .await
                    .context("refresh canonical token claim confirmation")?;
                } else {
                    let matched = tx
                        .query_opt(
                            "WITH candidate AS (\
                               SELECT id FROM faucet_claim_ledger \
                               WHERE recipient = $1 AND token_amount_wei = $2::text::numeric \
                                 AND token_status IN ('reserved', 'submitted') \
                                 AND (token_tx_hash IS NULL OR token_tx_hash = $3) \
                                 AND created_at <= $4::timestamptz + interval '5 minutes' \
                               ORDER BY created_at, id LIMIT 1 FOR UPDATE SKIP LOCKED\
                             ) \
                             UPDATE faucet_claim_ledger AS claim SET \
                               token_status = 'confirmed', token_tx_hash = $3, \
                               token_confirmed_at = $4, token_block_number = $5, token_log_index = $6 \
                             FROM candidate WHERE claim.id = candidate.id RETURNING claim.id",
                            &[
                                &transfer.to_address,
                                &transfer.amount_wei,
                                &transfer.tx_hash,
                                &transfer.block_time,
                                &transfer.block_number,
                                &transfer.log_index,
                            ],
                        )
                        .await
                        .context("recover a claim whose broadcast hash was not journaled")?;
                    if matched.is_none() {
                        tx.execute(
                            "INSERT INTO faucet_claim_ledger \
                               (recipient, token_amount_wei, native_amount_wei, token_status, native_status, \
                                token_tx_hash, created_at, token_confirmed_at, token_block_number, token_log_index) \
                             VALUES ($1, $2::text::numeric, 0, 'confirmed', 'not_requested', $3, $4, $4, $5, $6) \
                             ON CONFLICT (token_tx_hash) DO NOTHING",
                            &[
                                &transfer.to_address,
                                &transfer.amount_wei,
                                &transfer.tx_hash,
                                &transfer.block_time,
                                &transfer.block_number,
                                &transfer.log_index,
                            ],
                        )
                        .await
                        .context("import historical faucet claim")?;
                    }
                }
            }
        }

        let net_text: String = tx
            .query_one(
                "SELECT (\
                   COALESCE(SUM(amount_wei) FILTER (WHERE to_address = $1), 0) - \
                   COALESCE(SUM(amount_wei) FILTER (WHERE from_address = $1), 0)\
                 )::text FROM faucet_token_transfers WHERE chain_id = $2 AND token_address = $3",
                &[&faucet_address, &chain_id, &token_address],
            )
            .await
            .context("sum imported faucet Transfer history")?
            .get(0);
        let net_balance = net_text
            .parse::<u128>()
            .context("parse imported faucet Transfer balance")?;
        if net_balance != faucet_balance_wei {
            anyhow::bail!(
                "on-chain reconciliation mismatch: Transfer history gives {} wei, balanceOf gives {} wei",
                net_balance,
                faucet_balance_wei
            );
        }

        tx.execute(
            "INSERT INTO faucet_reconciliation_state \
               (chain_id, token_address, deployment_tx_hash, deployment_block, \
                reconciled_through_block, reconciled_through_hash, faucet_balance_wei, total_supply_wei, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::text::numeric, $8::text::numeric, clock_timestamp()) \
             ON CONFLICT (chain_id, token_address) DO UPDATE SET \
               deployment_tx_hash = EXCLUDED.deployment_tx_hash, deployment_block = EXCLUDED.deployment_block, \
               reconciled_through_block = EXCLUDED.reconciled_through_block, \
               reconciled_through_hash = EXCLUDED.reconciled_through_hash, \
               faucet_balance_wei = EXCLUDED.faucet_balance_wei, \
               total_supply_wei = EXCLUDED.total_supply_wei, updated_at = clock_timestamp()",
            &[
                &chain_id,
                &token_address,
                &deployment_tx_hash,
                &deployment_block,
                &through_block,
                &through_hash,
                &faucet_balance_wei.to_string(),
                &total_supply_wei.to_string(),
            ],
        )
        .await
        .context("persist verified faucet reconciliation watermark")?;
        tx.commit()
            .await
            .context("commit verified on-chain faucet history")?;

        // The unresolved-claim check acquires `self.pg` again. Release the
        // single-connection guard first to avoid self-deadlocking here.
        drop(client);
        self.token_claims_have_no_unresolved_rows().await
    }

    async fn submitted_token_claims(&self) -> anyhow::Result<Vec<(i64, String, String, String)>> {
        let rows = self
            .pg
            .lock()
            .await
            .query(
                "SELECT id, token_tx_hash, recipient, token_amount_wei::text \
                 FROM faucet_claim_ledger WHERE token_status = 'submitted' AND token_tx_hash IS NOT NULL",
                &[],
            )
            .await
            .context("load submitted token transfers for receipt recovery")?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
            .collect())
    }

    async fn submitted_native_claims(&self) -> anyhow::Result<Vec<(i64, String, String, String)>> {
        let rows = self
            .pg
            .lock()
            .await
            .query(
                "SELECT id, native_tx_hash, recipient, native_amount_wei::text \
                 FROM faucet_claim_ledger WHERE native_status = 'submitted' AND native_tx_hash IS NOT NULL",
                &[],
            )
            .await
            .context("load submitted native transfers for receipt recovery")?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
            .collect())
    }

    async fn unresolved_claim_details(&self) -> anyhow::Result<Vec<serde_json::Value>> {
        let rows = self
            .pg
            .lock()
            .await
            .query(
                "SELECT id, recipient, token_amount_wei::text, token_status, token_tx_hash, \
                        native_amount_wei::text, native_status, native_tx_hash, created_at \
                 FROM faucet_claim_ledger \
                 WHERE token_status IN ('reserved', 'submitted') \
                    OR native_status IN ('reserved', 'submitted') \
                 ORDER BY created_at, id LIMIT 500",
                &[],
            )
            .await
            .context("read unresolved faucet claim details")?;
        Ok(rows
            .into_iter()
            .map(|row| {
                serde_json::json!({
                    "claim_id": row.get::<_, i64>(0),
                    "recipient": row.get::<_, String>(1),
                    "token_amount_wei": row.get::<_, String>(2),
                    "token_status": row.get::<_, String>(3),
                    "token_tx_hash": row.get::<_, Option<String>>(4),
                    "native_amount_wei": row.get::<_, String>(5),
                    "native_status": row.get::<_, String>(6),
                    "native_tx_hash": row.get::<_, Option<String>>(7),
                    "created_at": row.get::<_, DateTime<Utc>>(8),
                })
            })
            .collect())
    }

    async fn confirmed_stats(&self) -> anyhow::Result<FaucetStats> {
        let client = self.pg.lock().await;
        let row = client
            .query_one(
                "SELECT \
                   COUNT(*) FILTER (WHERE token_status = 'confirmed' OR native_status = 'confirmed'), \
                   COALESCE(SUM(token_amount_wei) FILTER (WHERE token_status = 'confirmed'), 0)::text, \
                   COALESCE(SUM(native_amount_wei) FILTER (WHERE native_status = 'confirmed'), 0)::text, \
                   COUNT(DISTINCT recipient) FILTER (WHERE token_status = 'confirmed' OR native_status = 'confirmed'), \
                   MAX(GREATEST(token_confirmed_at, native_confirmed_at)) \
                     FILTER (WHERE token_status = 'confirmed' OR native_status = 'confirmed'), \
                   COUNT(*) FILTER (WHERE token_status IN ('reserved', 'submitted') OR native_status IN ('reserved', 'submitted')) \
                 FROM faucet_claim_ledger",
                &[],
            )
            .await
            .context("read durable confirmed faucet statistics")?;
        let token_distributed = row
            .get::<_, String>(1)
            .parse::<u128>()
            .context("parse durable confirmed tCREG total")?;
        let native_distributed = row
            .get::<_, String>(2)
            .parse::<u128>()
            .context("parse durable confirmed native total")?;
        let last_drip = row.get::<_, Option<DateTime<Utc>>>(4);
        Ok(FaucetStats {
            total_drips: row.get::<_, i64>(0).max(0) as u64,
            total_distributed: token_distributed,
            total_native_distributed: native_distributed,
            unique_addresses: row.get::<_, i64>(3).max(0) as usize,
            last_drip,
            unresolved_claims: row.get::<_, i64>(5).max(0) as u64,
        })
    }

    async fn address_count(&self) -> usize {
        let client = self.pg.lock().await;
        match client
            .query_one(
                "SELECT COUNT(*) FROM faucet_rate_limits WHERE key LIKE 'addr:%'",
                &[],
            )
            .await
        {
            Ok(row) => row.get::<_, i64>(0).max(0) as usize,
            Err(err) => {
                warn!("Could not read faucet address count: {}", err);
                0
            }
        }
    }

    async fn check_ready(&self) -> anyhow::Result<()> {
        self.pg
            .lock()
            .await
            .simple_query("SELECT 1")
            .await
            .context("durable faucet cooldown database is unavailable")?;
        Ok(())
    }
}

const ERC20_TRANSFER_TOPIC: &str =
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const SEPOLIA_CHAIN_ID: u64 = 11_155_111;
const MAX_TESTNET_TOKEN_SUPPLY_WEI: u128 = 42_000_000_000_000_000_000_000_000;

async fn routescan_rpc_request(
    client: &reqwest::Client,
    base_url: &str,
    method: &str,
    params: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let mut url = reqwest::Url::parse(base_url).context("invalid Routescan RPC URL")?;
    if url.scheme() != "https"
        || url.host_str() != Some("api.routescan.io")
        || url.path_segments().is_none()
    {
        anyhow::bail!("Routescan read endpoint must use the official HTTPS API host");
    }
    let segments = url
        .path_segments()
        .context("Routescan RPC URL has no path")?
        .collect::<Vec<_>>();
    if segments.len() != 7
        || segments[0] != "v2"
        || segments[1] != "network"
        || segments[2] != "testnet"
        || segments[3] != "evm"
        || segments[5] != "etherscan"
        || segments[6] != "api"
    {
        anyhow::bail!("Routescan read endpoint must identify an EVM testnet chain");
    }
    let chain_id = segments[4]
        .parse::<u64>()
        .context("Routescan endpoint chain ID is invalid")?;
    if chain_id != SEPOLIA_CHAIN_ID {
        anyhow::bail!("faucet reconciliation Routescan endpoint must target Sepolia");
    }

    let rpc_params = params
        .as_array()
        .context("Routescan RPC parameters must be an array")?;
    let text_param = |index: usize, label: &str| -> anyhow::Result<&str> {
        rpc_params
            .get(index)
            .and_then(serde_json::Value::as_str)
            .with_context(|| format!("{method} requires {label}"))
    };
    if method == "eth_chainId" {
        return Ok(serde_json::Value::String(format!("0x{chain_id:x}")));
    }
    {
        let mut query = url.query_pairs_mut();
        match method {
            "eth_blockNumber" => {
                query
                    .append_pair("module", "proxy")
                    .append_pair("action", method);
            }
            "eth_getBlockByNumber" => {
                let tag = text_param(0, "a block tag")?;
                let full_transactions = rpc_params
                    .get(1)
                    .and_then(serde_json::Value::as_bool)
                    .context("eth_getBlockByNumber requires a boolean transaction flag")?;
                query
                    .append_pair("module", "proxy")
                    .append_pair("action", method)
                    .append_pair("tag", tag)
                    .append_pair("boolean", if full_transactions { "true" } else { "false" });
            }
            "eth_getCode" => {
                query
                    .append_pair("module", "proxy")
                    .append_pair("action", method)
                    .append_pair("address", text_param(0, "an address")?)
                    .append_pair("tag", text_param(1, "a block tag")?);
            }
            "eth_getTransactionReceipt" => {
                query
                    .append_pair("module", "proxy")
                    .append_pair("action", method)
                    .append_pair("txhash", text_param(0, "a transaction hash")?);
            }
            "eth_call" => {
                let call = rpc_params
                    .first()
                    .and_then(serde_json::Value::as_object)
                    .context("eth_call requires a call object")?;
                let to = call
                    .get("to")
                    .and_then(serde_json::Value::as_str)
                    .context("eth_call requires a target address")?;
                let data = call
                    .get("data")
                    .and_then(serde_json::Value::as_str)
                    .context("eth_call requires call data")?;
                let tag = text_param(1, "a block tag")?;
                let block_number = tag
                    .strip_prefix("0x")
                    .and_then(|hex| u64::from_str_radix(hex, 16).ok())
                    .context("Routescan historical eth_call requires an exact block number")?;
                let selector = data.get(..10).unwrap_or_default().to_ascii_lowercase();
                if selector == "0x18160ddd" && data.len() == 10 {
                    query
                        .append_pair("module", "stats")
                        .append_pair("action", "tokensupplyhistory")
                        .append_pair("contractaddress", to)
                        .append_pair("blockno", &block_number.to_string());
                } else if selector == "0x70a08231"
                    && data.len() == 74
                    && data[10..].bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    let holder = format!("0x{}", &data[data.len() - 40..]);
                    query
                        .append_pair("module", "account")
                        .append_pair("action", "tokenbalancehistory")
                        .append_pair("contractaddress", to)
                        .append_pair("address", &holder)
                        .append_pair("blockno", &block_number.to_string());
                } else {
                    anyhow::bail!(
                        "Routescan history supports only ERC-20 balanceOf and totalSupply calls"
                    );
                }
            }
            _ => anyhow::bail!("Routescan read endpoint does not support {method}"),
        }
    }

    // Stay under the keyless provider's published request rate.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let body: serde_json::Value = client
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            "creg-faucet-ledger-reconciler/1",
        )
        .send()
        .await
        .with_context(|| format!("send {method} request to Routescan"))?
        .error_for_status()
        .with_context(|| format!("Routescan returned an HTTP error for {method}"))?
        .json()
        .await
        .with_context(|| format!("decode Routescan {method} response"))?;
    if let Some(error) = body.get("error") {
        anyhow::bail!("Routescan rejected {method} request: {error}");
    }
    if body.get("status").and_then(serde_json::Value::as_str) == Some("0") {
        anyhow::bail!("Routescan historical state is unavailable for {method}");
    }
    let result = body
        .get("result")
        .cloned()
        .with_context(|| format!("Routescan {method} response has no result"))?;
    if result
        .as_str()
        .is_some_and(|value| value.starts_with("Error!") || value == "Invalid querystring request")
    {
        anyhow::bail!("Routescan rejected the query for {method}");
    }
    if method == "eth_call" {
        let decimal = result
            .as_str()
            .context("Routescan historical contract state is not a decimal string")?
            .parse::<u128>()
            .context("Routescan historical contract state is not a valid uint128")?;
        return Ok(serde_json::Value::String(format!("0x{decimal:x}")));
    }
    if let Some(encoded) = result.as_str() {
        if encoded.trim_start().starts_with('{') {
            return serde_json::from_str(encoded)
                .with_context(|| format!("decode Routescan {method} JSON result"));
        }
    }
    Ok(result)
}

async fn rpc_request(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    if rpc_url.contains("api.routescan.io/v2/network/") && rpc_url.ends_with("/etherscan/api") {
        return routescan_rpc_request(client, rpc_url, method, &params).await;
    }
    let response = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .with_context(|| format!("send {method} RPC request"))?
        .error_for_status()
        .with_context(|| format!("{method} RPC returned an HTTP error"))?;
    let body: serde_json::Value = response
        .json()
        .await
        .with_context(|| format!("decode {method} RPC response"))?;
    if let Some(error) = body.get("error") {
        anyhow::bail!("{method} RPC error: {error}");
    }
    body.get("result")
        .cloned()
        .with_context(|| format!("{method} RPC response has no result"))
}

async fn rpc_request_with_receipt_fallback(
    client: &reqwest::Client,
    config: &FaucetConfig,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let mut endpoints = Vec::new();
    for endpoint in [
        config.history_rpc_url(),
        config.rpc_url.as_str(),
        config.deployment_receipt_rpc_url.as_str(),
    ] {
        let endpoint = endpoint.trim();
        if !endpoint.is_empty() && !endpoints.contains(&endpoint) {
            endpoints.push(endpoint);
        }
    }
    let mut failures = Vec::new();
    for (index, endpoint) in endpoints.into_iter().enumerate() {
        match rpc_request(client, endpoint, method, params.clone()).await {
            Ok(result) if !result.is_null() => return Ok(result),
            Ok(_) => failures.push(format!("provider {} returned no result", index + 1)),
            Err(_) => failures.push(format!("provider {} request failed", index + 1)),
        }
    }
    anyhow::bail!(
        "{method} was unavailable from all configured receipt RPC providers ({})",
        failures.join(", ")
    )
}

fn parse_rpc_hex_u64(value: &serde_json::Value, field: &str) -> anyhow::Result<u64> {
    let text = value
        .as_str()
        .with_context(|| format!("{field} is not a hexadecimal string"))?;
    let digits = text.strip_prefix("0x").unwrap_or(text);
    u64::from_str_radix(digits, 16).with_context(|| format!("parse {field}"))
}

fn parse_rpc_hex_u128(value: &serde_json::Value, field: &str) -> anyhow::Result<u128> {
    let text = value
        .as_str()
        .with_context(|| format!("{field} is not a hexadecimal string"))?;
    let digits = text.strip_prefix("0x").unwrap_or(text);
    if digits.is_empty() {
        return Ok(0);
    }
    u128::from_str_radix(digits, 16).with_context(|| format!("parse {field}"))
}

fn topic_address(value: &str) -> anyhow::Result<String> {
    let topic = value
        .strip_prefix("0x")
        .with_context(|| format!("invalid indexed address topic {value}"))?;
    if topic.len() != 64 {
        anyhow::bail!("indexed address topic has {} hex digits", topic.len());
    }
    Ok(format!("0x{}", &topic[24..]).to_ascii_lowercase())
}

fn indexed_address_topic(address: &str) -> anyhow::Result<String> {
    let address = address
        .strip_prefix("0x")
        .unwrap_or(address)
        .to_ascii_lowercase();
    if address.len() != 40 || !address.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("invalid configured faucet address for log filter");
    }
    Ok(format!("0x{:0>64}", address))
}

fn decode_transfer_log(
    log: &serde_json::Value,
    token_address: &str,
    block_time: DateTime<Utc>,
) -> anyhow::Result<Option<ChainTransfer>> {
    if log.get("removed").and_then(serde_json::Value::as_bool) == Some(true) {
        anyhow::bail!("RPC returned a removed Transfer log in finalized history");
    }
    if log
        .get("address")
        .and_then(serde_json::Value::as_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
        != Some(token_address)
    {
        return Ok(None);
    }
    let topics = log
        .get("topics")
        .and_then(serde_json::Value::as_array)
        .context("Transfer log is missing topics")?;
    if topics.len() != 3
        || topics[0].as_str().map(str::to_ascii_lowercase).as_deref() != Some(ERC20_TRANSFER_TOPIC)
    {
        return Ok(None);
    }
    let from_address = topic_address(
        topics[1]
            .as_str()
            .context("Transfer log has an invalid from topic")?,
    )?;
    let to_address = topic_address(
        topics[2]
            .as_str()
            .context("Transfer log has an invalid to topic")?,
    )?;
    let data = log
        .get("data")
        .and_then(serde_json::Value::as_str)
        .context("Transfer log has no amount data")?;
    let amount_wei = parse_rpc_hex_u128(
        &serde_json::Value::String(data.to_string()),
        "Transfer value",
    )?;
    let tx_hash = log
        .get("transactionHash")
        .and_then(serde_json::Value::as_str)
        .context("Transfer log has no transaction hash")?
        .to_ascii_lowercase();
    let block_hash = log
        .get("blockHash")
        .and_then(serde_json::Value::as_str)
        .context("Transfer log has no block hash")?
        .to_ascii_lowercase();
    let block_number = parse_rpc_hex_u64(
        log.get("blockNumber")
            .context("Transfer log has no block number")?,
        "Transfer block number",
    )?;
    let log_index = parse_rpc_hex_u64(
        log.get("logIndex")
            .context("Transfer log has no log index")?,
        "Transfer log index",
    )?;
    Ok(Some(ChainTransfer {
        tx_hash,
        log_index: i64::try_from(log_index)
            .context("Transfer log index exceeds Postgres BIGINT")?,
        block_number: i64::try_from(block_number)
            .context("Transfer block number exceeds Postgres BIGINT")?,
        block_hash,
        from_address,
        to_address,
        amount_wei: amount_wei.to_string(),
        block_time,
    }))
}

async fn finalized_chain_height(
    client: &reqwest::Client,
    config: &FaucetConfig,
) -> anyhow::Result<(u64, String)> {
    let rpc_url = config.history_rpc_url();
    let finalized = rpc_request(
        client,
        rpc_url,
        "eth_getBlockByNumber",
        serde_json::json!(["finalized", false]),
    )
    .await?;
    let finalized_number = parse_rpc_hex_u64(
        finalized
            .get("number")
            .context("finalized block has no number")?,
        "finalized block number",
    )?;
    let finalized_hash = finalized
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .context("finalized block has no hash")?
        .to_ascii_lowercase();
    let latest = rpc_request(
        client,
        rpc_url,
        "eth_getBlockByNumber",
        serde_json::json!(["latest", false]),
    )
    .await?;
    let latest_number = parse_rpc_hex_u64(
        latest.get("number").context("latest block has no number")?,
        "latest block number",
    )?;
    let confirmation_height = latest_number.saturating_sub(config.reconciliation_confirmations);
    if confirmation_height < finalized_number {
        let block = rpc_request(
            client,
            rpc_url,
            "eth_getBlockByNumber",
            serde_json::json!([format!("0x{confirmation_height:x}"), false]),
        )
        .await?;
        let number = parse_rpc_hex_u64(
            block
                .get("number")
                .context("confirmation block has no number")?,
            "confirmation block number",
        )?;
        let hash = block
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .context("confirmation block has no hash")?
            .to_ascii_lowercase();
        Ok((number, hash))
    } else {
        Ok((finalized_number, finalized_hash))
    }
}

async fn verify_token_deployment(
    client: &reqwest::Client,
    config: &FaucetConfig,
    token_address: &str,
    deployment_block: u64,
) -> anyhow::Result<()> {
    if config.chain_id != SEPOLIA_CHAIN_ID {
        anyhow::bail!(
            "configured faucet history reconciliation currently requires Sepolia (chain 11155111)"
        );
    }
    let tx_hash = config.token_deployment_tx_hash.trim();
    if tx_hash.is_empty() {
        anyhow::bail!("FAUCET_TOKEN_DEPLOYMENT_TX_HASH is required for history reconciliation");
    }
    let receipt = rpc_request_with_receipt_fallback(
        client,
        config,
        "eth_getTransactionReceipt",
        serde_json::json!([tx_hash]),
    )
    .await?
    .as_object()
    .cloned()
    .context("token deployment receipt is unavailable")?;
    let contract_address = receipt
        .get("contractAddress")
        .and_then(serde_json::Value::as_str)
        .context("configured deployment transaction did not create a contract")?
        .to_ascii_lowercase();
    if contract_address != token_address {
        anyhow::bail!(
            "configured deployment receipt contract address does not match FAUCET_TOKEN_CONTRACT"
        );
    }
    if parse_rpc_hex_u64(
        receipt
            .get("status")
            .context("deployment receipt has no status")?,
        "deployment receipt status",
    )? != 1
    {
        anyhow::bail!("configured token deployment transaction did not succeed");
    }
    let receipt_block = parse_rpc_hex_u64(
        receipt
            .get("blockNumber")
            .context("deployment receipt has no block number")?,
        "token deployment block",
    )?;
    if receipt_block != deployment_block {
        anyhow::bail!("FAUCET_TOKEN_DEPLOYMENT_BLOCK does not match the deployment receipt");
    }
    let receipt_hash = receipt
        .get("transactionHash")
        .and_then(serde_json::Value::as_str)
        .context("deployment receipt has no transaction hash")?
        .to_ascii_lowercase();
    if receipt_hash != tx_hash.to_ascii_lowercase() {
        anyhow::bail!("deployment receipt transaction hash does not match configuration");
    }
    let code = rpc_request(
        client,
        config.history_rpc_url(),
        "eth_getCode",
        serde_json::json!([token_address, "latest"]),
    )
    .await?;
    if code
        .as_str()
        .map_or(true, |value| value == "0x" || value == "0x0")
    {
        anyhow::bail!("configured token contract has no deployed bytecode");
    }
    let chain_id = parse_rpc_hex_u64(
        &rpc_request(
            client,
            config.history_rpc_url(),
            "eth_chainId",
            serde_json::json!([]),
        )
        .await?,
        "RPC chain ID",
    )?;
    if chain_id != config.chain_id {
        anyhow::bail!("FAUCET_CHAIN_ID does not match the connected RPC chain");
    }
    Ok(())
}

async fn eth_call_u128(
    client: &reqwest::Client,
    config: &FaucetConfig,
    to: &str,
    data: &str,
    block_tag: &str,
) -> anyhow::Result<u128> {
    let result = rpc_request(
        client,
        config.history_rpc_url(),
        "eth_call",
        serde_json::json!([{"to": to, "data": data}, block_tag]),
    )
    .await?;
    parse_rpc_hex_u128(&result, "eth_call result")
}

async fn load_faucet_contract_balances(
    client: &reqwest::Client,
    config: &FaucetConfig,
    token_address: &str,
    faucet_address: &str,
    at_block: u64,
) -> anyhow::Result<(u128, u128)> {
    let address = faucet_address
        .strip_prefix("0x")
        .unwrap_or(faucet_address)
        .to_ascii_lowercase();
    if address.len() != 40 || !address.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("invalid configured faucet address");
    }
    let balance_call = format!("0x70a08231{:0>64}", address);
    let block_tag = format!("0x{at_block:x}");
    let balance = eth_call_u128(client, config, token_address, &balance_call, &block_tag).await?;
    let total_supply =
        eth_call_u128(client, config, token_address, "0x18160ddd", &block_tag).await?;
    if total_supply > MAX_TESTNET_TOKEN_SUPPLY_WEI {
        anyhow::bail!("on-chain tCREG total supply exceeds the 42 million testnet cap");
    }
    Ok((balance, total_supply))
}

async fn indexer_logs_for_range(
    client: &reqwest::Client,
    base_url: &str,
    token_address: &str,
    faucet_topic: &str,
    indexed_topic: u8,
    from_block: u64,
    to_block: u64,
) -> anyhow::Result<Vec<serde_json::Value>> {
    const MAX_LOGS_PER_QUERY: usize = 100;
    let mut ranges = vec![(from_block, to_block)];
    let mut all_logs = Vec::new();
    while let Some((start, end)) = ranges.pop() {
        let mut url = reqwest::Url::parse(base_url.trim())
            .context("FAUCET_RECONCILIATION_INDEXER_URL is not a valid URL")?;
        if url.scheme() != "https" {
            anyhow::bail!("faucet reconciliation indexer must use HTTPS");
        }
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("module", "logs")
                .append_pair("action", "getLogs")
                .append_pair("fromBlock", &start.to_string())
                .append_pair("toBlock", &end.to_string())
                .append_pair("address", token_address)
                .append_pair("topic0", ERC20_TRANSFER_TOPIC);
            if indexed_topic == 1 {
                query
                    .append_pair("topic1", faucet_topic)
                    .append_pair("topic0_1_opr", "and");
            } else if indexed_topic == 2 {
                query
                    .append_pair("topic2", faucet_topic)
                    .append_pair("topic0_2_opr", "and");
            } else {
                anyhow::bail!("invalid indexed faucet Transfer topic position");
            }
        }
        let mut attempt = 0;
        let body: serde_json::Value = loop {
            attempt += 1;
            // Routescan's keyless tier is rate limited. Keep the production
            // reconciler below two requests per second, including retries.
            tokio::time::sleep(Duration::from_millis(600)).await;
            let response = client
                .get(url.clone())
                .header(
                    reqwest::header::USER_AGENT,
                    "creg-faucet-ledger-reconciler/1",
                )
                .send()
                .await;
            match response {
                Ok(response)
                    if (response.status().is_server_error()
                        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS)
                        && attempt < 3 =>
                {
                    tokio::time::sleep(Duration::from_millis(if attempt == 1 {
                        500
                    } else {
                        1_000
                    }))
                    .await;
                }
                Ok(response) if !response.status().is_success() => {
                    anyhow::bail!(
                        "faucet Transfer history indexer returned HTTP status {}",
                        response.status()
                    );
                }
                Ok(response) => {
                    break response
                        .json()
                        .await
                        .context("decode faucet Transfer history indexer response")?;
                }
                Err(_) if attempt < 3 => {
                    tokio::time::sleep(Duration::from_millis(if attempt == 1 {
                        500
                    } else {
                        1_000
                    }))
                    .await;
                }
                Err(_) => anyhow::bail!(
                    "request faucet Transfer history from configured indexer failed after retries"
                ),
            }
        };
        let status = body.get("status").and_then(serde_json::Value::as_str);
        let result = body.get("result");
        let logs = if status == Some("1") {
            result
                .and_then(serde_json::Value::as_array)
                .context("faucet Transfer history indexer returned a non-array result")?
                .clone()
        } else {
            let message = body
                .get("message")
                .and_then(serde_json::Value::as_str)
                .or_else(|| result.and_then(serde_json::Value::as_str))
                .unwrap_or_default();
            if message.eq_ignore_ascii_case("no records found")
                || message.eq_ignore_ascii_case("no logs found")
            {
                Vec::new()
            } else {
                anyhow::bail!("faucet Transfer history indexer rejected a range query");
            }
        };

        // Some Etherscan-compatible indexers cap each result page. Subdivide
        // dense ranges so a truncated response can never be accepted as full.
        if logs.len() >= MAX_LOGS_PER_QUERY {
            if start == end {
                anyhow::bail!(
                    "faucet Transfer history exceeds the indexer's per-block result limit"
                );
            }
            let middle = start + (end - start) / 2;
            ranges.push((middle.saturating_add(1), end));
            ranges.push((start, middle));
        } else {
            all_logs.extend(logs);
        }
    }
    Ok(all_logs)
}

async fn verify_indexed_transfer_receipts(
    client: &reqwest::Client,
    config: &FaucetConfig,
    logs: &[serde_json::Value],
) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut transaction_hashes = BTreeMap::new();
    let mut canonical_hashes = HashMap::new();
    for log in logs {
        let tx_hash = log
            .get("transactionHash")
            .and_then(serde_json::Value::as_str)
            .context("indexed Transfer log has no transaction hash")?
            .to_ascii_lowercase();
        transaction_hashes.insert(tx_hash, ());
    }

    let mut receipts = HashMap::new();
    for tx_hash in transaction_hashes.keys() {
        let receipt = rpc_request_with_receipt_fallback(
            client,
            config,
            "eth_getTransactionReceipt",
            serde_json::json!([tx_hash]),
        )
        .await?;
        if receipt.is_null() {
            anyhow::bail!(
                "indexer returned a Transfer log whose transaction receipt is unavailable"
            );
        }
        if parse_rpc_hex_u64(
            receipt
                .get("status")
                .context("indexed Transfer transaction receipt has no status")?,
            "indexed Transfer receipt status",
        )? != 1
        {
            anyhow::bail!("indexer returned a Transfer log from a failed transaction");
        }
        if receipt
            .get("transactionHash")
            .and_then(serde_json::Value::as_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
            != Some(tx_hash.as_str())
        {
            anyhow::bail!("receipt transaction hash does not match the indexed Transfer log");
        }
        receipts.insert(tx_hash.clone(), receipt);
    }

    let mut verified_logs = Vec::with_capacity(logs.len());
    for log in logs {
        let tx_hash = log
            .get("transactionHash")
            .and_then(serde_json::Value::as_str)
            .context("indexed Transfer log has no transaction hash")?
            .to_ascii_lowercase();
        let receipt = receipts
            .get(&tx_hash)
            .context("verified Transfer receipt was not cached")?;
        let expected_index = parse_rpc_hex_u64(
            log.get("logIndex")
                .context("indexed Transfer log has no log index")?,
            "indexed Transfer log index",
        )?;
        let expected_block = parse_rpc_hex_u64(
            log.get("blockNumber")
                .context("indexed Transfer log has no block number")?,
            "indexed Transfer block number",
        )?;
        let receipt_block = parse_rpc_hex_u64(
            receipt
                .get("blockNumber")
                .context("indexed Transfer receipt has no block number")?,
            "Transfer receipt block number",
        )?;
        let receipt_block_hash = receipt
            .get("blockHash")
            .and_then(serde_json::Value::as_str)
            .context("indexed Transfer receipt has no block hash")?
            .to_ascii_lowercase();
        if receipt_block != expected_block {
            anyhow::bail!("indexed Transfer log does not match its canonical receipt block");
        }
        if let Some(indexer_block_hash) = log.get("blockHash").and_then(serde_json::Value::as_str) {
            if !indexer_block_hash.eq_ignore_ascii_case(&receipt_block_hash) {
                anyhow::bail!("indexed Transfer log block hash does not match its receipt");
            }
        }
        if let std::collections::hash_map::Entry::Vacant(entry) =
            canonical_hashes.entry(receipt_block)
        {
            let canonical_block = rpc_request(
                client,
                config.history_rpc_url(),
                "eth_getBlockByNumber",
                serde_json::json!([format!("0x{receipt_block:x}"), false]),
            )
            .await?;
            let canonical_number = parse_rpc_hex_u64(
                canonical_block
                    .get("number")
                    .context("canonical Transfer block has no number")?,
                "canonical Transfer block number",
            )?;
            let canonical_hash = canonical_block
                .get("hash")
                .and_then(serde_json::Value::as_str)
                .context("canonical Transfer block has no hash")?
                .to_ascii_lowercase();
            if canonical_number != receipt_block || canonical_hash != receipt_block_hash {
                anyhow::bail!("Transfer receipt does not match the canonical block header");
            }
            entry.insert(canonical_hash);
        }
        let receipt_logs = receipt
            .get("logs")
            .and_then(serde_json::Value::as_array)
            .context("indexed Transfer receipt has no logs array")?;
        let matching_receipt_log = receipt_logs.iter().find(|receipt_log| {
            receipt_log
                .get("logIndex")
                .and_then(serde_json::Value::as_str)
                .and_then(|index| u64::from_str_radix(index.trim_start_matches("0x"), 16).ok())
                == Some(expected_index)
        });
        let receipt_log = matching_receipt_log
            .context("indexed Transfer log index is absent from its transaction receipt")?;
        for field in ["address", "data", "transactionHash"] {
            let expected = log
                .get(field)
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("indexed Transfer log has no {field}"))?;
            let actual = receipt_log
                .get(field)
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("receipt Transfer log has no {field}"))?;
            if !expected.eq_ignore_ascii_case(actual) {
                anyhow::bail!("indexed Transfer log {field} does not match its receipt");
            }
        }
        let expected_topics = log
            .get("topics")
            .and_then(serde_json::Value::as_array)
            .context("indexed Transfer log has no topics")?;
        let receipt_topics = receipt_log
            .get("topics")
            .and_then(serde_json::Value::as_array)
            .context("receipt Transfer log has no topics")?;
        if expected_topics.len() != receipt_topics.len()
            || expected_topics
                .iter()
                .zip(receipt_topics)
                .any(|(expected, actual)| {
                    expected
                        .as_str()
                        .zip(actual.as_str())
                        .map_or(true, |(expected, actual)| {
                            !expected.eq_ignore_ascii_case(actual)
                        })
                })
        {
            anyhow::bail!("indexed Transfer log topics do not match its receipt");
        }
        let mut verified_log = log.clone();
        let object = verified_log
            .as_object_mut()
            .context("indexed Transfer log is not an object")?;
        object.insert(
            "blockHash".to_string(),
            serde_json::Value::String(receipt_block_hash),
        );
        // The explorer-compatible endpoint omits blockHash on some responses.
        // Fill it only from the independently checked canonical RPC receipt.
        if !canonical_hashes.contains_key(&expected_block) {
            anyhow::bail!("canonical block hash was not recorded for indexed Transfer");
        }
        verified_logs.push(verified_log);
    }
    Ok(verified_logs)
}

async fn scan_faucet_transfer_history(
    client: &reqwest::Client,
    config: &FaucetConfig,
    token_address: &str,
    faucet_address: &str,
    from_block: u64,
    through_block: u64,
) -> anyhow::Result<Vec<ChainTransfer>> {
    if from_block > through_block {
        return Ok(Vec::new());
    }
    let faucet_topic = indexed_address_topic(faucet_address)?;
    let mut raw_logs = Vec::new();
    let mut start = from_block;
    const LOG_CHUNK_BLOCKS: u64 = 5_000;
    while start <= through_block {
        let end = start
            .saturating_add(LOG_CHUNK_BLOCKS - 1)
            .min(through_block);
        if config.reconciliation_indexer_url.trim().is_empty() {
            for topics in [
                serde_json::json!([ERC20_TRANSFER_TOPIC, faucet_topic, null]),
                serde_json::json!([ERC20_TRANSFER_TOPIC, null, faucet_topic]),
            ] {
                let logs = rpc_request(
                    client,
                    config.history_rpc_url(),
                    "eth_getLogs",
                    serde_json::json!([{
                        "address": token_address,
                        "fromBlock": format!("0x{start:x}"),
                        "toBlock": format!("0x{end:x}"),
                        "topics": topics,
                    }]),
                )
                .await?;
                raw_logs.extend(
                    logs.as_array()
                        .context("eth_getLogs returned a non-array result")?
                        .iter()
                        .cloned(),
                );
            }
        } else {
            raw_logs.extend(
                indexer_logs_for_range(
                    client,
                    &config.reconciliation_indexer_url,
                    token_address,
                    &faucet_topic,
                    1,
                    start,
                    end,
                )
                .await?,
            );
            raw_logs.extend(
                indexer_logs_for_range(
                    client,
                    &config.reconciliation_indexer_url,
                    token_address,
                    &faucet_topic,
                    2,
                    start,
                    end,
                )
                .await?,
            );
        }
        start = end.saturating_add(1);
    }

    if !config.reconciliation_indexer_url.trim().is_empty() {
        raw_logs = verify_indexed_transfer_receipts(client, config, &raw_logs).await?;
    }

    let mut block_numbers = raw_logs
        .iter()
        .map(|log| {
            parse_rpc_hex_u64(
                log.get("blockNumber")
                    .context("Transfer log has no block number")?,
                "Transfer block number",
            )
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    block_numbers.sort_unstable();
    block_numbers.dedup();
    let mut block_times = HashMap::new();
    for number in block_numbers {
        let block = rpc_request(
            client,
            config.history_rpc_url(),
            "eth_getBlockByNumber",
            serde_json::json!([format!("0x{number:x}"), false]),
        )
        .await?;
        let timestamp = parse_rpc_hex_u64(
            block
                .get("timestamp")
                .context("Transfer block has no timestamp")?,
            "Transfer block timestamp",
        )?;
        let timestamp =
            i64::try_from(timestamp).context("block timestamp exceeds signed seconds")?;
        let date_time = DateTime::<Utc>::from_timestamp(timestamp, 0)
            .context("Transfer block timestamp is outside chrono range")?;
        let canonical_hash = block
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .context("Transfer block has no canonical hash")?
            .to_ascii_lowercase();
        for log in raw_logs.iter() {
            let log_block = parse_rpc_hex_u64(
                log.get("blockNumber")
                    .context("Transfer log has no block number")?,
                "Transfer block number",
            )?;
            if log_block == number {
                let log_hash = log
                    .get("blockHash")
                    .and_then(serde_json::Value::as_str)
                    .context("Transfer log has no block hash")?;
                if !log_hash.eq_ignore_ascii_case(&canonical_hash) {
                    anyhow::bail!(
                        "faucet Transfer indexer log does not match the canonical block hash"
                    );
                }
            }
        }
        block_times.insert(number, date_time);
    }

    let normalized_token = token_address.to_ascii_lowercase();
    let normalized_faucet = faucet_address.to_ascii_lowercase();
    let mut transfers = BTreeMap::new();
    for log in raw_logs {
        let block_number = parse_rpc_hex_u64(
            log.get("blockNumber")
                .context("Transfer log has no block number")?,
            "Transfer block number",
        )?;
        if block_number < from_block || block_number > through_block {
            anyhow::bail!(
                "faucet Transfer indexer returned a log outside the requested block range"
            );
        }
        let block_time = *block_times
            .get(&block_number)
            .context("Transfer block timestamp was not loaded")?;
        let transfer = decode_transfer_log(&log, &normalized_token, block_time)?
            .context("faucet Transfer indexer returned a non-Transfer log")?;
        if transfer.from_address != normalized_faucet && transfer.to_address != normalized_faucet {
            anyhow::bail!("faucet-indexed log does not contain the configured faucet address");
        }
        let key = (transfer.tx_hash.clone(), transfer.log_index);
        if let Some(existing) = transfers.insert(key, transfer.clone()) {
            if existing.block_hash != transfer.block_hash
                || existing.from_address != transfer.from_address
                || existing.to_address != transfer.to_address
                || existing.amount_wei != transfer.amount_wei
            {
                anyhow::bail!(
                    "conflicting duplicate faucet Transfer log returned by history sources"
                );
            }
        }
    }
    Ok(transfers.into_values().collect())
}

async fn reconcile_faucet_ledger(state: &AppState) -> anyhow::Result<()> {
    let config = &state.config;
    let token_address = parse_address(&config.token_contract, "token contract")
        .map_err(anyhow::Error::msg)?
        .to_string()
        .to_ascii_lowercase();
    let faucet_address = parse_address(&config.faucet_address, "faucet address")
        .map_err(anyhow::Error::msg)?
        .to_string()
        .to_ascii_lowercase();
    let deployment_block = config
        .token_deployment_block
        .context("FAUCET_TOKEN_DEPLOYMENT_BLOCK is required for reconciliation")?;
    let deployment_block_i64 =
        i64::try_from(deployment_block).context("deployment block exceeds Postgres BIGINT")?;
    let chain_id_i64 =
        i64::try_from(config.chain_id).context("chain ID exceeds Postgres BIGINT")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("build faucet chain reconciliation HTTP client")?;
    verify_token_deployment(&client, config, &token_address, deployment_block).await?;

    let (through_block, through_hash) = finalized_chain_height(&client, config).await?;
    if through_block < deployment_block {
        anyhow::bail!("configured token deployment is newer than the finalized chain tip");
    }
    let through_block_i64 =
        i64::try_from(through_block).context("finalized block exceeds Postgres BIGINT")?;
    let deployment_tx_hash = config.token_deployment_tx_hash.to_ascii_lowercase();

    let cursor = state
        .rate_limiter
        .reconciliation_cursor(
            chain_id_i64,
            &token_address,
            &deployment_tx_hash,
            deployment_block_i64,
        )
        .await?;
    let scan_from = if let Some((cursor_block, cursor_hash)) = cursor {
        if through_block < u64::try_from(cursor_block).unwrap_or_default() {
            anyhow::bail!(
                "RPC finalized height moved behind the saved faucet reconciliation watermark"
            );
        }
        let saved_block = rpc_request(
            &client,
            config.history_rpc_url(),
            "eth_getBlockByNumber",
            serde_json::json!([format!("0x{cursor_block:x}"), false]),
        )
        .await?;
        let canonical_hash = saved_block
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .context("saved reconciliation block is unavailable from RPC")?
            .to_ascii_lowercase();
        if canonical_hash != cursor_hash {
            anyhow::bail!("saved faucet reconciliation watermark is no longer canonical; manual canonical-history repair is required");
        }
        u64::try_from(cursor_block)
            .context("negative faucet reconciliation cursor")?
            .saturating_add(1)
    } else {
        deployment_block
    };

    let transfers = scan_faucet_transfer_history(
        &client,
        config,
        &token_address,
        &faucet_address,
        scan_from,
        through_block,
    )
    .await?;
    let (faucet_balance, total_supply) = load_faucet_contract_balances(
        &client,
        config,
        &token_address,
        &faucet_address,
        through_block,
    )
    .await?;
    let no_unresolved = state
        .rate_limiter
        .apply_reconciled_transfers(
            chain_id_i64,
            &token_address,
            &deployment_tx_hash,
            deployment_block_i64,
            through_block_i64,
            &through_hash,
            faucet_balance,
            total_supply,
            &faucet_address,
            &transfers,
        )
        .await?;
    if !no_unresolved {
        anyhow::bail!(
            "one or more token claim reservations remain unresolved after on-chain history import"
        );
    }
    info!(
        "Faucet ledger reconciled through safe chain watermark {} ({} canonical transfers, balance {} wei, supply {} wei)",
        through_block,
        transfers.len(),
        faucet_balance,
        total_supply
    );
    Ok(())
}

async fn recover_submitted_receipts(state: &AppState) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build faucet receipt recovery HTTP client")?;
    let token_address = parse_address(&state.config.token_contract, "token contract")
        .map_err(anyhow::Error::msg)?
        .to_string()
        .to_ascii_lowercase();
    let faucet_address = parse_address(&state.config.faucet_address, "faucet address")
        .map_err(anyhow::Error::msg)?
        .to_string()
        .to_ascii_lowercase();
    for (claim_id, tx_hash, recipient, amount) in
        state.rate_limiter.submitted_token_claims().await?
    {
        let receipt = rpc_request_with_receipt_fallback(
            &client,
            &state.config,
            "eth_getTransactionReceipt",
            serde_json::json!([tx_hash]),
        )
        .await?;
        if receipt.is_null() {
            continue;
        }
        let status = parse_rpc_hex_u64(
            receipt
                .get("status")
                .context("submitted token receipt has no status")?,
            "submitted token receipt status",
        )?;
        if status == 0 {
            let receipt_hash = receipt
                .get("transactionHash")
                .and_then(serde_json::Value::as_str)
                .context("reverted native receipt has no transaction hash")?
                .to_ascii_lowercase();
            if receipt_hash != tx_hash.to_ascii_lowercase() {
                anyhow::bail!("reverted native receipt hash does not match its ledger row");
            }
            state
                .rate_limiter
                .mark_transfer_failed(claim_id, true, &tx_hash)
                .await?;
            continue;
        }
        if status != 1 {
            anyhow::bail!("submitted token receipt has an unsupported status value");
        }
        let receipt_hash = receipt
            .get("transactionHash")
            .and_then(serde_json::Value::as_str)
            .context("submitted token receipt has no transaction hash")?
            .to_ascii_lowercase();
        if receipt_hash != tx_hash.to_ascii_lowercase() {
            anyhow::bail!("submitted token receipt hash does not match its ledger row");
        }
        let logs = receipt
            .get("logs")
            .and_then(serde_json::Value::as_array)
            .context("successful submitted token receipt has no logs")?;
        let mut matched = false;
        for log in logs {
            if let Some(event) = decode_transfer_log(log, &token_address, Utc::now())? {
                if event.from_address == faucet_address
                    && event.to_address == recipient
                    && event.amount_wei == amount
                {
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            anyhow::bail!(
                "successful token receipt {} does not contain the exact expected faucet Transfer event",
                tx_hash
            );
        }
        state
            .rate_limiter
            .confirm_token_transfer(claim_id, &tx_hash)
            .await?;
        info!(
            "Recovered submitted tCREG claim {} from receipt {}",
            claim_id, tx_hash
        );
    }

    for (claim_id, tx_hash, recipient, amount) in
        state.rate_limiter.submitted_native_claims().await?
    {
        let receipt = rpc_request_with_receipt_fallback(
            &client,
            &state.config,
            "eth_getTransactionReceipt",
            serde_json::json!([tx_hash]),
        )
        .await?;
        if receipt.is_null() {
            continue;
        }
        let status = parse_rpc_hex_u64(
            receipt
                .get("status")
                .context("submitted native receipt has no status")?,
            "submitted native receipt status",
        )?;
        if status == 0 {
            state
                .rate_limiter
                .mark_transfer_failed(claim_id, false, &tx_hash)
                .await?;
            continue;
        }
        if status != 1 {
            anyhow::bail!("submitted native receipt has an unsupported status value");
        }
        let receipt_hash = receipt
            .get("transactionHash")
            .and_then(serde_json::Value::as_str)
            .context("submitted native receipt has no transaction hash")?
            .to_ascii_lowercase();
        if receipt_hash != tx_hash.to_ascii_lowercase() {
            anyhow::bail!("submitted native receipt hash does not match its ledger row");
        }
        let transaction = rpc_request_with_receipt_fallback(
            &client,
            &state.config,
            "eth_getTransactionByHash",
            serde_json::json!([tx_hash]),
        )
        .await?;
        let from = transaction
            .get("from")
            .and_then(serde_json::Value::as_str)
            .context("submitted native transaction has no sender")?
            .to_ascii_lowercase();
        let to = transaction
            .get("to")
            .and_then(serde_json::Value::as_str)
            .context("submitted native transaction has no recipient")?
            .to_ascii_lowercase();
        let value = parse_rpc_hex_u128(
            transaction
                .get("value")
                .context("submitted native transaction has no value")?,
            "submitted native transaction value",
        )?;
        let expected_from = parse_address(&state.config.faucet_address, "faucet address")
            .map_err(anyhow::Error::msg)?
            .to_string()
            .to_ascii_lowercase();
        if from != expected_from || to != recipient || value.to_string() != amount {
            anyhow::bail!(
                "successful native receipt {} does not match the expected faucet sender, recipient, and amount",
                tx_hash
            );
        }
        state
            .rate_limiter
            .confirm_native_transfer(claim_id, &tx_hash)
            .await?;
        info!(
            "Recovered submitted native faucet claim {} from receipt {}",
            claim_id, tx_hash
        );
    }
    Ok(())
}

/// Application state
struct AppState {
    config: FaucetConfig,
    rate_limiter: PersistentRateLimiter,
    /// Set only after the on-chain transfer history is verified against the
    /// deployed token, safe chain watermark, and historical faucet balance.
    ledger_reconciled: AtomicBool,
    /// Prevent a public page load from generating repeated indexer requests.
    supply_cache: Mutex<Option<(Instant, Option<serde_json::Value>)>>,
    /// Active PoW challenges keyed by challenge string.
    pow_challenges: DashMap<String, PowChallenge>,
    /// Operator pause flag — set via POST /admin/pause, cleared via POST /admin/resume.
    is_paused: Arc<AtomicBool>,
}

impl AppState {
    fn ledger_reconciled(&self) -> bool {
        self.ledger_reconciled.load(Ordering::Acquire)
    }

    fn token_claims_enabled(&self) -> bool {
        self.config.token_distribution_open
            && self.config.token_claim_policy_ready()
            && self.ledger_reconciled()
            && !self.is_paused.load(Ordering::Acquire)
    }
}

/// A proof-of-work challenge issued to clients.
#[derive(Clone)]
struct PowChallenge {
    difficulty: u8,
    created_at: Instant,
}

/// PoW difficulty — number of leading zero bits required in SHA-256(challenge || nonce).
/// 20 bits ≈ 1M hashes ≈ ~1 second on a modern browser.
const POW_DIFFICULTY: u8 = 20;
/// Challenge validity window.
const POW_TTL: Duration = Duration::from_secs(120);

/// Serialize a u128 as a JSON string so large amounts survive the JavaScript
/// 53-bit Number precision limit without truncation.
fn serialize_u128_as_string<S: serde::Serializer>(val: &u128, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&val.to_string())
}

#[derive(Serialize)]
struct FaucetStats {
    total_drips: u64,
    /// Total tCREG distributed in wei (serialised as string for JS compatibility).
    #[serde(serialize_with = "serialize_u128_as_string")]
    total_distributed: u128,
    /// Total native ETH distributed in wei (serialised as string).
    #[serde(serialize_with = "serialize_u128_as_string")]
    total_native_distributed: u128,
    unique_addresses: usize,
    last_drip: Option<DateTime<Utc>>,
    unresolved_claims: u64,
}

/// Request to drip tokens
#[derive(Deserialize)]
struct DripRequest {
    address: String,
    /// The PoW challenge string returned by /api/challenge.
    challenge: Option<String>,
    /// The nonce the client found such that SHA256(challenge||nonce) has N leading zero bits.
    nonce: Option<String>,
}

/// PoW challenge response
#[derive(Serialize)]
struct ChallengeResponse {
    challenge: String,
    difficulty: u8,
    ttl_secs: u64,
}

/// Drip response
#[derive(Serialize)]
struct DripResponse {
    success: bool,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cooldown_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_amount: Option<String>,
}

impl DripResponse {
    fn error(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self {
            success: false,
            message: msg.clone(),
            error: Some(msg),
            tx_hash: None,
            amount: None,
            retry_after_seconds: None,
            cooldown_seconds: None,
            token_tx_hash: None,
            native_tx_hash: None,
            token_amount: None,
            native_amount: None,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let (config, secrets) = FaucetConfig::from_env().await?;
    secrets.warn_hot_key_if_env(
        "faucet",
        chain_registry_secrets::HotKeyRole::Faucet,
        &config.faucet_key,
        common::is_testnet_env(),
    );

    info!("╔════════════════════════════════════════════════════════╗");
    info!("║        Chain Registry Testnet Faucet (REAL)            ║");
    info!("╚════════════════════════════════════════════════════════╝");
    info!(
        "  Drip amount: {} tCREG",
        config.drip_amount / 10_u128.pow(18)
    );
    if config.native_drip_amount > 0 {
        info!(
            "  Gas drip amount: {:.4} ETH",
            config.native_drip_amount as f64 / 10_f64.powi(18)
        );
    }
    info!("  Cooldown: {} seconds", config.cooldown_secs);
    info!("  Token contract: {}", config.token_contract);
    info!("  RPC: {}", config.rpc_url);
    info!("  Faucet address: {}", config.faucet_address);

    // ── Pre-flight balance check ──────────────────────────────────────────────
    // Warn loudly at startup if the faucet wallet has no tokens.  This catches
    // the common case where Anvil was restarted (losing on-chain state) without
    // re-running deploy-contracts + sync-testnet-artifacts.
    match get_token_balance(&config, &config.faucet_address).await {
        Ok(0) => {
            let chain_id = env_u64("FAUCET_CHAIN_ID", 31337);
            let fund_hint = if chain_id == 11155111 {
                "testnet\\fund-sepolia-faucet-governance.ps1 -FaucetAllocationCreg <approved-tranche> (preview)"
            } else {
                "scripts/start-testnet.ps1 (or Fund-TestnetFaucet)"
            };
            error!("╔═══════════════════════════════════════════════════════════╗");
            error!("║  FAUCET BALANCE IS ZERO — drip requests WILL FAIL         ║");
            error!("║  Next step: {fund_hint}");
            error!("╚═══════════════════════════════════════════════════════════╝");
        }
        Ok(bal) => {
            info!(
                "  Faucet token balance: {:.2} tCREG — ready to drip",
                bal as f64 / 1e18
            );
        }
        Err(e) => {
            warn!(
                "Could not read faucet token balance at startup (RPC may not be ready yet): {}",
                e
            );
        }
    }

    let pg_url = env_string("FAUCET_PG_URL", "");
    let rate_limiter = PersistentRateLimiter::new(&pg_url).await?;

    let state = Arc::new(AppState {
        config,
        rate_limiter,
        ledger_reconciled: AtomicBool::new(false),
        supply_cache: Mutex::new(None),
        pow_challenges: DashMap::new(),
        is_paused: Arc::new(AtomicBool::new(false)),
    });

    let reconciliation_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut retry_delay = Duration::from_secs(300);
        loop {
            let result = async {
                recover_submitted_receipts(&reconciliation_state).await?;
                reconcile_faucet_ledger(&reconciliation_state).await
            }
            .await;
            match result {
                Ok(()) => {
                    reconciliation_state
                        .ledger_reconciled
                        .store(true, Ordering::Release);
                    retry_delay = Duration::from_secs(300);
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
                Err(err) => {
                    reconciliation_state
                        .ledger_reconciled
                        .store(false, Ordering::Release);
                    error!("Faucet claim ledger reconciliation failed; token claims remain disabled: {:#}", err);
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = retry_delay.saturating_mul(2).min(Duration::from_secs(900));
                }
            }
        }
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // ── Prometheus metrics recorder ───────────────────────────────────────────
    let prometheus_handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .expect("failed to install Prometheus metrics recorder");

    // Pre-declare metric descriptions so they appear in /metrics even before
    // the first drip.
    metrics::describe_counter!(
        "faucet_drips_total",
        "Total number of successful drip operations"
    );
    metrics::describe_counter!("faucet_token_drips_total", "Successful tCREG token drips");
    metrics::describe_counter!(
        "faucet_native_drips_total",
        "Successful native ETH gas drips"
    );
    metrics::describe_counter!("faucet_failures_total", "Failed drip attempts");
    metrics::describe_counter!(
        "faucet_rate_limited_total",
        "Requests rejected by rate limiter"
    );
    metrics::describe_counter!(
        "faucet_pow_failures_total",
        "Requests rejected due to invalid PoW"
    );
    metrics::describe_gauge!(
        "faucet_token_balance",
        "Current faucet tCREG token balance (raw wei)"
    );
    metrics::describe_gauge!(
        "faucet_native_balance",
        "Current faucet native ETH balance (raw wei)"
    );

    // Clone config for the background balance gauge task before `state` is
    // moved into the axum router.
    let bg_config = state.config.clone();

    let app = Router::new()
        .route("/", get(index_page))
        .route("/favicon.ico", get(favicon))
        .route("/api/challenge", get(get_challenge))
        .route("/api/drip", post(handle_drip))
        .route("/api/stats", get(get_stats))
        .route("/api/supply", get(get_supply))
        .route("/api/balance/:address", get(get_balance))
        .route("/api/network", get(get_network_info))
        .route("/health", get(health_check))
        .route(
            "/metrics",
            get(move || {
                let handle = prometheus_handle.clone();
                async move { handle.render() }
            }),
        )
        // ── Operator admin endpoints (require FAUCET_ADMIN_TOKEN) ─────────────
        .route("/admin/pause", post(admin_pause))
        .route("/admin/resume", post(admin_resume))
        .route("/admin/status", get(admin_status))
        .route("/admin/recovery", get(admin_recovery))
        .layer(cors)
        .with_state(state);

    let port = env_u16("FAUCET_PORT", 8082);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // ── Optional TLS ──────────────────────────────────────────────────────────
    #[cfg(feature = "tls")]
    {
        let tls_cert = std::env::var("FAUCET_TLS_CERT").ok();
        let tls_key = std::env::var("FAUCET_TLS_KEY").ok();

        if let (Some(cert_path), Some(key_path)) = (tls_cert, tls_key) {
            use axum_server::tls_rustls::RustlsConfig;

            let tls_config = RustlsConfig::from_pem_file(&cert_path, &key_path)
                .await
                .expect("Failed to load TLS certificate/key");

            info!("Faucet listening on https://{}", addr);

            axum_server::bind_rustls(addr, tls_config)
                .serve(app.into_make_service())
                .await?;

            return Ok(());
        }
    }

    info!("Faucet listening on http://{}", addr);

    // ── Background balance gauge updater ─────────────────────────────────────
    // Refreshes the Prometheus balance gauges every 30 seconds so Grafana always
    // shows an up-to-date faucet wallet balance without waiting for a drip.
    {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(30));
            loop {
                ticker.tick().await;
                if let Ok(bal) = get_token_balance(&bg_config, &bg_config.faucet_address).await {
                    gauge!("faucet_token_balance").set(bal as f64);
                }
                if let Ok(bal) = get_native_balance(&bg_config, &bg_config.faucet_address).await {
                    gauge!("faucet_native_balance").set(bal as f64);
                }
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// HTML faucet page
async fn index_page() -> impl IntoResponse {
    Html(include_str!("faucet.html"))
}

/// Small inline favicon so browsers do not fall back to a missing default asset.
async fn favicon() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>💧</text></svg>",
    )
}

fn parse_address(value: &str, field_name: &str) -> Result<Address, String> {
    value
        .parse::<Address>()
        .map_err(|e| format!("Invalid {}: {}", field_name, e))
}

async fn execute_token_transfer(
    config: &FaucetConfig,
    rate_limiter: &PersistentRateLimiter,
    claim_id: i64,
    to_address: &str,
) -> Result<String, String> {
    let signer: PrivateKeySigner = config
        .faucet_key
        .parse()
        .map_err(|e| format!("Invalid faucet private key: {}", e))?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(
            config
                .rpc_url
                .parse()
                .map_err(|e| format!("Invalid faucet RPC URL: {}", e))?,
        );

    let token_address = parse_address(&config.token_contract, "token contract")?;
    let recipient = parse_address(to_address, "recipient address")?;
    let contract = IERC20::new(token_address, &provider);

    let pending_tx = contract
        .transfer(recipient, U256::from(config.drip_amount))
        .send()
        .await
        .map_err(|e| format!("Transfer failed: {}", e))?;
    let tx_hash = *pending_tx.tx_hash();

    if let Err(err) = rate_limiter
        .mark_token_submitted(claim_id, &tx_hash.to_string())
        .await
    {
        error!(
            "tCREG transfer {} was broadcast but its submitted state could not be journaled: {}",
            tx_hash, err
        );
        return Err(format!(
            "Transfer {} was broadcast; durable receipt recovery is required",
            tx_hash
        ));
    }

    pending_tx
        .watch()
        .await
        .map_err(|e| format!("Transfer confirmation failed: {}", e))?;
    let receipt = provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(|e| format!("Could not read tCREG transfer receipt {}: {}", tx_hash, e))?
        .ok_or_else(|| format!("tCREG transfer receipt {} is not available", tx_hash))?;
    if !receipt.status() {
        rate_limiter
            .mark_transfer_failed(claim_id, true, &tx_hash.to_string())
            .await
            .map_err(|err| {
                format!("tCREG transfer reverted but failure journaling failed: {err}")
            })?;
        return Err(format!("tCREG transfer transaction {} reverted", tx_hash));
    }

    Ok(tx_hash.to_string())
}

async fn execute_native_transfer(
    config: &FaucetConfig,
    rate_limiter: &PersistentRateLimiter,
    claim_id: i64,
    to_address: &str,
) -> Result<String, String> {
    let signer: PrivateKeySigner = config
        .faucet_key
        .parse()
        .map_err(|e| format!("Invalid faucet private key: {}", e))?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(
            config
                .rpc_url
                .parse()
                .map_err(|e| format!("Invalid faucet RPC URL: {}", e))?,
        );

    let recipient = parse_address(to_address, "recipient address")?;
    let tx = TransactionRequest::default()
        .to(recipient)
        .value(U256::from(config.native_drip_amount));

    let pending_tx = provider
        .send_transaction(tx)
        .await
        .map_err(|e| format!("Native ETH transfer failed: {}", e))?;
    let tx_hash = *pending_tx.tx_hash();

    if let Err(err) = rate_limiter
        .mark_native_submitted(claim_id, &tx_hash.to_string())
        .await
    {
        error!(
            "Native faucet transfer {} was broadcast but its submitted state could not be journaled: {}",
            tx_hash, err
        );
        return Err(format!(
            "Native transfer {} was broadcast; durable receipt recovery is required",
            tx_hash
        ));
    }

    pending_tx
        .watch()
        .await
        .map_err(|e| format!("Native ETH confirmation failed: {}", e))?;
    let receipt = provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(|e| format!("Could not read native transfer receipt {}: {}", tx_hash, e))?
        .ok_or_else(|| format!("Native transfer receipt {} is not available", tx_hash))?;
    if !receipt.status() {
        rate_limiter
            .mark_transfer_failed(claim_id, false, &tx_hash.to_string())
            .await
            .map_err(|err| {
                format!("native transfer reverted but failure journaling failed: {err}")
            })?;
        return Err(format!(
            "Native ETH transfer transaction {} reverted",
            tx_hash
        ));
    }

    Ok(tx_hash.to_string())
}

async fn get_token_balance(config: &FaucetConfig, address: &str) -> Result<u128, String> {
    let holder = parse_address(address, "holder address")?;
    let token_addr = parse_address(&config.token_contract, "token contract")?;

    // Read-only provider — no wallet or nonce management needed for view calls.
    let provider = ProviderBuilder::new().on_http(
        config
            .rpc_url
            .parse()
            .map_err(|e| format!("Invalid RPC URL for balance check: {e}"))?,
    );

    let token = IERC20::new(token_addr, provider);
    let ret = token
        .balanceOf(holder)
        .call()
        .await
        .map_err(|e| format!("balanceOf({address}) call failed: {e}"))?;

    // Realistic token supplies (≤ 10^26 wei) fit well within u128 (≤ 3.4 × 10^38).
    Ok(ret._0.to::<u128>())
}

async fn get_native_balance(config: &FaucetConfig, address: &str) -> Result<u128, String> {
    let response: serde_json::Value = reqwest::Client::new()
        .post(&config.rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getBalance",
            "params": [address, "latest"],
            "id": 1
        }))
        .send()
        .await
        .map_err(|e| format!("Native balance check failed: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Native balance response decode failed: {}", e))?;

    if let Some(err) = response.get("error") {
        return Err(format!("Native balance check failed: {}", err));
    }

    let result = response
        .get("result")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "Native balance check failed: missing result".to_string())?;

    u128::from_str_radix(result.trim_start_matches("0x"), 16)
        .map_err(|e| format!("Failed to parse native balance: {}", e))
}

/// Issue a proof-of-work challenge. Client must find a nonce such that
/// SHA-256(challenge || nonce) has `difficulty` leading zero bits.
async fn get_challenge(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    info!(">>> get_challenge request received");
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let challenge = hex::encode(bytes);

    // Prune expired challenges periodically.
    state
        .pow_challenges
        .retain(|_, v| v.created_at.elapsed() < POW_TTL);

    state.pow_challenges.insert(
        challenge.clone(),
        PowChallenge {
            difficulty: POW_DIFFICULTY,
            created_at: Instant::now(),
        },
    );

    (
        StatusCode::OK,
        JsonResponse(ChallengeResponse {
            challenge,
            difficulty: POW_DIFFICULTY,
            ttl_secs: POW_TTL.as_secs(),
        }),
    )
}

/// Verify proof-of-work: SHA-256(challenge || nonce) must have `difficulty` leading zero bits.
fn verify_pow(challenge: &str, nonce: &str, difficulty: u8) -> bool {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(challenge.as_bytes());
    hasher.update(nonce.as_bytes());
    let hash = hasher.finalize();

    // Count leading zero bits.
    let mut leading_zeros = 0u8;
    for byte in hash.iter() {
        if *byte == 0 {
            leading_zeros += 8;
        } else {
            leading_zeros += byte.leading_zeros() as u8;
            break;
        }
        if leading_zeros >= difficulty {
            break;
        }
    }
    leading_zeros >= difficulty
}

/// Handle drip request
async fn handle_drip(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<DripRequest>,
) -> impl IntoResponse {
    // ── Operator pause check ──────────────────────────────────────────────────
    if state.is_paused.load(Ordering::Relaxed) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            JsonResponse(DripResponse::error(
                "Faucet is temporarily paused by the operator. Please try again later.",
            )),
        );
    }

    let address = request.address.to_lowercase();

    // ── PoW validation ────────────────────────────────────────────────────────
    let pow_enabled = std::env::var("FAUCET_POW_DISABLED").unwrap_or_default() != "true";
    if pow_enabled {
        let challenge = match &request.challenge {
            Some(c) => c.clone(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    JsonResponse(DripResponse::error(
                        "Missing proof-of-work challenge. Call GET /api/challenge first.",
                    )),
                );
            }
        };

        let nonce = match &request.nonce {
            Some(n) => n.clone(),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    JsonResponse(DripResponse::error("Missing proof-of-work nonce.")),
                );
            }
        };

        // Look up and consume the challenge (single-use).
        let pow_entry = state.pow_challenges.remove(&challenge);
        match pow_entry {
            Some((_, pc)) if pc.created_at.elapsed() < POW_TTL => {
                if !verify_pow(&challenge, &nonce, pc.difficulty) {
                    counter!("faucet_pow_failures_total", "reason" => "invalid_solution")
                        .increment(1);
                    return (
                        StatusCode::BAD_REQUEST,
                        JsonResponse(DripResponse::error("Invalid proof-of-work solution.")),
                    );
                }
            }
            _ => {
                counter!("faucet_pow_failures_total", "reason" => "expired_or_unknown")
                    .increment(1);
                return (
                    StatusCode::BAD_REQUEST,
                    JsonResponse(DripResponse::error(
                        "Unknown or expired challenge. Request a new one.",
                    )),
                );
            }
        }
    }

    // Validate address format
    if !address.starts_with("0x") || address.len() != 42 {
        return (
            StatusCode::BAD_REQUEST,
            JsonResponse(DripResponse::error("Invalid Ethereum address format")),
        );
    }
    if address.parse::<Address>().is_err() {
        return (
            StatusCode::BAD_REQUEST,
            JsonResponse(DripResponse::error("Invalid Ethereum address format")),
        );
    }

    let client_ip =
        client_ip_from_request(peer_addr.ip(), &headers, &state.config.trusted_proxy_cidrs);

    // Balance reads gate a real transfer. An RPC error must never be treated
    // as a zero balance or as permission to send.
    let token_balance = match get_token_balance(&state.config, &address).await {
        Ok(balance) => balance,
        Err(err) => {
            error!(
                "Could not read recipient token balance before drip: {}",
                err
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                JsonResponse(DripResponse::error(
                    "The token balance service is temporarily unavailable; no funds were sent.",
                )),
            );
        }
    };
    let native_balance = if state.config.native_drip_amount > 0 {
        match get_native_balance(&state.config, &address).await {
            Ok(balance) => Some(balance),
            Err(err) => {
                error!(
                    "Could not read recipient native balance before drip: {}",
                    err
                );
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    JsonResponse(DripResponse::error(
                        "The native balance service is temporarily unavailable; no funds were sent.",
                    )),
                );
            }
        }
    } else {
        None
    };

    // Check the resulting balance, not only the pre-transfer balance. This
    // prevents a fixed drip from carrying a recipient over the configured cap.
    let should_send_token = token_balance
        .checked_add(state.config.drip_amount)
        .is_some_and(|post_balance| post_balance <= state.config.max_balance);
    let should_send_native = if state.config.native_drip_amount == 0 {
        false
    } else {
        native_balance
            .and_then(|balance| balance.checked_add(state.config.native_drip_amount))
            .is_some_and(|post_balance| post_balance <= state.config.native_max_balance)
    };

    if should_send_token && !state.token_claims_enabled() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            JsonResponse(DripResponse::error(
                "The faucet distribution gate, ledger, or governed token policy is not ready; no tCREG was sent.",
            )),
        );
    }

    if should_send_token {
        match get_token_balance(&state.config, &state.config.faucet_address).await {
            Ok(faucet_balance)
                if faucet_balance
                    <= state.config.token_hot_wallet_float_wei.unwrap_or_default()
                    && faucet_balance >= state.config.drip_amount => {}
            Ok(faucet_balance)
                if faucet_balance > state.config.token_hot_wallet_float_wei.unwrap_or_default() =>
            {
                return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        JsonResponse(DripResponse::error(
                            "Faucet balance exceeds the approved hot-wallet float; reconcile it with Governance before serving claims.",
                        )),
                    );
            }
            Ok(_) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    JsonResponse(DripResponse::error(
                        "Faucet is out of tCREG. An operator must fund the faucet wallet before more token drips are possible.",
                    )),
                );
            }
            Err(err) => {
                error!("Could not read faucet token balance before drip: {}", err);
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    JsonResponse(DripResponse::error(
                        "The faucet balance service is temporarily unavailable; no funds were sent.",
                    )),
                );
            }
        }
    }

    if should_send_native {
        match get_native_balance(&state.config, &state.config.faucet_address).await {
            Ok(faucet_native) if faucet_native >= state.config.native_drip_amount => {}
            Ok(_) => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    JsonResponse(DripResponse::error(
                        "Faucet is out of testnet ETH for gas drips. Fund the faucet wallet with Sepolia ETH.",
                    )),
                );
            }
            Err(err) => {
                error!("Could not read faucet native balance before drip: {}", err);
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    JsonResponse(DripResponse::error(
                        "The faucet native balance service is temporarily unavailable; no funds were sent.",
                    )),
                );
            }
        }
    }

    if !should_send_token && !should_send_native {
        let token_msg = format!("{} tCREG", token_balance / 10_u128.pow(18));
        let native_msg = native_balance
            .map(|balance| format!("{:.4} ETH", balance as f64 / 10_f64.powi(18)))
            .unwrap_or_else(|| "sufficient testnet ETH".to_string());
        return (
            StatusCode::BAD_REQUEST,
            JsonResponse(DripResponse {
                success: false,
                message: format!(
                    "Address already has enough test funds for now ({}, {}).",
                    token_msg, native_msg
                ),
                error: None,
                tx_hash: None,
                amount: None,
                retry_after_seconds: None,
                cooldown_seconds: None,
                token_tx_hash: None,
                native_tx_hash: None,
                token_amount: None,
                native_amount: None,
            }),
        );
    }

    // Reserve durable address/IP cooldowns and token budgets in one transaction
    // before any transfer. Reserved amounts continue to count against policy
    // after a process restart, including when a transfer outcome needs review.
    let claim_id = match state
        .rate_limiter
        .claim(
            &address,
            &client_ip,
            Duration::from_secs(state.config.cooldown_secs),
            Duration::from_secs(state.config.ip_cooldown_secs),
            if should_send_token {
                state.config.drip_amount
            } else {
                0
            },
            if should_send_native {
                state.config.native_drip_amount
            } else {
                0
            },
            state.config.token_allocation_wei,
            state.config.token_daily_cap_wei,
            state.config.token_address_lifetime_cap_wei,
        )
        .await
    {
        Ok(Ok(claim_id)) => claim_id,
        Ok(Err(rejection)) => {
            let reason = if rejection.message.starts_with("IP rate limit:") {
                "ip"
            } else if rejection.message.contains("allocation")
                || rejection.message.contains("distribution cap")
                || rejection.message.contains("lifetime tCREG")
            {
                "budget"
            } else {
                "address"
            };
            counter!("faucet_rate_limited_total", "reason" => reason).increment(1);
            return (
                StatusCode::TOO_MANY_REQUESTS,
                JsonResponse(DripResponse {
                    retry_after_seconds: Some(rejection.retry_after_seconds),
                    cooldown_seconds: Some(rejection.cooldown_seconds),
                    ..DripResponse::error(rejection.message)
                }),
            );
        }
        Err(err) => {
            error!("Durable faucet claim reservation failed: {}", err);
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                JsonResponse(DripResponse::error(
                    "Faucet accounting storage is unavailable; no funds were sent.",
                )),
            );
        }
    };

    let mut token_tx_hash = None;
    let mut native_tx_hash = None;
    let mut parts = Vec::new();
    let mut failures = Vec::new();

    if should_send_native {
        match execute_native_transfer(&state.config, &state.rate_limiter, claim_id, &address).await
        {
            Ok(tx_hash) => {
                if let Err(err) = state
                    .rate_limiter
                    .confirm_native_transfer(claim_id, &tx_hash)
                    .await
                {
                    error!(
                        "Native faucet transfer {} confirmed but durable ledger update failed: {}",
                        tx_hash, err
                    );
                    failures.push(
                        "native transfer confirmed; durable ledger update needs reconciliation"
                            .to_string(),
                    );
                }
                match get_native_balance(&state.config, &address).await {
                    Ok(post_balance) if post_balance <= state.config.native_max_balance => {}
                    Ok(post_balance) => {
                        warn!(
                            "Confirmed faucet native transfer {} left recipient {} at {} wei, above configured cap {} wei",
                            tx_hash, address, post_balance, state.config.native_max_balance
                        );
                        failures.push(
                            "confirmed native transfer exceeded the recipient balance cap"
                                .to_string(),
                        );
                    }
                    Err(err) => {
                        warn!("Could not verify recipient balance after confirmed native transfer {}: {}", tx_hash, err);
                        failures.push("confirmed native transfer; post-transfer balance could not be verified".to_string());
                    }
                }
                parts.push(format!(
                    "{:.4} ETH for gas",
                    state.config.native_drip_amount as f64 / 10_f64.powi(18)
                ));
                native_tx_hash = Some(tx_hash);
                counter!("faucet_native_drips_total").increment(1);
            }
            Err(err) => {
                error!("Native gas drip failed: {}", err);
                failures.push(format!("native ETH: {}", err));
                counter!("faucet_failures_total", "kind" => "native").increment(1);
            }
        }
    }

    if should_send_token {
        match execute_token_transfer(&state.config, &state.rate_limiter, claim_id, &address).await {
            Ok(tx_hash) => {
                if let Err(err) = state
                    .rate_limiter
                    .confirm_token_transfer(claim_id, &tx_hash)
                    .await
                {
                    error!(
                        "tCREG faucet transfer {} confirmed but durable ledger update failed: {}",
                        tx_hash, err
                    );
                    failures.push(
                        "tCREG transfer confirmed; durable ledger update needs reconciliation"
                            .to_string(),
                    );
                }
                match get_token_balance(&state.config, &address).await {
                    Ok(post_balance) if post_balance <= state.config.max_balance => {}
                    Ok(post_balance) => {
                        warn!(
                            "Confirmed faucet token transfer {} left recipient {} at {} wei, above configured cap {} wei",
                            tx_hash, address, post_balance, state.config.max_balance
                        );
                        failures.push(
                            "confirmed token transfer exceeded the recipient balance cap"
                                .to_string(),
                        );
                    }
                    Err(err) => {
                        warn!(
                            "Could not verify recipient balance after confirmed transfer {}: {}",
                            tx_hash, err
                        );
                        failures.push(
                            "confirmed token transfer; post-transfer balance could not be verified"
                                .to_string(),
                        );
                    }
                }
                parts.push(format!(
                    "{} tCREG",
                    state.config.drip_amount / 10_u128.pow(18)
                ));
                token_tx_hash = Some(tx_hash);
                counter!("faucet_token_drips_total").increment(1);
            }
            Err(err) => {
                error!("Token drip failed: {}", err);
                failures.push(format!("tCREG: {}", err));
                counter!("faucet_failures_total", "kind" => "token").increment(1);
            }
        }
    }

    if token_tx_hash.is_some() || native_tx_hash.is_some() {
        counter!("faucet_drips_total").increment(1);

        info!(
            "Dripped {} to {}{}{}",
            parts.join(" + "),
            address,
            token_tx_hash
                .as_ref()
                .map(|tx| format!(" (token tx: {})", tx))
                .unwrap_or_default(),
            native_tx_hash
                .as_ref()
                .map(|tx| format!(" (gas tx: {})", tx))
                .unwrap_or_default()
        );

        let token_amount = token_tx_hash
            .as_ref()
            .map(|_| state.config.drip_amount / 10_u128.pow(18));
        let native_amount = native_tx_hash.as_ref().map(|_| {
            format!(
                "{:.4}",
                state.config.native_drip_amount as f64 / 10_f64.powi(18)
            )
        });
        (
            StatusCode::OK,
            JsonResponse(DripResponse {
                success: true,
                message: if failures.is_empty() {
                    format!("Sent {}.", parts.join(" + "))
                } else {
                    format!(
                        "Sent {}. Partial issue: {}",
                        parts.join(" + "),
                        failures.join("; ")
                    )
                },
                error: None,
                tx_hash: token_tx_hash.clone().or_else(|| native_tx_hash.clone()),
                amount: Some(parts.join(" + ")),
                retry_after_seconds: None,
                cooldown_seconds: Some(state.config.cooldown_secs),
                token_tx_hash,
                native_tx_hash,
                token_amount: token_amount.map(|amount| amount.to_string()),
                native_amount,
            }),
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            JsonResponse(DripResponse {
                success: false,
                message: if failures.is_empty() {
                    "Faucet transfer failed for an unknown reason.".to_string()
                } else {
                    format!("Faucet transfer failed: {}", failures.join("; "))
                },
                error: Some(if failures.is_empty() {
                    "unknown_error".to_string()
                } else {
                    failures.join("; ")
                }),
                tx_hash: None,
                amount: None,
                retry_after_seconds: None,
                cooldown_seconds: None,
                token_tx_hash: None,
                native_tx_hash: None,
                token_amount: None,
                native_amount: None,
            }),
        )
    }
}

fn supply_holder_address(configured: &str, name: &str) -> anyhow::Result<String> {
    let value = configured.trim();
    value.parse::<Address>().with_context(|| {
        format!("invalid or missing {name} address for public supply reporting")
    })?;
    Ok(value.to_ascii_lowercase())
}

async fn supply_holder_balance(
    client: &reqwest::Client,
    config: &FaucetConfig,
    token_address: &str,
    holder_address: &str,
    block_tag: &str,
) -> anyhow::Result<u128> {
    let data = format!("0x70a08231{:0>64}", holder_address.trim_start_matches("0x"));
    eth_call_u128(client, config, token_address, &data, block_tag).await
}

async fn load_verified_public_supply(state: &AppState) -> anyhow::Result<serde_json::Value> {
    if !state.ledger_reconciled() {
        anyhow::bail!("faucet ledger is not reconciled");
    }
    let config = &state.config;
    let treasury = supply_holder_address(&config.treasury_address, "treasury")?;
    let governance = supply_holder_address(&config.governance_address, "governance")?;
    let staking = supply_holder_address(&config.staking_address, "staking")?;
    let faucet = supply_holder_address(&config.faucet_address, "faucet")?;
    let token = supply_holder_address(&config.token_contract, "token contract")?;
    let chain_id = i64::try_from(config.chain_id).context("chain ID exceeds database range")?;
    let (block, saved_hash, faucet_balance, total_supply, reconciled_at) = state
        .rate_limiter
        .reconciled_supply_snapshot(chain_id, &token)
        .await?
        .context("no verified supply snapshot is available")?;
    let block = u64::try_from(block).context("verified supply block is negative")?;
    if total_supply > MAX_TESTNET_TOKEN_SUPPLY_WEI {
        anyhow::bail!("verified total supply exceeds the testnet cap");
    }
    let block_tag = format!("0x{block:x}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .context("construct supply read client")?;
    let canonical = rpc_request(
        &client,
        config.history_rpc_url(),
        "eth_getBlockByNumber",
        serde_json::json!([block_tag.clone(), false]),
    )
    .await?;
    let canonical_hash = canonical
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .context("verified supply block has no canonical hash")?;
    if !canonical_hash.eq_ignore_ascii_case(&saved_hash) {
        anyhow::bail!("verified supply block is no longer canonical");
    }
    let treasury_balance =
        supply_holder_balance(&client, config, &token, &treasury, &block_tag).await?;
    let governance_balance =
        supply_holder_balance(&client, config, &token, &governance, &block_tag).await?;
    let staking_balance =
        supply_holder_balance(&client, config, &token, &staking, &block_tag).await?;
    let named_balance = [
        treasury_balance,
        faucet_balance,
        governance_balance,
        staking_balance,
    ]
    .into_iter()
    .try_fold(0_u128, |sum, balance| sum.checked_add(balance))
    .context("named supply balances overflow")?;
    let other_balance = total_supply
        .checked_sub(named_balance)
        .context("named supply balances exceed verified total supply")?;
    let capacity = MAX_TESTNET_TOKEN_SUPPLY_WEI
        .checked_sub(total_supply)
        .context("verified total supply exceeds the testnet cap")?;
    Ok(serde_json::json!({
        "status": "available",
        "source": "verified_production_ledger_and_exact_block_sepolia_reads",
        "chain_id": config.chain_id,
        "block_number": block,
        "block_hash": saved_hash,
        "reconciled_at": reconciled_at,
        "token_contract": token,
        "total_supply_wei": total_supply.to_string(),
        "max_outstanding_supply_wei": MAX_TESTNET_TOKEN_SUPPLY_WEI.to_string(),
        "remaining_mint_capacity_wei": capacity.to_string(),
        "treasury": { "address": treasury, "balance_wei": treasury_balance.to_string() },
        "faucet": { "address": faucet, "balance_wei": faucet_balance.to_string() },
        "governance": { "address": governance, "balance_wei": governance_balance.to_string() },
        "staking": { "address": staking, "balance_wei": staking_balance.to_string() },
        "other_holders_wei": other_balance.to_string(),
        "token_claims_enabled": state.token_claims_enabled(),
        "faucet_allocation_status": if state.token_claims_enabled() { "open" } else if config.token_claim_policy_ready() { "staged_not_open" } else { "pending_governed_policy" },
    }))
}

/// Show where the minted supply is held at one verified Sepolia block.
async fn get_supply(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut cache = state.supply_cache.lock().await;
    if let Some((at, result)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(if result.is_some() { 60 } else { 15 }) {
            return match result {
                Some(value) => (StatusCode::OK, JsonResponse(value.clone())),
                None => (
                    StatusCode::BAD_GATEWAY,
                    JsonResponse(
                        serde_json::json!({ "status": "unavailable", "error": "verified Sepolia supply data is temporarily unavailable" }),
                    ),
                ),
            };
        }
    }
    match load_verified_public_supply(&state).await {
        Ok(value) => {
            *cache = Some((Instant::now(), Some(value.clone())));
            (StatusCode::OK, JsonResponse(value))
        }
        Err(_) => {
            error!("Verified public supply read failed");
            *cache = Some((Instant::now(), None));
            (
                StatusCode::BAD_GATEWAY,
                JsonResponse(
                    serde_json::json!({ "status": "unavailable", "error": "verified Sepolia supply data is temporarily unavailable" }),
                ),
            )
        }
    }
}

/// Get faucet statistics
async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Report an unavailable chain read as unavailable; zero is a valid balance
    // and must not be fabricated when the RPC is failing.
    let faucet_balance = match get_token_balance(&state.config, &state.config.faucet_address).await
    {
        Ok(balance) => balance,
        Err(err) => {
            error!("Could not read faucet token balance for stats: {}", err);
            return (
                StatusCode::BAD_GATEWAY,
                JsonResponse(serde_json::json!({
                    "status": "unavailable",
                    "error": "could not read faucet token balance from the configured chain RPC"
                })),
            );
        }
    };
    let faucet_native_balance = match get_native_balance(
        &state.config,
        &state.config.faucet_address,
    )
    .await
    {
        Ok(balance) => balance,
        Err(err) => {
            error!("Could not read faucet native balance for stats: {}", err);
            return (
                StatusCode::BAD_GATEWAY,
                JsonResponse(serde_json::json!({
                    "status": "unavailable",
                    "error": "could not read faucet native balance from the configured chain RPC"
                })),
            );
        }
    };
    let stats = match state.rate_limiter.confirmed_stats().await {
        Ok(stats) => stats,
        Err(err) => {
            error!("Could not read durable faucet claim ledger: {}", err);
            return (
                StatusCode::BAD_GATEWAY,
                JsonResponse(serde_json::json!({
                    "status": "unavailable",
                    "error": "could not read durable faucet claim ledger"
                })),
            );
        }
    };

    (
        StatusCode::OK,
        JsonResponse(serde_json::json!({
        "status": "available",
        "stats_scope": "confirmed_receipts_and_canonical_logs_recorded_in_postgres",
        "ledger_reconciled": state.ledger_reconciled(),
        "token_claims_enabled": state.token_claims_enabled(),
        "token_claim_policy_ready": state.config.token_claim_policy_ready(),
        "token_distribution_open": state.config.token_distribution_open,
        "token_allocation_wei": state.config.token_allocation_wei.map(|value| value.to_string()),
        "token_daily_cap_wei": state.config.token_daily_cap_wei.map(|value| value.to_string()),
        "token_address_lifetime_cap_wei": state.config.token_address_lifetime_cap_wei.map(|value| value.to_string()),
        "token_hot_wallet_float_wei": state.config.token_hot_wallet_float_wei.map(|value| value.to_string()),
        "drip_amount": state.config.drip_amount.to_string(),
        "native_drip_amount": state.config.native_drip_amount.to_string(),
        "cooldown_seconds": state.config.cooldown_secs,
        "max_balance": state.config.max_balance.to_string(),
        "native_max_balance": state.config.native_max_balance.to_string(),
        "token_contract": state.config.token_contract,
        "faucet_address": state.config.faucet_address,
        "faucet_balance": faucet_balance.to_string(),
        "faucet_balance_formatted": format!("{:.2}", faucet_balance as f64 / 10_f64.powi(18)),
        "faucet_native_balance": faucet_native_balance.to_string(),
        "faucet_native_balance_formatted": format!("{:.4}", faucet_native_balance as f64 / 10_f64.powi(18)),
        "stats": stats,
        })),
    )
}

/// Get balance for address (REAL)
async fn get_balance(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(address): axum::extract::Path<String>,
) -> impl IntoResponse {
    info!(">>> get_balance request for address: {}", address);
    if address.parse::<Address>().is_err() {
        return (
            StatusCode::BAD_REQUEST,
            JsonResponse(serde_json::json!({"error": "invalid Ethereum address"})),
        );
    }
    let token_balance = get_token_balance(&state.config, &address).await;
    let native_balance = get_native_balance(&state.config, &address).await;

    match (token_balance, native_balance) {
        (Ok(balance), Ok(native)) => (
            StatusCode::OK,
            JsonResponse(serde_json::json!({
                "status": "available",
                "address": address,
                "balance": balance.to_string(),
                "balance_formatted": format!("{:.2}", balance as f64 / 10_f64.powi(18)),
                "token_balance": balance.to_string(),
                "token_balance_formatted": format!("{:.2}", balance as f64 / 10_f64.powi(18)),
                "native_balance": native.to_string(),
                "native_balance_formatted": format!("{:.4}", native as f64 / 10_f64.powi(18)),
            })),
        ),
        (token_result, native_result) => (
            StatusCode::BAD_GATEWAY,
            JsonResponse(serde_json::json!({
                "status": "unavailable",
                "address": address,
                "error": format!(
                    "token={}, native={}",
                    token_result.err().unwrap_or_else(|| "ok".to_string()),
                    native_result.err().unwrap_or_else(|| "ok".to_string())
                ),
            })),
        ),
    }
}

fn chain_display_name(chain_id: u64) -> &'static str {
    match chain_id {
        11155111 => "CREG Testnet (Sepolia)",
        31337 => "CREG Testnet (Anvil)",
        _ => "CREG Testnet",
    }
}

/// Network configuration info for wallet setup and next-step guidance
async fn get_network_info(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let explorer_url = env_string("FAUCET_EXPLORER_URL", "http://localhost:3007");
    let rpc_url = env_string("FAUCET_PUBLIC_RPC_URL", "http://localhost:8545");
    let chain_id = env_u64("FAUCET_CHAIN_ID", 31337);

    JsonResponse(serde_json::json!({
        "chain_id": chain_id,
        "rpc_url": rpc_url,
        "token_contract": state.config.token_contract,
        "explorer_url": explorer_url,
        "chain_name": chain_display_name(chain_id),
        "currency": "ETH",
        "token_symbol": "tCREG",
        "native_currency_symbol": "ETH",
        "gas_note": "Gas on EVM testnets is paid in the native testnet ETH for that chain, not in ERC-20 tokens.",
    }))
}

/// Health check
async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if let Err(err) = state.rate_limiter.check_ready().await {
        error!("Faucet cooldown database health check failed: {}", err);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            JsonResponse(serde_json::json!({
                "status": "degraded",
                "faucet": "offline",
                "mode": "real",
                "error": "durable cooldown storage is unavailable",
            })),
        );
    }

    let health_timeout = Duration::from_secs(6);
    let (token_result, native_result) = tokio::join!(
        tokio::time::timeout(
            health_timeout,
            get_token_balance(&state.config, &state.config.faucet_address),
        ),
        tokio::time::timeout(
            health_timeout,
            get_native_balance(&state.config, &state.config.faucet_address),
        ),
    );
    let token_result = token_result.unwrap_or_else(|_| Err("RPC request timed out".to_string()));
    let native_result = native_result.unwrap_or_else(|_| Err("RPC request timed out".to_string()));
    match (token_result, native_result) {
        (Ok(faucet_balance), Ok(faucet_native_balance)) => {
            let token_funded = faucet_balance >= state.config.drip_amount;
            let ledger_reconciled = state.ledger_reconciled();
            let token_policy_ready = state.config.token_claim_policy_ready();
            let token_ready = token_funded && state.token_claims_enabled();
            let native_ready = state.config.native_drip_amount == 0
                || faucet_native_balance >= state.config.native_drip_amount;
            let fully_ready = token_ready && native_ready;
            // Liveness is separate from claim readiness. Return 200 while the
            // process can reach its database and chain RPC, even if governed
            // token claims are deliberately disabled pending reconciliation.
            let status_code = StatusCode::OK;
            let status = if fully_ready { "healthy" } else { "degraded" };
            let mut body = serde_json::json!({
                "status": status,
                "faucet": "online",
                "mode": "real",
                "faucet_balance": faucet_balance.to_string(),
                "faucet_native_balance": faucet_native_balance.to_string(),
                "token_drips_available": token_ready,
                "token_wallet_funded": token_funded,
                "token_policy_ready": token_policy_ready,
                "token_distribution_open": state.config.token_distribution_open,
                "ledger_reconciled": ledger_reconciled,
                "token_claims_enabled": state.token_claims_enabled(),
                "native_drips_available": native_ready,
            });
            if !token_funded {
                body["warning"] = serde_json::json!(
                    "Faucet wallet has insufficient tCREG; an approved Governance reserve tranche is required"
                );
            } else if !token_policy_ready {
                body["warning"] = serde_json::json!(
                    "Token claims are disabled until allocation caps are configured and the durable ledger is reconciled"
                );
            } else if !state.config.token_distribution_open {
                body["warning"] = serde_json::json!(
                    "Token policy is staged; public distribution has not been opened"
                );
            }
            if !native_ready {
                body["warning_native"] =
                    serde_json::json!("Faucet wallet has insufficient Sepolia ETH for gas drips");
            }
            (status_code, JsonResponse(body))
        }
        (token_result, native_result) => (
            StatusCode::SERVICE_UNAVAILABLE,
            JsonResponse(serde_json::json!({
                "status": "degraded",
                "faucet": "offline",
                "mode": "real",
                "error": "one or more live chain RPC checks failed",
                "token_rpc": if token_result.is_err() { "unavailable" } else { "available" },
                "native_rpc": if native_result.is_err() { "unavailable" } else { "available" },
                "ledger_reconciled": state.ledger_reconciled(),
                "token_claims_enabled": state.token_claims_enabled(),
                "token_drips_available": false,
                "native_drips_available": false,
            })),
        ),
    }
}

// ── Admin endpoints ───────────────────────────────────────────────────────────

/// Verify the `Authorization: Bearer <token>` header against `FAUCET_ADMIN_TOKEN`.
/// Returns `Err((status, json))` when authentication fails so handlers can
/// `return err` immediately.
fn check_admin_auth(
    headers: &HeaderMap,
) -> Result<(), (StatusCode, axum::response::Json<serde_json::Value>)> {
    let token = std::env::var("FAUCET_ADMIN_TOKEN").unwrap_or_default();
    if token.is_empty() {
        return Err((
            StatusCode::FORBIDDEN,
            axum::response::Json(serde_json::json!({
                "error": "Admin endpoints disabled — set FAUCET_ADMIN_TOKEN to enable them"
            })),
        ));
    }
    let provided = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if provided != format!("Bearer {}", token) {
        return Err((
            StatusCode::UNAUTHORIZED,
            axum::response::Json(serde_json::json!({"error": "Unauthorized"})),
        ));
    }
    Ok(())
}

/// `POST /admin/pause` — stop accepting new drip requests.
async fn admin_pause(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(e) = check_admin_auth(&headers) {
        return e;
    }
    state.is_paused.store(true, Ordering::Relaxed);
    warn!("Faucet PAUSED by operator request");
    (
        StatusCode::OK,
        axum::response::Json(serde_json::json!({"status": "paused"})),
    )
}

/// `POST /admin/resume` — re-enable drip requests after a pause.
async fn admin_resume(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(e) = check_admin_auth(&headers) {
        return e;
    }
    state.is_paused.store(false, Ordering::Relaxed);
    info!("Faucet RESUMED by operator request");
    (
        StatusCode::OK,
        axum::response::Json(serde_json::json!({"status": "running"})),
    )
}

/// `GET /admin/status` — live operational snapshot (address count, pause state, stats).
async fn admin_status(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(e) = check_admin_auth(&headers) {
        return e;
    }
    let address_count = state.rate_limiter.address_count().await;
    let stats = match state.rate_limiter.confirmed_stats().await {
        Ok(stats) => stats,
        Err(err) => {
            error!(
                "Could not read durable faucet ledger for operator status: {}",
                err
            );
            return (
                StatusCode::BAD_GATEWAY,
                axum::response::Json(serde_json::json!({
                    "status": "unavailable",
                    "error": "could not read durable faucet claim ledger"
                })),
            );
        }
    };
    (
        StatusCode::OK,
        axum::response::Json(serde_json::json!({
            "is_paused": state.is_paused.load(Ordering::Relaxed),
            "stats_scope": "confirmed_receipts_and_canonical_logs_recorded_in_postgres",
            "ledger_reconciled": state.ledger_reconciled(),
            "token_claims_enabled": state.token_claims_enabled(),
            "address_count": address_count,
            "total_drips": stats.total_drips,
            "total_distributed_wei": stats.total_distributed.to_string(),
            "total_native_distributed_wei": stats.total_native_distributed.to_string(),
            "unique_addresses": stats.unique_addresses,
            "last_drip": stats.last_drip,
            "unresolved_claims": stats.unresolved_claims,
        })),
    )
}

/// `GET /admin/recovery` — inspect durable claims awaiting receipt or event recovery.
async fn admin_recovery(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(error) = check_admin_auth(&headers) {
        return error;
    }
    match state.rate_limiter.unresolved_claim_details().await {
        Ok(claims) => (
            StatusCode::OK,
            axum::response::Json(serde_json::json!({
                "status": if claims.is_empty() { "clear" } else { "recovery_required" },
                "claims": claims,
                "automatic_recovery": "submitted hashes are checked against receipts; hashless token reservations are matched against canonical faucet Transfer logs",
                "token_claims_enabled": state.token_claims_enabled(),
            })),
        ),
        Err(error) => {
            error!("Could not read unresolved faucet claim details: {}", error);
            (
                StatusCode::BAD_GATEWAY,
                axum::response::Json(serde_json::json!({
                    "status": "unavailable",
                    "error": "could not read unresolved faucet claims"
                })),
            )
        }
    }
}

// Helper functions
fn env_string(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_bool(key: &str, default: bool) -> anyhow::Result<bool> {
    match std::env::var(key) {
        Ok(value) if value.eq_ignore_ascii_case("true") => Ok(true),
        Ok(value) if value.eq_ignore_ascii_case("false") => Ok(false),
        Ok(_) => anyhow::bail!("{key} must be true or false"),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error).with_context(|| format!("read {key}")),
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_optional_u64(key: &str) -> anyhow::Result<Option<u64>> {
    match std::env::var(key) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => value
            .parse::<u64>()
            .map(Some)
            .with_context(|| format!("{key} must be a non-negative integer")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {key}")),
    }
}

fn env_u128(key: &str, default: u128) -> u128 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_optional_u128(key: &str) -> anyhow::Result<Option<u128>> {
    match std::env::var(key) {
        Ok(value) => value
            .parse::<u128>()
            .map(Some)
            .with_context(|| format!("{key} must be an unsigned integer in wei")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(anyhow::anyhow!("Could not read {key}: {err}")),
    }
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_ipnets(key: &str) -> anyhow::Result<Vec<IpNet>> {
    let value = std::env::var(key).unwrap_or_default();
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .parse::<IpNet>()
                .map_err(|err| anyhow::anyhow!("invalid CIDR in {key}: {entry}: {err}"))
        })
        .collect()
}

fn client_ip_from_request(
    peer_ip: IpAddr,
    headers: &HeaderMap,
    trusted_proxy_cidrs: &[IpNet],
) -> String {
    let is_trusted = |address: &IpAddr| trusted_proxy_cidrs.iter().any(|net| net.contains(address));
    if !is_trusted(&peer_ip) {
        return peer_ip.to_string();
    }

    // Walk the forwarding chain from the trusted peer toward the client. This
    // skips trusted proxy hops and returns the first untrusted address, while
    // ignoring all forwarding headers when the socket peer is not trusted.
    let forwarded: Vec<IpAddr> = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .into_iter()
        .flat_map(|value| value.split(','))
        .filter_map(|value| value.trim().parse().ok())
        .collect();
    let mut client = peer_ip;
    for hop in forwarded.iter().rev() {
        if !is_trusted(&client) {
            break;
        }
        client = *hop;
    }

    client.to_string()
}
