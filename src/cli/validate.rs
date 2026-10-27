//! `adf validate`: check an ADF document or envelope, or dry-run a Markdown conversion.

use serde::Serialize;

use super::Outcome;
use super::args::{ValidateArgs, ValidateFrom};
use super::convert::{
    check_adf_field, check_field_used, check_include_root, check_no_csv_input_flags, csv_options,
    csv_pipeline, markdown_options, markdown_pipeline_reporting, not_markdown_include_root,
    parse_and_unwrap, parse_envelope, prefix_pointers, read_input, resolve_target, strict_failure,
};
use super::report::DiagnosticJson;
use crate::adf::envelope::Envelope;
use crate::catalog::ADF_SCHEMA;
use crate::diagnostics::{Code, Diagnostic, cap_per_code, reported_count};
use crate::limits::MAX_LISTED_PER_CODE;
use crate::md::IncludedFile;
use crate::options::Target;

/// The `validate --json` report (`schemas/validate-report.v1.json`).
#[derive(Debug, Serialize)]
struct Report<'a> {
    schema_version: u32,
    valid: bool,
    from: &'static str,
    target: &'static str,
    envelope: Option<&'static str>,
    adf_schema: &'static str,
    errors: Vec<DiagnosticJson<'a>>,
    warnings: Vec<DiagnosticJson<'a>>,
    /// The files `adf:csv` includes read, in document order.
    includes: &'a [IncludedFile],
}

/// What was checked.
struct Checked {
    from: &'static str,
    target: Target,
    envelope: Option<&'static str>,
    label: Option<String>,
    diagnostics: Vec<Diagnostic>,
    includes: Vec<IncludedFile>,
}

/// Runs `adf validate`.
pub(crate) fn run(args: &ValidateArgs) -> Outcome {
    let mut checked = match check(args) {
        Ok(checked) => checked,
        Err(outcome) => return *outcome,
    };
    // At most 100 of each code are listed (in the report too); the counts include the rest.
    checked.diagnostics = cap_per_code(
        std::mem::take(&mut checked.diagnostics),
        MAX_LISTED_PER_CODE,
    );
    let exit = checked
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.is_error())
        .map_or(0, |error| error.code.exit_code());
    let errors = reported_count(checked.diagnostics.iter().filter(|d| d.is_error()));
    let warnings = reported_count(checked.diagnostics.iter().filter(|d| !d.is_error()));

    if args.json {
        let label = checked.label.as_deref();
        let report = Report {
            schema_version: 1,
            valid: errors == 0,
            from: checked.from,
            target: checked.target.as_str(),
            envelope: checked.envelope,
            adf_schema: ADF_SCHEMA,
            errors: checked
                .diagnostics
                .iter()
                .filter(|d| d.is_error())
                .map(|d| DiagnosticJson::new(d, label))
                .collect(),
            warnings: checked
                .diagnostics
                .iter()
                .filter(|d| !d.is_error())
                .map(|d| DiagnosticJson::new(d, label))
                .collect(),
            includes: &checked.includes,
        };
        let mut stdout = serde_json::to_vec(&report).unwrap_or_default();
        stdout.push(b'\n');
        return Outcome {
            stdout,
            exit,
            ..Outcome::default()
        };
    }

    let summary = summary(&checked, errors, warnings);
    Outcome {
        diagnostics: checked.diagnostics,
        label: checked.label,
        exit,
        summary: Some(summary),
        ..Outcome::default()
    }
}

/// Flags that only apply to CSV input are refused for any other input, before it is read.
/// `-e` names the envelope of a conversion; validating ADF detects the envelope instead.
fn check_no_envelope(args: &ValidateArgs) -> Result<(), Box<Outcome>> {
    if args.envelope.envelope.is_some() {
        return Err(Box::new(Outcome::usage(
            "-e/--envelope cannot be used when validating ADF; the envelope is detected from the input",
            "Drop -e, or use --from md or --from csv to validate a conversion",
        )));
    }
    Ok(())
}

