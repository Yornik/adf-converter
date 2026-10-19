//! Raw ```` ```adf ```` blocks for nodes Markdown cannot express (`W_ADF_PASSTHROUGH`).
//!
//! The JSON is the node as it was, pretty-printed in its original key order (compact below
//! [`PRETTY_LEVELS`] levels), except that optional `localId`s are removed (they are editor
//! bookkeeping and `normalize` drops them). `localId`s the schema requires (task and decision
//! lists, sync blocks, fragments) are kept.

use std::io;

use serde::ser::{Error as _, Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_json::{Map, Value};

use super::escape::longest_run;
use super::{Ctx, Fail};
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

/// JSON levels below a raw block's top that are pretty-printed. Deeper values are written
/// compactly: indentation grows by two spaces per level, so a deeply nested value with many
/// small leaves would otherwise make the Markdown hundreds of times larger than the input.
/// Sixteen levels show a table cell's paragraphs, or a list in a layout column, in full.
const PRETTY_LEVELS: usize = 16;

/// A fenced ```` ```adf ```` block holding `nodes` (one object, or an array of several).
///
/// The JSON is written once, straight into the output, and fails with `E_INPUT_TOO_LARGE` as
/// soon as it would exceed the output budget.
pub(super) fn raw_block(nodes: &[&Node<'_>], cx: &Ctx) -> Result<String, Fail> {
    let Some(first) = nodes.first() else {
        return Ok(String::new());
    };
    let values: Vec<Stripped<'_>> = nodes
        .iter()
        .map(|node| Stripped {
            value: node.value,
            depth: 0,
            max_depth: cx.max_depth,
        })
        .collect();
    let longest = nodes
        .iter()
        .map(|node| longest_backtick_run(node.value))
        .max()
        .unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    let mut writer = Budget {
        buf: Vec::new(),
        max: cx.max_output,
    };
    writer.buf.extend_from_slice(fence.as_bytes());
    writer.buf.extend_from_slice(b"adf\n");
    let mut serializer = serde_json::Serializer::with_formatter(&mut writer, Formatter::default());
    let written = match values.as_slice() {
        [single] => single.serialize(&mut serializer),
        several => several.serialize(&mut serializer),
    };
    if let Err(error) = written {
        return Err(if error.is_io() {
            cx.check_output(usize::MAX, first)
                .err()
                .unwrap_or_else(|| Fail::internal("a raw ADF block exceeded the output limit"))
        } else {
            Fail::internal(format!("could not serialise a raw ADF block: {error}"))
        });
    }
    let mut buf = writer.buf;
    buf.push(b'\n');
    buf.extend_from_slice(fence.as_bytes());
    String::from_utf8(buf).map_err(|e| Fail::internal(format!("a raw ADF block is not UTF-8: {e}")))
}

/// Collects serialised JSON, failing once it grows beyond `max` bytes.
struct Budget {
    buf: Vec<u8>,
    max: usize,
}

impl io::Write for Budget {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.buf.len().saturating_add(data.len()) > self.max {
            return Err(io::Error::other("the output limit was reached"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The longest run of backticks in any string of `value` (`serde_json` writes backticks as they
/// are, and only inside strings, so this decides the fence length). Walks without recursion.
fn longest_backtick_run(value: &Value) -> usize {
    let mut longest = 0;
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::String(s) => longest = longest.max(longest_run(s, '`')),
            Value::Array(items) => stack.extend(items),
            Value::Object(map) => {
                for (key, item) in map {
                    longest = longest.max(longest_run(key, '`'));
                    stack.push(item);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    longest
}

/// A node's JSON as written in a raw block: the value without optional `localId`s, serialised
/// in place (no copy of the tree).
struct Stripped<'v> {
    value: &'v Value,
    depth: usize,
    max_depth: usize,
}

impl Stripped<'_> {
    fn child<'c>(&self, value: &'c Value, levels: usize) -> Stripped<'c> {
        Stripped {
            value,
            depth: self.depth + levels,
            max_depth: self.max_depth,
        }
    }
}

impl Serialize for Stripped<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.depth > self.max_depth {
            return Err(S::Error::custom(
                "raw ADF block nests deeper than the limit",
            ));
        }
        match self.value {
            Value::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(&self.child(item, 1))?;
                }
                seq.end()
            }
            Value::Object(map) => {
                let keep_local_id = map
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|t| LOCAL_ID_REQUIRED.contains(&t));
                let mut out = serializer.serialize_map(Some(map.len()))?;
                for (key, item) in map {
                    if key == "attrs"
                        && !keep_local_id
                        && let Value::Object(attrs) = item
                    {
                        out.serialize_entry(
                            key,
                            &Attrs {
                                attrs,
                                parent: self.child(item, 1),
                            },
                        )?;
                    } else {
                        out.serialize_entry(key, &self.child(item, 1))?;
                    }
                }
                out.end()
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                self.value.serialize(serializer)
            }
        }
    }
}

