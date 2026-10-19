//! End-to-end tests of Markdown → ADF: the exact JSON of each construct, schema validity,
//! diagnostics, limits and determinism.
//!
//! Every conversion through [`run`] is also checked against the vendored schema (with
//! `jsonschema::draft4` directly), converted twice to check determinism, and converted again
//! with CRLF line endings, which must give byte-identical output.

// JSON builder helpers take `Value`s by value so expectations read naturally.
#![allow(clippy::too_many_lines, clippy::needless_pass_by_value)]

use std::sync::OnceLock;

use serde_json::{Value, json};

use crate::diagnostics::{Code, Diagnostic};
use crate::limits::Limits;
use crate::options::{Extensions, HtmlPolicy, ImagePolicy, MdToAdfOptions, SoftBreak};

fn validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let schema: Value = serde_json::from_str(crate::ADF_SCHEMA_JSON).unwrap();
        jsonschema::draft4::new(&schema).unwrap()
    })
}

fn assert_schema_valid(value: &Value) {
    let errors: Vec<String> = validator()
        .iter_errors(value)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{errors:#?}\n{}",
        serde_json::to_string_pretty(value).unwrap()
    );
}

/// Converts, checks schema validity, determinism and CRLF equivalence; returns the document
/// JSON and the warnings.
fn run_with(md: &str, opts: &MdToAdfOptions) -> (Value, Vec<Diagnostic>) {
    let conversion = super::to_adf(md, opts).unwrap_or_else(|e| panic!("{md:?}: {e}"));
    let value = conversion.output.to_value().unwrap();
    assert_schema_valid(&value);
    let text = serde_json::to_string(&value).unwrap();
    let again = super::to_adf(md, opts).unwrap();
    assert_eq!(
        serde_json::to_string(&again.output).unwrap(),
        text,
        "not deterministic"
    );
    assert_eq!(again.warnings, conversion.warnings);
    if !md.contains('\r') {
        let crlf = md.replace('\n', "\r\n");
        let crlf = super::to_adf(&crlf, opts).unwrap();
        assert_eq!(
            serde_json::to_string(&crlf.output).unwrap(),
            text,
            "CRLF differs"
        );
        assert_eq!(crlf.warnings, conversion.warnings, "CRLF warnings differ");
    }
    for warning in &conversion.warnings {
        let location = warning.location.as_ref().expect("warning without location");
        assert!(
            location.line.is_some() && location.column.is_some(),
            "{warning:?}"
        );
        assert!(!warning.is_error());
    }
    (value, conversion.warnings)
}

fn run(md: &str) -> (Value, Vec<Diagnostic>) {
    run_with(md, &MdToAdfOptions::default())
}

/// The document's `content` array.
fn content(md: &str) -> Value {
    run(md).0["content"].clone()
}

fn content_with(md: &str, opts: &MdToAdfOptions) -> Value {
    run_with(md, opts).0["content"].clone()
}

fn codes(warnings: &[Diagnostic]) -> Vec<Code> {
    warnings.iter().map(|w| w.code).collect()
}

fn error_of(md: &str, opts: &MdToAdfOptions) -> Diagnostic {
    let err = super::to_adf(md, opts).expect_err("expected an error");
    assert!(err.has_errors());
    err.0.into_iter().find(Diagnostic::is_error).unwrap()
}

fn t(s: &str) -> Value {
    json!({"type": "text", "text": s})
}

fn tm(s: &str, marks: Value) -> Value {
    json!({"type": "text", "text": s, "marks": marks})
}

fn p(inlines: Value) -> Value {
    json!({"type": "paragraph", "content": inlines})
}

fn pt(s: &str) -> Value {
    p(json!([t(s)]))
}

fn li(blocks: Value) -> Value {
    json!({"type": "listItem", "content": blocks})
}

fn bullet(items: Value) -> Value {
    json!({"type": "bulletList", "content": items})
}

fn ordered(order: u32, items: Value) -> Value {
    json!({"type": "orderedList", "attrs": {"order": order}, "content": items})
}

fn link(href: &str) -> Value {
    json!({"type": "link", "attrs": {"href": href}})
}

fn opts_with(f: impl FnOnce(&mut MdToAdfOptions)) -> MdToAdfOptions {
    let mut opts = MdToAdfOptions::default();
    f(&mut opts);
    opts
}

/// Replaces every `localId` with `"ID"` after checking they are unique UUID-shaped strings.
fn strip_ids(value: &mut Value) {
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![value];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(map) => {
                if let Some(Value::Object(attrs)) = map.get_mut("attrs")
                    && let Some(id) = attrs.get_mut("localId")
                {
                    let s = id.as_str().unwrap().to_owned();
                    assert_eq!(s.len(), 36, "{s}");
                    assert!(seen.insert(s.clone()), "duplicate localId {s}");
                    *id = json!("ID");
                }
                stack.extend(map.values_mut());
            }
            Value::Array(items) => stack.extend(items.iter_mut()),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}

fn task_list(items: Value) -> Value {
    json!({"type": "taskList", "attrs": {"localId": "ID"}, "content": items})
}

fn task(state: &str, inlines: Value) -> Value {
    json!({"type": "taskItem", "attrs": {"localId": "ID", "state": state}, "content": inlines})
}

fn content_no_ids(md: &str) -> Value {
    let mut c = content(md);
    strip_ids(&mut c);
    c
}

fn one_cell_box(blocks: Value) -> Value {
    json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"},
           "content": [{"type": "tableRow", "content": [{"type": "tableCell", "content": blocks}]}]})
}

// ---------------------------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------------------------

#[test]
fn headings_all_levels_and_setext() {
    for level in 1..=6u32 {
        let md = format!("{} Title *x*", "#".repeat(level as usize));
        assert_eq!(
            content(&md),
            json!([{"type": "heading", "attrs": {"level": level},
                    "content": [t("Title "), tm("x", json!([{"type": "em"}]))]}])
        );
    }
    assert_eq!(
        content("Big\n===\n\nSmall\n---\n"),
        json!([
            {"type": "heading", "attrs": {"level": 1}, "content": [t("Big")]},
            {"type": "heading", "attrs": {"level": 2}, "content": [t("Small")]}
        ])
    );
    // F20: closing hashes and extra spaces are not text.
    assert_eq!(
        content("###  spaced ###"),
        json!([{"type": "heading", "attrs": {"level": 3}, "content": [t("spaced")]}])
    );
    assert_eq!(
        content("#"),
        json!([{"type": "heading", "attrs": {"level": 1}}])
    );
    // F3: `## X` is a heading, not text.
    assert_eq!(content("## X")[0]["attrs"]["level"], 2);
}

#[test]
fn paragraphs_soft_and_hard_breaks() {
    // F4: lines of one paragraph join with a space; no paragraph per line.
    assert_eq!(
        content("line one\nline two"),
        json!([pt("line one line two")])
    );
    assert_eq!(
        content("a  \nb\\\nc"),
        json!([p(
            json!([t("a"), {"type": "hardBreak"}, t("b"), {"type": "hardBreak"}, t("c")])
        )])
    );
    let hard = opts_with(|o| o.softbreak = SoftBreak::HardBreak);
    assert_eq!(
        content_with("a\nb", &hard),
        json!([p(json!([t("a"), {"type": "hardBreak"}, t("b")]))])
    );
    // A trailing hard break is dropped.
    assert_eq!(content("a<br>"), json!([pt("a")]));
    assert_eq!(content("a\n\n\n\nb"), json!([pt("a"), pt("b")]));
    assert_eq!(content(""), json!([]));
    assert_eq!(content("\n\n  \n"), json!([]));
}

