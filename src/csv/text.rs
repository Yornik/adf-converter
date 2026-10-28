//! One CSV field as a table cell's inline content. The text is never parsed as Markdown: the
//! only things made of it are hard breaks (for line breaks) and links (for bare `http://` and
//! `https://` URLs).

use crate::adf::model::{Formatting, Inline, Link, NonEmptyString, Text, TextMarks};
use crate::md::{find_autolinks, strip_controls};

/// The text of a cell: control characters other than tab and line feed removed, spaces, tabs and
/// line breaks trimmed from both ends. Also tells whether control characters were removed.
pub(super) fn clean(raw: &str) -> (String, bool) {
    let stripped = strip_controls(raw);
    let removed = stripped.len() != raw.len();
    let trimmed = stripped.trim_matches([' ', '\t', '\n']);
    (trimmed.to_owned(), removed)
}

/// The inline content of a cleaned cell text: text, links and a hard break per line break. An
/// empty text gives no content.
pub(super) fn inlines(text: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push(Inline::HardBreak);
        }
        push_line(line, &mut out);
    }
    out
}

fn push_line(line: &str, out: &mut Vec<Inline>) {
    let mut done = 0;
    if line.contains("://") {
        for (start, end, href) in find_autolinks(line) {
            let Some(url) = line.get(start..end).filter(|url| is_http(url)) else {
                continue;
            };
            push_plain(line.get(done..start).unwrap_or_default(), out);
            let marks = Formatting {
                link: Some(Link::new(href, None)),
                ..Formatting::default()
            };
            if let Some(text) = NonEmptyString::new(url) {
                out.push(Inline::Text(Text {
                    text,
                    marks: TextMarks::Formatted(marks),
                }));
            }
            done = end;
        }
    }
    push_plain(line.get(done..).unwrap_or_default(), out);
}

fn push_plain(text: &str, out: &mut Vec<Inline>) {
    if let Some(text) = Text::plain(text) {
        out.push(Inline::Text(text));
    }
}

/// Whether `url` starts with `http://` or `https://` (any case). Autolinks of the other kinds
/// (`www.` and email addresses) are not made in a CSV cell.
fn is_http(url: &str) -> bool {
    let head = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    head.starts_with("http://") || head.starts_with("https://")
}

/// Rough size in bytes of the compact JSON of `inlines`, to keep output within budget.
pub(super) fn estimated_json_bytes(inlines: &[Inline]) -> usize {
    inlines
        .iter()
        .map(|inline| match inline {
            Inline::Text(text) => {
                let link = match &text.marks {
                    TextMarks::Formatted(f) => f
                        .link
                        .as_ref()
                        .map_or(0, |l| l.href().as_str().len().saturating_add(60)),
                    TextMarks::Code { .. } => 0,
                };
                text.text
                    .as_str()
                    .len()
                    .saturating_add(link)
                    .saturating_add(30)
            }
            Inline::HardBreak
            | Inline::Mention(_)
            | Inline::Emoji(_)
            | Inline::Date(_)
            | Inline::Status(_)
            | Inline::InlineCard(_) => 30,
            Inline::MediaInline(media) => 120usize
                .saturating_add(media.file.id.as_str().len())
                .saturating_add(media.file.collection.as_str().len())
                .saturating_add(media.file.alt.as_str().len()),
        })
        .fold(0, usize::saturating_add)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn value(inlines: &[Inline]) -> Value {
        serde_json::to_value(inlines).unwrap()
    }

    fn plain(text: &str) -> Value {
        json!({"type": "text", "text": text})
    }

    fn link(text: &str, href: &str) -> Value {
        json!({"type": "text", "text": text,
               "marks": [{"type": "link", "attrs": {"href": href}}]})
    }

    #[test]
    fn clean_trims_spaces_tabs_and_line_breaks() {
        assert_eq!(clean(" \t\na b \n\t "), ("a b".to_owned(), false));
        assert_eq!(clean("a\nb"), ("a\nb".to_owned(), false));
        assert_eq!(clean("   "), (String::new(), false));
        // Other whitespace stays.
        assert_eq!(clean("\u{a0}a").0, "\u{a0}a");
    }

    #[test]
    fn clean_removes_control_characters_first_and_then_trims() {
        assert_eq!(clean("\u{1} a"), ("a".to_owned(), true));
        assert_eq!(clean("a\0b\u{7f}c\u{85}d"), ("abcd".to_owned(), true));
        assert_eq!(clean("a\tb\nc"), ("a\tb\nc".to_owned(), false));
        assert_eq!(clean("\0"), (String::new(), true));
    }

    #[test]
    fn plain_text_and_hard_breaks() {
        assert_eq!(inlines("").len(), 0);
        assert_eq!(value(&inlines("a")), json!([plain("a")]));
        assert_eq!(
            value(&inlines("a\nb")),
            json!([plain("a"), {"type": "hardBreak"}, plain("b")])
        );
        assert_eq!(
            value(&inlines("a\n\nb")),
            json!([plain("a"), {"type": "hardBreak"}, {"type": "hardBreak"}, plain("b")])
        );
    }

    #[test]
    fn markdown_is_not_parsed() {
        for text in [
            "*a* _b_ `c` <b>d</b> [x](y) C:\\temp \\ & # | ~~s~~ **b**",
            "# heading",
            "- item",
            "1. item",
            "> quote",
            "![alt](http-ish)",
        ] {
            assert_eq!(value(&inlines(text)), json!([plain(text)]), "{text}");
        }
    }

    #[test]
    fn bare_http_urls_become_links() {
        assert_eq!(
            value(&inlines("see https://example.com/a?b=1&c=2 now")),
            json!([
                plain("see "),
                link(
                    "https://example.com/a?b=1&c=2",
                    "https://example.com/a?b=1&c=2"
                ),
                plain(" now"),
            ])
        );
        assert_eq!(
            value(&inlines("http://example.com")),
            json!([link("http://example.com", "http://example.com")])
        );
    }

    #[test]
    fn trailing_punctuation_is_not_part_of_the_url() {
        for (text, url, before, after) in [
            ("https://example.com/a.", "https://example.com/a", "", "."),
            ("https://example.com/a,", "https://example.com/a", "", ","),
            ("(https://example.com/a)", "https://example.com/a", "(", ")"),
        ] {
            let mut expected = Vec::new();
            if !before.is_empty() {
                expected.push(plain(before));
            }
            expected.push(link(url, url));
            expected.push(plain(after));
            assert_eq!(value(&inlines(text)), Value::Array(expected), "{text}");
        }
    }

    #[test]
    fn other_autolinks_are_not_made() {
        for text in [
            "www.example.com",
            "a@example.com",
            "ftp://example.com/x",
            "mailto:a@b.c",
        ] {
            assert_eq!(value(&inlines(text)), json!([plain(text)]), "{text}");
        }
    }

    #[test]
    fn a_url_does_not_span_a_line_break() {
        let got = value(&inlines("https://a.example/x\nhttps://b.example/y"));
        assert_eq!(
            got,
            json!([
                link("https://a.example/x", "https://a.example/x"),
                {"type": "hardBreak"},
                link("https://b.example/y", "https://b.example/y"),
            ])
        );
    }

    #[test]
    fn a_scheme_without_a_host_is_text() {
        assert_eq!(value(&inlines("https://")), json!([plain("https://")]));
    }
}
