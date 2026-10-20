//! Reading the input: a file or stdin, at most [`Limits::max_input_bytes`], valid UTF-8, without a
//! byte-order mark; and parsing JSON input with located errors.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde_json::Value;

use super::output::describe_io_error;
use crate::adf::envelope::strip_position;
use crate::diagnostics::{Code, Diagnostic, LineIndex, Location};
use crate::limits::Limits;

/// The input of one command.
#[derive(Debug)]
pub(crate) struct Input {
    /// The name used in diagnostics: the path as given, or `-` for stdin.
    pub label: String,
    /// The text, without a leading UTF-8 byte-order mark.
    pub text: String,
}

/// Whether `path` means stdin.
pub(crate) fn is_stdin(path: Option<&Path>) -> bool {
    path.is_none_or(|path| path.as_os_str() == "-")
}

/// The diagnostic label of an input path.
pub(crate) fn label(path: Option<&Path>) -> String {
    match path {
        Some(path) if !is_stdin(Some(path)) => path.display().to_string(),
        _ => "-".to_owned(),
    }
}

/// Reads the whole input. Errors are `E_INPUT_READ`, `E_INPUT_TOO_LARGE` or `E_INPUT_ENCODING`.
pub(crate) fn read(path: Option<&Path>, limits: &Limits) -> Result<Input, Diagnostic> {
    let label = label(path);
    let bytes = match path.filter(|path| !is_stdin(Some(path))) {
        None => read_limited(io::stdin().lock(), limits.max_input_bytes)
            .map_err(|err| read_error("stdin", &err))?,
        Some(path) => {
            if path.is_dir() {
                return Err(Diagnostic::new(
                    Code::InputRead,
                    format!("cannot read {}: is a directory", quote(&label)),
                ));
            }
            let file = File::open(path).map_err(|err| read_error(&quote(&label), &err))?;
            read_limited(file, limits.max_input_bytes)
                .map_err(|err| read_error(&quote(&label), &err))?
        }
    };
    if bytes.len() > limits.max_input_bytes {
        return Err(Diagnostic::new(
            Code::InputTooLarge,
            format!(
                "the input is larger than the {} limit ({} bytes)",
                human_size(limits.max_input_bytes),
                limits.max_input_bytes
            ),
        )
        .with_hint("Split the input into smaller documents"));
    }
    let text = decode(bytes)?;
    Ok(Input { label, text })
}

/// Reads at most `limit + 1` bytes, so oversized input is detected without reading all of it.
fn read_limited(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    reader.take(cap).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_error(what: &str, err: &io::Error) -> Diagnostic {
    Diagnostic::new(
        Code::InputRead,
        format!("cannot read {what}: {}", describe_io_error(err)),
    )
}

/// UTF-8 decoding that reports the first invalid byte; strips a byte-order mark.
fn decode(bytes: Vec<u8>) -> Result<String, Diagnostic> {
    let text = String::from_utf8(bytes).map_err(|err| {
        let offset = err.utf8_error().valid_up_to();
        let valid = err.as_bytes().get(..offset).unwrap_or_default();
        // The valid prefix is UTF-8 by definition.
        let prefix = std::str::from_utf8(valid).unwrap_or_default();
        let (line, column) = LineIndex::new(prefix).line_col(prefix.len());
        Diagnostic::new(
            Code::InputEncoding,
            format!("the input is not valid UTF-8 (first invalid byte at offset {offset})"),
        )
        .at(Location {
            line: Some(line),
            column: Some(column),
            byte_offset: Some(offset),
            ..Location::default()
        })
        .with_hint("Convert the input to UTF-8")
    })?;
    Ok(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_owned(),
        None => text,
    })
}

/// Parses JSON input. Errors are `E_INPUT_JSON`, or `E_INPUT_TOO_DEEP` beyond
/// [`Limits::max_json_depth`](crate::Limits::max_json_depth) levels, with the line and column
/// (counted in characters).
pub(crate) fn parse_json(text: &str) -> Result<Value, Diagnostic> {
    let max_depth = crate::Limits::default().max_json_depth();
    crate::json::parse_bounded(text, max_depth).map_err(|err| match err {
        crate::json::JsonError::TooDeep { offset } => {
            let (line, column) = crate::LineIndex::new(text).line_col(offset);
            Diagnostic::new(
                Code::InputTooDeep,
                format!("the JSON input nests deeper than {max_depth} levels"),
            )
            .at(Location {
                line: Some(line),
                column: Some(column),
                byte_offset: Some(offset),
                ..Location::default()
            })
            .with_hint(crate::limits::json_depth_hint(max_depth))
        }
        crate::json::JsonError::Syntax(err) => {
            let message = err.to_string();
            let mut diagnostic = Diagnostic::new(
                Code::InputJson,
                format!("invalid JSON: {}", strip_position(&message)),
            );
            if err.line() > 0 {
                let column = char_column(text, err.line(), err.column());
                diagnostic = diagnostic.at(Location {
                    line: u32::try_from(err.line()).ok(),
                    column: Some(column),
                    ..Location::default()
                });
            }
            diagnostic
        }
    })
}

