//! Sign in with Apple for the iOS app. The app gets an identity token (a JWT
//! signed by Apple) from the system sheet and posts it to
//! `/api/v1/auth/apple`. We check Apple's signature against Apple's published
//! keys, plus the issuer, the audience (our bundle id) and the expiry, then sign
//! in the email inside it.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;

pub const ISSUER: &str = "https://appleid.apple.com";
const KEYS_URL: &str = "https://appleid.apple.com/auth/keys";
const KEYS_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Where Apple's public keys come from (Apple's site in production, a fixed set in tests).
#[async_trait::async_trait]
pub trait AppleKeys: Send + Sync {
    async fn keys(&self, refresh: bool) -> Result<JwkSet, String>;
}

/// Fetches Apple's keys over HTTPS and keeps them for a few hours.
#[derive(Default)]
pub struct AppleHttpKeys {
    http: reqwest::Client,
    cache: Mutex<Option<(Instant, JwkSet)>>,
}

#[async_trait::async_trait]
impl AppleKeys for AppleHttpKeys {
    async fn keys(&self, refresh: bool) -> Result<JwkSet, String> {
        if !refresh {
            if let Some((at, set)) = self.cache.lock().expect("apple keys lock").as_ref() {
                if at.elapsed() < KEYS_TTL {
                    return Ok(set.clone());
                }
            }
        }
        let set: JwkSet = self
            .http
            .get(KEYS_URL)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        *self.cache.lock().expect("apple keys lock") = Some((Instant::now(), set.clone()));
        Ok(set)
    }
}

#[derive(Clone)]
pub struct AppleVerifier {
    audience: String,
    keys: Arc<dyn AppleKeys>,
}

#[derive(Debug, PartialEq)]
pub struct AppleIdentity {
    pub email: String,
}

#[derive(Deserialize)]
struct Claims {
    email: Option<String>,
    /// Apple sends `true` or `"true"`.
    #[serde(default)]
    email_verified: serde_json::Value,
    #[serde(default)]
    nonce: Option<String>,
}

impl AppleVerifier {
    /// `audience` is the app's bundle id, e.g. `com.aqrgo.app`.
    pub fn new(audience: impl Into<String>, keys: Arc<dyn AppleKeys>) -> Self {
        Self {
            audience: audience.into(),
            keys,
        }
    }

    /// Who the tokens must be for (bundle id or Services ID).
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// The verified email in an Apple identity token, or why it was refused (for logs).
    pub async fn verify(&self, token: &str) -> Result<AppleIdentity, String> {
        self.verify_with_nonce(token, None).await
    }

    /// As [`Self::verify`], and when `nonce` is given the token must carry it
    /// (the website's sign-in sends one so a token can't be replayed).
    pub async fn verify_with_nonce(
        &self,
        token: &str,
        nonce: Option<&str>,
    ) -> Result<AppleIdentity, String> {
        let header = decode_header(token).map_err(|e| format!("header: {e}"))?;
        if header.alg != Algorithm::RS256 {
            return Err("unexpected algorithm".into());
        }
        let kid = header.kid.ok_or("no key id")?;
        let mut set = self.keys.keys(false).await?;
        if set.find(&kid).is_none() {
            // Apple rotates keys now and then.
            set = self.keys.keys(true).await?;
        }
        let jwk = set.find(&kid).ok_or("unknown key id")?;
        let key = DecodingKey::from_jwk(jwk).map_err(|e| format!("key: {e}"))?;

        let mut v = Validation::new(Algorithm::RS256);
        v.set_audience(&[self.audience.as_str()]);
        v.set_issuer(&[ISSUER]);
        let claims = decode::<Claims>(token, &key, &v)
            .map_err(|e| format!("token: {e}"))?
            .claims;
        if let Some(n) = nonce {
            if claims.nonce.as_deref() != Some(n) {
                return Err("nonce mismatch".into());
            }
        }

        let verified = matches!(&claims.email_verified, serde_json::Value::Bool(true))
            || claims.email_verified.as_str() == Some("true");
        match claims.email {
            Some(email) if verified && !email.is_empty() => Ok(AppleIdentity { email }),
            _ => Err("no verified email".into()),
        }
    }
}
