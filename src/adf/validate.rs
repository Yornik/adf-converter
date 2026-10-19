//! Validation of ADF documents: the vendored JSON schema, the target's rules, and invariants that
//! Jira/Confluence enforce beyond the schema (see `docs/syntax.md#rules-the-converter-always-enforces`).
//!
//! Three layers, all located with JSON Pointers relative to the document root:
//!
//! 1. **Schema** (`E_SCHEMA`): the vendored `@atlaskit/adf-schema` draft-04 schema. Failures are
//!    explained node by node (see the `schema` submodule) instead of as raw `anyOf` noise.
//! 2. **Target** (`E_TARGET`): nodes, marks and attributes the product does not accept
//!    ([`crate::adf::targets`]).
//! 3. **Invariants** (`E_INVARIANT`): list items start with a paragraph, unique non-empty task
//!    `localId`s, rectangular tables, no duplicate or empty marks, integer levels, allowed link
//!    URLs, no `null` attributes.
//!
//! Plus two warnings: `W_EMPTY_DOCUMENT` and (Jira only) `W_SIZE_LIMIT`.

mod invariants;
mod schema;

use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde_json::Value;

use crate::diagnostics::{Code, Diagnostic, Location};
use crate::limits::{DEFAULT_MAX_DEPTH, JIRA_FIELD_CHAR_LIMIT};
use crate::options::Target;

/// Documents nested deeper than this (in JSON objects and arrays) are rejected with
/// `E_INPUT_TOO_DEEP` before any recursive check runs. Markdown → ADF can legitimately produce
/// about four JSON levels per container level (`bulletList` > `content` > `listItem` > `content`),
/// so this leaves room for [`DEFAULT_MAX_DEPTH`] containers.
pub const MAX_JSON_DEPTH: usize = DEFAULT_MAX_DEPTH * 8;

/// Validates a bare `doc`. Returns errors (`E_SCHEMA`, `E_TARGET`, `E_INVARIANT`) and warnings
/// (`W_SIZE_LIMIT`, `W_EMPTY_DOCUMENT`), each located with a JSON Pointer relative to the
/// document root. An empty vector means the document is valid.
///
/// Errors come first, in document order, then warnings. A document nested deeper than
/// [`MAX_JSON_DEPTH`] gives a single `E_INPUT_TOO_DEEP`.
pub fn validate(doc: &Value, target: Target) -> Vec<Diagnostic> {
    if json_depth_exceeds(doc, MAX_JSON_DEPTH) {
        return vec![
            Diagnostic::new(
                Code::InputTooDeep,
                format!("the document nests deeper than {MAX_JSON_DEPTH} JSON levels"),
            )
            .at(Location::pointer("")),
        ];
    }

    let mut errors = Vec::new();
    match schema::engine() {
        Ok(engine) => {
            for issue in engine.check(doc) {
                let mut diagnostic = Diagnostic::new(Code::Schema, issue.message)
                    .at(Location::pointer(issue.pointer));
                if let Some(hint) = issue.hint {
                    diagnostic = diagnostic.with_hint(hint);
                }
                errors.push(diagnostic);
            }
        }
        Err(reason) => errors.push(Diagnostic::new(
            Code::Internal,
            format!("the embedded ADF schema could not be loaded: {reason}"),
        )),
    }
    let schema_pointers: BTreeSet<String> = errors.iter().filter_map(pointer_of).collect();
    errors.extend(invariants::check(
        doc,
        target,
        DEFAULT_MAX_DEPTH * 4,
        &schema_pointers,
    ));
    errors.sort_by(compare_diagnostics);

    let mut warnings = Vec::new();
    if is_empty_document(doc) {
        warnings.push(
            Diagnostic::new(Code::EmptyDocument, "the document has no content")
                .at(Location::pointer("/content"))
                .with_hint("Jira rejects empty comments; add some text"),
        );
    }
    if target == Target::Jira {
        let chars = compact_json_chars(doc);
        if chars > JIRA_FIELD_CHAR_LIMIT {
            warnings.push(
                Diagnostic::new(
                    Code::SizeLimit,
                    format!(
                        "the document is {chars} characters of compact JSON, more than Jira's {JIRA_FIELD_CHAR_LIMIT}-character field limit"
                    ),
                )
                .at(Location::pointer(""))
                .with_hint("Split the content into several comments, or shorten it"),
            );
        }
    }
    errors.extend(warnings);
    errors
}

fn pointer_of(diagnostic: &Diagnostic) -> Option<String> {
    diagnostic.location.as_ref()?.pointer.clone()
}

/// Orders diagnostics by location in the document (array indices numerically), then by code.
fn compare_diagnostics(a: &Diagnostic, b: &Diagnostic) -> Ordering {
    let pa = pointer_of(a).unwrap_or_default();
    let pb = pointer_of(b).unwrap_or_default();
    compare_pointers(&pa, &pb)
        .then_with(|| a.code.cmp(&b.code))
        .then_with(|| a.message.cmp(&b.message))
}

