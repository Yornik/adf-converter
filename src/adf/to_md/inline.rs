//! Inline content: text with marks, hard breaks, and the inline nodes (mention, emoji, date,
//! status, inlineCard).
//!
//! Rendering works in three steps:
//! 1. the ADF inline nodes become *atoms* (text with a [`Style`], breaks, `adf:` links);
//! 2. per line (hard breaks close every mark), the marks are nested into *spans*, longest-running
//!    mark outermost, so the Markdown never has crossing delimiters;
//! 3. the line is written with `*`, `**`, `~~` where possible, then parsed back with
//!    pulldown-cmark ([`super::verify`]); spans that did not parse as intended switch to their
//!    HTML form (`<em>`, `<strong>`, `<del>`), which always parses.

use crate::adf::model::{HexColor, Href};
use crate::adf::reader::Node;

use super::escape::{
    TextContext, TextEscape, code_span, escape_text, has_unsafe_control, link_destination,
    link_title, normalize_cr,
};
use super::verify::{self, Run};
use super::{Ctx, Fail};

/// Where inline content is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineKind {
    /// Paragraphs and task items; hard breaks are `\` + newline.
    Block,
    /// ATX heading of the given level; hard breaks are `<br>`.
    Heading(u8),
    /// A GFM table cell; hard breaks are `<br>`.
    TableCell,
}

impl InlineKind {
    fn context(self) -> TextContext {
        match self {
            Self::Block => TextContext::Block,
            Self::Heading(_) => TextContext::Heading,
            Self::TableCell => TextContext::TableCell,
        }
    }

    fn break_markup(self) -> &'static str {
        match self {
            Self::Block => "\\\n",
            Self::Heading(_) | Self::TableCell => "<br>",
        }
    }

    fn in_table(self) -> bool {
        self == Self::TableCell
    }
}

/// A link mark's target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LinkTarget {
    pub href: String,
    pub title: Option<String>,
}

/// `subsup` mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubSup {
    Sub,
    Sup,
}

/// The marks of one atom, as far as Markdown can express them.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Style {
    pub link: Option<LinkTarget>,
    pub em: bool,
    pub strong: bool,
    pub strike: bool,
    pub underline: bool,
    pub subsup: Option<SubSup>,
    pub color: Option<String>,
    pub bg: Option<String>,
    pub code: bool,
}

/// One piece of inline content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Atom {
    /// Text (never empty, never containing `\n`). With `style.code` it is a code span.
    Text { text: String, style: Style },
    /// A hard break.
    Break,
    /// An `adf:` link (`[label](dest)`); these nodes carry no marks.
    Ext { label: String, dest: String },
    /// Markdown written verbatim (plain flavour only, e.g. `[DONE]`).
    Raw { text: String, style: Style },
    /// `<url>` (plain flavour only), which parses as a link to `url` with the URL as its text.
    Autolink { url: String },
}

impl Atom {
    fn style(&self) -> Option<&Style> {
        match self {
            Self::Text { style, .. } | Self::Raw { style, .. } => Some(style),
            Self::Break | Self::Ext { .. } | Self::Autolink { .. } => None,
        }
    }
}

