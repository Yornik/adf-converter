//! Unit tests for ADF → Markdown: exact Markdown per node and mark, escaping and flanking edge
//! cases, passthrough, warnings, and the plain flavour.

#![allow(clippy::needless_pass_by_value)]

use serde_json::{Value, json};

use super::to_markdown;
use crate::diagnostics::Code;
use crate::limits::Limits;
use crate::options::{AdfToMdOptions, MdFlavor};

/// No warnings, for `assert_eq!` against [`warnings`].
const NO_WARNINGS: &[(Code, String)] = &[];

fn doc(content: Value) -> Value {
    json!({"type": "doc", "version": 1, "content": content})
}

fn opts(flavor: MdFlavor) -> AdfToMdOptions {
    AdfToMdOptions {
        flavor,
        limits: Limits::default(),
    }
}

/// Roundtrip Markdown for a document with the given content.
fn md(content: Value) -> String {
    to_markdown(&doc(content), &opts(MdFlavor::Roundtrip))
        .unwrap()
        .output
}

/// Plain Markdown for a document with the given content.
fn plain(content: Value) -> String {
    to_markdown(&doc(content), &opts(MdFlavor::Plain))
        .unwrap()
        .output
}

/// `(code, pointer)` of every warning (roundtrip).
fn warnings(content: Value) -> Vec<(Code, String)> {
    to_markdown(&doc(content), &opts(MdFlavor::Roundtrip))
        .unwrap()
        .warnings
        .into_iter()
        .map(|w| {
            (
                w.code,
                w.location.and_then(|l| l.pointer).unwrap_or_default(),
            )
        })
        .collect()
}

