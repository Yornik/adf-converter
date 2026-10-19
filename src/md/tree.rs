//! Builds the [IR](super::ir) from pulldown-cmark events.
//!
//! The builder is an explicit stack machine (no recursion), so arbitrarily deep input cannot
//! overflow the stack; container depth is counted and limited (`E_INPUT_TOO_DEEP`).

use std::collections::BTreeMap;

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd,
};

use crate::adf::model::{Align, HeadingLevel, NonEmptyString, PanelType, TaskState};
use crate::diagnostics::{Code, Diagnostic};
use crate::limits::MAX_TABLE_CELLS;
use crate::options::MdToAdfOptions;

use super::budget;
use super::cx::Cx;
use super::extensions::parse_raw;
use super::html::{BlockPiece, HtmlTag, Recognised, classify, decode_entities, scan_block};
use super::inline::{InlineBuilder, strip_controls};
use super::ir::{Block, Footnote, Item, Row, Span, Tok};
use super::prepass::Prepass;

/// The parsed document.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub(crate) blocks: Vec<Block>,
    /// Footnote definitions in number order.
    pub(crate) footnotes: Vec<Footnote>,
}

#[derive(Debug)]
enum Frame {
    Doc {
        children: Vec<Block>,
    },
    List {
        start: Option<u32>,
        items: Vec<Item>,
        span: Span,
    },
    Item {
        task: Option<TaskState>,
        children: Vec<Block>,
        /// Inline content directly in a tight item (no paragraph event).
        implicit: Option<(InlineBuilder, Span)>,
        span: Span,
    },
    Quote {
        alert: Option<PanelType>,
        children: Vec<Block>,
        span: Span,
    },
    Footnote {
        label: String,
        children: Vec<Block>,
        span: Span,
    },
    Box {
        children: Vec<Block>,
        span: Span,
    },
    Details {
        title: Option<String>,
        awaiting_summary: bool,
        children: Vec<Block>,
        span: Span,
    },
    Paragraph {
        inline: InlineBuilder,
        span: Span,
    },
    Heading {
        level: HeadingLevel,
        inline: InlineBuilder,
        span: Span,
    },
    Code {
        info: Option<String>,
        text: String,
        span: Span,
    },
    Html {
        lines: Vec<(String, Span)>,
    },
    Table {
        aligns: Vec<Option<Align>>,
        rows: Vec<Row>,
        span: Span,
    },
    Row {
        header: bool,
        cells: Vec<Vec<Tok>>,
        last_cell_end: Option<usize>,
    },
    Cell {
        inline: InlineBuilder,
    },
}

/// A structural item found in an HTML block.
#[derive(Debug)]
enum HtmlItem {
    Para(Vec<Tok>, Span),
    DetailsOpen(Span),
    Summary(String, Span),
    DetailsClose(Span),
    CellOpen(Span),
    CellClose(Span),
}

struct Builder<'c, 'a> {
    cx: &'c mut Cx<'a>,
    pre: &'c Prepass,
    stack: Vec<Frame>,
    depth: usize,
    /// Footnote label → number, assigned at the first reference.
    numbers: BTreeMap<String, u32>,
    definitions: Vec<(String, Vec<Block>, Span)>,
    /// GFM table cells so far, padding included (the ADF cells the rows lower to).
    table_cells: usize,
}

fn internal(message: &str) -> Diagnostic {
    Diagnostic::new(
        Code::Internal,
        format!("Markdown tree builder: {message}; please report this input"),
    )
}

/// The pulldown-cmark options: GFM tables, footnotes, strikethrough and task lists always;
/// alerts with the `alerts` extension. Smart punctuation, heading attributes, math, definition
/// lists, wikilinks and super/subscript stay off (`~x~` is strikethrough).
fn parser_options(opts: &MdToAdfOptions) -> Options {
    let mut options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    if opts.extensions.alerts {
        options |= Options::ENABLE_GFM;
    }
    options
}

/// Whether pulldown-cmark can stream `text` to the end.
///
/// pulldown-cmark 0.13's offset iterator panics on an empty tight paragraph (a list item that
/// holds only link reference definitions, followed by a deeply indented blank line); its plain
/// iterator stops early at the same point instead, leaving unbalanced start events. Running the
/// plain iterator first detects the condition without panicking.
pub(crate) fn parses_completely(text: &str, opts: &MdToAdfOptions) -> bool {
    let mut depth: usize = 0;
    for event in Parser::new_ext(text, parser_options(opts)) {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Text(_)
            | Event::Code(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::Rule
            | Event::TaskListMarker(_) => {}
        }
    }
    depth == 0
}

