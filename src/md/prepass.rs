//! Source normalisation before parsing.
//!
//! - strips a UTF-8 BOM;
//! - turns CRLF and lone CR line endings into LF;
//! - blanks leading YAML front matter (`W_FRONT_MATTER_DROPPED`);
//! - replaces `::: cell` box fences with HTML-comment marker lines (`cells` extension).
//!
//! Every transformation keeps the number of lines and the column of every character outside the
//! replaced lines, so a [`LineIndex`](crate::diagnostics::LineIndex) over the normalised text
//! reports the same line and column as the original input (a BOM is not counted, like editors
//! do). Front matter is replaced by empty lines rather than removed for the same reason.
//!
//! Marker lines are HTML comments, which CommonMark parses as an HTML block that interrupts a
//! paragraph and closes open lists and quotes, exactly like a fence at column 0 should. They
//! are identified by their offset, never by their text, so user comments cannot forge a box.

use std::collections::BTreeSet;

use crate::diagnostics::Code;

use super::ir::Span;

/// Marker line that replaces a `::: cell` opening line.
const OPEN_MARKER: &str = "<!--adf-cell-->";
/// Marker line that replaces a box's closing `:::` line.
const CLOSE_MARKER: &str = "<!--/adf-cell-->";

/// The normalised source and what the prepass found.
#[derive(Debug, Clone, Default)]
pub(crate) struct Prepass {
    /// The normalised Markdown.
    pub(crate) text: String,
    /// Offsets (in `text`) of lines that open a box.
    pub(crate) cell_opens: BTreeSet<usize>,
    /// Offsets (in `text`) of lines that close a box.
    pub(crate) cell_closes: BTreeSet<usize>,
    /// Warnings, located by spans in `text`.
    pub(crate) warnings: Vec<(Code, String, Span)>,
    /// Leading front matter that was blanked: the 1-based line of its closing `---` and the
    /// column just after it. Its lines are empty in `text`, so it is located here instead.
    pub(crate) front_matter: Option<(usize, usize)>,
}

/// Runs the prepass. `cells` enables `::: cell` boxes.
pub(crate) fn run(source: &str, cells: bool) -> Prepass {
    run_impl(source, cells, false)
}

/// Like [`run`], but also removes trailing spaces and tabs from blank lines (lines of only
/// spaces, tabs and quote markers), outside top-level fenced code.
///
/// CommonMark treats such lines as blank, so this only changes trailing whitespace on blank
/// lines inside indented code (or code nested in containers). It is a fallback for input that
/// trips a pulldown-cmark 0.13 bug (a list item holding only link reference definitions,
/// followed by a deeply indented blank line, makes its offset iterator panic).
pub(crate) fn run_blanking_whitespace_lines(source: &str, cells: bool) -> Prepass {
    run_impl(source, cells, true)
}

