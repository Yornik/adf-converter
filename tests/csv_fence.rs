//! ```` ```csv ```` / ```` ```tsv ```` fences (issue #13, phase 2; `docs/syntax.md#inline-csv-tables`).
//!
//! The Markdown fixtures (`tests/fixtures/extensions/csv-fence-*.md`) and the CLI cases
//! (`tests/cli/md/convert-md-csv-*`) cover the mapping and the error output; these tests cover what
//! those cannot: agreement with `--from csv`, line numbers under CRLF and a BOM, the cell limit
//! shared with GFM tables, and the round trip.

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use adf_converter::{
    AdfToMdOptions, Code, CsvDelimiter, CsvToAdfOptions, Diagnostic, Diagnostics, Extensions,
    MdToAdfOptions, adf_to_md, csv_to_adf, md_to_adf,
};
use serde_json::Value;

const FENCE: &str = "```";

fn convert(markdown: &str, opts: &MdToAdfOptions) -> Result<(Value, Vec<Diagnostic>), Diagnostics> {
    let done = md_to_adf(markdown, opts)?;
    Ok((done.output.to_value().expect("serialises"), done.warnings))
}

fn ok(markdown: &str) -> (Value, Vec<Diagnostic>) {
    convert(markdown, &MdToAdfOptions::default())
        .unwrap_or_else(|e| panic!("conversion failed: {e:?}"))
}

fn err(markdown: &str) -> Diagnostic {
    convert(markdown, &MdToAdfOptions::default())
        .expect_err("conversion should fail")
        .0
        .into_iter()
        .next()
        .expect("an error")
}

fn fenced(info: &str, body: &str) -> String {
    format!("{FENCE}{info}\n{body}\n{FENCE}\n")
}

fn csv_doc(csv: &str, opts: &CsvToAdfOptions) -> Value {
    csv_to_adf(csv, opts)
        .expect("CSV converts")
        .output
        .to_value()
        .expect("serialises")
}

fn line_of(d: &Diagnostic) -> Option<u32> {
    d.location.as_ref().and_then(|l| l.line)
}

#[test]
fn a_fence_makes_the_table_that_csv_input_makes() {
    let cases: &[(&str, &str, CsvToAdfOptions)] = &[
        ("csv", "a,b\n1,2\n3,\"x,y\"", CsvToAdfOptions::default()),
        (
            "csv delimiter=semicolon",
            "a;b\n1;2",
            CsvToAdfOptions {
                delimiter: CsvDelimiter::parse("semicolon").unwrap(),
                ..CsvToAdfOptions::default()
            },
        ),
        (
            "csv header=false",
            "a,b\n1,2",
            CsvToAdfOptions {
                header: false,
                ..CsvToAdfOptions::default()
            },
        ),
        (
            "csv max-rows=1 align=numbers",
            "n,v\n1,2\n3,4",
            CsvToAdfOptions {
                max_rows: Some(1),
                align_numbers: true,
                ..CsvToAdfOptions::default()
            },
        ),
        (
            "tsv",
            "a\tb\n1\t2",
            CsvToAdfOptions {
                delimiter: CsvDelimiter::TAB,
                ..CsvToAdfOptions::default()
            },
        ),
    ];
    for (info, body, opts) in cases {
        let (doc, _) = ok(&fenced(info, body));
        let expected = csv_doc(body, opts);
        assert_eq!(doc["content"], expected["content"], "{info}: {body:?}");
    }
}

