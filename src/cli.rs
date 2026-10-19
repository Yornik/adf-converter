//! The `adf` command line. Contract: `docs/cli-contract.md`.
//!
//! Every command returns an `Outcome` (the result bytes, the diagnostics, the exit code);
//! nothing is written until the command has finished, and all writing goes through the private
//! `output` module. Usage errors from clap become `E_USAGE` diagnostics (exit 2) so that agents
//! get JSON for them too.

mod args;
mod convert;
mod info;
mod input;
mod output;
mod report;
mod validate;

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

use args::{Cli, Command};
use report::Reporter;

use crate::diagnostics::{Code, Diagnostic, Location, cap_per_code};
use crate::limits::MAX_LISTED_PER_CODE;

/// Stack size of the thread that runs a command. The converters and validators recurse only to
/// bounded depths, but those bounds (64 containers, 512 JSON levels) need more than the 1 MiB main
/// thread stack of Windows; the memory is only committed when used.
const WORKER_STACK_BYTES: usize = 64 * 1024 * 1024;

/// What a command produced. [`finish`] writes it.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    /// The result, for stdout or the output file. Empty means nothing is written.
    pub stdout: Vec<u8>,
    /// `-o PATH`: write [`Outcome::stdout`] there (atomically) instead.
    pub output_file: Option<PathBuf>,
    /// Printed to stderr; warnings are skipped with `-q`.
    pub diagnostics: Vec<Diagnostic>,
    /// The input name for diagnostic locations (`-` for stdin).
    pub label: Option<String>,
    /// The process exit code.
    pub exit: u8,
    /// A line printed after the diagnostics in text mode (`validate`).
    pub summary: Option<String>,
}

impl Outcome {
    /// Success with output.
    pub(crate) fn print(stdout: Vec<u8>) -> Self {
        Self {
            stdout,
            ..Self::default()
        }
    }

    /// Failure: no output; the exit code is that of the first error.
    pub(crate) fn failed(diagnostics: Vec<Diagnostic>, label: Option<String>) -> Self {
        let exit = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.is_error())
            .map_or(Code::Internal.exit_code(), |error| error.code.exit_code());
        Self {
            diagnostics,
            label,
            exit,
            ..Self::default()
        }
    }

    /// A usage error (`E_USAGE`, exit 2).
    pub(crate) fn usage(message: &str, hint: &str) -> Self {
        Self::failed(
            vec![Diagnostic::new(Code::Usage, message).with_hint(hint)],
            None,
        )
    }
}

/// Runs the CLI with `std::env::args_os()` and returns the process exit code.
pub fn run() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let worker_args = args.clone();
    let worker = std::thread::Builder::new()
        .name("adf".to_owned())
        .stack_size(WORKER_STACK_BYTES)
        .spawn(move || run_args(worker_args));
    let code = match worker {
        Ok(handle) => match handle.join() {
            Ok(code) => code,
            // The panic hook has already reported it; let `main` map it to exit 1.
            Err(payload) => std::panic::resume_unwind(payload),
        },
        Err(_) => run_args(args),
    };
    ExitCode::from(code)
}

/// Installs a panic hook that reports panics as an `E_INTERNAL` diagnostic (in the selected
/// format, without the panic message, which could contain input) instead of Rust's default
/// message. `main` catches the unwind and exits with code 1.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // Paths of dependencies are absolute (and contain the builder's home directory); keep
        // only the file name for those.
        let place = info
            .location()
            .map(|location| {
                let file = std::path::Path::new(location.file());
                let shown = if file.is_absolute() {
                    file.file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
                } else {
                    location.file().to_owned()
                };
                format!(" at {shown}:{}", location.line())
            })
            .unwrap_or_default();
        let diagnostic = Diagnostic::new(
            Code::Internal,
            format!("adf crashed{place}; this is a bug"),
        )
        .with_hint(
            "Please report it at https://github.com/Yornik/adf-converter/issues with the command and input that caused it",
        );
        Reporter::for_panic().emit(&[diagnostic], None);
    }));
}

fn run_args(args: Vec<OsString>) -> u8 {
    match Cli::try_parse_from(args) {
        Ok(cli) => match Reporter::from_args(&cli.global) {
            Ok(reporter) => {
                let outcome = dispatch(&cli.command);
                finish(outcome, reporter)
            }
            Err((reporter, diagnostic)) => {
                reporter.emit(&[diagnostic], None);
                Code::Usage.exit_code()
            }
        },
        Err(err) => clap_error(&err),
    }
}

