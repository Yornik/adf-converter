//! Attached files in ADF → Markdown: the `adf:attachment` link syntax (roundtrip flavour) for a
//! `mediaGroup` and a `mediaInline`, and what keeps a raw block.

#![allow(clippy::needless_pass_by_value, clippy::too_many_lines)]

use serde_json::{Value, json};

use super::to_markdown;
use crate::diagnostics::Code;
use crate::options::{AdfToMdOptions, MdFlavor};

const A: &str = "66666666-7777-8888-9999-000000000000";
const B: &str = "66666666-7777-8888-9999-000000000001";

fn convert(content: Value, flavor: MdFlavor) -> crate::Conversion<String> {
    let doc = json!({"type": "doc", "version": 1, "content": content});
    let opts = AdfToMdOptions {
        flavor,
        ..AdfToMdOptions::default()
    };
    to_markdown(&doc, &opts).unwrap()
}

fn md(content: Value) -> String {
    convert(content, MdFlavor::Roundtrip).output
}

fn plain(content: Value) -> String {
    convert(content, MdFlavor::Plain).output
}

fn codes(content: Value) -> Vec<Code> {
    // (Several of the documents below are invalid ADF on purpose.)
    convert(content, MdFlavor::Roundtrip)
        .warnings
        .into_iter()
        .map(|w| w.code)
        .filter(|&code| code != Code::InputSchema)
        .collect()
}

fn media(attrs: Value) -> Value {
    let mut base = json!({"type": "file", "id": A, "collection": "", "alt": "report.pdf"});
    if let (Some(base), Some(extra)) = (base.as_object_mut(), attrs.as_object()) {
        for (k, v) in extra {
            if v.is_null() {
                base.remove(k);
            } else {
                base.insert(k.clone(), v.clone());
            }
        }
    }
    json!({"type": "media", "attrs": base})
}

fn group(files: Vec<Value>) -> Value {
    json!({"type": "mediaGroup", "content": files})
}

fn inline(attrs: Value) -> Value {
    let mut node = media(attrs);
    node["type"] = json!("mediaInline");
    node
}

