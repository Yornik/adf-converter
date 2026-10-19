//! Block nodes and containers.
//!
//! Every block renders to Markdown lines without a trailing newline; siblings are separated by a
//! blank line. Containers prefix their children's lines (`> ` for quotes and panels, spaces for
//! list items), so laziness rules never come into play.

use crate::adf::model::Href;
use crate::adf::reader::Node;

use super::escape::{
    TextContext, TextEscape, escape_text, html_text, link_destination, longest_run,
};
use super::inline::{InlineKind, is_blank, render_inline};
use super::passthrough::raw_block;
use super::table;
use super::{Ctx, Fail};

/// The container a block is written in. Decides which children Markdown → ADF accepts there
/// (`docs/syntax.md#container-rules`); anything else is passed through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Container {
    Doc,
    ListItem,
    Blockquote,
    Panel,
    TableCell,
    Expand,
    NestedExpand,
}

impl Container {
    fn name(self) -> &'static str {
        match self {
            Self::Doc => "the document",
            Self::ListItem => "a list item",
            Self::Blockquote => "a quote",
            Self::Panel => "a panel",
            Self::TableCell => "a table cell",
            Self::Expand => "an expand",
            Self::NestedExpand => "a nested expand",
        }
    }

    /// Whether Markdown → ADF produces a `ty` node directly in this container.
    fn allows(self, ty: &str) -> bool {
        let common = matches!(
            ty,
            "paragraph" | "bulletList" | "orderedList" | "codeBlock" | "mediaSingle"
        );
        common
            || match self {
                Self::Doc => matches!(
                    ty,
                    "heading" | "taskList" | "blockquote" | "rule" | "panel" | "table" | "expand"
                ),
                Self::Expand => matches!(
                    ty,
                    "heading"
                        | "taskList"
                        | "blockquote"
                        | "rule"
                        | "panel"
                        | "table"
                        | "nestedExpand"
                ),
                Self::ListItem => ty == "taskList",
                Self::Blockquote => false,
                Self::Panel => matches!(ty, "heading" | "taskList" | "rule"),
                Self::TableCell => matches!(
                    ty,
                    "heading" | "taskList" | "blockquote" | "panel" | "rule" | "nestedExpand"
                ),
                Self::NestedExpand => {
                    matches!(ty, "heading" | "taskList" | "blockquote" | "panel" | "rule")
                }
            }
    }
}

/// The list marker a rendered list used. Adjacent lists alternate markers so CommonMark does not
/// merge them into one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Marker {
    Bullet(char),
    Ordered(char),
}

/// One rendered block.
#[derive(Debug, Clone)]
pub(super) struct Rendered {
    pub md: String,
    pub marker: Option<Marker>,
    /// A list that may follow a paragraph line directly (bullet or task list, or an ordered
    /// list starting at 1).
    pub interrupts: bool,
}

impl Rendered {
    fn plain(md: String) -> Self {
        Self {
            md,
            marker: None,
            interrupts: false,
        }
    }
}

/// Renders the whole document; output ends with exactly one newline.
pub(super) fn render_doc(root: &Node<'_>, cx: &mut Ctx) -> Result<String, crate::Diagnostic> {
    let blocks = match render_children(&root.content, Container::Doc, cx) {
        Ok(blocks) => blocks,
        Err(Fail::Fatal(diagnostic)) => return Err(diagnostic),
        Err(Fail::Unrepresentable { pointer, reason }) => {
            return Err(crate::Diagnostic::new(
                crate::Code::Internal,
                format!("a document-level node could not be passed through: {reason}"),
            )
            .at(crate::Location::pointer(pointer)));
        }
    };
    // Blocks never end with a newline; trimming makes "exactly one" hold regardless.
    let mut out = join_blocks(&blocks).trim_end_matches('\n').to_owned();
    out.push('\n');
    Ok(out)
}

