//! Table-cell budget: a linear pre-scan that refuses Markdown whose GFM tables would expand to
//! more than [`MAX_TABLE_CELLS`](crate::limits::MAX_TABLE_CELLS) cells, before pulldown-cmark
//! sees it.
//!
//! GFM pads every short row with empty cells up to the header width, so a 3 KB table of 256
//! columns and a thousand one-character rows becomes 256,000 cells (about 14 MB of ADF).
//! pulldown-cmark builds its whole tree, padding included, before it emits the first event, so
//! the tree builder alone cannot stop such input in time; this scan estimates the padded size
//! from the source text instead.
//!
//! The scan looks at one line at a time, after stripping the leading spaces, tabs and `>` of
//! quotes and list indentation:
//!
//! - a *delimiter row* is a line of only spaces, tabs, `:`, `-` and `|`, with at least one `|`
//!   and one `-`, right after a line holding a `|` (the header row). Its column count is
//!   computed exactly like pulldown-cmark does;
//! - from the header row on, every line until a blank line (only spaces, tabs and `>`) counts
//!   as a row of that many cells. When another delimiter row appears in the run, the run uses
//!   the wider of the two widths and counts the new header row again.
//!
//! Over all lines, this over-estimates (a heading right after a table still counts as a row, for
//! example) but never under-estimates the cells pulldown-cmark builds: it ends a table no later
//! than at such a blank line, a table needs a header line with a pipe right before its
//! delimiter row, and its width is the delimiter row's column count (rows with more cells are
//! cut to it).
//!
//! Fenced code is not Markdown, so its table-like lines do not count towards the limit. Telling
//! fenced code apart reliably needs the full block parser, though (an HTML block or a closed
//! list can end a fence early), and input that fools the fence tracking must not get through.
//! The scan therefore keeps a second tally over every line, fenced or not, and refuses the
//! input when that one exceeds [`FENCED_FACTOR`] times the limit: a bound pulldown-cmark can
//! handle comfortably. The tree builder also counts the cells of the tables it actually builds
//! (see `tree.rs`), so the limit holds exactly whatever the estimate says.
//!
//! The scan is `O(input length)` and allocates nothing.

use crate::diagnostics::{Code, Diagnostic};

use super::cx::Locator;

/// How many times the limit the tally over all lines, including fenced code, may reach.
pub(crate) const FENCED_FACTOR: usize = 10;

/// Hint for every table-budget error.
pub(crate) const HINT: &str = "GFM pads every table row with empty cells up to the header width; split large tables, drop unused columns, or attach the data as a file";

/// The message of the error reported when tables exceed `limit` cells.
pub(crate) fn message(limit: usize) -> String {
    format!("Tables in the input would expand to more than {limit} cells")
}

/// The table budget was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Exceeded {
    /// Offset of the header row of the table that crossed the budget.
    pub(crate) header: usize,
    /// The safety tally that includes fenced code crossed `FENCED_FACTOR × limit`, not the tally
    /// of tables outside fenced code.
    pub(crate) fenced: bool,
}

impl Exceeded {
    /// The `E_INPUT_TOO_LARGE` diagnostic for this result, located in `text`.
    pub(crate) fn diagnostic(self, text: &str, limit: usize) -> Diagnostic {
        let message = if self.fenced {
            format!(
                "Table-like lines in the input, including those in fenced code, would expand to more than {} cells",
                limit.saturating_mul(FENCED_FACTOR)
            )
        } else {
            message(limit)
        };
        let location = Locator::new(text).location(&(self.header..self.header));
        Diagnostic::new(Code::InputTooLarge, message)
            .at(location)
            .with_hint(HINT)
    }
}

/// Checks that the tables of the normalised Markdown `text` expand to at most `limit` cells, and
/// table-like lines anywhere (fenced code included) to at most `FENCED_FACTOR × limit`.
pub(crate) fn check(text: &str, limit: usize) -> Result<(), Exceeded> {
    scan(text, limit, limit.saturating_mul(FENCED_FACTOR))
}

