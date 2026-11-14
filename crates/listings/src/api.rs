//! JSON API over the property table, for the iOS and Android apps.
//! Same service and rules as the web pages; every route is scoped to the
//! signed-in user put in the request by the auth module's token middleware.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use common::CurrentUser;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Filter, Listing, ListingInput, ListingsError, ListingsService, Source, Status, FIELDS,
    OFFER_TYPE_SUGGESTIONS,
};

pub fn router(service: ListingsService) -> Router {
    Router::new()
        .route("/api/v1/listings", get(list).post(create))
        .route("/api/v1/listings/meta", get(meta))
        .route(
            "/api/v1/listings/{id}",
            get(show).put(update).delete(remove),
        )
        .with_state(service)
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "السجل غير موجود")
}

fn failed(e: ListingsError) -> Response {
    match e {
        ListingsError::Invalid(m) => error(StatusCode::UNPROCESSABLE_ENTITY, m),
        ListingsError::Db(e) => {
            tracing::error!(error = %e, "listings api failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "حدث خطأ، حاول مرة أخرى")
        }
    }
}

/// A row as the app shows it: the stored fields plus ready-made Arabic labels.
#[derive(Serialize)]
struct Row {
    #[serde(flatten)]
    listing: Listing,
    number: String,
    status_label: &'static str,
    date_hijri: String,
    date_gregorian: String,
}

impl From<Listing> for Row {
    fn from(l: Listing) -> Self {
        Row {
            number: l.number(),
            status_label: l.status().label_ar(),
            date_hijri: l.date_hijri(),
            date_gregorian: l.date_gregorian(),
            listing: l,
        }
    }
}

#[derive(Deserialize)]
struct ListQuery {
    q: Option<String>,
    status: Option<String>,
}

async fn list(
    State(s): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Query(q): Query<ListQuery>,
) -> Response {
    let filter = Filter {
        q: q.q,
        status: q.status.as_deref().and_then(Status::parse),
    };
    match s.list(user.id, &filter).await {
        Ok(rows) => {
            let rows: Vec<Row> = rows.into_iter().map(Row::from).collect();
            Json(json!({ "listings": rows })).into_response()
        }
        Err(e) => failed(e),
    }
}

async fn show(
    State(s): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> Response {
    match s.get(user.id, id).await {
        Ok(Some(l)) => Json(Row::from(l)).into_response(),
        Ok(None) => not_found(),
        Err(e) => failed(e),
    }
}

/// Every field is optional in the request body; missing ones are empty.
#[derive(Deserialize)]
struct Body {
    #[serde(flatten)]
    fields: std::collections::HashMap<String, serde_json::Value>,
}

impl Body {
    /// Errors are Arabic messages for a 422.
    fn into_input(self) -> Result<ListingInput, &'static str> {
        let mut input = ListingInput::default();
        for (key, _) in FIELDS {
            if let Some(v) = self.fields.get(*key).and_then(|v| v.as_str()) {
                input.set(key, v.to_string());
            }
        }
        if let Some(d) = self.fields.get("listed_on").and_then(|v| v.as_str()) {
            if !d.is_empty() {
                input.listed_on = Some(d.parse().map_err(|_| "التاريخ غير صحيح")?);
            }
        }
        if let Some(st) = self.fields.get("status").and_then(|v| v.as_str()) {
            input.status = Status::parse(st).ok_or("الحالة غير صحيحة")?;
        }
        Ok(input)
    }
}

async fn create(
    State(s): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Json(body): Json<Body>,
) -> Response {
    let input = match body.into_input() {
        Ok(i) => i,
        Err(m) => return error(StatusCode::UNPROCESSABLE_ENTITY, m),
    };
    match s.create(user.id, input, Source::Web).await {
        Ok(l) => (StatusCode::CREATED, Json(Row::from(l))).into_response(),
        Err(e) => failed(e),
    }
}

async fn update(
    State(s): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<i64>,
    Json(body): Json<Body>,
) -> Response {
    let input = match body.into_input() {
        Ok(i) => i,
        Err(m) => return error(StatusCode::UNPROCESSABLE_ENTITY, m),
    };
    match s.update(user.id, id, input).await {
        Ok(Some(l)) => Json(Row::from(l)).into_response(),
        Ok(None) => not_found(),
        Err(e) => failed(e),
    }
}

async fn remove(
    State(s): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> Response {
    match s.delete(user.id, id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => not_found(),
        Err(e) => failed(e),
    }
}

/// The form's fields, quick offer types and statuses, so the app builds its
/// form from the server and stays in step with the web.
async fn meta() -> Json<serde_json::Value> {
    Json(json!({
        "fields": FIELDS.iter().map(|(k, l)| json!({ "key": k, "label": l })).collect::<Vec<_>>(),
        "offer_types": OFFER_TYPE_SUGGESTIONS,
        "statuses": Status::ALL.iter().map(|s| json!({ "value": s.as_str(), "label": s.label_ar() })).collect::<Vec<_>>(),
    }))
}
