//! Bounded JSON parsing for untrusted ADF input.
//!
//! `serde_json`'s built-in recursion limit (128) is too low for ADF that this tool itself
//! produces, and turning it off would let deeply nested input overflow the stack. So the input
//! is first scanned iteratively for its nesting depth, and only parsed (with the recursion limit
//! disabled) when it is within [`crate::Limits::max_json_depth`].

use serde::Deserialize;
use serde_json::Value;

/// Why bounded parsing failed.
#[derive(Debug)]
pub enum JsonError {
    /// Nesting exceeds the limit; `offset` is the byte offset of the first bracket past it.
    TooDeep { offset: usize },
    /// Not valid JSON.
    Syntax(serde_json::Error),
}

/// Returns the byte offset of the first `[` or `{` that nests deeper than `max_depth`, without
/// recursion. Brackets inside strings are ignored. Malformed JSON is left to the parser.
pub fn find_too_deep(text: &str, max_depth: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in text.bytes().enumerate() {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > max_depth {
                    return Some(offset);
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// Parses `text` as one JSON value, failing with [`JsonError::TooDeep`] beyond `max_depth`
/// levels of arrays/objects.
pub fn parse_bounded(text: &str, max_depth: usize) -> Result<Value, JsonError> {
    if let Some(offset) = find_too_deep(text, max_depth) {
        return Err(JsonError::TooDeep { offset });
    }
    let mut deserializer = serde_json::Deserializer::from_str(text);
    deserializer.disable_recursion_limit();
    let value = Value::deserialize(&mut deserializer).map_err(JsonError::Syntax)?;
    deserializer.end().map_err(JsonError::Syntax)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_scan_ignores_strings_and_finds_the_offender() {
        assert_eq!(find_too_deep(r#"{"a":"[[[[[["}"#, 1), None);
        assert_eq!(find_too_deep(r#"{"a":"\"[[[","b":[[1]]}"#, 2), Some(18));
        assert_eq!(find_too_deep("[[[]]]", 3), None);
        assert_eq!(find_too_deep("[[[[]]]]", 3), Some(3));
    }

    #[test]
    fn parses_beyond_serde_json_default_limit_but_not_beyond_ours() {
        let deep = format!("{}{}", "[".repeat(300), "]".repeat(300));
        assert!(serde_json::from_str::<Value>(&deep).is_err());
        assert!(parse_bounded(&deep, 512).is_ok());
        assert!(matches!(
            parse_bounded(&deep, 299),
            Err(JsonError::TooDeep { offset: 299 })
        ));
        // Huge depth is rejected by the scan, never by a stack overflow.
        let huge = "[".repeat(1_000_000);
        assert!(matches!(
            parse_bounded(&huge, 512),
            Err(JsonError::TooDeep { .. })
        ));
        assert!(matches!(
            parse_bounded("{} x", 8),
            Err(JsonError::Syntax(_))
        ));
    }
}
