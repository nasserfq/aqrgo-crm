//! Sending through the WhatsApp Cloud API, and mapping the bot's messages
//! (Telegram-style HTML and buttons) onto what WhatsApp can show.

use auth::Channel;
use serde_json::{json, Value};
use telegram::{Buttons, Outbox};

pub const GRAPH_URL: &str = "https://graph.facebook.com";
pub const GRAPH_VERSION: &str = "v23.0";

/// Reply ids carry what a tap means: `cb:` is a button's callback data,
/// `tx:` is text the user would otherwise have typed.
pub const CALLBACK_PREFIX: &str = "cb:";
pub const TEXT_PREFIX: &str = "tx:";

const MAX_TEXT: usize = 4096;
const MAX_BODY: usize = 1024;
const MAX_BUTTONS: usize = 3;
const MAX_ROWS: usize = 10;
const BUTTON_TITLE: usize = 20;
const ROW_TITLE: usize = 24;

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Telegram HTML (`<b>` and escaped text) to WhatsApp formatting.
pub fn to_whatsapp_text(html: &str) -> String {
    html.replace("<b>", "*")
        .replace("</b>", "*")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn text_message(to: &str, body: &str) -> Value {
    json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": to,
        "type": "text",
        "text": { "body": cut(body, MAX_TEXT), "preview_url": false },
    })
}

fn interactive(to: &str, interactive: Value) -> Value {
    json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": to,
        "type": "interactive",
        "interactive": interactive,
    })
}

/// The WhatsApp messages that show `text` with `buttons` to `to`.
///
/// Up to three choices become reply buttons; more become list messages of up
/// to ten rows each. A body too long for an interactive message is sent as
/// plain text first.
pub fn messages(to: &str, html: &str, buttons: &Buttons) -> Vec<Value> {
    let text = to_whatsapp_text(html);
    let choices: Vec<(String, String)> = match buttons {
        Buttons::None | Buttons::RemoveReply => Vec::new(),
        Buttons::Inline(rows) => rows
            .iter()
            .flatten()
            .map(|(label, data)| (label.clone(), format!("{CALLBACK_PREFIX}{data}")))
            .collect(),
        Buttons::Reply(rows) => rows
            .iter()
            .flatten()
            .map(|label| (label.clone(), format!("{TEXT_PREFIX}{label}")))
            .collect(),
    };
    if choices.is_empty() {
        return vec![text_message(to, &text)];
    }

    let mut out = Vec::new();
    let mut body = text.clone();
    if body.chars().count() > MAX_BODY {
        out.push(text_message(to, &text));
        body = "اختر:".into();
    }
    if choices.len() <= MAX_BUTTONS {
        let buttons: Vec<Value> = choices
            .iter()
            .map(|(label, id)| {
                json!({ "type": "reply", "reply": { "id": id, "title": cut(label, BUTTON_TITLE) } })
            })
            .collect();
        out.push(interactive(
            to,
            json!({ "type": "button", "body": { "text": body }, "action": { "buttons": buttons } }),
        ));
        return out;
    }
    for (i, chunk) in choices.chunks(MAX_ROWS).enumerate() {
        let rows: Vec<Value> = chunk
            .iter()
            .map(|(label, id)| json!({ "id": id, "title": cut(label, ROW_TITLE) }))
            .collect();
        let body = if i == 0 {
            body.clone()
        } else {
            "خيارات أخرى:".into()
        };
        out.push(interactive(
            to,
            json!({
                "type": "list",
                "body": { "text": body },
                "action": { "button": "اختر", "sections": [{ "title": "الخيارات", "rows": rows }] },
            }),
        ));
    }
    out
}

/// A WhatsApp Cloud API client for one business phone number.
#[derive(Clone)]
pub struct CloudApi {
    http: reqwest::Client,
    base_url: String,
    token: String,
    phone_number_id: String,
}

impl CloudApi {
    pub fn new(token: String, phone_number_id: String) -> Self {
        Self::with_base_url(GRAPH_URL.into(), token, phone_number_id)
    }

