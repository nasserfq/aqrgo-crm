//! What the window may open, and the sign-in links coming back from the
//! browser. Kept free of Tauri so it can be tested on its own.

use std::sync::OnceLock;

use sha2::{Digest, Sha256};
use url::Url;

/// The site the window shows.
pub const SITE: &str = "https://aqrgo.com";

/// The site in use: aqrgo.com, except that debug builds can point at a local
/// server with `AQRGO_SITE` (for end-to-end tests). Release builds ignore it.
pub fn site() -> &'static str {
    static CURRENT: OnceLock<String> = OnceLock::new();
    CURRENT.get_or_init(|| {
        #[cfg(debug_assertions)]
        if let Ok(s) = std::env::var("AQRGO_SITE") {
            if Url::parse(&s).is_ok() {
                return s.trim_end_matches('/').to_string();
            }
        }
        SITE.to_string()
    })
}
/// The app's own link, which Google sign-in returns to (`aqrgo-desktop://auth?code=…`).
pub const SCHEME: &str = "aqrgo-desktop";

/// Apple's sign-in pages, which run inside the window.
const APPLE_HOSTS: &[&str] = &["appleid.apple.com", "idmsa.apple.com", "iforgot.apple.com"];
/// Links the system hands to another app (mail, phone, WhatsApp, Telegram).
const SYSTEM_SCHEMES: &[&str] = &["mailto", "tel", "sms", "whatsapp", "tg"];

#[derive(Debug, PartialEq)]
pub enum Nav {
    /// Open it in the window.
    Stay,
    /// Open it in the default browser (or the app the system picks).
    Browser,
    /// Google sign-in: Google refuses embedded windows, so it runs in the
    /// browser and comes back through the app's link.
    Google,
    /// Ignore it.
    Block,
}

pub fn is_site(url: &Url) -> bool {
    let ours =
        url.scheme() == "https" && matches!(url.host_str(), Some("aqrgo.com" | "www.aqrgo.com"));
    ours || Url::parse(site()).is_ok_and(|s| s.origin() == url.origin())
}

/// The app's own start page (`tauri://localhost` on Mac and Linux,
/// `http(s)://tauri.localhost` on Windows).
fn is_local(url: &Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => url.host_str() == Some("tauri.localhost"),
        "about" | "data" | "blob" => true,
        _ => false,
    }
}

pub fn decide(url: &Url) -> Nav {
    if is_local(url) {
        return Nav::Stay;
    }
    if is_site(url) {
        let from_app = url.query_pairs().any(|(k, _)| k == "app");
        return if url.path() == "/auth/google" && !from_app {
            Nav::Google
        } else {
            Nav::Stay
        };
    }
    match url.scheme() {
        "https" if url.host_str().is_some_and(|h| APPLE_HOSTS.contains(&h)) => Nav::Stay,
        "http" | "https" => Nav::Browser,
        s if SYSTEM_SCHEMES.contains(&s) => Nav::Browser,
        _ => Nav::Block,
    }
}

