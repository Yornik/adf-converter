//! ```` ```csv ```` and ```` ```tsv ```` fences: a fenced code block whose info string starts with
//! `csv` or `tsv` becomes one table (extension `csv`). The other words of the info string are
//! `key=value` options (`delimiter`, `header`, `max-rows`, `align`); the body is the CSV text.
//! Contract: `docs/syntax.md#inline-csv-tables`.

use std::io::{self, Write};

use crate::adf::model::Table;
use crate::csv::params::{CsvParams, embedded_hint, embedded_table, shift_lines};
use crate::diagnostics::{Code, Diagnostic};
use crate::options::CsvDelimiter;

use super::cx::Cx;
use super::extensions::truncate;
use super::ir::Span;

/// The two fence kinds: `tsv` only changes the default delimiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Csv,
    Tsv,
}

impl Kind {
    /// The fence kind named by the first word of `info`, if it is exactly `csv` or `tsv`.
    pub(super) fn of(info: Option<&str>) -> Option<Self> {
        match info?.split_whitespace().next()? {
            "csv" => Some(Self::Csv),
            "tsv" => Some(Self::Tsv),
            _ => None,
        }
    }

    /// The word that opens the fence.
    pub(super) fn word(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
        }
    }

    fn default_delimiter(self) -> Option<CsvDelimiter> {
        match self {
            Self::Csv => None,
            Self::Tsv => Some(CsvDelimiter::TAB),
        }
    }
}

/// Counts the bytes written to it and keeps none of them.
struct Counter(usize);

impl Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The span of the opening fence line, from the fence to the end of its line.
fn fence_line(cx: &Cx<'_>, span: &Span) -> Span {
    let rest = cx.text.get(span.start..).unwrap_or_default();
    let len = rest.find('\n').unwrap_or(rest.len());
    span.start..span.start + len
}

/// The span of the info string within the fence line (the line itself if it cannot be found).
fn info_span(cx: &Cx<'_>, span: &Span, info: &str) -> Span {
    let line = fence_line(cx, span);
    let text = cx.text.get(line.clone()).unwrap_or_default();
    let word = info.split_whitespace().next().unwrap_or_default();
    match text.find(word) {
        Some(at) if !word.is_empty() => line.start + at..line.end,
        _ => line,
    }
}

/// Parses the options of a fence: every word after the first is `key=value`.
fn params(info: &str) -> Result<CsvParams, String> {
    let mut pairs = Vec::new();
    for word in info.split_whitespace().skip(1) {
        let Some(pair) = word.split_once('=') else {
            let shown: String = truncate(word, 20).escape_debug().collect();
            return Err(format!(
                "'{shown}' is not a key=value option; expected delimiter, header, max-rows or align"
            ));
        };
        pairs.push(pair);
    }
    CsvParams::from_pairs(pairs)
}

/// Moves a diagnostic of the CSV conversion to Markdown lines; one without a location goes to the
/// opening fence.
fn anchor(diagnostic: &mut Diagnostic, first_line: u32, cx: &Cx<'_>, fence: &Span) {
    diagnostic.hint = diagnostic.hint.take().map(|hint| embedded_hint(&hint));
    if diagnostic.location.is_some() {
        shift_lines(diagnostic, first_line);
    } else {
        diagnostic.location = Some(cx.location(fence));
    }
}

/// Converts the body of a ```` ```csv ```` / ```` ```tsv ```` fence to a table. `span` covers the
/// whole code block. Warnings of the CSV conversion are added to `cx`. The table's cells are the
/// caller's to count against the document-wide limit.
pub(super) fn convert(
    cx: &mut Cx<'_>,
    kind: Kind,
    info: &str,
    body: &str,
    span: &Span,
) -> Result<Table, Diagnostic> {
    let parsed = params(info).map_err(|message| {
        cx.diag(
            Code::CsvOption,
            format!("```{} fence: {message}", kind.word()),
            &info_span(cx, span, info),
        )
        .with_hint(
            "Options are key=value words after the language: delimiter, header, max-rows, align",
        )
    })?;
    let opts = parsed.options(cx.opts.target, &cx.opts.limits, kind.default_delimiter());
    // The CSV's line 1 is the line after the opening fence.
    let first_line = cx.line_of(span.start).saturating_add(1);
    let fence = fence_line(cx, span);
    match embedded_table(body, &opts) {
        Ok(converted) => {
            for mut warning in converted.warnings {
                anchor(&mut warning, first_line, cx, &fence);
                cx.warnings.push(warning);
            }
            // The share of the document's output budget; a table always serialises.
            let mut counter = Counter(0);
            let _ = serde_json::to_writer(&mut counter, &converted.output);
            if !cx.charge(counter.0, span) {
                return Err(cx.diag(
                    Code::InputTooLarge,
                    "The tables and text of this document would make the ADF output too large",
                    &fence,
                ));
            }
            Ok(converted.output)
        }
        Err(errors) => {
            // The error comes first; the warnings found before it follow.
            let mut diagnostics = errors.0.into_iter();
            let mut error = diagnostics.next().unwrap_or_else(|| {
                Diagnostic::new(Code::Internal, "CSV conversion failed without a diagnostic")
            });
            for mut warning in diagnostics {
                anchor(&mut warning, first_line, cx, &fence);
                cx.warnings.push(warning);
            }
            anchor(&mut error, first_line, cx, &fence);
            // Positions inside the message count from the CSV's own first line.
            error.message = format!("In the ```{} block: {}", kind.word(), error.message);
            Err(error)
        }
    }
}
