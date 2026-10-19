//! Escaping of text, code spans, link destinations and HTML attribute values so that the
//! Markdown parses back to exactly the same characters (CommonMark 0.31 / GFM plus this tool's
//! extensions, see `docs/syntax.md`).

use std::borrow::Cow;
use std::fmt::Write as _;

/// Where a run of text is written. Decides which characters are special.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextContext {
    /// Paragraph-like content: paragraphs, list items, quotes, task items. Lines can start
    /// blocks, so the start of a line needs extra escaping.
    Block,
    /// ATX heading content: `#` and trailing `{…}` are special; no block syntax.
    Heading,
    /// A GFM table cell: one line; leading/trailing whitespace is trimmed by the parser.
    TableCell,
}

/// Options for [`escape_text`].
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy)]
pub(super) struct TextEscape {
    pub context: TextContext,
    /// Roundtrip flavour: escape everything that this tool's own parser could misread.
    /// Plain flavour escapes only what changes the rendering.
    pub roundtrip: bool,
    /// The text is the first thing on its line (no markup before it): block syntax is special,
    /// and the parser strips leading spaces and tabs.
    pub line_start: bool,
    /// Only opening marks and blank text precede the text in its block (paragraph, heading,
    /// task item or table cell). Markdown → ADF trims spaces and tabs at the block's edges even
    /// inside marks, so leading ones are written as character references.
    pub block_start: bool,
    /// Trailing spaces and tabs would be trimmed: the text ends its block, followed at most by
    /// closing marks and blank text (or it ends a line the parser trims).
    pub line_end: bool,
    /// The text is inside a link label, where autolinks cannot occur.
    pub in_link: bool,
}

/// Escapes `text` (which must not contain `\n`) and appends it to `out`.
pub(super) fn escape_text(text: &str, opts: TextEscape, out: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    // Leading spaces/tabs are stripped by the parser at the start of a line, trailing ones at
    // the end of a block (and in cells), and Markdown → ADF trims both at a block's edges, also
    // inside marks; write them as character references, which are never trimmed.
    let lead = if opts.line_start || opts.block_start {
        chars.iter().take_while(|c| is_space_or_tab(**c)).count()
    } else {
        0
    };
    let trail = if opts.line_end {
        // Plain drops trailing whitespace (below), also when the text is only whitespace.
        chars
            .iter()
            .skip(if opts.roundtrip { lead } else { 0 })
            .rev()
            .take_while(|c| is_space_or_tab(**c))
            .count()
    } else {
        0
    };
    let body_end = chars.len() - trail;
    for (i, &c) in chars.iter().enumerate() {
        if i >= body_end && !opts.roundtrip {
            // Plain: trailing spaces carry no meaning for a reader; drop them instead of
            // writing `&#32;` noise.
            continue;
        }
        if i < lead || i >= body_end {
            push_char_ref(c, out);
            continue;
        }
        let prev = i.checked_sub(1).and_then(|p| chars.get(p)).copied();
        let next = chars.get(i + 1).copied();
        let at_start = opts.line_start && i == lead && lead == 0;
        if needs_escape(c, prev, next, &chars, i, at_start, opts) {
            out.push('\\');
        }
        if c.is_control() && c != '\t' {
            push_char_ref(c, out);
        } else {
            out.push(c);
        }
    }
}

