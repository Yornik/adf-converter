//! Block nodes and containers.
//!
//! Every block renders to Markdown lines without a trailing newline; siblings are separated by a
//! blank line. Containers prefix their children's lines (`> ` for quotes and panels, spaces for
//! list items), so laziness rules never come into play.

use crate::adf::model::Href;
use crate::adf::reader::Node;

use super::escape::{
    TextContext, TextEscape, escape_text, has_unsafe_control, html_text, info_string,
    link_destination, longest_run, normalize_cr, replace_controls,
};
use super::inline::{InlineKind, is_blank, plain_autolink, render_inline};
use super::media::{file_media_dest, group_markers, marker};
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
    /// list starting at 1, whose first item does not start with an empty line).
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
    let fatal = |fail: Fail| match fail {
        Fail::Fatal(diagnostic) => *diagnostic,
        Fail::Unrepresentable { pointer, reason } => crate::Diagnostic::new(
            crate::Code::Internal,
            format!("a document-level node could not be passed through: {reason}"),
        )
        .at(crate::Location::pointer(pointer)),
    };
    let mut blocks = render_children(&root.content, Container::Doc, cx).map_err(fatal)?;
    // A leading `---` could open YAML front matter, which Markdown → ADF removes.
    if let Some(first) = blocks.first_mut()
        && first.md == "---"
    {
        "***".clone_into(&mut first.md);
    }
    let mut out = join_blocks(blocks);
    // Blocks never end with a newline; trimming makes "exactly one" hold regardless.
    out.truncate(out.trim_end_matches('\n').len());
    // Markdown → ADF strips a byte order mark at the start of the input. Only paragraph text
    // can start the output with U+FEFF, and there a character reference keeps it.
    if let Some(rest) = out.strip_prefix('\u{feff}') {
        out = format!("&#65279;{rest}");
    }
    out.push('\n');
    cx.check_output(out.len(), root).map_err(fatal)?;
    Ok(out)
}

// Containers build their Markdown from their children's strings. Hundreds of levels deep, a new
// string per level (each a little longer than the one below) keeps lines × depth² bytes alive in
// the allocator, so the largest child's buffer is reused instead: joining inserts the other
// parts around it, and prefixing moves its lines in place.

/// The blocks separated by blank lines, empty ones left out.
fn join_blocks(blocks: Vec<Rendered>) -> String {
    let mut parts = Vec::with_capacity(blocks.len());
    for block in blocks.into_iter().filter(|b| !b.md.is_empty()) {
        let sep = if parts.is_empty() { "" } else { "\n\n" };
        parts.push((sep, block.md));
    }
    join_parts(parts)
}

/// Concatenates the parts, each after its separator, in the buffer of the largest part.
fn join_parts(parts: Vec<(&str, String)>) -> String {
    let largest = parts
        .iter()
        .enumerate()
        .max_by_key(|(_, (_, part))| part.len())
        .map_or(0, |(i, _)| i);
    let mut before = String::new();
    let mut out = String::new();
    let mut after = String::new();
    for (i, (sep, part)) in parts.into_iter().enumerate() {
        match i.cmp(&largest) {
            std::cmp::Ordering::Less => {
                before.push_str(sep);
                before.push_str(&part);
            }
            std::cmp::Ordering::Equal => {
                before.push_str(sep);
                out = part;
            }
            std::cmp::Ordering::Greater => {
                after.push_str(sep);
                after.push_str(&part);
            }
        }
    }
    out.reserve(before.len() + after.len());
    out.insert_str(0, &before);
    out.push_str(&after);
    out
}

/// Line prefixes for [`prefix_in_place`]: one for the first line, one for the others, each with
/// a variant for empty lines.
#[derive(Debug, Clone, Copy)]
struct Prefixes<'p> {
    first: &'p str,
    first_empty: &'p str,
    rest: &'p str,
    rest_empty: &'p str,
}

impl Prefixes<'_> {
    fn of(&self, first: bool, empty: bool) -> &str {
        match (first, empty) {
            (true, false) => self.first,
            (true, true) => self.first_empty,
            (false, false) => self.rest,
            (false, true) => self.rest_empty,
        }
    }
}

