//! Strict *writer* model for Atlassian Document Format (ADF).
//!
//! Every value of these types serializes to JSON that is valid against the vendored schema
//! (`schemas/adf/full.json`) and against the extra rules Jira enforces. The rules are encoded in
//! the types instead of being checked afterwards:
//!
//! - each container has its own child enum (`ListItemChild`, `BlockquoteChild`, …), so a heading
//!   inside a blockquote or a table inside a list item cannot be represented;
//! - a list item always starts with a paragraph, a task list always starts with a task item;
//! - text is never empty ([`NonEmptyString`]) and its marks are a *set* ([`Formatting`]) or the
//!   code form ([`TextMarks::Code`]), which only combines with a link;
//! - optional attributes are omitted, never `null`, and empty `marks`/`content` arrays are not
//!   emitted where the schema forbids them.
//!
//! Serialization is hand-written so the key order is stable: `type`, then `version`/`text`/`attrs`,
//! then `content`, then `marks` — the same order Jira itself produces.
//!
//! This model is only used for *producing* ADF. Reading arbitrary ADF (ADF → Markdown, `validate`)
//! works on `serde_json::Value`, because real documents contain nodes and attributes this crate
//! never emits.

use std::sync::Arc;

use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

// ---------------------------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------------------------

/// A vector with at least one element. ADF requires `minItems: 1` for most container content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmpty<T>(Vec<T>);

impl<T> NonEmpty<T> {
    pub fn new(first: T) -> Self {
        Self(vec![first])
    }

    /// Returns `None` for an empty vector.
    pub fn from_vec(items: Vec<T>) -> Option<Self> {
        if items.is_empty() {
            None
        } else {
            Some(Self(items))
        }
    }

    pub fn push(&mut self, item: T) {
        self.0.push(item);
    }

    pub fn as_slice(&self) -> &[T] {
        &self.0
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always `false`; provided for clippy's `len_without_is_empty`.
    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn last_mut(&mut self) -> Option<&mut T> {
        self.0.last_mut()
    }

    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}

impl<'a, T> IntoIterator for &'a NonEmpty<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<T: Serialize> Serialize for NonEmpty<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// A string with at least one character (`text_node.text` has `minLength: 1`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonEmptyString(String);

impl NonEmptyString {
    /// Returns `None` for an empty string. Whitespace-only text is valid ADF.
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        if value.is_empty() {
            None
        } else {
            Some(Self(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// Appends text; the result stays non-empty.
    pub fn push_str(&mut self, more: &str) {
        self.0.push_str(more);
    }
}

impl Serialize for NonEmptyString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Serializes a node as a JSON object with `"type"` first.
trait Node {
    const TYPE: &'static str;
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error>;
}

macro_rules! serialize_as_node {
    ($($ty:ty),* $(,)?) => {$(
        impl Serialize for $ty {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(None)?;
                map.serialize_entry("type", <$ty as Node>::TYPE)?;
                self.fields(&mut map)?;
                map.end()
            }
        }
    )*};
}

/// One attribute value. Attribute objects are built from typed fields, never from `json!`.
#[derive(Clone, Copy)]
enum AttrValue<'a> {
    Str(&'a str),
    U32(u32),
    Bool(bool),
}

impl Serialize for AttrValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match *self {
            Self::Str(s) => serializer.serialize_str(s),
            Self::U32(n) => serializer.serialize_u32(n),
            Self::Bool(b) => serializer.serialize_bool(b),
        }
    }
}

struct AttrMap<'a>(&'a [(&'static str, AttrValue<'a>)]);

impl Serialize for AttrMap<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Writes `"attrs": {…}` with the given entries, in order.
fn attrs<M: SerializeMap>(
    map: &mut M,
    entries: &[(&'static str, AttrValue<'_>)],
) -> Result<(), M::Error> {
    map.serialize_entry("attrs", &AttrMap(entries))
}

// ---------------------------------------------------------------------------------------------
// Document
// ---------------------------------------------------------------------------------------------

/// The ADF root: `{"type":"doc","version":1,"content":[…]}`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Doc {
    pub content: Vec<DocBlock>,
}

impl Doc {
    /// Converts to a JSON value with stable key order (`serde_json` with `preserve_order`).
    pub fn to_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

impl Node for Doc {
    const TYPE: &'static str = "doc";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        map.serialize_entry("version", &1)?;
        map.serialize_entry("content", &self.content)
    }
}

// ---------------------------------------------------------------------------------------------
// Container child enums — one per container, mirroring the schema's content expressions.
// ---------------------------------------------------------------------------------------------

macro_rules! child_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident($inner:ty)),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        pub enum $name {
            $($variant($inner),)*
            /// A node spliced verbatim from a ```` ```adf ```` block. The producer must have
            /// validated it for this position.
            Raw(RawNode),
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match self {
                    $(Self::$variant(node) => node.serialize(serializer),)*
                    Self::Raw(node) => node.0.serialize(serializer),
                }
            }
        }
    };
}

child_enum! {
    /// Children of `doc`.
    DocBlock {
        Paragraph(AlignedParagraph),
        Heading(Heading),
        BulletList(BulletList),
        OrderedList(OrderedList),
        TaskList(TaskList),
        CodeBlock(CodeBlock),
        Blockquote(Blockquote),
        Rule(Rule),
        Panel(Panel),
        Table(Table),
        Expand(Expand),
        MediaSingle(MediaSingle),
    }
}

child_enum! {
    /// Children of `listItem` after its leading paragraph (`listItem_node`). No headings,
    /// blockquotes, tables, rules, panels or expands.
    ListItemChild {
        Paragraph(Paragraph),
        BulletList(BulletList),
        OrderedList(OrderedList),
        TaskList(TaskList),
        CodeBlock(CodeBlock),
        MediaSingle(MediaSingle),
    }
}

child_enum! {
    /// Children of `blockquote` (`blockquote_node`). No headings, rules, tables, panels, task
    /// lists or nested quotes.
    BlockquoteChild {
        Paragraph(Paragraph),
        BulletList(BulletList),
        OrderedList(OrderedList),
        CodeBlock(CodeBlock),
        MediaSingle(MediaSingle),
    }
}

