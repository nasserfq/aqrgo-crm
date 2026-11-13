//! Each module owns a Postgres schema and its own migrations.
//! Migration history is kept per schema (`<schema>._sqlx_migrations`),
//! so modules can evolve, or be split out, independently.

use sqlx::{migrate::Migrator, PgPool};

pub async fn migrate_module(
    pool: &PgPool,
    schema: &str,
    migrator: &Migrator,
) -> Result<(), sqlx::Error> {
    assert!(
        schema.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
        "bad schema name"
    );
    let mut conn = pool.acquire().await?;
    sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
        .execute(&mut *conn)
        .await?;
    sqlx::query(&format!("SET search_path TO {schema}"))
        .execute(&mut *conn)
        .await?;
    migrator.run(&mut *conn).await?;
    // Do not hand a connection with a changed search_path back to the pool.
    sqlx::query("RESET search_path").execute(&mut *conn).await?;
    Ok(())
}
