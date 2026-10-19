//! Golden tests: every `tests/fixtures/**/<case>.md` is converted with its options and compared
//! with the blessed `<case>.adf.json` (and `<case>.diag.jsonl` for warnings). Every output is also
//! checked to be valid ADF.
//!
//! Bless (write the expected files instead of comparing) with:
//!
//! ```text
//! ADF_BLESS=1 cargo test --locked --test golden
//! ```
//!
//! Blessed files are generated, never edited by hand. See `tests/fixtures/README.md`.

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use support::Fixture;

const BLESS_HINT: &str = "run `ADF_BLESS=1 cargo test --locked --test golden` and review the diff";

/// Suffixes of the files that belong to a fixture.
const COMPANION_SUFFIXES: &[&str] = &[".adf.json", ".diag.jsonl", ".opts.json"];

fn blessing() -> bool {
    std::env::var("ADF_BLESS").is_ok_and(|v| v == "1")
}

#[test]
fn fixtures_match_blessed_output() {
    let bless = blessing();
    let fixtures = support::fixtures();
    let mut failures = Vec::new();
    let mut written = Vec::new();
    for fixture in &fixtures {
        if let Err(failure) = check_fixture(fixture, bless, &mut written) {
            failures.push(format!(
                "{}:\n    {}",
                fixture.name,
                failure.replace('\n', "\n    ")
            ));
        }
    }
    failures.extend(orphan_files());
    if bless && !written.is_empty() {
        eprintln!(
            "blessed {} file(s):\n  {}",
            written.len(),
            written.join("\n  ")
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {} fixtures failed:\n\n{}\n",
        failures.len(),
        fixtures.len(),
        failures.join("\n\n")
    );
}

fn check_fixture(fixture: &Fixture, bless: bool, written: &mut Vec<String>) -> Result<(), String> {
    let (doc, warnings) = support::md_to_value(&fixture.source, &fixture.options).map_err(|e| {
        format!(
            "{e}\n(fixtures must convert successfully; error cases are CLI cases in tests/cli/)"
        )
    })?;
    let problems = support::adf_problems(&doc, fixture.options.target);
    if !problems.is_empty() {
        return Err(format!(
            "output is not valid ADF ({} problems):\n  {}\n{}",
            problems.len(),
            problems.join("\n  "),
            support::excerpt(&support::to_pretty_json(&doc), 60)
        ));
    }
    let adf = support::to_pretty_json(&doc);
    let diag = support::render_diag_jsonl(&warnings);
    let adf_result = check_file(&fixture.sibling(".adf.json"), &adf, bless, written);
    let diag_result = check_file(&fixture.sibling(".diag.jsonl"), &diag, bless, written);
    match (adf_result, diag_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(a), Ok(())) => Err(a),
        (Ok(()), Err(d)) => Err(d),
        (Err(a), Err(d)) => Err(format!("{a}\n{d}")),
    }
}

/// Compares `actual` with the file, or writes it when blessing. An empty `actual` means the file
/// must not exist (fixtures without warnings have no `.diag.jsonl`).
fn check_file(
    path: &Path,
    actual: &str,
    bless: bool,
    written: &mut Vec<String>,
) -> Result<(), String> {
    let label = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    let expected = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{label}: cannot read: {e}")),
    };
    if expected.as_deref() == Some(actual) || (expected.is_none() && actual.is_empty()) {
        return Ok(());
    }
    if bless {
        if actual.is_empty() {
            fs::remove_file(path).map_err(|e| format!("{label}: cannot remove: {e}"))?;
        } else {
            fs::write(path, actual).map_err(|e| format!("{label}: cannot write: {e}"))?;
        }
        written.push(path.display().to_string());
        return Ok(());
    }
    Err(match expected {
        None => format!(
            "{label} is missing; the conversion produced:\n{}\n{BLESS_HINT}",
            support::excerpt(actual, 40)
        ),
        Some(_) if actual.is_empty() => {
            format!("{label} exists but the conversion produced no warnings; {BLESS_HINT}")
        }
        Some(expected) => format!(
            "{label} differs: {}\n{BLESS_HINT}",
            support::first_difference(&expected, actual)
        ),
    })
}

