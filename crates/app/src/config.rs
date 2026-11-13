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
    /// WhatsApp Cloud API (Meta): all four are needed to turn WhatsApp on.
    pub whatsapp: Option<WhatsAppConfig>,
    pub bedrock_model_id: Option<String>,
    /// SES can live in another region than Bedrock (e.g. us-east-2 for email).
    pub ses_region: Option<String>,
    /// TypeSafe Jev: typed judgments around Bedrock extraction (optional).
    pub typesafe_api_key: Option<String>,
    pub typesafe_model: Option<String>,
    /// Use AWS (SES/Bedrock). Off in local development without credentials.
    pub aws_enabled: bool,
}

#[derive(Clone, Debug)]
pub struct WhatsAppConfig {
    /// A permanent System User access token with whatsapp_business_messaging.
    pub token: String,
    pub phone_number_id: String,
    pub app_secret: String,
    pub verify_token: String,
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
        let whatsapp = match (
            opt("WHATSAPP_TOKEN"),
            opt("WHATSAPP_PHONE_NUMBER_ID"),
            opt("WHATSAPP_APP_SECRET"),
            opt("WHATSAPP_VERIFY_TOKEN"),
        ) {
            (None, None, None, None) => None,
            (Some(token), Some(phone_number_id), Some(app_secret), Some(verify_token)) => {
                Some(WhatsAppConfig {
                    token,
                    phone_number_id,
                    app_secret,
                    verify_token,
                })
            }
            // Secrets are added one by one in GitHub; a half-done setup must not
            // take the site down, so WhatsApp just stays off until all four exist.
            _ => {
                tracing::error!("WhatsApp is off: it needs all of WHATSAPP_TOKEN, WHATSAPP_PHONE_NUMBER_ID, WHATSAPP_APP_SECRET and WHATSAPP_VERIFY_TOKEN");
                None
            }
        };
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
            whatsapp,
            bedrock_model_id: opt("BEDROCK_MODEL_ID"),
            ses_region: opt("SES_REGION"),
            typesafe_api_key: opt("TYPESAFE_API_KEY"),
            typesafe_model: opt("TYPESAFE_MODEL"),
            aws_enabled: opt("AWS_REGION").is_some(),
        })
    }
}
