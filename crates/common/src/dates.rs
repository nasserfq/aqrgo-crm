//! Dates in Saudi format: weekday, Umm al-Qura Hijri date, and Gregorian date.

use chrono::{Datelike, FixedOffset, NaiveDate, Utc};
use icu_calendar::{cal::Hijri, Date};

use crate::digits::to_ascii_digits;

const WEEKDAYS: [&str; 7] = [
    "الاثنين",
    "الثلاثاء",
    "الأربعاء",
    "الخميس",
    "الجمعة",
    "السبت",
    "الأحد",
];
const HIJRI_MONTHS: [&str; 12] = [
    "محرم",
    "صفر",
    "ربيع الأول",
    "ربيع الآخر",
    "جمادى الأولى",
    "جمادى الآخرة",
    "رجب",
    "شعبان",
    "رمضان",
    "شوال",
    "ذو القعدة",
    "ذو الحجة",
];

/// Today's date in Riyadh (UTC+3, no daylight saving).
pub fn today_riyadh() -> NaiveDate {
    let riyadh = FixedOffset::east_opt(3 * 3600).expect("valid offset");
    Utc::now().with_timezone(&riyadh).date_naive()
}

pub fn weekday_ar(d: NaiveDate) -> &'static str {
    WEEKDAYS[d.weekday().num_days_from_monday() as usize]
}

/// Hijri (Umm al-Qura) date as "22 جمادى الأولى 1448هـ".
pub fn hijri_ar(d: NaiveDate) -> String {
    match Date::try_new_iso(d.year(), d.month() as u8, d.day() as u8) {
        Ok(iso) => {
            let h = iso.to_calendar(Hijri::new_umm_al_qura());
            let month = HIJRI_MONTHS[(h.month().ordinal as usize).saturating_sub(1).min(11)];
            format!(
                "{} {} {}هـ",
                h.day_of_month().0,
                month,
                h.year().extended_year()
            )
        }
        Err(_) => String::new(),
    }
}

/// Gregorian as "13-11-2026".
pub fn gregorian(d: NaiveDate) -> String {
    d.format("%d-%m-%Y").to_string()
}

/// Full label: "الجمعة 22 جمادى الأولى 1448هـ / 13-11-2026" (plain text, for CSV).
pub fn full_ar(d: NaiveDate) -> String {
    format!("{} {} / {}", weekday_ar(d), hijri_ar(d), gregorian(d))
}

/// Same label, with the Gregorian date wrapped in Unicode left-to-right
/// isolates so it isn't shown reversed inside Arabic text (web, Telegram).
pub fn full_ar_display(d: NaiveDate) -> String {
    format!(
        "{} {} / \u{2066}{}\u{2069}",
        weekday_ar(d),
        hijri_ar(d),
        gregorian(d)
    )
}

/// Parses a date typed by a person: "اليوم", "أمس", 2026-11-13, 13/11/2026, 13-11-2026.
/// Arabic-Indic digits are accepted. Gregorian only.
pub fn parse_user_date(input: &str, today: NaiveDate) -> Option<NaiveDate> {
    let s = to_ascii_digits(input.trim());
    match s.as_str() {
        "اليوم" | "today" => return Some(today),
        "أمس" | "امس" | "yesterday" => return today.pred_opt(),
        _ => {}
    }
    for fmt in ["%Y-%m-%d", "%d/%m/%Y", "%d-%m-%Y", "%Y/%m/%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(&s, fmt) {
            return Some(d);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn known_umm_al_qura_dates() {
        // 1 Ramadan 1447 = 18 Feb 2026 (Umm al-Qura)
        assert_eq!(hijri_ar(d(2026, 2, 18)), "1 رمضان 1447هـ");
        // 1 Muharram 1447 = 26 June 2025
        assert_eq!(hijri_ar(d(2025, 6, 26)), "1 محرم 1447هـ");
    }

    #[test]
    fn full_label() {
        let s = full_ar(d(2026, 11, 13));
        assert!(s.starts_with("الجمعة "), "{s}");
        assert!(s.ends_with(" / 13-11-2026"), "{s}");
    }

    #[test]
    fn parses_user_input() {
        let today = d(2026, 11, 13);
        assert_eq!(parse_user_date("اليوم", today), Some(today));
        assert_eq!(parse_user_date("أمس", today), Some(d(2026, 11, 12)));
        assert_eq!(parse_user_date("١٠/١١/٢٠٢٦", today), Some(d(2026, 11, 10)));
        assert_eq!(parse_user_date("2026-11-01", today), Some(d(2026, 11, 1)));
        assert_eq!(parse_user_date("بكرة", today), None);
    }
}
