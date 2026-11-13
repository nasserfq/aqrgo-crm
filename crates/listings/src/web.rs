//! Server-rendered pages for the listings table. Expects `auth` middleware
//! to have put a `common::CurrentUser` into the request extensions.

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Form, Router,
};
use chrono::NaiveDate;
use common::{ui::Nav, CurrentUser};
use serde::Deserialize;

use crate::{
    Filter, Listing, ListingInput, ListingsError, ListingsService, Source, Status, FIELDS,
    OFFER_TYPE_SUGGESTIONS,
};

pub fn router(service: ListingsService) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/rows", get(rows))
        .route("/export.csv", get(export_csv))
        .route("/listings/new", get(new_form).post(create))
        .route("/listings/{id}", get(edit_form).post(update))
        .route("/listings/{id}/delete", post(delete))
        .with_state(service)
}

#[derive(Deserialize, Default)]
struct FilterQuery {
    q: Option<String>,
    status: Option<String>,
}

impl FilterQuery {
    fn to_filter(&self) -> Filter {
        Filter {
            q: self.q.clone().filter(|s| !s.trim().is_empty()),
            status: self.status.as_deref().and_then(Status::parse),
        }
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexPage {
    user: CurrentUser,
    nav: Nav,
    listings: Vec<Listing>,
    q: String,
    status: String,
    statuses: [Status; 3],
    fields: &'static [(&'static str, &'static str)],
}

#[derive(Template)]
#[template(path = "rows.html")]
struct RowsPartial {
    listings: Vec<Listing>,
}

#[derive(Template)]
#[template(path = "form.html")]
struct FormPage {
    user: CurrentUser,
    nav: Nav,
    id: Option<i64>,
    number: String,
    input: ListingInput,
    date_value: String,
    date_label: String,
    error: Option<String>,
    statuses: [Status; 3],
    fields: &'static [(&'static str, &'static str)],
    suggestions: &'static [&'static str],
}

impl FormPage {
    fn new(
        user: CurrentUser,
        id: Option<i64>,
        number: String,
        input: ListingInput,
        error: Option<String>,
    ) -> Self {
        let date = input.listed_on.unwrap_or_else(common::dates::today_riyadh);
        Self {
            user,
            nav: Nav::Listings,
            id,
            number,
            date_value: date.format("%Y-%m-%d").to_string(),
            date_label: common::dates::full_ar_display(date),
            input,
            error,
            statuses: Status::ALL,
            fields: FIELDS,
            suggestions: OFFER_TYPE_SUGGESTIONS,
        }
    }

    fn is_multiline(&self, key: &str) -> bool {
        matches!(key, "frontages" | "buyer" | "seller" | "broker")
    }
}

/// Error shown to the browser. Database details are only logged.
struct WebError(StatusCode, String);

impl From<ListingsError> for WebError {
    fn from(e: ListingsError) -> Self {
        tracing::error!(error = %e, "listings error");
        WebError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "حدث خطأ، حاول مرة أخرى".into(),
        )
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        (self.0, Html(format!("<!doctype html><html dir=rtl lang=ar><body style='font-family:sans-serif;padding:2rem'><p>{}</p><a href='/'>العودة</a></body></html>", self.1))).into_response()
    }
}

fn page<T: Template>(t: &T) -> Result<Html<String>, WebError> {
    common::ui::render(t).map(Html).map_err(|e| {
        tracing::error!(error = %e);
        WebError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "خطأ في عرض الصفحة".into(),
        )
    })
}

async fn index(
    State(svc): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Query(fq): Query<FilterQuery>,
) -> Result<Html<String>, WebError> {
    let listings = svc.list(user.id, &fq.to_filter()).await?;
    page(&IndexPage {
        user,
        nav: Nav::Listings,
        listings,
        q: fq.q.clone().unwrap_or_default(),
        status: fq.status.clone().unwrap_or_default(),
        statuses: Status::ALL,
        fields: FIELDS,
    })
}

async fn rows(
    State(svc): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Query(fq): Query<FilterQuery>,
) -> Result<Html<String>, WebError> {
    let listings = svc.list(user.id, &fq.to_filter()).await?;
    page(&RowsPartial { listings })
}

async fn new_form(Extension(user): Extension<CurrentUser>) -> Result<Html<String>, WebError> {
    page(&FormPage::new(
        user,
        None,
        String::new(),
        ListingInput::default(),
        None,
    ))
}

