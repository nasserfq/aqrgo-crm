//! Arabic-Indic digit helpers (٠١٢٣٤٥٦٧٨٩).

const ARABIC_INDIC: [char; 10] = ['٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨', '٩'];
const EXTENDED: [char; 10] = ['۰', '۱', '۲', '۳', '۴', '۵', '۶', '۷', '۸', '۹'];

/// Converts any Arabic-Indic (or Persian) digits to ASCII, leaving the rest untouched.
pub fn to_ascii_digits(s: &str) -> String {
    s.chars()
        .map(|c| {
            ARABIC_INDIC
                .iter()
                .position(|&d| d == c)
                .or_else(|| EXTENDED.iter().position(|&d| d == c))
                .map(|i| char::from(b'0' + i as u8))
                .unwrap_or(c)
        })
        .collect()
}

/// Renders ASCII digits as Arabic-Indic, as in the original Word table's م column.
pub fn to_arabic_indic(s: &str) -> String {
    s.chars()
        .map(|c| {
            c.to_digit(10)
                .map(|d| ARABIC_INDIC[d as usize])
                .unwrap_or(c)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert_eq!(to_ascii_digits("١٢٣/٤٥"), "123/45");
        assert_eq!(to_ascii_digits("۱۲"), "12");
        assert_eq!(to_arabic_indic("12"), "١٢");
    }
}
