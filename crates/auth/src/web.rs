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
/// Where the browser goes back to the phone app after Google sign-in.
const APP_RETURN: &str = "aqrgo://auth";
/// Where it goes back to the desktop app (its own link, so a phone app
/// installed on the same Mac can't catch it).
const DESKTOP_RETURN: &str = "aqrgo-desktop://auth";
/// The app cookie's prefix when the desktop app started sign-in.
const DESKTOP_PREFIX: &str = "desktop.";
/// Ties Apple's answer to the browser that asked. Apple posts the answer from
/// its own site, so this cookie must be `SameSite=None` to come along.
const APPLE_STATE_COOKIE: &str = "aqrgo_apple_state";

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
    /// The Apple Services ID for Sign in with Apple on the website and the
    /// desktop app (e.g. `com.aqrgo.web`). No Apple button without it.
    pub apple_web_client_id: Option<String>,
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
    /// Checks the website's Apple tokens (audience: the Services ID).
    pub apple_web: Option<AppleVerifier>,
    http: reqwest::Client,
}

impl AuthWeb {
    pub fn new(service: AuthService, config: WebConfig) -> Self {
        let apple_keys = std::sync::Arc::new(AppleHttpKeys::default());
        Self {
            apple_web: config
                .apple_web_client_id
                .clone()
                .map(|id| AppleVerifier::new(id, apple_keys.clone())),
            handoff: AppHandoff::new(service.pool().clone()),
            service,
            config,
            apple: AppleVerifier::new(APP_BUNDLE_ID, apple_keys),
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
        .route("/auth/app/session", post(app_session))
        .route("/auth/apple", get(apple_start))
        .route("/auth/logout", post(logout))
        .route("/privacy", get(privacy_page))
        .with_state(state)
}

/// Apple's answer to the website's sign-in. Apple posts it from its own site,
/// so this route must stay outside [`reject_cross_site`]; the state cookie,
/// the nonce and Apple's signature protect it instead.
pub fn apple_callback_router(state: AuthWeb) -> Router {
    Router::new()
        .route("/auth/apple/callback", post(apple_callback))
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
    apple: bool,
}

#[derive(PartialEq, Eq)]
enum Step {
    Email,
    Code,
}

/// The desktop app's return page: Chrome drops a redirect to `aqrgo-desktop://`
/// at the end of Google's redirect chain, so the page offers a button to click.
#[derive(Template)]
#[template(path = "app_return.html")]
struct AppReturnPage {
    link: String,
    failed: bool,
}

/// Sends a sign-in back to the app link `link`: the phone app follows a
/// redirect, the desktop app needs a page with a button.
fn back_to_app(link: String) -> Response {
    if link.starts_with(DESKTOP_RETURN) {
        let failed = link.contains("error=");
        render(&AppReturnPage { link, failed })
    } else {
        Redirect::to(&link).into_response()
    }
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

async fn login_page(
    State(s): State<AuthWeb>,
    jar: CookieJar,
    Query(q): Query<LoginQuery>,
) -> Response {
    // Already signed in (the desktop app opens here): straight to the table.
    if let Some(c) = jar.get(SESSION_COOKIE) {
        if let Ok(Some(_)) = s.service.user_for_session(c.value()).await {
            return Redirect::to("/").into_response();
        }
    }
    let error = q.error.map(|e| match e.as_str() {
        "google" => "تعذر الدخول عبر Google، حاول مرة أخرى".to_string(),
        "apple" => "تعذر الدخول عبر Apple، حاول مرة أخرى".to_string(),
        "not_allowed" => AuthError::NotAllowed.to_string(),
        _ => "حدث خطأ، حاول مرة أخرى".to_string(),
    });
    render(&LoginPage {
        step: Step::Email,
        email: String::new(),
        error,
        google: s.config.google.is_some(),
        apple: s.apple_web.is_some(),
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
            apple: s.apple_web.is_some(),
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
                apple: s.apple_web.is_some(),
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
                apple: s.apple_web.is_some(),
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
    /// `desktop` when the desktop app started sign-in.
    client: Option<String>,
}

/// Which app link a sign-in goes back to, from the app cookie's value.
fn app_return(cookie: &str) -> (&'static str, &str) {
    match cookie.strip_prefix(DESKTOP_PREFIX) {
        Some(challenge) => (DESKTOP_RETURN, challenge),
        None => (APP_RETURN, cookie),
    }
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
    let desktop = q.client.as_deref() == Some("desktop");
    let back = if desktop { DESKTOP_RETURN } else { APP_RETURN };
    let app = match q.app {
        Some(c) if crate::handoff::valid_challenge(&c) => Some(if desktop {
            format!("{DESKTOP_PREFIX}{c}")
        } else {
            c
        }),
        Some(_) => return back_to_app(format!("{back}?error=google")),
        None => None,
    };
    let Some(g) = &s.config.google else {
        return match app {
            Some(_) => back_to_app(format!("{back}?error=google")),
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
    let failed_to = if let Some(a) = &app {
        format!("{}?error=google", app_return(a).0)
    } else {
        "/login?error=google".to_string()
    };
    let fail = |jar: CookieJar, why: &str| {
        tracing::warn!(reason = why, "google sign-in failed");
        (
            jar.remove(Cookie::build(STATE_COOKIE).path("/auth/google").build())
                .remove(Cookie::build(APP_COOKIE).path("/auth/google").build()),
            back_to_app(failed_to.clone()),
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
        (Ok(token), Some(app)) => match s.handoff.issue(token, app_return(&app).1.into()).await {
            Ok(code) => (
                jar,
                back_to_app(format!("{}?code={code}", app_return(&app).0)),
            )
                .into_response(),
            Err(e) => {
                tracing::error!(error = %e, "google app handoff");
                (jar, back_to_app(failed_to)).into_response()
            }
        },
        (Ok(token), None) => {
            (jar.add(session_cookie(&s.config, token)), Redirect::to("/")).into_response()
        }
        (Err(AuthError::NotAllowed), Some(app)) => (
            jar,
            back_to_app(format!("{}?error=not_allowed", app_return(&app).0)),
        )
            .into_response(),
        (Err(AuthError::NotAllowed), None) => {
            (jar, Redirect::to("/login?error=not_allowed")).into_response()
        }
        (Err(e), _) => {
            tracing::error!(error = %e, "google sign-in");
            (jar, back_to_app(failed_to)).into_response()
        }
    }
}

#[derive(Deserialize)]
struct AppSessionForm {
    code: String,
    verifier: String,
}

/// The desktop app's window, back from Google in the system browser, trades
/// the one-time code and its verifier for a normal web session cookie.
async fn app_session(
    State(s): State<AuthWeb>,
    jar: CookieJar,
    Form(f): Form<AppSessionForm>,
) -> Response {
    match s.handoff.redeem(f.code.trim(), &f.verifier).await {
        Ok(Some(token)) => {
            (jar.add(session_cookie(&s.config, token)), Redirect::to("/")).into_response()
        }
        Ok(None) => {
            tracing::warn!("desktop handoff refused (unknown, used, expired or wrong verifier)");
            Redirect::to("/login?error=google").into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "desktop handoff");
            Redirect::to("/login?error=google").into_response()
        }
    }
}

fn sha256_hex(s: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(s.as_bytes()))
}

/// Sign in with Apple on the website (and in the desktop app's window).
async fn apple_start(State(s): State<AuthWeb>, jar: CookieJar) -> Response {
    let Some(client_id) = s.apple_web.as_ref().map(|v| v.audience().to_string()) else {
        return Redirect::to("/login").into_response();
    };
    let state: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let redirect_uri = format!("{}/auth/apple/callback", s.config.public_url);
    let nonce = sha256_hex(&state);
    let url = reqwest::Url::parse_with_params(
        "https://appleid.apple.com/auth/authorize",
        &[
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code id_token"),
            ("response_mode", "form_post"),
            ("scope", "name email"),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
        ],
    )
    .expect("valid apple url");
    let cookie = Cookie::build((APPLE_STATE_COOKIE, state))
        .path("/auth/apple")
        .http_only(true)
        .secure(s.config.secure())
        .same_site(SameSite::None)
        .max_age(time::Duration::minutes(10))
        .build();
    (jar.add(cookie), Redirect::to(url.as_str())).into_response()
}

#[derive(Deserialize)]
struct AppleCallback {
    state: Option<String>,
    id_token: Option<String>,
    /// JSON with the person's name, sent only on their first sign-in.
    user: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct AppleUser {
    #[serde(default)]
    name: AppleName,
}

#[derive(Deserialize, Default)]
struct AppleName {
    #[serde(default, rename = "firstName")]
    first: String,
    #[serde(default, rename = "lastName")]
    last: String,
}

async fn apple_callback(
    State(s): State<AuthWeb>,
    jar: CookieJar,
    Form(f): Form<AppleCallback>,
) -> Response {
    let expected = jar.get(APPLE_STATE_COOKIE).map(|c| c.value().to_string());
    let jar = jar.remove(
        Cookie::build(APPLE_STATE_COOKIE)
            .path("/auth/apple")
            .build(),
    );
    let fail = |jar: CookieJar, why: &str| {
        tracing::warn!(reason = why, "apple web sign-in failed");
        (jar, Redirect::to("/login?error=apple")).into_response()
    };
    let Some(verifier) = &s.apple_web else {
        return Redirect::to("/login").into_response();
    };
    if let Some(e) = f.error {
        return fail(jar, &format!("apple said {e}"));
    }
    let (Some(state), Some(token)) = (f.state, f.id_token) else {
        return fail(jar, "missing state/id_token");
    };
    if expected.as_deref() != Some(state.as_str()) {
        return fail(jar, "state mismatch");
    }
    let identity = match verifier
        .verify_with_nonce(&token, Some(&sha256_hex(&state)))
        .await
    {
        Ok(i) => i,
        Err(why) => return fail(jar, &why),
    };
    let name = f
        .user
        .and_then(|u| serde_json::from_str::<AppleUser>(&u).ok())
        .map(|u| format!("{} {}", u.name.first.trim(), u.name.last.trim()))
        .unwrap_or_default();
    match s
        .service
        .sign_in_verified_email(&identity.email, name.trim())
        .await
    {
        Ok(token) => (jar.add(session_cookie(&s.config, token)), Redirect::to("/")).into_response(),
        Err(AuthError::NotAllowed) => {
            (jar, Redirect::to("/login?error=not_allowed")).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "apple web sign-in");
            (jar, Redirect::to("/login?error=apple")).into_response()
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

/// Opens a WhatsApp chat with the business number, pre-filling
/// "ربط حسابي في AQRGO: <code>".
pub(crate) fn whatsapp_link_url(number: &str, token: &str) -> String {
    let text = url_encode(&format!("ربط حسابي في AQRGO: {token}"));
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
