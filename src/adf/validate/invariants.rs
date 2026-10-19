//! Rules Jira and Confluence enforce beyond the JSON schema (`E_INVARIANT`), plus the target
//! rules from [`crate::adf::targets`] (`E_TARGET`), checked in one walk over the document.
//!
//! See `docs/syntax.md#rules-the-converter-always-enforces`. The walk only follows `content`
//! arrays (ADF children); it never trusts the shape of the input, so it is safe to run on documents
//! that fail the schema.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use super::schema::{code, preview};
use crate::adf::envelope::escape_pointer_token;
use crate::adf::model::Href;
use crate::adf::targets;
use crate::diagnostics::{Code, Diagnostic, Location};
use crate::options::Target;

/// Node types whose `localId` is required and must be unique.
const LOCAL_ID_NODES: &[&str] = &[
    "taskList",
    "taskItem",
    "blockTaskItem",
    "decisionList",
    "decisionItem",
];

/// Smart-link nodes whose `url` must pass the URL allowlist.
const CARD_NODES: &[&str] = &["inlineCard", "blockCard", "embedCard"];

const URL_HINT: &str =
    "Use an http(s), ftp(s), mailto: or tel: URL, a /root-relative path or a #anchor";

/// Runs the target and invariant checks. `schema_pointers` are locations the schema layer already
/// reported; `null` attributes there are not reported twice.
pub(crate) fn check(
    doc: &Value,
    target: Target,
    max_depth: usize,
    schema_pointers: &BTreeSet<String>,
) -> Vec<Diagnostic> {
    let mut walker = Walker {
        target,
        max_depth,
        schema_pointers,
        local_ids: BTreeMap::new(),
        out: Vec::new(),
    };
    let mut pointer = String::new();
    walker.node(doc, &mut pointer, 0, false);
    walker.out
}

struct Walker<'a> {
    target: Target,
    max_depth: usize,
    schema_pointers: &'a BTreeSet<String>,
    /// `localId` → pointer of its first use.
    local_ids: BTreeMap<String, String>,
    out: Vec<Diagnostic>,
}

