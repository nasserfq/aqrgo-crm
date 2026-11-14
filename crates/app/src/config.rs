//! Configuration from environment variables. See `.env.example`.

use anyhow::{bail, Context};

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub public_url: String,
    pub allowed_emails: Vec<String>,
    pub google_client_id: Option<String>,
    pub google_client_secret: Option<String>,
    pub mail_from: Option<String>,
    pub telegram_bot_token: Option<String>,
    pub telegram_webhook_secret: Option<String>,
    pub bedrock_model_id: Option<String>,
    /// SES can live in another region than Bedrock (e.g. us-east-2 for email).
    pub ses_region: Option<String>,
    /// TypeSafe Jev: typed judgments around Bedrock extraction (optional).
    pub typesafe_api_key: Option<String>,
    pub typesafe_model: Option<String>,
    /// Sentry error reporting (off without SENTRY_DSN).
    pub sentry: crate::telemetry::SentryConfig,
    /// Use AWS (SES/Bedrock). Off in local development without credentials.
    pub aws_enabled: bool,
}

fn opt(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let public_url = opt("PUBLIC_URL").unwrap_or_else(|| "http://localhost:8080".into());
        let public_url = public_url.trim_end_matches('/').to_string();
        let telegram_bot_token = opt("TELEGRAM_BOT_TOKEN");
        let telegram_webhook_secret = opt("TELEGRAM_WEBHOOK_SECRET");
        if telegram_bot_token.is_some() {
            match &telegram_webhook_secret {
                Some(s) if s.len() >= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') => {}
                _ => bail!("TELEGRAM_WEBHOOK_SECRET must be at least 16 characters of A-Z, a-z, 0-9, _ or -"),
            }
        }
        Ok(Self {
            database_url: opt("DATABASE_URL").context("DATABASE_URL is required")?,
            port: opt("PORT")
                .map(|p| p.parse())
                .transpose()
                .context("PORT must be a number")?
                .unwrap_or(8080),
            public_url,
            allowed_emails: opt("ALLOWED_EMAILS")
                .map(|s| {
                    s.split(',')
                        .map(|e| e.trim().to_lowercase())
                        .filter(|e| !e.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            google_client_id: opt("GOOGLE_CLIENT_ID"),
            google_client_secret: opt("GOOGLE_CLIENT_SECRET"),
            mail_from: opt("MAIL_FROM"),
            telegram_bot_token,
            telegram_webhook_secret,
            bedrock_model_id: opt("BEDROCK_MODEL_ID"),
            ses_region: opt("SES_REGION"),
            typesafe_api_key: opt("TYPESAFE_API_KEY"),
            typesafe_model: opt("TYPESAFE_MODEL"),
            sentry: crate::telemetry::SentryConfig::from_env(opt)?,
            aws_enabled: opt("AWS_REGION").is_some(),
        })
    }
}