child_enum! {
    /// Children of `panel` (`panel_node`). No tables, blockquotes, panels or expands.
    PanelChild {
        Paragraph(Paragraph),
        Heading(Heading),
        BulletList(BulletList),
        OrderedList(OrderedList),
        TaskList(TaskList),
        CodeBlock(CodeBlock),
        Rule(Rule),
        MediaSingle(MediaSingle),
    }
}

child_enum! {
    /// Children of `tableCell` / `tableHeader` (`table_cell_content`). No tables and no
    /// top-level `expand` (only `nestedExpand`).
    TableCellChild {
        Paragraph(AlignedParagraph),
        Heading(Heading),
        BulletList(BulletList),
        OrderedList(OrderedList),
        TaskList(TaskList),
        CodeBlock(CodeBlock),
        Blockquote(Blockquote),
        Panel(Panel),
        Rule(Rule),
        NestedExpand(NestedExpand),
        MediaSingle(MediaSingle),
    }
}

child_enum! {
    /// Children of a root-level `expand` (`non_nestable_block_content` + `nestedExpand`).
    ExpandChild {
        Paragraph(Paragraph),
        Heading(Heading),
        BulletList(BulletList),
        OrderedList(OrderedList),
        TaskList(TaskList),
        CodeBlock(CodeBlock),
        Blockquote(Blockquote),
        Panel(Panel),
        Rule(Rule),
        Table(Table),
        NestedExpand(NestedExpand),
        MediaSingle(MediaSingle),
    }
}

child_enum! {
    /// Children of `nestedExpand` (`nestedExpand_content`). No tables and no nested expands.
    NestedExpandChild {
        Paragraph(Paragraph),
        Heading(Heading),
        BulletList(BulletList),
        OrderedList(OrderedList),
        TaskList(TaskList),
        CodeBlock(CodeBlock),
        Blockquote(Blockquote),
        Panel(Panel),
        Rule(Rule),
        MediaSingle(MediaSingle),
    }
}

/// Raw ADF JSON for one node, from a ```` ```adf ```` block.
#[derive(Debug, Clone, PartialEq)]
pub struct RawNode(pub serde_json::Value);

// ---------------------------------------------------------------------------------------------
// Block nodes
// ---------------------------------------------------------------------------------------------

/// A paragraph without block marks (`paragraph_with_no_marks_node`). Empty content is valid
/// and serializes as `{"type":"paragraph"}`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Paragraph {
    pub content: Vec<Inline>,
}

impl Node for Paragraph {
    const TYPE: &'static str = "paragraph";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        if !self.content.is_empty() {
            map.serialize_entry("content", &self.content)?;
        }
        Ok(())
    }
}

/// Horizontal alignment of a paragraph (`alignment` mark). Left is the default and has no mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Center,
    End,
}

impl Align {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::End => "end",
        }
    }
}

/// A paragraph that may carry an `alignment` mark (`paragraph_with_alignment_node`). Only valid
/// directly in `doc` and in table cells.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AlignedParagraph {
    pub content: Vec<Inline>,
    pub align: Option<Align>,
}

impl From<Paragraph> for AlignedParagraph {
    fn from(p: Paragraph) -> Self {
        Self {
            content: p.content,
            align: None,
        }
    }
}

impl Node for AlignedParagraph {
    const TYPE: &'static str = "paragraph";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        if !self.content.is_empty() {
            map.serialize_entry("content", &self.content)?;
        }
        if let Some(align) = self.align {
            map.serialize_entry("marks", &[AlignmentMark(align)])?;
        }
        Ok(())
    }
}

struct AlignmentMark(Align);

impl Serialize for AlignmentMark {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "alignment")?;
        attrs(&mut map, &[("align", AttrValue::Str(self.0.as_str()))])?;
        map.end()
    }
}

/// Heading level 1–6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HeadingLevel(u8);

impl HeadingLevel {
    /// Returns `None` outside 1–6.
    pub fn new(level: u8) -> Option<Self> {
        (1..=6).contains(&level).then_some(Self(level))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

/// `{"type":"heading","attrs":{"level":N},"content":[…]}`. Allowed in doc, panel, expand,
/// nestedExpand and table cells — never in a list item or blockquote.
#[derive(Debug, Clone, PartialEq)]
pub struct Heading {
    pub level: HeadingLevel,
    pub content: Vec<Inline>,
}

impl Node for Heading {
    const TYPE: &'static str = "heading";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        attrs(map, &[("level", AttrValue::U32(u32::from(self.level.0)))])?;
        if !self.content.is_empty() {
            map.serialize_entry("content", &self.content)?;
        }
        Ok(())
    }
}

/// `{"type":"bulletList","content":[listItem…]}`.
#[derive(Debug, Clone, PartialEq)]
pub struct BulletList {
    pub items: NonEmpty<ListItem>,
}

impl Node for BulletList {
    const TYPE: &'static str = "bulletList";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        map.serialize_entry("content", &self.items)
    }
}

/// `{"type":"orderedList","attrs":{"order":N},"content":[listItem…]}`. `order` is always emitted.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderedList {
    pub order: u32,
    pub items: NonEmpty<ListItem>,
}

impl Node for OrderedList {
    const TYPE: &'static str = "orderedList";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        attrs(map, &[("order", AttrValue::U32(self.order))])?;
        map.serialize_entry("content", &self.items)
    }
}

/// A list item: a leading paragraph (Jira rejects other first children) plus more blocks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListItem {
    pub first: Paragraph,
    pub rest: Vec<ListItemChild>,
}

impl Node for ListItem {
    const TYPE: &'static str = "listItem";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        map.serialize_entry("content", &ListItemContent(self))
    }
}

struct ListItemContent<'a>(&'a ListItem);

impl Serialize for ListItemContent<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(1 + self.0.rest.len()))?;
        seq.serialize_element(&self.0.first)?;
        for child in &self.0.rest {
            seq.serialize_element(child)?;
        }
        seq.end()
    }
}

/// `TODO` or `DONE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Todo,
    Done,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "TODO",
            Self::Done => "DONE",
        }
    }
}

/// `{"type":"taskList","attrs":{"localId":…},"content":[taskItem, (taskItem|taskList)…]}`.
/// Jira rejects task lists without `localId`; the first child must be a task item.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskList {
    pub local_id: String,
    pub first: TaskItem,
    pub rest: Vec<TaskListChild>,
}

/// Children of a task list after the first task item.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskListChild {
    TaskItem(TaskItem),
    TaskList(TaskList),
}