/// An `attrs` object without its `localId`.
struct Attrs<'v> {
    attrs: &'v Map<String, Value>,
    parent: Stripped<'v>,
}

impl Serialize for Attrs<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let kept = self.attrs.iter().filter(|(name, _)| *name != "localId");
        let mut out = serializer.serialize_map(Some(kept.clone().count()))?;
        for (name, attr) in kept {
            out.serialize_entry(name, &self.parent.child(attr, 1))?;
        }
        out.end()
    }
}

/// `serde_json`'s pretty format (two-space indent) for the first [`PRETTY_LEVELS`] levels,
/// compact below.
#[derive(Debug, Default)]
struct Formatter {
    /// Nesting level of the container being written (0 outside any).
    level: usize,
    has_value: bool,
}

impl Formatter {
    fn pretty(&self) -> bool {
        self.level <= PRETTY_LEVELS
    }

    fn indent<W: ?Sized + io::Write>(writer: &mut W, level: usize) -> io::Result<()> {
        for _ in 0..level {
            writer.write_all(b"  ")?;
        }
        Ok(())
    }

    fn begin<W: ?Sized + io::Write>(&mut self, writer: &mut W, open: &[u8]) -> io::Result<()> {
        self.level += 1;
        self.has_value = false;
        writer.write_all(open)
    }

    fn end<W: ?Sized + io::Write>(&mut self, writer: &mut W, close: &[u8]) -> io::Result<()> {
        let pretty = self.pretty();
        self.level = self.level.saturating_sub(1);
        if pretty && self.has_value {
            writer.write_all(b"\n")?;
            Self::indent(writer, self.level)?;
        }
        writer.write_all(close)
    }

    fn begin_value<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if !first {
            writer.write_all(b",")?;
        }
        if self.pretty() {
            writer.write_all(b"\n")?;
            Self::indent(writer, self.level)?;
        }
        Ok(())
    }
}

