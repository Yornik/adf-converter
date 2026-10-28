//! Lowers the IR into the strict ADF model, applying the container rules of
//! `docs/syntax.md#container-rules`:
//!
//! 1. **demote**: a heading in a list item or quote becomes a bold paragraph;
//! 2. **flatten**: a quote in a quote is merged, an alert or `<details>` where a panel or expand
//!    is not allowed is converted;
//! 3. **convert**: `<details>` in an expand or a table cell becomes a `nestedExpand`;
//! 4. **hoist**: anything else that a container cannot hold closes the container, is placed in
//!    the nearest ancestor that allows it, and the container is reopened after it.
//!
//! Hoisting moves *IR* blocks, not lowered nodes: a hoisted block is lowered by the container
//! that finally holds it, so context-dependent conversions happen in the right place and the
//! document order is preserved. Each container converts its lowered children into its own
//! child enum from `adf::model`, which cannot represent an invalid placement.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::adf::model::{
    Align, AlignedParagraph, Blockquote, BlockquoteChild, BulletList, CellKind, CodeBlock, Doc,
    DocBlock, Expand, ExpandChild, Formatting, Heading, HeadingLevel, Inline, ListItem,
    ListItemChild, MediaGroup, MediaSingle, NestedExpand, NestedExpandChild, NonEmpty, OrderedList,
    Panel, PanelChild, PanelType, Paragraph, RawNode, Rule, Table, TableCell, TableCellChild,
    TableRow, TaskItem, TaskList, TaskListChild, TaskState,
};
use crate::diagnostics::{Code, Diagnostic};

use super::cx::Cx;
use super::extensions::{RawPlace, truncate, validate_raw};
use super::finalize::{
    Segment, embolden, ends_with_space, finalize, split_media, trim_leading_space,
};
use super::ir::{Block, Footnote, Item, Marks, Row, Span, Tok};
use super::tree::Parsed;

/// A container kind, deciding what may be placed directly in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Doc,
    ListItem,
    Quote,
    Panel,
    Cell,
    Expand,
    NestedExpand,
}

impl Ctx {
    fn describe(self) -> &'static str {
        match self {
            Self::Doc => "the document",
            Self::ListItem => "a list item",
            Self::Quote => "a quote",
            Self::Panel => "a panel",
            Self::Cell => "a table cell",
            Self::Expand => "an expand",
            Self::NestedExpand => "a nested expand",
        }
    }

    fn allows_heading_rule(self) -> bool {
        !matches!(self, Self::ListItem | Self::Quote)
    }

    fn allows_quote(self) -> bool {
        matches!(
            self,
            Self::Doc | Self::Cell | Self::Expand | Self::NestedExpand
        )
    }

    fn allows_panel(self) -> bool {
        self.allows_quote()
    }

    fn allows_table(self) -> bool {
        matches!(self, Self::Doc | Self::Expand)
    }

    fn allows_task_list(self) -> bool {
        self != Self::Quote
    }

    /// Whether a paragraph of only attachment links becomes a `mediaGroup` (the schema allows it
    /// everywhere except in a list item; there the links stay inline).
    fn allows_media_group(self) -> bool {
        self != Self::ListItem
    }

    fn raw_place(self) -> RawPlace {
        match self {
            Self::Doc => RawPlace::Doc,
            Self::ListItem => RawPlace::ListItem,
            Self::Quote => RawPlace::Blockquote,
            Self::Panel => RawPlace::Panel,
            Self::Cell => RawPlace::TableCell,
            Self::Expand => RawPlace::Expand,
            Self::NestedExpand => RawPlace::NestedExpand,
        }
    }
}

/// A lowered block, not yet converted to a container's child enum.
#[derive(Debug)]
enum Node {
    Para(Paragraph),
    Heading(Heading),
    Bullet(BulletList),
    Ordered(OrderedList),
    Task(TaskList),
    Code(CodeBlock),
    Quote(Blockquote),
    Rule,
    Panel(Panel),
    Table(Table),
    Expand(Expand),
    NestedExpand(NestedExpand),
    Media(MediaSingle),
    /// A paragraph of attachment links (`adf:attachment`).
    Attachments(MediaGroup),
    Raw(RawNode),
}

impl Node {
    fn name(&self) -> &'static str {
        match self {
            Self::Para(_) => "paragraph",
            Self::Heading(_) => "heading",
            Self::Bullet(_) => "bulletList",
            Self::Ordered(_) => "orderedList",
            Self::Task(_) => "taskList",
            Self::Code(_) => "codeBlock",
            Self::Quote(_) => "blockquote",
            Self::Rule => "rule",
            Self::Panel(_) => "panel",
            Self::Table(_) => "table",
            Self::Expand(_) => "expand",
            Self::NestedExpand(_) => "nestedExpand",
            Self::Media(_) => "mediaSingle",
            Self::Attachments(_) => "mediaGroup",
            Self::Raw(_) => "raw ADF",
        }
    }
}

/// A table from an included CSV file, with the position of its image.
type IncludedTable = (Box<Table>, Span);

/// What a leaf's inline content holds besides text, in document order.
#[derive(Debug)]
enum Embedded {
    Media(MediaSingle),
    Table(Box<Table>, Span),
}

/// Output of lowering a block in some container.
#[derive(Debug)]
enum Piece {
    /// A node valid in that container.
    Node(Node),
    /// A block the container cannot hold; its parent must place it.
    Up(Block),
}

fn misplaced(node: &Node, ctx: &str) -> Diagnostic {
    Diagnostic::new(
        Code::Internal,
        format!(
            "Markdown lowering placed a {} in {ctx}; please report this input",
            node.name()
        ),
    )
}