/// Parses the normalised source into blocks. The caller must have checked
/// [`parses_completely`].
pub(crate) fn build(cx: &mut Cx<'_>, pre: &Prepass) -> Result<Parsed, Diagnostic> {
    let options = parser_options(cx.opts);
    let text = cx.text;
    let mut builder = Builder {
        cx,
        pre,
        stack: vec![Frame::Doc {
            children: Vec::new(),
        }],
        depth: 0,
        numbers: BTreeMap::new(),
        definitions: Vec::new(),
        table_cells: 0,
    };
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        builder.event(event, range)?;
    }
    builder.finish()
}

impl Builder<'_, '_> {
    fn event(&mut self, event: Event<'_>, span: Span) -> Result<(), Diagnostic> {
        match event {
            Event::Start(tag) => self.start(tag, span)?,
            Event::End(end) => self.end(end, span)?,
            // Math is not enabled in the parser options; treat it as text if it ever appears.
            Event::Text(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                self.text(&text, span);
            }
            Event::Code(code) => {
                let cx = &mut *self.cx;
                if let Some(inline) = inline_of(&mut self.stack, &span) {
                    inline.code(&code, span, cx);
                }
            }
            Event::Html(html) => {
                if let Some(Frame::Html { lines }) = self.stack.last_mut() {
                    lines.push((html.into_string(), span));
                } else {
                    self.inline_html(&html, span)?;
                }
            }
            Event::InlineHtml(html) => self.inline_html(&html, span)?,
            Event::FootnoteReference(label) => {
                let key = label.to_lowercase();
                let next = u32::try_from(self.numbers.len() + 1).unwrap_or(u32::MAX);
                let number = *self.numbers.entry(key).or_insert(next);
                let cx = &*self.cx;
                if let Some(inline) = inline_of(&mut self.stack, &span) {
                    inline.footnote_ref(number, span, cx);
                }
            }
            Event::SoftBreak => {
                let cx = &*self.cx;
                if let Some(inline) = inline_of(&mut self.stack, &span) {
                    inline.soft_break(span, cx);
                }
            }
            Event::HardBreak => {
                let cx = &*self.cx;
                if let Some(inline) = inline_of(&mut self.stack, &span) {
                    inline.hard_break(cx);
                }
            }
            Event::Rule => self.add_block(Block::Rule { span })?,
            Event::TaskListMarker(checked) => {
                let state = if checked {
                    TaskState::Done
                } else {
                    TaskState::Todo
                };
                if let Some(Frame::Item { task, .. }) = self
                    .stack
                    .iter_mut()
                    .rev()
                    .find(|f| matches!(f, Frame::Item { .. }))
                {
                    *task = Some(state);
                }
            }
        }
        Ok(())
    }

    fn text(&mut self, text: &str, span: Span) {
        if let Some(Frame::Code { text: code, .. }) = self.stack.last_mut() {
            code.push_str(text);
            return;
        }
        let literal = self.cx.text.get(span.clone()) == Some(text);
        let cx = &*self.cx;
        if let Some(inline) = inline_of(&mut self.stack, &span) {
            inline.text(text, span, literal, cx);
        }
    }

    fn inline_html(&mut self, html: &str, span: Span) -> Result<(), Diagnostic> {
        let cx = &mut *self.cx;
        match inline_of(&mut self.stack, &span) {
            Some(inline) => inline.inline_html(html, span, cx),
            None => Ok(()),
        }
    }

    fn enter_container(&mut self, span: &Span) -> Result<(), Diagnostic> {
        self.depth += 1;
        let max = self.cx.max_depth();
        if self.depth > max {
            return Err(self
                .cx
                .diag(
                    Code::InputTooDeep,
                    format!("The input nests containers (lists, quotes, boxes, details) deeper than {max} levels"),
                    &(span.start..span.start),
                )
                .with_hint("Flatten the nesting; ADF documents this deep are not useful in Jira or Confluence"));
        }
        Ok(())
    }

    /// Flushes the implicit paragraph of a tight list item before a block starts in it.
    fn flush_implicit(&mut self) {
        if let Some(Frame::Item {
            implicit, children, ..
        }) = self.stack.last_mut()
            && let Some((inline, span)) = implicit.take()
        {
            let content = inline.finish(self.cx);
            children.push(Block::Paragraph { content, span });
        }
    }

