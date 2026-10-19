//! The round-trip guarantee from `docs/syntax.md`:
//!
//! ```text
//! md_to_adf(adf_to_md(x)) == normalize(x)
//! ```
//!
//! checked for every fixture (`x = md_to_adf(fixture)`), for Jira-shaped documents with editor
//! bookkeeping (`localId`s, column widths, panel icons), plus a smoke test of the `plain` flavour.

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

mod support;

use adf_converter::{ImagePolicy, MdFlavor, MdToAdfOptions};
use serde_json::{Value, json};

#[test]
fn every_fixture_round_trips() {
    let fixtures = support::fixtures();
    let mut failures = Vec::new();
    for fixture in &fixtures {
        let result = support::md_to_value(&fixture.source, &fixture.options)
            .and_then(|(x, _)| support::check_round_trip(&x, &fixture.options));
        if let Err(e) = result {
            failures.push(format!(
                "{}:\n    {}",
                fixture.name,
                e.replace('\n', "\n    ")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} fixtures do not round-trip:\n\n{}\n",
        failures.len(),
        fixtures.len(),
        failures.join("\n\n")
    );
}

/// `--md-flavor plain` never fails on converter output, never writes ```` ```adf ```` blocks and
/// reports no warnings (it is lossy by design).
#[test]
fn plain_flavour_smoke_test() {
    let fixtures = support::fixtures();
    let mut failures = Vec::new();
    for fixture in &fixtures {
        let x = match support::md_to_value(&fixture.source, &fixture.options) {
            Ok((x, _)) => x,
            Err(e) => {
                failures.push(format!("{}: {e}", fixture.name));
                continue;
            }
        };
        match support::adf_to_markdown(&x, MdFlavor::Plain) {
            Err(e) => failures.push(format!("{}: {e}", fixture.name)),
            Ok((markdown, warnings)) => {
                if has_adf_fence(&markdown) && !has_adf_code_block(&x) {
                    failures.push(format!(
                        "{}: plain output contains an ```adf block:\n{}",
                        fixture.name,
                        support::excerpt(&markdown, 30)
                    ));
                }
                if !warnings.is_empty() {
                    failures.push(format!(
                        "{}: plain flavour reported warnings:\n  {}",
                        fixture.name,
                        support::render_diagnostics(&warnings)
                    ));
                }
                if !markdown.is_empty() && !markdown.ends_with('\n') {
                    failures.push(format!(
                        "{}: plain output does not end with a newline",
                        fixture.name
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}\n", failures.join("\n\n"));
}

fn has_adf_fence(markdown: &str) -> bool {
    markdown.lines().any(|line| {
        let line = line.trim_start_matches(['>', ' ']);
        let fence = line.trim_start_matches(['`', '~']);
        line.len() - fence.len() >= 3 && fence.trim_start().starts_with("adf")
    })
}

/// A code block whose language is literally `adf` (raw-adf disabled) legitimately prints as one.
fn has_adf_code_block(doc: &Value) -> bool {
    support::nodes_of_type(doc, "codeBlock")
        .iter()
        .any(|n| n.pointer("/attrs/language").and_then(Value::as_str) == Some("adf"))
}

fn doc(content: Value) -> Value {
    let mut doc = serde_json::Map::new();
    doc.insert("type".to_owned(), Value::from("doc"));
    doc.insert("version".to_owned(), Value::from(1));
    doc.insert("content".to_owned(), content);
    Value::Object(doc)
}

fn round_trip_document(name: &str, x: &Value, options: &MdToAdfOptions) -> Option<String> {
    support::check_round_trip(x, options)
        .err()
        .map(|e| format!("{name}:\n    {}", e.replace('\n', "\n    ")))
}

/// Documents as Jira stores them: `localId`s everywhere, attributes Markdown cannot express, and
/// `\n` inside text. After `normalize`, they survive ADF → Markdown → ADF.
#[test]
fn jira_shaped_documents_round_trip_after_normalize() {
    let defaults = MdToAdfOptions::default();
    let external_images = MdToAdfOptions {
        images: ImagePolicy::External,
        ..MdToAdfOptions::default()
    };
    let cases: Vec<(&str, Value, &MdToAdfOptions)> = vec![
        (
            "localIds on blocks",
            doc(json!([
                {"type":"heading","attrs":{"level":2,"localId":"h-1"},"content":[{"type":"text","text":"Status"}]},
                {"type":"paragraph","attrs":{"localId":"p-1"},"content":[{"type":"text","text":"line one\nline two"}]},
                {"type":"bulletList","attrs":{"localId":"b-1"},"content":[
                    {"type":"listItem","attrs":{"localId":"li-1"},"content":[{"type":"paragraph","content":[{"type":"text","text":"item"}]}]}]},
                {"type":"codeBlock","attrs":{"localId":"c-1"},"content":[{"type":"text","text":"keep\nnewlines"}]}
            ])),
            &defaults,
        ),
        (
            "task ids compared by position",
            doc(json!([
                {"type":"taskList","attrs":{"localId":"jira-list"},"content":[
                    {"type":"taskItem","attrs":{"localId":"jira-item-1","state":"TODO"},"content":[{"type":"text","text":"one"}]},
                    {"type":"taskItem","attrs":{"localId":"jira-item-2","state":"DONE"},"content":[{"type":"text","text":"two"}]}]}
            ])),
            &defaults,
        ),
        (
            "table attributes without a Markdown form",
            doc(json!([
                {"type":"table","attrs":{"isNumberColumnEnabled":true,"layout":"wide","width":900,"displayMode":"fixed","localId":"t-1"},"content":[
                    {"type":"tableRow","content":[
                        {"type":"tableHeader","attrs":{"colwidth":[200],"background":"#DEEBFF"},"content":[{"type":"paragraph","content":[{"type":"text","text":"Key","marks":[{"type":"strong"}]}]}]},
                        {"type":"tableHeader","attrs":{"colwidth":[700]},"content":[{"type":"paragraph","content":[{"type":"text","text":"Value","marks":[{"type":"strong"}]}]}]}]},
                    {"type":"tableRow","content":[
                        {"type":"tableCell","attrs":{"colwidth":[200]},"content":[{"type":"paragraph","content":[{"type":"text","text":"region"}]}]},
                        {"type":"tableCell","attrs":{"background":"#FFFAE6"},"content":[{"type":"paragraph","content":[{"type":"text","text":"eu-west-1","marks":[{"type":"code"}]}]}]}]}]}
            ])),
            &defaults,
        ),
        (
            "panel icons and expand localId",
            doc(json!([
                {"type":"panel","attrs":{"panelType":"warning","panelIcon":":warning:","panelIconId":"26a0","panelIconText":"⚠️","localId":"pn-1"},"content":[
                    {"type":"paragraph","content":[{"type":"text","text":"Careful."}]}]},
                {"type":"expand","attrs":{"title":"Details","localId":"e-1"},"content":[
                    {"type":"paragraph","content":[{"type":"text","text":"Hidden."}]}]}
            ])),
            &defaults,
        ),
        (
            "external media size",
            doc(json!([
                {"type":"mediaSingle","attrs":{"layout":"center","width":50,"widthType":"percentage"},"content":[
                    {"type":"media","attrs":{"type":"external","url":"https://example.com/diagram.png","alt":"diagram","width":640,"height":480}}]}
            ])),
            &external_images,
        ),
    ];
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(name, x, options)| round_trip_document(name, x, options))
        .collect();
    assert!(failures.is_empty(), "{}\n", failures.join("\n\n"));
}

// ---------------------------------------------------------------------------------------------
// normalize() itself (independent of the converter)
// ---------------------------------------------------------------------------------------------

#[test]
fn normalize_replaces_task_ids_by_position_and_drops_other_local_ids() {
    let x = doc(json!([
        {"type":"paragraph","attrs":{"localId":"p"},"content":[{"type":"text","text":"a"}]},
        {"type":"taskList","attrs":{"localId":"x"},"content":[
            {"type":"taskItem","attrs":{"localId":"y","state":"TODO"}},
            {"type":"taskList","attrs":{"localId":"z"},"content":[{"type":"taskItem","attrs":{"localId":"w","state":"DONE"}}]}]},
        {"type":"expand","attrs":{"localId":"e"},"content":[{"type":"paragraph"}]}
    ]));
    let expected = doc(json!([
        {"type":"paragraph","content":[{"type":"text","text":"a"}]},
        {"type":"taskList","attrs":{"localId":"task-1"},"content":[
            {"type":"taskItem","attrs":{"localId":"task-2","state":"TODO"}},
            {"type":"taskList","attrs":{"localId":"task-3"},"content":[{"type":"taskItem","attrs":{"localId":"task-4","state":"DONE"}}]}]},
        {"type":"expand","attrs":{},"content":[{"type":"paragraph"}]}
    ]));
    assert_eq!(support::normalize(&x), expected);
}

#[test]
fn normalize_splits_newlines_in_text_but_not_in_code() {
    let strong = json!([{"type":"strong"}]);
    let x = doc(json!([
        {"type":"paragraph","content":[{"type":"text","text":"a\n\nb\n","marks":strong}]},
        {"type":"codeBlock","content":[{"type":"text","text":"x\ny"}]}
    ]));
    let expected = doc(json!([
        {"type":"paragraph","content":[
            {"type":"text","text":"a","marks":strong},
            {"type":"hardBreak"},
            {"type":"hardBreak"},
            {"type":"text","text":"b","marks":strong},
            {"type":"hardBreak"}]},
        {"type":"codeBlock","content":[{"type":"text","text":"x\ny"}]}
    ]));
    assert_eq!(support::normalize(&x), expected);
}

#[test]
fn normalize_drops_attributes_without_a_markdown_form() {
    let x = doc(json!([
        {"type":"table","attrs":{"layout":"wide","width":1},"content":[{"type":"tableRow","content":[
            {"type":"tableCell","attrs":{"colwidth":[1],"background":"#ffffff"},"content":[{"type":"paragraph"}]},
            {"type":"tableCell","attrs":{"colspan":2,"colwidth":[1,2]},"content":[{"type":"paragraph"}]}]}]},
        {"type":"panel","attrs":{"panelType":"note","panelColor":"#ffffff","panelIcon":":x:"},"content":[{"type":"paragraph"}]}
    ]));
    let expected = doc(json!([
        {"type":"table","attrs":{"isNumberColumnEnabled":false,"layout":"default"},"content":[{"type":"tableRow","content":[
            {"type":"tableCell","content":[{"type":"paragraph"}]},
            {"type":"tableCell","attrs":{"colspan":2},"content":[{"type":"paragraph"}]}]}]},
        {"type":"panel","attrs":{"panelType":"note"},"content":[{"type":"paragraph"}]}
    ]));
    assert_eq!(support::normalize(&x), expected);
}

#[test]
fn normalize_is_idempotent() {
    let x = doc(json!([
        {"type":"paragraph","attrs":{"localId":"p"},"content":[{"type":"text","text":"a\nb"}]},
        {"type":"taskList","attrs":{"localId":"x"},"content":[{"type":"taskItem","attrs":{"localId":"y","state":"TODO"}}]}
    ]));
    let once = support::normalize(&x);
    assert_eq!(support::normalize(&once), once);
}
