//! Test helpers: a fresh, empty Postgres database per test.
//! Set TEST_DATABASE_URL to a server where the user may CREATE DATABASE,
//! e.g. postgres://postgres@localhost:5433/postgres

use rand::Rng;
use sqlx::{postgres::PgPoolOptions, Connection, PgConnection, PgPool};

pub async fn fresh_pool() -> PgPool {
    let admin_url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres@localhost:5433/postgres".into());
    let name = format!("t_{:016x}", rand::thread_rng().gen::<u64>());
    let mut admin = PgConnection::connect(&admin_url)
        .await
        .expect("connect to TEST_DATABASE_URL");
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&mut admin)
        .await
        .expect("create test db");
    let (base, _) = admin_url.rsplit_once('/').expect("url has a database part");
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&format!("{base}/{name}"))
        .await
        .expect("connect to test db")
}
