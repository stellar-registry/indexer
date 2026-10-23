//! Scratch-database setup shared by the DB-backed tests. Each test drops
//! and recreates the v1/archive schemas, so they hold LOCK to run one at a
//! time against TEST_DATABASE_URL.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::const_new(());

/// Applies the Goldsky table stand-ins, post_init.sql and `seed`. None
/// unless TEST_DATABASE_URL is set.
pub async fn pool(seed: &str) -> Option<(PgPool, MutexGuard<'static, ()>)> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let guard = LOCK.lock().await;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA IF EXISTS v1 CASCADE; DROP SCHEMA IF EXISTS archive CASCADE;")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../tests/fixtures/goldsky_tables.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../../goldsky/v1/post_init.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(seed).execute(&pool).await.unwrap();
    Some((pool, guard))
}
