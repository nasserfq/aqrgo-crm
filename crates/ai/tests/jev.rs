//! Jev (TypeSafe) judgments around Bedrock extraction: Jev answers typed
//! questions, code decides what to do with them.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use ai::jev::{parse_answers, request_body, Judge, JudgedExtractor, Judgments};
use ai::LeadExtractor;
use chrono::NaiveDate;
use listings::{ListingInput, Status};
use serde_json::json;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()
}

struct FakeBedrock {
    rows: usize,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl LeadExtractor for FakeBedrock {
    async fn extract(&self, _text: &str, _today: NaiveDate) -> Result<Vec<ListingInput>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok((0..self.rows)
            .map(|i| ListingInput {
                offer_type: "أرض للبيع".into(),
                plot_no: format!("{}", i + 1),
                ..Default::default()
            })
            .collect())
    }
}

struct FakeJudge(Result<Judgments, String>);

#[async_trait::async_trait]
impl Judge for FakeJudge {
    async fn judge(&self, _text: &str) -> Result<Judgments, String> {
        self.0.clone()
    }
}

fn judged(rows: usize, j: Result<Judgments, String>) -> (JudgedExtractor, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let inner = FakeBedrock {
        rows,
        calls: calls.clone(),
    };
    (
        JudgedExtractor::new(Arc::new(inner), Arc::new(FakeJudge(j))),
        calls,
    )
}

fn j(is_lead: f64, mentions_status: f64, status: Status) -> Judgments {
    Judgments {
        is_lead,
        mentions_status,
        status,
    }
}

#[tokio::test]
async fn not_a_lead_skips_bedrock_and_returns_nothing() {
    let (x, calls) = judged(1, Ok(j(0.1, 0.0, Status::Available)));
    assert!(x.extract("السلام عليكم", today()).await.unwrap().is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unsure_lead_still_goes_to_bedrock() {
    let (x, calls) = judged(1, Ok(j(0.5, 0.0, Status::Available)));
    assert_eq!(x.extract("أرض", today()).await.unwrap().len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stated_status_is_applied_to_a_single_row() {
    let (x, _) = judged(1, Ok(j(0.95, 0.9, Status::Sold)));
    let rows = x.extract("أرض في الملقا تم البيع", today()).await.unwrap();
    assert_eq!(rows[0].status, Status::Sold);
}

#[tokio::test]
async fn unclear_status_keeps_the_default() {
    let (x, _) = judged(1, Ok(j(0.95, 0.5, Status::Reserved)));
    let rows = x.extract("أرض في الملقا", today()).await.unwrap();
    assert_eq!(rows[0].status, Status::Available);
}

#[tokio::test]
async fn status_is_not_guessed_across_several_rows() {
    let (x, _) = judged(2, Ok(j(0.95, 0.9, Status::Sold)));
    let rows = x.extract("قطعتين", today()).await.unwrap();
    assert!(rows.iter().all(|r| r.status == Status::Available));
}

#[tokio::test]
async fn jev_failure_falls_back_to_bedrock_alone() {
    let (x, calls) = judged(1, Err("timeout".into()));
    assert_eq!(x.extract("أرض", today()).await.unwrap().len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn request_asks_the_three_questions_about_the_message() {
    let body = request_body("jev-1.14.0", "أرض للبيع");
    assert_eq!(body["model"], "jev-1.14.0");
    assert_eq!(body["state"]["message"], "أرض للبيع");
    assert_eq!(body["questions"]["is_lead"]["type"], "noul");
    assert_eq!(body["questions"]["mentions_status"]["type"], "noul");
    let status = &body["questions"]["status"];
    assert_eq!(status["type"], "choice");
    let opts: Vec<&String> = status["criteria"].as_object().unwrap().keys().collect();
    assert_eq!(opts.len(), 3);
}

#[test]
fn parses_answers() {
    let res = json!({
        "model": "jev-1.14.0",
        "answers": {
            "is_lead": {"type": "noul", "noul": 0.93},
            "mentions_status": {"type": "noul", "noul": 0.81},
            "status": {"type": "choice", "choice": "reserved", "confidence": 0.7,
                       "probabilities": {"available": 0.1, "reserved": 0.8, "sold": 0.1}}
        },
        "usage": {"input_tokens": 120, "output_tokens": 3}
    });
    let a = parse_answers(&res).unwrap();
    assert_eq!(a, j(0.93, 0.81, Status::Reserved));
}

#[test]
fn missing_answer_is_an_error_not_a_no() {
    let res = json!({"answers": {"is_lead": {"type": "noul", "noul": 0.9}}});
    assert!(parse_answers(&res).is_err());
}