/// Prefixes every line of `text` in its own buffer: the buffer grows once and the lines move
/// back, last line first. Fails with `E_INPUT_TOO_LARGE` beyond the output budget, before it
/// grows.
fn prefix_in_place(
    text: String,
    prefixes: Prefixes<'_>,
    node: &Node<'_>,
    cx: &Ctx,
) -> Result<String, Fail> {
    let mut bytes = text.into_bytes();
    let old_len = bytes.len();
    let mut new_len = old_len;
    let mut start = 0;
    loop {
        let end = bytes
            .get(start..)
            .and_then(|rest| rest.iter().position(|&b| b == b'\n'))
            .map_or(old_len, |p| start + p);
        new_len = new_len.saturating_add(prefixes.of(start == 0, end == start).len());
        if end >= old_len {
            break;
        }
        start = end + 1;
    }
    cx.check_output(new_len, node)?;
    bytes.resize(new_len, 0);
    // Invariant: everything before `read_end` is unread input, everything from `write_end` on
    // is output, and `write_end - read_end` is the size of the prefixes still to write (≥ 0).
    let mut read_end = old_len;
    let mut write_end = new_len;
    loop {
        let start = bytes
            .get(..read_end)
            .and_then(|head| head.iter().rposition(|&b| b == b'\n'))
            .map_or(0, |p| p + 1);
        let len = read_end - start;
        let prefix = prefixes.of(start == 0, len == 0);
        let dest = write_end - len;
        bytes.copy_within(start..read_end, dest);
        let prefix_start = dest - prefix.len();
        if let Some(slot) = bytes.get_mut(prefix_start..dest) {
            slot.copy_from_slice(prefix.as_bytes());
        }
        if start == 0 {
            break;
        }
        write_end = prefix_start - 1;
        if let Some(newline) = bytes.get_mut(write_end) {
            *newline = b'\n';
        }
        read_end = start - 1;
    }
    String::from_utf8(bytes).map_err(|_| Fail::internal("prefixed Markdown is not UTF-8"))
}

/// Renders the children of a container. In roundtrip mode, children that cannot be rendered are
/// written as ```` ```adf ```` blocks (consecutive ones share a block).
pub(super) fn render_children(
    children: &[Node<'_>],
    container: Container,
    cx: &mut Ctx,
) -> Result<Vec<Rendered>, Fail> {
    let mut out = Blocks::with_capacity(children.len());
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
            out.flush_raw(&mut raw, cx)?;
            let saved = cx.mark();
            let md = match render_inline(run, InlineKind::Block, cx) {
                Ok(md) => md,
                Err(Fail::Unrepresentable { .. }) => {
                    cx.rollback(saved);
                    let text: String = run.iter().map(Node::text_content).collect();
                    plain_text_paragraph(&text)
                }
                Err(fatal @ Fail::Fatal(_)) => return Err(fatal),
            };
            if !md.is_empty() {
                out.push(Rendered::plain(md), child, cx)?;
                prev = None;
            }
            continue;
        }
        let saved = cx.mark();
        let result = if cx.roundtrip && !container.allows(child.ty) {
            let reason = if child.ty.is_empty() {
                "it has no type".to_owned()
            } else if is_known_block(child.ty) {
                format!("Markdown cannot hold it inside {}", container.name())
            } else {
                "it has no Markdown form".to_owned()
            };
            Err(Fail::unrepresentable(&child.pointer(), &reason))
        } else {
            render_block(child, container, prev, cx)
        };
        match result {
            Ok(block) => {
                out.flush_raw(&mut raw, cx)?;
                if !block.md.is_empty() {
                    prev = block.marker;
                    out.push(block, child, cx)?;
                }
            }
            Err(Fail::Unrepresentable { pointer, reason }) if cx.roundtrip => {
                cx.rollback(saved);
                cx.warn_passthrough(child, &pointer, &reason);
                raw.push(child);
                prev = None;
            }
            Err(Fail::Unrepresentable { .. }) => {
                // Plain never fails on content; keep the text as a last resort.
                cx.rollback(saved);
                let text = plain_text_paragraph(&child.text_content());
                if !text.is_empty() {
                    out.push(Rendered::plain(text), child, cx)?;
                    prev = None;
                }
            }
            Err(fatal @ Fail::Fatal(_)) => return Err(fatal),
        }
    }
    out.flush_raw(&mut raw, cx)?;
    Ok(out.blocks)
}

