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
//! Codes live two minutes in `auth.app_handoffs` (hashed), so a server restart
//! between Google and the app's redeem doesn't break the sign-in.

use rand::{distributions::Alphanumeric, Rng};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use subtle::ConstantTimeEq;

const TTL_SECONDS: i32 = 120;

#[derive(Clone)]
pub struct AppHandoff {
    pool: PgPool,
}

/// A challenge is the lowercase hex SHA-256 of the app's verifier.
pub fn valid_challenge(c: &str) -> bool {
    c.len() == 64 && c.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

impl AppHandoff {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Stores a signed-in session for the app and returns the one-time code.
    pub async fn issue(&self, session: String, challenge: String) -> Result<String, sqlx::Error> {
        let code: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(40)
            .map(char::from)
            .collect();
        sqlx::query("DELETE FROM auth.app_handoffs WHERE expires_at <= now()")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "INSERT INTO auth.app_handoffs (code_hash, session, challenge, expires_at)
             VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
        )
        .bind(sha256_hex(&code))
        .bind(session)
        .bind(challenge)
        .bind(TTL_SECONDS as f64)
        .execute(&self.pool)
        .await?;
        Ok(code)
    }

    /// Trades a code and the app's verifier for the session token, once.
    /// Any try uses the code up, right or wrong.
    pub async fn redeem(&self, code: &str, verifier: &str) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String, String)> = sqlx::query_as(
            "DELETE FROM auth.app_handoffs WHERE code_hash = $1 AND expires_at > now()
             RETURNING session, challenge",
        )
        .bind(sha256_hex(code))
        .fetch_optional(&self.pool)
        .await?;
        let Some((session, challenge)) = row else {
            return Ok(None);
        };
        let actual = sha256_hex(verifier);
        Ok(bool::from(actual.as_bytes().ct_eq(challenge.as_bytes())).then_some(session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenges_must_be_hex_sha256() {
        assert!(valid_challenge(&sha256_hex("x")));
        assert!(!valid_challenge("abc"));
        assert!(!valid_challenge(&"Z".repeat(64)));
    }
}
