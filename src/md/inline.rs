//! Inline content: turns pulldown-cmark inline events into a flat list of [`Tok`]s.
//!
//! Marks come from a stack of open frames (Markdown emphasis, links, images, recognised HTML
//! tags). The effective mark set is kept in counters, so a token costs O(1) however deep the
//! nesting is, and HTML frames can be closed out of order (`<u>*a</u> b*`).

use linkify::{LinkFinder, LinkKind};

use crate::adf::model::{
    ALLOWED_URL_FORMS, Formatting, HexColor, Href, Link, MediaSingle, MediaSource, NonEmptyString,
    SubSup,
};
use crate::diagnostics::{Code, Diagnostic};
use crate::options::{HtmlPolicy, ImagePolicy, SoftBreak};

use super::cx::Cx;
use super::extensions::{build_link_node, split_adf_url, truncate};
use super::html::{HtmlMark, HtmlTag, Recognised, classify, closes_by_name, parse_tag};
use super::ir::{Marks, Span, Tok};
use super::media;

/// What a frame adds to the mark set; used to undo it when the frame closes.
#[derive(Debug, Clone, Copy, Default)]
#[allow(clippy::struct_excessive_bools)]
struct Adds {
    em: bool,
    strong: bool,
    strike: bool,
    underline: bool,
    code: bool,
    link: bool,
    subsup: bool,
    color: bool,
    background: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageMode {
    Link,
    External,
    Drop,
    /// `adf:media/<id>…`: media from Media Services.
    File,
    /// `adf:media/<id>…` while the `media` extension is disabled: only the alt text is kept.
    FileDisabled,
}

#[derive(Debug)]
struct LinkFrame {
    /// `adf:` extension: (kind, argument).
    adf: Option<(String, Option<String>)>,
    /// Text shown when the link text is empty.
    display: String,
    link: Option<Link>,
    start_tok: usize,
    span: Span,
}

#[derive(Debug)]
struct ImageFrame {
    mode: ImageMode,
    url: String,
    link: Option<Link>,
    /// Alt text, collected when the image does not become text. While the frame is open the
    /// text goes to [`MarkStack::captures`]; it is moved here when the frame is removed.
    alt: String,
    /// The image has a title (which external media cannot carry).
    titled: bool,
    start_tok: usize,
    span: Span,
}

#[derive(Debug)]
enum FrameKind {
    Em,
    Strong,
    Strike,
    Link(LinkFrame),
    Image(ImageFrame),
    /// An HTML tag frame, closed by `</name>`. A `literal` frame stands for an unrecognised
    /// opening tag (`<span class="x">`): it adds no mark, and its closing tag is handled by the
    /// HTML policy instead of closing a recognised frame of the same name.
    Html {
        name: String,
        literal: bool,
    },
}

/// What [`InlineBuilder::close_html`] found for a closing tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Closed {
    /// It closed a recognised mark frame.
    Mark,
    /// It closed the frame of an unrecognised opening tag; the HTML policy applies to it.
    Literal,
    /// No frame of that name is open.
    NotOpen,
}

/// What [`InlineBuilder::unknown_tag`] did with a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unknown {
    /// The tag was handled by the HTML policy.
    Reported,
    /// `<script>` or `<style>` under `--html drop`: its content is dropped up to the closing tag.
    RawTextOpened,
    /// The closing tag of such an element; its content was dropped.
    RawTextClosed,
}

/// An open `<script>` or `<style>` element whose content is dropped (`--html drop`).
#[derive(Debug)]
struct RawText {
    name: String,
    /// Tokens from this index on belong to the element.
    start_tok: usize,
    /// The opening tag, for messages.
    shown: String,
    span: Span,
}

/// Elements whose content is not shown by browsers, so `--html drop` removes it with them.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style"];

#[derive(Debug)]
struct Frame {
    id: u64,
    kind: FrameKind,
    adds: Adds,
}

/// The open frames and the mark set they produce.
#[derive(Debug, Default)]
struct MarkStack {
    frames: Vec<Frame>,
    next_id: u64,
    em: u32,
    strong: u32,
    strike: u32,
    underline: u32,
    code: u32,
    links: Vec<(u64, Link)>,
    subsup: Vec<(u64, SubSup)>,
    colors: Vec<(u64, HexColor)>,
    backgrounds: Vec<(u64, HexColor)>,
    /// Open links and images: no autolinking inside them.
    in_link: u32,
    /// Open `adf:` links (also counted in `in_link`).
    adf_links: u32,
    /// Open images whose alt text is captured instead of emitted.
    capturing: u32,
    /// The alt text of each capturing image frame, innermost last, so text inside one is
    /// appended in constant time however deep the emphasis inside it is nested.
    captures: Vec<(u64, String)>,
    /// Open HTML tag frames (bounded by [`MAX_HTML_FRAMES`]).
    html: usize,
    /// Open HTML tag frames per tag name (at most one entry per recognised tag).
    html_names: Vec<(String, usize)>,
}

/// Most HTML tag frames open at once in one block. Closing a tag searches the open frames, so
/// this keeps pathological input (thousands of unclosed tags) linear; real documents never get
/// close.
const MAX_HTML_FRAMES: usize = 256;

/// Values a new frame contributes.
#[derive(Debug, Default)]
struct Contrib {
    adds: Adds,
    link: Option<Link>,
    subsup: Option<SubSup>,
    color: Option<HexColor>,
    background: Option<HexColor>,
}

fn bump(counter: &mut u32, on: bool, up: bool) {
    if on {
        if up {
            *counter = counter.saturating_add(1);
        } else {
            *counter = counter.saturating_sub(1);
        }
    }
}

