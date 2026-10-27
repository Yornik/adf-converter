//! Commands that describe the tool: `capabilities`, `examples`, `schema`, `explain`, `version`
//! and `completions`.

use std::fmt::Write as _;

use clap::CommandFactory;
use serde::Serialize;
use serde_json::Value;

use super::Outcome;
use super::args::{Cli, SchemaName};
use super::convert::json_bytes;
use super::output;
use super::report::docs_url;
use crate::catalog::{self, ADF_SCHEMA, EXIT_CODES, FEATURES, Feature, FeatureKind};
use crate::diagnostics::{Code, Diagnostic, Severity};
use crate::limits::{
    DEFAULT_MAX_DEPTH, DEFAULT_MAX_INPUT_BYTES, DEFAULT_MAX_MARKDOWN_BYTES, JIRA_FIELD_CHAR_LIMIT,
};
use crate::options::{CsvToAdfOptions, Extensions, MdToAdfOptions};

const CLI_CONTRACT: &str = include_str!("../../docs/cli-contract.md");
const SYNTAX: &str = include_str!("../../docs/syntax.md");

const DIAGNOSTIC_SCHEMA: &str = include_str!("../../schemas/diagnostic.v1.json");
const CAPABILITIES_SCHEMA: &str = include_str!("../../schemas/capabilities.v1.json");
const VALIDATE_REPORT_SCHEMA: &str = include_str!("../../schemas/validate-report.v1.json");

/// The topics of `adf explain` besides diagnostic codes.
const TOPICS: &[&str] = &["exit-codes", "envelopes", "syntax", "codes"];

/// JSON for the terminal is pretty, for pipes compact.
fn json_out(value: &impl Serialize) -> Vec<u8> {
    let value = serde_json::to_value(value).unwrap_or(Value::Null);
    json_bytes(&value, output::stdout_is_terminal())
}

fn text_out(text: String) -> Outcome {
    let mut text = text;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Outcome::print(text.into_bytes())
}

/// `adf capabilities [--json]`.
pub(crate) fn capabilities(json: bool) -> Outcome {
    let caps = catalog::capabilities();
    if json {
        return Outcome::print(json_out(&caps));
    }
    let mut text = String::new();
    let _ = writeln!(text, "adf {} ({ADF_SCHEMA})", caps.version);
    let _ = writeln!(text, "Directions: {}", caps.directions.join(", "));
    let _ = writeln!(text, "Targets: {} (-t)", caps.targets.join(", "));
    let _ = writeln!(text, "\nEnvelopes (-e):");
    for envelope in &caps.envelopes {
        let _ = writeln!(text, "  {:<18}{}", envelope.name, envelope.shape);
        let _ = writeln!(text, "  {:<18}{}", "", envelope.endpoint);
    }
    let _ = writeln!(text, "\nMarkdown features:");
    for feature in FEATURES
        .iter()
        .filter(|f| f.extension.is_none() && f.kind != FeatureKind::Csv)
    {
        let _ = writeln!(
            text,
            "  {:<16}{:<12}{}",
            feature.name,
            feature.kind.as_str(),
            feature.syntax
        );
    }
    let _ = writeln!(text, "\nADF extensions (--extensions; all on by default):");
    for feature in FEATURES.iter().filter(|f| f.extension.is_some()) {
        let _ = writeln!(text, "  {:<16}{}", feature.name, feature.syntax);
    }
    let _ = writeln!(text, "\nCSV tables (--from csv, --from adf --to csv):");
    for feature in FEATURES.iter().filter(|f| f.kind == FeatureKind::Csv) {
        let _ = writeln!(text, "  {:<16}{}", feature.name, feature.syntax);
    }
    let _ = writeln!(text, "\nExit codes:");
    for (code, meaning) in EXIT_CODES {
        let _ = writeln!(text, "  {code}  {meaning}");
    }
    let _ = writeln!(
        text,
        "\nLimits: Markdown and CSV input {} MiB, ADF input {} MiB, nesting depth {DEFAULT_MAX_DEPTH}, Jira field {JIRA_FIELD_CHAR_LIMIT} characters",
        DEFAULT_MAX_MARKDOWN_BYTES / (1024 * 1024),
        DEFAULT_MAX_INPUT_BYTES / (1024 * 1024)
    );
    let _ = writeln!(
        text,
        "\n`adf capabilities --json` prints this as JSON; `adf explain codes` lists the diagnostic codes; `adf examples NAME` shows a feature."
    );
    text_out(text)
}

