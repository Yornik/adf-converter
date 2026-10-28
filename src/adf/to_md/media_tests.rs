//! Media Services files in ADF → Markdown: the `adf:media` image syntax (roundtrip flavour) and
//! the markers that keep files visible in the plain flavour.

#![allow(clippy::needless_pass_by_value, clippy::too_many_lines)]

use serde_json::{Value, json};

use super::to_markdown;
use crate::diagnostics::Code;
use crate::limits::Limits;
use crate::options::{AdfToMdOptions, MdFlavor};

const ID: &str = "11111111-2222-3333-4444-555555555555";

fn doc(content: Value) -> Value {
    json!({"type": "doc", "version": 1, "content": content})
}

fn convert(content: Value, flavor: MdFlavor) -> crate::Conversion<String> {
    let opts = AdfToMdOptions {
        flavor,
        limits: Limits::default(),
    };
    to_markdown(&doc(content), &opts).unwrap()
}

fn md(content: Value) -> String {
    convert(content, MdFlavor::Roundtrip).output
}

fn plain(content: Value) -> String {
    convert(content, MdFlavor::Plain).output
}

fn codes(content: Value, flavor: MdFlavor) -> Vec<Code> {
    convert(content, flavor)
        .warnings
        .into_iter()
        .map(|w| w.code)
        .collect()
}

fn assert_no_warnings(content: Value, flavor: MdFlavor) {
    assert_eq!(codes(content, flavor), Vec::<Code>::new());
}

fn para(text: &str) -> Value {
    json!({"type": "paragraph", "content": [{"type": "text", "text": text}]})
}

fn file(attrs: Value) -> Value {
    let mut base = json!({"type": "file", "id": ID, "collection": ""});
    if let (Some(base), Some(extra)) = (base.as_object_mut(), attrs.as_object()) {
        for (k, v) in extra {
            if v.is_null() {
                base.remove(k);
            } else {
                base.insert(k.clone(), v.clone());
            }
        }
    }
    json!({"type": "media", "attrs": base})
}

fn single(media: Value, attrs: Value) -> Value {
    json!({"type": "mediaSingle", "attrs": attrs, "content": [media]})
}

/// A `mediaSingle` with default layout around a file with the given extra attributes (a `null`
/// removes an attribute).
fn image(media_attrs: Value) -> Value {
    single(file(media_attrs), json!({"layout": "center"}))
}

fn inline_media(attrs: Value) -> Value {
    json!({"type": "mediaInline", "attrs": attrs})
}