fn remove_value<T>(stack: &mut Vec<(u64, T)>, id: u64) {
    if let Some(pos) = stack.iter().rposition(|(i, _)| *i == id) {
        stack.remove(pos);
    }
}

impl MarkStack {
    fn push(&mut self, kind: FrameKind, contrib: Contrib) {
        let id = self.next_id;
        self.next_id += 1;
        let Contrib {
            mut adds,
            link,
            subsup,
            color,
            background,
        } = contrib;
        self.apply(adds, true);
        if let Some(link) = link {
            self.links.push((id, link));
            adds.link = true;
        }
        if let Some(v) = subsup {
            self.subsup.push((id, v));
            adds.subsup = true;
        }
        if let Some(v) = color {
            self.colors.push((id, v));
            adds.color = true;
        }
        if let Some(v) = background {
            self.backgrounds.push((id, v));
            adds.background = true;
        }
        match &kind {
            FrameKind::Link(link) => {
                self.in_link += 1;
                if link.adf.is_some() {
                    self.adf_links += 1;
                }
            }
            FrameKind::Image(image) => {
                self.in_link += 1;
                if image.mode != ImageMode::Link {
                    self.capturing += 1;
                    self.captures.push((id, String::new()));
                }
            }
            FrameKind::Html { name, .. } => {
                self.html += 1;
                match self.html_names.iter_mut().find(|(n, _)| n == name) {
                    Some((_, count)) => *count += 1,
                    None => self.html_names.push((name.clone(), 1)),
                }
            }
            FrameKind::Em | FrameKind::Strong | FrameKind::Strike => {}
        }
        self.frames.push(Frame { id, kind, adds });
    }

    fn apply(&mut self, adds: Adds, up: bool) {
        bump(&mut self.em, adds.em, up);
        bump(&mut self.strong, adds.strong, up);
        bump(&mut self.strike, adds.strike, up);
        bump(&mut self.underline, adds.underline, up);
        bump(&mut self.code, adds.code, up);
    }

    /// Removes the frame at `index`, undoing its contribution.
    fn remove(&mut self, index: usize) -> Option<Frame> {
        if index >= self.frames.len() {
            return None;
        }
        let mut frame = self.frames.remove(index);
        self.apply(frame.adds, false);
        if frame.adds.link {
            remove_value(&mut self.links, frame.id);
        }
        if frame.adds.subsup {
            remove_value(&mut self.subsup, frame.id);
        }
        if frame.adds.color {
            remove_value(&mut self.colors, frame.id);
        }
        if frame.adds.background {
            remove_value(&mut self.backgrounds, frame.id);
        }
        let id = frame.id;
        match &mut frame.kind {
            FrameKind::Link(link) => {
                self.in_link = self.in_link.saturating_sub(1);
                if link.adf.is_some() {
                    self.adf_links = self.adf_links.saturating_sub(1);
                }
            }
            FrameKind::Image(image) => {
                self.in_link = self.in_link.saturating_sub(1);
                if image.mode != ImageMode::Link {
                    self.capturing = self.capturing.saturating_sub(1);
                    if let Some(pos) = self.captures.iter().rposition(|(i, _)| *i == id) {
                        image.alt = self.captures.remove(pos).1;
                    }
                }
            }
            FrameKind::Html { name, .. } => {
                self.html = self.html.saturating_sub(1);
                if let Some((_, count)) = self.html_names.iter_mut().find(|(n, _)| n == name) {
                    *count = count.saturating_sub(1);
                }
            }
            FrameKind::Em | FrameKind::Strong | FrameKind::Strike => {}
        }
        Some(frame)
    }

    /// Pops frames from the top until one matching `pred` has been popped; returns it. Frames
    /// above it (unclosed HTML tags) are closed with it.
    fn pop_until(&mut self, pred: impl Fn(&FrameKind) -> bool) -> Option<Frame> {
        let index = self.frames.iter().rposition(|f| pred(&f.kind))?;
        while self.frames.len() > index + 1 {
            self.remove(self.frames.len() - 1);
        }
        self.remove(index)
    }

    fn marks(&self) -> Marks {
        Marks {
            fmt: Formatting {
                link: self.links.last().map(|(_, l)| l.clone()),
                em: self.em > 0,
                strong: self.strong > 0,
                text_color: self.colors.last().map(|(_, c)| c.clone()),
                background_color: self.backgrounds.last().map(|(_, c)| c.clone()),
                strike: self.strike > 0,
                subsup: self.subsup.last().map(|(_, s)| *s),
                underline: self.underline > 0,
            },
            code: self.code > 0,
        }
    }

    /// The alt text of the innermost capturing image frame.
    fn capture_alt(&mut self) -> Option<&mut String> {
        self.captures.last_mut().map(|(_, alt)| alt)
    }
}

/// One source segment of buffered text.
#[derive(Debug, Clone)]
struct Seg {
    buf_start: usize,
    buf_end: usize,
    span: Span,
    literal: bool,
}

/// Text waiting to be emitted, so URLs split over several text events can be autolinked.
#[derive(Debug, Default)]
struct Pending {
    text: String,
    marks: Marks,
    autolink: bool,
    segs: Vec<Seg>,
}

/// Builds the inline content of one leaf block.
#[derive(Debug, Default)]
pub(crate) struct InlineBuilder {
    toks: Vec<Tok>,
    stack: MarkStack,
    pending: Pending,
    /// An open `<script>`/`<style>` element under `--html drop`.
    raw_text: Option<RawText>,
}

