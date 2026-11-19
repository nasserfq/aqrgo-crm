//! JSON sign-in for the iOS and Android apps. The app keeps the token from
//! `/api/v1/auth/verify` in the phone's secure storage and sends it as
//! `Authorization: Bearer <token>`. Tokens are ordinary sessions (same expiry,
//! same allow-list check), so signing out on the web and in the app behave alike.

use axum::{
    extract::{Path, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use common::CurrentUser;
use serde::Deserialize;
use serde_json::json;

use crate::{
    web::{telegram_link_url, whatsapp_link_url, AuthWeb},
    AuthError, Channel,
};

/// A JSON error body: `{"error": "<Arabic message>"}`.
pub fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn auth_error(e: AuthError) -> Response {
    let status = match &e {
        AuthError::InvalidEmail => StatusCode::UNPROCESSABLE_ENTITY,
        AuthError::InvalidCode => StatusCode::UNAUTHORIZED,
        AuthError::TooManyAttempts | AuthError::TooManyRequests | AuthError::LockedForToday => {
            StatusCode::TOO_MANY_REQUESTS
        }
        AuthError::NotAllowed => StatusCode::FORBIDDEN,
        AuthError::Mail(_) | AuthError::Db(_) => {
            tracing::error!(error = %e, "api sign-in failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    let message = match e {
        AuthError::Db(_) => "حدث خطأ، حاول مرة أخرى".to_string(),
        other => other.to_string(),
    };
    error(status, message)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Public sign-in routes.
pub fn public_router(state: AuthWeb) -> Router {
    Router::new()
        .route("/api/v1/auth/code", post(request_code))
        .route("/api/v1/auth/verify", post(verify_code))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/options", get(options))
        .route("/api/v1/auth/google", post(google))
        .route("/api/v1/auth/apple", post(apple))
        .with_state(state)
}

/// Routes that need a token (wrap with [`require_token`]).
pub fn account_router(state: AuthWeb) -> Router {
    Router::new()
        .route("/api/v1/me", get(me))
        .route("/api/v1/auth/apple/link", post(link_apple))
        .route("/api/v1/channels", get(channels))
        .route("/api/v1/channels/{channel}/link", post(link_channel))
        .route("/api/v1/channels/{channel}", delete(unlink_channel))
        .with_state(state)
}

/// Middleware: loads the user from the bearer token, or answers 401 JSON.
pub async fn require_token(State(state): State<AuthWeb>, mut req: Request, next: Next) -> Response {
    let user = match bearer(req.headers()) {
        Some(t) => match state.service.user_for_session(t).await {
            Ok(u) => u,
            Err(e) => return auth_error(e),
        },
        None => None,
    };
    match user {
        Some(user) => {
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        None => error(StatusCode::UNAUTHORIZED, "سجّل الدخول من جديد"),
    }
}

#[derive(Deserialize)]
struct CodeRequest {
    email: String,
}

async fn request_code(State(state): State<AuthWeb>, Json(r): Json<CodeRequest>) -> Response {
    match state.service.request_code(&r.email).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => auth_error(e),
    }
}

#[derive(Deserialize)]
struct VerifyRequest {
    email: String,
    code: String,
}

async fn verify_code(State(state): State<AuthWeb>, Json(r): Json<VerifyRequest>) -> Response {
    let token = match state.service.verify_code(&r.email, &r.code).await {
        Ok(t) => t,
        Err(e) => return auth_error(e),
    };
    match state.service.user_for_session(&token).await {
        Ok(Some(user)) => Json(json!({ "token": token, "user": user })).into_response(),
        Ok(None) => auth_error(AuthError::NotAllowed),
        Err(e) => auth_error(e),
    }
}

/// Which sign-in buttons the app should show.
async fn options(State(state): State<AuthWeb>) -> Response {
    Json(json!({ "google": state.config.google.is_some() })).into_response()
}

#[derive(Deserialize)]
struct GoogleRequest {
    code: String,
    verifier: String,
}

/// Finishes Google sign-in for the app: trades the one-time code from the
/// `aqrgo://auth?code=…` redirect, plus the app's verifier, for a token.
async fn google(State(state): State<AuthWeb>, Json(r): Json<GoogleRequest>) -> Response {
    // iOS can hand the app the return URL with an empty fragment
    // (`aqrgo://auth?code=…#`), and app builds 1.0.0 (9) and (10) keep the `#`.
    let code = r.code.split('#').next().unwrap_or_default().trim();
    let redeemed = match state.handoff.redeem(code, &r.verifier).await {
        Ok(t) => t,
        Err(e) => return auth_error(AuthError::Db(e)),
    };
    let Some(token) = redeemed else {
        tracing::warn!("google app handoff refused (unknown, used, expired or wrong verifier)");
        return error(
            StatusCode::UNAUTHORIZED,
            "تعذر الدخول عبر Google، حاول مرة أخرى",
        );
    };
    match state.service.user_for_session(&token).await {
        Ok(Some(user)) => Json(json!({ "token": token, "user": user })).into_response(),
        Ok(None) => auth_error(AuthError::NotAllowed),
        Err(e) => auth_error(e),
    }
}

#[derive(Deserialize)]
struct AppleRequest {
    identity_token: String,
    /// Apple shares the name only on the first sign-in, from the device.
    #[serde(default)]
    name: String,
}

/// Sign in with Apple: checks Apple's identity token and returns our token.
async fn apple(State(state): State<AuthWeb>, Json(r): Json<AppleRequest>) -> Response {
    let identity = match state.apple.verify(&r.identity_token).await {
        Ok(i) => i,
        Err(why) => {
            tracing::warn!(reason = %why, "apple sign-in refused");
            return error(
                StatusCode::UNAUTHORIZED,
                "تعذر الدخول عبر Apple، حاول مرة أخرى",
            );
        }
    };
    let token = match state
        .service
        .sign_in_verified_email(&identity.email, r.name.trim())
        .await
    {
        Ok(t) => t,
        Err(e) => return auth_error(e),
    };
    match state.service.user_for_session(&token).await {
        Ok(Some(user)) => Json(json!({ "token": token, "user": user })).into_response(),
        Ok(None) => auth_error(AuthError::NotAllowed),
        Err(e) => auth_error(e),
    }
}

/// Links the Apple ID in `identity_token` to the signed-in account, so Sign in
/// with Apple opens this account even when Apple hides the email.
async fn link_apple(
    State(state): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
    Json(r): Json<AppleRequest>,
) -> Response {
    let identity = match state.apple.verify(&r.identity_token).await {
        Ok(i) => i,
        Err(why) => {
            tracing::warn!(reason = %why, "apple link refused");
            return error(StatusCode::UNAUTHORIZED, "تعذر ربط Apple، حاول مرة أخرى");
        }
    };
    match state
        .service
        .add_sign_in_email(user.id, &identity.email)
        .await
    {
        Ok(()) => Json(json!({ "linked": true })).into_response(),
        Err(e) => auth_error(e),
    }
}

async fn logout(State(state): State<AuthWeb>, headers: HeaderMap) -> Response {
    if let Some(t) = bearer(&headers) {
        if let Err(e) = state.service.sign_out(t).await {
            return auth_error(e);
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn me(Extension(user): Extension<CurrentUser>) -> Json<CurrentUser> {
    Json(user)
}

/// The chat channel in a URL, if this server runs a bot for it.
fn channel(state: &AuthWeb, name: &str) -> Option<Channel> {
    match name {
        "telegram" if state.config.telegram_bot.is_some() => Some(Channel::Telegram),
        "whatsapp" if state.config.whatsapp_number.is_some() => Some(Channel::WhatsApp),
        _ => None,
    }
}

fn no_channel() -> Response {
    error(StatusCode::NOT_FOUND, "هذه القناة غير متاحة")
}

/// Which chat bots this server runs, and which the user has linked.
async fn channels(
    State(state): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
) -> Response {
    let mut out = serde_json::Map::new();
    for (name, ch, available) in [
        (
            "telegram",
            Channel::Telegram,
            state.config.telegram_bot.is_some(),
        ),
        (
            "whatsapp",
            Channel::WhatsApp,
            state.config.whatsapp_number.is_some(),
        ),
    ] {
        let linked = match state.service.linked(ch, user.id).await {
            Ok(l) => l,
            Err(e) => return auth_error(e),
        };
        out.insert(
            name.into(),
            json!({ "available": available, "linked": linked }),
        );
    }
    Json(out).into_response()
}

/// A one-time link that opens the bot and ties that chat account to the user.
async fn link_channel(
    State(state): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
    Path(name): Path<String>,
) -> Response {
    let Some(ch) = channel(&state, &name) else {
        return no_channel();
    };
    let token = match state.service.create_link(user.id).await {
        Ok(t) => t,
        Err(e) => return auth_error(e),
    };
    let url = match ch {
        Channel::Telegram => telegram_link_url(
            state.config.telegram_bot.as_deref().unwrap_or_default(),
            &token,
        ),
        Channel::WhatsApp => whatsapp_link_url(
            state.config.whatsapp_number.as_deref().unwrap_or_default(),
            &token,
        ),
    };
    Json(json!({ "url": url })).into_response()
}

async fn unlink_channel(
    State(state): State<AuthWeb>,
    Extension(user): Extension<CurrentUser>,
    Path(name): Path<String>,
) -> Response {
    let Some(ch) = channel(&state, &name) else {
        return no_channel();
    };
    match state.service.unlink(ch, user.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => auth_error(e),
    }
}
