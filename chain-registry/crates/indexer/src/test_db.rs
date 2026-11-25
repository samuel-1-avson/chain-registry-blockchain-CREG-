use std::{
    future::Future,
    sync::atomic::{AtomicU32, Ordering},
};

use anyhow::{Context, Result};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};

static NEXT_SCHEMA: AtomicU32 = AtomicU32::new(0);

pub async fn isolate<F, Fut>(test: F) -> Result<()>
where
    F: FnOnce(PgPool) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let url =
        std::env::var("CREG_INDEXER_TEST_DATABASE_URL").context("test database URL is required")?;
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let schema = format!(
        "creg_indexer_test_{}_{}",
        std::process::id(),
        NEXT_SCHEMA.fetch_add(1, Ordering::Relaxed)
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await?;
    let options = url
        .parse::<PgConnectOptions>()?
        .options([("search_path", &schema)]);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    let result = test(pool.clone()).await;
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await?;
    result
}
