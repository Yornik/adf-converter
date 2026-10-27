//! `![alt](adf:csv/PATH?options)`: a CSV file included as a table.
//!
//! This is the ONLY module of the library that touches the file system (`tests/source_lints.rs`
//! checks it), and only when the caller set [`IncludeOptions`](crate::options::IncludeOptions)
//! (the CLI flag `--include-root`). The Markdown decides what is read, so every rule here
//! assumes hostile Markdown:
//!
//! * The path is validated as text before any file system call: relative, `/`-separated, no
//!   `..`/`.`/empty components, no backslash, no `:`, no control character (after percent-decoding
//!   it once).
//! * The file is found by canonicalising `root/PATH` (symlinks followed) and must lie inside the
//!   canonical root and be a regular file. A FIFO or device is refused from its metadata before
//!   it is opened (opening a FIFO blocks). After opening, the file is checked again (still a
//!   regular file, still the same path and inode), so a path swapped in between is refused
//!   before any byte is read.
//! * Reading is bounded while it happens: at most the per-file limit (plus one byte, to see that
//!   it was exceeded) and the rest of the per-document budget. Nothing reads to the end of an
//!   unbounded source.
//! * Messages name the rule and the PATH as written, never a resolved path.
//!
//! Not covered, and said so in `docs/syntax.md`: a hard link inside the root to a file outside it
//! (the file system gives no way to tell), and someone who can write inside the root while the
//! conversion runs. The root should hold only data that may be published.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::adf::model::Table;
use crate::csv::params::{CsvParams, embedded_hint, embedded_table};
use crate::diagnostics::{Code, Diagnostic, Location};
use crate::options::IncludeOptions;

use super::cx::Cx;
use super::extensions::truncate;
use super::ir::Span;

/// One file read for an include, in document order. `path` is the PATH as written in the
/// Markdown (before percent-decoding), without the query.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IncludedFile {
    pub path: String,
    pub bytes: usize,
}

/// What a document has read so far, and the cached canonical root.
#[derive(Debug, Default)]
pub(crate) struct Includes {
    root: Option<PathBuf>,
    files: usize,
    bytes: usize,
    pub(crate) read: Vec<IncludedFile>,
}

/// The syntax, shown as the hint of path problems.
const SYNTAX_HINT: &str = "Write ![alt](adf:csv/dir/file.csv?delimiter=semicolon&header=false&max-rows=50&align=numbers): a relative path with / separators, inside the --include-root directory";

/// Longest PATH shown in a message.
const SHOWN_CHARS: usize = 80;

/// A PATH as written, made safe to print: control characters escaped, short.
fn shown(raw: &str) -> String {
    truncate(&raw.escape_debug().to_string(), SHOWN_CHARS)
}

/// Splits the text after `adf:csv/` into the PATH and the query, both as written.
fn split_query(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (arg, None),
    }
}

/// The PATH of an include as written, for the alt text of a disabled include.
pub(crate) fn raw_path(arg: Option<&str>) -> &str {
    split_query(arg.unwrap_or_default()).0
}

/// Percent-decodes `text` once. `+` stays a plus. An invalid escape or a result that is not
/// UTF-8 is an error.
fn percent_decode(text: &str) -> Result<String, &'static str> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        if byte == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .ok_or("has an incomplete %-escape")?;
            let high = hex_value(hex.first().copied()).ok_or("has an invalid %-escape")?;
            let low = hex_value(hex.get(1).copied()).ok_or("has an invalid %-escape")?;
            out.push(high * 16 + low);
            i += 3;
        } else {
            out.push(byte);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "is not valid UTF-8 after %-decoding")
}