/// The blocks of one container, with their joined size checked against the output budget as
/// they are collected.
struct Blocks {
    blocks: Vec<Rendered>,
    len: usize,
}

impl Blocks {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            blocks: Vec::with_capacity(capacity),
            len: 0,
        }
    }

    fn push(&mut self, block: Rendered, node: &Node<'_>, cx: &Ctx) -> Result<(), Fail> {
        self.len = self.len.saturating_add(block.md.len()).saturating_add(2);
        self.blocks.push(block);
        cx.check_output(self.len, node)
    }

    fn flush_raw(&mut self, raw: &mut Vec<&Node<'_>>, cx: &Ctx) -> Result<(), Fail> {
        if let Some(first) = raw.first().copied() {
            let md = raw_block(raw, cx)?;
            raw.clear();
            self.push(Rendered::plain(md), first, cx)?;
        }
        Ok(())
    }
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

/// Renders one block of a given type.
type Render = fn(&Node<'_>, Container, Option<Marker>, &mut Ctx) -> Result<Rendered, Fail>;

/// Renders one block. The type picks a [`Render`] function first, so that the recursion through
/// nested containers keeps one small frame here per level: a `match` calling every renderer
/// directly holds all their temporaries at once in debug builds, and 250 nested quotes would
/// then overflow a 2 MiB thread stack.
fn render_block(
    node: &Node<'_>,
    container: Container,
    prev: Option<Marker>,
    cx: &mut Ctx,
) -> Result<Rendered, Fail> {
    let render: Render = match node.ty {
        "paragraph" => |node, _, _, cx| paragraph(node, cx).map(Rendered::plain),
        "heading" => |node, _, _, cx| heading(node, cx).map(Rendered::plain),
        "bulletList" | "orderedList" => |node, _, prev, cx| list(node, prev, cx),
        "taskList" => |node, _, prev, cx| {
            task_list(node, prev, cx).map(|md| Rendered {
                md,
                marker: Some(Marker::Bullet(bullet_after(prev))),
                interrupts: true,
            })
        },
        "codeBlock" => |node, _, _, cx| code_block(node, cx).map(Rendered::plain),
        "blockquote" => |node, _, _, cx| blockquote(node, cx).map(Rendered::plain),
        "rule" => |node, _, _, cx| {
            cx.drop_attrs(node, &[], &[]);
            Ok(Rendered::plain("---".to_owned()))
        },
        "panel" => |node, _, _, cx| panel(node, cx).map(Rendered::plain),
        "table" => |node, _, _, cx| table::render_table(node, cx).map(Rendered::plain),
        "expand" | "nestedExpand" => |node, _, _, cx| expand(node, cx).map(Rendered::plain),
        "mediaSingle" => |node, _, _, cx| media_single(node, false, cx).map(Rendered::plain),
        _ => |node, container, _, cx| {
            if cx.roundtrip {
                Err(Fail::unrepresentable(
                    &node.pointer(),
                    &format!("node '{}' has no Markdown form", node.ty),
                ))
            } else {
                plain_fallback(node, container, cx).map(Rendered::plain)
            }
        },
    };
    cx.enter(node)?;
    let result = render(node, container, prev, cx);
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
                &node.pointer(),
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
                &node.pointer(),
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
                    &node.pointer(),
                    "a code block language containing spaces has no Markdown form",
                ));
            }
            lang.split_whitespace().next()
        }
        // The fence would be read back as raw ADF (or fail to parse as such).
        Some("adf") if cx.roundtrip => {
            return Err(Fail::unrepresentable(
                &node.pointer(),
                "a code block in the language adf would be read back as raw ADF",
            ));
        }
        // The fence would be read back as a table (extension `csv`).
        Some(lang @ ("csv" | "tsv")) if cx.roundtrip => {
            return Err(Fail::unrepresentable(
                &node.pointer(),
                &format!("a code block in the language {lang} would be read back as a table"),
            ));
        }
        other => other,
    };
    let mut text = String::new();
    for child in &node.content {
        match child.ty {
            "text" => text.push_str(child.text.unwrap_or("")),
            _ if cx.roundtrip => {
                return Err(Fail::unrepresentable(
                    &child.pointer(),
                    "a code block may only contain text",
                ));
            }
            _ => text.push_str(&child.text_content()),
        }
    }
    let (normalized, had_cr) = normalize_cr(&text);
    if had_cr {
        text = normalized.into_owned();
        cx.warn_control_chars(
            &node.pointer(),
            "Carriage returns in a code block were written as line breaks",
        );
    }
    if has_unsafe_control(&text) || language.is_some_and(has_unsafe_control) {
        if cx.roundtrip {
            return Err(Fail::unrepresentable(
                &node.pointer(),
                "Markdown → ADF removes control characters from code",
            ));
        }
        text = replace_controls(&text, true);
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
    let info = info_string(language.unwrap_or(""));
    // A language starting with the fence character would lengthen the fence; the info string
    // is trimmed, so a space keeps them apart.
    let gap = if info.starts_with(fence_char) {
        " "
    } else {
        ""
    };
    let mut md = format!("{fence}{gap}{info}\n");
    if !text.is_empty() {
        md.push_str(&text);
        md.push('\n');
    }
    md.push_str(&fence);
    Ok(md)
}

