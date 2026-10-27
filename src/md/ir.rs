//! Intermediate representation between pulldown-cmark events and the strict ADF model.
//!
//! The tree builder produces these blocks without applying any ADF container rule; the lowering
//! pass (`lower.rs`) then places every block in a container that can hold it. Keeping blocks
//! unlowered until their final container is known is what lets hoisting preserve document order
//! and apply context-dependent conversions (heading demotion, alert → panel or quote, details →
//! expand or nestedExpand) in the right place.
//!
//! Inline content is a flat token list with resolved marks, never a tree, so arbitrarily deep
//! emphasis nesting costs no recursion.

use std::ops::Range;

use serde_json::Value;

use crate::adf::model::{
    Align, Formatting, HeadingLevel, Inline, MediaSingle, NonEmptyString, PanelType, Table,
    TaskState, TextMarks,
};

/// Byte range in the normalised source (see `prepass.rs`).
pub(crate) type Span = Range<usize>;

/// The marks of one inline text run.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Marks {
    /// Formatting marks, including the link.
    pub(crate) fmt: Formatting,
    /// The `code` mark. When set, only `fmt.link` survives into the output.
    pub(crate) code: bool,
}

impl Marks {
    /// The model form: code text keeps only its link.
    pub(crate) fn to_text_marks(&self) -> TextMarks {
        if self.code {
            TextMarks::Code {
                link: self.fmt.link.clone(),
            }
        } else {
            TextMarks::Formatted(self.fmt.clone())
        }
    }

    /// Whether any formatting mark other than `link` is set.
    pub(crate) fn has_non_link_formatting(&self) -> bool {
        let f = &self.fmt;
        f.em || f.strong
            || f.strike
            || f.underline
            || f.subsup.is_some()
            || f.text_color.is_some()
            || f.background_color.is_some()
    }

    /// The number of marks other than `link` (used to estimate output size).
    pub(crate) fn count(&self) -> usize {
        let f = &self.fmt;
        [
            self.code,
            f.em,
            f.strong,
            f.strike,
            f.underline,
            f.subsup.is_some(),
            f.text_color.is_some(),
            f.background_color.is_some(),
        ]
        .into_iter()
        .filter(|&set| set)
        .count()
    }

    /// Whether any mark at all (including link and code) is set.
    pub(crate) fn is_empty(&self) -> bool {
        !self.code && self.fmt.is_empty()
    }
}

/// One inline token.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    /// Text with marks. `trim` is true when the text was written literally in the source, so
    /// whitespace at the edge of a block may be trimmed (entity-encoded spaces are kept).
    Text {
        text: String,
        marks: Marks,
        span: Span,
        trim: bool,
    },
    /// A hard line break.
    HardBreak,
    /// An inline node that carries no marks (mention, status, date, emoji, inline card).
    Node(Inline),
    /// An external image (`--images external`), placed at block level by the lowering pass.
    Media { media: MediaSingle, span: Span },
}

/// A block, before ADF container rules are applied.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Block {
    Paragraph {
        content: Vec<Tok>,
        span: Span,
    },
    Heading {
        level: HeadingLevel,
        content: Vec<Tok>,
        span: Span,
    },
    Code {
        language: Option<NonEmptyString>,
        text: Option<NonEmptyString>,
        span: Span,
    },
    Rule {
        span: Span,
    },
    List {
        /// Start number for ordered lists.
        start: Option<u32>,
        items: Vec<Item>,
        span: Span,
    },
    Quote {
        children: Vec<Block>,
        span: Span,
    },
    /// A GitHub alert (`> [!NOTE]`), recognised only with the `alerts` extension.
    Alert {
        panel: PanelType,
        children: Vec<Block>,
        span: Span,
    },
    /// A GFM table. `aligns` has one entry per column (the header width).
    Table {
        aligns: Vec<Option<Align>>,
        rows: Vec<Row>,
        span: Span,
    },
    /// A table made from a ```` ```csv ```` or ```` ```tsv ```` fence, already built from the
    /// typed model; placed like a GFM table.
    Csv {
        table: Table,
        span: Span,
    },
    /// A `::: cell` box.
    Box {
        children: Vec<Block>,
        span: Span,
    },
    /// `<details>` with an optional `<summary>` title.
    Details {
        title: Option<String>,
        children: Vec<Block>,
        span: Span,
    },
    /// Nodes from a ```` ```adf ```` block, validated for their final container when lowered.
    Raw {
        nodes: Vec<Value>,
        span: Span,
    },
    /// An external image hoisted out of inline content.
    Media {
        media: MediaSingle,
        span: Span,
    },
}

impl Block {
    /// The source span, for diagnostics.
    pub(crate) fn span(&self) -> Span {
        match self {
            Self::Paragraph { span, .. }
            | Self::Heading { span, .. }
            | Self::Code { span, .. }
            | Self::Rule { span }
            | Self::List { span, .. }
            | Self::Quote { span, .. }
            | Self::Alert { span, .. }
            | Self::Table { span, .. }
            | Self::Csv { span, .. }
            | Self::Box { span, .. }
            | Self::Details { span, .. }
            | Self::Raw { span, .. }
            | Self::Media { span, .. } => span.clone(),
        }
    }

    /// A short human-readable name used in diagnostics.
    pub(crate) fn describe(&self) -> &'static str {
        match self {
            Self::Paragraph { .. } => "paragraph",
            Self::Heading { .. } => "heading",
            Self::Code { .. } => "code block",
            Self::Rule { .. } => "horizontal rule",
            Self::List { items, .. } if items.iter().all(|i| i.task.is_some()) => "task list",
            Self::List { .. } => "list",
            Self::Quote { .. } => "quote",
            Self::Alert { .. } => "alert",
            Self::Table { .. } => "table",
            Self::Csv { .. } => "CSV table",
            Self::Box { .. } => "::: cell box",
            Self::Details { .. } => "<details> block",
            Self::Raw { .. } => "```adf block",
            Self::Media { .. } => "image",
        }
    }
}

/// A list item.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Item {
    /// `Some` for a GFM task item (`- [ ]` / `- [x]`).
    pub(crate) task: Option<TaskState>,
    pub(crate) children: Vec<Block>,
    pub(crate) span: Span,
}

/// A GFM table row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row {
    pub(crate) header: bool,
    pub(crate) cells: Vec<Vec<Tok>>,
}

/// A footnote definition (`[^label]: …`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Footnote {
    /// Number shown at references; definitions without references are numbered last.
    pub(crate) number: u32,
    pub(crate) children: Vec<Block>,
    pub(crate) span: Span,
}
