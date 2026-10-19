//! Raw HTML: a small tag tokenizer (CommonMark's raw-HTML grammar), the set of always-recognised
//! inline tags, and entity decoding for text that pulldown-cmark leaves undecoded (HTML blocks,
//! `<summary>` titles).

use crate::adf::model::{HexColor, SubSup};

/// Longest tag the tokenizer accepts; longer "tags" are treated as text.
const MAX_TAG_LEN: usize = 16 * 1024;

/// One HTML construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HtmlTag {
    /// `<!-- … -->`.
    Comment,
    /// `<name attr=value …>` or `<name …/>`. `name` is lowercase.
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        self_closing: bool,
    },
    /// `</name>`. `name` is lowercase.
    Close { name: String },
    /// Processing instruction, declaration or CDATA section.
    Other,
}

/// An inline formatting tag that maps to an ADF mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HtmlMark {
    Underline,
    Strike,
    Strong,
    Em,
    SubSup(SubSup),
    /// `<kbd>` → `code`.
    Code,
    /// `<span style="color: …; background-color: …">`.
    Span {
        color: Option<HexColor>,
        background: Option<HexColor>,
    },
}

/// What a tag means to the converter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Recognised {
    /// Opens a mark; the frame is closed by a `</name>` with the same name.
    OpenMark {
        name: String,
        mark: HtmlMark,
    },
    /// `</name>` of a mark tag.
    CloseMark {
        name: String,
    },
    /// A self-closing mark tag such as `<u/>`: no effect.
    Empty,
    /// `<br>`.
    Br,
    /// `<!-- … -->`: removed silently.
    Comment,
    DetailsOpen,
    DetailsClose,
    SummaryOpen,
    SummaryClose,
    /// No ADF equivalent: subject to the HTML policy.
    Unknown,
}

/// Parses one HTML construct at the start of `s` (which must start with `<`). Returns the
/// construct and its length in bytes.
pub(crate) fn parse_tag(s: &str) -> Option<(HtmlTag, usize)> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'<') {
        return None;
    }
    if let Some(rest) = s.strip_prefix("<!--") {
        if rest.starts_with('>') {
            return Some((HtmlTag::Comment, 5));
        }
        if rest.starts_with("->") {
            return Some((HtmlTag::Comment, 6));
        }
        let end = rest.find("-->")?;
        return Some((HtmlTag::Comment, 4 + end + 3));
    }
    if let Some(rest) = s.strip_prefix("<?") {
        let end = rest.find("?>")?;
        return Some((HtmlTag::Other, 2 + end + 2));
    }
    if let Some(rest) = s.strip_prefix("<![CDATA[") {
        let end = rest.find("]]>")?;
        return Some((HtmlTag::Other, 9 + end + 3));
    }
    if let Some(rest) = s.strip_prefix("<!") {
        if rest.bytes().next().is_some_and(|b| b.is_ascii_alphabetic()) {
            let end = rest.find('>')?;
            return Some((HtmlTag::Other, 2 + end + 1));
        }
        return None;
    }
    // Open and closing tags are short; bounding them keeps block scanning linear.
    let window = s.get(..s.len().min(MAX_TAG_LEN)).unwrap_or(s);
    if let Some(rest) = window.strip_prefix("</") {
        let name_len = tag_name_len(rest)?;
        let name = rest.get(..name_len)?.to_ascii_lowercase();
        let after = rest.get(name_len..)?;
        let ws = after.len() - after.trim_start().len();
        if after.get(ws..)?.starts_with('>') {
            return Some((HtmlTag::Close { name }, 2 + name_len + ws + 1));
        }
        return None;
    }
    parse_open_tag(window)
}

fn tag_name_len(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if !bytes.first()?.is_ascii_alphabetic() {
        return None;
    }
    Some(
        bytes
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || **b == b'-')
            .count(),
    )
}

fn parse_open_tag(s: &str) -> Option<(HtmlTag, usize)> {
    let rest = s.get(1..)?;
    let name_len = tag_name_len(rest)?;
    let name = rest.get(..name_len)?.to_ascii_lowercase();
    let mut pos = 1 + name_len;
    let mut attrs = Vec::new();
    loop {
        let tail = s.get(pos..)?;
        let ws = tail.len() - tail.trim_start().len();
        let after_ws = tail.get(ws..)?;
        if after_ws.starts_with("/>") {
            return Some((
                HtmlTag::Open {
                    name,
                    attrs,
                    self_closing: true,
                },
                pos + ws + 2,
            ));
        }
        if after_ws.starts_with('>') {
            return Some((
                HtmlTag::Open {
                    name,
                    attrs,
                    self_closing: false,
                },
                pos + ws + 1,
            ));
        }
        if ws == 0 {
            return None;
        }
        let (attr, len) = parse_attribute(after_ws)?;
        attrs.push(attr);
        pos += ws + len;
    }
}