#[test]
fn tsv_defaults_to_a_tab_and_an_explicit_delimiter_wins() {
    let (tab, _) = ok(&fenced("tsv", "a\tb\n1\t2"));
    assert_eq!(
        tab["content"][0]["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let (comma, _) = ok(&fenced("tsv delimiter=comma", "a,b\n1,2"));
    assert_eq!(
        comma["content"][0]["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // A csv fence does not default to a tab: one column.
    let (one, _) = ok(&fenced("csv", "a\tb\n1\t2"));
    assert_eq!(
        one["content"][0]["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn cell_text_is_never_markdown() {
    let (doc, _) = ok(&fenced("csv", "h\n**bold** and _it_"));
    let text = doc["content"][0]["content"][1]["content"][0]["content"][0]["content"][0].clone();
    assert_eq!(text["text"], "**bold** and _it_");
    assert!(text.get("marks").is_none());
}

#[test]
fn bad_options_are_located_at_the_info_string() {
    for (info, needle) in [
        ("csv colour=red", "colour"),
        ("csv header", "header"),
        ("csv header=true header=true", "twice"),
        ("csv delimiter=ab", "delimiter"),
        ("csv max-rows=0", "max-rows"),
        ("csv align=left", "align"),
        ("tsv =x", "unknown option"),
    ] {
        let d = err(&format!("Intro\n\n{}", fenced(info, "a,b")));
        assert_eq!(d.code, Code::CsvOption, "{info}");
        assert!(d.message.contains(needle), "{info}: {}", d.message);
        let location = d.location.expect("a location");
        assert_eq!(location.line, Some(3), "{info}");
        assert_eq!(location.column, Some(4), "{info}");
    }
}

#[test]
fn csv_errors_are_located_at_markdown_lines() {
    // Fence on line 3; the CSV starts on line 4.
    let d = err(&format!("Intro\n\n{}", fenced("csv", "a,b\n1,\"two")));
    assert_eq!(d.code, Code::InputCsv);
    assert_eq!(line_of(&d), Some(5));
    // An empty body, and one with only blank lines, point at the fence.
    for body in ["", "\n\n"] {
        let d = err(&format!("Intro\n\n{}", fenced("csv", body)));
        assert_eq!(d.code, Code::InputCsv, "{body:?}");
        assert_eq!(line_of(&d), Some(3), "{body:?}");
    }
}

#[test]
fn warnings_found_before_an_error_follow_it_with_markdown_lines() {
    // Fence on line 3: the control character is on line 5, the open quote on line 6.
    let markdown = format!("Intro\n\n{}", fenced("csv", "a\n\u{1}b\n\"x"));
    let diagnostics = convert(&markdown, &MdToAdfOptions::default())
        .unwrap_err()
        .0;
    assert_eq!(diagnostics[0].code, Code::InputCsv);
    assert_eq!(line_of(&diagnostics[0]), Some(6));
    assert_eq!(diagnostics[1].code, Code::ControlChar);
    assert_eq!(line_of(&diagnostics[1]), Some(5));
}

#[test]
fn line_numbers_do_not_depend_on_line_endings_or_a_bom() {
    let lf = format!("Intro\n\n{}", fenced("csv max-rows=1", "a\n1\n2\n3"));
    let crlf = lf.replace('\n', "\r\n");
    let bom = format!("\u{feff}{crlf}");
    let reference = ok(&lf);
    assert_eq!(line_of(&reference.1[0]), Some(6));
    for variant in [&crlf, &bom] {
        let (doc, warnings) = ok(variant);
        assert_eq!(doc, reference.0);
        assert_eq!(warnings, reference.1);
    }
    let broken = format!("Intro\n\n{}", fenced("csv", "a\n\"x"));
    let a = err(&broken);
    let b = err(&broken.replace('\n', "\r\n"));
    assert_eq!((a.code, line_of(&a)), (b.code, line_of(&b)));
    assert_eq!(line_of(&a), Some(5));
}

#[test]
fn warnings_of_the_csv_are_kept_with_markdown_lines() {
    let (_, warnings) = ok(&format!(
        "{}\n{}",
        "Intro",
        fenced("csv max-rows=1", "a\n1\n2"),
    ));
    let truncated: Vec<_> = warnings
        .iter()
        .filter(|w| w.code == Code::CsvTruncated)
        .collect();
    assert_eq!(truncated.len(), 1);
    assert_eq!(line_of(truncated[0]), Some(5));
}

#[test]
fn a_disabled_extension_leaves_a_code_block() {
    let opts = MdToAdfOptions {
        extensions: Extensions::parse("alerts").unwrap(),
        ..MdToAdfOptions::default()
    };
    let (doc, warnings) = convert(&fenced("csv delimiter=bogus", "a,b"), &opts).unwrap();
    assert_eq!(doc["content"][0]["type"], "codeBlock");
    assert_eq!(doc["content"][0]["attrs"]["language"], "csv");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, Code::ExtensionDisabled);
}

#[test]
fn only_the_exact_lowercase_word_makes_a_table() {
    for info in [
        "CSV", "Csv", "csvx", "xcsv", "csv-data", "text csv", "tsv2", "",
    ] {
        let (doc, warnings) = ok(&fenced(info, "a,b\n1,2"));
        assert_eq!(doc["content"][0]["type"], "codeBlock", "{info:?}");
        assert!(warnings.is_empty(), "{info:?}: {warnings:?}");
    }
}

#[test]
fn a_fence_in_a_list_item_is_hoisted_like_a_gfm_table() {
    let (fence, fence_warnings) = ok("- item\n\n  ```csv\n  a,b\n  1,2\n  ```\n");
    let (gfm, gfm_warnings) = ok("- item\n\n  | a | b |\n  |---|---|\n  | 1 | 2 |\n");
    let shape = |doc: &Value| -> Vec<String> {
        doc["content"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["type"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(shape(&fence), shape(&gfm));
    assert_eq!(fence_warnings.len(), 1);
    assert_eq!(fence_warnings[0].code, Code::BlockHoisted);
    assert_eq!(fence_warnings[0].code, gfm_warnings[0].code);
}

/// A GFM table of `rows` one-cell rows (the header included).
fn gfm_table(rows: usize) -> String {
    let mut out = String::from("| a |\n|---|\n");
    out.push_str(&"| x |\n".repeat(rows.saturating_sub(1)));
    out
}

fn csv_rows(rows: usize) -> String {
    format!("```csv\n{}```\n", "x\n".repeat(rows))
}

#[test]
fn csv_fences_share_the_table_cell_limit_with_gfm_tables() {
    // Each alone is within the 100,000 cells.
    ok(&gfm_table(60_000));
    ok(&csv_rows(60_000));
    // Together they are not, whichever comes first, and two fences count together too.
    for markdown in [
        format!("{}\n{}", gfm_table(60_000), csv_rows(60_000)),
        format!("{}\n{}", csv_rows(60_000), gfm_table(60_000)),
        format!("{}\n{}", csv_rows(60_000), csv_rows(60_000)),
    ] {
        let d = err(&markdown);
        assert_eq!(d.code, Code::InputTooLarge);
    }
    // 100,000 cells exactly is accepted, one more is not.
    ok(&csv_rows(100_000));
    let d = err(&format!("{}\n| a |\n|---|\n", csv_rows(100_000)));
    assert_eq!(d.code, Code::InputTooLarge);
}

#[test]
fn a_fence_inside_deep_containers_still_converts() {
    let (doc, warnings) = ok("> > ```csv\n> > a,b\n> > 1,2\n> > ```\n");
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["table"]);
    assert!(warnings.iter().all(|w| w.code != Code::Internal));
}

#[test]
fn the_round_trip_writes_a_gfm_table() {
    let markdown = "```csv\nItem,Qty\nWidget,3\n```\n";
    let (doc, _) = ok(markdown);
    let back = adf_to_md(&doc, &AdfToMdOptions::default())
        .expect("ADF to Markdown")
        .output;
    assert!(back.contains("| Item"), "{back}");
    assert!(!back.contains("```csv"), "{back}");
    let (again, _) = ok(&back);
    assert_eq!(again, doc);
}