/// Renders inline nodes. In roundtrip mode, content that cannot be written so that it parses
/// back to the same ADF fails with [`Fail::Unrepresentable`].
pub(super) fn render_inline(
    nodes: &[Node<'_>],
    kind: InlineKind,
    cx: &mut Ctx,
) -> Result<String, Fail> {
    let atoms = build_atoms(nodes, cx)?;
    render_atoms(atoms, kind, cx.roundtrip).ok_or_else(|| {
        Fail::unrepresentable(
            &nodes.first().map(Node::pointer).unwrap_or_default(),
            "its inline formatting could not be written as Markdown that parses back unchanged",
        )
    })
}

/// `true` when the inline content renders to nothing (no atoms after dropping trailing breaks).
pub(super) fn is_blank(nodes: &[Node<'_>]) -> bool {
    nodes.iter().all(|n| match n.ty {
        "hardBreak" => true,
        "text" => n.text.is_none_or(str::is_empty),
        _ => false,
    })
}

// ---------------------------------------------------------------------------------------------
// ADF nodes → atoms
// ---------------------------------------------------------------------------------------------

fn build_atoms(nodes: &[Node<'_>], cx: &mut Ctx) -> Result<Vec<Atom>, Fail> {
    let mut atoms = Vec::new();
    for node in nodes {
        push_node_atoms(node, cx, &mut atoms)?;
    }
    if !cx.roundtrip {
        atoms = shrink_delimited_whitespace(atoms);
    }
    let mut atoms = merge(atoms);
    while matches!(atoms.last(), Some(Atom::Break)) {
        atoms.pop();
    }
    Ok(atoms)
}

#[allow(clippy::too_many_lines)]
fn push_node_atoms(node: &Node<'_>, cx: &mut Ctx, atoms: &mut Vec<Atom>) -> Result<(), Fail> {
    match node.ty {
        "text" => {
            let style = parse_marks(node, cx);
            let (text, had_cr) = normalize_cr(node.text.unwrap_or(""));
            if had_cr {
                cx.warn_control_chars(
                    &node.pointer(),
                    "Carriage returns in text were written as line breaks",
                );
            }
            let text = text.as_ref();
            let title = style.link.as_ref().and_then(|l| l.title.as_deref());
            if cx.roundtrip && (has_unsafe_control(text) || title.is_some_and(has_unsafe_control)) {
                return Err(Fail::unrepresentable(
                    &node.pointer(),
                    "Markdown → ADF removes control characters from text",
                ));
            }
            match &style.link {
                // Plain: a link whose text is its URL reads best as an autolink.
                Some(link)
                    if !cx.roundtrip
                        && link.title.is_none()
                        && link.href == text
                        && style == link_only(&style)
                        && is_autolink_safe(text) =>
                {
                    atoms.push(Atom::Autolink {
                        url: text.to_owned(),
                    });
                }
                _ => push_text(text, &style, atoms),
            }
        }
        "hardBreak" => atoms.push(Atom::Break),
        "mention" => {
            ext_marks_dropped(node, cx);
            let id = node.attr_str("id").unwrap_or("");
            let text = node.attr_str("text").unwrap_or("");
            if cx.roundtrip {
                if !is_safe_path(id, "") || text.is_empty() || has_unsafe_control(text) {
                    return Err(Fail::unrepresentable(
                        &node.pointer(),
                        "a mention without an ID or display text has no Markdown form",
                    ));
                }
                cx.drop_attrs(node, &["id", "text"], &[("userType", "DEFAULT")]);
                atoms.push(Atom::Ext {
                    label: text.to_owned(),
                    dest: format!("adf:mention/{id}"),
                });
            } else {
                let name = if text.is_empty() { id } else { text };
                let label = if name.starts_with('@') {
                    name.to_owned()
                } else {
                    format!("@{name}")
                };
                push_text(&label, &Style::default(), atoms);
            }
        }
        "emoji" => {
            ext_marks_dropped(node, cx);
            let short = node.attr_str("shortName").unwrap_or("");
            let text = node.attr_str("text").unwrap_or("");
            if cx.roundtrip {
                let wrapped = short.len() > 2 && short.starts_with(':') && short.ends_with(':');
                if !wrapped || !is_safe_path(short, "+") || has_unsafe_control(text) {
                    return Err(Fail::unrepresentable(
                        &node.pointer(),
                        "an emoji without a :short-name: has no Markdown form",
                    ));
                }
                cx.drop_attrs(node, &["shortName", "text"], &[]);
                atoms.push(Atom::Ext {
                    label: text.to_owned(),
                    dest: format!("adf:emoji/{short}"),
                });
            } else {
                push_text(
                    if text.is_empty() { short } else { text },
                    &Style::default(),
                    atoms,
                );
            }
        }
        "date" => {
            ext_marks_dropped(node, cx);
            let millis = node
                .attr("timestamp")
                .and_then(super::date::timestamp_millis);
            let date = millis.and_then(super::date::civil_date);
            match (date, cx.roundtrip) {
                (Some((label, true)), true) => {
                    cx.drop_attrs(node, &["timestamp"], &[]);
                    atoms.push(Atom::Ext {
                        label,
                        dest: "adf:date".to_owned(),
                    });
                }
                (Some((label, _)), false) => push_text(&label, &Style::default(), atoms),
                (None, false) => {
                    let raw = node
                        .attr("timestamp")
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    push_text(raw.trim_matches('"'), &Style::default(), atoms);
                }
                (Some((_, false)) | None, true) => {
                    return Err(Fail::unrepresentable(
                        &node.pointer(),
                        "a date that is not midnight UTC has no Markdown form",
                    ));
                }
            }
        }
        "status" => {
            ext_marks_dropped(node, cx);
            let text = node.attr_str("text").unwrap_or("");
            let color = node.attr_str("color").unwrap_or("");
            if cx.roundtrip {
                let named = matches!(
                    color,
                    "neutral" | "purple" | "blue" | "red" | "yellow" | "green"
                );
                if text.is_empty() || !named || has_unsafe_control(text) {
                    return Err(Fail::unrepresentable(
                        &node.pointer(),
                        "a status without text or with a non-named colour has no Markdown form",
                    ));
                }
                cx.drop_attrs(node, &["text", "color"], &[]);
                atoms.push(Atom::Ext {
                    label: text.to_owned(),
                    dest: format!("adf:status/{color}"),
                });
            } else if !text.is_empty() {
                let mut raw = String::from("[");
                escape_text(
                    text,
                    TextEscape {
                        context: TextContext::Block,
                        roundtrip: false,
                        line_start: false,
                        block_start: false,
                        line_end: false,
                        in_link: true,
                    },
                    &mut raw,
                );
                raw.push(']');
                atoms.push(Atom::Raw {
                    text: raw,
                    style: Style::default(),
                });
            }
        }
        "inlineCard" => {
            ext_marks_dropped(node, cx);
            let url = node.attr_str("url").filter(|u| Href::parse(u).is_some());
            match (url, cx.roundtrip) {
                (Some(url), true) => {
                    cx.drop_attrs(node, &["url"], &[]);
                    atoms.push(Atom::Ext {
                        label: url.to_owned(),
                        dest: "adf:card".to_owned(),
                    });
                }
                // Plain: the URL is untrusted text, so it is escaped unless an autolink holds it.
                (Some(url), false) if plain_autolink(url) => atoms.push(Atom::Autolink {
                    url: url.to_owned(),
                }),
                (Some(url), false) => push_text(url, &Style::default(), atoms),
                (None, true) => {
                    return Err(Fail::unrepresentable(
                        &node.pointer(),
                        "a smart link without an allowed URL has no Markdown form",
                    ));
                }
                (None, false) => {}
            }
        }
        other => {
            if cx.roundtrip {
                return Err(Fail::unrepresentable(
                    &node.pointer(),
                    &format!("inline node '{other}' has no Markdown form"),
                ));
            }
            if other != "placeholder" {
                push_text(&node.text_content(), &Style::default(), atoms);
            }
        }
    }
    Ok(())
}

/// `style` reduced to its link mark.
fn link_only(style: &Style) -> Style {
    Style {
        link: style.link.clone(),
        ..Style::default()
    }
}

/// Plain flavour: whether a card URL is written as an autolink `<url>`, because it holds
/// characters that would need escaping as text and an autolink can hold it.
pub(super) fn plain_autolink(url: &str) -> bool {
    url.chars().any(|c| "*_~`[]<>|\\&".contains(c)) && is_autolink_safe(url)
}

/// A URL that can be written as a CommonMark autolink `<…>` (absolute URI, no spaces or angle
/// brackets; backslash escapes do not work inside autolinks).
fn is_autolink_safe(url: &str) -> bool {
    let scheme_ok = url.split_once(':').is_some_and(|(scheme, _)| {
        (2..=32).contains(&scheme.len())
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
    });
    scheme_ok
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '\\'))
}

/// Splits text at `\n` into text atoms and breaks.
fn push_text(text: &str, style: &Style, atoms: &mut Vec<Atom>) {
    for (i, part) in text.split('\n').enumerate() {
        if i > 0 {
            atoms.push(Atom::Break);
        }
        if !part.is_empty() {
            atoms.push(Atom::Text {
                text: part.to_owned(),
                style: style.clone(),
            });
        }
    }
}

/// Account IDs and short names go into an `adf:` URL path; only allow characters that need no
/// escaping there.
fn is_safe_path(value: &str, extra: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.') || extra.contains(c)
        })
}