fn parse_attribute(s: &str) -> Option<((String, String), usize)> {
    let bytes = s.as_bytes();
    let first = *bytes.first()?;
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b':') {
        return None;
    }
    let name_len = bytes
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
        .count();
    let name = s.get(..name_len)?.to_ascii_lowercase();
    let after = s.get(name_len..)?;
    let ws = after.len() - after.trim_start().len();
    let Some(value_part) = after.get(ws..).and_then(|v| v.strip_prefix('=')) else {
        return Some(((name, String::new()), name_len));
    };
    let ws2 = value_part.len() - value_part.trim_start().len();
    let value_str = value_part.get(ws2..)?;
    let consumed_before_value = name_len + ws + 1 + ws2;
    let quote = value_str.bytes().next()?;
    if quote == b'"' || quote == b'\'' {
        let inner = value_str.get(1..)?;
        let end = inner.find(char::from(quote))?;
        let value = inner.get(..end)?.to_owned();
        return Some(((name, value), consumed_before_value + 1 + end + 1));
    }
    let len = value_str
        .bytes()
        .take_while(|b| {
            !b.is_ascii_whitespace() && !matches!(b, b'"' | b'\'' | b'=' | b'<' | b'>' | b'`')
        })
        .count();
    if len == 0 {
        return None;
    }
    let value = value_str.get(..len)?.to_owned();
    Some(((name, value), consumed_before_value + len))
}

/// Classifies a tag. `details` enables `<details>`/`<summary>`.
pub(crate) fn classify(tag: &HtmlTag, details: bool) -> Recognised {
    match tag {
        HtmlTag::Comment => Recognised::Comment,
        HtmlTag::Other => Recognised::Unknown,
        HtmlTag::Open {
            name,
            attrs,
            self_closing,
        } => {
            if name == "br" {
                return Recognised::Br;
            }
            if details && name == "details" && !self_closing {
                return Recognised::DetailsOpen;
            }
            if details && name == "summary" && !self_closing {
                return Recognised::SummaryOpen;
            }
            let Some(mark) = mark_for(name, attrs) else {
                return Recognised::Unknown;
            };
            if *self_closing {
                Recognised::Empty
            } else {
                Recognised::OpenMark {
                    name: name.clone(),
                    mark,
                }
            }
        }
        HtmlTag::Close { name } => match name.as_str() {
            "br" => Recognised::Br,
            "details" if details => Recognised::DetailsClose,
            "summary" if details => Recognised::SummaryClose,
            "u" | "ins" | "s" | "del" | "strike" | "strong" | "b" | "em" | "i" | "sub" | "sup"
            | "kbd" | "span" => Recognised::CloseMark { name: name.clone() },
            _ => Recognised::Unknown,
        },
    }
}

fn mark_for(name: &str, attrs: &[(String, String)]) -> Option<HtmlMark> {
    Some(match name {
        "u" | "ins" => HtmlMark::Underline,
        "s" | "del" | "strike" => HtmlMark::Strike,
        "strong" | "b" => HtmlMark::Strong,
        "em" | "i" => HtmlMark::Em,
        "sub" => HtmlMark::SubSup(SubSup::Sub),
        "sup" => HtmlMark::SubSup(SubSup::Sup),
        "kbd" => HtmlMark::Code,
        "span" => {
            let style = attrs.iter().find(|(k, _)| k == "style").map(|(_, v)| v)?;
            let (color, background) = parse_style(style)?;
            HtmlMark::Span { color, background }
        }
        _ => return None,
    })
}

/// Parses `color: #RRGGBB; background-color: #RRGGBB`. Any other declaration, or an invalid
/// colour, makes the whole span unrecognised.
fn parse_style(style: &str) -> Option<(Option<HexColor>, Option<HexColor>)> {
    let mut color = None;
    let mut background = None;
    for decl in style.split(';') {
        if decl.trim().is_empty() {
            continue;
        }
        let (prop, value) = decl.split_once(':')?;
        let value = HexColor::parse(value.trim())?;
        match prop.trim().to_ascii_lowercase().as_str() {
            "color" => color = Some(value),
            "background-color" => background = Some(value),
            _ => return None,
        }
    }
    (color.is_some() || background.is_some()).then_some((color, background))
}

/// One piece of an HTML block's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BlockPiece {
    /// Plain text, `start..end` in the scanned string.
    Text { start: usize, end: usize },
    /// A tag, `start..end` in the scanned string.
    Tag {
        tag: HtmlTag,
        start: usize,
        end: usize,
    },
}