fn scan(text: &str, limit: usize, fenced_limit: usize) -> Result<(), Exceeded> {
    // Tables outside fenced code, and table-like lines everywhere.
    let mut tables = Tally::default();
    let mut everything = Tally::default();
    let mut fence: Option<Fence> = None;
    let mut previous: Option<Line> = None;
    // `previous`, unless that line was fenced code.
    let mut previous_outside: Option<Line> = None;
    let mut start = 0;
    for raw in text.split('\n') {
        let line = Line::classify(raw, start);
        start += raw.len() + 1;
        everything
            .feed(previous.as_ref(), &line, fenced_limit)
            .map_err(|header| Exceeded {
                header,
                fenced: true,
            })?;
        let fenced = match &fence {
            Some(open) if open.continues(raw) => {
                if open.closes(raw) {
                    fence = None;
                }
                true
            }
            Some(_) | None => {
                fence = Fence::open(raw);
                fence.is_some()
            }
        };
        if fenced {
            tables.end();
            previous_outside = None;
        } else {
            tables
                .feed(previous_outside.as_ref(), &line, limit)
                .map_err(|header| Exceeded {
                    header,
                    fenced: false,
                })?;
            previous_outside = Some(line);
        }
        previous = Some(line);
    }
    Ok(())
}

/// What the scan needs to know about one line.
#[derive(Debug, Clone, Copy)]
struct Line {
    /// Offset of the first byte after the leading spaces, tabs and `>`.
    content: usize,
    /// Only spaces, tabs and `>`: no table continues past such a line.
    blank: bool,
    /// The line holds a `|`, so it may be a header row.
    pipe: bool,
    /// The column count, if the line looks like a delimiter row.
    delimiter: Option<usize>,
}

impl Line {
    fn classify(raw: &str, start: usize) -> Self {
        let prefix = raw
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'\t' | b'>'))
            .count();
        let rest = raw.get(prefix..).unwrap_or_default();
        Self {
            content: start + prefix,
            blank: rest.is_empty(),
            pipe: rest.contains('|'),
            delimiter: delimiter_columns(rest),
        }
    }
}

/// The column count of a delimiter row (`|---|:-:|`), computed like pulldown-cmark's
/// `scan_table_head`, if `rest` (a line without its leading whitespace) looks like one. Accepts
/// a superset of what pulldown-cmark accepts (tabs, `||`), which only over-estimates.
fn delimiter_columns(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    if !bytes
        .iter()
        .all(|b| matches!(b, b' ' | b'\t' | b':' | b'-' | b'|'))
        || !bytes.contains(&b'-')
    {
        return None;
    }
    let pipes = rest.matches('|').count();
    if pipes == 0 {
        return None;
    }
    let trimmed = rest.trim_end_matches([' ', '\t']);
    let leading = usize::from(trimmed.starts_with('|'));
    let trailing = usize::from(!trimmed.ends_with('|'));
    Some((pipes - leading + trailing).max(1))
}

/// A running count of table cells.
#[derive(Debug, Default)]
struct Tally {
    cells: usize,
    /// Width of the current run of table rows; 0 outside tables.
    columns: usize,
    /// Offset of the current run's latest header row.
    header: usize,
}

impl Tally {
    /// Counts `line`, whose predecessor (in this tally) is `previous`. Fails with the offset of
    /// the current header row when the count exceeds `limit`.
    fn feed(&mut self, previous: Option<&Line>, line: &Line, limit: usize) -> Result<(), usize> {
        if line.blank {
            self.end();
            return Ok(());
        }
        let header = previous.filter(|p| p.pipe);
        match (line.delimiter, header) {
            (Some(columns), Some(header)) => {
                let open = self.columns > 0;
                self.columns = self.columns.max(columns);
                self.header = header.content;
                // The header row, counted again at the run's width.
                self.add(limit)?;
                if open {
                    // A delimiter-like row inside a table is a body row of that table.
                    self.add(limit)?;
                }
            }
            _ if self.columns > 0 => self.add(limit)?,
            _ => {}
        }
        Ok(())
    }

    fn add(&mut self, limit: usize) -> Result<(), usize> {
        self.cells = self.cells.saturating_add(self.columns);
        if self.cells > limit {
            Err(self.header)
        } else {
            Ok(())
        }
    }

    fn end(&mut self) {
        self.columns = 0;
    }
}

/// An open fenced code block, as far as a line-by-line scan can tell.
#[derive(Debug, Clone, Copy)]
struct Fence {
    ch: u8,
    len: usize,
    /// `>` before the opening fence: a line with fewer has left the quote.
    quotes: usize,
    /// Byte column of the opening fence when a list marker precedes it on its line: a non-blank
    /// line indented less has left the list item.
    item_column: Option<usize>,
}