impl Serialize for TaskListChild {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::TaskItem(item) => item.serialize(serializer),
            Self::TaskList(list) => list.serialize(serializer),
        }
    }
}

impl Node for TaskList {
    const TYPE: &'static str = "taskList";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        attrs(map, &[("localId", AttrValue::Str(&self.local_id))])?;
        map.serialize_entry("content", &TaskListContent(self))
    }
}

struct TaskListContent<'a>(&'a TaskList);

impl Serialize for TaskListContent<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(1 + self.0.rest.len()))?;
        seq.serialize_element(&self.0.first)?;
        for child in &self.0.rest {
            seq.serialize_element(child)?;
        }
        seq.end()
    }
}

/// `{"type":"taskItem","attrs":{"localId":…,"state":"TODO"},"content":[inline…]}`. Task items
/// hold inline content only.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskItem {
    pub local_id: String,
    pub state: TaskState,
    pub content: Vec<Inline>,
}

impl Node for TaskItem {
    const TYPE: &'static str = "taskItem";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        attrs(
            map,
            &[
                ("localId", AttrValue::Str(&self.local_id)),
                ("state", AttrValue::Str(self.state.as_str())),
            ],
        )?;
        if !self.content.is_empty() {
            map.serialize_entry("content", &self.content)?;
        }
        Ok(())
    }
}

/// `{"type":"codeBlock","attrs":{"language":…},"content":[{"type":"text","text":…}]}`.
/// Holds one unmarked text node; lines are joined with `\n`. Empty → `{"type":"codeBlock"}`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CodeBlock {
    /// Omitted when `None`; never emitted as `null` or as an empty string.
    pub language: Option<NonEmptyString>,
    pub text: Option<NonEmptyString>,
}

impl Node for CodeBlock {
    const TYPE: &'static str = "codeBlock";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        if let Some(language) = &self.language {
            attrs(map, &[("language", AttrValue::Str(language.as_str()))])?;
        }
        if let Some(text) = &self.text {
            map.serialize_entry("content", &[PlainText(text)])?;
        }
        Ok(())
    }
}

/// A text node without marks (`text_with_no_marks_node`), used inside code blocks.
struct PlainText<'a>(&'a NonEmptyString);

impl Serialize for PlainText<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "text")?;
        map.serialize_entry("text", self.0)?;
        map.end()
    }
}

/// `{"type":"blockquote","content":[…]}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Blockquote {
    pub content: NonEmpty<BlockquoteChild>,
}

impl Node for Blockquote {
    const TYPE: &'static str = "blockquote";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        map.serialize_entry("content", &self.content)
    }
}

/// `{"type":"rule"}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rule;

impl Node for Rule {
    const TYPE: &'static str = "rule";
    fn fields<M: SerializeMap>(&self, _map: &mut M) -> Result<(), M::Error> {
        Ok(())
    }
}

/// Panel types that Jira and Confluence both render. The schema also has `tip` and `custom`,
/// which are not emitted (support in Jira is unverified).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelType {
    Info,
    Note,
    Warning,
    Success,
    Error,
}

impl PanelType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Note => "note",
            Self::Warning => "warning",
            Self::Success => "success",
            Self::Error => "error",
        }
    }
}

/// `{"type":"panel","attrs":{"panelType":…},"content":[…]}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub panel_type: PanelType,
    pub content: NonEmpty<PanelChild>,
}

impl Node for Panel {
    const TYPE: &'static str = "panel";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        attrs(
            map,
            &[("panelType", AttrValue::Str(self.panel_type.as_str()))],
        )?;
        map.serialize_entry("content", &self.content)
    }
}

/// `{"type":"table","attrs":{"isNumberColumnEnabled":false,"layout":"default"},"content":[…]}`.
/// Only allowed in `doc` and `expand`. Rows should be rectangular (the producer pads them).
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub rows: NonEmpty<TableRow>,
}

impl Node for Table {
    const TYPE: &'static str = "table";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        attrs(
            map,
            &[
                ("isNumberColumnEnabled", AttrValue::Bool(false)),
                ("layout", AttrValue::Str("default")),
            ],
        )?;
        map.serialize_entry("content", &self.rows)
    }
}

/// `{"type":"tableRow","content":[cell…]}`.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRow {
    pub cells: NonEmpty<TableCell>,
}

impl Node for TableRow {
    const TYPE: &'static str = "tableRow";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        map.serialize_entry("content", &self.cells)
    }
}

/// Header (`tableHeader`) or data (`tableCell`) cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Header,
    Data,
}

/// A table cell. Content is block nodes; inline content must be wrapped in a paragraph, and an
/// empty cell is `[{"type":"paragraph"}]` (see [`TableCell::empty`]).
#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub kind: CellKind,
    pub content: NonEmpty<TableCellChild>,
}

impl TableCell {
    /// A cell holding one empty paragraph.
    pub fn empty(kind: CellKind) -> Self {
        Self {
            kind,
            content: NonEmpty::new(TableCellChild::Paragraph(AlignedParagraph::default())),
        }
    }
}

impl Serialize for TableCell {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        let ty = match self.kind {
            CellKind::Header => "tableHeader",
            CellKind::Data => "tableCell",
        };
        map.serialize_entry("type", ty)?;
        map.serialize_entry("content", &self.content)?;
        map.end()
    }
}

/// Root-level `expand`: `{"type":"expand","attrs":{"title":…},"content":[…]}`. `attrs` is always
/// emitted (the docs mark it required), as `{}` when there is no title.
#[derive(Debug, Clone, PartialEq)]
pub struct Expand {
    pub title: Option<String>,
    pub content: NonEmpty<ExpandChild>,
}

impl Node for Expand {
    const TYPE: &'static str = "expand";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        match &self.title {
            Some(title) => attrs(map, &[("title", AttrValue::Str(title))])?,
            None => attrs(map, &[])?,
        }
        map.serialize_entry("content", &self.content)
    }
}

/// `nestedExpand`, allowed inside `expand` and table cells. `attrs` is required by the schema.
#[derive(Debug, Clone, PartialEq)]
pub struct NestedExpand {
    pub title: Option<String>,
    pub content: NonEmpty<NestedExpandChild>,
}