#[test]
fn bullet_and_ordered_lists() {
    let expected = json!([bullet(json!([li(json!([pt("a")])), li(json!([pt("b")]))]))]);
    assert_eq!(content("- a\n- b"), expected);
    // F15: other markers.
    assert_eq!(content("* a\n* b"), expected);
    assert_eq!(content("+ a\n+ b"), expected);
    assert_eq!(content("-\ta\n-\tb"), expected);
    assert_eq!(
        content("1) a\n2) b"),
        json!([ordered(
            1,
            json!([li(json!([pt("a")])), li(json!([pt("b")]))])
        )])
    );
    // F6: blank-line separated items stay one list; the start number is kept.
    assert_eq!(
        content("1. a\n\n2. b"),
        json!([ordered(
            1,
            json!([li(json!([pt("a")])), li(json!([pt("b")]))])
        )])
    );
    assert_eq!(
        content("3. x"),
        json!([ordered(3, json!([li(json!([pt("x")]))]))])
    );
    assert_eq!(content("0. x")[0]["attrs"]["order"], 0);
    // An empty item and a loose item with several paragraphs.
    assert_eq!(
        content("- \n- a\n\n  b"),
        json!([bullet(json!([
            li(json!([{"type": "paragraph"}])),
            li(json!([pt("a"), pt("b")]))
        ]))])
    );
}

#[test]
fn nested_lists_start_with_a_paragraph() {
    // F5: nesting and continuation lines.
    assert_eq!(
        content("- a\n  - b\n- one\n  cont"),
        json!([bullet(json!([
            li(json!([pt("a"), bullet(json!([li(json!([pt("b")]))]))])),
            li(json!([pt("one cont")]))
        ]))])
    );
    // An item that starts with a sublist gets an empty leading paragraph.
    assert_eq!(
        content("- - x"),
        json!([bullet(json!([li(json!([
            {"type": "paragraph"},
            bullet(json!([li(json!([pt("x")]))]))
        ]))]))])
    );
    assert_eq!(
        content("1. a\n   1. b"),
        json!([ordered(
            1,
            json!([li(json!([
                pt("a"),
                ordered(1, json!([li(json!([pt("b")]))]))
            ]))])
        )])
    );
}

#[test]
fn task_lists() {
    assert_eq!(
        content_no_ids("- [ ] todo\n- [x] done\n- [X] also"),
        json!([task_list(json!([
            task("TODO", json!([t("todo")])),
            task("DONE", json!([t("done")])),
            task("DONE", json!([t("also")]))
        ]))])
    );
    // Nested task lists stay nested, after their item.
    assert_eq!(
        content_no_ids("- [ ] a\n  - [x] b\n- [ ] c"),
        json!([task_list(json!([
            task("TODO", json!([t("a")])),
            task_list(json!([task("DONE", json!([t("b")]))])),
            task("TODO", json!([t("c")]))
        ]))])
    );
    // Several paragraphs are joined with a hard break.
    assert_eq!(
        content_no_ids("- [ ] a\n\n  b"),
        json!([task_list(json!([task(
            "TODO",
            json!([t("a"), {"type": "hardBreak"}, t("b")])
        )]))])
    );
    // An empty task item.
    assert_eq!(
        content_no_ids("- [ ] "),
        json!([task_list(json!([task("TODO", json!(null))]))])
            .as_array()
            .map(|_| content_no_ids("- [ ] "))
            .unwrap()
    );
}

#[test]
fn task_list_ids_are_deterministic_and_input_dependent() {
    let a = content("- [ ] x\n- [ ] y");
    let b = content("- [ ] x\n- [ ] y");
    let c = content("- [ ] x\n- [ ] z");
    assert_eq!(a, b);
    assert_ne!(a[0]["attrs"]["localId"], c[0]["attrs"]["localId"]);
    let mut ids = a.clone();
    strip_ids(&mut ids); // checks uniqueness
    // Version-8 UUID shape.
    let id = a[0]["attrs"]["localId"].as_str().unwrap();
    assert_eq!(&id[14..15], "8");
}

#[test]
fn mixed_and_ordered_task_lists() {
    let (mut value, warnings) = run("1. [ ] a\n2. b\n3. [x] c");
    strip_ids(&mut value);
    assert_eq!(
        value["content"],
        json!([
            task_list(json!([task("TODO", json!([t("a")]))])),
            ordered(2, json!([li(json!([pt("b")]))])),
            task_list(json!([task("DONE", json!([t("c")]))]))
        ])
    );
    assert_eq!(codes(&warnings), vec![Code::TaskNumberingLost]);
    assert_eq!(
        content_no_ids("- a\n- [ ] b"),
        json!([
            bullet(json!([li(json!([pt("a")]))])),
            task_list(json!([task("TODO", json!([t("b")]))]))
        ])
    );
}

#[test]
fn blocks_under_task_items_are_hoisted() {
    let md = "- [ ] task\n  ```\n  code\n  ```\n  - [ ] sub\n  - plain\n- [ ] next";
    let (mut value, warnings) = run(md);
    strip_ids(&mut value);
    assert_eq!(
        value["content"],
        json!([
            task_list(json!([
                task("TODO", json!([t("task")])),
                task_list(json!([task("TODO", json!([t("sub")]))]))
            ])),
            {"type": "codeBlock", "content": [t("code")]},
            bullet(json!([li(json!([pt("plain")]))])),
            task_list(json!([task("TODO", json!([t("next")]))]))
        ])
    );
    assert_eq!(
        codes(&warnings),
        vec![Code::BlockHoisted, Code::BlockHoisted]
    );
    // A heading in a task item becomes bold text (`- [ ] # x` is literal text in GFM).
    let (mut value, warnings) = run("- [ ] task\n\n  # Big");
    strip_ids(&mut value);
    assert_eq!(
        value["content"],
        json!([task_list(json!([task(
            "TODO",
            json!([t("task"), {"type": "hardBreak"}, tm("Big", json!([{"type": "strong"}]))])
        )]))])
    );
    assert_eq!(
        content_no_ids("- [ ] # x"),
        json!([task_list(json!([task("TODO", json!([t("# x")]))]))])
    );
    assert_eq!(codes(&warnings), vec![Code::HeadingDemoted]);
}

#[test]
fn code_blocks() {
    assert_eq!(
        content("```rust extra words\nfn main() {}\n```"),
        json!([{"type": "codeBlock", "attrs": {"language": "rust"}, "content": [t("fn main() {}")]}])
    );
    // F2: the body is never parsed as Markdown.
    assert_eq!(
        content("```\nx = 2**3 ** 4\n# not a heading\n```"),
        json!([{"type": "codeBlock", "content": [t("x = 2**3 ** 4\n# not a heading")]}])
    );
    assert_eq!(content("```\n```"), json!([{"type": "codeBlock"}]));
    // Only one trailing newline is stripped.
    assert_eq!(
        content("```\na\n\n```"),
        json!([{"type": "codeBlock", "content": [t("a\n")]}])
    );
    assert_eq!(
        content("    indented\n    more"),
        json!([{"type": "codeBlock", "content": [t("indented\nmore")]}])
    );
    assert_eq!(
        content("~~~ py\nx\n~~~"),
        json!([{"type": "codeBlock", "attrs": {"language": "py"}, "content": [t("x")]}])
    );
}

#[test]
fn blockquotes_and_rules() {
    assert_eq!(
        content("> a\n> b\n\n---\n\n***\n\n___"),
        json!([
            {"type": "blockquote", "content": [pt("a b")]},
            {"type": "rule"}, {"type": "rule"}, {"type": "rule"}
        ])
    );
    assert_eq!(
        content(">"),
        json!([{"type": "blockquote", "content": [{"type": "paragraph"}]}])
    );
    assert_eq!(
        content("> - a\n> ```\n> c\n> ```"),
        json!([{"type": "blockquote", "content": [
            bullet(json!([li(json!([pt("a")]))])),
            {"type": "codeBlock", "content": [t("c")]}
        ]}])
    );
}

#[test]
fn container_rules_in_quotes() {
    let (value, warnings) = run("> quote\n> > nested\n> # heading\n> ---\n> after");
    assert_eq!(
        value["content"],
        json!([
            {"type": "blockquote", "content": [
                pt("quote"), pt("nested"),
                p(json!([tm("heading", json!([{"type": "strong"}]))]))
            ]},
            {"type": "rule"},
            {"type": "blockquote", "content": [pt("after")]}
        ])
    );
    assert_eq!(
        codes(&warnings),
        vec![
            Code::BlockFlattened,
            Code::HeadingDemoted,
            Code::BlockHoisted
        ]
    );
    // A quote whose only content is hoisted disappears.
    assert_eq!(content("> ---"), json!([{"type": "rule"}]));
    // A task list in a quote is hoisted.
    let (mut value, warnings) = run("> - [ ] t");
    strip_ids(&mut value);
    assert_eq!(
        value["content"],
        json!([task_list(json!([task("TODO", json!([t("t")]))]))])
    );
    assert_eq!(codes(&warnings), vec![Code::BlockHoisted]);
    assert!(
        warnings[0].message.contains("task list"),
        "{}",
        warnings[0].message
    );
}

