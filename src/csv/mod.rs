//! CSV and TSV tables: CSV → ADF table, and one ADF table → CSV. Contract: `docs/cli-contract.md`
//! (issue #13).
//!
//! - `parse`: the RFC 4180 reader (one linear pass) and line/column lookup;
//! - `text`, `numbers`: one field as cell content (hard breaks, links); numeric columns;
//! - `to_adf`: the records become the rows of one table, within the limits;
//! - `grid`, `cell_text`, `write`, `from_adf`: a table's `colspan`/`rowspan` expanded to a grid,
//!   each cell as plain text, quoting, and the writer.
//!
//! Cell text is never parsed as Markdown. No walk recurses without a bound: the reader checks the
//! JSON depth first, and everything else is iterative.

mod cell_text;
mod from_adf;
mod grid;
mod numbers;
mod parse;
mod text;
mod to_adf;
mod write;

use serde_json::Value;

use crate::Conversion;
use crate::adf::model::Doc;
use crate::diagnostics::Diagnostics;
use crate::options::{AdfToCsvOptions, CsvToAdfOptions};

/// Converts CSV text to an ADF document holding one table. Warnings carry line/column locations.
///
/// A leading BOM is ignored; CRLF, LF and lone CR end a record. The first record is the header
/// unless `opts.header` is off. Rows are padded to the widest record and cell text is never read
/// as Markdown; see `docs/cli-contract.md` for the limits and warnings.
pub fn to_adf(csv: &str, opts: &CsvToAdfOptions) -> Result<Conversion<Doc>, Diagnostics> {
    to_adf::convert(csv, opts)
}

/// Converts table number `opts.table` of a bare `doc` to CSV text. Warnings use JSON Pointers
/// relative to the document root.
///
/// A merged cell's text is repeated in every position it covers (`W_CSV_MERGED_CELLS`). Records
/// end with a line feed.
pub fn from_adf(doc: &Value, opts: &AdfToCsvOptions) -> Result<Conversion<String>, Diagnostics> {
    from_adf::convert(doc, opts)
}
