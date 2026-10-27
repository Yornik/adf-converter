//! Options for conversions. These mirror the CLI flags in `docs/cli-contract.md`.

use std::path::PathBuf;

use crate::limits::Limits;

/// The product the ADF is meant for. Selects the node/mark allowlist used by validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    Jira,
    Confluence,
}

impl Target {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jira => "jira",
            Self::Confluence => "confluence",
        }
    }
}

/// How a soft line break (a newline inside a paragraph) is converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SoftBreak {
    /// A space, as CommonMark renders it (default).
    #[default]
    Space,
    /// A `hardBreak` node, as GitHub comments render it.
    HardBreak,
}

/// What happens to raw HTML that has no ADF equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HtmlPolicy {
    /// Keep it as literal text, with `W_HTML_AS_TEXT` (default).
    #[default]
    Text,
    /// Drop it, with `W_HTML_DROPPED`.
    Drop,
    /// Fail with `E_LOSSY`.
    Error,
}

/// What happens to Markdown images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImagePolicy {
    /// A text link to the image URL, with `W_IMAGE_AS_LINK` (default; always valid).
    #[default]
    Link,
    /// An external `mediaSingle` (Jira rendering unverified).
    External,
    /// Drop it, with `W_IMAGE_DROPPED`.
    Drop,
}

/// Which ADF extensions to Markdown are recognised. Standard CommonMark/GFM syntax (including
/// task lists, tables and footnotes) is always on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Extensions {
    /// `> [!NOTE]` … → `panel`.
    pub alerts: bool,
    /// `<details><summary>` → `expand` / `nestedExpand`.
    pub details: bool,
    /// `::: cell` → a 1×1 table used as a box.
    pub cells: bool,
    /// ```` ```adf ```` → raw ADF spliced in.
    pub raw_adf: bool,
    /// `[@Name](adf:mention/ID)` → `mention`.
    pub mention: bool,
    /// `[TEXT](adf:status/COLOR)` → `status`.
    pub status: bool,
    /// `[YYYY-MM-DD](adf:date)` → `date`.
    pub date: bool,
    /// `[😄](adf:emoji/:smile:)` → `emoji`.
    pub emoji: bool,
    /// `[https://…](adf:card)` → `inlineCard` (smart link).
    pub card: bool,
    /// `![alt](adf:media/UUID?…)` → a `mediaSingle` holding an existing Media Services file.
    pub media: bool,
    /// ```` ```csv ```` / ```` ```tsv ```` fences → a table, and `![alt](adf:csv/PATH?…)` → a
    /// table read from a file under `MdToAdfOptions::include` (issue #13, phases 2 and 3).
    pub csv: bool,
}

impl Extensions {
    /// The names accepted by `--extensions`, in documentation order.
    pub const NAMES: &'static [&'static str] = &[
        "alerts", "details", "cells", "raw-adf", "mention", "status", "date", "emoji", "card",
        "media", "csv",
    ];

    pub fn all() -> Self {
        Self {
            alerts: true,
            details: true,
            cells: true,
            raw_adf: true,
            mention: true,
            status: true,
            date: true,
            emoji: true,
            card: true,
            media: true,
            csv: true,
        }
    }

    pub fn none() -> Self {
        Self {
            alerts: false,
            details: false,
            cells: false,
            raw_adf: false,
            mention: false,
            status: false,
            date: false,
            emoji: false,
            card: false,
            media: false,
            csv: false,
        }
    }

    /// Parses `all`, `none`, or a comma-separated list of [`Self::NAMES`].
    pub fn parse(spec: &str) -> Result<Self, String> {
        match spec.trim() {
            "all" => return Ok(Self::all()),
            "none" | "" => return Ok(Self::none()),
            _ => {}
        }
        let mut ext = Self::none();
        for name in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            match name {
                "alerts" => ext.alerts = true,
                "details" => ext.details = true,
                "cells" => ext.cells = true,
                "raw-adf" => ext.raw_adf = true,
                "mention" => ext.mention = true,
                "status" => ext.status = true,
                "date" => ext.date = true,
                "emoji" => ext.emoji = true,
                "card" => ext.card = true,
                "media" => ext.media = true,
                "csv" => ext.csv = true,
                other => {
                    return Err(format!(
                        "unknown extension '{other}'; expected all, none, or a list of: {}",
                        Self::NAMES.join(", ")
                    ));
                }
            }
        }
        Ok(ext)
    }
}

impl Default for Extensions {
    fn default() -> Self {
        Self::all()
    }
}