fn ext_marks_dropped(node: &Node<'_>, cx: &mut Ctx) {
    if !node.marks.is_empty() {
        let names: Vec<&str> = node.marks.iter().map(|m| m.ty).collect();
        cx.warn_attr_dropped(
            &format!("{}/marks", node.pointer()),
            &format!(
                "Marks on a {} node have no Markdown form and were dropped: {}",
                node.ty,
                names.join(", ")
            ),
        );
    }
}

/// Reads the marks of a text node, warning (roundtrip) about marks Markdown cannot express.
fn parse_marks(node: &Node<'_>, cx: &mut Ctx) -> Style {
    let mut style = Style::default();
    for (i, mark) in node.marks.iter().enumerate() {
        // Only built for a warning: most marks need none.
        let pointer = || format!("{}/marks/{i}", node.pointer());
        match mark.ty {
            "link" => {
                let href = mark.attr_str("href").filter(|h| Href::parse(h).is_some());
                if let Some(href) = href {
                    style.link = Some(LinkTarget {
                        href: href.to_owned(),
                        title: mark
                            .attr_str("title")
                            .filter(|t| !t.is_empty())
                            .map(str::to_owned),
                    });
                    let extra: Vec<&str> = mark
                        .attr_names()
                        .filter(|n| !matches!(*n, "href" | "title"))
                        .collect();
                    if !extra.is_empty() {
                        cx.warn_attr_dropped(
                            &pointer(),
                            &format!(
                                "Link attributes without a Markdown form were dropped: {}",
                                extra.join(", ")
                            ),
                        );
                    }
                } else {
                    cx.warn_attr_dropped(
                        &pointer(),
                        "A link whose target is not an allowed URL was dropped; the text is kept",
                    );
                }
            }
            "em" => style.em = true,
            "strong" => style.strong = true,
            "strike" => style.strike = true,
            "code" => style.code = true,
            "underline" => {
                if cx.roundtrip {
                    style.underline = true;
                }
            }
            "subsup" => match mark.attr_str("type") {
                Some("sub") => style.subsup = Some(SubSup::Sub),
                Some("sup") => style.subsup = Some(SubSup::Sup),
                _ => {
                    cx.warn_attr_dropped(
                        &pointer(),
                        "A subsup mark without a valid type was dropped",
                    );
                }
            },
            "textColor" | "backgroundColor" => {
                let color = mark.attr_str("color").and_then(HexColor::parse);
                match color {
                    Some(color) if cx.roundtrip => {
                        let color = color.as_str().to_owned();
                        if mark.ty == "textColor" {
                            style.color = Some(color);
                        } else {
                            style.bg = Some(color);
                        }
                    }
                    Some(_) => {}
                    None => cx.warn_attr_dropped(
                        &pointer(),
                        &format!("A {} mark without a #RRGGBB colour was dropped", mark.ty),
                    ),
                }
            }
            other => cx.warn_attr_dropped(
                &pointer(),
                &format!("Mark '{other}' has no Markdown form and was dropped"),
            ),
        }
    }
    if style.code {
        // ADF allows only `link` next to `code`; anything else is invalid input.
        style = Style {
            link: style.link,
            code: true,
            ..Style::default()
        };
    }
    style
}

