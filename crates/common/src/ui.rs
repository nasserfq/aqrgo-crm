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