fn doc_child(node: Node) -> Result<DocBlock, Diagnostic> {
    Ok(match node {
        Node::Para(p) => DocBlock::Paragraph(p.into()),
        Node::Heading(h) => DocBlock::Heading(h),
        Node::Bullet(l) => DocBlock::BulletList(l),
        Node::Ordered(l) => DocBlock::OrderedList(l),
        Node::Task(l) => DocBlock::TaskList(l),
        Node::Code(c) => DocBlock::CodeBlock(c),
        Node::Quote(q) => DocBlock::Blockquote(q),
        Node::Rule => DocBlock::Rule(Rule),
        Node::Panel(p) => DocBlock::Panel(p),
        Node::Table(t) => DocBlock::Table(t),
        Node::Expand(e) => DocBlock::Expand(e),
        Node::Media(m) => DocBlock::MediaSingle(m),
        Node::Attachments(g) => DocBlock::MediaGroup(g),
        Node::Raw(r) => DocBlock::Raw(r),
        n @ Node::NestedExpand(_) => return Err(misplaced(&n, "the document")),
    })
}

fn list_item_child(node: Node) -> Result<ListItemChild, Diagnostic> {
    Ok(match node {
        Node::Para(p) => ListItemChild::Paragraph(p),
        Node::Bullet(l) => ListItemChild::BulletList(l),
        Node::Ordered(l) => ListItemChild::OrderedList(l),
        Node::Task(l) => ListItemChild::TaskList(l),
        Node::Code(c) => ListItemChild::CodeBlock(c),
        Node::Media(m) => ListItemChild::MediaSingle(m),
        Node::Raw(r) => ListItemChild::Raw(r),
        n @ (Node::Heading(_)
        | Node::Quote(_)
        | Node::Rule
        | Node::Panel(_)
        | Node::Table(_)
        | Node::Expand(_)
        | Node::NestedExpand(_)
        | Node::Attachments(_)) => return Err(misplaced(&n, "a list item")),
    })
}

fn quote_child(node: Node) -> Result<BlockquoteChild, Diagnostic> {
    Ok(match node {
        Node::Para(p) => BlockquoteChild::Paragraph(p),
        Node::Bullet(l) => BlockquoteChild::BulletList(l),
        Node::Ordered(l) => BlockquoteChild::OrderedList(l),
        Node::Code(c) => BlockquoteChild::CodeBlock(c),
        Node::Media(m) => BlockquoteChild::MediaSingle(m),
        Node::Attachments(g) => BlockquoteChild::MediaGroup(g),
        Node::Raw(r) => BlockquoteChild::Raw(r),
        n @ (Node::Heading(_)
        | Node::Task(_)
        | Node::Quote(_)
        | Node::Rule
        | Node::Panel(_)
        | Node::Table(_)
        | Node::Expand(_)
        | Node::NestedExpand(_)) => return Err(misplaced(&n, "a quote")),
    })
}

fn panel_child(node: Node) -> Result<PanelChild, Diagnostic> {
    Ok(match node {
        Node::Para(p) => PanelChild::Paragraph(p),
        Node::Heading(h) => PanelChild::Heading(h),
        Node::Bullet(l) => PanelChild::BulletList(l),
        Node::Ordered(l) => PanelChild::OrderedList(l),
        Node::Task(l) => PanelChild::TaskList(l),
        Node::Code(c) => PanelChild::CodeBlock(c),
        Node::Rule => PanelChild::Rule(Rule),
        Node::Media(m) => PanelChild::MediaSingle(m),
        Node::Attachments(g) => PanelChild::MediaGroup(g),
        Node::Raw(r) => PanelChild::Raw(r),
        n @ (Node::Quote(_)
        | Node::Panel(_)
        | Node::Table(_)
        | Node::Expand(_)
        | Node::NestedExpand(_)) => return Err(misplaced(&n, "a panel")),
    })
}

fn cell_child(node: Node) -> Result<TableCellChild, Diagnostic> {
    Ok(match node {
        Node::Para(p) => TableCellChild::Paragraph(p.into()),
        Node::Heading(h) => TableCellChild::Heading(h),
        Node::Bullet(l) => TableCellChild::BulletList(l),
        Node::Ordered(l) => TableCellChild::OrderedList(l),
        Node::Task(l) => TableCellChild::TaskList(l),
        Node::Code(c) => TableCellChild::CodeBlock(c),
        Node::Quote(q) => TableCellChild::Blockquote(q),
        Node::Panel(p) => TableCellChild::Panel(p),
        Node::Rule => TableCellChild::Rule(Rule),
        Node::NestedExpand(e) => TableCellChild::NestedExpand(e),
        Node::Media(m) => TableCellChild::MediaSingle(m),
        Node::Attachments(g) => TableCellChild::MediaGroup(g),
        Node::Raw(r) => TableCellChild::Raw(r),
        n @ (Node::Table(_) | Node::Expand(_)) => return Err(misplaced(&n, "a table cell")),
    })
}

fn expand_child(node: Node) -> Result<ExpandChild, Diagnostic> {
    Ok(match node {
        Node::Para(p) => ExpandChild::Paragraph(p),
        Node::Heading(h) => ExpandChild::Heading(h),
        Node::Bullet(l) => ExpandChild::BulletList(l),
        Node::Ordered(l) => ExpandChild::OrderedList(l),
        Node::Task(l) => ExpandChild::TaskList(l),
        Node::Code(c) => ExpandChild::CodeBlock(c),
        Node::Quote(q) => ExpandChild::Blockquote(q),
        Node::Panel(p) => ExpandChild::Panel(p),
        Node::Rule => ExpandChild::Rule(Rule),
        Node::Table(t) => ExpandChild::Table(t),
        Node::NestedExpand(e) => ExpandChild::NestedExpand(e),
        Node::Media(m) => ExpandChild::MediaSingle(m),
        Node::Attachments(g) => ExpandChild::MediaGroup(g),
        Node::Raw(r) => ExpandChild::Raw(r),
        n @ Node::Expand(_) => return Err(misplaced(&n, "an expand")),
    })
}

