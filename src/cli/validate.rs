//! `adf validate`: check an ADF document or envelope, or dry-run a Markdown conversion.

use serde::Serialize;

use super::Outcome;
use super::args::{ValidateArgs, ValidateFrom};
use super::convert::{
    markdown_options, markdown_pipeline, parse_and_unwrap, parse_envelope, prefix_pointers,
    read_input, resolve_target, strict_failure,
};
use super::report::DiagnosticJson;
use crate::adf::envelope::Envelope;
use crate::catalog::ADF_SCHEMA;
use crate::diagnostics::{Code, Diagnostic};
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
}

/// What was checked.
struct Checked {
    from: &'static str,
    target: Target,
    envelope: Option<&'static str>,
    label: Option<String>,
    diagnostics: Vec<Diagnostic>,
}

/// Runs `adf validate`.
pub(crate) fn run(args: &ValidateArgs) -> Outcome {
    let checked = match check(args) {
        Ok(checked) => checked,
        Err(outcome) => return *outcome,
    };
    let exit = checked
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.is_error())
        .map_or(0, |error| error.code.exit_code());
    let errors = checked.diagnostics.iter().filter(|d| d.is_error()).count();
    let warnings = checked.diagnostics.len() - errors;

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

fn check(args: &ValidateArgs) -> Result<Checked, Box<Outcome>> {
    let input = match read_input(args.input.as_deref(), "validate") {
        Ok(input) => input,
        // Usage errors stay on stderr; with --json, input errors become a report.
        Err(outcome) if args.json && outcome.exit != Code::Usage.exit_code() => {
            let from = match args.from {
                ValidateFrom::Md => "md",
                ValidateFrom::Auto | ValidateFrom::Adf => "adf",
            };
            return Ok(Checked {
                from,
                target: args.target.explicit().unwrap_or_default(),
                envelope: None,
                label: outcome.label.clone(),
                diagnostics: outcome.diagnostics,
            });
        }
        Err(outcome) => return Err(outcome),
    };
    let from_adf = match args.from {
        ValidateFrom::Adf => true,
        ValidateFrom::Md => false,
        ValidateFrom::Auto => input.text.trim_start().starts_with('{'),
    };
    let label = Some(input.label.clone());

    if from_adf {
        if args.envelope.envelope.is_some() {
            return Err(Box::new(Outcome::usage(
                "-e/--envelope cannot be used when validating ADF; the envelope is detected from the input",
                "Drop -e, or use --from md to validate a Markdown conversion",
            )));
        }
        let unwrapped = match parse_and_unwrap(&input.text, args.envelope.field.as_deref()) {
            Ok(unwrapped) => unwrapped,
            Err(diagnostic) => {
                return Ok(Checked {
                    from: "adf",
                    target: args.target.explicit().unwrap_or_default(),
                    envelope: None,
                    label,
                    diagnostics: vec![diagnostic],
                });
            }
        };
        let target = resolve_target(args.target, &unwrapped.envelope);
        let mut diagnostics = prefix_pointers(
            crate::validate_adf(&unwrapped.doc, target),
            &unwrapped.pointer,
        );
        apply_strict(args.strict, &mut diagnostics, "the document has warnings");
        return Ok(Checked {
            from: "adf",
            target,
            envelope: Some(unwrapped.envelope.name()),
            label,
            diagnostics,
        });
    }

    let envelope: Envelope =
        parse_envelope(args.envelope.envelope, args.envelope.field.as_deref())?;
    let target = resolve_target(args.target, &envelope);
    let opts = markdown_options(&args.markdown, target);
    let mut diagnostics = match markdown_pipeline(&input.text, &opts) {
        Ok(converted) => converted.warnings,
        Err(diagnostics) => diagnostics,
    };
    apply_strict(args.strict, &mut diagnostics, "the conversion is lossy");
    Ok(Checked {
        from: "md",
        target,
        envelope: Some(envelope.name()),
        label,
        diagnostics,
    })
}

/// Under `--strict`, warnings without errors fail with `E_LOSSY`.
fn apply_strict(strict: bool, diagnostics: &mut Vec<Diagnostic>, what: &str) {
    if strict && !diagnostics.is_empty() && !diagnostics.iter().any(Diagnostic::is_error) {
        *diagnostics = strict_failure(std::mem::take(diagnostics), what);
    }
}

/// The one-line summary printed after the diagnostics in text mode.
fn summary(checked: &Checked, errors: usize, warnings: usize) -> String {
    let label = checked.label.as_deref().unwrap_or("-");
    let what = match checked.from {
        "md" => "Markdown conversion",
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
        "lossy (--strict)"
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
