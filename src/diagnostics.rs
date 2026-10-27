//! Diagnostics: the single source of truth for every error and warning this tool reports.
//!
//! Codes are part of the public contract (`docs/cli-contract.md`): they are stable, documented,
//! and listed by `adf capabilities --json` and `adf explain codes`. Add new codes at the end
//! of their group; never rename or reuse one.

use std::fmt;

use serde::Serialize;

/// Error or warning. Errors stop the conversion; warnings describe a lossy or repaired construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

macro_rules! codes {
    ($( $variant:ident = $name:literal, $severity:ident, $exit:literal, $summary:literal; )*) => {
        /// Every diagnostic code. See `docs/cli-contract.md` for the meaning of each.
        #[allow(clippy::doc_markdown)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Code {
            $( #[doc = $summary] $variant, )*
        }

        impl Code {
            /// All codes, in documentation order.
            pub const ALL: &'static [Code] = &[$(Code::$variant,)*];

            /// The stable string form, e.g. `W_IMAGE_AS_LINK`.
            pub fn as_str(self) -> &'static str {
                match self { $( Code::$variant => $name, )* }
            }

            pub fn severity(self) -> Severity {
                match self { $( Code::$variant => Severity::$severity, )* }
            }

            /// Process exit code for errors; `0` for warnings.
            pub fn exit_code(self) -> u8 {
                match self { $( Code::$variant => $exit, )* }
            }

            /// One-line description used in docs, `--help` and `capabilities`.
            pub fn summary(self) -> &'static str {
                match self { $( Code::$variant => $summary, )* }
            }

            /// Parses the string form.
            pub fn parse(name: &str) -> Option<Code> {
                match name { $( $name => Some(Code::$variant), )* _ => None }
            }
        }
    };
}

