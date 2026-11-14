//! The JSON API the iOS and Android apps use: sign in with an emailed code,
//! get a bearer token, and manage your own rows.

use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
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
        telegram_bot: None,
    };
    let modules = app::Modules::new(
        pool.clone(),
        jobs::Jobs::new(pool),
        Arc::new(inbox.clone()),
        auth::AuthConfig::default(),
        web,
        None,
    );
    (app::router(&modules), inbox)
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            panic!(
                "{uri} answered non-JSON: {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, json)
}

async fn sign_in(app: &Router, inbox: &Inbox, email: &str) -> String {
    let (status, _) = call(
        app,
        Method::POST,
        "/api/v1/auth/code",
        None,
        Some(json!({ "email": email })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let code = inbox.0.lock().unwrap().take().expect("code emailed");
    let (status, body) = call(
        app,
        Method::POST,
        "/api/v1/auth/verify",
        None,
        Some(json!({ "email": email, "code": code })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["email"], email);
    body["token"].as_str().expect("token").to_string()
}

#[tokio::test]
async fn sign_in_with_a_code_and_get_a_token() {
    let (app, inbox) = app().await;

    // No token, or a made-up one: 401 with a JSON error, never a redirect.
    let (status, body) = call(&app, Method::GET, "/api/v1/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body["error"].is_string());
    let (status, _) = call(&app, Method::GET, "/api/v1/me", Some("nope"), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A wrong code is refused with an Arabic message.
    call(
        &app,
        Method::POST,
        "/api/v1/auth/code",
        None,
        Some(json!({ "email": "nasser@example.com" })),
    )
    .await;
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/auth/verify",
        None,
        Some(json!({ "email": "nasser@example.com", "code": "000000x" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "الرمز غير صحيح أو منتهي");

    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/auth/code",
        None,
        Some(json!({ "email": "not an email" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "البريد الإلكتروني غير صحيح");

    let token = sign_in(&app, &inbox, "nasser@example.com").await;
    let (status, me) = call(&app, Method::GET, "/api/v1/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["email"], "nasser@example.com");

    // Signing out ends that token.
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/auth/logout",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, Method::GET, "/api/v1/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn manage_your_own_rows() {
    let (app, inbox) = app().await;
    let me = sign_in(&app, &inbox, "nasser@example.com").await;
    let other = sign_in(&app, &inbox, "other@example.com").await;

    let (status, _) = call(&app, Method::GET, "/api/v1/listings", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A row needs at least an offer type, location or plan number.
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/listings",
        Some(&me),
        Some(json!({ "price": "مليون" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"].as_str().unwrap().contains("نوع العرض"));

    let (status, row) = call(
        &app,
        Method::POST,
        "/api/v1/listings",
        Some(&me),
        Some(json!({
            "offer_type": "أرض للبيع",
            "plan_no": "٢٤٥٦",
            "location": "حي الملقا",
            "listed_on": "2026-11-13",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{row}");
    let id = row["id"].as_i64().unwrap();
    assert_eq!(row["number"], "١");
    assert_eq!(row["plan_no"], "2456", "digits stored as ASCII");
    assert_eq!(row["status"], "available");
    assert_eq!(row["status_label"], "متاح");
    assert_eq!(row["listed_on"], "2026-11-13");
    assert!(row["date_hijri"].as_str().unwrap().contains("1448هـ"));

    call(
        &app,
        Method::POST,
        "/api/v1/listings",
        Some(&me),
        Some(json!({ "offer_type": "شقة للإيجار", "location": "حي النرجس", "status": "sold" })),
    )
    .await;

    let (status, list) = call(&app, Method::GET, "/api/v1/listings", Some(&me), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["listings"].as_array().unwrap().len(), 2);

    // Search (Arabic digits match too) and the status filter.
    let (_, found) = call(
        &app,
        Method::GET,
        "/api/v1/listings?q=%D9%A2%D9%A4%D9%A5%D9%A6",
        Some(&me),
        None,
    )
    .await;
    assert_eq!(found["listings"].as_array().unwrap().len(), 1);
    assert_eq!(found["listings"][0]["id"], id);
    let (_, sold) = call(
        &app,
        Method::GET,
        "/api/v1/listings?status=sold",
        Some(&me),
        None,
    )
    .await;
    assert_eq!(sold["listings"].as_array().unwrap().len(), 1);
    assert_eq!(sold["listings"][0]["location"], "حي النرجس");

    // Another user sees none of it and can't touch it.
    let (_, theirs) = call(&app, Method::GET, "/api/v1/listings", Some(&other), None).await;
    assert_eq!(theirs["listings"].as_array().unwrap().len(), 0);
    let path = format!("/api/v1/listings/{id}");
    for method in [Method::GET, Method::DELETE] {
        let (status, _) = call(&app, method, &path, Some(&other), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    let (status, _) = call(
        &app,
        Method::PUT,
        &path,
        Some(&other),
        Some(json!({ "offer_type": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Update replaces the row's fields.
    let mut edited = row.clone();
    edited["status"] = json!("reserved");
    edited["buyer"] = json!("أبو فهد");
    let (status, updated) = call(&app, Method::PUT, &path, Some(&me), Some(edited)).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["status_label"], "محجوز");
    assert_eq!(updated["buyer"], "أبو فهد");
    assert_eq!(updated["number"], "١", "running number kept");

    let (status, got) = call(&app, Method::GET, &path, Some(&me), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got["buyer"], "أبو فهد");

    let (status, _) = call(&app, Method::DELETE, &path, Some(&me), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, Method::GET, &path, Some(&me), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_app_can_read_the_form_choices() {
    let (app, inbox) = app().await;
    let me = sign_in(&app, &inbox, "nasser@example.com").await;
    let (status, meta) = call(&app, Method::GET, "/api/v1/listings/meta", Some(&me), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(meta["fields"][0]["key"], "offer_type");
    assert_eq!(meta["fields"][0]["label"], "نوع العرض");
    assert!(meta["offer_types"]
        .as_array()
        .unwrap()
        .contains(&json!("أرض للبيع")));
    assert_eq!(meta["statuses"][2]["value"], "sold");
    assert_eq!(meta["statuses"][2]["label"], "مباع");
}
