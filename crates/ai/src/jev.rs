//! Jev (TypeSafe System One model): typed judgments about an incoming lead.
//!
//! Jev answers small questions about the message; [`JudgedExtractor`] holds
//! the decision logic in code. All question wording lives in this file.

use std::{sync::Arc, time::Duration};

use chrono::NaiveDate;
use listings::{ListingInput, Status};
use serde_json::{json, Value};

use crate::{
    area,
    faithful::{self, Suspect},
    LeadExtractor,
};

/// Pinned so thresholds stay meaningful when the `jev-latest` alias moves.
pub const DEFAULT_MODEL: &str = "jev-1.14.0";
const API_URL: &str = "https://api.typesafe.ai/v1/systemone";

/// Below this, the message is treated as "not a property lead".
const NOT_A_LEAD_BELOW: f64 = 0.3;
/// Above this, the message is treated as stating the property's status.
const STATUS_STATED_ABOVE: f64 = 0.7;
/// At or above this, a suspect word is a copying slip and is repaired.
const SLIP_AT_OR_ABOVE: f64 = 0.5;

/// Jev's answers about one message.
#[derive(Clone, Debug, PartialEq)]
pub struct Judgments {
    /// Probability the message offers or seeks a specific property.
    pub is_lead: f64,
    /// Probability the message says whether the property is reserved/sold.
    pub mentions_status: f64,
    /// Most likely status, only meaningful when `mentions_status` is high.
    pub status: Status,
    /// Each size-like span of the message ([`area::candidates`]) with the
    /// probability that it is the property's size.
    pub area: Vec<(String, f64)>,
}

#[async_trait::async_trait]
pub trait Judge: Send + Sync {
    async fn judge(&self, text: &str) -> Result<Judgments, String>;

    /// For each suspect, the probability that Bedrock miscopied the
    /// message's word (see [`faithful`]). Same order as `suspects`.
    async fn confirm_slips(&self, text: &str, suspects: &[Suspect]) -> Result<Vec<f64>, String> {
        let _ = (text, suspects);
        Err("this judge does not check spelling".into())
    }
}

/// The request sent to `POST /v1/systemone`. The size questions are asked
/// up front, before Bedrock runs, so they cost no extra call.
pub fn request_body(model: &str, text: &str) -> Value {
    let mut body = json!({
        "model": model,
        "state": {
            "message": text,
            "context": "A message sent to the Telegram or WhatsApp bot of a Saudi real estate office by a broker or agent. \
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
    });
    for (i, candidate) in area::candidates(text).iter().enumerate() {
        body["state"][format!("candidate_{i}")] = json!(candidate);
        body["questions"][format!("is_area_{i}")] = json!({
            "type": "noul",
            "instructions": format!("Is `candidate_{i}` the size of the property in `message`?"),
            "criteria": {
                "true": format!("`candidate_{i}` is the land or floor area of the property being offered or sought, \
                                 for example ٦٠٠ متر, مساحة 625م or 20×30."),
                "false": format!("`candidate_{i}` measures something else, such as the width of a street next to it \
                                  (شارع 20م), the length of a frontage, or a price.")
            }
        });
    }
    body
}

/// The request asking whether each suspect word is a copying slip.
pub fn slips_request_body(model: &str, text: &str, suspects: &[Suspect]) -> Value {
    let mut state = serde_json::Map::new();
    state.insert("message".into(), json!(text));
    state.insert(
        "context".into(),
        json!("An AI copied details of a property listing out of `message` (usually Gulf Arabic). \
               Each `written_N` is a word the AI wrote; `original_N` is the closest word in `message`."),
    );
    let mut questions = serde_json::Map::new();
    for (i, s) in suspects.iter().enumerate() {
        state.insert(format!("written_{i}"), json!(s.written));
        state.insert(format!("original_{i}"), json!(s.original));
        questions.insert(
            format!("slip_{i}"),
            json!({
                "type": "noul",
                "instructions": format!("Is `written_{i}` the word `original_{i}` from `message`, copied with a wrong letter?"),
                "criteria": {
                    "true": format!("`written_{i}` is `original_{i}` with one letter changed, added or dropped by mistake, \
                                     for example أرد written for أرض, or العارد for the district العارض."),
                    "false": format!("`written_{i}` is a different, correctly spelled word that the AI chose on purpose, \
                                      not a miscopy of `original_{i}`.")
                }
            }),
        );
    }
    json!({ "model": model, "state": state, "questions": questions })
}

/// Reads one probability per suspect. A missing answer is an error.
pub fn parse_slips(res: &Value, count: usize) -> Result<Vec<f64>, String> {
    (0..count)
        .map(|i| noul(res, &format!("slip_{i}")))
        .collect()
}

fn noul(res: &Value, key: &str) -> Result<f64, String> {
    res.pointer(&format!("/answers/{key}/noul"))
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("Jev response has no answer for {key}"))
}

