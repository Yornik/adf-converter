//! API envelopes around an ADF document (`-e/--envelope`). See `docs/cli-contract.md#envelopes`.
//!
//! Wrapping is used by Markdown → ADF; unwrapping finds the document inside whatever the user
//! piped in for ADF → Markdown and `validate` (our own envelopes, Jira `GET` responses, Confluence
//! bodies).

use serde_json::{Map, Value};

use crate::diagnostics::{Code, Diagnostic, Location};
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

/// The CLI names accepted by `-e/--envelope`, in documentation order.
pub const ENVELOPE_NAMES: &[&str] = &[
    "doc",
    "jira-comment",
    "jira-description",
    "jira-field",
    "confluence",
];

/// The `fields` entry used when `--field` is not given.
const DEFAULT_FIELD: &str = "description";

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

    /// Parses a CLI name; `jira-field` requires `field`, and `field` is only accepted with
    /// `jira-field` (anything else would silently ignore it).
    pub fn parse(name: &str, field: Option<&str>) -> Result<Self, String> {
        let envelope = match name {
            "doc" => Self::Doc,
            "jira-comment" => Self::JiraComment,
            "jira-description" => Self::JiraDescription,
            "confluence" => Self::Confluence,
            "jira-field" => {
                let Some(field) = field else {
                    return Err(
                        "-e jira-field requires --field NAME (for example --field customfield_10010)"
                            .to_owned(),
                    );
                };
                check_field_name(field)?;
                return Ok(Self::JiraField(field.to_owned()));
            }
            other => {
                return Err(format!(
                    "unknown envelope '{other}'; expected one of: {}",
                    ENVELOPE_NAMES.join(", ")
                ));
            }
        };
        if field.is_some() {
            return Err(format!(
                "--field is only used with -e jira-field (got -e {})",
                envelope.name()
            ));
        }
        Ok(envelope)
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

/// Checks a `--field` name: non-empty and free of control characters.
pub(crate) fn check_field_name(field: &str) -> Result<(), String> {
    if field.trim().is_empty() {
        return Err("--field must not be empty".to_owned());
    }
    if field.chars().any(char::is_control) {
        return Err("--field must not contain control characters".to_owned());
    }
    Ok(())
}

/// Wraps a bare document.
///
/// For [`Envelope::Confluence`] the `value` is the compact JSON text of the document, which is
/// what the Confluence v2 API expects (an object there is rejected with HTTP 400).
pub fn wrap(doc: Value, envelope: &Envelope) -> Value {
    match envelope {
        Envelope::Doc => doc,
        Envelope::JiraComment => object([("body", doc)]),
        Envelope::JiraDescription => object([("fields", object([(DEFAULT_FIELD, doc)]))]),
        Envelope::JiraField(name) => {
            let mut fields = Map::new();
            fields.insert(name.clone(), doc);
            object([("fields", Value::Object(fields))])
        }
        Envelope::Confluence => object([
            ("representation", Value::from("atlas_doc_format")),
            // `Value`'s `Display` is compact JSON and cannot fail.
            ("value", Value::from(doc.to_string())),
        ]),
    }
}

fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
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
/// `field` selects the `fields` entry (default `description`). Fails with `E_INPUT_NOT_ADF`
/// (or `E_INPUT_JSON` / `E_INPUT_TOO_DEEP` when a Confluence `value` string is not valid JSON).
pub fn unwrap(input: &Value, field: Option<&str>) -> Result<Unwrapped, Diagnostic> {
    let Some(object) = input.as_object() else {
        return Err(not_adf(format!(
            "expected an ADF document or an envelope object, found {}",
            json_kind(input)
        )));
    };

    // 1. A bare document.
    if is_doc(input) {
        return Ok(found(input.clone(), Envelope::Doc, String::new()));
    }

    // 2. A Jira comment: `{"body": DOC}` (also a `GET …/comment/{id}` response).
    if let Some(body) = object.get("body").filter(|body| is_doc(body)) {
        return Ok(found(
            body.clone(),
            Envelope::JiraComment,
            "/body".to_owned(),
        ));
    }

    // 3. Jira issue fields: `{"fields": {...}}` (also a `GET /issue/{key}` response).
    if let Some(fields) = object.get("fields").and_then(Value::as_object) {
        let name = field.unwrap_or(DEFAULT_FIELD);
        let pointer = format!("/fields/{}", escape_pointer_token(name));
        let envelope = if name == DEFAULT_FIELD {
            Envelope::JiraDescription
        } else {
            Envelope::JiraField(name.to_owned())
        };
        return match fields.get(name) {
            Some(doc) if is_doc(doc) => Ok(found(doc.clone(), envelope, pointer)),
            Some(Value::Null) => Err(not_adf(format!(
                "fields.{name} is null (the issue has no {name})"
            ))
            .at(Location::pointer(pointer))
            .with_hint(field_hint(fields))),
            Some(other) => Err(not_adf(format!(
                "fields.{name} is {}, not an ADF document",
                json_kind(other)
            ))
            .at(Location::pointer(pointer))
            .with_hint(
                "Rich-text fields hold ADF only in the Jira REST API v3; use --field to pick another field",
            )),
            None => Err(not_adf(format!("the input has no fields.{name}"))
                .at(Location::pointer("/fields"))
                .with_hint(field_hint(fields))),
        };
    }

    // 4. A Confluence body: `{"representation": "atlas_doc_format", "value": "…"}`.
    if object.get("representation").and_then(Value::as_str) == Some("atlas_doc_format")
        && let Some(value) = object.get("value")
    {
        return confluence_value(value, "/value");
    }

    // 5. A Confluence `GET` response: `{"body": {"atlas_doc_format": {"value": "…"}}}`.
    if let Some(value) = object
        .get("body")
        .and_then(|body| body.get("atlas_doc_format"))
        .and_then(|adf| adf.get("value"))
    {
        return confluence_value(value, "/body/atlas_doc_format/value");
    }

    Err(not_adf(describe_object(object)))
}

/// Unwraps a Confluence `value`, normally a JSON string holding the document.
fn confluence_value(value: &Value, pointer: &str) -> Result<Unwrapped, Diagnostic> {
    let doc = if let Value::String(text) = value {
        let max_depth = crate::Limits::default().max_json_depth();
        crate::json::parse_bounded(text, max_depth).map_err(|err| match err {
            crate::json::JsonError::TooDeep { .. } => Diagnostic::new(
                Code::InputTooDeep,
                format!("the Confluence value string nests deeper than {max_depth} JSON levels"),
            )
            .at(Location::pointer(pointer)),
            crate::json::JsonError::Syntax(err) => Diagnostic::new(
                Code::InputJson,
                format!(
                    "the Confluence value string is not valid JSON: {}",
                    strip_position(&err.to_string())
                ),
            )
            .at(Location::pointer(pointer)),
        })?
    } else {
        // Lenient: accept an already-decoded document as well.
        value.clone()
    };
    if is_doc(&doc) {
        Ok(found(doc, Envelope::Confluence, pointer.to_owned()))
    } else {
        Err(not_adf(format!(
            "the Confluence value holds {}, not an ADF document",
            json_kind(&doc)
        ))
        .at(Location::pointer(pointer)))
    }
}

fn found(doc: Value, envelope: Envelope, pointer: String) -> Unwrapped {
    Unwrapped {
        doc,
        envelope,
        pointer,
    }
}

fn is_doc(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("doc")
}

fn not_adf(message: String) -> Diagnostic {
    Diagnostic::new(Code::InputNotAdf, message).with_hint(
        "Pass a document ({\"type\":\"doc\",…}) or an envelope listed by `adf explain envelopes`",
    )
}

fn field_hint(fields: &Map<String, Value>) -> String {
    let docs: Vec<&str> = fields
        .iter()
        .filter(|(_, value)| is_doc(value))
        .map(|(name, _)| name.as_str())
        .take(5)
        .collect();
    if docs.is_empty() {
        "None of the fields holds an ADF document".to_owned()
    } else {
        format!(
            "Fields holding ADF: {}; select one with --field NAME",
            docs.join(", ")
        )
    }
}

/// Describes an unrecognised object without echoing its values.
fn describe_object(object: &Map<String, Value>) -> String {
    if object.is_empty() {
        return "expected an ADF document or a known envelope, found an empty object".to_owned();
    }
    if let Some(ty) = object.get("type").and_then(Value::as_str) {
        return format!(
            "expected an ADF document or a known envelope, found a node of type {}",
            preview_str(ty)
        );
    }
    let mut keys: Vec<String> = object.keys().take(6).map(|key| preview_str(key)).collect();
    if object.len() > keys.len() {
        keys.push("…".to_owned());
    }
    format!(
        "expected an ADF document or a known envelope, found an object with keys {}",
        keys.join(", ")
    )
}

/// A short, escaped rendering of input text for messages (never more than 40 characters).
fn preview_str(text: &str) -> String {
    let mut short: String = text.chars().take(40).collect();
    if short.len() < text.len() {
        short.push('…');
    }
    Value::from(short).to_string()
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Escapes one JSON Pointer reference token (RFC 6901).
pub(crate) fn escape_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// Removes the ` at line L column C` suffix of `serde_json` errors (the location is reported
/// separately).
pub(crate) fn strip_position(message: &str) -> &str {
    match message.rfind(" at line ") {
        Some(index) => message.get(..index).unwrap_or(message),
        None => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Value {
        json!({"type":"doc","version":1,"content":[{"type":"paragraph"}]})
    }

    #[test]
    fn parse_names() {
        assert_eq!(Envelope::parse("doc", None), Ok(Envelope::Doc));
        assert_eq!(
            Envelope::parse("jira-comment", None),
            Ok(Envelope::JiraComment)
        );
        assert_eq!(
            Envelope::parse("jira-description", None),
            Ok(Envelope::JiraDescription)
        );
        assert_eq!(
            Envelope::parse("confluence", None),
            Ok(Envelope::Confluence)
        );
        assert_eq!(
            Envelope::parse("jira-field", Some("customfield_10010")),
            Ok(Envelope::JiraField("customfield_10010".into()))
        );
        for name in ENVELOPE_NAMES {
            let field = (*name == "jira-field").then_some("x");
            assert_eq!(Envelope::parse(name, field).unwrap().name(), *name);
        }
    }

    #[test]
    fn parse_errors() {
        assert!(
            Envelope::parse("jira-field", None)
                .unwrap_err()
                .contains("--field")
        );
        assert!(Envelope::parse("jira-field", Some(" ")).is_err());
        assert!(Envelope::parse("jira-field", Some("a\nb")).is_err());
        assert!(Envelope::parse("jira-comment", Some("x")).is_err());
        assert!(
            Envelope::parse("body", None)
                .unwrap_err()
                .contains("doc, jira-comment")
        );
    }

    #[test]
    fn default_targets() {
        assert_eq!(Envelope::Confluence.default_target(), Target::Confluence);
        assert_eq!(Envelope::Doc.default_target(), Target::Jira);
        assert_eq!(
            Envelope::JiraField("x".into()).default_target(),
            Target::Jira
        );
    }

    #[test]
    fn wrap_shapes() {
        assert_eq!(wrap(doc(), &Envelope::Doc), doc());
        assert_eq!(wrap(doc(), &Envelope::JiraComment), json!({"body": doc()}));
        assert_eq!(
            wrap(doc(), &Envelope::JiraDescription),
            json!({"fields": {"description": doc()}})
        );
        assert_eq!(
            wrap(doc(), &Envelope::JiraField("customfield_1".into())),
            json!({"fields": {"customfield_1": doc()}})
        );
        let confluence = wrap(doc(), &Envelope::Confluence);
        assert_eq!(
            serde_json::to_string(&confluence).unwrap(),
            r#"{"representation":"atlas_doc_format","value":"{\"type\":\"doc\",\"version\":1,\"content\":[{\"type\":\"paragraph\"}]}"}"#
        );
    }

    #[test]
    fn wrap_then_unwrap_round_trips() {
        for envelope in [
            Envelope::Doc,
            Envelope::JiraComment,
            Envelope::JiraDescription,
            Envelope::JiraField("customfield_10010".into()),
            Envelope::Confluence,
        ] {
            let field = if let Envelope::JiraField(name) = &envelope {
                Some(name.as_str())
            } else {
                None
            };
            let unwrapped = unwrap(&wrap(doc(), &envelope), field).unwrap();
            assert_eq!(unwrapped.doc, doc(), "{envelope:?}");
            assert_eq!(unwrapped.envelope, envelope);
        }
    }

    #[test]
    fn unwrap_pointers() {
        let cases = [
            (doc(), None, ""),
            (json!({"id":"1","body": doc()}), None, "/body"),
            (
                json!({"key":"A-1","fields": {"summary":"s","description": doc()}}),
                None,
                "/fields/description",
            ),
            (
                json!({"fields": {"customfield_10010": doc()}}),
                Some("customfield_10010"),
                "/fields/customfield_10010",
            ),
            (
                json!({"fields": {"a/b~c": doc()}}),
                Some("a/b~c"),
                "/fields/a~1b~0c",
            ),
            (
                json!({"representation":"atlas_doc_format","value": doc().to_string()}),
                None,
                "/value",
            ),
            (
                json!({"id":"9","body":{"atlas_doc_format":{"value": doc().to_string(),"representation":"atlas_doc_format"}}}),
                None,
                "/body/atlas_doc_format/value",
            ),
        ];
        for (input, field, pointer) in cases {
            let unwrapped = unwrap(&input, field).unwrap();
            assert_eq!(unwrapped.pointer, pointer, "{input}");
            assert_eq!(unwrapped.doc, doc());
        }
    }

    #[test]
    fn detection_order_prefers_bare_doc_then_body() {
        // A document that happens to have a `body` key is still a document.
        let odd = json!({"type":"doc","version":1,"content":[],"body": doc()});
        assert_eq!(unwrap(&odd, None).unwrap().pointer, "");
        // `body` wins over `fields`.
        let both = json!({"body": doc(), "fields": {"description": doc()}});
        assert_eq!(unwrap(&both, None).unwrap().envelope, Envelope::JiraComment);
    }

    #[test]
    fn unwrap_failures() {
        let not_adf = |input: Value, field: Option<&str>| {
            let err = unwrap(&input, field).unwrap_err();
            (err.code, err.location.and_then(|l| l.pointer))
        };
        assert_eq!(not_adf(json!([1]), None).0, Code::InputNotAdf);
        assert_eq!(not_adf(json!("doc"), None).0, Code::InputNotAdf);
        assert_eq!(
            not_adf(json!({"type":"paragraph"}), None).0,
            Code::InputNotAdf
        );
        assert_eq!(
            not_adf(json!({"fields": {"description": null}}), None),
            (Code::InputNotAdf, Some("/fields/description".into()))
        );
        assert_eq!(
            not_adf(json!({"fields": {"description": "wiki *markup*"}}), None),
            (Code::InputNotAdf, Some("/fields/description".into()))
        );
        assert_eq!(
            not_adf(
                json!({"fields": {"description": doc()}}),
                Some("environment")
            ),
            (Code::InputNotAdf, Some("/fields".into()))
        );
        assert_eq!(
            not_adf(
                json!({"representation":"atlas_doc_format","value":"{not json"}),
                None
            ),
            (Code::InputJson, Some("/value".into()))
        );
        assert_eq!(
            not_adf(
                json!({"representation":"atlas_doc_format","value":"[1]"}),
                None
            ),
            (Code::InputNotAdf, Some("/value".into()))
        );
        let deep = format!("{}{}", "[".repeat(600), "]".repeat(600));
        assert_eq!(
            not_adf(
                json!({"representation":"atlas_doc_format","value": deep}),
                None
            )
            .0,
            Code::InputTooDeep
        );
    }

    #[test]
    fn not_adf_messages_do_not_echo_values() {
        let secret = "s".repeat(500);
        let err = unwrap(&json!({"token": secret.clone()}), None).unwrap_err();
        assert!(!err.message.contains(&secret));
        assert!(err.message.contains("\"token\""), "{}", err.message);
        let err = unwrap(
            &json!({"fields": {"description": doc(), "x": doc()}}),
            Some("y"),
        )
        .unwrap_err();
        assert_eq!(
            err.hint.as_deref(),
            Some("Fields holding ADF: description, x; select one with --field NAME")
        );
    }

    #[test]
    fn strip_serde_position() {
        assert_eq!(
            strip_position("EOF while parsing an object at line 1 column 5"),
            "EOF while parsing an object"
        );
        assert_eq!(strip_position("no position"), "no position");
    }
}
