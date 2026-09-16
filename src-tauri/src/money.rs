//! Minimal USD money helpers for valuation. The Flip pillar will extend this with the Steam fee math.

/// Parse a Steam price string ("$1,234.56") into integer cents. Currency symbol and grouping commas
/// are ignored; v1 assumes USD.
pub fn parse_usd_cents(s: &str) -> Option<u64> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    let value: f64 = cleaned.parse().ok()?;
    if value < 0.0 {
        return None;
    }
    Some((value * 100.0).round() as u64)
}

pub fn format_usd_cents(cents: u64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

/// Parse a *human-typed* price into cents, tolerant of locale. Both `.` and `,` are accepted as the
/// decimal separator: the **last** separator is the decimal point, any earlier ones are grouping. So
/// `0,36` and `0.36` both yield 36; `1.234,56` and `1,234.56` both yield 123_456.
pub fn parse_price_input(s: &str) -> Option<u64> {
    let filtered: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect();
    let decimal_at = filtered.rfind(['.', ',']);
    let normalized: String = filtered
        .char_indices()
        .filter_map(|(i, c)| {
            if Some(i) == decimal_at {
                Some('.')
            } else if c == '.' || c == ',' {
                None // grouping separator → drop
            } else {
                Some(c)
            }
        })
        .collect();

    let value: f64 = normalized.parse().ok()?;
    if value < 0.0 {
        return None;
    }
    Some((value * 100.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_steam_price_strings() {
        assert_eq!(parse_usd_cents("$0.06"), Some(6));
        assert_eq!(parse_usd_cents("$0.49"), Some(49));
        assert_eq!(parse_usd_cents("$1,234.56"), Some(123_456));
        assert_eq!(parse_usd_cents("nope"), None);
    }

    #[test]
    fn formats_cents() {
        assert_eq!(format_usd_cents(6), "$0.06");
        assert_eq!(format_usd_cents(123_456), "$1234.56");
    }

    #[test]
    fn parses_human_input_in_either_locale() {
        // comma or dot as the decimal separator
        assert_eq!(parse_price_input("0,36"), Some(36));
        assert_eq!(parse_price_input("0.36"), Some(36));
        assert_eq!(parse_price_input("$1.50"), Some(150));
        // grouping + decimal, both locales
        assert_eq!(parse_price_input("1.234,56"), Some(123_456));
        assert_eq!(parse_price_input("1,234.56"), Some(123_456));
        assert_eq!(parse_price_input("abc"), None);
    }
}
