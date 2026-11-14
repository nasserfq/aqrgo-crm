//! Pages for the sites module: the owner's settings form (behind sign-in),
//! and the public site at `/s/<slug>` and at `<slug>.<base domain>`.

use std::collections::BTreeSet;

use askama::Template;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Extension, Form, Router,
};
use common::{ui::Nav, CurrentUser};
use serde::Deserialize;

use crate::{
    model::{slug_in_host, MAX_LOGO_BYTES},
    Logo, PublicListing, PublicSite, Site, SiteInput, SitesError, SitesService, DEFAULT_ACCENT,
};

#[derive(Clone)]
pub struct SitesWeb {
    service: SitesService,
    /// Sites live one level under this domain, e.g. `aqrgo.com`.
    base_domain: String,
    /// `https` in production, so the settings page links the right address.
    scheme: String,
}

impl SitesWeb {
    /// `public_url` is the CRM's own address, e.g. `https://aqrgo.com`.
    pub fn new(service: SitesService, public_url: &str) -> Self {
        let (scheme, rest) = public_url
            .split_once("://")
            .unwrap_or(("https", public_url));
        let host = rest.split(['/', ':']).next().unwrap_or("").to_lowercase();
        let base_domain = host.strip_prefix("www.").unwrap_or(&host).to_string();
        Self {
            service,
            base_domain,
            scheme: scheme.to_string(),
        }
    }
}

/// The owner's settings page. Needs `common::CurrentUser` in extensions.
pub fn settings_router(state: SitesWeb) -> Router {
    Router::new()
        .route("/site", get(settings_page).post(save))
        .route(
            "/site/logo",
            get(own_logo)
                .post(upload_logo)
                .layer(DefaultBodyLimit::max(MAX_LOGO_BYTES + 64 * 1024)),
        )
        .with_state(state)
}

/// Public pages on the CRM's own address, plus the certificate check the
/// proxy calls before it asks Let's Encrypt for `<slug>.<base domain>`.
pub fn public_router(state: SitesWeb) -> Router {
    Router::new()
        .route("/s/{slug}", get(site_by_path))
        .route("/s/{slug}/logo", get(logo_by_path))
        .route("/sites/tls-check", get(tls_check))
        .with_state(state)
}

/// Middleware for the whole app: a request for `<slug>.<base domain>` is
/// answered here with that site, and never reaches the CRM's own pages.
pub async fn subdomain(State(s): State<SitesWeb>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let Some(slug) = slug_in_host(host, &s.base_domain) else {
        return next.run(req).await;
    };
    match req.uri().path() {
        "/" => render_site(&s, &slug, "/logo".into()).await,
        "/logo" => serve_logo(&s, &slug).await,
        _ => not_found(),
    }
}

// ---------- public site ----------

#[derive(Template)]
#[template(path = "site.html")]
struct SitePage {
    site: Site,
    listings: Vec<PublicListing>,
    logo_url: String,
}

impl SitePage {
    fn initials(&self) -> String {
        self.site.company_name.chars().take(1).collect()
    }
}

async fn site_by_path(State(s): State<SitesWeb>, Path(slug): Path<String>) -> Response {
    let logo = format!("/s/{slug}/logo");
    render_site(&s, &slug, logo).await
}

async fn logo_by_path(State(s): State<SitesWeb>, Path(slug): Path<String>) -> Response {
    serve_logo(&s, &slug).await
}

async fn render_site(s: &SitesWeb, slug: &str, logo_url: String) -> Response {
    match s.service.public(slug).await {
        Ok(Some(PublicSite { site, listings })) => html(&SitePage {
            site,
            listings,
            logo_url,
        }),
        Ok(None) => not_found(),
        Err(e) => server_error(e),
    }
}

async fn serve_logo(s: &SitesWeb, slug: &str) -> Response {
    match s.service.logo(slug).await {
        Ok(Some(logo)) => logo_response(logo),
        Ok(None) => not_found(),
        Err(e) => server_error(e),
    }
}

#[derive(Deserialize)]
struct TlsQuery {
    host: Option<String>,
}

