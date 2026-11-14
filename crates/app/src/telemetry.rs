//! Error reporting to Sentry. Everything here is off unless `SENTRY_DSN` is set.
//!
//! Errors only, no performance tracing. What reaches Sentry:
//! - panics (with a backtrace),
//! - `tracing::error!` events from any module, as Sentry events,
//! - `tracing::warn!` events, as breadcrumbs on the next error.
//!
//! Info logs are never sent: some carry emails or lead text. Request headers,
//! cookies and IP addresses are not sent either (`send_default_pii` is off), and
//! query strings are dropped (they can hold a Google sign-in code or a search
//! for a client's phone number).

use std::{borrow::Cow, str::FromStr, sync::Arc};

use anyhow::Context;
use sentry::integrations::tracing::{BreadcrumbLayer, ErrorLayer};
use tracing_subscriber::{filter::LevelFilter, Layer};

pub use sentry::ClientInitGuard;

use crate::Config;

/// Sentry settings read from the environment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SentryConfig {
    /// Project DSN. Not a password, but kept out of the repo like one.
    pub dsn: Option<String>,
    /// e.g. `production`. Defaults to `production`.
    pub environment: Option<String>,
    /// Version shown on each issue; the deploy sets it to the git commit.
    pub release: Option<String>,
}

impl SentryConfig {
    pub fn from_env(opt: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let dsn = opt("SENTRY_DSN");
        if let Some(d) = &dsn {
            sentry::types::Dsn::from_str(d).context("SENTRY_DSN is not a valid Sentry DSN")?;
        }
        Ok(Self {
            dsn,
            environment: opt("SENTRY_ENVIRONMENT"),
            release: opt("SENTRY_RELEASE"),
        })
    }

    /// Client options, or `None` when Sentry is off.
    pub fn client_options(&self) -> Option<sentry::ClientOptions> {
        let dsn = self.dsn.as_deref()?.parse().ok()?;
        let release = self
            .release
            .clone()
            .map(Cow::Owned)
            .or_else(|| sentry::release_name!());
        let mut opts = sentry::ClientOptions::new()
            .send_default_pii(false)
            .attach_stacktrace(true);
        opts.dsn = Some(dsn);
        opts.environment = Some(Cow::Owned(
            self.environment
                .clone()
                .unwrap_or_else(|| "production".into()),
        ));
        opts.release = release;
        opts.before_send = Some(Arc::new(|e| Some(scrub_event(e))));
        Some(opts)
    }
}

/// Removes query strings from the request attached to an event.
pub fn scrub_event(
    mut event: sentry::protocol::Event<'static>,
) -> sentry::protocol::Event<'static> {
    if let Some(r) = event.request.as_mut() {
        scrub_request(r);
    }
    event
}

fn scrub_request(r: &mut sentry::protocol::Request) {
    r.query_string = None;
    if let Some(url) = r.url.as_mut() {
        url.set_query(None);
        url.set_fragment(None);
    }
}

/// Starts Sentry if a DSN is configured. Keep the guard alive for the whole
/// program: dropping it flushes queued events.
pub fn init(cfg: &Config) -> Option<ClientInitGuard> {
    cfg.sentry.client_options().map(sentry::init)
}

/// The `tracing` layers that turn error logs into Sentry events and warnings
/// into breadcrumbs. They do nothing while Sentry is off.
pub fn tracing_layer<S>() -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    ErrorLayer::new()
        .with_filter(LevelFilter::ERROR)
        .and_then(BreadcrumbLayer::new().with_filter(LevelFilter::WARN))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    const DSN: &str = "https://public@o0.ingest.sentry.io/1";

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn off_without_a_dsn() {
        let cfg = SentryConfig::from_env(env(&[])).unwrap();
        assert!(cfg.client_options().is_none());
    }

    #[test]
    fn on_with_a_dsn_and_never_sends_personal_data() {
        let cfg = SentryConfig::from_env(env(&[("SENTRY_DSN", DSN), ("SENTRY_RELEASE", "abc123")]))
            .unwrap();
        let opts = cfg.client_options().expect("sentry on");
        assert!(!opts.send_default_pii);
        assert_eq!(opts.environment.as_deref(), Some("production"));
        assert_eq!(opts.release.as_deref(), Some("abc123"));
    }

    #[test]
    fn rejects_a_bad_dsn() {
        assert!(SentryConfig::from_env(env(&[("SENTRY_DSN", "not a dsn")])).is_err());
    }

    #[test]
    fn query_strings_are_dropped() {
        let event = sentry::protocol::Event {
            request: Some(sentry::protocol::Request {
                url: "https://aqrgo.com/auth/google/callback?code=secret&state=x"
                    .parse()
                    .ok(),
                query_string: Some("code=secret&state=x".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let r = scrub_event(event).request.unwrap();
        assert_eq!(r.query_string, None);
        assert_eq!(
            r.url.unwrap().as_str(),
            "https://aqrgo.com/auth/google/callback"
        );
    }

    #[test]
    fn errors_become_events_warnings_breadcrumbs_info_is_dropped() {
        let events = sentry::test::with_captured_events(|| {
            let subscriber = tracing_subscriber::registry().with(tracing_layer());
            tracing::subscriber::with_default(subscriber, || {
                tracing::info!(email = "someone@example.com", "login code requested");
                tracing::warn!("lead extraction failed");
                tracing::error!("listings error");
            });
        });
        assert_eq!(events.len(), 1, "only the error is an event");
        let event = &events[0];
        assert_eq!(event.message.as_deref(), Some("listings error"));
        let crumbs: Vec<_> = event
            .breadcrumbs
            .iter()
            .filter_map(|b| b.message.clone())
            .collect();
        assert_eq!(crumbs, vec!["lead extraction failed".to_string()]);
    }
}