impl Node for NestedExpand {
    const TYPE: &'static str = "nestedExpand";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        match &self.title {
            Some(title) => attrs(map, &[("title", AttrValue::Str(title))])?,
            None => attrs(map, &[])?,
        }
        map.serialize_entry("content", &self.content)
    }
}

/// Largest width or height (pixels) that `adf` writes for file media.
pub const MAX_MEDIA_PIXELS: u32 = 100_000;

/// Longest media `collection` that `adf` writes.
pub const MAX_COLLECTION_LEN: usize = 64;

/// The id of a Media Services file: a UUID (8-4-4-4-12 hex digits), kept exactly as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaId(String);

impl MediaId {
    /// Returns `None` unless `value` is a UUID in the 8-4-4-4-12 form (either hex case).
    pub fn parse(value: &str) -> Option<Self> {
        const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
        let mut parts = value.split('-');
        for len in GROUPS {
            let part = parts.next()?;
            if part.len() != len || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
        }
        parts.next().is_none().then(|| Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The `collection` of a Media Services file: 0 to [`MAX_COLLECTION_LEN`] characters from
/// `[A-Za-z0-9_-]`. Empty is valid (Jira writes it). The target does not enforce the charset.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaCollection(String);

impl MediaCollection {
    /// Returns `None` for a value that is too long or has a character outside the charset.
    pub fn parse(value: &str) -> Option<Self> {
        let ok = value.len() <= MAX_COLLECTION_LEN
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        ok.then(|| Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A pixel width or height of a file: an integer from 1 to [`MAX_MEDIA_PIXELS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaDimension(u32);

impl MediaDimension {
    /// Returns `None` outside `1..=100000`.
    pub fn new(value: u32) -> Option<Self> {
        (1..=MAX_MEDIA_PIXELS)
            .contains(&value)
            .then_some(Self(value))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

/// `mediaSingle` layouts of the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaLayout {
    Wide,
    FullWidth,
    #[default]
    Center,
    WrapRight,
    WrapLeft,
    AlignEnd,
    AlignStart,
}

impl MediaLayout {
    /// Every layout, in schema order.
    pub const ALL: [Self; 7] = [
        Self::Wide,
        Self::FullWidth,
        Self::Center,
        Self::WrapRight,
        Self::WrapLeft,
        Self::AlignEnd,
        Self::AlignStart,
    ];

    /// The schema name of the layout.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wide => "wide",
            Self::FullWidth => "full-width",
            Self::Center => "center",
            Self::WrapRight => "wrap-right",
            Self::WrapLeft => "wrap-left",
            Self::AlignEnd => "align-end",
            Self::AlignStart => "align-start",
        }
    }

    /// Parses a schema name (lower case only).
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == value)
    }
}

/// The width of a `mediaSingle`: a percentage of the container (1 to 100, no `widthType`) or a
/// pixel width (`widthType: "pixel"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaSize(SizeRepr);

/// The two forms of [`MediaSize`]; private so that only the validating constructors can build one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeRepr {
    Percent(u8),
    Pixels(u32),
}

impl MediaSize {
    /// Returns `None` outside `1..=100`.
    pub fn percent(value: u32) -> Option<Self> {
        u8::try_from(value)
            .ok()
            .filter(|v| (1..=100).contains(v))
            .map(|v| Self(SizeRepr::Percent(v)))
    }

    /// Returns `None` outside `1..=100000`.
    pub fn pixels(value: u32) -> Option<Self> {
        MediaDimension::new(value).map(|d| Self(SizeRepr::Pixels(d.get())))
    }
}

/// A file in Media Services, referenced by id. `adf` never uploads or looks anything up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMedia {
    pub id: MediaId,
    pub collection: MediaCollection,
    /// Original pixel size of the file.
    pub width: Option<MediaDimension>,
    pub height: Option<MediaDimension>,
}

/// Where a `mediaSingle` gets its picture from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaSource {
    /// An image by URL (`--images external`); Jira rendering of external media is unverified.
    External { url: String },
    /// A file in Media Services (`adf:media`).
    File(FileMedia),
}

/// A block-level image:
/// `{"type":"mediaSingle","attrs":{"layout":…},"content":[{"type":"media","attrs":{…}}]}`.
///
/// External media always uses layout `center` and no size.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaSingle {
    pub source: MediaSource,
    pub alt: Option<NonEmptyString>,
    pub layout: MediaLayout,
    pub size: Option<MediaSize>,
}

impl MediaSingle {
    /// An image by URL.
    pub fn external(url: String, alt: Option<NonEmptyString>) -> Self {
        Self {
            source: MediaSource::External { url },
            alt,
            layout: MediaLayout::Center,
            size: None,
        }
    }
}

impl Node for MediaSingle {
    const TYPE: &'static str = "mediaSingle";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        let mut entries = vec![("layout", AttrValue::Str(self.layout.as_str()))];
        match self.size {
            None => {}
            Some(MediaSize(SizeRepr::Percent(n))) => {
                entries.push(("width", AttrValue::U32(n.into())));
            }
            Some(MediaSize(SizeRepr::Pixels(n))) => {
                entries.push(("width", AttrValue::U32(n)));
                entries.push(("widthType", AttrValue::Str("pixel")));
            }
        }
        attrs(map, &entries)?;
        map.serialize_entry("content", &[MediaNode(self)])
    }
}

/// The `media` child of a [`MediaSingle`].
struct MediaNode<'a>(&'a MediaSingle);

impl Serialize for MediaNode<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "media")?;
        let mut entries = Vec::new();
        match &self.0.source {
            MediaSource::External { url } => {
                entries.push(("type", AttrValue::Str("external")));
                entries.push(("url", AttrValue::Str(url)));
            }
            MediaSource::File(file) => {
                entries.push(("type", AttrValue::Str("file")));
                entries.push(("id", AttrValue::Str(file.id.as_str())));
                entries.push(("collection", AttrValue::Str(file.collection.as_str())));
            }
        }
        if let Some(alt) = &self.0.alt {
            entries.push(("alt", AttrValue::Str(alt.as_str())));
        }
        if let MediaSource::File(file) = &self.0.source {
            if let Some(width) = file.width {
                entries.push(("width", AttrValue::U32(width.get())));
            }
            if let Some(height) = file.height {
                entries.push(("height", AttrValue::U32(height.get())));
            }
        }
        attrs(&mut map, &entries)?;
        map.end()
    }
}

