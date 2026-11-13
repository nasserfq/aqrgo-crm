//! Meta's webhook for WhatsApp: the verification handshake, the signature
//! check, and queueing each incoming message for the bot.

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use telegram::{Bot, Incoming};

use crate::cloud::{CloudOutbox, CALLBACK_PREFIX, TEXT_PREFIX};

pub const JOB_KIND: &str = "whatsapp_message";
pub const WEBHOOK_PATH: &str = "/whatsapp/webhook";

/// What Meta needs to reach us: both values are set in the Meta app dashboard.
#[derive(Clone, Debug)]
pub struct HookConfig {
    /// The Meta app's secret; Meta signs every delivery with it.
    pub app_secret: String,
    /// Any string we choose, typed into the dashboard when adding the webhook.
    pub verify_token: String,
}

#[derive(Clone)]
struct HookState {
    jobs: jobs::Jobs,
    pool: PgPool,
    config: Arc<HookConfig>,
}

/// GET and POST /whatsapp/webhook.
pub fn router(jobs: jobs::Jobs, pool: PgPool, config: HookConfig) -> Router {
    Router::new()
        .route(WEBHOOK_PATH, get(verify).post(receive))
        .with_state(HookState {
            jobs,
            pool,
            config: Arc::new(config),
        })
}

/// Meta's one-time check when the webhook URL is saved in the dashboard.
async fn verify(State(s): State<HookState>, Query(q): Query<HashMap<String, String>>) -> Response {
    let mode = q.get("hub.mode").map(String::as_str);
    let token = q.get("hub.verify_token").map(String::as_bytes);
    let ok = mode == Some("subscribe")
        && !s.config.verify_token.is_empty()
        && token.is_some_and(|t| bool::from(t.ct_eq(s.config.verify_token.as_bytes())));
    match (ok, q.get("hub.challenge")) {
        (true, Some(challenge)) => challenge.clone().into_response(),
        _ => StatusCode::FORBIDDEN.into_response(),
    }
}

/// True when `header` (`sha256=<hex>`) is the HMAC of `body` under `secret`.
pub fn signature_ok(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(hex_sig) = header.and_then(|h| h.strip_prefix("sha256=")) else {
        return false;
    };
    let Ok(sig) = hex::decode(hex_sig) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    !secret.is_empty() && mac.verify_slice(&sig).is_ok()
}

