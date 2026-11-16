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
use sha2::Digest;
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
        whatsapp_number: None,
    };
    let modules = app::Modules::new(
        pool.clone(),
        jobs::Jobs::new(pool),
        Arc::new(inbox.clone()),
        auth::AuthConfig::default(),
        web,
        None,
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

#[tokio::test]
async fn a_phone_registers_for_notifications() {
    let (app, inbox) = app().await;
    let me = sign_in(&app, &inbox, "nasser@example.com").await;
    let device = json!({ "token": "ExponentPushToken[abc]", "platform": "ios" });

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/push/devices",
        None,
        Some(device.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/push/devices",
        Some(&me),
        Some(device.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/push/devices",
        Some(&me),
        Some(json!({ "token": "nope", "platform": "ios" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "رمز الإشعارات غير صحيح");

    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/v1/push/devices",
        Some(&me),
        Some(device),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

async fn app_with_google() -> (Router, app::Modules) {
    let pool = testkit::fresh_pool().await;
    app::migrate(&pool).await.unwrap();
    let web = auth::web::WebConfig {
        public_url: "http://localhost".into(),
        google: Some(auth::web::GoogleConfig {
            client_id: "client".into(),
            client_secret: "secret".into(),
        }),
        telegram_bot: None,
        whatsapp_number: None,
    };
    let modules = app::Modules::new(
        pool.clone(),
        jobs::Jobs::new(pool),
        Arc::new(Inbox::default()),
        auth::AuthConfig::default(),
        web,
        None,
        None,
    );
    (app::router(&modules), modules)
}

async fn redirect(app: &Router, uri: &str) -> (String, Vec<String>) {
    let res = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(res.status().is_redirection(), "{uri}: {}", res.status());
    let location = res.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_string();
    let cookies = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    (location, cookies)
}

#[tokio::test]
async fn the_app_signs_in_with_google_through_a_one_time_code() {
    let (app, modules) = app_with_google().await;

    // The app asks which buttons to show.
    let (status, options) = call(&app, Method::GET, "/api/v1/auth/options", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(options["google"], true);

    // Starting from the app goes to Google and remembers the app's challenge.
    let verifier = "a-long-random-verifier-the-app-keeps";
    let challenge = hex::encode(sha2::Sha256::digest(verifier.as_bytes()));
    let (location, cookies) = redirect(&app, &format!("/auth/google?app={challenge}")).await;
    assert!(location.starts_with("https://accounts.google.com/"));
    assert!(cookies
        .iter()
        .any(|c| c.starts_with(&format!("aqrgo_oauth_app={challenge}"))));

    // A malformed challenge goes straight back to the app with an error.
    let (location, _) = redirect(&app, "/auth/google?app=nope").await;
    assert_eq!(location, "aqrgo://auth?error=google");

    // After Google, the app holds a one-time code; with the verifier it gets a token.
    let session = modules
        .auth_web
        .service
        .sign_in_verified_email("nasser@example.com", "Nasser")
        .await
        .unwrap();
    let code = modules.auth_web.handoff.issue(session, challenge);
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/auth/google",
        None,
        Some(json!({ "code": code, "verifier": "wrong" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "تعذر الدخول عبر Google، حاول مرة أخرى");

    let code = {
        let session = modules
            .auth_web
            .service
            .sign_in_verified_email("nasser@example.com", "Nasser")
            .await
            .unwrap();
        let challenge = hex::encode(sha2::Sha256::digest(verifier.as_bytes()));
        modules.auth_web.handoff.issue(session, challenge)
    };
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/auth/google",
        None,
        Some(json!({ "code": code, "verifier": verifier })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["email"], "nasser@example.com");
    let token = body["token"].as_str().unwrap();
    let (status, me) = call(&app, Method::GET, "/api/v1/me", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["email"], "nasser@example.com");
}

#[tokio::test]
async fn google_is_hidden_from_the_app_when_not_set_up() {
    let (app, _) = app().await;
    let (_, options) = call(&app, Method::GET, "/api/v1/auth/options", None, None).await;
    assert_eq!(options["google"], false);
    let (location, _) = redirect(&app, &format!("/auth/google?app={}", "a".repeat(64))).await;
    assert_eq!(location, "aqrgo://auth?error=google");
}

/// Apple's keys for tests: one key pair made just for these tests
/// (fixtures/apple_test_key.pem); it signs nothing real.
struct TestAppleKeys;

#[async_trait::async_trait]
impl auth::apple::AppleKeys for TestAppleKeys {
    async fn keys(&self, _refresh: bool) -> Result<jsonwebtoken::jwk::JwkSet, String> {
        Ok(serde_json::from_str(include_str!("fixtures/apple_test_jwks.json")).unwrap())
    }
}

fn apple_token(aud: &str, email_verified: bool, exp_in_secs: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-key".into());
    jsonwebtoken::encode(
        &header,
        &json!({
            "iss": "https://appleid.apple.com",
            "aud": aud,
            "sub": "001234.abcd",
            "iat": now,
            "exp": now + exp_in_secs,
            "email": "abc123@privaterelay.appleid.com",
            "email_verified": if email_verified { json!("true") } else { json!("false") },
        }),
        &jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("fixtures/apple_test_key.pem"))
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn the_app_signs_in_with_apple() {
    let (_, mut modules) = app_with_google().await;
    modules.auth_web.apple =
        auth::apple::AppleVerifier::new("com.aqrgo.app", Arc::new(TestAppleKeys));
    let app = app::router(&modules);

    let refused = [
        apple_token("com.someone.else", true, 600),
        apple_token("com.aqrgo.app", false, 600),
        apple_token("com.aqrgo.app", true, -600),
        "not a token".to_string(),
    ];
    for token in refused {
        let (status, body) = call(
            &app,
            Method::POST,
            "/api/v1/auth/apple",
            None,
            Some(json!({ "identity_token": token })),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{token}");
        assert_eq!(body["error"], "تعذر الدخول عبر Apple، حاول مرة أخرى");
    }

    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/auth/apple",
        None,
        Some(json!({ "identity_token": apple_token("com.aqrgo.app", true, 600), "name": "ناصر" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["email"], "abc123@privaterelay.appleid.com");
    assert_eq!(body["user"]["name"], "ناصر");
    let token = body["token"].as_str().unwrap();
    let (status, _) = call(&app, Method::GET, "/api/v1/me", Some(token), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn deleting_the_account_removes_it_and_its_rows() {
    let (app, inbox) = app().await;
    let me = sign_in(&app, &inbox, "nasser@example.com").await;
    let other = sign_in(&app, &inbox, "other@example.com").await;
    for token in [&me, &other] {
        let (status, _) = call(
            &app,
            Method::POST,
            "/api/v1/listings",
            Some(token),
            Some(json!({ "offer_type": "أرض للبيع", "location": "حي الملقا" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    call(
        &app,
        Method::POST,
        "/api/v1/push/devices",
        Some(&me),
        Some(json!({ "token": "ExponentPushToken[abc]", "platform": "ios" })),
    )
    .await;

    let (status, _) = call(&app, Method::DELETE, "/api/v1/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(&app, Method::DELETE, "/api/v1/me", Some(&me), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Signed out everywhere; signing in again starts an empty account.
    let (status, _) = call(&app, Method::GET, "/api/v1/me", Some(&me), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let again = sign_in(&app, &inbox, "nasser@example.com").await;
    let (_, list) = call(&app, Method::GET, "/api/v1/listings", Some(&again), None).await;
    assert_eq!(list["listings"].as_array().unwrap().len(), 0);

    // Other people's rows are untouched.
    let (_, theirs) = call(&app, Method::GET, "/api/v1/listings", Some(&other), None).await;
    assert_eq!(theirs["listings"].as_array().unwrap().len(), 1);
}

struct FakeExtractor;

#[async_trait::async_trait]
impl ai::LeadExtractor for FakeExtractor {
    async fn extract(
        &self,
        text: &str,
        _today: chrono::NaiveDate,
    ) -> Result<Vec<listings::ListingInput>, String> {
        if text.contains("للبيع") {
            Ok(vec![listings::ListingInput {
                offer_type: "أرض للبيع".into(),
                location: "حي الملقا".into(),
                ..Default::default()
            }])
        } else {
            Ok(vec![])
        }
    }
}

#[tokio::test]
async fn the_app_pastes_a_lead_and_gets_draft_rows() {
    let (_, mut modules) = app_with_google().await;
    let app = app::router(&modules);
    let me = {
        let session = modules
            .auth_web
            .service
            .sign_in_verified_email("nasser@example.com", "Nasser")
            .await
            .unwrap();
        session
    };
    let paste = json!({ "text": "للبيع أرض في حي الملقا" });

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/listings/extract",
        None,
        Some(paste.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Without the AI set up, the app is told to fill the form by hand.
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/listings/extract",
        Some(&me),
        Some(paste.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["error"].as_str().unwrap().contains("يدوياً"));

    modules.extractor = Some(Arc::new(FakeExtractor));
    let app = app::router(&modules);
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/listings/extract",
        Some(&me),
        Some(paste),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["listings"][0]["offer_type"], "أرض للبيع");
    assert_eq!(body["listings"][0]["location"], "حي الملقا");

    // Nothing is saved until the person saves the form.
    let (_, list) = call(&app, Method::GET, "/api/v1/listings", Some(&me), None).await;
    assert_eq!(list["listings"].as_array().unwrap().len(), 0);

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/listings/extract",
        Some(&me),
        Some(json!({ "text": "  " })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}
