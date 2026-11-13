//! Glue between the Telegram Bot API and [`Bot`]:
//! the webhook route queues each update as a background job, and the job
//! runs the conversation and replies through the Bot API.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use subtle::ConstantTimeEq;
use teloxide_core::{
    payloads::{
        AnswerCallbackQuerySetters, EditMessageTextSetters, SendMessageSetters, SetWebhookSetters,
    },
    prelude::Requester,
    types::{
        AllowedUpdate, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton,
        KeyboardMarkup, KeyboardRemove, MessageId, ParseMode, ReplyMarkup, Update, UpdateKind,
    },
};

use crate::{Bot, Buttons, Incoming, Outbox};

pub const JOB_KIND: &str = "telegram_update";
pub const WEBHOOK_PATH: &str = "/telegram/webhook";

/// Real Bot API client implementing [`Outbox`].
#[derive(Clone)]
pub struct TelegramOutbox(pub teloxide_core::Bot);

fn markup(b: Buttons) -> Option<ReplyMarkup> {
    match b {
        Buttons::None => None,
        Buttons::Inline(rows) => Some(ReplyMarkup::InlineKeyboard(InlineKeyboardMarkup::new(
            rows.into_iter().map(|r| {
                r.into_iter()
                    .map(|(l, d)| InlineKeyboardButton::callback(l, d))
                    .collect::<Vec<_>>()
            }),
        ))),
        Buttons::Reply(rows) => Some(ReplyMarkup::Keyboard(
            KeyboardMarkup::new(
                rows.into_iter()
                    .map(|r| r.into_iter().map(KeyboardButton::new).collect::<Vec<_>>()),
            )
            .resize_keyboard(),
        )),
        Buttons::RemoveReply => Some(ReplyMarkup::KeyboardRemove(KeyboardRemove::new())),
    }
}

#[async_trait::async_trait]
impl Outbox for TelegramOutbox {
    async fn send(&self, chat_id: i64, text: &str, buttons: Buttons) -> Result<i32, String> {
        let mut req = self
            .0
            .send_message(ChatId(chat_id), text)
            .parse_mode(ParseMode::Html);
        if let Some(m) = markup(buttons) {
            req = req.reply_markup(m);
        }
        req.await.map(|m| m.id.0).map_err(|e| e.to_string())
    }

