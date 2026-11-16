//! Hands a session from the browser to the phone app after Google sign-in.
//!
//! The app opens `/auth/google?app=<challenge>` in the system browser, where
//! `challenge` is the hex SHA-256 of a random `verifier` the app keeps. After
//! Google, the server sends the browser to `aqrgo://auth?code=<code>` with a
//! one-time code instead of the session token. The app then trades code and
//! verifier for the token at `/api/v1/auth/google`. Another app that grabs the
//! redirect has the code but not the verifier, so it can't use it (the same idea
//! as OAuth PKCE).
//!
//! Codes live in memory for two minutes; a restart just means signing in again.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use rand::{distributions::Alphanumeric, Rng};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const TTL: Duration = Duration::from_secs(120);

struct Pending {
    session: String,
    challenge: String,
    issued: Instant,
}

#[derive(Clone, Default)]
pub struct AppHandoff {
    pending: Arc<Mutex<HashMap<String, Pending>>>,
}

/// A challenge is the lowercase hex SHA-256 of the app's verifier.
pub fn valid_challenge(c: &str) -> bool {
    c.len() == 64 && c.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl AppHandoff {
    /// Stores a signed-in session for the app and returns the one-time code.
    pub fn issue(&self, session: String, challenge: String) -> String {
        let code: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(40)
            .map(char::from)
            .collect();
        let mut pending = self.pending.lock().expect("handoff lock");
        pending.retain(|_, p| p.issued.elapsed() < TTL);
        pending.insert(
            code.clone(),
            Pending {
                session,
                challenge,
                issued: Instant::now(),
            },
        );
        code
    }

    /// Trades a code and the app's verifier for the session token, once.
    pub fn redeem(&self, code: &str, verifier: &str) -> Option<String> {
        let p = self.pending.lock().expect("handoff lock").remove(code)?;
        if p.issued.elapsed() >= TTL {
            return None;
        }
        let actual = hex::encode(Sha256::digest(verifier.as_bytes()));
        if bool::from(actual.as_bytes().ct_eq(p.challenge.as_bytes())) {
            Some(p.session)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn challenge(verifier: &str) -> String {
        hex::encode(Sha256::digest(verifier.as_bytes()))
    }

    #[test]
    fn the_app_with_the_verifier_gets_the_session_once() {
        let h = AppHandoff::default();
        let code = h.issue("sess".into(), challenge("secret-verifier"));
        assert_eq!(h.redeem(&code, "secret-verifier").as_deref(), Some("sess"));
        assert_eq!(h.redeem(&code, "secret-verifier"), None, "one use only");
    }

    #[test]
    fn a_wrong_verifier_or_unknown_code_gets_nothing() {
        let h = AppHandoff::default();
        let code = h.issue("sess".into(), challenge("secret-verifier"));
        assert_eq!(h.redeem(&code, "guess"), None);
        assert_eq!(
            h.redeem(&code, "secret-verifier"),
            None,
            "a failed try burns the code"
        );
        assert_eq!(h.redeem("nope", "secret-verifier"), None);
    }

    #[test]
    fn challenges_must_be_hex_sha256() {
        assert!(valid_challenge(&challenge("x")));
        assert!(!valid_challenge("abc"));
        assert!(!valid_challenge(&"Z".repeat(64)));
    }
}
