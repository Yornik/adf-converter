//! `normalize()` from `docs/syntax.md`, section "Round-trip guarantee".
//!
//! For a document `x` that only uses constructs with a round-trip mapping,
//! `md_to_adf(adf_to_md(x)) == normalize(x)`, where `normalize`:
//!
//! - removes all `localId` attributes, except on task lists and items, which are compared by
//!   position (here: replaced by their ordinal in document order);
//! - splits `\n` inside text nodes into `hardBreak`s (code blocks keep their newlines);
//! - drops the attributes listed under `W_ATTR_DROPPED`: cell `colwidth` and `background`, table
//!   `width`/`layout`/`displayMode`, a numbered column, the size of external media (file media
//!   keeps `width`, `height` and `size`, which `adf:media` carries), panel icons. Tables get the
//!   attributes Markdown → ADF always writes (`isNumberColumnEnabled: false`, `layout: "default"`).
//! - gives a `mediaSingle` without `layout` the layout `center`, removes `widthType: "percentage"`
//!   (the default, which `size=<n>%` leaves out) and an empty `alt` of a `media`.
//!
//! The function is idempotent, so the tests apply it to both sides of the comparison (the left side
//! carries freshly generated task `localId`s).
//!
//! This file depends only on `serde_json`, so the fuzz targets include it with `#[path]`.

use serde_json::{Map, Value};

/// Attributes without a Markdown form (`W_ATTR_DROPPED`), per node type. `table` is handled
/// separately because Markdown → ADF always writes its attributes.
const DROPPED_ATTRS: &[(&str, &[&str])] = &[
    ("tableCell", &["colwidth", "background"]),
    ("tableHeader", &["colwidth", "background"]),
    ("mediaSingle", &["width", "widthType"]),
    ("media", &["width", "height"]),
    (
        "panel",
        &["panelIcon", "panelIconId", "panelIconText", "panelColor"],
    ),
];

/// Nodes whose `attrs` object Markdown → ADF always writes, even when it is empty.
const ATTRS_ALWAYS_PRESENT: &[&str] = &["expand", "nestedExpand"];

/// Nesting deeper than this is left untouched instead of recursing further. Documents produced by
/// the converter are far shallower (the nesting limit is 64 containers).
const MAX_DEPTH: usize = 4096;

/// Returns the normalized copy of an ADF document (or of any ADF node).
pub fn normalize(doc: &Value) -> Value {
    let mut out = doc.clone();
    let mut task_ordinal = 0_usize;
    normalize_node(&mut out, &mut task_ordinal, 0);
    sort_keys(&mut out, 0);
    out
}