    fn start(&mut self, tag: Tag<'_>, span: Span) -> Result<(), Diagnostic> {
        match tag {
            Tag::Paragraph => {
                self.flush_implicit();
                self.check_disabled_alert(&span);
                self.stack.push(Frame::Paragraph {
                    inline: InlineBuilder::new(),
                    span,
                });
            }
            Tag::Heading { level, .. } => {
                self.flush_implicit();
                let level =
                    HeadingLevel::new(level as u8).ok_or_else(|| internal("heading level"))?;
                self.stack.push(Frame::Heading {
                    level,
                    inline: InlineBuilder::new(),
                    span,
                });
            }
            Tag::BlockQuote(kind) => {
                self.flush_implicit();
                self.enter_container(&span)?;
                let alert = if self.cx.opts.extensions.alerts {
                    kind.map(panel_type)
                } else {
                    None
                };
                self.stack.push(Frame::Quote {
                    alert,
                    children: Vec::new(),
                    span,
                });
            }
            Tag::CodeBlock(kind) => {
                self.flush_implicit();
                let info = match kind {
                    CodeBlockKind::Fenced(info) => Some(info.into_string()),
                    CodeBlockKind::Indented => None,
                };
                self.stack.push(Frame::Code {
                    info,
                    text: String::new(),
                    span,
                });
            }
            Tag::HtmlBlock => {
                self.flush_implicit();
                self.stack.push(Frame::Html { lines: Vec::new() });
            }
            Tag::List(start) => {
                self.flush_implicit();
                self.enter_container(&span)?;
                self.stack.push(Frame::List {
                    start: start.map(|n| u32::try_from(n).unwrap_or(u32::MAX)),
                    items: Vec::new(),
                    span,
                });
            }
            Tag::Item => self.stack.push(Frame::Item {
                task: None,
                children: Vec::new(),
                implicit: None,
                span,
            }),
            Tag::FootnoteDefinition(label) => {
                self.flush_implicit();
                self.enter_container(&span)?;
                self.stack.push(Frame::Footnote {
                    label: label.to_lowercase(),
                    children: Vec::new(),
                    span,
                });
            }
            tag @ (Tag::Table(_) | Tag::TableHead | Tag::TableRow | Tag::TableCell) => {
                self.start_table_part(tag, &span)?;
            }
            tag @ (Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Link { .. }
            | Tag::Image { .. }) => self.start_inline(tag, span),
            // Not enabled in the parser options; nothing to do if they ever appear.
            Tag::Superscript
            | Tag::Subscript
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => {}
        }
        Ok(())
    }

    fn start_table_part(&mut self, tag: Tag<'_>, span: &Span) -> Result<(), Diagnostic> {
        let frame = match tag {
            Tag::Table(aligns) => {
                self.flush_implicit();
                self.enter_container(span)?;
                Frame::Table {
                    aligns: aligns.into_iter().map(align).collect(),
                    rows: Vec::new(),
                    span: span.clone(),
                }
            }
            Tag::TableHead | Tag::TableRow => {
                self.count_row_cells()?;
                Frame::Row {
                    header: matches!(tag, Tag::TableHead),
                    cells: Vec::new(),
                    last_cell_end: None,
                }
            }
            Tag::TableCell => Frame::Cell {
                inline: InlineBuilder::new(),
            },
            Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::Item
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
            | Tag::MetadataBlock(_) => return Err(internal("not a table tag")),
        };
        self.stack.push(frame);
        Ok(())
    }

    /// Counts the cells of a table row about to start against [`MAX_TABLE_CELLS`]: every row
    /// lowers to one cell per column. A backstop for the `budget` pre-scan, which estimates the
    /// same count from the source before parsing.
    fn count_row_cells(&mut self) -> Result<(), Diagnostic> {
        let Some(Frame::Table { aligns, span, .. }) = self.stack.last() else {
            return Err(internal("table row outside a table"));
        };
        self.table_cells = self.table_cells.saturating_add(aligns.len().max(1));
        if self.table_cells > MAX_TABLE_CELLS {
            let at = span.start..span.start;
            return Err(self
                .cx
                .diag(Code::InputTooLarge, budget::message(MAX_TABLE_CELLS), &at)
                .with_hint(budget::HINT));
        }
        Ok(())
    }