fn hex_value(byte: Option<u8>) -> Option<u8> {
    match byte? {
        b @ b'0'..=b'9' => Some(b - b'0'),
        b @ b'a'..=b'f' => Some(b - b'a' + 10),
        b @ b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// The components of a decoded PATH, or the rule it breaks. Purely textual: no file system call.
fn components(decoded: &str) -> Result<Vec<&str>, &'static str> {
    if decoded.is_empty() {
        return Err("the path is empty");
    }
    if decoded.chars().any(char::is_control) {
        return Err("the path contains a control character");
    }
    if decoded.contains('\\') {
        return Err("the path contains a backslash; use / as the separator");
    }
    if decoded.starts_with('/') {
        return Err("the path is absolute; it must be relative to the include root");
    }
    let parts: Vec<&str> = decoded.split('/').collect();
    if parts.iter().any(|part| part.contains(':')) {
        return Err("the path contains ':' (a drive letter or a scheme is not allowed)");
    }
    if parts.contains(&"..") {
        return Err("the path contains '..'");
    }
    if parts.contains(&".") {
        return Err("the path contains a '.' component");
    }
    if parts.contains(&"") {
        return Err("the path contains an empty component (// or a trailing /)");
    }
    Ok(parts)
}

/// Why reading a file failed.
enum ReadError {
    /// `E_INCLUDE`: the rule that failed.
    Rule(String),
    /// `E_INPUT_TOO_LARGE`.
    TooLarge(usize),
    /// `E_INPUT_ENCODING` at this byte offset.
    Encoding(usize),
}

fn rule(message: &str) -> ReadError {
    ReadError::Rule(message.to_owned())
}

/// A fixed sentence for an I/O failure. `io::Error` itself is never shown: its text can hold a path.
fn io_rule(error: &std::io::Error) -> ReadError {
    let kind = error.kind();
    if kind == std::io::ErrorKind::NotFound {
        rule("the file does not exist or is not inside the include root")
    } else if kind == std::io::ErrorKind::PermissionDenied {
        rule("the file cannot be read (permission denied)")
    } else {
        rule("the file cannot be read")
    }
}

/// Where the next read may go, and how much.
struct Budget {
    /// Per file.
    file_limit: usize,
    /// What is left of the per-document total.
    remaining: usize,
}

/// Reads the file at `components` under the canonical `root`, with every check described in the
/// module documentation. Returns the text.
fn read_file(root: &Path, parts: &[&str], budget: &Budget) -> Result<String, ReadError> {
    let mut joined = root.to_path_buf();
    joined.extend(parts);
    let canonical = fs::canonicalize(&joined).map_err(|e| io_rule(&e))?;
    if !canonical.starts_with(root) {
        // The same words as for a missing file: the message must not tell which names exist
        // outside the root.
        return Err(rule(
            "the file does not exist or is not inside the include root",
        ));
    }
    // From the metadata, before `open`: opening a FIFO would block.
    let metadata = fs::metadata(&canonical).map_err(|e| io_rule(&e))?;
    if !metadata.is_file() {
        return Err(rule(
            "the path is not a regular file (a directory, device, FIFO or socket)",
        ));
    }
    let mut file = File::open(&canonical).map_err(|e| io_rule(&e))?;
    verify_opened(&file, &joined, &canonical)?;
    let cap = budget.file_limit.min(budget.remaining).saturating_add(1);
    let mut buffer = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(usize::MAX)
            .min(cap),
    );
    // `take` ends the read at `cap` bytes whatever the file turns out to be.
    (&mut file)
        .take(u64::try_from(cap).unwrap_or(u64::MAX))
        .read_to_end(&mut buffer)
        .map_err(|e| io_rule(&e))?;
    if buffer.len() > budget.file_limit {
        return Err(ReadError::TooLarge(budget.file_limit));
    }
    if buffer.len() > budget.remaining {
        return Err(rule("the per-document include budget is used up"));
    }
    String::from_utf8(buffer).map_err(|err| ReadError::Encoding(err.utf8_error().valid_up_to()))
}

/// After `open`: the handle is a regular file, and the path still leads to the file that was
/// checked (nobody swapped a symbolic link in between).
fn verify_opened(file: &File, joined: &Path, canonical: &Path) -> Result<(), ReadError> {
    let swapped = || rule("the file changed while it was being opened");
    let opened = file.metadata().map_err(|e| io_rule(&e))?;
    if !opened.is_file() {
        return Err(rule("the path is not a regular file"));
    }
    if fs::canonicalize(joined).map_err(|e| io_rule(&e))? != canonical {
        return Err(swapped());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let now = fs::metadata(canonical).map_err(|e| io_rule(&e))?;
        if (now.dev(), now.ino()) != (opened.dev(), opened.ino()) {
            return Err(swapped());
        }
    }
    Ok(())
}

/// The canonical include root, canonicalised once per document.
fn canonical_root(options: &IncludeOptions, state: &mut Includes) -> Result<PathBuf, ReadError> {
    if let Some(root) = &state.root {
        return Ok(root.clone());
    }
    let root = fs::canonicalize(&options.root)
        .ok()
        .filter(|root| root.is_dir())
        .ok_or_else(|| rule("the include root does not exist or is not a directory"))?;
    state.root = Some(root.clone());
    Ok(root)
}

