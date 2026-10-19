//! `adf-converter`: convert Markdown to Atlassian Document Format (ADF) JSON and back.
//!
//! Independent, free-time open-source project; not affiliated with, endorsed by, or supported by
//! Atlassian. Atlassian, Jira and Confluence are trademarks of Atlassian.
//!
//! The library is the engine behind the `adf` CLI. The stable contract is the CLI
//! (`docs/cli-contract.md`); the Rust API may change between minor versions.
//!
//! ```
//! use adf_converter::{md_to_adf, MdToAdfOptions};
//!
//! let conversion = md_to_adf("# Hello\n\n- [ ] ship it\n", &MdToAdfOptions::default()).unwrap();
//! let json = conversion.output.to_value().unwrap();
//! assert_eq!(json["type"], "doc");
//! ```

#![forbid(unsafe_code)]

pub mod adf;
pub mod catalog;
pub mod cli;
pub mod diagnostics;
pub mod limits;
pub mod md;
pub mod options;

pub use diagnostics::{Code, Diagnostic, Diagnostics, LineIndex, Location, Severity};
pub use limits::Limits;
pub use options::{
    AdfToMdOptions, Extensions, HtmlPolicy, ImagePolicy, MdFlavor, MdToAdfOptions, SoftBreak,
    Target,
};

/// The version of the vendored ADF JSON schema, e.g. `@atlaskit/adf-schema@57.7.18`.
pub const ADF_SCHEMA_VERSION: &str = concat!(
    "@atlaskit/adf-schema@",
    include_str!("../schemas/adf/VERSION")
);

/// The vendored ADF JSON schema (`schemas/adf/full.json`, draft-04).
pub const ADF_SCHEMA_JSON: &str = include_str!("../schemas/adf/full.json");

/// A successful conversion: the output plus any warnings (lossy or repaired constructs).
#[derive(Debug, Clone, PartialEq)]
pub struct Conversion<T> {
    pub output: T,
    pub warnings: Vec<Diagnostic>,
}

/// Converts Markdown to an ADF document.
///
/// The result is guaranteed to be valid ADF for `opts.target` unless a ```` ```adf ```` block
/// was spliced in, which is checked separately (`E_RAW_ADF`). On failure the returned
/// [`Diagnostics`] contain at least one error.
pub fn md_to_adf(
    markdown: &str,
    opts: &MdToAdfOptions,
) -> Result<Conversion<adf::model::Doc>, Diagnostics> {
    md::to_adf(markdown, opts)
}

/// Converts an ADF document (a bare `doc` JSON value) to Markdown.
///
/// Use [`adf::envelope::unwrap`] first if the input may be wrapped in an API envelope.
pub fn adf_to_md(
    doc: &serde_json::Value,
    opts: &AdfToMdOptions,
) -> Result<Conversion<String>, Diagnostics> {
    adf::to_md::to_markdown(doc, opts)
}

/// Validates an ADF document (a bare `doc`) against the schema, the target's rules and the
/// invariants. Returns errors (`E_SCHEMA`, `E_TARGET`, `E_INVARIANT`) and warnings
/// (`W_SIZE_LIMIT`, `W_EMPTY_DOCUMENT`); locations are JSON Pointers relative to the document.
pub fn validate_adf(doc: &serde_json::Value, target: Target) -> Vec<Diagnostic> {
    adf::validate::validate(doc, target)
}
