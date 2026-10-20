//! Turns inline tokens into model inline nodes: removes control characters, turns decoded
//! newlines into `hardBreak`s, trims block-edge whitespace, drops trailing hard breaks, merges
//! adjacent text with equal marks and never emits an empty text node.

use crate::adf::model::{Formatting, Inline, MediaSingle, NonEmptyString, Text};
use crate::diagnostics::Code;

use super::cx::Cx;
use super::inline::strip_controls;
use super::ir::{Marks, Span, Tok};

/// Inline content split at external images.
#[derive(Debug)]
pub(crate) enum Segment {
    Inline(Vec<Tok>),
    Media(MediaSingle),
}

/// Splits tokens at external images (`--images external`), which become blocks.
pub(crate) fn split_media(toks: Vec<Tok>) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    for tok in toks {
        match tok {
            Tok::Media { media, .. } => {
                if !current.is_empty() {
                    out.push(Segment::Inline(std::mem::take(&mut current)));
                }
                out.push(Segment::Media(media));
            }
            Tok::Text { .. } | Tok::HardBreak | Tok::Node(_) => current.push(tok),
        }
    }
    if !current.is_empty() {
        out.push(Segment::Inline(current));
    }
    out
}

/// Whether the last token is text ending in a space or tab.
pub(crate) fn ends_with_space(toks: &[Tok]) -> bool {
    matches!(toks.last(), Some(Tok::Text { text, .. }) if text.ends_with([' ', '\t']))
}

/// Removes leading spaces and tabs written literally in the source (entity-encoded ones are
/// kept, as at the edge of a block).
pub(crate) fn trim_leading_space(toks: &mut Vec<Tok>) {
    let mut empty = 0;
    for tok in toks.iter_mut() {
        match tok {
            Tok::Text {
                text, trim: true, ..
            } => {
                let cut = text.len() - text.trim_start_matches([' ', '\t']).len();
                if cut == text.len() {
                    empty += 1;
                } else {
                    text.drain(..cut);
                    break;
                }
            }
            Tok::Text { .. } | Tok::HardBreak | Tok::Node(_) | Tok::Media { .. } => break,
        }
    }
    toks.drain(..empty);
}

/// Adds `strong` to all non-code text (heading demotion, flattened titles).
pub(crate) fn embolden(toks: &mut [Tok]) {
    for tok in toks {
        if let Tok::Text { marks, .. } = tok
            && !marks.code
        {
            marks.fmt.strong = true;
        }
    }
}

#[derive(Debug)]
enum Item {
    Text {
        text: String,
        marks: Marks,
        trim: bool,
        span: Span,
    },
    Break,
    Node(Inline),
}

/// Finalises inline content. External images must have been split off (they are ignored).
pub(crate) fn finalize(toks: Vec<Tok>, cx: &mut Cx<'_>) -> Vec<Inline> {
    let mut items: Vec<Item> = Vec::with_capacity(toks.len());
    for tok in toks {
        match tok {
            Tok::Text {
                text,
                marks,
                span,
                trim,
            } => push_text(&mut items, &text, &marks, trim, &span, cx),
            Tok::HardBreak => items.push(Item::Break),
            Tok::Node(node) => items.push(Item::Node(node)),
            Tok::Media { .. } => {}
        }
    }
    trim_edges(&mut items);
    let mut out: Vec<Inline> = Vec::with_capacity(items.len());
    let mut pending: Option<(String, Marks, Span)> = None;
    for item in items {
        if cx.over_budget.is_some() {
            break;
        }
        match item {
            Item::Text {
                text, marks, span, ..
            } => match pending.as_mut() {
                Some((buf, m, _)) if *m == marks => buf.push_str(&text),
                _ => {
                    flush(&mut out, pending.take(), cx);
                    pending = Some((text, marks, span));
                }
            },
            Item::Break => {
                flush(&mut out, pending.take(), cx);
                out.push(Inline::HardBreak);
            }
            Item::Node(node) => {
                flush(&mut out, pending.take(), cx);
                out.push(node);
            }
        }
    }
    flush(&mut out, pending, cx);
    out
}

/// Emits a text run, charging its estimated JSON size to the output budget: every text node of
/// a link repeats the link's URL and title, so a long URL over a label with many differently
/// formatted runs would otherwise multiply into gigabytes.
fn flush(out: &mut Vec<Inline>, pending: Option<(String, Marks, Span)>, cx: &mut Cx<'_>) {
    if let Some((text, marks, span)) = pending
        && let Some(text) = NonEmptyString::new(text)
    {
        let link = marks.fmt.link.as_ref().map_or(0, |link| {
            link.href().as_str().len() + link.title().map_or(0, |t| t.as_str().len()) + 48
        });
        if !cx.charge(32 + text.as_str().len() + link + 24 * marks.count(), &span) {
            return;
        }
        out.push(Inline::Text(Text {
            text,
            marks: marks.to_text_marks(),
        }));
    }
}

fn push_text(
    items: &mut Vec<Item>,
    text: &str,
    marks: &Marks,
    trim: bool,
    span: &Span,
    cx: &mut Cx<'_>,
) {
    let cleaned = strip_controls(text);
    if cleaned.len() != text.len() {
        cx.warn(
            Code::ControlChar,
            "Control characters were removed from text",
            span,
        );
    }
    // Code spans drop other marks when they are built; `<kbd>` text arrives here with them.
    if marks.code && marks.has_non_link_formatting() {
        cx.warn(
            Code::MarkDropped,
            "Formatting on <kbd> text was dropped; ADF code text can only also be a link",
            span,
        );
    }
    // Compare and merge by the marks that reach the output: `<kbd>a *b*</kbd>` is one code run.
    let marks = &if marks.code {
        Marks {
            fmt: Formatting {
                link: marks.fmt.link.clone(),
                ..Formatting::default()
            },
            code: true,
        }
    } else {
        marks.clone()
    };
    let mut first = true;
    for part in cleaned.split('\n') {
        if !first {
            items.push(Item::Break);
        }
        first = false;
        if !part.is_empty() {
            items.push(Item::Text {
                text: part.to_owned(),
                marks: marks.clone(),
                trim,
                span: span.clone(),
            });
        }
    }
}

/// Trims literal spaces and tabs at the start and end, and drops trailing hard breaks.
fn trim_edges(items: &mut Vec<Item>) {
    // Leading.
    let mut start = 0;
    while let Some(item) = items.get_mut(start) {
        match item {
            Item::Text {
                text, trim: true, ..
            } => {
                let trimmed = text.trim_start_matches([' ', '\t']);
                if trimmed.is_empty() {
                    start += 1;
                } else {
                    *text = trimmed.to_owned();
                    break;
                }
            }
            Item::Text { .. } | Item::Break | Item::Node(_) => break,
        }
    }
    items.drain(..start);
    // Trailing.
    loop {
        match items.last_mut() {
            Some(Item::Break) => {
                items.pop();
            }
            Some(Item::Text {
                text, trim: true, ..
            }) => {
                let trimmed = text.trim_end_matches([' ', '\t']);
                if trimmed.is_empty() {
                    items.pop();
                } else {
                    let len = trimmed.len();
                    text.truncate(len);
                    break;
                }
            }
            Some(Item::Text { .. } | Item::Node(_)) | None => break,
        }
    }
}
