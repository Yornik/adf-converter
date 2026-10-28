//! `[file name](adf:attachment/<uuid>)` in Markdown → ADF: a `mediaGroup` for a paragraph of only
//! attachment links, a `mediaInline` anywhere else, and the failures that keep the link text.

// JSON builder helpers take `Value`s by value so expectations read naturally.
#![allow(clippy::too_many_lines, clippy::needless_pass_by_value)]

use serde_json::{Value, json};

use crate::Target;
use crate::diagnostics::{Code, Diagnostic};
use crate::options::{Extensions, ImagePolicy, MdToAdfOptions, SoftBreak};

const A: &str = "66666666-7777-8888-9999-000000000000";
const B: &str = "66666666-7777-8888-9999-000000000001";

/// Converts, checks that the document is valid for both products, and returns it with the
/// warnings.
fn run_with(md: &str, opts: &MdToAdfOptions) -> (Value, Vec<Diagnostic>) {
    let conversion = super::to_adf(md, opts).unwrap_or_else(|e| panic!("{md:?}: {e}"));
    let value = conversion.output.to_value().unwrap();
    for target in [Target::Jira, Target::Confluence] {
        let errors: Vec<_> = crate::adf::validate::validate(&value, target)
            .into_iter()
            .filter(Diagnostic::is_error)
            .collect();
        assert!(errors.is_empty(), "{md:?}: {errors:?}");
    }
    let again = super::to_adf(md, opts).unwrap();
    assert_eq!(
        serde_json::to_string(&again.output).unwrap(),
        serde_json::to_string(&value).unwrap(),
        "not deterministic"
    );
    (value, conversion.warnings)
}

fn run(md: &str) -> (Value, Vec<Diagnostic>) {
    run_with(md, &MdToAdfOptions::default())
}

fn content(md: &str) -> Value {
    run(md).0["content"].clone()
}

fn codes(warnings: &[Diagnostic]) -> Vec<Code> {
    warnings.iter().map(|w| w.code).collect()
}

fn media(id: &str, collection: &str, alt: &str) -> Value {
    json!({"type": "media", "attrs": {"type": "file", "id": id, "collection": collection, "alt": alt}})
}

fn group(files: Vec<Value>) -> Value {
    json!({"type": "mediaGroup", "content": files})
}

fn inline(id: &str, collection: &str, alt: &str) -> Value {
    json!({"type": "mediaInline", "attrs": {"type": "file", "id": id, "collection": collection, "alt": alt}})
}

fn t(s: &str) -> Value {
    json!({"type": "text", "text": s})
}

fn p(inlines: Vec<Value>) -> Value {
    json!({"type": "paragraph", "content": inlines})
}

#[test]
fn a_paragraph_of_only_links_is_a_media_group() {
    assert_eq!(
        content(&format!(
            "[report.pdf](adf:attachment/{A}) [log.txt](adf:attachment/{B}?collection=contentId-123456)"
        )),
        json!([group(vec![
            media(A, "", "report.pdf"),
            media(B, "contentId-123456", "log.txt")
        ])])
    );
}

#[test]
fn one_link_is_a_group_of_one() {
    assert_eq!(
        content(&format!("[report.pdf](adf:attachment/{A})")),
        json!([group(vec![media(A, "", "report.pdf")])])
    );
}

#[test]
fn soft_breaks_hard_breaks_and_wide_gaps_keep_the_group() {
    let expected = json!([group(vec![media(A, "", "a"), media(B, "", "b")])]);
    for separator in [" ", "\n", "   ", "\t", "\\\n", "  \n", "<br>"] {
        assert_eq!(
            content(&format!(
                "[a](adf:attachment/{A}){separator}[b](adf:attachment/{B})"
            )),
            expected,
            "{separator:?}"
        );
    }
    // The line break may also come from `--softbreak hardbreak`.
    let opts = MdToAdfOptions {
        softbreak: SoftBreak::HardBreak,
        ..MdToAdfOptions::default()
    };
    let (value, _) = run_with(
        &format!("[a](adf:attachment/{A})\n[b](adf:attachment/{B})"),
        &opts,
    );
    assert_eq!(value["content"], expected);
}

