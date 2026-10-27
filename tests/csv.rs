//! CSV tests (issue #13, phase 1).
//!
//! - Golden tests. Every `tests/csv/<case>.csv` is converted to ADF with the options in
//!   `<case>.opts.json` and compared with the blessed `<case>.adf.json` (and `<case>.diag.jsonl`
//!   for warnings). Every `tests/csv/to-csv/<case>.adf.json` is converted to CSV with its options
//!   and compared with the blessed `<case>.csv` (and `<case>.diag.jsonl`).
//! - Every produced ADF is valid (schema, `validate_adf` for both targets, independent
//!   invariants) and every conversion is deterministic (CRLF, CR and BOM variants of the input
//!   give the same result).
//! - The native reader agrees with the Agent Skill's Python helper (`csv2md.py`).
//! - The limits hold exactly, and fast.
//!
//! Bless (write the expected files instead of comparing) with:
//!
//! ```text
//! ADF_BLESS=1 cargo test --locked --test csv
//! ```
//!
//! Blessed files are generated, never edited by hand. Inputs and `*.opts.json` are hand-written.

// Test helpers report failures by panicking.
#![allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::print_stderr
)]

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use adf_converter::{
    AdfToCsvOptions, Code, CsvDelimiter, CsvToAdfOptions, Diagnostic, Target, adf_to_csv,
    csv_to_adf, md_to_adf,
};
use serde::Deserialize;
use serde_json::Value;

const BLESS_HINT: &str = "run `ADF_BLESS=1 cargo test --locked --test csv` and review the diff";

/// The fewest fixtures: guards against a discovery bug silently testing nothing.
const MIN_FIXTURES: usize = 30;

fn blessing() -> bool {
    std::env::var("ADF_BLESS").is_ok_and(|v| v == "1")
}

fn csv_dir() -> PathBuf {
    support::repo_path("tests/csv")
}

fn to_csv_dir() -> PathBuf {
    csv_dir().join("to-csv")
}

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// `<case>.opts.json` of a CSV → ADF fixture. Unknown fields are rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToAdfOptionsFile {
    target: Option<String>,
    delimiter: Option<String>,
    header: Option<bool>,
    max_rows: Option<usize>,
    align_numbers: Option<bool>,
}

/// `<case>.opts.json` of an ADF → CSV fixture. Unknown fields are rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToCsvOptionsFile {
    delimiter: Option<String>,
    table: Option<usize>,
    csv_safe: Option<bool>,
}

fn parse_delimiter(value: &str) -> CsvDelimiter {
    CsvDelimiter::parse(value).unwrap_or_else(|e| panic!("delimiter {value:?}: {e}"))
}

fn parse_to_adf_options(json: &str) -> CsvToAdfOptions {
    let file: ToAdfOptionsFile = serde_json::from_str(json).expect("valid .opts.json");
    let mut options = CsvToAdfOptions::default();
    if let Some(target) = file.target {
        options.target = match target.as_str() {
            "jira" => Target::Jira,
            "confluence" => Target::Confluence,
            other => panic!("target: expected jira|confluence, got {other:?}"),
        };
    }
    if let Some(delimiter) = file.delimiter {
        options.delimiter = parse_delimiter(&delimiter);
    }
    if let Some(header) = file.header {
        options.header = header;
    }
    options.max_rows = file.max_rows;
    options.align_numbers = file.align_numbers.unwrap_or(false);
    options
}

fn parse_to_csv_options(json: &str) -> AdfToCsvOptions {
    let file: ToCsvOptionsFile = serde_json::from_str(json).expect("valid .opts.json");
    let mut options = AdfToCsvOptions::default();
    if let Some(delimiter) = file.delimiter {
        options.delimiter = parse_delimiter(&delimiter);
    }
    if let Some(table) = file.table {
        options.table = table;
    }
    options.csv_safe = file.csv_safe.unwrap_or(false);
    options
}