codes! {
    // --- errors -------------------------------------------------------------------------------
    Internal          = "E_INTERNAL",          Error, 1, "Internal error (a bug in adf); please report it with the input that caused it.";
    Usage             = "E_USAGE",             Error, 2, "Invalid command-line usage.";
    InputRead         = "E_INPUT_READ",        Error, 3, "The input file or stdin could not be read.";
    InputEncoding     = "E_INPUT_ENCODING",    Error, 3, "The input is not valid UTF-8.";
    InputTooLarge     = "E_INPUT_TOO_LARGE",   Error, 3, "The input is larger than the size limit.";
    InputTooDeep      = "E_INPUT_TOO_DEEP",    Error, 3, "The input nests deeper than the nesting limit.";
    InputJson         = "E_INPUT_JSON",        Error, 3, "The input is not valid JSON.";
    InputNotAdf       = "E_INPUT_NOT_ADF",     Error, 3, "The JSON input is not an ADF document or a known envelope around one.";
    RawAdf            = "E_RAW_ADF",           Error, 3, "An ```adf block is not valid JSON, or not valid ADF where it is placed.";
    OutputWrite       = "E_OUTPUT_WRITE",      Error, 3, "The output could not be written.";
    Schema            = "E_SCHEMA",            Error, 4, "The ADF document does not match the ADF JSON schema.";
    Target            = "E_TARGET",            Error, 4, "The ADF document uses a node, mark or attribute the target product does not accept.";
    Invariant         = "E_INVARIANT",         Error, 4, "The ADF document breaks a rule Jira/Confluence enforce beyond the JSON schema.";
    Lossy             = "E_LOSSY",             Error, 5, "The conversion would lose information and --strict (or --html error) is set.";
    // --- warnings: Markdown -> ADF ------------------------------------------------------------
    MarkDropped       = "W_MARK_DROPPED",       Warning, 0, "A text mark was dropped because ADF does not allow it there (for example bold on inline code, or a subscript around a superscript), or an image title, or a link around an image, was dropped (--images external, or an adf:media image).";
    HeadingDemoted    = "W_HEADING_DEMOTED",    Warning, 0, "A heading inside a list item or quote became a bold paragraph.";
    BlockHoisted      = "W_BLOCK_HOISTED",      Warning, 0, "A block was moved out of a container that cannot hold it (for example a table inside a list).";
    BlockFlattened    = "W_BLOCK_FLATTENED",    Warning, 0, "A nested container was merged into its parent (for example a quote inside a quote).";
    HtmlAsText        = "W_HTML_AS_TEXT",       Warning, 0, "Raw HTML without an ADF equivalent was kept as literal text.";
    HtmlDropped       = "W_HTML_DROPPED",       Warning, 0, "Raw HTML was dropped (--html drop), or an unrecognised tag was removed from an expand title.";
    ImageAsLink       = "W_IMAGE_AS_LINK",      Warning, 0, "An image became a link; ADF images need media uploaded to Jira/Confluence.";
    ImageDropped      = "W_IMAGE_DROPPED",      Warning, 0, "An image was dropped (--images drop), including an adf:media file image.";
    UnsafeUrl         = "W_UNSAFE_URL",         Warning, 0, "A link target is not an allowed URL (http, https, ftp, ftps, mailto, tel, /path, #anchor) and became plain text.";
    ControlChar       = "W_CONTROL_CHAR",       Warning, 0, "Control characters were removed from text.";
    TableCellDropped  = "W_TABLE_CELL_DROPPED", Warning, 0, "Table cells beyond the header row's width were dropped.";
    FootnoteFlattened = "W_FOOTNOTE_FLATTENED", Warning, 0, "Footnotes became superscript numbers plus a numbered list at the end; ADF has no footnotes.";
    TaskNumberingLost = "W_TASK_NUMBERING_LOST", Warning, 0, "An ordered task list lost its numbering; ADF task lists are unordered.";
    FrontMatterDropped = "W_FRONT_MATTER_DROPPED", Warning, 0, "Leading YAML front matter was removed.";
    UnknownExtension  = "W_UNKNOWN_EXTENSION",  Warning, 0, "An adf: link names an unknown extension and was kept as plain text.";
    InvalidExtensionArg = "W_INVALID_EXTENSION_ARG", Warning, 0, "An adf: extension has an invalid argument and was kept as plain text.";
    ExtensionDisabled = "W_EXTENSION_DISABLED", Warning, 0, "Extension syntax was found but the extension is disabled (--extensions), so it was converted as plain Markdown.";
    NodeNotInTarget   = "W_NODE_NOT_IN_TARGET", Warning, 0, "A construct is not supported by the target product and was replaced by a fallback.";
    SizeLimit         = "W_SIZE_LIMIT",         Warning, 0, "The output is larger than Jira's 32,767-character field limit and may be rejected.";
    EmptyDocument     = "W_EMPTY_DOCUMENT",     Warning, 0, "The document is empty; Jira rejects empty comment bodies.";
    // --- warnings: ADF -> Markdown ------------------------------------------------------------
    AdfPassthrough    = "W_ADF_PASSTHROUGH",    Warning, 0, "An ADF node without a Markdown equivalent was written as a raw ```adf block.";
    AttrDropped       = "W_ATTR_DROPPED",       Warning, 0, "ADF attributes without a Markdown equivalent were dropped (for example column widths or cell colours).";
    InputSchema       = "W_INPUT_SCHEMA",       Warning, 0, "The input ADF does not match the schema; it was converted anyway.";
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Code {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Where a diagnostic applies. All fields are optional; renderers print what is present.
///
/// - Markdown input: `line`/`column` (1-based; columns count Unicode scalar values) and
///   optionally `end_line`/`end_column`.
/// - Invalid UTF-8: `byte_offset`.
/// - ADF input or output: `pointer` (RFC 6901 JSON Pointer, e.g. `/content/0/content/1`).
///
/// The CLI adds the file name when rendering.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Location {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

impl Location {
    pub fn pointer(pointer: impl Into<String>) -> Self {
        Self {
            pointer: Some(pointer.into()),
            ..Self::default()
        }
    }

    pub fn byte_offset(offset: usize) -> Self {
        Self {
            byte_offset: Some(offset),
            ..Self::default()
        }
    }
}

/// One error or warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: Code,
    /// Human-readable, one sentence, no trailing newline. Must not echo large chunks of input.
    pub message: String,
    pub location: Option<Location>,
    /// What to do about it, if there is something useful to say.
    pub hint: Option<String>,
}

impl Diagnostic {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            location: None,
            hint: None,
        }
    }

    #[must_use]
    pub fn at(mut self, location: Location) -> Self {
        self.location = Some(location);
        self
    }

    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn severity(&self) -> Severity {
        self.code.severity()
    }

    pub fn is_error(&self) -> bool {
        self.severity() == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// The hint of every "N more … were not listed" summary; [`omitted_count`] recognises summaries
/// by it.
const OMITTED_HINT: &str =
    "Only the first diagnostics of each code are listed; fix them and run again to see the rest";

/// The summary that stands for `count` diagnostics of `code` that were not listed: same code
/// and severity, no location. See [`cap_per_code`].
pub fn omitted(code: Code, count: usize) -> Diagnostic {
    Diagnostic::new(code, omitted_message(code, count)).with_hint(OMITTED_HINT)
}

fn omitted_message(code: Code, count: usize) -> String {
    let noun = match (code.severity(), count) {
        (Severity::Error, 1) => "error was",
        (Severity::Error, _) => "errors were",
        (Severity::Warning, 1) => "warning was",
        (Severity::Warning, _) => "warnings were",
    };
    format!("{count} more {code} {noun} not listed")
}

/// How many diagnostics `diagnostic` stands for if it is an [`omitted`] summary.
pub fn omitted_count(diagnostic: &Diagnostic) -> Option<usize> {
    if diagnostic.location.is_some() || diagnostic.hint.as_deref() != Some(OMITTED_HINT) {
        return None;
    }
    let (count, _) = diagnostic.message.split_once(' ')?;
    let count: usize = count.parse().ok()?;
    (diagnostic.message == omitted_message(diagnostic.code, count)).then_some(count)
}

/// How many problems `diagnostics` report: a summary counts for the diagnostics it stands for.
pub fn reported_count<'a>(diagnostics: impl IntoIterator<Item = &'a Diagnostic>) -> usize {
    diagnostics
        .into_iter()
        .map(|diagnostic| omitted_count(diagnostic).unwrap_or(1))
        .fold(0, usize::saturating_add)
}

