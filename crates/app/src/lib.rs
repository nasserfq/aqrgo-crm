//! Composition root: wires the modules together. No business logic here.

use std::sync::Arc;

use axum::{http::header, middleware, response::IntoResponse, routing::get, Router};
use sqlx::PgPool;

pub mod config;

pub use config::Config;

/// Runs every module's migrations (each in its own schema).
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    for (schema, m) in [
        (jobs::SCHEMA, &jobs::MIGRATOR),
        (auth::SCHEMA, &auth::MIGRATOR),
        (listings::SCHEMA, &listings::MIGRATOR),
        (telegram::SCHEMA, &telegram::MIGRATOR),
    ] {
        common::db::migrate_module(pool, schema, m).await?;
    }
    Ok(())
}

pub struct Modules {
    pub auth_web: auth::web::AuthWeb,
    pub listings: listings::ListingsService,
    pub jobs: jobs::Jobs,
    pub telegram_secret: Option<String>,
}

impl Modules {
    pub fn new(
        pool: PgPool,
        code_mailer: Arc<dyn auth::CodeMailer>,
        auth_config: auth::AuthConfig,
        web_config: auth::web::WebConfig,
        telegram_secret: Option<String>,
    ) -> Self {
        let auth = auth::AuthService::new(pool.clone(), code_mailer, auth_config);
        Self {
            auth_web: auth::web::AuthWeb::new(auth, web_config),
            listings: listings::ListingsService::new(pool.clone()),
            jobs: jobs::Jobs::new(pool),
            telegram_secret,
        }
    }
}

const APP_CSS: &str = include_str!("../../common/static/app.css");

async fn css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        APP_CSS,
    )
}

/// The full HTTP app.
pub fn router(m: &Modules) -> Router {
    let protected = listings::web::router(m.listings.clone())
        .merge(auth::web::account_router(m.auth_web.clone()))
        .layer(middleware::from_fn_with_state(
            m.auth_web.clone(),
            auth::web::require_user,
        ));

    let mut app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/static/app.css", get(css))
        .merge(auth::web::public_router(m.auth_web.clone()))
        .merge(protected);

    if let Some(secret) = &m.telegram_secret {
        app = app.merge(telegram::webhook::router(m.jobs.clone(), secret.clone()));
    }
    app.layer(tower_http::trace::TraceLayer::new_for_http())
}