/// What a [`NextFinder`] knows from its last search.
#[derive(Debug, Clone, Copy)]
enum Cached {
    /// No search yet.
    Unknown,
    /// No occurrence after the last query position.
    Absent,
    /// The next occurrence starts here.
    At(usize),
}

/// Finds the next occurrence of a terminator. Queries never move backwards, so a whole scan
/// costs O(n) per terminator even with many unterminated openers.
struct NextFinder<'a> {
    text: &'a str,
    needle: &'static str,
    cached: Cached,
}

impl<'a> NextFinder<'a> {
    fn new(text: &'a str, needle: &'static str) -> Self {
        Self {
            text,
            needle,
            cached: Cached::Unknown,
        }
    }

    /// Absolute position of the first occurrence at or after `from`.
    fn next_from(&mut self, from: usize) -> Option<usize> {
        match self.cached {
            Cached::Absent => return None,
            Cached::At(found) if found >= from => return Some(found),
            Cached::At(_) | Cached::Unknown => {}
        }
        let found = self
            .text
            .get(from..)
            .and_then(|rest| rest.find(self.needle))
            .map(|i| from + i);
        self.cached = found.map_or(Cached::Absent, Cached::At);
        found
    }
}

/// Splits the text of an HTML block into text and tags, in linear time.
pub(crate) fn scan_block(text: &str) -> Vec<BlockPiece> {
    let mut comment_end = NextFinder::new(text, "-->");
    let mut pi_end = NextFinder::new(text, "?>");
    let mut cdata_end = NextFinder::new(text, "]]>");
    let mut decl_end = NextFinder::new(text, ">");
    let mut pieces = Vec::new();
    let mut text_start = 0;
    let mut pos = 0;
    while let Some(found) = text.get(pos..).and_then(|rest| rest.find('<')) {
        let at = pos + found;
        let rest = text.get(at..).unwrap_or_default();
        let parsed = if rest.starts_with("<!-->") {
            Some((HtmlTag::Comment, 5))
        } else if rest.starts_with("<!--->") {
            Some((HtmlTag::Comment, 6))
        } else if rest.starts_with("<!--") {
            comment_end
                .next_from(at + 4)
                .map(|end| (HtmlTag::Comment, end + 3 - at))
        } else if rest.starts_with("<?") {
            pi_end
                .next_from(at + 2)
                .map(|end| (HtmlTag::Other, end + 2 - at))
        } else if rest.starts_with("<![CDATA[") {
            cdata_end
                .next_from(at + 9)
                .map(|end| (HtmlTag::Other, end + 3 - at))
        } else if rest.starts_with("<!") {
            if rest.as_bytes().get(2).is_some_and(u8::is_ascii_alphabetic) {
                decl_end
                    .next_from(at + 2)
                    .map(|end| (HtmlTag::Other, end + 1 - at))
            } else {
                None
            }
        } else {
            parse_tag(rest)
        };
        match parsed {
            Some((tag, len)) => {
                if text_start < at {
                    pieces.push(BlockPiece::Text {
                        start: text_start,
                        end: at,
                    });
                }
                pieces.push(BlockPiece::Tag {
                    tag,
                    start: at,
                    end: at + len,
                });
                pos = at + len;
                text_start = pos;
            }
            None => pos = at + 1,
        }
    }
    if text_start < text.len() {
        pieces.push(BlockPiece::Text {
            start: text_start,
            end: text.len(),
        });
    }
    pieces
}