async fn tls_check(State(s): State<SitesWeb>, Query(q): Query<TlsQuery>) -> Response {
    let host = q.host.unwrap_or_default();
    match s.service.slug_for_host(&host, &s.base_domain).await {
        Ok(Some(_)) => StatusCode::OK.into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => server_error(e),
    }
}

// ---------- settings ----------

/// One available listing on the form, with whether the site shows it.
struct ListingChoice {
    id: i64,
    label: String,
    shown: bool,
}

#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsPage {
    user: CurrentUser,
    nav: Nav,
    input: SiteInput,
    exists: bool,
    has_logo: bool,
    choices: Vec<ListingChoice>,
    site_url: String,
    path_url: String,
    base_domain: String,
    saved: bool,
    error: Option<String>,
}

#[derive(Deserialize, Default)]
struct SettingsQuery {
    saved: Option<String>,
}

impl SettingsPage {
    async fn build(
        s: &SitesWeb,
        user: CurrentUser,
        input: SiteInput,
        site: Option<&Site>,
        error: Option<String>,
        saved: bool,
    ) -> Result<Self, SitesError> {
        let hidden = input.hidden_listings.clone();
        let choices = s
            .service
            .available_listings(user.id)
            .await?
            .into_iter()
            .map(|l| ListingChoice {
                id: l.id,
                label: [l.offer_type.as_str(), l.location.as_str(), l.price.as_str()]
                    .iter()
                    .filter(|v| !v.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" · "),
                shown: !hidden.contains(&l.id),
            })
            .collect();
        let slug = site.map(|x| x.slug.clone()).unwrap_or_default();
        Ok(Self {
            nav: Nav::Site,
            site_url: format!("{}://{}.{}", s.scheme, slug, s.base_domain),
            path_url: format!("/s/{slug}"),
            base_domain: s.base_domain.clone(),
            exists: site.is_some(),
            has_logo: site.is_some_and(|x| x.has_logo),
            user,
            input,
            choices,
            saved,
            error,
        })
    }
}

async fn settings_page(
    State(s): State<SitesWeb>,
    Extension(user): Extension<CurrentUser>,
    Query(q): Query<SettingsQuery>,
) -> Response {
    let result = async {
        let site = s.service.get(user.id).await?;
        let input = match &site {
            Some(site) => site.to_input(s.service.hidden_listings(user.id).await?),
            None => SiteInput {
                company_name: user.name.clone(),
                accent: DEFAULT_ACCENT.into(),
                ..Default::default()
            },
        };
        SettingsPage::build(&s, user, input, site.as_ref(), None, q.saved.is_some()).await
    }
    .await;
    match result {
        Ok(page) => html(&page),
        Err(e) => server_error(e),
    }
}

/// The posted form, as pairs: listing checkboxes repeat the same key.
#[derive(Deserialize)]
#[serde(transparent)]
struct SettingsForm(Vec<(String, String)>);

impl SettingsForm {
    fn value(&self, key: &str) -> String {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }

    fn ids(&self, key: &str) -> BTreeSet<i64> {
        self.0
            .iter()
            .filter(|(k, _)| k == key)
            .filter_map(|(_, v)| v.parse().ok())
            .collect()
    }

    /// Listings on the form that were left unticked are hidden. Hidden
    /// listings that were not on the form (reserved or sold now) stay hidden.
    fn into_input(self, previously_hidden: Vec<i64>) -> SiteInput {
        let listed = self.ids("listed");
        let shown = self.ids("show");
        let mut hidden: BTreeSet<i64> = previously_hidden
            .into_iter()
            .filter(|id| !listed.contains(id))
            .collect();
        hidden.extend(listed.difference(&shown));
        SiteInput {
            slug: self.value("slug"),
            published: self.value("published") == "on",
            company_name: self.value("company_name"),
            tagline: self.value("tagline"),
            about: self.value("about"),
            services: self.value("services"),
            license_no: self.value("license_no"),
            phone: self.value("phone"),
            whatsapp: self.value("whatsapp"),
            email: self.value("email"),
            address: self.value("address"),
            map_url: self.value("map_url"),
            accent: self.value("accent"),
            hidden_listings: hidden.into_iter().collect(),
        }
    }
}