/// Blessed or options files without a matching `.md` input.
fn orphan_files() -> Vec<String> {
    let inputs: BTreeSet<_> = support::fixture_paths().into_iter().collect();
    let mut orphans = Vec::new();
    for path in support::files_under(&support::fixtures_dir()) {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let Some(suffix) = COMPANION_SUFFIXES.iter().find(|s| name.ends_with(*s)) else {
            continue;
        };
        let stem = name.trim_end_matches(suffix);
        if !inputs.contains(&path.with_file_name(format!("{stem}.md"))) {
            orphans.push(format!("{}: no matching {stem}.md input", path.display()));
        }
    }
    orphans
}

/// Every Markdown → ADF warning code is triggered by at least one fixture (`lossy/` has one per
/// code). Three codes are not required here:
///
/// - `W_NODE_NOT_IN_TARGET` is reserved and never emitted (see
///   `markdown_output_does_not_depend_on_the_target`).
/// - `W_SIZE_LIMIT` and `W_EMPTY_DOCUMENT` come from the CLI's self-check of the output
///   (`validate_adf`), not from `md_to_adf`, so this harness never sees them.
///   `markdown_fixtures_warn_from_the_self_check` in `tests/cli.rs` converts the `lossy/`
///   fixtures for them with the CLI.
#[test]
fn every_markdown_warning_is_covered_by_a_fixture() {
    use adf_converter::Code;
    let first = Code::ALL.iter().position(|c| *c == Code::MarkDropped);
    let last = Code::ALL.iter().position(|c| *c == Code::ExtensionDisabled);
    let (Some(first), Some(last)) = (first, last) else {
        panic!("Code::ALL lost MarkDropped or ExtensionDisabled");
    };
    let required: BTreeSet<&str> = Code::ALL[first..=last].iter().map(|c| c.as_str()).collect();
    let mut seen = BTreeSet::new();
    for path in support::files_under(&support::fixtures_dir()) {
        if !path.to_string_lossy().ends_with(".diag.jsonl") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("readable");
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let record: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("{}: invalid JSON line {line:?}: {e}", path.display()));
            if let Some(code) = record.get("code").and_then(serde_json::Value::as_str) {
                seen.insert(code.to_owned());
            }
        }
    }
    let missing: Vec<&str> = required
        .iter()
        .filter(|code| !seen.contains(**code))
        .copied()
        .collect();
    assert!(
        missing.is_empty(),
        "no blessed .diag.jsonl contains {missing:?}; add or fix a fixture in tests/fixtures/lossy/"
    );
}

