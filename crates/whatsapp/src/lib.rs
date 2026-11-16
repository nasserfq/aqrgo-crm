//! WhatsApp module: the lead bot on WhatsApp, through Meta's WhatsApp Cloud
//! API directly (no reseller), so replies inside the 24-hour window are free.
//!
//! It runs the same conversation as Telegram ([`telegram::Bot`]): the
//! [`webhook`] checks Meta's signature and queues each incoming message as a
//! background job, and [`cloud::CloudOutbox`] turns the bot's replies into
//! WhatsApp text, reply-button and list messages. Owns the `whatsapp` schema.

pub mod cloud;
pub mod webhook;

pub const SCHEMA: &str = "whatsapp";
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
