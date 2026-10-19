//! Hard limits that keep the converter safe on untrusted input (agents generate it).

/// Default maximum input size in bytes (4 MiB). Larger inputs fail with `E_INPUT_TOO_LARGE`.
pub const DEFAULT_MAX_INPUT_BYTES: usize = 4 * 1024 * 1024;

/// Default maximum nesting depth of containers (lists, quotes, cells, …) in either direction.
/// Deeper input fails with `E_INPUT_TOO_DEEP`; no code path may recurse without checking it.
pub const DEFAULT_MAX_DEPTH: usize = 64;

/// JSON nesting levels allowed per container level in ADF *input*. Markdown → ADF emits up to
/// about six JSON levels per container (a table cell is `table` → `content` → `tableRow` →
/// `content` → `tableCell` → `content`), so ADF input may nest
/// `JSON_LEVELS_PER_CONTAINER × max_depth` levels of arrays/objects (512 by default) and
/// everything this tool writes can be read back.
pub const JSON_LEVELS_PER_CONTAINER: usize = 8;

/// Jira's documented limit for a text field / comment, in characters. Larger output gets
/// `W_SIZE_LIMIT`.
pub const JIRA_FIELD_CHAR_LIMIT: usize = 32_767;

/// Limits applied to one conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_depth: usize,
}

impl Limits {
    /// Maximum nesting of JSON arrays/objects accepted in ADF input.
    pub fn max_json_depth(&self) -> usize {
        self.max_depth.saturating_mul(JSON_LEVELS_PER_CONTAINER)
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}