fn is_space_or_tab(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// Writes `&#N;` for `c`.
fn push_char_ref(c: char, out: &mut String) {
    let _ = write!(out, "&#{};", u32::from(c));
}

#[allow(clippy::fn_params_excessive_bools, clippy::too_many_arguments)]
fn needs_escape(
    c: char,
    prev: Option<char>,
    next: Option<char>,
    chars: &[char],
    i: usize,
    at_start: bool,
    opts: TextEscape,
) -> bool {
    let block = opts.context == TextContext::Block;
    let heading = opts.context == TextContext::Heading;
    let special = match c {
        '\\' | '`' | '*' | '[' | ']' | '<' | '|' | '~' => true,
        '_' => {
            !(prev.is_some_and(char::is_alphanumeric) && next.is_some_and(char::is_alphanumeric))
        }
        '&' => next.is_some_and(|n| n.is_ascii_alphanumeric() || n == '#'),
        '#' => heading || (block && at_start && opens_heading(chars, i)),
        '>' | '-' | '+' | '=' => block && at_start,
        ':' => {
            (block && at_start)
                || (opts.roundtrip
                    && !opts.in_link
                    && next == Some('/')
                    && chars.get(i + 2) == Some(&'/'))
        }
        '.' | ')' => {
            block
                && is_ordered_marker_end(chars, i, opts.line_start)
                && next.is_none_or(is_space_or_tab)
        }
        '$' | '^' => opts.roundtrip,
        '{' | '}' => opts.roundtrip && heading,
        '@' => opts.roundtrip && !opts.in_link && is_email_at(prev, next),
        _ => false,
    };
    special || (c == '.' && opts.roundtrip && !opts.in_link && is_www_dot(chars, i))
}

/// `#`…`######` followed by a space, a tab or the end (of this text, conservatively) would open
/// an ATX heading.
fn opens_heading(chars: &[char], i: usize) -> bool {
    let rest = chars.get(i..).unwrap_or_default();
    let run = rest.iter().take_while(|c| **c == '#').count();
    run <= 6 && rest.get(run).is_none_or(|c| is_space_or_tab(*c))
}

/// `1.` / `1)` at the start of a line would start an ordered list.
fn is_ordered_marker_end(chars: &[char], i: usize, line_start: bool) -> bool {
    line_start
        && i > 0
        && i <= 9
        && chars
            .get(..i)
            .is_some_and(|digits| digits.iter().all(char::is_ascii_digit))
}

/// The dot of `www.` at a word start (GFM extended `www.` autolinks).
fn is_www_dot(chars: &[char], i: usize) -> bool {
    if i < 3 {
        return false;
    }
    let Some(www) = chars.get(i - 3..i) else {
        return false;
    };
    if !www.iter().all(|c| c.eq_ignore_ascii_case(&'w')) {
        return false;
    }
    let before = i.checked_sub(4).and_then(|p| chars.get(p));
    !before.is_some_and(|c| c.is_alphanumeric())
}

/// An `@` that could be part of an email autolink (`a@b.c`) as Markdown → ADF finds them
/// (`linkify`: RFC 5321 atoms plus any non-ASCII character in the local part, and
/// internationalised domains).
fn is_email_at(prev: Option<char>, next: Option<char>) -> bool {
    let local =
        |c: char| c.is_ascii_alphanumeric() || !c.is_ascii() || "!#$%&'*+-/=?^_`{|}~.".contains(c);
    prev.is_some_and(local) && next.is_some_and(|c| c.is_ascii_alphanumeric() || !c.is_ascii())
}

/// A control character other than tab and line feed. Markdown → ADF removes these
/// (`W_CONTROL_CHAR`), so roundtrip passes text holding them through as raw ADF; and written
/// raw, ESC and BEL would reach the terminal of whoever prints the output.
pub(super) fn is_unsafe_control(c: char) -> bool {
    c.is_control() && c != '\t' && c != '\n'
}

/// Whether `text` holds a character that [`is_unsafe_control`].
pub(super) fn has_unsafe_control(text: &str) -> bool {
    text.chars().any(is_unsafe_control)
}

/// Where code is written verbatim (code spans, info strings, plain-flavour code blocks), control
/// characters other than tab become U+FFFD, so none reaches stdout raw. Line feeds are kept only
/// where `keep_lines` (code blocks), where CR LF and a lone CR also become a line feed.
pub(super) fn replace_controls(text: &str, keep_lines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if keep_lines => {
                if chars.peek() != Some(&'\n') {
                    out.push('\n');
                }
            }
            '\n' if keep_lines => out.push('\n'),
            c if c != '\t' && c.is_control() => out.push(char::REPLACEMENT_CHARACTER),
            c => out.push(c),
        }
    }
    out
}

/// A code span for `content` (non-empty, no `\n`), with a backtick fence longer than any run
/// inside and padding where the parser would otherwise strip or merge characters. In table
/// cells `|` is written `\|`, which GFM unescapes even inside code spans. Control characters
/// are never written raw ([`replace_controls`]).
pub(super) fn code_span(content: &str, in_table: bool) -> String {
    let fence = "`".repeat(longest_run(content, '`') + 1);
    let all_spaces = content.chars().all(|c| c == ' ');
    let pad = content.starts_with('`')
        || content.ends_with('`')
        || (!all_spaces && content.starts_with(' ') && content.ends_with(' '));
    let mut out = String::with_capacity(content.len() + 2 * fence.len() + 2);
    out.push_str(&fence);
    if pad {
        out.push(' ');
    }
    let content = if content.chars().any(char::is_control) {
        Cow::Owned(replace_controls(content, false))
    } else {
        Cow::Borrowed(content)
    };
    if in_table {
        out.push_str(&content.replace('|', "\\|"));
    } else {
        out.push_str(&content);
    }
    if pad {
        out.push(' ');
    }
    out.push_str(&fence);
    out
}