#[test]
fn text_around_a_link_makes_it_inline() {
    assert_eq!(
        content(&format!("see [report.pdf](adf:attachment/{A}) now")),
        json!([p(vec![t("see "), inline(A, "", "report.pdf"), t(" now")])])
    );
    // Any other inline node counts as text.
    assert_eq!(
        content(&format!(
            "[@Jane](adf:mention/5b10) [report.pdf](adf:attachment/{A})"
        )),
        json!([p(vec![
            json!({"type": "mention", "attrs": {"id": "5b10", "text": "@Jane"}}),
            t(" "),
            inline(A, "", "report.pdf")
        ])])
    );
    assert_eq!(
        content(&format!("[a](adf:attachment/{A}) and")),
        json!([p(vec![inline(A, "", "a"), t(" and")])])
    );
}

#[test]
fn links_with_nothing_between_them_stay_inline() {
    assert_eq!(
        content(&format!("[a](adf:attachment/{A})[b](adf:attachment/{B})")),
        json!([p(vec![inline(A, "", "a"), inline(B, "", "b")])])
    );
    // One gap is enough for a group; a missing gap anywhere makes the paragraph inline.
    assert_eq!(
        content(&format!(
            "[a](adf:attachment/{A}) [b](adf:attachment/{B})[c](adf:attachment/{A})"
        )),
        json!([p(vec![
            inline(A, "", "a"),
            t(" "),
            inline(B, "", "b"),
            inline(A, "", "c")
        ])])
    );
}

#[test]
fn the_kind_and_scheme_are_case_insensitive_and_the_id_keeps_its_case() {
    let id = "AAAAAAAA-BBBB-2222-CCCC-DDDDEEEEFFFF";
    assert_eq!(
        content(&format!("[a](ADF:Attachment/{id})")),
        json!([group(vec![media(id, "", "a")])])
    );
}

#[test]
fn the_file_name_is_trimmed_and_may_not_be_empty() {
    assert_eq!(
        content(&format!("[  a b  ](adf:attachment/{A})")),
        json!([group(vec![media(A, "", "a b")])])
    );
    for md in [
        format!("x [](adf:attachment/{A}) y"),
        format!("x [ ](adf:attachment/{A}) y"),
        format!("x [\u{a0}](adf:attachment/{A}) y"),
    ] {
        let (value, warnings) = run(&md);
        assert_eq!(codes(&warnings), [Code::InvalidExtensionArg], "{md:?}");
        assert!(
            warnings[0].message.contains("needs the file name"),
            "{warnings:?}"
        );
        assert!(warnings[0].hint.is_some());
        assert_eq!(
            value["content"][0]["content"].as_array().unwrap().len(),
            1,
            "{md:?}"
        );
    }
}

#[test]
fn escapes_and_entities_in_the_file_name_are_decoded() {
    assert_eq!(
        content(&format!("[a \\[1\\] \\*b\\* &amp; c](adf:attachment/{A})")),
        json!([group(vec![media(A, "", "a [1] *b* & c")])])
    );
}

#[test]
fn control_characters_are_removed_and_reported() {
    let (value, warnings) = run(&format!("[a\u{7}b](adf:attachment/{A})"));
    assert_eq!(value["content"], json!([group(vec![media(A, "", "ab")])]));
    assert_eq!(codes(&warnings), [Code::ControlChar]);
}

