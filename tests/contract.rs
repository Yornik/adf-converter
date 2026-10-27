//! The machine-readable CLI contract (`docs/cli-contract.md`): the JSON written by the built `adf`
//! binary validates against the JSON Schemas shipped in `schemas/`, using the draft each schema
//! declares in `$schema`:
//!
//! - `adf capabilities --json` → `schemas/capabilities.v1.json`
//! - `adf validate --json` → `schemas/validate-report.v1.json`
//! - JSON diagnostics on stderr (`--error-format json`) → `schemas/diagnostic.v1.json`
//!
//! It also checks that `adf schema NAME` prints exactly those files, and that `capabilities`
//! agrees with the code (diagnostic codes, extensions, limits, exit codes).

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::io::Write as _;
use std::process::{Command, Output, Stdio};

use adf_converter::{ADF_SCHEMA_VERSION, Code, Extensions, Severity, limits};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// Running the binary
// ---------------------------------------------------------------------------------------------

fn adf() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_adf"));
    command.env_remove("ADF_ERROR_FORMAT").env("NO_COLOR", "1");
    command
}

fn run(command: &mut Command, stdin: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the adf binary starts");
    let written = child.stdin.take().expect("stdin is piped").write_all(stdin);
    // A command that does not read stdin may exit before the write; that is not a failure.
    if let Err(e) = written
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        panic!("cannot write stdin: {e}");
    }
    child.wait_with_output().expect("adf finishes")
}

fn describe(output: &Output) -> String {
    format!(
        "exit status {:?}\n--- stdout\n{}\n--- stderr\n{}",
        output.status.code(),
        support::excerpt(&String::from_utf8_lossy(&output.stdout), 40),
        support::excerpt(&String::from_utf8_lossy(&output.stderr), 40),
    )
}

#[track_caller]
fn assert_exit(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", describe(output));
}

#[track_caller]
fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}):\n{}", describe(output)))
}

/// The JSON diagnostic lines on stderr.
#[track_caller]
fn stderr_diagnostics(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|e| {
                panic!(
                    "stderr line is not JSON ({e}): {line:?}\n{}",
                    describe(output)
                )
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------------------------

/// Loads `schemas/<file>`. The `*.v1.json` schemas belong to the CLI; a missing file is a clear
/// failure rather than a skipped test.
fn load_schema(file: &str) -> Value {
    let path = support::repo_path(&format!("schemas/{file}"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{} is missing or unreadable ({e}); the CLI contract requires it (it is printed by \
             `adf schema …`)",
            path.display()
        )
    });
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is not JSON: {e}", path.display()))
}

/// A validator for the draft the schema declares in `$schema`; the schema itself must be valid
/// against that draft's meta-schema.
fn validator(file: &str) -> jsonschema::Validator {
    let schema = load_schema(file);
    let declared = schema
        .get("$schema")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("schemas/{file} does not declare `$schema`"));
    if let Err(e) = jsonschema::meta::validate(&schema) {
        panic!("schemas/{file} is not a valid JSON Schema: {e}");
    }
    let uri = declared.trim_end_matches('#');
    let built = match uri {
        "http://json-schema.org/draft-04/schema" => jsonschema::draft4::new(&schema),
        "http://json-schema.org/draft-06/schema" => jsonschema::draft6::new(&schema),
        "http://json-schema.org/draft-07/schema" => jsonschema::draft7::new(&schema),
        "https://json-schema.org/draft/2019-09/schema" => jsonschema::draft201909::new(&schema),
        "https://json-schema.org/draft/2020-12/schema" => jsonschema::draft202012::new(&schema),
        other => panic!("schemas/{file} declares an unknown draft {other:?}"),
    };
    built.unwrap_or_else(|e| panic!("schemas/{file} does not compile: {e}"))
}

#[track_caller]
fn assert_matches_schema(file: &str, instance: &Value) {
    let validator = validator(file);
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "output does not match schemas/{file}:\n  {}\n\noutput:\n{}",
        errors.join("\n  "),
        support::excerpt(&support::to_pretty_json(instance), 80)
    );
}