/// The length of the longest run of `ch` in `s`.
pub(super) fn longest_run(s: &str, ch: char) -> usize {
    let mut best = 0;
    let mut current = 0;
    for c in s.chars() {
        if c == ch {
            current += 1;
            best = best.max(current);
        } else {
            current = 0;
        }
    }
    best
}

/// A link destination: bare when possible, `<…>` when it contains spaces or angle brackets.
pub(super) fn link_destination(url: &str, in_table: bool) -> String {
    let angle = url.is_empty()
        || url
            .chars()
            .any(|c| c == ' ' || c == '<' || c == '>' || c.is_control());
    let mut out = String::with_capacity(url.len() + 2);
    if angle {
        out.push('<');
    }
    let chars: Vec<char> = url.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        match c {
            '\\' | '<' | '>' => out.push('\\'),
            '(' | ')' if !angle => out.push('\\'),
            '|' if in_table => out.push('\\'),
            '&' if next.is_some_and(|n| n.is_ascii_alphanumeric() || n == '#') => out.push('\\'),
            _ => {}
        }
        if c.is_control() {
            push_char_ref(c, &mut out);
        } else {
            out.push(c);
        }
    }
    if angle {
        out.push('>');
    }
    out
}

/// A code fence info string (a language without whitespace): backslash escapes and entities
/// are decoded there, so `\` and entity-like `&` are escaped. Control characters become U+FFFD.
pub(super) fn info_string(language: &str) -> String {
    let mut out = String::with_capacity(language.len());
    let chars: Vec<char> = language.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        if c == '\\' || (c == '&' && next.is_some_and(|n| n.is_ascii_alphanumeric() || n == '#')) {
            out.push('\\');
        }
        out.push(if c.is_control() {
            char::REPLACEMENT_CHARACTER
        } else {
            c
        });
    }
    out
}