#[test]
fn container_rules_in_list_items() {
    let (value, warnings) = run("- # H\n- > q\n- a\n\n  ---\n\n  b");
    assert_eq!(
        value["content"],
        json!([
            bullet(json!([li(json!([p(json!([tm("H", json!([{"type": "strong"}]))]))]))])),
            {"type": "blockquote", "content": [pt("q")]},
            bullet(json!([li(json!([pt("a")]))])),
            {"type": "rule"},
            bullet(json!([li(json!([pt("b")]))]))
        ])
    );
    assert_eq!(
        codes(&warnings),
        vec![Code::HeadingDemoted, Code::BlockHoisted, Code::BlockHoisted]
    );
}

#[test]
fn reopened_ordered_list_continues_numbering() {
    // The table ends item 2; item 3 continues in a new list starting at 3.
    let md = "1. a\n2. b\n\n   | t |\n   |---|\n   | x |\n3. c";
    let c = content(md);
    assert_eq!(
        c[0],
        ordered(1, json!([li(json!([pt("a")])), li(json!([pt("b")]))]))
    );
    assert_eq!(c[1]["type"], "table");
    assert_eq!(c[2], ordered(3, json!([li(json!([pt("c")]))])));
    // Content of item 2 after the table continues as item 2.
    let md = "1. a\n2. b\n\n   ---\n\n   rest\n3. c";
    let c = content(md);
    assert_eq!(c[1], json!({"type": "rule"}));
    assert_eq!(
        c[2],
        ordered(2, json!([li(json!([pt("rest")])), li(json!([pt("c")]))]))
    );
}

#[test]
fn tables() {
    let md = "| a | **b** |\n|:---:|---:|\n| `x\\|y` | z \\| w |\n| 1 |";
    let center = json!([{"type": "alignment", "attrs": {"align": "center"}}]);
    let end = json!([{"type": "alignment", "attrs": {"align": "end"}}]);
    assert_eq!(
        content(md),
        json!([{"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": [
            {"type": "tableRow", "content": [
                {"type": "tableHeader", "content": [{"type": "paragraph", "content": [t("a")], "marks": center}]},
                {"type": "tableHeader", "content": [{"type": "paragraph", "content": [tm("b", json!([{"type": "strong"}]))], "marks": end}]}
            ]},
            {"type": "tableRow", "content": [
                {"type": "tableCell", "content": [{"type": "paragraph", "content": [tm("x|y", json!([{"type": "code"}]))], "marks": center}]},
                {"type": "tableCell", "content": [{"type": "paragraph", "content": [t("z | w")], "marks": end}]}
            ]},
            {"type": "tableRow", "content": [
                {"type": "tableCell", "content": [{"type": "paragraph", "content": [t("1")], "marks": center}]},
                {"type": "tableCell", "content": [{"type": "paragraph", "marks": end}]}
            ]}
        ]}])
    );
    // F11: separators with alignment colons and trailing spaces; F21: <br> in a cell.
    let c = content("| a | b |\n|:---|---:| \n| x<br>y |  |");
    assert_eq!(c[0]["type"], "table");
    assert_eq!(
        c[0]["content"][1]["content"][0]["content"][0]["content"],
        json!([t("x"), {"type": "hardBreak"}, t("y")])
    );
    assert_eq!(
        c[0]["content"][1]["content"][1]["content"],
        json!([{"type": "paragraph", "marks": [{"type": "alignment", "attrs": {"align": "end"}}]}])
    );
}

#[test]
fn table_extra_cells_are_dropped_with_a_warning() {
    let (value, warnings) = run("| a | b |\n|---|---|\n| 1 | 2 | 3 |\n| x | y | |");
    let rows = value["content"][0]["content"].as_array().unwrap().clone();
    assert!(
        rows.iter()
            .all(|r| r["content"].as_array().unwrap().len() == 2)
    );
    assert_eq!(codes(&warnings), vec![Code::TableCellDropped]);
    assert_eq!(warnings[0].location.as_ref().unwrap().line, Some(3));
}

#[test]
fn table_detection_edge_cases() {
    // F13: no separator row → not a table.
    assert_eq!(content("| a |\n| b |"), json!([pt("| a | | b |")]));
    // Without outer pipes.
    assert_eq!(content("a | b\n--|--\n1 | 2")[0]["type"], "table");
    // Two tables separated by a blank line stay separate.
    let c = content("| a |\n|---|\n| 1 |\n\n| b |\n|---|\n| 2 |");
    assert_eq!(c.as_array().unwrap().len(), 2);
}

#[test]
fn footnotes_become_superscripts_and_an_end_list() {
    let (value, warnings) =
        run("x[^n] y[^m] again[^n]\n\n[^m]: second\n[^n]: first *e*\n[^z]: unused");
    let sup = json!([{"type": "subsup", "attrs": {"type": "sup"}}]);
    assert_eq!(
        value["content"],
        json!([
            p(json!([t("x"), tm("1", sup.clone()), t(" y"), tm("2", sup.clone()), t(" again"), tm("1", sup)])),
            {"type": "rule"},
            ordered(1, json!([
                li(json!([p(json!([t("first "), tm("e", json!([{"type": "em"}]))]))])),
                li(json!([pt("second")])),
                li(json!([pt("unused")]))
            ]))
        ])
    );
    assert_eq!(codes(&warnings), vec![Code::FootnoteFlattened; 3]);
    // An undefined reference is literal text.
    assert_eq!(content("x[^none]"), json!([pt("x[^none]")]));
}

#[test]
fn front_matter_is_dropped_and_lines_still_match() {
    let (value, warnings) = run("---\ntitle: x\n---\n# After\n\n![i](http://x/i.png)");
    assert_eq!(value["content"][0]["content"], json!([t("After")]));
    assert_eq!(
        codes(&warnings),
        vec![Code::FrontMatterDropped, Code::ImageAsLink]
    );
    assert_eq!(warnings[1].location.as_ref().unwrap().line, Some(6));
}

// ---------------------------------------------------------------------------------------------
// Inline
// ---------------------------------------------------------------------------------------------

#[test]
fn emphasis_strong_strike() {
    assert_eq!(
        content("*e* _e_ **s** __s__ ***b*** ~~d~~ ~d~"),
        json!([p(json!([
            tm("e", json!([{"type": "em"}])),
            t(" "),
            tm("e", json!([{"type": "em"}])),
            t(" "),
            tm("s", json!([{"type": "strong"}])),
            t(" "),
            tm("s", json!([{"type": "strong"}])),
            t(" "),
            tm("b", json!([{"type": "em"}, {"type": "strong"}])),
            t(" "),
            tm("d", json!([{"type": "strike"}])),
            t(" "),
            tm("d", json!([{"type": "strike"}]))
        ]))])
    );
}

