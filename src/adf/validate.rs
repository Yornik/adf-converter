//! Validation of ADF documents: the vendored JSON schema, the target's rules, and invariants that
//! Jira/Confluence enforce beyond the schema (see `docs/syntax.md#rules-the-converter-always-enforces`).
//!
//! OWNER: stream S3. The signature is frozen; implement the body.

use serde_json::Value;

use crate::diagnostics::Diagnostic;
use crate::options::Target;

/// Validates a bare `doc`. Returns errors (`E_SCHEMA`, `E_TARGET`, `E_INVARIANT`) and warnings
/// (`W_SIZE_LIMIT`, `W_EMPTY_DOCUMENT`), each located with a JSON Pointer relative to the
/// document root. An empty vector means the document is valid.
pub fn validate(doc: &Value, target: Target) -> Vec<Diagnostic> {
    let _ = (doc, target);
    Vec::new()
}
