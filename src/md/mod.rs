//! Markdown → ADF. Mapping and repair rules: `docs/syntax.md#markdown--adf`.
//!
//! OWNER: stream S1. The signature is frozen; implement the body (split into submodules as needed).

use crate::Conversion;
use crate::adf::model::Doc;
use crate::diagnostics::{Code, Diagnostic, Diagnostics};
use crate::options::MdToAdfOptions;

/// Converts Markdown source to a strict ADF document. Warnings carry line/column locations.
pub fn to_adf(source: &str, opts: &MdToAdfOptions) -> Result<Conversion<Doc>, Diagnostics> {
    let _ = (source, opts);
    Err(Diagnostics::single(Diagnostic::new(
        Code::Internal,
        "Markdown to ADF is not implemented yet",
    )))
}
