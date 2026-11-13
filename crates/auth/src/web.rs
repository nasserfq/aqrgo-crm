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

use crate::{AuthError, AuthService};

pub const SESSION_COOKIE: &str = "aqrgo_session";
const STATE_COOKIE: &str = "aqrgo_oauth_state";

#[derive(Clone, Debug)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Clone, Debug)]
pub struct WebConfig {
    /// e.g. https://aqrgo.com (no trailing slash)
    pub public_url: String,
    pub google: Option<GoogleConfig>,
    /// Telegram bot username without '@', for the account-linking deep link.
    pub telegram_bot: Option<String>,
}

impl WebConfig {
    fn secure(&self) -> bool {
        self.public_url.starts_with("https://")
    }
}

#[derive(Clone)]
pub struct AuthWeb {
    pub service: AuthService,
    pub config: WebConfig,
    http: reqwest::Client,
}

impl AuthWeb {
    pub fn new(service: AuthService, config: WebConfig) -> Self {
        Self {
            service,
            config,
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
        .with_state(state)
}

/// Routes that need a signed-in user (wrap with [`require_user`]).
pub fn account_router(state: AuthWeb) -> Router {
    Router::new()
        .route("/account", get(account_page))
        .route("/account/telegram", post(telegram_link))
        .route("/account/telegram/unlink", post(telegram_unlink))
        .with_state(state)
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
        .max_age(time_days(30))
        .build()
}

fn time_days(d: i64) -> time::Duration {
    time::Duration::days(d)
}

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
            let step = if matches!(e, AuthError::TooManyAttempts) {
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

async fn google_start(State(s): State<AuthWeb>, jar: CookieJar) -> Response {
    let Some(g) = &s.config.google else {
        return Redirect::to("/login").into_response();
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
    let fail = |jar: CookieJar, why: &str| {
        tracing::warn!(reason = why, "google sign-in failed");
        (
            jar.remove(Cookie::build(STATE_COOKIE).path("/auth/google").build()),
            Redirect::to("/login?error=google"),
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
    let jar = jar.remove(Cookie::build(STATE_COOKIE).path("/auth/google").build());
    match s
        .service
        .sign_in_verified_email(&user.email, &user.name)
        .await
    {
        Ok(token) => (jar.add(session_cookie(&s.config, token)), Redirect::to("/")).into_response(),
        Err(AuthError::NotAllowed) => Redirect::to("/login?error=not_allowed").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "google sign-in");
            Redirect::to("/login?error=google").into_response()
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
    link_url: Option<String>,
}

async fn account_page(
    State(s): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    let telegram_linked = s.service.telegram_linked(user.id).await.unwrap_or(false);
    render(&AccountPage {
        user,
        nav: Nav::Account,
        telegram_linked,
        telegram_bot: s.config.telegram_bot.clone(),
        link_url: None,
    })
}

async fn telegram_link(
    State(s): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    let Some(bot) = s.config.telegram_bot.clone() else {
        return Redirect::to("/account").into_response();
    };
    match s.service.create_telegram_link(user.id).await {
        Ok(token) => {
            let telegram_linked = s.service.telegram_linked(user.id).await.unwrap_or(false);
            let link_url = Some(format!("https://t.me/{bot}?start={token}"));
            render(&AccountPage {
                user,
                nav: Nav::Account,
                telegram_linked,
                telegram_bot: Some(bot),
                link_url,
            })
        }
        Err(e) => {
            tracing::error!(error = %e, "create telegram link");
            (StatusCode::INTERNAL_SERVER_ERROR, "error").into_response()
        }
    }
}

async fn telegram_unlink(
    State(s): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    if let Err(e) = s.service.unlink_telegram(user.id).await {
        tracing::error!(error = %e, "unlink telegram");
    }
    (StatusCode::SEE_OTHER, [(header::LOCATION, "/account")]).into_response()
}