/// A file next to `path` with the same stem (`name.csv` → `name<suffix>`; `name.adf.json` →
/// `name<suffix>`).
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let stem = name.split('.').next().unwrap_or_default();
    path.with_file_name(format!("{stem}{suffix}"))
}

fn read_options(path: &Path) -> Option<String> {
    fs::read_to_string(sibling(path, ".opts.json")).ok()
}

#[derive(Debug)]
struct ToAdfCase {
    name: String,
    path: PathBuf,
    source: String,
    options: CsvToAdfOptions,
}

#[derive(Debug)]
struct ToCsvCase {
    name: String,
    path: PathBuf,
    doc: Value,
    options: AdfToCsvOptions,
}

fn to_adf_cases() -> Vec<ToAdfCase> {
    let mut cases: Vec<_> = fs::read_dir(csv_dir())
        .expect("tests/csv exists")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "csv"))
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let bytes = fs::read(&path).expect("readable fixture");
            let source = String::from_utf8(bytes).expect("fixtures are UTF-8");
            let options = read_options(&path)
                .map_or_else(CsvToAdfOptions::default, |j| parse_to_adf_options(&j));
            ToAdfCase {
                name,
                path,
                source,
                options,
            }
        })
        .collect();
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

fn to_csv_cases() -> Vec<ToCsvCase> {
    let mut cases: Vec<_> = fs::read_dir(to_csv_dir())
        .expect("tests/csv/to-csv exists")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|p| p.to_string_lossy().ends_with(".adf.json"))
        .map(|path| {
            let name = sibling(&path, "")
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let text = fs::read_to_string(&path).expect("readable fixture");
            let doc: Value = serde_json::from_str(&text).expect("fixtures are JSON");
            let options = read_options(&path)
                .map_or_else(AdfToCsvOptions::default, |j| parse_to_csv_options(&j));
            ToCsvCase {
                name: format!("to-csv/{name}"),
                path,
                doc,
                options,
            }
        })
        .collect();
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

// ---------------------------------------------------------------------------------------------
// Blessing
// ---------------------------------------------------------------------------------------------

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

fn report(bless: bool, written: &[String], failures: &[String], total: usize) {
    if bless && !written.is_empty() {
        eprintln!(
            "blessed {} file(s):\n  {}",
            written.len(),
            written.join("\n  ")
        );
    }
    assert!(
        failures.is_empty(),
        "{} of {total} fixtures failed:\n\n{}\n",
        failures.len(),
        failures.join("\n\n")
    );
}

fn indent(failure: &str) -> String {
    failure.replace('\n', "\n    ")
}

// ---------------------------------------------------------------------------------------------
// CSV → ADF
// ---------------------------------------------------------------------------------------------

/// The document as JSON, plus the warnings.
fn convert(source: &str, options: &CsvToAdfOptions) -> Result<(Value, Vec<Diagnostic>), String> {
    let conversion = csv_to_adf(source, options).map_err(|d| {
        format!(
            "csv_to_adf failed:\n  {}",
            support::render_diagnostics(&d.0)
        )
    })?;
    let value = conversion
        .output
        .to_value()
        .map_err(|e| format!("Doc::to_value failed: {e}"))?;
    Ok((value, conversion.warnings))
}

/// The input with its line endings changed: `eol` for every line break outside and inside quotes.
fn with_line_endings(source: &str, eol: &str) -> String {
    source
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', eol)
}

/// Variants of an input that must convert exactly like it.
fn equivalent_inputs(source: &str) -> Vec<(&'static str, String)> {
    let bom_free = source.strip_prefix('\u{feff}').unwrap_or(source);
    vec![
        ("CRLF", with_line_endings(source, "\r\n")),
        ("CR", with_line_endings(source, "\r")),
        ("LF", with_line_endings(source, "\n")),
        ("a BOM", format!("\u{feff}{bom_free}")),
        ("no BOM", bom_free.to_owned()),
    ]
}