/// Keeps at most `limit` diagnostics of each code, in order. The others of a code are replaced by
/// one [`omitted`] summary right after the last one kept; summaries already in the list are merged
/// into it. The first error is always kept, so the exit code does not change.
pub fn cap_per_code(diagnostics: Vec<Diagnostic>, limit: usize) -> Vec<Diagnostic> {
    use std::collections::BTreeMap;
    // Pass 1: per code, how many problems are reported in total (summaries count for what they
    // stand for) and how many diagnostics are listed (not summaries).
    let mut totals: BTreeMap<Code, (usize, usize)> = BTreeMap::new();
    for diagnostic in &diagnostics {
        let entry = totals.entry(diagnostic.code).or_default();
        let summarised = omitted_count(diagnostic);
        entry.0 = entry.0.saturating_add(summarised.unwrap_or(1));
        if summarised.is_none() {
            entry.1 = entry.1.saturating_add(1);
        }
    }
    let needs_summary = |code: Code| {
        totals
            .get(&code)
            .is_some_and(|&(total, listed)| total > listed.min(limit))
    };
    if !totals.keys().any(|&code| needs_summary(code)) {
        return diagnostics;
    }
    // Pass 2: keep the first `limit` of each code; the summary follows the last one kept.
    let mut kept: BTreeMap<Code, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for diagnostic in diagnostics {
        if omitted_count(&diagnostic).is_some() {
            continue;
        }
        let code = diagnostic.code;
        let count = kept.entry(code).or_default();
        if *count >= limit {
            continue;
        }
        *count += 1;
        let (total, listed) = totals.get(&code).copied().unwrap_or_default();
        let last_kept = *count == listed.min(limit);
        out.push(diagnostic);
        if last_kept && total > *count {
            out.push(omitted(code, total - *count));
        }
    }
    // Codes that only had summaries (nothing listed) keep one summary, at the end.
    for (&code, &(total, listed)) in &totals {
        if listed == 0 && total > 0 {
            out.push(omitted(code, total));
        }
    }
    out
}