    async fn edit(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        buttons: Buttons,
    ) -> Result<(), String> {
        let mut req = self
            .0
            .edit_message_text(ChatId(chat_id), MessageId(message_id), text)
            .parse_mode(ParseMode::Html);
        if let Some(ReplyMarkup::InlineKeyboard(k)) = markup(buttons) {
            req = req.reply_markup(k);
        }
        req.await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn answer_callback(&self, callback_id: &str, text: Option<&str>) -> Result<(), String> {
        let mut req = self
            .0
            .answer_callback_query(teloxide_core::types::CallbackQueryId(
                callback_id.to_string(),
            ));
        if let Some(t) = text {
            req = req.text(t);
        }
        req.await.map(|_| ()).map_err(|e| e.to_string())
    }
}

/// teloxide's `Update` must be parsed from text (it borrows map keys), not from a `Value`.
pub fn parse_update(v: &serde_json::Value) -> Result<Update, String> {
    serde_json::from_str(&v.to_string()).map_err(|e| format!("bad update: {e}"))
}

/// Converts a Telegram update into what the bot understands. Private chats only.
pub fn to_incoming(update: Update) -> Option<Incoming> {
    match update.kind {
        UpdateKind::Message(m) => {
            if !m.chat.is_private() {
                return None;
            }
            let from = m.from.as_ref()?;
            let text = m.text().or(m.caption())?.to_string();
            Some(Incoming::Text {
                chat_id: m.chat.id.0,
                from_id: from.id.0 as i64,
                text,
            })
        }
        UpdateKind::CallbackQuery(q) => {
            let msg = q.message.as_ref()?;
            Some(Incoming::Callback {
                chat_id: msg.chat().id.0,
                from_id: q.from.id.0 as i64,
                message_id: msg.id().0,
                callback_id: q.id.0.clone(),
                data: q.data.clone()?,
            })
        }
        _ => None,
    }
}

#[derive(Clone)]
struct HookState {
    jobs: jobs::Jobs,
    secret: Arc<String>,
}

/// POST /telegram/webhook — verifies Telegram's secret header and queues the update.
pub fn router(jobs: jobs::Jobs, secret: String) -> Router {
    Router::new()
        .route(WEBHOOK_PATH, post(receive))
        .with_state(HookState {
            jobs,
            secret: Arc::new(secret),
        })
}

async fn receive(State(s): State<HookState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let given = headers
        .get("x-telegram-bot-api-secret-token")
        .map(|v| v.as_bytes())
        .unwrap_or_default();
    if s.secret.is_empty() || !bool::from(given.ct_eq(s.secret.as_bytes())) {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(update) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return StatusCode::OK;
    };
    // One attempt only: a retried update could send duplicate replies.
    if let Err(e) = s.jobs.enqueue_with_attempts(JOB_KIND, &update, 1).await {
        tracing::error!(error = %e, "could not queue telegram update");
        return StatusCode::SERVICE_UNAVAILABLE; // Telegram will retry delivery.
    }
    StatusCode::OK
}

/// Registers the job that runs the conversation for each queued update.
pub fn register(worker: jobs::Worker, bot: Bot, outbox: TelegramOutbox) -> jobs::Worker {
    let bot = Arc::new(bot);
    worker.register(JOB_KIND, move |payload| {
        let bot = bot.clone();
        let outbox = outbox.clone();
        async move {
            let update = parse_update(&payload)?;
            let Some(incoming) = to_incoming(update) else {
                return Ok(());
            };
            bot.handle(&outbox, incoming)
                .await
                .map_err(|e| e.to_string())
        }
    })
}

/// Points Telegram at our webhook and returns the bot's username.
pub async fn setup(
    api: &teloxide_core::Bot,
    public_url: &str,
    secret: &str,
) -> Result<String, String> {
    let me = api.get_me().await.map_err(|e| e.to_string())?;
    let url = format!("{public_url}{WEBHOOK_PATH}")
        .parse()
        .map_err(|e| format!("bad PUBLIC_URL: {e}"))?;
    api.set_webhook(url)
        .secret_token(secret.to_string())
        .allowed_updates(vec![AllowedUpdate::Message, AllowedUpdate::CallbackQuery])
        .drop_pending_updates(false)
        .await
        .map_err(|e| e.to_string())?;
    Ok(me.username.clone().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_private_text_and_callbacks() {
        let text = parse_update(&serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 5, "date": 1700000000,
                "chat": { "id": 42, "type": "private", "first_name": "N" },
                "from": { "id": 7, "is_bot": false, "first_name": "N" },
                "text": "أرض للبيع"
            }
        }))
        .unwrap();
        match to_incoming(text) {
            Some(Incoming::Text {
                chat_id: 42,
                from_id: 7,
                text,
            }) => assert_eq!(text, "أرض للبيع"),
            other => panic!("{other:?}"),
        }

        let group = parse_update(&serde_json::json!({
            "update_id": 2,
            "message": {
                "message_id": 5, "date": 1700000000,
                "chat": { "id": -100, "type": "group", "title": "g" },
                "from": { "id": 7, "is_bot": false, "first_name": "N" },
                "text": "hi"
            }
        }))
        .unwrap();
        assert!(to_incoming(group).is_none());

        let cb = parse_update(&serde_json::json!({
            "update_id": 3,
            "callback_query": {
                "id": "abc", "chat_instance": "x", "data": "s:9",
                "from": { "id": 7, "is_bot": false, "first_name": "N" },
                "message": {
                    "message_id": 11, "date": 1700000000,
                    "chat": { "id": 42, "type": "private", "first_name": "N" },
                    "text": "preview"
                }
            }
        }))
        .unwrap();
        match to_incoming(cb) {
            Some(Incoming::Callback {
                chat_id: 42,
                from_id: 7,
                message_id: 11,
                data,
                ..
            }) => assert_eq!(data, "s:9"),
            other => panic!("{other:?}"),
        }
    }
}
