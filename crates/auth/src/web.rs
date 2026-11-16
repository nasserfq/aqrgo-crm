//! Login pages, Google OAuth, the account page, and the middleware that
//! puts `common::CurrentUser` into every authenticated request.

use askama::Template;
use axum::{
    extract::{Query, Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Form, Router,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use common::{ui::Nav, CurrentUser};
use rand::{distributions::Alphanumeric, Rng};
use serde::Deserialize;

use crate::{
    apple::{AppleHttpKeys, AppleVerifier},
    handoff::AppHandoff,
    AuthError, AuthService, Channel,
};

/// The iOS app's bundle id: the audience of its Sign in with Apple tokens.
pub const APP_BUNDLE_ID: &str = "com.aqrgo.app";

pub const SESSION_COOKIE: &str = "aqrgo_session";
const STATE_COOKIE: &str = "aqrgo_oauth_state";
/// Set when the phone app started Google sign-in; holds the app's challenge.
const APP_COOKIE: &str = "aqrgo_oauth_app";
/// Where the browser goes back to the app after Google sign-in.
const APP_RETURN: &str = "aqrgo://auth";

#[derive(Clone, Debug)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
}

/// The privacy and support inbox shown on the site and in the app.
pub const CONTACT_EMAIL: &str = "nafa050@gmail.com";

#[derive(Clone, Debug)]
pub struct WebConfig {
    /// e.g. https://aqrgo.com (no trailing slash)
    pub public_url: String,
    pub google: Option<GoogleConfig>,
    /// Telegram bot username without '@', for the account-linking deep link.
    pub telegram_bot: Option<String>,
    /// The WhatsApp business number in international digits, for `wa.me` links.
    pub whatsapp_number: Option<String>,
}

impl WebConfig {
    /// Where people write about privacy or support (Nasser's choice, 2026-11-16).
    pub fn contact_email(&self) -> String {
        CONTACT_EMAIL.to_string()
    }

    fn secure(&self) -> bool {
        self.public_url.starts_with("https://")
    }
}

#[derive(Clone)]
pub struct AuthWeb {
    pub service: AuthService,
    pub config: WebConfig,
    pub handoff: AppHandoff,
    pub apple: AppleVerifier,
    http: reqwest::Client,
}

impl AuthWeb {
    pub fn new(service: AuthService, config: WebConfig) -> Self {
        Self {
            handoff: AppHandoff::new(service.pool().clone()),
            service,
            config,
            apple: AppleVerifier::new(APP_BUNDLE_ID, std::sync::Arc::new(AppleHttpKeys::default())),
            http: reqwest::Client::new(),
        }
    }
}

/// Public routes (no session needed).
pub fn public_router(state: AuthWeb) -> Router {
    Router::new()
        .route("/login", get(login_page))
        .route("/login/email", post(request_code))
        .route("/login/verify", post(verify_code))
        .route("/auth/google", get(google_start))
        .route("/auth/google/callback", get(google_callback))
        .route("/auth/logout", post(logout))
        .route("/privacy", get(privacy_page))
        .with_state(state)
}

/// Routes that need a signed-in user (wrap with [`require_user`]).
pub fn account_router(state: AuthWeb) -> Router {
    Router::new()
        .route("/account", get(account_page))
        .route("/account/telegram", post(telegram_link))
        .route("/account/telegram/unlink", post(telegram_unlink))
        .route("/account/whatsapp", post(whatsapp_link))
        .route("/account/whatsapp/unlink", post(whatsapp_unlink))
        .with_state(state)
}

/// Middleware for cookie-signed-in forms: refuses a POST (or any non-read
/// request) that a browser says came from another site, so a hostile page
/// can't sign you in as someone else, sign you out, or submit your forms.
/// Browsers send `Sec-Fetch-Site`; older ones only `Origin`, which must then
/// match the `Host` the request was sent to. Requests with neither (servers,
/// scripts) are let through: they carry no victim's cookies.
pub async fn reject_cross_site(req: Request, next: Next) -> Response {
    if is_cross_site(&req) {
        tracing::warn!(path = %req.uri().path(), "refused a cross-site form post");
        return (StatusCode::FORBIDDEN, "طلب مرفوض").into_response();
    }
    next.run(req).await
}

fn is_cross_site(req: &Request) -> bool {
    if req.method().is_safe() {
        return false;
    }
    let headers = req.headers();
    if let Some(site) = headers.get("sec-fetch-site") {
        // `none` is a user typing the address or using a bookmark.
        return !matches!(site.as_bytes(), b"same-origin" | b"none");
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return false;
    };
    let origin_host = origin
        .to_str()
        .ok()
        .and_then(|o| o.split_once("://"))
        .map(|(_, host)| host.trim_end_matches('/'));
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    match (origin_host, host) {
        (Some(o), Some(h)) => !o.eq_ignore_ascii_case(h),
        _ => true,
    }
}

/// Middleware: loads the user from the session cookie or sends them to /login.
pub async fn require_user(
    State(state): State<AuthWeb>,
    jar: CookieJar,
    mut req: Request,
    next: Next,
) -> Response {
    let user = match jar.get(SESSION_COOKIE) {
        Some(c) => state
            .service
            .user_for_session(c.value())
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "session lookup failed");
                None
            }),
        None => None,
    };
    match user {
        Some(user) => {
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        None if req.method() == axum::http::Method::GET && req.uri().path() == "/" => {
            render(&LandingPage)
        }
        None if req.headers().contains_key("hx-request") => {
            (StatusCode::UNAUTHORIZED, [("hx-redirect", "/login")]).into_response()
        }
        None => Redirect::to("/login").into_response(),
    }
}

