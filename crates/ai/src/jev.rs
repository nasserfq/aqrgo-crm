//! Jev (TypeSafe System One model): typed judgments about an incoming lead.
//!
//! Jev answers small questions about the message; [`JudgedExtractor`] holds
//! the decision logic in code. All question wording lives in this file.

use std::{sync::Arc, time::Duration};

use chrono::NaiveDate;
use listings::{ListingInput, Status};
use serde_json::{json, Value};

use crate::LeadExtractor;

/// Pinned so thresholds stay meaningful when the `jev-latest` alias moves.
pub const DEFAULT_MODEL: &str = "jev-1.14.0";
const API_URL: &str = "https://api.typesafe.ai/v1/systemone";

/// Below this, the message is treated as "not a property lead".
const NOT_A_LEAD_BELOW: f64 = 0.3;
/// Above this, the message is treated as stating the property's status.
const STATUS_STATED_ABOVE: f64 = 0.7;

/// Jev's answers about one message.
#[derive(Clone, Debug, PartialEq)]
pub struct Judgments {
    /// Probability the message offers or seeks a specific property.
    pub is_lead: f64,
    /// Probability the message says whether the property is reserved/sold.
    pub mentions_status: f64,
    /// Most likely status, only meaningful when `mentions_status` is high.
    pub status: Status,
}

#[async_trait::async_trait]
pub trait Judge: Send + Sync {
    async fn judge(&self, text: &str) -> Result<Judgments, String>;
}

/// The request sent to `POST /v1/systemone`.
pub fn request_body(model: &str, text: &str) -> Value {
    json!({
        "model": model,
        "state": {
            "message": text,
            "context": "A message sent to the Telegram bot of a Saudi real estate office by a broker or agent. \
                        Usually Gulf Arabic, often copied from WhatsApp."
        },
        "questions": {
            "is_lead": {
                "type": "noul",
                "instructions": "Does `message` describe a specific property being offered or sought?",
                "criteria": {
                    "true": "The message gives details of at least one property for sale, for rent, or wanted, \
                             such as its type, neighbourhood, plan or plot number, area, or price.",
                    "false": "The message is a greeting, a question about how the bot works, small talk, \
                              or general market news with no specific property."
                }
            },
            "mentions_status": {
                "type": "noul",
                "instructions": "Does `message` say whether the property has been reserved or sold?",
                "criteria": {
                    "true": "The message states the property's availability, for example that it is reserved \
                             (محجوز) or already sold (مباع، تم البيع), or that it is still available.",
                    "false": "The message presents the property without saying anything about whether it is \
                              reserved, sold, or still available."
                }
            },
            "status": {
                "type": "choice",
                "instructions": "What is the current status of the property in `message`?",
                "criteria": {
                    "available": "The property is still on the market and can be bought or rented.",
                    "reserved": "Someone has reserved the property or paid a deposit, but the deal is not complete (محجوز).",
                    "sold": "The property has already been sold or rented out (مباع، تم البيع)."
                }
            }
        }
    })
}

fn noul(res: &Value, key: &str) -> Result<f64, String> {
    res.pointer(&format!("/answers/{key}/noul"))
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("Jev response has no answer for {key}"))
}

/// Reads Jev's answers. A missing answer is an error, never a "no".
pub fn parse_answers(res: &Value) -> Result<Judgments, String> {
    let status = res
        .pointer("/answers/status/choice")
        .and_then(Value::as_str)
        .and_then(Status::parse)
        .ok_or("Jev response has no answer for status")?;
    Ok(Judgments {
        is_lead: noul(res, "is_lead")?,
        mentions_status: noul(res, "mentions_status")?,
        status,
    })
}

/// Calls the TypeSafe HTTP API.
pub struct JevClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

impl JevClient {
    pub fn new(api_key: String, model: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("http client");
        Self {
            http,
            api_key,
            model,
        }
    }
}

#[async_trait::async_trait]
impl Judge for JevClient {
    async fn judge(&self, text: &str) -> Result<Judgments, String> {
        let res = self
            .http
            .post(API_URL)
            .bearer_auth(&self.api_key)
            .json(&request_body(&self.model, text))
            .send()
            .await
            .map_err(|e| format!("Jev request failed: {e}"))?;
        let status = res.status();
        let body: Value = res
            .json()
            .await
            .map_err(|e| format!("Jev response unreadable ({status}): {e}"))?;
        if !status.is_success() {
            return Err(format!("Jev returned {status}: {}", body["detail"]));
        }
        parse_answers(&body)
    }
}

/// Bedrock fills in the columns; Jev's judgments decide whether to bother
/// and fill in الحالة. If Jev fails, Bedrock runs on its own.
pub struct JudgedExtractor {
    inner: Arc<dyn LeadExtractor>,
    judge: Arc<dyn Judge>,
}

impl JudgedExtractor {
    pub fn new(inner: Arc<dyn LeadExtractor>, judge: Arc<dyn Judge>) -> Self {
        Self { inner, judge }
    }
}

#[async_trait::async_trait]
impl LeadExtractor for JudgedExtractor {
    async fn extract(&self, text: &str, today: NaiveDate) -> Result<Vec<ListingInput>, String> {
        let judgments = match self.judge.judge(text).await {
            Ok(j) => Some(j),
            Err(e) => {
                tracing::warn!(error = %e, "Jev unavailable, using Bedrock alone");
                None
            }
        };
        if let Some(j) = &judgments {
            tracing::info!(
                is_lead = j.is_lead,
                mentions_status = j.mentions_status,
                status = j.status.as_str(),
                "Jev judgments"
            );
            if j.is_lead < NOT_A_LEAD_BELOW {
                return Ok(Vec::new());
            }
        }
        let mut rows = self.inner.extract(text, today).await?;
        // A stated status only applies when the message holds one property.
        if let (Some(j), [row]) = (&judgments, rows.as_mut_slice()) {
            if j.mentions_status > STATUS_STATED_ABOVE {
                row.status = j.status;
            }
        }
        Ok(rows)
    }
}
