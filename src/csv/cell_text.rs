//! The plain text of one ADF table cell, for ADF → CSV.
//!
//! Each block of the cell (paragraph, heading, list item, code block, quote, panel, nested
//! expand, …) gives one piece of text; the pieces are joined with one space. Inside a block a
//! hard break is a line feed. Marks are ignored.

use crate::adf::reader::{Node, marker_alt};
use crate::adf::to_md::date::{civil_date, timestamp_millis};
use crate::md::strip_controls;

/// The text of `cell` with control characters (other than tab and line feed) removed, and
/// whether any were removed.
pub(super) fn cell_text(cell: &Node<'_>) -> (String, bool) {
    let mut parts = Vec::new();
    container_parts(cell, &mut parts);
    let text = parts.join(" ");
    let cleaned = strip_controls(&text);
    let removed = cleaned.len() != text.len();
    (cleaned, removed)
}

/// Whether `node` is inline content (or text that is not in a block of its own).
fn is_run_content(node: &Node<'_>) -> bool {
    node.is_inline() || node.ty == "media" || node.text.is_some()
}

/// The pieces of the children of `node`: runs of inline children make one piece, every block
/// child makes its own.
fn container_parts(node: &Node<'_>, parts: &mut Vec<String>) {
    let mut run = String::new();
    for child in &node.content {
        if is_run_content(child) {
            inline_text(child, &mut run);
        } else {
            push_part(std::mem::take(&mut run), parts);
            block_parts(child, parts);
        }
    }
    push_part(run, parts);
}

fn push_part(part: String, parts: &mut Vec<String>) {
    if !part.is_empty() {
        parts.push(part);
    }
}

fn block_parts(node: &Node<'_>, parts: &mut Vec<String>) {
    match node.ty {
        "mediaSingle" | "mediaGroup" => push_part(media_text(node), parts),
        "blockCard" | "embedCard" => {
            push_part(node.attr_str("url").unwrap_or_default().to_owned(), parts);
        }
        "paragraph" | "heading" | "codeBlock" | "taskItem" | "decisionItem" | "caption" => {
            // Everything inside a text block is part of its one piece, known or not.
            let mut piece = String::new();
            for child in &node.content {
                inline_text(child, &mut piece);
            }
            push_part(piece, parts);
        }
        _ => container_parts(node, parts),
    }
}