fn nested_expand_child(node: Node) -> Result<NestedExpandChild, Diagnostic> {
    Ok(match node {
        Node::Para(p) => NestedExpandChild::Paragraph(p),
        Node::Heading(h) => NestedExpandChild::Heading(h),
        Node::Bullet(l) => NestedExpandChild::BulletList(l),
        Node::Ordered(l) => NestedExpandChild::OrderedList(l),
        Node::Task(l) => NestedExpandChild::TaskList(l),
        Node::Code(c) => NestedExpandChild::CodeBlock(c),
        Node::Quote(q) => NestedExpandChild::Blockquote(q),
        Node::Panel(p) => NestedExpandChild::Panel(p),
        Node::Rule => NestedExpandChild::Rule(Rule),
        Node::Media(m) => NestedExpandChild::MediaSingle(m),
        Node::Attachments(g) => NestedExpandChild::MediaGroup(g),
        Node::Raw(r) => NestedExpandChild::Raw(r),
        n @ (Node::Table(_) | Node::Expand(_) | Node::NestedExpand(_)) => {
            return Err(misplaced(&n, "a nested expand"));
        }
    })
}

fn non_empty<T>(items: Vec<T>) -> Result<NonEmpty<T>, Diagnostic> {
    NonEmpty::from_vec(items).ok_or_else(|| {
        Diagnostic::new(
            Code::Internal,
            "Markdown lowering built an empty container; please report this input",
        )
    })
}

fn convert_all<T>(
    nodes: Vec<Node>,
    f: impl Fn(Node) -> Result<T, Diagnostic>,
) -> Result<Vec<T>, Diagnostic> {
    nodes.into_iter().map(f).collect()
}

/// A list item from lowered nodes: a leading paragraph (inserted if needed) plus the rest.
fn make_item(nodes: Vec<Node>) -> Result<ListItem, Diagnostic> {
    let mut nodes = nodes.into_iter();
    let mut item = ListItem::default();
    let mut rest = Vec::new();
    match nodes.next() {
        Some(Node::Para(p)) => item.first = p,
        Some(other) => rest.push(list_item_child(other)?),
        None => {}
    }
    for node in nodes {
        rest.push(list_item_child(node)?);
    }
    item.rest = rest;
    Ok(item)
}

fn make_list(order: Option<u32>, items: Vec<ListItem>) -> Result<Node, Diagnostic> {
    let items = non_empty(items)?;
    Ok(match order {
        Some(order) => Node::Ordered(OrderedList { order, items }),
        None => Node::Bullet(BulletList { items }),
    })
}

/// A task list being assembled.
#[derive(Debug)]
struct TaskListBuilder {
    local_id: String,
    first: Option<TaskItem>,
    rest: Vec<TaskListChild>,
}

impl TaskListBuilder {
    fn push_item(&mut self, item: TaskItem) {
        if self.first.is_none() {
            self.first = Some(item);
        } else {
            self.rest.push(TaskListChild::TaskItem(item));
        }
    }

    fn finish(self) -> Option<TaskList> {
        Some(TaskList {
            local_id: self.local_id,
            first: self.first?,
            rest: self.rest,
        })
    }
}

/// Output of lowering a run of task items.
#[derive(Debug)]
enum TaskPart {
    List(TaskList),
    Up(Block),
}

struct Lowerer<'c, 'a> {
    cx: &'c mut Cx<'a>,
    /// `localId`s used by ```` ```adf ```` blocks; generated IDs avoid them.
    raw_ids: BTreeSet<String>,
}

/// Lowers a parsed document.
pub(crate) fn lower(parsed: Parsed, cx: &mut Cx<'_>) -> Result<Doc, Diagnostic> {
    let raw_ids = raw_local_ids(&parsed, cx)?;
    let mut lowerer = Lowerer { cx, raw_ids };
    let mut content = Vec::new();
    for piece in lowerer.seq(Ctx::Doc, parsed.blocks)? {
        content.extend(lowerer.place_in_doc(piece)?);
    }
    if !parsed.footnotes.is_empty() {
        for piece in lowerer.footnotes(parsed.footnotes)? {
            content.extend(lowerer.place_in_doc(piece)?);
        }
    }
    Ok(Doc { content })
}

/// Node types whose `localId` must be unique in the document (as checked by
/// `adf::validate::invariants`).
const LOCAL_ID_NODES: &[&str] = &[
    "taskList",
    "taskItem",
    "blockTaskItem",
    "decisionList",
    "decisionItem",
];

