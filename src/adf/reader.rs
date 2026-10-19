//! Lenient reading of arbitrary ADF for ADF → Markdown. Real documents contain `localId`s, attributes
//! and node types this crate never emits; nothing here may fail on them.
//!
//! The reader turns a `serde_json::Value` into a light tree of [`Node`]s that borrow from the input.
//! It only rejects input that is not ADF at all (`E_INPUT_NOT_ADF`) or that nests deeper than the
//! limit (`E_INPUT_TOO_DEEP`). Everything else — unknown node types, unknown attributes, missing
//! fields, wrong value types — is kept as it is, so the renderer can pass it through unchanged.

use serde_json::{Map, Value};

use crate::diagnostics::{Code, Diagnostic, Location};
use crate::limits::Limits;

/// One ADF node, read leniently. Borrows from the input document.
#[derive(Debug, Clone)]
pub struct Node<'a> {
    /// The node's `type`, or `""` when it is missing or not a string.
    pub ty: &'a str,
    /// The original JSON of this node (used for passthrough).
    pub value: &'a Value,
    /// RFC 6901 JSON Pointer of this node, relative to the document root.
    pub pointer: String,
    /// `attrs`, when it is an object.
    attrs: Option<&'a Map<String, Value>>,
    /// `text`, when it is a string.
    pub text: Option<&'a str>,
    /// `marks`, when it is an array (non-object entries are kept as unnamed marks).
    pub marks: Vec<Mark<'a>>,
    /// `content`, when it is an array.
    pub content: Vec<Node<'a>>,
    /// `true` when the node has a `content` key at all (even an empty or malformed one).
    pub has_content_key: bool,
}

/// One mark of a text node (or a block mark such as `alignment`).
#[derive(Debug, Clone)]
pub struct Mark<'a> {
    /// The mark's `type`, or `""` when it is missing or not a string.
    pub ty: &'a str,
    attrs: Option<&'a Map<String, Value>>,
}

impl<'a> Mark<'a> {
    /// A string attribute.
    pub fn attr_str(&self, name: &str) -> Option<&'a str> {
        self.attrs.and_then(|a| a.get(name)).and_then(Value::as_str)
    }

    /// Attribute names, in input order.
    pub fn attr_names(&self) -> impl Iterator<Item = &'a str> {
        self.attrs
            .into_iter()
            .flat_map(|a| a.keys().map(String::as_str))
    }
}

impl<'a> Node<'a> {
    /// An attribute value.
    pub fn attr(&self, name: &str) -> Option<&'a Value> {
        self.attrs.and_then(|a| a.get(name))
    }

    /// A string attribute.
    pub fn attr_str(&self, name: &str) -> Option<&'a str> {
        self.attr(name).and_then(Value::as_str)
    }

    /// Attribute names, in input order.
    pub fn attr_names(&self) -> impl Iterator<Item = &'a str> {
        self.attrs
            .into_iter()
            .flat_map(|a| a.keys().map(String::as_str))
    }

    /// `true` for nodes that may appear in inline content.
    pub fn is_inline(&self) -> bool {
        is_inline_type(self.ty)
    }

    /// The node's text content: text nodes and the display text of inline nodes, recursively.
    /// Hard breaks become `\n`. Used by the plain flavour for nodes without a Markdown form.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        if let Some(text) = self.text {
            out.push_str(text);
        }
        match self.ty {
            "hardBreak" => out.push('\n'),
            "mention" | "emoji" | "status" | "placeholder" | "inlineExtension" => {
                if let Some(text) = self.attr_str("text").or_else(|| self.attr_str("shortName")) {
                    out.push_str(text);
                }
            }
            "inlineCard" | "blockCard" | "embedCard" => {
                if let Some(url) = self.attr_str("url") {
                    out.push_str(url);
                }
            }
            _ => {}
        }
        for child in &self.content {
            child.collect_text(out);
        }
    }
}

