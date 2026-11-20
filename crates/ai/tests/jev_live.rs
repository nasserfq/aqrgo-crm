//! Live probe against the TypeSafe API with sample Arabic messages.
//! Run by hand only: TYPESAFE_API_KEY=... cargo test -p ai --test jev_live -- --ignored --nocapture

use ai::jev::{request_body, JevClient, Judge, DEFAULT_MODEL};

const CASES: &[(&str, &str)] = &[
    ("lead", "أرض للبيع في حي الملقا مخطط 2456 قطعة 12 مساحة 625م شمالية شارع 20 السعر على السوم للتواصل 0550000000"),
    ("lead", "مطلوب فيلا للشراء في شمال الرياض حدود ٢ مليون"),
    ("lead", "عمارة للإيجار حي النرجس ٨ شقق الدخل ٤٠٠ ألف"),
    ("lead", "قطعتين تجاري على طريق الملك سلمان مخطط ٣٠١٢ رقم ٥ و٦ السعر ٣٥٠٠ للمتر"),
    ("lead sold", "تم البيع ✅ أرض حي الياسمين مخطط 1950 قطعة 88"),
    ("lead reserved", "الأرض اللي في العارض قطعة ٤٤ محجوزة الحين لا تعرضونها"),
    ("lead", "شقة للبيع الدور الثاني ٣ غرف حي الصحافة"),
    ("not lead", "السلام عليكم"),
    ("not lead", "كيف اضيف عرض جديد؟"),
    ("not lead", "الله يعطيك العافية يا ابو محمد"),
    ("not lead", "أسعار الأراضي في الرياض ارتفعت هالسنة حسب تقرير الهيئة"),
    ("not lead", "/help"),
];

#[tokio::test]
#[ignore]
async fn probe_sample_messages() {
    let key = std::env::var("TYPESAFE_API_KEY").expect("set TYPESAFE_API_KEY");
    let jev = JevClient::new(key, DEFAULT_MODEL.into());
    println!(
        "payload shape: {}",
        request_body(DEFAULT_MODEL, "<message>")
    );
    for (expected, text) in CASES {
        match jev.judge(text).await {
            Ok(j) => println!(
                "{expected:<14} is_lead={:.2} mentions_status={:.2} status={:<9} | {text}",
                j.is_lead,
                j.mentions_status,
                j.status.as_str()
            ),
            Err(e) => println!("{expected:<14} ERROR {e} | {text}"),
        }
    }
}

/// Jev's view of words Bedrock may have miscopied. The first two are real
/// slips (Nova wrote أرد and العارد); the last is a deliberate plural.
#[tokio::test]
#[ignore]
async fn probe_slips() {
    use ai::faithful::Suspect;
    let key = std::env::var("TYPESAFE_API_KEY").expect("set TYPESAFE_API_KEY");
    let jev = JevClient::new(key, DEFAULT_MODEL.into());
    let s = |field, written: &str, original: &str| Suspect {
        row: 0,
        field,
        written: written.into(),
        original: original.into(),
    };
    let text = "أرض للبيع في حي العارض شارع 20 والسعر على السوم";
    let suspects = [
        s("offer_type", "أرد", "أرض"),
        s("location", "العارد", "العارض"),
        s("frontages", "شوارع", "شارع"),
    ];
    match jev.confirm_slips(text, &suspects).await {
        Ok(p) => {
            for (s, p) in suspects.iter().zip(p) {
                println!("slip={p:.2} {} -> {}", s.written, s.original);
            }
        }
        Err(e) => println!("ERROR {e}"),
    }
}

/// Jev's view of each size-like span: the first message is the one where
/// Bedrock put the district in المساحة; the second has a street width too.
#[tokio::test]
#[ignore]
async fn probe_area() {
    let key = std::env::var("TYPESAFE_API_KEY").expect("set TYPESAFE_API_KEY");
    let jev = JevClient::new(key, DEFAULT_MODEL.into());
    for text in [
        "أرض للبيع في حي الملقا ٦٠٠ متر بسعر ٢ مليون",
        "أرض للبيع في حي الملقا مخطط 2456 قطعة 12 مساحة 625م شمالية شارع 20م",
        "فيلا للبيع حي النرجس الأرض 400 متر والبناء 550 متر شارع 15 م",
    ] {
        match jev.judge(text).await {
            Ok(j) => println!("{:?} | {text}", j.area),
            Err(e) => println!("ERROR {e} | {text}"),
        }
    }
}

/// Jev comparing نوع العرض with the message. The Nahda building was saved as
/// أرض للبيع (wrong); the other two types are right (one is a synonym).
#[tokio::test]
#[ignore]
async fn probe_offer_type() {
    let key = std::env::var("TYPESAFE_API_KEY").expect("set TYPESAFE_API_KEY");
    let jev = JevClient::new(key, DEFAULT_MODEL.into());
    let nahda = include_str!("fixtures/nahda_hotel_building.txt");
    for (text, offer_type) in [
        (nahda, "أرض للبيع"),
        (nahda, "مبنى شقق فندقية مخدومة للبيع"),
        ("للبيع فلة دورين بحي النرجس", "فيلا للبيع"),
    ] {
        match jev.check_offer_type(text, offer_type).await {
            Ok(p) => println!("matches={p:.2} {offer_type}"),
            Err(e) => println!("ERROR {e} | {offer_type}"),
        }
    }
}