/// Merges adjacent text atoms with the same style (ADF does the same).
fn merge(atoms: Vec<Atom>) -> Vec<Atom> {
    let mut out: Vec<Atom> = Vec::with_capacity(atoms.len());
    for atom in atoms {
        if let (
            Some(Atom::Text { text, style }),
            Atom::Text {
                text: more,
                style: next,
            },
        ) = (out.last_mut(), &atom)
            && style == next
        {
            text.push_str(more);
            continue;
        }
        out.push(atom);
    }
    out
}

/// Plain flavour: moves whitespace at the edges of `*`/`**`/`~~` spans outside the span, so the
/// delimiters can be used instead of HTML tags.
fn shrink_delimited_whitespace(atoms: Vec<Atom>) -> Vec<Atom> {
    let styles: Vec<Option<Style>> = atoms.iter().map(|a| a.style().cloned()).collect();
    let mut out = Vec::with_capacity(atoms.len());
    for (i, atom) in atoms.into_iter().enumerate() {
        let Atom::Text { text, style } = atom else {
            out.push(atom);
            continue;
        };
        if style.code {
            out.push(Atom::Text { text, style });
            continue;
        }
        let prev = i
            .checked_sub(1)
            .and_then(|p| styles.get(p))
            .cloned()
            .flatten();
        let next = styles.get(i + 1).cloned().flatten();
        let opening = without_marks(&style, prev.as_ref());
        let closing = without_marks(&style, next.as_ref());
        let body = text.trim_matches(char::is_whitespace);
        if body.is_empty() {
            let mut bare = opening;
            bare.em &= closing.em;
            bare.strong &= closing.strong;
            bare.strike &= closing.strike;
            out.push(Atom::Text { text, style: bare });
            continue;
        }
        let lead_len = text.len() - text.trim_start_matches(char::is_whitespace).len();
        let trail_start = text.trim_end_matches(char::is_whitespace).len();
        if let Some(lead) = text.get(..lead_len).filter(|s| !s.is_empty()) {
            out.push(Atom::Text {
                text: lead.to_owned(),
                style: opening,
            });
        }
        out.push(Atom::Text {
            text: body.to_owned(),
            style: style.clone(),
        });
        if let Some(trail) = text.get(trail_start..).filter(|s| !s.is_empty()) {
            out.push(Atom::Text {
                text: trail.to_owned(),
                style: closing,
            });
        }
    }
    out
}

