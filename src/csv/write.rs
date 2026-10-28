//! The CSV writer: quoting and spreadsheet-formula protection for one field.

use std::borrow::Cow;

/// Whether a spreadsheet may run a field that starts with `first` as a formula.
fn is_formula_start(first: char) -> bool {
    matches!(first, '=' | '+' | '-' | '@' | '\t' | '\r')
}

/// A field is quoted when it contains the delimiter, a double quote, a line feed or a carriage
/// return, or starts or ends with a space or tab, or starts with U+FEFF (a first field of that
/// kind would make the file start with a byte order mark, which readers drop).
fn needs_quotes(text: &str, delimiter: char) -> bool {
    text.contains([delimiter, '"', '\n', '\r'])
        || text.starts_with([' ', '\t', '\u{feff}'])
        || text.ends_with([' ', '\t'])
}

/// One field as it is written. With `csv_safe`, a field whose first character is `=`, `+`, `-`,
/// `@`, tab or carriage return gets a single quote in front, before quoting. Quotes inside a
/// quoted field are doubled.
pub(super) fn field(text: &str, delimiter: char, csv_safe: bool) -> String {
    let protect = csv_safe && text.chars().next().is_some_and(is_formula_start);
    let text: Cow<'_, str> = if protect {
        Cow::Owned(format!("'{text}"))
    } else {
        Cow::Borrowed(text)
    };
    if !needs_quotes(&text, delimiter) {
        return text.into_owned();
    }
    let mut out = String::with_capacity(text.len().saturating_add(2));
    out.push('"');
    for c in text.chars() {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> String {
        field(text, ',', false)
    }

    fn safe(text: &str) -> String {
        field(text, ',', true)
    }

    #[test]
    fn plain_fields_are_not_quoted() {
        for text in [
            "", "a", "a b", "é", "a;b", "a\u{a0}", "'x", "=1+1", "a'b", "\\",
        ] {
            assert_eq!(plain(text), text);
        }
    }

    #[test]
    fn a_field_is_quoted_for_the_delimiter_a_quote_or_a_line_break() {
        assert_eq!(plain("a,b"), "\"a,b\"");
        assert_eq!(plain("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(plain("\""), "\"\"\"\"");
        assert_eq!(plain("a\nb"), "\"a\nb\"");
        assert_eq!(plain("a\rb"), "\"a\rb\"");
    }

    #[test]
    fn a_field_that_starts_with_a_byte_order_mark_is_quoted() {
        assert_eq!(plain("\u{feff}a"), "\"\u{feff}a\"");
        // Only at the start: a mark inside the text is data and stays bare.
        assert_eq!(plain("a\u{feff}"), "a\u{feff}");
    }

    #[test]
    fn a_field_is_quoted_for_surrounding_spaces_and_tabs() {
        assert_eq!(plain(" a"), "\" a\"");
        assert_eq!(plain("a "), "\"a \"");
        assert_eq!(plain("\ta"), "\"\ta\"");
        assert_eq!(plain("a\t"), "\"a\t\"");
        assert_eq!(plain(" "), "\" \"");
    }

    #[test]
    fn the_delimiter_decides() {
        assert_eq!(field("a,b", ';', false), "a,b");
        assert_eq!(field("a;b", ';', false), "\"a;b\"");
        assert_eq!(field("a\tb", '\t', false), "\"a\tb\"");
        assert_eq!(field("a|b", '|', false), "\"a|b\"");
        // A tab inside is fine for a comma file.
        assert_eq!(field("a\tb", ',', false), "a\tb");
    }

    #[test]
    fn csv_safe_prefixes_formula_starts() {
        for text in ["=1+1", "+1", "-1", "@SUM(A1)"] {
            assert_eq!(safe(text), format!("'{text}"));
            assert_eq!(plain(text), text);
        }
    }

    #[test]
    fn csv_safe_covers_a_leading_tab_or_carriage_return() {
        assert_eq!(safe("\tx"), "'\tx");
        assert_eq!(safe("\rx"), "\"'\rx\"");
    }

    #[test]
    fn csv_safe_prefixes_before_quoting() {
        assert_eq!(safe("=A1,B1"), "\"'=A1,B1\"");
        assert_eq!(safe("=\"x\""), "\"'=\"\"x\"\"\"");
        assert_eq!(safe("-1 "), "\"'-1 \"");
    }

    #[test]
    fn csv_safe_leaves_other_fields_alone() {
        for text in ["", "a", "1", " =1", "a=1", "'=1", "x,y"] {
            assert_eq!(safe(text), plain(text));
        }
    }
}
