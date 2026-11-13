//! AI module: turns a free-text lead (as agents write it on WhatsApp or
//! Telegram) into listing rows, using Amazon Bedrock (Amazon Nova by default).
//!
//! Public API: the [`LeadExtractor`] trait and [`BedrockExtractor`].

use std::collections::HashMap;

use aws_sdk_bedrockruntime::types::{
    ContentBlock, ConversationRole, InferenceConfiguration, Message, SpecificToolChoice,
    SystemContentBlock, Tool, ToolChoice, ToolConfiguration, ToolInputSchema, ToolSpecification,
};
use aws_smithy_types::{Document, Number};
use chrono::NaiveDate;
use listings::{ListingInput, FIELDS};
use serde_json::{json, Map, Value};

/// Reads a message and returns zero or more draft listings.
#[async_trait::async_trait]
pub trait LeadExtractor: Send + Sync {
    async fn extract(&self, text: &str, today: NaiveDate) -> Result<Vec<ListingInput>, String>;
}

const TOOL_NAME: &str = "record_listings";

fn system_prompt(today: NaiveDate) -> String {
    format!(
        "أنت مساعد لمكتب عقاري سعودي. تستلم رسالة من وسيط عقاري (غالباً بالعامية ومنسوخة من واتساب) \
وتستخرج منها العروض العقارية إلى جدول.\n\
القواعد:\n\
- كل قطعة أو عقار مستقل = عنصر مستقل في القائمة.\n\
- انسخ القيم كما وردت ولا تخترع أي معلومة. إذا لم تُذكر قيمة فاتركها نصاً فارغاً.\n\
- offer_type: نوع العرض بصيغة قصيرة مثل «أرض للبيع» أو «فيلا للإيجار» أو «طلب شراء».\n\
- plan_no رقم المخطط، plot_no رقم القطعة (قد تكون عدة أرقام)، area المساحة مع الوحدة إن ذُكرت.\n\
- frontages: الواجهات وعروض الشوارع، مثل «شمالية شارع 20م، شرقية ممر 6م».\n\
- location: الحي والمدينة أو أي وصف للموقع، ورابط الخريطة إن وجد.\n\
- price: السعر كما ورد (إجمالي أو سعر المتر، مع «على السوم» إن ذُكر).\n\
- seller بيانات البائع/المالك، buyer بيانات المشتري، broker بيانات الوسيط: الاسم ورقم الجوال.\n\
- rooms عدد الغرف، bathrooms عدد دورات المياه، floors عدد الأدوار (مثل «دورين ونص»)، units عدد الشقق أو المحلات أو الوحدات.\n\
- income: الدخل والإيجار والعقود القائمة (مثل «عقد واحد مع شركة 5 سنوات بقيمة 600 ألف دفعتين»).\n\
- details: كل معلومة أخرى لم تدخل في الخانات السابقة (المصعد، التشطيب، عمر العقار، المساحات، طريقة البيع، مباشر أو وسيط…)، كل معلومة في سطر قصير. لا تُسقط أي معلومة من الرسالة.\n\
- listed_on: تاريخ العرض بصيغة YYYY-MM-DD إن ذُكر صراحة، وإلا فاتركه فارغاً. تاريخ اليوم {today}.\n\
- إذا لم تكن الرسالة عرضاً عقارياً فأرجع قائمة فارغة.\n\
استدعِ الأداة {TOOL_NAME} مرة واحدة بالنتيجة.",
        today = today.format("%Y-%m-%d"),
    )
}

fn input_schema() -> Value {
    let mut props = Map::new();
    for (key, label) in FIELDS {
        props.insert(
            (*key).into(),
            json!({ "type": "string", "description": label }),
        );
    }
    props.insert(
        "listed_on".into(),
        json!({ "type": "string", "description": "YYYY-MM-DD أو فارغ" }),
    );
    let required: Vec<&str> = FIELDS
        .iter()
        .map(|(k, _)| *k)
        .chain(["listed_on"])
        .collect();
    json!({
        "type": "object",
        "properties": {
            "listings": {
                "type": "array",
                "items": { "type": "object", "properties": props, "required": required }
            }
        },
        "required": ["listings"]
    })
}

/// Converts the tool's JSON input into listing drafts. Unknown keys are
/// ignored; rows with nothing useful in them are dropped.
pub fn parse_tool_input(input: &Value, today: NaiveDate) -> Vec<ListingInput> {
    let Some(items) = input.get("listings").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let mut l = ListingInput::default();
            for (key, _) in FIELDS {
                if let Some(v) = item.get(*key).and_then(Value::as_str) {
                    l.set(key, v.trim().to_string());
                }
            }
            l.listed_on = item
                .get("listed_on")
                .and_then(Value::as_str)
                .and_then(|s| common::dates::parse_user_date(s, today))
                .or(Some(today));
            l.normalized().ok()
        })
        .collect()
}

pub mod jev;

pub struct BedrockExtractor {
    client: aws_sdk_bedrockruntime::Client,
    model_id: String,
}

impl BedrockExtractor {
    pub fn new(client: aws_sdk_bedrockruntime::Client, model_id: String) -> Self {
        Self { client, model_id }
    }
}