impl Fence {
    /// The fence `raw` opens, if any: after quotes, list markers and indentation, three or more
    /// backticks (with no backtick in the info string) or tildes.
    fn open(raw: &str) -> Option<Self> {
        let bytes = raw.as_bytes();
        let mut at = 0;
        let mut quotes = 0;
        let mut marker = false;
        loop {
            while let Some(&b) = bytes.get(at) {
                match b {
                    b'>' => quotes += 1,
                    b' ' | b'\t' => {}
                    _ => break,
                }
                at += 1;
            }
            match list_marker(bytes.get(at..).unwrap_or_default()) {
                Some(len) => {
                    at += len;
                    marker = true;
                }
                None => break,
            }
        }
        let rest = bytes.get(at..)?;
        let ch = *rest.first()?;
        if ch != b'`' && ch != b'~' {
            return None;
        }
        let len = rest.iter().take_while(|&&b| b == ch).count();
        if len < 3 || (ch == b'`' && rest.get(len..).is_some_and(|info| info.contains(&b'`'))) {
            return None;
        }
        Some(Self {
            ch,
            len,
            quotes,
            item_column: marker.then_some(at),
        })
    }

    /// Whether `raw` is still inside the containers the fence was opened in.
    fn continues(&self, raw: &str) -> bool {
        let bytes = raw.as_bytes();
        let prefix = bytes
            .iter()
            .take_while(|b| matches!(b, b' ' | b'\t' | b'>'))
            .count();
        let quotes = raw.get(..prefix).map_or(0, |p| p.matches('>').count());
        let blank = prefix == bytes.len();
        quotes >= self.quotes && (blank || self.item_column.is_none_or(|column| prefix >= column))
    }

    /// Whether `raw` closes the fence.
    fn closes(&self, raw: &str) -> bool {
        let rest = raw.trim_start_matches([' ', '\t', '>']);
        let run = rest.bytes().take_while(|&b| b == self.ch).count();
        run >= self.len
            && rest
                .get(run..)
                .is_some_and(|tail| tail.trim_matches([' ', '\t']).is_empty())
    }
}