fn session_cookie(config: &WebConfig, token: String) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        .secure(config.secure())
        .same_site(SameSite::Lax)
        .max_age(time_days(crate::SESSION_DAYS.into()))
        .build()
}

fn time_days(d: i64) -> time::Duration {
    time::Duration::days(d)
}

/// The privacy policy (linked from the App Store listing and the footer).
#[derive(Template)]
#[template(path = "privacy.html")]
struct PrivacyPage {
    contact: String,
}

async fn privacy_page(State(s): State<AuthWeb>) -> Response {
    render(&PrivacyPage {
        contact: s.config.contact_email(),
    })
}

/// The public front page, shown at `/` to visitors who are not signed in.
#[derive(Template)]
#[template(path = "landing.html")]
struct LandingPage;

#[derive(Template)]
#[template(path = "login.html")]
struct LoginPage {
    step: Step,
    email: String,
    error: Option<String>,
    google: bool,
}

#[derive(PartialEq, Eq)]
enum Step {
    Email,
    Code,
}

fn render<T: Template>(t: &T) -> Response {
    match common::ui::render(t) {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!(error = %e);
            (StatusCode::INTERNAL_SERVER_ERROR, "template error").into_response()
        }
    }
}

#[derive(Deserialize)]
struct LoginQuery {
    error: Option<String>,
}

async fn login_page(State(s): State<AuthWeb>, Query(q): Query<LoginQuery>) -> Response {
    let error = q.error.map(|e| match e.as_str() {
        "google" => "تعذر الدخول عبر Google، حاول مرة أخرى".to_string(),
        "not_allowed" => AuthError::NotAllowed.to_string(),
        _ => "حدث خطأ، حاول مرة أخرى".to_string(),
    });
    render(&LoginPage {
        step: Step::Email,
        email: String::new(),
        error,
        google: s.config.google.is_some(),
    })
}

#[derive(Deserialize)]
struct EmailForm {
    email: String,
}

async fn request_code(State(s): State<AuthWeb>, Form(f): Form<EmailForm>) -> Response {
    let google = s.config.google.is_some();
    match s.service.request_code(&f.email).await {
        Ok(()) => render(&LoginPage {
            step: Step::Code,
            email: f.email.trim().to_lowercase(),
            error: None,
            google,
        }),
        Err(e) => {
            if let AuthError::Db(_) | AuthError::Mail(_) = &e {
                tracing::error!(error = %e, "request_code failed");
            }
            render(&LoginPage {
                step: Step::Email,
                email: f.email,
                error: Some(e.to_string()),
                google,
            })
        }
    }
}

#[derive(Deserialize)]
struct VerifyForm {
    email: String,
    code: String,
}

async fn verify_code(
    State(s): State<AuthWeb>,
    jar: CookieJar,
    Form(f): Form<VerifyForm>,
) -> Response {
    match s.service.verify_code(&f.email, &f.code).await {
        Ok(token) => (jar.add(session_cookie(&s.config, token)), Redirect::to("/")).into_response(),
        Err(e) => {
            if let AuthError::Db(_) = &e {
                tracing::error!(error = %e, "verify_code failed");
            }
            let step = if matches!(e, AuthError::TooManyAttempts | AuthError::LockedForToday) {
                Step::Email
            } else {
                Step::Code
            };
            render(&LoginPage {
                step,
                email: f.email,
                error: Some(e.to_string()),
                google: s.config.google.is_some(),
            })
        }
    }
}