    fn start_inline(&mut self, tag: Tag<'_>, span: Span) {
        let cx = &mut *self.cx;
        let Some(inline) = inline_of(&mut self.stack, &span) else {
            return;
        };
        match tag {
            Tag::Emphasis => inline.start_em(cx),
            Tag::Strong => inline.start_strong(cx),
            Tag::Strikethrough => inline.start_strike(cx),
            Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            } => inline.start_link(link_type, &dest_url, &title, span, cx),
            Tag::Image {
                dest_url, title, ..
            } => inline.start_image(&dest_url, &title, span, cx),
            Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::Item
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::Superscript
            | Tag::Subscript
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn with_inline(&mut self, span: &Span, f: impl FnOnce(&mut InlineBuilder, &mut Cx<'_>)) {
        let cx = &mut *self.cx;
        if let Some(inline) = inline_of(&mut self.stack, span) {
            f(inline, cx);
        }
    }

    /// `[!NOTE]` at the start of a quote while the `alerts` extension is off.
    fn check_disabled_alert(&mut self, span: &Span) {
        if self.cx.opts.extensions.alerts {
            return;
        }
        let Some(Frame::Quote { children, .. }) = self.stack.last() else {
            return;
        };
        if !children.is_empty() {
            return;
        }
        let rest = self.cx.text.get(span.start..span.end).unwrap_or_default();
        let line = rest.split('\n').next().unwrap_or_default().trim_end();
        let is_alert = line
            .strip_prefix("[!")
            .and_then(|l| l.strip_suffix(']'))
            .is_some_and(|kind| {
                ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"]
                    .iter()
                    .any(|k| k.eq_ignore_ascii_case(kind))
            });
        if is_alert {
            let line_span = span.start..span.start + line.len();
            self.cx.warn(
                Code::ExtensionDisabled,
                "GitHub alert syntax found, but the 'alerts' extension is disabled; converted as a plain quote",
                &line_span,
            );
        }
    }

    #[allow(clippy::too_many_lines)]
    fn end(&mut self, end: TagEnd, span: Span) -> Result<(), Diagnostic> {
        match end {
            TagEnd::Paragraph => {
                let Some(Frame::Paragraph { inline, span }) = self.stack.pop() else {
                    return Err(internal("unbalanced paragraph"));
                };
                let content = inline.finish(self.cx);
                self.add_block(Block::Paragraph { content, span })?;
            }
            TagEnd::Heading(_) => {
                let Some(Frame::Heading {
                    level,
                    inline,
                    span,
                }) = self.stack.pop()
                else {
                    return Err(internal("unbalanced heading"));
                };
                let content = inline.finish(self.cx);
                self.add_block(Block::Heading {
                    level,
                    content,
                    span,
                })?;
            }
            TagEnd::BlockQuote(_) => {
                self.close_extras_until(|f| matches!(f, Frame::Quote { .. }))?;
                let Some(Frame::Quote {
                    alert,
                    children,
                    span,
                }) = self.stack.pop()
                else {
                    return Err(internal("unbalanced quote"));
                };
                self.depth = self.depth.saturating_sub(1);
                let block = match alert {
                    Some(panel) => Block::Alert {
                        panel,
                        children,
                        span,
                    },
                    None => Block::Quote { children, span },
                };
                self.add_block(block)?;
            }
            TagEnd::CodeBlock => {
                let Some(Frame::Code { info, text, span }) = self.stack.pop() else {
                    return Err(internal("unbalanced code block"));
                };
                let block = self.code_block(info.as_deref(), text, span)?;
                self.add_block(block)?;
            }
            TagEnd::HtmlBlock => {
                let Some(Frame::Html { lines }) = self.stack.pop() else {
                    return Err(internal("unbalanced HTML block"));
                };
                let items = self.html_block(lines)?;
                for item in items {
                    self.apply_html_item(item)?;
                }
            }
            TagEnd::List(_) => {
                let Some(Frame::List { start, items, span }) = self.stack.pop() else {
                    return Err(internal("unbalanced list"));
                };
                self.depth = self.depth.saturating_sub(1);
                self.add_block(Block::List { start, items, span })?;
            }
            TagEnd::Item => {
                self.close_extras_until(|f| matches!(f, Frame::Item { .. }))?;
                self.flush_implicit();
                let Some(Frame::Item {
                    task,
                    children,
                    span,
                    ..
                }) = self.stack.pop()
                else {
                    return Err(internal("unbalanced list item"));
                };
                let Some(Frame::List { items, .. }) = self.stack.last_mut() else {
                    return Err(internal("list item outside a list"));
                };
                items.push(Item {
                    task,
                    children,
                    span,
                });
            }
            TagEnd::FootnoteDefinition => {
                self.close_extras_until(|f| matches!(f, Frame::Footnote { .. }))?;
                let Some(Frame::Footnote {
                    label,
                    children,
                    span,
                }) = self.stack.pop()
                else {
                    return Err(internal("unbalanced footnote definition"));
                };
                self.depth = self.depth.saturating_sub(1);
                self.definitions.push((label, children, span));
            }
            TagEnd::Table => {
                let Some(Frame::Table { aligns, rows, span }) = self.stack.pop() else {
                    return Err(internal("unbalanced table"));
                };
                self.depth = self.depth.saturating_sub(1);
                self.add_block(Block::Table { aligns, rows, span })?;
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                let Some(Frame::Row {
                    header,
                    cells,
                    last_cell_end,
                }) = self.stack.pop()
                else {
                    return Err(internal("unbalanced table row"));
                };
                self.check_dropped_cells(last_cell_end, &span);
                let Some(Frame::Table { rows, .. }) = self.stack.last_mut() else {
                    return Err(internal("table row outside a table"));
                };
                rows.push(Row { header, cells });
            }
            TagEnd::TableCell => {
                let Some(Frame::Cell { inline }) = self.stack.pop() else {
                    return Err(internal("unbalanced table cell"));
                };
                let content = inline.finish(self.cx);
                let Some(Frame::Row {
                    cells,
                    last_cell_end,
                    ..
                }) = self.stack.last_mut()
                else {
                    return Err(internal("table cell outside a row"));
                };
                cells.push(content);
                *last_cell_end = Some(span.end);
            }
            TagEnd::Emphasis => self.with_inline(&span, |inline, cx| inline.end_em(cx)),
            TagEnd::Strong => self.with_inline(&span, |inline, cx| inline.end_strong(cx)),
            TagEnd::Strikethrough => self.with_inline(&span, |inline, cx| inline.end_strike(cx)),
            TagEnd::Link => self.with_inline(&span, InlineBuilder::end_link),
            TagEnd::Image => self.with_inline(&span, InlineBuilder::end_image),
            TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::MetadataBlock(_)
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
        Ok(())
    }

    /// GFM silently ignores cells beyond the header width; report them.
    fn check_dropped_cells(&mut self, last_cell_end: Option<usize>, row: &Span) {
        let Some(from) = last_cell_end else {
            return;
        };
        let rest = self.cx.text.get(from..row.end).unwrap_or_default();
        let rest = rest.trim_end();
        let rest = rest.strip_prefix('|').unwrap_or(rest);
        if rest.chars().any(|c| c != '|' && !c.is_whitespace()) {
            let span = from..from + rest.len() + 1;
            self.cx.warn(
                Code::TableCellDropped,
                "Table cells beyond the header row's width were dropped",
                &span,
            );
        }
    }

    fn code_block(
        &mut self,
        info: Option<&str>,
        text: String,
        span: Span,
    ) -> Result<Block, Diagnostic> {
        let language = info
            .and_then(|i| i.split_whitespace().next())
            .map(strip_controls);
        let mut text = text;
        if text.ends_with('\n') {
            text.pop();
        }
        if language.as_deref() == Some("adf") {
            if self.cx.opts.extensions.raw_adf {
                return match parse_raw(&text) {
                    Ok(nodes) => Ok(Block::Raw { nodes, span }),
                    Err(message) => {
                        Err(self
                            .cx
                            .diag(Code::RawAdf, format!("```adf block: {message}"), &span))
                    }
                };
            }
            self.cx.warn(
                Code::ExtensionDisabled,
                "```adf block found, but the 'raw-adf' extension is disabled; kept as a code block",
                &span,
            );
        }
        let cleaned = strip_controls(&text);
        if cleaned != text {
            self.cx.warn(
                Code::ControlChar,
                "Control characters were removed from a code block",
                &span,
            );
        }
        Ok(Block::Code {
            language: language.and_then(NonEmptyString::new),
            text: NonEmptyString::new(cleaned),
            span,
        })
    }

    /// Adds a finished block to the innermost container.
    fn add_block(&mut self, block: Block) -> Result<(), Diagnostic> {
        self.flush_implicit();
        if let Some(Frame::Details {
            awaiting_summary, ..
        }) = self.stack.last_mut()
        {
            *awaiting_summary = false;
        }
        match self.stack.last_mut() {
            Some(
                Frame::Doc { children }
                | Frame::Item { children, .. }
                | Frame::Quote { children, .. }
                | Frame::Footnote { children, .. }
                | Frame::Box { children, .. }
                | Frame::Details { children, .. },
            ) => {
                children.push(block);
                Ok(())
            }
            Some(
                Frame::List { .. }
                | Frame::Paragraph { .. }
                | Frame::Heading { .. }
                | Frame::Code { .. }
                | Frame::Html { .. }
                | Frame::Table { .. }
                | Frame::Row { .. }
                | Frame::Cell { .. },
            )
            | None => Err(internal("block outside a container")),
        }
    }

    /// Closes `<details>` and box frames above the innermost frame matching `pred`.
    fn close_extras_until(&mut self, pred: impl Fn(&Frame) -> bool) -> Result<(), Diagnostic> {
        while let Some(top) = self.stack.last() {
            if pred(top) {
                return Ok(());
            }
            if !is_extra(top) {
                return Err(internal("unexpected frame while closing a container"));
            }
            self.close_top_extra()?;
        }
        Err(internal("container frame not found"))
    }

    /// Pops a `<details>` or box frame and adds the block to its parent.
    fn close_top_extra(&mut self) -> Result<(), Diagnostic> {
        let block = if let Some(Frame::Details {
            title,
            children,
            span,
            ..
        }) = self.stack.last_mut()
        {
            Block::Details {
                title: title.take(),
                children: std::mem::take(children),
                span: span.clone(),
            }
        } else if let Some(Frame::Box { children, span }) = self.stack.last_mut() {
            Block::Box {
                children: std::mem::take(children),
                span: span.clone(),
            }
        } else {
            return Err(internal("expected a details or box frame"));
        };
        self.stack.pop();
        self.depth = self.depth.saturating_sub(1);
        self.add_block(block)
    }

    /// Splits an HTML block into paragraphs and structural items.
    fn html_block(&mut self, lines: Vec<(String, Span)>) -> Result<Vec<HtmlItem>, Diagnostic> {
        let mut items = Vec::new();
        let mut segment: Vec<(String, Span)> = Vec::new();
        for (line, span) in lines {
            let open = self.pre.cell_opens.contains(&span.start);
            let close = self.pre.cell_closes.contains(&span.start);
            if open || close {
                self.html_segment(&segment, &mut items)?;
                segment.clear();
                items.push(if open {
                    HtmlItem::CellOpen(span)
                } else {
                    HtmlItem::CellClose(span)
                });
            } else {
                segment.push((line, span));
            }
        }
        self.html_segment(&segment, &mut items)?;
        Ok(items)
    }

    /// Tokenises consecutive HTML block lines (tags may span lines).
    fn html_segment(
        &mut self,
        lines: &[(String, Span)],
        items: &mut Vec<HtmlItem>,
    ) -> Result<(), Diagnostic> {
        if lines.is_empty() {
            return Ok(());
        }
        let mut scan = HtmlScan::new(lines);
        let buf = std::mem::take(&mut scan.buf);
        for piece in scan_block(&buf) {
            match piece {
                BlockPiece::Text { start, end } => {
                    scan.text(buf.get(start..end).unwrap_or_default(), start, self.cx);
                    scan.last_end = end;
                }
                BlockPiece::Tag { tag, start, end } => {
                    scan.last_end = end;
                    let raw = buf.get(start..end).unwrap_or_default();
                    scan.tag(&tag, raw, start, end, items, self.cx)?;
                }
            }
        }
        scan.finish(items, self.cx);
        Ok(())
    }

    fn apply_html_item(&mut self, item: HtmlItem) -> Result<(), Diagnostic> {
        match item {
            HtmlItem::Para(content, span) => self.add_block(Block::Paragraph { content, span }),
            HtmlItem::DetailsOpen(span) => {
                self.flush_implicit();
                self.enter_container(&span)?;
                self.stack.push(Frame::Details {
                    title: None,
                    awaiting_summary: true,
                    children: Vec::new(),
                    span,
                });
                Ok(())
            }
            HtmlItem::Summary(title, span) => {
                let title = collapse_whitespace(&strip_controls(&title));
                if let Some(Frame::Details {
                    title: slot,
                    awaiting_summary,
                    ..
                }) = self.stack.last_mut()
                    && *awaiting_summary
                {
                    *awaiting_summary = false;
                    *slot = (!title.is_empty()).then_some(title);
                    return Ok(());
                }
                // A summary outside <details>: keep its text.
                if title.is_empty() {
                    return Ok(());
                }
                let content = vec![Tok::Text {
                    text: title,
                    marks: super::ir::Marks::default(),
                    span: span.clone(),
                    trim: true,
                }];
                self.add_block(Block::Paragraph { content, span })
            }
            HtmlItem::DetailsClose(span) => {
                if let Some(Frame::Details { span: open, .. }) = self.stack.last_mut() {
                    open.end = span.end;
                    return self.close_top_extra();
                }
                // A stray </details>: plain HTML.
                let mut inline = InlineBuilder::new();
                inline.unknown_html("</details>", span.clone(), self.cx)?;
                let content = inline.finish(self.cx);
                self.add_block(Block::Paragraph { content, span })
            }
            HtmlItem::CellOpen(span) => {
                self.flush_implicit();
                self.enter_container(&span)?;
                self.stack.push(Frame::Box {
                    children: Vec::new(),
                    span,
                });
                Ok(())
            }
            HtmlItem::CellClose(span) => {
                if !self.stack.iter().any(|f| matches!(f, Frame::Box { .. })) {
                    return Ok(());
                }
                while let Some(top) = self.stack.last_mut() {
                    if let Frame::Box { span: open, .. } = top {
                        open.end = span.end;
                        return self.close_top_extra();
                    }
                    if !is_extra(top) {
                        return Err(internal("box closed inside another container"));
                    }
                    self.close_top_extra()?;
                }
                Ok(())
            }
        }
    }

    fn finish(mut self) -> Result<Parsed, Diagnostic> {
        while self.stack.len() > 1 {
            if !self.stack.last().is_some_and(is_extra) {
                return Err(internal("unclosed container at end of input"));
            }
            self.close_top_extra()?;
        }
        let Some(Frame::Doc { children }) = self.stack.pop() else {
            return Err(internal("missing document frame"));
        };
        let footnotes = self.number_footnotes();
        Ok(Parsed {
            blocks: children,
            footnotes,
        })
    }

    /// Orders definitions by reference number; unreferenced ones are numbered after.
    fn number_footnotes(&mut self) -> Vec<Footnote> {
        let mut next = u32::try_from(self.numbers.len()).unwrap_or(u32::MAX);
        let mut used = std::collections::BTreeSet::new();
        let mut out: Vec<Footnote> = Vec::new();
        for (label, children, span) in std::mem::take(&mut self.definitions) {
            let number = match self.numbers.get(&label) {
                Some(&n) if used.insert(label.clone()) => n,
                _ => {
                    next = next.saturating_add(1);
                    next
                }
            };
            out.push(Footnote {
                number,
                children,
                span,
            });
        }
        out.sort_by_key(|f| f.number);
        out
    }
}

/// `<details>` and box frames, which close implicitly with their parent container.
fn is_extra(frame: &Frame) -> bool {
    matches!(frame, Frame::Details { .. } | Frame::Box { .. })
}

/// State while tokenising the lines of an HTML block.
#[derive(Debug)]
struct HtmlScan {
    buf: String,
    /// (offset in `buf`, source offset) of each line.
    map: Vec<(usize, usize)>,
    inline: InlineBuilder,
    /// Source offset of the first non-blank content of the current paragraph.
    para_start: Option<usize>,
    /// The `<summary>` title being collected, and where it started.
    summary: Option<(String, usize)>,
    last_end: usize,
}

impl HtmlScan {
    fn new(lines: &[(String, Span)]) -> Self {
        let mut buf = String::new();
        let mut map = Vec::with_capacity(lines.len());
        for (line, span) in lines {
            map.push((buf.len(), span.start));
            buf.push_str(line);
        }
        Self {
            buf,
            map,
            inline: InlineBuilder::new(),
            para_start: None,
            summary: None,
            last_end: 0,
        }
    }

    fn to_src(&self, offset: usize) -> usize {
        let index = self
            .map
            .partition_point(|&(b, _)| b <= offset)
            .saturating_sub(1);
        self.map.get(index).map_or(0, |&(b, s)| s + (offset - b))
    }

    /// Text between tags: entities are decoded and newlines are whitespace, as in HTML.
    fn text(&mut self, text: &str, start: usize, cx: &Cx<'_>) {
        if let Some((title, _)) = self.summary.as_mut() {
            title.push_str(&decode_entities(text));
            return;
        }
        let mut offset = start;
        for part in text.split_inclusive('\n') {
            let (body, newline) = match part.strip_suffix('\n') {
                Some(body) => (body, true),
                None => (part, false),
            };
            let src = self.to_src(offset);
            if !body.is_empty() {
                if !body.trim().is_empty() {
                    self.para_start.get_or_insert(src);
                }
                let decoded = decode_entities(body);
                let literal = decoded == body;
                self.inline
                    .text(&decoded, src..src + body.len(), literal, cx);
            }
            if newline {
                let nl = src + body.len();
                self.inline.text(" ", nl..nl + 1, true, cx);
            }
            offset += part.len();
        }
    }

    fn tag(
        &mut self,
        tag: &HtmlTag,
        raw: &str,
        start: usize,
        end: usize,
        items: &mut Vec<HtmlItem>,
        cx: &mut Cx<'_>,
    ) -> Result<(), Diagnostic> {
        let span = self.to_src(start)..self.to_src(end.saturating_sub(1)) + 1;
        let details = cx.opts.extensions.details;
        let kind = classify(tag, details);
        if let Some((title, at)) = self.summary.as_mut() {
            // Inside <summary>, tags are removed; <br> is a space.
            match kind {
                Recognised::SummaryClose => {
                    let title = std::mem::take(title);
                    let at = *at;
                    self.summary = None;
                    items.push(HtmlItem::Summary(title, at..span.end));
                }
                Recognised::Br => title.push(' '),
                Recognised::OpenMark { .. }
                | Recognised::CloseMark { .. }
                | Recognised::Empty
                | Recognised::Comment
                | Recognised::DetailsOpen
                | Recognised::DetailsClose
                | Recognised::SummaryOpen
                | Recognised::Unknown => {}
            }
            return Ok(());
        }
        match kind {
            Recognised::Comment | Recognised::Empty | Recognised::SummaryClose => {}
            Recognised::Br => self.inline.hard_break(cx),
            Recognised::OpenMark { name, mark } => self.inline.open_html(name, mark, cx),
            Recognised::CloseMark { name } => {
                if !self.inline.close_html(&name, cx) {
                    self.para_start.get_or_insert(span.start);
                    self.inline.unknown_html(raw, span, cx)?;
                }
            }
            Recognised::DetailsOpen => {
                self.flush(span.start, items, cx);
                items.push(HtmlItem::DetailsOpen(span));
            }
            Recognised::DetailsClose => {
                self.flush(span.start, items, cx);
                items.push(HtmlItem::DetailsClose(span));
            }
            Recognised::SummaryOpen => {
                self.flush(span.start, items, cx);
                self.summary = Some((String::new(), span.start));
            }
            Recognised::Unknown => {
                if !details && matches!(tag, HtmlTag::Open { name, .. } if name == "details") {
                    cx.warn(
                        Code::ExtensionDisabled,
                        "<details> found, but the 'details' extension is disabled; treated as HTML",
                        &span,
                    );
                }
                self.para_start.get_or_insert(span.start);
                self.inline.unknown_html(raw, span, cx)?;
            }
        }
        Ok(())
    }

    /// Ends the current paragraph. Only whitespace, hard breaks or empty tags since the last
    /// paragraph: nothing to keep.
    fn flush(&mut self, end: usize, items: &mut Vec<HtmlItem>, cx: &Cx<'_>) {
        let finished = std::mem::take(&mut self.inline);
        if let Some(start) = self.para_start.take() {
            let content = finished.finish(cx);
            items.push(HtmlItem::Para(content, start..end.max(start)));
        }
    }

    fn finish(mut self, items: &mut Vec<HtmlItem>, cx: &Cx<'_>) {
        let end = self.to_src(self.last_end);
        if let Some((title, at)) = self.summary.take() {
            items.push(HtmlItem::Summary(title, at..end));
        }
        self.flush(end, items, cx);
    }
}

/// The inline builder that receives inline events at the top of the stack. A tight list item
/// gets an implicit paragraph on its first inline event.
fn inline_of<'s>(stack: &'s mut [Frame], span: &Span) -> Option<&'s mut InlineBuilder> {
    match stack.last_mut()? {
        Frame::Paragraph { inline, .. }
        | Frame::Heading { inline, .. }
        | Frame::Cell { inline } => Some(inline),
        Frame::Item { implicit, .. } => {
            let (inline, implicit_span) =
                implicit.get_or_insert_with(|| (InlineBuilder::new(), span.clone()));
            implicit_span.end = implicit_span.end.max(span.end);
            Some(inline)
        }
        Frame::Doc { .. }
        | Frame::List { .. }
        | Frame::Quote { .. }
        | Frame::Footnote { .. }
        | Frame::Box { .. }
        | Frame::Details { .. }
        | Frame::Code { .. }
        | Frame::Html { .. }
        | Frame::Table { .. }
        | Frame::Row { .. } => None,
    }
}

fn panel_type(kind: BlockQuoteKind) -> PanelType {
    match kind {
        BlockQuoteKind::Note => PanelType::Info,
        BlockQuoteKind::Tip => PanelType::Success,
        BlockQuoteKind::Important => PanelType::Note,
        BlockQuoteKind::Warning => PanelType::Warning,
        BlockQuoteKind::Caution => PanelType::Error,
    }
}

fn align(alignment: Alignment) -> Option<Align> {
    match alignment {
        Alignment::Center => Some(Align::Center),
        Alignment::Right => Some(Align::End),
        Alignment::Left | Alignment::None => None,
    }
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
