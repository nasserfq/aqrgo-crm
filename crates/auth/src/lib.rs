//! Auth module: sign-in with an emailed 6-digit code or Google, sessions,
//! and linking one Telegram and one WhatsApp account to each user.
//!
//! Public API: [`AuthService`], [`AuthConfig`], [`AuthError`], [`CodeMailer`],
//! [`web`] (login pages, Google OAuth, the `require_user` middleware), and
//! [`api`] (JSON sign-in with bearer tokens for the mobile apps).
//! Owns the `auth` Postgres schema. Other modules only ever see
//! `common::CurrentUser`.

pub mod api;
pub mod handoff;
mod service;
pub mod web;

pub use service::{
    login_code_email, AuthConfig, AuthError, AuthService, Channel, CodeMailer, MailerCodeSender,
};

pub const SCHEMA: &str = "auth";
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