/// Node types that are inline content (children of paragraphs, headings and task items).
pub fn is_inline_type(ty: &str) -> bool {
    matches!(
        ty,
        "text"
            | "hardBreak"
            | "mention"
            | "emoji"
            | "date"
            | "status"
            | "inlineCard"
            | "mediaInline"
            | "placeholder"
            | "inlineExtension"
    )
}

/// Reads a bare `doc`. Fails with `E_INPUT_NOT_ADF` if the value is not a JSON object with
/// `"type": "doc"`, and with `E_INPUT_TOO_DEEP` if it nests deeper than
/// [`Limits::max_json_depth`] (JSON depth: every object and array counts one level).
pub fn read_doc<'a>(doc: &'a Value, limits: &Limits) -> Result<Node<'a>, Diagnostic> {
    let Some(object) = doc.as_object() else {
        return Err(not_adf("the input is not a JSON object"));
    };
    if object.get("type").and_then(Value::as_str) != Some("doc") {
        return Err(not_adf("the root object does not have \"type\": \"doc\""));
    }
    let max_depth = limits.max_json_depth();
    if let Some(pointer) = find_too_deep(doc, max_depth) {
        return Err(Diagnostic::new(
            Code::InputTooDeep,
            format!("The ADF document nests deeper than the limit of {max_depth} JSON levels"),
        )
        .at(Location::pointer(pointer)));
    }
    read_node(doc, String::new(), 0, max_depth)
}

fn not_adf(reason: &str) -> Diagnostic {
    Diagnostic::new(
        Code::InputNotAdf,
        format!("The input is not an ADF document: {reason}"),
    )
    .with_hint("Pass a document like {\"type\":\"doc\",\"version\":1,\"content\":[…]}")
}