serialize_as_node!(
    Doc,
    Paragraph,
    AlignedParagraph,
    Heading,
    BulletList,
    OrderedList,
    ListItem,
    TaskList,
    TaskItem,
    CodeBlock,
    Blockquote,
    Rule,
    Panel,
    Table,
    TableRow,
    Expand,
    NestedExpand,
    MediaSingle,
);

// ---------------------------------------------------------------------------------------------
// Inline nodes
// ---------------------------------------------------------------------------------------------

/// Inline content of paragraphs, headings and task items.
#[derive(Debug, Clone, PartialEq)]
pub enum Inline {
    Text(Text),
    /// `{"type":"hardBreak"}` — never carries attrs or marks.
    HardBreak,
    Mention(Mention),
    Emoji(Emoji),
    Date(Date),
    Status(Status),
    InlineCard(InlineCard),
}

impl Serialize for Inline {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(text) => text.serialize(serializer),
            Self::HardBreak => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("type", "hardBreak")?;
                map.end()
            }
            Self::Mention(node) => node.serialize(serializer),
            Self::Emoji(node) => node.serialize(serializer),
            Self::Date(node) => node.serialize(serializer),
            Self::Status(node) => node.serialize(serializer),
            Self::InlineCard(node) => node.serialize(serializer),
        }
    }
}

/// `{"type":"text","text":…,"marks":[…]}`. `marks` is omitted when there are none.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub text: NonEmptyString,
    pub marks: TextMarks,
}

impl Text {
    /// Unmarked text; `None` for an empty string.
    pub fn plain(text: impl Into<String>) -> Option<Self> {
        NonEmptyString::new(text).map(|text| Self {
            text,
            marks: TextMarks::default(),
        })
    }
}

impl Node for Text {
    const TYPE: &'static str = "text";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        map.serialize_entry("text", &self.text)?;
        if !self.marks.is_empty() {
            map.serialize_entry("marks", &self.marks)?;
        }
        Ok(())
    }
}

/// The marks of one text node. ADF splits text into two disjoint families
/// (`formatted_text_inline_node` / `code_inline_node`): code text may only add a link.
#[derive(Debug, Clone, PartialEq)]
pub enum TextMarks {
    Formatted(Formatting),
    Code { link: Option<Link> },
}

impl Default for TextMarks {
    fn default() -> Self {
        Self::Formatted(Formatting::default())
    }
}

impl TextMarks {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Formatted(f) => f.is_empty(),
            Self::Code { .. } => false,
        }
    }
}

/// Serializes in Atlassian's canonical mark order (`marks-in-order.js`): link, em, strong,
/// textColor, backgroundColor, strike, subsup, underline, code.
impl Serialize for TextMarks {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        match self {
            Self::Formatted(f) => {
                if let Some(link) = &f.link {
                    seq.serialize_element(link)?;
                }
                if f.em {
                    seq.serialize_element(&SimpleMark("em"))?;
                }
                if f.strong {
                    seq.serialize_element(&SimpleMark("strong"))?;
                }
                if let Some(color) = &f.text_color {
                    seq.serialize_element(&ColorMark("textColor", color))?;
                }
                if let Some(color) = &f.background_color {
                    seq.serialize_element(&ColorMark("backgroundColor", color))?;
                }
                if f.strike {
                    seq.serialize_element(&SimpleMark("strike"))?;
                }
                if let Some(subsup) = f.subsup {
                    seq.serialize_element(&subsup)?;
                }
                if f.underline {
                    seq.serialize_element(&SimpleMark("underline"))?;
                }
            }
            Self::Code { link } => {
                if let Some(link) = link {
                    seq.serialize_element(link)?;
                }
                seq.serialize_element(&SimpleMark("code"))?;
            }
        }
        seq.end()
    }
}

/// The set of formatting marks on non-code text. Being a set, it cannot hold duplicates.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Formatting {
    pub link: Option<Link>,
    pub em: bool,
    pub strong: bool,
    pub text_color: Option<HexColor>,
    pub background_color: Option<HexColor>,
    pub strike: bool,
    pub subsup: Option<SubSup>,
    pub underline: bool,
}

impl Formatting {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

struct SimpleMark(&'static str);

impl Serialize for SimpleMark {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("type", self.0)?;
        map.end()
    }
}

struct ColorMark<'a>(&'static str, &'a HexColor);

impl Serialize for ColorMark<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", self.0)?;
        attrs(&mut map, &[("color", AttrValue::Str(self.1.as_str()))])?;
        map.end()
    }
}

/// `#RRGGBB` (the schema pattern for `textColor` / `backgroundColor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexColor(String);

impl HexColor {
    /// Accepts exactly `#` followed by six hex digits.
    pub fn parse(value: &str) -> Option<Self> {
        let digits = value.strip_prefix('#')?;
        (digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `subsup` mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubSup {
    Sub,
    Sup,
}

impl Serialize for SubSup {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "subsup")?;
        let value = match self {
            Self::Sub => "sub",
            Self::Sup => "sup",
        };
        attrs(&mut map, &[("type", AttrValue::Str(value))])?;
        map.end()
    }
}

/// `link` mark: `{"type":"link","attrs":{"href":…,"title":…}}`.
///
/// The attributes are shared, so a clone is cheap and compares in constant time with the value
/// it was cloned from: every text run of one link label carries the same link, and a label of
/// many runs must not copy a long `href` or `title` once per run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link(Arc<LinkAttrs>);

#[derive(Debug, PartialEq, Eq)]
struct LinkAttrs {
    href: Href,
    title: Option<NonEmptyString>,
}

impl Link {
    /// A link to `href`; `title` is omitted when `None`.
    pub fn new(href: Href, title: Option<NonEmptyString>) -> Self {
        Self(Arc::new(LinkAttrs { href, title }))
    }

    pub fn href(&self) -> &Href {
        &self.0.href
    }

    /// The title; never empty.
    pub fn title(&self) -> Option<&NonEmptyString> {
        self.0.title.as_ref()
    }