    /// For tests: talk to a fake Graph API.
    pub fn with_base_url(base_url: String, token: String, phone_number_id: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
            phone_number_id,
        }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{GRAPH_VERSION}/{}{path}",
            self.base_url, self.phone_number_id
        )
    }

    pub async fn send(&self, message: &Value) -> Result<(), String> {
        let res = self
            .http
            .post(self.url("/messages"))
            .bearer_auth(&self.token)
            .json(message)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if res.status().is_success() {
            return Ok(());
        }
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        Err(format!("whatsapp send failed ({status}): {body}"))
    }

    /// The business number in international digits, for `wa.me/<number>` links.
    pub async fn phone_number(&self) -> Result<String, String> {
        let res = self
            .http
            .get(self.url("?fields=display_phone_number"))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!("whatsapp number lookup failed ({status}): {body}"));
        }
        let v: Value = res.json().await.map_err(|e| e.to_string())?;
        let digits: String = v["display_phone_number"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        if digits.is_empty() {
            return Err("whatsapp number lookup returned no number".into());
        }
        Ok(digits)
    }
}

/// The bot's [`Outbox`] on WhatsApp. WhatsApp can't edit a sent message, so an
/// edit is sent as a new message, and there is no callback to acknowledge.
#[derive(Clone)]
pub struct CloudOutbox(pub CloudApi);

#[async_trait::async_trait]
impl Outbox for CloudOutbox {
    fn channel(&self) -> Channel {
        Channel::WhatsApp
    }

    async fn send(&self, chat_id: i64, text: &str, buttons: Buttons) -> Result<i32, String> {
        for m in messages(&chat_id.to_string(), text, &buttons) {
            self.0.send(&m).await?;
        }
        Ok(0)
    }

    async fn edit(
        &self,
        chat_id: i64,
        _message_id: i32,
        text: &str,
        buttons: Buttons,
    ) -> Result<(), String> {
        self.send(chat_id, text, buttons).await.map(|_| ())
    }

    async fn answer_callback(&self, _callback_id: &str, _text: Option<&str>) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_and_entities_become_whatsapp_text() {
        assert_eq!(
            to_whatsapp_text("<b>السعر:</b> 1&amp;2 &lt;ok&gt;"),
            "*السعر:* 1&2 <ok>"
        );
    }

    #[test]
    fn three_inline_buttons_become_reply_buttons_carrying_callback_data() {
        let m = messages(
            "966500000001",
            "مسودة",
            &Buttons::Inline(vec![vec![
                ("✅ حفظ".into(), "s:1".into()),
                ("✏️ تعديل".into(), "e:1".into()),
                ("❌ إلغاء".into(), "x:1".into()),
            ]]),
        );
        assert_eq!(m.len(), 1);
        let i = &m[0]["interactive"];
        assert_eq!(i["type"], "button");
        assert_eq!(i["action"]["buttons"][0]["reply"]["id"], "cb:s:1");
        assert_eq!(i["action"]["buttons"][2]["reply"]["title"], "❌ إلغاء");
        assert_eq!(m[0]["to"], "966500000001");
    }

    #[test]
    fn a_long_keyboard_becomes_lists_of_ten_with_short_titles() {
        let labels: Vec<String> = (0..17)
            .map(|i| format!("خانة طويلة جداً جداً جداً رقم {i}"))
            .collect();
        let m = messages(
            "1",
            "اختر الخانة",
            &Buttons::Reply(labels.chunks(2).map(|c| c.to_vec()).collect()),
        );
        assert_eq!(m.len(), 2, "17 choices need two lists");
        let rows = &m[0]["interactive"]["action"]["sections"][0]["rows"];
        assert_eq!(rows.as_array().unwrap().len(), 10);
        assert!(rows[0]["id"].as_str().unwrap().starts_with("tx:"));
        assert!(rows[0]["title"].as_str().unwrap().chars().count() <= ROW_TITLE);
        assert_eq!(m[0]["interactive"]["body"]["text"], "اختر الخانة");
    }

    #[test]
    fn plain_messages_and_removed_keyboards_are_text() {
        let m = messages("1", "<b>تم</b>", &Buttons::RemoveReply);
        assert_eq!(m[0]["type"], "text");
        assert_eq!(m[0]["text"]["body"], "*تم*");
    }

    #[test]
    fn a_body_too_long_for_buttons_is_sent_first_as_text() {
        let long = "ت".repeat(1500);
        let m = messages(
            "1",
            &long,
            &Buttons::Inline(vec![vec![("حفظ".into(), "s:1".into())]]),
        );
        assert_eq!(m.len(), 2);
        assert_eq!(m[0]["type"], "text");
        assert_eq!(m[1]["type"], "interactive");
    }
}
