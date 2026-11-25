use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{ensure, Context, Result};
use creg_indexer::{
    api::ApiState,
    apply::{apply_block, indexed_hash},
    bounty::BountyLane,
    source::{Local, Remote},
    MIGRATOR,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tokio::time::MissedTickBehavior;
use tracing_subscriber::EnvFilter;

const BLOCKS_PER_TICK: u64 = 128;

enum Source {
    Local(Arc<Local>),
    Remote(Box<Remote>),
}

impl Source {
    async fn open() -> Result<Self> {
        if let Ok(url) = std::env::var("CREG_INDEXER_REMOTE_URL") {
            if !url.trim().is_empty() {
                return Ok(Self::Remote(Box::new(Remote::connect(&url).await?)));
            }
        }
        let data_dir = std::env::var_os("CREG_INDEXER_DATA_DIR")
            .or_else(|| std::env::var_os("CREG_DATA_DIR"))
            .map_or_else(|| PathBuf::from("/data"), PathBuf::from);
        let secondary_dir = std::env::var_os("CREG_INDEXER_SECONDARY_DIR").map_or_else(
            || std::env::temp_dir().join("creg-indexer-secondary"),
            PathBuf::from,
        );
        let local =
            tokio::task::spawn_blocking(move || Local::open(&data_dir, &secondary_dir)).await??;
        Ok(Self::Local(Arc::new(local)))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let pg_url = std::env::var("CREG_PG_URL").context("CREG_PG_URL must be set")?;
    let addr: SocketAddr = std::env::var("CREG_INDEXER_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8084".into())
        .parse()
        .context("invalid CREG_INDEXER_LISTEN")?;
    let interval_secs = std::env::var("CREG_INDEXER_POLL_INTERVAL_SECS")
        .unwrap_or_else(|_| "1".into())
        .parse::<u64>()
        .context("invalid CREG_INDEXER_POLL_INTERVAL_SECS")?;
    ensure!(
        interval_secs > 0,
        "CREG_INDEXER_POLL_INTERVAL_SECS must be positive"
    );
    let interval = Duration::from_secs(interval_secs);
    let source = Source::open().await?;
    let pool = PgPoolOptions::new()
        .max_connections(12)
        .connect(&pg_url)
        .await
        .context("connect to indexer PostgreSQL")?;
    MIGRATOR
        .run(&pool)
        .await
        .context("migrate indexer PostgreSQL")?;

    let bounty = BountyLane::from_env(&pool).await?;
    let chain_ready = Arc::new(AtomicBool::new(false));
    let bounty_ready = Arc::new(AtomicBool::new(bounty.is_none()));
    let app = ApiState::new(pool.clone(), chain_ready.clone(), bounty_ready.clone()).router();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "indexer serving");
    let chain = run_chain(source, pool.clone(), interval, chain_ready);
    let l1 = async {
        if let Some(lane) = bounty {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let result = lane.tick(&pool).await;
                bounty_ready.store(result.is_ok(), Ordering::Release);
                if let Err(error) = result {
                    tracing::warn!(%error, "L1 bounty ingestion paused");
                }
            }
        }
        std::future::pending::<Result<()>>().await
    };
    tokio::select! {
        result = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()) => result?,
        result = chain => result?,
        result = l1 => result?,
        result = tokio::signal::ctrl_c() => result?,
    }
    Ok(())
}

async fn run_chain(
    source: Source,
    pool: PgPool,
    interval: Duration,
    ready: Arc<AtomicBool>,
) -> Result<()> {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        match tick(&source, &pool).await {
            Ok(replayed) => ready.store(replayed, Ordering::Release),
            Err(error) => {
                ready.store(false, Ordering::Release);
                tracing::warn!(%error, "chain ingestion paused");
            }
        }
    }
}

async fn tick(source: &Source, pool: &PgPool) -> Result<bool> {
    let (last, replay_through): (i64, i64) =
        sqlx::query_as("SELECT last_height,replay_through FROM sync_state WHERE id = 1")
            .fetch_one(pool)
            .await?;
    let current_hash = if last >= 0 {
        Some(
            indexed_hash(pool, last)
                .await?
                .context("indexed cursor lacks a block")?,
        )
    } else {
        None
    };
    let next = u64::try_from(last.checked_add(1).context("indexer height overflow")?)?;
    let max_batch = next.saturating_add(BLOCKS_PER_TICK - 1);

    let through = match source {
        Source::Local(local) => {
            // Do blocking RocksDB reads on a blocking thread against the same
            // secondary instance. The next tick is the only refresh point.
            let local = local.clone();
            let (end, blocks) = tokio::task::spawn_blocking(move || {
                let tip = local.catch_up()?;
                let end = tip.min(max_batch);
                if let Some(hash) = current_hash {
                    ensure!(tip >= next - 1, "source tip is below indexed cursor");
                    ensure!(
                        local.block(next - 1)?.hash() == hash,
                        "indexed hash conflict at height {}",
                        next - 1
                    );
                }
                let blocks = (next..=end)
                    .map(|height| local.block(height))
                    .collect::<Result<Vec<_>>>()?;
                Ok::<_, anyhow::Error>((end, blocks))
            })
            .await??;
            for block in blocks {
                apply_block(pool, &block).await?;
            }
            end
        }
        Source::Remote(remote) => {
            let tip = remote.tip().await?;
            let end = tip.min(max_batch);
            let authority = remote.authority(pool, last, end).await?;
            if let Some(hash) = current_hash {
                ensure!(tip >= next - 1, "source tip is below indexed cursor");
                let anchor = remote.block(next - 1, &authority).await?;
                ensure!(
                    anchor.hash() == hash,
                    "indexed hash conflict at height {}",
                    next - 1
                );
            }
            for height in next..=end {
                apply_block(pool, &remote.block(height, &authority).await?).await?;
            }
            end
        }
    };
    Ok(i64::try_from(through)? >= replay_through)
}
