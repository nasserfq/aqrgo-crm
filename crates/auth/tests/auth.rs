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