/// `![alt](url)` for a single external image. `in_table`: written inside a GFM table cell.
pub(super) fn media_single(node: &Node<'_>, in_table: bool, cx: &mut Ctx) -> Result<String, Fail> {
    if !cx.roundtrip {
        return plain_media_single(node, in_table, cx);
    }
    let media = match node.content.as_slice() {
        [media] if media.ty == "media" => Some(media),
        _ => None,
    };
    let mut file_reason = None;
    if let Some(file) = media.filter(|m| m.attr_str("type") == Some("file")) {
        match file_media_dest(node, file) {
            Ok(dest) => {
                return Ok(image_line(
                    file.attr_str("alt").unwrap_or(""),
                    &dest,
                    in_table,
                    true,
                ));
            }
            Err(reason) => file_reason = Some(reason),
        }
    }
    let external = media.filter(|m| is_external_image(m) && m.marks.is_empty());
    let Some(media) = external.filter(|_| node.marks.is_empty()) else {
        return Err(Fail::unrepresentable(
            &node.pointer(),
            &file_reason.unwrap_or_else(|| {
                "only a single external image or a plain file image maps to ![alt](url)".to_owned()
            }),
        ));
    };
    if media.attr_str("alt").is_some_and(has_unsafe_control) {
        return Err(Fail::unrepresentable(
            &media.pointer(),
            "Markdown → ADF removes control characters from alt text",
        ));
    }
    cx.drop_attrs(node, &[], &[("layout", "center")]);
    cx.drop_attrs(media, &["type", "url", "alt"], &[]);
    Ok(image(media, in_table, cx.roundtrip))
}

/// A `media` node with type `external` and an allowed URL.
fn is_external_image(media: &Node<'_>) -> bool {
    media.attr_str("type") == Some("external")
        && media
            .attr_str("url")
            .is_some_and(|u| Href::parse(u).is_some())
}

/// `![alt](url)` for an external `media` node.
fn image(media: &Node<'_>, in_table: bool, roundtrip: bool) -> String {
    let url = media.attr_str("url").unwrap_or("");
    image_line(
        media.attr_str("alt").unwrap_or(""),
        &link_destination(url, in_table),
        in_table,
        roundtrip,
    )
}

