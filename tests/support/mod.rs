//! Helpers shared by the integration tests (`mod support;` in each test crate).
//!
//! - [`assert_valid_adf`]: the vendored JSON schema (draft-04, checked directly with `jsonschema`,
//!   independent of the crate's validator), plus `adf_converter::validate_adf`, plus the
//!   independent invariant checks in [`invariants`].
//! - Fixture discovery and loading (`tests/fixtures/**/<case>.md`, optional `<case>.opts.json`);
//!   see `tests/fixtures/README.md`.
//! - [`normalize`] for round-trip comparisons, exactly as in `docs/syntax.md`.
//! - [`to_pretty_json`]: the stable JSON format of blessed files.

// Each test crate uses a different subset of these helpers.
#![allow(dead_code)]
// Test helpers report failures by panicking, like the `#[test]` functions that call them
// (`clippy.toml` only exempts the test functions themselves).
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

pub mod invariants;
pub mod normalize;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use adf_converter::{
    AdfToMdOptions, Diagnostic, Extensions, HtmlPolicy, ImagePolicy, MdFlavor, MdToAdfOptions,
    SoftBreak, Target,
};
use serde::Deserialize;
use serde_json::{Map, Value};

pub use invariants::invariant_errors;
pub use normalize::normalize;

/// The minimum number of fixtures; guards against a discovery bug silently testing nothing.
pub const MIN_FIXTURES: usize = 120;

/// Absolute path of a file in the repository.
pub fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// The fixture root, `tests/fixtures`.
pub fn fixtures_dir() -> PathBuf {
    repo_path("tests/fixtures")
}

// ---------------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------------

fn schema_validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let path = repo_path("schemas/adf/full.json");
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let schema: Value = serde_json::from_str(&text).expect("schemas/adf/full.json is JSON");
        jsonschema::draft4::new(&schema).expect("schemas/adf/full.json is a valid draft-04 schema")
    })
}

/// Errors from the vendored ADF JSON schema (draft-04), each as `pointer: message`.
pub fn schema_errors(doc: &Value) -> Vec<String> {
    schema_validator()
        .iter_errors(doc)
        .map(|e| format!("{}: {e}", pointer_or_root(e.instance_path().as_str())))
        .collect()
}

/// Errors (not warnings) from the crate's own validator for `target`.
pub fn crate_validation_errors(doc: &Value, target: Target) -> Vec<String> {
    adf_converter::validate_adf(doc, target)
        .into_iter()
        .filter(Diagnostic::is_error)
        .map(|d| {
            let pointer = d
                .location
                .as_ref()
                .and_then(|l| l.pointer.clone())
                .unwrap_or_default();
            format!("{}: {} {}", pointer_or_root(&pointer), d.code, d.message)
        })
        .collect()
}

/// Every problem found by the three layers, labelled by layer. Empty means valid.
pub fn adf_problems(doc: &Value, target: Target) -> Vec<String> {
    let mut problems: Vec<String> = schema_errors(doc)
        .into_iter()
        .map(|e| format!("[schema] {e}"))
        .collect();
    problems.extend(
        crate_validation_errors(doc, target)
            .into_iter()
            .map(|e| format!("[validate_adf] {e}")),
    );
    problems.extend(
        invariant_errors(doc)
            .into_iter()
            .map(|e| format!("[invariant] {e}")),
    );
    problems
}

/// Panics unless `doc` is valid ADF for Jira (see [`adf_problems`]).
#[track_caller]
pub fn assert_valid_adf(doc: &Value) {
    assert_valid_adf_for(doc, Target::Jira);
}

/// Panics unless `doc` is valid ADF for `target` (see [`adf_problems`]).
#[track_caller]
pub fn assert_valid_adf_for(doc: &Value, target: Target) {
    let problems = adf_problems(doc, target);
    assert!(
        problems.is_empty(),
        "invalid ADF ({} problems):\n  {}\n\ndocument:\n{}",
        problems.len(),
        problems.join("\n  "),
        excerpt(&to_pretty_json(doc), 200),
    );
}

fn pointer_or_root(pointer: &str) -> &str {
    if pointer.is_empty() { "/" } else { pointer }
}

// ---------------------------------------------------------------------------------------------
// JSON formatting
// ---------------------------------------------------------------------------------------------