/// `style` without the em/strong/strike marks that `neighbour` does not share (those are the
/// spans that open or close at this edge).
fn without_marks(style: &Style, neighbour: Option<&Style>) -> Style {
    let mut out = style.clone();
    let keep = |mine: bool, theirs: Option<bool>| mine && theirs.unwrap_or(false);
    out.em = keep(style.em, neighbour.map(|n| n.em));
    out.strong = keep(style.strong, neighbour.map(|n| n.strong));
    out.strike = keep(style.strike, neighbour.map(|n| n.strike));
    out
}

// ---------------------------------------------------------------------------------------------
// Atoms → Markdown
// ---------------------------------------------------------------------------------------------

/// A mark that is opened and closed as one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkKey {
    Color(String),
    Bg(String),
    Underline,
    Sub,
    Sup,
    Link(LinkTarget),
    Strike,
    Strong,
    Em,
}

impl MarkKey {
    /// Outer marks first when several open at the same atom and run equally long: HTML tags
    /// outside, then link brackets, then the delimiter marks (which flank text best).
    fn rank(&self) -> u8 {
        match self {
            Self::Color(_) => 0,
            Self::Bg(_) => 1,
            Self::Underline => 2,
            Self::Sub | Self::Sup => 3,
            Self::Link(_) => 4,
            Self::Strike => 5,
            Self::Strong => 6,
            Self::Em => 7,
        }
    }

    fn has_delimiter_form(&self) -> bool {
        matches!(self, Self::Em | Self::Strong | Self::Strike)
    }
}

fn mark_keys(style: &Style) -> Vec<MarkKey> {
    let mut keys = Vec::new();
    if let Some(color) = &style.color {
        keys.push(MarkKey::Color(color.clone()));
    }
    if let Some(color) = &style.bg {
        keys.push(MarkKey::Bg(color.clone()));
    }
    if style.underline {
        keys.push(MarkKey::Underline);
    }
    match style.subsup {
        Some(SubSup::Sub) => keys.push(MarkKey::Sub),
        Some(SubSup::Sup) => keys.push(MarkKey::Sup),
        None => {}
    }
    if let Some(link) = &style.link {
        keys.push(MarkKey::Link(link.clone()));
    }
    if style.strike {
        keys.push(MarkKey::Strike);
    }
    if style.strong {
        keys.push(MarkKey::Strong);
    }
    if style.em {
        keys.push(MarkKey::Em);
    }
    keys
}

#[derive(Debug, Clone)]
struct Span {
    key: MarkKey,
    line: usize,
    first: usize,
    last: usize,
}

#[derive(Debug, Clone, Copy)]
enum Ev {
    Open(usize),
    Close(usize),
    Atom(usize),
}

#[derive(Debug)]
struct Layout {
    lines: Vec<Vec<Atom>>,
    events: Vec<Vec<Ev>>,
    spans: Vec<Span>,
}