#[track_caller]
fn assert_diagnostics_match_schema(diagnostics: &[Value]) {
    for diagnostic in diagnostics {
        assert_matches_schema("diagnostic.v1.json", diagnostic);
        assert_eq!(diagnostic["schema_version"], json!(1), "{diagnostic}");
        let code = diagnostic["code"].as_str().unwrap_or_default();
        let parsed = Code::parse(code).unwrap_or_else(|| panic!("unknown code {code:?}"));
        let severity = if parsed.severity() == Severity::Error {
            "error"
        } else {
            "warning"
        };
        assert_eq!(diagnostic["severity"], json!(severity), "{diagnostic}");
        if parsed.severity() == Severity::Error {
            assert_eq!(
                diagnostic["exit_code"],
                json!(parsed.exit_code()),
                "{diagnostic}"
            );
        }
        let docs = diagnostic["docs"].as_str().unwrap_or_default();
        assert!(
            docs.ends_with(&format!("cli-contract.md#{}", code.to_ascii_lowercase())),
            "docs URL {docs:?} does not point at the code's anchor"
        );
    }
}

fn sample_doc() -> Value {
    json!({"type": "doc", "version": 1, "content": [
        {"type": "paragraph", "content": [{"type": "text", "text": "Hello"}]}
    ]})
}

/// Breaks two rules: code combined with strong, and an empty text node.
fn invalid_doc() -> Value {
    json!({"type": "doc", "version": 1, "content": [
        {"type": "paragraph", "content": [
            {"type": "text", "text": "x", "marks": [{"type": "code"}, {"type": "strong"}]},
            {"type": "text", "text": ""}
        ]}
    ]})
}

// ---------------------------------------------------------------------------------------------
// capabilities
// ---------------------------------------------------------------------------------------------

