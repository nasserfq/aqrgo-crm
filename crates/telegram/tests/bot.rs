use std::sync::{Arc, Mutex};

use auth::{AuthConfig, AuthService, CodeMailer};
use chrono::NaiveDate;
use listings::{Filter, ListingInput, ListingsService};
use telegram::{Bot, Buttons, Incoming, Outbox};

// ---------- fakes ----------

#[allow(dead_code)]
#[derive(Clone, Debug)]
enum Out {
    Send {
        text: String,
        buttons: Buttons,
    },
    Edit {
        message_id: i32,
        text: String,
        buttons: Buttons,
    },
    Answer,
}

#[derive(Default, Clone)]
struct Recorder(Arc<Mutex<Vec<Out>>>);

#[async_trait::async_trait]
impl Outbox for Recorder {
    async fn send(&self, _chat: i64, text: &str, buttons: Buttons) -> Result<i32, String> {
        let mut v = self.0.lock().unwrap();
        v.push(Out::Send {
            text: text.into(),
            buttons,
        });
        Ok(100 + v.len() as i32)
    }
    async fn edit(
        &self,
        _chat: i64,
        message_id: i32,
        text: &str,
        buttons: Buttons,
    ) -> Result<(), String> {
        self.0.lock().unwrap().push(Out::Edit {
            message_id,
            text: text.into(),
            buttons,
        });
        Ok(())
    }
    async fn answer_callback(&self, _id: &str, _text: Option<&str>) -> Result<(), String> {
        self.0.lock().unwrap().push(Out::Answer);
        Ok(())
    }
}

impl Recorder {
    fn take(&self) -> Vec<Out> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
    fn last_text(&self) -> String {
        match self
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|o| !matches!(o, Out::Answer))
            .cloned()
        {
            Some(Out::Send { text, .. }) | Some(Out::Edit { text, .. }) => text,
            _ => panic!("nothing sent"),
        }
    }
    /// callback data of the inline button whose label contains `label`, on the latest message that has it
    fn button(&self, label: &str) -> String {
        for o in self.0.lock().unwrap().iter().rev() {
            if let Out::Send {
                buttons: Buttons::Inline(rows),
                ..
            }
            | Out::Edit {
                buttons: Buttons::Inline(rows),
                ..
            } = o
            {
                for (l, data) in rows.iter().flatten() {
                    if l.contains(label) {
                        return data.clone();
                    }
                }
            }
        }
        panic!("no button {label}");
    }
}