/// Stable pretty JSON for blessed files: 2-space indent, keys in document order, trailing newline.
pub fn to_pretty_json(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("JSON values always serialize");
    text.push('\n');
    text
}

/// Compact JSON, as the CLI writes it to a pipe.
pub fn to_compact_json(value: &Value) -> String {
    serde_json::to_string(value).expect("JSON values always serialize")
}

/// The first `max_lines` lines of `text`, with a marker when truncated.
pub fn excerpt(text: &str, max_lines: usize) -> String {
    let mut out: String = text.lines().take(max_lines).collect::<Vec<_>>().join("\n");
    let total = text.lines().count();
    if total > max_lines {
        let _ = write!(out, "\n… ({} more lines)", total - max_lines);
    }
    out
}

/// A short report of where two texts first differ, with a few lines of context.
pub fn first_difference(expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    let index = expected_lines
        .iter()
        .zip(&actual_lines)
        .position(|(e, a)| e != a)
        .unwrap_or(expected_lines.len().min(actual_lines.len()));
    let from = index.saturating_sub(3);
    let context = |lines: &[&str]| {
        lines
            .iter()
            .enumerate()
            .skip(from)
            .take(8)
            .map(|(i, l)| format!("{:>5} | {l}", i + 1))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "first difference at line {} (expected {} lines, got {}):\n--- expected\n{}\n+++ actual\n{}",
        index + 1,
        expected_lines.len(),
        actual_lines.len(),
        context(&expected_lines),
        context(&actual_lines),
    )
}

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// One `tests/fixtures/<area>/<case>.md` input.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// `<area>/<case>`, e.g. `gfm/table-alignment`.
    pub name: String,
    /// Path of the `.md` file.
    pub path: PathBuf,
    /// The Markdown, exactly as stored (it may contain a BOM or CRLF line endings).
    pub source: String,
    /// Options from `<case>.opts.json`, or the defaults.
    pub options: MdToAdfOptions,
}

impl Fixture {
    /// A sibling file: `<case><suffix>`, e.g. `.adf.json`.
    pub fn sibling(&self, suffix: &str) -> PathBuf {
        sibling(&self.path, suffix)
    }
}

/// `<dir>/<case><suffix>` for `<dir>/<case>.md`.
pub fn sibling(md_path: &Path, suffix: &str) -> PathBuf {
    let stem = md_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    md_path.with_file_name(format!("{stem}{suffix}"))
}

/// Documentation files (`README.md`, `SOURCES.md`, …) have an all-uppercase stem and are not
/// fixtures.
pub fn is_documentation(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| {
            stem.chars().any(|c| c.is_ascii_uppercase())
                && stem
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '-')
        })
}

/// Every file under `dir`, recursively, sorted by path.
pub fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read directory {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Paths of all fixture inputs, sorted.
pub fn fixture_paths() -> Vec<PathBuf> {
    files_under(&fixtures_dir())
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "md") && !is_documentation(p))
        .collect()
}

/// Loads one fixture: the Markdown and its options.
pub fn load_fixture(path: &Path) -> Result<Fixture, String> {
    let name = path
        .strip_prefix(fixtures_dir())
        .map_err(|_| format!("{} is outside tests/fixtures", path.display()))?
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");
    let bytes = fs::read(path).map_err(|e| format!("{name}: cannot read: {e}"))?;
    let source = String::from_utf8(bytes).map_err(|e| format!("{name}: not UTF-8: {e}"))?;
    let opts_path = sibling(path, ".opts.json");
    let options = if opts_path.exists() {
        let text = fs::read_to_string(&opts_path)
            .map_err(|e| format!("{name}: cannot read .opts.json: {e}"))?;
        parse_options(&text).map_err(|e| format!("{name}.opts.json: {e}"))?
    } else {
        MdToAdfOptions::default()
    };
    Ok(Fixture {
        name,
        path: path.to_path_buf(),
        source,
        options,
    })
}

/// All fixtures. Panics (listing every problem) if any fixture cannot be loaded.
pub fn fixtures() -> Vec<Fixture> {
    let mut fixtures = Vec::new();
    let mut errors = Vec::new();
    for path in fixture_paths() {
        match load_fixture(&path) {
            Ok(fixture) => fixtures.push(fixture),
            Err(e) => errors.push(e),
        }
    }
    assert!(
        errors.is_empty(),
        "cannot load fixtures:\n  {}",
        errors.join("\n  ")
    );
    assert!(
        fixtures.len() >= MIN_FIXTURES,
        "found only {} fixtures under tests/fixtures (expected at least {MIN_FIXTURES})",
        fixtures.len()
    );
    fixtures
}