/// Markdown never produces a node, mark or attribute that only one product accepts, so the target
/// changes nothing: every fixture converts to the same document and warnings for Jira and for
/// Confluence. That is why `W_NODE_NOT_IN_TARGET` is never emitted. The target matters only for
/// raw ```` ```adf ```` blocks, which fail (`E_RAW_ADF`) where the target rejects their nodes.
#[test]
fn markdown_output_does_not_depend_on_the_target() {
    use adf_converter::Target;
    let mut failures = Vec::new();
    for fixture in support::fixtures() {
        let other = adf_converter::MdToAdfOptions {
            target: match fixture.options.target {
                Target::Jira => Target::Confluence,
                Target::Confluence => Target::Jira,
            },
            ..fixture.options.clone()
        };
        let Ok((doc, warnings)) = support::md_to_value(&fixture.source, &fixture.options) else {
            continue; // reported by fixtures_match_blessed_output
        };
        match support::md_to_value(&fixture.source, &other) {
            Ok((other_doc, other_warnings)) => {
                if other_doc != doc {
                    failures.push(format!("{}: the document differs", fixture.name));
                }
                if support::render_diag_jsonl(&other_warnings)
                    != support::render_diag_jsonl(&warnings)
                {
                    failures.push(format!("{}: the warnings differ", fixture.name));
                }
            }
            // A raw block with a node the other target rejects.
            Err(_) if fixture.source.contains("```adf") => {}
            Err(e) => failures.push(format!("{}: fails for the other target: {e}", fixture.name)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // Most fixtures are for Jira; this one runs the extension nodes for Confluence.
    let path = support::fixtures_dir().join("extensions/target-confluence.md");
    let fixture = support::load_fixture(&path).expect("extensions/target-confluence fixture");
    assert_eq!(fixture.options.target, Target::Confluence);
}

/// Whether a node of type `outer` has a descendant of type `inner`.
fn holds(doc: &serde_json::Value, outer: &str, inner: &str) -> bool {
    support::nodes_of_type(doc, outer).into_iter().any(|node| {
        node.get("content")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|children| {
                children
                    .iter()
                    .any(|child| !support::nodes_of_type(child, inner).is_empty())
            })
    })
}

/// A blessed output agrees with whatever it is compared with, so a fixture can lose its point
/// without any test failing: an editor strips the trailing spaces of a hard break, or an input
/// edit turns a code block into a paragraph, and blessing records the new output. These checks
/// pin what the fixtures are about.
#[test]
fn fixtures_still_test_what_they_say() {
    use serde_json::Value;
    type Claim = (&'static str, &'static str, fn(&Value) -> bool);
    let claims: &[Claim] = &[
        (
            "commonmark/hard-breaks",
            "at least 5 hardBreak nodes (editors and write tools strip two trailing spaces)",
            |doc| support::nodes_of_type(doc, "hardBreak").len() >= 5,
        ),
        (
            "commonmark/soft-breaks-as-hardbreak-containers",
            "hard breaks in a heading, a task item, a list item and a quote",
            |doc| {
                ["heading", "taskItem", "listItem", "blockquote"]
                    .iter()
                    .all(|outer| holds(doc, outer, "hardBreak"))
            },
        ),
        (
            "commonmark/tabs",
            "a top-level code block indented with a tab",
            |doc| {
                doc["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "codeBlock"))
            },
        ),
        (
            "commonmark/ordered-lists",
            "an ordered list starting at 0",
            |doc| {
                support::nodes_of_type(doc, "orderedList")
                    .iter()
                    .any(|list| list["attrs"]["order"] == 0)
            },
        ),
        (
            "extensions/alert-in-nested-expand",
            "a panel inside a nested expand",
            |doc| holds(doc, "nestedExpand", "panel"),
        ),
        (
            "regression/marklassian-10",
            "a task list nested in a task list",
            |doc| holds(doc, "taskList", "taskList"),
        ),
        (
            "schema-gotchas/image-external-containers",
            "external images in a panel, an expand, a nested expand and a table cell",
            |doc| {
                ["panel", "expand", "nestedExpand", "tableCell"]
                    .iter()
                    .all(|outer| holds(doc, outer, "mediaSingle"))
            },
        ),
    ];
    let mut failures = Vec::new();
    for (name, claim, check) in claims {
        let path = support::fixtures_dir().join(format!("{name}.adf.json"));
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let doc: Value = serde_json::from_str(&text).expect("blessed JSON");
        if !check(&doc) {
            failures.push(format!("{name}: expected {claim}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The support code itself must not be vacuous: known-bad documents are rejected by the
/// independent checks, whatever the crate's validator says.
#[test]
fn validators_reject_known_bad_documents() {
    use serde_json::json;
    let para = |inline: serde_json::Value| json!({"type":"doc","version":1,"content":[{"type":"paragraph","content":[inline]}]});
    let bad = [
        ("empty text", para(json!({"type":"text","text":""}))),
        (
            "code + strong",
            para(json!({"type":"text","text":"x","marks":[{"type":"code"},{"type":"strong"}]})),
        ),
        (
            "marks: []",
            para(json!({"type":"text","text":"x","marks":[]})),
        ),
        (
            "null attr",
            json!({"type":"doc","version":1,"content":[{"type":"codeBlock","attrs":{"language":null}}]}),
        ),
        (
            "heading in quote",
            json!({"type":"doc","version":1,"content":[{"type":"blockquote","content":[{"type":"heading","attrs":{"level":1}}]}]}),
        ),
        (
            "list item starting with a list",
            json!({"type":"doc","version":1,"content":[{"type":"bulletList","content":[{"type":"listItem","content":[
                {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph"}]}]}]}]}]}),
        ),
        (
            "task list without localId",
            json!({"type":"doc","version":1,"content":[{"type":"taskList","attrs":{"localId":""},"content":[
                {"type":"taskItem","attrs":{"localId":"a","state":"TODO"}}]}]}),
        ),
        (
            "duplicate localId",
            json!({"type":"doc","version":1,"content":[{"type":"taskList","attrs":{"localId":"a"},"content":[
                {"type":"taskItem","attrs":{"localId":"a","state":"TODO"}}]}]}),
        ),
        (
            "ragged table",
            json!({"type":"doc","version":1,"content":[{"type":"table","content":[
                {"type":"tableRow","content":[{"type":"tableHeader","content":[{"type":"paragraph"}]},{"type":"tableHeader","content":[{"type":"paragraph"}]}]},
                {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph"}]}]}]}]}),
        ),
        (
            "status hex colour",
            para(json!({"type":"status","attrs":{"text":"X","color":"#ff0000"}})),
        ),
        (
            "unmerged text",
            json!({"type":"doc","version":1,"content":[{"type":"paragraph","content":[
                {"type":"text","text":"a"},{"type":"text","text":"b"}]}]}),
        ),
        (
            "marks out of order",
            para(json!({"type":"text","text":"x","marks":[{"type":"strong"},{"type":"em"}]})),
        ),
        (
            "newline in text",
            para(json!({"type":"text","text":"a\nb"})),
        ),
        (
            "unsafe link",
            para(
                json!({"type":"text","text":"x","marks":[{"type":"link","attrs":{"href":"javascript:alert(1)"}}]}),
            ),
        ),
        (
            "trailing hardBreak",
            json!({"type":"doc","version":1,"content":[{"type":"paragraph","content":[
                {"type":"text","text":"a"},{"type":"hardBreak"}]}]}),
        ),
        (
            "codeBlock wrap",
            json!({"type":"doc","version":1,"content":[{"type":"codeBlock","attrs":{"wrap":true}}]}),
        ),
        (
            "fractional level",
            json!({"type":"doc","version":1,"content":[{"type":"heading","attrs":{"level":1.5}}]}),
        ),
    ];
    for (what, doc) in bad {
        let problems = support::adf_problems(&doc, adf_converter::Target::Jira);
        assert!(!problems.is_empty(), "{what} was accepted: {doc}");
    }
}

#[test]
fn validators_accept_a_known_good_document() {
    let doc: serde_json::Value = serde_json::from_str(
        r#"{"type":"doc","version":1,"content":[
          {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"✍️ "},{"type":"text","text":"Summary","marks":[{"type":"strong"}]}]},
          {"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"link","attrs":{"href":"https://example.com"}},{"type":"code"}]},{"type":"hardBreak"},{"type":"text","text":"b\tc"}]},
          {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph"},{"type":"orderedList","attrs":{"order":3},"content":[{"type":"listItem","content":[{"type":"paragraph"}]}]}]}]},
          {"type":"taskList","attrs":{"localId":"l1"},"content":[{"type":"taskItem","attrs":{"localId":"i1","state":"DONE"},"content":[{"type":"text","text":"t"}]}]},
          {"type":"codeBlock","content":[{"type":"text","text":"x\n\ty"}]},
          {"type":"table","attrs":{"isNumberColumnEnabled":false,"layout":"default"},"content":[
            {"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"status","attrs":{"text":"OK","color":"green"}}]}]}]}]},
          {"type":"panel","attrs":{"panelType":"warning"},"content":[{"type":"paragraph"}]},
          {"type":"expand","attrs":{},"content":[{"type":"paragraph"}]}
        ]}"#,
    )
    .expect("valid JSON");
    assert_eq!(support::schema_errors(&doc), Vec::<String>::new());
    assert_eq!(support::invariant_errors(&doc), Vec::<String>::new());
}

#[test]
fn options_files_parse() {
    use adf_converter::{Extensions, HtmlPolicy, ImagePolicy, MdToAdfOptions, SoftBreak, Target};
    let options = support::parse_options(
        r#"{"target":"confluence","softbreak":"hardbreak","html":"drop","images":"external","extensions":"alerts,details"}"#,
    )
    .expect("valid options");
    assert_eq!(options.target, Target::Confluence);
    assert_eq!(options.softbreak, SoftBreak::HardBreak);
    assert_eq!(options.html, HtmlPolicy::Drop);
    assert_eq!(options.images, ImagePolicy::External);
    assert_eq!(
        options.extensions,
        Extensions::parse("alerts,details").expect("names")
    );
    let list = support::parse_options(r#"{"extensions":["mention","raw-adf"]}"#).expect("list");
    assert_eq!(
        list.extensions,
        Extensions::parse("mention,raw-adf").expect("names")
    );
    let none = support::parse_options(r#"{"extensions":[]}"#).expect("empty list");
    assert_eq!(none.extensions, Extensions::none());
    assert_eq!(
        support::parse_options("{}").expect("empty"),
        MdToAdfOptions::default()
    );
    for bad in [
        r#"{"softbrake":"space"}"#,
        r#"{"html":"strip"}"#,
        r#"{"extensions":"bogus"}"#,
        r#"{"target":"wiki"}"#,
        "[]",
    ] {
        assert!(support::parse_options(bad).is_err(), "accepted {bad}");
    }
}

#[test]
fn every_options_file_is_valid() {
    let mut errors = Vec::new();
    for path in support::files_under(&support::fixtures_dir()) {
        if path.to_string_lossy().ends_with(".opts.json") {
            let text = fs::read_to_string(&path).expect("readable");
            if let Err(e) = support::parse_options(&text) {
                errors.push(format!("{}: {e}", path.display()));
            }
        }
    }
    assert!(
        errors.is_empty(),
        "invalid options files:\n  {}",
        errors.join("\n  ")
    );
}

/// The JSON of each ```` ```adf ```` fence (one per line range, unindented) in a fixture input.
fn raw_adf_blocks(source: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in source.lines() {
        match current.as_mut() {
            None if line.trim_end() == "```adf" => current = Some(String::new()),
            None => {}
            Some(_) if line.trim_end() == "```" => blocks.extend(current.take()),
            Some(json) => {
                json.push_str(line);
                json.push('\n');
            }
        }
    }
    blocks
}

/// Fixtures must convert successfully, so every raw ADF block they splice in must itself be valid
/// ADF (checked here as the content of a document, independently of the converter).
#[test]
fn raw_adf_blocks_in_fixtures_are_valid() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for fixture in support::fixtures() {
        if !fixture.options.extensions.raw_adf {
            continue; // the block is an ordinary code block
        }
        for json in raw_adf_blocks(&fixture.source) {
            checked += 1;
            let nodes = match serde_json::from_str::<serde_json::Value>(&json) {
                Ok(serde_json::Value::Array(nodes)) => nodes,
                Ok(node) => vec![node],
                Err(e) => {
                    failures.push(format!("{}: not JSON: {e}", fixture.name));
                    continue;
                }
            };
            let doc = serde_json::json!({"type": "doc", "version": 1, "content": nodes});
            let mut problems = support::schema_errors(&doc);
            problems.extend(support::invariant_errors(&doc));
            if !problems.is_empty() {
                failures.push(format!("{}:\n  {}", fixture.name, problems.join("\n  ")));
            }
        }
    }
    assert!(
        checked >= 4,
        "expected several raw ADF fixtures, found {checked}"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn pretty_json_is_stable() {
    let value: serde_json::Value =
        serde_json::from_str(r#"{"type":"doc","version":1,"content":[{"type":"rule"}],"a":[]}"#)
            .expect("JSON");
    assert_eq!(
        support::to_pretty_json(&value),
        "{\n  \"type\": \"doc\",\n  \"version\": 1,\n  \"content\": [\n    {\n      \"type\": \"rule\"\n    }\n  ],\n  \"a\": []\n}\n"
    );
}
