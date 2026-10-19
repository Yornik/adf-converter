//! ADF → Markdown (`adf convert --from adf`). Mapping and round-trip rules: `docs/syntax.md#adf--markdown`.
//!
//! OWNER: stream S2. The signature is frozen; implement the body.

use serde_json::Value;

use crate::Conversion;
use crate::diagnostics::{Code, Diagnostic, Diagnostics};
use crate::options::AdfToMdOptions;

/// Converts a bare `doc` to Markdown. Warnings use pointers relative to the document root.
pub fn to_markdown(doc: &Value, opts: &AdfToMdOptions) -> Result<Conversion<String>, Diagnostics> {
    let _ = (doc, opts);
    Err(Diagnostics::single(Diagnostic::new(
        Code::Internal,
        "ADF to Markdown is not implemented yet",
    )))
}
