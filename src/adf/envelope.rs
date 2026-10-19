//! API envelopes around an ADF document (`-e/--envelope`). See `docs/cli-contract.md#envelopes`.
//!
//! OWNER: stream S3 (CLI). Signatures are frozen; implement the bodies.

use serde_json::Value;

use crate::diagnostics::{Code, Diagnostic};
use crate::options::Target;

/// How the document is wrapped for the API being called.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Envelope {
    /// The bare document.
    #[default]
    Doc,
    /// `{"body": DOC}`.
    JiraComment,
    /// `{"fields": {"description": DOC}}`.
    JiraDescription,
    /// `{"fields": {"<NAME>": DOC}}`.
    JiraField(String),
    /// `{"representation": "atlas_doc_format", "value": "<DOC as a JSON string>"}`.
    Confluence,
}

impl Envelope {
    /// The CLI name (`doc`, `jira-comment`, …).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Doc => "doc",
            Self::JiraComment => "jira-comment",
            Self::JiraDescription => "jira-description",
            Self::JiraField(_) => "jira-field",
            Self::Confluence => "confluence",
        }
    }

    /// Parses a CLI name; `jira-field` requires `field`.
    pub fn parse(name: &str, field: Option<&str>) -> Result<Self, String> {
        let _ = (name, field);
        Err("not implemented".to_owned())
    }

    /// The target implied by `--target auto`.
    pub fn default_target(&self) -> Target {
        match self {
            Self::Confluence => Target::Confluence,
            Self::Doc | Self::JiraComment | Self::JiraDescription | Self::JiraField(_) => {
                Target::Jira
            }
        }
    }
}

/// Wraps a bare document.
pub fn wrap(doc: Value, envelope: &Envelope) -> Value {
    let _ = envelope;
    doc
}

/// A document found inside an input value.
#[derive(Debug, Clone, PartialEq)]
pub struct Unwrapped {
    /// The bare `doc` value.
    pub doc: Value,
    /// The envelope that was detected.
    pub envelope: Envelope,
    /// JSON Pointer of the document inside the input (`""`, `"/body"`, `"/fields/description"`,
    /// `"/value"` for string-encoded Confluence bodies). Prefix diagnostics with it.
    pub pointer: String,
}

/// Finds the ADF document inside `input` (detection order in `docs/cli-contract.md`).
/// `field` selects the `fields` entry (default `description`). Fails with `E_INPUT_NOT_ADF`.
pub fn unwrap(input: &Value, field: Option<&str>) -> Result<Unwrapped, Diagnostic> {
    let _ = (input, field);
    Err(Diagnostic::new(
        Code::Internal,
        "envelope::unwrap is not implemented yet",
    ))
}