/// Orders the keys of every object alphabetically: the order of keys in ADF carries no meaning
/// (Jira and Confluence write them in any order), but the comparison is textual.
fn sort_keys(node: &mut Value, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match node {
        Value::Object(object) => {
            let mut entries: Vec<(String, Value)> = std::mem::take(object).into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (_, value) in &mut entries {
                sort_keys(value, depth + 1);
            }
            object.extend(entries);
        }
        Value::Array(items) => {
            for item in items {
                sort_keys(item, depth + 1);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn normalize_node(node: &mut Value, task_ordinal: &mut usize, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(object) = node.as_object_mut() else {
        if let Some(items) = node.as_array_mut() {
            for item in items {
                normalize_node(item, task_ordinal, depth + 1);
            }
        }
        return;
    };
    let node_type = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    normalize_attrs(object, &node_type, task_ordinal);

    if node_type != "codeBlock"
        && let Some(Value::Array(children)) = object.get_mut("content")
    {
        split_newlines(children);
    }
    if let Some(Value::Array(marks)) = object.get_mut("marks") {
        for mark in marks.iter_mut() {
            if let Some(mark) = mark.as_object_mut() {
                remove_local_id(mark);
            }
        }
    }
    if let Some(content) = object.get_mut("content") {
        normalize_node(content, task_ordinal, depth + 1);
    }
}

fn normalize_attrs(object: &mut Map<String, Value>, node_type: &str, task_ordinal: &mut usize) {
    if node_type == "table" {
        let mut attrs = Map::new();
        attrs.insert("isNumberColumnEnabled".to_owned(), Value::Bool(false));
        attrs.insert("layout".to_owned(), Value::String("default".to_owned()));
        object.insert("attrs".to_owned(), Value::Object(attrs));
        return;
    }
    let is_file = is_file_media(object, node_type);
    if node_type == "mediaSingle" && !object.contains_key("attrs") {
        object.insert("attrs".to_owned(), Value::Object(Map::new()));
    }
    let Some(Value::Object(attrs)) = object.get_mut("attrs") else {
        return;
    };
    if node_type == "taskList" || node_type == "taskItem" {
        *task_ordinal += 1;
        if attrs.contains_key("localId") {
            attrs.insert(
                "localId".to_owned(),
                Value::String(format!("task-{task_ordinal}")),
            );
        }
        return;
    }
    attrs.remove("localId");
    // File media keeps its size (`adf:media` carries it); only external media drops it.
    if !is_file {
        for (ty, names) in DROPPED_ATTRS {
            if *ty == node_type {
                for name in *names {
                    attrs.remove(*name);
                }
            }
        }
    }
    if node_type == "media" && attrs.get("alt").and_then(Value::as_str) == Some("") {
        attrs.remove("alt");
    }
    if node_type == "mediaSingle" {
        // `size=<n>%` writes no `widthType`, and an absent layout reads back as `center`.
        if attrs.get("widthType").and_then(Value::as_str) == Some("percentage") {
            attrs.remove("widthType");
        }
        attrs
            .entry("layout")
            .or_insert_with(|| Value::String("center".to_owned()));
    }
    // Markdown → ADF never writes an empty `attrs` object, except on expands.
    if attrs.is_empty() && !ATTRS_ALWAYS_PRESENT.contains(&node_type) {
        object.remove("attrs");
    }
}

/// Whether the node is a `media` of type `file`, or a `mediaSingle` whose first child is one.
fn is_file_media(object: &Map<String, Value>, node_type: &str) -> bool {
    let media_type = |node: &Map<String, Value>| {
        node.get("attrs")
            .and_then(|a| a.get("type"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    match node_type {
        "media" => media_type(object).as_deref() == Some("file"),
        "mediaSingle" => object
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(Value::as_object)
            .is_some_and(|child| {
                child.get("type").and_then(Value::as_str) == Some("media")
                    && media_type(child).as_deref() == Some("file")
            }),
        _ => false,
    }
}

fn remove_local_id(mark: &mut Map<String, Value>) {
    if let Some(Value::Object(attrs)) = mark.get_mut("attrs") {
        attrs.remove("localId");
        if attrs.is_empty() {
            mark.remove("attrs");
        }
    }
}

/// Replaces each text node containing `\n` by text segments separated by `hardBreak`s. Empty
/// segments are dropped; every segment keeps the node's marks.
fn split_newlines(children: &mut Vec<Value>) {
    let needs_split = children.iter().any(|child| {
        child.get("type").and_then(Value::as_str) == Some("text")
            && child
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains('\n'))
    });
    if !needs_split {
        return;
    }
    let mut out = Vec::with_capacity(children.len() + 2);
    for child in children.drain(..) {
        let text = match (
            child.get("type").and_then(Value::as_str),
            child.get("text").and_then(Value::as_str),
        ) {
            (Some("text"), Some(text)) if text.contains('\n') => text.to_owned(),
            _ => {
                out.push(child);
                continue;
            }
        };
        for (i, segment) in text.split('\n').enumerate() {
            if i > 0 {
                out.push(hard_break());
            }
            if !segment.is_empty() {
                let mut piece = child.clone();
                if let Some(object) = piece.as_object_mut() {
                    object.insert("text".to_owned(), Value::String(segment.to_owned()));
                }
                out.push(piece);
            }
        }
    }
    *children = out;
}

fn hard_break() -> Value {
    let mut object = Map::new();
    object.insert("type".to_owned(), Value::String("hardBreak".to_owned()));
    Value::Object(object)
}