/// The text of an inline node, recursively for nodes this does not know.
fn inline_text(node: &Node<'_>, out: &mut String) {
    match node.ty {
        "text" => out.push_str(node.text.unwrap_or_default()),
        "hardBreak" => out.push('\n'),
        "mention" => mention_text(node, out),
        "status" | "placeholder" | "inlineExtension" => {
            out.push_str(node.attr_str("text").unwrap_or_default());
        }
        "emoji" => out.push_str(
            non_empty(node.attr_str("text"))
                .or_else(|| node.attr_str("shortName"))
                .unwrap_or_default(),
        ),
        "date" => out.push_str(&date_text(node)),
        "inlineCard" | "blockCard" | "embedCard" => {
            out.push_str(node.attr_str("url").unwrap_or_default());
        }
        "media" | "mediaInline" => push_marker(out, "attachment", node),
        "mediaSingle" | "mediaGroup" => out.push_str(&media_text(node)),
        _ => {
            out.push_str(node.text.unwrap_or_default());
            for child in &node.content {
                inline_text(child, out);
            }
        }
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

/// The mention's `text`, else `@` and its `id`.
fn mention_text(node: &Node<'_>, out: &mut String) {
    if let Some(text) = non_empty(node.attr_str("text")) {
        out.push_str(text);
    } else if let Some(id) = non_empty(node.attr_str("id")) {
        out.push('@');
        out.push_str(id);
    }
}

/// `YYYY-MM-DD`; the raw timestamp when it is not a date.
fn date_text(node: &Node<'_>) -> String {
    let timestamp = node.attr("timestamp");
    timestamp
        .and_then(timestamp_millis)
        .and_then(civil_date)
        .map_or_else(
            || {
                timestamp
                    .map(ToString::to_string)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned()
            },
            |(date, _)| date,
        )
}

/// `[kind: alt]`, or `[kind]` without alt text.
fn push_marker(out: &mut String, kind: &str, media: &Node<'_>) {
    out.push('[');
    out.push_str(kind);
    let alt = marker_alt(media.attr_str("alt"));
    if !alt.is_empty() {
        out.push_str(": ");
        out.push_str(&alt);
    }
    out.push(']');
}

/// `mediaSingle`: `[image: alt]` per file; `mediaGroup`: `[attachment: alt]` per file; a caption
/// or anything else adds its text; all separated by spaces.
fn media_text(node: &Node<'_>) -> String {
    let kind = if node.ty == "mediaSingle" {
        "image"
    } else {
        "attachment"
    };
    let mut pieces = Vec::new();
    for child in &node.content {
        if child.ty == "media" {
            let mut piece = String::new();
            push_marker(&mut piece, kind, child);
            pieces.push(piece);
        } else if is_run_content(child) {
            let mut piece = String::new();
            inline_text(child, &mut piece);
            push_part(piece, &mut pieces);
        } else {
            block_parts(child, &mut pieces);
        }
    }
    pieces.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adf::reader::read_doc;
    use crate::limits::Limits;
    use serde_json::{Value, json};

    /// The text of the only cell of a document holding `content` in a table cell.
    fn text_of(content: &Value) -> String {
        let doc = json!({"type": "doc", "version": 1, "content": [
            {"type": "table", "content": [{"type": "tableRow", "content": [
                {"type": "tableCell", "content": content}
            ]}]}
        ]});
        let root = read_doc(&doc, &Limits::default()).unwrap();
        let cell = &root.content[0].content[0].content[0];
        cell_text(cell).0
    }

    fn para(inline: &Value) -> Value {
        json!({"type": "paragraph", "content": inline})
    }

    fn t(text: &str) -> Value {
        json!({"type": "text", "text": text})
    }

    #[test]
    fn paragraphs_are_joined_with_one_space() {
        let content = json!([para(&json!([t("one")])), para(&json!([t("two")]))]);
        assert_eq!(text_of(&content), "one two");
    }

    #[test]
    fn empty_blocks_add_nothing() {
        let content = json!([{"type": "paragraph"}, para(&json!([t("a")])), {"type": "paragraph"}]);
        assert_eq!(text_of(&content), "a");
        assert_eq!(text_of(&json!([{"type": "paragraph"}])), "");
        assert_eq!(text_of(&json!([])), "");
    }

    #[test]
    fn a_hard_break_is_a_line_feed_and_marks_are_ignored() {
        let content = json!([para(&json!([
            {"type": "text", "text": "a", "marks": [{"type": "strong"}]},
            {"type": "hardBreak"},
            {"type": "text", "text": "b", "marks": [{"type": "link", "attrs": {"href": "https://x.example"}}]}
        ]))]);
        assert_eq!(text_of(&content), "a\nb");
    }

    #[test]
    fn headings_code_blocks_and_rules() {
        let content = json!([
            {"type": "heading", "attrs": {"level": 2}, "content": [t("Title")]},
            {"type": "codeBlock", "content": [t("let x = 1;\nlet y = 2;")]},
            {"type": "rule"},
            para(&json!([t("end")]))
        ]);
        assert_eq!(text_of(&content), "Title let x = 1;\nlet y = 2; end");
    }

    #[test]
    fn lists_give_one_piece_per_item_block() {
        let content = json!([
            {"type": "bulletList", "content": [
                {"type": "listItem", "content": [para(&json!([t("a")]))]},
                {"type": "listItem", "content": [para(&json!([t("b")])),
                    {"type": "orderedList", "content": [
                        {"type": "listItem", "content": [para(&json!([t("c")]))]}
                    ]}]}
            ]},
            {"type": "taskList", "content": [
                {"type": "taskItem", "attrs": {"state": "TODO"}, "content": [t("d")]}
            ]}
        ]);
        assert_eq!(text_of(&content), "a b c d");
    }

    #[test]
    fn quotes_panels_and_expands() {
        let content = json!([
            {"type": "blockquote", "content": [para(&json!([t("q")]))]},
            {"type": "panel", "attrs": {"panelType": "info"}, "content": [para(&json!([t("p1")])), para(&json!([t("p2")]))]},
            {"type": "nestedExpand", "attrs": {"title": "T"}, "content": [para(&json!([t("e")]))]}
        ]);
        assert_eq!(text_of(&content), "q p1 p2 e");
    }

    #[test]
    fn mentions() {
        let content = json!([para(&json!([
            {"type": "mention", "attrs": {"id": "u1", "text": "@Ann"}}, t(" and "),
            {"type": "mention", "attrs": {"id": "u2"}}, t(" and "),
            {"type": "mention", "attrs": {"id": "u3", "text": ""}}
        ]))]);
        assert_eq!(text_of(&content), "@Ann and @u2 and @u3");
    }

    #[test]
    fn status_date_emoji_and_cards() {
        let content = json!([para(&json!([
            {"type": "status", "attrs": {"text": "DONE", "color": "green"}}, t(" "),
            {"type": "date", "attrs": {"timestamp": "1700000000000"}}, t(" "),
            {"type": "emoji", "attrs": {"shortName": ":smile:", "text": "\u{1F604}"}}, t(" "),
            {"type": "emoji", "attrs": {"shortName": ":wave:"}}, t(" "),
            {"type": "inlineCard", "attrs": {"url": "https://example.com/x"}}
        ])), {"type": "blockCard", "attrs": {"url": "https://example.com/y"}}]);
        assert_eq!(
            text_of(&content),
            "DONE 2023-11-14 \u{1F604} :wave: https://example.com/x https://example.com/y"
        );
    }

    #[test]
    fn a_date_that_is_not_a_date_keeps_its_timestamp() {
        let content = json!([para(
            &json!([{"type": "date", "attrs": {"timestamp": "soon"}}])
        )]);
        assert_eq!(text_of(&content), "soon");
        let content = json!([para(&json!([{"type": "date"}]))]);
        assert_eq!(text_of(&content), "");
    }

    #[test]
    fn media_gives_the_plain_markers() {
        let file = |alt: &str| json!({"type": "media", "attrs": {"type": "file", "id": "1", "collection": "", "alt": alt}});
        let content = json!([
            {"type": "mediaSingle", "content": [file("shot.png"),
                {"type": "caption", "content": [t("Fig 1")]}]},
            {"type": "mediaGroup", "content": [file("a.zip"), file("")]},
            para(&json!([
                {"type": "mediaInline", "attrs": {"id": "2", "collection": "", "alt": "i.zip"}},
                {"type": "mediaInline", "attrs": {"id": "3", "collection": ""}}
            ]))
        ]);
        assert_eq!(
            text_of(&content),
            "[image: shot.png] Fig 1 [attachment: a.zip] [attachment] [attachment: i.zip][attachment]"
        );
    }

    #[test]
    fn unknown_nodes_give_their_text_content() {
        let content = json!([
            {"type": "bodiedExtension", "attrs": {"extensionType": "x", "extensionKey": "y"}, "content": [para(&json!([t("inside")]))]},
            para(&json!([{"type": "futureInline", "content": [t("f1"), t("f2")]}])),
            {"type": "futureBlock", "text": "loose"}
        ]);
        assert_eq!(text_of(&content), "inside f1f2 loose");
    }

    #[test]
    fn inline_content_directly_in_the_cell_is_one_piece() {
        assert_eq!(text_of(&json!([t("a"), t("b")])), "ab");
    }

    #[test]
    fn control_characters_are_removed_and_reported() {
        let doc = json!({"type": "doc", "content": [{"type": "table", "content": [
            {"type": "tableRow", "content": [{"type": "tableCell", "content": [
                para(&json!([t("a\u{1}b\tc\nd\re")]))
            ]}]}
        ]}]});
        let root = read_doc(&doc, &Limits::default()).unwrap();
        let cell = &root.content[0].content[0].content[0];
        assert_eq!(cell_text(cell), ("ab\tc\nde".to_owned(), true));
    }
}