/// `![alt](dest)`: `alt` is escaped, `dest` is written as given.
fn image_line(alt: &str, dest: &str, in_table: bool, roundtrip: bool) -> String {
    let mut md = String::from("![");
    escape_text(
        alt,
        TextEscape {
            context: if in_table {
                TextContext::TableCell
            } else {
                TextContext::Block
            },
            roundtrip,
            line_start: false,
            block_start: false,
            line_end: false,
            in_link: true,
        },
        &mut md,
    );
    md.push_str("](");
    md.push_str(dest);
    md.push(')');
    md
}

/// Plain: an external image is `![alt](url)` whatever its caption or marks; an uploaded file
/// (no URL in ADF) is `[image: alt]`. The caption follows on the next line.
fn plain_media_single(node: &Node<'_>, in_table: bool, cx: &mut Ctx) -> Result<String, Fail> {
    let mut md = match node.content.iter().find(|n| n.ty == "media") {
        Some(media) if is_external_image(media) => image(media, in_table, false),
        Some(media) => marker("image", media.attr_str("alt"), in_table),
        None => String::new(),
    };
    if let Some(caption) = node.content.iter().find(|n| n.ty == "caption") {
        let kind = if in_table {
            InlineKind::TableCell
        } else {
            InlineKind::Block
        };
        let text = render_inline(&caption.content, kind, cx)?;
        if !text.is_empty() {
            if !md.is_empty() {
                md.push_str(if in_table { "<br>" } else { "\\\n" });
            }
            md.push_str(&text);
        }
    }
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
                &node.pointer(),
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
                    &node.pointer(),
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
    let mut empty_first_line = false;
    let mut total: usize = 0;
    for (i, item) in (0_u64..).zip(&node.content) {
        let marker_text = match marker {
            Marker::Bullet(c) => c.to_string(),
            Marker::Ordered(delim) => {
                let n = if sequential { start + i } else { start };
                format!("{n}{delim}")
            }
        };
        let md = list_item(item, &marker_text, cx)?;
        if i == 0 {
            empty_first_line = md.split('\n').next() == Some(marker_text.as_str());
        }
        total = total.saturating_add(md.len()).saturating_add(2);
        cx.check_output(total, item)?;
        items.push(md);
    }
    let loose = items.iter().any(|md| md.contains("\n\n"));
    let sep = if loose { "\n\n" } else { "\n" };
    let parts = items
        .into_iter()
        .enumerate()
        .map(|(i, md)| (if i == 0 { "" } else { sep }, md))
        .collect();
    Ok(Rendered {
        md: join_parts(parts),
        marker: Some(marker),
        // A marker alone on its line cannot interrupt a paragraph (`-` would even underline it
        // as a setext heading).
        interrupts: interrupts && !empty_first_line,
    })
}

fn list_item(item: &Node<'_>, marker: &str, cx: &mut Ctx) -> Result<String, Fail> {
    if item.ty != "listItem" {
        if cx.roundtrip {
            return Err(Fail::unrepresentable(
                &item.pointer(),
                "a list may only contain list items",
            ));
        }
        let text = plain_text_paragraph(&item.text_content());
        return indent_item(text, marker, item, cx);
    }
    cx.enter(item)?;
    let result = list_item_body(item, cx);
    cx.leave();
    indent_item(result?, marker, item, cx)
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
                &item.pointer(),
                "a list item must start with a paragraph",
            ));
        }
        _ => (String::new(), item.content.as_slice()),
    };
    let (first, rest) = match rest.split_first() {
        // `- ![alt](url)`: Markdown → ADF puts the image after an empty first paragraph.
        Some((image, after)) if cx.roundtrip && first.is_empty() && image.ty == "mediaSingle" => {
            cx.enter(image)?;
            let md = media_single(image, false, cx);
            cx.leave();
            (md?, after)
        }
        Some((next, _)) if cx.roundtrip && first.is_empty() && next.ty == "paragraph" => {
            return Err(Fail::unrepresentable(
                &item.pointer(),
                "a list item whose empty first paragraph is followed by a paragraph has no Markdown form",
            ));
        }
        _ => (first, rest),
    };
    let blocks = render_children(rest, Container::ListItem, cx)?;
    let first_empty = first.is_empty();
    let mut parts = vec![("", first)];
    for (k, block) in blocks.into_iter().enumerate() {
        let tight = k == 0 && (first_empty || block.interrupts);
        parts.push((if tight { "\n" } else { "\n\n" }, block.md));
    }
    Ok(join_parts(parts))
}