/// Returns the pointer of the first value nested deeper than `max_depth`, without recursion.
fn find_too_deep(root: &Value, max_depth: usize) -> Option<String> {
    // (value, depth of the value, pointer)
    let mut stack: Vec<(&Value, usize, String)> = vec![(root, 0, String::new())];
    while let Some((value, depth, pointer)) = stack.pop() {
        let level = match value {
            Value::Array(_) | Value::Object(_) => depth + 1,
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => continue,
        };
        if level > max_depth {
            return Some(pointer);
        }
        match value {
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate().rev() {
                    stack.push((item, level, format!("{pointer}/{i}")));
                }
            }
            Value::Object(map) => {
                for (key, item) in map.iter().rev() {
                    stack.push((item, level, format!("{pointer}/{}", escape_pointer(key))));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    None
}

/// Escapes one JSON Pointer reference token (RFC 6901).
pub fn escape_pointer(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn read_node(
    value: &Value,
    pointer: String,
    depth: usize,
    max_depth: usize,
) -> Result<Node<'_>, Diagnostic> {
    if depth > max_depth {
        return Err(Diagnostic::new(
            Code::InputTooDeep,
            format!("The ADF document nests deeper than the limit of {max_depth} JSON levels"),
        )
        .at(Location::pointer(pointer)));
    }
    let object = value.as_object();
    let field = |name: &str| object.and_then(|o| o.get(name));
    let ty = field("type").and_then(Value::as_str).unwrap_or("");
    let attrs = field("attrs").and_then(Value::as_object);
    let text = field("text").and_then(Value::as_str);
    let marks = field("marks")
        .and_then(Value::as_array)
        .map(|marks| {
            marks
                .iter()
                .map(|mark| Mark {
                    ty: mark.get("type").and_then(Value::as_str).unwrap_or(""),
                    attrs: mark.get("attrs").and_then(Value::as_object),
                })
                .collect()
        })
        .unwrap_or_default();
    let has_content_key = field("content").is_some();
    let mut content = Vec::new();
    if let Some(children) = field("content").and_then(Value::as_array) {
        content.reserve(children.len());
        for (i, child) in children.iter().enumerate() {
            content.push(read_node(
                child,
                format!("{pointer}/content/{i}"),
                depth + 1,
                max_depth,
            )?);
        }
    }
    Ok(Node {
        ty,
        value,
        pointer,
        attrs,
        text,
        marks,
        content,
        has_content_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_non_documents() {
        let limits = Limits::default();
        for bad in [
            json!([]),
            json!("doc"),
            json!({"type": "paragraph"}),
            json!({"body": {"type": "doc"}}),
        ] {
            let err = read_doc(&bad, &limits).unwrap_err();
            assert_eq!(err.code, Code::InputNotAdf, "{bad}");
        }
    }

    #[test]
    fn reads_leniently() {
        let doc = json!({"type": "doc", "content": [
            {"type": "paragraph", "attrs": {"localId": "x", "bogus": 1}, "content": [
                {"type": "text", "text": "hi", "marks": [{"type": "strong"}, "junk"]}
            ]},
            {"type": "weird", "content": "not an array"},
            42
        ]});
        let root = read_doc(&doc, &Limits::default()).unwrap();
        assert_eq!(root.content.len(), 3);
        let para = &root.content[0];
        assert_eq!(para.ty, "paragraph");
        assert_eq!(para.pointer, "/content/0");
        assert_eq!(para.attr_names().collect::<Vec<_>>(), ["localId", "bogus"]);
        let text = &para.content[0];
        assert_eq!(text.text, Some("hi"));
        assert_eq!(text.pointer, "/content/0/content/0");
        assert_eq!(text.marks.len(), 2);
        assert_eq!(text.marks[1].ty, "");
        assert!(root.content[1].content.is_empty());
        assert!(root.content[1].has_content_key);
        assert_eq!(root.content[2].ty, "");
    }

    #[test]
    fn depth_limit_is_enforced_without_recursion() {
        // Built by hand: `json!` would serialise (recursively) the nested value.
        let wrap = |ty: &str, child: Value| {
            let mut map = Map::new();
            map.insert("type".into(), Value::String(ty.into()));
            map.insert("content".into(), Value::Array(vec![child]));
            Value::Object(map)
        };
        let mut deep = json!({"type": "text", "text": "x"});
        for _ in 0..10_000 {
            deep = wrap("blockquote", deep);
        }
        let doc = wrap("doc", deep);
        let err = read_doc(&doc, &Limits::default()).unwrap_err();
        assert_eq!(err.code, Code::InputTooDeep);
        let pointer = err.location.unwrap().pointer.unwrap();
        assert!(pointer.starts_with("/content/0/content/0"), "{pointer}");
        // Dropping a 10k-deep serde_json value recurses; leak it to keep the test about us.
        std::mem::forget(doc);
    }

    #[test]
    fn depth_counts_json_levels() {
        // max_depth 1 container level = 8 JSON levels.
        let limits = Limits {
            max_depth: 1,
            ..Limits::default()
        };
        // doc(1) > content(2) > paragraph(3) > content(4) > text(5): fine.
        let ok = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]});
        assert!(read_doc(&ok, &limits).is_ok());
        // doc(1) > content(2) > quote(3) > content(4) > quote(5) > content(6) > paragraph(7)
        // > content(8) > text(9): too deep.
        let deep = json!({"type": "doc", "content": [{"type": "blockquote", "content": [{"type": "blockquote", "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]}]}]});
        assert_eq!(
            read_doc(&deep, &limits).unwrap_err().code,
            Code::InputTooDeep
        );
    }

    #[test]
    fn text_content_collects_inline_text() {
        let doc = json!({"type": "doc", "content": [{"type": "decisionList", "content": [
            {"type": "decisionItem", "content": [
                {"type": "text", "text": "a"}, {"type": "hardBreak"},
                {"type": "mention", "attrs": {"id": "1", "text": "@B"}}
            ]}
        ]}]});
        let root = read_doc(&doc, &Limits::default()).unwrap();
        assert_eq!(root.content[0].text_content(), "a\n@B");
    }

    #[test]
    fn pointer_tokens_are_escaped() {
        assert_eq!(escape_pointer("a/b~c"), "a~1b~0c");
    }
}