fn layout(atoms: Vec<Atom>) -> Layout {
    let mut lines: Vec<Vec<Atom>> = vec![Vec::new()];
    for atom in atoms {
        if atom == Atom::Break {
            lines.push(Vec::new());
        } else if let Some(line) = lines.last_mut() {
            line.push(atom);
        }
    }
    let mut spans = Vec::new();
    let mut events = Vec::with_capacity(lines.len());
    for (line_no, line) in lines.iter().enumerate() {
        let keys: Vec<Vec<MarkKey>> = line
            .iter()
            .map(|a| a.style().map(mark_keys).unwrap_or_default())
            .collect();
        let mut evs = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        for (i, want) in keys.iter().enumerate() {
            let keep = open
                .iter()
                .take_while(|&&id| spans.get(id).is_some_and(|s: &Span| want.contains(&s.key)))
                .count();
            while open.len() > keep {
                if let Some(id) = open.pop() {
                    if let Some(span) = spans.get_mut(id) {
                        span.last = i.saturating_sub(1);
                    }
                    evs.push(Ev::Close(id));
                }
            }
            let mut new: Vec<&MarkKey> = want
                .iter()
                .filter(|k| {
                    !open
                        .iter()
                        .any(|&id| spans.get(id).is_some_and(|s: &Span| &s.key == *k))
                })
                .collect();
            let extent = |k: &MarkKey| keys.iter().skip(i).take_while(|w| w.contains(k)).count();
            new.sort_by(|a, b| extent(b).cmp(&extent(a)).then(a.rank().cmp(&b.rank())));
            for key in new {
                let id = spans.len();
                spans.push(Span {
                    key: key.clone(),
                    line: line_no,
                    first: i,
                    last: i,
                });
                open.push(id);
                evs.push(Ev::Open(id));
            }
            evs.push(Ev::Atom(i));
        }
        while let Some(id) = open.pop() {
            if let Some(span) = spans.get_mut(id) {
                span.last = line.len().saturating_sub(1);
            }
            evs.push(Ev::Close(id));
        }
        events.push(evs);
    }
    Layout {
        lines,
        events,
        spans,
    }
}

/// Renders atoms, verifying the result. `None` when no rendering parses back unchanged
/// (roundtrip only; plain returns its best attempt).
fn render_atoms(atoms: Vec<Atom>, kind: InlineKind, roundtrip: bool) -> Option<String> {
    if atoms.is_empty() {
        return Some(String::new());
    }
    let layout = layout(atoms);
    let intended = intended_runs(&layout);
    let mut html: Vec<bool> = layout
        .spans
        .iter()
        .map(|s| s.key.has_delimiter_form() && edge_whitespace(&layout, s))
        .collect();
    let mut last = String::new();
    for attempt in 0..4 {
        let md = write_layout(&layout, &html, kind, roundtrip);
        let parsed = verify::parse_inline(&md, kind);
        let Some(parsed) = parsed else {
            last = md;
            if !switch_all(&layout, &mut html) {
                break;
            }
            continue;
        };
        match verify::first_differences(&intended.runs, &parsed) {
            None => return Some(md),
            Some(positions) => {
                last = md;
                let changed = if attempt < 2 {
                    switch_at(&layout, &intended, &positions, &mut html)
                } else {
                    false
                } || switch_all(&layout, &mut html);
                if !changed {
                    break;
                }
            }
        }
    }
    (!roundtrip).then_some(last)
}

/// Whether a span starts or ends with whitespace (the delimiter could not open/close there).
fn edge_whitespace(layout: &Layout, span: &Span) -> bool {
    let line = layout.lines.get(span.line);
    let text_of = |i: usize| match line.and_then(|l| l.get(i)) {
        Some(Atom::Text { text, style }) if !style.code => Some(text.as_str()),
        _ => None,
    };
    text_of(span.first).is_some_and(|t| t.starts_with(char::is_whitespace))
        || text_of(span.last).is_some_and(|t| t.ends_with(char::is_whitespace))
}

/// Switches every delimiter span to HTML. Returns whether anything changed.
fn switch_all(layout: &Layout, html: &mut [bool]) -> bool {
    let mut changed = false;
    for (span, form) in layout.spans.iter().zip(html.iter_mut()) {
        if span.key.has_delimiter_form() && !*form {
            *form = true;
            changed = true;
        }
    }
    changed
}

/// Switches the delimiter spans covering the mismatched character positions to HTML.
fn switch_at(layout: &Layout, intended: &Intended, positions: &[usize], html: &mut [bool]) -> bool {
    let mut changed = false;
    for &pos in positions {
        let Some(&(line, atom)) = intended.owner.get(pos) else {
            continue;
        };
        for (span, form) in layout.spans.iter().zip(html.iter_mut()) {
            if span.key.has_delimiter_form()
                && !*form
                && span.line == line
                && (span.first..=span.last).contains(&atom)
            {
                *form = true;
                changed = true;
            }
        }
    }
    changed
}

