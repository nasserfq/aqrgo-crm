//! The WhatsApp seam end to end: Meta's signed webhook in, the bot's replies
//! out to a fake Graph API, leads saved to the right account.

use std::sync::{Arc, Mutex};

use auth::{AuthConfig, AuthService, Channel, CodeMailer};
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    routing::post,
    Json, Router,
};
use chrono::NaiveDate;
use hmac::{Hmac, Mac};
use http_body_util::BodyExt;
use listings::{Filter, ListingInput, ListingsService};
use serde_json::{json, Value};
use sha2::Sha256;
use tower::ServiceExt;
use whatsapp::{
    cloud::{CloudApi, CloudOutbox},
    webhook::{self, HookConfig},
};

const SECRET: &str = "app-secret";
const PHONE: &str = "966500000001";

struct NoMail;
#[async_trait::async_trait]
impl CodeMailer for NoMail {
    async fn send_login_code(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

struct FakeAi;
#[async_trait::async_trait]
impl ai::LeadExtractor for FakeAi {
    async fn extract(&self, _: &str, _: NaiveDate) -> Result<Vec<ListingInput>, String> {
        Ok(vec![ListingInput {
            offer_type: "أرض للبيع".into(),
            location: "الملقا".into(),
            ..Default::default()
        }])
    }
}

type Sent = Arc<Mutex<Vec<Value>>>;

/// A stand-in for graph.facebook.com that records every message sent.
async fn fake_graph() -> (String, Sent) {
    let sent: Sent = Arc::default();
    let app = Router::new()
        .route(
            "/{version}/{phone_id}/messages",
            post(|State(s): State<Sent>, Json(v): Json<Value>| async move {
                s.lock().unwrap().push(v);
                Json(json!({ "messages": [{ "id": "wamid.out" }] }))
            }),
        )
        .with_state(sent.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), sent)
}

struct World {
    hook: Router,
    worker: jobs::Worker,
    auth: AuthService,
    listings: ListingsService,
    owner: i64,
    sent: Sent,
}

async fn world() -> World {
    let pool = testkit::fresh_pool().await;
    for (schema, m) in [
        (auth::SCHEMA, &auth::MIGRATOR),
        (listings::SCHEMA, &listings::MIGRATOR),
        (telegram::SCHEMA, &telegram::MIGRATOR),
        (whatsapp::SCHEMA, &whatsapp::MIGRATOR),
        (jobs::SCHEMA, &jobs::MIGRATOR),
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
    let bot = telegram::Bot::new(
        pool.clone(),
        auth.clone(),
        listings.clone(),
        Some(Arc::new(FakeAi)),
    );
    let (graph, sent) = fake_graph().await;
    let api = CloudApi::with_base_url(graph, "token".into(), "123".into());
    let jobs = jobs::Jobs::new(pool.clone());
    let worker = webhook::register(jobs.worker(), bot, CloudOutbox(api));
    let hook = webhook::router(
        jobs,
        pool,
        HookConfig {
            app_secret: SECRET.into(),
            verify_token: "verify-me".into(),
        },
    );
    World {
        hook,
        worker,
        auth,
        listings,
        owner,
        sent,
    }
}

fn delivery(message: Value) -> Vec<u8> {
    json!({
        "object": "whatsapp_business_account",
        "entry": [{ "id": "waba", "changes": [{ "field": "messages", "value": {
            "messaging_product": "whatsapp",
            "metadata": { "display_phone_number": "966500000000", "phone_number_id": "123" },
            "contacts": [{ "profile": { "name": "ناصر" }, "wa_id": PHONE }],
            "messages": [message]
        }}]}]
    })
    .to_string()
    .into_bytes()
}

fn text(id: &str, body: &str) -> Value {
    json!({ "from": PHONE, "id": id, "timestamp": "1700000000", "type": "text", "text": { "body": body } })
}

fn tap(id: &str, reply_id: &str) -> Value {
    json!({ "from": PHONE, "id": id, "timestamp": "1700000000", "type": "interactive",
            "interactive": { "type": "button_reply", "button_reply": { "id": reply_id, "title": "x" } } })
}

impl World {
    async fn post(&self, body: Vec<u8>, signature: Option<String>) -> StatusCode {
        let mut req =
            Request::post(webhook::WEBHOOK_PATH).header("content-type", "application/json");
        if let Some(sig) = signature {
            req = req.header("x-hub-signature-256", sig);
        }
        let res = self
            .hook
            .clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        res.status()
    }

    /// Delivers a correctly signed message and lets the worker handle it.
    async fn deliver(&self, message: Value) {
        let body = delivery(message);
        let status = self.post(body.clone(), Some(sign(&body))).await;
        assert_eq!(status, StatusCode::OK);
        while self.worker.run_one().await.unwrap() {}
    }

    fn sent(&self) -> Vec<Value> {
        self.sent.lock().unwrap().clone()
    }

    fn last_body(&self) -> String {
        let last = self.sent().last().cloned().expect("a reply");
        last["text"]["body"]
            .as_str()
            .or(last["interactive"]["body"]["text"].as_str())
            .unwrap_or_default()
            .to_string()
    }
}

fn sign(body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

#[tokio::test]
async fn meta_can_verify_the_webhook_only_with_our_token() {
    let w = world().await;
    let get = |token: &str| {
        Request::get(format!(
            "{}?hub.mode=subscribe&hub.verify_token={token}&hub.challenge=4242",
            webhook::WEBHOOK_PATH
        ))
        .body(Body::empty())
        .unwrap()
    };
    let res = w.hook.clone().oneshot(get("verify-me")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"4242");
    let res = w.hook.clone().oneshot(get("wrong")).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unsigned_or_forged_deliveries_are_rejected_and_never_answered() {
    let w = world().await;
    let body = delivery(text("wamid.1", "أرض للبيع"));
    assert_eq!(w.post(body.clone(), None).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        w.post(body, Some("sha256=00".into())).await,
        StatusCode::UNAUTHORIZED
    );
    assert!(!w.worker.run_one().await.unwrap(), "nothing queued");
    assert!(w.sent().is_empty());
}

#[tokio::test]
async fn a_linked_number_sends_a_lead_confirms_it_and_it_lands_in_the_table() {
    let w = world().await;
    let token = w.auth.create_link(w.owner).await.unwrap();
    w.deliver(text("wamid.1", &format!("ربط {token}"))).await;
    assert!(w.last_body().contains("تم ربط"), "{}", w.last_body());
    assert_eq!(
        w.auth
            .user_for(Channel::WhatsApp, PHONE.parse().unwrap())
            .await
            .unwrap()
            .unwrap()
            .id,
        w.owner
    );

    w.deliver(text("wamid.2", "للبيع أرض بالملقا")).await;
    let preview = w.sent().last().cloned().unwrap();
    assert_eq!(preview["to"], PHONE);
    assert_eq!(preview["interactive"]["type"], "button");
    assert!(
        w.last_body().contains("*نوع العرض:* أرض للبيع"),
        "{}",
        w.last_body()
    );
    let save = preview["interactive"]["action"]["buttons"][0]["reply"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(save.starts_with("cb:s:"), "{save}");
    let rows = w.listings.list(w.owner, &Filter::default()).await.unwrap();
    assert!(rows.is_empty(), "nothing saved before confirm");

    w.deliver(tap("wamid.3", &save)).await;
    let rows = w.listings.list(w.owner, &Filter::default()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].location, "الملقا");
    assert_eq!(rows[0].source, "whatsapp");
    assert!(w.last_body().contains("تم الحفظ"), "{}", w.last_body());
}

#[tokio::test]
async fn a_message_meta_delivers_twice_is_answered_once() {
    let w = world().await;
    let body = delivery(text("wamid.dup", "مرحبا"));
    assert_eq!(
        w.post(body.clone(), Some(sign(&body))).await,
        StatusCode::OK
    );
    assert_eq!(
        w.post(body.clone(), Some(sign(&body))).await,
        StatusCode::OK
    );
    while w.worker.run_one().await.unwrap() {}
    assert_eq!(w.sent().len(), 1, "{:?}", w.sent());
    assert!(
        w.last_body().contains("ربط واتساب"),
        "strangers are told how to link"
    );
}
