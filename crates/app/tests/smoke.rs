//! End-to-end through HTTP: sign in with an emailed code, add a row, see it,
//! and confirm another user can't.

use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[derive(Default, Clone)]
struct Inbox(Arc<Mutex<Option<String>>>);

#[async_trait::async_trait]
impl auth::CodeMailer for Inbox {
    async fn send_login_code(&self, _email: &str, code: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(code.to_string());
        Ok(())
    }
}

async fn app() -> (Router, Inbox) {
    let pool = testkit::fresh_pool().await;
    app::migrate(&pool).await.unwrap();
    let inbox = Inbox::default();
    let web = auth::web::WebConfig {
        public_url: "http://localhost".into(),
        google: None,
        telegram_bot: Some("aqrgo_bot".into()),
    };
    let modules = app::Modules::new(
        pool.clone(),
        jobs::Jobs::new(pool),
        Arc::new(inbox.clone()),
        auth::AuthConfig::default(),
        web,
        Some("s".repeat(20)),
    );
    (app::router(&modules), inbox)
}

async fn call(
    app: &Router,
    req: Request<Body>,
) -> (StatusCode, Option<String>, Option<String>, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
    let location = res
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_string());
    let body =
        String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    (status, cookie, location, body)
}

fn form(uri: &str, body: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b =
        Request::post(uri).header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn get(uri: &str, cookie: Option<&str>) -> Request<Body> {
    let mut b = Request::get(uri);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    b.body(Body::empty()).unwrap()
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

async fn sign_in(app: &Router, inbox: &Inbox, email: &str) -> String {
    let (status, _, _, body) = call(
        app,
        form("/login/email", &format!("email={}", enc(email)), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("رمز الدخول"), "code step shown");
    let code = inbox.0.lock().unwrap().clone().unwrap();
    let (status, cookie, location, _) = call(
        app,
        form(
            "/login/verify",
            &format!("email={}&code={code}", enc(email)),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/"));
    cookie.expect("session cookie")
}

#[tokio::test]
async fn sign_in_add_a_row_and_see_it_in_the_table() {
    let (app, inbox) = app().await;

    // Not signed in: private pages send you to the login page.
    let (status, _, location, _) = call(&app, get("/listings/new", None)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/login"));

    let me = sign_in(&app, &inbox, "nasser@example.com").await;

    let (_, _, _, page) = call(&app, get("/", Some(&me))).await;
    assert!(
        page.contains("بيانات الوسيط") && page.contains("اليوم والتاريخ"),
        "all columns present"
    );
    assert!(page.contains("dir=\"rtl\""));

    let row = format!(
        "offer_type={}&plan_no=2456&plot_no=12&area=625&frontages=&location={}&price={}&buyer=&seller=&broker=&rooms=44&details={}&listed_on=2026-11-13&status=available",
        enc("أرض للبيع"), enc("حي الملقا"), enc("على السوم"), enc("مصعد راكب")
    );
    let (status, _, location, _) = call(&app, form("/listings/new", &row, Some(&me))).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/"));

    let (_, _, _, page) = call(&app, get("/", Some(&me))).await;
    assert!(page.contains("حي الملقا"));
    assert!(page.contains("الجمعة"), "weekday shown for 13 Nov 2026");
    assert!(page.contains("١"), "row number in Arabic digits");
    assert!(
        page.contains("عدد الغرف") && page.contains("مصعد راكب"),
        "extra details shown"
    );

    let (_, _, _, csv) = call(&app, get("/export.csv", Some(&me))).await;
    assert!(csv.contains("حي الملقا"));

    // Someone else signs in and sees an empty table.
    let other = sign_in(&app, &inbox, "other@example.com").await;
    let (_, _, _, page) = call(&app, get("/", Some(&other))).await;
    assert!(!page.contains("حي الملقا"));
}

#[tokio::test]
async fn telegram_webhook_rejects_wrong_secret() {
    let (app, _) = app().await;
    let req = Request::post("/telegram/webhook")
        .header("x-telegram-bot-api-secret-token", "wrong")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _, _, _) = call(&app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let req = Request::post("/telegram/webhook")
        .header("x-telegram-bot-api-secret-token", "s".repeat(20))
        .body(Body::from(r#"{"update_id":1}"#))
        .unwrap();
    let (status, _, _, _) = call(&app, req).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn account_page_offers_telegram_link() {
    let (app, inbox) = app().await;
    let me = sign_in(&app, &inbox, "nasser@example.com").await;
    let (_, _, _, page) = call(&app, form("/account/telegram", "", Some(&me))).await;
    assert!(page.contains("https://t.me/aqrgo_bot?start="), "{page}");
}

#[tokio::test]
async fn signed_out_visitors_see_the_landing_page() {
    let (app, _) = app().await;
    let (status, _, _, page) = call(&app, get("/", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(r#"href="/login""#), "a way in");
    assert!(page.contains("تيليجرام"), "explains the Telegram flow");
    assert!(!page.contains("بيانات الوسيط"), "no private table");
    assert!(!page.contains("—"), "no em-dashes");
}