fn run_impl(source: &str, cells: bool, blank_whitespace_lines: bool) -> Prepass {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let normalised = normalise_newlines(source);
    let mut out = Prepass::default();
    let lines: Vec<&str> = split_lines(&normalised);
    let front_matter_end = front_matter_end(&lines);
    let mut fence: Option<(u8, usize)> = None;
    let mut in_comment = false;
    // Colon count of the open box, if any.
    let mut open_box: Option<usize> = None;

    if let Some(end) = front_matter_end {
        let closing = lines.get(end).map_or(0, |line| line.chars().count());
        out.front_matter = Some((end + 1, closing + 1));
    }
    for (index, line) in lines.iter().enumerate() {
        let start = out.text.len();
        let span = start..start + line.len();
        if front_matter_end.is_some_and(|end| index <= end) {
            push_line(&mut out.text, "", index + 1 < lines.len());
            continue;
        }
        let mut replacement: Option<&str> = None;
        if let Some((ch, len)) = fence {
            if is_fence_close(line, ch, len) {
                fence = None;
            }
        } else if in_comment {
            if line.contains("-->") {
                in_comment = false;
            }
        } else if let Some(open) = fence_open(line) {
            fence = Some(open);
        } else if opens_comment(line) {
            in_comment = true;
        } else if let Some(colons) = cell_open(line) {
            if !cells {
                out.warnings.push((
                    Code::ExtensionDisabled,
                    "'::: cell' box syntax found, but the 'cells' extension is disabled; kept as text"
                        .to_owned(),
                    span,
                ));
            } else if open_box.is_some() {
                out.warnings.push((
                    Code::BlockFlattened,
                    "'::: cell' boxes cannot be nested; the inner opening line was kept as text"
                        .to_owned(),
                    span,
                ));
            } else {
                open_box = Some(colons);
                out.cell_opens.insert(start);
                replacement = Some(OPEN_MARKER);
            }
        } else if let Some(colons) = open_box
            && is_cell_close(line, colons)
        {
            open_box = None;
            out.cell_closes.insert(start);
            replacement = Some(CLOSE_MARKER);
        }
        // A blank line, possibly inside quotes (`>\t\t`): drop its trailing whitespace.
        if blank_whitespace_lines
            && fence.is_none()
            && replacement.is_none()
            && line.bytes().all(|b| matches!(b, b' ' | b'\t' | b'>'))
        {
            replacement = Some(line.trim_end_matches([' ', '\t']));
        }
        push_line(
            &mut out.text,
            replacement.unwrap_or(line),
            index + 1 < lines.len(),
        );
    }
    out
}

fn push_line(text: &mut String, line: &str, newline: bool) {
    text.push_str(line);
    if newline {
        text.push('\n');
    }
}

