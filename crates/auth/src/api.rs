//! JSON sign-in for the iOS and Android apps. The app keeps the token from
//! `/api/v1/auth/verify` in the phone's secure storage and sends it as
//! `Authorization: Bearer <token>`. Tokens are ordinary sessions (same expiry,
//! same allow-list check), so signing out on the web and in the app behave alike.

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use common::CurrentUser;
use serde::Deserialize;
use serde_json::json;

use crate::{web::AuthWeb, AuthError};

/// A JSON error body: `{"error": "<Arabic message>"}`.
pub fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

fn auth_error(e: AuthError) -> Response {
    let status = match &e {
        AuthError::InvalidEmail => StatusCode::UNPROCESSABLE_ENTITY,
        AuthError::InvalidCode => StatusCode::UNAUTHORIZED,
        AuthError::TooManyAttempts | AuthError::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
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
        .with_state(state)
}

/// Routes that need a token (wrap with [`require_token`]).
pub fn account_router() -> Router {
    Router::new().route("/api/v1/me", get(me))
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