/// Link types whose destination may be an `adf:` extension.
fn is_written_link(link_type: pulldown_cmark::LinkType) -> bool {
    use pulldown_cmark::LinkType;
    match link_type {
        LinkType::Inline
        | LinkType::Reference
        | LinkType::ReferenceUnknown
        | LinkType::Collapsed
        | LinkType::CollapsedUnknown
        | LinkType::Shortcut
        | LinkType::ShortcutUnknown => true,
        LinkType::Autolink | LinkType::Email | LinkType::WikiLink { .. } => false,
    }
}

/// Removes control characters other than tab and newline.
pub(crate) fn strip_controls(s: &str) -> String {
    s.chars()
        .filter(|&c| c == '\t' || c == '\n' || !c.is_control())
        .collect()
}

/// How an image is converted: by `--images`, or as `adf:media` unless that policy drops it.
fn image_mode(dest: &str, cx: &Cx<'_>) -> ImageMode {
    let policy = match cx.opts.images {
        ImagePolicy::Link => ImageMode::Link,
        ImagePolicy::External => ImageMode::External,
        ImagePolicy::Drop => return ImageMode::Drop,
    };
    if split_adf_url(dest).is_none_or(|(kind, _)| kind != "media") {
        return policy;
    }
    if cx.opts.extensions.media {
        ImageMode::File
    } else {
        ImageMode::FileDisabled
    }
}

/// The alt text of an image that is not shown as text: control characters removed, newlines
/// turned into spaces, trimmed last so a removed control character or newline cannot leave the
/// text starting or ending with a space.
fn clean_alt(frame: &ImageFrame, cx: &mut Cx<'_>) -> String {
    strip_controls_reported(&frame.alt, "an image's alt text", &frame.span, cx)
        .replace('\n', " ")
        .trim()
        .to_owned()
}

/// [`strip_controls`] for text that goes into an attribute (a title, a language, a label):
/// reports `W_CONTROL_CHAR` at `span` when something was removed. `what` names the attribute,
/// with its article ("a link title").
pub(crate) fn strip_controls_reported(s: &str, what: &str, span: &Span, cx: &mut Cx<'_>) -> String {
    let cleaned = strip_controls(s);
    if cleaned.len() != s.len() {
        cx.warn(
            Code::ControlChar,
            format!("Control characters were removed from {what}"),
            span,
        );
    }
    cleaned
}

/// The plain-text label of an `adf:` link. A line break or an image between two words reads as
/// one space (status and mention text cannot hold a break), and never doubles a space that is
/// already there; at the start or end of the label it leaves no space at all.
fn adf_label(toks: &[Tok]) -> String {
    const SPACE: [char; 2] = [' ', '\t'];
    let mut label = String::new();
    let mut gap = false;
    for tok in toks {
        match tok {
            Tok::Text { text, .. } => {
                for (index, part) in text.split('\n').enumerate() {
                    if index > 0 {
                        gap = true;
                    }
                    if part.is_empty() {
                        continue;
                    }
                    if !gap {
                        label.push_str(part);
                    } else if label.is_empty() || label.ends_with(SPACE) {
                        label.push_str(part.trim_start_matches(SPACE));
                    } else {
                        if !part.starts_with(SPACE) {
                            label.push(' ');
                        }
                        label.push_str(part);
                    }
                    gap = false;
                }
            }
            Tok::HardBreak | Tok::Node(_) | Tok::Media { .. } => gap = true,
        }
    }
    if gap {
        label.truncate(label.trim_end_matches(SPACE).len());
    }
    label
}

/// Link text made only of literal spaces (`[ ](url)`) is kept at the edge of a block: it is the
/// whole link, and trimming it would remove the link.
fn keep_linked_whitespace(toks: &mut [Tok]) {
    let only_linked_space = !toks.is_empty()
        && toks.iter().all(|t| match t {
            Tok::Text { text, marks, .. } => {
                marks.fmt.link.is_some() && text.chars().all(|c| c == ' ' || c == '\t')
            }
            Tok::HardBreak | Tok::Node(_) | Tok::Media { .. } => false,
        });
    if only_linked_space {
        for tok in toks {
            if let Tok::Text { trim, .. } = tok {
                *trim = false;
            }
        }
    }
}