#[test]
fn capabilities_json_matches_its_schema_and_the_code() {
    let output = run(adf().args(["capabilities", "--json"]), b"");
    assert_exit(&output, 0);
    let caps = stdout_json(&output);
    assert_matches_schema("capabilities.v1.json", &caps);

    assert_eq!(caps["schema_version"], json!(1));
    assert_eq!(caps["version"], json!(env!("CARGO_PKG_VERSION")));
    assert_eq!(caps["adf_schema"], json!(ADF_SCHEMA_VERSION));
    assert_eq!(
        caps["directions"],
        json!(["md->adf", "adf->md", "csv->adf", "adf->csv"])
    );
    assert_eq!(caps["targets"], json!(["jira", "confluence"]));

    let envelopes: BTreeSet<&str> = caps["envelopes"]
        .as_array()
        .expect("envelopes is an array")
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    let expected: BTreeSet<&str> = [
        "doc",
        "jira-comment",
        "jira-description",
        "jira-field",
        "confluence",
    ]
    .into();
    assert_eq!(envelopes, expected);

    let extensions: Vec<&str> = caps["extensions"]
        .as_array()
        .expect("extensions is an array")
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert_eq!(
        extensions,
        Extensions::NAMES,
        "extensions in documentation order"
    );

    let codes: Vec<(&str, &str)> = caps["diagnostic_codes"]
        .as_array()
        .expect("diagnostic_codes is an array")
        .iter()
        .map(|c| {
            (
                c["code"].as_str().unwrap_or_default(),
                c["severity"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    let expected: Vec<(&str, &str)> = Code::ALL
        .iter()
        .map(|c| {
            let severity = if c.severity() == Severity::Error {
                "error"
            } else {
                "warning"
            };
            (c.as_str(), severity)
        })
        .collect();
    assert_eq!(codes, expected, "every code, in documentation order");

    let exit_codes: BTreeSet<&str> = caps["exit_codes"]
        .as_object()
        .expect("exit_codes is an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(exit_codes, ["0", "1", "2", "3", "4", "5"].into());

    assert_eq!(
        caps["limits"]["max_input_bytes"],
        json!(limits::DEFAULT_MAX_INPUT_BYTES)
    );
    assert_eq!(
        caps["limits"]["max_nesting_depth"],
        json!(limits::DEFAULT_MAX_DEPTH)
    );
    assert_eq!(
        caps["limits"]["jira_field_char_limit"],
        json!(limits::JIRA_FIELD_CHAR_LIMIT)
    );
}

// ---------------------------------------------------------------------------------------------
// validate --json
// ---------------------------------------------------------------------------------------------

fn validate_report(args: &[&str], stdin: &Value, exit: i32) -> Value {
    let input = serde_json::to_vec(stdin).expect("serializes");
    let output = run(
        adf().arg("validate").args(args).args(["--json", "-"]),
        &input,
    );
    assert_exit(&output, exit);
    let report = stdout_json(&output);
    assert_matches_schema("validate-report.v1.json", &report);
    assert_eq!(report.get("schema_version"), Some(&json!(1)));
    assert_eq!(report.get("adf_schema"), Some(&json!(ADF_SCHEMA_VERSION)));
    for key in ["errors", "warnings"] {
        let items = report
            .get(key)
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("`{key}` is an array: {report}"));
        assert_diagnostics_match_schema(items);
    }
    report
}

#[test]
fn validate_report_for_a_valid_document() {
    let report = validate_report(&["--from", "adf"], &sample_doc(), 0);
    assert_eq!(report["valid"], json!(true));
    assert_eq!(report["from"], json!("adf"));
    assert_eq!(report["target"], json!("jira"));
    assert_eq!(report["envelope"], json!("doc"));
    assert_eq!(report["errors"], json!([]));
}

#[test]
fn validate_report_for_an_invalid_document() {
    let report = validate_report(&["--from", "adf"], &invalid_doc(), 4);
    assert_eq!(report["valid"], json!(false));
    let errors = report["errors"].as_array().expect("errors");
    assert!(!errors.is_empty(), "{report}");
    for error in errors {
        let pointer = error["location"]["pointer"].as_str();
        assert!(
            pointer.is_some_and(|p| p.starts_with("/content/0")),
            "every error is located with a JSON Pointer: {error}"
        );
        let code = error["code"].as_str().unwrap_or_default();
        assert!(
            ["E_SCHEMA", "E_TARGET", "E_INVARIANT"].contains(&code),
            "validation errors use E_SCHEMA/E_TARGET/E_INVARIANT: {error}"
        );
    }
}

#[test]
fn validate_report_detects_the_envelope() {
    let wrapped = json!({"body": sample_doc()});
    let report = validate_report(&["--from", "adf"], &wrapped, 0);
    assert_eq!(report["envelope"], json!("jira-comment"));
    let confluence = json!({
        "representation": "atlas_doc_format",
        "value": serde_json::to_string(&sample_doc()).expect("serializes"),
    });
    let report = validate_report(&["--from", "adf"], &confluence, 0);
    assert_eq!(report["envelope"], json!("confluence"));
}

#[test]
fn validate_report_for_markdown_lists_warnings() {
    let output = run(
        adf().args(["validate", "--from", "md", "--json", "-"]),
        b"![diagram](https://example.com/d.png)\n",
    );
    assert_exit(&output, 0);
    let report = stdout_json(&output);
    assert_matches_schema("validate-report.v1.json", &report);
    assert_eq!(report["from"], json!("md"));
    assert_eq!(report["valid"], json!(true));
    let warnings = report["warnings"].as_array().expect("warnings");
    assert_diagnostics_match_schema(warnings);
    assert!(
        warnings
            .iter()
            .any(|w| w["code"] == json!("W_IMAGE_AS_LINK")),
        "{report}"
    );
}

// ---------------------------------------------------------------------------------------------
// JSON diagnostics on stderr
// ---------------------------------------------------------------------------------------------

#[track_caller]
fn assert_failure(args: &[&str], stdin: &[u8], exit: i32, code: &str) {
    let output = run(adf().args(args), stdin);
    assert_exit(&output, exit);
    assert!(
        output.stdout.is_empty(),
        "nothing on stdout on failure:\n{}",
        describe(&output)
    );
    let diagnostics = stderr_diagnostics(&output);
    assert_diagnostics_match_schema(&diagnostics);
    let first_error = diagnostics
        .iter()
        .find(|d| d["severity"] == json!("error"))
        .unwrap_or_else(|| panic!("no error diagnostic:\n{}", describe(&output)));
    assert_eq!(first_error["code"], json!(code), "{}", describe(&output));
    assert_eq!(first_error["exit_code"], json!(exit));
}

#[test]
fn failing_conversions_report_json_diagnostics() {
    let json_errors = ["--error-format", "json"];
    let from_adf = [&["convert", "--from", "adf", "-"][..], &json_errors[..]].concat();
    assert_failure(&from_adf, b"not json", 3, "E_INPUT_JSON");
    assert_failure(&from_adf, br#"{"foo": 1}"#, 3, "E_INPUT_NOT_ADF");
    assert_failure(
        &[&["convert", "-"][..], &json_errors[..]].concat(),
        b"\xff\xfe bad utf-8",
        3,
        "E_INPUT_ENCODING",
    );
    assert_failure(
        &[&["convert", "--strict", "-"][..], &json_errors[..]].concat(),
        b"![x](https://example.com/x.png)\n",
        5,
        "E_LOSSY",
    );
    assert_failure(
        &[&["convert", "-"][..], &json_errors[..]].concat(),
        b"```adf\n{\"type\": \"bogus\"}\n```\n",
        3,
        "E_RAW_ADF",
    );
}

#[test]
fn invalid_utf8_is_located_by_byte_offset() {
    let output = run(
        adf().args(["convert", "--error-format", "json", "-"]),
        b"ok\n\xffbad",
    );
    assert_exit(&output, 3);
    let diagnostics = stderr_diagnostics(&output);
    assert_diagnostics_match_schema(&diagnostics);
    assert_eq!(
        diagnostics[0]["location"]["byte_offset"],
        json!(3),
        "{}",
        describe(&output)
    );
}

#[test]
fn usage_errors_report_json_diagnostics() {
    let output = run(adf().env("ADF_ERROR_FORMAT", "json").arg("README.md"), b"");
    assert_exit(&output, 2);
    let diagnostics = stderr_diagnostics(&output);
    assert_diagnostics_match_schema(&diagnostics);
    assert_eq!(diagnostics[0]["code"], json!("E_USAGE"));
}

#[test]
fn warnings_are_json_lines_and_the_output_is_valid_adf() {
    let output = run(
        adf().args(["convert", "--error-format", "json", "-"]),
        b"# Title\n\n![diagram](https://example.com/d.png)\n",
    );
    assert_exit(&output, 0);
    let doc = stdout_json(&output);
    support::assert_valid_adf(&doc);
    let diagnostics = stderr_diagnostics(&output);
    assert_diagnostics_match_schema(&diagnostics);
    let warning = diagnostics
        .iter()
        .find(|d| d["code"] == json!("W_IMAGE_AS_LINK"))
        .unwrap_or_else(|| panic!("no W_IMAGE_AS_LINK:\n{}", describe(&output)));
    assert_eq!(warning["severity"], json!("warning"));
    assert_eq!(warning["location"]["line"], json!(3));
    assert_eq!(warning["location"]["column"], json!(1));
    assert_eq!(warning["location"]["file"], json!("-"));
}

// ---------------------------------------------------------------------------------------------
// schema, version
// ---------------------------------------------------------------------------------------------

#[test]
fn schema_command_prints_the_shipped_schemas() {
    for (name, file) in [
        ("adf", "adf/full.json"),
        ("diagnostic", "diagnostic.v1.json"),
        ("capabilities", "capabilities.v1.json"),
        ("validate-report", "validate-report.v1.json"),
    ] {
        let output = run(adf().args(["schema", name]), b"");
        assert_exit(&output, 0);
        assert_eq!(
            stdout_json(&output),
            load_schema(file),
            "`adf schema {name}` differs from schemas/{file}"
        );
    }
}

#[test]
fn shipped_schemas_are_valid_and_declare_their_draft() {
    for file in [
        "diagnostic.v1.json",
        "capabilities.v1.json",
        "validate-report.v1.json",
    ] {
        let _ = validator(file);
    }
}

#[test]
fn version_json_describes_the_build() {
    let output = run(adf().args(["version", "--json"]), b"");
    assert_exit(&output, 0);
    let version = stdout_json(&output);
    assert_eq!(version["version"], json!(env!("CARGO_PKG_VERSION")));
    assert_eq!(version["adf_schema"], json!(ADF_SCHEMA_VERSION));
    assert!(
        version["os"].is_string() && version["arch"].is_string(),
        "{version}"
    );
    assert!(
        version["commit"].is_string() || version["commit"].is_null(),
        "{version}"
    );
}