fn check_to_adf_case(
    case: &ToAdfCase,
    bless: bool,
    written: &mut Vec<String>,
) -> Result<(), String> {
    let (doc, warnings) = convert(&case.source, &case.options)
        .map_err(|e| format!("{e}\n(fixtures must convert successfully)"))?;
    for target in [Target::Jira, Target::Confluence] {
        let problems = support::adf_problems(&doc, target);
        if !problems.is_empty() {
            return Err(format!(
                "output is not valid ADF for {target:?} ({} problems):\n  {}\n{}",
                problems.len(),
                problems.join("\n  "),
                support::excerpt(&support::to_pretty_json(&doc), 60)
            ));
        }
    }
    let again = convert(&case.source, &case.options)?;
    if again != (doc.clone(), warnings.clone()) {
        return Err("two conversions of the same input differ".to_owned());
    }
    for (what, variant) in equivalent_inputs(&case.source) {
        let converted = convert(&variant, &case.options)?;
        if converted.0 != doc {
            return Err(format!("the output differs for the input with {what}"));
        }
        if support::warning_positions(&converted.1) != support::warning_positions(&warnings) {
            return Err(format!("the warnings differ for the input with {what}"));
        }
    }
    let adf = support::to_pretty_json(&doc);
    let diag = support::render_diag_jsonl(&warnings);
    let adf_result = check_file(&sibling(&case.path, ".adf.json"), &adf, bless, written);
    let diag_result = check_file(&sibling(&case.path, ".diag.jsonl"), &diag, bless, written);
    match (adf_result, diag_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(a), Ok(())) => Err(a),
        (Ok(()), Err(d)) => Err(d),
        (Err(a), Err(d)) => Err(format!("{a}\n{d}")),
    }
}

#[test]
fn csv_fixtures_match_blessed_output() {
    let bless = blessing();
    let cases = to_adf_cases();
    let mut failures = Vec::new();
    let mut written = Vec::new();
    for case in &cases {
        if let Err(failure) = check_to_adf_case(case, bless, &mut written) {
            failures.push(format!("{}:\n    {}", case.name, indent(&failure)));
        }
    }
    report(bless, &written, &failures, cases.len());
}

// ---------------------------------------------------------------------------------------------
// ADF → CSV
// ---------------------------------------------------------------------------------------------

fn check_to_csv_case(
    case: &ToCsvCase,
    bless: bool,
    written: &mut Vec<String>,
) -> Result<(), String> {
    let conversion = adf_to_csv(&case.doc, &case.options).map_err(|d| {
        format!(
            "adf_to_csv failed:\n  {}",
            support::render_diagnostics(&d.0)
        )
    })?;
    let again = adf_to_csv(&case.doc, &case.options).map_err(|d| format!("{:?}", d.0))?;
    if again != conversion {
        return Err("two conversions of the same input differ".to_owned());
    }
    let csv = &conversion.output;
    if !csv.ends_with('\n') || csv.starts_with('\u{feff}') || csv.contains('\r') {
        return Err("the CSV must end with a line feed, have no BOM and no CR".to_owned());
    }
    let diag = support::render_diag_jsonl(&conversion.warnings);
    let csv_result = check_file(&sibling(&case.path, ".csv"), csv, bless, written);
    let diag_result = check_file(&sibling(&case.path, ".diag.jsonl"), &diag, bless, written);
    match (csv_result, diag_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(a), Ok(())) => Err(a),
        (Ok(()), Err(d)) => Err(d),
        (Err(a), Err(d)) => Err(format!("{a}\n{d}")),
    }
}

#[test]
fn to_csv_fixtures_match_blessed_output() {
    let bless = blessing();
    let cases = to_csv_cases();
    let mut failures = Vec::new();
    let mut written = Vec::new();
    for case in &cases {
        if let Err(failure) = check_to_csv_case(case, bless, &mut written) {
            failures.push(format!("{}:\n    {}", case.name, indent(&failure)));
        }
    }
    report(bless, &written, &failures, cases.len());
}

