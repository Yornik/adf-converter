//! The `adf:media` image syntax (`![alt](adf:media/<id>?…)`) for a single Media Services file.
//!
//! Only a `mediaSingle` that the syntax says exactly is written this way; anything else keeps
//! the raw ```` ```adf ```` block. Every piece of the destination is validated here (UUID,
//! `[A-Za-z0-9_-]` collection, integers, allowlisted layout), so nothing can inject Markdown.

use serde_json::Value;

use crate::adf::reader::{Node, marker_alt};

use super::escape::{TextContext, TextEscape, escape_text, has_unsafe_control};

/// The `mediaSingle` layouts of the syntax (`layout=`).
const LAYOUTS: [&str; 7] = [
    "wide",
    "full-width",
    "center",
    "wrap-right",
    "wrap-left",
    "align-end",
    "align-start",
];

/// Attributes of a file `media` node the syntax carries (`localId` is dropped silently).
const MEDIA_ATTRS: [&str; 7] = [
    "type",
    "id",
    "collection",
    "alt",
    "width",
    "height",
    "localId",
];

/// Attributes of a `mediaSingle` the syntax carries.
const SINGLE_ATTRS: [&str; 4] = ["layout", "width", "widthType", "localId"];

/// Largest pixel size: `width`, `height` and `size=<n>px`.
const MAX_PIXELS: u64 = 100_000;

/// Longest `collection`.
const MAX_COLLECTION_LEN: usize = 64;

/// The destination `adf:media/<id>?…` of a single file image, or `None` when the `mediaSingle`
/// `single` and its only child `media` hold anything the syntax cannot say: another media type,
/// marks, other attributes, a malformed id or collection, a size out of range, or alt text that
/// Markdown → ADF would change (control characters, line breaks, surrounding whitespace).
pub(super) fn file_media_dest(single: &Node<'_>, media: &Node<'_>) -> Option<String> {
    if media.attr_str("type") != Some("file")
        || !media.marks.is_empty()
        || !single.marks.is_empty()
        || !media.content.is_empty()
        || !media.attr_names().all(|n| MEDIA_ATTRS.contains(&n))
        || !single.attr_names().all(|n| SINGLE_ATTRS.contains(&n))
    {
        return None;
    }
    let id = media.attr_str("id").filter(|id| is_uuid(id))?;
    let collection = media.attr_str("collection").filter(|c| is_collection(c))?;
    if let Some(alt) = media.attr("alt") {
        let alt = alt.as_str()?;
        if has_unsafe_control(alt) || alt.contains(['\n', '\r']) || alt.trim() != alt {
            return None;
        }
    }
    // In the order of the syntax: collection, width, height, layout, size.
    let mut query: Vec<String> = Vec::new();
    if !collection.is_empty() {
        query.push(format!("collection={collection}"));
    }
    push_pixels(media, "width", &mut query)?;
    push_pixels(media, "height", &mut query)?;
    match single.attr("layout") {
        None => {}
        Some(layout) => {
            let layout = layout.as_str().filter(|l| LAYOUTS.contains(l))?;
            if layout != "center" {
                query.push(format!("layout={layout}"));
            }
        }
    }
    push_size(single, &mut query)?;
    let mut dest = format!("adf:media/{id}");
    if !query.is_empty() {
        dest.push('?');
        dest.push_str(&query.join("&"));
    }
    Some(dest)
}

/// Adds `name=<n>` for the `width` or `height` of `media` when it is present. `None` when it is
/// not an integer in `1..=MAX_PIXELS`.
fn push_pixels(media: &Node<'_>, name: &str, query: &mut Vec<String>) -> Option<()> {
    if let Some(value) = media.attr(name) {
        let n = value.as_u64().filter(|n| (1..=MAX_PIXELS).contains(n))?;
        query.push(format!("{name}={n}"));
    }
    Some(())
}

/// Adds `size=<n>%` or `size=<n>px` for the `width` and `widthType` of a `mediaSingle`. `None`
/// when the pair has no such form (a `widthType` alone, a percentage above 100, …).
fn push_size(single: &Node<'_>, query: &mut Vec<String>) -> Option<()> {
    let Some(width) = single.attr("width") else {
        return single.attr("widthType").is_none().then_some(());
    };
    let n = width.as_u64()?;
    let size = match single.attr("widthType").map(Value::as_str) {
        None | Some(Some("percentage")) if (1..=100).contains(&n) => format!("{n}%"),
        Some(Some("pixel")) if (1..=MAX_PIXELS).contains(&n) => format!("{n}px"),
        _ => return None,
    };
    query.push(format!("size={size}"));
    Some(())
}

/// A UUID: 8-4-4-4-12 hex digits, either case.
fn is_uuid(id: &str) -> bool {
    id.len() == 36
        && id.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Plain flavour: `[kind: alt]` (or `[kind]` without alt text) for a Media Services file, which
/// has no URL in ADF. The alt text is escaped, with line breaks and control characters removed
/// (`in_table`: written inside a GFM table cell).
pub(super) fn marker(kind: &str, alt: Option<&str>, in_table: bool) -> String {
    let mut md = format!("[{kind}");
    let alt = marker_alt(alt);
    if !alt.is_empty() {
        md.push_str(": ");
        escape_text(
            &alt,
            TextEscape {
                context: if in_table {
                    TextContext::TableCell
                } else {
                    TextContext::Block
                },
                roundtrip: false,
                line_start: false,
                block_start: false,
                line_end: false,
                in_link: true,
            },
            &mut md,
        );
    }
    md.push(']');
    md
}

/// Plain flavour: one `[attachment: alt]` marker per `media` node of a `mediaGroup`.
pub(super) fn group_markers(group: &Node<'_>, in_table: bool) -> Vec<String> {
    group
        .content
        .iter()
        .filter(|n| n.ty == "media")
        .map(|media| marker("attachment", media.attr_str("alt"), in_table))
        .collect()
}

/// 0..=64 characters from `[A-Za-z0-9_-]`.
fn is_collection(collection: &str) -> bool {
    collection.len() <= MAX_COLLECTION_LEN
        && collection
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}