/// Prefixes the first line with the marker and indents the rest to the content column. Fails
/// with `E_INPUT_TOO_LARGE` beyond the output budget.
fn indent_item(body: String, marker: &str, node: &Node<'_>, cx: &Ctx) -> Result<String, Fail> {
    let first = format!("{marker} ");
    let indent = " ".repeat(marker.chars().count() + 1);
    let prefixes = Prefixes {
        first: &first,
        first_empty: marker,
        rest: &indent,
        rest_empty: "",
    };
    prefix_in_place(body, prefixes, node, cx)
}

/// The size of `text` with `prefix_len` bytes added in front of every line (an upper bound for
/// [`push_lines`]).
fn prefixed_len(text: &str, prefix_len: usize) -> usize {
    let lines = text.bytes().filter(|&b| b == b'\n').count() + 1;
    text.len().saturating_add(lines.saturating_mul(prefix_len))
}

fn task_list(node: &Node<'_>, prev: Option<Marker>, cx: &mut Ctx) -> Result<String, Fail> {
    let mut out = String::new();
    write_task_list(node, prev, 0, cx, &mut out)?;
    Ok(out)
}

/// Writes a task list nested `level` lists deep into `out`, indented two spaces per level.
/// Nested lists write straight into `out`, so every line is copied once however deep the lists
/// nest (re-indenting each level's text would cost lines × depth² memory).
fn write_task_list(
    node: &Node<'_>,
    prev: Option<Marker>,
    level: usize,
    cx: &mut Ctx,
    out: &mut String,
) -> Result<(), Fail> {
    let bullet = bullet_after(prev);
    if cx.roundtrip && node.content.first().is_none_or(|n| n.ty != "taskItem") {
        return Err(Fail::unrepresentable(
            &node.pointer(),
            "a task list must start with a task item",
        ));
    }
    cx.drop_attrs(node, &[], &[]);
    let indent = "  ".repeat(level);
    let mut has_item = false;
    let mut nested_prev: Option<Marker> = None;
    for child in &node.content {
        match child.ty {
            "taskItem" => {
                let item = task_item(child, bullet, cx)?;
                push_lines(out, &item, &indent, child, cx)?;
                has_item = true;
                nested_prev = None;
            }
            "taskList" => {
                // A nested list belongs to the item before it; without one (plain only) it is
                // written at this level.
                let nested_level = if has_item { level + 1 } else { level };
                let before = out.len();
                cx.enter(child)?;
                let result = write_task_list(child, nested_prev, nested_level, cx, out);
                cx.leave();
                result?;
                if out.len() > before {
                    nested_prev = Some(Marker::Bullet(bullet_after(nested_prev)));
                    has_item = true;
                }
            }
            _ if cx.roundtrip => {
                return Err(Fail::unrepresentable(
                    &child.pointer(),
                    "a task list may only contain task items and task lists",
                ));
            }
            _ => {
                let text = plain_text_paragraph(&child.text_content());
                push_lines(out, &task_line(bullet, "[ ]", &text), &indent, child, cx)?;
                has_item = true;
            }
        }
    }
    Ok(())
}

