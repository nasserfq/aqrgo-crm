use std::sync::Arc;

use ai::LeadExtractor;
use auth::AuthService;
use common::{dates, html_escape, CurrentUser};
use listings::{ListingInput, ListingsService, Source, FIELDS, OFFER_TYPE_SUGGESTIONS};
use sqlx::PgPool;

use crate::store::{Mode, Store};

/// What arrives from Telegram, already stripped of everything we don't use.
#[derive(Clone, Debug)]
pub enum Incoming {
    Text {
        chat_id: i64,
        from_id: i64,
        text: String,
    },
    Callback {
        chat_id: i64,
        from_id: i64,
        message_id: i32,
        callback_id: String,
        data: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Buttons {
    None,
    /// Buttons under the message: (label, callback data).
    Inline(Vec<Vec<(String, String)>>),
    /// Quick-reply keyboard that types its label.
    Reply(Vec<Vec<String>>),
    RemoveReply,
}

/// Where the bot's replies go. Messages use Telegram's HTML parse mode.
#[async_trait::async_trait]
pub trait Outbox: Send + Sync {
    async fn send(&self, chat_id: i64, text: &str, buttons: Buttons) -> Result<i32, String>;
    async fn edit(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        buttons: Buttons,
    ) -> Result<(), String>;
    async fn answer_callback(&self, callback_id: &str, text: Option<&str>) -> Result<(), String>;
}

#[derive(Debug, thiserror::Error)]
pub enum BotError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("auth: {0}")]
    Auth(#[from] auth::AuthError),
    #[error("listings: {0}")]
    Listings(#[from] listings::ListingsError),
    #[error("telegram: {0}")]
    Send(String),
}

const SKIP: &str = "⏭ تخطي";
const CANCEL: &str = "❌ إلغاء";
const TODAY: &str = "اليوم";
const MENU_NEW: &str = "➕ عرض جديد خطوة بخطوة";
const MENU_LAST: &str = "📋 آخر العروض";
const DATE_FIELD: &str = "listed_on";

#[derive(Clone)]
pub struct Bot {
    store: Store,
    auth: AuthService,
    listings: ListingsService,
    extractor: Option<Arc<dyn LeadExtractor>>,
}

fn label(key: &str) -> &'static str {
    if key == DATE_FIELD {
        return "اليوم والتاريخ";
    }
    FIELDS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, l)| *l)
        .unwrap_or("")
}

fn menu() -> Buttons {
    Buttons::Reply(vec![vec![MENU_NEW.into()], vec![MENU_LAST.into()]])
}

fn preview_text(d: &ListingInput) -> String {
    let mut s = String::from("📝 <b>مسودة عرض</b>\n");
    for (key, lbl) in FIELDS {
        let v = d.get(key);
        if !v.is_empty() {
            s.push_str(&format!("<b>{lbl}:</b> {}\n", html_escape(v)));
        }
    }
    let date = d.listed_on.unwrap_or_else(dates::today_riyadh);
    s.push_str(&format!(
        "<b>اليوم والتاريخ:</b> {}\n",
        dates::full_ar_display(date)
    ));
    s
}

fn preview_buttons(id: i64) -> Buttons {
    Buttons::Inline(vec![vec![
        ("✅ حفظ".into(), format!("s:{id}")),
        ("✏️ تعديل".into(), format!("e:{id}")),
        ("❌ إلغاء".into(), format!("x:{id}")),
    ]])
}

fn field_buttons(id: i64) -> Buttons {
    let mut keys: Vec<&str> = FIELDS.iter().map(|(k, _)| *k).collect();
    keys.push(DATE_FIELD);
    let mut rows: Vec<Vec<(String, String)>> = keys
        .chunks(2)
        .map(|c| {
            c.iter()
                .map(|k| (label(k).to_string(), format!("f:{id}:{k}")))
                .collect()
        })
        .collect();
    rows.push(vec![("↩️ رجوع".into(), format!("b:{id}"))]);
    Buttons::Inline(rows)
}