/// The `localId`s of the ```` ```adf ```` blocks of the document. Each block is validated on its
/// own when it is lowered, so an ID that two blocks share is reported here, at the later block
/// (`E_RAW_ADF`).
fn raw_local_ids(parsed: &Parsed, cx: &Cx<'_>) -> Result<BTreeSet<String>, Diagnostic> {
    let mut raws: Vec<(&Span, &[Value])> = Vec::new();
    let mut blocks: Vec<&Block> = parsed.blocks.iter().collect();
    blocks.extend(parsed.footnotes.iter().flat_map(|f| f.children.iter()));
    while let Some(block) = blocks.pop() {
        match block {
            Block::Raw { nodes, span } => raws.push((span, nodes)),
            Block::List { items, .. } => {
                blocks.extend(items.iter().flat_map(|item| item.children.iter()));
            }
            Block::Quote { children, .. }
            | Block::Alert { children, .. }
            | Block::Box { children, .. }
            | Block::Details { children, .. } => blocks.extend(children.iter()),
            Block::Paragraph { .. }
            | Block::Heading { .. }
            | Block::Code { .. }
            | Block::Rule { .. }
            | Block::Table { .. }
            | Block::Media { .. }
            | Block::Csv { .. } => {}
        }
    }
    raws.sort_by_key(|(span, _)| span.start);
    let mut ids = BTreeSet::new();
    for (span, nodes) in raws {
        let mut own = BTreeSet::new();
        let mut values: Vec<&Value> = nodes.iter().collect();
        while let Some(value) = values.pop() {
            match value {
                Value::Object(map) => {
                    let typed = map
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|ty| LOCAL_ID_NODES.contains(&ty));
                    if typed
                        && let Some(id) = map
                            .get("attrs")
                            .and_then(|attrs| attrs.get("localId"))
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                    {
                        own.insert(id);
                    }
                    values.extend(map.values());
                }
                Value::Array(items) => values.extend(items.iter()),
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
        for id in own {
            if !ids.insert(id.to_owned()) {
                return Err(cx
                    .diag(
                        Code::RawAdf,
                        format!(
                            "```adf block is invalid: its `localId` `{}` is already used by an earlier ```adf block",
                            truncate(id, 40)
                        ),
                        span,
                    )
                    .with_hint("Give every task and decision node a unique, non-empty localId"));
            }
        }
    }
    Ok(ids)
}

/// `noun` with "A" or "An" before it, for the start of a message.
fn with_article(noun: &str) -> String {
    let article = if noun.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "An"
    } else {
        "A"
    };
    format!("{article} {noun}")
}

impl Lowerer<'_, '_> {
    /// A fresh task `localId`, never one a ```` ```adf ```` block uses.
    fn next_local_id(&mut self) -> String {
        loop {
            let id = self.cx.ids.next_id();
            if !self.raw_ids.contains(&id) {
                return id;
            }
        }
    }

    fn place_in_doc(&mut self, piece: Piece) -> Result<Vec<DocBlock>, Diagnostic> {
        match piece {
            Piece::Node(node) => Ok(vec![doc_child(node)?]),
            // The document holds everything; re-lowering here always places the block.
            Piece::Up(block) => {
                let mut out = Vec::new();
                for piece in self.block(Ctx::Doc, block, true)? {
                    match piece {
                        Piece::Node(node) => out.push(doc_child(node)?),
                        Piece::Up(block) => {
                            return Err(Diagnostic::new(
                                Code::Internal,
                                format!(
                                    "Markdown lowering could not place a {} in the document",
                                    block.describe()
                                ),
                            ));
                        }
                    }
                }
                Ok(out)
            }
        }
    }

    fn seq(&mut self, ctx: Ctx, blocks: Vec<Block>) -> Result<Vec<Piece>, Diagnostic> {
        let mut out = Vec::new();
        for block in blocks {
            out.extend(self.block(ctx, block, false)?);
        }
        Ok(out)
    }

    /// Hoists `block` out of `ctx`. `arrived` blocks were already hoisted from a descendant and
    /// have been reported.
    fn hoist(&mut self, ctx: Ctx, block: Block, arrived: bool) -> Vec<Piece> {
        if !arrived {
            self.cx.warn(
                Code::BlockHoisted,
                format!(
                    "{} is not allowed in {}; it was moved out of it",
                    with_article(block.describe()),
                    ctx.describe()
                ),
                &block.span(),
            );
        }
        vec![Piece::Up(block)]
    }

