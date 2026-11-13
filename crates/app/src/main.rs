use std::{sync::Arc, time::Duration};

use anyhow::Context;
use app::{Config, Modules};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();

    let cfg = Config::from_env()?;
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&cfg.database_url)
        .await
        .context("connecting to Postgres")?;
    app::migrate(&pool).await.context("running migrations")?;

    // AWS (SES for email, Bedrock for AI) uses the standard AWS env vars.
    let aws = if cfg.aws_enabled {
        Some(aws_config::load_from_env().await)
    } else {
        None
    };
    if aws.is_none() {
        tracing::warn!("AWS_REGION not set: emails go to the log and AI lead reading is off");
    }

    let jobs = jobs::Jobs::new(pool.clone());
    let mailer = mailer::Mailer::new(jobs.clone());
    let code_mailer: Arc<dyn auth::CodeMailer> = Arc::new(auth::MailerCodeSender(mailer));
    let auth_config = auth::AuthConfig {
        allowed_emails: cfg.allowed_emails.clone(),
    };

    // Telegram: point the webhook at us and learn the bot's username.
    let tg_api = cfg.telegram_bot_token.clone().map(teloxide_core::Bot::new);
    let mut bot_username = None;
    if let (Some(api), Some(secret)) = (&tg_api, &cfg.telegram_webhook_secret) {
        match telegram::webhook::setup(api, &cfg.public_url, secret).await {
            Ok(name) => {
                tracing::info!(bot = %name, "telegram webhook set");
                bot_username = Some(name);
            }
            Err(e) => {
                tracing::error!(error = %e, "telegram webhook setup failed (is PUBLIC_URL reachable over https?)")
            }
        }
    }

    let google = match (&cfg.google_client_id, &cfg.google_client_secret) {
        (Some(id), Some(secret)) => Some(auth::web::GoogleConfig {
            client_id: id.clone(),
            client_secret: secret.clone(),
        }),
        _ => None,
    };
    let web_config = auth::web::WebConfig {
        public_url: cfg.public_url.clone(),
        google,
        telegram_bot: bot_username,
    };
    let modules = Modules::new(
        pool.clone(),
        jobs.clone(),
        code_mailer,
        auth_config.clone(),
        web_config,
        tg_api.as_ref().and(cfg.telegram_webhook_secret.clone()),
    );

    // Background worker: emails + Telegram updates.
    let ses = mailer::SesSender {
        client: aws
            .as_ref()
            .filter(|_| cfg.mail_from.is_some())
            .map(aws_sdk_sesv2::Client::new),
        from: cfg.mail_from.clone().unwrap_or_default(),
    };
    let mut worker = mailer::register(jobs.worker(), ses);
    if let Some(api) = tg_api.clone() {
        let extractor: Option<Arc<dyn ai::LeadExtractor>> = aws.as_ref().map(|a| {
            let model = cfg
                .bedrock_model_id
                .clone()
                .unwrap_or_else(|| "amazon.nova-pro-v1:0".into());
            tracing::info!(%model, "AI lead reading via Amazon Bedrock");
            Arc::new(ai::BedrockExtractor::new(
                aws_sdk_bedrockruntime::Client::new(a),
                model,
            )) as _
        });
        let auth_service = modules.auth_web.service.clone();
        let bot = telegram::Bot::new(
            pool.clone(),
            auth_service,
            modules.listings.clone(),
            extractor,
        );
        let cleanup_bot = bot.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = cleanup_bot.cleanup().await {
                    tracing::warn!(error = %e, "draft cleanup failed");
                }
                tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
            }
        });
        worker = telegram::webhook::register(worker, bot, telegram::webhook::TelegramOutbox(api));
    }
    // A few workers side by side, so one slow job doesn't block the rest.
    for _ in 0..3 {
        tokio::spawn(worker.clone().run());
    }

    let app = app::router(&modules);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", cfg.port)).await?;
    tracing::info!(port = cfg.port, url = %cfg.public_url, "AQRGO CRM listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Stops on Ctrl-C or SIGTERM (what `docker stop` sends).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutting down");
}
