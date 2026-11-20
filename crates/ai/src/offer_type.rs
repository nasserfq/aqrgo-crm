//! Keeps نوع العرض to the property the agent wrote about.
//!
//! Bedrock once saved a listing for «مبنى شقق فندقية مخدومة» as «أرض للبيع»,
//! a word that is nowhere in the message. Two pieces of code help:
//! - [`foreign_kinds`] finds property words in the type that the message
//!   never uses (a sign the type was made up).
//! - [`from_message`] reads the property type in the agent's own words, used
//!   in place of a type that is wrong.
//!
//! Whether a type is wrong is Jev's call when Jev is available (see
//! [`crate::jev`]); without Jev, [`foreign_kinds`] decides.

use crate::faithful::normalize;

/// Property words, normalised ([`normalize`]: أ→ا، ة→ه، ى→ي).
const KINDS: &[&str] = &[
    "ارض",
    "اراضي",
    "فيلا",
    "فله",
    "فلل",
    "شقه",
    "شقق",
    "عماره",
    "عماير",
    "مبني",
    "مباني",
    "دبلكس",
    "دوبلكس",
    "محل",
    "محلات",
    "معرض",
    "مستودع",
    "مستودعات",
    "استراحه",
    "مزرعه",
    "شاليه",
    "بيت",
    "قصر",
    "برج",
    "فندق",
    "مكتب",
    "مكاتب",
    "مجمع",
    "ورشه",
    "دور",
    "بنايه",
];
/// Deal words, in the order they are looked for.
const DEALS: &[&str] = &["للبيع", "للإيجار", "للايجار", "للتقبيل", "للاستثمار"];
/// Words that end the property phrase: «فلة دورين بحي النرجس» → «فلة دورين».
const STOPS: &[&str] = &[
    "في",
    "فى",
    "حي",
    "بحي",
    "مساحه",
    "المساحه",
    "بمساحه",
    "على",
    "علي",
    "مع",
    "من",
    "شمال",
    "جنوب",
    "شرق",
    "غرب",
    "مطلوب",
    "السعر",
    "بسعر",
];
const MAX_WORDS: usize = 5;

/// The property type as the agent wrote it, with the deal: «مبنى شقق فندقية
/// مخدومة للبيع». `None` when the message names no property.
pub fn from_message(message: &str) -> Option<String> {
    for line in message.lines() {
        let tokens: Vec<&str> = line
            .split_whitespace()
            .map(|t| t.trim_matches(|c: char| "*_~".contains(c)))
            .filter(|t| !t.is_empty())
            .collect();
        let Some(start) = tokens.iter().position(|t| is_kind(clean(t))) else {
            continue;
        };
        let mut words: Vec<&str> = Vec::new();
        for t in &tokens[start..] {
            let w = clean(t);
            let n = normalize(w);
            if w.is_empty()
                || words.len() == MAX_WORDS
                || STOPS.contains(&n.as_str())
                || is_deal(w)
                || w.chars()
                    .any(|c| c.is_ascii_digit() || ('٠'..='٩').contains(&c))
            {
                break;
            }
            words.push(w);
            if w != *t && t.ends_with(|c: char| "،,:|".contains(c)) {
                break; // «فلة، …»: the punctuation ends the phrase.
            }
        }
        let phrase = words.join(" ");
        let wanted = tokens[..start]
            .iter()
            .any(|t| normalize(clean(t)) == "مطلوب");
        return Some(match (wanted, deal(message)) {
            (true, _) => format!("مطلوب {phrase}"),
            (false, Some(d)) => format!("{phrase} {d}"),
            (false, None) => phrase,
        });
    }
    None
}

/// Property words in `offer_type` that `message` never uses, as written in
/// `offer_type`. «الأرض» in the message counts for «أرض».
pub fn foreign_kinds<'a>(offer_type: &'a str, message: &str) -> Vec<&'a str> {
    let used: Vec<String> = words(message).map(stem).collect();
    words(offer_type)
        .filter(|w| {
            let s = stem(w);
            KINDS.contains(&s.as_str()) && !used.contains(&s)
        })
        .collect()
}

fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_alphanumeric() || ('\u{064B}'..='\u{065F}').contains(&c)))
        .filter(|w| !w.is_empty())
}

/// Normalised, without a leading «ال» (or «بال», «وال», «فال», «لل») when
/// what is left is a property word.
fn stem(w: &str) -> String {
    let n = normalize(w);
    for prefix in ["وال", "بال", "فال", "ال", "لل"] {
        if let Some(rest) = n.strip_prefix(prefix) {
            if KINDS.contains(&rest) {
                return rest.to_string();
            }
        }
    }
    n
}

fn is_kind(w: &str) -> bool {
    !w.is_empty() && KINDS.contains(&stem(w).as_str())
}

fn is_deal(w: &str) -> bool {
    DEALS.iter().any(|d| normalize(d) == normalize(w))
}

/// The first deal word in the message, as the message writes it.
fn deal(message: &str) -> Option<&str> {
    let all: Vec<&str> = words(message).collect();
    DEALS
        .iter()
        .find_map(|d| all.iter().find(|w| normalize(w) == normalize(d)).copied())
}

fn clean(t: &str) -> &str {
    t.trim_matches(|c: char| !(c.is_alphanumeric() || ('\u{064B}'..='\u{065F}').contains(&c)))
}

/// Below this, Jev's "the type matches the message" means it does not.
pub const MATCHES_AT_OR_ABOVE: f64 = 0.5;

/// Decides نوع العرض for a row read from `message`. `jev_matches` is Jev's
/// probability that `current` names the message's property, or `None`
/// without Jev. Returns the type to keep.
pub fn decide(message: &str, current: &str, jev_matches: Option<f64>) -> String {
    let wrong = match jev_matches {
        _ if current.trim().is_empty() => true,
        Some(p) => p < MATCHES_AT_OR_ABOVE,
        None => !foreign_kinds(current, message).is_empty(),
    };
    match from_message(message) {
        Some(own) if wrong => {
            tracing::info!(written = %current, replaced_by = %own, jev = ?jev_matches, "offer type replaced");
            own
        }
        _ => current.to_string(),
    }
}
