//! Rendering diagnostics for stderr: JSON lines (`schemas/diagnostic.v1.json`) or text
//! (`file:line:col: severity[CODE]: message (hint: …)`).

use std::sync::atomic::{AtomicU8, Ordering};

use serde::Serialize;

use super::args::{ErrorFormat, GlobalArgs};
use super::output;
use crate::diagnostics::{Code, Diagnostic, Location, Severity};

/// Base URL of the documentation anchors in the `docs` field.
pub(crate) const DOCS_URL: &str =
    "https://github.com/Yornik/adf-converter/blob/main/docs/cli-contract.md";

/// The resolved diagnostic format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Text,
    Json,
}

/// How diagnostics are printed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reporter {
    pub format: Format,
    pub color: bool,
    pub quiet: bool,
}

/// The format chosen for this process, for the panic hook (0 = not decided yet).
static PANIC_FORMAT: AtomicU8 = AtomicU8::new(0);

impl Reporter {
    /// Resolves `--error-format auto` and colour for this process.
    pub fn new(format: ErrorFormat, no_color: bool, quiet: bool) -> Self {
        let format = match format {
            ErrorFormat::Text => Format::Text,
            ErrorFormat::Json => Format::Json,
            ErrorFormat::Auto => {
                if output::stderr_is_terminal() {
                    Format::Text
                } else {
                    Format::Json
                }
            }
        };
        let color = format == Format::Text
            && !no_color
            && !output::no_color_env()
            && output::stderr_is_terminal();
        let reporter = Self {
            format,
            color,
            quiet,
        };
        reporter.remember();
        reporter
    }

    pub fn from_args(global: &GlobalArgs) -> Self {
        Self::new(global.error_format, global.no_color, global.quiet)
    }

    /// Makes this the format the panic hook uses.
    fn remember(self) {
        let value = match self.format {
            Format::Text => 1,
            Format::Json => 2,
        };
        PANIC_FORMAT.store(value, Ordering::Relaxed);
    }

    /// The reporter the panic hook should use.
    pub fn for_panic() -> Self {
        match PANIC_FORMAT.load(Ordering::Relaxed) {
            1 => Self {
                format: Format::Text,
                color: false,
                quiet: false,
            },
            2 => Self {
                format: Format::Json,
                color: false,
                quiet: false,
            },
            _ => Self::new(early_error_format(), true, false),
        }
    }

    /// Prints diagnostics to stderr (warnings are skipped with `-q`). `file` is the input name
    /// (`-` for stdin) added to every location.
    pub fn emit(self, diagnostics: &[Diagnostic], file: Option<&str>) {
        let mut text = String::new();
        for diagnostic in diagnostics {
            if self.quiet && !diagnostic.is_error() {
                continue;
            }
            text.push_str(&self.line(diagnostic, file));
            text.push('\n');
        }
        if !text.is_empty() {
            output::stderr(&text);
        }
    }

    /// One rendered diagnostic, without the trailing newline.
    pub fn line(self, diagnostic: &Diagnostic, file: Option<&str>) -> String {
        match self.format {
            Format::Json => json_line(diagnostic, file),
            Format::Text => text_line(diagnostic, file, self.color),
        }
    }
}

/// `ADF_ERROR_FORMAT` or `--error-format`, read from the raw arguments before clap has parsed
/// them (for usage errors and early panics). Invalid values fall back to `auto`.
pub(crate) fn early_error_format() -> ErrorFormat {
    let mut format = std::env::var("ADF_ERROR_FORMAT")
        .ok()
        .and_then(|value| parse_format(&value))
        .unwrap_or(ErrorFormat::Auto);
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let Some(arg) = arg.to_str() else { continue };
        if arg == "--" {
            break;
        }
        let value = if arg == "--error-format" {
            args.next().and_then(|value| value.into_string().ok())
        } else {
            arg.strip_prefix("--error-format=").map(str::to_owned)
        };
        if let Some(parsed) = value.as_deref().and_then(parse_format) {
            format = parsed;
        }
    }
    format
}