fn t(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn p(inline: &[Value]) -> Value {
    json!({"type": "paragraph", "content": inline})
}

// ---------------------------------------------------------------------------------------------
// Roundtrip: the syntax
// ---------------------------------------------------------------------------------------------

#[test]
fn full_example_from_the_spec() {
    let content = json!([single(
        file(json!({
            "collection": "contentId-123456", "alt": "shot.png", "width": 1512, "height": 945
        })),
        json!({"layout": "wide", "width": 720, "widthType": "pixel"})
    )]);
    assert_eq!(
        md(content.clone()),
        format!(
            "![shot.png](adf:media/{ID}?collection=contentId-123456&width=1512&height=945&layout=wide&size=720px)\n"
        )
    );
    assert_no_warnings(content, MdFlavor::Roundtrip);
}

#[test]
fn defaults_write_no_query() {
    let none = json!([image(json!({}))]);
    assert_eq!(md(none.clone()), format!("![](adf:media/{ID})\n"));
    assert_no_warnings(none, MdFlavor::Roundtrip);
    assert_eq!(
        md(json!([image(json!({"alt": "a"}))])),
        format!("![a](adf:media/{ID})\n")
    );
    // An empty alt and an absent one read back the same.
    assert_eq!(
        md(json!([image(json!({"alt": ""}))])),
        format!("![](adf:media/{ID})\n")
    );
    // An absent layout is `center`, the default.
    assert_eq!(
        md(json!([single(file(json!({})), json!({}))])),
        format!("![](adf:media/{ID})\n")
    );
    let no_attrs = json!([{"type": "mediaSingle", "content": [file(json!({}))]}]);
    assert_eq!(md(no_attrs), format!("![](adf:media/{ID})\n"));
}

#[test]
fn each_parameter_alone() {
    let cases = [
        (
            file(json!({"collection": "comment-container-1"})),
            json!({"layout": "center"}),
            "?collection=comment-container-1",
        ),
        (
            file(json!({"width": 1512})),
            json!({"layout": "center"}),
            "?width=1512",
        ),
        (
            file(json!({"height": 945})),
            json!({"layout": "center"}),
            "?height=945",
        ),
        (file(json!({})), json!({"layout": "wide"}), "?layout=wide"),
        (
            file(json!({})),
            json!({"layout": "center", "width": 50}),
            "?size=50%",
        ),
    ];
    for (media, attrs, query) in cases {
        assert_eq!(
            md(json!([single(media, attrs)])),
            format!("![](adf:media/{ID}{query})\n"),
            "{query}"
        );
    }
}

#[test]
fn every_layout() {
    for layout in [
        "wide",
        "full-width",
        "wrap-right",
        "wrap-left",
        "align-end",
        "align-start",
    ] {
        assert_eq!(
            md(json!([single(file(json!({})), json!({"layout": layout}))])),
            format!("![](adf:media/{ID}?layout={layout})\n")
        );
    }
}

#[test]
fn sizes() {
    let size = |attrs: Value| md(json!([single(file(json!({})), attrs)]));
    let at = |query: &str| format!("![](adf:media/{ID}?{query})\n");
    assert_eq!(size(json!({"layout": "center", "width": 1})), at("size=1%"));
    assert_eq!(
        size(json!({"layout": "center", "width": 100})),
        at("size=100%")
    );
    assert_eq!(
        size(json!({"layout": "center", "width": 50, "widthType": "percentage"})),
        at("size=50%")
    );
    assert_eq!(
        size(json!({"layout": "center", "width": 720, "widthType": "pixel"})),
        at("size=720px")
    );
    assert_eq!(
        size(json!({"layout": "center", "width": 1, "widthType": "pixel"})),
        at("size=1px")
    );
    assert_eq!(
        size(json!({"layout": "center", "width": 100, "widthType": "pixel"})),
        at("size=100px")
    );
    assert_eq!(
        size(json!({"layout": "center", "width": 100_000, "widthType": "pixel"})),
        at("size=100000px")
    );
}

#[test]
fn ids_are_written_as_given() {
    let upper = "AAAAAAAA-BBBB-2222-cccc-DDDDEEEEFFFF";
    let content = json!([image(json!({"id": upper}))]);
    assert_eq!(md(content), format!("![](adf:media/{upper})\n"));
}

#[test]
fn collections() {
    let at = |collection: &str| md(json!([image(json!({"collection": collection}))]));
    let long = "a".repeat(64);
    assert_eq!(
        at(&long),
        format!("![](adf:media/{ID}?collection={long})\n")
    );
    assert_eq!(
        at("Aa0_-"),
        format!("![](adf:media/{ID}?collection=Aa0_-)\n")
    );
    assert_eq!(at(""), format!("![](adf:media/{ID})\n"));
}

#[test]
fn local_ids_are_dropped_silently() {
    let content = json!([single(
        file(json!({"localId": "m-1", "alt": "x"})),
        json!({"layout": "center", "localId": "ms-1"})
    )]);
    assert_eq!(md(content.clone()), format!("![x](adf:media/{ID})\n"));
    assert_no_warnings(content, MdFlavor::Roundtrip);
}

#[test]
fn alt_text_is_escaped_like_an_external_image() {
    let alt = "A [chart] *x* `y` <b> \\ ](javascript:alert(1))";
    let file_md = md(json!([image(json!({"alt": alt}))]));
    let external_md = md(json!([single(
        json!({"type": "media", "attrs": {"type": "external", "url": "https://e.com/a.png", "alt": alt}}),
        json!({"layout": "center"})
    )]));
    let escaped = |markdown: &str| {
        markdown
            .strip_prefix("![")
            .and_then(|m| m.split_once("](").map(|(a, _)| a.to_owned()))
    };
    // Both writers escape the alt text the same way, up to the first `](` that is not escaped.
    assert_eq!(escaped(&file_md), escaped(&external_md));
    assert!(
        file_md.ends_with(&format!("](adf:media/{ID})\n")),
        "{file_md}"
    );
    assert!(!file_md.contains("javascript:alert(1))\n"), "{file_md}");
}

#[test]
fn alt_text_in_a_table_cell_escapes_pipes() {
    let table = json!([{"type": "table", "content": [
        {"type": "tableRow", "content": [
            {"type": "tableHeader", "content": [para("h")]}]},
        {"type": "tableRow", "content": [
            {"type": "tableCell", "content": [para("a"), image(json!({"alt": "x|y"}))]}]}
    ]}]);
    assert_eq!(
        md(table),
        format!("| h |\n| --- |\n| a![x\\|y](adf:media/{ID}) |\n")
    );
}

// ---------------------------------------------------------------------------------------------
// Roundtrip: everything the syntax cannot say stays a raw block
// ---------------------------------------------------------------------------------------------

/// Asserts that the block is written as a raw `adf` block with exactly one `W_ADF_PASSTHROUGH`.
fn assert_raw(name: &str, content: Value) {
    let result = convert(json!([content]), MdFlavor::Roundtrip);
    assert!(
        result.output.starts_with("```adf\n"),
        "{name}: {}",
        result.output
    );
    assert!(!result.output.contains("adf:media"), "{name}");
    let codes: Vec<Code> = result.warnings.iter().map(|w| w.code).collect();
    assert!(
        codes.contains(&Code::AdfPassthrough),
        "{name}: {:?}",
        result.warnings
    );
}

#[test]
fn each_condition_failing_alone_keeps_the_raw_block() {
    let long = "a".repeat(65);
    let media_cases: Vec<(&str, Value)> = vec![
        ("type link", file(json!({"type": "link"}))),
        ("type missing", file(json!({"type": null}))),
        ("id missing", file(json!({"id": null}))),
        ("id not a string", file(json!({"id": 5}))),
        ("id not a uuid", file(json!({"id": "legacy-id"}))),
        (
            "id without dashes",
            file(json!({"id": "11111111222233334444555555555555"})),
        ),
        (
            "id not hex",
            file(json!({"id": "1111111g-2222-3333-4444-555555555555"})),
        ),
        (
            "id too long",
            file(json!({"id": "111111111-2222-3333-4444-555555555555"})),
        ),
        (
            "id with a brace",
            file(json!({"id": "11111111-2222-3333-4444-55555555555)"})),
        ),
        ("collection missing", file(json!({"collection": null}))),
        ("collection not a string", file(json!({"collection": 1}))),
        ("collection too long", file(json!({"collection": long}))),
        (
            "collection with a space",
            file(json!({"collection": "a b"})),
        ),
        ("collection with a dot", file(json!({"collection": "a.b"}))),
        (
            "collection with a slash",
            file(json!({"collection": "a/b"})),
        ),
        (
            "collection with a query char",
            file(json!({"collection": "a&b=c"})),
        ),
        ("collection with a paren", file(json!({"collection": "a)"}))),
        (
            "collection non-ascii",
            file(json!({"collection": "caf\u{e9}"})),
        ),
        ("width zero", file(json!({"width": 0}))),
        ("width negative", file(json!({"width": -3}))),
        ("width fractional", file(json!({"width": 1.5}))),
        ("width float integer", file(json!({"width": 5.0}))),
        ("width string", file(json!({"width": "5"}))),
        ("width too large", file(json!({"width": 100_001}))),
        ("height zero", file(json!({"height": 0}))),
        ("height fractional", file(json!({"height": 2.5}))),
        ("height string", file(json!({"height": "5"}))),
        ("height too large", file(json!({"height": 100_001}))),
        ("alt not a string", file(json!({"alt": 5}))),
        ("alt null", file(json!({"alt": Value::Null, "x": 1}))),
        ("alt with a line feed", file(json!({"alt": "a\nb"}))),
        ("alt with a carriage return", file(json!({"alt": "a\rb"}))),
        (
            "alt with a control character",
            file(json!({"alt": "a\u{1}b"})),
        ),
        ("alt with a leading space", file(json!({"alt": " a"}))),
        ("alt with a trailing space", file(json!({"alt": "a "}))),
        ("extra media attribute", file(json!({"occurrenceKey": "k"}))),
        ("url on a file", file(json!({"url": "https://e.com/a.png"}))),
    ];
    for (name, media) in media_cases {
        // `alt: null` is an explicit JSON null, which `file` would remove; build it by hand.
        let media = if name == "alt null" {
            json!({"type": "media", "attrs": {"type": "file", "id": ID, "collection": "", "alt": null}})
        } else {
            media
        };
        assert_raw(name, single(media, json!({"layout": "center"})));
    }

    let single_cases: Vec<(&str, Value)> = vec![
        ("layout unknown", json!({"layout": "diagonal"})),
        ("layout not a string", json!({"layout": 1})),
        ("layout null", json!({"layout": null})),
        ("layout uppercase", json!({"layout": "Wide"})),
        ("single attribute", json!({"layout": "center", "foo": 1})),
        ("width zero", json!({"layout": "center", "width": 0})),
        (
            "percentage above 100",
            json!({"layout": "center", "width": 101}),
        ),
        (
            "percentage typed above 100",
            json!({"layout": "center", "width": 101, "widthType": "percentage"}),
        ),
        (
            "fractional width",
            json!({"layout": "center", "width": 50.5}),
        ),
        ("string width", json!({"layout": "center", "width": "50"})),
        (
            "width type alone",
            json!({"layout": "center", "widthType": "pixel"}),
        ),
        (
            "unknown width type",
            json!({"layout": "center", "width": 50, "widthType": "em"}),
        ),
        (
            "null width type",
            json!({"layout": "center", "width": 50, "widthType": null}),
        ),
        (
            "zero pixels",
            json!({"layout": "center", "width": 0, "widthType": "pixel"}),
        ),
        (
            "too many pixels",
            json!({"layout": "center", "width": 100_001, "widthType": "pixel"}),
        ),
    ];
    for (name, attrs) in single_cases {
        assert_raw(name, single(file(json!({})), attrs));
    }
}

#[test]
fn structure_that_the_syntax_cannot_say_keeps_the_raw_block() {
    let center = json!({"layout": "center"});
    let caption = json!({"type": "caption", "content": [t("Figure 1")]});
    assert_raw(
        "caption",
        json!({"type": "mediaSingle", "attrs": center, "content": [file(json!({})), caption]}),
    );
    assert_raw(
        "two files",
        json!({"type": "mediaSingle", "attrs": center, "content": [file(json!({})), file(json!({}))]}),
    );
    assert_raw(
        "no media",
        json!({"type": "mediaSingle", "attrs": center, "content": []}),
    );
    assert_raw(
        "link on the image",
        json!({"type": "mediaSingle", "attrs": center,
            "marks": [{"type": "link", "attrs": {"href": "https://example.com"}}],
            "content": [file(json!({}))]}),
    );
    assert_raw(
        "border on the media",
        json!({"type": "mediaSingle", "attrs": center, "content": [
            {"type": "media", "attrs": {"type": "file", "id": ID, "collection": ""},
             "marks": [{"type": "border", "attrs": {"color": "#091e4224", "size": 2}}]}]}),
    );
}

#[test]
fn file_media_that_stays_raw_is_not_changed() {
    // Same output as before: whole node as JSON, nothing dropped.
    let content = json!([image(json!({"id": "legacy-id"}))]);
    let out = md(content);
    assert!(out.contains("\"id\": \"legacy-id\""), "{out}");
}

// ---------------------------------------------------------------------------------------------
// Roundtrip: contexts
// ---------------------------------------------------------------------------------------------

#[test]
fn top_level_between_paragraphs() {
    assert_eq!(
        md(json!([
            para("before"),
            image(json!({"alt": "x"})),
            para("after")
        ])),
        format!("before\n\n![x](adf:media/{ID})\n\nafter\n")
    );
}

#[test]
fn list_items() {
    let only_image = json!([{"type": "bulletList", "content": [
        {"type": "listItem", "content": [{"type": "paragraph"}, image(json!({"alt": "x"}))]}]}]);
    assert_eq!(md(only_image), format!("- ![x](adf:media/{ID})\n"));
    let text_and_image = json!([{"type": "orderedList", "attrs": {"order": 1}, "content": [
        {"type": "listItem", "content": [para("item"), image(json!({"alt": "y"}))]}]}]);
    assert_eq!(
        md(text_and_image),
        format!("1. item\n\n   ![y](adf:media/{ID})\n")
    );
}

#[test]
fn quotes_panels_and_expands() {
    let content = json!([
        {"type": "blockquote", "content": [para("q"), image(json!({"alt": "q"}))]},
        {"type": "panel", "attrs": {"panelType": "info"}, "content": [image(json!({"alt": "p"}))]},
        {"type": "expand", "attrs": {"title": "More"}, "content": [image(json!({"alt": "e"}))]},
    ]);
    assert_eq!(
        md(content.clone()),
        format!(
            "> q\n>\n> ![q](adf:media/{ID})\n\n> [!NOTE]\n> ![p](adf:media/{ID})\n\n<details>\n<summary>More</summary>\n\n![e](adf:media/{ID})\n\n</details>\n"
        )
    );
    assert_no_warnings(content, MdFlavor::Roundtrip);
}

#[test]
fn table_cells_append_the_images_after_the_cell_text() {
    let table = json!([{"type": "table", "content": [
        {"type": "tableRow", "content": [
            {"type": "tableHeader", "content": [para("shot")]},
            {"type": "tableHeader", "content": [para("note")]}]},
        {"type": "tableRow", "content": [
            {"type": "tableCell", "content": [para("see"), image(json!({"alt": "a", "width": 3, "height": 4}))]},
            {"type": "tableCell", "content": [para("n")]}]}
    ]}]);
    let out = md(table.clone());
    assert_eq!(
        out,
        format!(
            "| shot | note |\n| --- | --- |\n| see![a](adf:media/{ID}?width=3&height=4) | n |\n"
        )
    );
    assert_no_warnings(table, MdFlavor::Roundtrip);
}

#[test]
fn box_cells() {
    let boxed = json!([{"type": "table", "content": [{"type": "tableRow", "content": [
        {"type": "tableCell", "content": [para("intro"), image(json!({"alt": "b"})), para("outro")]}]}]}]);
    assert_eq!(
        md(boxed),
        format!("::: cell\nintro\n\n![b](adf:media/{ID})\n\noutro\n:::\n")
    );
}

#[test]
fn nested_expand_in_a_cell() {
    let nested = json!([{"type": "table", "content": [{"type": "tableRow", "content": [
        {"type": "tableCell", "content": [
            {"type": "nestedExpand", "attrs": {"title": "N"}, "content": [image(json!({"alt": "n"}))]}]}]}]}]);
    let out = md(nested);
    assert!(out.contains(&format!("![n](adf:media/{ID})")), "{out}");
    assert!(!out.contains("```adf"), "{out}");
}

#[test]
fn an_inexpressible_image_in_a_cell_still_passes_the_table_through() {
    let table = json!([{"type": "table", "content": [
        {"type": "tableRow", "content": [{"type": "tableHeader", "content": [para("h")]}]},
        {"type": "tableRow", "content": [
            {"type": "tableCell", "content": [para("a"), image(json!({"id": "legacy-id"}))]}]}
    ]}]);
    let out = md(table);
    assert!(out.starts_with("```adf\n"), "{out}");
}

#[test]
fn media_group_and_media_inline_keep_their_raw_blocks_when_the_link_cannot_say_them() {
    // A file without a name has no link text; the link syntax (`adf:attachment`) needs one.
    let group = json!({"type": "mediaGroup", "content": [file(json!({}))]});
    let out = md(json!([group]));
    assert!(out.starts_with("```adf\n"), "{out}");
    let inline = p(&[
        t("see "),
        inline_media(json!({"type": "file", "id": ID, "collection": ""})),
    ]);
    let out = md(json!([inline]));
    assert!(out.starts_with("```adf\n"), "{out}");
}

// ---------------------------------------------------------------------------------------------
// Plain flavour
// ---------------------------------------------------------------------------------------------

fn cell_table(cell_content: Value) -> Value {
    json!({"type": "table", "content": [
        {"type": "tableRow", "content": [
            {"type": "tableHeader", "content": [para("h1")]},
            {"type": "tableHeader", "content": [para("h2")]}]},
        {"type": "tableRow", "content": [
            {"type": "tableCell", "content": cell_content},
            {"type": "tableCell", "content": [para("x")]}]}
    ]})
}

#[test]
fn plain_markers_for_files() {
    assert_eq!(
        plain(json!([image(json!({"alt": "shot.png"}))])),
        "[image: shot.png]\n"
    );
    assert_eq!(plain(json!([image(json!({}))])), "[image]\n");
    let group =
        json!({"type": "mediaGroup", "content": [file(json!({"alt": "r.pdf"})), file(json!({}))]});
    assert_eq!(
        plain(json!([group])),
        "[attachment: r.pdf]\\\n[attachment]\n"
    );
}

#[test]
fn plain_alt_text_in_markers_is_one_line_and_escaped() {
    assert_eq!(
        plain(json!([image(json!({"alt": "a\n  b\u{1}*c*"}))])),
        "[image: a b\\*c\\*]\n"
    );
}

#[test]
fn plain_table_cells_keep_their_media() {
    // Before: `| a |  |` and `| |  |`; the images vanished.
    let with_image = json!([cell_table(json!([
        para("a"),
        image(json!({"alt": "s.png"}))
    ]))]);
    assert_eq!(
        plain(with_image.clone()),
        "| h1 | h2 |\n| --- | --- |\n| a<br>[image: s.png] | x |\n"
    );
    assert_no_warnings(with_image, MdFlavor::Plain);

    let only_image = json!([cell_table(json!([image(json!({}))]))]);
    assert_eq!(
        plain(only_image),
        "| h1 | h2 |\n| --- | --- |\n| [image] | x |\n"
    );

    let group = json!({"type": "mediaGroup", "content": [file(json!({"alt": "r.pdf"})), file(json!({"alt": "l|g.txt"}))]});
    assert_eq!(
        plain(json!([cell_table(json!([group]))])),
        "| h1 | h2 |\n| --- | --- |\n| [attachment: r.pdf]<br>[attachment: l\\|g.txt] | x |\n"
    );

    let external = single(
        json!({"type": "media", "attrs": {"type": "external", "url": "https://e.com/a.png", "alt": "ext"}}),
        json!({"layout": "center"}),
    );
    assert_eq!(
        plain(json!([cell_table(json!([para("a"), external]))])),
        "| h1 | h2 |\n| --- | --- |\n| a<br>![ext](https://e.com/a.png) | x |\n"
    );
}

#[test]
fn plain_cell_with_a_captioned_image() {
    let captioned = json!({"type": "mediaSingle", "attrs": {"layout": "center"}, "content": [
        file(json!({"alt": "s.png"})),
        {"type": "caption", "content": [t("Figure 1")]}]});
    assert_eq!(
        plain(json!([cell_table(json!([captioned]))])),
        "| h1 | h2 |\n| --- | --- |\n| [image: s.png]<br>Figure 1 | x |\n"
    );
}

#[test]
fn plain_media_nested_in_a_cell_leaves_a_marker() {
    let panel = json!({"type": "panel", "attrs": {"panelType": "info"}, "content": [
        {"type": "mediaGroup", "content": [file(json!({"alt": "r.pdf"}))]}]});
    let out = plain(json!([cell_table(json!([panel]))]));
    assert_eq!(
        out,
        "| h1 | h2 |\n| --- | --- |\n| \\[attachment: r.pdf\\] | x |\n"
    );
    let item = json!({"type": "bulletList", "content": [
        {"type": "listItem", "content": [p(&[t("see "), inline_media(json!({"type": "file", "id": ID, "collection": "", "alt": "x.zip"}))])]}]});
    let out = plain(json!([cell_table(json!([item]))]));
    assert_eq!(
        out,
        "| h1 | h2 |\n| --- | --- |\n| - see \\[attachment: x.zip\\] | x |\n"
    );
}

#[test]
fn plain_media_inline_is_a_marker_in_every_inline_context() {
    let m = |alt: Option<&str>| {
        let mut attrs = json!({"type": "file", "id": ID, "collection": ""});
        if let (Some(alt), Some(map)) = (alt, attrs.as_object_mut()) {
            map.insert("alt".to_owned(), json!(alt));
        }
        inline_media(attrs)
    };
    // Before: `see  now` (two spaces), an empty `-` item and a bare `## H`.
    assert_eq!(
        plain(json!([p(&[t("see "), m(None), t(" now")])])),
        "see [attachment] now\n"
    );
    assert_eq!(
        plain(json!([p(&[t("see "), m(Some("x.zip")), t(" now")])])),
        "see [attachment: x.zip] now\n"
    );
    assert_eq!(
        plain(
            json!([{"type": "heading", "attrs": {"level": 2}, "content": [t("H "), m(Some("x.zip"))]}])
        ),
        "## H [attachment: x.zip]\n"
    );
    assert_eq!(
        plain(json!([{"type": "bulletList", "content": [
            {"type": "listItem", "content": [p(&[m(None)])]},
            {"type": "listItem", "content": [p(&[m(Some("a"))])]}]}])),
        "- [attachment]\n- [attachment: a]\n"
    );
    assert_eq!(plain(json!([p(&[m(None)])])), "[attachment]\n");
    assert_eq!(
        plain(json!([p(&[m(None), m(None)])])),
        "[attachment][attachment]\n"
    );
    assert_eq!(
        plain(json!([p(&[m(Some("a")), t("(b)")])])),
        "[attachment: a]\\(b)\n"
    );
    assert_eq!(
        plain(json!([p(&[m(Some("*a*[b]"))])])),
        "[attachment: \\*a\\*\\[b\\]]\n"
    );
    assert_eq!(
        plain(
            json!([{"type": "taskList", "attrs": {"localId": "t"}, "content": [
            {"type": "taskItem", "attrs": {"localId": "i", "state": "TODO"}, "content": [t("do "), m(Some("f"))]}]}])
        ),
        "- [ ] do [attachment: f]\n"
    );
    assert_eq!(
        plain(json!([cell_table(json!([p(&[t("a"), m(Some("p|q"))])]))])),
        "| h1 | h2 |\n| --- | --- |\n| a[attachment: p\\|q] | x |\n"
    );
}

#[test]
fn plain_media_inline_marks_do_not_matter_and_nothing_warns() {
    let content = json!([p(&[t("a "), {
        let mut node = inline_media(json!({"type": "file", "id": ID, "collection": ""}));
        node["marks"] = json!([{"type": "link", "attrs": {"href": "https://example.com"}}]);
        node
    }])]);
    assert_eq!(plain(content.clone()), "a [attachment]\n");
    assert_no_warnings(content, MdFlavor::Plain);
}

#[test]
fn plain_media_in_other_blocks_stays_visible() {
    let group = json!({"type": "mediaGroup", "content": [file(json!({"alt": "r.pdf"}))]});
    let out = plain(json!([
        {"type": "blockquote", "content": [group.clone()]},
        {"type": "expand", "attrs": {"title": "E"}, "content": [image(json!({"alt": "i"}))]},
        {"type": "bulletList", "content": [{"type": "listItem", "content": [para("l"), image(json!({}))]}]},
    ]));
    assert!(out.contains("[attachment: r.pdf]"), "{out}");
    assert!(out.contains("[image: i]"), "{out}");
    assert!(out.contains("[image]"), "{out}");
    // A `media` node outside `mediaSingle` and `mediaGroup` is invalid ADF but not nothing.
    assert_eq!(
        plain(json!([file(json!({"alt": "m"}))])),
        "[attachment: m]\n"
    );
}

#[test]
fn plain_output_is_deterministic_and_has_no_double_spaces() {
    let content = json!([
        cell_table(json!([para("a"), image(json!({"alt": "s"}))])),
        p(&[
            t("x "),
            inline_media(json!({"type": "file", "id": ID, "collection": ""})),
            t(" y")
        ]),
    ]);
    let first = plain(content.clone());
    assert_eq!(first, plain(content));
    assert!(!first.contains("  "), "{first}");
}

/// The `W_ADF_PASSTHROUGH` message names the condition that keeps a file image raw, instead of
/// one generic sentence.
#[test]
fn passthrough_message_names_the_failing_condition() {
    let reason = |content: Value| -> String {
        convert(json!([content]), MdFlavor::Roundtrip)
            .warnings
            .into_iter()
            .find(|w| w.code == Code::AdfPassthrough)
            .map(|w| w.message)
            .unwrap_or_default()
    };
    let cases: Vec<(Value, &str)> = vec![
        (image(json!({"id": "10001"})), "not a UUID"),
        (image(json!({"collection": "a.b"})), "collection"),
        (image(json!({"occurrenceKey": "k"})), "occurrenceKey"),
        (image(json!({"width": 0})), "media width"),
        (image(json!({"alt": " padded "})), "alt text"),
        (
            single(file(json!({})), json!({"layout": "diagonal"})),
            "layout",
        ),
        (
            single(file(json!({})), json!({"layout": "center", "width": 150})),
            "width and widthType",
        ),
        (
            single(
                json!({"type": "media", "attrs": {"type": "file", "id": ID, "collection": ""},
                       "marks": [{"type": "border", "attrs": {"color": "#000000", "size": 2}}]}),
                json!({"layout": "center"}),
            ),
            "mark",
        ),
    ];
    for (content, needle) in cases {
        let message = reason(content.clone());
        assert!(message.contains(needle), "{needle}: {message}");
    }
}