fn field_prompt_buttons(key: &str) -> Buttons {
    let mut rows: Vec<Vec<String>> = Vec::new();
    if key == "offer_type" {
        rows.extend(
            OFFER_TYPE_SUGGESTIONS
                .chunks(2)
                .map(|c| c.iter().map(|s| s.to_string()).collect()),
        );
    }
    if key == DATE_FIELD {
        rows.push(vec![TODAY.into(), "أمس".into()]);
    }
    rows.push(vec![SKIP.into(), CANCEL.into()]);
    Buttons::Reply(rows)
}

/// The order the step-by-step flow asks in.
fn next_field(current: &str) -> Option<&'static str> {
    let mut keys = FIELDS.iter().map(|(k, _)| *k).chain([DATE_FIELD]);
    keys.position(|k| k == current).and_then(|i| {
        FIELDS
            .iter()
            .map(|(k, _)| *k)
            .chain([DATE_FIELD])
            .nth(i + 1)
    })
}

fn summary_line(l: &listings::Listing) -> String {
    let mut parts = vec![l.offer_type.as_str(), l.location.as_str(), l.price.as_str()];
    parts.retain(|p| !p.is_empty());
    format!(
        "<b>{}</b> · {} · {}",
        l.number(),
        html_escape(&parts.join(" · ")),
        l.status().label_ar()
    )
}

impl Bot {
    pub fn new(
        pool: PgPool,
        auth: AuthService,
        listings: ListingsService,
        extractor: Option<Arc<dyn LeadExtractor>>,
    ) -> Self {
        Self {
            store: Store::new(pool),
            auth,
            listings,
            extractor,
        }
    }

    pub async fn cleanup(&self) -> Result<(), sqlx::Error> {
        self.store.cleanup().await
    }

    pub async fn handle(&self, out: &dyn Outbox, msg: Incoming) -> Result<(), BotError> {
        match msg {
            Incoming::Text {
                chat_id,
                from_id,
                text,
            } => self.on_text(out, chat_id, from_id, text.trim()).await,
            Incoming::Callback {
                chat_id,
                from_id,
                message_id,
                callback_id,
                data,
            } => {
                let r = self
                    .on_callback(out, chat_id, from_id, message_id, &data)
                    .await;
                let _ = out.answer_callback(&callback_id, None).await;
                r
            }
        }
    }

    async fn send(
        &self,
        out: &dyn Outbox,
        chat: i64,
        text: &str,
        b: Buttons,
    ) -> Result<i32, BotError> {
        out.send(chat, text, b).await.map_err(BotError::Send)
    }

    async fn on_text(
        &self,
        out: &dyn Outbox,
        chat: i64,
        from: i64,
        text: &str,
    ) -> Result<(), BotError> {
        if let Some(token) = text.strip_prefix("/start ") {
            return match self.auth.redeem_telegram_link(token, from).await? {
                Some(user) => {
                    let msg = format!(
                        "تم ربط حسابك ✅ ({})\n\nأرسل أي عرض أو طلب كما تكتبه في واتساب (أو أعد توجيه الرسالة) وسأرتبه في جدولك، ثم أعرضه عليك قبل الحفظ.",
                        html_escape(&user.email)
                    );
                    self.send(out, chat, &msg, menu()).await.map(|_| ())
                }
                None => self
                    .send(out, chat, "رابط الربط غير صالح أو منتهي. أنشئ رابطاً جديداً من الموقع: حسابي ← ربط تيليجرام.", Buttons::None)
                    .await
                    .map(|_| ()),
            };
        }

        let Some(user) = self.auth.user_for_telegram(from).await? else {
            return self
                .send(
                    out,
                    chat,
                    "مرحباً 👋 هذا بوت AQRGO CRM.\nلاستخدامه اربط حسابك أولاً: افتح الموقع ← حسابي ← ربط تيليجرام.",
                    Buttons::None,
                )
                .await
                .map(|_| ());
        };

        match text {
            "/start" | "/help" => {
                return self
                    .send(out, chat, "أرسل العرض كنص حر وسأرتبه في الجدول.\n/new إضافة خطوة بخطوة\n/last آخر العروض\n/cancel إلغاء", menu())
                    .await
                    .map(|_| ());
            }
            "/cancel" | CANCEL | "إلغاء" => {
                if let Some(st) = self.store.state(chat).await? {
                    self.store.take_draft(user.id, st.draft_id).await?;
                }
                self.store.clear_state(chat).await?;
                return self.send(out, chat, "تم الإلغاء.", menu()).await.map(|_| ());
            }
            "/last" | MENU_LAST => return self.last(out, chat, &user).await,
            "/new" | MENU_NEW => {
                let draft = ListingInput::default();
                let id = self.store.create_draft(user.id, &draft).await?;
                self.store
                    .set_state(chat, id, Mode::Guided, "offer_type")
                    .await?;
                return self.ask(out, chat, "offer_type").await;
            }
            _ => {}
        }

        if let Some(st) = self.store.state(chat).await? {
            return self.on_field_value(out, chat, &user, st, text).await;
        }
        self.on_free_text(out, chat, &user, text).await
    }

