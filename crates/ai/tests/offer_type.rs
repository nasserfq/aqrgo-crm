//! نوع العرض must name the property the agent wrote about. Regression: a
//! WhatsApp listing for «مبنى شقق فندقية مخدومة» was saved as «أرض للبيع»,
//! a word that is nowhere in the message (it was the prompt's example).

use std::sync::Arc;

use ai::jev::{offer_type_request_body, parse_offer_type, Judge, JudgedExtractor, Judgments};
use ai::{offer_type, LeadExtractor};
use chrono::NaiveDate;
use listings::{ListingInput, Status};
use serde_json::json;

const NAHDA: &str = include_str!("fixtures/nahda_hotel_building.txt");
const HOTEL_BUILDING: &str = "مبنى شقق فندقية مخدومة للبيع";

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 20).unwrap()
}

#[test]
fn reads_the_property_type_in_the_agents_words() {
    assert_eq!(
        offer_type::from_message(NAHDA).as_deref(),
        Some(HOTEL_BUILDING)
    );
    assert_eq!(
        offer_type::from_message("أرض للبيع في حي الملقا ٦٠٠ متر").as_deref(),
        Some("أرض للبيع")
    );
    assert_eq!(
        offer_type::from_message("للإيجار فلة دورين بحي النرجس").as_deref(),
        Some("فلة دورين للإيجار")
    );
    assert_eq!(
        offer_type::from_message("مطلوب شقة في شمال الرياض").as_deref(),
        Some("مطلوب شقة")
    );
    assert_eq!(offer_type::from_message("السلام عليكم"), None);
}

#[test]
fn flags_property_words_the_message_never_used() {
    assert_eq!(offer_type::foreign_kinds("أرض للبيع", NAHDA), vec!["أرض"]);
    assert!(offer_type::foreign_kinds("عمارة تجارية للبيع", NAHDA).is_empty());
    assert!(offer_type::foreign_kinds(HOTEL_BUILDING, NAHDA).is_empty());
    // «الأرض» in the message covers «أرض» in the type.
    assert!(offer_type::foreign_kinds("أرض للبيع", "بيع الأرض في الملقا").is_empty());
}

/// Bedrock's answer for the Nahda message as it was saved in production.
struct LandBedrock;

#[async_trait::async_trait]
impl LeadExtractor for LandBedrock {
    async fn extract(&self, _text: &str, _today: NaiveDate) -> Result<Vec<ListingInput>, String> {
        Ok(vec![ListingInput {
            offer_type: "أرض للبيع".into(),
            area: "900 م²".into(),
            location: "حي النهضة – الرياض".into(),
            price: "على السوم".into(),
            ..Default::default()
        }])
    }
}

/// Jev, answering whether Bedrock's type matches the message.
struct TypeJudge(Result<f64, String>);

#[async_trait::async_trait]
impl Judge for TypeJudge {
    async fn judge(&self, _text: &str) -> Result<Judgments, String> {
        Ok(Judgments {
            is_lead: 0.97,
            mentions_status: 0.1,
            status: Status::Available,
            area: Vec::new(),
        })
    }

    async fn check_offer_type(&self, _text: &str, _offer_type: &str) -> Result<f64, String> {
        self.0.clone()
    }
}

async fn run(answer: Result<f64, String>) -> String {
    let x = JudgedExtractor::new(Arc::new(LandBedrock), Arc::new(TypeJudge(answer)));
    x.extract(NAHDA, today())
        .await
        .unwrap()
        .remove(0)
        .offer_type
}

#[tokio::test]
async fn when_jev_says_the_type_is_wrong_the_agents_words_are_used() {
    assert_eq!(run(Ok(0.04)).await, HOTEL_BUILDING);
}

#[tokio::test]
async fn when_jev_says_the_type_matches_it_is_kept() {
    assert_eq!(run(Ok(0.9)).await, "أرض للبيع");
}

#[tokio::test]
async fn without_jev_a_property_word_not_in_the_message_is_replaced() {
    assert_eq!(run(Err("timeout".into())).await, HOTEL_BUILDING);
}

#[tokio::test]
async fn the_guard_without_jev_fixes_the_type_too() {
    let x = ai::faithful::SpellingGuard::new(Arc::new(LandBedrock));
    let row = x.extract(NAHDA, today()).await.unwrap().remove(0);
    assert_eq!(row.offer_type, HOTEL_BUILDING);
}

#[test]
fn jev_is_asked_to_compare_the_type_with_the_message() {
    let body = offer_type_request_body("jev-1.14.0", NAHDA, "أرض للبيع");
    assert_eq!(body["state"]["message"], NAHDA);
    assert_eq!(body["state"]["extracted_offer_type"], "أرض للبيع");
    assert_eq!(body["questions"]["offer_type_matches"]["type"], "noul");
    assert_eq!(body["questions"].as_object().unwrap().len(), 1);
}

#[test]
fn parses_the_type_answer_and_refuses_a_missing_one() {
    let res = json!({"answers": {"offer_type_matches": {"type": "noul", "noul": 0.03}}});
    assert_eq!(parse_offer_type(&res).unwrap(), 0.03);
    assert!(parse_offer_type(&json!({"answers": {}})).is_err());
}