    #[allow(clippy::too_many_lines)]
    fn block(&mut self, ctx: Ctx, block: Block, arrived: bool) -> Result<Vec<Piece>, Diagnostic> {
        match block {
            Block::Paragraph { content, .. } => Ok(self.paragraph(ctx, content)),
            Block::Heading {
                level,
                mut content,
                span,
            } => {
                if ctx.allows_heading_rule() {
                    return Ok(self.heading(ctx, level, content));
                }
                self.cx.warn(
                    Code::HeadingDemoted,
                    format!(
                        "A heading is not allowed in {}; it became a bold paragraph",
                        ctx.describe()
                    ),
                    &span,
                );
                embolden(&mut content);
                Ok(self.paragraph(ctx, content))
            }
            Block::Code { language, text, .. } => {
                Ok(vec![Piece::Node(Node::Code(CodeBlock { language, text }))])
            }
            block @ Block::Rule { .. } => {
                if ctx.allows_heading_rule() {
                    Ok(vec![Piece::Node(Node::Rule)])
                } else {
                    Ok(self.hoist(ctx, block, arrived))
                }
            }
            Block::List { start, items, span } => self.list(ctx, start, items, &span, arrived),
            Block::Quote { children, span } => match ctx {
                Ctx::Doc | Ctx::Cell | Ctx::Expand | Ctx::NestedExpand => self.quote(ctx, children),
                Ctx::Quote => {
                    self.cx.warn(
                        Code::BlockFlattened,
                        "A quote inside a quote was merged into the outer quote",
                        &span,
                    );
                    self.seq(Ctx::Quote, children)
                }
                Ctx::ListItem | Ctx::Panel => {
                    Ok(self.hoist(ctx, Block::Quote { children, span }, arrived))
                }
            },
            Block::Alert {
                panel,
                children,
                span,
            } => {
                if ctx.allows_panel() {
                    return self.panel(ctx, panel, children);
                }
                if ctx != Ctx::Quote {
                    // A list item or a panel: the alert is hoisted as it is, and the container
                    // that finally holds it decides (a panel in the document, an expand or a
                    // cell; a plain quote's content in a quote).
                    let block = Block::Alert {
                        panel,
                        children,
                        span,
                    };
                    return Ok(self.hoist(ctx, block, arrived));
                }
                // Reported even for hoisted blocks: the conversion is a separate step.
                self.cx.warn(
                    Code::BlockFlattened,
                    "An alert cannot be a panel in a quote; it was merged into the quote",
                    &span,
                );
                self.seq(Ctx::Quote, children)
            }
            block @ (Block::Table { .. } | Block::Box { .. } | Block::Csv { .. })
                if !ctx.allows_table() =>
            {
                Ok(self.hoist(ctx, block, arrived))
            }
            Block::Table { aligns, rows, .. } => {
                let (table, extra) = self.gfm_table(&aligns, rows)?;
                let mut out = vec![Piece::Node(table)];
                for (included, span) in extra {
                    self.cx.warn(
                        Code::BlockHoisted,
                        "A table is not allowed in a table cell; it was moved after the table",
                        &span,
                    );
                    out.push(Piece::Node(Node::Table(*included)));
                }
                Ok(out)
            }
            Block::Csv { table, .. } => Ok(vec![Piece::Node(Node::Table(*table))]),
            Block::Box { children, .. } => self.boxed(ctx, children),
            Block::Details {
                title,
                children,
                span,
            } => match ctx {
                Ctx::Doc => self.expand(ctx, title.as_deref(), children),
                Ctx::Expand | Ctx::Cell => self.nested_expand(ctx, title.as_deref(), children),
                Ctx::ListItem | Ctx::Quote | Ctx::Panel | Ctx::NestedExpand => {
                    self.cx.warn(
                        Code::BlockFlattened,
                        format!(
                            "A <details> block cannot be an expand in {}; its title became a bold paragraph followed by its content",
                            ctx.describe()
                        ),
                        &span,
                    );
                    let mut blocks = Vec::with_capacity(children.len() + 1);
                    if let Some(title) = title {
                        blocks.push(Block::Paragraph {
                            content: vec![Tok::Text {
                                text: title,
                                marks: Marks {
                                    fmt: Formatting {
                                        strong: true,
                                        ..Formatting::default()
                                    },
                                    code: false,
                                },
                                span: span.clone(),
                                trim: true,
                            }],
                            span: span.clone(),
                        });
                    }
                    blocks.extend(children);
                    self.seq(ctx, blocks)
                }
            },
            Block::Raw { nodes, span } => {
                if let Err(message) = validate_raw(&nodes, ctx.raw_place(), self.cx.opts.target) {
                    return Err(self.cx.diag(
                        Code::RawAdf,
                        format!("```adf block is {message}"),
                        &span,
                    ));
                }
                Ok(nodes
                    .into_iter()
                    .map(|n| Piece::Node(Node::Raw(RawNode(n))))
                    .collect())
            }
            Block::Media { media, .. } => Ok(vec![Piece::Node(Node::Media(media))]),
        }
    }

    /// A paragraph, split around external images and included tables.
    fn paragraph(&mut self, ctx: Ctx, content: Vec<Tok>) -> Vec<Piece> {
        if ctx.allows_media_group()
            && let Some(group) = attachment_group(&content)
        {
            return vec![Piece::Node(Node::Attachments(group))];
        }
        let mut out = Vec::new();
        for segment in split_media(content) {
            match segment {
                Segment::Inline(toks) => {
                    let content = finalize(toks, self.cx);
                    if !content.is_empty() {
                        out.push(Piece::Node(Node::Para(Paragraph { content })));
                    }
                }
                Segment::Media(media) => out.push(Piece::Node(Node::Media(media))),
                Segment::Table(table, span) => out.extend(self.included_table(ctx, table, span)),
            }
        }
        out
    }

    /// A table from an included CSV file, placed like a GFM table.
    fn included_table(&mut self, ctx: Ctx, table: Box<Table>, span: Span) -> Vec<Piece> {
        if ctx.allows_table() {
            vec![Piece::Node(Node::Table(*table))]
        } else {
            self.hoist(ctx, Block::Csv { table, span }, false)
        }
    }

    /// Inline content of a non-paragraph leaf: text plus the external images and included
    /// tables found in it.
    fn leaf_inline(&mut self, content: Vec<Tok>) -> (Vec<Inline>, Vec<Embedded>) {
        let mut toks = Vec::new();
        let mut media = Vec::new();
        let mut after_media = false;
        for segment in split_media(content) {
            match segment {
                Segment::Inline(mut t) => {
                    // The text on both sides of a removed image joins: keep one space.
                    if after_media && ends_with_space(&toks) {
                        trim_leading_space(&mut t);
                    }
                    toks.extend(t);
                    after_media = false;
                }
                Segment::Media(m) => {
                    media.push(Embedded::Media(m));
                    after_media = true;
                }
                Segment::Table(table, span) => {
                    media.push(Embedded::Table(table, span));
                    after_media = true;
                }
            }
        }
        (finalize(toks, self.cx), media)
    }

    fn heading(&mut self, ctx: Ctx, level: HeadingLevel, content: Vec<Tok>) -> Vec<Piece> {
        let (content, embedded) = self.leaf_inline(content);
        let mut out = vec![Piece::Node(Node::Heading(Heading { level, content }))];
        for item in embedded {
            match item {
                Embedded::Media(m) => out.push(Piece::Node(Node::Media(m))),
                Embedded::Table(table, span) => out.extend(self.included_table(ctx, table, span)),
            }
        }
        out
    }