    async fn ask(&self, out: &dyn Outbox, chat: i64, key: &str) -> Result<(), BotError> {
        let q = if key == DATE_FIELD {
            "اليوم والتاريخ؟ اختر «اليوم» أو اكتب التاريخ مثل 13/11/2026".to_string()
        } else {
            format!("{}؟", label(key))
        };
        self.send(out, chat, &q, field_prompt_buttons(key))
            .await
            .map(|_| ())
    }

    async fn on_field_value(
        &self,
        out: &dyn Outbox,
        chat: i64,
        user: &CurrentUser,
        st: crate::store::ChatState,
        text: &str,
    ) -> Result<(), BotError> {
        let Some(mut draft) = self.store.draft(user.id, st.draft_id).await? else {
            self.store.clear_state(chat).await?;
            return self.on_free_text(out, chat, user, text).await;
        };
        if text != SKIP {
            if st.field == DATE_FIELD {
                match dates::parse_user_date(text, dates::today_riyadh()) {
                    Some(d) => draft.listed_on = Some(d),
                    None => {
                        return self
                            .send(
                                out,
                                chat,
                                "لم أفهم التاريخ. اكتبه مثل 13/11/2026 أو اختر «اليوم».",
                                field_prompt_buttons(DATE_FIELD),
                            )
                            .await
                            .map(|_| ());
                    }
                }
            } else {
                draft.set(&st.field, common::digits::to_ascii_digits(text));
            }
            self.store
                .update_draft(user.id, st.draft_id, &draft)
                .await?;
        }
        if st.mode == Mode::Guided {
            if let Some(next) = next_field(&st.field) {
                self.store
                    .set_state(chat, st.draft_id, Mode::Guided, next)
                    .await?;
                return self.ask(out, chat, next).await;
            }
        }
        self.store.clear_state(chat).await?;
        self.send(out, chat, "تمام 👌", Buttons::RemoveReply)
            .await?;
        self.send(
            out,
            chat,
            &preview_text(&draft),
            preview_buttons(st.draft_id),
        )
        .await
        .map(|_| ())
    }

