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

/// The destination `adf:media/<id>?…` of a single file image, or the first reason why the
/// `mediaSingle` `single` and its only child `media` hold something the syntax cannot say: marks,
/// children, other attributes, a malformed id or collection, a size out of range, or alt text that
/// Markdown → ADF would change (control characters, line breaks, surrounding whitespace). The
/// reason names what to change, so it can be shown to the user.
pub(super) fn file_media_dest(single: &Node<'_>, media: &Node<'_>) -> Result<String, String> {
    if media.attr_str("type") != Some("file") {
        return Err("the media is not a file".to_owned());
    }
    if !media.marks.is_empty() || !single.marks.is_empty() {
        return Err("a mark (border, link, …) on the media has no adf:media form".to_owned());
    }
    if !media.content.is_empty() {
        return Err("the media has children".to_owned());
    }
    if let Some(name) = media.attr_names().find(|n| !MEDIA_ATTRS.contains(n)) {
        return Err(format!(
            "the media attribute '{}' has no adf:media form",
            clip(name)
        ));
    }
    if let Some(name) = single.attr_names().find(|n| !SINGLE_ATTRS.contains(n)) {
        return Err(format!(
            "the mediaSingle attribute '{}' has no adf:media form",
            clip(name)
        ));
    }
    let id = media
        .attr_str("id")
        .filter(|id| is_uuid(id))
        .ok_or("the media id is not a UUID (an attachment id is not a media id)")?;
    let collection = media
        .attr_str("collection")
        .filter(|c| is_collection(c))
        .ok_or("the media collection is missing or has characters outside A-Z a-z 0-9 - _ (at most 64)")?;
    if let Some(alt) = media.attr("alt") {
        let alt = alt.as_str().ok_or("the alt text is not a string")?;
        if has_unsafe_control(alt) || alt.contains(['\n', '\r']) || alt.trim() != alt {
            return Err(
                "the alt text has control characters, line breaks or surrounding spaces".to_owned(),
            );
        }
    }
    // In the order of the syntax: collection, width, height, layout, size.
    let mut query: Vec<String> = Vec::new();
    if !collection.is_empty() {
        query.push(format!("collection={collection}"));
    }
    push_pixels(media, "width", &mut query)?;
    push_pixels(media, "height", &mut query)?;
    if let Some(layout) = single.attr("layout") {
        let layout = layout
            .as_str()
            .filter(|l| LAYOUTS.contains(l))
            .ok_or("the layout is not one of the mediaSingle layouts")?;
        if layout != "center" {
            query.push(format!("layout={layout}"));
        }
    }
    push_size(single, &mut query)?;
    let mut dest = format!("adf:media/{id}");
    if !query.is_empty() {
        dest.push('?');
        dest.push_str(&query.join("&"));
    }
    Ok(dest)
}

/// An attribute name for a message: control characters escaped, at most 40 characters.
fn clip(name: &str) -> String {
    name.escape_debug().take(40).collect()
}

/// Adds `name=<n>` for the `width` or `height` of `media` when it is present; an error when it is
/// not an integer in `1..=MAX_PIXELS`.
fn push_pixels(media: &Node<'_>, name: &str, query: &mut Vec<String>) -> Result<(), String> {
    if let Some(value) = media.attr(name) {
        let n = value
            .as_u64()
            .filter(|n| (1..=MAX_PIXELS).contains(n))
            .ok_or_else(|| format!("the media {name} is not an integer from 1 to {MAX_PIXELS}"))?;
        query.push(format!("{name}={n}"));
    }
    Ok(())
}

/// Adds `size=<n>%` or `size=<n>px` for the `width` and `widthType` of a `mediaSingle`; an error
/// when the pair has no such form (a `widthType` alone, a percentage above 100, …).
fn push_size(single: &Node<'_>, query: &mut Vec<String>) -> Result<(), String> {
    const NO_FORM: &str = "the mediaSingle width and widthType have no size= form";
    let Some(width) = single.attr("width") else {
        return if single.attr("widthType").is_none() {
            Ok(())
        } else {
            Err(NO_FORM.to_owned())
        };
    };
    let n = width.as_u64().ok_or(NO_FORM)?;
    let size = match single.attr("widthType").map(Value::as_str) {
        None | Some(Some("percentage")) if (1..=100).contains(&n) => format!("{n}%"),
        Some(Some("pixel")) if (1..=MAX_PIXELS).contains(&n) => format!("{n}px"),
        _ => return Err(NO_FORM.to_owned()),
    };
    query.push(format!("size={size}"));
    Ok(())
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