struct NoMail;
#[async_trait::async_trait]
impl CodeMailer for NoMail {
    async fn send_login_code(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

struct FakeAi(Result<Vec<ListingInput>, String>);
#[async_trait::async_trait]
impl ai::LeadExtractor for FakeAi {
    async fn extract(&self, _text: &str, _today: NaiveDate) -> Result<Vec<ListingInput>, String> {
        self.0.clone()
    }
}

// ---------- harness ----------

const CHAT: i64 = 555;
const TG_USER: i64 = 777;

struct World {
    bot: Bot,
    out: Recorder,
    auth: AuthService,
    listings: ListingsService,
    owner: i64,
}

async fn world(ai: Option<FakeAi>) -> World {
    let pool = testkit::fresh_pool().await;
    for (schema, m) in [
        (auth::SCHEMA, &auth::MIGRATOR),
        (listings::SCHEMA, &listings::MIGRATOR),
        (telegram::SCHEMA, &telegram::MIGRATOR),
    ] {
        common::db::migrate_module(&pool, schema, m).await.unwrap();
    }
    let auth = AuthService::new(pool.clone(), Arc::new(NoMail), AuthConfig::default());
    let listings = ListingsService::new(pool.clone());
    let session = auth
        .sign_in_verified_email("nasser@example.com", "ناصر")
        .await
        .unwrap();
    let owner = auth.user_for_session(&session).await.unwrap().unwrap().id;
    let extractor: Option<Arc<dyn ai::LeadExtractor>> = ai.map(|a| Arc::new(a) as _);
    let bot = Bot::new(pool, auth.clone(), listings.clone(), extractor);
    World {
        bot,
        out: Recorder::default(),
        auth,
        listings,
        owner,
    }
}

impl World {
    async fn say(&self, text: &str) {
        let msg = Incoming::Text {
            chat_id: CHAT,
            from_id: TG_USER,
            text: text.into(),
        };
        self.bot.handle(&self.out, msg).await.unwrap();
    }
    async fn tap(&self, data: &str) {
        let msg = Incoming::Callback {
            chat_id: CHAT,
            from_id: TG_USER,
            message_id: 101,
            callback_id: "cb".into(),
            data: data.into(),
        };
        self.bot.handle(&self.out, msg).await.unwrap();
    }
    async fn link(&self) {
        let token = self.auth.create_telegram_link(self.owner).await.unwrap();
        self.say(&format!("/start {token}")).await;
        self.out.take();
    }
    async fn rows(&self) -> Vec<listings::Listing> {
        self.listings
            .list(self.owner, &Filter::default())
            .await
            .unwrap()
    }
}

fn plot(location: &str) -> ListingInput {
    ListingInput {
        offer_type: "أرض للبيع".into(),
        location: location.into(),
        price: "على السوم".into(),
        ..Default::default()
    }
}

// ---------- tests ----------

#[tokio::test]
async fn strangers_are_told_how_to_link_and_nothing_is_saved() {
    let w = world(Some(FakeAi(Ok(vec![plot("الملقا")])))).await;
    w.say("أرض للبيع في الملقا").await;
    assert!(w.out.last_text().contains("ربط"), "{}", w.out.last_text());
    assert!(w.rows().await.is_empty());
}

#[tokio::test]
async fn start_link_connects_the_account() {
    let w = world(None).await;
    let token = w.auth.create_telegram_link(w.owner).await.unwrap();
    w.say(&format!("/start {token}")).await;
    assert!(
        w.out.last_text().contains("تم ربط"),
        "{}",
        w.out.last_text()
    );
    assert_eq!(
        w.auth.user_for_telegram(TG_USER).await.unwrap().unwrap().id,
        w.owner
    );
}

#[tokio::test]
async fn ai_message_becomes_a_preview_and_is_saved_only_after_confirm() {
    let w = world(Some(FakeAi(Ok(vec![plot("الملقا")])))).await;
    w.link().await;

    w.say("للبيع أرض بالملقا على السوم").await;
    let preview = w.out.last_text();
    assert!(
        preview.contains("الملقا") && preview.contains("على السوم"),
        "{preview}"
    );
    assert!(w.rows().await.is_empty(), "nothing saved before confirm");

    let save = w.out.button("حفظ");
    w.tap(&save).await;
    let rows = w.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].location, "الملقا");
    assert_eq!(rows[0].source, "telegram");
    assert!(w.out.last_text().contains("تم الحفظ"));

    // Tapping save again does not duplicate.
    w.tap(&save).await;
    assert_eq!(w.rows().await.len(), 1);
}