    async fn on_free_text(
        &self,
        out: &dyn Outbox,
        chat: i64,
        user: &CurrentUser,
        text: &str,
    ) -> Result<(), BotError> {
        let Some(extractor) = &self.extractor else {
            return self
                .send(out, chat, "لإضافة عرض اضغط /new وسأسألك عن كل خانة.", menu())
                .await
                .map(|_| ());
        };
        self.send(out, chat, "⏳ أقرأ الرسالة وأرتبها…", Buttons::None)
            .await?;
        let today = dates::today_riyadh();
        match extractor.extract(text, today).await {
            Ok(drafts) if drafts.is_empty() => {
                self.send(out, chat, "لم أجد عرضاً عقارياً في الرسالة. أرسل تفاصيل العرض، أو اضغط /new للإضافة خطوة بخطوة.", menu())
                    .await
                    .map(|_| ())
            }
            Ok(drafts) => {
                for d in drafts {
                    let id = self.store.create_draft(user.id, &d).await?;
                    self.send(out, chat, &preview_text(&d), preview_buttons(id)).await?;
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!(error = %e, "lead extraction failed");
                self.send(out, chat, "تعذرت قراءة الرسالة تلقائياً الآن. اضغط /new لإضافة العرض خطوة بخطوة.", menu())
                    .await
                    .map(|_| ())
            }
        }
    }

    async fn last(&self, out: &dyn Outbox, chat: i64, user: &CurrentUser) -> Result<(), BotError> {
        let rows = self.listings.recent(user.id, 5).await?;
        let text = if rows.is_empty() {
            "لا توجد عروض بعد.".to_string()
        } else {
            let mut s = String::from("📋 <b>آخر العروض</b>\n");
            for l in &rows {
                s.push_str(&summary_line(l));
                s.push('\n');
            }
            s
        };
        self.send(out, chat, &text, Buttons::None).await.map(|_| ())
    }

    async fn on_callback(
        &self,
        out: &dyn Outbox,
        chat: i64,
        from: i64,
        message_id: i32,
        data: &str,
    ) -> Result<(), BotError> {
        let Some(user) = self.auth.user_for_telegram(from).await? else {
            return Ok(());
        };
        let mut parts = data.splitn(3, ':');
        let (Some(action), Some(id)) = (
            parts.next(),
            parts.next().and_then(|s| s.parse::<i64>().ok()),
        ) else {
            return Ok(());
        };
        let field = parts.next();
        let edit = |text: String, b: Buttons| async move {
            out.edit(chat, message_id, &text, b)
                .await
                .map_err(BotError::Send)
        };

        match action {
            "s" => {
                let Some(draft) = self.store.take_draft(user.id, id).await? else {
                    return Ok(());
                };
                self.store.clear_state(chat).await?;
                match self
                    .listings
                    .create(user.id, draft.clone(), Source::Telegram)
                    .await
                {
                    Ok(l) => {
                        edit(
                            format!(
                                "✅ تم الحفظ — رقم {}\n\n{}",
                                l.number(),
                                preview_text(&draft).replacen("📝 <b>مسودة عرض</b>\n", "", 1)
                            ),
                            Buttons::None,
                        )
                        .await
                    }
                    Err(listings::ListingsError::Invalid(msg)) => {
                        let id = self.store.create_draft(user.id, &draft).await?;
                        edit(
                            format!("⚠️ {msg}\n\n{}", preview_text(&draft)),
                            preview_buttons(id),
                        )
                        .await
                    }
                    Err(e) => Err(e.into()),
                }
            }
            "x" => {
                if self.store.take_draft(user.id, id).await?.is_some() {
                    self.store.clear_state(chat).await?;
                    edit("❌ أُلغي هذا العرض ولم يُحفظ.".into(), Buttons::None).await?;
                }
                Ok(())
            }
            "e" => match self.store.draft(user.id, id).await? {
                Some(d) => {
                    edit(
                        format!("{}\nاختر الخانة التي تريد تعديلها:", preview_text(&d)),
                        field_buttons(id),
                    )
                    .await
                }
                None => Ok(()),
            },
            "b" => match self.store.draft(user.id, id).await? {
                Some(d) => edit(preview_text(&d), preview_buttons(id)).await,
                None => Ok(()),
            },
            "f" => {
                let Some(key) =
                    field.filter(|k| *k == DATE_FIELD || FIELDS.iter().any(|(f, _)| f == k))
                else {
                    return Ok(());
                };
                let Some(d) = self.store.draft(user.id, id).await? else {
                    return Ok(());
                };
                self.store.set_state(chat, id, Mode::Edit, key).await?;
                let current = if key == DATE_FIELD {
                    d.listed_on.map(dates::full_ar_display).unwrap_or_default()
                } else {
                    d.get(key).to_string()
                };
                let msg = if current.is_empty() {
                    format!("اكتب {}:", label(key))
                } else {
                    format!(
                        "اكتب {} الجديد.\nالحالي: {}",
                        label(key),
                        html_escape(&current)
                    )
                };
                self.send(out, chat, &msg, field_prompt_buttons(key))
                    .await
                    .map(|_| ())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guided_order_ends_with_the_date() {
        assert_eq!(next_field("offer_type"), Some("plan_no"));
        assert_eq!(next_field("broker"), Some(DATE_FIELD));
        assert_eq!(next_field(DATE_FIELD), None);
    }
}
