//! CLI contract tests (`docs/cli-contract.md`).
//!
//! - `tests/cli/*.toml`: trycmd cases that only need validation, envelopes and the CLI itself.
//! - `tests/cli/md/*.toml`: cases that need Markdown → ADF; `tests/cli/to-md/*.toml`: cases that
//!   need ADF → Markdown.
//! - Plain tests below for what trycmd cannot express: atomic `-o`, a closed stdout, inputs of
//!   several MiB, environment variables, and that JSON outputs match `schemas/*.v1.json`.
//!
//! Bless the trycmd expectations with `TRYCMD=overwrite cargo test --locked --test cli`.

// Test helpers outside `#[test]` functions: failing loudly is the point.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_adf"))
}

fn cases(glob: &str) {
    trycmd::TestCases::new()
        .register_bin("adf", bin())
        // Neutralise the developer's environment; `auto` is JSON because stderr is a pipe.
        .env("ADF_ERROR_FORMAT", "auto")
        .case(glob);
}

#[test]
fn cli_cases() {
    cases("tests/cli/*.toml");
}

#[test]
fn cli_markdown_to_adf_cases() {
    cases("tests/cli/md/*.toml");
}

#[test]
fn cli_adf_to_markdown_cases() {
    cases("tests/cli/to-md/*.toml");
}

