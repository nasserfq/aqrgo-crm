use askama::Template;
use std::sync::Arc;

use common::CurrentUser;
use rand::{distributions::Alphanumeric, Rng};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use subtle::ConstantTimeEq;

const CODE_TTL_MINUTES: i32 = 10;
const MAX_ATTEMPTS: i32 = 5;
const MAX_CODES_PER_HOUR: i64 = 5;
const SESSION_DAYS: i32 = 30;
const LINK_MINUTES: i32 = 15;

/// A chat app a user can link one account of, to send leads from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    Telegram,
    /// Keyed by the sender's phone number in international digits (WhatsApp's `wa_id`).
    WhatsApp,
}

impl Channel {
    /// Column in `auth.users` holding the linked id. Fixed strings only.
    fn column(self) -> &'static str {
        match self {
            Channel::Telegram => "telegram_id",
            Channel::WhatsApp => "whatsapp_id",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("البريد الإلكتروني غير صحيح")]
    InvalidEmail,
    #[error("الرمز غير صحيح أو منتهي")]
    InvalidCode,
    #[error("محاولات كثيرة، اطلب رمزاً جديداً")]
    TooManyAttempts,
    #[error("طلبات كثيرة، حاول بعد قليل")]
    TooManyRequests,
    #[error("هذا الحساب غير مسموح له بالدخول")]
    NotAllowed,
    #[error("تعذر إرسال البريد، حاول مرة أخرى")]
    Mail(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Delivers login codes. Production uses [`MailerCodeSender`] (a background job).
#[async_trait::async_trait]
pub trait CodeMailer: Send + Sync {
    async fn send_login_code(&self, email: &str, code: &str) -> Result<(), String>;
}

/// Queues the login-code email on the mailer's background job.
pub struct MailerCodeSender {
    pub mailer: mailer::Mailer,
    /// Public site address, used for the logo and link in the email.
    pub public_url: String,
}

#[async_trait::async_trait]
impl CodeMailer for MailerCodeSender {
    async fn send_login_code(&self, email: &str, code: &str) -> Result<(), String> {
        self.mailer
            .queue(&login_code_email(email, code, &self.public_url))
            .await
            .map_err(|e| e.to_string())
    }
}

#[derive(Template)]
#[template(path = "email_code.html")]
struct CodeEmailHtml<'a> {
    code: &'a str,
    minutes: i32,
    public_url: &'a str,
}

/// The branded sign-in email: Arabic RTL HTML plus a plain-text fallback.
pub fn login_code_email(to: &str, code: &str, public_url: &str) -> mailer::Email {
    let public_url = public_url.trim_end_matches('/');
    let text = format!(
        "رمز الدخول إلى AQRGO CRM هو: {code}\n\nالرمز صالح لمدة {CODE_TTL_MINUTES} دقائق. إذا لم تطلبه فتجاهل هذه الرسالة.\n\nYour AQRGO CRM sign-in code is {code}"
    );
    let html = CodeEmailHtml {
        code,
        minutes: CODE_TTL_MINUTES,
        public_url,
    }
    .render()
    .ok();
    mailer::Email {
        to: to.to_string(),
        subject: format!("رمز الدخول: {code}"),
        text,
        html,
    }
}

#[derive(Clone, Debug, Default)]
pub struct AuthConfig {
    /// Lowercase emails allowed to sign in. Empty means anyone may sign up.
    pub allowed_emails: Vec<String>,
}

impl AuthConfig {
    fn allows(&self, email: &str) -> bool {
        self.allowed_emails.is_empty()
            || self
                .allowed_emails
                .iter()
                .any(|e| e.eq_ignore_ascii_case(email))
    }
}

#[derive(Clone)]
pub struct AuthService {
    pool: PgPool,
    mailer: Arc<dyn CodeMailer>,
    config: AuthConfig,
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn random_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(40)
        .map(char::from)
        .collect()
}

fn normalize_email(raw: &str) -> Result<String, AuthError> {
    let e = raw.trim().to_lowercase();
    let valid = e.len() <= 254
        && !e.contains(char::is_whitespace)
        && matches!(e.split_once('@'), Some((local, domain)) if !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !domain.contains('@'));
    if valid {
        Ok(e)
    } else {
        Err(AuthError::InvalidEmail)
    }
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: i64,
    email: String,
    name: String,
}

impl From<UserRow> for CurrentUser {
    fn from(r: UserRow) -> Self {
        let name = if r.name.is_empty() {
            r.email.split('@').next().unwrap_or_default().to_string()
        } else {
            r.name
        };
        CurrentUser {
            id: r.id,
            email: r.email,
            name,
            is_admin: false,
        }
    }
}

impl AuthService {
    pub fn new(pool: PgPool, mailer: Arc<dyn CodeMailer>, config: AuthConfig) -> Self {
        Self {
            pool,
            mailer,
            config,
        }
    }