    /// Groups the pieces of a container's children into container nodes, re-lowering hoisted
    /// blocks in `parent`. A container with no content at all holds one empty paragraph. A
    /// container whose content was entirely hoisted disappears, unless `keep` is set (panels
    /// and expands, whose type or title is content): then it stays, empty, before the
    /// hoisted blocks.
    fn assemble(
        &mut self,
        parent: Ctx,
        pieces: Vec<Piece>,
        keep: bool,
        mut wrap: impl FnMut(Vec<Node>) -> Result<Node, Diagnostic>,
    ) -> Result<Vec<Piece>, Diagnostic> {
        let mut out = Vec::new();
        let mut current = Vec::new();
        let mut any_up = false;
        let mut wrapped = false;
        for piece in pieces {
            match piece {
                Piece::Node(node) => current.push(node),
                Piece::Up(block) => {
                    any_up = true;
                    if !current.is_empty() {
                        out.push(Piece::Node(wrap(std::mem::take(&mut current))?));
                        wrapped = true;
                    }
                    out.extend(self.block(parent, block, true)?);
                }
            }
        }
        if !current.is_empty() {
            out.push(Piece::Node(wrap(current)?));
        } else if !any_up || (keep && !wrapped) {
            let empty = wrap(vec![Node::Para(Paragraph::default())])?;
            out.insert(0, Piece::Node(empty));
        }
        Ok(out)
    }

    fn quote(&mut self, ctx: Ctx, children: Vec<Block>) -> Result<Vec<Piece>, Diagnostic> {
        let pieces = self.seq(Ctx::Quote, children)?;
        self.assemble(ctx, pieces, false, |nodes| {
            Ok(Node::Quote(Blockquote {
                content: non_empty(convert_all(nodes, quote_child)?)?,
            }))
        })
    }

    fn panel(
        &mut self,
        ctx: Ctx,
        panel_type: PanelType,
        children: Vec<Block>,
    ) -> Result<Vec<Piece>, Diagnostic> {
        let pieces = self.seq(Ctx::Panel, children)?;
        self.assemble(ctx, pieces, true, |nodes| {
            Ok(Node::Panel(Panel {
                panel_type,
                content: non_empty(convert_all(nodes, panel_child)?)?,
            }))
        })
    }

    fn boxed(&mut self, ctx: Ctx, children: Vec<Block>) -> Result<Vec<Piece>, Diagnostic> {
        let pieces = self.seq(Ctx::Cell, children)?;
        self.assemble(ctx, pieces, false, |nodes| {
            let cell = TableCell {
                kind: CellKind::Data,
                content: non_empty(convert_all(nodes, cell_child)?)?,
            };
            Ok(Node::Table(Table {
                rows: NonEmpty::new(TableRow {
                    cells: NonEmpty::new(cell),
                }),
            }))
        })
    }

    fn expand(
        &mut self,
        ctx: Ctx,
        title: Option<&str>,
        children: Vec<Block>,
    ) -> Result<Vec<Piece>, Diagnostic> {
        let pieces = self.seq(Ctx::Expand, children)?;
        self.assemble(ctx, pieces, true, |nodes| {
            Ok(Node::Expand(Expand {
                title: title.map(str::to_owned),
                content: non_empty(convert_all(nodes, expand_child)?)?,
            }))
        })
    }

    fn nested_expand(
        &mut self,
        ctx: Ctx,
        title: Option<&str>,
        children: Vec<Block>,
    ) -> Result<Vec<Piece>, Diagnostic> {
        let pieces = self.seq(Ctx::NestedExpand, children)?;
        self.assemble(ctx, pieces, true, |nodes| {
            Ok(Node::NestedExpand(NestedExpand {
                title: title.map(str::to_owned),
                content: non_empty(convert_all(nodes, nested_expand_child)?)?,
            }))
        })
    }

    fn gfm_table(
        &mut self,
        aligns: &[Option<Align>],
        rows: Vec<Row>,
    ) -> Result<(Node, Vec<IncludedTable>), Diagnostic> {
        let mut extra = Vec::new();
        let width = aligns.len().max(1);
        let mut out_rows = Vec::with_capacity(rows.len());
        for row in rows {
            let kind = if row.header {
                CellKind::Header
            } else {
                CellKind::Data
            };
            let mut cells = Vec::with_capacity(width);
            let mut source = row.cells.into_iter();
            for column in 0..width {
                let align = aligns.get(column).copied().flatten();
                let (content, embedded) = match source.next() {
                    Some(toks) => self.leaf_inline(toks),
                    None => (Vec::new(), Vec::new()),
                };
                let mut children = vec![TableCellChild::Paragraph(AlignedParagraph {
                    content,
                    align,
                })];
                for item in embedded {
                    match item {
                        Embedded::Media(m) => children.push(TableCellChild::MediaSingle(m)),
                        Embedded::Table(table, span) => extra.push((table, span)),
                    }
                }
                cells.push(TableCell {
                    kind,
                    content: non_empty(children)?,
                });
            }
            out_rows.push(TableRow {
                cells: non_empty(cells)?,
            });
        }
        let table = Node::Table(Table {
            rows: non_empty(out_rows)?,
        });
        Ok((table, extra))
    }

    fn list(
        &mut self,
        ctx: Ctx,
        start: Option<u32>,
        items: Vec<Item>,
        span: &Span,
        arrived: bool,
    ) -> Result<Vec<Piece>, Diagnostic> {
        self.warn_task_numbering(start, &items);
        let mut out = Vec::new();
        let mut number = start;
        for (is_task, run) in split_runs(items) {
            let len = u32::try_from(run.len()).unwrap_or(u32::MAX);
            if is_task {
                if ctx.allows_task_list() {
                    for part in self.task_parts(run)? {
                        match part {
                            TaskPart::List(list) => out.push(Piece::Node(Node::Task(list))),
                            TaskPart::Up(block) => out.extend(self.block(ctx, block, true)?),
                        }
                    }
                } else {
                    let block = Block::List {
                        start: None,
                        items: run,
                        span: span.clone(),
                    };
                    out.extend(self.hoist(ctx, block, arrived));
                }
            } else {
                out.extend(self.plain_run(ctx, number, run)?);
            }
            number = number.map(|n| n.saturating_add(len));
        }
        Ok(out)
    }