/// Whether `--no-color` appears in the raw arguments.
pub(crate) fn early_no_color() -> bool {
    std::env::args_os()
        .skip(1)
        .take_while(|arg| arg != "--")
        .any(|arg| arg == "--no-color")
}

/// Whether `-q`/`--quiet` appears in the raw arguments.
pub(crate) fn early_quiet() -> bool {
    std::env::args_os()
        .skip(1)
        .take_while(|arg| arg != "--")
        .any(|arg| arg == "-q" || arg == "--quiet")
}

fn parse_format(value: &str) -> Option<ErrorFormat> {
    match value.trim() {
        "auto" => Some(ErrorFormat::Auto),
        "text" => Some(ErrorFormat::Text),
        "json" => Some(ErrorFormat::Json),
        _ => None,
    }
}

/// The `docs` anchor URL of a code.
pub(crate) fn docs_url(code: Code) -> String {
    format!("{DOCS_URL}#{}", code.as_str().to_ascii_lowercase())
}

/// The JSON object of one diagnostic, as in a stderr line and in `validate --json` reports.
#[derive(Debug, Serialize)]
pub(crate) struct DiagnosticJson<'a> {
    schema_version: u32,
    severity: Severity,
    code: Code,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<LocationJson<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'a str>,
    docs: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<u8>,
}

#[derive(Debug, Serialize)]
struct LocationJson<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<&'a str>,
    #[serde(flatten)]
    location: &'a Location,
}

impl<'a> DiagnosticJson<'a> {
    pub(crate) fn new(diagnostic: &'a Diagnostic, file: Option<&'a str>) -> Self {
        static EMPTY: Location = Location {
            line: None,
            column: None,
            end_line: None,
            end_column: None,
            byte_offset: None,
            pointer: None,
        };
        let location = match (&diagnostic.location, file) {
            (None, None) => None,
            (location, file) => Some(LocationJson {
                file,
                location: location.as_ref().unwrap_or(&EMPTY),
            }),
        };
        Self {
            schema_version: 1,
            severity: diagnostic.severity(),
            code: diagnostic.code,
            message: &diagnostic.message,
            location,
            hint: diagnostic.hint.as_deref(),
            docs: docs_url(diagnostic.code),
            exit_code: diagnostic.is_error().then(|| diagnostic.code.exit_code()),
        }
    }
}

fn json_line(diagnostic: &Diagnostic, file: Option<&str>) -> String {
    serde_json::to_string(&DiagnosticJson::new(diagnostic, file)).unwrap_or_else(|_| {
        // Serializing plain strings and numbers cannot fail; keep a valid line regardless.
        format!(
            "{{\"schema_version\":1,\"severity\":\"error\",\"code\":\"E_INTERNAL\",\"message\":\"diagnostic could not be rendered\",\"docs\":\"{}\",\"exit_code\":1}}",
            docs_url(Code::Internal)
        )
    })
}

fn text_line(diagnostic: &Diagnostic, file: Option<&str>, color: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(file) = file {
        parts.push(sanitize(file));
    }
    let location = diagnostic.location.as_ref();
    if let Some(line) = location.and_then(|location| location.line) {
        parts.push(line.to_string());
        if let Some(column) = location.and_then(|location| location.column) {
            parts.push(column.to_string());
        }
    }
    let mut prefix = parts.join(":");
    if let Some(pointer) = location
        .and_then(|location| location.pointer.as_deref())
        .filter(|pointer| !pointer.is_empty())
    {
        prefix.push('#');
        prefix.push_str(&sanitize(pointer));
    }
    let severity = match (diagnostic.severity(), color) {
        (Severity::Error, true) => "\u{1b}[1;31merror\u{1b}[0m",
        (Severity::Warning, true) => "\u{1b}[1;33mwarning\u{1b}[0m",
        (Severity::Error, false) => "error",
        (Severity::Warning, false) => "warning",
    };
    let mut line = String::new();
    if !prefix.is_empty() {
        line.push_str(&prefix);
        line.push_str(": ");
    }
    line.push_str(severity);
    line.push('[');
    line.push_str(diagnostic.code.as_str());
    line.push_str("]: ");
    line.push_str(&sanitize(&diagnostic.message));
    if let Some(hint) = &diagnostic.hint {
        line.push_str(" (hint: ");
        line.push_str(&sanitize(hint));
        line.push(')');
    }
    line
}

