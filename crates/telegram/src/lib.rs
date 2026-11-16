//! Telegram module: the bot conversation.
//!
//! [`Bot::handle`] is the whole conversation, written against the
//! [`Outbox`] trait so it can be tested without Telegram. [`webhook`]
//! receives Telegram updates, queues them as background jobs, and replies
//! through the real Bot API. Owns the `telegram` Postgres schema.

mod bot;
mod store;
pub mod webhook;

pub use bot::{Bot, Buttons, Incoming, Outbox};

pub const SCHEMA: &str = "telegram";
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Deletes a user's unconfirmed drafts (their account is being deleted).
pub async fn forget_owner(pool: &sqlx::PgPool, owner: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM telegram.drafts WHERE owner_id = $1")
        .bind(owner)
        .execute(pool)
        .await?;
    Ok(())
}