    /// Lowers consecutive non-task items into bullet/ordered list fragments, splitting around
    /// hoisted blocks. A continuation fragment keeps the number of the item it continues.
    fn plain_run(
        &mut self,
        ctx: Ctx,
        start: Option<u32>,
        items: Vec<Item>,
    ) -> Result<Vec<Piece>, Diagnostic> {
        let mut out = Vec::new();
        let mut list_items: Vec<ListItem> = Vec::new();
        let mut list_order = start;
        let mut number = start;
        for item in items {
            let pieces = self.seq(Ctx::ListItem, item.children)?;
            let mut current: Vec<Node> = Vec::new();
            let mut any_up = false;
            for piece in pieces {
                match piece {
                    Piece::Node(node) => current.push(node),
                    Piece::Up(block) => {
                        any_up = true;
                        if !current.is_empty() {
                            if list_items.is_empty() {
                                list_order = number;
                            }
                            list_items.push(make_item(std::mem::take(&mut current))?);
                        }
                        if !list_items.is_empty() {
                            let fragment = make_list(list_order, std::mem::take(&mut list_items))?;
                            out.push(Piece::Node(fragment));
                        }
                        out.extend(self.block(ctx, block, true)?);
                    }
                }
            }
            // An item with no content at all is an empty item; one whose remaining content was
            // hoisted ends with the hoisted block.
            if !current.is_empty() || !any_up {
                if list_items.is_empty() {
                    list_order = number;
                }
                list_items.push(make_item(current)?);
            }
            number = number.map(|n| n.saturating_add(1));
        }
        if !list_items.is_empty() {
            out.push(Piece::Node(make_list(list_order, list_items)?));
        }
        Ok(out)
    }

    /// Lowers consecutive task items into task list fragments. Blocks a task item cannot hold
    /// close the task list after that item and follow it.
    fn task_parts(&mut self, items: Vec<Item>) -> Result<Vec<TaskPart>, Diagnostic> {
        let mut out = Vec::new();
        let mut current: Option<TaskListBuilder> = None;
        for item in items {
            if current.is_none() {
                current = Some(TaskListBuilder {
                    local_id: self.next_local_id(),
                    first: None,
                    rest: Vec::new(),
                });
            }
            let (task, nested, ups) = self.task_item(item)?;
            if let Some(builder) = current.as_mut() {
                builder.push_item(task);
                builder
                    .rest
                    .extend(nested.into_iter().map(TaskListChild::TaskList));
            }
            if !ups.is_empty() {
                if let Some(list) = current.take().and_then(TaskListBuilder::finish) {
                    out.push(TaskPart::List(list));
                }
                out.extend(ups.into_iter().map(TaskPart::Up));
            }
        }
        if let Some(list) = current.and_then(TaskListBuilder::finish) {
            out.push(TaskPart::List(list));
        }
        Ok(out)
    }

    /// A task item: its paragraphs joined with hard breaks, its nested task lists, and the
    /// blocks it cannot hold (hoisted out of the task list).
    fn task_item(
        &mut self,
        item: Item,
    ) -> Result<(TaskItem, Vec<TaskList>, Vec<Block>), Diagnostic> {
        let local_id = self.next_local_id();
        let state = item.task.unwrap_or(TaskState::Todo);
        let mut toks: Vec<Tok> = Vec::new();
        let mut nested = Vec::new();
        let mut ups = Vec::new();
        for child in item.children {
            match child {
                Block::Paragraph { content, .. } => self.task_text(&mut toks, content, &mut ups),
                Block::Heading {
                    mut content, span, ..
                } => {
                    self.cx.warn(
                        Code::HeadingDemoted,
                        "A heading is not allowed in a task item; it became bold text",
                        &span,
                    );
                    embolden(&mut content);
                    self.task_text(&mut toks, content, &mut ups);
                }
                Block::List { start, items, span } => {
                    self.warn_task_numbering(start, &items);
                    let mut number = start;
                    for (is_task, run) in split_runs(items) {
                        let len = u32::try_from(run.len()).unwrap_or(u32::MAX);
                        if is_task {
                            for part in self.task_parts(run)? {
                                match part {
                                    TaskPart::List(list) => nested.push(list),
                                    TaskPart::Up(block) => ups.push(block),
                                }
                            }
                        } else {
                            let block = Block::List {
                                start: number,
                                items: run,
                                span: span.clone(),
                            };
                            self.cx.warn(
                                Code::BlockHoisted,
                                "A plain list is not allowed in a task item; it was moved out of the task list",
                                &span,
                            );
                            ups.push(block);
                        }
                        number = number.map(|n| n.saturating_add(len));
                    }
                }
                other @ (Block::Code { .. }
                | Block::Rule { .. }
                | Block::Quote { .. }
                | Block::Alert { .. }
                | Block::Table { .. }
                | Block::Csv { .. }
                | Block::Box { .. }
                | Block::Details { .. }
                | Block::Raw { .. }
                | Block::Media { .. }) => {
                    self.cx.warn(
                        Code::BlockHoisted,
                        format!(
                            "{} is not allowed in a task item; it was moved out of the task list",
                            with_article(other.describe())
                        ),
                        &other.span(),
                    );
                    ups.push(other);
                }
            }
        }
        let content = finalize(toks, self.cx);
        Ok((
            TaskItem {
                local_id,
                state,
                content,
            },
            nested,
            ups,
        ))
    }