    /// Emails a fresh 6-digit code. For emails outside the allow-list this
    /// quietly does nothing, so the page doesn't reveal who has access.
    pub async fn request_code(&self, raw_email: &str) -> Result<(), AuthError> {
        let email = normalize_email(raw_email)?;
        if !self.config.allows(&email) {
            tracing::info!(%email, "login code requested for email outside allow-list");
            return Ok(());
        }
        let recent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM auth.login_codes WHERE email = $1 AND created_at > now() - interval '1 hour'",
        )
        .bind(&email)
        .fetch_one(&self.pool)
        .await?;
        if recent >= MAX_CODES_PER_HOUR {
            return Err(AuthError::TooManyRequests);
        }
        let code = format!("{:06}", rand::thread_rng().gen_range(0..1_000_000));
        let mut tx = self.pool.begin().await?;
        // Only the newest code is valid.
        sqlx::query(
            "UPDATE auth.login_codes SET used_at = now() WHERE email = $1 AND used_at IS NULL",
        )
        .bind(&email)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO auth.login_codes (email, code_hash, expires_at)
             VALUES ($1, $2, now() + make_interval(mins => $3))",
        )
        .bind(&email)
        .bind(sha256_hex(&format!("{email}:{code}")))
        .bind(CODE_TTL_MINUTES)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.mailer
            .send_login_code(&email, &code)
            .await
            .map_err(AuthError::Mail)
    }

    /// Checks the code and returns a new session token.
    pub async fn verify_code(&self, raw_email: &str, raw_code: &str) -> Result<String, AuthError> {
        let email = normalize_email(raw_email)?;
        let code: String = common::digits::to_ascii_digits(raw_code)
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect();
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT id, code_hash FROM auth.login_codes
             WHERE email = $1 AND used_at IS NULL AND expires_at > now()
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&email)
        .fetch_optional(&self.pool)
        .await?;
        let Some((id, hash)) = row else {
            return Err(AuthError::InvalidCode);
        };
        // Reserve one attempt atomically before checking, so a burst of
        // parallel guesses can't all slip under the limit.
        let reserved = sqlx::query(
            "UPDATE auth.login_codes SET attempts = attempts + 1 WHERE id = $1 AND attempts < $2",
        )
        .bind(id)
        .bind(MAX_ATTEMPTS)
        .execute(&self.pool)
        .await?;
        if reserved.rows_affected() == 0 {
            return Err(AuthError::TooManyAttempts);
        }
        let given = sha256_hex(&format!("{email}:{code}"));
        if !bool::from(given.as_bytes().ct_eq(hash.as_bytes())) {
            return Err(AuthError::InvalidCode);
        }
        let used = sqlx::query(
            "UPDATE auth.login_codes SET used_at = now() WHERE id = $1 AND used_at IS NULL",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if used.rows_affected() == 0 {
            return Err(AuthError::InvalidCode);
        }
        let user_id = self.upsert_user(&email, None).await?;
        self.new_session(user_id).await
    }

    /// For identity providers that already verified the email (Google).
    pub async fn sign_in_verified_email(
        &self,
        raw_email: &str,
        name: &str,
    ) -> Result<String, AuthError> {
        let email = normalize_email(raw_email)?;
        if !self.config.allows(&email) {
            return Err(AuthError::NotAllowed);
        }
        let user_id = self
            .upsert_user(&email, Some(name.trim()).filter(|n| !n.is_empty()))
            .await?;
        self.new_session(user_id).await
    }

    async fn upsert_user(&self, email: &str, name: Option<&str>) -> Result<i64, AuthError> {
        Ok(sqlx::query_scalar(
            "INSERT INTO auth.users (email, name, last_login_at) VALUES ($1, COALESCE($2, ''), now())
             ON CONFLICT (email) DO UPDATE SET last_login_at = now(),
                 name = CASE WHEN $2 IS NOT NULL THEN $2 ELSE auth.users.name END
             RETURNING id",
        )
        .bind(email)
        .bind(name)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn new_session(&self, user_id: i64) -> Result<String, AuthError> {
        let token = random_token();
        sqlx::query(
            "INSERT INTO auth.sessions (token_hash, user_id, expires_at)
             VALUES ($1, $2, now() + make_interval(days => $3))",
        )
        .bind(sha256_hex(&token))
        .bind(user_id)
        .bind(SESSION_DAYS)
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    pub async fn user_for_session(&self, token: &str) -> Result<Option<CurrentUser>, AuthError> {
        let row: Option<UserRow> = sqlx::query_as(
            "SELECT u.id, u.email, u.name FROM auth.sessions s JOIN auth.users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(sha256_hex(token))
        .fetch_optional(&self.pool)
        .await?;
        // An email removed from ALLOWED_EMAILS loses its existing sessions too.
        Ok(row.filter(|u| self.config.allows(&u.email)).map(Into::into))
    }

    pub async fn sign_out(&self, token: &str) -> Result<(), AuthError> {
        sqlx::query("DELETE FROM auth.sessions WHERE token_hash = $1")
            .bind(sha256_hex(token))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// One-time token that links a chat account to this user: the Telegram
    /// deep link `t.me/<bot>?start=<token>`, or a WhatsApp message `ربط <token>`.
    pub async fn create_link(&self, user_id: i64) -> Result<String, AuthError> {
        let token = random_token()[..32].to_string();
        sqlx::query(
            "INSERT INTO auth.telegram_link_tokens (token_hash, user_id, expires_at)
             VALUES ($1, $2, now() + make_interval(mins => $3))",
        )
        .bind(sha256_hex(&token))
        .bind(user_id)
        .bind(LINK_MINUTES)
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    /// Links the chat account `id` on `channel` to the token's user,
    /// replacing any earlier link on that channel.
    pub async fn redeem_link(
        &self,
        channel: Channel,
        token: &str,
        id: i64,
    ) -> Result<Option<CurrentUser>, AuthError> {
        let col = channel.column();
        let mut tx = self.pool.begin().await?;
        let user_id: Option<i64> = sqlx::query_scalar(
            "DELETE FROM auth.telegram_link_tokens WHERE token_hash = $1 AND expires_at > now() RETURNING user_id",
        )
        .bind(sha256_hex(token.trim()))
        .fetch_optional(&mut *tx)
        .await?;
        let Some(user_id) = user_id else {
            return Ok(None);
        };
        sqlx::query(&format!(
            "UPDATE auth.users SET {col} = NULL WHERE {col} = $1"
        ))
        .bind(id)
        .execute(&mut *tx)
        .await?;
        let row: UserRow = sqlx::query_as(&format!(
            "UPDATE auth.users SET {col} = $2 WHERE id = $1 RETURNING id, email, name"
        ))
        .bind(user_id)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some(row.into()))
    }

    /// The user whose `channel` account is `id`, if linked.
    pub async fn user_for(
        &self,
        channel: Channel,
        id: i64,
    ) -> Result<Option<CurrentUser>, AuthError> {
        let row: Option<UserRow> = sqlx::query_as(&format!(
            "SELECT id, email, name FROM auth.users WHERE {} = $1",
            channel.column()
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.filter(|u| self.config.allows(&u.email)).map(Into::into))
    }

    pub async fn linked(&self, channel: Channel, user_id: i64) -> Result<bool, AuthError> {
        Ok(sqlx::query_scalar(&format!(
            "SELECT {} IS NOT NULL FROM auth.users WHERE id = $1",
            channel.column()
        ))
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(false))
    }

    pub async fn unlink(&self, channel: Channel, user_id: i64) -> Result<(), AuthError> {
        sqlx::query(&format!(
            "UPDATE auth.users SET {} = NULL WHERE id = $1",
            channel.column()
        ))
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_normalization() {
        assert_eq!(normalize_email("  A@B.com ").unwrap(), "a@b.com");
        for bad in ["", "a", "a@b", "@b.com", "a@.com", "a b@c.com", "a@b.com."] {
            assert!(normalize_email(bad).is_err(), "{bad}");
        }
    }
}