/// A list of diagnostics returned when an operation fails. Contains at least one error, plus any
/// warnings collected before the failure.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn single(diagnostic: Diagnostic) -> Self {
        Self(vec![diagnostic])
    }

    pub fn has_errors(&self) -> bool {
        self.0.iter().any(Diagnostic::is_error)
    }

    /// The exit code of the first error, or `0` if there is none.
    pub fn exit_code(&self) -> u8 {
        self.0
            .iter()
            .find(|d| d.is_error())
            .map_or(0, |d| d.code.exit_code())
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a Diagnostics {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl From<Diagnostic> for Diagnostics {
    fn from(diagnostic: Diagnostic) -> Self {
        Self::single(diagnostic)
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

/// Maps byte offsets in a source text to 1-based line and column numbers.
#[derive(Debug, Clone)]
pub struct LineIndex<'a> {
    source: &'a str,
    /// Byte offset of the start of each line.
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
        Self { source, starts }
    }

    /// 1-based `(line, column)`; the column counts Unicode scalar values. Offsets past the end
    /// clamp to the end; offsets inside a UTF-8 sequence round down to its start.
    pub fn line_col(&self, offset: usize) -> (u32, u32) {
        let mut offset = offset.min(self.source.len());
        while !self.source.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self
            .starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let start = self.starts.get(line).copied().unwrap_or(0);
        let column = self
            .source
            .get(start..offset)
            .map_or(0, |s| s.chars().count());
        (to_u32(line + 1), to_u32(column + 1))
    }

    /// A location covering `range` (byte offsets).
    pub fn location(&self, range: std::ops::Range<usize>) -> Location {
        let (line, column) = self.line_col(range.start);
        let (end_line, end_column) = self.line_col(range.end);
        Location {
            line: Some(line),
            column: Some(column),
            end_line: Some(end_line),
            end_column: Some(end_column),
            ..Location::default()
        }
    }
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn located(code: Code, index: usize) -> Diagnostic {
        Diagnostic::new(code, format!("problem {index}"))
            .at(Location::pointer(format!("/content/{index}")))
    }

    fn shape(diagnostics: &[Diagnostic]) -> Vec<String> {
        diagnostics
            .iter()
            .map(|d| match omitted_count(d) {
                Some(count) => format!("{} +{count}", d.code),
                None => format!("{} {}", d.code, d.message),
            })
            .collect()
    }

    #[test]
    fn summaries_are_recognised() {
        let summary = omitted(Code::Schema, 3);
        assert_eq!(summary.message, "3 more E_SCHEMA errors were not listed");
        assert_eq!(omitted_count(&summary), Some(3));
        assert_eq!(
            omitted(Code::HtmlAsText, 1).message,
            "1 more W_HTML_AS_TEXT warning was not listed"
        );
        let mut located = summary.clone();
        located.location = Some(Location::pointer("/content"));
        assert_eq!(omitted_count(&located), None);
        assert_eq!(
            omitted_count(&Diagnostic::new(
                Code::Schema,
                "3 more E_SCHEMA errors were not listed"
            )),
            None
        );
        assert_eq!(
            reported_count(&[located, summary, omitted(Code::Schema, 2)]),
            6
        );
    }

    #[test]
    fn cap_keeps_the_first_of_each_code_and_summarises_the_rest() {
        let mut list: Vec<Diagnostic> = (0..5).map(|i| located(Code::HtmlAsText, i)).collect();
        list.insert(1, located(Code::ImageAsLink, 9));
        list.push(Diagnostic::new(Code::Lossy, "strict"));
        assert_eq!(
            shape(&cap_per_code(list.clone(), 2)),
            [
                "W_HTML_AS_TEXT problem 0",
                "W_IMAGE_AS_LINK problem 9",
                "W_HTML_AS_TEXT problem 1",
                "W_HTML_AS_TEXT +3",
                "E_LOSSY strict",
            ]
        );
        // Nothing to cap: unchanged.
        assert_eq!(cap_per_code(list.clone(), 5), list);
    }

    #[test]
    fn cap_merges_existing_summaries() {
        let mut list: Vec<Diagnostic> = (0..3).map(|i| located(Code::Schema, i)).collect();
        list.push(omitted(Code::Schema, 10));
        list.push(omitted(Code::Invariant, 4));
        let capped = cap_per_code(list.clone(), 3);
        assert_eq!(
            shape(&capped),
            [
                "E_SCHEMA problem 0",
                "E_SCHEMA problem 1",
                "E_SCHEMA problem 2",
                "E_SCHEMA +10",
                "E_INVARIANT +4",
            ]
        );
        // Capping again changes nothing, and a lower limit folds more into the summary.
        assert_eq!(cap_per_code(capped.clone(), 3), capped);
        assert_eq!(
            shape(&cap_per_code(list, 1)),
            ["E_SCHEMA problem 0", "E_SCHEMA +12", "E_INVARIANT +4"]
        );
    }

    #[test]
    fn codes_round_trip_and_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for &code in Code::ALL {
            assert!(seen.insert(code.as_str()), "duplicate {code}");
            assert_eq!(Code::parse(code.as_str()), Some(code));
            let prefix = if code.severity() == Severity::Error {
                "E_"
            } else {
                "W_"
            };
            assert!(code.as_str().starts_with(prefix), "{code}");
            assert_eq!(
                code.exit_code() == 0,
                code.severity() == Severity::Warning,
                "{code}"
            );
        }
    }

    #[test]
    fn line_index() {
        let idx = LineIndex::new("ab\ncé\n\nx");
        assert_eq!(idx.line_col(0), (1, 1));
        assert_eq!(idx.line_col(2), (1, 3));
        assert_eq!(idx.line_col(3), (2, 1));
        assert_eq!(idx.line_col(5), (2, 2)); // inside 'é' rounds down
        assert_eq!(idx.line_col(6), (2, 3));
        assert_eq!(idx.line_col(8), (4, 1));
        assert_eq!(idx.line_col(999), (4, 2));
    }
}
