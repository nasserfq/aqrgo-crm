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
        apple_web_client_id: None,
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
        apple_web_client_id: None,
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
    let code = modules
        .auth_web
        .handoff
        .issue(session, challenge)
        .await
        .unwrap();
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
        modules
            .auth_web
            .handoff
            .issue(session, challenge)
            .await
            .unwrap()
    };
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/auth/google",
        None,
        // iOS can leave an empty fragment on the return URL; older app builds send it along.
        Some(json!({ "code": format!("{code}#"), "verifier": verifier })),
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

/// Apple's "Hide My Email" gives a relay address that matches no account.
/// Linking Apple from the account page makes later Apple sign-ins open that
/// account instead of a new, empty one.
#[tokio::test]
async fn apple_with_a_hidden_email_can_be_linked_to_an_existing_account() {
    let pool = testkit::fresh_pool().await;
    app::migrate(&pool).await.unwrap();
    let inbox = Inbox::default();
    let web = auth::web::WebConfig {
        public_url: "http://localhost".into(),
        google: None,
        telegram_bot: None,
        whatsapp_number: None,
        apple_web_client_id: None,
    };
    let mut modules = app::Modules::new(
        pool.clone(),
        jobs::Jobs::new(pool),
        Arc::new(inbox.clone()),
        auth::AuthConfig::default(),
        web,
        None,
        None,
    );
    modules.auth_web.apple =
        auth::apple::AppleVerifier::new("com.aqrgo.app", Arc::new(TestAppleKeys));
    let app = app::router(&modules);
    let me = sign_in(&app, &inbox, "nasser@example.com").await;
    let apple = apple_token("com.aqrgo.app", true, 600);

    // Signed out, or with a bad token, nothing is linked.
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/auth/apple/link",
        None,
        Some(json!({ "identity_token": apple })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/auth/apple/link",
        Some(&me),
        Some(json!({ "identity_token": apple_token("com.aqrgo.app", true, -600) })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/auth/apple/link",
        Some(&me),
        Some(json!({ "identity_token": apple })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = call(
        &app,
        Method::POST,
        "/api/v1/auth/apple",
        None,
        Some(json!({ "identity_token": apple })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user"]["email"], "nasser@example.com");
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

async fn app_with_bots() -> (Router, Inbox, app::Modules) {
    let pool = testkit::fresh_pool().await;
    app::migrate(&pool).await.unwrap();
    let inbox = Inbox::default();
    let web = auth::web::WebConfig {
        public_url: "http://localhost".into(),
        google: None,
        telegram_bot: Some("aqrgo_crm_bot".into()),
        whatsapp_number: Some("966500000000".into()),
        apple_web_client_id: None,
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
    (app::router(&modules), inbox, modules)
}

#[tokio::test]
async fn the_app_links_telegram_and_whatsapp() {
    let (app, inbox, modules) = app_with_bots().await;
    let me = sign_in(&app, &inbox, "agent@example.com").await;

    let (status, _) = call(&app, Method::GET, "/api/v1/channels", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, channels) = call(&app, Method::GET, "/api/v1/channels", Some(&me), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        channels["telegram"],
        json!({ "available": true, "linked": false })
    );
    assert_eq!(
        channels["whatsapp"],
        json!({ "available": true, "linked": false })
    );

    // Telegram: a deep link that opens the bot with a one-time start token.
    let (status, link) = call(
        &app,
        Method::POST,
        "/api/v1/channels/telegram/link",
        Some(&me),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{link}");
    let url = link["url"].as_str().unwrap();
    let token = url
        .strip_prefix("https://t.me/aqrgo_crm_bot?start=")
        .expect(url);
    assert!(matches!(
        modules
            .auth_web
            .service
            .redeem_link(auth::Channel::Telegram, token, 42)
            .await
            .unwrap(),
        auth::LinkOutcome::Linked(_)
    ));
    let (_, channels) = call(&app, Method::GET, "/api/v1/channels", Some(&me), None).await;
    assert_eq!(channels["telegram"]["linked"], true);

    // WhatsApp: a wa.me link that pre-fills "ربط حسابي في AQRGO: <code>".
    let (status, link) = call(
        &app,
        Method::POST,
        "/api/v1/channels/whatsapp/link",
        Some(&me),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(link["url"]
        .as_str()
        .unwrap()
        .starts_with("https://wa.me/966500000000?text=%D8%B1%D8%A8%D8%B7%20%D8%AD%D8%B3%D8%A7%D8%A8%D9%8A%20%D9%81%D9%8A%20AQRGO%3A%20"));

    // Unlinking.
    let (status, _) = call(
        &app,
        Method::DELETE,
        "/api/v1/channels/telegram",
        Some(&me),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, channels) = call(&app, Method::GET, "/api/v1/channels", Some(&me), None).await;
    assert_eq!(channels["telegram"]["linked"], false);

    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/channels/fax/link",
        Some(&me),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn channels_without_a_bot_are_unavailable() {
    let (app, inbox) = app().await;
    let me = sign_in(&app, &inbox, "agent@example.com").await;
    let (_, channels) = call(&app, Method::GET, "/api/v1/channels", Some(&me), None).await;
    assert_eq!(channels["telegram"]["available"], false);
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/v1/channels/telegram/link",
        Some(&me),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A form POST as the desktop app's window (on aqrgo.com itself) sends it.
async fn post_form(
    app: &Router,
    uri: &str,
    form: &str,
    cookie: Option<&str>,
    site: &str,
) -> (StatusCode, String, Vec<String>) {
    let mut b = Request::post(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("sec-fetch-site", site);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    let res = app
        .clone()
        .oneshot(b.body(Body::from(form.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .map(|l| l.to_str().unwrap().to_string())
        .unwrap_or_default();
    let cookies = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    (status, location, cookies)
}

async fn get_with_cookie(app: &Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::get(uri)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let location = res
        .headers()
        .get(header::LOCATION)
        .map(|l| l.to_str().unwrap().to_string())
        .unwrap_or_default();
    (status, location)
}

async fn page_with_cookie(app: &Router, uri: &str, cookie: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::get(uri)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn session_from(cookies: &[String]) -> Option<String> {
    cookies
        .iter()
        .find(|c| c.starts_with("aqrgo_session=") && !c.starts_with("aqrgo_session=;"))
        .map(|c| c.split(';').next().unwrap().to_string())
}

#[tokio::test]
async fn the_desktop_app_signs_in_with_google_through_its_own_link() {
    let (app, modules) = app_with_google().await;
    let verifier = "a-long-random-verifier-the-desktop-app-keeps";
    let challenge = hex::encode(sha2::Sha256::digest(verifier.as_bytes()));

    // The desktop app opens Google in the system browser and says it is the desktop app.
    let (location, cookies) = redirect(
        &app,
        &format!("/auth/google?app={challenge}&client=desktop"),
    )
    .await;
    assert!(location.starts_with("https://accounts.google.com/"));
    let app_cookie = cookies
        .iter()
        .find(|c| c.starts_with("aqrgo_oauth_app="))
        .expect("the app cookie")
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // Errors go back to the desktop app's own link, not the phone app's.
    // Chrome won't follow a redirect into an app at the end of Google's
    // redirect chain, so the way back is a page with a button to click.
    let back = page(&app, "/auth/google?app=nope&client=desktop").await;
    assert!(back.contains(r#"href="aqrgo-desktop://auth?error=google""#));
    let (status, back) = page_with_cookie(&app, "/auth/google/callback", &app_cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(back.contains(r#"href="aqrgo-desktop://auth?error=google""#));
    assert!(back.contains("افتح تطبيق AQRGO"));

    // Back in its window, the app trades the one-time code for a web session.
    let issue = || async {
        let session = modules
            .auth_web
            .service
            .sign_in_verified_email("nasser@example.com", "Nasser")
            .await
            .unwrap();
        modules
            .auth_web
            .handoff
            .issue(session, challenge.clone())
            .await
            .unwrap()
    };
    let code = issue().await;
    let (status, location, cookies) = post_form(
        &app,
        "/auth/app/session",
        &format!("code={code}&verifier=wrong"),
        None,
        "same-origin",
    )
    .await;
    assert!(status.is_redirection());
    assert_eq!(location, "/login?error=google");
    assert!(session_from(&cookies).is_none());

    let code = issue().await;
    // Another site can't post a code into someone's browser.
    let (status, _, _) = post_form(
        &app,
        "/auth/app/session",
        &format!("code={code}&verifier={verifier}"),
        None,
        "cross-site",
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, location, cookies) = post_form(
        &app,
        "/auth/app/session",
        &format!("code={code}&verifier={verifier}"),
        None,
        "same-origin",
    )
    .await;
    assert!(status.is_redirection());
    assert_eq!(location, "/");
    let session = session_from(&cookies).expect("a session cookie");
    let (status, _) = get_with_cookie(&app, "/account", &session).await;
    assert_eq!(status, StatusCode::OK);
    // The app opens at /login; signed in, that goes straight to the table.
    let (status, location) = get_with_cookie(&app, "/login", &session).await;
    assert!(status.is_redirection());
    assert_eq!(location, "/");

    // The code works once.
    let (_, location, _) = post_form(
        &app,
        "/auth/app/session",
        &format!("code={code}&verifier={verifier}"),
        None,
        "same-origin",
    )
    .await;
    assert_eq!(location, "/login?error=google");
}

fn apple_web_token(aud: &str, nonce: &str) -> String {
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
            "exp": now + 600,
            "nonce": nonce,
            "email": "web@privaterelay.appleid.com",
            "email_verified": true,
        }),
        &jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("fixtures/apple_test_key.pem"))
            .unwrap(),
    )
    .unwrap()
}

async fn page(app: &Router, uri: &str) -> String {
    let res = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

#[tokio::test]
async fn the_website_and_desktop_app_sign_in_with_apple() {
    let (_, mut modules) = app_with_google().await;

    // Without a Services ID there is no Apple button.
    let app = app::router(&modules);
    assert!(!page(&app, "/login").await.contains("/auth/apple"));
    let (location, _) = redirect(&app, "/auth/apple").await;
    assert_eq!(location, "/login");

    modules.auth_web.apple_web = Some(auth::apple::AppleVerifier::new(
        "com.aqrgo.web",
        Arc::new(TestAppleKeys),
    ));
    let app = app::router(&modules);
    assert!(page(&app, "/login").await.contains("href=\"/auth/apple\""));

    // Apple's page posts the answer back to us (a cross-site form post).
    let (location, cookies) = redirect(&app, "/auth/apple").await;
    assert!(location.starts_with("https://appleid.apple.com/auth/authorize?"));
    assert!(location.contains("client_id=com.aqrgo.web"));
    assert!(location.contains("response_mode=form_post"));
    let state_cookie = cookies
        .iter()
        .find(|c| c.starts_with("aqrgo_apple_state="))
        .expect("a state cookie");
    assert!(state_cookie.contains("SameSite=None"));
    let state_cookie = state_cookie.split(';').next().unwrap().to_string();
    let state = state_cookie.split_once('=').unwrap().1.to_string();
    let nonce = hex::encode(sha2::Sha256::digest(state.as_bytes()));

    let refused = [
        // Wrong state.
        (format!(
            "state=other&id_token={}",
            apple_web_token("com.aqrgo.web", &nonce)
        )),
        // Meant for someone else.
        (format!(
            "state={state}&id_token={}",
            apple_web_token("com.someone.else", &nonce)
        )),
        // Replayed from another sign-in.
        (format!(
            "state={state}&id_token={}",
            apple_web_token("com.aqrgo.web", "old")
        )),
        // The person pressed cancel.
        (format!("state={state}&error=user_cancelled_authorize")),
    ];
    for form in refused {
        let (status, location, cookies) = post_form(
            &app,
            "/auth/apple/callback",
            &form,
            Some(&state_cookie),
            "cross-site",
        )
        .await;
        assert!(status.is_redirection(), "{form}: {status}");
        assert_eq!(location, "/login?error=apple", "{form}");
        assert!(session_from(&cookies).is_none(), "{form}");
    }
    assert!(page(&app, "/login?error=apple")
        .await
        .contains("تعذر الدخول عبر Apple، حاول مرة أخرى"));

    let user = r#"{"name":{"firstName":"ناصر","lastName":"الفهد"},"email":"web@privaterelay.appleid.com"}"#;
    let form = format!(
        "state={state}&id_token={}&user={}",
        apple_web_token("com.aqrgo.web", &nonce),
        urlencode(user)
    );
    let (status, location, cookies) = post_form(
        &app,
        "/auth/apple/callback",
        &form,
        Some(&state_cookie),
        "cross-site",
    )
    .await;
    assert!(status.is_redirection());
    assert_eq!(location, "/");
    let session = session_from(&cookies).expect("a session cookie");
    let (status, _) = get_with_cookie(&app, "/account", &session).await;
    assert_eq!(status, StatusCode::OK);
    let token = session.split_once('=').unwrap().1;
    let (_, me) = call(&app, Method::GET, "/api/v1/me", Some(token), None).await;
    assert_eq!(me["email"], "web@privaterelay.appleid.com");
    assert_eq!(me["name"], "ناصر الفهد");
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
