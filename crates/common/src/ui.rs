//! Helpers shared by the server-rendered pages of every module.

use askama::Template;

/// Renders an Askama template into an HTML string, or a plain error page.
pub fn render<T: Template>(t: &T) -> Result<String, String> {
    t.render().map_err(|e| format!("template error: {e}"))
}

/// Navigation item highlighted in the shared layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    Listings,
    Account,
    None,
}

/// The shared stylesheet, served at `/static/app.css`.
pub const CSS: &str = include_str!("../static/app.css");

/// A fingerprint of [`CSS`]. Pages link `app.css?v={CSS_VERSION}` so a new
/// deploy is picked up at once while browsers may cache each version forever.
pub const CSS_VERSION: u64 = fnv1a(CSS.as_bytes());

const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
        i += 1;
    }
    hash
}
