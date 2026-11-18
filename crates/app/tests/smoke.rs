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
        whatsapp_number: Some("966500000000".into()),
    };
    let modules = app::Modules::new(
        pool.clone(),
        jobs::Jobs::new(pool),
        Arc::new(inbox.clone()),
        auth::AuthConfig::default(),
        web,
        Some("s".repeat(20)),
        Some(whatsapp::webhook::HookConfig {
            app_secret: "app-secret".into(),
            verify_token: "verify-me".into(),
        }),
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
async fn account_page_offers_a_whatsapp_link_that_prefills_the_link_message() {
    let (app, inbox) = app().await;
    let me = sign_in(&app, &inbox, "nasser@example.com").await;
    let (_, _, _, page) = call(&app, get("/account", Some(&me))).await;
    assert!(page.contains("ربط واتساب"), "{page}");
    let (_, _, _, page) = call(&app, form("/account/whatsapp", "", Some(&me))).await;
    // "ربط حسابي في AQRGO: " percent-encoded, then the one-time code.
    assert!(
        page.contains("https://wa.me/966500000000?text=%D8%B1%D8%A8%D8%B7%20%D8%AD%D8%B3%D8%A7%D8%A8%D9%8A%20%D9%81%D9%8A%20AQRGO%3A%20"),
        "{page}"
    );
}

#[tokio::test]
async fn whatsapp_webhook_is_mounted_and_checks_the_verify_token() {
    let (app, _) = app().await;
    let path = "/whatsapp/webhook?hub.mode=subscribe&hub.challenge=7&hub.verify_token=";
    let (status, _, _, body) = call(&app, get(&format!("{path}verify-me"), None)).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "7"));
    let (status, _, _, _) = call(&app, get(&format!("{path}nope"), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
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

#[tokio::test]
async fn stylesheet_url_changes_with_its_content_so_phones_never_keep_old_styles() {
    let (app, _) = app().await;
    let (_, _, _, page) = call(&app, get("/", None)).await;
    let v = common::ui::CSS_VERSION;
    assert!(
        page.contains(&format!("/static/app.css?v={v}")),
        "versioned stylesheet link"
    );
}

#[tokio::test]
async fn htmx_is_served_from_our_own_domain_so_live_search_never_depends_on_a_cdn() {
    let (app, _) = app().await;
    let (_, _, _, page) = call(&app, get("/", None)).await;
    assert!(
        page.contains(r#"src="/static/htmx-2.0.4.js""#),
        "self-hosted htmx"
    );
    assert!(!page.contains("unpkg.com"), "no third-party script host");
    let (status, _, _, js) = call(&app, get("/static/htmx-2.0.4.js", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(js.contains("htmx"), "serves the library");
}

fn on_host(host: &str, uri: &str) -> Request<Body> {
    Request::get(uri)
        .header(header::HOST, host)
        .body(Body::empty())
        .unwrap()
}

async fn agent_with_a_site(app: &Router, inbox: &Inbox) -> String {
    let me = sign_in(app, inbox, "nasser@example.com").await;
    let row = format!(
        "offer_type={}&location={}&price=1500000&rooms=5&seller={}&buyer={}&details={}&status=available",
        enc("فيلا للبيع"),
        enc("حي الملقا"),
        enc("أبو فهد 0500000001"),
        enc("سالم 0500000002"),
        enc("المالك مستعجل")
    );
    call(app, form("/listings/new", &row, Some(&me))).await;
    let site = format!(
        "slug=alnukhba&published=on&company_name={}&tagline={}&services={}&license_no=1200012345&whatsapp=0551234567&phone=0551234567&accent=%230f766e",
        enc("مكتب النخبة العقاري"),
        enc("نبيع ونؤجر في شمال الرياض"),
        enc("تسويق العقارات\nإدارة الأملاك")
    );
    let (status, _, location, body) = call(app, form("/site", &site, Some(&me))).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{body}");
    assert_eq!(location.as_deref(), Some("/site?saved=1"));
    me
}

#[tokio::test]
async fn an_agent_publishes_a_site_that_anyone_can_open() {
    let (app, inbox) = app().await;
    let me = agent_with_a_site(&app, &inbox).await;

    let (status, _, _, page) = call(&app, get("/site?saved=1", Some(&me))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("مكتب النخبة العقاري") && page.contains("alnukhba"));
    assert!(page.contains("حي الملقا"), "listings to show or hide");
    assert!(
        page.contains(r#"href="http://alnukhba.localhost""#),
        "the site's own address"
    );
    assert!(!page.contains("/s/alnukhba"), "no path address");

    {
        let (status, _, _, site) = call(&app, on_host("alnukhba.localhost", "/")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(site.contains("dir=\"rtl\""));
        assert!(site.contains("مكتب النخبة العقاري") && site.contains("إدارة الأملاك"));
        assert!(site.contains("1200012345"), "Fal licence shown");
        assert!(site.contains("https://wa.me/966551234567"));
        assert!(site.contains("tel:+966551234567"));
        assert!(site.contains("حي الملقا") && site.contains("فيلا للبيع"));
        assert!(site.contains("#0f766e"), "owner's colour");
        for private in ["0500000001", "0500000002", "مستعجل"] {
            assert!(!site.contains(private), "leaked {private}");
        }
    }
}

#[tokio::test]
async fn unknown_or_unpublished_sites_are_not_found() {
    let (app, inbox) = app().await;
    let me = agent_with_a_site(&app, &inbox).await;
    let (status, _, _, _) = call(&app, on_host("nobody.localhost", "/")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let off = format!("slug=alnukhba&company_name={}", enc("مكتب النخبة العقاري"));
    call(&app, form("/site", &off, Some(&me))).await;
    let (status, _, _, _) = call(&app, on_host("alnukhba.localhost", "/")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn old_path_links_move_to_the_sites_own_address() {
    let (app, inbox) = app().await;
    agent_with_a_site(&app, &inbox).await;
    let (status, _, location, _) = call(&app, get("/s/alnukhba", None)).await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(location.as_deref(), Some("http://alnukhba.localhost/"));
    let (status, _, _, _) = call(&app, get("/s/nobody", None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_site_address_never_opens_the_crm() {
    let (app, inbox) = app().await;
    agent_with_a_site(&app, &inbox).await;
    for path in ["/listings/new", "/account", "/login", "/export.csv"] {
        let (status, _, _, _) = call(&app, on_host("alnukhba.localhost", path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    // The main address is untouched.
    let (status, _, _, _) = call(&app, on_host("localhost", "/login")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn certificates_are_only_approved_for_published_sites() {
    let (app, inbox) = app().await;
    agent_with_a_site(&app, &inbox).await;
    let (status, _, _, _) = call(&app, get("/sites/tls-check?host=alnukhba.localhost", None)).await;
    assert_eq!(status, StatusCode::OK);
    for host in ["nobody.localhost", "evil.com", "localhost"] {
        let (status, _, _, _) =
            call(&app, get(&format!("/sites/tls-check?host={host}"), None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{host}");
    }
}

#[tokio::test]
async fn an_agent_uploads_a_logo_for_their_site() {
    let (app, inbox) = app().await;
    let me = agent_with_a_site(&app, &inbox).await;
    let png: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];
    let mut body = b"--XyZ\r\nContent-Disposition: form-data; name=\"logo\"; filename=\"l.png\"\r\nContent-Type: image/png\r\n\r\n".to_vec();
    body.extend_from_slice(png);
    body.extend_from_slice(b"\r\n--XyZ--\r\n");
    let req = Request::post("/site/logo")
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=XyZ")
        .header(header::COOKIE, &me)
        .body(Body::from(body))
        .unwrap();
    let (status, _, location, _) = call(&app, req).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/site?saved=1"));

    {
        let res = app
            .clone()
            .oneshot(on_host("alnukhba.localhost", "/logo"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "image/png");
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&bytes[..], png);
    }
    let (_, _, _, site) = call(&app, on_host("alnukhba.localhost", "/")).await;
    assert!(site.contains("src=\"/logo"));
    let res = app
        .clone()
        .oneshot(get("/site/logo", Some(&me)))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "owner preview");
}

#[tokio::test]
async fn a_logo_that_is_too_big_says_so() {
    let (app, inbox) = app().await;
    let me = agent_with_a_site(&app, &inbox).await;
    let mut body = b"--XyZ\r\nContent-Disposition: form-data; name=\"logo\"; filename=\"l.png\"\r\nContent-Type: image/png\r\n\r\n".to_vec();
    body.extend_from_slice(&vec![0u8; 700 * 1024]);
    body.extend_from_slice(b"\r\n--XyZ--\r\n");
    let req = Request::post("/site/logo")
        .header(header::CONTENT_TYPE, "multipart/form-data; boundary=XyZ")
        .header(header::COOKIE, &me)
        .body(Body::from(body))
        .unwrap();
    let (status, _, _, page) = call(&app, req).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("أكبر من"), "{page}");
}

#[tokio::test]
async fn the_privacy_policy_is_public() {
    let (app, _) = app().await;
    let (status, _, _, html) = call(&app, get("/privacy", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("سياسة الخصوصية"));
    assert!(html.contains("nafa050@gmail.com"));
    assert!(html.contains("حذف الحساب"));
}

fn with_header(mut req: Request<Body>, name: &'static str, value: &str) -> Request<Body> {
    req.headers_mut().insert(name, value.parse().unwrap());
    req
}

#[tokio::test]
async fn a_hostile_page_cannot_post_the_login_form_for_you() {
    let (app, inbox) = app().await;
    let (status, ..) = call(
        &app,
        form("/login/email", "email=attacker%40example.com", None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let code = inbox.0.lock().unwrap().clone().unwrap();
    let body = format!("email=attacker%40example.com&code={code}");

    // Modern browsers say where a form post came from.
    let req = with_header(
        form("/login/verify", &body, None),
        "sec-fetch-site",
        "cross-site",
    );
    let (status, cookie, ..) = call(&app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(cookie.is_none(), "no session cookie for a cross-site post");

    // Older ones only send Origin.
    let req = with_header(
        with_header(form("/login/verify", &body, None), "host", "localhost"),
        "origin",
        "https://evil.example",
    );
    assert_eq!(call(&app, req).await.0, StatusCode::FORBIDDEN);

    // The real login page posting to itself still works.
    let req = with_header(
        with_header(
            with_header(form("/login/verify", &body, None), "host", "localhost"),
            "origin",
            "http://localhost",
        ),
        "sec-fetch-site",
        "same-origin",
    );
    let (status, cookie, ..) = call(&app, req).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(cookie.is_some());
}

#[tokio::test]
async fn signed_in_forms_and_logout_refuse_cross_site_posts() {
    let (app, inbox) = app().await;
    let session = sign_in(&app, &inbox, "agent@example.com").await;

    for uri in ["/auth/logout", "/account/telegram", "/site"] {
        let req = with_header(
            form(uri, "", Some(&session)),
            "sec-fetch-site",
            "cross-site",
        );
        assert_eq!(call(&app, req).await.0, StatusCode::FORBIDDEN, "{uri}");
    }
    // Still signed in.
    let (status, ..) = call(&app, get("/account", Some(&session))).await;
    assert_eq!(status, StatusCode::OK);

    // Telegram's webhook is a server-to-server post and is not affected.
    let req = with_header(
        Request::post("/telegram/webhook")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
        "origin",
        "https://evil.example",
    );
    assert_ne!(call(&app, req).await.0, StatusCode::FORBIDDEN);
}