async fn save(
    State(s): State<SitesWeb>,
    Extension(user): Extension<CurrentUser>,
    Form(form): Form<SettingsForm>,
) -> Response {
    let previously_hidden = match s.service.hidden_listings(user.id).await {
        Ok(h) => h,
        Err(e) => return server_error(e),
    };
    let input = form.into_input(previously_hidden);
    match s.service.save(user.id, input.clone()).await {
        Ok(_) => Redirect::to("/site?saved=1").into_response(),
        Err(SitesError::Invalid(msg)) => invalid(&s, user, Some(input), msg).await,
        Err(e) => server_error(e),
    }
}

const LOGO_TOO_BIG: &str = "الشعار أكبر من ٥١٢ كيلوبايت";

async fn upload_logo(
    State(s): State<SitesWeb>,
    Extension(user): Extension<CurrentUser>,
    mut multipart: Multipart,
) -> Response {
    let mut upload = Err(SitesError::Invalid("اختر صورة الشعار أولاً"));
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) if field.name() == Some("logo") => {
                // Reading fails when the body is over the route's size limit.
                match field.bytes().await {
                    Ok(b) if !b.is_empty() => upload = Ok(b.to_vec()),
                    Ok(_) => {}
                    Err(_) => upload = Err(SitesError::Invalid(LOGO_TOO_BIG)),
                }
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {
                upload = Err(SitesError::Invalid(LOGO_TOO_BIG));
                break;
            }
        }
    }
    let result = match upload {
        Ok(bytes) => s.service.set_logo(user.id, bytes).await,
        Err(e) => Err(e),
    };
    match result {
        Ok(()) => Redirect::to("/site?saved=1").into_response(),
        Err(SitesError::Invalid(msg)) => invalid(&s, user, None, msg).await,
        Err(e) => server_error(e),
    }
}

/// The owner's logo for the settings preview, even before publishing.
async fn own_logo(State(s): State<SitesWeb>, Extension(user): Extension<CurrentUser>) -> Response {
    match s.service.own_logo(user.id).await {
        Ok(Some(logo)) => logo_response(logo),
        Ok(None) => not_found(),
        Err(e) => server_error(e),
    }
}

/// The settings form again with an error. `input` is what was posted, or
/// `None` to show what is saved.
async fn invalid(
    s: &SitesWeb,
    user: CurrentUser,
    input: Option<SiteInput>,
    msg: &'static str,
) -> Response {
    let page = async {
        let site = s.service.get(user.id).await?;
        let input = match (input, &site) {
            (Some(i), _) => i,
            (None, Some(x)) => x.to_input(s.service.hidden_listings(user.id).await?),
            (None, None) => SiteInput::default(),
        };
        SettingsPage::build(s, user, input, site.as_ref(), Some(msg.into()), false).await
    }
    .await;
    match page {
        Ok(page) => (StatusCode::UNPROCESSABLE_ENTITY, html(&page)).into_response(),
        Err(e) => server_error(e),
    }
}

// ---------- helpers ----------

fn logo_response(Logo(bytes, content_type): Logo) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=300".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        bytes,
    )
        .into_response()
}

fn html<T: Template>(t: &T) -> Response {
    match common::ui::render(t) {
        Ok(body) => Html(body).into_response(),
        Err(e) => {
            tracing::error!(error = %e);
            (StatusCode::INTERNAL_SERVER_ERROR, "خطأ في عرض الصفحة").into_response()
        }
    }
}

fn not_found() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/html; charset=utf-8".parse().expect("valid header"),
    );
    (
        StatusCode::NOT_FOUND,
        headers,
        Body::from("<!doctype html><html dir=rtl lang=ar><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><body style='font-family:sans-serif;padding:3rem;text-align:center'><h1>الصفحة غير موجودة</h1><p>لا يوجد موقع منشور بهذا العنوان.</p></body></html>"),
    )
        .into_response()
}

fn server_error(e: SitesError) -> Response {
    tracing::error!(error = %e, "sites error");
    (StatusCode::INTERNAL_SERVER_ERROR, "حدث خطأ، حاول مرة أخرى").into_response()
}
