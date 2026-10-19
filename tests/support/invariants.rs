//! An independent check of the rules in `docs/syntax.md` ("Rules the converter always enforces"
//! and the inline/table/task rules), written without using the crate's validator. The JSON
//! schema catches most structural problems; these are the rules it does not express.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

/// Atlassian's canonical mark order (`marks-in-order.js`).
const MARK_ORDER: &[&str] = &[
    "link",
    "em",
    "strong",
    "textColor",
    "backgroundColor",
    "strike",
    "subsup",
    "underline",
    "code",
];

/// Named status colours (hex colours are not Jira-safe).
const STATUS_COLORS: &[&str] = &["neutral", "purple", "blue", "red", "yellow", "green"];

/// Node types the converter never emits for Jira (layouts, extensions, recently promoted or
/// stage-0 nodes).
const FORBIDDEN_NODES: &[&str] = &[
    "layoutSection",
    "layoutColumn",
    "extension",
    "bodiedExtension",
    "inlineExtension",
    "multiBodiedExtension",
    "extensionFrame",
    "blockTaskItem",
    "syncBlock",
    "bodiedSyncBlock",
];

/// Marks the converter never emits (C27 blocklist).
const FORBIDDEN_MARKS: &[&str] = &[
    "fontSize",
    "breakout",
    "indentation",
    "border",
    "annotation",
    "dataConsumer",
    "fragment",
];

/// Inline node types (they may only appear in inline content).
const INLINE_NODES: &[&str] = &[
    "text",
    "hardBreak",
    "mention",
    "emoji",
    "date",
    "status",
    "inlineCard",
    "placeholder",
    "mediaInline",
];

/// A JSON depth no converter output reaches; deeper values are reported instead of walked.
const MAX_WALK_DEPTH: usize = 2048;

/// Returns one message per violated rule, each prefixed with a JSON Pointer.
pub fn invariant_errors(doc: &Value) -> Vec<String> {
    let mut checker = Checker::default();
    checker.walk(doc);
    checker.check_local_ids();
    checker.errors
}

#[derive(Default)]
struct Checker {
    errors: Vec<String>,
    /// `localId` → pointers of the task nodes using it.
    task_ids: BTreeMap<String, Vec<String>>,
}

impl Checker {
    fn error(&mut self, pointer: &str, message: impl Into<String>) {
        let pointer = if pointer.is_empty() { "/" } else { pointer };
        self.errors.push(format!("{pointer}: {}", message.into()));
    }

    /// Visits every node (the root and the elements of `content` arrays) without recursion, so
    /// deep input cannot overflow the stack.
    fn walk(&mut self, doc: &Value) {
        self.check_no_nulls(doc);
        let mut stack: Vec<(String, &Value, Option<&str>, usize)> =
            vec![(String::new(), doc, None, 0)];
        while let Some((pointer, value, parent, depth)) = stack.pop() {
            if depth > MAX_WALK_DEPTH {
                self.error(&pointer, "nesting is deeper than any converter output");
                continue;
            }
            let Some(node) = value.as_object() else {
                self.error(&pointer, "node is not a JSON object");
                continue;
            };
            self.check_node(&pointer, node, parent);
            let node_type = node.get("type").and_then(Value::as_str);
            if let Some(Value::Array(children)) = node.get("content") {
                for (i, child) in children.iter().enumerate().rev() {
                    stack.push((
                        format!("{pointer}/content/{i}"),
                        child,
                        node_type,
                        depth + 1,
                    ));
                }
            }
        }
    }