impl serde_json::ser::Formatter for Formatter {
    fn begin_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.begin(writer, b"[")
    }

    fn end_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.end(writer, b"]")
    }

    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.begin_value(writer, first)
    }

    fn end_array_value<W: ?Sized + io::Write>(&mut self, _writer: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }

    fn begin_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.begin(writer, b"{")
    }

    fn end_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.end(writer, b"}")
    }

    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.begin_value(writer, first)
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(if self.pretty() { b": " } else { b":" })
    }

    fn end_object_value<W: ?Sized + io::Write>(&mut self, _writer: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }

    /// `serde_json` escapes C0 controls itself; DEL and C1 controls (U+009B is CSI) are escaped
    /// here, so no control character reaches the terminal raw.
    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        let mut start = 0;
        for (i, c) in fragment.char_indices() {
            if c.is_control() {
                writer.write_all(fragment.get(start..i).unwrap_or_default().as_bytes())?;
                write!(writer, "\\u{:04x}", u32::from(c))?;
                start = i + c.len_utf8();
            }
        }
        writer.write_all(fragment.get(start..).unwrap_or_default().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adf::reader::read_doc;
    use crate::diagnostics::Code;
    use crate::limits::Limits;
    use serde_json::json;

    fn cx(max_output: usize) -> Ctx {
        Ctx {
            roundtrip: true,
            max_depth: 512,
            depth: 0,
            max_output,
            table_padding: 0,
            warnings: Vec::new(),
            unlisted: Vec::new(),
        }
    }

    fn block_of(node: &Value, max_output: usize) -> Result<String, Fail> {
        let doc = json!({"type": "doc", "content": [node]});
        let root = read_doc(&doc, &Limits::default()).unwrap();
        raw_block(&[&root.content[0]], &cx(max_output))
    }

    fn body(md: &str) -> Value {
        use serde::Deserialize as _;
        let json = md.trim_start_matches("```adf\n").trim_end_matches("\n```");
        let mut de = serde_json::Deserializer::from_str(json);
        de.disable_recursion_limit();
        Value::deserialize(&mut de).unwrap()
    }

    /// `[[[…[1, 1, …]…]]]` nested `depth` arrays deep.
    fn nested(depth: usize, leaves: usize) -> Value {
        let mut value = Value::Array(vec![json!(1); leaves]);
        for _ in 1..depth {
            value = Value::Array(vec![value]);
        }
        value
    }

    #[test]
    fn shallow_nodes_are_pretty_printed_like_serde_json() {
        let node = json!({"type": "panel", "attrs": {"panelType": "custom", "panelColor": "#fff"},
            "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a`b",
                "marks": [{"type": "link", "attrs": {"href": "https://e.com"}}]}]}]});
        let md = block_of(&node, usize::MAX).unwrap();
        let pretty = serde_json::to_string_pretty(&node).unwrap();
        assert_eq!(md, format!("```adf\n{pretty}\n```"));
    }

    #[test]
    fn deep_values_are_compact_below_the_pretty_levels() {
        let params = nested(400, 2_000);
        let node = json!({"type": "extension", "attrs": {"extensionType": "t", "extensionKey": "k", "parameters": params}});
        let md = block_of(&node, usize::MAX).unwrap();
        assert_eq!(body(&md), node);
        let compact = serde_json::to_string(&node).unwrap();
        // Pretty-printing all 400 levels would add about 800 bytes of indentation per leaf.
        assert!(
            md.len() < compact.len() + 4_000,
            "{} vs {}",
            md.len(),
            compact.len()
        );
        let deepest_indent = md
            .lines()
            .map(|l| l.len() - l.trim_start().len())
            .max()
            .unwrap();
        assert_eq!(deepest_indent, 2 * PRETTY_LEVELS);
    }

    #[test]
    fn fence_outgrows_backtick_runs_in_keys_and_values() {
        let node = json!({"type": "x", "attrs": {"a````b": "``` c"}});
        let md = block_of(&node, usize::MAX).unwrap();
        assert!(md.starts_with("`````adf\n"), "{md}");
        assert!(md.ends_with("\n`````"), "{md}");
    }

    #[test]
    fn output_beyond_the_budget_is_too_large() {
        let node = json!({"type": "x", "attrs": {"data": "y".repeat(10_000)}});
        assert!(block_of(&node, 20_000).is_ok());
        match block_of(&node, 5_000) {
            Err(Fail::Fatal(d)) => {
                assert_eq!(d.code, Code::InputTooLarge);
                assert_eq!(
                    d.location.and_then(|l| l.pointer).as_deref(),
                    Some("/content/0")
                );
            }
            other => panic!("{other:?}"),
        }
    }

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
        let md = raw_block(&nodes, &cx(usize::MAX)).unwrap();
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
        let md = raw_block(&[&root.content[0]], &cx(usize::MAX)).unwrap();
        assert_eq!(
            md,
            "```adf\n{\n  \"type\": \"extension\",\n  \"attrs\": {\n    \"extensionType\": \"t\",\n    \"extensionKey\": \"k\"\n  }\n}\n```"
        );
    }
}
