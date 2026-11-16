//! Composition root: wires the modules together. No business logic here.

use std::sync::Arc;

use axum::{http::header, middleware, response::IntoResponse, routing::get, Router};
use sqlx::PgPool;

pub mod config;
pub mod telemetry;

pub use config::{Config, WhatsAppConfig};

/// Runs every module's migrations (each in its own schema).
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    for (schema, m) in [
        (jobs::SCHEMA, &jobs::MIGRATOR),
        (auth::SCHEMA, &auth::MIGRATOR),
        (listings::SCHEMA, &listings::MIGRATOR),
        (telegram::SCHEMA, &telegram::MIGRATOR),
        (whatsapp::SCHEMA, &whatsapp::MIGRATOR),
        (sites::SCHEMA, &sites::MIGRATOR),
        (push::SCHEMA, &push::MIGRATOR),
    ] {
        common::db::migrate_module(pool, schema, m).await?;
    }
    Ok(())
}

pub struct Modules {
    pub auth_web: auth::web::AuthWeb,
    pub listings: listings::ListingsService,
    pub sites: sites::web::SitesWeb,
    pub push: push::PushService,
    pub jobs: jobs::Jobs,
    pub telegram_secret: Option<String>,
    pub whatsapp: Option<whatsapp::webhook::HookConfig>,
    pool: PgPool,
}

impl Modules {
    pub fn new(
        pool: PgPool,
        jobs: jobs::Jobs,
        code_mailer: Arc<dyn auth::CodeMailer>,
        auth_config: auth::AuthConfig,
        web_config: auth::web::WebConfig,
        telegram_secret: Option<String>,
        whatsapp: Option<whatsapp::webhook::HookConfig>,
    ) -> Self {
        let auth = auth::AuthService::new(pool.clone(), code_mailer, auth_config);
        let push = push::PushService::new(pool.clone(), jobs.clone());
        // New rows that arrive by message notify the owner's phones.
        let listings =
            listings::ListingsService::new(pool.clone()).with_listener(Arc::new(push.clone()));
        let sites = sites::web::SitesWeb::new(
            sites::SitesService::new(pool.clone(), listings.clone()),
            &web_config.public_url,
        );
        Self {
            auth_web: auth::web::AuthWeb::new(auth, web_config),
            listings,
            sites,
            push,
            jobs,
            telegram_secret,
            whatsapp,
            pool,
        }
    }
}

const LOGO_SVG: &str = include_str!("../../common/static/logo.svg");
const LOGO_PNG: &[u8] = include_bytes!("../../common/static/logo.png");
const HTMX: &str = include_str!("../../common/static/htmx.min.js");

fn asset(content_type: &'static str, body: impl IntoResponse) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        body,
    )
}

async fn css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            // Pages link a versioned URL, so each version can be cached for good.
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        common::ui::CSS,
    )
}

async fn htmx() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            // The version is in the URL, so it never changes under that name.
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        HTMX,
    )
}

/// The full HTTP app.
pub fn router(m: &Modules) -> Router {
    let protected = listings::web::router(m.listings.clone())
        .merge(auth::web::account_router(m.auth_web.clone()))
        .merge(sites::web::settings_router(m.sites.clone()))
        .layer(middleware::from_fn_with_state(
            m.auth_web.clone(),
            auth::web::require_user,
        ));

    // JSON API for the iOS and Android apps (bearer tokens, no cookies).
    let api = listings::api::router(m.listings.clone())
        .merge(push::api::router(m.push.clone()))
        .merge(auth::api::account_router())
        .layer(middleware::from_fn_with_state(
            m.auth_web.clone(),
            auth::api::require_token,
        ));

    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/static/app.css", get(css))
        .route("/static/htmx-2.0.4.js", get(htmx))
        .route(
            "/static/logo.svg",
            get(|| async { asset("image/svg+xml", LOGO_SVG) }),
        )
        .route(
            "/favicon.ico",
            get(|| async { asset("image/svg+xml", LOGO_SVG) }),
        )
        .route(
            "/static/logo.png",
            get(|| async { asset("image/png", LOGO_PNG) }),
        )
        .merge(auth::web::public_router(m.auth_web.clone()))
        .merge(sites::web::public_router(m.sites.clone()))
        .merge(protected)
        .merge(auth::api::public_router(m.auth_web.clone()))
        .merge(api);

    if let Some(secret) = &m.telegram_secret {
        app = app.merge(telegram::webhook::router(m.jobs.clone(), secret.clone()));
    }
    if let Some(hook) = &m.whatsapp {
        app = app.merge(whatsapp::webhook::router(
            m.jobs.clone(),
            m.pool.clone(),
            hook.clone(),
        ));
    }
    // Agents' sites at <slug>.<domain> are answered before any CRM route.
    app.layer(middleware::from_fn_with_state(
        m.sites.clone(),
        sites::web::subdomain,
    ))
    .layer(tower_http::trace::TraceLayer::new_for_http())
    // Sentry: one hub per request (so breadcrumbs don't mix between users)
    // and the request's method, route and safe headers on each error.
    .layer(sentry::integrations::tower::SentryHttpLayer::new())
    .layer(sentry::integrations::tower::NewSentryLayer::<
        axum::extract::Request,
    >::new_from_top())
}