/// Options for Markdown → ADF.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MdToAdfOptions {
    pub target: Target,
    pub softbreak: SoftBreak,
    pub html: HtmlPolicy,
    pub images: ImagePolicy,
    pub extensions: Extensions,
    /// Reading CSV files named by `![alt](adf:csv/PATH)`. `None` (the default) never touches the
    /// file system: the image becomes its alt text with `W_INCLUDE_DISABLED`.
    pub include: Option<IncludeOptions>,
    pub limits: Limits,
}

/// Where `adf:csv/PATH` includes may read from, and how much. The only file access the Markdown
/// can trigger; see "Including a CSV file" in `docs/syntax.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeOptions {
    /// Every path is relative to this directory and must stay inside it.
    pub root: PathBuf,
    /// At most this many includes per document.
    pub max_files: usize,
    /// At most this many bytes read from files per document.
    pub max_total_bytes: usize,
}

impl IncludeOptions {
    /// Includes per document, by default.
    pub const DEFAULT_MAX_FILES: usize = 16;
    /// Bytes read per document, by default (4 MiB).
    pub const DEFAULT_MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;

    /// Includes under `root` with the default budgets.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_files: Self::DEFAULT_MAX_FILES,
            max_total_bytes: Self::DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

/// The field delimiter of a CSV file: one ASCII character that is not a quote or a line break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvDelimiter(char);

impl CsvDelimiter {
    pub const COMMA: Self = Self(',');
    pub const TAB: Self = Self('\t');

    /// Parses `comma`, `tab`, `semicolon`, `pipe` or a single ASCII character other than `"`,
    /// a line break and control characters (a literal tab is accepted).
    pub fn parse(value: &str) -> Result<Self, String> {
        let c = match value {
            "comma" => ',',
            "tab" | "\\t" => '\t',
            "semicolon" => ';',
            "pipe" => '|',
            other => {
                let mut chars = other.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => c,
                    _ => {
                        return Err(format!(
                            "'{}' is not a delimiter; use comma, tab, semicolon, pipe or one character",
                            other.escape_debug().take(20).collect::<String>()
                        ));
                    }
                }
            }
        };
        if !c.is_ascii() || c == '"' || (c.is_ascii_control() && c != '\t') {
            return Err(
                "the delimiter must be one printable ASCII character (or a tab) other than a double quote"
                    .to_owned(),
            );
        }
        Ok(Self(c))
    }

    pub fn as_char(self) -> char {
        self.0
    }
}

impl Default for CsvDelimiter {
    fn default() -> Self {
        Self::COMMA
    }
}

/// Options for CSV → ADF (a table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvToAdfOptions {
    pub target: Target,
    pub delimiter: CsvDelimiter,
    /// The first row is the header. `false` writes `Column 1`, `Column 2`, … as the header.
    pub header: bool,
    /// Keep only this many data rows (`W_CSV_TRUNCATED` when rows are left out).
    pub max_rows: Option<usize>,
    /// Right-align columns whose cells are all numbers.
    pub align_numbers: bool,
    pub limits: Limits,
}

impl Default for CsvToAdfOptions {
    fn default() -> Self {
        Self {
            target: Target::default(),
            delimiter: CsvDelimiter::default(),
            header: true,
            max_rows: None,
            align_numbers: false,
            limits: Limits::default(),
        }
    }
}

/// Options for ADF → CSV (one table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdfToCsvOptions {
    pub delimiter: CsvDelimiter,
    /// Which table of the document, 1-based.
    pub table: usize,
    /// Prefix cells that a spreadsheet would run as a formula (starting with `=`, `+`, `-`, `@`,
    /// tab or carriage return) with a single quote.
    pub csv_safe: bool,
    pub limits: Limits,
}

impl Default for AdfToCsvOptions {
    fn default() -> Self {
        Self {
            delimiter: CsvDelimiter::default(),
            table: 1,
            csv_safe: false,
            limits: Limits::default(),
        }
    }
}

/// Markdown flavour produced by ADF → Markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MdFlavor {
    /// Lossless where possible: uses this tool's extension syntax so that converting back gives
    /// the same ADF; unsupported nodes become ```` ```adf ```` blocks (default).
    #[default]
    Roundtrip,
    /// Compact, readable Markdown for people and LLMs; drops what Markdown cannot express
    /// (no ```` ```adf ```` blocks, mentions as `@Name`, statuses as `[TEXT]`).
    Plain,
}

/// Options for ADF → Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdfToMdOptions {
    pub flavor: MdFlavor,
    pub limits: Limits,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extensions() {
        assert_eq!(Extensions::parse("all").unwrap(), Extensions::all());
        assert_eq!(Extensions::parse("none").unwrap(), Extensions::none());
        let e = Extensions::parse("alerts, raw-adf").unwrap();
        assert!(e.alerts && e.raw_adf && !e.details);
        assert!(Extensions::parse("alerts,bogus").is_err());
    }
}