    /// Whether `self` and `other` share their attributes (one is a clone of the other).
    #[cfg(test)]
    pub(crate) fn shares(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Serialize for Link {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("type", "link")?;
        let mut entries = vec![("href", AttrValue::Str(self.href().as_str()))];
        if let Some(title) = self.title() {
            entries.push(("title", AttrValue::Str(title.as_str())));
        }
        attrs(&mut map, &entries)?;
        map.end()
    }
}

/// The URL forms [`Href::parse`] accepts, for messages and hints.
pub const ALLOWED_URL_FORMS: &str = "http, https, ftp, ftps, mailto, tel, /path, #anchor";

/// A link target that passes this tool's URL allowlist (a strict subset of Atlassian's
/// `isSafeUrl`): `http`, `https`, `mailto`, `tel`, `ftp`, `ftps`, root-relative `/…` and `#…`
/// ([`ALLOWED_URL_FORMS`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Href(String);

impl Href {
    /// Schemes are matched case-insensitively after trimming whitespace. Returns `None` for
    /// anything else (`javascript:`, `data:`, relative paths, empty strings, …).
    ///
    /// A root-relative path must not start with `//` or `/\`: browsers read both as a
    /// protocol-relative URL to another host (`/\evil.com` resolves to `https://evil.com/`).
    pub fn parse(url: &str) -> Option<Self> {
        let url = url.trim();
        if url.is_empty() || url.chars().any(char::is_control) {
            return None;
        }
        if url.starts_with('#') {
            return Some(Self(url.to_owned()));
        }
        if let Some(path) = url.strip_prefix('/') {
            return (!path.starts_with(['/', '\\'])).then(|| Self(url.to_owned()));
        }
        let (scheme, rest) = url.split_once(':')?;
        let scheme = scheme.to_ascii_lowercase();
        let ok = match scheme.as_str() {
            "http" | "https" | "ftp" | "ftps" => rest.starts_with("//") && rest.len() > 2,
            "mailto" | "tel" => !rest.is_empty(),
            _ => false,
        };
        ok.then(|| Self(url.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `{"type":"mention","attrs":{"id":…,"text":"@Name"}}`. Never carries marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    /// Atlassian account ID.
    pub id: NonEmptyString,
    /// Display text, conventionally starting with `@`.
    pub text: Option<String>,
}

impl Node for Mention {
    const TYPE: &'static str = "mention";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        let mut entries = vec![("id", AttrValue::Str(self.id.as_str()))];
        if let Some(text) = &self.text {
            entries.push(("text", AttrValue::Str(text)));
        }
        attrs(map, &entries)
    }
}

/// `{"type":"emoji","attrs":{"shortName":":smile:","text":"😄"}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emoji {
    pub short_name: NonEmptyString,
    pub id: Option<String>,
    pub text: Option<String>,
}

impl Node for Emoji {
    const TYPE: &'static str = "emoji";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        let mut entries = vec![("shortName", AttrValue::Str(self.short_name.as_str()))];
        if let Some(id) = &self.id {
            entries.push(("id", AttrValue::Str(id)));
        }
        if let Some(text) = &self.text {
            entries.push(("text", AttrValue::Str(text)));
        }
        attrs(map, &entries)
    }
}

/// `{"type":"date","attrs":{"timestamp":"<ms since epoch, UTC midnight>"}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Date {
    /// Milliseconds since the Unix epoch, as a decimal string.
    pub timestamp: NonEmptyString,
}

impl Node for Date {
    const TYPE: &'static str = "date";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        attrs(
            map,
            &[("timestamp", AttrValue::Str(self.timestamp.as_str()))],
        )
    }
}

/// Named status colours. Hex colours are schema-valid only since 57.5.0 and are not emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusColor {
    Neutral,
    Purple,
    Blue,
    Red,
    Yellow,
    Green,
}

impl StatusColor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Purple => "purple",
            Self::Blue => "blue",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "neutral" => Self::Neutral,
            "purple" => Self::Purple,
            "blue" => Self::Blue,
            "red" => Self::Red,
            "yellow" => Self::Yellow,
            "green" => Self::Green,
            _ => return None,
        })
    }
}

/// `{"type":"status","attrs":{"text":"DONE","color":"green"}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: NonEmptyString,
    pub color: StatusColor,
}

impl Node for Status {
    const TYPE: &'static str = "status";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        attrs(
            map,
            &[
                ("text", AttrValue::Str(self.text.as_str())),
                ("color", AttrValue::Str(self.color.as_str())),
            ],
        )
    }
}

/// `{"type":"inlineCard","attrs":{"url":…}}` (a smart link).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineCard {
    pub url: Href,
}

impl Node for InlineCard {
    const TYPE: &'static str = "inlineCard";
    fn fields<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        attrs(map, &[("url", AttrValue::Str(self.url.as_str()))])
    }
}

