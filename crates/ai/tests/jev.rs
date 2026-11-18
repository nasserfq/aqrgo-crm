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
        area: Vec::new(),
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
    let a = parse_answers(&res, "أرض للبيع").unwrap();
    assert_eq!(a, j(0.93, 0.81, Status::Reserved));
}

#[test]
fn missing_answer_is_an_error_not_a_no() {
    let res = json!({"answers": {"is_lead": {"type": "noul", "noul": 0.9}}});
    assert!(parse_answers(&res, "أرض للبيع").is_err());
}

// The lead from the first WhatsApp test: Bedrock put the district in المساحة.
const MALQA: &str = "أرض للبيع في حي الملقا ٦٠٠ متر بسعر ٢ مليون";

struct AreaBedrock;

#[async_trait::async_trait]
impl LeadExtractor for AreaBedrock {
    async fn extract(&self, _text: &str, _today: NaiveDate) -> Result<Vec<ListingInput>, String> {
        Ok(vec![ListingInput {
            offer_type: "أرض للبيع".into(),
            area: "الملقا 600 متر".into(),
            location: "حي الملقا".into(),
            ..Default::default()
        }])
    }
}

async fn area_run(area: Vec<(&str, f64)>) -> String {
    let mut judgments = j(0.95, 0.1, Status::Available);
    judgments.area = area.into_iter().map(|(c, p)| (c.to_string(), p)).collect();
    let x = JudgedExtractor::new(Arc::new(AreaBedrock), Arc::new(FakeJudge(Ok(judgments))));
    x.extract(MALQA, today()).await.unwrap().remove(0).area
}

#[tokio::test]
async fn the_size_jev_picks_from_the_message_becomes_the_area() {
    assert_eq!(area_run(vec![("٦٠٠ متر", 0.93)]).await, "600 متر");
}

#[tokio::test]
async fn when_jev_is_unsure_the_area_is_cleaned_in_code() {
    assert_eq!(area_run(vec![("٦٠٠ متر", 0.4)]).await, "600 متر");
    assert_eq!(area_run(vec![]).await, "600 متر");
}

#[test]
fn request_asks_about_each_size_in_the_message() {
    let body = request_body("jev-1.14.0", "مساحة 625م شمالية شارع 20م");
    assert_eq!(body["state"]["candidate_0"], "625م");
    assert_eq!(body["state"]["candidate_1"], "20م");
    assert_eq!(body["questions"]["is_area_0"]["type"], "noul");
    assert_eq!(body["questions"]["is_area_1"]["type"], "noul");
    assert!(body["questions"].get("is_area_2").is_none());
    // No size in the message: no size questions, no candidates in the state.
    let body = request_body("jev-1.14.0", "مطلوب فيلا");
    assert!(body["questions"].get("is_area_0").is_none());
    assert!(body["state"].get("candidate_0").is_none());
}

#[test]
fn parses_size_answers_in_candidate_order() {
    let res = json!({"answers": {
        "is_lead": {"noul": 0.9}, "mentions_status": {"noul": 0.1},
        "status": {"choice": "available"},
        "is_area_0": {"noul": 0.92}, "is_area_1": {"noul": 0.05}
    }});
    let a = parse_answers(&res, "مساحة 625م شمالية شارع 20م").unwrap();
    assert_eq!(
        a.area,
        vec![("625م".to_string(), 0.92), ("20م".to_string(), 0.05)]
    );
}

// Bedrock copying "أرض … العارض" as "أرد … العارد".
const SLIPPED: &str = "أرض للبيع في حي العارض مخطط 2456";

struct MiscopyingBedrock;

#[async_trait::async_trait]
impl LeadExtractor for MiscopyingBedrock {
    async fn extract(&self, _text: &str, _today: NaiveDate) -> Result<Vec<ListingInput>, String> {
        Ok(vec![ListingInput {
            offer_type: "أرد للبيع".into(),
            location: "حي العارد".into(),
            plan_no: "2456".into(),
            ..Default::default()
        }])
    }
}

struct SlipJudge(Result<Vec<f64>, String>);

#[async_trait::async_trait]
impl Judge for SlipJudge {
    async fn judge(&self, _text: &str) -> Result<Judgments, String> {
        Ok(j(0.95, 0.0, Status::Available))
    }

    async fn confirm_slips(
        &self,
        _text: &str,
        _s: &[ai::faithful::Suspect],
    ) -> Result<Vec<f64>, String> {
        self.0.clone()
    }
}

async fn slip_run(answers: Result<Vec<f64>, String>) -> ListingInput {
    let x = JudgedExtractor::new(Arc::new(MiscopyingBedrock), Arc::new(SlipJudge(answers)));
    x.extract(SLIPPED, today()).await.unwrap().remove(0)
}

#[tokio::test]
async fn words_jev_confirms_as_slips_get_the_agents_spelling_back() {
    let row = slip_run(Ok(vec![0.9, 0.8])).await;
    assert_eq!(row.offer_type, "أرض للبيع");
    assert_eq!(row.location, "حي العارض");
}

#[tokio::test]
async fn words_jev_says_are_intended_are_kept() {
    let row = slip_run(Ok(vec![0.9, 0.2])).await;
    assert_eq!(row.offer_type, "أرض للبيع");
    assert_eq!(row.location, "حي العارد");
}

#[tokio::test]
async fn without_jev_every_slip_is_repaired() {
    let row = slip_run(Err("timeout".into())).await;
    assert_eq!(row.offer_type, "أرض للبيع");
    assert_eq!(row.location, "حي العارض");
}

#[tokio::test]
async fn the_guard_without_jev_repairs_slips_too() {
    let x = ai::faithful::SpellingGuard::new(Arc::new(MiscopyingBedrock));
    let row = x.extract(SLIPPED, today()).await.unwrap().remove(0);
    assert_eq!(row.offer_type, "أرض للبيع");
    assert_eq!(row.location, "حي العارض");
}

#[test]
fn slip_request_asks_one_question_per_suspect() {
    let suspects = ai::faithful::find_suspects(
        SLIPPED,
        &[ListingInput {
            offer_type: "أرد للبيع".into(),
            location: "حي العارد".into(),
            ..Default::default()
        }],
    );
    let body = ai::jev::slips_request_body("jev-1.14.0", SLIPPED, &suspects);
    assert_eq!(body["state"]["message"], SLIPPED);
    assert_eq!(body["state"]["written_1"], "العارد");
    assert_eq!(body["state"]["original_1"], "العارض");
    assert_eq!(body["questions"]["slip_0"]["type"], "noul");
    assert_eq!(body["questions"].as_object().unwrap().len(), 2);
}

#[test]
fn parses_slip_answers_and_refuses_missing_ones() {
    let res = json!({"answers": {"slip_0": {"type": "noul", "noul": 0.9}, "slip_1": {"type": "noul", "noul": 0.1}}});
    assert_eq!(ai::jev::parse_slips(&res, 2).unwrap(), vec![0.9, 0.1]);
    assert!(ai::jev::parse_slips(&res, 3).is_err());
}