/// Keeps a text line on one line and free of terminal escape sequences, whatever the input was.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reporter(format: Format) -> Reporter {
        Reporter {
            format,
            color: false,
            quiet: false,
        }
    }

    fn warning() -> Diagnostic {
        Diagnostic::new(
            Code::ImageAsLink,
            "Image converted to a link; ADF images need uploaded media",
        )
        .at(Location {
            line: Some(12),
            column: Some(3),
            end_line: Some(12),
            end_column: Some(41),
            ..Location::default()
        })
        .with_hint("Use --images external to embed by URL, or attach the file in Jira")
    }

    #[test]
    fn json_shape_matches_the_contract() {
        let line = reporter(Format::Json).line(&warning(), Some("comment.md"));
        assert_eq!(
            line,
            r#"{"schema_version":1,"severity":"warning","code":"W_IMAGE_AS_LINK","message":"Image converted to a link; ADF images need uploaded media","location":{"file":"comment.md","line":12,"column":3,"end_line":12,"end_column":41},"hint":"Use --images external to embed by URL, or attach the file in Jira","docs":"https://github.com/Yornik/adf-converter/blob/main/docs/cli-contract.md#w_image_as_link"}"#
        );
    }

    #[test]
    fn errors_carry_exit_codes_and_pointers() {
        let error = Diagnostic::new(Code::Schema, "bad").at(Location::pointer("/content/0"));
        let line = reporter(Format::Json).line(&error, Some("doc.json"));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["exit_code"], 4);
        assert_eq!(value["severity"], "error");
        assert_eq!(value["location"]["pointer"], "/content/0");
        assert_eq!(value["location"]["file"], "doc.json");
        assert_eq!(
            reporter(Format::Text).line(&error, Some("doc.json")),
            "doc.json#/content/0: error[E_SCHEMA]: bad"
        );
    }

    #[test]
    fn locations_are_optional() {
        let usage = Diagnostic::new(Code::Usage, "unknown flag");
        let line = reporter(Format::Json).line(&usage, None);
        assert!(!line.contains("location"), "{line}");
        assert_eq!(
            reporter(Format::Text).line(&usage, None),
            "error[E_USAGE]: unknown flag"
        );
        let read = Diagnostic::new(Code::InputRead, "cannot read");
        let line = reporter(Format::Json).line(&read, Some("-"));
        assert!(line.contains(r#""location":{"file":"-"}"#), "{line}");
        let root = Diagnostic::new(Code::SizeLimit, "big").at(Location::pointer(""));
        assert_eq!(
            reporter(Format::Text).line(&root, Some("x.json")),
            "x.json: warning[W_SIZE_LIMIT]: big"
        );
    }

    #[test]
    fn text_shape_matches_the_contract() {
        assert_eq!(
            reporter(Format::Text).line(&warning(), Some("comment.md")),
            "comment.md:12:3: warning[W_IMAGE_AS_LINK]: Image converted to a link; ADF images need uploaded media (hint: Use --images external to embed by URL, or attach the file in Jira)"
        );
        let colored = Reporter {
            format: Format::Text,
            color: true,
            quiet: false,
        };
        assert!(
            colored
                .line(&warning(), None)
                .starts_with("12:3: \u{1b}[1;33mwarning")
        );
    }

    #[test]
    fn text_is_sanitized() {
        let nasty = Diagnostic::new(Code::Schema, "line one\nline two \u{1b}[31m");
        let line = reporter(Format::Text).line(&nasty, Some("a\nb"));
        assert!(!line.contains('\n') && !line.contains('\u{1b}'), "{line}");
    }

    #[test]
    fn docs_anchor_is_lowercase() {
        assert_eq!(
            docs_url(Code::InputTooDeep),
            "https://github.com/Yornik/adf-converter/blob/main/docs/cli-contract.md#e_input_too_deep"
        );
    }
}