fn check_flags(args: &ValidateArgs) -> Result<(), Box<Outcome>> {
    if args.from == ValidateFrom::Adf {
        check_no_envelope(args)?;
    }
    // Auto-detection (ADF or Markdown) can only be settled once the input is read; a root that
    // cannot be used is refused before that.
    check_include_root(
        args.markdown.include_root.as_deref(),
        !matches!(args.from, ValidateFrom::Adf | ValidateFrom::Csv),
    )?;
    if args.from == ValidateFrom::Csv {
        return Ok(());
    }
    let hint = "Add --from csv to dry-run a CSV conversion";
    if args.csv.delimiter.is_some() {
        return Err(Box::new(Outcome::usage(
            "--delimiter only applies with --from csv",
            hint,
        )));
    }
    check_no_csv_input_flags(&args.csv, hint)
}

fn check(args: &ValidateArgs) -> Result<Checked, Box<Outcome>> {
    check_flags(args)?;
    let input = match read_input(args.input.as_deref(), "validate") {
        Ok(input) => input,
        // Usage errors stay on stderr; with --json, input errors become a report.
        Err(outcome) if args.json && outcome.exit != Code::Usage.exit_code() => {
            let from = match args.from {
                ValidateFrom::Md => "md",
                ValidateFrom::Csv => "csv",
                ValidateFrom::Auto | ValidateFrom::Adf => "adf",
            };
            return Ok(Checked {
                from,
                target: args.target.explicit().unwrap_or_default(),
                envelope: None,
                label: outcome.label.clone(),
                diagnostics: outcome.diagnostics,
                includes: Vec::new(),
            });
        }
        Err(outcome) => return Err(outcome),
    };
    let from_adf = match args.from {
        ValidateFrom::Adf => true,
        ValidateFrom::Md | ValidateFrom::Csv => false,
        ValidateFrom::Auto => input.text.trim_start().starts_with('{'),
    };
    let label = Some(input.label.clone());

    if from_adf {
        check_no_envelope(args)?;
        if args.markdown.include_root.is_some() {
            return Err(Box::new(not_markdown_include_root()));
        }
        let field = args.envelope.field.as_deref();
        check_adf_field(field)?;
        let unwrapped = match parse_and_unwrap(&input.text, field) {
            Ok(unwrapped) => unwrapped,
            Err(diagnostic) => {
                return Ok(Checked {
                    from: "adf",
                    target: args.target.explicit().unwrap_or_default(),
                    envelope: None,
                    label,
                    diagnostics: vec![diagnostic],
                    includes: Vec::new(),
                });
            }
        };
        check_field_used(field, &unwrapped)?;
        let target = resolve_target(args.target, &unwrapped.envelope);
        let mut diagnostics = prefix_pointers(
            crate::validate_adf(&unwrapped.doc, target),
            &unwrapped.pointer,
        );
        apply_strict(
            args.strict,
            &mut diagnostics,
            "--strict: the document has warnings ({codes})",
        );
        return Ok(Checked {
            from: "adf",
            target,
            envelope: Some(unwrapped.envelope.name()),
            label,
            diagnostics,
            includes: Vec::new(),
        });
    }

    let envelope: Envelope =
        parse_envelope(args.envelope.envelope, args.envelope.field.as_deref())?;
    let target = resolve_target(args.target, &envelope);
    let (converted, includes) = if args.from == ValidateFrom::Csv {
        (
            csv_pipeline(&input.text, &csv_options(&args.csv, target)),
            Vec::new(),
        )
    } else {
        markdown_pipeline_reporting(&input.text, &markdown_options(&args.markdown, target))
    };
    let mut diagnostics = match converted {
        Ok(converted) => converted.warnings,
        Err(diagnostics) => diagnostics,
    };
    apply_strict(
        args.strict,
        &mut diagnostics,
        "--strict: the conversion would be lossy ({codes})",
    );
    Ok(Checked {
        from: if args.from == ValidateFrom::Csv {
            "csv"
        } else {
            "md"
        },
        target,
        envelope: Some(envelope.name()),
        label,
        diagnostics,
        includes,
    })
}