async fn receive(State(s): State<HookState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok());
    if !signature_ok(&s.config.app_secret, &body, sig) {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(update) = serde_json::from_slice::<Value>(&body) else {
        return StatusCode::OK;
    };
    for message in incoming_messages(&update) {
        let Some(id) = message["id"].as_str() else {
            continue;
        };
        // Meta retries deliveries; only the first copy of a message is queued.
        let first: Result<Option<String>, _> = sqlx::query_scalar(
            "INSERT INTO whatsapp.seen_messages (id) VALUES ($1) ON CONFLICT DO NOTHING RETURNING id",
        )
        .bind(id)
        .fetch_optional(&s.pool)
        .await;
        match first {
            Ok(Some(_)) => {}
            Ok(None) => continue,
            Err(e) => {
                tracing::error!(error = %e, "could not record whatsapp message");
                return StatusCode::SERVICE_UNAVAILABLE; // Meta will retry.
            }
        }
        // One attempt only: a retried job could send duplicate replies.
        if let Err(e) = s.jobs.enqueue_with_attempts(JOB_KIND, message, 1).await {
            tracing::error!(error = %e, "could not queue whatsapp message");
            let _ = sqlx::query("DELETE FROM whatsapp.seen_messages WHERE id = $1")
                .bind(id)
                .execute(&s.pool)
                .await;
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    }
    StatusCode::OK
}

/// The user messages in a webhook delivery (status updates are skipped).
fn incoming_messages(update: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    for entry in update["entry"].as_array().into_iter().flatten() {
        for change in entry["changes"].as_array().into_iter().flatten() {
            if change["field"] != "messages" {
                continue;
            }
            out.extend(change["value"]["messages"].as_array().into_iter().flatten());
        }
    }
    out
}

/// Converts one WhatsApp message into what the bot understands.
/// The sender's number is both the chat and the sender id.
pub fn to_incoming(message: &Value) -> Option<Incoming> {
    let from: i64 = message["from"].as_str()?.parse().ok()?;
    let text = |text: String| Incoming::Text {
        chat_id: from,
        from_id: from,
        text,
    };
    match message["type"].as_str()? {
        "text" => Some(text(message["text"]["body"].as_str()?.to_string())),
        kind @ ("image" | "video" | "document") => {
            Some(text(message[kind]["caption"].as_str()?.to_string()))
        }
        "interactive" => {
            let i = &message["interactive"];
            let reply = match i["type"].as_str()? {
                "button_reply" => &i["button_reply"],
                "list_reply" => &i["list_reply"],
                _ => return None,
            };
            let id = reply["id"].as_str()?;
            if let Some(data) = id.strip_prefix(CALLBACK_PREFIX) {
                return Some(Incoming::Callback {
                    chat_id: from,
                    from_id: from,
                    message_id: 0,
                    callback_id: String::new(),
                    data: data.to_string(),
                });
            }
            let typed = id.strip_prefix(TEXT_PREFIX).or(reply["title"].as_str())?;
            Some(text(typed.to_string()))
        }
        _ => None,
    }
}

/// Registers the job that runs the conversation for each queued message.
pub fn register(worker: jobs::Worker, bot: Bot, outbox: CloudOutbox) -> jobs::Worker {
    let bot = Arc::new(bot);
    worker.register(JOB_KIND, move |payload| {
        let bot = bot.clone();
        let outbox = outbox.clone();
        async move {
            let Some(incoming) = to_incoming(&payload) else {
                return Ok(());
            };
            bot.handle(&outbox, incoming)
                .await
                .map_err(|e| e.to_string())
        }
    })
}

/// Forgets message ids older than a week (Meta only retries for a few days).
pub async fn cleanup(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM whatsapp.seen_messages WHERE seen_at < now() - interval '7 days'")
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_text_captions_and_taps() {
        let t = to_incoming(&json!({
            "from": "966500000001", "id": "wamid.1", "type": "text",
            "text": { "body": "أرض للبيع" }
        }));
        assert!(
            matches!(t, Some(Incoming::Text { chat_id: 966500000001, ref text, .. }) if text == "أرض للبيع")
        );

        let cap = to_incoming(&json!({
            "from": "1", "id": "w", "type": "image", "image": { "id": "m", "caption": "فيلا" }
        }));
        assert!(matches!(cap, Some(Incoming::Text { ref text, .. }) if text == "فيلا"));

        let tap = to_incoming(&json!({
            "from": "1", "id": "w", "type": "interactive",
            "interactive": { "type": "button_reply", "button_reply": { "id": "cb:s:9", "title": "✅ حفظ" } }
        }));
        assert!(matches!(tap, Some(Incoming::Callback { ref data, .. }) if data == "s:9"));

        let row = to_incoming(&json!({
            "from": "1", "id": "w", "type": "interactive",
            "interactive": { "type": "list_reply", "list_reply": { "id": "tx:⏭ تخطي", "title": "⏭ تخطي" } }
        }));
        assert!(matches!(row, Some(Incoming::Text { ref text, .. }) if text == "⏭ تخطي"));

        let sticker = to_incoming(&json!({ "from": "1", "id": "w", "type": "sticker" }));
        assert!(sticker.is_none());
    }

    #[test]
    fn signatures_must_match_the_app_secret() {
        let body = br#"{"a":1}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(body);
        let good = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(signature_ok("secret", body, Some(&good)));
        assert!(!signature_ok("other", body, Some(&good)));
        assert!(!signature_ok("secret", b"{}", Some(&good)));
        assert!(!signature_ok("secret", body, None));
        assert!(!signature_ok("", body, Some("sha256=")));
    }
}