/// A fresh secret for one Google sign-in, and the challenge sent with it.
pub fn new_verifier() -> (String, String) {
    let bytes: [u8; 32] = rand::random();
    let verifier = hex::encode(bytes);
    let challenge = hex::encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Where the browser starts Google sign-in for the desktop app.
pub fn google_url(challenge: &str) -> String {
    format!("{}/auth/google?app={challenge}&client=desktop", site())
}

#[derive(Debug, PartialEq)]
pub enum Link {
    /// Signed in with Google: trade this one-time code for a session.
    Code(String),
    /// Sign-in failed; show this error on the login page.
    Error(&'static str),
}

/// Reads `aqrgo-desktop://auth?code=…` or `…?error=…`. Any website can open
/// this link, so everything else is ignored and the code must look like one.
pub fn parse_link(url: &Url) -> Option<Link> {
    if url.scheme() != SCHEME || url.host_str() != Some("auth") {
        return None;
    }
    for (k, v) in url.query_pairs() {
        match (k.as_ref(), v.as_ref()) {
            ("code", c) => {
                let c = c.split('#').next().unwrap_or_default();
                let ok =
                    !c.is_empty() && c.len() <= 128 && c.bytes().all(|b| b.is_ascii_alphanumeric());
                return ok.then(|| Link::Code(c.to_string()));
            }
            ("error", "not_allowed") => return Some(Link::Error("not_allowed")),
            ("error", _) => return Some(Link::Error("google")),
            _ => {}
        }
    }
    None
}

/// Run in the window (on aqrgo.com): posts the code and verifier so the site
/// sets its session cookie. Both are plain letters and digits (checked).
pub fn handoff_script(code: &str, verifier: &str) -> Option<String> {
    let plain = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric());
    if !plain(code) || !plain(verifier) {
        return None;
    }
    Some(format!(
        "(function(){{var f=document.createElement('form');f.method='post';\
         f.action='/auth/app/session';f.style.display='none';\
         [['code','{code}'],['verifier','{verifier}']].forEach(function(p){{\
         var i=document.createElement('input');i.type='hidden';i.name=p[0];i.value=p[1];\
         f.appendChild(i);}});document.body.appendChild(f);f.submit();}})();"
    ))
}

/// Runs in every page: links that would open a new window (WhatsApp,
/// Telegram, maps) go through normal navigation instead, which sends
/// anything outside aqrgo.com to the browser.
pub const LINKS_SCRIPT: &str = r#"(function () {
  if (location.protocol === "tauri:" || location.hostname === "tauri.localhost") return;
  document.addEventListener("click", function (e) {
    var a = e.target && e.target.closest ? e.target.closest('a[target="_blank"]') : null;
    if (!a || !a.href) return;
    e.preventDefault();
    location.href = a.href;
  }, true);
  window.open = function (u) {
    if (u) location.href = new URL(u, location.href).href;
    return null;
  };
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn the_site_and_the_start_page_stay_in_the_window() {
        for s in [
            "https://aqrgo.com/",
            "https://aqrgo.com/listings/4",
            "https://aqrgo.com/export.csv?q=1",
            "https://aqrgo.com/auth/apple",
            "https://appleid.apple.com/auth/authorize?client_id=com.aqrgo.web",
            "tauri://localhost/index.html",
            "http://tauri.localhost/index.html",
            "about:blank",
        ] {
            assert_eq!(decide(&u(s)), Nav::Stay, "{s}");
        }
    }

    #[test]
    fn other_sites_and_apps_open_outside() {
        for s in [
            "https://wa.me/966500000000",
            "https://t.me/aqrgo_crm_bot",
            "https://maps.google.com/?q=riyadh",
            "https://nasser.aqrgo.com/",
            "http://aqrgo.com/",
            "mailto:nafa050@gmail.com",
            "tel:+966500000000",
        ] {
            assert_eq!(decide(&u(s)), Nav::Browser, "{s}");
        }
        for s in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ms-settings:display",
        ] {
            assert_eq!(decide(&u(s)), Nav::Block, "{s}");
        }
    }

    #[test]
    fn google_sign_in_moves_to_the_browser() {
        assert_eq!(decide(&u("https://aqrgo.com/auth/google")), Nav::Google);
        // Our own browser link is never caught again.
        let (_, challenge) = new_verifier();
        assert_eq!(decide(&u(&google_url(&challenge))), Nav::Stay);
        assert!(google_url(&challenge).ends_with("&client=desktop"));
    }

    #[test]
    fn the_challenge_is_the_hash_of_the_verifier() {
        let (v, c) = new_verifier();
        assert_eq!(v.len(), 64);
        assert_eq!(c, hex::encode(Sha256::digest(v.as_bytes())));
        assert_ne!(new_verifier().0, v);
    }

    #[test]
    fn reads_the_links_back_from_the_browser() {
        assert_eq!(
            parse_link(&u("aqrgo-desktop://auth?code=Abc123")),
            Some(Link::Code("Abc123".into()))
        );
        assert_eq!(
            parse_link(&u("aqrgo-desktop://auth?code=Abc123#")),
            Some(Link::Code("Abc123".into()))
        );
        assert_eq!(
            parse_link(&u("aqrgo-desktop://auth?error=google")),
            Some(Link::Error("google"))
        );
        assert_eq!(
            parse_link(&u("aqrgo-desktop://auth?error=not_allowed")),
            Some(Link::Error("not_allowed"))
        );
        assert_eq!(
            parse_link(&u("aqrgo-desktop://auth?error=%3Cscript%3E")),
            Some(Link::Error("google"))
        );
        for s in [
            "aqrgo-desktop://auth?code=a'%29%3Balert(1)",
            "aqrgo-desktop://auth?code=",
            "aqrgo-desktop://other?code=abc",
            "aqrgo://auth?code=abc",
        ] {
            assert_eq!(parse_link(&u(s)), None, "{s}");
        }
    }

    #[test]
    fn the_handoff_script_only_takes_plain_values() {
        let js = handoff_script("Abc123", "deadbeef").unwrap();
        assert!(js.contains("/auth/app/session"));
        assert!(js.contains("['code','Abc123']"));
        assert!(handoff_script("a'b", "deadbeef").is_none());
        assert!(handoff_script("abc", "").is_none());
    }
}