impl Walker<'_> {
    fn invariant(&mut self, pointer: String, message: String, hint: Option<&str>) {
        let mut diagnostic =
            Diagnostic::new(Code::Invariant, message).at(Location::pointer(pointer));
        if let Some(hint) = hint {
            diagnostic = diagnostic.with_hint(hint);
        }
        self.out.push(diagnostic);
    }

    /// Checks one node and its subtree. `blocked` is set inside a node the target rejects, so
    /// target errors are reported once per rejected subtree.
    fn node(&mut self, value: &Value, pointer: &mut String, depth: usize, blocked: bool) {
        if depth > self.max_depth {
            return;
        }
        let Some(node) = value.as_object() else {
            return;
        };
        let ty = node.get("type").and_then(Value::as_str).unwrap_or("");
        let mut blocked = blocked;
        if !blocked && !ty.is_empty() {
            blocked = targets::check_node(self.target, ty, node, pointer, &mut self.out);
        }
        self.attrs(node, ty, pointer);
        self.marks(node, pointer, blocked);
        match ty {
            "listItem" => {
                self.first_child(node, pointer, &["paragraph"], "a list item", "a paragraph")
            }
            "taskList" => self.first_child(
                node,
                pointer,
                &["taskItem", "blockTaskItem"],
                "a task list",
                "a task item",
            ),
            "heading" => self.integer_attr(node, pointer, "level"),
            "orderedList" => self.integer_attr(node, pointer, "order"),
            "table" => self.table(node, pointer),
            _ => {}
        }
        if LOCAL_ID_NODES.contains(&ty) {
            self.local_id(node, ty, pointer);
        }
        if CARD_NODES.contains(&ty) {
            self.card_url(node, ty, pointer);
        }
        let Some(children) = node.get("content").and_then(Value::as_array) else {
            return;
        };
        let len = pointer.len();
        for (index, child) in children.iter().enumerate() {
            pointer.push_str("/content/");
            pointer.push_str(&index.to_string());
            self.node(child, pointer, depth + 1, blocked);
            pointer.truncate(len);
        }
    }

    /// `null` attribute values (Jira rejects them; omit the attribute instead).
    fn attrs(&mut self, node: &Map<String, Value>, ty: &str, pointer: &str) {
        let Some(attrs) = node.get("attrs").and_then(Value::as_object) else {
            return;
        };
        let what = if ty.is_empty() {
            "the node".to_owned()
        } else {
            format!("`{ty}`")
        };
        self.null_attrs(attrs, &format!("{pointer}/attrs"), &what);
    }

    fn null_attrs(&mut self, attrs: &Map<String, Value>, attrs_pointer: &str, owner: &str) {
        for (name, value) in attrs {
            if !value.is_null() {
                continue;
            }
            let at = format!("{attrs_pointer}/{}", escape_pointer_token(name));
            if self.schema_pointers.contains(&at) {
                continue;
            }
            self.invariant(
                at,
                format!("attribute {} of {owner} is null", code(name)),
                Some("Omit the attribute instead of setting it to null"),
            );
        }
    }

    /// Mark rules: no empty `marks`, no duplicate mark types, safe link targets, no `null`
    /// attributes, and the target's mark blocklist.
    fn marks(&mut self, node: &Map<String, Value>, pointer: &str, blocked: bool) {
        let Some(marks) = node.get("marks").and_then(Value::as_array) else {
            return;
        };
        if marks.is_empty() {
            self.invariant(
                format!("{pointer}/marks"),
                "empty `marks` array".to_owned(),
                Some("Omit the \"marks\" key when there are no marks"),
            );
            return;
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for (index, mark) in marks.iter().enumerate() {
            let at = format!("{pointer}/marks/{index}");
            let Some(mark_type) = mark.get("type").and_then(Value::as_str) else {
                continue;
            };
            if !seen.insert(mark_type) {
                let message = if mark_type == "subsup" {
                    "a node can have only one `subsup` mark (sub or sup)".to_owned()
                } else {
                    format!("duplicate {} mark", code(mark_type))
                };
                self.invariant(
                    at.clone(),
                    message,
                    Some("Keep each mark type at most once"),
                );
            }
            if !blocked {
                targets::check_mark(self.target, mark_type, &at, &mut self.out);
            }
            let Some(attrs) = mark.get("attrs").and_then(Value::as_object) else {
                continue;
            };
            self.null_attrs(
                attrs,
                &format!("{at}/attrs"),
                &format!("the {} mark", code(mark_type)),
            );
            if mark_type == "link"
                && let Some(href) = attrs.get("href").and_then(Value::as_str)
                && Href::parse(href).is_none()
            {
                self.invariant(
                    format!("{at}/attrs/href"),
                    format!(
                        "link target {} is not an allowed URL",
                        preview(&Value::from(href))
                    ),
                    Some(URL_HINT),
                );
            }
        }
    }

    /// The first child of `node` must be one of `allowed`.
    fn first_child(
        &mut self,
        node: &Map<String, Value>,
        pointer: &str,
        allowed: &[&str],
        container: &str,
        expected: &str,
    ) {
        let Some(first) = node
            .get("content")
            .and_then(Value::as_array)
            .and_then(|children| children.first())
        else {
            return;
        };
        let Some(first_type) = first.get("type").and_then(Value::as_str) else {
            return;
        };
        if !allowed.contains(&first_type) {
            self.invariant(
                format!("{pointer}/content/0"),
                format!(
                    "{container} must start with {expected}, not {}",
                    code(first_type)
                ),
                Some(if container == "a list item" {
                    "Insert an empty paragraph {\"type\":\"paragraph\"} as the first child"
                } else {
                    "Put a taskItem first; nested task lists follow the item they belong to"
                }),
            );
        }
    }

    /// Numeric attributes that must be integers (the schema only says `number`).
    fn integer_attr(&mut self, node: &Map<String, Value>, pointer: &str, name: &str) {
        let Some(value) = node.get("attrs").and_then(|attrs| attrs.get(name)) else {
            return;
        };
        if let Some(number) = value.as_f64()
            && !value.is_i64()
            && !value.is_u64()
            && number.fract() != 0.0
        {
            self.invariant(
                format!("{pointer}/attrs/{name}"),
                format!("attribute `{name}` must be an integer, found {value}"),
                None,
            );
        }
    }

    /// Task and decision nodes need a non-empty `localId` that is unique in the document.
    fn local_id(&mut self, node: &Map<String, Value>, ty: &str, pointer: &str) {
        let Some(Value::String(id)) = node.get("attrs").and_then(|attrs| attrs.get("localId"))
        else {
            return;
        };
        let at = format!("{pointer}/attrs/localId");
        if id.is_empty() {
            self.invariant(
                at,
                format!("`{ty}` has an empty `localId`"),
                Some("Give every task and decision node a unique, non-empty localId"),
            );
            return;
        }
        if let Some(first) = self.local_ids.get(id) {
            let message = format!(
                "duplicate `localId` {} (first used at {})",
                preview(&Value::from(id.as_str())),
                if first.is_empty() { "the root" } else { first }
            );
            self.invariant(
                at,
                message,
                Some("Give every task and decision node a unique, non-empty localId"),
            );
        } else {
            self.local_ids.insert(id.clone(), pointer.to_owned());
        }
    }

    /// Smart links must use an allowed URL.
    fn card_url(&mut self, node: &Map<String, Value>, ty: &str, pointer: &str) {
        let Some(url) = node
            .get("attrs")
            .and_then(|attrs| attrs.get("url"))
            .and_then(Value::as_str)
        else {
            return;
        };
        if Href::parse(url).is_none() {
            self.invariant(
                format!("{pointer}/attrs/url"),
                format!(
                    "`{ty}` URL {} is not an allowed URL",
                    preview(&Value::from(url))
                ),
                Some(URL_HINT),
            );
        }
    }

    /// Rows must be rectangular once `colspan` and `rowspan` are counted, spans must be positive
    /// integers, and no `rowspan` may reach past the last row.
    fn table(&mut self, node: &Map<String, Value>, pointer: &str) {
        let Some(rows) = node.get("content").and_then(Value::as_array) else {
            return;
        };
        // Cells from earlier rows that still cover later rows: (rows left, columns, cell pointer).
        let mut pending: Vec<(u64, u64, String)> = Vec::new();
        let mut widths: Vec<(usize, u64)> = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            let Some(cells) = row.get("content").and_then(Value::as_array) else {
                continue;
            };
            let mut width = pending
                .iter()
                .fold(0u64, |sum, (_, columns, _)| sum.saturating_add(*columns));
            for entry in &mut pending {
                entry.0 -= 1;
            }
            pending.retain(|(left, _, _)| *left > 0);
            for (c, cell) in cells.iter().enumerate() {
                let cell_pointer = format!("{pointer}/content/{r}/content/{c}");
                let colspan = self.span(cell, "colspan", &cell_pointer);
                let rowspan = self.span(cell, "rowspan", &cell_pointer);
                width = width.saturating_add(colspan);
                if rowspan > 1 {
                    pending.push((rowspan - 1, colspan, cell_pointer));
                }
            }
            widths.push((r, width));
        }
        if let Some((_, _, cell)) = pending.first() {
            self.invariant(
                format!("{cell}/attrs/rowspan"),
                "`rowspan` reaches past the last row of the table".to_owned(),
                Some("Reduce the rowspan or add the missing rows"),
            );
        }
        let Some(&(_, expected)) = widths.first() else {
            return;
        };
        for &(r, width) in widths.iter().skip(1) {
            if width != expected {
                self.invariant(
                    format!("{pointer}/content/{r}"),
                    format!(
                        "table row {r} is {} wide, but row 0 is {} (rows must be rectangular, counting colspan and rowspan)",
                        plural_columns(width),
                        plural_columns(expected)
                    ),
                    Some("Pad short rows with empty cells: {\"type\":\"tableCell\",\"content\":[{\"type\":\"paragraph\"}]}"),
                );
            }
        }
    }

    /// A `colspan`/`rowspan` value (default 1). Invalid values are reported and count as 1.
    fn span(&mut self, cell: &Value, name: &str, cell_pointer: &str) -> u64 {
        let Some(value) = cell.get("attrs").and_then(|attrs| attrs.get(name)) else {
            return 1;
        };
        if let Some(span) = value.as_u64().filter(|span| *span >= 1) {
            return span;
        }
        if let Some(number) = value.as_f64() {
            if number >= 1.0 && number.fract() == 0.0 && number <= 1e9 {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                return number as u64;
            }
            self.invariant(
                format!("{cell_pointer}/attrs/{name}"),
                format!("`{name}` must be an integer of at least 1, found {value}"),
                None,
            );
        }
        // Other types are schema errors and count as 1 here.
        1
    }
}