/// Decodes HTML entities and numeric character references (`&amp;`, `&#32;`, `&#x1F600;`)
/// with pulldown-cmark's own entity table: every other ASCII punctuation character is
/// backslash-escaped so no other Markdown syntax can apply, and the text is parsed as the inside
/// of a one-line paragraph.
pub(crate) fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut escaped = String::with_capacity(text.len() * 2 + 2);
    escaped.push('x');
    for c in text.chars() {
        match c {
            '\n' | '\r' => escaped.push(' '),
            '&' | '#' | ';' => escaped.push(c),
            c if c.is_ascii_punctuation() => {
                escaped.push('\\');
                escaped.push(c);
            }
            c => escaped.push(c),
        }
    }
    escaped.push('x');
    let mut out = String::with_capacity(text.len());
    for event in pulldown_cmark::Parser::new_ext(&escaped, pulldown_cmark::Options::empty()) {
        if let pulldown_cmark::Event::Text(t) = event {
            out.push_str(&t);
        }
    }
    let out = out.strip_prefix('x').unwrap_or(&out);
    out.strip_suffix('x').unwrap_or(out).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(name: &str) -> HtmlTag {
        HtmlTag::Open {
            name: name.into(),
            attrs: vec![],
            self_closing: false,
        }
    }

    #[test]
    fn tokenizer() {
        assert_eq!(parse_tag("<u>x"), Some((open("u"), 3)));
        assert_eq!(parse_tag("<!-- a -->b"), Some((HtmlTag::Comment, 10)));
        assert_eq!(parse_tag("<!-->"), Some((HtmlTag::Comment, 5)));
        assert_eq!(
            parse_tag("</SPAN >"),
            Some((
                HtmlTag::Close {
                    name: "span".into()
                },
                8
            ))
        );
        assert_eq!(
            parse_tag("<br/>"),
            Some((
                HtmlTag::Open {
                    name: "br".into(),
                    attrs: vec![],
                    self_closing: true
                },
                5
            ))
        );
        let (tag, len) = parse_tag("<span style=\"color: #FF0000\" data-x=1 hidden>").unwrap();
        assert_eq!(len, 45);
        assert_eq!(
            tag,
            HtmlTag::Open {
                name: "span".into(),
                attrs: vec![
                    ("style".into(), "color: #FF0000".into()),
                    ("data-x".into(), "1".into()),
                    ("hidden".into(), String::new())
                ],
                self_closing: false
            }
        );
        assert_eq!(parse_tag("<?php x ?>"), Some((HtmlTag::Other, 10)));
        assert_eq!(parse_tag("<!DOCTYPE html>"), Some((HtmlTag::Other, 15)));
        assert_eq!(parse_tag("<![CDATA[x]]>"), Some((HtmlTag::Other, 13)));
        for bad in [
            "<",
            "< u>",
            "<1>",
            "<a b=\"x>",
            "<!-- open",
            "<a<b>",
            "</a b>",
        ] {
            assert_eq!(parse_tag(bad), None, "{bad}");
        }
    }

    #[test]
    fn classification() {
        let c = |s: &str| classify(&parse_tag(s).unwrap().0, true);
        assert!(matches!(
            c("<b>"),
            Recognised::OpenMark {
                mark: HtmlMark::Strong,
                ..
            }
        ));
        assert!(matches!(
            c("<KBD>"),
            Recognised::OpenMark {
                mark: HtmlMark::Code,
                ..
            }
        ));
        assert_eq!(c("<br>"), Recognised::Br);
        assert_eq!(c("</br>"), Recognised::Br);
        assert_eq!(c("<u/>"), Recognised::Empty);
        assert_eq!(c("<div>"), Recognised::Unknown);
        assert_eq!(c("<a href=\"x\">"), Recognised::Unknown);
        assert_eq!(c("<span>"), Recognised::Unknown);
        assert_eq!(c("<span style=\"font-size: 2em\">"), Recognised::Unknown);
        assert_eq!(c("<span style=\"color: red\">"), Recognised::Unknown);
        assert_eq!(
            c("<span style=\"background-color:#00ff00;color: #123456;\">"),
            Recognised::OpenMark {
                name: "span".into(),
                mark: HtmlMark::Span {
                    color: HexColor::parse("#123456"),
                    background: HexColor::parse("#00ff00")
                }
            }
        );
        assert_eq!(c("<details open>"), Recognised::DetailsOpen);
        assert_eq!(classify(&open("details"), false), Recognised::Unknown);
        assert_eq!(c("</div>"), Recognised::Unknown);
    }

    #[test]
    fn block_scanning() {
        let pieces = scan_block("a<b>c</b> 1 < 2 <!-- x");
        assert_eq!(
            pieces,
            vec![
                BlockPiece::Text { start: 0, end: 1 },
                BlockPiece::Tag {
                    tag: open("b"),
                    start: 1,
                    end: 4
                },
                BlockPiece::Text { start: 4, end: 5 },
                BlockPiece::Tag {
                    tag: HtmlTag::Close { name: "b".into() },
                    start: 5,
                    end: 9
                },
                BlockPiece::Text { start: 9, end: 22 },
            ]
        );
        // Many unterminated openers stay linear.
        for opener in ["<!--", "<?", "<![CDATA[", "<!a", "<a b='"] {
            let many = opener.repeat(200_000);
            assert_eq!(scan_block(&many).len(), 1, "{opener}");
        }
        // Terminated only at the very end: the first opener spans everything.
        let mut tail = "<?".repeat(100_000);
        tail.push_str("?>");
        assert_eq!(scan_block(&tail).len(), 1);
    }

    #[test]
    fn entities() {
        assert_eq!(decode_entities("a &amp; b"), "a & b");
        assert_eq!(decode_entities("&#32;x&#x41;&lt;&bogus;"), " xA<&bogus;");
        assert_eq!(
            decode_entities("*not em* `x` [l](u) # h &copy;"),
            "*not em* `x` [l](u) # h ©"
        );
        assert_eq!(decode_entities("  lead & trail  "), "  lead & trail  ");
        assert_eq!(decode_entities("no entities"), "no entities");
    }
}