impl InlineBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Text that appeared literally (`literal`) or decoded (entities) in the source.
    pub(crate) fn text(&mut self, text: &str, span: Span, literal: bool, cx: &Cx<'_>) {
        if text.is_empty() {
            return;
        }
        if self.stack.capturing > 0 {
            if let Some(alt) = self.stack.capture_alt() {
                alt.push_str(text);
            }
            return;
        }
        let marks = self.stack.marks();
        let autolink = self.stack.in_link == 0 && !marks.code;
        if !self.pending.text.is_empty()
            && (self.pending.marks != marks || self.pending.autolink != autolink)
        {
            self.flush(cx);
        }
        if self.pending.text.is_empty() {
            self.pending.marks = marks;
            self.pending.autolink = autolink;
        }
        let buf_start = self.pending.text.len();
        self.pending.text.push_str(text);
        self.pending.segs.push(Seg {
            buf_start,
            buf_end: self.pending.text.len(),
            span,
            literal,
        });
    }

    /// Emits the pending text, autolinking bare URLs that appear literally in the source.
    fn flush(&mut self, _cx: &Cx<'_>) {
        let pending = std::mem::take(&mut self.pending);
        if pending.text.is_empty() {
            return;
        }
        let mut cuts: Vec<(usize, usize, Option<Link>)> = Vec::new();
        if pending.autolink && pending.text.contains(['.', '@', ':']) {
            for (start, end, href) in find_autolinks(&pending.text) {
                if literal_contiguous(&pending.segs, start, end) {
                    cuts.push((start, end, Some(Link::new(href, None))));
                }
            }
        }
        // Emit pieces split at link boundaries and segment boundaries. Pieces and segments are
        // both in buffer order, so one cursor over the segments keeps this linear.
        let mut pos = 0;
        let mut seg_cursor = 0;
        let mut cut_iter = cuts.into_iter().peekable();
        while pos < pending.text.len() {
            let (end, link) = match cut_iter.peek() {
                Some((start, end, link)) if *start == pos => {
                    let item = (*end, link.clone());
                    cut_iter.next();
                    item
                }
                Some((start, _, _)) => (*start, None),
                None => (pending.text.len(), None),
            };
            self.emit_range(&pending, pos, end, link, &mut seg_cursor);
            pos = end;
        }
    }

    /// Emits buffer bytes `start..end` as text tokens, one per source segment. `cursor` is the
    /// index of the first segment that may overlap `start..`; it is advanced past segments
    /// that end before `end`.
    fn emit_range(
        &mut self,
        pending: &Pending,
        start: usize,
        end: usize,
        link: Option<Link>,
        cursor: &mut usize,
    ) {
        let mut marks = pending.marks.clone();
        if link.is_some() {
            marks.fmt.link = link;
        }
        while pending
            .segs
            .get(*cursor)
            .is_some_and(|s| s.buf_end <= start)
        {
            *cursor += 1;
        }
        for seg in pending.segs.get(*cursor..).unwrap_or_default() {
            if seg.buf_start >= end {
                break;
            }
            let a = seg.buf_start.max(start);
            let b = seg.buf_end.min(end);
            if a >= b {
                continue;
            }
            let Some(text) = pending.text.get(a..b) else {
                continue;
            };
            let span = if seg.literal {
                seg.span.start + (a - seg.buf_start)..seg.span.start + (b - seg.buf_start)
            } else {
                seg.span.clone()
            };
            self.toks.push(Tok::Text {
                text: text.to_owned(),
                marks: marks.clone(),
                span,
                trim: seg.literal,
            });
        }
    }

    fn push_tok(&mut self, tok: Tok, cx: &Cx<'_>) {
        self.flush(cx);
        self.toks.push(tok);
    }

    /// An inline code span.
    pub(crate) fn code(&mut self, text: &str, span: Span, cx: &mut Cx<'_>) {
        if self.stack.capturing > 0 {
            if let Some(alt) = self.stack.capture_alt() {
                alt.push_str(text);
            }
            return;
        }
        if text.is_empty() {
            return;
        }
        let marks = self.stack.marks();
        if marks.has_non_link_formatting() {
            cx.warn(
                Code::MarkDropped,
                "Formatting around inline code was dropped; ADF code text can only also be a link",
                &span,
            );
        }
        let marks = Marks {
            fmt: Formatting {
                link: marks.fmt.link,
                ..Formatting::default()
            },
            code: true,
        };
        self.push_tok(
            Tok::Text {
                text: text.to_owned(),
                marks,
                span,
                trim: false,
            },
            cx,
        );
    }

    /// A soft line break.
    pub(crate) fn soft_break(&mut self, span: Span, cx: &Cx<'_>) {
        if self.stack.capturing > 0 {
            if let Some(alt) = self.stack.capture_alt() {
                alt.push(' ');
            }
            return;
        }
        match cx.opts.softbreak {
            SoftBreak::Space => {
                let marks = self.stack.marks();
                self.push_tok(
                    Tok::Text {
                        text: " ".to_owned(),
                        marks,
                        span,
                        trim: true,
                    },
                    cx,
                );
            }
            SoftBreak::HardBreak => self.push_tok(Tok::HardBreak, cx),
        }
    }

    /// A hard line break (`<br>`, two trailing spaces, backslash).
    pub(crate) fn hard_break(&mut self, cx: &Cx<'_>) {
        if self.stack.capturing > 0 {
            if let Some(alt) = self.stack.capture_alt() {
                alt.push(' ');
            }
            return;
        }
        self.push_tok(Tok::HardBreak, cx);
    }

    /// A footnote reference, shown as a superscript number.
    pub(crate) fn footnote_ref(&mut self, number: u32, span: Span, cx: &Cx<'_>) {
        if self.stack.capturing > 0 {
            return;
        }
        let mut marks = self.stack.marks();
        marks.code = false;
        marks.fmt.subsup = Some(SubSup::Sup);
        self.push_tok(
            Tok::Text {
                text: number.to_string(),
                marks,
                span,
                trim: false,
            },
            cx,
        );
    }

    pub(crate) fn start_em(&mut self, cx: &Cx<'_>) {
        self.open_simple(FrameKind::Em, cx);
    }

    pub(crate) fn start_strong(&mut self, cx: &Cx<'_>) {
        self.open_simple(FrameKind::Strong, cx);
    }

    pub(crate) fn start_strike(&mut self, cx: &Cx<'_>) {
        self.open_simple(FrameKind::Strike, cx);
    }

    fn open_simple(&mut self, kind: FrameKind, cx: &Cx<'_>) {
        self.flush(cx);
        let adds = Adds {
            em: matches!(kind, FrameKind::Em),
            strong: matches!(kind, FrameKind::Strong),
            strike: matches!(kind, FrameKind::Strike),
            ..Adds::default()
        };
        self.stack.push(
            kind,
            Contrib {
                adds,
                ..Contrib::default()
            },
        );
    }

    pub(crate) fn end_em(&mut self, cx: &Cx<'_>) {
        self.flush(cx);
        self.stack.pop_until(|k| matches!(k, FrameKind::Em));
    }

    pub(crate) fn end_strong(&mut self, cx: &Cx<'_>) {
        self.flush(cx);
        self.stack.pop_until(|k| matches!(k, FrameKind::Strong));
    }

    pub(crate) fn end_strike(&mut self, cx: &Cx<'_>) {
        self.flush(cx);
        self.stack.pop_until(|k| matches!(k, FrameKind::Strike));
    }

    /// Start of a link.
    pub(crate) fn start_link(
        &mut self,
        link_type: pulldown_cmark::LinkType,
        dest: &str,
        title: &str,
        span: Span,
        cx: &mut Cx<'_>,
    ) {
        self.flush(cx);
        let adf = if is_written_link(link_type) {
            split_adf_url(dest).map(|(kind, arg)| (kind, arg.map(str::to_owned)))
        } else {
            None
        };
        let mut link = None;
        if adf.is_none() {
            let href = if link_type == pulldown_cmark::LinkType::Email {
                format!("mailto:{dest}")
            } else {
                dest.to_owned()
            };
            link = Href::parse(&href).map(|href| {
                let title = strip_controls_reported(title, "a link title", &span, cx);
                Link::new(href, NonEmptyString::new(title.replace('\n', " ")))
            });
            if link.is_none() {
                cx.warn_hint(
                    Code::UnsafeUrl,
                    format!(
                        "Link target '{}' is not an allowed URL; the link text was kept without a link",
                        truncate(dest, 80)
                    ),
                    format!("Use an allowed URL: {ALLOWED_URL_FORMS}"),
                    &span,
                );
            }
        }
        let frame = LinkFrame {
            adf,
            display: dest.to_owned(),
            link: link.clone(),
            start_tok: self.toks.len(),
            span,
        };
        self.stack.push(
            FrameKind::Link(frame),
            Contrib {
                link,
                ..Contrib::default()
            },
        );
    }

    /// End of a link.
    pub(crate) fn end_link(&mut self, cx: &mut Cx<'_>) {
        self.flush(cx);
        let Some(Frame {
            kind: FrameKind::Link(frame),
            ..
        }) = self.stack.pop_until(|k| matches!(k, FrameKind::Link(_)))
        else {
            return;
        };
        if self.stack.capturing > 0 {
            return;
        }
        let start = frame.start_tok.min(self.toks.len());
        if let Some((kind, arg)) = &frame.adf {
            self.end_adf_link(kind, arg.as_deref(), start, &frame.span, cx);
            return;
        }
        if self.toks.len() == start {
            // Empty link text: show the URL. Control characters in it are removed, and
            // reported, with the rest of the text.
            let mut marks = self.stack.marks();
            if frame.link.is_some() {
                marks.fmt.link = frame.link;
            }
            self.toks.push(Tok::Text {
                text: frame.display,
                marks,
                span: frame.span,
                trim: false,
            });
        } else if let Some(inner) = self.toks.get_mut(start..) {
            keep_linked_whitespace(inner);
        }
    }

    /// Replaces the label tokens of an `adf:` link (from `start`) with its node, or keeps them
    /// as text when the link is invalid.
    fn end_adf_link(
        &mut self,
        kind: &str,
        arg: Option<&str>,
        start: usize,
        span: &Span,
        cx: &mut Cx<'_>,
    ) {
        let inner = self.toks.get(start..).unwrap_or_default();
        let inner_marked = inner.iter().any(|t| match t {
            Tok::Text { marks, .. } => !marks.is_empty(),
            Tok::HardBreak | Tok::Node(_) | Tok::Media { .. } => false,
        });
        let written = adf_label(inner);
        let label = strip_controls(&written);
        match build_link_node(kind, arg, &label, &cx.opts.extensions) {
            Ok(node) => {
                // (When the link stays text, the text rules report control characters.)
                if label.len() != written.len() {
                    cx.warn(
                        Code::ControlChar,
                        format!("Control characters were removed from an adf:{kind} label"),
                        span,
                    );
                }
                if inner_marked || !self.stack.marks().is_empty() {
                    cx.warn(
                        Code::MarkDropped,
                        format!(
                            "Formatting on an adf:{kind} link was dropped; ADF {kind} nodes cannot carry marks"
                        ),
                        span,
                    );
                }
                // External images in the label cannot be part of the node: they follow it, and
                // become blocks like any other external image.
                let images: Vec<Tok> = self
                    .toks
                    .drain(start..)
                    .filter(|t| matches!(t, Tok::Media { .. }))
                    .collect();
                for image in &images {
                    if let Tok::Media { span, .. } = image {
                        cx.warn(
                            Code::BlockHoisted,
                            format!(
                                "An image cannot be part of an adf:{kind} label; it was moved after the {kind}"
                            ),
                            span,
                        );
                    }
                }
                self.toks.push(Tok::Node(node));
                self.toks.extend(images);
            }
            Err(fallback) => cx.warn(fallback.code, fallback.message, span),
        }
    }

    /// Start of an image.
    pub(crate) fn start_image(&mut self, dest: &str, title: &str, span: Span, cx: &mut Cx<'_>) {
        self.flush(cx);
        let mode = if self.stack.capturing > 0 {
            ImageMode::External
        } else {
            image_mode(dest, cx)
        };
        let mut push_link = None;
        let mut link = None;
        if mode == ImageMode::Link {
            link = Href::parse(dest).map(|href| {
                let title = strip_controls_reported(title, "an image title", &span, cx);
                Link::new(href, NonEmptyString::new(title.replace('\n', " ")))
            });
            if link.is_none() {
                cx.warn(
                    Code::UnsafeUrl,
                    format!(
                        "Image URL '{}' is not an allowed URL; only the alt text was kept",
                        truncate(dest, 80)
                    ),
                    &span,
                );
            } else {
                cx.warn_hint(
                    Code::ImageAsLink,
                    "Image converted to a link; ADF images need media uploaded to Jira or Confluence",
                    "Use --images external to embed it by URL, or attach the file in Jira",
                    &span,
                );
            }
            // Inside a link, the outer link wins (e.g. a badge linking elsewhere).
            if self.stack.links.is_empty() {
                push_link.clone_from(&link);
            }
        }
        let frame = ImageFrame {
            mode,
            url: dest.to_owned(),
            link,
            titled: !strip_controls(title).trim().is_empty(),
            alt: String::new(),
            start_tok: self.toks.len(),
            span,
        };
        self.stack.push(
            FrameKind::Image(frame),
            Contrib {
                link: push_link,
                ..Contrib::default()
            },
        );
    }

    /// End of an image.
    pub(crate) fn end_image(&mut self, cx: &mut Cx<'_>) {
        self.flush(cx);
        let Some(Frame {
            kind: FrameKind::Image(frame),
            ..
        }) = self.stack.pop_until(|k| matches!(k, FrameKind::Image(_)))
        else {
            return;
        };
        if self.stack.capturing > 0 {
            // A nested image inside captured alt text: its alt text joins the outer one.
            if let Some(outer) = self.stack.capture_alt() {
                outer.push_str(&frame.alt);
            }
            return;
        }
        match frame.mode {
            ImageMode::Link => {
                if self.toks.len() == frame.start_tok {
                    let mut marks = self.stack.marks();
                    if marks.fmt.link.is_none() {
                        marks.fmt.link = frame.link;
                    }
                    self.toks.push(Tok::Text {
                        text: frame.url,
                        marks,
                        span: frame.span,
                        trim: false,
                    });
                } else if let Some(inner) = self.toks.get_mut(frame.start_tok..) {
                    keep_linked_whitespace(inner);
                }
            }
            ImageMode::External => self.end_external_image(frame, cx),
            ImageMode::File => self.end_file_image(frame, cx),
            ImageMode::FileDisabled => {
                cx.warn(
                    Code::ExtensionDisabled,
                    "adf:media image found, but the 'media' extension is disabled; the alt text was kept",
                    &frame.span,
                );
                let alt = clean_alt(&frame, cx);
                self.push_alt_text(alt, frame.span);
            }
            ImageMode::Drop => cx.warn(
                Code::ImageDropped,
                "Image dropped (--images drop)",
                &frame.span,
            ),
        }
    }

    /// An image with `--images external`: a media token, or its alt text for a bad URL.
    fn end_external_image(&mut self, frame: ImageFrame, cx: &mut Cx<'_>) {
        let alt = clean_alt(&frame, cx);
        if Href::parse(&frame.url).is_none() {
            cx.warn(
                Code::UnsafeUrl,
                format!(
                    "Image URL '{}' is not an allowed URL; only the alt text was kept",
                    truncate(&frame.url, 80)
                ),
                &frame.span,
            );
            self.push_alt_text(alt, frame.span);
            return;
        }
        self.warn_media_marks(&frame, cx);
        self.toks.push(Tok::Media {
            media: MediaSingle::external(frame.url.trim().to_owned(), NonEmptyString::new(alt)),
            span: frame.span,
        });
    }

    /// An `adf:media` image: a media token, or its alt text for an invalid argument.
    fn end_file_image(&mut self, frame: ImageFrame, cx: &mut Cx<'_>) {
        let alt = clean_alt(&frame, cx);
        let arg = split_adf_url(&frame.url).and_then(|(_, arg)| arg);
        match media::parse(arg) {
            Ok(image) => {
                self.warn_media_marks(&frame, cx);
                self.toks.push(Tok::Media {
                    media: MediaSingle {
                        source: MediaSource::File(image.file),
                        alt: NonEmptyString::new(alt),
                        layout: image.layout,
                        size: image.size,
                    },
                    span: frame.span,
                });
            }
            Err(message) => {
                cx.warn_hint(
                    Code::InvalidExtensionArg,
                    message,
                    media::SYNTAX_HINT,
                    &frame.span,
                );
                self.push_alt_text(alt, frame.span);
            }
        }
    }

    /// Reports the link and the title that ADF media cannot carry.
    fn warn_media_marks(&self, frame: &ImageFrame, cx: &mut Cx<'_>) {
        // In an `adf:` link label, the image is moved after the node (see `end_adf_link`).
        if self.stack.in_link > self.stack.adf_links {
            cx.warn(
                Code::MarkDropped,
                "The link around an image was dropped; ADF media cannot be linked",
                &frame.span,
            );
        }
        if frame.titled {
            cx.warn(
                Code::MarkDropped,
                "The image title was dropped; ADF media cannot carry a title",
                &frame.span,
            );
        }
    }

    /// The alt text of an image that did not become media, as plain text.
    fn push_alt_text(&mut self, alt: String, span: Span) {
        if alt.is_empty() {
            return;
        }
        self.toks.push(Tok::Text {
            text: alt,
            marks: self.stack.marks(),
            span,
            trim: false,
        });
    }

    /// An inline HTML tag or comment.
    pub(crate) fn inline_html(
        &mut self,
        html: &str,
        span: Span,
        cx: &mut Cx<'_>,
    ) -> Result<(), Diagnostic> {
        if self.stack.capturing > 0 {
            // Alt text is plain text: tags are removed, and `<br>` is a space like the other
            // line breaks.
            if parse_tag(html).is_some_and(|(tag, _)| classify(&tag, false) == Recognised::Br)
                && let Some(alt) = self.stack.capture_alt()
            {
                alt.push(' ');
            }
            return Ok(());
        }
        let Some((tag, _)) = parse_tag(html) else {
            if self.raw_text.is_some() {
                return Ok(());
            }
            return self.unknown_html(html, span, cx);
        };
        if self.raw_text.is_some() {
            // Inside `<script>`/`<style>` (`--html drop`), only its closing tag counts.
            if self.closes_raw_text(&tag) {
                self.unknown_tag(&tag, html, span, cx)?;
            }
            return Ok(());
        }
        let details = cx.opts.extensions.details;
        match classify(&tag, details) {
            Recognised::Comment | Recognised::Empty => Ok(()),
            Recognised::Br => {
                self.hard_break(cx);
                Ok(())
            }
            Recognised::OpenMark { name, mark } => {
                self.open_html(name, mark, &span, cx);
                Ok(())
            }
            Recognised::CloseMark { name } => match self.close_html(&name, cx) {
                Closed::Mark => Ok(()),
                Closed::Literal | Closed::NotOpen => self.unknown_html(html, span, cx),
            },
            Recognised::DetailsOpen
            | Recognised::DetailsClose
            | Recognised::SummaryOpen
            | Recognised::SummaryClose
            | Recognised::Unknown => {
                if !details && is_details_open(&tag) {
                    cx.warn(
                        Code::ExtensionDisabled,
                        "<details> found, but the 'details' extension is disabled; treated as HTML",
                        &span,
                    );
                }
                self.unknown_tag(&tag, html, span, cx).map(|_| ())
            }
        }
    }

    /// Opens a recognised HTML mark frame. Beyond [`MAX_HTML_FRAMES`] open tags the tag is
    /// ignored.
    pub(crate) fn open_html(&mut self, name: String, mark: HtmlMark, span: &Span, cx: &mut Cx<'_>) {
        if self.stack.html >= MAX_HTML_FRAMES {
            return;
        }
        self.flush(cx);
        if let HtmlMark::SubSup(inner) = mark
            && let Some((_, outer)) = self.stack.subsup.last()
            && *outer != inner
        {
            let (outer_tag, outer_kind) = match outer {
                SubSup::Sub => ("sub", "subscript"),
                SubSup::Sup => ("sup", "superscript"),
            };
            cx.warn(
                Code::MarkDropped,
                format!(
                    "<{name}> inside <{outer_tag}> dropped the {outer_kind}; ADF text cannot be both subscript and superscript"
                ),
                span,
            );
        }
        let mut contrib = Contrib::default();
        match mark {
            HtmlMark::Underline => contrib.adds.underline = true,
            HtmlMark::Strike => contrib.adds.strike = true,
            HtmlMark::Strong => contrib.adds.strong = true,
            HtmlMark::Em => contrib.adds.em = true,
            HtmlMark::Code => contrib.adds.code = true,
            HtmlMark::SubSup(v) => contrib.subsup = Some(v),
            HtmlMark::Span { color, background } => {
                contrib.color = color;
                contrib.background = background;
            }
        }
        self.stack.push(
            FrameKind::Html {
                name,
                literal: false,
            },
            contrib,
        );
    }

    /// Closes the innermost open HTML frame named `name`.
    pub(crate) fn close_html(&mut self, name: &str, cx: &Cx<'_>) -> Closed {
        let open = self
            .stack
            .html_names
            .iter()
            .any(|(n, count)| n == name && *count > 0);
        if !open {
            return Closed::NotOpen;
        }
        let Some(index) = self
            .stack
            .frames
            .iter()
            .rposition(|f| matches!(&f.kind, FrameKind::Html { name: n, .. } if n == name))
        else {
            return Closed::NotOpen;
        };
        self.flush(cx);
        match self.stack.remove(index) {
            Some(Frame {
                kind: FrameKind::Html { literal: true, .. },
                ..
            }) => Closed::Literal,
            Some(_) | None => Closed::Mark,
        }
    }

    /// Whether `tag` closes the open `<script>`/`<style>` element (`--html drop`).
    pub(crate) fn closes_raw_text(&self, tag: &HtmlTag) -> bool {
        matches!(
            (tag, &self.raw_text),
            (HtmlTag::Close { name }, Some(raw)) if *name == raw.name
        )
    }

    /// Whether a `<script>`/`<style>` element is open (`--html drop`): until its closing tag,
    /// other tags are part of its content.
    pub(crate) fn in_raw_text(&self) -> bool {
        self.raw_text.is_some()
    }

    /// A tag without an ADF equivalent: applies `--html` to it ([`Self::unknown_html`]), and
    /// remembers the opening tags whose closing tag must pair with them:
    ///
    /// - an unrecognised `<span …>` gets a frame without marks, so its `</span>` does not close
    ///   a recognised `<span style="color: …">` around it;
    /// - under `--html drop`, `<script>` and `<style>` drop their content with them, up to the
    ///   closing tag (browsers never show it). Without a closing tag the content is kept.
    pub(crate) fn unknown_tag(
        &mut self,
        tag: &HtmlTag,
        html: &str,
        span: Span,
        cx: &mut Cx<'_>,
    ) -> Result<Unknown, Diagnostic> {
        if self.closes_raw_text(tag) {
            self.flush(cx);
            if let Some(raw) = self.raw_text.take() {
                self.toks.truncate(raw.start_tok.min(self.toks.len()));
                cx.warn(
                    Code::HtmlDropped,
                    format!(
                        "HTML '{}' was dropped with the content of '{}' (--html drop)",
                        truncate(html.trim(), 60),
                        raw.shown
                    ),
                    &(raw.span.start..span.end),
                );
            }
            return Ok(Unknown::RawTextClosed);
        }
        self.unknown_html(html, span.clone(), cx)?;
        let HtmlTag::Open {
            name,
            self_closing: false,
            ..
        } = tag
        else {
            return Ok(Unknown::Reported);
        };
        if cx.opts.html == HtmlPolicy::Drop
            && self.raw_text.is_none()
            && RAW_TEXT_ELEMENTS.contains(&name.as_str())
        {
            self.flush(cx);
            self.raw_text = Some(RawText {
                name: name.clone(),
                start_tok: self.toks.len(),
                shown: truncate(html.trim(), 60),
                span,
            });
            return Ok(Unknown::RawTextOpened);
        }
        if closes_by_name(name) && self.stack.html < MAX_HTML_FRAMES {
            self.flush(cx);
            self.stack.push(
                FrameKind::Html {
                    name: name.clone(),
                    literal: true,
                },
                Contrib::default(),
            );
        }
        Ok(Unknown::Reported)
    }

    /// HTML without an ADF equivalent, handled per `--html`.
    pub(crate) fn unknown_html(
        &mut self,
        html: &str,
        span: Span,
        cx: &mut Cx<'_>,
    ) -> Result<(), Diagnostic> {
        let shown = truncate(html.trim(), 60);
        match cx.opts.html {
            HtmlPolicy::Text => {
                cx.warn_hint(
                    Code::HtmlAsText,
                    format!("HTML '{shown}' has no ADF equivalent and was kept as text"),
                    "Use --html drop to remove it, or rewrite it as Markdown",
                    &span,
                );
                let marks = self.stack.marks();
                let text = html.replace('\n', " ");
                self.push_tok(
                    Tok::Text {
                        text,
                        marks,
                        span,
                        trim: false,
                    },
                    cx,
                );
                Ok(())
            }
            HtmlPolicy::Drop => {
                cx.warn(
                    Code::HtmlDropped,
                    format!("HTML '{shown}' was dropped (--html drop)"),
                    &span,
                );
                Ok(())
            }
            HtmlPolicy::Error => Err(cx.diag(
                Code::Lossy,
                format!("HTML '{shown}' has no ADF equivalent (--html error)"),
                &span,
            )),
        }
    }

    /// Finishes the content: flushes pending text and closes unclosed HTML frames.
    pub(crate) fn finish(mut self, cx: &Cx<'_>) -> Vec<Tok> {
        self.flush(cx);
        self.toks
    }
}

