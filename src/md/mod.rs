//! Markdown → ADF. Mapping and repair rules: `docs/syntax.md#markdown--adf`.
//!
//! OWNER: stream S1. The signature is frozen; the work is split into stages:
//!
//! 1. `prepass`: BOM, line endings, front matter, `::: cell` boxes;
//! 2. `tree`: pulldown-cmark events → an intermediate block tree (`ir`), with inline content
//!    handled by `inline` (marks, links, `adf:` extensions, HTML tags, autolinks);
//! 3. `lower`: container rules (demote, flatten, convert, hoist) while building the strict
//!    model of [`crate::adf::model`]; inline tokens are finalised by `finalize`.
//!
//! No stage recurses without a depth bound: the tree builder is an explicit stack machine that
//! enforces `Limits::max_depth` (capped at 100 internally), and the lowering pass only recurses
//! along that bounded tree.

mod cx;
mod extensions;
mod finalize;
mod html;
mod inline;
mod ir;
mod lower;
mod prepass;
mod tree;

#[cfg(test)]
mod tests;

use crate::Conversion;
use crate::adf::model::Doc;
use crate::diagnostics::{Code, Diagnostic, Diagnostics};
use crate::options::MdToAdfOptions;

use cx::Cx;

/// Converts Markdown source to a strict ADF document. Warnings carry line/column locations.
pub fn to_adf(source: &str, opts: &MdToAdfOptions) -> Result<Conversion<Doc>, Diagnostics> {
    let limit = opts.limits.max_input_bytes;
    if source.len() > limit {
        return Err(Diagnostics::single(
            Diagnostic::new(
                Code::InputTooLarge,
                format!(
                    "The input is {} bytes, larger than the {limit}-byte limit",
                    source.len()
                ),
            )
            .with_hint("Split the document into smaller parts"),
        ));
    }
    let mut pre = prepass::run(source, opts.extensions.cells);
    if !tree::parses_completely(&pre.text, opts) {
        // Work around a pulldown-cmark bug triggered by indented blank lines (see
        // `tree::parses_completely`); blank lines then carry no whitespace.
        pre = prepass::run_blanking_whitespace_lines(source, opts.extensions.cells);
        if !tree::parses_completely(&pre.text, opts) {
            return Err(Diagnostics::single(Diagnostic::new(
                Code::Internal,
                "The Markdown parser could not process this input; please report it",
            )));
        }
    }
    let mut cx = Cx::new(&pre.text, opts);
    for (code, message, span) in &pre.warnings {
        cx.warn(*code, message.clone(), span);
    }
    let result = tree::build(&mut cx, &pre).and_then(|parsed| lower::lower(parsed, &mut cx));
    let mut warnings = cx.warnings;
    // Source order; the sort is stable, so warnings at one position keep their order.
    warnings.sort_by_key(|d| {
        d.location
            .as_ref()
            .map_or((0, 0), |l| (l.line.unwrap_or(0), l.column.unwrap_or(0)))
    });
    match result {
        Ok(output) => Ok(Conversion { output, warnings }),
        Err(error) => {
            let mut all = Vec::with_capacity(warnings.len() + 1);
            all.push(error);
            all.extend(warnings);
            Err(Diagnostics(all))
        }
    }
}