/// Length of a list marker and the whitespace after it (`- `, `* `, `+ `, `1. `, `1) `) at the
/// start of `bytes`, if any.
fn list_marker(bytes: &[u8]) -> Option<usize> {
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    let marker = match bytes.first()? {
        b'-' | b'*' | b'+' => 1,
        _ if (1..=9).contains(&digits) && matches!(bytes.get(digits), Some(b'.' | b')')) => {
            digits + 1
        }
        _ => return None,
    };
    let space = bytes
        .get(marker..)?
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count();
    (space > 0).then_some(marker + space)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::MAX_TABLE_CELLS;

    /// The smallest limit `passes` accepts, by bisection.
    fn smallest(passes: impl Fn(usize) -> bool) -> usize {
        let (mut low, mut high) = (0, 10_000_000);
        while low < high {
            let mid = usize::midpoint(low, high);
            if passes(mid) {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        low
    }

    /// The cells counted for the tables of `text`, outside fenced code.
    fn estimate(text: &str) -> usize {
        smallest(|limit| scan(text, limit, usize::MAX).is_ok())
    }

    /// The cells counted for the table-like lines of `text`, fenced code included.
    fn estimate_all(text: &str) -> usize {
        smallest(|limit| scan(text, usize::MAX, limit).is_ok())
    }

    fn table(columns: usize, body_rows: usize) -> String {
        let mut text = format!("|{}\n|{}\n", "h|".repeat(columns), "-|".repeat(columns));
        text.push_str(&"x\n".repeat(body_rows));
        text
    }

    #[test]
    fn counts_padded_cells() {
        assert_eq!(estimate("| a | b |\n|---|---|\n| 1 | 2 |\n| 3 |\n"), 6);
        assert_eq!(estimate("a | b\n-|-\n1\n"), 4);
        assert_eq!(estimate(&table(256, 1_000)), 256 * 1_001);
        // A table ends at a blank line.
        assert_eq!(estimate("|a|b|c|\n|-|-|-|\n\nx\ny\n"), 3);
        // No header row with a pipe, no delimiter row, or no table at all.
        assert_eq!(estimate("a\n---|---\nx\n"), 0);
        assert_eq!(estimate("| a |\n| b |\n"), 0);
        assert_eq!(estimate("# Title\n\n---\n\ntext\n"), 0);
    }

    #[test]
    fn delimiter_columns_match_pulldown_cmark() {
        assert_eq!(delimiter_columns("|---|---|"), Some(2));
        assert_eq!(delimiter_columns("---|---"), Some(2));
        assert_eq!(delimiter_columns("| :-: | --: |  "), Some(2));
        assert_eq!(delimiter_columns("|-"), Some(1));
        assert_eq!(delimiter_columns("-|"), Some(1));
        assert_eq!(delimiter_columns("---"), None);
        assert_eq!(delimiter_columns("|||"), None);
        assert_eq!(delimiter_columns("|-a-|"), None);
    }

    #[test]
    fn tables_in_quotes_and_lists_count() {
        assert_eq!(estimate("> | a | b |\n> |---|---|\n> | 1 |\n>\n"), 4);
        assert_eq!(estimate("> > |a|b|\n> > |-|-|\n> > x\n"), 4);
        assert_eq!(estimate("- | a | b |\n  |---|---|\n  | 1 |\n"), 4);
        assert_eq!(estimate("1. item\n\n   |a|b|c|\n   |-|-|-|\n   x\n"), 6);
    }

    #[test]
    fn fenced_code_is_ignored() {
        let code = format!("```md\n{}```\n", table(4, 9));
        assert_eq!(estimate(&code), 0);
        // The safety tally counts it, and the closing fence as one more row.
        assert_eq!(estimate_all(&code), 40 + 4);
        let code = format!("~~~~\n{}~~~\n~~~~~\n", table(4, 9));
        assert_eq!(estimate(&code), 0);
        assert_eq!(estimate("> ```\n> |a|b|\n> |-|-|\n> x\n> ```\n"), 0);
        assert_eq!(estimate("- ```\n  |a|b|\n  |-|-|\n  ```\n"), 0);
        // A table after the fence counts.
        let text = format!("```\n|a|\n|-|\n```\n{}", table(3, 1));
        assert_eq!(estimate(&text), 6);
        // A fence that the quote or list item closes ends with it.
        assert_eq!(estimate("> ```\n|a|b|\n|-|-|\nx\n"), 4);
        assert_eq!(estimate("- ```\n|a|b|\n|-|-|\nx\n"), 4);
    }

    #[test]
    fn fenced_tally_caps_what_the_fence_tracking_misses() {
        // An HTML block ends at the blank line, so the table is real; the line-based fence
        // tracking does not know that and takes it for fenced code.
        let text = format!("<div>\n```\n\n{}", table(10, 99));
        assert_eq!(
            check(&text, 99),
            Err(Exceeded {
                header: 11,
                fenced: true
            })
        );
        assert_eq!(check(&text, 100), Ok(()));
    }

    #[test]
    fn several_tables_add_up() {
        let text = format!("{}\ntext\n\n{}", table(3, 1), table(5, 2));
        assert_eq!(estimate(&text), 6 + 15);
        // The table that crosses the budget is reported, at its header row.
        let header = table(3, 1).len() + "\ntext\n\n".len();
        assert_eq!(
            check(&text, 20),
            Err(Exceeded {
                header,
                fenced: false
            })
        );
        // Adjacent tables without a blank line: the wider width and the header counted again.
        assert_eq!(
            estimate("|a|\n|-|\nx\n|a|b|\n|-|-|\nx\n"),
            1 + 1 + 1 + 2 + 2 + 2
        );
    }

    #[test]
    fn exactly_at_the_limit_is_allowed() {
        // 100 columns × (999 body rows + header) = 100,000 cells.
        let text = table(100, 999);
        assert_eq!(check(&text, MAX_TABLE_CELLS), Ok(()));
        let text = table(100, 1_000);
        assert_eq!(
            check(&text, MAX_TABLE_CELLS),
            Err(Exceeded {
                header: 0,
                fenced: false
            })
        );
    }

    #[test]
    fn list_markers() {
        assert_eq!(list_marker(b"- x"), Some(2));
        assert_eq!(list_marker(b"12)  x"), Some(5));
        assert_eq!(list_marker(b"-x"), None);
        assert_eq!(list_marker(b"1234567890. x"), None);
        assert_eq!(list_marker(b""), None);
    }
}
