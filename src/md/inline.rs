//! Inline content: turns pulldown-cmark inline events into a flat list of [`Tok`]s.
//!
//! Marks come from a stack of open frames (Markdown emphasis, links, images, recognised HTML
//! tags). The effective mark set is kept in counters, so a token costs O(1) however deep the
//! nesting is, and HTML frames can be closed out of order (`<u>*a</u> b*`).

use linkify::{LinkFinder, LinkKind};

use crate::adf::model::{Formatting, HexColor, Href, Link, MediaSingle, NonEmptyString, SubSup};
use crate::diagnostics::{Code, Diagnostic};
use crate::options::{HtmlPolicy, ImagePolicy, SoftBreak};

use super::cx::Cx;
use super::extensions::{build_link_node, split_adf_url, truncate};
use super::html::{HtmlMark, Recognised, classify, parse_tag};
use super::ir::{Marks, Span, Tok};

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
    /// Whether the image has a non-empty title (`![alt](url "title")`).
    titled: bool,
    /// Alt text, collected when the image does not become text.
    alt: String,
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
    Html(String),
}

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
    /// Open images whose alt text is captured instead of emitted.
    capturing: u32,
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
            FrameKind::Link(_) => self.in_link += 1,
            FrameKind::Image(image) => {
                self.in_link += 1;
                if image.mode != ImageMode::Link {
                    self.capturing += 1;
                }
            }
            FrameKind::Html(name) => {
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
        let frame = self.frames.remove(index);
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
        match &frame.kind {
            FrameKind::Link(_) => self.in_link = self.in_link.saturating_sub(1),
            FrameKind::Image(image) => {
                self.in_link = self.in_link.saturating_sub(1);
                if image.mode != ImageMode::Link {
                    self.capturing = self.capturing.saturating_sub(1);
                }
            }
            FrameKind::Html(name) => {
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

    /// The innermost capturing image frame.
    fn capture_frame(&mut self) -> Option<&mut ImageFrame> {
        self.frames
            .iter_mut()
            .rev()
            .find_map(|f| match &mut f.kind {
                FrameKind::Image(image) if image.mode != ImageMode::Link => Some(image),
                FrameKind::Em
                | FrameKind::Strong
                | FrameKind::Strike
                | FrameKind::Link(_)
                | FrameKind::Image(_)
                | FrameKind::Html(_) => None,
            })
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
            if let Some(image) = self.stack.capture_frame() {
                image.alt.push_str(text);
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
                    cuts.push((start, end, Some(Link { href, title: None })));
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
            if let Some(image) = self.stack.capture_frame() {
                image.alt.push_str(text);
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
            if let Some(image) = self.stack.capture_frame() {
                image.alt.push(' ');
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
            if let Some(image) = self.stack.capture_frame() {
                image.alt.push(' ');
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
            link = Href::parse(&href).map(|href| Link {
                href,
                title: NonEmptyString::new(strip_controls(title).replace('\n', " ")),
            });
            if link.is_none() {
                cx.warn_hint(
                    Code::UnsafeUrl,
                    format!(
                        "Link target '{}' is not an allowed URL; the link text was kept without a link",
                        truncate(dest, 80)
                    ),
                    "Use an absolute http(s), mailto or tel URL, a /path or an #anchor",
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
            let inner = self.toks.get(start..).unwrap_or_default();
            let label: String = inner
                .iter()
                .filter_map(|t| match t {
                    Tok::Text { text, .. } => Some(text.as_str()),
                    Tok::HardBreak | Tok::Node(_) | Tok::Media { .. } => None,
                })
                .collect();
            let inner_marked = inner.iter().any(|t| match t {
                Tok::Text { marks, .. } => !marks.is_empty(),
                Tok::HardBreak | Tok::Node(_) | Tok::Media { .. } => false,
            });
            match build_link_node(kind, arg.as_deref(), &label, &cx.opts.extensions) {
                Ok(node) => {
                    if inner_marked || !self.stack.marks().is_empty() {
                        cx.warn(
                            Code::MarkDropped,
                            format!(
                                "Formatting on an adf:{kind} link was dropped; ADF {kind} nodes cannot carry marks"
                            ),
                            &frame.span,
                        );
                    }
                    self.toks.truncate(start);
                    self.toks.push(Tok::Node(node));
                }
                Err(fallback) => cx.warn(fallback.code, fallback.message, &frame.span),
            }
            return;
        }
        if self.toks.len() == start {
            // Empty link text: show the URL.
            let mut marks = self.stack.marks();
            if frame.link.is_some() {
                marks.fmt.link = frame.link;
            }
            self.toks.push(Tok::Text {
                text: strip_controls(&frame.display),
                marks,
                span: frame.span,
                trim: false,
            });
        }
    }

    /// Start of an image.
    pub(crate) fn start_image(&mut self, dest: &str, title: &str, span: Span, cx: &mut Cx<'_>) {
        self.flush(cx);
        let mode = if self.stack.capturing > 0 {
            ImageMode::External
        } else {
            match cx.opts.images {
                ImagePolicy::Link => ImageMode::Link,
                ImagePolicy::External => ImageMode::External,
                ImagePolicy::Drop => ImageMode::Drop,
            }
        };
        let mut push_link = None;
        let mut link = None;
        if mode == ImageMode::Link {
            link = Href::parse(dest).map(|href| Link {
                href,
                title: NonEmptyString::new(strip_controls(title).replace('\n', " ")),
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
            if let Some(outer) = self.stack.capture_frame() {
                outer.alt.push_str(&frame.alt);
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
                        text: strip_controls(&frame.url),
                        marks,
                        span: frame.span,
                        trim: false,
                    });
                }
            }
            ImageMode::External => {
                // Trim last: a removed control character or newline must not leave the alt
                // text starting or ending with a space.
                let alt = strip_controls(&frame.alt)
                    .replace('\n', " ")
                    .trim()
                    .to_owned();
                if Href::parse(&frame.url).is_none() {
                    cx.warn(
                        Code::UnsafeUrl,
                        format!(
                            "Image URL '{}' is not an allowed URL; only the alt text was kept",
                            truncate(&frame.url, 80)
                        ),
                        &frame.span,
                    );
                    let marks = self.stack.marks();
                    if !alt.is_empty() {
                        self.toks.push(Tok::Text {
                            text: alt,
                            marks,
                            span: frame.span,
                            trim: false,
                        });
                    }
                    return;
                }
                if !self.stack.links.is_empty() || self.stack.in_link > 0 {
                    cx.warn_hint(
                        Code::MarkDropped,
                        "The link around an image was dropped; --images external does not link media",
                        "Write the link as text next to the image",
                        &frame.span,
                    );
                }
                if frame.titled {
                    cx.warn(
                        Code::MarkDropped,
                        "The image title was dropped; ADF media has no title",
                        &frame.span,
                    );
                }
                self.toks.push(Tok::Media {
                    media: MediaSingle {
                        url: frame.url.trim().to_owned(),
                        alt: NonEmptyString::new(alt),
                    },
                    span: frame.span,
                });
            }
            ImageMode::Drop => cx.warn(
                Code::ImageDropped,
                "Image dropped (--images drop)",
                &frame.span,
            ),
        }
    }

    /// An inline HTML tag or comment.
    pub(crate) fn inline_html(
        &mut self,
        html: &str,
        span: Span,
        cx: &mut Cx<'_>,
    ) -> Result<(), Diagnostic> {
        if self.stack.capturing > 0 {
            return Ok(());
        }
        let Some((tag, _)) = parse_tag(html) else {
            return self.unknown_html(html, span, cx);
        };
        let details = cx.opts.extensions.details;
        match classify(&tag, details) {
            Recognised::Comment | Recognised::Empty => Ok(()),
            Recognised::Br => {
                self.hard_break(cx);
                Ok(())
            }
            Recognised::OpenMark { name, mark } => {
                self.open_html(name, mark, cx);
                Ok(())
            }
            Recognised::CloseMark { name } => {
                if self.close_html(&name, cx) {
                    Ok(())
                } else {
                    self.unknown_html(html, span, cx)
                }
            }
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
                self.unknown_html(html, span, cx)
            }
        }
    }

    /// Opens a recognised HTML mark frame. Beyond [`MAX_HTML_FRAMES`] open tags the tag is
    /// ignored.
    pub(crate) fn open_html(&mut self, name: String, mark: HtmlMark, cx: &Cx<'_>) {
        if self.stack.html >= MAX_HTML_FRAMES {
            return;
        }
        self.flush(cx);
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
        self.stack.push(FrameKind::Html(name), contrib);
    }

    /// Closes the innermost open HTML frame named `name`; `false` if there is none.
    pub(crate) fn close_html(&mut self, name: &str, cx: &Cx<'_>) -> bool {
        let open = self
            .stack
            .html_names
            .iter()
            .any(|(n, count)| n == name && *count > 0);
        if !open {
            return false;
        }
        let Some(index) = self
            .stack
            .frames
            .iter()
            .rposition(|f| matches!(&f.kind, FrameKind::Html(n) if n == name))
        else {
            return false;
        };
        self.flush(cx);
        self.stack.remove(index);
        true
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