fn t(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn p(inlines: Vec<Value>) -> Value {
    json!({"type": "paragraph", "content": inlines})
}

fn is_raw(markdown: &str) -> bool {
    markdown.contains("```adf")
}

// ---------------------------------------------------------------------------------------------
// mediaGroup
// ---------------------------------------------------------------------------------------------

#[test]
fn a_group_is_one_paragraph_of_links_joined_by_a_space() {
    let content = json!([group(vec![
        media(json!({})),
        media(json!({"id": B, "collection": "contentId-123456", "alt": "trace.zip"}))
    ])]);
    assert_eq!(
        md(content.clone()),
        format!(
            "[report.pdf](adf:attachment/{A}) [trace.zip](adf:attachment/{B}?collection=contentId-123456)\n"
        )
    );
    assert_eq!(codes(content), []);
}

#[test]
fn a_group_of_one_and_upper_case_ids() {
    let id = "AAAAAAAA-BBBB-2222-CCCC-DDDDEEEEFFFF";
    assert_eq!(
        md(json!([group(vec![media(json!({"id": id}))])])),
        format!("[report.pdf](adf:attachment/{id})\n")
    );
}

#[test]
fn file_names_are_escaped_so_that_they_read_back_identically() {
    for alt in [
        "a [1] b",
        "*x* _y_ `z`",
        "a\\b",
        "<b> & c",
        "&amp; &#35; &copy;",
        "a | b",
        "a  b",
        "a](adf:attachment/x) [b",
        "![img](https://e.com/i.png)",
        "https://example.com/a_b_c www.example.com a@b.co",
        "# not a heading",
        "1. not a list",
        "> not a quote",
        "- not a list",
        "~~strike~~ <u>u</u>",
        "[ref]: https://e.com",
        "日本語.pdf",
        "a\tb",
    ] {
        let out = md(json!([group(vec![media(json!({"alt": alt}))])]));
        assert!(!is_raw(&out), "{alt:?}: {out}");
        let back = crate::md_to_adf(&out, &crate::options::MdToAdfOptions::default())
            .unwrap()
            .output
            .to_value()
            .unwrap();
        assert_eq!(
            back["content"],
            json!([group(vec![media(json!({"alt": alt}))])]),
            "{alt:?}: {out}"
        );
    }
}

#[test]
fn what_the_link_cannot_say_keeps_the_raw_block() {
    let cases: Vec<(&str, Value)> = vec![
        ("localId", media(json!({"localId": "x"}))),
        ("width", media(json!({"width": 10}))),
        ("height", media(json!({"height": 10}))),
        ("occurrenceKey", media(json!({"occurrenceKey": "k"}))),
        ("type link", media(json!({"type": "link"}))),
        (
            "type external",
            media(json!({"type": "external", "url": "https://e.com/a"})),
        ),
        ("id not a uuid", media(json!({"id": "legacy-id"}))),
        ("id numeric", media(json!({"id": "10001"}))),
        ("collection charset", media(json!({"collection": "a b"}))),
        (
            "collection too long",
            media(json!({"collection": "a".repeat(65)})),
        ),
        ("no collection", media(json!({"collection": null}))),
        ("no alt", media(json!({"alt": null}))),
        ("empty alt", media(json!({"alt": ""}))),
        ("alt not a string", media(json!({"alt": 5}))),
        ("alt with a line break", media(json!({"alt": "a\nb"}))),
        ("alt with a carriage return", media(json!({"alt": "a\rb"}))),
        (
            "alt with a control character",
            media(json!({"alt": "a\u{7}b"})),
        ),
        ("alt with leading space", media(json!({"alt": " a"}))),
        ("alt with trailing space", media(json!({"alt": "a "}))),
        (
            "alt with a no-break space edge",
            media(json!({"alt": "a\u{a0}"})),
        ),
    ];
    for (name, bad) in cases {
        for content in [
            json!([group(vec![bad.clone()])]),
            // One bad file keeps the whole group raw.
            json!([group(vec![media(json!({})), bad.clone()])]),
        ] {
            let out = md(content.clone());
            assert!(is_raw(&out), "{name}: {out}");
            assert!(!out.contains("adf:attachment"), "{name}: {out}");
            assert_eq!(codes(content), [Code::AdfPassthrough], "{name}");
        }
    }
}

#[test]
fn marks_attributes_and_other_children_keep_the_raw_block() {
    let mut marked = media(json!({}));
    marked["marks"] = json!([{"type": "border", "attrs": {"color": "#000000", "size": 1}}]);
    let mut with_content = media(json!({}));
    with_content["content"] = json!([t("x")]);
    let mut group_attr = group(vec![media(json!({}))]);
    group_attr["attrs"] = json!({"localId": "g"});
    let mut group_mark = group(vec![media(json!({}))]);
    group_mark["marks"] = json!([{"type": "border"}]);
    for (name, content) in [
        ("mark on a media", group(vec![marked])),
        ("children of a media", group(vec![with_content])),
        ("attrs on the group", group_attr),
        ("marks on the group", group_mark),
        ("empty group", group(vec![])),
        ("a non-media child", group(vec![media(json!({})), t("x")])),
        ("a mediaInline child", group(vec![inline(json!({}))])),
    ] {
        let out = md(json!([content.clone()]));
        assert!(is_raw(&out), "{name}: {out}");
        assert_eq!(codes(json!([content])), [Code::AdfPassthrough], "{name}");
    }
}

#[test]
fn the_passthrough_message_names_what_to_change() {
    let doc = json!({"type": "doc", "version": 1, "content": [group(vec![media(json!({"id": "legacy-id"}))])]});
    let out = to_markdown(&doc, &AdfToMdOptions::default()).unwrap();
    let message = &out.warnings[0].message;
    assert!(message.contains("mediaGroup"), "{message}");
    assert!(message.contains("not a UUID"), "{message}");
    assert_eq!(
        out.warnings[0]
            .location
            .as_ref()
            .and_then(|l| l.pointer.as_deref()),
        Some("/content/0")
    );
}

#[test]
fn a_group_is_written_where_markdown_reads_it_back_as_a_group() {
    let g = || group(vec![media(json!({}))]);
    let link = format!("[report.pdf](adf:attachment/{A})");
    assert_eq!(md(json!([g()])), format!("{link}\n"));
    assert_eq!(
        md(json!([{"type": "blockquote", "content": [g()]}])),
        format!("> {link}\n")
    );
    assert_eq!(
        md(json!([{"type": "panel", "attrs": {"panelType": "info"}, "content": [g()]}])),
        format!("> [!NOTE]\n> {link}\n")
    );
    let expand = md(json!([{"type": "expand", "attrs": {"title": "T"}, "content": [g()]}]));
    assert!(expand.contains(&link) && !is_raw(&expand), "{expand}");
    let boxed = md(
        json!([{"type": "table", "content": [{"type": "tableRow", "content": [
        {"type": "tableCell", "content": [g()]}]}]}]),
    );
    assert!(boxed.contains(&link) && !is_raw(&boxed), "{boxed}");
}

#[test]
fn a_group_in_a_list_item_or_a_gfm_table_cell_keeps_the_raw_block() {
    // Markdown → ADF cannot make a group there: a list item cannot hold one, and a GFM cell is
    // one paragraph.
    let g = || group(vec![media(json!({}))]);
    let list = md(
        json!([{"type": "bulletList", "content": [{"type": "listItem", "content": [
        p(vec![t("x")]), g()]}]}]),
    );
    assert!(is_raw(&list), "{list}");
    let table = md(json!([{"type": "table", "content": [
        {"type": "tableRow", "content": [
            {"type": "tableHeader", "content": [p(vec![t("h")])]},
            {"type": "tableHeader", "content": [p(vec![t("i")])]}]},
        {"type": "tableRow", "content": [
            {"type": "tableCell", "content": [g()]},
            {"type": "tableCell", "content": [p(vec![t("x")])]}]}]}]));
    assert!(is_raw(&table), "{table}");
}

#[test]
fn plain_flavour_keeps_its_markers() {
    assert_eq!(
        plain(json!([group(vec![
            media(json!({})),
            media(json!({"alt": null}))
        ])])),
        "[attachment: report.pdf]\\\n[attachment]\n"
    );
    assert_eq!(
        plain(json!([p(vec![t("see "), inline(json!({})), t(" now")])])),
        "see [attachment: report.pdf] now\n"
    );
}

// ---------------------------------------------------------------------------------------------
// mediaInline
// ---------------------------------------------------------------------------------------------

#[test]
fn an_inline_file_amid_text_is_a_link() {
    let content = json!([p(vec![
        t("see "),
        inline(json!({"collection": "contentId-123456"})),
        t(" for details")
    ])]);
    assert_eq!(
        md(content.clone()),
        format!("see [report.pdf](adf:attachment/{A}?collection=contentId-123456) for details\n")
    );
    assert_eq!(codes(content), []);
}

#[test]
fn inline_files_in_every_inline_context() {
    let link = format!("[report.pdf](adf:attachment/{A})");
    let heading = md(
        json!([{"type": "heading", "attrs": {"level": 2}, "content": [
        t("H "), inline(json!({}))]}]),
    );
    assert_eq!(heading, format!("## H {link}\n"));
    // Where Markdown → ADF makes no group, a lone file is a link: a list item, its second
    // paragraph, a heading, a task item and a GFM cell.
    let list = md(json!([{"type": "bulletList", "content": [
        {"type": "listItem", "content": [p(vec![inline(json!({}))]), p(vec![inline(json!({}))])]}]}]));
    assert_eq!(list, format!("- {link}\n\n  {link}\n"));
    let lone_heading =
        md(json!([{"type": "heading", "attrs": {"level": 1}, "content": [inline(json!({}))]}]));
    assert_eq!(lone_heading, format!("# {link}\n"));
    let task = md(
        json!([{"type": "taskList", "attrs": {"localId": "l"}, "content": [
        {"type": "taskItem", "attrs": {"localId": "i", "state": "TODO"}, "content": [t("t "), inline(json!({}))]}]}]),
    );
    assert_eq!(task, format!("- [ ] t {link}\n"));
    let table = md(json!([{"type": "table", "content": [
        {"type": "tableRow", "content": [
            {"type": "tableHeader", "content": [p(vec![t("h")])]},
            {"type": "tableHeader", "content": [p(vec![t("i")])]}]},
        {"type": "tableRow", "content": [
            {"type": "tableCell", "content": [p(vec![inline(json!({"alt": "a|b"}))])]},
            {"type": "tableCell", "content": [p(vec![t("x")])]}]}]}]));
    assert!(
        table.contains(&format!("| [a\\|b](adf:attachment/{A}) | x |")),
        "{table}"
    );
    assert!(!is_raw(&table), "{table}");
}

#[test]
fn what_the_inline_link_cannot_say_keeps_the_paragraph_raw() {
    let text = |node: Value| p(vec![t("see "), node, t(" now")]);
    let mut marked = inline(json!({}));
    marked["marks"] = json!([{"type": "link", "attrs": {"href": "https://e.com"}}]);
    for (name, node) in [
        ("no alt (real Jira data)", inline(json!({"alt": null}))),
        ("localId", inline(json!({"localId": "x"}))),
        ("width", inline(json!({"width": 1}))),
        ("occurrenceKey", inline(json!({"occurrenceKey": "k"}))),
        ("type image", inline(json!({"type": "image"}))),
        ("type link", inline(json!({"type": "link"}))),
        ("id", inline(json!({"id": "legacy"}))),
        ("collection", inline(json!({"collection": "a.b"}))),
        ("alt with a line break", inline(json!({"alt": "a\nb"}))),
        ("alt with an edge space", inline(json!({"alt": "a "}))),
        ("a mark", marked),
    ] {
        let content = json!([text(node)]);
        let out = md(content.clone());
        assert!(is_raw(&out), "{name}: {out}");
        assert_eq!(codes(content), [Code::AdfPassthrough], "{name}");
    }
}

// ---------------------------------------------------------------------------------------------
// A paragraph that Markdown would read back as a group
// ---------------------------------------------------------------------------------------------

/// The pieces of a paragraph: a file, one and two spaces, a hard break, a text line break and a
/// word.
fn piece(i: usize) -> Value {
    match i {
        0 => inline(json!({})),
        1 => t(" "),
        2 => t("  "),
        3 => json!({"type": "hardBreak"}),
        4 => t("\n"),
        _ => t("x"),
    }
}

#[test]
fn a_paragraph_that_reads_back_as_a_group_keeps_the_raw_block() {
    // Every file in the document container: spaces and breaks between files make Markdown →
    // ADF build a group, so writing the links would change the node.
    for content in [
        vec![inline(json!({}))],
        vec![t(" "), inline(json!({}))],
        vec![inline(json!({})), t(" "), inline(json!({"id": B}))],
        vec![
            inline(json!({})),
            json!({"type": "hardBreak"}),
            inline(json!({"id": B})),
        ],
        vec![inline(json!({})), t("\t"), inline(json!({"id": B})), t(" ")],
    ] {
        for container in [
            |c: Value| c,
            |c: Value| json!({"type": "blockquote", "content": [c]}),
            |c: Value| json!({"type": "panel", "attrs": {"panelType": "info"}, "content": [c]}),
            |c: Value| json!({"type": "expand", "attrs": {"title": "T"}, "content": [c]}),
        ] {
            let node = container(p(content.clone()));
            let out = md(json!([node.clone()]));
            assert!(is_raw(&out), "{content:?}: {out}");
            assert!(!out.contains("adf:attachment"), "{content:?}: {out}");
            assert_eq!(codes(json!([node])), [Code::AdfPassthrough]);
        }
    }
}

#[test]
fn a_paragraph_that_does_not_read_back_as_a_group_is_written_as_links() {
    for content in [
        // Text around, no gap between two files, a gap-less pair among gaps.
        vec![t("x"), inline(json!({}))],
        vec![inline(json!({})), t(" and")],
        vec![inline(json!({})), inline(json!({"id": B}))],
        vec![
            inline(json!({})),
            t(" "),
            inline(json!({"id": B})),
            inline(json!({})),
        ],
        vec![inline(json!({})), t(" x "), inline(json!({"id": B}))],
    ] {
        let out = md(json!([p(content.clone())]));
        assert!(!is_raw(&out), "{content:?}: {out}");
        assert!(out.contains("adf:attachment/"), "{content:?}: {out}");
    }
}

/// The writer's test for "reads back as a group" must be the reader's test: for every short
/// paragraph, either the Markdown is raw ADF or it reads back as the same ADF.
#[test]
fn every_short_paragraph_of_files_and_gaps_is_exact_or_raw() {
    use crate::options::MdToAdfOptions;
    let mut sequences: Vec<Vec<usize>> = vec![vec![]];
    for _ in 0..4 {
        let next: Vec<Vec<usize>> = sequences
            .iter()
            .filter(|s| s.len() < 4)
            .flat_map(|s| {
                (0..6).map(move |i| {
                    let mut longer = s.clone();
                    longer.push(i);
                    longer
                })
            })
            .collect();
        sequences.extend(next);
        sequences.sort();
        sequences.dedup();
    }
    let mut exact = 0;
    let mut raw = 0;
    for sequence in sequences.iter().filter(|s| !s.is_empty()) {
        let nodes: Vec<Value> = sequence.iter().map(|&i| piece(i)).collect();
        // Adjacent texts merge in ADF; build the document the way a reader would see it.
        let mut merged: Vec<Value> = Vec::new();
        for node in nodes {
            match (merged.last_mut(), node["type"] == "text") {
                (Some(last), true) if last["type"] == "text" => {
                    let more = node["text"].as_str().unwrap_or_default().to_owned();
                    let text = format!("{}{more}", last["text"].as_str().unwrap_or_default());
                    last["text"] = json!(text);
                }
                _ => merged.push(node),
            }
        }
        let doc = json!({"type": "doc", "version": 1, "content": [p(merged.clone())]});
        let converted = to_markdown(&doc, &AdfToMdOptions::default()).unwrap();
        if is_raw(&converted.output) {
            raw += 1;
            continue;
        }
        let back = crate::md_to_adf(&converted.output, &MdToAdfOptions::default())
            .unwrap()
            .output
            .to_value()
            .unwrap();
        // A line break in text is a hard break, and Markdown cannot end a paragraph with one.
        let mut expected: Vec<Value> = Vec::new();
        for node in merged {
            match node["text"].as_str().filter(|s| s.contains('\n')) {
                Some(text) => {
                    for (i, part) in text.split('\n').enumerate() {
                        if i > 0 {
                            expected.push(json!({"type": "hardBreak"}));
                        }
                        if !part.is_empty() {
                            expected.push(t(part));
                        }
                    }
                }
                None => expected.push(node),
            }
        }
        while expected.last().is_some_and(|n| n["type"] == "hardBreak") {
            expected.pop();
        }
        if expected.is_empty() {
            continue;
        }
        assert_eq!(
            back["content"],
            json!([p(expected)]),
            "{sequence:?}\n{}",
            converted.output
        );
        exact += 1;
    }
    assert!(exact > 50 && raw > 50, "exact {exact}, raw {raw}");
}