/// The `<case>.opts.json` file. Every field is optional and mirrors an `MdToAdfOptions` field
/// (and the matching CLI flag). Unknown fields are rejected so typos do not go unnoticed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionsFile {
    target: Option<String>,
    softbreak: Option<String>,
    html: Option<String>,
    images: Option<String>,
    extensions: Option<ExtensionsSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ExtensionsSpec {
    /// `"all"`, `"none"` or `"alerts,details"`, as for `--extensions`.
    Spec(String),
    /// `["alerts", "details"]`.
    List(Vec<String>),
}

/// Parses the options file format documented in `tests/fixtures/README.md`.
pub fn parse_options(json: &str) -> Result<MdToAdfOptions, String> {
    let file: OptionsFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let mut options = MdToAdfOptions::default();
    if let Some(target) = file.target {
        options.target = match target.as_str() {
            "jira" => Target::Jira,
            "confluence" => Target::Confluence,
            other => return Err(format!("target: expected jira|confluence, got {other:?}")),
        };
    }
    if let Some(softbreak) = file.softbreak {
        options.softbreak = match softbreak.as_str() {
            "space" => SoftBreak::Space,
            "hardbreak" => SoftBreak::HardBreak,
            other => {
                return Err(format!(
                    "softbreak: expected space|hardbreak, got {other:?}"
                ));
            }
        };
    }
    if let Some(html) = file.html {
        options.html = match html.as_str() {
            "text" => HtmlPolicy::Text,
            "drop" => HtmlPolicy::Drop,
            "error" => HtmlPolicy::Error,
            other => return Err(format!("html: expected text|drop|error, got {other:?}")),
        };
    }
    if let Some(images) = file.images {
        options.images = match images.as_str() {
            "link" => ImagePolicy::Link,
            "external" => ImagePolicy::External,
            "drop" => ImagePolicy::Drop,
            other => {
                return Err(format!(
                    "images: expected link|external|drop, got {other:?}"
                ));
            }
        };
    }
    if let Some(spec) = file.extensions {
        let spec = match spec {
            ExtensionsSpec::Spec(spec) => spec,
            ExtensionsSpec::List(names) if names.is_empty() => "none".to_owned(),
            ExtensionsSpec::List(names) => names.join(","),
        };
        options.extensions = Extensions::parse(&spec).map_err(|e| format!("extensions: {e}"))?;
    }
    Ok(options)
}

// ---------------------------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------------------------

/// Markdown → ADF JSON plus warnings; errors are rendered as text.
pub fn md_to_value(
    markdown: &str,
    options: &MdToAdfOptions,
) -> Result<(Value, Vec<Diagnostic>), String> {
    let conversion = adf_converter::md_to_adf(markdown, options)
        .map_err(|d| format!("md_to_adf failed:\n  {}", render_diagnostics(&d.0)))?;
    let value = conversion
        .output
        .to_value()
        .map_err(|e| format!("Doc::to_value failed: {e}"))?;
    Ok((value, conversion.warnings))
}

/// ADF → Markdown in the given flavour; errors are rendered as text.
pub fn adf_to_markdown(doc: &Value, flavor: MdFlavor) -> Result<(String, Vec<Diagnostic>), String> {
    let options = AdfToMdOptions {
        flavor,
        ..AdfToMdOptions::default()
    };
    adf_converter::adf_to_md(doc, &options)
        .map(|c| (c.output, c.warnings))
        .map_err(|d| format!("adf_to_md failed:\n  {}", render_diagnostics(&d.0)))
}

/// The options roundtrip Markdown is read back with: `options`' target and policies, but every
/// ADF extension (the roundtrip syntax is written with them: `> [!TIP]`, `::: cell`, raw
/// ```` ```adf ```` blocks…) and `--images external` (`![alt](url)` is an external image). ADF →
/// Markdown cannot know which options produced `x`; a panel spliced in from raw ADF with
/// `--extensions raw-adf` is still written as an alert.
pub fn read_back_options(options: &MdToAdfOptions) -> MdToAdfOptions {
    MdToAdfOptions {
        extensions: Extensions::all(),
        images: ImagePolicy::External,
        ..options.clone()
    }
}