/// Converts `serde_json`'s byte column (1-based) on `line` to a character column.
fn char_column(text: &str, line: usize, byte_column: usize) -> u32 {
    let Some(line_text) = text.split('\n').nth(line.saturating_sub(1)) else {
        return u32::try_from(byte_column).unwrap_or(u32::MAX);
    };
    let mut end = byte_column.saturating_sub(1).min(line_text.len());
    while !line_text.is_char_boundary(end) {
        end -= 1;
    }
    let chars = line_text
        .get(..end)
        .map_or(0, |prefix| prefix.chars().count());
    u32::try_from(chars + 1).unwrap_or(u32::MAX)
}

fn quote(label: &str) -> String {
    format!("'{label}'")
}

fn human_size(bytes: usize) -> String {
    if bytes.is_multiple_of(1024 * 1024) {
        format!("{} MiB", bytes / (1024 * 1024))
    } else {
        format!("{bytes}-byte")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdin_labels() {
        assert!(is_stdin(None));
        assert!(is_stdin(Some(Path::new("-"))));
        assert!(!is_stdin(Some(Path::new("a.md"))));
        assert_eq!(label(None), "-");
        assert_eq!(label(Some(Path::new("dir/a.md"))), "dir/a.md");
    }

    #[test]
    fn invalid_utf8_reports_the_byte_offset() {
        let err = decode(b"ok\nab\xffcd".to_vec()).unwrap_err();
        assert_eq!(err.code, Code::InputEncoding);
        let location = err.location.unwrap();
        assert_eq!(location.byte_offset, Some(5));
        assert_eq!((location.line, location.column), (Some(2), Some(3)));
    }

    #[test]
    fn bom_is_removed() {
        assert_eq!(decode("\u{feff}# x".as_bytes().to_vec()).unwrap(), "# x");
    }

    #[test]
    fn size_limit() {
        let limits = Limits {
            max_input_bytes: 4,
            ..Limits::default()
        };
        assert_eq!(read_limited(&b"12345678"[..], 4).unwrap().len(), 5);
        let dir = std::env::temp_dir().join(format!("adf-input-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.md");
        std::fs::write(&path, "12345").unwrap();
        assert_eq!(
            read(Some(&path), &limits).unwrap_err().code,
            Code::InputTooLarge
        );
        std::fs::write(&path, "1234").unwrap();
        assert_eq!(read(Some(&path), &limits).unwrap().text, "1234");
        let err = read(Some(&dir.join("missing.md")), &limits).unwrap_err();
        assert_eq!(err.code, Code::InputRead);
        assert!(
            err.message.ends_with("no such file or directory"),
            "{}",
            err.message
        );
        assert!(
            read(Some(&dir), &limits)
                .unwrap_err()
                .message
                .ends_with("is a directory")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn json_errors_are_located() {
        let err = parse_json("{\n  \"type\": \"doc\",,\n}").unwrap_err();
        assert_eq!(err.code, Code::InputJson);
        let location = err.location.unwrap();
        assert_eq!(location.line, Some(2));
        assert!(!err.message.contains(" at line "), "{}", err.message);
        // Columns count characters, not bytes.
        let err = parse_json("{\"é\": x}").unwrap_err();
        assert_eq!(err.location.unwrap().column, Some(7));
        // Deeper than serde_json's own limit (128) but within ours (512) parses fine.
        let ok = format!("{}{}", "[".repeat(200), "]".repeat(200));
        assert!(parse_json(&ok).is_ok());
        let deep = "[".repeat(600);
        let err = parse_json(&deep).unwrap_err();
        assert_eq!(err.code, Code::InputTooDeep);
        assert_eq!(err.location.unwrap().byte_offset, Some(512));
    }
}
