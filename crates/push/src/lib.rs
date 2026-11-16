//! Push module: phone notifications for the iOS and Android apps, through
//! Expo's push service (which talks to Apple and Google for us).
//!
//! Public API: [`PushService`] (register phones, queue notifications; it is
//! also a [`listings::ListingListener`]), [`register`] (the job handler),
//! [`PushSender`] / [`ExpoSender`], and [`api::router`].
//! Owns the `push` Postgres schema. Sending happens in a background job.

use std::sync::Arc;

use listings::{Listing, ListingListener, Source};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;

pub mod api;

pub const SCHEMA: &str = "push";
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
pub const JOB_KIND: &str = "push.notify";

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("رمز الإشعارات غير صحيح")]
    InvalidToken,
    #[error("نوع الجهاز غير صحيح")]
    InvalidPlatform,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Queue(sqlx::Error),
}

/// One notification to one phone.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub to: String,
    pub title: String,
    pub body: String,
    /// Read by the app when the notification is tapped.
    pub data: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Ok,
    /// The app was uninstalled or notifications turned off: forget the token.
    DeviceGone,
    Failed(String),
}

/// Delivers messages. Production uses [`ExpoSender`].
#[async_trait::async_trait]
pub trait PushSender: Send + Sync {
    /// One outcome per message, in order.
    async fn send(&self, messages: &[Message]) -> Result<Vec<SendOutcome>, String>;
}

/// What a job carries: who to notify and what to say.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Notify {
    owner: i64,
    title: String,
    body: String,
    data: Value,
}

#[derive(Clone)]
pub struct PushService {
    pool: PgPool,
    jobs: jobs::Jobs,
}

fn valid_token(t: &str) -> bool {
    (t.starts_with("ExponentPushToken[") || t.starts_with("ExpoPushToken["))
        && t.ends_with(']')
        && t.len() <= 200
}

