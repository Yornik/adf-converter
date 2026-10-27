//! Which cells count as numbers, for `--align-numbers`.

/// Whether `text` (already trimmed) matches `^[+-]?(\d[\d,. _]*)?\d%?$`: an optional sign, digits
/// that may be grouped with commas, dots, spaces and underscores, and an optional percent sign.
/// `\d` is an ASCII digit.
pub(super) fn is_number(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let body = unsigned.strip_suffix('%').unwrap_or(unsigned);
    let mut chars = body.chars();
    let (Some(first), Some(last)) = (chars.next(), body.chars().next_back()) else {
        return false;
    };
    if !first.is_ascii_digit() || !last.is_ascii_digit() {
        return false;
    }
    chars.all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | ' ' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        for yes in [
            "0", "7", "42", "-3", "+3", "9.99", "1,250.50", "1 250", "1_000", "12%", "-12.5%",
            "1.2.3", "1,2", "1 2 3", "00", "1__2", "+1%",
        ] {
            assert!(is_number(yes), "{yes:?}");
        }
    }

    #[test]
    fn not_numbers() {
        for no in [
            "", "-", "+", "%", "-%", "+%", "a", "1a", "a1", "1.", ".5", ",5", "5,", "5 ", " 5",
            "1e5", "--1", "+-1", "1-", "12%%", "5%5", "%5", "1\t2", "1\n2", "١٢", "€5", "5€",
            "0x10", "1.0f", "−3",
        ] {
            assert!(!is_number(no), "{no:?}");
        }
    }
}