    /// No `null` anywhere: optional attributes are omitted, never null.
    fn check_no_nulls(&mut self, doc: &Value) {
        let mut stack: Vec<(String, &Value, usize)> = vec![(String::new(), doc, 0)];
        while let Some((pointer, value, depth)) = stack.pop() {
            if depth > MAX_WALK_DEPTH {
                continue;
            }
            match value {
                Value::Null => self.error(&pointer, "null value"),
                Value::Array(items) => {
                    for (i, item) in items.iter().enumerate() {
                        stack.push((format!("{pointer}/{i}"), item, depth + 1));
                    }
                }
                Value::Object(object) => {
                    for (key, item) in object {
                        stack.push((
                            format!("{pointer}/{}", escape_pointer(key)),
                            item,
                            depth + 1,
                        ));
                    }
                }
                Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
    }

    fn check_node(&mut self, pointer: &str, node: &Map<String, Value>, parent: Option<&str>) {
        let node_type = node.get("type").and_then(Value::as_str).unwrap_or_default();
        if FORBIDDEN_NODES.contains(&node_type) {
            self.error(pointer, format!("`{node_type}` is not Jira-safe"));
        }
        if let Some(marks) = node.get("marks") {
            match marks.as_array() {
                Some(marks) if marks.is_empty() => self.error(pointer, "`marks: []`"),
                Some(marks) => self.check_marks(pointer, node_type, marks),
                None => self.error(pointer, "`marks` is not an array"),
            }
        }
        if let Some(Value::Array(content)) = node.get("content") {
            if content.is_empty() && node_type != "doc" {
                self.error(pointer, format!("`{node_type}` has `content: []`"));
            }
            self.check_siblings(pointer, node_type, content);
        }
        match node_type {
            "text" => self.check_text(pointer, node, parent),
            "hardBreak" => self.check_hard_break(pointer, node),
            "listItem" => self.check_list_item(pointer, node),
            "taskList" | "taskItem" => self.check_task(pointer, node_type, node),
            "table" => self.check_table(pointer, node),
            "heading" => self.check_integer_attr(pointer, node, "level", 1, 6),
            "orderedList" => self.check_integer_attr(pointer, node, "order", 0, u64::MAX),
            "status" => self.check_status(pointer, node),
            "codeBlock" => self.check_forbidden_attrs(pointer, node, &["wrap", "hideLineNumbers"]),
            "tableCell" | "tableHeader" => self.check_forbidden_attrs(pointer, node, &["valign"]),
            "inlineCard" => {
                let url = attr_str(node, "url").unwrap_or_default().to_owned();
                if !is_allowed_url(&url) {
                    self.error(pointer, format!("inlineCard url `{url}` is not allowed"));
                }
            }
            _ => {}
        }
    }

    fn check_marks(&mut self, pointer: &str, node_type: &str, marks: &[Value]) {
        let types: Vec<&str> = marks
            .iter()
            .map(|m| m.get("type").and_then(Value::as_str).unwrap_or_default())
            .collect();
        for ty in &types {
            if FORBIDDEN_MARKS.contains(ty) {
                self.error(pointer, format!("mark `{ty}` is not Jira-safe"));
            }
        }
        let mut seen: Vec<&str> = Vec::new();
        for ty in &types {
            if seen.contains(ty) {
                self.error(pointer, format!("duplicate mark `{ty}`"));
            }
            seen.push(ty);
        }
        if node_type != "text" {
            return;
        }
        if types.contains(&"code") && types.iter().any(|t| *t != "code" && *t != "link") {
            self.error(pointer, format!("code combined with {types:?}"));
        }
        let ranks: Vec<Option<usize>> = types
            .iter()
            .map(|t| MARK_ORDER.iter().position(|o| o == t))
            .collect();
        if ranks.windows(2).any(|w| match (w.first(), w.get(1)) {
            (Some(Some(a)), Some(Some(b))) => a > b,
            _ => false,
        }) {
            self.error(pointer, format!("marks not in canonical order: {types:?}"));
        }
        for mark in marks {
            if mark.get("type").and_then(Value::as_str) == Some("link") {
                let href = mark
                    .pointer("/attrs/href")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !is_allowed_url(href) {
                    self.error(pointer, format!("link href `{href}` is not allowed"));
                }
            }
        }
    }

    fn check_siblings(&mut self, pointer: &str, node_type: &str, content: &[Value]) {
        for (i, pair) in content.windows(2).enumerate() {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            if a.get("type").and_then(Value::as_str) == Some("text")
                && b.get("type").and_then(Value::as_str) == Some("text")
                && a.get("marks") == b.get("marks")
            {
                self.error(
                    &format!("{pointer}/content/{}", i + 1),
                    "adjacent text nodes with the same marks are not merged",
                );
            }
        }
        if node_type == "paragraph"
            && content
                .last()
                .and_then(|n| n.get("type"))
                .and_then(Value::as_str)
                == Some("hardBreak")
        {
            self.error(pointer, "paragraph ends with a hardBreak");
        }
        if node_type != "doc" {
            return;
        }
        for (i, child) in content.iter().enumerate() {
            let ty = child
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if INLINE_NODES.contains(&ty) {
                self.error(
                    &format!("{pointer}/content/{i}"),
                    "inline node in the document",
                );
            }
        }
    }

    fn check_text(&mut self, pointer: &str, node: &Map<String, Value>, parent: Option<&str>) {
        let text = node.get("text").and_then(Value::as_str).unwrap_or_default();
        if text.is_empty() {
            self.error(pointer, "empty text node");
        }
        if node.contains_key("attrs") {
            self.error(pointer, "text node with attrs");
        }
        if parent != Some("codeBlock") && text.chars().any(|c| c.is_control() && c != '\t') {
            self.error(
                pointer,
                format!("control character in inline text {text:?}"),
            );
        }
    }

    fn check_hard_break(&mut self, pointer: &str, node: &Map<String, Value>) {
        if node.contains_key("marks") {
            self.error(pointer, "hardBreak with marks");
        }
        if let Some(attrs) = node.get("attrs")
            && (attrs.get("text").and_then(Value::as_str) != Some("\n")
                || attrs.as_object().is_some_and(|a| a.len() != 1))
        {
            self.error(pointer, format!("hardBreak attrs {attrs}"));
        }
    }

    fn check_list_item(&mut self, pointer: &str, node: &Map<String, Value>) {
        let first = node
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("type"))
            .and_then(Value::as_str);
        if first != Some("paragraph") {
            self.error(
                pointer,
                format!("listItem starts with {first:?}, not a paragraph"),
            );
        }
    }

    fn check_task(&mut self, pointer: &str, node_type: &str, node: &Map<String, Value>) {
        match attr_str(node, "localId") {
            Some(id) if !id.is_empty() => {
                self.task_ids
                    .entry(id.to_owned())
                    .or_default()
                    .push(pointer.to_owned());
            }
            _ => self.error(pointer, format!("{node_type} without a localId")),
        }
        if node_type == "taskItem" {
            let state = attr_str(node, "state").unwrap_or_default();
            if state != "TODO" && state != "DONE" {
                self.error(pointer, format!("taskItem state `{state}`"));
            }
        } else {
            let first = node
                .get("content")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("type"))
                .and_then(Value::as_str);
            if first != Some("taskItem") {
                self.error(pointer, format!("taskList starts with {first:?}"));
            }
        }
    }