impl PushService {
    /// Forgets every phone of a user (their account is being deleted).
    pub async fn forget_owner(&self, owner: i64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM push.devices WHERE owner_id = $1")
            .bind(owner)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub fn new(pool: PgPool, jobs: jobs::Jobs) -> Self {
        Self { pool, jobs }
    }

    /// Remembers this phone for `owner`. A phone belongs to whoever signed in on it last.
    pub async fn register_device(
        &self,
        owner: i64,
        token: &str,
        platform: &str,
    ) -> Result<(), PushError> {
        let token = token.trim();
        if !valid_token(token) {
            return Err(PushError::InvalidToken);
        }
        if !matches!(platform, "ios" | "android") {
            return Err(PushError::InvalidPlatform);
        }
        sqlx::query(
            "INSERT INTO push.devices (token, owner_id, platform) VALUES ($1, $2, $3)
             ON CONFLICT (token) DO UPDATE SET owner_id = $2, platform = $3, updated_at = now()",
        )
        .bind(token)
        .bind(owner)
        .bind(platform)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stops notifications to this phone (on sign-out). Only its owner can.
    pub async fn unregister_device(&self, owner: i64, token: &str) -> Result<(), PushError> {
        sqlx::query("DELETE FROM push.devices WHERE token = $1 AND owner_id = $2")
            .bind(token.trim())
            .bind(owner)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn devices(&self, owner: i64) -> Result<Vec<String>, PushError> {
        Ok(sqlx::query_scalar(
            "SELECT token FROM push.devices WHERE owner_id = $1 ORDER BY created_at, token",
        )
        .bind(owner)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Queues a notification to every phone of `owner`.
    pub async fn notify(
        &self,
        owner: i64,
        title: &str,
        body: &str,
        data: Value,
    ) -> Result<(), PushError> {
        let job = Notify {
            owner,
            title: title.into(),
            body: body.into(),
            data,
        };
        self.jobs
            .enqueue(JOB_KIND, &job)
            .await
            .map(|_| ())
            .map_err(PushError::Queue)
    }

    async fn forget(&self, token: &str) -> Result<(), PushError> {
        sqlx::query("DELETE FROM push.devices WHERE token = $1")
            .bind(token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

/// New rows that arrive by message (not typed in by the user) notify their phones.
#[async_trait::async_trait]
impl ListingListener for PushService {
    async fn created(&self, owner: i64, listing: &Listing, source: Source) -> Result<(), String> {
        let title = match source {
            Source::Web => return Ok(()),
            Source::Telegram => "عرض جديد من تيليجرام",
            Source::WhatsApp => "عرض جديد من واتساب",
        };
        let body = [listing.offer_type.as_str(), listing.location.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let body = if body.is_empty() {
            format!("عرض رقم {}", listing.number())
        } else {
            body
        };
        self.notify(owner, title, &body, json!({ "listing_id": listing.id }))
            .await
            .map_err(|e| e.to_string())
    }
}

/// Registers the sending handler on the job worker.
pub fn register(
    worker: jobs::Worker,
    service: PushService,
    sender: Arc<dyn PushSender>,
) -> jobs::Worker {
    worker.register(JOB_KIND, move |payload| {
        let service = service.clone();
        let sender = sender.clone();
        async move {
            let job: Notify =
                serde_json::from_value(payload).map_err(|e| format!("bad push payload: {e}"))?;
            let tokens = service
                .devices(job.owner)
                .await
                .map_err(|e| e.to_string())?;
            if tokens.is_empty() {
                return Ok(());
            }
            let messages: Vec<Message> = tokens
                .into_iter()
                .map(|to| Message {
                    to,
                    title: job.title.clone(),
                    body: job.body.clone(),
                    data: job.data.clone(),
                })
                .collect();
            let outcomes = sender.send(&messages).await?;
            for (m, o) in messages.iter().zip(outcomes) {
                match o {
                    SendOutcome::Ok => {}
                    SendOutcome::DeviceGone => {
                        service.forget(&m.to).await.map_err(|e| e.to_string())?
                    }
                    // Don't retry the whole batch for one phone: the others already got it.
                    SendOutcome::Failed(e) => {
                        tracing::warn!(error = %e, "push to one phone failed")
                    }
                }
            }
            Ok(())
        }
    })
}

/// Sends through Expo's push API. `access_token` is optional (only needed when
/// "enhanced push security" is turned on in the Expo account).
pub struct ExpoSender {
    http: reqwest::Client,
    access_token: Option<String>,
}

impl ExpoSender {
    pub fn new(access_token: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            access_token,
        }
    }
}

const EXPO_URL: &str = "https://exp.host/--/api/v2/push/send";

/// Reads Expo's answer: `{"data": [{"status": "ok"} | {"status": "error", "details": {"error": "..."}}]}`.
pub fn parse_expo_response(body: &Value, count: usize) -> Result<Vec<SendOutcome>, String> {
    let tickets = body["data"]
        .as_array()
        .ok_or_else(|| format!("unexpected Expo answer: {body}"))?;
    if tickets.len() != count {
        return Err(format!(
            "Expo answered {} tickets for {count} messages",
            tickets.len()
        ));
    }
    Ok(tickets
        .iter()
        .map(|t| match t["status"].as_str() {
            Some("ok") => SendOutcome::Ok,
            _ if t["details"]["error"] == "DeviceNotRegistered" => SendOutcome::DeviceGone,
            _ => SendOutcome::Failed(t["message"].as_str().unwrap_or("unknown").to_string()),
        })
        .collect())
}

#[async_trait::async_trait]
impl PushSender for ExpoSender {
    async fn send(&self, messages: &[Message]) -> Result<Vec<SendOutcome>, String> {
        let mut outcomes = Vec::with_capacity(messages.len());
        // Expo takes at most 100 messages per request.
        for chunk in messages.chunks(100) {
            let body: Vec<Value> = chunk
                .iter()
                .map(|m| {
                    json!({ "to": m.to, "title": m.title, "body": m.body, "data": m.data,
                            "sound": "default", "channelId": "leads" })
                })
                .collect();
            let mut req = self.http.post(EXPO_URL).json(&body);
            if let Some(t) = &self.access_token {
                req = req.bearer_auth(t);
            }
            let res = req.send().await.map_err(|e| format!("Expo push: {e}"))?;
            let status = res.status();
            let answer: Value = res.json().await.map_err(|e| format!("Expo push: {e}"))?;
            if !status.is_success() {
                return Err(format!("Expo push HTTP {status}: {answer}"));
            }
            outcomes.extend(parse_expo_response(&answer, chunk.len())?);
        }
        Ok(outcomes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_expo_tickets() {
        let answer = json!({ "data": [
            { "status": "ok", "id": "x" },
            { "status": "error", "message": "gone", "details": { "error": "DeviceNotRegistered" } },
            { "status": "error", "message": "too big", "details": { "error": "MessageTooBig" } },
        ]});
        assert_eq!(
            parse_expo_response(&answer, 3).unwrap(),
            vec![
                SendOutcome::Ok,
                SendOutcome::DeviceGone,
                SendOutcome::Failed("too big".into())
            ]
        );
        assert!(parse_expo_response(&answer, 2).is_err());
        assert!(parse_expo_response(&json!({ "errors": [] }), 1).is_err());
    }

    #[test]
    fn only_expo_tokens_are_accepted() {
        assert!(valid_token("ExponentPushToken[abc]"));
        assert!(valid_token("ExpoPushToken[abc]"));
        assert!(!valid_token("abc"));
        assert!(!valid_token("ExponentPushToken[abc"));
    }
}