#[test]
fn formatting_is_dropped_with_a_warning() {
    for md in [
        format!("**[a](adf:attachment/{A})**"),
        format!("[**a**](adf:attachment/{A})"),
        format!("[`a`](adf:attachment/{A})"),
    ] {
        let (value, warnings) = run(&md);
        assert_eq!(
            value["content"],
            json!([group(vec![media(A, "", "a")])]),
            "{md}"
        );
        assert_eq!(codes(&warnings), [Code::MarkDropped], "{md}");
    }
    let (value, warnings) = run(&format!("x *[a](adf:attachment/{A})* y"));
    assert_eq!(
        value["content"],
        json!([p(vec![t("x "), inline(A, "", "a"), t(" y")])])
    );
    assert_eq!(codes(&warnings), [Code::MarkDropped]);
    // A link title is not an attribute of the node and is not reported, as for other adf: links.
    let (value, warnings) = run(&format!("[a](adf:attachment/{A} \"Title\")"));
    assert_eq!(value["content"], json!([group(vec![media(A, "", "a")])]));
    assert_eq!(codes(&warnings), []);
}

#[test]
fn containers_decide_between_group_and_inline() {
    // A list item cannot hold a mediaGroup (the schema), so its links stay inline even alone.
    let list = content(&format!(
        "- [a](adf:attachment/{A})\n- [b](adf:attachment/{A}) [c](adf:attachment/{B})"
    ));
    assert_eq!(
        list[0]["content"][0]["content"][0],
        p(vec![inline(A, "", "a")])
    );
    assert_eq!(
        list[0]["content"][1]["content"][0],
        p(vec![inline(A, "", "b"), t(" "), inline(B, "", "c")])
    );
    // So do the second paragraph of a list item and a heading.
    let item = content(&format!("- text\n\n  [a](adf:attachment/{A})"));
    assert_eq!(
        item[0]["content"][0]["content"][1],
        p(vec![inline(A, "", "a")])
    );
    let heading = content(&format!("# [a](adf:attachment/{A})"));
    assert_eq!(heading[0]["content"], json!([inline(A, "", "a")]));
    // A GFM table cell is one paragraph with its alignment mark: inline as well.
    let table = content(&format!(
        "| h |\n|---|\n| [a](adf:attachment/{A}) [b](adf:attachment/{B}) |"
    ));
    assert_eq!(
        table[0]["content"][1]["content"][0]["content"],
        json!([p(vec![inline(A, "", "a"), t(" "), inline(B, "", "b")])])
    );
    // Quotes, panels, expands, nested expands and box cells take the group.
    let g = group(vec![media(A, "", "a")]);
    assert_eq!(
        content(&format!("> [a](adf:attachment/{A})"))[0]["content"],
        json!([g])
    );
    assert_eq!(
        content(&format!("> [!NOTE]\n> [a](adf:attachment/{A})"))[0]["content"],
        json!([g])
    );
    assert_eq!(
        content(&format!(
            "<details><summary>S</summary>\n\n[a](adf:attachment/{A})\n\n</details>"
        ))[0]["content"],
        json!([g])
    );
    assert_eq!(
        content(&format!("::: cell\n[a](adf:attachment/{A})\n:::"))[0]["content"][0]["content"][0]
            ["content"],
        json!([g])
    );
    let nested = content(&format!(
        "::: cell\n<details><summary>S</summary>\n\n[a](adf:attachment/{A})\n\n</details>\n:::"
    ));
    assert_eq!(
        nested[0]["content"][0]["content"][0]["content"][0]["content"],
        json!([g])
    );
    // A task item holds inline content only.
    let task = content(&format!("- [ ] [a](adf:attachment/{A})"));
    assert_eq!(
        task[0]["content"][0]["content"],
        json!([inline(A, "", "a")])
    );
}

#[test]
fn a_demoted_heading_of_only_links_becomes_a_group_where_groups_are_allowed() {
    // In a quote a heading becomes a paragraph (W_HEADING_DEMOTED); a paragraph of links there is a group.
    let (value, warnings) = run(&format!("> # [a](adf:attachment/{A})"));
    assert_eq!(codes(&warnings), [Code::HeadingDemoted]);
    assert_eq!(
        value["content"][0]["content"],
        json!([group(vec![media(A, "", "a")])])
    );
}

