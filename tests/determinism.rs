//! Determinism (`docs/cli-contract.md#determinism`): the same input and options always give
//! byte-identical output; CRLF and LF input give identical output; a UTF-8 BOM is ignored.

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

mod support;

use adf_converter::{MdFlavor, MdToAdfOptions};

const BOM: char = '\u{feff}';

/// Compact JSON and warnings of one conversion, or the error report.
fn convert(
    source: &str,
    options: &MdToAdfOptions,
) -> Result<(String, Vec<adf_converter::Diagnostic>), String> {
    support::md_to_value(source, options)
        .map(|(doc, warnings)| (support::to_compact_json(&doc), warnings))
}

/// The LF form of a fixture (the stored file may itself use CRLF or start with a BOM).
fn lf(source: &str) -> String {
    source.trim_start_matches(BOM).replace("\r\n", "\n")
}

#[test]
fn converting_twice_gives_identical_bytes() {
    let mut failures = Vec::new();
    for fixture in support::fixtures() {
        let first = convert(&fixture.source, &fixture.options);
        let second = convert(&fixture.source, &fixture.options);
        match (first, second) {
            (Ok(a), Ok(b)) if a == b => {}
            (Ok((a, _)), Ok((b, _))) if a != b => failures.push(format!(
                "{}: output differs between runs\n{}",
                fixture.name,
                support::first_difference(&a, &b)
            )),
            (Ok(_), Ok(_)) => {
                failures.push(format!("{}: warnings differ between runs", fixture.name));
            }
            (Err(e), _) | (_, Err(e)) => failures.push(format!("{}: {e}", fixture.name)),
        }
    }
    assert!(failures.is_empty(), "{}\n", failures.join("\n\n"));
}

#[test]
fn crlf_and_lf_give_identical_output() {
    let mut failures = Vec::new();
    for fixture in support::fixtures() {
        let unix = lf(&fixture.source);
        let windows = unix.replace('\n', "\r\n");
        match (
            convert(&unix, &fixture.options),
            convert(&windows, &fixture.options),
        ) {
            (Ok((a, wa)), Ok((b, wb))) => {
                if a != b {
                    failures.push(format!(
                        "{}: CRLF output differs from LF output\n{}",
                        fixture.name,
                        support::first_difference(&a, &b)
                    ));
                }
                let (pa, pb) = (
                    support::warning_positions(&wa),
                    support::warning_positions(&wb),
                );
                if pa != pb {
                    failures.push(format!(
                        "{}: CRLF warnings differ from LF warnings\n  LF:   {pa:?}\n  CRLF: {pb:?}",
                        fixture.name
                    ));
                }
            }
            (Err(e), _) | (_, Err(e)) => failures.push(format!("{}: {e}", fixture.name)),
        }
    }
    assert!(failures.is_empty(), "{}\n", failures.join("\n\n"));
}

#[test]
fn a_bom_is_ignored() {
    let mut failures = Vec::new();
    for fixture in support::fixtures() {
        let plain = lf(&fixture.source);
        let with_bom = format!("{BOM}{plain}");
        match (
            convert(&plain, &fixture.options),
            convert(&with_bom, &fixture.options),
        ) {
            (Ok((a, wa)), Ok((b, wb))) => {
                if a != b {
                    failures.push(format!(
                        "{}: output with a BOM differs\n{}",
                        fixture.name,
                        support::first_difference(&a, &b)
                    ));
                }
                // Codes and lines must match; whether a BOM counts as a column is not specified.
                let lines = |w: &[adf_converter::Diagnostic]| {
                    support::warning_positions(w)
                        .into_iter()
                        .map(|(code, line, _)| (code, line))
                        .collect::<Vec<_>>()
                };
                if lines(&wa) != lines(&wb) {
                    failures.push(format!("{}: warnings differ with a BOM", fixture.name));
                }
            }
            (Err(e), _) | (_, Err(e)) => failures.push(format!("{}: {e}", fixture.name)),
        }
    }
    assert!(failures.is_empty(), "{}\n", failures.join("\n\n"));
}

#[test]
fn adf_to_md_is_deterministic() {
    let mut failures = Vec::new();
    for fixture in support::fixtures() {
        let doc = match support::md_to_value(&fixture.source, &fixture.options) {
            Ok((doc, _)) => doc,
            Err(e) => {
                failures.push(format!("{}: {e}", fixture.name));
                continue;
            }
        };
        for flavor in [MdFlavor::Roundtrip, MdFlavor::Plain] {
            let first = support::adf_to_markdown(&doc, flavor);
            let second = support::adf_to_markdown(&doc, flavor);
            if first != second {
                failures.push(format!(
                    "{} ({flavor:?}): output differs between runs",
                    fixture.name
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}\n", failures.join("\n"));
}

/// Task `localId`s depend on the input: the same document gives the same IDs, a different
/// document gives different ones, and they never repeat within a document.
#[test]
fn task_local_ids_are_derived_from_the_input() {
    let ids = |source: &str| -> Vec<String> {
        let (doc, _) = support::md_to_value(source, &MdToAdfOptions::default()).expect("converts");
        support::assert_valid_adf(&doc);
        let mut ids: Vec<String> = ["taskList", "taskItem"]
            .iter()
            .flat_map(|ty| support::nodes_of_type(&doc, ty))
            .filter_map(|n| {
                n.pointer("/attrs/localId")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            })
            .collect();
        ids.sort();
        ids
    };
    let a = ids("- [ ] one\n- [x] two\n");
    assert_eq!(a.len(), 3, "one list and two items: {a:?}");
    assert_eq!(a, ids("- [ ] one\n- [x] two\n"));
    assert_eq!(
        a,
        ids("- [ ] one\r\n- [x] two\r\n"),
        "CRLF gives the same IDs"
    );
    let b = ids("- [ ] one\n- [x] three\n");
    assert!(
        a.iter().all(|id| !b.contains(id)),
        "different input, different IDs: {a:?} {b:?}"
    );
    let many = ids(&"- [ ] same\n".repeat(50));
    let unique: std::collections::BTreeSet<_> = many.iter().collect();
    assert_eq!(unique.len(), many.len(), "IDs are unique within a document");
}

/// The fixture set itself contains the stored CRLF and BOM inputs (see their `.gitattributes`).
#[test]
fn line_ending_fixtures_are_stored_verbatim() {
    let fixtures = support::fixtures();
    let crlf = fixtures
        .iter()
        .find(|f| f.name == "commonmark/line-endings-crlf")
        .expect("fixture commonmark/line-endings-crlf");
    assert!(
        crlf.source.contains("\r\n"),
        "the CRLF fixture lost its CRLF line endings"
    );
    let bom = fixtures
        .iter()
        .find(|f| f.name == "commonmark/byte-order-mark")
        .expect("fixture commonmark/byte-order-mark");
    assert!(bom.source.starts_with(BOM), "the BOM fixture lost its BOM");
}