/// Appends `text` as the next lines of `out`, every non-empty line prefixed with `indent`;
/// fails with `E_INPUT_TOO_LARGE` beyond the output budget.
fn push_lines(
    out: &mut String,
    text: &str,
    indent: &str,
    node: &Node<'_>,
    cx: &Ctx,
) -> Result<(), Fail> {
    let len = prefixed_len(text, indent.len()).saturating_add(1);
    cx.check_output(out.len().saturating_add(len), node)?;
    out.reserve(len);
    if !out.is_empty() {
        out.push('\n');
    }
    push_prefixed(out, text, indent, "");
    Ok(())
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
                &node.pointer(),
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
            &node.pointer(),
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

/// Appends `text` to `out` with every line prefixed; empty lines get `empty`.
fn push_prefixed(out: &mut String, text: &str, prefix: &str, empty: &str) {
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.is_empty() {
            out.push_str(empty);
        } else {
            out.push_str(prefix);
            out.push_str(line);
        }
    }
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
                &node.pointer(),
                "an empty quote has no Markdown form",
            ))
        } else {
            Ok(String::new())
        };
    }
    cx.drop_attrs(node, &[], &[]);
    // A lone `>` parses as a quote holding one empty paragraph (what Markdown → ADF writes for
    // a quote whose only content moved elsewhere, such as a footnote definition).
    let body = if is_single_empty_paragraph(&node.content) {
        String::new()
    } else {
        join_blocks(render_children(&node.content, Container::Blockquote, cx)?)
    };
    prefix_in_place(body, QUOTE, node, cx)
}

/// `> ` before every line of a quote or alert, `>` alone on empty lines.
const QUOTE: Prefixes<'static> = Prefixes {
    first: "> ",
    first_empty: ">",
    rest: "> ",
    rest_empty: ">",
};

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
            &node.pointer(),
            "only info, success, note, warning and error panels map to GitHub alerts",
        ));
    };
    if cx.roundtrip && node.content.is_empty() {
        return Err(Fail::unrepresentable(
            &node.pointer(),
            "an empty panel has no Markdown form",
        ));
    }
    cx.drop_attrs(node, &["panelType"], &[]);
    let blocks = if is_single_empty_paragraph(&node.content) {
        Vec::new()
    } else {
        render_children(&node.content, Container::Panel, cx)?
    };
    // The `[!KIND]` line is a paragraph line to CommonMark: a list that cannot interrupt a
    // paragraph (`2.`, or a marker alone on its line) needs a blank line before it.
    let gap = blocks
        .first()
        .is_some_and(|b| b.marker.is_some() && !b.interrupts);
    let body = join_blocks(blocks);
    let mut head = format!("> [!{kind}]");
    if body.is_empty() {
        return Ok(head);
    }
    if gap {
        head.push_str("\n>");
    }
    head.push('\n');
    let mut md = prefix_in_place(body, QUOTE, node, cx)?;
    cx.check_output(md.len() + head.len(), node)?;
    md.insert_str(0, &head);
    Ok(md)
}

fn expand(node: &Node<'_>, cx: &mut Ctx) -> Result<String, Fail> {
    if cx.roundtrip && node.content.is_empty() {
        return Err(Fail::unrepresentable(
            &node.pointer(),
            "an empty expand has no Markdown form",
        ));
    }
    let container = if node.ty == "expand" {
        Container::Expand
    } else {
        Container::NestedExpand
    };
    if cx.roundtrip
        && let Some(title) = node.attr_str("title")
        && !title_survives(title)
    {
        return Err(Fail::unrepresentable(
            &node.pointer(),
            "Markdown → ADF collapses whitespace and removes control characters in a <summary> title",
        ));
    }
    cx.drop_attrs(node, &["title"], &[]);
    let body = if is_single_empty_paragraph(&node.content) {
        String::new()
    } else {
        join_blocks(render_children(&node.content, container, cx)?)
    };
    let mut head = String::from("<details>\n");
    if let Some(title) = node.attr_str("title").filter(|t| !t.is_empty()) {
        head.push_str("<summary>");
        head.push_str(&html_text(title));
        head.push_str("</summary>\n");
    }
    head.push('\n');
    if body.is_empty() {
        head.push_str("</details>");
        return Ok(head);
    }
    // Around the body, in its buffer.
    let mut md = body;
    cx.check_output(md.len() + head.len() + 12, node)?;
    md.insert_str(0, &head);
    md.push_str("\n\n</details>");
    Ok(md)
}

/// Whether Markdown → ADF reads `<summary>title</summary>` back as `title`: it removes control
/// characters, collapses runs of whitespace to one space and trims the ends.
fn title_survives(title: &str) -> bool {
    !title.chars().any(char::is_control)
        && title.split_whitespace().collect::<Vec<_>>().join(" ") == title
}