#[tokio::test]
async fn one_message_with_two_plots_gives_two_previews() {
    let w = world(Some(FakeAi(Ok(vec![plot("الملقا"), plot("حطين")])))).await;
    w.link().await;
    w.say("قطعتين: وحدة بالملقا ووحدة بحطين").await;
    let previews = w
        .out
        .take()
        .into_iter()
        .filter(|o| {
            matches!(
                o,
                Out::Send {
                    buttons: Buttons::Inline(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(previews, 2);
}

#[tokio::test]
async fn cancel_discards_the_draft() {
    let w = world(Some(FakeAi(Ok(vec![plot("الملقا")])))).await;
    w.link().await;
    w.say("أرض بالملقا").await;
    w.tap(&w.out.button("إلغاء")).await;
    assert!(w.rows().await.is_empty());
    assert!(w.out.last_text().contains("أُلغي"));
}

#[tokio::test]
async fn editing_a_field_before_saving() {
    let w = world(Some(FakeAi(Ok(vec![plot("الملقا")])))).await;
    w.link().await;
    w.say("أرض بالملقا").await;
    w.tap(&w.out.button("تعديل")).await;
    w.tap(&w.out.button("السعر")).await;
    assert!(w.out.last_text().contains("السعر"));
    w.say("١٬٢٠٠٬٠٠٠ ريال").await;
    assert!(
        w.out.last_text().contains("1٬200٬000 ريال"),
        "{}",
        w.out.last_text()
    );
    w.tap(&w.out.button("حفظ")).await;
    assert_eq!(w.rows().await[0].price, "1٬200٬000 ريال");
}

#[tokio::test]
async fn non_listing_message_gets_a_helpful_reply() {
    let w = world(Some(FakeAi(Ok(vec![])))).await;
    w.link().await;
    w.say("السلام عليكم").await;
    assert!(
        w.out.last_text().contains("لم أجد"),
        "{}",
        w.out.last_text()
    );
}

#[tokio::test]
async fn ai_failure_falls_back_to_step_by_step() {
    let w = world(Some(FakeAi(Err("bedrock down".into())))).await;
    w.link().await;
    w.say("أرض بالملقا").await;
    assert!(w.out.last_text().contains("/new"), "{}", w.out.last_text());
}

#[tokio::test]
async fn step_by_step_with_skips() {
    let w = world(None).await;
    w.link().await;
    w.say("/new").await;
    assert!(w.out.last_text().contains("نوع العرض"));
    w.say("فيلا للبيع").await; // offer_type
    for _ in 0..4 {
        w.say("⏭ تخطي").await; // plan, plot, area, frontages
    }
    w.say("حي النرجس").await; // location
    for _ in 0..4 {
        w.say("⏭ تخطي").await; // price, buyer, seller, broker
    }
    assert!(
        w.out.last_text().contains("التاريخ"),
        "{}",
        w.out.last_text()
    );
    w.say("اليوم").await;
    let preview = w.out.last_text();
    assert!(
        preview.contains("فيلا للبيع") && preview.contains("حي النرجس"),
        "{preview}"
    );
    w.tap(&w.out.button("حفظ")).await;
    let rows = w.rows().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].plan_no, "");
}

#[tokio::test]
async fn last_shows_recent_rows() {
    let w = world(None).await;
    w.link().await;
    w.listings
        .create(w.owner, plot("الملقا"), listings::Source::Web)
        .await
        .unwrap();
    w.say("/last").await;
    assert!(w.out.last_text().contains("الملقا"));
}

#[tokio::test]
async fn another_users_draft_cannot_be_saved() {
    let w = world(Some(FakeAi(Ok(vec![plot("الملقا")])))).await;
    w.link().await;
    w.say("أرض بالملقا").await;
    let save = w.out.button("حفظ");

    // A second user links a different Telegram account and taps the first user's button.
    let s = w
        .auth
        .sign_in_verified_email("other@example.com", "x")
        .await
        .unwrap();
    let other = w.auth.user_for_session(&s).await.unwrap().unwrap();
    let token = w.auth.create_telegram_link(other.id).await.unwrap();
    let intruder = Incoming::Text {
        chat_id: 999,
        from_id: 888,
        text: format!("/start {token}"),
    };
    w.bot.handle(&w.out, intruder).await.unwrap();
    let tap = Incoming::Callback {
        chat_id: 999,
        from_id: 888,
        message_id: 1,
        callback_id: "x".into(),
        data: save,
    };
    w.bot.handle(&w.out, tap).await.unwrap();

    assert!(w.rows().await.is_empty());
    assert!(w
        .listings
        .list(other.id, &Filter::default())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn cancelling_a_field_edit_keeps_the_draft() {
    let w = world(Some(FakeAi(Ok(vec![plot("الملقا")])))).await;
    w.link().await;
    w.say("أرض بالملقا").await;
    let save = w.out.button("حفظ");
    w.tap(&w.out.button("تعديل")).await;
    w.tap(&w.out.button("السعر")).await;
    w.say("❌ إلغاء").await;
    w.tap(&save).await;
    assert_eq!(w.rows().await.len(), 1, "draft survived the cancelled edit");
}
