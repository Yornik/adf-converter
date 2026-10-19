//! Round-trip tests: `md_to_adf(adf_to_md(x)) == normalize(x)` (`docs/syntax.md#round-trip-guarantee`).
//!
//! They run both directions; the self-contained checks at the bottom only re-parse escaped text
//! with CommonMark.

#![allow(clippy::needless_pass_by_value, clippy::too_many_lines)]

use serde_json::{Map, Value, json};

use super::to_markdown;
use crate::options::{AdfToMdOptions, ImagePolicy, MdToAdfOptions};

// ---------------------------------------------------------------------------------------------
// normalize
// ---------------------------------------------------------------------------------------------

/// Attributes `W_ATTR_DROPPED` covers, per node type.
fn droppable(ty: &str, attr: &str) -> bool {
    match ty {
        "tableCell" | "tableHeader" => matches!(attr, "colwidth" | "background" | "valign"),
        "table" => matches!(
            attr,
            "width" | "layout" | "displayMode" | "isNumberColumnEnabled"
        ),
        "mediaSingle" => matches!(attr, "width" | "widthType" | "layout"),
        "media" => matches!(attr, "width" | "height"),
        "panel" => matches!(attr, "panelIcon" | "panelIconId" | "panelIconText"),
        _ => false,
    }
}