#[test]
fn inline_code_and_marks() {
    // F1: the critical empty-text-node case.
    assert_eq!(
        content("``a`b``"),
        json!([p(json!([tm("a`b", json!([{"type": "code"}]))]))])
    );
    // F9: strong around code is dropped with a warning.
    let (value, warnings) = run("**`x`**");
    assert_eq!(
        value["content"],
        json!([p(json!([tm("x", json!([{"type": "code"}]))]))])
    );
    assert_eq!(codes(&warnings), vec![Code::MarkDropped]);
    // Code keeps a link.
    assert_eq!(
        content("[`c`](https://l)"),
        json!([p(json!([tm(
            "c",
            json!([link("https://l"), {"type": "code"}])
        )]))])
    );
    // F10: code spans win over emphasis.
    assert_eq!(
        content("text ** `a**b`"),
        json!([p(json!([
            t("text ** "),
            tm("a**b", json!([{"type": "code"}]))
        ]))])
    );
    assert_eq!(
        content("***bi***"),
        json!([p(json!([tm(
            "bi",
            json!([{"type": "em"}, {"type": "strong"}])
        )]))])
    );
    // F8: nothing is deleted silently and no empty paragraph appears.
    assert_eq!(
        content("`x`**"),
        json!([p(json!([tm("x", json!([{"type": "code"}])), t("**")]))])
    );
    assert_eq!(content("****"), json!([{"type": "rule"}]));
    assert_eq!(content("a ****"), json!([pt("a ****")]));
    assert_eq!(content("a ** **"), json!([pt("a ** **")]));
    // Code spans keep inner spaces.
    assert_eq!(
        content("`` ` ``"),
        json!([p(json!([tm("`", json!([{"type": "code"}]))]))])
    );
}

#[test]
fn adjacent_equal_marks_are_merged() {
    assert_eq!(
        content("**a**__b__ *c*\n*d*"),
        json!([p(json!([
            tm("ab", json!([{"type": "strong"}])),
            t(" "),
            tm("c", json!([{"type": "em"}])),
            t(" "),
            tm("d", json!([{"type": "em"}]))
        ]))])
    );
    assert_eq!(content("a\\*b"), json!([pt("a*b")]));
}

#[test]
fn links_and_url_allowlist() {
    assert_eq!(
        content("[t](https://x.y \"Title\") [ref][r] [](http://empty)\n\n[r]: /browse/X-1"),
        json!([p(json!([
            tm(
                "t",
                json!([{"type": "link", "attrs": {"href": "https://x.y", "title": "Title"}}])
            ),
            t(" "),
            tm("ref", json!([link("/browse/X-1")])),
            t(" "),
            tm("http://empty", json!([link("http://empty")]))
        ]))])
    );
    let (value, warnings) =
        run("[js](javascript:alert(1)) [rel](docs/x.md) [p](//evil.com) [ok](#top)");
    assert_eq!(
        value["content"],
        json!([p(json!([t("js rel p "), tm("ok", json!([link("#top")]))]))])
    );
    assert_eq!(codes(&warnings), vec![Code::UnsafeUrl; 3]);
    assert_eq!(
        content("<https://a.b/c> <me@ex.com> <javascript:x>"),
        json!([p(json!([
            tm("https://a.b/c", json!([link("https://a.b/c")])),
            t(" "),
            tm("me@ex.com", json!([link("mailto:me@ex.com")])),
            t(" javascript:x")
        ]))])
    );
}

#[test]
fn bare_url_autolinks() {
    assert_eq!(
        content("see https://ex.com/a_b and www.x.com, or a@b.co. Not https\\://no or ex.com"),
        json!([p(json!([
            t("see "),
            tm("https://ex.com/a_b", json!([link("https://ex.com/a_b")])),
            t(" and "),
            tm("www.x.com", json!([link("http://www.x.com")])),
            t(", or "),
            tm("a@b.co", json!([link("mailto:a@b.co")])),
            t(". Not https://no or ex.com")
        ]))])
    );
    // Inside a link or code, nothing is autolinked.
    assert_eq!(
        content("[see https://ab.com](https://cd.com) `https://ef.com`"),
        json!([p(json!([
            tm("see https://ab.com", json!([link("https://cd.com")])),
            t(" "),
            tm("https://ef.com", json!([{"type": "code"}]))
        ]))])
    );
    // Marks apply to the link text.
    assert_eq!(
        content("**https://ab.com**"),
        json!([p(json!([tm(
            "https://ab.com",
            json!([link("https://ab.com"), {"type": "strong"}])
        )]))])
    );
    // An entity inside a URL breaks the literal match.
    assert_eq!(content("https://ab&#46;com"), json!([pt("https://ab.com")]));
    // A URL split across text events (here by an unmatched bracket) is still found.
    assert_eq!(
        content("https://ab.com/x[y"),
        json!([p(json!([tm(
            "https://ab.com/x[y",
            json!([link("https://ab.com/x[y")])
        )]))])
    );
}

