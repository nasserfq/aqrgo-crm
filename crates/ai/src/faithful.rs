//! Keeps extracted fields faithful to the agent's own words.
//!
//! Bedrock sometimes changes a letter while copying Arabic (أرض became أرد,
//! العارض became العارد). A word in a field that is not in the message, but is
//! one letter away from exactly one word of the message, is a [`Suspect`]:
//! most likely a copying slip. [`repair`] puts the message's own word back.
//! Who confirms a suspect (Jev, or nobody) is up to the caller.

use std::{collections::HashSet, sync::Arc};

use chrono::NaiveDate;
use listings::{ListingInput, FIELDS};

use crate::LeadExtractor;

/// A word Bedrock wrote that looks like a miscopied word of the message.
#[derive(Clone, Debug, PartialEq)]
pub struct Suspect {
    /// Index of the row in the extracted list.
    pub row: usize,
    /// Column key, as in [`listings::FIELDS`].
    pub field: &'static str,
    /// The word as Bedrock wrote it.
    pub written: String,
    /// The word as the message has it.
    pub original: String,
}

/// Finds words in the rows that look like miscopies of the message's words.
pub fn find_suspects(message: &str, rows: &[ListingInput]) -> Vec<Suspect> {
    let originals = words(message);
    let known: HashSet<String> = originals.iter().map(|w| normalize(w)).collect();
    let mut out = Vec::new();
    for (row, input) in rows.iter().enumerate() {
        for (field, _) in FIELDS {
            for written in words(input.get(field)) {
                let n = normalize(&written);
                if n.chars().count() < 3 || !has_arabic(&n) || known.contains(&n) {
                    continue;
                }
                let mut near: Vec<&String> = originals
                    .iter()
                    .filter(|o| one_edit_apart(&n, &normalize(o)))
                    .collect();
                near.sort_by_key(|o| normalize(o));
                near.dedup_by_key(|o| normalize(o));
                if let [original] = near.as_slice() {
                    out.push(Suspect {
                        row,
                        field,
                        written: written.clone(),
                        original: (*original).clone(),
                    });
                }
            }
        }
    }
    out
}

/// Puts the message's own word back for each confirmed suspect.
pub fn repair(rows: &mut [ListingInput], fixes: &[&Suspect]) {
    for s in fixes {
        let Some(input) = rows.get_mut(s.row) else {
            continue;
        };
        let fixed = replace_word(input.get(s.field), &s.written, &s.original);
        input.set(s.field, fixed);
    }
}

/// Without Jev, every suspect is repaired: the agent's own spelling is never
/// worse than a letter Bedrock made up.
pub struct SpellingGuard {
    inner: Arc<dyn LeadExtractor>,
}