// ---------------------------------------------------------------------------------------------
// Plain flavour fallbacks
// ---------------------------------------------------------------------------------------------

/// Plain flavour: a node without a Markdown form becomes its content.
fn plain_fallback(node: &Node<'_>, container: Container, cx: &mut Ctx) -> Result<String, Fail> {
    match node.ty {
        "decisionList" => {
            let mut out = String::new();
            for item in &node.content {
                let md = render_inline(&item.content, InlineKind::Block, cx)?;
                if !md.is_empty() {
                    let line = indent_item(md, "-", item, cx)?;
                    push_lines(&mut out, &line, "", item, cx)?;
                }
            }
            Ok(out)
        }
        // Cards are bare URLs, filtered and escaped like links: the URL is untrusted text.
        "blockCard" | "embedCard" => Ok(match node.attr_str("url").and_then(Href::parse) {
            Some(href) if plain_autolink(href.as_str()) => format!("<{}>", href.as_str()),
            Some(href) => plain_text_paragraph(href.as_str()),
            None => String::new(),
        }),
        // Media Services files have no URL in ADF; a marker shows that something was there.
        "mediaGroup" => Ok(group_markers(node, false).join("\\\n")),
        // A `media` node outside `mediaSingle` and `mediaGroup` is invalid ADF, but not nothing.
        "media" => Ok(marker("attachment", node.attr_str("alt"), false)),
        "extension" | "inlineExtension" | "placeholder" => Ok(String::new()),
        _ if !node.content.is_empty() && node.content.iter().all(Node::is_inline) => {
            render_inline(&node.content, InlineKind::Block, cx)
        }
        _ if !node.content.is_empty() => {
            Ok(join_blocks(render_children(&node.content, container, cx)?))
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
                block_start: false,
                line_end: i == last,
                in_link: false,
            },
            &mut out,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn prefixed(text: &str, prefixes: Prefixes<'_>) -> String {
        let doc = json!({"type": "doc", "content": []});
        let root = crate::adf::reader::read_doc(&doc, &Limits::default()).unwrap();
        prefix_in_place(text.to_owned(), prefixes, &root, &cx(usize::MAX)).unwrap()
    }

    #[test]
    fn prefixing_in_place_matches_line_by_line_prefixing() {
        let item = Prefixes {
            first: "12. ",
            first_empty: "12.",
            rest: "    ",
            rest_empty: "",
        };
        for text in ["", "a", "\n", "a\nb", "a\n\nb\n", "\n\nx", "äö\n\n€ y\n"] {
            let mut expected = String::new();
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 {
                    expected.push('\n');
                }
                expected.push_str(item.of(i == 0, line.is_empty()));
                expected.push_str(line);
            }
            assert_eq!(prefixed(text, item), expected, "{text:?}");
        }
        assert_eq!(prefixed("a\n\nb", QUOTE), "> a\n>\n> b");
    }

    #[test]
    fn prefixing_beyond_the_budget_fails_before_growing() {
        let doc = json!({"type": "doc", "content": []});
        let root = crate::adf::reader::read_doc(&doc, &Limits::default()).unwrap();
        let text = "x\n".repeat(100);
        // 200 bytes, 100 lines with `> ` and an empty last one with `>`.
        assert!(prefix_in_place(text.clone(), QUOTE, &root, &cx(401)).is_ok());
        assert!(matches!(
            prefix_in_place(text, QUOTE, &root, &cx(400)),
            Err(Fail::Fatal(d)) if d.code == crate::Code::InputTooLarge
        ));
    }

    #[test]
    fn joining_reuses_the_largest_part() {
        let parts = vec![
            ("", "a".to_owned()),
            ("\n", "long part".to_owned()),
            ("\n\n", String::new()),
            ("\n", "c".to_owned()),
        ];
        assert_eq!(join_parts(parts), "a\nlong part\n\n\nc");
        assert_eq!(join_parts(Vec::new()), "");
    }
}