fn is_details_open(tag: &super::html::HtmlTag) -> bool {
    matches!(tag, super::html::HtmlTag::Open { name, .. } if name == "details")
}

/// URLs with a scheme, and email addresses.
fn scheme_finder() -> &'static LinkFinder {
    static FINDER: std::sync::OnceLock<LinkFinder> = std::sync::OnceLock::new();
    FINDER.get_or_init(|| {
        let mut finder = LinkFinder::new();
        finder.kinds(&[LinkKind::Url, LinkKind::Email]);
        finder
    })
}

/// Scheme-less URLs; only `www.` ones are used.
fn www_finder() -> &'static LinkFinder {
    static FINDER: std::sync::OnceLock<LinkFinder> = std::sync::OnceLock::new();
    FINDER.get_or_init(|| {
        let mut finder = LinkFinder::new();
        finder.url_must_have_scheme(false);
        finder.kinds(&[LinkKind::Url]);
        finder
    })
}

/// GFM extended autolinks in `text`: `http(s)://…`, `www.…` (gets `http://`) and email
/// addresses (get `mailto:`), as sorted, non-overlapping `(start, end, href)`.
fn find_autolinks(text: &str) -> Vec<(usize, usize, Href)> {
    let mut found: Vec<(usize, usize, Href)> = Vec::new();
    for link in scheme_finder().links(text) {
        let s = link.as_str();
        let href = if matches!(link.kind(), LinkKind::Email) {
            Href::parse(&format!("mailto:{s}"))
        } else {
            let lower = s.get(..8).unwrap_or(s).to_ascii_lowercase();
            if lower.starts_with("http://") || lower.starts_with("https://") {
                Href::parse(s)
            } else {
                None
            }
        };
        if let Some(href) = href {
            found.push((link.start(), link.end(), href));
        }
    }
    if !text.to_ascii_lowercase().contains("www.") {
        return found;
    }
    // `found` is sorted by start and non-overlapping, so overlap checks are binary searches.
    let mut www = Vec::new();
    for link in www_finder().links(text) {
        let s = link.as_str();
        if !s.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("www.")) {
            continue;
        }
        let before = found.partition_point(|(start, _, _)| *start < link.end());
        let overlaps = before > 0
            && found
                .get(before - 1)
                .is_some_and(|(_, end, _)| *end > link.start());
        if !overlaps && let Some(href) = Href::parse(&format!("http://{s}")) {
            www.push((link.start(), link.end(), href));
        }
    }
    found.extend(www);
    found.sort_by_key(|(start, _, _)| *start);
    found
}

/// Whether buffer bytes `start..end` come from literal source text with no gaps, so a URL that
/// only exists after unescaping (`https\://x`) is not linked.
fn literal_contiguous(segs: &[Seg], start: usize, end: usize) -> bool {
    let mut prev_end: Option<usize> = None;
    let first = segs.partition_point(|s| s.buf_end <= start);
    for seg in segs.get(first..).unwrap_or_default() {
        if seg.buf_start >= end {
            break;
        }
        if !seg.literal {
            return false;
        }
        if let Some(prev) = prev_end
            && prev != seg.span.start
        {
            return false;
        }
        prev_end = Some(seg.span.end);
    }
    true
}