    /// Appends a paragraph's text to a task item's content (separated by a hard break).
    ///
    /// External images are moved out of the task list. The spaces around a moved image collapse
    /// to one (`task ![i](u) after` → `task after`), or to none at the edge of the text.
    fn task_text(&mut self, toks: &mut Vec<Tok>, content: Vec<Tok>, ups: &mut Vec<Block>) {
        let mut text = Vec::new();
        let mut after_media = false;
        for tok in content {
            match tok {
                Tok::Table { table, span } => {
                    self.cx.warn(
                        Code::BlockHoisted,
                        "A table is not allowed in a task item; it was moved out of the task list",
                        &span,
                    );
                    ups.push(Block::Csv { table, span });
                    after_media = true;
                }
                Tok::Media { media, span } => {
                    self.cx.warn(
                        Code::BlockHoisted,
                        "An image is not allowed in a task item; it was moved out of the task list",
                        &span,
                    );
                    ups.push(Block::Media { media, span });
                    after_media = true;
                }
                Tok::Text {
                    text: value,
                    marks,
                    span,
                    trim: true,
                } if after_media && ends_in_space(&text) => {
                    let value = value.trim_start_matches([' ', '\t']);
                    if !value.is_empty() {
                        text.push(Tok::Text {
                            text: value.to_owned(),
                            marks,
                            span,
                            trim: true,
                        });
                        after_media = false;
                    }
                }
                other @ (Tok::Text { .. } | Tok::HardBreak | Tok::Node(_)) => {
                    text.push(other);
                    after_media = false;
                }
            }
        }
        if after_media {
            trim_end_spaces(&mut text);
        }
        if text.is_empty() {
            return;
        }
        if !toks.is_empty() {
            toks.push(Tok::HardBreak);
        }
        toks.extend(text);
    }

    /// Footnote definitions: a rule and a numbered list at the end of the document.
    fn footnotes(&mut self, footnotes: Vec<Footnote>) -> Result<Vec<Piece>, Diagnostic> {
        let mut items = Vec::with_capacity(footnotes.len());
        for footnote in footnotes {
            self.cx.warn(
                Code::FootnoteFlattened,
                "Footnotes became superscript numbers plus a numbered list at the end; ADF has no footnotes",
                &footnote.span,
            );
            items.push(Item {
                task: None,
                children: footnote.children,
                span: footnote.span,
            });
        }
        let mut out = vec![Piece::Node(Node::Rule)];
        out.extend(self.plain_run(Ctx::Doc, Some(1), items)?);
        Ok(out)
    }
}

impl Lowerer<'_, '_> {
    /// `W_TASK_NUMBERING_LOST` for an ordered list with task items.
    fn warn_task_numbering(&mut self, start: Option<u32>, items: &[Item]) {
        if start.is_some()
            && let Some(first_task) = items.iter().find(|i| i.task.is_some())
        {
            self.cx.warn(
                Code::TaskNumberingLost,
                "An ordered task list became an unnumbered ADF task list",
                &first_task.span,
            );
        }
    }
}

/// White space between two attachment links: a line break, or unlinked text of only spaces and
/// tabs.
fn is_link_separator(tok: &Tok) -> bool {
    match tok {
        Tok::HardBreak => true,
        Tok::Text { text, marks, .. } => {
            !marks.code && marks.fmt.link.is_none() && text.chars().all(|c| c == ' ' || c == '\t')
        }
        Tok::Node(_) | Tok::Media { .. } | Tok::Table { .. } => false,
    }
}

/// The files of a paragraph that holds nothing but attachment links, each pair of them apart by
/// white space or a line break: those paragraphs become a `mediaGroup`. `None` for any other
/// paragraph (text around the links, two links with nothing between them, no link at all). One
/// pass, so a paragraph with thousands of links stays linear.
fn attachment_group(toks: &[Tok]) -> Option<MediaGroup> {
    let mut files = Vec::new();
    // Whether a link was seen and no separator has followed it yet.
    let mut touching = false;
    for tok in toks {
        if let Tok::Node(Inline::MediaInline(media)) = tok
            && !touching
        {
            files.push(media.file.clone());
            touching = true;
        } else if is_link_separator(tok) {
            touching = false;
        } else {
            return None;
        }
    }
    NonEmpty::from_vec(files).map(|files| MediaGroup { files })
}

/// Whether inline text is empty so far, or ends in a hard break or a literal space or tab.
fn ends_in_space(toks: &[Tok]) -> bool {
    match toks.last() {
        None | Some(Tok::HardBreak) => true,
        Some(Tok::Text {
            text, trim: true, ..
        }) => text.ends_with([' ', '\t']),
        Some(Tok::Text { .. } | Tok::Node(_) | Tok::Media { .. } | Tok::Table { .. }) => false,
    }
}

/// Removes literal spaces and tabs at the end of inline text.
fn trim_end_spaces(toks: &mut Vec<Tok>) {
    while let Some(Tok::Text {
        text, trim: true, ..
    }) = toks.last_mut()
    {
        let len = text.trim_end_matches([' ', '\t']).len();
        if len > 0 {
            text.truncate(len);
            return;
        }
        toks.pop();
    }
}

/// Splits list items into consecutive runs of task items and plain items.
fn split_runs(items: Vec<Item>) -> Vec<(bool, Vec<Item>)> {
    let mut runs: Vec<(bool, Vec<Item>)> = Vec::new();
    for item in items {
        let is_task = item.task.is_some();
        match runs.last_mut() {
            Some((task, run)) if *task == is_task => run.push(item),
            Some(_) | None => runs.push((is_task, vec![item])),
        }
    }
    runs
}
