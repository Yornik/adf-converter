//! CSV and TSV tables: CSV → ADF table, and one ADF table → CSV. Contract: `docs/cli-contract.md`
//! (issue #13).
//!
//! OWNER: stream csv-core. The signatures are frozen; implement the bodies.

use serde_json::Value;

use crate::Conversion;
use crate::adf::model::Doc;
use crate::diagnostics::{Code, Diagnostic, Diagnostics};
use crate::options::{AdfToCsvOptions, CsvToAdfOptions};

/// Converts CSV text to an ADF document holding one table. Warnings carry line/column locations.
pub fn to_adf(csv: &str, opts: &CsvToAdfOptions) -> Result<Conversion<Doc>, Diagnostics> {
    let _ = (csv, opts);
    Err(Diagnostics::single(Diagnostic::new(
        Code::Internal,
        "CSV to ADF is not implemented yet",
    )))
}

/// Converts table number `opts.table` of a bare `doc` to CSV text. Warnings use JSON Pointers
/// relative to the document root.
pub fn from_adf(doc: &Value, opts: &AdfToCsvOptions) -> Result<Conversion<String>, Diagnostics> {
    let _ = (doc, opts);
    Err(Diagnostics::single(Diagnostic::new(
        Code::Internal,
        "ADF to CSV is not implemented yet",
    )))
}