// ---------------------------------------------------------------------------------------------
// Fixture hygiene
// ---------------------------------------------------------------------------------------------

#[test]
fn there_are_enough_fixtures() {
    let total = to_adf_cases().len() + to_csv_cases().len();
    assert!(
        total >= MIN_FIXTURES,
        "found only {total} CSV fixtures (expected at least {MIN_FIXTURES})"
    );
}

#[test]
fn every_blessed_or_options_file_has_an_input() {
    let inputs: BTreeSet<String> = to_adf_cases()
        .iter()
        .map(|c| c.name.clone())
        .chain(to_csv_cases().iter().map(|c| c.name.clone()))
        .collect();
    let mut orphans = Vec::new();
    for path in support::files_under(&csv_dir()) {
        let relative = path
            .strip_prefix(csv_dir())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let name = relative.rsplit('/').next().unwrap_or_default();
        if name == ".gitattributes" || name == "README.md" {
            continue;
        }
        let dir = relative.strip_suffix(name).unwrap_or_default();
        let stem = name.split('.').next().unwrap_or_default();
        let key = format!("{dir}{stem}");
        let is_csv = Path::new(name).extension().is_some_and(|e| e == "csv");
        let is_input = is_csv && !dir.starts_with("to-csv")
            || name.ends_with(".adf.json") && dir.starts_with("to-csv");
        let is_companion = name.ends_with(".opts.json")
            || name.ends_with(".diag.jsonl")
            || name.ends_with(".adf.json")
            || (is_csv && dir.starts_with("to-csv"));
        if !(is_input || is_companion) {
            orphans.push(format!("{relative}: not a fixture file"));
        } else if !inputs.contains(&key) {
            orphans.push(format!("{relative}: no input for it"));
        }
    }
    assert!(
        orphans.is_empty(),
        "orphan files:\n  {}",
        orphans.join("\n  ")
    );
}

fn find_case<'a>(cases: &'a [ToAdfCase], name: &str) -> &'a ToAdfCase {
    cases
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no fixture {name}"))
}

fn warning_codes(case: &ToAdfCase) -> Vec<Code> {
    convert(&case.source, &case.options)
        .unwrap()
        .1
        .iter()
        .map(|w| w.code)
        .collect()
}

#[test]
fn fixtures_still_test_what_they_say() {
    let cases = to_adf_cases();
    assert!(find_case(&cases, "bom").source.starts_with('\u{feff}'));
    assert!(find_case(&cases, "crlf").source.contains("\r\n"));
    let lone_cr = &find_case(&cases, "lone-cr").source;
    assert!(lone_cr.contains('\r') && !lone_cr.contains('\n'));
    assert!(find_case(&cases, "control-chars").source.contains('\0'));
    assert!(find_case(&cases, "tab-delimiter").source.contains('\t'));
    assert!(
        find_case(&cases, "embedded-newlines")
            .source
            .contains("\"first line\nsecond")
    );
    assert!(
        !find_case(&cases, "no-trailing-newline")
            .source
            .ends_with('\n')
    );
    assert!(find_case(&cases, "formulas").source.contains("=SUM"));
    assert!(
        find_case(&cases, "whitespace-cells")
            .source
            .contains("  Name  ")
    );
    assert_eq!(
        warning_codes(find_case(&cases, "max-rows")),
        [Code::CsvTruncated]
    );
    assert_eq!(
        warning_codes(find_case(&cases, "max-rows-zero")),
        [Code::CsvTruncated]
    );
    assert_eq!(warning_codes(find_case(&cases, "max-rows-enough")), []);
    assert!(
        warning_codes(find_case(&cases, "control-chars"))
            .iter()
            .all(|code| *code == Code::ControlChar)
    );
    assert_eq!(warning_codes(find_case(&cases, "control-chars")).len(), 6);
    assert_eq!(warning_codes(find_case(&cases, "basic-header")), []);
    let merged = to_csv_cases()
        .into_iter()
        .find(|c| c.name == "to-csv/colspan")
        .unwrap();
    assert_eq!(
        adf_to_csv(&merged.doc, &merged.options)
            .unwrap()
            .warnings
            .iter()
            .map(|w| w.code)
            .collect::<Vec<_>>(),
        [Code::CsvMergedCells]
    );
}

