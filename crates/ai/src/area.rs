//! Keeps المساحة to the property's size.
//!
//! Bedrock sometimes copies nearby words into the area, as in «الملقا 600 متر»
//! for "أرض في حي الملقا ٦٠٠ متر". Two layers fix this:
//! - [`clean`] (code, always): drops words of the area that belong to the
//!   location.
//! - [`candidates`] (code) finds every size-like span in the message, Jev says
//!   which one is the property's size, and [`choose`] (code) decides whether
//!   to use it.

use crate::faithful::normalize;

/// Unit words that may follow a number in a size: «600 متر», «625 م2».
const UNITS: &[&str] = &["متر", "م", "م2", "م²", "مربع", "متر2", "متر²"];
/// Words that introduce a size: «مساحة 625».
const AREA_WORDS: &[&str] = &["مساحة", "المساحة", "مساحته", "مساحتها", "بمساحة"];
/// Jev must be at least this sure a candidate is the property's size.
pub const AREA_AT_OR_ABOVE: f64 = 0.7;

/// Size-like spans in the message, as written, without repeats: a number
/// with a unit («٦٠٠ متر», «625م»), dimensions («20×30»), or a number after
/// «مساحة». Street widths («شارع 20م») are candidates too; Jev tells them apart.
pub fn candidates(message: &str) -> Vec<String> {
    let tokens: Vec<&str> = message
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| ",.،؛:()".contains(c)))
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        let mut span = None;
        if is_number(t) || is_dimensions(t) {
            let mut end = i;
            while end + 1 < tokens.len() && UNITS.contains(&tokens[end + 1]) {
                end += 1;
            }
            let after_area_word = i > 0 && AREA_WORDS.contains(&tokens[i - 1]);
            if end > i || is_dimensions(t) || after_area_word {
                span = Some((i, end));
            }
        } else if has_unit_suffix(t) {
            span = Some((i, i));
        }
        match span {
            Some((start, end)) => {
                let s = tokens[start..=end].join(" ");
                if !out.contains(&s) {
                    out.push(s);
                }
                i = end + 1;
            }
            None => i += 1,
        }
    }
    out
}

/// The candidate Jev picked, when exactly one is clearly the property's
/// size. Digits are written as 0-9, like the other columns.
pub fn choose(scored: &[(String, f64)]) -> Option<String> {
    let mut sure = scored.iter().filter(|(_, p)| *p >= AREA_AT_OR_ABOVE);
    let first = sure.next()?;
    if sure.any(|(c, _)| ascii_digits(c) != ascii_digits(&first.0)) {
        return None; // Two different sizes (land and building?): keep Bedrock's.
    }
    Some(ascii_digits(&first.0))
}

/// Drops words of `area` that are part of `location` (and the word «حي»).
/// Leaves `area` alone if nothing would be left.
pub fn clean(area: &str, location: &str) -> String {
    let place: Vec<String> = location
        .split_whitespace()
        .map(|w| normalize(w.trim_matches(|c: char| ",.،؛:()".contains(c))))
        .collect();
    let kept: Vec<&str> = area
        .split_whitespace()
        .filter(|w| {
            let n = normalize(w);
            n != "حي" && !(has_letters(&n) && place.contains(&n))
        })
        .collect();
    if kept.is_empty() {
        area.to_string()
    } else {
        kept.join(" ")
    }
}

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('٠'..='٩').contains(&c)
}

fn is_number(t: &str) -> bool {
    !t.is_empty()
        && t.chars().next().is_some_and(is_digit)
        && t.chars()
            .all(|c| is_digit(c) || c == '.' || c == '٫' || c == ',')
}

fn is_dimensions(t: &str) -> bool {
    let parts: Vec<&str> = t.split(['×', 'x', 'X', '*']).collect();
    parts.len() == 2 && parts.iter().all(|p| is_number(p))
}

/// «625م», «600م2», «٦٠٠متر».
fn has_unit_suffix(t: &str) -> bool {
    let Some(split) = t.find(|c: char| !(is_digit(c) || c == '.' || c == '٫')) else {
        return false;
    };
    let (num, unit) = t.split_at(split);
    is_number(num) && UNITS.contains(&unit)
}

fn has_letters(s: &str) -> bool {
    s.chars().any(char::is_alphabetic)
}

fn ascii_digits(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '٠'..='٩' => char::from(b'0' + (c as u32 - '٠' as u32) as u8),
            '٫' => '.',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_sizes_written_in_different_ways() {
        assert_eq!(
            candidates("أرض للبيع في حي الملقا ٦٠٠ متر بسعر ٢ مليون"),
            vec!["٦٠٠ متر"]
        );
        assert_eq!(
            candidates("مخطط 2456 قطعة 12 مساحة 625م شمالية شارع 20م"),
            vec!["625م", "20م"]
        );
        assert_eq!(candidates("أرض 20×30 على شارع 15 م"), vec!["20×30", "15 م"]);
        assert_eq!(candidates("المساحة 900 والسعر 3500 للمتر"), vec!["900"]);
        assert!(candidates("مطلوب فيلا حدود ٢ مليون").is_empty());
    }

    #[test]
    fn uses_the_one_size_jev_is_sure_of() {
        let s = |c: &str, p| (c.to_string(), p);
        assert_eq!(choose(&[s("٦٠٠ متر", 0.95)]), Some("600 متر".to_string()));
        assert_eq!(
            choose(&[s("625م", 0.9), s("20م", 0.1)]),
            Some("625م".to_string())
        );
        // Unsure, or two different sizes: Jev does not decide.
        assert_eq!(choose(&[s("625م", 0.5)]), None);
        assert_eq!(choose(&[s("600 متر", 0.9), s("400 متر", 0.8)]), None);
        assert_eq!(choose(&[]), None);
    }

    #[test]
    fn location_words_are_taken_out_of_the_area() {
        assert_eq!(clean("الملقا 600 متر", "حي الملقا"), "600 متر");
        assert_eq!(clean("حي الملقا 600 متر", "حي الملقا، الرياض"), "600 متر");
        assert_eq!(clean("625م", "حي العارض"), "625م");
        assert_eq!(clean("الملقا", "الملقا"), "الملقا");
    }
}