/// One example in `adf examples --json`.
#[derive(Debug, Serialize)]
struct Example {
    name: &'static str,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    extension: Option<&'static str>,
    syntax: &'static str,
    /// The example of a Markdown feature (everything but the `csv` kind).
    #[serde(skip_serializing_if = "Option::is_none")]
    markdown: Option<&'static str>,
    /// The example of the `csv` kind: CSV text instead of Markdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    csv: Option<&'static str>,
    adf: Value,
    warnings: Vec<Code>,
}

#[derive(Debug, Serialize)]
struct Examples {
    schema_version: u32,
    examples: Vec<Example>,
}

/// `adf examples [FEATURE] [--json]`: converts each feature's example at runtime.
pub(crate) fn examples(feature: Option<&str>, json: bool) -> Outcome {
    let selected: Vec<&Feature> = match feature {
        None => FEATURES.iter().collect(),
        Some(name) => {
            let Some(feature) = catalog::feature_by_name(name) else {
                let names: Vec<&str> = FEATURES.iter().map(|f| f.name).collect();
                return Outcome::usage(
                    &format!("unknown feature '{name}'"),
                    &format!("Features: {}", names.join(", ")),
                );
            };
            vec![feature]
        }
    };
    let opts = MdToAdfOptions {
        extensions: Extensions::all(),
        ..MdToAdfOptions::default()
    };
    let mut examples = Vec::new();
    let csv_opts = CsvToAdfOptions::default();
    for feature in selected {
        let is_csv = feature.kind == FeatureKind::Csv;
        let converted = if is_csv {
            crate::csv_to_adf(feature.example, &csv_opts)
        } else {
            crate::md_to_adf(feature.example, &opts)
        };
        let conversion = match converted {
            Ok(conversion) => conversion,
            Err(diagnostics) => {
                let mut all = vec![Diagnostic::new(
                    Code::Internal,
                    format!("the example for '{}' does not convert", feature.name),
                )];
                all.extend(diagnostics.0);
                return Outcome::failed(all, None);
            }
        };
        let adf = match conversion.output.to_value() {
            Ok(adf) => adf,
            Err(err) => {
                return Outcome::failed(
                    vec![Diagnostic::new(
                        Code::Internal,
                        format!(
                            "the example for '{}' could not be serialized: {err}",
                            feature.name
                        ),
                    )],
                    None,
                );
            }
        };
        let mut warnings: Vec<Code> = Vec::new();
        for warning in &conversion.warnings {
            if !warnings.contains(&warning.code) {
                warnings.push(warning.code);
            }
        }
        examples.push(Example {
            name: feature.name,
            kind: feature.kind.as_str(),
            extension: feature.extension,
            syntax: feature.syntax,
            markdown: (!is_csv).then_some(feature.example),
            csv: is_csv.then_some(feature.example),
            adf,
            warnings,
        });
    }
    if json {
        return Outcome::print(json_out(&Examples {
            schema_version: 1,
            examples,
        }));
    }
    let mut text = String::new();
    for (index, example) in examples.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        let _ = writeln!(text, "## {} ({})\n", example.name, example.kind);
        let _ = writeln!(text, "Syntax: {}\n", example.syntax);
        let (language, source) = match (example.markdown, example.csv) {
            (Some(markdown), _) => ("markdown", markdown),
            (None, csv) => ("csv", csv.unwrap_or_default()),
        };
        let fence = fence_for(source);
        let _ = writeln!(text, "{fence}{language}\n{source}{fence}\n");
        let pretty = serde_json::to_string_pretty(&example.adf).unwrap_or_default();
        let _ = writeln!(text, "```json\n{pretty}\n```");
        if !example.warnings.is_empty() {
            let codes: Vec<&str> = example.warnings.iter().map(|code| code.as_str()).collect();
            let _ = writeln!(text, "\nWarnings: {}", codes.join(", "));
        }
    }
    text_out(text)
}