async fn edit_form(
    State(svc): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> Result<Response, WebError> {
    match svc.get(user.id, id).await? {
        Some(l) => Ok(page(&FormPage::new(
            user,
            Some(id),
            l.number(),
            l.to_input(),
            None,
        ))?
        .into_response()),
        None => Ok(Redirect::to("/").into_response()),
    }
}

#[derive(Deserialize)]
struct ListingForm {
    #[serde(default)]
    offer_type: String,
    #[serde(default)]
    plan_no: String,
    #[serde(default)]
    plot_no: String,
    #[serde(default)]
    area: String,
    #[serde(default)]
    frontages: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    price: String,
    #[serde(default)]
    buyer: String,
    #[serde(default)]
    seller: String,
    #[serde(default)]
    broker: String,
    #[serde(default)]
    listed_on: String,
    #[serde(default)]
    status: String,
}

impl From<ListingForm> for ListingInput {
    fn from(f: ListingForm) -> Self {
        ListingInput {
            offer_type: f.offer_type,
            plan_no: f.plan_no,
            plot_no: f.plot_no,
            area: f.area,
            frontages: f.frontages,
            location: f.location,
            price: f.price,
            buyer: f.buyer,
            seller: f.seller,
            broker: f.broker,
            listed_on: NaiveDate::parse_from_str(&f.listed_on, "%Y-%m-%d").ok(),
            status: Status::parse(&f.status).unwrap_or_default(),
        }
    }
}

async fn create(
    State(svc): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Form(form): Form<ListingForm>,
) -> Result<Response, WebError> {
    let input: ListingInput = form.into();
    match svc.create(user.id, input.clone(), Source::Web).await {
        Ok(_) => Ok(Redirect::to("/").into_response()),
        Err(ListingsError::Invalid(msg)) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            page(&FormPage::new(
                user,
                None,
                String::new(),
                input,
                Some(msg.into()),
            ))?,
        )
            .into_response()),
        Err(e) => Err(e.into()),
    }
}

async fn update(
    State(svc): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<i64>,
    Form(form): Form<ListingForm>,
) -> Result<Response, WebError> {
    let input: ListingInput = form.into();
    match svc.update(user.id, id, input.clone()).await {
        Ok(_) => Ok(Redirect::to("/").into_response()),
        Err(ListingsError::Invalid(msg)) => {
            let number = svc
                .get(user.id, id)
                .await?
                .map(|l| l.number())
                .unwrap_or_default();
            Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                page(&FormPage::new(
                    user,
                    Some(id),
                    number,
                    input,
                    Some(msg.into()),
                ))?,
            )
                .into_response())
        }
        Err(e) => Err(e.into()),
    }
}

async fn delete(
    State(svc): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<i64>,
) -> Result<Redirect, WebError> {
    svc.delete(user.id, id).await?;
    Ok(Redirect::to("/"))
}

/// CSV with a UTF-8 BOM so Excel opens the Arabic text correctly.
async fn export_csv(
    State(svc): State<ListingsService>,
    Extension(user): Extension<CurrentUser>,
    Query(fq): Query<FilterQuery>,
) -> Result<Response, WebError> {
    let listings = svc.list(user.id, &fq.to_filter()).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"aqrgo-listings.csv\"",
            ),
        ],
        to_csv(&listings),
    )
        .into_response())
}

fn csv_cell(s: &str) -> String {
    // Neutralise spreadsheet formulas and quote everything.
    let s = if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s.to_string()
    };
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub(crate) fn to_csv(listings: &[Listing]) -> String {
    let mut out = String::from("\u{FEFF}");
    let mut header = vec!["م"];
    header.extend(FIELDS.iter().map(|(_, label)| *label));
    header.extend(["اليوم والتاريخ", "الحالة"]);
    out.push_str(
        &header
            .iter()
            .map(|h| csv_cell(h))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push_str("\r\n");
    for l in listings {
        let input = l.to_input();
        let mut row = vec![l.ref_no.to_string()];
        row.extend(FIELDS.iter().map(|(k, _)| input.get(k).to_string()));
        row.push(l.date_label());
        row.push(l.status().label_ar().to_string());
        out.push_str(
            &row.iter()
                .map(|c| csv_cell(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_has_bom_header_and_escapes() {
        let l = Listing {
            id: 1,
            ref_no: 3,
            offer_type: "أرض للبيع".into(),
            plan_no: "=cmd".into(),
            plot_no: "12".into(),
            area: "625".into(),
            frontages: "شمالية \"30م\"".into(),
            location: "".into(),
            price: "".into(),
            buyer: "".into(),
            seller: "".into(),
            broker: "".into(),
            listed_on: NaiveDate::from_ymd_opt(2026, 11, 13).unwrap(),
            status: "sold".into(),
            source: "web".into(),
        };
        let csv = to_csv(&[l]);
        assert!(csv.starts_with("\u{FEFF}\"م\",\"نوع العرض\""));
        assert!(csv.contains("\"'=cmd\""));
        assert!(csv.contains("\"شمالية \"\"30م\"\"\""));
        assert!(csv.contains("\"مباع\""));
    }
}