async fn logout(State(s): State<AuthWeb>, jar: CookieJar) -> Response {
    if let Some(c) = jar.get(SESSION_COOKIE) {
        let _ = s.service.sign_out(c.value()).await;
    }
    (
        jar.remove(Cookie::build(SESSION_COOKIE).path("/").build()),
        Redirect::to("/login"),
    )
        .into_response()
}

#[derive(Deserialize)]
struct GoogleStart {
    /// The phone app's challenge (hex SHA-256 of its verifier); see [`crate::handoff`].
    app: Option<String>,
}

fn app_cookie(config: &WebConfig, challenge: String) -> Cookie<'static> {
    Cookie::build((APP_COOKIE, challenge))
        .path("/auth/google")
        .http_only(true)
        .secure(config.secure())
        .same_site(SameSite::Lax)
        .max_age(time::Duration::minutes(10))
        .build()
}

async fn google_start(
    State(s): State<AuthWeb>,
    jar: CookieJar,
    Query(q): Query<GoogleStart>,
) -> Response {
    let app = match q.app {
        Some(c) if crate::handoff::valid_challenge(&c) => Some(c),
        Some(_) => return Redirect::to(&format!("{APP_RETURN}?error=google")).into_response(),
        None => None,
    };
    let Some(g) = &s.config.google else {
        return match app {
            Some(_) => Redirect::to(&format!("{APP_RETURN}?error=google")).into_response(),
            None => Redirect::to("/login").into_response(),
        };
    };
    let state: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let redirect_uri = format!("{}/auth/google/callback", s.config.public_url);
    let url = reqwest::Url::parse_with_params(
        "https://accounts.google.com/o/oauth2/v2/auth",
        &[
            ("client_id", g.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", "openid email profile"),
            ("state", state.as_str()),
            ("prompt", "select_account"),
        ],
    )
    .expect("valid google url");
    let cookie = Cookie::build((STATE_COOKIE, state))
        .path("/auth/google")
        .http_only(true)
        .secure(s.config.secure())
        .same_site(SameSite::Lax)
        .max_age(time::Duration::minutes(10))
        .build();
    let jar = match app {
        Some(c) => jar.add(app_cookie(&s.config, c)),
        None => jar.remove(Cookie::build(APP_COOKIE).path("/auth/google").build()),
    };
    (jar.add(cookie), Redirect::to(url.as_str())).into_response()
}

#[derive(Deserialize)]
struct GoogleCallback {
    code: Option<String>,
    state: Option<String>,
}

#[derive(Deserialize)]
struct GoogleToken {
    access_token: String,
}

#[derive(Deserialize)]
struct GoogleUser {
    email: String,
    #[serde(default)]
    email_verified: bool,
    #[serde(default)]
    name: String,
}

async fn google_callback(
    State(s): State<AuthWeb>,
    jar: CookieJar,
    Query(q): Query<GoogleCallback>,
) -> Response {
    let app = jar.get(APP_COOKIE).map(|c| c.value().to_string());
    let failed_to = if app.is_some() {
        format!("{APP_RETURN}?error=google")
    } else {
        "/login?error=google".to_string()
    };
    let fail = |jar: CookieJar, why: &str| {
        tracing::warn!(reason = why, "google sign-in failed");
        (
            jar.remove(Cookie::build(STATE_COOKIE).path("/auth/google").build())
                .remove(Cookie::build(APP_COOKIE).path("/auth/google").build()),
            Redirect::to(&failed_to),
        )
            .into_response()
    };
    let Some(g) = s.config.google.clone() else {
        return Redirect::to("/login").into_response();
    };
    let expected = jar.get(STATE_COOKIE).map(|c| c.value().to_string());
    let (Some(code), Some(state)) = (q.code, q.state) else {
        return fail(jar, "missing code/state");
    };
    if expected.as_deref() != Some(state.as_str()) {
        return fail(jar, "state mismatch");
    }
    let redirect_uri = format!("{}/auth/google/callback", s.config.public_url);
    let token = s
        .http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code.as_str()),
            ("client_id", g.client_id.as_str()),
            ("client_secret", g.client_secret.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .and_then(|r| r.error_for_status());
    let token: GoogleToken = match token {
        Ok(r) => match r.json().await {
            Ok(t) => t,
            Err(_) => return fail(jar, "bad token json"),
        },
        Err(_) => return fail(jar, "token exchange"),
    };
    let user: GoogleUser = match s
        .http
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(token.access_token)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => match r.json().await {
            Ok(u) => u,
            Err(_) => return fail(jar, "bad userinfo json"),
        },
        Err(_) => return fail(jar, "userinfo"),
    };
    if !user.email_verified {
        return fail(jar, "email not verified");
    }
    let jar = jar
        .remove(Cookie::build(STATE_COOKIE).path("/auth/google").build())
        .remove(Cookie::build(APP_COOKIE).path("/auth/google").build());
    let signed_in = s
        .service
        .sign_in_verified_email(&user.email, &user.name)
        .await;
    match (signed_in, app) {
        // The phone app gets a one-time code, never the token, in the redirect.
        (Ok(token), Some(challenge)) => match s.handoff.issue(token, challenge).await {
            Ok(code) => (jar, Redirect::to(&format!("{APP_RETURN}?code={code}"))).into_response(),
            Err(e) => {
                tracing::error!(error = %e, "google app handoff");
                (jar, Redirect::to(&failed_to)).into_response()
            }
        },
        (Ok(token), None) => {
            (jar.add(session_cookie(&s.config, token)), Redirect::to("/")).into_response()
        }
        (Err(AuthError::NotAllowed), Some(_)) => (
            jar,
            Redirect::to(&format!("{APP_RETURN}?error=not_allowed")),
        )
            .into_response(),
        (Err(AuthError::NotAllowed), None) => {
            (jar, Redirect::to("/login?error=not_allowed")).into_response()
        }
        (Err(e), _) => {
            tracing::error!(error = %e, "google sign-in");
            (jar, Redirect::to(&failed_to)).into_response()
        }
    }
}