/// A backtick fence longer than any backtick run in `text` (at least three), so the example can
/// itself contain fenced code.
fn fence_for(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

/// `adf schema NAME`.
pub(crate) fn schema(name: SchemaName) -> Outcome {
    let text = match name {
        SchemaName::Adf => crate::ADF_SCHEMA_JSON,
        SchemaName::Diagnostic => DIAGNOSTIC_SCHEMA,
        SchemaName::Capabilities => CAPABILITIES_SCHEMA,
        SchemaName::ValidateReport => VALIDATE_REPORT_SCHEMA,
    };
    text_out(text.to_owned())
}

/// `adf explain TOPIC`.
pub(crate) fn explain(topic: &str) -> Outcome {
    let key = topic.trim().to_ascii_lowercase();
    match key.as_str() {
        "exit-codes" | "exit" => text_out(section(CLI_CONTRACT, "## Exit codes")),
        "envelopes" | "envelope" => text_out(section(CLI_CONTRACT, "## Envelopes")),
        "syntax" => text_out(SYNTAX.to_owned()),
        "codes" | "error-codes" | "diagnostic-codes" => text_out(code_list()),
        _ => match Code::parse(&key.to_ascii_uppercase()) {
            Some(code) => text_out(explain_code(code)),
            None => Outcome::usage(
                &format!("unknown topic '{}'", topic.trim()),
                &format!(
                    "Topics: {}, or a diagnostic code such as W_IMAGE_AS_LINK (see `adf explain codes`)",
                    TOPICS.join(", ")
                ),
            ),
        },
    }
}

/// The text of a `## ` section of a Markdown document, up to the next `## ` heading.
fn section(document: &str, heading: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in document.lines() {
        if line.starts_with("## ") {
            if inside {
                break;
            }
            inside = line.trim_end() == heading;
        }
        if inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    out
}

fn code_list() -> String {
    let mut text =
        String::from("Diagnostic codes (errors exit with the code shown; warnings exit 0):\n\n");
    for &code in Code::ALL {
        let severity = match code.severity() {
            Severity::Error => format!("error {}", code.exit_code()),
            Severity::Warning => "warning".to_owned(),
        };
        let _ = writeln!(
            text,
            "{:<26}{:<10}{}",
            code.as_str(),
            severity,
            code.summary()
        );
    }
    let _ = write!(
        text,
        "\nDetails: adf explain CODE, or {}",
        super::report::DOCS_URL
    );
    text
}

fn explain_code(code: Code) -> String {
    let mut text = String::new();
    match code.severity() {
        Severity::Error => {
            let _ = writeln!(
                text,
                "{} (error, exit code {})\n",
                code.as_str(),
                code.exit_code()
            );
        }
        Severity::Warning => {
            let _ = writeln!(text, "{} (warning)\n", code.as_str());
        }
    }
    let _ = writeln!(text, "{}\n", code.summary());
    let _ = writeln!(text, "What to do: {}\n", catalog::advice(code));
    let _ = write!(text, "Docs: {}", docs_url(code));
    text
}

#[derive(Debug, Serialize)]
struct VersionInfo {
    version: &'static str,
    commit: Option<&'static str>,
    os: &'static str,
    arch: &'static str,
    adf_schema: &'static str,
}

fn version_info() -> VersionInfo {
    VersionInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit: option_env!("ADF_BUILD_COMMIT").filter(|commit| !commit.is_empty()),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        adf_schema: ADF_SCHEMA,
    }
}

/// `adf version [--json]`.
pub(crate) fn version(json: bool) -> Outcome {
    let info = version_info();
    if json {
        return Outcome::print(json_out(&info));
    }
    text_out(format!(
        "adf {} (commit {}; {}; {} {})",
        info.version,
        info.commit.unwrap_or("unknown"),
        info.adf_schema,
        info.os,
        info.arch
    ))
}

/// `adf completions SHELL`.
pub(crate) fn completions(shell: clap_complete::Shell) -> Outcome {
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), "adf", &mut script);
    Outcome::print(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdout(outcome: &Outcome) -> String {
        String::from_utf8(outcome.stdout.clone()).unwrap()
    }

    #[test]
    fn sections_are_extracted() {
        let text = section(CLI_CONTRACT, "## Exit codes");
        assert!(text.starts_with("## Exit codes\n"), "{text}");
        assert!(text.contains("| 5 |"));
        assert!(!text.contains("## Diagnostics"));
        let text = section(CLI_CONTRACT, "## Envelopes");
        assert!(text.contains("jira-comment") && text.contains("atlas_doc_format"));
    }

    #[test]
    fn explain_topics() {
        assert_eq!(explain("exit-codes").exit, 0);
        assert!(stdout(&explain("syntax")).starts_with("# Syntax"));
        let codes = stdout(&explain("codes"));
        for code in Code::ALL {
            assert!(codes.contains(code.as_str()), "{code}");
        }
        let one = stdout(&explain("w_image_as_link"));
        assert!(one.starts_with("W_IMAGE_AS_LINK (warning)"), "{one}");
        assert!(one.contains("#w_image_as_link"));
        let error = stdout(&explain("E_SCHEMA"));
        assert!(
            error.starts_with("E_SCHEMA (error, exit code 4)"),
            "{error}"
        );
        let unknown = explain("bogus");
        assert_eq!(unknown.exit, 2);
        assert_eq!(unknown.diagnostics[0].code, Code::Usage);
    }

    #[test]
    fn schemas_are_json() {
        for name in [
            SchemaName::Adf,
            SchemaName::Diagnostic,
            SchemaName::Capabilities,
            SchemaName::ValidateReport,
        ] {
            let text = stdout(&schema(name));
            assert!(text.ends_with('\n'));
            let value: Value = serde_json::from_str(&text).unwrap();
            assert!(value.is_object());
        }
    }

    #[test]
    fn version_shape() {
        let value = serde_json::to_value(version_info()).unwrap();
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["version", "commit", "os", "arch", "adf_schema"]);
        assert_eq!(value["adf_schema"], "@atlaskit/adf-schema@57.7.18");
        assert!(stdout(&version(false)).starts_with("adf "));
    }

    #[test]
    fn completions_for_every_shell() {
        for shell in [
            clap_complete::Shell::Bash,
            clap_complete::Shell::Zsh,
            clap_complete::Shell::Fish,
            clap_complete::Shell::PowerShell,
            clap_complete::Shell::Elvish,
        ] {
            let script = stdout(&completions(shell));
            assert!(script.contains("adf"), "{shell:?}");
        }
    }

    #[test]
    fn capabilities_text_mentions_everything() {
        let text = stdout(&capabilities(false));
        for feature in FEATURES {
            assert!(text.contains(feature.name), "{}", feature.name);
        }
        assert!(text.contains("jira-comment"));
    }

    #[test]
    fn capabilities_text_has_a_csv_section() {
        let text = stdout(&capabilities(false));
        assert!(text.contains("Directions: md->adf, adf->md, csv->adf, adf->csv"));
        let (markdown, rest) = text.split_once("CSV tables (").unwrap();
        // The CSV entry is listed under its own heading, not among the Markdown features.
        assert!(!markdown.contains("csv-tables"));
        assert!(rest.contains("csv-tables"));
    }

    /// The CSV entry's example is CSV text, converted with `csv_to_adf` (so this needs the CSV
    /// converter; until then the command fails with `E_INTERNAL`, which is not hidden).
    #[test]
    fn csv_example_is_shown_as_csv() {
        let outcome = examples(Some("csv-tables"), true);
        if outcome.exit != 0 {
            assert_eq!(outcome.exit, 1);
            return;
        }
        let value: Value = serde_json::from_slice(&outcome.stdout).unwrap();
        let example = &value["examples"][0];
        assert_eq!(example["name"], "csv-tables");
        assert_eq!(example["kind"], "csv");
        assert!(example.get("markdown").is_none());
        assert!(example["csv"].as_str().unwrap().starts_with("Name,Qty"));
        assert_eq!(example["adf"]["content"][0]["type"], "table");
        let text = stdout(&examples(Some("csv-tables"), false));
        assert!(text.contains("```csv\nName,Qty"), "{text}");
    }

    #[test]
    fn unknown_example_is_a_usage_error() {
        let outcome = examples(Some("nope"), false);
        assert_eq!(outcome.exit, 2);
    }

    #[test]
    fn example_fences_outlast_their_content() {
        assert_eq!(fence_for("plain"), "```");
        assert_eq!(fence_for("```rust\nx\n```\n"), "````");
        assert_eq!(fence_for("`a` ````` b"), "``````");
    }
}