/// CRLF and lone CR → LF.
fn normalise_newlines(source: &str) -> String {
    if !source.contains('\r') {
        return source.to_owned();
    }
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// Splits on `\n`, keeping a final empty line when the text ends with a newline, so that
/// re-joining with `\n` restores the text exactly.
fn split_lines(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

/// Index of the line closing leading front matter (`---` … `---` or `...`), if any.
///
/// A document may simply start with a horizontal rule, and Markdown taken for front matter is
/// lost, so the block counts only if it reads as YAML: the first non-blank line inside is a
/// mapping key (`key: value`), and every other non-blank line is a key, an indented line (a
/// nested value or a continuation) or a sequence item (`- x`). Prose, headings, fences, tables
/// or quotes before the closing line make it Markdown. Empty "front matter" (`---`, blank lines,
/// `---`) is two thematic breaks, as CommonMark reads it.
fn front_matter_end(lines: &[&str]) -> Option<usize> {
    let first = lines.first()?;
    if first.trim_end_matches([' ', '\t']) != "---" {
        return None;
    }
    let mut keyed = false;
    // The last element after a trailing newline is an empty pseudo-line; it never closes.
    for (index, line) in lines.iter().enumerate().skip(1) {
        let trimmed = line.trim_end_matches([' ', '\t']);
        if trimmed == "---" || trimmed == "..." {
            return keyed.then_some(index);
        }
        if line.trim().is_empty() {
            continue;
        }
        let yaml = if keyed {
            looks_like_yaml_line(line)
        } else {
            looks_like_yaml_key(line)
        };
        if !yaml {
            return None;
        }
        keyed = true;
    }
    None
}

/// A line that can continue YAML front matter: a mapping key, an indented line, a sequence
/// item, or a `#` comment (only after the first key, so `---` followed by a Markdown heading is
/// not mistaken for front matter).
fn looks_like_yaml_line(line: &str) -> bool {
    looks_like_yaml_key(line)
        || line.starts_with([' ', '\t'])
        || line.trim_end() == "-"
        || line.starts_with("- ")
        || line.trim_end() == "#"
        || line.starts_with("# ")
}

/// `key:` followed by a space or the end of the line; the key does not start with whitespace,
/// `#`, `-` or `>`.
fn looks_like_yaml_key(line: &str) -> bool {
    let Some(first) = line.chars().next() else {
        return false;
    };
    if first.is_whitespace() || matches!(first, '#' | '-' | '>' | '*' | '+' | '|' | '`') {
        return false;
    }
    line.split_once(':').is_some_and(|(key, rest)| {
        !key.trim().is_empty() && (rest.is_empty() || rest.starts_with([' ', '\t']))
    })
}

/// Strips up to three leading spaces (the CommonMark block indentation allowance).
fn strip_indent(line: &str) -> Option<&str> {
    let spaces = line.bytes().take_while(|&b| b == b' ').count();
    (spaces <= 3).then(|| line.get(spaces..)).flatten()
}

/// A fenced code block opening: (fence character, fence length).
fn fence_open(line: &str) -> Option<(u8, usize)> {
    let rest = strip_indent(line)?;
    let ch = *rest.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = rest.bytes().take_while(|&b| b == ch).count();
    if len < 3 {
        return None;
    }
    // A backtick fence's info string may not contain backticks.
    if ch == b'`' && rest.get(len..).is_some_and(|info| info.contains('`')) {
        return None;
    }
    Some((ch, len))
}

fn is_fence_close(line: &str, ch: u8, len: usize) -> bool {
    let Some(rest) = strip_indent(line) else {
        return false;
    };
    let run = rest.bytes().take_while(|&b| b == ch).count();
    run >= len
        && rest
            .get(run..)
            .is_some_and(|tail| tail.trim_matches([' ', '\t']).is_empty())
}

/// A line that starts an HTML comment block which does not end on the same line.
fn opens_comment(line: &str) -> bool {
    strip_indent(line)
        .and_then(|rest| rest.strip_prefix("<!--"))
        .is_some_and(|rest| !rest.contains("-->"))
}

/// `:::` (three or more), optional spaces, `cell`, optional spaces; at column 0 only.
fn cell_open(line: &str) -> Option<usize> {
    let colons = line.bytes().take_while(|&b| b == b':').count();
    if colons < 3 {
        return None;
    }
    let rest = line.get(colons..)?.trim_matches([' ', '\t']);
    (rest == "cell").then_some(colons)
}

/// Exactly `colons` colons, then optional spaces.
fn is_cell_close(line: &str, colons: usize) -> bool {
    let run = line.bytes().take_while(|&b| b == b':').count();
    run == colons
        && line
            .get(run..)
            .is_some_and(|tail| tail.trim_matches([' ', '\t']).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_and_newlines() {
        let p = run("\u{feff}a\r\nb\rc\n", true);
        assert_eq!(p.text, "a\nb\nc\n");
        assert_eq!(p.warnings, vec![]);
    }

    #[test]
    fn front_matter_is_blanked_keeping_lines() {
        let p = run("---\ntitle: x\n---\n# H\n", true);
        assert_eq!(p.text, "\n\n\n# H\n");
        assert_eq!(p.warnings, vec![]);
        // Located by its original lines: up to the end of the closing `---`.
        assert_eq!(p.front_matter, Some((3, 4)));
        assert_eq!(run("# H\n", true).front_matter, None);
        // Unclosed front matter is not front matter.
        assert_eq!(run("---\nx\n", true).text, "---\nx\n");
        // `...` closes too.
        let p = run("---\na: 1\n...\nb", true);
        assert_eq!(p.text, "\n\n\nb");
        assert_eq!(p.front_matter, Some((3, 4)));
        // A leading rule followed by ordinary Markdown is not front matter.
        let src = "---\n# Title\n\ntext\n---\n";
        assert_eq!(run(src, true).text, src);
        // YAML comments after the first key are part of the front matter.
        let yaml = "---\ntitle: Runbook\n# owner is the on-call team\nowner: sre\n#\n---\n# H\n";
        assert_eq!(run(yaml, true).text, "\n\n\n\n\n\n# H\n");
        // A comment-like first line is a heading after a rule, not front matter.
        let src = "---\n# Title\nkey: value\n---\n";
        assert_eq!(run(src, true).text, src);
        // Empty front matter is two rules (CommonMark examples 43 and 47).
        assert_eq!(run("---\n---\nx", true).text, "---\n---\nx");
        assert_eq!(run("---\n\n---\n", true).text, "---\n\n---\n");
    }

    #[test]
    fn front_matter_must_read_as_yaml() {
        // Keys, nested values, block scalars, sequences, blank lines.
        let src = "---\ntitle: \"Runbook\"\ntags:\n  - ops\n- db\n\ndescription: >\n  Long\n  text\nempty:\n---\nx";
        let p = run(src, true);
        assert_eq!(p.text, "\n".repeat(11) + "x");
        assert_eq!(p.front_matter, Some((11, 4)));
        // A rule, then Markdown whose first line happens to look like a key: prose, a list...
        for src in [
            "---\nSummary: the outage lasted 2h.\n\nThe failover did not trigger because **the health check** was misconfigured.\n\n- step one\n- step two\n\n---\n\nMore text\n",
            "---\nStatus: open\n\n# Incident report\n\n| a | b |\n|---|---|\n\n---\n\nEnd",
            "---\nNote: see below\n\n```yaml\n---\n```\nmore",
            "---\nOwner: me\n> quoted\n---\n",
        ] {
            let p = run(src, true);
            assert_eq!(p.text, src);
            assert_eq!(p.front_matter, None);
        }
    }

    #[test]
    fn cells_are_replaced_by_markers() {
        let p = run("a\n::: cell\nx\n:::\nb\n", true);
        assert_eq!(p.text, "a\n<!--adf-cell-->\nx\n<!--/adf-cell-->\nb\n");
        assert_eq!(p.cell_opens.iter().copied().collect::<Vec<_>>(), vec![2]);
        assert_eq!(p.cell_closes.iter().copied().collect::<Vec<_>>(), vec![20]);
    }

    #[test]
    fn colon_counts_must_match_and_nesting_is_flattened() {
        let p = run(":::: cell\n::: cell\n:::\n::::\n", true);
        assert_eq!(p.text, "<!--adf-cell-->\n::: cell\n:::\n<!--/adf-cell-->\n");
        assert_eq!(p.warnings.len(), 1);
        assert_eq!(p.warnings[0].0, Code::BlockFlattened);
        assert_eq!(p.warnings[0].2, 16..24);
    }

    #[test]
    fn cells_not_recognised_in_code_comments_or_indented() {
        let src = "```\n::: cell\n```\n<!--\n::: cell\n-->\n ::: cell\n";
        let p = run(src, true);
        assert_eq!(p.text, src);
        assert!(p.cell_opens.is_empty());
        // Tilde fences and longer closing fences.
        let src = "~~~~\n::: cell\n~~~\n::: cell\n~~~~~\n";
        assert_eq!(run(src, true).text, src);
    }

    #[test]
    fn whitespace_only_lines_blanked_outside_fences() {
        let src = "a\n\t\t\n```\n  \n```\n    \n> >\t \nb";
        assert_eq!(run(src, true).text, src);
        assert_eq!(
            run_blanking_whitespace_lines(src, true).text,
            "a\n\n```\n  \n```\n\n> >\nb"
        );
    }

    #[test]
    fn unclosed_box_and_disabled_cells() {
        let p = run("::: cell\nx", true);
        assert_eq!(p.text, "<!--adf-cell-->\nx");
        assert!(p.cell_closes.is_empty());
        let p = run(":::cell\nx\n:::", false);
        assert_eq!(p.text, ":::cell\nx\n:::");
        assert_eq!(p.warnings[0].0, Code::ExtensionDisabled);
    }
}
