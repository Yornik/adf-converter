//! Raw ```` ```adf ```` blocks for nodes Markdown cannot express (`W_ADF_PASSTHROUGH`).
//!
//! The JSON is the node as it was, pretty-printed in its original key order, except that
//! optional `localId`s are removed (they are editor bookkeeping and `normalize` drops them).
//! `localId`s the schema requires (task and decision lists, sync blocks, fragments) are kept.

use serde_json::{Map, Value};

use super::Fail;
use super::escape::longest_run;
use crate::adf::reader::Node;

/// Node and mark types whose `localId` is required by the schema.
const LOCAL_ID_REQUIRED: &[&str] = &[
    "taskList",
    "taskItem",
    "blockTaskItem",
    "decisionList",
    "decisionItem",
    "syncBlock",
    "bodiedSyncBlock",
    "fragment",
];

/// A fenced ```` ```adf ```` block holding `nodes` (one object, or an array of several).
pub(super) fn raw_block(nodes: &[&Node<'_>], max_depth: usize) -> Result<String, Fail> {
    let mut values = Vec::with_capacity(nodes.len());
    for node in nodes {
        values.push(strip_local_ids(node.value, 0, max_depth)?);
    }
    let value = match <[Value; 1]>::try_from(values) {
        Ok([single]) => single,
        Err(values) => Value::Array(values),
    };
    let json = serde_json::to_string_pretty(&value)
        .map_err(|e| Fail::internal(format!("could not serialise a raw ADF block: {e}")))?;
    let fence = "`".repeat((longest_run(&json, '`') + 1).max(3));
    Ok(format!("{fence}adf\n{json}\n{fence}"))
}

fn strip_local_ids(value: &Value, depth: usize, max_depth: usize) -> Result<Value, Fail> {
    if depth > max_depth {
        return Err(Fail::internal("raw ADF block nests deeper than the limit"));
    }
    Ok(match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| strip_local_ids(item, depth + 1, max_depth))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => {
            let keep_local_id = map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| LOCAL_ID_REQUIRED.contains(&t));
            let mut out = Map::new();
            for (key, item) in map {
                if key == "attrs"
                    && !keep_local_id
                    && let Value::Object(attrs) = item
                {
                    let mut kept = Map::new();
                    for (name, attr) in attrs {
                        if name != "localId" {
                            kept.insert(name.clone(), strip_local_ids(attr, depth + 2, max_depth)?);
                        }
                    }
                    out.insert(key.clone(), Value::Object(kept));
                    continue;
                }
                out.insert(key.clone(), strip_local_ids(item, depth + 1, max_depth)?);
            }
            Value::Object(out)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adf::reader::read_doc;
    use crate::limits::Limits;
    use serde_json::json;

    #[test]
    fn strips_optional_local_ids_only() {
        let doc = json!({"type": "doc", "content": [
            {"type": "decisionList", "attrs": {"localId": "d1"}, "content": [
                {"type": "decisionItem", "attrs": {"localId": "d2", "state": "DECIDED"}, "content": [
                    {"type": "status", "attrs": {"text": "X", "color": "blue", "localId": "s"}}
                ]}
            ]},
            {"type": "rule", "attrs": {"localId": "r"}}
        ]});
        let root = read_doc(&doc, &Limits::default()).unwrap();
        let nodes: Vec<&Node<'_>> = root.content.iter().collect();
        let md = raw_block(&nodes, 64).unwrap();
        assert!(md.starts_with("```adf\n[\n"), "{md}");
        assert!(md.ends_with("\n```"), "{md}");
        let body = md.trim_start_matches("```adf\n").trim_end_matches("\n```");
        let value: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            value,
            json!([
                {"type": "decisionList", "attrs": {"localId": "d1"}, "content": [
                    {"type": "decisionItem", "attrs": {"localId": "d2", "state": "DECIDED"}, "content": [
                        {"type": "status", "attrs": {"text": "X", "color": "blue"}}
                    ]}
                ]},
                {"type": "rule", "attrs": {}}
            ])
        );
    }

    #[test]
    fn single_node_is_an_object_and_keeps_key_order() {
        let doc = json!({"type": "doc", "content": [
            {"type": "extension", "attrs": {"extensionType": "t", "extensionKey": "k"}}
        ]});
        let root = read_doc(&doc, &Limits::default()).unwrap();
        let md = raw_block(&[&root.content[0]], 64).unwrap();
        assert_eq!(
            md,
            "```adf\n{\n  \"type\": \"extension\",\n  \"attrs\": {\n    \"extensionType\": \"t\",\n    \"extensionKey\": \"k\"\n  }\n}\n```"
        );
    }
}