impl SpellingGuard {
    pub fn new(inner: Arc<dyn LeadExtractor>) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl LeadExtractor for SpellingGuard {
    async fn extract(&self, text: &str, today: NaiveDate) -> Result<Vec<ListingInput>, String> {
        let mut rows = self.inner.extract(text, today).await?;
        let suspects = find_suspects(text, &rows);
        log(&suspects);
        repair(&mut rows, &suspects.iter().collect::<Vec<_>>());
        Ok(rows)
    }
}

pub(crate) fn log(suspects: &[Suspect]) {
    for s in suspects {
        tracing::info!(field = s.field, written = %s.written, original = %s.original, "miscopied word");
    }
}

/// Splits on anything that is not a letter, digit or Arabic mark.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || is_mark(c)))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Replaces whole-word occurrences of `from` in `s`.
fn replace_word(s: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut word = String::new();
    for c in s.chars().chain(std::iter::once('\0')) {
        if c != '\0' && (c.is_alphanumeric() || is_mark(c)) {
            word.push(c);
            continue;
        }
        out.push_str(if word == from { to } else { &word });
        word.clear();
        if c != '\0' {
            out.push(c);
        }
    }
    out
}

fn is_mark(c: char) -> bool {
    matches!(c, '\u{064B}'..='\u{065F}' | '\u{0670}' | '\u{0640}')
}

fn has_arabic(s: &str) -> bool {
    s.chars().any(|c| ('\u{0621}'..='\u{064A}').contains(&c))
}

/// Folds spellings agents use interchangeably, so they never count as slips.
pub(crate) fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !is_mark(*c))
        .map(|c| match c {
            'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
            'ى' => 'ي',
            'ة' => 'ه',
            'ؤ' => 'و',
            'ئ' => 'ي',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
}

/// True when one substitution, insertion or deletion turns `a` into `b`.
fn one_edit_apart(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (short, long) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };
    match long.len() - short.len() {
        0 => {
            short
                .iter()
                .zip(long.iter())
                .filter(|(x, y)| x != y)
                .count()
                == 1
        }
        1 => (0..long.len()).any(|skip| {
            long.iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, c)| c)
                .eq(short.iter())
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(offer_type: &str, location: &str) -> ListingInput {
        ListingInput {
            offer_type: offer_type.into(),
            location: location.into(),
            ..Default::default()
        }
    }

    #[test]
    fn finds_a_letter_bedrock_changed() {
        let msg = "أرض للبيع في حي العارض مخطط 2456";
        let rows = [row("أرد للبيع", "حي العارد")];
        let s = find_suspects(msg, &rows);
        let pairs: Vec<(&str, &str, &str)> = s
            .iter()
            .map(|s| (s.field, s.written.as_str(), s.original.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("offer_type", "أرد", "أرض"),
                ("location", "العارد", "العارض")
            ]
        );
    }

    #[test]
    fn repair_puts_the_agents_word_back() {
        let msg = "أرض للبيع في حي العارض";
        let mut rows = vec![row("أرد للبيع", "حي العارد، الرياض")];
        let s = find_suspects(msg, &rows);
        repair(&mut rows, &s.iter().collect::<Vec<_>>());
        assert_eq!(rows[0].offer_type, "أرض للبيع");
        assert_eq!(rows[0].location, "حي العارض، الرياض");
    }

    #[test]
    fn spelling_variants_are_not_slips() {
        let msg = "ارض للبيع واجهة شماليه حي الملقا";
        let rows = [row("أرض للبيع", "حي الملقا واجهة شمالية")];
        assert!(find_suspects(msg, &rows).is_empty());
    }

    #[test]
    fn words_the_agent_never_wrote_are_left_alone() {
        // "فيلا" is far from every word of the message: a rewording, not a slip.
        let msg = "مطلوب بيت في النرجس";
        let rows = [row("طلب شراء فيلا", "النرجس")];
        assert!(find_suspects(msg, &rows).is_empty());
    }

    #[test]
    fn ambiguous_words_are_left_alone() {
        // "سارع" is one letter from both "شارع" and "سارح": no way to tell which.
        let msg = "شارع سارح";
        let rows = [row("", "سارع")];
        assert!(find_suspects(msg, &rows).is_empty());
    }

    #[test]
    fn short_words_and_numbers_are_left_alone() {
        let msg = "قطعة 12 في حي";
        let rows = [row("", "قطعة 13 في حى")];
        assert!(find_suspects(msg, &rows).is_empty());
    }

    #[test]
    fn replace_only_touches_whole_words() {
        assert_eq!(
            replace_word("أرد وأرد، أردن", "أرد", "أرض"),
            "أرض وأرد، أردن"
        );
    }

    #[test]
    fn edit_distance_of_one() {
        assert!(one_edit_apart("ارد", "ارض"));
        assert!(one_edit_apart("العارض", "العارص"));
        assert!(one_edit_apart("الملقا", "الملق"));
        assert!(!one_edit_apart("الملقا", "الملقا"));
        assert!(!one_edit_apart("ارض", "بيت"));
    }
}
