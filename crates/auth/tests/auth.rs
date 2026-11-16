use std::sync::{Arc, Mutex};

use auth::{AuthConfig, AuthError, AuthService, Channel, CodeMailer};

#[derive(Default, Clone)]
struct Inbox(Arc<Mutex<Vec<(String, String)>>>);

#[async_trait::async_trait]
impl CodeMailer for Inbox {
    async fn send_login_code(&self, email: &str, code: &str) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .push((email.to_string(), code.to_string()));
        Ok(())
    }
}

impl Inbox {
    fn last_code(&self) -> String {
        self.0
            .lock()
            .unwrap()
            .last()
            .expect("an email was sent")
            .1
            .clone()
    }
    fn count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

async fn setup(config: AuthConfig) -> (AuthService, Inbox) {
    let pool = testkit::fresh_pool().await;
    common::db::migrate_module(&pool, auth::SCHEMA, &auth::MIGRATOR)
        .await
        .unwrap();
    let inbox = Inbox::default();
    (
        AuthService::new(pool, Arc::new(inbox.clone()), config),
        inbox,
    )
}

#[tokio::test]
async fn emailed_code_signs_in_once_and_creates_the_account() {
    let (auth, inbox) = setup(AuthConfig::default()).await;
    auth.request_code(" Nasser@Example.com ").await.unwrap();

    let code = inbox.last_code();
    assert_eq!(code.len(), 6);
    assert!(code.chars().all(|c| c.is_ascii_digit()));

    let session = auth.verify_code("nasser@example.com", &code).await.unwrap();
    let user = auth
        .user_for_session(&session)
        .await
        .unwrap()
        .expect("signed in");
    assert_eq!(user.email, "nasser@example.com");

    // The same code cannot be used twice.
    assert!(matches!(
        auth.verify_code("nasser@example.com", &code).await,
        Err(AuthError::InvalidCode)
    ));
}

#[tokio::test]
async fn code_typed_with_arabic_digits_works() {
    let (auth, inbox) = setup(AuthConfig::default()).await;
    auth.request_code("a@example.com").await.unwrap();
    let arabic = common::digits::to_arabic_indic(&inbox.last_code());
    assert!(auth.verify_code("a@example.com", &arabic).await.is_ok());
}

#[tokio::test]
async fn wrong_codes_lock_the_code_after_five_tries() {
    let (auth, inbox) = setup(AuthConfig::default()).await;
    auth.request_code("a@example.com").await.unwrap();
    let real = inbox.last_code();
    let wrong = if real == "000000" { "111111" } else { "000000" };
    for _ in 0..5 {
        assert!(matches!(
            auth.verify_code("a@example.com", wrong).await,
            Err(AuthError::InvalidCode)
        ));
    }
    assert!(matches!(
        auth.verify_code("a@example.com", &real).await,
        Err(AuthError::TooManyAttempts)
    ));
}

#[tokio::test]
async fn a_new_code_replaces_the_old_one() {
    let (auth, inbox) = setup(AuthConfig::default()).await;
    auth.request_code("a@example.com").await.unwrap();
    let first = inbox.last_code();
    auth.request_code("a@example.com").await.unwrap();
    let second = inbox.last_code();
    if first != second {
        assert!(auth.verify_code("a@example.com", &first).await.is_err());
    }
    assert!(auth.verify_code("a@example.com", &second).await.is_ok());
}

#[tokio::test]
async fn code_requests_are_rate_limited() {
    let (auth, inbox) = setup(AuthConfig::default()).await;
    for _ in 0..5 {
        auth.request_code("a@example.com").await.unwrap();
    }
    assert!(matches!(
        auth.request_code("a@example.com").await,
        Err(AuthError::TooManyRequests)
    ));
    assert_eq!(inbox.count(), 5);
}

#[tokio::test]
async fn rejects_malformed_email() {
    let (auth, inbox) = setup(AuthConfig::default()).await;
    assert!(matches!(
        auth.request_code("not-an-email").await,
        Err(AuthError::InvalidEmail)
    ));
    assert_eq!(inbox.count(), 0);
}

#[tokio::test]
async fn allow_list_blocks_other_emails_without_revealing_it() {
    let config = AuthConfig {
        allowed_emails: vec!["boss@aqrgo.com".into()],
        ..Default::default()
    };
    let (auth, inbox) = setup(config).await;
    // Looks like success to the stranger, but no email is sent and no account is made.
    auth.request_code("stranger@example.com").await.unwrap();
    assert_eq!(inbox.count(), 0);
    assert!(matches!(
        auth.sign_in_verified_email("stranger@example.com", "x")
            .await,
        Err(AuthError::NotAllowed)
    ));
    auth.request_code("BOSS@aqrgo.com").await.unwrap();
    assert_eq!(inbox.count(), 1);
}

#[tokio::test]
async fn google_verified_email_signs_into_the_same_account() {
    let (auth, inbox) = setup(AuthConfig::default()).await;
    auth.request_code("a@example.com").await.unwrap();
    let s1 = auth
        .verify_code("a@example.com", &inbox.last_code())
        .await
        .unwrap();
    let s2 = auth
        .sign_in_verified_email("A@example.com", "Nasser")
        .await
        .unwrap();
    let u1 = auth.user_for_session(&s1).await.unwrap().unwrap();
    let u2 = auth.user_for_session(&s2).await.unwrap().unwrap();
    assert_eq!(u1.id, u2.id);
    assert_eq!(u2.name, "Nasser");
}

#[tokio::test]
async fn sign_out_ends_the_session() {
    let (auth, _) = setup(AuthConfig::default()).await;
    let s = auth
        .sign_in_verified_email("a@example.com", "A")
        .await
        .unwrap();
    auth.sign_out(&s).await.unwrap();
    assert!(auth.user_for_session(&s).await.unwrap().is_none());
    assert!(auth.user_for_session("garbage").await.unwrap().is_none());
}

#[tokio::test]
async fn one_telegram_account_per_user_via_link_token() {
    let (auth, _) = setup(AuthConfig::default()).await;
    let s = auth
        .sign_in_verified_email("a@example.com", "A")
        .await
        .unwrap();
    let user = auth.user_for_session(&s).await.unwrap().unwrap();

    let token = auth.create_link(user.id).await.unwrap();
    let linked = auth
        .redeem_link(Channel::Telegram, &token, 777)
        .await
        .unwrap()
        .expect("linked");
    assert_eq!(linked.id, user.id);
    assert_eq!(
        auth.user_for(Channel::Telegram, 777)
            .await
            .unwrap()
            .unwrap()
            .id,
        user.id
    );
    // Tokens are single use.
    assert!(auth
        .redeem_link(Channel::Telegram, &token, 888)
        .await
        .unwrap()
        .is_none());

    // Linking a new Telegram account replaces the old one.
    let token2 = auth.create_link(user.id).await.unwrap();
    auth.redeem_link(Channel::Telegram, &token2, 999)
        .await
        .unwrap()
        .unwrap();
    assert!(auth
        .user_for(Channel::Telegram, 777)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        auth.user_for(Channel::Telegram, 999)
            .await
            .unwrap()
            .unwrap()
            .id,
        user.id
    );
}

#[tokio::test]
async fn whatsapp_links_separately_from_telegram() {
    let (auth, _) = setup(AuthConfig::default()).await;
    let s = auth
        .sign_in_verified_email("a@example.com", "A")
        .await
        .unwrap();
    let user = auth.user_for_session(&s).await.unwrap().unwrap();
    let phone = 966_500_000_001;

    assert!(!auth.linked(Channel::WhatsApp, user.id).await.unwrap());
    let token = auth.create_link(user.id).await.unwrap();
    auth.redeem_link(Channel::WhatsApp, &token, phone)
        .await
        .unwrap()
        .expect("linked");
    assert!(auth.linked(Channel::WhatsApp, user.id).await.unwrap());
    assert!(!auth.linked(Channel::Telegram, user.id).await.unwrap());
    assert_eq!(
        auth.user_for(Channel::WhatsApp, phone)
            .await
            .unwrap()
            .unwrap()
            .id,
        user.id
    );
    assert!(
        auth.user_for(Channel::Telegram, phone)
            .await
            .unwrap()
            .is_none(),
        "a phone number is not a Telegram id"
    );

    auth.unlink(Channel::WhatsApp, user.id).await.unwrap();
    assert!(auth
        .user_for(Channel::WhatsApp, phone)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_burst_of_parallel_guesses_still_gets_only_five_tries() {
    let (auth, inbox) = setup(AuthConfig::default()).await;
    auth.request_code("a@example.com").await.unwrap();
    let wrong = if inbox.last_code() == "000000" {
        "111111"
    } else {
        "000000"
    };
    let mut handles = Vec::new();
    for _ in 0..20 {
        let auth = auth.clone();
        handles.push(tokio::spawn(async move {
            auth.verify_code("a@example.com", wrong).await
        }));
    }
    let mut checked = 0;
    for h in handles {
        if matches!(h.await.unwrap(), Err(AuthError::InvalidCode)) {
            checked += 1;
        }
    }
    assert!(checked <= 5, "{checked} guesses were checked");
}

#[tokio::test]
async fn removing_an_email_from_the_allow_list_ends_its_sessions() {
    let pool = testkit::fresh_pool().await;
    common::db::migrate_module(&pool, auth::SCHEMA, &auth::MIGRATOR)
        .await
        .unwrap();
    let open = AuthService::new(
        pool.clone(),
        Arc::new(Inbox::default()),
        AuthConfig::default(),
    );
    let session = open
        .sign_in_verified_email("old@example.com", "x")
        .await
        .unwrap();

    let restricted = AuthService::new(
        pool,
        Arc::new(Inbox::default()),
        AuthConfig {
            allowed_emails: vec!["boss@aqrgo.com".into()],
            ..Default::default()
        },
    );
    assert!(restricted
        .user_for_session(&session)
        .await
        .unwrap()
        .is_none());
}

#[test]
fn login_code_email_is_branded_arabic_html_with_a_text_fallback() {
    let email = auth::login_code_email("nasser@example.com", "694953", "https://aqrgo.com");
    assert_eq!(email.to, "nasser@example.com");
    assert!(email.subject.contains("694953"));
    assert!(
        email.text.contains("694953"),
        "plain-text fallback keeps the code"
    );
    let html = email.html.expect("html body");
    assert!(html.contains(r#"dir="rtl""#));
    assert!(html.contains("694953"));
    assert!(
        html.contains("https://aqrgo.com/static/logo.png"),
        "logo is an absolute URL"
    );
    assert!(!html.contains("—"), "no em-dashes");
}

#[tokio::test]
async fn a_session_in_use_stays_signed_in_until_sign_out() {
    let pool = testkit::fresh_pool().await;
    common::db::migrate_module(&pool, auth::SCHEMA, &auth::MIGRATOR)
        .await
        .unwrap();
    let auth = AuthService::new(
        pool.clone(),
        Arc::new(Inbox::default()),
        AuthConfig::default(),
    );
    let session = auth
        .sign_in_verified_email("nasser@example.com", "Nasser")
        .await
        .unwrap();

    // Pretend the session was made long ago: one day left.
    sqlx::query("UPDATE auth.sessions SET expires_at = now() + interval '1 day'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(auth.user_for_session(&session).await.unwrap().is_some());

    // Using it pushed the expiry a full SESSION_DAYS out again.
    let left: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at - now())::float8 / 86400 FROM auth.sessions",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        left > f64::from(auth::SESSION_DAYS) - 0.1,
        "expiry slid forward, {left} days left"
    );

    // A session left unused past its expiry is gone.
    sqlx::query("UPDATE auth.sessions SET expires_at = now() - interval '1 minute'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(auth.user_for_session(&session).await.unwrap().is_none());
}

#[tokio::test]
async fn the_app_review_login_uses_a_fixed_code_and_sends_no_email() {
    assert!(
        auth::ReviewLogin::new("review@aqrgo.com", "12345").is_none(),
        "short codes are refused"
    );
    let (auth, inbox) = setup(AuthConfig {
        review_login: auth::ReviewLogin::new("Review@aqrgo.com", "1234567890"),
        ..Default::default()
    })
    .await;

    auth.request_code("review@aqrgo.com").await.unwrap();
    assert_eq!(inbox.count(), 0, "no email for the review login");
    assert!(matches!(
        auth.verify_code("review@aqrgo.com", "0000000000").await,
        Err(AuthError::InvalidCode)
    ));
    let session = auth
        .verify_code("review@aqrgo.com", "١٢٣٤٥٦٧٨٩٠")
        .await
        .unwrap();
    let user = auth.user_for_session(&session).await.unwrap().unwrap();
    assert_eq!(user.email, "review@aqrgo.com");

    // Everyone else still gets an emailed code.
    auth.request_code("nasser@example.com").await.unwrap();
    assert_eq!(inbox.count(), 1);
}

mod app_handoff {
    use auth::handoff::AppHandoff;
    use sha2::{Digest, Sha256};

    fn challenge(verifier: &str) -> String {
        hex::encode(Sha256::digest(verifier.as_bytes()))
    }

    async fn pool() -> sqlx::PgPool {
        let pool = testkit::fresh_pool().await;
        common::db::migrate_module(&pool, auth::SCHEMA, &auth::MIGRATOR)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn a_code_survives_a_restart_and_works_once() {
        let pool = pool().await;
        let code = AppHandoff::new(pool.clone())
            .issue("sess".into(), challenge("secret-verifier"))
            .await
            .unwrap();
        // A new server process (after a deploy) still knows the code.
        let after_restart = AppHandoff::new(pool);
        assert_eq!(
            after_restart
                .redeem(&code, "secret-verifier")
                .await
                .unwrap()
                .as_deref(),
            Some("sess")
        );
        assert_eq!(
            after_restart
                .redeem(&code, "secret-verifier")
                .await
                .unwrap(),
            None,
            "one use only"
        );
    }

    #[tokio::test]
    async fn a_wrong_verifier_burns_the_code() {
        let h = AppHandoff::new(pool().await);
        let code = h
            .issue("sess".into(), challenge("secret-verifier"))
            .await
            .unwrap();
        assert_eq!(h.redeem(&code, "guess").await.unwrap(), None);
        assert_eq!(h.redeem(&code, "secret-verifier").await.unwrap(), None);
        assert_eq!(h.redeem("nope", "secret-verifier").await.unwrap(), None);
    }
}
