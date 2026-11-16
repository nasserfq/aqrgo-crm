//! `POST /api/v1/push/devices` `{"token", "platform"}` and
//! `DELETE /api/v1/push/devices` `{"token"}`. Behind the auth token middleware.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Extension, Json, Router,
};
use common::CurrentUser;
use serde::Deserialize;
use serde_json::json;

use crate::{PushError, PushService};

pub fn router(service: PushService) -> Router {
    Router::new()
        .route("/api/v1/push/devices", post(add).delete(remove))
        .with_state(service)
}

fn failed(e: PushError) -> Response {
    let status = match &e {
        PushError::InvalidToken | PushError::InvalidPlatform => StatusCode::UNPROCESSABLE_ENTITY,
        PushError::Db(_) | PushError::Queue(_) => {
            tracing::error!(error = %e, "push api failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "حدث خطأ، حاول مرة أخرى" })),
            )
                .into_response();
        }
    };
    (status, Json(json!({ "error": e.to_string() }))).into_response()
}

#[derive(Deserialize)]
struct Device {
    token: String,
    #[serde(default)]
    platform: String,
}

async fn add(
    State(s): State<PushService>,
    Extension(user): Extension<CurrentUser>,
    Json(d): Json<Device>,
) -> Response {
    match s.register_device(user.id, &d.token, &d.platform).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failed(e),
    }
}

async fn remove(
    State(s): State<PushService>,
    Extension(user): Extension<CurrentUser>,
    Json(d): Json<Device>,
) -> Response {
    match s.unregister_device(user.id, &d.token).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failed(e),
    }
}