#[async_trait::async_trait]
impl LeadExtractor for BedrockExtractor {
    async fn extract(&self, text: &str, today: NaiveDate) -> Result<Vec<ListingInput>, String> {
        let spec = ToolSpecification::builder()
            .name(TOOL_NAME)
            .description("يسجل العروض العقارية المستخرجة من الرسالة")
            .input_schema(ToolInputSchema::Json(json_to_doc(&input_schema())))
            .build()
            .map_err(|e| e.to_string())?;
        let tools = ToolConfiguration::builder()
            .tools(Tool::ToolSpec(spec))
            .tool_choice(ToolChoice::Tool(
                SpecificToolChoice::builder()
                    .name(TOOL_NAME)
                    .build()
                    .map_err(|e| e.to_string())?,
            ))
            .build()
            .map_err(|e| e.to_string())?;
        let message = Message::builder()
            .role(ConversationRole::User)
            .content(ContentBlock::Text(text.to_string()))
            .build()
            .map_err(|e| e.to_string())?;

        let out = self
            .client
            .converse()
            .model_id(&self.model_id)
            .system(SystemContentBlock::Text(system_prompt(today)))
            .messages(message)
            .tool_config(tools)
            .inference_config(
                InferenceConfiguration::builder()
                    .max_tokens(4000)
                    .temperature(0.0)
                    .build(),
            )
            .send()
            .await
            .map_err(|e| {
                format!(
                    "bedrock: {}",
                    aws_sdk_bedrockruntime::error::DisplayErrorContext(e)
                )
            })?;

        let content = out
            .output()
            .and_then(|o| o.as_message().ok())
            .map(|m| m.content().to_vec())
            .unwrap_or_default();
        for block in content {
            if let ContentBlock::ToolUse(tu) = block {
                if tu.name() == TOOL_NAME {
                    return Ok(parse_tool_input(&doc_to_json(tu.input()), today));
                }
            }
        }
        Err("the model did not return listings".into())
    }
}

fn json_to_doc(v: &Value) -> Document {
    match v {
        Value::Null => Document::Null,
        Value::Bool(b) => Document::Bool(*b),
        Value::Number(n) => match (n.as_u64(), n.as_i64(), n.as_f64()) {
            (Some(u), _, _) => Document::Number(Number::PosInt(u)),
            (_, Some(i), _) => Document::Number(Number::NegInt(i)),
            (_, _, Some(f)) => Document::Number(Number::Float(f)),
            _ => Document::Null,
        },
        Value::String(s) => Document::String(s.clone()),
        Value::Array(a) => Document::Array(a.iter().map(json_to_doc).collect()),
        Value::Object(o) => Document::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), json_to_doc(v)))
                .collect::<HashMap<_, _>>(),
        ),
    }
}

fn doc_to_json(d: &Document) -> Value {
    match d {
        Document::Null => Value::Null,
        Document::Bool(b) => Value::Bool(*b),
        Document::Number(Number::PosInt(u)) => json!(u),
        Document::Number(Number::NegInt(i)) => json!(i),
        Document::Number(Number::Float(f)) => json!(f),
        Document::String(s) => Value::String(s.clone()),
        Document::Array(a) => Value::Array(a.iter().map(doc_to_json).collect()),
        Document::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), doc_to_json(v))).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 13).unwrap()
    }

    #[test]
    fn turns_tool_output_into_clean_rows() {
        let input = json!({ "listings": [
            { "offer_type": "أرض للبيع", "plan_no": "٢٤٥٦", "plot_no": "12 و 13", "area": "625م",
              "frontages": "شمالية 20م", "location": "حي الملقا - الرياض", "price": "على السوم",
              "buyer": "", "seller": "أبو فهد 0555", "broker": "", "listed_on": "" },
            { "offer_type": "", "plan_no": "", "plot_no": "", "area": "", "frontages": "",
              "location": "", "price": "", "buyer": "", "seller": "", "broker": "", "listed_on": "" },
            { "offer_type": "فيلا للإيجار", "location": "النرجس", "listed_on": "2026-11-01", "extra": "ignored" }
        ]});
        let rows = parse_tool_input(&input, today());
        assert_eq!(rows.len(), 2, "empty row is dropped");
        assert_eq!(rows[0].plan_no, "2456", "digits normalised");
        assert_eq!(rows[0].listed_on, Some(today()), "missing date = today");
        assert_eq!(rows[1].listed_on, NaiveDate::from_ymd_opt(2026, 11, 1));
        assert_eq!(rows[1].price, "");
    }

    #[test]
    fn keeps_building_details() {
        let input = json!({ "listings": [
            { "offer_type": "عمارة للبيع", "location": "حي الخليج", "rooms": "44", "bathrooms": "33",
              "floors": "دورين ونص", "units": "7 شقق", "income": "600 ألف سنوياً",
              "details": "مصعد راكب\nمباشر مع الوكيل" }
        ]});
        let rows = parse_tool_input(&input, today());
        assert_eq!(rows[0].rooms, "44");
        assert_eq!(rows[0].units, "7 شقق");
        assert!(rows[0].details.contains("مصعد"));
    }

    #[test]
    fn schema_asks_for_every_column() {
        let s = input_schema();
        let props = &s["properties"]["listings"]["items"]["properties"];
        for key in ["rooms", "bathrooms", "floors", "units", "income", "details"] {
            assert!(props.get(key).is_some(), "{key} in schema");
        }
    }

    #[test]
    fn non_listing_output_is_empty() {
        assert!(parse_tool_input(&json!({ "listings": [] }), today()).is_empty());
        assert!(parse_tool_input(&json!({ "oops": 1 }), today()).is_empty());
    }

    #[test]
    fn schema_round_trips_through_document() {
        let s = input_schema();
        assert_eq!(doc_to_json(&json_to_doc(&s)), s);
    }
}