#[derive(Template)]
#[template(path = "account.html")]
struct AccountPage {
    user: CurrentUser,
    nav: Nav,
    telegram_linked: bool,
    telegram_bot: Option<String>,
    telegram_link_url: Option<String>,
    whatsapp_linked: bool,
    whatsapp_number: Option<String>,
    whatsapp_link_url: Option<String>,
}

impl AuthWeb {
    async fn account(&self, user: CurrentUser) -> AccountPage {
        let id = user.id;
        AccountPage {
            user,
            nav: Nav::Account,
            telegram_linked: self
                .service
                .linked(Channel::Telegram, id)
                .await
                .unwrap_or(false),
            telegram_bot: self.config.telegram_bot.clone(),
            telegram_link_url: None,
            whatsapp_linked: self
                .service
                .linked(Channel::WhatsApp, id)
                .await
                .unwrap_or(false),
            whatsapp_number: self.config.whatsapp_number.clone(),
            whatsapp_link_url: None,
        }
    }
}

/// Opens the Telegram bot with a one-time link token.
pub(crate) fn telegram_link_url(bot: &str, token: &str) -> String {
    format!("https://t.me/{bot}?start={token}")
}

/// Opens a WhatsApp chat with the business number, pre-filling "ربط <token>".
pub(crate) fn whatsapp_link_url(number: &str, token: &str) -> String {
    let text = url_encode(&format!("ربط {token}"));
    format!("https://wa.me/{number}?text={text}")
}

/// Percent-encodes everything but ASCII letters and digits (for URL queries).
fn url_encode(s: &str) -> String {
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

async fn account_page(
    State(s): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    render(&s.account(user).await)
}

async fn telegram_link(
    State(s): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    let Some(bot) = s.config.telegram_bot.clone() else {
        return Redirect::to("/account").into_response();
    };
    match s.service.create_link(user.id).await {
        Ok(token) => {
            let mut page = s.account(user).await;
            page.telegram_link_url = Some(telegram_link_url(&bot, &token));
            render(&page)
        }
        Err(e) => {
            tracing::error!(error = %e, "create telegram link");
            (StatusCode::INTERNAL_SERVER_ERROR, "error").into_response()
        }
    }
}

async fn whatsapp_link(
    State(s): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    let Some(number) = s.config.whatsapp_number.clone() else {
        return Redirect::to("/account").into_response();
    };
    match s.service.create_link(user.id).await {
        Ok(token) => {
            let mut page = s.account(user).await;
            page.whatsapp_link_url = Some(whatsapp_link_url(&number, &token));
            render(&page)
        }
        Err(e) => {
            tracing::error!(error = %e, "create whatsapp link");
            (StatusCode::INTERNAL_SERVER_ERROR, "error").into_response()
        }
    }
}

async fn whatsapp_unlink(
    State(s): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    if let Err(e) = s.service.unlink(Channel::WhatsApp, user.id).await {
        tracing::error!(error = %e, "unlink whatsapp");
    }
    (StatusCode::SEE_OTHER, [(header::LOCATION, "/account")]).into_response()
}

async fn telegram_unlink(
    State(s): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    if let Err(e) = s.service.unlink(Channel::Telegram, user.id).await {
        tracing::error!(error = %e, "unlink telegram");
    }
    (StatusCode::SEE_OTHER, [(header::LOCATION, "/account")]).into_response()
}