fn join_blocks(blocks: &[Rendered]) -> String {
    blocks
        .iter()
        .map(|b| b.md.as_str())
        .filter(|md| !md.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Renders the children of a container. In roundtrip mode, children that cannot be rendered are
/// written as ```` ```adf ```` blocks (consecutive ones share a block).
pub(super) fn render_children(
    children: &[Node<'_>],
    container: Container,
    cx: &mut Ctx,
) -> Result<Vec<Rendered>, Fail> {
    let mut out = Vec::with_capacity(children.len());
    let mut raw: Vec<&Node<'_>> = Vec::new();
    let mut prev: Option<Marker> = None;
    let mut index = 0;
    while let Some(child) = children.get(index) {
        index += 1;
        if child.is_inline() {
            // Invalid input (inline content directly in a block container, reported as
            // W_INPUT_SCHEMA): write the run of inline nodes as one paragraph.
            let run_len = children
                .get(index - 1..)
                .unwrap_or_default()
                .iter()
                .take_while(|n| n.is_inline())
                .count();
            let run = children
                .get(index - 1..index - 1 + run_len)
                .unwrap_or_default();
            index += run_len - 1;
            flush_raw(&mut raw, &mut out, cx)?;
            let saved = cx.warnings.len();
            let md = match render_inline(run, InlineKind::Block, cx) {
                Ok(md) => md,
                Err(Fail::Unrepresentable { .. }) => {
                    cx.warnings.truncate(saved);
                    let text: String = run.iter().map(Node::text_content).collect();
                    plain_text_paragraph(&text)
                }
                Err(fatal @ Fail::Fatal(_)) => return Err(fatal),
            };
            if !md.is_empty() {
                out.push(Rendered::plain(md));
                prev = None;
            }
            continue;
        }
        let saved = cx.warnings.len();
        let result = if cx.roundtrip && !container.allows(child.ty) {
            let reason = if child.ty.is_empty() {
                "it has no type".to_owned()
            } else if is_known_block(child.ty) {
                format!("Markdown cannot hold it inside {}", container.name())
            } else {
                "it has no Markdown form".to_owned()
            };
            Err(Fail::unrepresentable(&child.pointer, &reason))
        } else {
            render_block(child, container, prev, cx)
        };
        match result {
            Ok(block) => {
                flush_raw(&mut raw, &mut out, cx)?;
                if !block.md.is_empty() {
                    prev = block.marker;
                    out.push(block);
                }
            }
            Err(Fail::Unrepresentable { pointer, reason }) if cx.roundtrip => {
                cx.warnings.truncate(saved);
                cx.warn_passthrough(child, &pointer, &reason);
                raw.push(child);
                prev = None;
            }
            Err(Fail::Unrepresentable { .. }) => {
                // Plain never fails on content; keep the text as a last resort.
                cx.warnings.truncate(saved);
                let text = plain_text_paragraph(&child.text_content());
                if !text.is_empty() {
                    out.push(Rendered::plain(text));
                    prev = None;
                }
            }
            Err(fatal @ Fail::Fatal(_)) => return Err(fatal),
        }
    }
    flush_raw(&mut raw, &mut out, cx)?;
    Ok(out)
}

fn flush_raw(raw: &mut Vec<&Node<'_>>, out: &mut Vec<Rendered>, cx: &Ctx) -> Result<(), Fail> {
    if !raw.is_empty() {
        out.push(Rendered::plain(raw_block(raw, cx.max_depth)?));
        raw.clear();
    }
    Ok(())
}

fn is_known_block(ty: &str) -> bool {
    matches!(
        ty,
        "paragraph"
            | "heading"
            | "bulletList"
            | "orderedList"
            | "taskList"
            | "codeBlock"
            | "blockquote"
            | "rule"
            | "panel"
            | "table"
            | "expand"
            | "nestedExpand"
            | "mediaSingle"
    )
}

fn render_block(
    node: &Node<'_>,
    container: Container,
    prev: Option<Marker>,
    cx: &mut Ctx,
) -> Result<Rendered, Fail> {
    cx.enter(node)?;
    let result = match node.ty {
        "paragraph" => paragraph(node, cx).map(Rendered::plain),
        "heading" => heading(node, cx).map(Rendered::plain),
        "bulletList" | "orderedList" => list(node, prev, cx),
        "taskList" => task_list(node, prev, cx).map(|md| Rendered {
            md,
            marker: Some(Marker::Bullet(bullet_after(prev))),
            interrupts: true,
        }),
        "codeBlock" => code_block(node, cx).map(Rendered::plain),
        "blockquote" => blockquote(node, cx).map(Rendered::plain),
        "rule" => {
            cx.drop_attrs(node, &[], &[]);
            Ok(Rendered::plain("---".to_owned()))
        }
        "panel" => panel(node, cx).map(Rendered::plain),
        "table" => table::render_table(node, cx).map(Rendered::plain),
        "expand" | "nestedExpand" => expand(node, cx).map(Rendered::plain),
        "mediaSingle" => media_single(node, cx).map(Rendered::plain),
        other => {
            if cx.roundtrip {
                Err(Fail::unrepresentable(
                    &node.pointer,
                    &format!("node '{other}' has no Markdown form"),
                ))
            } else {
                plain_fallback(node, container, cx).map(Rendered::plain)
            }
        }
    };
    cx.leave();
    result
}

// ---------------------------------------------------------------------------------------------
// Leaf blocks
// ---------------------------------------------------------------------------------------------

fn paragraph(node: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    cx.drop_block_marks(node);
    cx.drop_attrs(node, &[], &[]);
    if is_blank(&node.content) {
        return if cx.roundtrip {
            Err(Fail::unrepresentable(
                &node.pointer,
                "Markdown has no empty paragraphs here",
            ))
        } else {
            Ok(String::new())
        };
    }
    render_inline(&node.content, InlineKind::Block, cx)
}

fn heading(node: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    let level = node
        .attr("level")
        .and_then(serde_json::Value::as_u64)
        .filter(|l| (1..=6).contains(l))
        .and_then(|l| u8::try_from(l).ok());
    let level = match (level, cx.roundtrip) {
        (Some(level), _) => level,
        (None, true) => {
            return Err(Fail::unrepresentable(
                &node.pointer,
                "a heading level outside 1-6 has no Markdown form",
            ));
        }
        (None, false) => {
            let raw = node.attr("level").and_then(serde_json::Value::as_u64);
            u8::try_from(raw.unwrap_or(1).clamp(1, 6)).unwrap_or(1)
        }
    };
    cx.drop_block_marks(node);
    cx.drop_attrs(node, &["level"], &[]);
    let content = render_inline(&node.content, InlineKind::Heading(level), cx)?;
    let mut md = "#".repeat(usize::from(level));
    if !content.is_empty() {
        md.push(' ');
        md.push_str(&content);
    }
    Ok(md)
}

fn code_block(node: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    let language = node.attr_str("language").filter(|l| !l.is_empty());
    let language = match language {
        Some(lang) if lang.chars().any(char::is_whitespace) => {
            if cx.roundtrip {
                return Err(Fail::unrepresentable(
                    &node.pointer,
                    "a code block language containing spaces has no Markdown form",
                ));
            }
            lang.split_whitespace().next()
        }
        other => other,
    };
    let mut text = String::new();
    for child in &node.content {
        match child.ty {
            "text" => text.push_str(child.text.unwrap_or("")),
            _ if cx.roundtrip => {
                return Err(Fail::unrepresentable(
                    &child.pointer,
                    "a code block may only contain text",
                ));
            }
            _ => text.push_str(&child.text_content()),
        }
    }
    cx.drop_block_marks(node);
    cx.drop_attrs(node, &["language"], &[]);
    let fence_char = if language.is_some_and(|l| l.contains('`')) {
        '~'
    } else {
        '`'
    };
    let fence = fence_char
        .to_string()
        .repeat((longest_run(&text, fence_char) + 1).max(3));
    let mut md = format!("{fence}{}\n", language.unwrap_or(""));
    if !text.is_empty() {
        md.push_str(&text);
        md.push('\n');
    }
    md.push_str(&fence);
    Ok(md)
}

fn media_single(node: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    let media = match node.content.as_slice() {
        [media] if media.ty == "media" => Some(media),
        _ => None,
    };
    let external = media.filter(|m| {
        m.attr_str("type") == Some("external")
            && m.attr_str("url").is_some_and(|u| Href::parse(u).is_some())
            && m.marks.is_empty()
    });
    let Some(media) = external.filter(|_| node.marks.is_empty()) else {
        if cx.roundtrip {
            return Err(Fail::unrepresentable(
                &node.pointer,
                "only a single external image without caption or link maps to ![alt](url)",
            ));
        }
        // Plain: uploaded files have no URL to show; keep the alt text if there is one.
        let alt = node
            .content
            .iter()
            .find_map(|m| m.attr_str("alt"))
            .unwrap_or("");
        return Ok(plain_text_paragraph(alt));
    };
    cx.drop_attrs(node, &[], &[("layout", "center")]);
    cx.drop_attrs(media, &["type", "url", "alt"], &[]);
    let url = media.attr_str("url").unwrap_or("");
    let mut md = String::from("![");
    escape_text(
        media.attr_str("alt").unwrap_or(""),
        TextEscape {
            context: TextContext::Block,
            roundtrip: cx.roundtrip,
            line_start: false,
            line_end: false,
            in_link: true,
        },
        &mut md,
    );
    md.push_str("](");
    md.push_str(&link_destination(url, false));
    md.push(')');
    Ok(md)
}

// ---------------------------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------------------------

fn bullet_after(prev: Option<Marker>) -> char {
    if prev == Some(Marker::Bullet('-')) {
        '*'
    } else {
        '-'
    }
}

/// CommonMark ordered list numbers have at most nine digits.
const MAX_ORDER: u64 = 999_999_999;

fn list(node: &Node<'_>, prev: Option<Marker>, cx: &mut Ctx) -> Result<Rendered, Fail> {
    let ordered = node.ty == "orderedList";
    if node.content.is_empty() {
        return if cx.roundtrip {
            Err(Fail::unrepresentable(
                &node.pointer,
                "an empty list has no Markdown form",
            ))
        } else {
            Ok(Rendered::plain(String::new()))
        };
    }
    let start = if ordered {
        let order = node.attr("order");
        let parsed = match order {
            None => Some(1),
            Some(v) => v.as_u64(),
        };
        match parsed.filter(|n| *n <= MAX_ORDER) {
            Some(n) => n,
            None if cx.roundtrip => {
                return Err(Fail::unrepresentable(
                    &node.pointer,
                    "an ordered list must start at a whole number up to 999999999",
                ));
            }
            None => 1,
        }
    } else {
        0
    };
    cx.drop_attrs(node, if ordered { &["order"] } else { &[] }, &[]);
    let (marker, interrupts) = if ordered {
        let delim = if prev == Some(Marker::Ordered('.')) {
            ')'
        } else {
            '.'
        };
        (Marker::Ordered(delim), start == 1)
    } else {
        (Marker::Bullet(bullet_after(prev)), true)
    };
    let count = u64::try_from(node.content.len()).unwrap_or(u64::MAX);
    let sequential = start.saturating_add(count) <= MAX_ORDER + 1;
    let mut items = Vec::with_capacity(node.content.len());
    for (i, item) in (0_u64..).zip(&node.content) {
        let marker_text = match marker {
            Marker::Bullet(c) => c.to_string(),
            Marker::Ordered(delim) => {
                let n = if sequential { start + i } else { start };
                format!("{n}{delim}")
            }
        };
        items.push(list_item(item, &marker_text, cx)?);
    }
    let loose = items.iter().any(|md| md.contains("\n\n"));
    Ok(Rendered {
        md: items.join(if loose { "\n\n" } else { "\n" }),
        marker: Some(marker),
        interrupts,
    })
}

fn list_item(item: &Node<'_>, marker: &str, cx: &mut Ctx) -> Result<String, Fail> {
    if item.ty != "listItem" {
        if cx.roundtrip {
            return Err(Fail::unrepresentable(
                &item.pointer,
                "a list may only contain list items",
            ));
        }
        let text = plain_text_paragraph(&item.text_content());
        return Ok(indent_item(&text, marker));
    }
    cx.enter(item)?;
    let result = list_item_body(item, cx);
    cx.leave();
    Ok(indent_item(&result?, marker))
}

fn list_item_body(item: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    cx.drop_attrs(item, &[], &[]);
    let (first, rest) = match item.content.split_first() {
        Some((first, rest)) if first.ty == "paragraph" => {
            cx.drop_block_marks(first);
            cx.drop_attrs(first, &[], &[]);
            (render_inline(&first.content, InlineKind::Block, cx)?, rest)
        }
        _ if cx.roundtrip => {
            return Err(Fail::unrepresentable(
                &item.pointer,
                "a list item must start with a paragraph",
            ));
        }
        _ => (String::new(), item.content.as_slice()),
    };
    if cx.roundtrip
        && first.is_empty()
        && rest
            .first()
            .is_some_and(|n| matches!(n.ty, "paragraph" | "mediaSingle"))
    {
        return Err(Fail::unrepresentable(
            &item.pointer,
            "a list item whose empty first paragraph is followed by a paragraph has no Markdown form",
        ));
    }
    let blocks = render_children(rest, Container::ListItem, cx)?;
    let mut body = first;
    for (k, block) in blocks.iter().enumerate() {
        let tight = k == 0 && (body.is_empty() || block.interrupts);
        body.push_str(if tight { "\n" } else { "\n\n" });
        body.push_str(&block.md);
    }
    Ok(body)
}

/// Prefixes the first line with the marker and indents the rest to the content column.
fn indent_item(body: &str, marker: &str) -> String {
    let indent = " ".repeat(marker.chars().count() + 1);
    let mut out = String::with_capacity(body.len() + marker.len() + 8);
    for (i, line) in body.split('\n').enumerate() {
        if i == 0 {
            out.push_str(marker);
            if !line.is_empty() {
                out.push(' ');
                out.push_str(line);
            }
        } else {
            out.push('\n');
            if !line.is_empty() {
                out.push_str(&indent);
                out.push_str(line);
            }
        }
    }
    out
}

fn task_list(node: &Node<'_>, prev: Option<Marker>, cx: &mut Ctx) -> Result<String, Fail> {
    let bullet = bullet_after(prev);
    if cx.roundtrip && node.content.first().is_none_or(|n| n.ty != "taskItem") {
        return Err(Fail::unrepresentable(
            &node.pointer,
            "a task list must start with a task item",
        ));
    }
    cx.drop_attrs(node, &[], &[]);
    let mut items: Vec<String> = Vec::new();
    let mut nested_prev: Option<Marker> = None;
    for child in &node.content {
        match child.ty {
            "taskItem" => {
                items.push(task_item(child, bullet, cx)?);
                nested_prev = None;
            }
            "taskList" => {
                cx.enter(child)?;
                let nested = task_list(child, nested_prev, cx);
                cx.leave();
                let nested = nested?;
                if nested.is_empty() {
                    continue;
                }
                nested_prev = Some(Marker::Bullet(bullet_after(nested_prev)));
                let indented: String = nested
                    .split('\n')
                    .map(|line| {
                        if line.is_empty() {
                            String::new()
                        } else {
                            format!("  {line}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                match items.last_mut() {
                    Some(last) => {
                        last.push('\n');
                        last.push_str(&indented);
                    }
                    None => items.push(nested),
                }
            }
            _ if cx.roundtrip => {
                return Err(Fail::unrepresentable(
                    &child.pointer,
                    "a task list may only contain task items and task lists",
                ));
            }
            _ => {
                let text = plain_text_paragraph(&child.text_content());
                items.push(task_line(bullet, "[ ]", &text));
            }
        }
    }
    Ok(items.join("\n"))
}

fn task_item(node: &Node<'_>, bullet: char, cx: &mut Ctx) -> Result<String, Fail> {
    cx.enter(node)?;
    let result = task_item_body(node, bullet, cx);
    cx.leave();
    result
}

fn task_item_body(node: &Node<'_>, bullet: char, cx: &mut Ctx) -> Result<String, Fail> {
    let checkbox = match node.attr_str("state") {
        Some("TODO") => "[ ]",
        Some("DONE") => "[x]",
        _ if cx.roundtrip => {
            return Err(Fail::unrepresentable(
                &node.pointer,
                "a task item needs the state TODO or DONE",
            ));
        }
        _ => "[ ]",
    };
    cx.drop_attrs(node, &["state"], &[]);
    let content = if node.content.iter().all(Node::is_inline) {
        render_inline(&node.content, InlineKind::Block, cx)?
    } else if cx.roundtrip {
        return Err(Fail::unrepresentable(
            &node.pointer,
            "a task item may only contain inline content",
        ));
    } else {
        plain_text_paragraph(&node.text_content())
    };
    Ok(task_line(bullet, checkbox, &content))
}

/// `- [ ] content`, with continuation lines indented to the content column.
fn task_line(bullet: char, checkbox: &str, content: &str) -> String {
    let mut out = format!("{bullet} {checkbox}");
    for (i, line) in content.split('\n').enumerate() {
        if i == 0 {
            if !line.is_empty() {
                out.push(' ');
                out.push_str(line);
            }
        } else {
            out.push('\n');
            if !line.is_empty() {
                out.push_str("  ");
                out.push_str(line);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------------------------

/// Prefixes every line; empty lines get `empty`.
fn prefix_lines(text: &str, prefix: &str, empty: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.is_empty() {
                empty.to_owned()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `true` when the content is exactly one empty paragraph — what Markdown → ADF creates for an
/// empty alert, details body or box cell.
pub(super) fn is_single_empty_paragraph(content: &[Node<'_>]) -> bool {
    matches!(content, [p] if p.ty == "paragraph" && p.content.is_empty() && p.marks.is_empty())
}

fn blockquote(node: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    if node.content.is_empty() {
        return if cx.roundtrip {
            Err(Fail::unrepresentable(
                &node.pointer,
                "an empty quote has no Markdown form",
            ))
        } else {
            Ok(String::new())
        };
    }
    cx.drop_attrs(node, &[], &[]);
    let body = join_blocks(&render_children(&node.content, Container::Blockquote, cx)?);
    Ok(prefix_lines(&body, "> ", ">"))
}

fn panel(node: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    let kind = match node.attr_str("panelType") {
        Some("info") => Some("NOTE"),
        Some("success") => Some("TIP"),
        Some("note") => Some("IMPORTANT"),
        Some("warning") => Some("WARNING"),
        Some("error") => Some("CAUTION"),
        Some("tip") if !cx.roundtrip => Some("TIP"),
        _ if !cx.roundtrip => Some("NOTE"),
        _ => None,
    };
    let Some(kind) = kind else {
        return Err(Fail::unrepresentable(
            &node.pointer,
            "only info, success, note, warning and error panels map to GitHub alerts",
        ));
    };
    if cx.roundtrip && node.content.is_empty() {
        return Err(Fail::unrepresentable(
            &node.pointer,
            "an empty panel has no Markdown form",
        ));
    }
    cx.drop_attrs(node, &["panelType"], &[]);
    let body = if is_single_empty_paragraph(&node.content) {
        String::new()
    } else {
        join_blocks(&render_children(&node.content, Container::Panel, cx)?)
    };
    let mut md = format!("> [!{kind}]");
    if !body.is_empty() {
        md.push('\n');
        md.push_str(&prefix_lines(&body, "> ", ">"));
    }
    Ok(md)
}

fn expand(node: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    if cx.roundtrip && node.content.is_empty() {
        return Err(Fail::unrepresentable(
            &node.pointer,
            "an empty expand has no Markdown form",
        ));
    }
    let container = if node.ty == "expand" {
        Container::Expand
    } else {
        Container::NestedExpand
    };
    cx.drop_attrs(node, &["title"], &[]);
    let body = if is_single_empty_paragraph(&node.content) {
        String::new()
    } else {
        join_blocks(&render_children(&node.content, container, cx)?)
    };
    let mut md = String::from("<details>\n");
    if let Some(title) = node.attr_str("title").filter(|t| !t.is_empty()) {
        md.push_str("<summary>");
        md.push_str(&html_text(title));
        md.push_str("</summary>\n");
    }
    md.push('\n');
    if !body.is_empty() {
        md.push_str(&body);
        md.push_str("\n\n");
    }
    md.push_str("</details>");
    Ok(md)
}

// ---------------------------------------------------------------------------------------------
// Plain flavour fallbacks
// ---------------------------------------------------------------------------------------------

/// Plain flavour: a node without a Markdown form becomes its content.
fn plain_fallback(node: &Node<'_>, container: Container, cx: &mut Ctx) -> Result<String, Fail> {
    match node.ty {
        "decisionList" => {
            let items: Vec<String> = node
                .content
                .iter()
                .map(|item| render_inline(&item.content, InlineKind::Block, cx))
                .collect::<Result<_, _>>()?;
            Ok(items
                .iter()
                .filter(|md| !md.is_empty())
                .map(|md| indent_item(md, "-"))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "blockCard" | "embedCard" => Ok(node.attr_str("url").unwrap_or("").to_owned()),
        "extension" | "inlineExtension" | "mediaGroup" | "media" | "placeholder" => {
            Ok(String::new())
        }
        _ if !node.content.is_empty() && node.content.iter().all(Node::is_inline) => {
            render_inline(&node.content, InlineKind::Block, cx)
        }
        _ if !node.content.is_empty() => {
            Ok(join_blocks(&render_children(&node.content, container, cx)?))
        }
        _ => Ok(plain_text_paragraph(&node.text_content())),
    }
}

/// Escaped plain text as a paragraph (plain flavour); lines become hard breaks.
fn plain_text_paragraph(text: &str) -> String {
    let lines: Vec<&str> = text.trim_end_matches('\n').split('\n').collect();
    let last = lines.len().saturating_sub(1);
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push_str("\\\n");
        }
        escape_text(
            line,
            TextEscape {
                context: TextContext::Block,
                roundtrip: false,
                line_start: true,
                line_end: i == last,
                in_link: false,
            },
            &mut out,
        );
    }
    out
}