/// Which atoms sit at the edges of the block, where Markdown → ADF trims spaces and tabs even
/// inside marks (`md/finalize.rs`): on the first line, every atom up to and including the first
/// one that is not blank text; on the last line, every atom from the last such one on.
#[derive(Debug, Clone, Copy)]
struct BlockEdges {
    /// Index (on the first line) of the first atom that is not blank text.
    lead_until: usize,
    /// Index (on the last line) of the last atom that is not blank text.
    trail_from: usize,
}

impl BlockEdges {
    fn of(layout: &Layout) -> Self {
        let lead_until = layout
            .lines
            .first()
            .and_then(|line| line.iter().position(|a| !is_blank_text(a)))
            .unwrap_or(usize::MAX);
        let trail_from = layout
            .lines
            .last()
            .and_then(|line| line.iter().rposition(|a| !is_blank_text(a)))
            .unwrap_or(0);
        Self {
            lead_until,
            trail_from,
        }
    }
}

/// Text (not code) made only of spaces and tabs: Markdown → ADF trims all of it at an edge.
fn is_blank_text(atom: &Atom) -> bool {
    matches!(atom, Atom::Text { text, style } if !style.code && text.chars().all(|c| c == ' ' || c == '\t'))
}

/// Where a text atom sits, for escaping its edges.
#[derive(Debug, Clone, Copy)]
struct Edges {
    /// Only opening marks and blank text precede it in the block.
    block_start: bool,
    /// Only closing marks and blank text follow it in the block.
    block_end: bool,
}

fn write_layout(layout: &Layout, html: &[bool], kind: InlineKind, roundtrip: bool) -> String {
    let edges = BlockEdges::of(layout);
    let mut out = String::new();
    for (line_no, (line, events)) in layout.lines.iter().zip(&layout.events).enumerate() {
        if line_no > 0 {
            out.push_str(kind.break_markup());
        }
        // Whitespace before a hard break is kept by the parser; only the block's end trims it.
        let last_line = line_no + 1 == layout.lines.len();
        let mut buf = String::new();
        for (pos, ev) in events.iter().enumerate() {
            let at_end = last_line && pos + 1 == events.len();
            match *ev {
                // Roundtrip keeps every space; plain only guards the line edges the parser
                // strips (readability over exactness inside marks).
                Ev::Atom(i) if roundtrip => {
                    if let Some(atom) = line.get(i) {
                        let at = Edges {
                            block_start: line_no == 0 && i <= edges.lead_until,
                            block_end: last_line && i >= edges.trail_from,
                        };
                        write_atom(atom, kind, roundtrip, at, &mut buf);
                    }
                }
                Ev::Open(id) => {
                    if let Some(span) = layout.spans.get(id) {
                        let html_form = html.get(id).copied().unwrap_or(true);
                        open_mark(&span.key, html_form, &mut buf);
                    }
                }
                Ev::Close(id) => {
                    if let Some(span) = layout.spans.get(id) {
                        let html_form = html.get(id).copied().unwrap_or(true);
                        close_mark(&span.key, html_form, kind, &mut buf);
                    }
                }
                Ev::Atom(i) => {
                    if let Some(atom) = line.get(i) {
                        let at = Edges {
                            block_start: false,
                            block_end: at_end,
                        };
                        write_atom(atom, kind, roundtrip, at, &mut buf);
                    }
                }
            }
        }
        out.push_str(&buf);
    }
    out
}

/// Before a `[`: a `!` written just before would turn the link into an image.
fn guard_bang(buf: &mut String) {
    if buf.ends_with('!') {
        buf.pop();
        buf.push_str("\\!");
    }
}

fn open_mark(key: &MarkKey, html: bool, buf: &mut String) {
    match key {
        MarkKey::Color(c) => {
            buf.push_str("<span style=\"color: ");
            buf.push_str(c);
            buf.push_str("\">");
        }
        MarkKey::Bg(c) => {
            buf.push_str("<span style=\"background-color: ");
            buf.push_str(c);
            buf.push_str("\">");
        }
        MarkKey::Underline => buf.push_str("<u>"),
        MarkKey::Sub => buf.push_str("<sub>"),
        MarkKey::Sup => buf.push_str("<sup>"),
        MarkKey::Link(_) => {
            guard_bang(buf);
            buf.push('[');
        }
        MarkKey::Strike => buf.push_str(if html { "<del>" } else { "~~" }),
        MarkKey::Strong => buf.push_str(if html { "<strong>" } else { "**" }),
        MarkKey::Em => buf.push_str(if html { "<em>" } else { "*" }),
    }
}