/// Under `--strict`, warnings without errors fail with `E_LOSSY`.
fn apply_strict(strict: bool, diagnostics: &mut Vec<Diagnostic>, template: &str) {
    if strict && !diagnostics.is_empty() && !diagnostics.iter().any(Diagnostic::is_error) {
        *diagnostics = strict_failure(std::mem::take(diagnostics), template);
    }
}

/// The one-line summary printed after the diagnostics in text mode.
fn summary(checked: &Checked, errors: usize, warnings: usize) -> String {
    let label = checked.label.as_deref().unwrap_or("-");
    let what = match checked.from {
        "md" => "Markdown conversion",
        "csv" => "CSV conversion",
        _ => "ADF",
    };
    let envelope = checked
        .envelope
        .map(|envelope| format!(", envelope {envelope}"))
        .unwrap_or_default();
    let counts = format!(
        "{} {}, {} {}",
        errors,
        if errors == 1 { "error" } else { "errors" },
        warnings,
        if warnings == 1 { "warning" } else { "warnings" }
    );
    let lossy = checked.diagnostics.iter().any(|d| d.code == Code::Lossy);
    let verdict = if lossy {
        // --strict, or --html error during a Markdown conversion
        "lossy"
    } else if errors == 0 {
        "valid"
    } else {
        "invalid"
    };
    format!(
        "{label}: {verdict} {what} for {}{envelope}: {counts}",
        checked.target.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::{Cli, Command};
    use clap::Parser as _;
    use serde_json::Value;

    fn validate_args(argv: &[&str]) -> ValidateArgs {
        let mut full = vec!["adf", "validate"];
        full.extend_from_slice(argv);
        let Command::Validate(parsed) = Cli::try_parse_from(full).unwrap().command else {
            panic!("expected validate");
        };
        parsed
    }

    #[test]
    fn csv_flags_need_csv_input() {
        let cases: &[(&[&str], &str)] = &[
            (
                &["--delimiter", "tab"],
                "--delimiter only applies with --from csv",
            ),
            (&["--no-header"], "--no-header only applies with --from csv"),
            (
                &["--max-rows", "2"],
                "--max-rows only applies with --from csv",
            ),
            (
                &["--align-numbers"],
                "--align-numbers only applies with --from csv",
            ),
            (
                &["--from", "md", "--no-header"],
                "--no-header only applies with --from csv",
            ),
            (
                &["--from", "adf", "--delimiter", ";"],
                "--delimiter only applies with --from csv",
            ),
        ];
        for (argv, message) in cases {
            let outcome = *check_flags(&validate_args(argv)).unwrap_err();
            assert_eq!(outcome.exit, 2, "{argv:?}");
            assert_eq!(&outcome.diagnostics[0].message, message, "{argv:?}");
        }
        let args = validate_args(&[
            "--from",
            "csv",
            "--delimiter",
            "pipe",
            "--no-header",
            "--max-rows",
            "2",
            "--align-numbers",
        ]);
        assert!(check_flags(&args).is_ok());
        // `--table` and `--csv-safe` belong to `convert --to csv`; validate does not have them.
        assert!(Cli::try_parse_from(["adf", "validate", "--from", "csv", "--table", "1"]).is_err());
    }

    #[test]
    fn auto_never_picks_csv() {
        let dir = std::env::temp_dir().join(format!("adf-validate-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.csv");
        std::fs::write(&path, "a,b\n1,2\n").unwrap();
        let path = path.display().to_string();
        let checked = check(&validate_args(&[&path])).unwrap();
        assert_eq!(checked.from, "md");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn report_says_csv_for_unreadable_csv_input() {
        let args = validate_args(&["--from", "csv", "--json", "does-not-exist.csv"]);
        let outcome = run(&args);
        assert_eq!(outcome.exit, 3);
        let report: Value = serde_json::from_slice(&outcome.stdout).unwrap();
        assert_eq!(report["from"], "csv");
        assert_eq!(report["valid"], false);
        assert_eq!(report["errors"][0]["code"], "E_INPUT_READ");
    }
}