#[test]
fn bad_arguments_keep_the_link_text() {
    let cases = [
        (
            "x [a.pdf](adf:attachment/not-a-uuid) y".to_owned(),
            "is not a UUID",
        ),
        (
            "x [a.pdf](adf:attachment/) y".to_owned(),
            "needs the file's media ID",
        ),
        (
            "x [a.pdf](adf:attachment) y".to_owned(),
            "needs the file's media ID",
        ),
        (
            format!("x [a.pdf](adf:attachment/{A}?) y"),
            "empty parameter",
        ),
        (
            format!("x [a.pdf](adf:attachment/{A}?collection) y"),
            "has no value",
        ),
        (
            format!("x [a.pdf](adf:attachment/{A}?collection=a%20b) y"),
            "collection 'a%20b' is not valid",
        ),
        (
            format!("x [a.pdf](adf:attachment/{A}?collection=a&collection=b) y"),
            "repeats the parameter",
        ),
        (
            format!("x [a.pdf](adf:attachment/{A}?width=1) y"),
            "unknown parameter 'width'",
        ),
        (
            format!("x [a.pdf](adf:attachment/{A}?layout=wide) y"),
            "unknown parameter 'layout'",
        ),
        (
            format!("x [a.pdf](adf:attachment/{A}?size=50%) y"),
            "unknown parameter 'size'",
        ),
        (
            format!("x [a.pdf](adf:attachment/{A}?colour=red) y"),
            "unknown parameter 'colour'",
        ),
    ];
    for (md, expected) in cases {
        let (value, warnings) = run(&md);
        assert_eq!(value["content"], json!([p(vec![t("x a.pdf y")])]), "{md}");
        assert_eq!(codes(&warnings), [Code::InvalidExtensionArg], "{md}");
        assert!(
            warnings[0].message.contains(expected),
            "{md}: {}",
            warnings[0].message
        );
        assert!(
            warnings[0]
                .hint
                .as_deref()
                .is_some_and(|h| h.contains("adf:attachment/<uuid>"))
        );
    }
}

#[test]
fn one_bad_link_leaves_the_others_a_group() {
    let (value, warnings) = run(&format!("[a](adf:attachment/{A}) [b](adf:attachment/bad)"));
    // The text "b" is left, so the paragraph is no longer only links.
    assert_eq!(
        value["content"],
        json!([p(vec![inline(A, "", "a"), t(" b")])])
    );
    assert_eq!(codes(&warnings), [Code::InvalidExtensionArg]);
}

#[test]
fn the_image_form_is_not_an_attachment() {
    let (value, warnings) = run(&format!("x ![report.pdf](adf:attachment/{A}) y"));
    assert_eq!(value["content"], json!([p(vec![t("x report.pdf y")])]));
    assert_eq!(codes(&warnings), [Code::UnknownExtension]);
    assert!(warnings[0].message.contains("link form"), "{warnings:?}");
    assert!(
        warnings[0]
            .hint
            .as_deref()
            .is_some_and(|h| h.contains("[file name](adf:attachment/"))
    );
    // And a media image written as a link is the mirror case, unchanged.
    let (_, warnings) = run(&format!("[x](adf:media/{A})"));
    assert_eq!(codes(&warnings), [Code::UnknownExtension]);
}