fn close_mark(key: &MarkKey, html: bool, kind: InlineKind, buf: &mut String) {
    match key {
        MarkKey::Color(_) | MarkKey::Bg(_) => buf.push_str("</span>"),
        MarkKey::Underline => buf.push_str("</u>"),
        MarkKey::Sub => buf.push_str("</sub>"),
        MarkKey::Sup => buf.push_str("</sup>"),
        MarkKey::Link(link) => {
            buf.push_str("](");
            buf.push_str(&link_destination(&link.href, kind.in_table()));
            if let Some(title) = &link.title {
                buf.push(' ');
                buf.push_str(&link_title(title, kind.in_table()));
            }
            buf.push(')');
        }
        MarkKey::Strike => buf.push_str(if html { "</del>" } else { "~~" }),
        MarkKey::Strong => buf.push_str(if html { "</strong>" } else { "**" }),
        MarkKey::Em => buf.push_str(if html { "</em>" } else { "*" }),
    }
}

fn write_atom(atom: &Atom, kind: InlineKind, roundtrip: bool, at: Edges, buf: &mut String) {
    match atom {
        Atom::Text { text, style } if style.code => {
            buf.push_str(&code_span(text, kind.in_table()));
        }
        Atom::Text { text, style } => {
            // Plain: after a `[TEXT]` status, `(` would make it a link and `:` a link
            // reference definition (which renders as nothing).
            if !roundtrip && buf.ends_with(']') && text.starts_with(['(', ':']) {
                buf.push('\\');
            }
            let opts = TextEscape {
                context: kind.context(),
                roundtrip,
                line_start: buf.is_empty(),
                block_start: at.block_start,
                line_end: at.block_end,
                in_link: style.link.is_some(),
            };
            escape_text(text, opts, buf);
        }
        Atom::Autolink { url } => {
            buf.push('<');
            buf.push_str(url);
            buf.push('>');
        }
        Atom::Ext { label, dest } => {
            guard_bang(buf);
            buf.push('[');
            let opts = TextEscape {
                context: kind.context(),
                roundtrip,
                line_start: false,
                block_start: false,
                line_end: false,
                in_link: true,
            };
            escape_text(label, opts, buf);
            buf.push_str("](");
            buf.push_str(&link_destination(dest, kind.in_table()));
            buf.push(')');
        }
        Atom::Raw { text, .. } => {
            if text.starts_with('[') {
                guard_bang(buf);
            }
            buf.push_str(text);
        }
        Atom::Break => {}
    }
}

// ---------------------------------------------------------------------------------------------
// What the Markdown should parse back to
// ---------------------------------------------------------------------------------------------

/// The intended parse: runs, plus for each character position the atom it came from.
pub(super) struct Intended {
    pub runs: Vec<Run>,
    /// `(line, atom index)` per character position (breaks count as one position).
    owner: Vec<(usize, usize)>,
}

fn intended_runs(layout: &Layout) -> Intended {
    let mut runs: Vec<Run> = Vec::new();
    let mut owner = Vec::new();
    for (line_no, line) in layout.lines.iter().enumerate() {
        if line_no > 0 {
            runs.push(Run::Break);
            owner.push((line_no, 0));
        }
        for (i, atom) in line.iter().enumerate() {
            let (text, style) = match atom {
                Atom::Text { text, style } | Atom::Raw { text, style } => {
                    (text.clone(), style.clone())
                }
                Atom::Ext { label, dest } => (
                    label.clone(),
                    Style {
                        link: Some(LinkTarget {
                            href: dest.clone(),
                            title: None,
                        }),
                        ..Style::default()
                    },
                ),
                Atom::Autolink { url } => (
                    url.clone(),
                    Style {
                        link: Some(LinkTarget {
                            href: url.clone(),
                            title: None,
                        }),
                        ..Style::default()
                    },
                ),
                Atom::Break => continue,
            };
            owner.extend(std::iter::repeat_n((line_no, i), text.chars().count()));
            verify::push_run(&mut runs, &text, &style);
        }
    }
    Intended { runs, owner }
}
