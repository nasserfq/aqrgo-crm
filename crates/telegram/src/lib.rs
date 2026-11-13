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