/// Checks the round-trip guarantee for a document produced by Markdown → ADF:
/// `normalize(md_to_adf(adf_to_md(x))) == normalize(x)` (read back with
/// [`read_back_options`]), and that the Markdown is a fixed point (`adf_to_md` of the result gives
/// the same Markdown). Returns a failure report.
pub fn check_round_trip(x: &Value, options: &MdToAdfOptions) -> Result<(), String> {
    let (markdown, _) = adf_to_markdown(x, MdFlavor::Roundtrip)?;
    let (y, _) = md_to_value(&markdown, &read_back_options(options)).map_err(|e| {
        format!(
            "{e}\n\nMarkdown from adf_to_md:\n{}",
            excerpt(&markdown, 80)
        )
    })?;
    let expected = to_pretty_json(&normalize(x));
    let actual = to_pretty_json(&normalize(&y));
    if expected != actual {
        return Err(format!(
            "md_to_adf(adf_to_md(x)) != normalize(x)\n{}\n\nMarkdown from adf_to_md:\n{}",
            first_difference(&expected, &actual),
            excerpt(&markdown, 80),
        ));
    }
    let (again, _) = adf_to_markdown(&y, MdFlavor::Roundtrip)?;
    if again != markdown {
        return Err(format!(
            "Markdown is not a fixed point: adf_to_md(md_to_adf(md)) != md\n{}",
            first_difference(&markdown, &again),
        ));
    }
    Ok(())
}

/// One line per diagnostic: `CODE line:column message`.
pub fn render_diagnostics(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| {
            let location = d.location.as_ref().map_or_else(String::new, |l| {
                match (l.line, l.column, &l.pointer, l.byte_offset) {
                    (Some(line), Some(column), _, _) => format!(" {line}:{column}"),
                    (Some(line), None, _, _) => format!(" {line}"),
                    (None, _, Some(pointer), _) => format!(" {pointer}"),
                    (None, _, None, Some(offset)) => format!(" byte {offset}"),
                    (None, _, None, None) => String::new(),
                }
            });
            format!("{}{location} {}", d.code, d.message)
        })
        .collect::<Vec<_>>()
        .join("\n  ")
}

/// The golden form of a warning: `{"code":…,"line":…,"column":…}` (`line`/`column` omitted when
/// the warning has no source position; `pointer` included when it has one instead).
pub fn warning_record(diagnostic: &Diagnostic) -> Value {
    let mut record = Map::new();
    record.insert(
        "code".to_owned(),
        Value::String(diagnostic.code.as_str().to_owned()),
    );
    if let Some(location) = &diagnostic.location {
        if let Some(line) = location.line {
            record.insert("line".to_owned(), Value::from(line));
            if let Some(column) = location.column {
                record.insert("column".to_owned(), Value::from(column));
            }
        } else if let Some(pointer) = &location.pointer {
            record.insert("pointer".to_owned(), Value::String(pointer.clone()));
        }
    }
    Value::Object(record)
}

/// The `.diag.jsonl` text for a list of warnings: one compact JSON object per line.
pub fn render_diag_jsonl(warnings: &[Diagnostic]) -> String {
    let mut out = String::new();
    for warning in warnings {
        out.push_str(&to_compact_json(&warning_record(warning)));
        out.push('\n');
    }
    out
}

/// `(code, line, column)` of each warning, for comparisons that ignore messages.
pub fn warning_positions(warnings: &[Diagnostic]) -> Vec<(String, Option<u32>, Option<u32>)> {
    warnings
        .iter()
        .map(|w| {
            let location = w.location.as_ref();
            (
                w.code.as_str().to_owned(),
                location.and_then(|l| l.line),
                location.and_then(|l| l.column),
            )
        })
        .collect()
}

/// Every node of `doc` with the given `type`, in document order.
pub fn nodes_of_type<'a>(doc: &'a Value, node_type: &str) -> Vec<&'a Value> {
    let mut found = Vec::new();
    let mut stack = vec![doc];
    while let Some(node) = stack.pop() {
        if node.get("type").and_then(Value::as_str) == Some(node_type) {
            found.push(node);
        }
        if let Some(Value::Array(children)) = node.get("content") {
            stack.extend(children.iter().rev());
        }
    }
    found
}