fn dispatch(command: &Command) -> Outcome {
    match command {
        Command::Convert(args) => convert::run(args),
        Command::Validate(args) => validate::run(args),
        Command::Capabilities { json } => info::capabilities(*json),
        Command::Examples { feature, json } => info::examples(feature.as_deref(), *json),
        Command::Schema { name } => info::schema(*name),
        Command::Explain { topic } => info::explain(topic),
        Command::Completions { shell } => info::completions(*shell),
        Command::Version { json } => info::version(*json),
    }
}

/// `--help` and `--version` go to stdout (exit 0); everything else is `E_USAGE` (exit 2).
#[allow(clippy::wildcard_enum_match_arm)] // every other kind is a usage error
fn clap_error(err: &clap::Error) -> u8 {
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            let text = err.render().to_string();
            match output::stdout(text.as_bytes()) {
                Ok(()) => 0,
                Err(io_err) if io_err.kind() == io::ErrorKind::BrokenPipe => 0,
                Err(_) => Code::OutputWrite.exit_code(),
            }
        }
        _ => {
            let reporter = Reporter::new(
                report::early_error_format(),
                report::early_no_color(),
                report::early_quiet(),
            );
            reporter.emit(&[usage_diagnostic(err)], None);
            Code::Usage.exit_code()
        }
    }
}

/// Splits clap's rendered error into a one-line message and a one-line hint.
fn usage_diagnostic(err: &clap::Error) -> Diagnostic {
    let rendered = err.render().to_string();
    let mut lines = rendered
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let first = lines.next().unwrap_or("invalid usage");
    let message = first.strip_prefix("error: ").unwrap_or(first);
    let rest: Vec<&str> = lines.collect();
    let missing = matches!(
        err.kind(),
        ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
    if missing {
        return Diagnostic::new(Code::Usage, "a command is required").with_hint(
            "Commands: convert, validate, capabilities, examples, schema, explain, completions, version; see `adf --help`",
        );
    }
    if err.kind() == ErrorKind::MissingRequiredArgument {
        return missing_argument(&rest);
    }
    let hint = if rest.is_empty() {
        "See `adf --help`".to_owned()
    } else {
        rest.iter()
            .map(|line| hint_sentence(line))
            .collect::<Vec<_>>()
            .join(" ")
    };
    Diagnostic::new(Code::Usage, message).with_hint(hint)
}

/// A missing required argument. clap puts the argument names on the lines after its header
/// ("the following required arguments were not provided:"); they belong in the message, and the
/// values the argument accepts in the hint.
fn missing_argument(rest: &[&str]) -> Diagnostic {
    let usage_at = rest
        .iter()
        .position(|line| line.starts_with("Usage:"))
        .unwrap_or(rest.len());
    let (names, after) = rest.split_at_checked(usage_at).unwrap_or((rest, &[]));
    let message = match names {
        [] => "a required argument is missing".to_owned(),
        [name] => format!("missing required argument {name}"),
        _ => format!("missing required arguments {}", names.join(", ")),
    };
    let command = after
        .first()
        .and_then(|usage| usage.split_whitespace().nth(2))
        .and_then(|name| Cli::command().find_subcommand(name).cloned());
    let mut sentences: Vec<String> = Vec::new();
    if let Some(command) = &command {
        for name in names {
            if let Some(accepted) = accepted_values(command, name) {
                sentences.push(hint_sentence(&accepted));
            }
        }
    }
    sentences.extend(after.iter().map(|line| hint_sentence(line)));
    let hint = if sentences.is_empty() {
        "See `adf --help`".to_owned()
    } else {
        sentences.join(" ")
    };
    Diagnostic::new(Code::Usage, message).with_hint(hint)
}

/// What the argument rendered as `name` (`<SHELL>`) of `command` accepts: its possible values, or
/// else its help text.
fn accepted_values(command: &clap::Command, name: &str) -> Option<String> {
    let arg = command.get_arguments().find(|arg| {
        let value_name = arg
            .get_value_names()
            .and_then(|names| names.first())
            .map_or_else(|| arg.get_id().as_str().to_uppercase(), ToString::to_string);
        format!("<{value_name}>") == name
    })?;
    let values: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| value.get_name().to_owned())
        .collect();
    if values.is_empty() {
        arg.get_help().map(|help| format!("{name}: {help}"))
    } else {
        Some(format!("Possible values: {}", values.join(", ")))
    }
}

/// Turns one of clap's continuation lines (`[possible values: …]`, `tip: …`, `Usage: …`) into a
/// sentence for the one-line hint.
fn hint_sentence(line: &str) -> String {
    let line = line
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(line);
    let line = line.strip_prefix("tip: ").unwrap_or(line);
    let mut sentence: String = match line.chars().next() {
        Some(first) => first.to_uppercase().chain(line.chars().skip(1)).collect(),
        None => String::new(),
    };
    if !sentence.ends_with('.') {
        sentence.push('.');
    }
    sentence
}