/// Reads the query `key=value&key=value` into [`CsvParams`]; values are percent-decoded once.
fn parse_query(query: Option<&str>) -> Result<CsvParams, String> {
    let Some(query) = query.filter(|q| !q.is_empty()) else {
        return Ok(CsvParams::default());
    };
    let mut pairs = Vec::new();
    for pair in query.split('&') {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("the option '{}' has no value; write key=value", shown(pair)))?;
        let value = percent_decode(value)
            .map_err(|why| format!("the value of the option '{}' {why}", shown(key)))?;
        pairs.push((key, value));
    }
    CsvParams::from_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())))
}

/// A message for a diagnostic: no control characters.
fn plain(message: &str) -> String {
    message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Reads and converts one `adf:csv` include. `arg` is the text after `adf:csv/`. Warnings of the
/// CSV go to `cx`; an error is returned for the caller to fail the conversion with.
///
/// Called only when [`MdToAdfOptions::include`](crate::options::MdToAdfOptions::include) is set.
pub(crate) fn load(arg: Option<&str>, span: &Span, cx: &mut Cx<'_>) -> Result<Table, Diagnostic> {
    let settings = cx.opts;
    let Some(options) = settings.include.as_ref() else {
        return Err(cx.diag(
            Code::Internal,
            "adf:csv include without an include root; please report this input",
            span,
        ));
    };
    let (raw, query) = split_query(arg.unwrap_or_default());
    let label = shown(raw);
    let refuse = |cx: &Cx<'_>, message: &str| {
        cx.diag(
            Code::Include,
            plain(&format!("adf:csv/{label}: {message}")),
            span,
        )
        .with_hint(SYNTAX_HINT)
    };

    let decoded = percent_decode(raw).map_err(|why| refuse(cx, &format!("the path {why}")))?;
    let parts = components(&decoded).map_err(|why| refuse(cx, why))?;
    let params = parse_query(query).map_err(|why| {
        cx.diag(
            Code::CsvOption,
            plain(&format!("adf:csv/{label}: {why}")),
            span,
        )
    })?;

    if cx.includes.files >= options.max_files {
        return Err(refuse(
            cx,
            &format!("a document may include at most {} files", options.max_files),
        ));
    }
    let limits = &cx.opts.limits;
    let budget = Budget {
        file_limit: limits.max_markdown_bytes.min(limits.max_input_bytes),
        remaining: options.max_total_bytes.saturating_sub(cx.includes.bytes),
    };
    let text = canonical_root(options, &mut cx.includes)
        .and_then(|root| read_file(&root, &parts, &budget))
        .map_err(|error| read_failure(error, &label, cx, span))?;

    cx.includes.files += 1;
    cx.includes.bytes += text.len();
    cx.includes.read.push(IncludedFile {
        path: raw.to_owned(),
        bytes: text.len(),
    });

    let opts = params.options(cx.opts.target, &cx.opts.limits, None);
    match embedded_table(&text, &opts, cx.output_remaining()) {
        Ok(converted) => {
            for warning in converted.warnings {
                let relocated = relocate(warning, &label, cx, span);
                cx.warnings.push(relocated);
            }
            cx.charge_table(&converted.output, span)?;
            Ok(converted.output)
        }
        Err(diagnostics) => {
            let mut error = None;
            for diagnostic in diagnostics.0 {
                let relocated = relocate(diagnostic, &label, cx, span);
                if relocated.is_error() && error.is_none() {
                    error = Some(relocated);
                } else {
                    cx.warnings.push(relocated);
                }
            }
            Err(error.unwrap_or_else(|| {
                cx.diag(
                    Code::Internal,
                    "CSV conversion failed without an error",
                    span,
                )
            }))
        }
    }
}