#[test]
fn images_by_policy() {
    let (value, warnings) = run("![alt *t*](http://i.png \"T\") ![](https://x/i.png)");
    let l = json!({"type": "link", "attrs": {"href": "http://i.png", "title": "T"}});
    assert_eq!(
        value["content"],
        json!([p(json!([
            tm("alt ", json!([l.clone()])),
            tm("t", json!([l, {"type": "em"}])),
            t(" "),
            tm("https://x/i.png", json!([link("https://x/i.png")]))
        ]))])
    );
    assert_eq!(codes(&warnings), vec![Code::ImageAsLink, Code::ImageAsLink]);
    // A linked image keeps the outer link.
    assert_eq!(
        content("[![badge](https://img)](https://target)"),
        json!([p(json!([tm("badge", json!([link("https://target")]))]))])
    );
    let external = opts_with(|o| o.images = ImagePolicy::External);
    let (value, warnings) = run_with(
        "before ![a *b*](https://x/i.png) after\n\n- ![](https://x/j.png)",
        &external,
    );
    assert_eq!(
        value["content"],
        json!([
            pt("before"),
            {"type": "mediaSingle", "attrs": {"layout": "center"}, "content": [
                {"type": "media", "attrs": {"type": "external", "url": "https://x/i.png", "alt": "a b"}}]},
            pt("after"),
            bullet(json!([li(json!([
                {"type": "paragraph"},
                {"type": "mediaSingle", "attrs": {"layout": "center"}, "content": [
                    {"type": "media", "attrs": {"type": "external", "url": "https://x/j.png"}}]}
            ]))]))
        ])
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let (value, warnings) = run_with("![a](javascript:x)", &external);
    assert_eq!(value["content"], json!([pt("a")]));
    assert_eq!(codes(&warnings), vec![Code::UnsafeUrl]);
    let drop = opts_with(|o| o.images = ImagePolicy::Drop);
    let (value, warnings) = run_with("a ![x](https://i) b", &drop);
    assert_eq!(value["content"], json!([pt("a  b")]));
    assert_eq!(codes(&warnings), vec![Code::ImageDropped]);
}

#[test]
fn recognised_inline_html() {
    assert_eq!(
        content(
            "<u>u</u><ins>i</ins> <s>s</s><del>d</del><strike>k</strike> <b>b</b><strong>s</strong> \
             <i>i</i><em>e</em> <sub>1</sub><sup>2</sup> <kbd>K</kbd> \
             <span style=\"color: #FF0000\">r</span> <span style=\"background-color: #00ff00\">g</span>"
        ),
        json!([p(json!([
            tm("ui", json!([{"type": "underline"}])),
            t(" "),
            tm("sdk", json!([{"type": "strike"}])),
            t(" "),
            tm("bs", json!([{"type": "strong"}])),
            t(" "),
            tm("ie", json!([{"type": "em"}])),
            t(" "),
            tm("1", json!([{"type": "subsup", "attrs": {"type": "sub"}}])),
            tm("2", json!([{"type": "subsup", "attrs": {"type": "sup"}}])),
            t(" "),
            tm("K", json!([{"type": "code"}])),
            t(" "),
            tm(
                "r",
                json!([{"type": "textColor", "attrs": {"color": "#FF0000"}}])
            ),
            t(" "),
            tm(
                "g",
                json!([{"type": "backgroundColor", "attrs": {"color": "#00ff00"}}])
            )
        ]))])
    );
    // <kbd> is code: other formatting on it is dropped with a warning, a link is kept.
    let (value, warnings) = run("**<kbd>K</kbd>** [<kbd>L</kbd>](https://l.com)");
    assert_eq!(
        value["content"],
        json!([p(json!([
            tm("K", json!([{"type": "code"}])),
            t(" "),
            tm("L", json!([link("https://l.com"), {"type": "code"}]))
        ]))])
    );
    assert_eq!(codes(&warnings), vec![Code::MarkDropped]);
    // Overlapping HTML and Markdown marks, an unclosed tag, comments.
    assert_eq!(
        content("<u>*a</u> b* <b>open <!-- gone -->x"),
        json!([p(json!([
            tm("a", json!([{"type": "em"}, {"type": "underline"}])),
            tm(" b", json!([{"type": "em"}])),
            t(" "),
            tm("open x", json!([{"type": "strong"}]))
        ]))])
    );
    assert_eq!(content("<!-- only a comment -->"), json!([]));
    assert_eq!(
        content("a<br/>b<br />c"),
        json!([p(
            json!([t("a"), {"type": "hardBreak"}, t("b"), {"type": "hardBreak"}, t("c")])
        )])
    );
}

#[test]
fn html_policies() {
    let md = "a <div>x</div> <span class=\"c\">s</span>";
    let (value, warnings) = run(md);
    assert_eq!(
        value["content"],
        json!([pt("a <div>x</div> <span class=\"c\">s</span>")])
    );
    assert_eq!(codes(&warnings), vec![Code::HtmlAsText; 4]);
    let drop = opts_with(|o| o.html = HtmlPolicy::Drop);
    let (value, warnings) = run_with(md, &drop);
    assert_eq!(value["content"], json!([pt("a x s")]));
    assert_eq!(codes(&warnings), vec![Code::HtmlDropped; 4]);
    let error = opts_with(|o| o.html = HtmlPolicy::Error);
    let err = error_of(md, &error);
    assert_eq!(err.code, Code::Lossy);
    assert_eq!(err.location.as_ref().unwrap().column, Some(3));
    // Recognised tags and comments never trigger the policy.
    let (value, warnings) = run_with("<u>x</u> <!-- c -->", &error);
    assert_eq!(
        value["content"],
        json!([p(json!([tm("x", json!([{"type": "underline"}]))]))])
    );
    assert_eq!(warnings, vec![]);
    // HTML blocks.
    let (value, warnings) = run("<div>\nblock &amp; text\n</div>");
    assert_eq!(value["content"], json!([pt("<div> block & text </div>")]));
    assert_eq!(codes(&warnings), vec![Code::HtmlAsText, Code::HtmlAsText]);
    assert_eq!(
        content_with("<div>\nblock\n</div>", &drop),
        json!([pt("block")])
    );
    assert_eq!(error_of("<div>\nx\n</div>", &error).code, Code::Lossy);
    assert_eq!(
        content("<u>\nunder\n</u>"),
        json!([p(json!([tm("under", json!([{"type": "underline"}]))]))])
    );
}

#[test]
fn entities_escapes_and_control_characters() {
    assert_eq!(
        content("a &amp; b &copy; &#x41; \\* \\_"),
        json!([pt("a & b © A * _")])
    );
    // Entity-encoded edge spaces are kept (ADF → Markdown writes them this way).
    assert_eq!(content("&#32;x&#32;"), json!([pt(" x ")]));
    // A decoded newline becomes a hard break.
    assert_eq!(
        content("a&#10;b"),
        json!([p(json!([t("a"), {"type": "hardBreak"}, t("b")]))])
    );
    let (value, warnings) = run("a\u{1}b\tc\u{7f}");
    assert_eq!(value["content"], json!([pt("ab\tc")]));
    assert_eq!(codes(&warnings), vec![Code::ControlChar]);
    let (value, warnings) = run("```\na\u{2}b\n```");
    assert_eq!(value["content"][0]["content"], json!([t("ab")]));
    assert_eq!(codes(&warnings), vec![Code::ControlChar]);
}

// ---------------------------------------------------------------------------------------------
// ADF extensions
// ---------------------------------------------------------------------------------------------

#[test]
fn alerts_become_panels() {
    for (kind, panel) in [
        ("NOTE", "info"),
        ("TIP", "success"),
        ("IMPORTANT", "note"),
        ("WARNING", "warning"),
        ("CAUTION", "error"),
        ("note", "info"),
    ] {
        assert_eq!(
            content(&format!("> [!{kind}]\n> careful\n> - x")),
            json!([{"type": "panel", "attrs": {"panelType": panel}, "content": [
                pt("careful"),
                bullet(json!([li(json!([pt("x")]))]))
            ]}]),
            "{kind}"
        );
    }
    assert_eq!(
        content("> [!NOTE]"),
        json!([{"type": "panel", "attrs": {"panelType": "info"}, "content": [{"type": "paragraph"}]}])
    );
    // Headings and rules are allowed in panels; tables and quotes are hoisted.
    let (value, warnings) = run("> [!TIP]\n> # H\n> ***\n> > q");
    assert_eq!(
        value["content"],
        json!([
            {"type": "panel", "attrs": {"panelType": "success"}, "content": [
                {"type": "heading", "attrs": {"level": 1}, "content": [t("H")]},
                {"type": "rule"}
            ]},
            {"type": "blockquote", "content": [pt("q")]}
        ])
    );
    assert_eq!(codes(&warnings), vec![Code::BlockHoisted]);
}

#[test]
fn alerts_in_containers_that_cannot_hold_panels() {
    let (value, warnings) = run("- > [!TIP]\n  > in list");
    assert_eq!(
        value["content"],
        json!([{"type": "blockquote", "content": [pt("in list")]}])
    );
    assert_eq!(
        codes(&warnings),
        vec![Code::BlockFlattened, Code::BlockHoisted]
    );
    let (value, warnings) = run("> a\n>\n> > [!NOTE]\n> > b");
    assert_eq!(
        value["content"],
        json!([{"type": "blockquote", "content": [pt("a"), pt("b")]}])
    );
    assert_eq!(codes(&warnings), vec![Code::BlockFlattened]);
}

#[test]
fn alerts_disabled() {
    let opts = opts_with(|o| o.extensions.alerts = false);
    let (value, warnings) = run_with("> [!WARNING]\n> careful", &opts);
    assert_eq!(
        value["content"],
        json!([{"type": "blockquote", "content": [pt("[!WARNING] careful")]}])
    );
    assert_eq!(codes(&warnings), vec![Code::ExtensionDisabled]);
}

#[test]
fn details_become_expands() {
    assert_eq!(
        content("<details>\n<summary>Logs &amp; <b>more</b></summary>\n\nbody *x*\n\n</details>"),
        json!([{"type": "expand", "attrs": {"title": "Logs & more"}, "content": [
            p(json!([t("body "), tm("x", json!([{"type": "em"}]))]))
        ]}])
    );
    assert_eq!(
        content("<details>\n\ntext\n\n</details>"),
        json!([{"type": "expand", "attrs": {}, "content": [pt("text")]}])
    );
    assert_eq!(
        content("<details><summary>T</summary></details>"),
        json!([{"type": "expand", "attrs": {"title": "T"}, "content": [{"type": "paragraph"}]}])
    );
    assert_eq!(
        content("<details><summary>One</summary>inline body</details>"),
        json!([{"type": "expand", "attrs": {"title": "One"}, "content": [pt("inline body")]}])
    );
    // Nested details inside an expand become nestedExpand; tables are allowed in expands.
    let md = "<details><summary>A</summary>\n\n<details><summary>B</summary>\n\n| t |\n|---|\n| x |\n\n</details>\n\n</details>";
    let c = content(md);
    assert_eq!(c[0]["type"], "expand");
    // The table is hoisted out of the nested expand into the expand; the nested expand keeps
    // its title.
    assert_eq!(
        c[0]["content"][0],
        json!({"type": "nestedExpand", "attrs": {"title": "B"}, "content": [{"type": "paragraph"}]})
    );
    assert_eq!(c[0]["content"][1]["type"], "table");
    // Unclosed details end with the document.
    assert_eq!(
        content("<details>\n<summary>U</summary>\n\nrest"),
        json!([{"type": "expand", "attrs": {"title": "U"}, "content": [pt("rest")]}])
    );
}

#[test]
fn details_flattened_where_expands_are_not_allowed() {
    let (value, warnings) = run("- <details>\n  <summary>T</summary>\n\n  inner\n\n  </details>");
    assert_eq!(
        value["content"],
        json!([bullet(json!([li(json!([
            p(json!([tm("T", json!([{"type": "strong"}]))])),
            pt("inner")
        ]))]))])
    );
    assert_eq!(codes(&warnings), vec![Code::BlockFlattened]);
}

#[test]
fn details_disabled() {
    let opts = opts_with(|o| o.extensions.details = false);
    let (value, warnings) = run_with("<details>\n<summary>T</summary>\n\nx\n\n</details>", &opts);
    assert_eq!(
        value["content"],
        json!([
            pt("<details> <summary>T</summary>"),
            pt("x"),
            pt("</details>")
        ])
    );
    assert_eq!(
        codes(&warnings),
        vec![
            Code::ExtensionDisabled,
            Code::HtmlAsText,
            Code::HtmlAsText,
            Code::HtmlAsText,
            Code::HtmlAsText
        ]
    );
}

#[test]
fn box_cells() {
    assert_eq!(
        content("::: cell\n- one\n- two\n:::\nafter"),
        json!([
            one_cell_box(json!([bullet(json!([
                li(json!([pt("one")])),
                li(json!([pt("two")]))
            ]))])),
            pt("after")
        ])
    );
    assert_eq!(
        content(":::: cell  \nx\n::::"),
        json!([one_cell_box(json!([pt("x")]))])
    );
    // Unclosed: ends at the end of the input. Empty: one empty paragraph.
    assert_eq!(
        content("::: cell\nunclosed"),
        json!([one_cell_box(json!([pt("unclosed")]))])
    );
    assert_eq!(
        content("::: cell\n:::"),
        json!([one_cell_box(json!([{"type": "paragraph"}]))])
    );
    // Boxes interrupt paragraphs and close lists and quotes.
    assert_eq!(
        content("para\n::: cell\n> q\n:::"),
        json!([
            pt("para"),
            one_cell_box(json!([{"type": "blockquote", "content": [pt("q")]}]))
        ])
    );
    // Alerts and details inside boxes.
    let c = content(
        "::: cell\n> [!WARNING]\n> w\n\n<details><summary>N</summary>\n\nx\n\n</details>\n:::",
    );
    let cell = &c[0]["content"][0]["content"][0]["content"];
    assert_eq!(cell[0]["type"], "panel");
    assert_eq!(cell[1]["type"], "nestedExpand");
    assert_eq!(cell[1]["attrs"]["title"], "N");
}

#[test]
fn box_cell_rules() {
    let (value, warnings) = run("::: cell\n::: cell\n:::");
    assert_eq!(
        value["content"],
        json!([one_cell_box(json!([pt("::: cell")]))])
    );
    assert_eq!(codes(&warnings), vec![Code::BlockFlattened]);
    assert_eq!(warnings[0].location.as_ref().unwrap().line, Some(2));
    // A table in a box is hoisted after it; the box reopens for the rest.
    let (value, warnings) = run("::: cell\n| a |\n|---|\n| b |\n\nafter\n:::");
    let c = value["content"].as_array().unwrap();
    assert_eq!(c.len(), 2);
    assert_eq!(c[0]["content"][0]["content"][0]["type"], "tableHeader");
    assert_eq!(c[1], one_cell_box(json!([pt("after")])));
    assert_eq!(codes(&warnings), vec![Code::BlockHoisted]);
    // Not recognised in fenced code or when indented.
    assert_eq!(content("```\n::: cell\n```")[0]["type"], "codeBlock");
    assert_eq!(content(" ::: cell\nx"), json!([pt("::: cell x")]));
    let opts = opts_with(|o| o.extensions.cells = false);
    let (value, warnings) = run_with("::: cell\nx\n:::", &opts);
    assert_eq!(value["content"], json!([pt("::: cell x :::")]));
    assert_eq!(codes(&warnings), vec![Code::ExtensionDisabled]);
}

#[test]
fn raw_adf_blocks() {
    assert_eq!(
        content(
            "```adf\n{\"type\": \"panel\", \"attrs\": {\"panelType\": \"success\"}, \"content\": [{\"type\": \"paragraph\"}]}\n```"
        ),
        json!([{"type": "panel", "attrs": {"panelType": "success"}, "content": [{"type": "paragraph"}]}])
    );
    assert_eq!(
        content("```adf\n[{\"type\":\"rule\"},{\"type\":\"rule\"}]\n```"),
        json!([{"type": "rule"}, {"type": "rule"}])
    );
    // Valid in a list item.
    assert_eq!(
        content("- x\n\n  ```adf\n  {\"type\":\"codeBlock\"}\n  ```"),
        json!([bullet(json!([li(json!([pt("x"), {"type": "codeBlock"}]))]))])
    );
    let opts = MdToAdfOptions::default();
    let err = error_of("text\n\n```adf\n{not json\n```", &opts);
    assert_eq!(err.code, Code::RawAdf);
    assert_eq!(err.location.as_ref().unwrap().line, Some(3));
    let err = error_of("- ```adf\n  {\"type\":\"rule\"}\n  ```", &opts);
    assert_eq!(err.code, Code::RawAdf);
    assert!(err.message.contains("list item"), "{}", err.message);
    let err = error_of("```adf\n{\"type\":\"text\",\"text\":\"x\"}\n```", &opts);
    assert_eq!(err.code, Code::RawAdf);
    let disabled = opts_with(|o| o.extensions.raw_adf = false);
    let (value, warnings) = run_with("```adf\n{\"type\":\"rule\"}\n```", &disabled);
    assert_eq!(
        value["content"],
        json!([{"type": "codeBlock", "attrs": {"language": "adf"}, "content": [t("{\"type\":\"rule\"}")]}])
    );
    assert_eq!(codes(&warnings), vec![Code::ExtensionDisabled]);
}

#[test]
fn adf_link_extensions() {
    assert_eq!(
        content(
            "[@Jane Doe](adf:mention/5b10ac8d82e05b22cc7d4ef5) [DONE](adf:status/green) [todo](adf:status) \
             [2026-10-19](adf:date) [😄](adf:emoji/:smile:) [](adf:emoji/tada) [https://x.atlassian.net/browse/A-1](adf:card)"
        ),
        json!([p(json!([
            {"type": "mention", "attrs": {"id": "5b10ac8d82e05b22cc7d4ef5", "text": "@Jane Doe"}}, t(" "),
            {"type": "status", "attrs": {"text": "DONE", "color": "green"}}, t(" "),
            {"type": "status", "attrs": {"text": "todo", "color": "neutral"}}, t(" "),
            {"type": "date", "attrs": {"timestamp": "1792368000000"}}, t(" "),
            {"type": "emoji", "attrs": {"shortName": ":smile:", "text": "😄"}}, t(" "),
            {"type": "emoji", "attrs": {"shortName": ":tada:"}}, t(" "),
            {"type": "inlineCard", "attrs": {"url": "https://x.atlassian.net/browse/A-1"}}
        ]))])
    );
}

#[test]
fn adf_link_extension_fallbacks() {
    let (value, warnings) =
        run("[*x*](adf:bogus/1) [y](adf:status/pink) [z](adf:date) [](adf:status) [w](adf:card)");
    assert_eq!(
        value["content"],
        json!([p(json!([tm("x", json!([{"type": "em"}])), t(" y z  w")]))])
    );
    assert_eq!(
        codes(&warnings),
        vec![
            Code::UnknownExtension,
            Code::InvalidExtensionArg,
            Code::InvalidExtensionArg,
            Code::InvalidExtensionArg,
            Code::InvalidExtensionArg
        ]
    );
    // Marks around extension nodes are dropped.
    let (value, warnings) = run("**[@A](adf:mention/x)**");
    assert_eq!(
        value["content"],
        json!([p(
            json!([{"type": "mention", "attrs": {"id": "x", "text": "@A"}}])
        )])
    );
    assert_eq!(codes(&warnings), vec![Code::MarkDropped]);
    // Disabled extensions keep the label.
    let opts = opts_with(|o| o.extensions = Extensions::none());
    let (value, warnings) = run_with("[@A](adf:mention/x) [D](adf:status/red)", &opts);
    assert_eq!(value["content"], json!([pt("@A D")]));
    assert_eq!(codes(&warnings), vec![Code::ExtensionDisabled; 2]);
}

// ---------------------------------------------------------------------------------------------
// Limits, robustness, determinism
// ---------------------------------------------------------------------------------------------

#[test]
fn input_size_limit() {
    let big = "a".repeat(crate::limits::DEFAULT_MAX_INPUT_BYTES + 1);
    let err = error_of(&big, &MdToAdfOptions::default());
    assert_eq!(err.code, Code::InputTooLarge);
    let small = opts_with(|o| {
        o.limits = Limits {
            max_input_bytes: 3,
            ..Limits::default()
        };
    });
    assert!(super::to_adf("abc", &small).is_ok());
    assert_eq!(error_of("abcd", &small).code, Code::InputTooLarge);
}

/// Inputs close to the size limit that stress one stage each; all must stay roughly linear.
#[test]
fn large_inputs_stay_fast() {
    // Full size in release builds; a quarter in debug builds, which are ~7x slower but still
    // show any quadratic behaviour clearly.
    let max = if cfg!(debug_assertions) {
        crate::limits::DEFAULT_MAX_INPUT_BYTES / 4
    } else {
        crate::limits::DEFAULT_MAX_INPUT_BYTES
    };
    // Leave room for the short prefixes and suffixes some cases add.
    let fill = |unit: &str| unit.repeat((max - 64) / unit.len());
    let cases = [
        ("words", fill("word ")),
        (
            "urls",
            fill("see https://example.com/a and www.example.org "),
        ),
        ("emphasis", fill("*a **b** c* ")),
        ("inline html", fill("<u>x</u><b>y ")),
        ("unknown html", fill("<x-y>z ")),
        ("html block", format!("<div>\n{}", fill("<a b='<!--"))),
        ("list", fill("- item\n")),
        ("tasks", fill("- [ ] t\n")),
        (
            "table",
            format!("| a | b |\n|---|---|\n{}", fill("| 1 | 2 |\n")),
        ),
        ("quotes", fill("> q\n>\n")),
        ("headings", fill("# h\n")),
        ("footnotes", fill("x[^1] ") + "\n\n[^1]: n\n"),
        ("entities", fill("&amp;&#32;")),
        ("adf links", fill("[@A](adf:mention/x) ")),
        ("boxes", fill("::: cell\nx\n:::\n")),
        (
            "details",
            fill("<details><summary>s</summary>x</details>\n"),
        ),
        // Many text events and URLs in one buffer (brackets split text events).
        ("split urls", fill("https://example.com/[x ")),
        // Many unclosed tags, then many unmatched and matched closing tags.
        (
            "unclosed tags",
            "<u>".repeat(max / 8) + &"</b></u>".repeat(max / 16),
        ),
        (
            "closers under deep emphasis",
            "*a ".repeat(max / 16) + &"</u>".repeat(max / 8) + &" a*".repeat(max / 16),
        ),
    ];
    for (name, md) in cases {
        let start = std::time::Instant::now();
        let result = super::to_adf(&md, &MdToAdfOptions::default());
        assert!(result.is_ok(), "{name}: {:?}", result.err());
        let elapsed = start.elapsed();
        assert!(elapsed.as_secs() < 20, "{name}: {elapsed:?}");
    }
}

#[test]
fn deep_nesting_is_rejected_quickly() {
    let start = std::time::Instant::now();
    for md in [
        ">".repeat(100_000) + " x",
        "- ".repeat(100_000) + "x",
        "1. ".repeat(100_000) + "x",
        "> - ".repeat(50_000) + "x",
    ] {
        let err = error_of(&md, &MdToAdfOptions::default());
        assert_eq!(err.code, Code::InputTooDeep);
        assert_eq!(err.location.as_ref().unwrap().line, Some(1));
    }
    let boxes = "<details>\n".repeat(1_000);
    assert_eq!(
        error_of(&boxes, &MdToAdfOptions::default()).code,
        Code::InputTooDeep
    );
    assert!(start.elapsed().as_secs() < 20, "{:?}", start.elapsed());
}

/// A huge configured limit is capped internally, so lowering and serialisation (which recurse
/// once per level) stay within a test thread's 2 MiB stack, for every kind of container.
#[test]
fn configured_depth_is_capped() {
    let unlimited = opts_with(|o| o.limits.max_depth = usize::MAX);
    let cap = super::cx::HARD_MAX_DEPTH;
    let deepest = "- ".repeat(cap) + "x";
    let (value, _) = run_with(&deepest, &unlimited);
    let mut node = &value["content"][0];
    let mut levels = 0;
    while node["type"] == "bulletList" {
        levels += 1;
        node = &node["content"][0]["content"][1];
    }
    assert_eq!(levels, cap);
    for md in [
        "> - ".repeat(cap / 2) + "x",
        ">".repeat(cap) + " x",
        "- [ ] ".repeat(cap) + "x",
        "1. ".repeat(cap) + "x",
        "> [!NOTE]\n".to_owned() + &"> - ".repeat(cap / 2 - 1) + "x",
        "<details>\n\n".repeat(cap) + "x",
        "::: cell\n".to_owned() + &"<details>\n\n".repeat(cap - 2) + "- x",
    ] {
        let _ = run_with(&md, &unlimited);
    }
    let too_deep = "- ".repeat(cap + 1) + "x";
    assert_eq!(error_of(&too_deep, &unlimited).code, Code::InputTooDeep);
}

#[test]
fn nesting_up_to_the_limit_is_fine() {
    let max = Limits::default().max_depth;
    let ok = ">".repeat(max) + " x";
    let (value, warnings) = run(&ok);
    assert_eq!(
        value["content"],
        json!([{"type": "blockquote", "content": [pt("x")]}])
    );
    // Each inner quote level is merged into the outer one.
    assert_eq!(codes(&warnings), vec![Code::BlockFlattened; max - 1]);
    let too_deep = ">".repeat(max + 1) + " x";
    assert_eq!(
        error_of(&too_deep, &MdToAdfOptions::default()).code,
        Code::InputTooDeep
    );
    let lists = "- ".repeat(max) + "x";
    let (value, _) = run(&lists);
    assert_eq!(value["content"][0]["type"], "bulletList");
    // Deep inline nesting has no limit and no recursion.
    let deep_em = "*".repeat(20_000) + "x" + &"*".repeat(20_000);
    let _ = run(&deep_em);
}

#[test]
fn bom_crlf_and_cr_inputs_are_equivalent() {
    let lf = content("# T\n\n- [ ] a\n- b\n");
    let mut with_bom = lf.clone();
    strip_ids(&mut with_bom);
    let mut bom = content("\u{feff}# T\r\n\r\n- [ ] a\r\n- b\r\n");
    strip_ids(&mut bom);
    assert_eq!(bom, with_bom);
    assert_eq!(content("\u{feff}# T\n\n- [ ] a\n- b\n"), lf);
    assert_eq!(content("# T\r\r- [ ] a\r- b\r"), lf);
}

#[test]
fn warnings_point_at_original_lines_and_columns() {
    let (_, warnings) = run("\u{feff}ok\r\n\r\n  é **`x`**\r\n");
    assert_eq!(codes(&warnings), vec![Code::MarkDropped]);
    let location = warnings[0].location.as_ref().unwrap();
    assert_eq!((location.line, location.column), (Some(3), Some(7)));
}

#[test]
fn md2adf_regressions_produce_valid_adf() {
    // The failure cases from the review of the old line-based script (F1–F21).
    for md in [
        "``a`b``",
        "```\ncode\n```",
        "## H2\n#### H4",
        "line one\nline two\n\na  \nb",
        "- a\n  - b\n- one\n  cont",
        "1. a\n\n2. b\n\n3. x",
        "*it* _it_ ~~x~~ [t](https://u) <https://u> https://u \\*",
        "`x`** and ****",
        "**`x`**",
        "text ** `a**b` ***bi***",
        "|:---|---:|\n|---|---|\n| a | b |",
        "| a \\| b | `x|y` |\n|---|---|\n| 1 | 2 |",
        "| | |\n|---|---|\n| | |\n\n|x| is abs value",
        "> [!WARNING]\n> careful\n\n---\n\n<details><summary>s</summary>\n\nx\n\n</details>",
        "* a\n+ b\n1) c\n-\ttab\n-\n- [ ] todo",
        "\u{feff}# BOM",
        "###  spaced ###",
        "| a | b |\n|:-:|--:|\n| x<br>y | z |",
    ] {
        let _ = run(md);
    }
}

#[test]
fn synthetic_incident_report_shape() {
    let md = "## ✍️ **Summary**\n\n::: cell\nThe **payments** service returned errors.\n\n- first\n- second\n:::\n\n\
              ## 📡 **Timeline**\n\n| **Date & time (UTC)** | **Event** |\n|---|---|\n| 10:00 | Alert `HighErrorRate` fired |\n| 10:05 | Rolled back |\n\n\
              > [!WARNING]\n> Do not retry `deploy` before the fix.\n";
    let (value, warnings) = run(md);
    assert!(warnings.is_empty(), "{warnings:?}");
    let c = value["content"].as_array().unwrap();
    let types: Vec<&str> = c.iter().map(|b| b["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["heading", "table", "heading", "table", "panel"]);
    assert_eq!(
        c[0]["content"],
        json!([t("✍️ "), tm("Summary", json!([{"type": "strong"}]))])
    );
    assert_eq!(c[3]["content"].as_array().unwrap().len(), 3);
    assert_eq!(
        c[3]["content"][0]["content"][0]["content"][0]["content"],
        json!([tm("Date & time (UTC)", json!([{"type": "strong"}]))])
    );
}

#[test]
fn kitchen_sink_is_schema_valid_with_all_options() {
    let md = "---\na: 1\n---\n# H\n\npara *e* **s** `c` [l](https://x) ![i](https://x/i.png) x[^1]\n\n\
              - a\n  - b\n- [ ] t\n\n1. o\n\n> q\n\n> [!NOTE]\n> n\n\n| a | b |\n|:-:|-:|\n| 1 | 2 |\n\n\
              <details><summary>D</summary>\n\nd\n\n</details>\n\n::: cell\nbox\n:::\n\n\
              [@A](adf:mention/x) [S](adf:status/blue) [2020-02-29](adf:date) [e](adf:emoji/smile) [https://x](adf:card)\n\n\
              <u>u</u> <span style=\"color: #123456\">c</span> <div>x</div>\n\n[^1]: note\n";
    for images in [ImagePolicy::Link, ImagePolicy::External, ImagePolicy::Drop] {
        for html in [HtmlPolicy::Text, HtmlPolicy::Drop] {
            for softbreak in [SoftBreak::Space, SoftBreak::HardBreak] {
                for extensions in [Extensions::all(), Extensions::none()] {
                    let opts = MdToAdfOptions {
                        softbreak,
                        html,
                        images,
                        extensions,
                        ..MdToAdfOptions::default()
                    };
                    let _ = run_with(md, &opts);
                }
            }
        }
    }
}

#[test]
fn lib_doctest_example() {
    let conversion =
        crate::md_to_adf("# Hello\n\n- [ ] ship it\n", &MdToAdfOptions::default()).unwrap();
    let json = conversion.output.to_value().unwrap();
    assert_eq!(json["type"], "doc");
    assert_eq!(json["content"][1]["type"], "taskList");
}

/// pulldown-cmark 0.13's offset iterator panics on these (an empty tight paragraph left by a
/// link reference definition, then an indented blank line); the converter must not.
#[test]
fn pulldown_empty_tight_paragraph_does_not_panic() {
    assert_eq!(
        content("* [r]:e\n\t\t"),
        json!([bullet(json!([li(json!([{"type": "paragraph"}]))]))])
    );
    let c = content("- a\n- [r]: https://x.example\n          \n- b [r]");
    assert_eq!(c[0]["content"].as_array().unwrap().len(), 3);
    assert_eq!(
        c[0]["content"][2]["content"][0]["content"][1],
        tm("r", json!([link("https://x.example")]))
    );
    let c = content("> - [r]:e\n>\t\t\n\nafter");
    assert_eq!(c[1], pt("after"));
    // Whitespace on blank lines inside top-level fenced code is kept.
    let c = content("* [r]:e\n\t\t\n\n```\na\n  \nb\n```");
    assert_eq!(c[1]["content"][0]["text"], "a\n  \nb");
}

/// Markdown fragments that exercise container interactions when combined at random.
const FRAGMENTS: &[&str] = &[
    "- ",
    "* ",
    "1. ",
    "3) ",
    "> ",
    "> [!NOTE]\n",
    "> [!WARNING]\n",
    "# ",
    "### ",
    "```\n",
    "```adf\n{\"type\":\"rule\"}\n```\n",
    "```adf\n{\"type\":\"paragraph\"}\n```\n",
    "~~~\n",
    "::: cell\n",
    ":::\n",
    "<details>\n",
    "<summary>S</summary>\n",
    "</details>\n",
    "<details><summary>T</summary>",
    "| a | b |\n|---|---|\n",
    "| 1 | 2 |\n",
    "---\n",
    "***\n",
    "\n",
    "\n\n",
    "  ",
    "    ",
    "\t",
    "[ ] ",
    "[x] ",
    "*",
    "**",
    "_",
    "~~",
    "`",
    "``",
    "[",
    "]",
    "(",
    ")",
    "text ",
    "word",
    "https://example.com/x ",
    "www.example.org ",
    "a@b.co ",
    "![alt](https://i.example/i.png)",
    "[l](https://l.example)",
    "[^1]",
    "\n[^1]: note\n",
    "<u>",
    "</u>",
    "<kbd>",
    "</kbd>",
    "<span style=\"color: #ff0000\">",
    "</span>",
    "<div>",
    "</div>",
    "<br>",
    "<!-- c -->",
    "<!--",
    "-->",
    "&amp;",
    "&#10;",
    "\\",
    "\\|",
    "[@A](adf:mention/x)",
    "[2026-01-01](adf:date)",
    "[S](adf:status/red)",
    "[x](adf:nope)",
    "\u{1}",
    "é",
    "😀",
    "\r\n",
    "[r]: https://r.example\n",
    "[r]",
    "  \n",
    "<sub>",
    "</sub>",
];

/// A tiny deterministic generator (xorshift64*), so the test needs no extra dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % (n as u64)).unwrap()
    }
}

#[test]
fn random_fragment_documents_are_always_valid() {
    let mut rng = Rng(0x0BAD_C0DE_F00D_D00D);
    let option_sets = [
        MdToAdfOptions::default(),
        opts_with(|o| {
            o.images = ImagePolicy::External;
            o.softbreak = SoftBreak::HardBreak;
            o.html = HtmlPolicy::Drop;
        }),
        opts_with(|o| o.extensions = Extensions::none()),
    ];
    let cases = if cfg!(debug_assertions) { 1_500 } else { 6_000 };
    let mut ok = 0;
    for case in 0..cases {
        let len = 1 + rng.below(120);
        let md: String = (0..len)
            .map(|_| FRAGMENTS[rng.below(FRAGMENTS.len())])
            .collect();
        let opts = &option_sets[case % option_sets.len()];
        let caught = std::panic::catch_unwind(|| super::to_adf(&md, opts));
        let Ok(result) = caught else {
            panic!("PANIC on {md:?} with {opts:?}");
        };
        match result {
            Ok(conversion) => {
                ok += 1;
                let value = conversion.output.to_value().unwrap();
                let errors: Vec<String> = validator()
                    .iter_errors(&value)
                    .map(|e| format!("{e} at {}", e.instance_path()))
                    .collect();
                assert!(errors.is_empty(), "{md:?}\n{errors:#?}");
                for w in &conversion.warnings {
                    assert!(
                        w.location.as_ref().is_some_and(|l| l.line.is_some()),
                        "{md:?}: {w:?}"
                    );
                }
            }
            Err(diagnostics) => {
                let error = diagnostics.iter().find(|d| d.is_error()).unwrap();
                assert_eq!(error.code, Code::RawAdf, "{md:?}: {error}");
            }
        }
    }
    // Most documents must convert; only misplaced ```adf blocks may fail.
    assert!(ok * 10 >= cases * 7, "only {ok} of {cases} converted");
}