// ---------------------------------------------------------------------------------------------
// Cell text
// ---------------------------------------------------------------------------------------------

/// The text of every cell, row by row; a hard break is `\n`.
fn cell_texts(doc: &Value) -> Vec<Vec<String>> {
    support::nodes_of_type(doc, "tableRow")
        .iter()
        .map(|row| {
            row["content"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| {
                    let mut text = String::new();
                    collect_text(cell, &mut text);
                    text
                })
                .collect()
        })
        .collect()
}

fn collect_text(node: &Value, out: &mut String) {
    match node["type"].as_str() {
        Some("text") => out.push_str(node["text"].as_str().unwrap()),
        Some("hardBreak") => out.push('\n'),
        _ => {
            for child in node["content"].as_array().into_iter().flatten() {
                collect_text(child, out);
            }
        }
    }
}

fn rows(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|c| (*c).to_owned()).collect())
        .collect()
}

#[test]
fn hostile_cells_are_plain_text() {
    let cases = to_adf_cases();
    let case = find_case(&cases, "hostile-cells");
    let (doc, warnings) = convert(&case.source, &case.options).unwrap();
    assert_eq!(warnings.len(), 0);
    let texts = cell_texts(&doc);
    // Every cell equals the CSV text; nothing was read as Markdown.
    for (line, cells) in case.source.lines().zip(&texts) {
        assert_eq!(cells.join(","), line);
    }
    // No node other than table, rows, cells, paragraphs and text was made.
    let mut types: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![&doc];
    while let Some(node) = stack.pop() {
        types.insert(node["type"].as_str().unwrap().to_owned());
        stack.extend(node["content"].as_array().into_iter().flatten());
        assert!(node.get("marks").is_none(), "{node}");
    }
    let expected: BTreeSet<String> = [
        "doc",
        "table",
        "tableRow",
        "tableHeader",
        "tableCell",
        "paragraph",
        "text",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    assert_eq!(types, expected);
}

#[test]
fn only_http_and_https_urls_are_linked() {
    let cases = to_adf_cases();
    let case = find_case(&cases, "urls");
    let (doc, _) = convert(&case.source, &case.options).unwrap();
    let mut links: Vec<&str> = support::nodes_of_type(&doc, "text")
        .iter()
        .filter_map(|text| text["marks"][0]["attrs"]["href"].as_str())
        .collect();
    links.sort_unstable();
    assert_eq!(
        links,
        [
            "HTTPS://EXAMPLE.COM/UP",
            "http://example.com",
            "https://a.example/1",
            "https://b.example/2",
            "https://example.com/a?b=1&c=2",
            "https://example.com/docs",
            "https://example.com/p",
            "https://example.com/x",
            "https://example.org/y",
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// The Agent Skill's Python helper
// ---------------------------------------------------------------------------------------------

/// What `csv2md.py --align-numbers` makes of the sample, as cell text.
fn sample_texts() -> Vec<Vec<String>> {
    rows(&[
        &["Name", "Qty", "Price", "Note"],
        &["Widget | big", "3", "1,250.50", "line one\nline two"],
        &[
            "Gadget",
            "10",
            "9.99",
            "uses *stars*, _under_, `code`, <b>html</b> & C:\\temp\\x",
        ],
        &["Short row", "5", "", ""],
        &["Link", "1", "2", "https://example.com/a?b=1&c=2 [x](y)"],
    ])
}

#[test]
fn the_native_reader_agrees_with_the_python_helper() {
    let csv = fs::read_to_string(support::repo_path("tests/skill/csv2md-sample.csv")).unwrap();
    let markdown = fs::read_to_string(support::repo_path("tests/skill/csv2md-sample.md")).unwrap();
    let options = CsvToAdfOptions {
        align_numbers: true,
        ..CsvToAdfOptions::default()
    };
    let (native, warnings) = convert(&csv, &options).unwrap();
    assert_eq!(warnings.len(), 0);
    assert_eq!(cell_texts(&native), sample_texts());
    // The helper's Markdown table, converted by adf, is the very same document.
    let via_markdown = md_to_adf(&markdown, &adf_converter::MdToAdfOptions::default())
        .unwrap()
        .output
        .to_value()
        .unwrap();
    assert_eq!(cell_texts(&via_markdown), sample_texts());
    assert_eq!(
        support::to_pretty_json(&native),
        support::to_pretty_json(&via_markdown)
    );
}

#[test]
fn the_helper_and_the_native_reader_agree_without_alignment_too() {
    let csv = fs::read_to_string(support::repo_path("tests/skill/csv2md-sample.csv")).unwrap();
    let (native, _) = convert(&csv, &CsvToAdfOptions::default()).unwrap();
    assert_eq!(cell_texts(&native), sample_texts());
    assert!(!support::to_compact_json(&native).contains("alignment"));
}

// ---------------------------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------------------------

/// A bound for "this must not be quadratic": generous for debug builds on slow machines.
const FAST: Duration = Duration::from_secs(20);

fn timed<T>(f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = f();
    let elapsed = start.elapsed();
    assert!(elapsed < FAST, "took {elapsed:?}");
    result
}

fn grid(columns: usize, rows: usize) -> String {
    let line = vec!["x"; columns].join(",");
    let mut out = String::with_capacity((line.len() + 1) * rows);
    for _ in 0..rows {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn first_error(
    result: Result<
        adf_converter::Conversion<adf_converter::adf::model::Doc>,
        adf_converter::Diagnostics,
    >,
) -> Diagnostic {
    match result {
        Err(diagnostics) => diagnostics.0.into_iter().next().unwrap(),
        Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn the_cell_limit_holds_exactly() {
    let options = CsvToAdfOptions::default();
    // 100 x 1000 = 100,000 cells, header row included.
    let ok = timed(|| csv_to_adf(&grid(1000, 100), &options)).unwrap();
    assert_eq!(
        ok.output.to_value().unwrap()["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    let error = first_error(timed(|| csv_to_adf(&grid(1000, 101), &options)));
    assert_eq!(error.code, Code::InputTooLarge);
    assert_eq!(error.location.unwrap().line, Some(101));
    // One row of 100,000 columns, and of 100,001.
    assert!(timed(|| csv_to_adf(&grid(100_000, 1), &options)).is_ok());
    let error = first_error(timed(|| csv_to_adf(&grid(100_001, 1), &options)));
    assert_eq!(error.code, Code::InputTooLarge);
    // Without a header, the generated header row counts.
    let no_header = CsvToAdfOptions {
        header: false,
        ..CsvToAdfOptions::default()
    };
    assert!(timed(|| csv_to_adf(&grid(1000, 99), &no_header)).is_ok());
    assert_eq!(
        first_error(timed(|| csv_to_adf(&grid(1000, 100), &no_header))).code,
        Code::InputTooLarge
    );
}

#[test]
fn the_cell_limit_error_has_the_documented_code() {
    let error = first_error(csv_to_adf(&grid(2, 50_001), &CsvToAdfOptions::default()));
    assert_eq!(error.code, Code::InputTooLarge);
    assert_eq!(error.code.exit_code(), 3);
}

/// `lines` rows of 20 characters (`xxxxxxxxxxxxxxxxxxx` plus a line feed), the last one padded
/// or cut so that the whole input is exactly `bytes` long.
fn input_of_size(bytes: usize) -> String {
    let mut out = String::with_capacity(bytes);
    while out.len() + 20 <= bytes {
        out.push_str("xxxxxxxxxxxxxxxxxxx\n");
    }
    while out.len() < bytes {
        out.push('x');
    }
    assert_eq!(out.len(), bytes);
    out
}

#[test]
fn the_one_mebibyte_limit_holds_exactly() {
    const MIB: usize = 1024 * 1024;
    let options = CsvToAdfOptions::default();
    let result = timed(|| csv_to_adf(&input_of_size(MIB), &options)).unwrap();
    assert_eq!(result.warnings.len(), 0);
    let error = first_error(timed(|| csv_to_adf(&input_of_size(MIB + 1), &options)));
    assert_eq!(error.code, Code::InputTooLarge);
    assert!(error.message.contains("1048576"));
}

#[test]
fn a_huge_single_cell_is_fine() {
    let cell = "x".repeat(1024 * 1024 - 20);
    let csv = format!("h\n\"{cell}\"\n");
    let converted = timed(|| csv_to_adf(&csv, &CsvToAdfOptions::default())).unwrap();
    assert_eq!(
        converted.output.to_value().unwrap()["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn an_unterminated_quote_in_a_large_input_fails_fast() {
    let csv = format!("a,b\n\"{}", "y,\"\"\n".repeat(150_000));
    let error = first_error(timed(|| csv_to_adf(&csv, &CsvToAdfOptions::default())));
    assert_eq!(error.code, Code::InputCsv);
    let at = error.location.unwrap();
    assert_eq!((at.line, at.column), (Some(2), Some(1)));
}

#[test]
fn many_control_character_warnings_are_fast_and_located() {
    // 100,000 cells on one line, each with a control character.
    let line = vec!["\u{1}x"; 100_000].join(",");
    let converted = timed(|| csv_to_adf(&line, &CsvToAdfOptions::default())).unwrap();
    assert_eq!(converted.warnings.len(), 100_000);
    let last = converted
        .warnings
        .last()
        .unwrap()
        .location
        .as_ref()
        .unwrap();
    assert_eq!((last.line, last.column), (Some(1), Some(3 * 99_999 + 1)));
}

#[test]
fn many_links_in_one_cell_are_fast() {
    let cell = "https://example.com/a. ".repeat(40_000);
    let csv = format!("h\n{cell}\n");
    let converted = timed(|| csv_to_adf(&csv, &CsvToAdfOptions::default())).unwrap();
    let doc = converted.output.to_value().unwrap();
    assert_eq!(support::nodes_of_type(&doc, "text").len(), 80_001);
}

#[test]
fn max_rows_stops_building_but_not_counting() {
    let csv = format!("h\n{}", "row\n".repeat(200_000));
    let options = CsvToAdfOptions {
        max_rows: Some(3),
        ..CsvToAdfOptions::default()
    };
    let converted = timed(|| csv_to_adf(&csv, &options)).unwrap();
    assert_eq!(converted.warnings.len(), 1);
    assert_eq!(converted.warnings[0].message, "Kept 3 of 200000 data rows");
    assert_eq!(
        converted.output.to_value().unwrap()["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn the_grid_limit_of_adf_to_csv_holds_exactly() {
    let table = |columns: u64| {
        serde_json::json!({"type": "doc", "version": 1, "content": [{"type": "table", "content": [
            {"type": "tableRow", "content": [
                {"type": "tableCell", "attrs": {"colspan": columns},
                 "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}
            ]}
        ]}]})
    };
    let ok = timed(|| adf_to_csv(&table(100_000), &AdfToCsvOptions::default())).unwrap();
    assert_eq!(ok.output.matches(',').count(), 99_999);
    assert_eq!(ok.warnings[0].code, Code::CsvMergedCells);
    let error = timed(|| adf_to_csv(&table(100_001), &AdfToCsvOptions::default())).unwrap_err();
    assert_eq!(error.0[0].code, Code::InputTooLarge);
    let error = adf_to_csv(&table(u64::MAX), &AdfToCsvOptions::default()).unwrap_err();
    assert_eq!(error.0[0].code, Code::InputTooLarge);
}