/// Reads Jev's answers to [`request_body`] for `text`. A missing answer is
/// an error, never a "no".
pub fn parse_answers(res: &Value, text: &str) -> Result<Judgments, String> {
    let status = res
        .pointer("/answers/status/choice")
        .and_then(Value::as_str)
        .and_then(Status::parse)
        .ok_or("Jev response has no answer for status")?;
    Ok(Judgments {
        is_lead: noul(res, "is_lead")?,
        mentions_status: noul(res, "mentions_status")?,
        status,
        area: area::candidates(text)
            .into_iter()
            .enumerate()
            .map(|(i, c)| Ok((c, noul(res, &format!("is_area_{i}"))?)))
            .collect::<Result<_, String>>()?,
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

impl JevClient {
    async fn ask(&self, body: &Value) -> Result<Value, String> {
        let res = self
            .http
            .post(API_URL)
            .bearer_auth(&self.api_key)
            .json(body)
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
        Ok(body)
    }
}

#[async_trait::async_trait]
impl Judge for JevClient {
    async fn judge(&self, text: &str) -> Result<Judgments, String> {
        parse_answers(&self.ask(&request_body(&self.model, text)).await?, text)
    }

    async fn confirm_slips(&self, text: &str, suspects: &[Suspect]) -> Result<Vec<f64>, String> {
        let body = slips_request_body(&self.model, text, suspects);
        parse_slips(&self.ask(&body).await?, suspects.len())
    }
}

/// Bedrock fills in the columns; Jev's judgments decide whether to bother,
/// fill in الحالة, and confirm which miscopied words get the message's own
/// word back. If Jev fails, Bedrock runs on its own and every suspect word
/// is repaired, as [`faithful::SpellingGuard`] does.
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
        for row in &mut rows {
            row.area = area::clean(&row.area, &row.location);
        }
        // A stated status, or a size picked from the message, only applies
        // when the message holds one property.
        if let (Some(j), [row]) = (&judgments, rows.as_mut_slice()) {
            if j.mentions_status > STATUS_STATED_ABOVE {
                row.status = j.status;
            }
            if let Some(size) = area::choose(&j.area) {
                row.area = size;
            }
        }
        let suspects = faithful::find_suspects(text, &rows);
        if suspects.is_empty() {
            return Ok(rows);
        }
        let fixes: Vec<&Suspect> = match self.judge.confirm_slips(text, &suspects).await {
            Ok(p) if p.len() == suspects.len() => {
                for (s, p) in suspects.iter().zip(&p) {
                    tracing::info!(field = s.field, written = %s.written, original = %s.original, slip = p, "Jev spelling check");
                }
                suspects
                    .iter()
                    .zip(p)
                    .filter(|(_, p)| *p >= SLIP_AT_OR_ABOVE)
                    .map(|(s, _)| s)
                    .collect()
            }
            other => {
                if let Err(e) = other {
                    tracing::warn!(error = %e, "Jev spelling check unavailable, repairing every suspect");
                }
                faithful::log(&suspects);
                suspects.iter().collect()
            }
        };
        faithful::repair(&mut rows, &fixes);
        Ok(rows)
    }
}