/// `normalize` from the round-trip guarantee, applied to both sides: no `localId`s (task list IDs
/// are compared by position, i.e. ignored), `\n` in text split into `hardBreak`s, `W_ATTR_DROPPED`
/// attributes removed, and trailing hard breaks of a block removed (Markdown cannot end a
/// paragraph with one).
fn normalize(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
        Value::Object(map) => {
            let ty = map
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let mut out = Map::new();
            for (key, item) in map {
                match key.as_str() {
                    "attrs" => {
                        let Value::Object(attrs) = item else {
                            out.insert(key.clone(), item.clone());
                            continue;
                        };
                        let kept: Map<String, Value> = attrs
                            .iter()
                            .filter(|(name, _)| *name != "localId" && !droppable(&ty, name))
                            .map(|(name, v)| (name.clone(), normalize(v)))
                            .collect();
                        if !kept.is_empty() || matches!(ty.as_str(), "expand" | "nestedExpand") {
                            out.insert(key.clone(), Value::Object(kept));
                        }
                    }
                    "content" => {
                        let Value::Array(children) = item else {
                            out.insert(key.clone(), item.clone());
                            continue;
                        };
                        let mut split = Vec::new();
                        for child in children {
                            split_newlines(&normalize(child), &mut split);
                        }
                        if matches!(ty.as_str(), "paragraph" | "heading" | "taskItem") {
                            while split.last().is_some_and(|n| n["type"] == "hardBreak") {
                                split.pop();
                            }
                        }
                        if !split.is_empty()
                            || !matches!(ty.as_str(), "paragraph" | "heading" | "taskItem")
                        {
                            out.insert(key.clone(), Value::Array(split));
                        }
                    }
                    _ => {
                        out.insert(key.clone(), normalize(item));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

fn split_newlines(node: &Value, out: &mut Vec<Value>) {
    let text = node.get("text").and_then(Value::as_str);
    let (Some(text), true) = (text, node["type"] == "text") else {
        out.push(node.clone());
        return;
    };
    for (i, part) in text.split('\n').enumerate() {
        if i > 0 {
            out.push(json!({"type": "hardBreak"}));
        }
        if !part.is_empty() {
            let mut piece = node.clone();
            piece["text"] = Value::String(part.to_owned());
            out.push(piece);
        }
    }
}

fn assert_roundtrip_with(doc: &Value, md_opts: &MdToAdfOptions) {
    let markdown = to_markdown(doc, &AdfToMdOptions::default())
        .expect("adf -> md")
        .output;
    let back = crate::md_to_adf(&markdown, md_opts)
        .unwrap_or_else(|e| panic!("md -> adf failed: {e}\n--- markdown ---\n{markdown}"));
    let back = back.output.to_value().expect("serialise");
    let (expected, actual) = (normalize(doc), normalize(&back));
    assert_eq!(
        actual,
        expected,
        "\n--- markdown ---\n{markdown}\n--- actual ---\n{}\n--- expected ---\n{}",
        serde_json::to_string_pretty(&actual).unwrap_or_default(),
        serde_json::to_string_pretty(&expected).unwrap_or_default()
    );
}

fn assert_roundtrip(doc: &Value) {
    assert_roundtrip_with(doc, &MdToAdfOptions::default());
}

fn doc(content: Value) -> Value {
    json!({"type": "doc", "version": 1, "content": content})
}

fn t(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn strong(text: &str) -> Value {
    json!({"type": "text", "text": text, "marks": [{"type": "strong"}]})
}

fn code(text: &str) -> Value {
    json!({"type": "text", "text": text, "marks": [{"type": "code"}]})
}

fn p(inline: &[Value]) -> Value {
    json!({"type": "paragraph", "content": inline})
}

fn para(text: &str) -> Value {
    p(&[t(text)])
}

fn li(blocks: &[Value]) -> Value {
    json!({"type": "listItem", "content": blocks})
}

fn bullets(items: &[&str]) -> Value {
    json!({"type": "bulletList", "content": items.iter().map(|i| li(&[para(i)])).collect::<Vec<_>>()})
}

fn numbered(items: &[&str]) -> Value {
    json!({"type": "orderedList", "attrs": {"order": 1}, "content": items.iter().map(|i| li(&[para(i)])).collect::<Vec<_>>()})
}

fn table(rows: Vec<Value>) -> Value {
    json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": rows})
}

fn boxed(blocks: &[Value]) -> Value {
    table(vec![
        json!({"type": "tableRow", "content": [{"type": "tableCell", "content": blocks}]}),
    ])
}

fn h2(emoji: &str, title: &str) -> Value {
    json!({"type": "heading", "attrs": {"level": 2}, "content": [t(&format!("{emoji} ")), strong(title)]})
}

// ---------------------------------------------------------------------------------------------
// The synthetic sample: same *shape* as a real incident description (adf-contract.md §7), with
// invented, neutral content.
// ---------------------------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn synthetic_sample() -> Value {
    let boxes = vec![
        // paragraph
        boxed(&[p(&[
            strong("Status: "),
            t("Resolved. The reading room catalogue is searchable again."),
        ])]),
        // bulletList (a list as the cell's first child)
        boxed(&[bullets(&[
            "Catalogue search",
            "Loan renewals",
            "Reservation emails",
        ])]),
        // paragraph > bulletList > paragraph
        boxed(&[
            p(&[strong("Root cause")]),
            bullets(&[
                "The nightly index rebuild ran twice and the second run locked the shelf table.",
            ]),
            p(&[
                t("The duplicate run came from a "),
                code("cron"),
                t(" entry left behind after the move."),
            ]),
        ]),
        // paragraph > orderedList
        boxed(&[
            para("Recovery steps:"),
            numbered(&[
                "Stop the second rebuild",
                "Release the table lock",
                "Re-run the rebuild once",
            ]),
        ]),
        // paragraph > paragraph > bulletList > paragraph
        boxed(&[
            p(&[strong("Detection")]),
            p(&[
                t("Patrons reported empty search results at the help desk "),
                strong("before"),
                t(" any alert fired."),
            ]),
            bullets(&[
                "Alert threshold was 30 minutes",
                "Search errors were logged as warnings",
            ]),
            para("Both settings were changed after the review."),
        ]),
        // paragraph > paragraph > orderedList > paragraph
        boxed(&[
            p(&[strong("Timeline summary")]),
            para("All times are UTC."),
            numbered(&[
                "02:10 rebuild started",
                "02:12 second rebuild started",
                "07:45 service restored",
            ]),
            p(&[t("Total impact: "), strong("5 h 35 min"), t(".")]),
        ]),
        // paragraph > orderedList > paragraph > paragraph > orderedList
        boxed(&[
            para("Follow-up actions, in order:"),
            numbered(&[
                "Remove the stale cron entry",
                "Add a lock check to the rebuild script",
            ]),
            p(&[strong("Owner: "), t("Team Orchid")]),
            para("Longer term:"),
            numbered(&["Move the rebuild to the job scheduler"]),
        ]),
        // paragraph > bulletList > paragraph > paragraph > paragraph > bulletList
        boxed(&[
            para("What went well:"),
            bullets(&["The help desk escalated quickly"]),
            para("What went badly:"),
            p(&[
                t("The runbook still pointed at the "),
                code("old-catalogue"),
                t(" host."),
            ]),
            p(&[t("Nobody owned the "), strong("cron"), t(" configuration.")]),
            bullets(&["Assign an owner", "Update the runbook"]),
        ]),
    ];
    let mut rows = vec![json!({"type": "tableRow", "content": [
        {"type": "tableHeader", "content": [p(&[strong("Time (UTC)")])]},
        {"type": "tableHeader", "content": [p(&[strong("What happened")])]}
    ]})];
    for i in 0..25 {
        let minute = 10 + i * 2;
        let event = if i % 3 == 0 {
            p(&[
                t("Job "),
                code(&format!("rebuild-{i}")),
                t(" reported progress"),
            ])
        } else {
            para(&format!("Step {i} of the recovery completed"))
        };
        rows.push(json!({"type": "tableRow", "content": [
            {"type": "tableCell", "content": [para(&format!("02:{minute:02}"))]},
            {"type": "tableCell", "content": [event]}
        ]}));
    }
    let headings = [
        ("✍️", "Summary"),
        ("📡", "Affected services"),
        ("🔍", "Root cause"),
        ("🛠️", "Recovery"),
        ("🚨", "Detection"),
        ("🕒", "Timeline summary"),
        ("✅", "Follow-up"),
        ("💬", "Retrospective"),
        ("📋", "Detailed timeline"),
    ];
    let mut content = Vec::new();
    let mut tables = boxes.into_iter().chain(std::iter::once(table(rows)));
    for (emoji, title) in headings {
        content.push(h2(emoji, title));
        if let Some(table) = tables.next() {
            content.push(table);
        }
    }
    content.push(
        json!({"type": "panel", "attrs": {"panelType": "warning"}, "content": [
            para("This page is a draft. Do not share it outside the library team.")
        ]}),
    );
    doc(Value::Array(content))
}

#[test]
fn synthetic_sample_renders_as_expected_shape() {
    let markdown = to_markdown(&synthetic_sample(), &AdfToMdOptions::default())
        .unwrap()
        .output;
    assert_eq!(markdown.matches("::: cell\n").count(), 8, "{markdown}");
    assert!(markdown.contains("## ✍️ **Summary**\n"), "{markdown}");
    assert!(
        markdown.contains("<strong>Status: </strong>Resolved."),
        "{markdown}"
    );
    assert!(
        markdown.contains("| **Time (UTC)** | **What happened** |\n| --- | --- |\n"),
        "{markdown}"
    );
    assert!(
        markdown.contains("| 02:10 | Job `rebuild-0` reported progress |"),
        "{markdown}"
    );
    assert!(markdown.ends_with(
        "> [!WARNING]\n> This page is a draft. Do not share it outside the library team.\n"
    ));
    assert!(!markdown.contains("```adf"), "{markdown}");
    let warnings = to_markdown(&synthetic_sample(), &AdfToMdOptions::default())
        .unwrap()
        .warnings;
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn synthetic_sample_round_trips() {
    assert_roundtrip(&synthetic_sample());
}

// ---------------------------------------------------------------------------------------------
// Per-feature documents
// ---------------------------------------------------------------------------------------------

fn marked(text: &str, marks: Value) -> Value {
    json!({"type": "text", "text": text, "marks": marks})
}

fn feature_docs() -> Vec<(&'static str, Value)> {
    vec![
        (
            "marks",
            doc(json!([
                p(&[
                    t("plain "),
                    marked("em", json!([{"type": "em"}])),
                    t(" "),
                    strong("strong"),
                    t(" "),
                    marked("both", json!([{"type": "em"}, {"type": "strong"}])),
                    t(" "),
                    marked("gone", json!([{"type": "strike"}])),
                    t(" "),
                    marked("under", json!([{"type": "underline"}])),
                    t(" H"),
                    marked("2", json!([{"type": "subsup", "attrs": {"type": "sub"}}])),
                    t("O "),
                    marked(
                        "red",
                        json!([{"type": "textColor", "attrs": {"color": "#ff5630"}}])
                    ),
                    t(" "),
                    marked(
                        "lit",
                        json!([{"type": "backgroundColor", "attrs": {"color": "#fffae6"}}])
                    ),
                    t(" "),
                    code("x = 1")
                ]),
                p(&[
                    strong("edge "),
                    t("text "),
                    marked(" spaced ", json!([{"type": "em"}])),
                    t("x"),
                    marked("mid", json!([{"type": "strong"}])),
                    t("word")
                ]),
            ])),
        ),
        (
            "links",
            doc(json!([p(&[
                marked(
                    "site",
                    json!([{"type": "link", "attrs": {"href": "https://example.com/a_(b)", "title": "The \"site\""}}])
                ),
                t(" and "),
                marked(
                    "cmd",
                    json!([{"type": "link", "attrs": {"href": "https://example.com/x"}}, {"type": "code"}])
                ),
                t(" and "),
                marked(
                    "bold link",
                    json!([{"type": "link", "attrs": {"href": "mailto:a@example.com"}}, {"type": "strong"}])
                ),
                t(" not a link: https://example.com www.example.com a@example.com")
            ]),])),
        ),
        (
            "escaping",
            doc(json!([
                para("# * _ ` [ ] < > | ~ ! & \\ $ ^ { }"),
                para("- 1. + > ::: cell"),
                para("1) also not a list"),
                para("  leading and trailing  "),
                p(&[
                    t("line one"),
                    json!({"type": "hardBreak"}),
                    t("- line two"),
                    json!({"type": "hardBreak"}),
                    t("  three")
                ]),
                para("Wow!"),
                p(&[
                    t("x!"),
                    marked(
                        "link",
                        json!([{"type": "link", "attrs": {"href": "https://e.com"}}])
                    )
                ]),
                para("AT&T &amp; &#32; snake_case"),
            ])),
        ),
        (
            "lists",
            doc(json!([
                {"type": "bulletList", "content": [
                    li(&[para("one"), bullets(&["one.a", "one.b"])]),
                    li(&[para("two"), para("two, second paragraph")]),
                    li(&[json!({"type": "paragraph"}), numbered(&["empty first paragraph"])]),
                    li(&[para("code"), json!({"type": "codeBlock", "attrs": {"language": "sh"}, "content": [t("ls -la\n")]})]),
                ]},
                {"type": "bulletList", "content": [li(&[para("adjacent list")])]},
                {"type": "orderedList", "attrs": {"order": 7}, "content": [li(&[para("seven")]), li(&[para("eight"), json!({"type": "orderedList", "attrs": {"order": 3}, "content": [li(&[para("three")])]})])]},
                {"type": "orderedList", "attrs": {"order": 1}, "content": [li(&[para("adjacent ordered")])]},
            ])),
        ),
        (
            "tasks",
            doc(json!([
                {"type": "taskList", "attrs": {"localId": "a"}, "content": [
                    {"type": "taskItem", "attrs": {"localId": "b", "state": "TODO"}, "content": [t("write tests")]},
                    {"type": "taskItem", "attrs": {"localId": "c", "state": "DONE"}, "content": [strong("ship"), t(" it")]},
                    {"type": "taskList", "attrs": {"localId": "d"}, "content": [
                        {"type": "taskItem", "attrs": {"localId": "e", "state": "TODO"}, "content": [t("nested")]}
                    ]},
                ]},
            ])),
        ),
        (
            "blocks",
            doc(json!([
                {"type": "heading", "attrs": {"level": 1}, "content": [t("Title "), code("v2"), json!({"type": "hardBreak"}), t("second line #")]},
                {"type": "codeBlock", "content": [t("```\nfenced inside\n```")]},
                {"type": "codeBlock", "attrs": {"language": "json"}},
                {"type": "blockquote", "content": [para("quoted"), bullets(&["in a quote"]), {"type": "codeBlock", "content": [t("q")]}]},
                {"type": "rule"},
                {"type": "panel", "attrs": {"panelType": "info"}, "content": [
                    {"type": "heading", "attrs": {"level": 3}, "content": [t("Panel heading")]},
                    para("panel text"),
                    {"type": "rule"},
                    {"type": "taskList", "attrs": {"localId": "p"}, "content": [{"type": "taskItem", "attrs": {"localId": "q", "state": "TODO"}, "content": [t("in panel")]}]},
                ]},
                {"type": "panel", "attrs": {"panelType": "error"}, "content": [{"type": "paragraph"}]},
                {"type": "expand", "attrs": {"title": "Details & <more>"}, "content": [
                    para("inside"),
                    {"type": "nestedExpand", "attrs": {"title": "Nested"}, "content": [para("deeper")]},
                    table(vec![
                        json!({"type": "tableRow", "content": [{"type": "tableHeader", "content": [para("h")]}]}),
                        json!({"type": "tableRow", "content": [{"type": "tableCell", "content": [para("c")]}]}),
                    ]),
                ]},
                {"type": "expand", "attrs": {}, "content": [{"type": "paragraph"}]},
            ])),
        ),
        (
            "tables",
            doc(json!([
                table(vec![
                    json!({"type": "tableRow", "content": [
                        {"type": "tableHeader", "content": [para("left")]},
                        {"type": "tableHeader", "content": [{"type": "paragraph", "marks": [{"type": "alignment", "attrs": {"align": "center"}}], "content": [t("centre")]}]},
                        {"type": "tableHeader", "content": [{"type": "paragraph", "marks": [{"type": "alignment", "attrs": {"align": "end"}}], "content": [t("right")]}]},
                    ]}),
                    json!({"type": "tableRow", "content": [
                        {"type": "tableCell", "content": [p(&[code("a|b"), t(" pipe | text")])]},
                        {"type": "tableCell", "content": [{"type": "paragraph", "marks": [{"type": "alignment", "attrs": {"align": "center"}}], "content": [t("x"), {"type": "hardBreak"}, t("y")]}]},
                        {"type": "tableCell", "content": [{"type": "paragraph", "marks": [{"type": "alignment", "attrs": {"align": "end"}}]}]},
                    ]}),
                ]),
                boxed(&[
                    json!({"type": "panel", "attrs": {"panelType": "note"}, "content": [para("panel in a box")]}),
                    json!({"type": "blockquote", "content": [para("quote in a box")]}),
                    json!({"type": "nestedExpand", "attrs": {"title": "more"}, "content": [para("expand in a box")]}),
                    json!({"type": "codeBlock", "content": [t("::: cell\n:::")]}),
                ]),
                boxed(&[json!({"type": "paragraph"})]),
            ])),
        ),
        (
            "inline nodes",
            doc(json!([p(&[
                json!({"type": "mention", "attrs": {"id": "557058:0a1b2c3d", "text": "@Sam Example"}}),
                t(" set "),
                json!({"type": "status", "attrs": {"text": "IN PROGRESS", "color": "blue"}}),
                t(" on "),
                json!({"type": "date", "attrs": {"timestamp": "1792368000000"}}),
                t(" "),
                json!({"type": "emoji", "attrs": {"shortName": ":tada:", "text": "🎉"}}),
                t(" "),
                json!({"type": "emoji", "attrs": {"shortName": ":custom-thing:"}}),
                t(" see "),
                json!({"type": "inlineCard", "attrs": {"url": "https://example.atlassian.net/browse/LIB-42"}}),
            ]),])),
        ),
        (
            "passthrough",
            doc(json!([
                {"type": "decisionList", "attrs": {"localId": "d1"}, "content": [
                    {"type": "decisionItem", "attrs": {"localId": "d2", "state": "DECIDED"}, "content": [t("Use the scheduler")]}
                ]},
                {"type": "paragraph"},
                {"type": "panel", "attrs": {"panelType": "custom", "panelColor": "#e3fcef"}, "content": [para("custom panel")]},
                table(vec![json!({"type": "tableRow", "content": [
                    {"type": "tableCell", "content": [para("no header")]},
                    {"type": "tableCell", "content": [para("row")]}
                ]})]),
                // A nested passthrough must be valid ADF where it stands (a decisionList is not
                // allowed in a list item), or the ```adf block cannot convert back.
                {"type": "bulletList", "content": [li(&[para("item"), json!({"type": "mediaSingle", "attrs": {"layout": "center"}, "content": [
                    {"type": "media", "attrs": {"type": "file", "id": "6e7c7f2c-1b2a-4c3d-9e8f-0a1b2c3d4e5f", "collection": "contentId-1"}}
                ]})])]},
            ])),
        ),
    ]
}

#[test]
fn feature_documents_round_trip() {
    for (name, document) in feature_docs() {
        eprintln!("round trip: {name}");
        assert_roundtrip(&document);
    }
}

#[test]
fn external_images_round_trip_with_images_external() {
    let document = doc(json!([
        {"type": "mediaSingle", "attrs": {"layout": "center"}, "content": [
            {"type": "media", "attrs": {"type": "external", "url": "https://example.com/chart.png", "alt": "A chart"}}
        ]},
        {"type": "mediaSingle", "attrs": {"layout": "center"}, "content": [
            {"type": "media", "attrs": {"type": "external", "url": "https://example.com/plain.png"}}
        ]},
    ]));
    let opts = MdToAdfOptions {
        images: ImagePolicy::External,
        ..MdToAdfOptions::default()
    };
    assert_roundtrip_with(&document, &opts);
}

#[test]
fn feature_documents_render_without_internal_errors() {
    for (name, document) in feature_docs() {
        let result = to_markdown(&document, &AdfToMdOptions::default());
        assert!(result.is_ok(), "{name}: {result:?}");
        let output = result.unwrap_or_else(|_| unreachable!()).output;
        assert!(
            output.ends_with('\n') && !output.ends_with("\n\n"),
            "{name}: {output:?}"
        );
    }
}

#[test]
fn only_the_passthrough_document_uses_adf_blocks() {
    for (name, document) in feature_docs() {
        let result = to_markdown(&document, &AdfToMdOptions::default()).unwrap();
        let has_raw = result.output.contains("```adf");
        assert_eq!(has_raw, name == "passthrough", "{name}:\n{}", result.output);
    }
}

// ---------------------------------------------------------------------------------------------
// Self-contained: block structure as CommonMark/GFM (pulldown-cmark) sees it. Box cells and
// details are handled by Markdown → ADF's own pre-pass, so they are left out here.
// ---------------------------------------------------------------------------------------------

/// A compact block/inline skeleton of `markdown`: `Tag[…]` for containers, `'text'` for text.
fn skeleton(markdown: &str) -> String {
    use pulldown_cmark::{Event, Parser};
    let mut out = String::new();
    for event in Parser::new_ext(markdown, super::verify::options()) {
        if let Event::Start(tag) = &event {
            let name = format!("{tag:?}");
            let name = name
                .split(['(', ' ', '{'])
                .next()
                .unwrap_or_default()
                .to_owned();
            out.push_str(&name);
            out.push('[');
        } else if let Event::End(_) = &event {
            out.push(']');
        } else if let Event::Text(text) | Event::Code(text) = &event {
            out.push('\'');
            out.push_str(text);
            out.push('\'');
        } else if let Event::TaskListMarker(done) = &event {
            out.push_str(if *done { "[x]" } else { "[ ]" });
        } else if let Event::Rule = &event {
            out.push_str("---");
        } else if let Event::HardBreak = &event {
            out.push('/');
        } else {
            out.push_str("<?>");
        }
    }
    out
}

fn rendered(name: &str) -> String {
    let document = feature_docs()
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, d)| d)
        .unwrap_or_default();
    to_markdown(&document, &AdfToMdOptions::default())
        .unwrap()
        .output
}

#[test]
fn lists_parse_as_the_intended_structure() {
    assert_eq!(
        skeleton(&rendered("lists")),
        "List[Item[Paragraph['one']List[Item['one.a']Item['one.b']]]\
         Item[Paragraph['two']Paragraph['two, second paragraph']]\
         Item[List[Item['empty first paragraph']]]\
         Item[Paragraph['code']CodeBlock['ls -la\n\n']]]\
         List[Item['adjacent list']]\
         List[Item[Paragraph['seven']]Item[Paragraph['eight']List[Item['three']]]]\
         List[Item['adjacent ordered']]"
    );
}

#[test]
fn tasks_parse_as_the_intended_structure() {
    assert_eq!(
        skeleton(&rendered("tasks")),
        "List[Item[[ ]'write tests']Item[[x]Strong['ship']' it'List[Item[[ ]'nested']]]]"
    );
}

#[test]
fn quotes_alerts_and_code_parse_as_the_intended_structure() {
    let markdown = rendered("blocks");
    let before_details = markdown.split("<details>").next().unwrap_or_default();
    assert_eq!(
        skeleton(before_details),
        "Heading['Title ''v2'<?>'second line ''#']\
         CodeBlock['```\nfenced inside\n```\n']\
         CodeBlock[]\
         BlockQuote[Paragraph['quoted']List[Item['in a quote']]CodeBlock['q\n']]\
         ---\
         BlockQuote[Heading['Panel heading']Paragraph['panel text']---List[Item[[ ]'in panel']]]\
         BlockQuote[]"
    );
    assert!(before_details.contains("> [!NOTE]\n> ### Panel heading"));
    assert!(before_details.contains("> [!CAUTION]\n"));
}

#[test]
fn escaped_text_parses_as_text() {
    assert_eq!(
        skeleton(&rendered("escaping")),
        "Paragraph['# ''* ''_ ''`'' ''[ ''] ''< > ''| ''~ ! & ''\\ ''$ ''^ { }']\
         Paragraph['- 1. + > ::: cell']\
         Paragraph['1'') also not a list']\
         Paragraph[' '' ''leading and trailing'' '' ']\
         Paragraph['line one'/'- line two'/' '' ''three']\
         Paragraph['Wow!']\
         Paragraph['x''!'Link['link']]\
         Paragraph['AT''&T ''&amp; ''&#32; snake_case']"
    );
}

// ---------------------------------------------------------------------------------------------
// Self-contained: escaped text re-parses to the same characters under CommonMark/GFM.
// ---------------------------------------------------------------------------------------------

mod reparse {
    use proptest::prelude::*;
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    use serde_json::json;

    use super::super::to_markdown;
    use super::super::verify::options;
    use crate::options::AdfToMdOptions;

    fn text_of(markdown: &str) -> Option<String> {
        let mut out = String::new();
        for event in Parser::new_ext(markdown, options()) {
            match event {
                Event::Text(t) => out.push_str(&t),
                Event::HardBreak => out.push('\n'),
                Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => {}
                Event::Start(_)
                | Event::End(_)
                | Event::Code(_)
                | Event::InlineMath(_)
                | Event::DisplayMath(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::FootnoteReference(_)
                | Event::SoftBreak
                | Event::Rule
                | Event::TaskListMarker(_) => return None,
            }
        }
        Some(out)
    }

    fn tricky() -> impl Strategy<Value = String> {
        let pieces = prop::sample::select(vec![
            "a",
            "b",
            "1",
            "9.",
            "0)",
            " ",
            "  ",
            "\t",
            "\n",
            "*",
            "**",
            "_",
            "__",
            "`",
            "``",
            "~",
            "~~",
            "[",
            "]",
            "(",
            ")",
            "<",
            ">",
            "#",
            "-",
            "+",
            "=",
            "|",
            "!",
            "&",
            "&amp;",
            "&#32;",
            "\\",
            ":",
            "://",
            "www.",
            "@",
            "$",
            "^",
            "{",
            "}",
            "\"",
            "'",
            ".",
            ",",
            "é",
            "😄",
            ":::",
            "```",
            "---",
            "<br>",
            "<u>",
            "x@y.z",
            "https://e.com",
        ]);
        prop::collection::vec(pieces, 1..12).prop_map(|v| v.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Any text becomes a paragraph that parses back to exactly that text (with `\n` as a
        /// hard break), and never needs a raw block.
        #[test]
        fn text_survives_commonmark(text in tricky()) {
            // Trailing hard breaks are dropped by design; an empty paragraph passes through.
            let trimmed = text.trim_end_matches('\n');
            let trimmed = if trimmed.is_empty() { "x" } else { trimmed };
            let document = json!({"type": "doc", "version": 1, "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": trimmed}]}
            ]});
            let markdown = to_markdown(&document, &AdfToMdOptions::default()).unwrap().output;
            prop_assert!(!markdown.contains("```adf"), "passthrough for {:?}:\n{}", trimmed, markdown);
            let parsed = text_of(&markdown);
            prop_assert_eq!(parsed.as_deref(), Some(trimmed), "markdown: {:?}", markdown);
        }

        /// Marked text keeps its marks: the renderer verified it, so it must not fall back.
        #[test]
        fn marked_text_never_needs_passthrough(
            before in tricky(), inner in tricky(), after in tricky(),
            mark in prop::sample::select(vec!["em", "strong", "strike", "underline", "code"]),
        ) {
            let clean = |s: &str| s.replace('\n', " ");
            let (before, inner, after) = (clean(&before), clean(&inner), clean(&after));
            let document = json!({"type": "doc", "version": 1, "content": [
                {"type": "paragraph", "content": [
                    {"type": "text", "text": before},
                    {"type": "text", "text": inner, "marks": [{"type": mark}]},
                    {"type": "text", "text": after}
                ]}
            ]});
            let markdown = to_markdown(&document, &AdfToMdOptions::default()).unwrap().output;
            prop_assert!(!markdown.contains("```adf"), "passthrough:\n{}", markdown);
        }
    }
}