serialize_as_node!(Text, Mention, Emoji, Date, Status, InlineCard);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(s: &str) -> Inline {
        Inline::Text(Text::plain(s).unwrap())
    }

    fn para(s: &str) -> Paragraph {
        Paragraph {
            content: vec![text(s)],
        }
    }

    fn item(s: &str) -> ListItem {
        ListItem {
            first: para(s),
            rest: vec![],
        }
    }

    const ID: &str = "11111111-2222-3333-4444-555555555555";

    fn file_image(
        width: Option<MediaDimension>,
        height: Option<MediaDimension>,
        layout: MediaLayout,
        size: Option<MediaSize>,
    ) -> MediaSingle {
        MediaSingle {
            source: MediaSource::File(FileMedia {
                id: MediaId::parse(ID).unwrap(),
                collection: MediaCollection::parse("contentId-123456").unwrap(),
                width,
                height,
            }),
            alt: NonEmptyString::new("shot.png"),
            layout,
            size,
        }
    }

    #[test]
    fn media_id_is_a_uuid() {
        for ok in [ID, "AAAAAAAA-bbbb-CCCC-dddd-0123456789aB"] {
            assert_eq!(MediaId::parse(ok).unwrap().as_str(), ok);
        }
        for bad in [
            "",
            "abc",
            "11111111222233334444555555555555",
            "11111111-2222-3333-4444-55555555555",
            "11111111-2222-3333-4444-5555555555555",
            "11111111-2222-3333-4444-55555555555g",
            "11111111-2222-3333-4444-555555555555-",
            "11111111-2222-3333-4444-555555555555-1",
            " 11111111-2222-3333-4444-555555555555",
            "{11111111-2222-3333-4444-555555555555}",
            "1111111-12222-3333-4444-555555555555",
            "11111111-2222-3333-4444-5555555555é5",
        ] {
            assert!(MediaId::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn media_collection_charset_and_length() {
        for ok in ["", "contentId-123456", "comment-container-1", "a_B-9"] {
            assert_eq!(MediaCollection::parse(ok).unwrap().as_str(), ok);
        }
        assert!(MediaCollection::parse("").unwrap().is_empty());
        assert!(MediaCollection::parse(&"a".repeat(64)).is_some());
        for bad in [
            &"a".repeat(65),
            "a b",
            "a/b",
            "a.b",
            "é",
            "a\n",
            "a=b",
            "a&b",
        ] {
            assert!(MediaCollection::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn media_dimension_size_and_layout_bounds() {
        assert!(MediaDimension::new(0).is_none());
        assert_eq!(MediaDimension::new(1).unwrap().get(), 1);
        assert_eq!(MediaDimension::new(100_000).unwrap().get(), 100_000);
        assert!(MediaDimension::new(100_001).is_none());
        assert!(MediaSize::percent(0).is_none());
        assert_eq!(MediaSize::percent(1), Some(MediaSize(SizeRepr::Percent(1))));
        assert_eq!(
            MediaSize::percent(100),
            Some(MediaSize(SizeRepr::Percent(100)))
        );
        assert!(MediaSize::percent(101).is_none());
        assert!(MediaSize::percent(256 + 50).is_none());
        assert!(MediaSize::pixels(0).is_none());
        assert_eq!(
            MediaSize::pixels(100_000),
            Some(MediaSize(SizeRepr::Pixels(100_000)))
        );
        assert!(MediaSize::pixels(100_001).is_none());
        for layout in MediaLayout::ALL {
            assert_eq!(MediaLayout::parse(layout.as_str()), Some(layout));
        }
        assert_eq!(MediaLayout::default(), MediaLayout::Center);
        for bad in ["", "Wide", "left", "full_width", " center"] {
            assert!(MediaLayout::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn media_single_serialization_is_exact() {
        let external = MediaSingle::external("https://x/i.png".into(), NonEmptyString::new("a"));
        assert_eq!(
            serde_json::to_string(&external).unwrap(),
            r#"{"type":"mediaSingle","attrs":{"layout":"center"},"content":[{"type":"media","attrs":{"type":"external","url":"https://x/i.png","alt":"a"}}]}"#
        );
        let full = file_image(
            MediaDimension::new(1512),
            MediaDimension::new(945),
            MediaLayout::Wide,
            MediaSize::pixels(720),
        );
        assert_eq!(
            serde_json::to_string(&full).unwrap(),
            r#"{"type":"mediaSingle","attrs":{"layout":"wide","width":720,"widthType":"pixel"},"content":[{"type":"media","attrs":{"type":"file","id":"11111111-2222-3333-4444-555555555555","collection":"contentId-123456","alt":"shot.png","width":1512,"height":945}}]}"#
        );
        let minimal = MediaSingle {
            source: MediaSource::File(FileMedia {
                id: MediaId::parse(ID).unwrap(),
                collection: MediaCollection::default(),
                width: None,
                height: MediaDimension::new(5),
            }),
            alt: None,
            layout: MediaLayout::default(),
            size: MediaSize::percent(50),
        };
        assert_eq!(
            serde_json::to_string(&minimal).unwrap(),
            r#"{"type":"mediaSingle","attrs":{"layout":"center","width":50},"content":[{"type":"media","attrs":{"type":"file","id":"11111111-2222-3333-4444-555555555555","collection":"","height":5}}]}"#
        );
    }

    #[test]
    fn doc_key_order_and_shape() {
        let doc = Doc {
            content: vec![
                DocBlock::Heading(Heading {
                    level: HeadingLevel::new(2).unwrap(),
                    content: vec![text("Hi")],
                }),
                DocBlock::Paragraph(AlignedParagraph::default()),
            ],
        };
        let s = serde_json::to_string(&doc).unwrap();
        assert_eq!(
            s,
            r#"{"type":"doc","version":1,"content":[{"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Hi"}]},{"type":"paragraph"}]}"#
        );
    }

    #[test]
    fn marks_are_canonical_and_code_only_takes_link() {
        let link = Link::new(Href::parse("https://example.com").unwrap(), None);
        let formatted = Text {
            text: NonEmptyString::new("x").unwrap(),
            marks: TextMarks::Formatted(Formatting {
                strong: true,
                em: true,
                strike: true,
                link: Some(link.clone()),
                ..Formatting::default()
            }),
        };
        let code = Text {
            text: NonEmptyString::new("y").unwrap(),
            marks: TextMarks::Code { link: Some(link) },
        };
        assert_eq!(
            serde_json::to_value(&formatted).unwrap()["marks"],
            json!([{"type":"link","attrs":{"href":"https://example.com"}},{"type":"em"},{"type":"strong"},{"type":"strike"}])
        );
        assert_eq!(
            serde_json::to_value(&code).unwrap()["marks"],
            json!([{"type":"link","attrs":{"href":"https://example.com"}},{"type":"code"}])
        );
    }

    #[test]
    fn href_allowlist() {
        for ok in [
            "https://a.b",
            "HTTP://a",
            "mailto:a@b.c",
            "tel:+31",
            "/browse/X-1",
            "/a\\b",
            "#top",
            "ftp://x",
            "ftps://x",
        ] {
            assert!(Href::parse(ok).is_some(), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "data:text/html,x",
            "",
            " ",
            "//evil.com",
            "/\\evil.com",
            "/\\\\evil.com",
            " /\\evil.com",
            "relative/path",
            "http:",
            "https://",
            "vbscript:x",
        ] {
            assert!(Href::parse(bad).is_none(), "{bad}");
        }
    }

    /// Guards against a vacuous validator: known-bad documents must be rejected.
    #[test]
    fn schema_rejects_known_bad_documents() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/adf/full.json")).unwrap();
        let validator = jsonschema::draft4::new(&schema).unwrap();
        let p = |inline: serde_json::Value| json!({"type":"doc","version":1,"content":[{"type":"paragraph","content":[inline]}]});
        for bad in [
            p(json!({"type":"text","text":""})),
            p(json!({"type":"text","text":"x","marks":[{"type":"code"},{"type":"strong"}]})),
            p(json!({"type":"hardBreak","attrs":{"text":null}})),
            json!({"type":"doc","version":1,"content":[{"type":"blockquote","content":[{"type":"heading","attrs":{"level":1}}]}]}),
            json!({"type":"doc","version":1,"content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"table","content":[]}]}]}]}),
            json!({"type":"doc","version":1,"content":[{"type":"taskList","content":[{"type":"taskItem","attrs":{"state":"TODO"}}]}]}),
            json!({"type":"doc","version":2,"content":[]}),
        ] {
            assert!(!validator.is_valid(&bad), "accepted: {bad}");
        }
    }

    /// Every construct the model can produce must validate against the vendored schema.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn kitchen_sink_is_schema_valid() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../schemas/adf/full.json")).unwrap();
        let validator = jsonschema::draft4::new(&schema).unwrap();
        let link = Link::new(
            Href::parse("https://example.com").unwrap(),
            NonEmptyString::new("t"),
        );
        let task = TaskItem {
            local_id: "a".into(),
            state: TaskState::Done,
            content: vec![text("t")],
        };
        let doc = Doc {
            content: vec![
                DocBlock::Paragraph(AlignedParagraph {
                    content: vec![text("a"), Inline::HardBreak],
                    align: Some(Align::Center),
                }),
                DocBlock::Heading(Heading {
                    level: HeadingLevel::new(6).unwrap(),
                    content: vec![],
                }),
                DocBlock::BulletList(BulletList {
                    items: NonEmpty::new(ListItem {
                        first: Paragraph::default(),
                        rest: vec![
                            ListItemChild::OrderedList(OrderedList {
                                order: 3,
                                items: NonEmpty::new(item("n")),
                            }),
                            ListItemChild::CodeBlock(CodeBlock::default()),
                        ],
                    }),
                }),
                DocBlock::TaskList(TaskList {
                    local_id: "l".into(),
                    first: task.clone(),
                    rest: vec![TaskListChild::TaskList(TaskList {
                        local_id: "m".into(),
                        first: task,
                        rest: vec![],
                    })],
                }),
                DocBlock::CodeBlock(CodeBlock {
                    language: NonEmptyString::new("rust"),
                    text: NonEmptyString::new("fn main() {}\n// x"),
                }),
                DocBlock::Blockquote(Blockquote {
                    content: NonEmpty::new(BlockquoteChild::Paragraph(para("q"))),
                }),
                DocBlock::Rule(Rule),
                DocBlock::Panel(Panel {
                    panel_type: PanelType::Warning,
                    content: NonEmpty::new(PanelChild::Heading(Heading {
                        level: HeadingLevel::new(1).unwrap(),
                        content: vec![text("h")],
                    })),
                }),
                DocBlock::Table(Table {
                    rows: NonEmpty::from_vec(vec![
                        TableRow {
                            cells: NonEmpty::new(TableCell::empty(CellKind::Header)),
                        },
                        TableRow {
                            cells: NonEmpty::new(TableCell {
                                kind: CellKind::Data,
                                content: NonEmpty::from_vec(vec![
                                    TableCellChild::Paragraph(AlignedParagraph {
                                        content: vec![text("c")],
                                        align: Some(Align::End),
                                    }),
                                    TableCellChild::NestedExpand(NestedExpand {
                                        title: None,
                                        content: NonEmpty::new(NestedExpandChild::Paragraph(para(
                                            "x",
                                        ))),
                                    }),
                                ])
                                .unwrap(),
                            }),
                        },
                    ])
                    .unwrap(),
                }),
                DocBlock::Expand(Expand {
                    title: Some("T".into()),
                    content: NonEmpty::new(ExpandChild::Paragraph(para("e"))),
                }),
                DocBlock::MediaSingle(MediaSingle::external(
                    "https://example.com/a.png".into(),
                    NonEmptyString::new("alt"),
                )),
                DocBlock::MediaSingle(file_image(None, None, MediaLayout::Center, None)),
                DocBlock::MediaSingle(file_image(
                    MediaDimension::new(1512),
                    MediaDimension::new(945),
                    MediaLayout::Wide,
                    MediaSize::pixels(720),
                )),
                DocBlock::MediaSingle(file_image(
                    MediaDimension::new(1),
                    None,
                    MediaLayout::WrapLeft,
                    MediaSize::percent(100),
                )),
                DocBlock::MediaSingle(file_image(
                    None,
                    MediaDimension::new(MAX_MEDIA_PIXELS),
                    MediaLayout::FullWidth,
                    MediaSize::pixels(MAX_MEDIA_PIXELS),
                )),
                DocBlock::Paragraph(AlignedParagraph {
                    content: vec![
                        Inline::Text(Text {
                            text: NonEmptyString::new("f").unwrap(),
                            marks: TextMarks::Formatted(Formatting {
                                link: Some(link.clone()),
                                em: true,
                                strong: true,
                                text_color: HexColor::parse("#FF0000"),
                                background_color: HexColor::parse("#00ff00"),
                                strike: true,
                                subsup: Some(SubSup::Sup),
                                underline: true,
                            }),
                        }),
                        Inline::Text(Text {
                            text: NonEmptyString::new("c").unwrap(),
                            marks: TextMarks::Code { link: Some(link) },
                        }),
                        Inline::Mention(Mention {
                            id: NonEmptyString::new("abc").unwrap(),
                            text: Some("@A".into()),
                        }),
                        Inline::Emoji(Emoji {
                            short_name: NonEmptyString::new(":smile:").unwrap(),
                            id: None,
                            text: Some("😄".into()),
                        }),
                        Inline::Date(Date {
                            timestamp: NonEmptyString::new("1792368000000").unwrap(),
                        }),
                        Inline::Status(Status {
                            text: NonEmptyString::new("DONE").unwrap(),
                            color: StatusColor::Green,
                        }),
                        Inline::InlineCard(InlineCard {
                            url: Href::parse("https://example.com").unwrap(),
                        }),
                    ],
                    align: None,
                }),
            ],
        };
        let value = doc.to_value().unwrap();
        // `is_valid` only: `iter_errors` is exponential on nested invalid ADF.
        let errors: Vec<String> = if validator.is_valid(&value) {
            Vec::new()
        } else {
            crate::adf::validate::validate(&value, crate::Target::Jira)
                .into_iter()
                .map(|d| d.to_string())
                .collect()
        };
        assert!(
            errors.is_empty(),
            "{errors:#?}\n{}",
            serde_json::to_string_pretty(&value).unwrap()
        );
    }
}