/// The diagnostic for a failed read.
fn read_failure(error: ReadError, label: &str, cx: &Cx<'_>, span: &Span) -> Diagnostic {
    match error {
        ReadError::Rule(message) => {
            let hint = if message.contains("budget") || message.contains("at most") {
                "Split the document, or merge the data files"
            } else if message.contains("does not exist") || message.contains("cannot be read") {
                "Check the file name, relative to the --include-root directory"
            } else {
                SYNTAX_HINT
            };
            cx.diag(
                Code::Include,
                plain(&format!("adf:csv/{label}: {message}")),
                span,
            )
            .with_hint(hint)
        }
        ReadError::TooLarge(limit) => cx
            .diag(
                Code::InputTooLarge,
                plain(&format!(
                    "The file adf:csv/{label} is larger than the {limit}-byte limit"
                )),
                span,
            )
            .with_hint("Include a smaller file, or cut the file first, for example with head -n 100"),
        ReadError::Encoding(offset) => cx
            .diag(
                Code::InputEncoding,
                plain(&format!(
                    "The file adf:csv/{label} is not valid UTF-8 (first invalid byte at offset {offset})"
                )),
                span,
            )
            .with_hint("Convert the file to UTF-8"),
    }
}

/// A diagnostic of the CSV engine, moved to the image in the Markdown: the file's own line and
/// column go into the message, prefixed with the PATH.
fn relocate(diagnostic: Diagnostic, label: &str, cx: &Cx<'_>, span: &Span) -> Diagnostic {
    let at = diagnostic
        .location
        .as_ref()
        .filter(|_| !diagnostic.message.contains("line "))
        .and_then(|l: &Location| l.line.map(|line| (line, l.column)))
        .map(|(line, column)| match column {
            Some(column) => format!(" (line {line}, column {column} of the file)"),
            None => format!(" (line {line} of the file)"),
        })
        .unwrap_or_default();
    let message = plain(&format!("In {label}: {}{at}", diagnostic.message));
    let mut moved = cx.diag(diagnostic.code, message, span);
    moved.hint = diagnostic.hint.map(|hint| embedded_hint(&hint));
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_happens_once() {
        assert_eq!(percent_decode("a%20b").unwrap(), "a b");
        assert_eq!(percent_decode("a%2520b").unwrap(), "a%20b");
        assert_eq!(percent_decode("a+b").unwrap(), "a+b");
        assert!(percent_decode("%").is_err());
        assert!(percent_decode("%4").is_err());
        assert!(percent_decode("%zz").is_err());
        assert!(percent_decode("%ff").is_err());
    }

    #[test]
    fn paths_are_judged_after_decoding() {
        for (raw, needle) in [
            ("", "empty"),
            ("/etc/passwd", "absolute"),
            ("%2Fetc/passwd", "absolute"),
            ("a\\b.csv", "backslash"),
            ("a%5Cb.csv", "backslash"),
            ("C:/x.csv", "':'"),
            ("C:x.csv", "':'"),
            ("../x.csv", "'..'"),
            ("a/../../x.csv", "'..'"),
            ("%2e%2e%2fx.csv", "'..'"),
            ("./x.csv", "'.'"),
            ("a//b.csv", "empty component"),
            ("a/", "empty component"),
            ("a%00b.csv", "control"),
            ("a%0ab.csv", "control"),
            ("a\tb.csv", "control"),
            ("a\u{7f}b.csv", "control"),
            ("a\u{85}b.csv", "control"),
        ] {
            let decoded = percent_decode(raw).unwrap();
            let why = components(&decoded).unwrap_err();
            assert!(why.contains(needle), "{raw:?}: {why}");
        }
        assert_eq!(
            components(&percent_decode("dir/my%20file.csv").unwrap()).unwrap(),
            ["dir", "my file.csv"]
        );
        assert!(components("a/.hidden/b..c.csv").is_ok());
    }

    #[test]
    fn the_query_is_percent_decoded_once() {
        let params = parse_query(Some("delimiter=%3B&header=false")).unwrap();
        assert_eq!(params.header, Some(false));
        assert_eq!(
            params.delimiter.map(crate::options::CsvDelimiter::as_char),
            Some(';')
        );
        assert!(parse_query(Some("header")).is_err());
        assert!(parse_query(Some("header=true&&max-rows=2")).is_err());
        assert!(parse_query(Some("colour=red")).is_err());
        assert!(parse_query(Some("header=%zz")).is_err());
        assert_eq!(parse_query(Some("")).unwrap(), CsvParams::default());
        assert_eq!(parse_query(None).unwrap(), CsvParams::default());
    }

    #[test]
    fn shown_paths_are_short_and_printable() {
        let long = "x".repeat(500);
        assert!(shown(&long).chars().count() <= SHOWN_CHARS + 1);
        assert!(!shown("a\u{0}\nb").chars().any(char::is_control));
    }
}
