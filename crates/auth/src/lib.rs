//! Auth module: sign-in with an emailed 6-digit code or Google, sessions,
//! and linking one Telegram account to each user.
//!
//! Public API: [`AuthService`], [`AuthConfig`], [`AuthError`], [`CodeMailer`],
//! and [`web`] (login pages, Google OAuth, the `require_user` middleware).
//! Owns the `auth` Postgres schema. Other modules only ever see
//! `common::CurrentUser`.

mod service;
pub mod web;

pub use service::{
    login_code_email, AuthConfig, AuthError, AuthService, CodeMailer, MailerCodeSender,
};

pub const SCHEMA: &str = "auth";
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