/// A link title in double quotes.
pub(super) fn link_title(title: &str, in_table: bool) -> String {
    let mut out = String::with_capacity(title.len() + 2);
    out.push('"');
    let chars: Vec<char> = title.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        match c {
            '\\' | '"' => out.push('\\'),
            '|' if in_table => out.push('\\'),
            '&' if next.is_some_and(|n| n.is_ascii_alphanumeric() || n == '#') => out.push('\\'),
            _ => {}
        }
        if c.is_control() {
            push_char_ref(c, &mut out);
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// Text inside an HTML element such as `<summary>`: `&`, `<`, `>` and line breaks as references;
/// leading and trailing whitespace too, so it survives trimming.
pub(super) fn html_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lead = chars.iter().take_while(|c| c.is_whitespace()).count();
    let trail = chars
        .iter()
        .skip(lead)
        .rev()
        .take_while(|c| c.is_whitespace())
        .count();
    let body_end = chars.len() - trail;
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if c.is_control() || ((i < lead || i >= body_end) && c.is_whitespace()) => {
                push_char_ref(c, &mut out);
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esc(text: &str, context: TextContext, line_start: bool, line_end: bool) -> String {
        let mut out = String::new();
        escape_text(
            text,
            TextEscape {
                context,
                roundtrip: true,
                line_start,
                block_start: false,
                line_end,
                in_link: false,
            },
            &mut out,
        );
        out
    }

    fn block(text: &str) -> String {
        esc(text, TextContext::Block, true, true)
    }

    #[test]
    fn inline_specials_are_always_escaped() {
        assert_eq!(block(r"a\b*c`d[e]f<g|h~i"), r"a\\b\*c\`d\[e\]f\<g\|h\~i");
    }

    #[test]
    fn underscore_only_outside_words() {
        assert_eq!(block("snake_case _x_ a_"), r"snake_case \_x\_ a\_");
    }

    #[test]
    fn line_start_block_syntax() {
        assert_eq!(block("# h"), r"\# h");
        assert_eq!(block("- x"), r"\- x");
        assert_eq!(block("+ x"), r"\+ x");
        assert_eq!(block("> q"), r"\> q");
        assert_eq!(block("==="), r"\===");
        assert_eq!(block("::: cell"), r"\::: cell");
        assert_eq!(block("12. x"), r"12\. x");
        assert_eq!(block("3) x"), r"3\) x");
        assert_eq!(block("1234567890. x"), "1234567890. x");
        assert_eq!(block("1."), r"1\.");
        assert_eq!(block("#"), r"\#");
        assert_eq!(block("###### six"), r"\###### six");
        // Cannot open a heading or a list: left alone for readability.
        assert_eq!(block("#hashtag"), "#hashtag");
        assert_eq!(block("####### seven"), "####### seven");
        assert_eq!(block("1.5 million"), "1.5 million");
        // Not at the start of the line: harmless.
        assert_eq!(
            esc("# - > 1. x", TextContext::Block, false, false),
            "# - > 1. x"
        );
    }

    #[test]
    fn whitespace_at_line_edges_becomes_references() {
        assert_eq!(block("  a b "), "&#32;&#32;a b&#32;");
        assert_eq!(block("\tx"), "&#9;x");
        assert_eq!(block("   "), "&#32;&#32;&#32;");
        assert_eq!(esc(" a ", TextContext::Block, false, false), " a ");
    }

    #[test]
    fn whitespace_at_block_edges_inside_marks_becomes_references() {
        let mut out = String::new();
        escape_text(
            " \ta  ",
            TextEscape {
                context: TextContext::TableCell,
                roundtrip: true,
                line_start: false,
                block_start: true,
                line_end: true,
                in_link: false,
            },
            &mut out,
        );
        assert_eq!(out, "&#32;&#9;a&#32;&#32;");
        // Leading references do not make the following character line-initial.
        out.clear();
        escape_text(
            " # x",
            TextEscape {
                context: TextContext::Block,
                roundtrip: true,
                line_start: false,
                block_start: true,
                line_end: false,
                in_link: false,
            },
            &mut out,
        );
        assert_eq!(out, "&#32;# x");
    }

    #[test]
    fn autolink_triggers() {
        assert_eq!(
            block("see https://x.org, www.y.com or a@b.cd"),
            r"see https\://x.org, www\.y.com or a\@b.cd"
        );
        assert_eq!(block("awww.x @me"), "awww.x @me");
        let mut out = String::new();
        escape_text(
            "https://x.org",
            TextEscape {
                context: TextContext::Block,
                roundtrip: true,
                line_start: false,
                block_start: false,
                line_end: false,
                in_link: true,
            },
            &mut out,
        );
        assert_eq!(out, "https://x.org");
    }

    #[test]
    fn entities_and_controls() {
        assert_eq!(block("AT&T &amp; & x"), r"AT\&T \&amp; & x");
        assert_eq!(block("a\u{7}b"), "a&#7;b");
    }

    #[test]
    fn heading_specials() {
        assert_eq!(
            esc("a # b {#id}", TextContext::Heading, true, true),
            r"a \# b \{\#id\}"
        );
    }

    #[test]
    fn code_spans() {
        assert_eq!(code_span("x", false), "`x`");
        assert_eq!(code_span("a`b", false), "``a`b``");
        assert_eq!(code_span("`a", false), "`` `a ``");
        assert_eq!(code_span(" a ", false), "`  a  `");
        assert_eq!(code_span(" a", false), "` a`");
        assert_eq!(code_span("  ", false), "`  `");
        assert_eq!(code_span("a|b", true), r"`a\|b`");
    }

    #[test]
    fn destinations_and_titles() {
        assert_eq!(
            link_destination("https://x/a(b)", false),
            r"https://x/a\(b\)"
        );
        assert_eq!(link_destination("https://x/a b", false), "<https://x/a b>");
        assert_eq!(link_destination("a\\b&amp;", false), r"a\\b\&amp;");
        assert_eq!(link_destination("a|b", true), r"a\|b");
        assert_eq!(link_title(r#"say "hi" \o/"#, false), r#""say \"hi\" \\o/""#);
        assert_eq!(info_string(r"\*a&amp;b&"), r"\\*a\&amp;b&");
    }

    #[test]
    fn html_text_escapes() {
        assert_eq!(html_text(" a<b> & c\n"), "&#32;a&lt;b&gt; &amp; c&#10;");
    }
}
