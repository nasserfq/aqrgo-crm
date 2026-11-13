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