fn plural_columns(count: u64) -> String {
    if count == 1 {
        "1 column".to_owned()
    } else {
        format!("{count} columns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NONE: Vec<(Code, String, String)> = Vec::new();

    fn run(doc: &Value, target: Target) -> Vec<(Code, String, String)> {
        check(doc, target, 64, &BTreeSet::new())
            .into_iter()
            .map(|d| {
                (
                    d.code,
                    d.location.and_then(|l| l.pointer).unwrap_or_default(),
                    d.message,
                )
            })
            .collect()
    }

    #[allow(clippy::needless_pass_by_value)]
    fn doc(content: Value) -> Value {
        json!({"type":"doc","version":1,"content":content})
    }

    #[allow(clippy::needless_pass_by_value)]
    fn cell(attrs: Value) -> Value {
        json!({"type":"tableCell","attrs":attrs,"content":[{"type":"paragraph"}]})
    }

    #[test]
    fn list_item_must_start_with_paragraph() {
        let value = doc(
            json!([{"type":"bulletList","content":[{"type":"listItem","content":[
            {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph"}]}]}]}]}]),
        );
        assert_eq!(
            run(&value, Target::Jira),
            [(
                Code::Invariant,
                "/content/0/content/0/content/0".to_owned(),
                "a list item must start with a paragraph, not `bulletList`".to_owned()
            )]
        );
    }

    #[test]
    fn task_list_must_start_with_item() {
        let inner = json!({"type":"taskList","attrs":{"localId":"b"},"content":[{"type":"taskItem","attrs":{"localId":"c","state":"TODO"}}]});
        let value = doc(json!([{"type":"taskList","attrs":{"localId":"a"},"content":[inner]}]));
        let found = run(&value, Target::Jira);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].1, "/content/0/content/0");
    }

    #[test]
    fn local_ids_are_unique_and_non_empty() {
        let value = doc(json!([
            {"type":"taskList","attrs":{"localId":"x"},"content":[
                {"type":"taskItem","attrs":{"localId":"x","state":"TODO"}},
                {"type":"taskItem","attrs":{"localId":"","state":"DONE"}}]}
        ]));
        let found = run(&value, Target::Jira);
        assert_eq!(
            found,
            [
                (
                    Code::Invariant,
                    "/content/0/content/0/attrs/localId".to_owned(),
                    "duplicate `localId` \"x\" (first used at /content/0)".to_owned()
                ),
                (
                    Code::Invariant,
                    "/content/0/content/1/attrs/localId".to_owned(),
                    "`taskItem` has an empty `localId`".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn ragged_rows() {
        let row = |cells: Vec<Value>| json!({"type":"tableRow","content":cells});
        let value = doc(json!([{"type":"table","content":[
            row(vec![cell(json!({})), cell(json!({}))]),
            row(vec![cell(json!({}))]),
            row(vec![cell(json!({"colspan":2}))]),
        ]}]));
        assert_eq!(
            run(&value, Target::Jira),
            [(
                Code::Invariant,
                "/content/0/content/1".to_owned(),
                "table row 1 is 1 column wide, but row 0 is 2 columns (rows must be rectangular, counting colspan and rowspan)".to_owned()
            )]
        );
    }

    #[test]
    fn rowspans_count_in_later_rows() {
        let row = |cells: Vec<Value>| json!({"type":"tableRow","content":cells});
        let ok = doc(json!([{"type":"table","content":[
            row(vec![cell(json!({"rowspan":2})), cell(json!({}))]),
            row(vec![cell(json!({}))]),
        ]}]));
        assert_eq!(run(&ok, Target::Jira), NONE);
        let overflow = doc(json!([{"type":"table","content":[
            row(vec![cell(json!({"rowspan":3})), cell(json!({}))]),
            row(vec![cell(json!({}))]),
        ]}]));
        let found = run(&overflow, Target::Jira);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].1, "/content/0/content/0/content/0/attrs/rowspan");
        let bad_span = doc(
            json!([{"type":"table","content":[row(vec![cell(json!({"colspan":0})), cell(json!({"rowspan":1.5}))])]}]),
        );
        let found = run(&bad_span, Target::Jira);
        assert_eq!(found.len(), 2, "{found:?}");
        let huge = doc(
            json!([{"type":"table","content":[row(vec![cell(json!({"colspan":u64::MAX}))]), row(vec![cell(json!({}))])]}]),
        );
        assert_eq!(run(&huge, Target::Jira).len(), 1);
    }

    #[test]
    fn duplicate_and_empty_marks() {
        let value = doc(json!([{"type":"paragraph","content":[
            {"type":"text","text":"a","marks":[{"type":"strong"},{"type":"em"},{"type":"strong"}]},
            {"type":"text","text":"b","marks":[{"type":"subsup","attrs":{"type":"sub"}},{"type":"subsup","attrs":{"type":"sup"}}]},
            {"type":"text","text":"c","marks":[]}
        ]}]));
        let found = run(&value, Target::Jira);
        let pointers: Vec<&str> = found.iter().map(|f| f.1.as_str()).collect();
        assert_eq!(
            pointers,
            [
                "/content/0/content/0/marks/2",
                "/content/0/content/1/marks/1",
                "/content/0/content/2/marks"
            ]
        );
        assert_eq!(found[0].2, "duplicate `strong` mark");
    }

    #[test]
    fn unsafe_links() {
        let link = |href: &str| json!({"type":"text","text":"x","marks":[{"type":"link","attrs":{"href":href}}]});
        let value = doc(json!([{"type":"paragraph","content":[
            link("https://example.com"), link("javascript:alert(1)"), link("/browse/A-1"), link("data:text/html,x")
        ]}, {"type":"paragraph","content":[{"type":"inlineCard","attrs":{"url":"javascript:x"}}]}]));
        let found = run(&value, Target::Jira);
        let pointers: Vec<&str> = found.iter().map(|f| f.1.as_str()).collect();
        assert_eq!(
            pointers,
            [
                "/content/0/content/1/marks/0/attrs/href",
                "/content/0/content/3/marks/0/attrs/href",
                "/content/1/content/0/attrs/url"
            ]
        );
        assert_eq!(
            found[0].2,
            "link target \"javascript:alert(1)\" is not an allowed URL"
        );
    }

    #[test]
    fn null_attributes_and_integers() {
        let value = doc(json!([
            {"type":"codeBlock","attrs":{"language":null}},
            {"type":"heading","attrs":{"level":2.5}},
            {"type":"heading","attrs":{"level":2.0}},
            {"type":"orderedList","attrs":{"order":1.5},"content":[{"type":"listItem","content":[{"type":"paragraph"}]}]},
            {"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"link","attrs":{"href":"https://a.b","title":null}}]}]}
        ]));
        let found = run(&value, Target::Confluence);
        let pointers: Vec<&str> = found.iter().map(|f| f.1.as_str()).collect();
        assert_eq!(
            pointers,
            [
                "/content/0/attrs/language",
                "/content/1/attrs/level",
                "/content/3/attrs/order",
                "/content/4/content/0/marks/0/attrs/title"
            ]
        );
        // Already reported by the schema layer: not repeated.
        let schema: BTreeSet<String> = ["/content/0/attrs/language".to_owned()].into();
        let found = check(&value, Target::Confluence, 64, &schema);
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn target_rules_are_reported_once_per_rejected_subtree() {
        let value = doc(json!([
            {"type":"layoutSection","content":[
                {"type":"layoutColumn","attrs":{"width":50},"content":[{"type":"paragraph","content":[
                    {"type":"text","text":"x","marks":[{"type":"fontSize","attrs":{"fontSize":"small"}}]}]}]},
                {"type":"layoutColumn","attrs":{"width":50},"content":[{"type":"paragraph"}]}]},
            {"type":"paragraph","marks":[{"type":"fontSize","attrs":{"fontSize":"small"}}]}
        ]));
        let found = run(&value, Target::Jira);
        let pointers: Vec<(Code, &str)> = found.iter().map(|f| (f.0, f.1.as_str())).collect();
        assert_eq!(
            pointers,
            [
                (Code::Target, "/content/0"),
                (Code::Target, "/content/1/marks/0")
            ]
        );
        assert_eq!(run(&value, Target::Confluence), NONE);
    }

    #[test]
    fn deep_documents_stop_at_the_limit() {
        let mut node = json!({"type":"paragraph","marks":[]});
        for _ in 0..200 {
            node = json!({"type":"blockquote","content":[node]});
        }
        assert_eq!(run(&doc(json!([node])), Target::Jira), NONE);
    }

    #[test]
    fn garbage_is_ignored() {
        for value in [
            json!(null),
            json!([1, 2]),
            json!({"content": "x"}),
            json!({"type": 5, "content":[null, 1, "a"]}),
        ] {
            assert_eq!(run(&value, Target::Jira), NONE, "{value}");
        }
    }
}
