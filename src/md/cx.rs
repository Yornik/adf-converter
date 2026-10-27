//! State shared by every stage of one Markdown → ADF conversion.

use crate::adf::ids::LocalIdGen;
use crate::diagnostics::{Code, Diagnostic, Location};
use crate::limits::MAX_OUTPUT_BYTES;
use crate::options::MdToAdfOptions;

use super::include::Includes;
use super::ir::Span;

/// Internal hard cap on container nesting, whatever `Limits::max_depth` says (the default is
/// 64). The lowering pass and serialisation recurse once per container level; 100 levels fit
/// comfortably in a 2 MiB thread stack even in debug builds, so a caller that configures a
/// huge limit cannot cause a stack overflow.
pub(crate) const HARD_MAX_DEPTH: usize = 100;

/// Bytes between character-count checkpoints in [`Locator`].
const CHECKPOINT: usize = 256;

/// Maps byte offsets to 1-based line and column numbers with exactly the semantics of
/// [`LineIndex`](crate::diagnostics::LineIndex) (columns count Unicode scalar values, offsets
/// inside a UTF-8 sequence round down, offsets past the end clamp), but in constant time per
/// query: `LineIndex` counts characters from the start of the line, which is quadratic for
/// many diagnostics on one very long line.
#[derive(Debug, Clone)]
pub(crate) struct Locator<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
    /// `(byte offset, characters before it)` at char boundaries roughly every
    /// [`CHECKPOINT`] bytes, starting with `(0, 0)`.
    checkpoints: Vec<(usize, usize)>,
}

impl<'a> Locator<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        let mut checkpoints = vec![(0, 0)];
        let mut last = 0;
        for (chars, (offset, _)) in text.char_indices().enumerate() {
            if offset >= last + CHECKPOINT {
                checkpoints.push((offset, chars));
                last = offset;
            }
        }
        Self {
            text,
            line_starts,
            checkpoints,
        }
    }

    fn floor_boundary(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    /// Characters before the char boundary `offset`.
    fn chars_before(&self, offset: usize) -> usize {
        let index = self
            .checkpoints
            .partition_point(|&(at, _)| at <= offset)
            .saturating_sub(1);
        let (at, count) = self.checkpoints.get(index).copied().unwrap_or((0, 0));
        count + self.text.get(at..offset).map_or(0, |s| s.chars().count())
    }

    /// 1-based `(line, column)`.
    pub(crate) fn line_col(&self, offset: usize) -> (u32, u32) {
        let offset = self.floor_boundary(offset);
        let line = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let start = self.line_starts.get(line).copied().unwrap_or(0);
        let column = self.chars_before(offset) - self.chars_before(start);
        (to_u32(line + 1), to_u32(column + 1))
    }

    /// A location covering `span`.
    pub(crate) fn location(&self, span: &Span) -> Location {
        let (line, column) = self.line_col(span.start);
        let (end_line, end_column) = self.line_col(span.end);
        Location {
            line: Some(line),
            column: Some(column),
            end_line: Some(end_line),
            end_column: Some(end_column),
            ..Location::default()
        }
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Conversion context: options, the normalised source, collected warnings.
#[derive(Debug)]
pub(crate) struct Cx<'a> {
    pub(crate) opts: &'a MdToAdfOptions,
    /// The normalised source (see `prepass.rs`); spans index into it.
    pub(crate) text: &'a str,
    locator: Locator<'a>,
    pub(crate) warnings: Vec<Diagnostic>,
    pub(crate) ids: LocalIdGen,
    /// Estimated size of the compact JSON output so far (see [`Cx::charge`]).
    output_estimate: usize,
    /// Where the output first exceeded [`MAX_OUTPUT_BYTES`], if it did.
    pub(crate) over_budget: Option<Span>,
    /// Table cells so far (GFM tables, padding included, and included CSV tables), against
    /// [`MAX_TABLE_CELLS`](crate::limits::MAX_TABLE_CELLS).
    pub(crate) table_cells: usize,
    /// The files read by `adf:csv` includes.
    pub(crate) includes: Includes,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(text: &'a str, opts: &'a MdToAdfOptions) -> Self {
        Self {
            opts,
            text,
            locator: Locator::new(text),
            warnings: Vec::new(),
            ids: LocalIdGen::new(text),
            output_estimate: 0,
            over_budget: None,
            table_cells: 0,
            includes: Includes::default(),
        }
    }

    /// Adds `bytes` of estimated output for content that came from `span`. Returns `false` once
    /// the estimate exceeds [`MAX_OUTPUT_BYTES`]; callers then stop producing content and the
    /// conversion fails with `E_INPUT_TOO_LARGE`.
    pub(crate) fn charge(&mut self, bytes: usize, span: &Span) -> bool {
        self.output_estimate = self.output_estimate.saturating_add(bytes);
        if self.output_estimate > MAX_OUTPUT_BYTES {
            if self.over_budget.is_none() {
                self.over_budget = Some(span.clone());
            }
            return false;
        }
        true
    }

    /// The effective maximum container depth.
    pub(crate) fn max_depth(&self) -> usize {
        self.opts.limits.max_depth.min(HARD_MAX_DEPTH)
    }

    /// The 1-based line of byte `offset`.
    pub(crate) fn line_of(&self, offset: usize) -> u32 {
        self.locator.line_col(offset).0
    }

    /// The location covering `span`.
    pub(crate) fn location(&self, span: &Span) -> Location {
        self.locator.location(span)
    }

    /// A diagnostic located at `span`.
    pub(crate) fn diag(&self, code: Code, message: impl Into<String>, span: &Span) -> Diagnostic {
        Diagnostic::new(code, message).at(self.locator.location(span))
    }

    /// Records a warning located at `span`.
    pub(crate) fn warn(&mut self, code: Code, message: impl Into<String>, span: &Span) {
        let diagnostic = self.diag(code, message, span);
        self.warnings.push(diagnostic);
    }

    /// Records a warning with a hint.
    pub(crate) fn warn_hint(
        &mut self,
        code: Code,
        message: impl Into<String>,
        hint: impl Into<String>,
        span: &Span,
    ) {
        let diagnostic = self.diag(code, message, span).with_hint(hint);
        self.warnings.push(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::LineIndex;

    #[test]
    fn locator_matches_line_index() {
        let long_line = "é😀a".repeat(400);
        let texts = [
            "",
            "ab\ncé\n\nx",
            "\n\n\n",
            long_line.as_str(),
            "mixed\nline with ü and 😀\r\nend",
        ];
        for text in texts {
            let index = LineIndex::new(text);
            let locator = Locator::new(text);
            for offset in 0..=text.len() + 2 {
                assert_eq!(
                    locator.line_col(offset),
                    index.line_col(offset),
                    "{text:?} @ {offset}"
                );
            }
            let span = 1..text.len();
            assert_eq!(locator.location(&span), index.location(span.clone()));
        }
    }
}