/// Compares JSON Pointers segment by segment, numerically where both segments are indices, so
/// that `/content/2` sorts before `/content/10`.
pub(crate) fn compare_pointers(left: &str, right: &str) -> Ordering {
    let mut left = left.split('/');
    let mut right = right.split('/');
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(l), Some(r)) => {
                let order = match (l.parse::<u64>(), r.parse::<u64>()) {
                    (Ok(li), Ok(ri)) => li.cmp(&ri),
                    _ => l.cmp(r),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// Whether nesting of arrays and objects goes deeper than `limit`. Iterative, so it is safe on
/// any input.
fn json_depth_exceeds(value: &Value, limit: usize) -> bool {
    let mut stack: Vec<(&Value, usize)> = vec![(value, 0)];
    while let Some((value, depth)) = stack.pop() {
        let depth = depth + 1;
        match value {
            Value::Array(items) => {
                if depth > limit {
                    return true;
                }
                stack.extend(items.iter().map(|child| (child, depth)));
            }
            Value::Object(map) => {
                if depth > limit {
                    return true;
                }
                stack.extend(map.values().map(|child| (child, depth)));
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    false
}

/// No content, or only empty paragraphs.
fn is_empty_document(doc: &Value) -> bool {
    let Some(content) = doc.get("content").and_then(Value::as_array) else {
        return false;
    };
    content.iter().all(|block| {
        block.get("type").and_then(Value::as_str) == Some("paragraph")
            && block
                .get("content")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
    })
}

/// The length of the compact JSON serialization in characters (what Jira's limit counts),
/// without building the string.
fn compact_json_chars(doc: &Value) -> usize {
    struct CharCounter(usize);
    impl std::io::Write for CharCounter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            // Count UTF-8 lead bytes, i.e. everything except continuation bytes.
            self.0 += buf.iter().filter(|&&b| b & 0xC0 != 0x80).count();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = CharCounter(0);
    match serde_json::to_writer(&mut counter, doc) {
        Ok(()) => counter.0,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn codes(diagnostics: &[Diagnostic]) -> Vec<(Code, String)> {
        diagnostics
            .iter()
            .map(|d| (d.code, pointer_of(d).unwrap_or_default()))
            .collect()
    }

    #[allow(clippy::needless_pass_by_value)]
    fn doc(content: Value) -> Value {
        json!({"type":"doc","version":1,"content":content})
    }

    fn para(text: &str) -> Value {
        json!({"type":"paragraph","content":[{"type":"text","text":text}]})
    }

    const NONE: Vec<Diagnostic> = Vec::new();

    #[test]
    fn valid_document() {
        assert_eq!(validate(&doc(json!([para("hello")])), Target::Jira), NONE);
    }

    #[test]
    fn model_output_is_valid() {
        use crate::adf::model::{AlignedParagraph, Doc, DocBlock, Inline, Text};
        let value = Doc {
            content: vec![DocBlock::Paragraph(AlignedParagraph {
                content: vec![Inline::Text(Text::plain("x").unwrap())],
                align: None,
            })],
        }
        .to_value()
        .unwrap();
        assert_eq!(validate(&value, Target::Jira), NONE);
        assert_eq!(validate(&value, Target::Confluence), NONE);
    }

    #[test]
    fn known_bad_documents() {
        let cases = [
            (
                doc(
                    json!([{"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"code"},{"type":"strong"}]}]}]),
                ),
                vec![(Code::Schema, "/content/0/content/0/marks")],
            ),
            (
                doc(json!([{"type":"paragraph","content":[{"type":"text","text":""}]}])),
                vec![(Code::Schema, "/content/0/content/0/text")],
            ),
            (
                doc(
                    json!([{"type":"blockquote","content":[{"type":"heading","attrs":{"level":1}}]}]),
                ),
                vec![(Code::Schema, "/content/0/content/0")],
            ),
            (
                doc(
                    json!([{"type":"bulletList","content":[{"type":"listItem","content":[
                    {"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph"}]}]}]}]}]}]),
                ),
                vec![
                    (Code::Schema, "/content/0/content/0/content/0"),
                    (Code::Invariant, "/content/0/content/0/content/0"),
                ],
            ),
            (
                doc(
                    json!([{"type":"taskList","attrs":{"localId":"a"},"content":[{"type":"taskItem","attrs":{"state":"TODO"}}]}]),
                ),
                vec![(Code::Schema, "/content/0/content/0/attrs")],
            ),
            (
                doc(
                    json!([{"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"em"},{"type":"em"}]}]}]),
                ),
                vec![(Code::Invariant, "/content/0/content/0/marks/1")],
            ),
            (
                doc(json!([{"type":"table","content":[
                    {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph"}]},{"type":"tableCell","content":[{"type":"paragraph"}]}]},
                    {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph"}]}]}]}])),
                vec![(Code::Invariant, "/content/0/content/1")],
            ),
            (
                doc(
                    json!([{"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"link","attrs":{"href":"javascript:alert(1)"}}]}]}]),
                ),
                vec![(Code::Invariant, "/content/0/content/0/marks/0/attrs/href")],
            ),
            (
                doc(
                    json!([{"type":"paragraph","content":[{"type":"text","text":"x"}],"marks":[{"type":"fontSize","attrs":{"fontSize":"small"}}]}]),
                ),
                vec![(Code::Target, "/content/0/marks/0")],
            ),
        ];
        for (value, expected) in cases {
            let found = validate(&value, Target::Jira);
            let expected: Vec<(Code, String)> = expected
                .into_iter()
                .map(|(code, pointer)| (code, pointer.to_owned()))
                .collect();
            assert_eq!(codes(&found), expected, "{value}\n{found:#?}");
        }
    }

    #[test]
    fn font_size_is_fine_for_confluence() {
        let value = doc(
            json!([{"type":"paragraph","content":[{"type":"text","text":"x"}],"marks":[{"type":"fontSize","attrs":{"fontSize":"small"}}]}]),
        );
        assert_eq!(validate(&value, Target::Confluence), NONE);
    }

    #[test]
    fn errors_are_sorted_in_document_order() {
        let mut content: Vec<Value> = (0..12).map(|_| para("ok")).collect();
        content[10] = json!({"type":"paragraph","content":[{"type":"text","text":""}]});
        content[2] = json!({"type":"paragraph","content":[{"type":"text","text":""}]});
        let found = validate(&doc(Value::Array(content)), Target::Jira);
        assert_eq!(
            codes(&found),
            [
                (Code::Schema, "/content/2/content/0/text".to_owned()),
                (Code::Schema, "/content/10/content/0/text".to_owned())
            ]
        );
    }

    #[test]
    fn empty_document_warning() {
        for value in [doc(json!([])), doc(json!([{"type":"paragraph"}]))] {
            assert_eq!(
                codes(&validate(&value, Target::Jira)),
                [(Code::EmptyDocument, "/content".to_owned())]
            );
        }
        assert_eq!(validate(&doc(json!([para(" ")])), Target::Jira), NONE);
    }

    #[test]
    fn size_limit_warning_counts_characters() {
        // 'é' is two bytes but one character.
        let text = "é".repeat(JIRA_FIELD_CHAR_LIMIT);
        let big = doc(json!([para(&text)]));
        assert!(compact_json_chars(&big) > JIRA_FIELD_CHAR_LIMIT);
        assert_eq!(
            codes(&validate(&big, Target::Jira)),
            [(Code::SizeLimit, String::new())]
        );
        assert_eq!(validate(&big, Target::Confluence), NONE);
        let small = doc(json!([para(&"é".repeat(JIRA_FIELD_CHAR_LIMIT - 100))]));
        assert!(compact_json_chars(&small) <= JIRA_FIELD_CHAR_LIMIT);
        assert_eq!(validate(&small, Target::Jira), NONE);
        assert_eq!(
            compact_json_chars(&json!({"a":"é"})),
            serde_json::to_string(&json!({"a":"é"}))
                .unwrap()
                .chars()
                .count()
        );
    }

    #[test]
    fn too_deep_documents_are_rejected_without_recursion() {
        let mut value = json!(null);
        for _ in 0..100_000 {
            value = Value::Array(vec![value]);
        }
        let found = validate(&value, Target::Jira);
        assert_eq!(codes(&found), [(Code::InputTooDeep, String::new())]);
        // Dropping a 100 000-level value recurses in serde_json; drop it iteratively.
        let mut value = value;
        while let Value::Array(mut items) = value {
            value = items.pop().unwrap_or(Value::Null);
        }
    }

    #[test]
    fn deep_but_allowed_documents_validate() {
        let mut node = para("x");
        for _ in 0..DEFAULT_MAX_DEPTH {
            node = json!({"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph"}, node]}]});
        }
        assert_eq!(validate(&doc(json!([node])), Target::Jira), NONE);
    }

    #[test]
    fn pointer_order() {
        assert_eq!(
            compare_pointers("/content/2", "/content/10"),
            Ordering::Less
        );
        assert_eq!(compare_pointers("", "/content"), Ordering::Less);
        assert_eq!(
            compare_pointers("/content/1/marks", "/content/1"),
            Ordering::Greater
        );
        assert_eq!(compare_pointers("/a", "/a"), Ordering::Equal);
    }
}