/// Prints the diagnostics (at most [`MAX_LISTED_PER_CODE`] of each code, then a summary), writes
/// the result (to the output file atomically, or to stdout) and returns the exit code. A closed
/// stdout (`adf … | head`) is not an error.
fn finish(outcome: Outcome, reporter: Reporter) -> u8 {
    let diagnostics = cap_per_code(outcome.diagnostics, MAX_LISTED_PER_CODE);
    reporter.emit(&diagnostics, outcome.label.as_deref());
    let mut exit = outcome.exit;
    if !outcome.stdout.is_empty() {
        match outcome
            .output_file
            .as_deref()
            .filter(|path| !convert::is_stdout(path))
        {
            Some(path) => {
                if let Err(err) = output::write_atomic(path, &outcome.stdout) {
                    let label = path.display().to_string();
                    let diagnostic = Diagnostic::new(
                        Code::OutputWrite,
                        format!(
                            "cannot write '{label}': {}",
                            output::describe_io_error(&err)
                        ),
                    )
                    .at(Location::default());
                    reporter.emit(&[diagnostic], Some(&label));
                    if exit == 0 {
                        exit = Code::OutputWrite.exit_code();
                    }
                }
            }
            None => match output::stdout(&outcome.stdout) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::BrokenPipe => return exit,
                Err(err) => {
                    let diagnostic = Diagnostic::new(
                        Code::OutputWrite,
                        format!(
                            "cannot write to stdout: {}",
                            output::describe_io_error(&err)
                        ),
                    );
                    reporter.emit(&[diagnostic], None);
                    if exit == 0 {
                        exit = Code::OutputWrite.exit_code();
                    }
                }
            },
        }
    }
    if let Some(summary) = outcome.summary
        && reporter.format == report::Format::Text
    {
        // The summary starts with the input file name, which may contain control characters.
        output::stderr(&format!("{}\n", report::sanitize(&summary)));
    }
    exit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(argv: &[&str]) -> Diagnostic {
        let err = Cli::try_parse_from(argv).unwrap_err();
        usage_diagnostic(&err)
    }

    #[test]
    fn usage_errors_are_one_line() {
        let diagnostic = usage(&["adf", "notes.md"]);
        assert_eq!(diagnostic.code, Code::Usage);
        assert!(
            diagnostic.message.contains("notes.md"),
            "{}",
            diagnostic.message
        );
        assert!(!diagnostic.message.starts_with("error:"));
        let hint = diagnostic.hint.unwrap();
        assert!(!hint.contains('\n'), "{hint}");
        let missing = usage(&["adf"]);
        assert_eq!(missing.message, "a command is required");
        let target = usage(&["adf", "convert", "-t", "x"]);
        assert_eq!(
            target.hint.as_deref(),
            Some("Possible values: auto, jira, confluence. For more information, try '--help'.")
        );
    }

    /// The missing argument belongs in the message, and what it accepts in the hint.
    #[test]
    fn missing_arguments_are_named() {
        let schema = usage(&["adf", "schema"]);
        assert_eq!(schema.message, "missing required argument <NAME>");
        assert_eq!(
            schema.hint.as_deref(),
            Some(
                "Possible values: adf, diagnostic, capabilities, validate-report. Usage: adf schema <NAME>. For more information, try '--help'."
            )
        );
        let completions = usage(&["adf", "completions"]);
        assert_eq!(completions.message, "missing required argument <SHELL>");
        let hint = completions.hint.unwrap();
        assert!(
            hint.starts_with("Possible values: bash, elvish, fish, powershell, zsh."),
            "{hint}"
        );
        let explain = usage(&["adf", "--error-format", "json", "explain"]);
        assert_eq!(explain.message, "missing required argument <TOPIC>");
        let hint = explain.hint.unwrap();
        assert!(
            hint.starts_with(
                "<TOPIC>: exit-codes, envelopes, syntax, codes, or a code such as W_IMAGE_AS_LINK."
            ),
            "{hint}"
        );
    }

    #[test]
    fn failed_outcomes_use_the_first_error() {
        let outcome = Outcome::failed(
            vec![
                Diagnostic::new(Code::ImageAsLink, "w"),
                Diagnostic::new(Code::InputJson, "e"),
                Diagnostic::new(Code::Schema, "s"),
            ],
            None,
        );
        assert_eq!(outcome.exit, 3);
        assert_eq!(Outcome::failed(Vec::new(), None).exit, 1);
    }
}