    fn check_table(&mut self, pointer: &str, node: &Map<String, Value>) {
        let Some(rows) = node.get("content").and_then(Value::as_array) else {
            return;
        };
        let mut widths = Vec::new();
        for row in rows {
            let cells = row.get("content").and_then(Value::as_array);
            let mut width = 0_u64;
            for cell in cells.into_iter().flatten() {
                if cell
                    .pointer("/attrs/rowspan")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    > 1
                {
                    return; // row spans make the width check meaningless
                }
                width += cell
                    .pointer("/attrs/colspan")
                    .and_then(Value::as_u64)
                    .unwrap_or(1);
            }
            widths.push(width);
        }
        if widths.windows(2).any(|w| w.first() != w.get(1)) {
            self.error(
                pointer,
                format!("table rows are not rectangular: {widths:?}"),
            );
        }
    }

    fn check_integer_attr(
        &mut self,
        pointer: &str,
        node: &Map<String, Value>,
        name: &str,
        min: u64,
        max: u64,
    ) {
        match node.get("attrs").and_then(|a| a.get(name)) {
            Some(value) => match value.as_u64() {
                Some(n) if (min..=max).contains(&n) => {}
                _ => self.error(
                    pointer,
                    format!("`{name}` is {value}, not an integer in range"),
                ),
            },
            None => self.error(pointer, format!("missing `{name}`")),
        }
    }

    fn check_status(&mut self, pointer: &str, node: &Map<String, Value>) {
        let color = attr_str(node, "color").unwrap_or_default();
        if !STATUS_COLORS.contains(&color) {
            self.error(
                pointer,
                format!("status colour `{color}` is not a named colour"),
            );
        }
        if attr_str(node, "text").unwrap_or_default().is_empty() {
            self.error(pointer, "status with empty text");
        }
    }

    fn check_forbidden_attrs(&mut self, pointer: &str, node: &Map<String, Value>, names: &[&str]) {
        for name in names {
            if node.get("attrs").and_then(|a| a.get(*name)).is_some() {
                self.error(pointer, format!("attribute `{name}` is not Jira-safe"));
            }
        }
    }

    fn check_local_ids(&mut self) {
        let duplicates: Vec<(String, Vec<String>)> = self
            .task_ids
            .iter()
            .filter(|(_, pointers)| pointers.len() > 1)
            .map(|(id, pointers)| (id.clone(), pointers.clone()))
            .collect();
        for (id, pointers) in duplicates {
            self.error(
                pointers.first().map_or("", String::as_str),
                format!(
                    "localId `{id}` is used by {} task nodes: {pointers:?}",
                    pointers.len()
                ),
            );
        }
    }
}

fn attr_str<'a>(node: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    node.get("attrs")?.get(name)?.as_str()
}

fn escape_pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// The URL allowlist from `docs/syntax.md`, written independently of `Href::parse`.
pub fn is_allowed_url(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() || url.chars().any(char::is_control) {
        return false;
    }
    if url.starts_with('#') || (url.starts_with('/') && !url.starts_with("//")) {
        return true;
    }
    let lower = url.to_ascii_lowercase();
    for scheme in ["http://", "https://", "ftp://", "ftps://"] {
        if let Some(rest) = lower.strip_prefix(scheme) {
            return !rest.is_empty();
        }
    }
    ["mailto:", "tel:"].iter().any(|scheme| {
        lower
            .strip_prefix(scheme)
            .is_some_and(|rest| !rest.is_empty())
    })
}