/// Runs `adf` with `args`, `stdin` and a clean environment.
fn adf(args: &[&str], stdin: &[u8], dir: Option<&Path>) -> Output {
    let mut command = Command::new(bin());
    command
        .args(args)
        .env_remove("ADF_ERROR_FORMAT")
        .env_remove("NO_COLOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().expect("spawn adf");
    let mut child_stdin = child.stdin.take().expect("stdin");
    let input = stdin.to_vec();
    // Write from a thread: adf may stop reading early (size limit) and close the pipe.
    let writer = std::thread::spawn(move || {
        let _ = child_stdin.write_all(&input);
    });
    let output = child.wait_with_output().expect("wait for adf");
    writer.join().expect("stdin writer");
    output
}

fn stderr_lines(output: &Output) -> Vec<Value> {
    String::from_utf8(output.stderr.clone())
        .expect("stderr is UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| panic!("not JSON: {line}")))
        .collect()
}

fn codes(output: &Output) -> Vec<String> {
    stderr_lines(output)
        .iter()
        .map(|line| line["code"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("adf-cli-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

const VALID_DOC: &str = r#"{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"x"}]}]}"#;

#[test]
fn failed_conversion_creates_no_output_file() {
    let dir = scratch("no-output");
    fs::write(dir.join("in.json"), r#"{"hello":"world"}"#).unwrap();
    let output = adf(
        &["convert", "--from", "adf", "in.json", "-o", "out.md"],
        b"",
        Some(&dir),
    );
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(codes(&output), ["E_INPUT_NOT_ADF"]);
    assert_eq!(
        entries(&dir),
        ["in.json"],
        "no output or temporary file is left"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn failed_conversion_keeps_the_existing_output_file() {
    let dir = scratch("keep-output");
    fs::write(dir.join("out.md"), "previous\n").unwrap();
    let output = adf(
        &["convert", "--from", "adf", "-", "-o", "out.md"],
        b"{not json",
        Some(&dir),
    );
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(codes(&output), ["E_INPUT_JSON"]);
    assert_eq!(
        fs::read_to_string(dir.join("out.md")).unwrap(),
        "previous\n"
    );
    assert_eq!(entries(&dir), ["out.md"]);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn strict_failure_creates_no_output_file() {
    let dir = scratch("strict-output");
    let output = adf(
        &["convert", "--strict", "-o", "out.json"],
        b"![diagram](https://example.com/d.png)\n",
        Some(&dir),
    );
    assert_eq!(output.status.code(), Some(5));
    assert!(codes(&output).contains(&"E_LOSSY".to_owned()));
    assert_eq!(entries(&dir), Vec::<String>::new());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn successful_conversion_writes_the_file_atomically() {
    let dir = scratch("write-output");
    fs::write(dir.join("out.md"), "previous\n").unwrap();
    let output = adf(
        &["convert", "--from", "adf", "-o", "out.md"],
        VALID_DOC.as_bytes(),
        Some(&dir),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"");
    assert_eq!(fs::read_to_string(dir.join("out.md")).unwrap(), "x\n");
    assert_eq!(entries(&dir), ["out.md"], "no temporary file is left");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn output_to_a_directory_is_an_output_error() {
    let dir = scratch("output-dir");
    fs::create_dir(dir.join("sub")).unwrap();
    let output = adf(
        &["convert", "--from", "adf", "-o", "sub"],
        VALID_DOC.as_bytes(),
        Some(&dir),
    );
    assert_eq!(output.status.code(), Some(3));
    let lines = stderr_lines(&output);
    assert_eq!(lines[0]["code"], "E_OUTPUT_WRITE");
    assert_eq!(lines[0]["location"]["file"], "sub");
    assert_eq!(entries(&dir), ["sub"]);
    fs::remove_dir_all(&dir).unwrap();
}

/// Special files cannot be replaced by a rename; `-o` writes them in place.
#[cfg(unix)]
#[test]
fn output_to_special_files() {
    let output = adf(
        &["convert", "--from", "adf", "-o", "/dev/null"],
        VALID_DOC.as_bytes(),
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
    let output = adf(
        &["convert", "--from", "adf", "-o", "/dev/stdout"],
        VALID_DOC.as_bytes(),
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"x\n");
}

#[test]
fn closed_stdout_exits_quietly() {
    // The ADF schema is larger than a pipe buffer, so adf is still writing when the pipe closes.
    let mut child = Command::new(bin())
        .args(["schema", "adf"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn adf");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut first = [0u8; 16];
    stdout.read_exact(&mut first).expect("read the start");
    drop(stdout);
    let output = child.wait_with_output().expect("wait for adf");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn input_over_the_size_limit() {
    let big = vec![b'a'; 4 * 1024 * 1024 + 1];
    let output = adf(&["validate", "--from", "md"], &big, None);
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(codes(&output), ["E_INPUT_TOO_LARGE"]);
    let exact = vec![b'['; 4 * 1024 * 1024];
    let output = adf(&["validate", "--from", "adf"], &exact, None);
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        codes(&output),
        ["E_INPUT_TOO_DEEP"],
        "4 MiB is accepted, then parsed"
    );
}

#[test]
fn size_limit_warning() {
    let text = "a".repeat(40_000);
    let doc = json!({"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":text}]}]});
    let input = serde_json::to_vec(&doc).unwrap();
    let output = adf(&["validate"], &input, None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(codes(&output), ["W_SIZE_LIMIT"]);
    let output = adf(&["validate", "-t", "confluence"], &input, None);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(codes(&output), Vec::<String>::new());
    let output = adf(&["validate", "--strict"], &input, None);
    assert_eq!(output.status.code(), Some(5));
}

/// `W_SIZE_LIMIT` and `W_EMPTY_DOCUMENT` come from the self-check of the converted output, which
/// the golden harness does not run, so their `lossy/` fixtures are converted here with the CLI.
#[test]
fn markdown_fixtures_warn_from_the_self_check() {
    let cases = [
        ("tests/fixtures/lossy/size-limit.md", "W_SIZE_LIMIT"),
        ("tests/fixtures/lossy/empty-document.md", "W_EMPTY_DOCUMENT"),
        (
            "tests/fixtures/lossy/comment-only-document.md",
            "W_EMPTY_DOCUMENT",
        ),
    ];
    for (path, code) in cases {
        let output = adf(&["convert", "--from", "md", path], b"", None);
        assert_eq!(output.status.code(), Some(0), "{path}");
        assert_eq!(codes(&output), [code], "{path}");
        assert!(!output.stdout.is_empty(), "{path}: the document is written");
        let output = adf(&["convert", "--from", "md", "--strict", path], b"", None);
        assert_eq!(output.status.code(), Some(5), "{path} --strict");
        assert_eq!(codes(&output), [code, "E_LOSSY"], "{path} --strict");
        assert!(
            output.stdout.is_empty(),
            "{path} --strict: nothing is written"
        );
    }
    // The size limit is Jira's; Confluence has no such warning.
    let path = "tests/fixtures/lossy/size-limit.md";
    let output = adf(
        &["convert", "--from", "md", "-t", "confluence", path],
        b"",
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(codes(&output), Vec::<String>::new());
}

#[test]
fn error_format_from_the_environment() {
    let mut command = Command::new(bin());
    let output = command
        .args(["convert", "--bogus"])
        .env("ADF_ERROR_FORMAT", "text")
        .output()
        .expect("run adf");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("error[E_USAGE]: unexpected argument '--bogus'"),
        "{stderr}"
    );
    // The flag wins over the environment.
    let output = Command::new(bin())
        .args(["--error-format", "json", "convert", "--bogus"])
        .env("ADF_ERROR_FORMAT", "text")
        .output()
        .expect("run adf");
    assert!(output.stderr.starts_with(b"{\"schema_version\":1,"));
}

/// Runs `adf` with `ADF_ERROR_FORMAT=value` on an empty document (one `W_EMPTY_DOCUMENT`).
fn with_error_format_env(value: &str, args: &[&str]) -> Output {
    let mut child = Command::new(bin())
        .args(args)
        .env("ADF_ERROR_FORMAT", value)
        .env_remove("NO_COLOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn adf");
    // adf may exit without reading stdin (`--help`, usage errors); ignore a closed pipe.
    let _ = child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"{"type":"doc","version":1,"content":[]}"#);
    child.wait_with_output().expect("wait for adf")
}

#[test]
fn error_format_environment_values() {
    let json_warning = |output: &Output| {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(codes(output), ["W_EMPTY_DOCUMENT"]);
    };
    let text_warning = |output: &Output| {
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let stderr = String::from_utf8(output.stderr.clone()).unwrap();
        assert!(
            stderr.starts_with("-#/content: warning[W_EMPTY_DOCUMENT]"),
            "{stderr}"
        );
    };
    // Empty is unset: `auto`, which is JSON because stderr is a pipe.
    json_warning(&with_error_format_env("", &["validate"]));
    json_warning(&with_error_format_env("  ", &["validate"]));
    // Case and surrounding spaces are ignored.
    text_warning(&with_error_format_env(" TEXT ", &["validate"]));
    json_warning(&with_error_format_env("Json", &["validate"]));
    // An invalid value names the variable, not the flag the user never typed.
    let output = with_error_format_env("yaml", &["validate"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let lines = stderr_lines(&output);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["code"], "E_USAGE");
    let message = lines[0]["message"].as_str().unwrap();
    assert!(
        message.contains("ADF_ERROR_FORMAT") && message.contains("\"yaml\""),
        "{message}"
    );
    assert!(!message.contains("--error-format"), "{message}");
    // The flag overrides the variable, before or after the command.
    text_warning(&with_error_format_env(
        "yaml",
        &["--error-format", "text", "validate"],
    ));
    json_warning(&with_error_format_env(
        "",
        &["validate", "--error-format", "json"],
    ));
    // Help still works.
    let output = with_error_format_env("yaml", &["--help"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

/// The `validate` summary line starts with the file name; control characters in it must not reach
/// the terminal (the diagnostic lines were already sanitised).
#[cfg(unix)]
#[test]
fn validate_summary_line_is_sanitized() {
    let dir = scratch("summary-escape");
    let name = "x\u{1b}[2Jy\nz.json";
    fs::write(dir.join(name), r#"{"type":"doc","version":1,"content":[]}"#).unwrap();
    let output = adf(
        &["--error-format", "text", "validate", name],
        b"",
        Some(&dir),
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains('\u{1b}'), "{stderr:?}");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 2, "{stderr:?}");
    assert!(
        lines[1].starts_with("x [2Jy z.json: valid ADF for jira"),
        "{stderr:?}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn no_color_is_plain_text() {
    // Colour is only used on a terminal; with a pipe the text is plain either way.
    let output = Command::new(bin())
        .args(["--error-format", "text", "explain", "nope"])
        .env("NO_COLOR", "1")
        .output()
        .expect("run adf");
    assert!(!output.stderr.contains(&0x1b));
}

#[test]
fn json_outputs_match_their_schemas() {
    let schema = |name: &str| {
        let text = fs::read_to_string(format!("schemas/{name}.v1.json")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        jsonschema::validator_for(&value).unwrap()
    };
    let check = |validator: &jsonschema::Validator, value: &Value| {
        let errors: Vec<String> = validator
            .iter_errors(value)
            .map(|e| format!("{} at {}", e, e.instance_path()))
            .collect();
        assert!(errors.is_empty(), "{errors:#?}\n{value}");
    };

    let capabilities = adf(&["capabilities", "--json"], b"", None);
    assert_eq!(capabilities.status.code(), Some(0));
    check(
        &schema("capabilities"),
        &serde_json::from_slice(&capabilities.stdout).unwrap(),
    );

    let report = schema("validate-report");
    for (input, code) in [
        (VALID_DOC.to_owned(), 0),
        (r#"{"type":"doc","version":1,"content":[]}"#.to_owned(), 0),
        (
            r#"{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":""}]}]}"#
                .to_owned(),
            4,
        ),
        (r#"{"fields":{"description":null}}"#.to_owned(), 3),
        ("{not json".to_owned(), 3),
        // More than 100 errors: the report ends with a summary of the unlisted ones.
        (
            fs::read_to_string("tests/cli/validate-many-errors.stdin").unwrap(),
            4,
        ),
    ] {
        let output = adf(&["validate", "--json"], input.as_bytes(), None);
        assert_eq!(output.status.code(), Some(code), "{input}");
        check(&report, &serde_json::from_slice(&output.stdout).unwrap());
    }

    let diagnostic = schema("diagnostic");
    for (args, input) in [
        (vec!["convert", "--bogus"], ""),
        (vec![], ""),
        (vec!["validate"], "{\"a\":"),
        (vec!["validate", "--from", "md"], "\u{0}"),
        (
            vec!["validate", "--strict"],
            r#"{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"","marks":[]}]},{"type":"bogus"}]}"#,
        ),
        (
            vec!["validate", "--strict"],
            r#"{"type":"doc","version":1,"content":[]}"#,
        ),
        (vec!["validate", "missing-file.json"], ""),
    ] {
        let output = adf(&args, input.as_bytes(), None);
        let lines = stderr_lines(&output);
        assert!(!lines.is_empty(), "{args:?}");
        for line in &lines {
            check(&diagnostic, line);
        }
    }
    let bad_utf8 = adf(&["validate", "--from", "md"], b"ok \xff", None);
    for line in stderr_lines(&bad_utf8) {
        check(&diagnostic, &line);
        assert_eq!(line["location"]["byte_offset"], 3);
    }
}

/// Compares `actual` with the snapshot `tests/cli/<name>`; `TRYCMD=overwrite` rewrites it. The
/// version number is replaced by `[VERSION]` so releases do not change the snapshots.
fn snapshot(name: &str, actual: &str) {
    let actual = actual.replace(env!("CARGO_PKG_VERSION"), "[VERSION]");
    let path = Path::new("tests/cli").join(name);
    if std::env::var_os("TRYCMD").is_some_and(|mode| mode == "overwrite") {
        fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {}; bless with TRYCMD=overwrite", path.display()));
    assert!(
        expected == actual,
        "{} differs (bless with TRYCMD=overwrite):\n{actual}",
        path.display()
    );
}

#[test]
fn capabilities_snapshots() {
    let json = adf(&["capabilities", "--json"], b"", None);
    assert_eq!(json.status.code(), Some(0));
    let value: Value = serde_json::from_slice(&json.stdout).unwrap();
    let mut pretty = serde_json::to_string_pretty(&value).unwrap();
    pretty.push('\n');
    snapshot("capabilities.json", &pretty);
    let text = adf(&["capabilities"], b"", None);
    assert_eq!(text.status.code(), Some(0));
    snapshot("capabilities.txt", &String::from_utf8(text.stdout).unwrap());
}

#[test]
fn output_is_deterministic() {
    let input = br#"{"type":"doc","version":1,"content":[{"type":"bogus"},{"type":"paragraph","content":[{"type":"text","text":""}]}]}"#;
    let first = adf(&["validate", "--json"], input, None);
    for _ in 0..3 {
        assert_eq!(
            adf(&["validate", "--json"], input, None).stdout,
            first.stdout
        );
    }
    let caps = adf(&["capabilities", "--json"], b"", None);
    assert_eq!(
        adf(&["capabilities", "--json"], b"", None).stdout,
        caps.stdout
    );
}
