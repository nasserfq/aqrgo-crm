//! Mailer module: sends email through AWS SES from a background job.
//! Other modules call [`Mailer::queue`]; only this module talks to SES.

use aws_sdk_sesv2::types::{Body, Content, Destination, EmailContent, Message};
use serde::{Deserialize, Serialize};

pub const JOB_KIND: &str = "send_email";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub text: String,
    /// Optional HTML body; mail clients show it, falling back to `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
}

/// Queues emails. Cheap to clone.
#[derive(Clone)]
pub struct Mailer {
    jobs: jobs::Jobs,
}

impl Mailer {
    pub fn new(jobs: jobs::Jobs) -> Self {
        Self { jobs }
    }

    pub async fn queue(&self, email: &Email) -> Result<(), sqlx::Error> {
        self.jobs.enqueue(JOB_KIND, email).await.map(|_| ())
    }
}

/// Where emails go. `None` client = development: print to the log instead.
pub struct SesSender {
    pub client: Option<aws_sdk_sesv2::Client>,
    pub from: String,
}

/// Registers the email-sending handler on the job worker.
pub fn register(worker: jobs::Worker, sender: SesSender) -> jobs::Worker {
    if sender.client.is_none() {
        tracing::warn!("SES is not configured: emails will be printed to the log, not sent");
    }
    let sender = std::sync::Arc::new(sender);
    worker.register(JOB_KIND, move |payload| {
        let sender = sender.clone();
        async move {
            let email: Email = serde_json::from_value(payload).map_err(|e| format!("bad email payload: {e}"))?;
            let Some(client) = &sender.client else {
                tracing::info!(to = %email.to, subject = %email.subject, body = %email.text, "DEV EMAIL (not sent)");
                return Ok(());
            };
            let utf8 = |s: &str| Content::builder().data(s).charset("UTF-8").build().map_err(|e| e.to_string());
            let message = Message::builder()
                .subject(utf8(&email.subject)?)
                .body(
                    Body::builder()
                        .text(utf8(&email.text)?)
                        .set_html(email.html.as_deref().map(utf8).transpose()?)
                        .build(),
                )
                .build();
            client
                .send_email()
                .from_email_address(&sender.from)
                .destination(Destination::builder().to_addresses(&email.to).build())
                .content(EmailContent::builder().simple(message).build())
                .send()
                .await
                .map(|_| ())
                .map_err(|e| format!("SES send failed: {}", aws_sdk_sesv2::error::DisplayErrorContext(e)))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_queued_before_html_existed_still_load() {
        let old = serde_json::json!({ "to": "a@b.c", "subject": "s", "text": "t" });
        let email: Email = serde_json::from_value(old).unwrap();
        assert_eq!(email.html, None);
    }
}