#[test]
fn disabling_the_media_extension_keeps_the_text() {
    let opts = MdToAdfOptions {
        extensions: Extensions::parse("alerts").unwrap(),
        ..MdToAdfOptions::default()
    };
    let (value, warnings) = run_with(&format!("x [a.pdf](adf:attachment/{A}) y"), &opts);
    assert_eq!(value["content"], json!([p(vec![t("x a.pdf y")])]));
    assert_eq!(codes(&warnings), [Code::ExtensionDisabled]);
    assert!(
        warnings[0].message.contains("'media' extension"),
        "{warnings:?}"
    );
    // The `media` name alone switches on both forms.
    let only_media = MdToAdfOptions {
        extensions: Extensions::parse("media").unwrap(),
        ..MdToAdfOptions::default()
    };
    let (value, warnings) = run_with(&format!("[a](adf:attachment/{A})"), &only_media);
    assert_eq!(value["content"], json!([group(vec![media(A, "", "a")])]));
    assert_eq!(codes(&warnings), []);
}

#[test]
fn images_policy_drop_removes_attachments_and_the_other_policies_do_not_matter() {
    let md = format!(
        "before [a.pdf](adf:attachment/{A}) after\n\n[b](adf:attachment/{B}) [c](adf:attachment/bad)"
    );
    for images in [ImagePolicy::Link, ImagePolicy::External] {
        let opts = MdToAdfOptions {
            images,
            ..MdToAdfOptions::default()
        };
        let (value, warnings) = run_with(&md, &opts);
        assert_eq!(
            value["content"][0],
            p(vec![t("before "), inline(A, "", "a.pdf"), t(" after")])
        );
        assert_eq!(codes(&warnings), [Code::InvalidExtensionArg], "{images:?}");
    }
    let opts = MdToAdfOptions {
        images: ImagePolicy::Drop,
        ..MdToAdfOptions::default()
    };
    let (value, warnings) = run_with(&md, &opts);
    assert_eq!(value["content"], json!([p(vec![t("before  after")])]));
    assert_eq!(
        codes(&warnings),
        [Code::ImageDropped, Code::ImageDropped, Code::ImageDropped]
    );
    // Dropped even when the extension is off or the arguments are bad.
    let off = MdToAdfOptions {
        images: ImagePolicy::Drop,
        extensions: Extensions::none(),
        ..MdToAdfOptions::default()
    };
    let (_, warnings) = run_with(
        &format!("x [a](adf:attachment/{A}) [b](adf:attachment/bad)"),
        &off,
    );
    assert_eq!(codes(&warnings), [Code::ImageDropped, Code::ImageDropped]);
}

#[test]
fn warnings_point_at_the_link() {
    let (_, warnings) = run("line one\n\nfoo [a](adf:attachment/bad) bar");
    let location = warnings[0].location.as_ref().unwrap();
    assert_eq!((location.line, location.column), (Some(3), Some(5)));
}

#[test]
fn a_link_around_an_image_in_the_file_name_follows_the_node() {
    let opts = MdToAdfOptions {
        images: ImagePolicy::External,
        ..MdToAdfOptions::default()
    };
    let (value, warnings) = run_with(
        &format!("x [![i](https://example.com/i.png) a.pdf](adf:attachment/{A}) y"),
        &opts,
    );
    assert_eq!(codes(&warnings), [Code::BlockHoisted]);
    assert_eq!(
        value["content"][0],
        p(vec![t("x "), inline(A, "", "a.pdf")])
    );
    assert_eq!(value["content"][1]["type"], "mediaSingle");
}

#[test]
fn thousands_of_links_stay_linear_and_bounded() {
    let link = format!("[f.pdf](adf:attachment/{A})");
    let n = 10_000;
    let start = std::time::Instant::now();
    let group_md = vec![link.as_str(); n].join(" ");
    let (value, warnings) = run(&group_md);
    assert_eq!(codes(&warnings), []);
    assert_eq!(value["content"][0]["content"].as_array().unwrap().len(), n);
    let inline_md = format!("see {}", vec![link.as_str(); n].join(" and "));
    let (value, warnings) = run(&inline_md);
    assert_eq!(codes(&warnings), []);
    assert_eq!(
        value["content"][0]["content"].as_array().unwrap().len(),
        2 * n
    );
    assert!(start.elapsed().as_secs() < 20, "{:?}", start.elapsed());
}