fn t(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn tm(text: &str, marks: &[&str]) -> Value {
    let marks: Vec<Value> = marks.iter().map(|m| json!({"type": m})).collect();
    json!({"type": "text", "text": text, "marks": marks})
}

fn link(text: &str, href: &str) -> Value {
    json!({"type": "text", "text": text, "marks": [{"type": "link", "attrs": {"href": href}}]})
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

fn bullets(items: &[Value]) -> Value {
    json!({"type": "bulletList", "content": items})
}

fn ordered(order: u64, items: &[Value]) -> Value {
    json!({"type": "orderedList", "attrs": {"order": order}, "content": items})
}

fn cell(kind: &str, blocks: &[Value]) -> Value {
    json!({"type": kind, "content": blocks})
}

fn row(cells: &[Value]) -> Value {
    json!({"type": "tableRow", "content": cells})
}

fn table(rows: &[Value]) -> Value {
    json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"}, "content": rows})
}

// ---------------------------------------------------------------------------------------------
// Document shape
// ---------------------------------------------------------------------------------------------

#[test]
fn empty_document_is_one_newline() {
    assert_eq!(md(json!([])), "\n");
    assert_eq!(plain(json!([])), "\n");
}

#[test]
fn blocks_are_separated_by_blank_lines_and_end_with_one_newline() {
    assert_eq!(md(json!([para("a"), para("b")])), "a\n\nb\n");
}

#[test]
fn rejects_non_adf_and_deep_input() {
    let err = to_markdown(&json!({"type": "paragraph"}), &AdfToMdOptions::default()).unwrap_err();
    assert_eq!(err.0[0].code, Code::InputNotAdf);
    let mut deep = para("x");
    for _ in 0..40 {
        deep = json!({"type": "blockquote", "content": [deep]});
    }
    let err = to_markdown(&doc(json!([deep])), &AdfToMdOptions::default()).unwrap_err();
    assert_eq!(err.0[0].code, Code::InputTooDeep);
    assert_eq!(err.exit_code(), 3);
}

#[test]
fn output_is_deterministic() {
    let content = json!([
        {"type": "heading", "attrs": {"level": 2}, "content": [t("✍️ "), tm("Summary", &["strong"])]},
        {"type": "decisionList", "attrs": {"localId": "x"}, "content": []},
        bullets(&[li(&[para("a")])]),
    ]);
    let first = to_markdown(&doc(content.clone()), &AdfToMdOptions::default()).unwrap();
    for _ in 0..5 {
        assert_eq!(
            to_markdown(&doc(content.clone()), &AdfToMdOptions::default()).unwrap(),
            first
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Text escaping
// ---------------------------------------------------------------------------------------------

#[test]
fn markdown_specials_in_text_are_escaped() {
    assert_eq!(
        md(json!([para(r"a*b_c `d` [e](f) <g> h|i ~j~ \k")])),
        "a\\*b_c \\`d\\` \\[e\\](f) \\<g> h\\|i \\~j\\~ \\\\k\n"
    );
}

#[test]
fn intraword_underscores_stay_readable() {
    assert_eq!(md(json!([para("snake_case_name")])), "snake_case_name\n");
}

#[test]
fn block_syntax_at_line_start_is_escaped() {
    for (text, expected) in [
        ("# not a heading", "\\# not a heading"),
        ("- not a list", "\\- not a list"),
        ("+ not a list", "\\+ not a list"),
        ("1. not a list", "1\\. not a list"),
        ("2) not a list", "2\\) not a list"),
        ("> not a quote", "\\> not a quote"),
        ("---", "\\---"),
        ("===", "\\==="),
        ("::: cell", "\\::: cell"),
        ("```", "\\`\\`\\`"),
        ("    indented", "&#32;&#32;&#32;&#32;indented"),
        ("<div>", "\\<div>"),
    ] {
        assert_eq!(md(json!([para(text)])), format!("{expected}\n"), "{text}");
    }
}

#[test]
fn leading_and_trailing_spaces_use_character_references() {
    assert_eq!(md(json!([para("  two  ")])), "&#32;&#32;two&#32;&#32;\n");
    assert_eq!(md(json!([para("\ttab")])), "&#9;tab\n");
}

#[test]
fn unlinked_urls_are_not_autolinked() {
    assert_eq!(
        md(json!([para(
            "see https://example.com or www.example.com or me@example.com"
        )])),
        "see https\\://example.com or www\\.example.com or me\\@example.com\n"
    );
    assert_eq!(
        plain(json!([para("see https://example.com")])),
        "see https://example.com\n"
    );
}

#[test]
fn entity_like_text_is_escaped() {
    assert_eq!(
        md(json!([para("AT&T &amp; R&D & co")])),
        "AT\\&T \\&amp; R\\&D & co\n"
    );
}

#[test]
fn bang_before_a_link_is_escaped() {
    assert_eq!(
        md(json!([p(&[t("Wow!"), link("x", "https://e.com")])])),
        "Wow\\![x](https://e.com)\n"
    );
}

#[test]
fn newlines_in_text_become_hard_breaks() {
    assert_eq!(md(json!([para("a\nb")])), "a\\\nb\n");
    assert_eq!(
        md(json!([p(&[
            t("a"),
            json!({"type": "hardBreak"}),
            t("- b")
        ])])),
        "a\\\n\\- b\n"
    );
}

#[test]
fn spaces_around_hard_breaks_are_kept() {
    assert_eq!(
        md(json!([p(&[
            t("a  "),
            json!({"type": "hardBreak"}),
            t("  b")
        ])])),
        "a  \\\n&#32;&#32;b\n"
    );
}

#[test]
fn trailing_hard_breaks_are_dropped() {
    assert_eq!(
        md(json!([p(&[t("a"), json!({"type": "hardBreak"})])])),
        "a\n"
    );
}

#[test]
fn leading_hard_break_is_kept() {
    assert_eq!(
        md(json!([p(&[json!({"type": "hardBreak"}), t("a")])])),
        "\\\na\n"
    );
}

// ---------------------------------------------------------------------------------------------
// Marks
// ---------------------------------------------------------------------------------------------

#[test]
fn basic_marks() {
    assert_eq!(md(json!([p(&[tm("a", &["em"])])])), "*a*\n");
    assert_eq!(md(json!([p(&[tm("a", &["strong"])])])), "**a**\n");
    assert_eq!(md(json!([p(&[tm("a", &["strike"])])])), "~~a~~\n");
    assert_eq!(md(json!([p(&[tm("a", &["em", "strong"])])])), "***a***\n");
    assert_eq!(md(json!([p(&[tm("a", &["underline"])])])), "<u>a</u>\n");
    assert_eq!(md(json!([p(&[tm("a", &["code"])])])), "`a`\n");
}

#[test]
fn subsup_and_colours() {
    let content = json!([p(&[
        t("H"),
        json!({"type": "text", "text": "2", "marks": [{"type": "subsup", "attrs": {"type": "sub"}}]}),
        t("O x"),
        json!({"type": "text", "text": "2", "marks": [{"type": "subsup", "attrs": {"type": "sup"}}]}),
        t(" "),
        json!({"type": "text", "text": "red", "marks": [{"type": "textColor", "attrs": {"color": "#ff5630"}}]}),
        t(" "),
        json!({"type": "text", "text": "hi", "marks": [{"type": "backgroundColor", "attrs": {"color": "#FFFF00"}}]}),
    ])]);
    assert_eq!(
        md(content.clone()),
        "H<sub>2</sub>O x<sup>2</sup> <span style=\"color: #ff5630\">red</span> <span style=\"background-color: #FFFF00\">hi</span>\n"
    );
    assert_eq!(plain(content), "H<sub>2</sub>O x<sup>2</sup> red hi\n");
}

#[test]
fn intraword_emphasis_uses_delimiters() {
    assert_eq!(
        md(json!([p(&[
            t("un"),
            tm("frigging", &["em"]),
            t("believable")
        ])])),
        "un*frigging*believable\n"
    );
}

#[test]
fn whitespace_at_mark_edges_falls_back_to_html() {
    assert_eq!(
        md(json!([p(&[tm("bold ", &["strong"]), t("x")])])),
        "<strong>bold </strong>x\n"
    );
    assert_eq!(
        md(json!([p(&[t("a"), tm(" em", &["em"])])])),
        "a<em> em</em>\n"
    );
    assert_eq!(
        md(json!([p(&[t("a"), tm(" gone ", &["strike"]), t("b")])])),
        "a<del> gone </del>b\n"
    );
}

#[test]
fn punctuation_at_mark_edges_inside_words_falls_back_to_html() {
    assert_eq!(
        md(json!([p(&[t("a"), tm(".b", &["strong"])])])),
        "a<strong>.b</strong>\n"
    );
    assert_eq!(
        md(json!([p(&[tm("(x)", &["em"]), t("y")])])),
        "<em>(x)</em>y\n"
    );
}

#[test]
fn only_the_failing_span_switches_to_html() {
    assert_eq!(
        md(json!([p(&[
            tm("ok", &["strong"]),
            t(" and "),
            tm("bad ", &["strong"]),
            t("x")
        ])])),
        "**ok** and <strong>bad </strong>x\n"
    );
}

#[test]
fn plain_moves_whitespace_out_of_delimiters() {
    assert_eq!(
        plain(json!([p(&[tm("bold ", &["strong"]), t("x")])])),
        "**bold** x\n"
    );
}

#[test]
fn overlapping_marks_nest_without_crossing() {
    // `**a*b****c*` would be ambiguous; the verifier falls back to tags, which always parse.
    assert_eq!(
        md(json!([p(&[
            tm("a", &["strong"]),
            tm("b", &["em", "strong"]),
            tm("c", &["em"])
        ])])),
        "<strong>a<em>b</em></strong><em>c</em>\n"
    );
    assert_eq!(
        md(json!([p(&[
            tm("a", &["strong"]),
            tm("b", &["em", "strong"]),
            t(" c")
        ])])),
        "**a*b*** c\n"
    );
}

#[test]
fn marks_across_a_link() {
    assert_eq!(
        md(json!([p(&[
            tm("x ", &["strong"]),
            json!({"type": "text", "text": "y", "marks": [{"type": "link", "attrs": {"href": "https://e.com"}}, {"type": "strong"}]}),
        ])])),
        "**x [y](https://e.com)**\n"
    );
}

#[test]
fn code_spans() {
    assert_eq!(md(json!([p(&[tm("a`b", &["code"])])])), "``a`b``\n");
    assert_eq!(md(json!([p(&[tm("`x`", &["code"])])])), "`` `x` ``\n");
    assert_eq!(
        md(json!([p(&[tm(" padded ", &["code"])])])),
        "`  padded  `\n"
    );
    assert_eq!(md(json!([p(&[tm("*not em*", &["code"])])])), "`*not em*`\n");
    assert_eq!(md(json!([p(&[tm("``", &["code"])])])), "``` `` ```\n");
}

#[test]
fn code_with_link() {
    assert_eq!(
        md(json!([p(&[
            json!({"type": "text", "text": "cmd", "marks": [
                {"type": "link", "attrs": {"href": "https://e.com/x"}}, {"type": "code"}
            ]})
        ])])),
        "[`cmd`](https://e.com/x)\n"
    );
}

#[test]
fn links_with_titles_and_awkward_urls() {
    assert_eq!(
        md(json!([p(&[
            json!({"type": "text", "text": "t", "marks": [
                {"type": "link", "attrs": {"href": "https://e.com/a(b)c", "title": "say \"hi\""}}
            ]})
        ])])),
        "[t](https://e.com/a\\(b\\)c \"say \\\"hi\\\"\")\n"
    );
    assert_eq!(
        md(json!([p(&[link("sp", "https://e.com/a b")])])),
        "[sp](<https://e.com/a b>)\n"
    );
    assert_eq!(
        md(json!([p(&[link("https://e.com", "https://e.com")])])),
        "[https://e.com](https://e.com)\n"
    );
    assert_eq!(
        plain(json!([p(&[link("https://e.com", "https://e.com")])])),
        "<https://e.com>\n"
    );
    // Not an autolink when other marks or a different text are involved.
    assert_eq!(
        plain(json!([p(&[link("site", "https://e.com")])])),
        "[site](https://e.com)\n"
    );
}

#[test]
fn plain_cards_and_headings() {
    assert_eq!(
        plain(json!([p(&[
            json!({"type": "inlineCard", "attrs": {"url": "https://e.com/a*b"}})
        ])])),
        "<https://e.com/a*b>\n"
    );
    assert_eq!(
        plain(json!([{"type": "heading", "attrs": {"level": 9}, "content": [t("deep")]}])),
        "###### deep\n"
    );
    assert!(
        md(json!([{"type": "heading", "attrs": {"level": 9}, "content": [t("deep")]}]))
            .starts_with("```adf\n")
    );
}

#[test]
fn unsafe_links_are_dropped_with_a_warning() {
    let content = json!([p(&[link("x", "javascript:alert(1)")])]);
    assert_eq!(md(content.clone()), "x\n");
    assert_eq!(
        warnings(content),
        vec![(Code::AttrDropped, "/content/0/content/0/marks/0".to_owned())]
    );
}

#[test]
fn unknown_marks_are_dropped_with_a_warning() {
    let content = json!([p(&[json!({"type": "text", "text": "x", "marks": [
        {"type": "annotation", "attrs": {"id": "a", "annotationType": "inlineComment"}}
    ]})])]);
    assert_eq!(md(content.clone()), "x\n");
    assert_eq!(
        warnings(content),
        vec![(Code::AttrDropped, "/content/0/content/0/marks/0".to_owned())]
    );
}

// ---------------------------------------------------------------------------------------------
// Inline nodes
// ---------------------------------------------------------------------------------------------

#[test]
fn inline_extension_nodes() {
    let content = json!([p(&[
        json!({"type": "mention", "attrs": {"id": "5b10ac8d82e05b22cc7d4ef5", "text": "@Jane Doe", "accessLevel": ""}}),
        t(" "),
        json!({"type": "status", "attrs": {"text": "DONE", "color": "green", "localId": "s1"}}),
        t(" "),
        json!({"type": "date", "attrs": {"timestamp": "1792368000000"}}),
        t(" "),
        json!({"type": "emoji", "attrs": {"shortName": ":smile:", "text": "😄"}}),
        t(" "),
        json!({"type": "inlineCard", "attrs": {"url": "https://x.atlassian.net/browse/A-1"}}),
    ])]);
    assert_eq!(
        md(content.clone()),
        "[@Jane Doe](adf:mention/5b10ac8d82e05b22cc7d4ef5) [DONE](adf:status/green) [2026-10-19](adf:date) [😄](adf:emoji/:smile:) [https://x.atlassian.net/browse/A-1](adf:card)\n"
    );
    assert_eq!(warnings(content.clone()), NO_WARNINGS);
    assert_eq!(
        plain(content),
        "@Jane Doe [DONE] 2026-10-19 😄 https://x.atlassian.net/browse/A-1\n"
    );
}

#[test]
fn emoji_id_is_dropped_with_a_warning() {
    let content = json!([p(&[
        json!({"type": "emoji", "attrs": {"shortName": ":x:", "id": "274c", "text": "❌"}})
    ])]);
    assert_eq!(md(content.clone()), "[❌](adf:emoji/:x:)\n");
    assert_eq!(
        warnings(content),
        vec![(Code::AttrDropped, "/content/0/content/0".to_owned())]
    );
}

#[test]
fn inline_nodes_without_markdown_form_pass_the_block_through() {
    let content = json!([
        para("before"),
        p(&[
            t("a "),
            json!({"type": "mediaInline", "attrs": {"id": "m", "collection": "c"}})
        ]),
    ]);
    let out = md(content.clone());
    assert!(
        out.starts_with("before\n\n```adf\n{\n  \"type\": \"paragraph\""),
        "{out}"
    );
    assert_eq!(
        warnings(content),
        vec![(Code::AdfPassthrough, "/content/1".to_owned())]
    );
}

#[test]
fn non_midnight_dates_pass_through() {
    let content = json!([p(&[
        json!({"type": "date", "attrs": {"timestamp": "1582152559"}})
    ])]);
    assert!(md(content.clone()).starts_with("```adf\n"));
    assert_eq!(plain(content), "1970-01-19\n");
}

// ---------------------------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------------------------

#[test]
fn headings() {
    assert_eq!(
        md(json!([{"type": "heading", "attrs": {"level": 2}, "content": [t("Title")]}])),
        "## Title\n"
    );
    assert_eq!(
        md(
            json!([{"type": "heading", "attrs": {"level": 1}, "content": [t("a"), {"type": "hardBreak"}, t("b #")]}])
        ),
        "# a<br>b \\#\n"
    );
    assert_eq!(
        md(json!([{"type": "heading", "attrs": {"level": 3}}])),
        "###\n"
    );
    assert_eq!(
        md(json!([{"type": "heading", "attrs": {"level": 2}, "content": [t(" x ")]}])),
        "## &#32;x&#32;\n"
    );
}

#[test]
fn heading_with_emoji_and_strong() {
    assert_eq!(
        md(
            json!([{"type": "heading", "attrs": {"level": 2}, "content": [t("📡 "), tm("Signal path", &["strong"])]}])
        ),
        "## 📡 **Signal path**\n"
    );
}

#[test]
fn bullet_and_ordered_lists() {
    assert_eq!(
        md(json!([bullets(&[li(&[para("a")]), li(&[para("b")])])])),
        "- a\n- b\n"
    );
    assert_eq!(
        md(json!([ordered(3, &[li(&[para("a")]), li(&[para("b")])])])),
        "3. a\n4. b\n"
    );
}

#[test]
fn nested_lists_are_tight_where_possible() {
    assert_eq!(
        md(json!([bullets(&[
            li(&[para("a"), bullets(&[li(&[para("b")])])]),
            li(&[para("c")]),
        ])])),
        "- a\n  - b\n- c\n"
    );
    assert_eq!(
        md(json!([ordered(
            1,
            &[li(&[para("a"), ordered(2, &[li(&[para("b")])])])]
        )])),
        "1. a\n\n   2. b\n"
    );
}

#[test]
fn loose_items_use_blank_lines() {
    assert_eq!(
        md(json!([bullets(&[
            li(&[para("a"), para("b")]),
            li(&[para("c")])
        ])])),
        "- a\n\n  b\n\n- c\n"
    );
}

#[test]
fn list_item_with_empty_first_paragraph() {
    assert_eq!(
        md(json!([bullets(&[li(&[
            json!({"type": "paragraph"}),
            bullets(&[li(&[para("x")])])
        ])])])),
        "-\n  - x\n"
    );
    assert_eq!(
        md(json!([bullets(&[li(&[json!({"type": "paragraph"})])])])),
        "-\n"
    );
}

#[test]
fn adjacent_lists_alternate_markers() {
    assert_eq!(
        md(json!([
            bullets(&[li(&[para("a")])]),
            bullets(&[li(&[para("b")])])
        ])),
        "- a\n\n* b\n"
    );
    assert_eq!(
        md(json!([
            ordered(1, &[li(&[para("a")])]),
            ordered(1, &[li(&[para("b")])])
        ])),
        "1. a\n\n1) b\n"
    );
}

#[test]
fn list_item_paragraph_with_line_start_text() {
    assert_eq!(
        md(json!([bullets(&[li(&[p(&[
            t("- x"),
            json!({"type": "hardBreak"}),
            t("1. y")
        ])])])])),
        "- \\- x\\\n  1\\. y\n"
    );
}

#[test]
fn list_without_leading_paragraph_passes_through() {
    let content = json!([bullets(&[li(&[bullets(&[li(&[para("x")])])])])]);
    assert!(md(content.clone()).starts_with("```adf\n{\n  \"type\": \"bulletList\""));
    assert_eq!(
        warnings(content),
        vec![(Code::AdfPassthrough, "/content/0".to_owned())]
    );
}

#[test]
fn task_lists() {
    let content = json!([{"type": "taskList", "attrs": {"localId": "l"}, "content": [
        {"type": "taskItem", "attrs": {"localId": "a", "state": "TODO"}, "content": [t("open")]},
        {"type": "taskItem", "attrs": {"localId": "b", "state": "DONE"}, "content": [t("done")]},
        {"type": "taskList", "attrs": {"localId": "m"}, "content": [
            {"type": "taskItem", "attrs": {"localId": "c", "state": "TODO"}, "content": [t("nested")]}
        ]},
        {"type": "taskItem", "attrs": {"localId": "d", "state": "TODO"}}
    ]}]);
    assert_eq!(
        md(content.clone()),
        "- [ ] open\n- [x] done\n  - [ ] nested\n- [ ]\n"
    );
    assert_eq!(warnings(content), NO_WARNINGS);
}

#[test]
fn code_blocks() {
    assert_eq!(
        md(
            json!([{"type": "codeBlock", "attrs": {"language": "rust"}, "content": [t("fn main() {}")]}])
        ),
        "```rust\nfn main() {}\n```\n"
    );
    assert_eq!(md(json!([{"type": "codeBlock"}])), "```\n```\n");
    assert_eq!(
        md(json!([{"type": "codeBlock", "content": [t("a\n```\nb\n")]}])),
        "````\na\n```\nb\n\n````\n"
    );
    assert_eq!(
        md(json!([{"type": "codeBlock", "attrs": {"language": "we`ird"}, "content": [t("x")]}])),
        "~~~we`ird\nx\n~~~\n"
    );
}

#[test]
fn code_block_attributes_are_dropped() {
    let content = json!([{"type": "codeBlock", "attrs": {"language": "sh", "wrap": true, "uniqueId": "u"}, "content": [t("ls")]}]);
    assert_eq!(md(content.clone()), "```sh\nls\n```\n");
    assert_eq!(
        warnings(content),
        vec![(Code::AttrDropped, "/content/0".to_owned())]
    );
}

#[test]
fn blockquotes() {
    assert_eq!(
        md(json!([{"type": "blockquote", "content": [para("a"), bullets(&[li(&[para("b")])])]}])),
        "> a\n>\n> - b\n"
    );
}

#[test]
fn rules() {
    assert_eq!(
        md(json!([para("a"), {"type": "rule"}, para("b")])),
        "a\n\n---\n\nb\n"
    );
}

#[test]
fn panels_become_alerts() {
    for (panel, alert) in [
        ("info", "NOTE"),
        ("success", "TIP"),
        ("note", "IMPORTANT"),
        ("warning", "WARNING"),
        ("error", "CAUTION"),
    ] {
        assert_eq!(
            md(json!([{"type": "panel", "attrs": {"panelType": panel}, "content": [para("x")]}])),
            format!("> [!{alert}]\n> x\n")
        );
    }
    assert_eq!(
        md(
            json!([{"type": "panel", "attrs": {"panelType": "info"}, "content": [{"type": "paragraph"}]}])
        ),
        "> [!NOTE]\n"
    );
}

#[test]
fn tip_and_custom_panels() {
    let content = json!([{"type": "panel", "attrs": {"panelType": "custom", "panelColor": "#abcdef"}, "content": [para("x")]}]);
    assert!(md(content.clone()).starts_with("```adf\n"));
    assert_eq!(plain(content), "> [!NOTE]\n> x\n");
    let tip = json!([{"type": "panel", "attrs": {"panelType": "tip"}, "content": [para("x")]}]);
    assert_eq!(plain(tip), "> [!TIP]\n> x\n");
}

#[test]
fn panel_icons_are_dropped() {
    let content = json!([{"type": "panel", "attrs": {"panelType": "success", "panelIcon": ":check_mark:"}, "content": [para("x")]}]);
    assert_eq!(md(content.clone()), "> [!TIP]\n> x\n");
    assert_eq!(
        warnings(content),
        vec![(Code::AttrDropped, "/content/0".to_owned())]
    );
}

#[test]
fn expands_become_details() {
    assert_eq!(
        md(
            json!([{"type": "expand", "attrs": {"title": "Logs & <stuff>"}, "content": [para("body")]}])
        ),
        "<details>\n<summary>Logs &amp; &lt;stuff&gt;</summary>\n\nbody\n\n</details>\n"
    );
    assert_eq!(
        md(json!([{"type": "expand", "attrs": {}, "content": [{"type": "paragraph"}]}])),
        "<details>\n\n</details>\n"
    );
}

#[test]
fn nested_expand_inside_expand() {
    assert_eq!(
        md(
            json!([{"type": "expand", "attrs": {"title": "Outer"}, "content": [
                {"type": "nestedExpand", "attrs": {"title": "Inner"}, "content": [para("x")]}
            ]}])
        ),
        "<details>\n<summary>Outer</summary>\n\n<details>\n<summary>Inner</summary>\n\nx\n\n</details>\n\n</details>\n"
    );
}

#[test]
fn external_images() {
    let content = json!([{"type": "mediaSingle", "attrs": {"layout": "center"}, "content": [
        {"type": "media", "attrs": {"type": "external", "url": "https://e.com/a.png", "alt": "A [chart]"}}
    ]}]);
    assert_eq!(
        md(content.clone()),
        "![A \\[chart\\]](https://e.com/a.png)\n"
    );
    assert_eq!(warnings(content), NO_WARNINGS);
}

#[test]
fn uploaded_media_passes_through() {
    let content = json!([{"type": "mediaSingle", "attrs": {"layout": "center"}, "content": [
        {"type": "media", "attrs": {"type": "file", "id": "abc", "collection": "", "alt": "shot.png"}}
    ]}]);
    assert!(md(content.clone()).starts_with("```adf\n"));
    assert_eq!(plain(content), "shot.png\n");
}

// ---------------------------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------------------------

#[test]
fn box_cells() {
    let content = json!([table(&[row(&[cell(
        "tableCell",
        &[para("a"), bullets(&[li(&[para("b")])])]
    )])])]);
    assert_eq!(md(content.clone()), "::: cell\na\n\n- b\n:::\n");
    assert_eq!(warnings(content.clone()), NO_WARNINGS);
    assert_eq!(plain(content), "a\n\n- b\n");
}

#[test]
fn box_cell_fence_outgrows_colon_lines() {
    let content = json!([table(&[row(&[cell(
        "tableCell",
        &[json!({"type": "codeBlock", "content": [t(":::: x")]})]
    )])])]);
    assert_eq!(md(content), "::::: cell\n```\n:::: x\n```\n:::::\n");
}

#[test]
fn empty_box_cell() {
    assert_eq!(
        md(json!([table(&[row(&[cell(
            "tableCell",
            &[json!({"type": "paragraph"})]
        )])])])),
        "::: cell\n:::\n"
    );
}

#[test]
fn gfm_tables() {
    let content = json!([table(&[
        row(&[
            cell("tableHeader", &[p(&[tm("Name", &["strong"])])]),
            cell("tableHeader", &[para("Value")])
        ]),
        row(&[
            cell("tableCell", &[p(&[tm("a|b", &["code"])])]),
            cell(
                "tableCell",
                &[p(&[t("x"), json!({"type": "hardBreak"}), t("y|z")])]
            )
        ]),
        row(&[
            cell("tableCell", &[json!({"type": "paragraph"})]),
            cell("tableCell", &[para(" pad ")])
        ]),
    ])]);
    assert_eq!(
        md(content.clone()),
        "| **Name** | Value |\n| --- | --- |\n| `a\\|b` | x<br>y\\|z |\n|  | &#32;pad&#32; |\n"
    );
    assert_eq!(warnings(content), NO_WARNINGS);
}

#[test]
fn gfm_table_alignment() {
    let aligned = |text: &str, align: &str| json!({"type": "paragraph", "marks": [{"type": "alignment", "attrs": {"align": align}}], "content": [t(text)]});
    let content = json!([table(&[
        row(&[
            cell("tableHeader", &[para("l")]),
            cell("tableHeader", &[aligned("c", "center")]),
            cell("tableHeader", &[aligned("r", "end")])
        ]),
        row(&[
            cell("tableCell", &[para("1")]),
            cell("tableCell", &[aligned("2", "center")]),
            cell("tableCell", &[aligned("3", "end")])
        ]),
    ])]);
    assert_eq!(
        md(content),
        "| l | c | r |\n| --- | :---: | ---: |\n| 1 | 2 | 3 |\n"
    );
}

#[test]
fn tables_without_header_row_pass_through() {
    let content = json!([table(&[row(&[
        cell("tableCell", &[para("a")]),
        cell("tableCell", &[para("b")])
    ]),])]);
    assert!(md(content.clone()).starts_with("```adf\n"));
    assert_eq!(plain(content), "| a | b |\n| --- | --- |\n");
}

#[test]
fn table_attributes_are_dropped() {
    let content = json!([{"type": "table", "attrs": {"isNumberColumnEnabled": true, "layout": "wide", "localId": "t"}, "content": [
        row(&[json!({"type": "tableHeader", "attrs": {"colwidth": [100], "background": "#ffffff"}, "content": [para("h")]})]),
        row(&[cell("tableCell", &[para("x")])]),
    ]}]);
    assert_eq!(md(content.clone()), "| h |\n| --- |\n| x |\n");
    assert_eq!(
        warnings(content),
        vec![
            (Code::AttrDropped, "/content/0".to_owned()),
            (
                Code::AttrDropped,
                "/content/0/content/0/content/0".to_owned()
            ),
        ]
    );
}

#[test]
fn plain_tables_join_blocks_with_br() {
    let content = json!([table(&[
        row(&[
            cell("tableHeader", &[para("h")]),
            cell("tableHeader", &[para("i")])
        ]),
        row(&[
            cell(
                "tableCell",
                &[para("a"), bullets(&[li(&[para("b")]), li(&[para("c")])])]
            ),
            cell("tableCell", &[para("d")])
        ]),
    ])]);
    assert_eq!(
        plain(content),
        "| h | i |\n| --- | --- |\n| a<br>- b<br>- c | d |\n"
    );
}

// ---------------------------------------------------------------------------------------------
// Passthrough
// ---------------------------------------------------------------------------------------------

#[test]
fn unknown_blocks_pass_through_and_merge() {
    let content = json!([
        {"type": "decisionList", "attrs": {"localId": "d"}, "content": [
            {"type": "decisionItem", "attrs": {"localId": "i", "state": "DECIDED"}, "content": [t("ship")]}
        ]},
        {"type": "extension", "attrs": {"extensionType": "x", "extensionKey": "y", "localId": "z"}},
        para("after"),
    ]);
    let out = md(content.clone());
    assert!(
        out.starts_with("```adf\n[\n  {\n    \"type\": \"decisionList\""),
        "{out}"
    );
    assert!(!out.contains("\"z\""), "optional localId stripped: {out}");
    assert!(
        out.contains("\"localId\": \"d\""),
        "required localId kept: {out}"
    );
    assert!(out.ends_with("]\n```\n\nafter\n"), "{out}");
    assert_eq!(
        warnings(content.clone()),
        vec![
            (Code::AdfPassthrough, "/content/0".to_owned()),
            (Code::AdfPassthrough, "/content/1".to_owned()),
        ]
    );
    assert_eq!(plain(content), "- ship\n\nafter\n");
}

#[test]
fn empty_paragraphs_pass_through_in_roundtrip_and_vanish_in_plain() {
    let content = json!([para("a"), {"type": "paragraph"}, para("b")]);
    assert_eq!(
        md(content.clone()),
        "a\n\n```adf\n{\n  \"type\": \"paragraph\"\n}\n```\n\nb\n"
    );
    assert_eq!(plain(content), "a\n\nb\n");
}

#[test]
fn passthrough_inside_containers() {
    let content = json!([{"type": "panel", "attrs": {"panelType": "info"}, "content": [
        para("a"),
        {"type": "decisionList", "attrs": {"localId": "d"}, "content": []}
    ]}]);
    let out = md(content);
    assert!(
        out.starts_with("> [!NOTE]\n> a\n>\n> ```adf\n> {\n>   \"type\": \"decisionList\""),
        "{out}"
    );
}

#[test]
fn misplaced_blocks_pass_through() {
    // A heading cannot be written inside a quote (Markdown → ADF would demote it).
    let content = json!([{"type": "blockquote", "content": [{"type": "heading", "attrs": {"level": 1}, "content": [t("h")]}]}]);
    let out = md(content.clone());
    assert!(out.starts_with("> ```adf\n"), "{out}");
    assert_eq!(
        warnings(content),
        vec![(Code::AdfPassthrough, "/content/0/content/0".to_owned())]
    );
}

#[test]
fn stray_inline_nodes_become_a_paragraph() {
    let content = json!([t("loose "), tm("text", &["strong"]), para("next")]);
    assert_eq!(md(content.clone()), "loose **text**\n\nnext\n");
    assert_eq!(plain(content), "loose **text**\n\nnext\n");
}

#[test]
fn local_ids_are_dropped_silently() {
    let content = json!([{"type": "paragraph", "attrs": {"localId": "p1"}, "content": [t("x")]}]);
    assert_eq!(md(content.clone()), "x\n");
    assert_eq!(warnings(content), NO_WARNINGS);
}

#[test]
fn plain_flavour_reports_no_lossy_warnings() {
    let content = json!([
        {"type": "decisionList", "attrs": {"localId": "d"}, "content": []},
        {"type": "paragraph"},
        p(&[json!({"type": "emoji", "attrs": {"shortName": ":x:", "id": "1"}})]),
    ]);
    let result = to_markdown(&doc(content), &opts(MdFlavor::Plain)).unwrap();
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
}

// ---------------------------------------------------------------------------------------------
// Robustness: arbitrary ADF-like trees never fail, and output is well-formed and deterministic.
// ---------------------------------------------------------------------------------------------

mod arbitrary {
    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::super::to_markdown;
    use crate::options::{AdfToMdOptions, MdFlavor};

    fn text() -> impl Strategy<Value = String> {
        let pieces = prop::sample::select(vec![
            "a",
            " ",
            "\n",
            "*",
            "_",
            "`",
            "~",
            "[",
            "]",
            "<",
            ">",
            "#",
            "-",
            "1.",
            "|",
            "!",
            "&",
            "\\",
            ":",
            "https://x.y",
            "é",
            "\t",
        ]);
        prop::collection::vec(pieces, 0..6).prop_map(|v| v.concat())
    }

    fn marks() -> impl Strategy<Value = Value> {
        let mark = prop::sample::select(vec![
            json!({"type": "em"}),
            json!({"type": "strong"}),
            json!({"type": "strike"}),
            json!({"type": "underline"}),
            json!({"type": "code"}),
            json!({"type": "link", "attrs": {"href": "https://e.com/x"}}),
            json!({"type": "link", "attrs": {"href": "javascript:x"}}),
            json!({"type": "subsup", "attrs": {"type": "sup"}}),
            json!({"type": "textColor", "attrs": {"color": "#112233"}}),
            json!({"type": "annotation", "attrs": {"id": "a", "annotationType": "inlineComment"}}),
            json!("junk"),
        ]);
        prop::collection::vec(mark, 0..3).prop_map(Value::Array)
    }

    fn leaf() -> impl Strategy<Value = Value> {
        prop_oneof![
            (text(), marks()).prop_map(|(t, m)| json!({"type": "text", "text": t, "marks": m})),
            Just(json!({"type": "hardBreak"})),
            Just(json!({"type": "mention", "attrs": {"id": "abc", "text": "@A"}})),
            Just(json!({"type": "status", "attrs": {"text": "OK", "color": "green"}})),
            Just(json!({"type": "date", "attrs": {"timestamp": "86400000"}})),
            Just(json!({"type": "emoji", "attrs": {"shortName": ":x:", "text": "❌"}})),
            Just(json!({"type": "inlineCard", "attrs": {"url": "https://e.com"}})),
            Just(json!({"type": "mediaInline", "attrs": {"id": "m", "collection": ""}})),
            Just(json!({"type": "rule"})),
            Just(json!({"type": "paragraph"})),
            Just(
                json!({"type": "codeBlock", "attrs": {"language": "sh"}, "content": [{"type": "text", "text": "ls\n```"}]})
            ),
            Just(
                json!({"type": "media", "attrs": {"type": "external", "url": "https://e.com/a.png"}})
            ),
            Just(json!(42)),
            Just(json!({"content": "nope"})),
        ]
    }

    fn node() -> impl Strategy<Value = Value> {
        leaf().prop_recursive(5, 48, 4, |inner| {
            let ty = prop::sample::select(vec![
                "paragraph",
                "heading",
                "bulletList",
                "orderedList",
                "listItem",
                "taskList",
                "taskItem",
                "blockquote",
                "panel",
                "table",
                "tableRow",
                "tableCell",
                "tableHeader",
                "expand",
                "nestedExpand",
                "mediaSingle",
                "decisionList",
                "layoutSection",
                "weird",
            ]);
            let attrs = prop::sample::select(vec![
                json!({}),
                json!({"level": 2, "order": 3, "state": "DONE", "panelType": "warning", "title": "T"}),
                json!({"localId": "x", "colspan": 2, "colwidth": [10], "layout": "wide"}),
                json!({"panelType": "custom", "level": 0, "order": -1, "state": "?"}),
            ]);
            (ty, attrs, prop::collection::vec(inner, 0..4)).prop_map(|(ty, attrs, content)| {
                json!({"type": ty, "attrs": attrs, "content": content})
            })
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(400))]

        #[test]
        fn never_fails_and_is_well_formed(content in prop::collection::vec(node(), 0..5)) {
            let document = json!({"type": "doc", "version": 1, "content": content});
            for flavor in [MdFlavor::Roundtrip, MdFlavor::Plain] {
                let opts = AdfToMdOptions { flavor, ..AdfToMdOptions::default() };
                let first = to_markdown(&document, &opts);
                prop_assert!(first.is_ok(), "{:?}", first);
                let first = first.unwrap_or_else(|_| unreachable!());
                prop_assert!(first.output.ends_with('\n'));
                prop_assert!(
                    first.output == "\n" || !first.output.ends_with("\n\n"),
                    "{:?}",
                    first.output
                );
                for warning in &first.warnings {
                    let pointer = warning
                        .location
                        .as_ref()
                        .and_then(|l| l.pointer.clone())
                        .unwrap_or_default();
                    prop_assert!(document.pointer(&pointer).is_some(), "dangling pointer {}", pointer);
                }
                if flavor == MdFlavor::Plain {
                    prop_assert!(first.warnings.is_empty(), "{:?}", first.warnings);
                    prop_assert!(!first.output.contains("```adf"), "{}", first.output);
                }
                let again = to_markdown(&document, &opts).unwrap_or_else(|_| unreachable!());
                prop_assert_eq!(&again, &first);
            }
        }
    }
}
