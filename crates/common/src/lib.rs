//! Shared kernel: types and helpers every module may use.
//! Nothing here knows about listings, auth or Telegram.

pub mod dates;
pub mod db;
pub mod digits;
pub mod ui;

/// The signed-in person, put into request extensions by the `auth` module's
/// middleware so other modules can read it without depending on `auth`.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CurrentUser {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub is_admin: bool,
}

/// Escape text for Telegram's HTML parse mode.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
