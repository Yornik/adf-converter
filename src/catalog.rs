//! The feature catalog behind `adf capabilities` and `adf examples`.
//!
//! One entry per construct in `docs/syntax.md`: its kind (CommonMark, GFM or an ADF extension), the
//! Markdown syntax, a small example, and the ADF node and mark types it produces. `capabilities`
//! also lists the envelopes, targets, exit codes, every diagnostic code ([`Code::ALL`]) and the
//! limits, so agents can discover what this build supports instead of guessing.

use serde::Serialize;

use crate::adf::envelope::{ENVELOPE_NAMES, Envelope};
use crate::diagnostics::{Code, Severity};
use crate::limits::{
    DEFAULT_MAX_DEPTH, DEFAULT_MAX_INPUT_BYTES, DEFAULT_MAX_MARKDOWN_BYTES, JIRA_FIELD_CHAR_LIMIT,
    JSON_LEVELS_PER_CONTAINER, MAX_EMPHASIS_DELIMITERS, MAX_OUTPUT_BYTES, MAX_TABLE_CELLS,
};
use crate::options::Extensions;

/// Where a construct comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FeatureKind {
    /// CommonMark 0.31.
    CommonMark,
    /// GitHub Flavored Markdown (and GitHub's footnotes).
    Gfm,
    /// An ADF extension of this tool (`--extensions`).
    Extension,
    /// CSV or TSV input (`--from csv`); the example is CSV text, not Markdown.
    Csv,
}

impl FeatureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CommonMark => "commonmark",
            Self::Gfm => "gfm",
            Self::Extension => "extension",
            Self::Csv => "csv",
        }
    }
}

/// One supported Markdown construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Feature {
    /// Stable name, used by `adf examples NAME`.
    pub name: &'static str,
    pub kind: FeatureKind,
    /// The extension name (one of [`Extensions::NAMES`]) for [`FeatureKind::Extension`].
    pub extension: Option<&'static str>,
    /// Short syntax summary.
    pub syntax: &'static str,
    /// A small, complete example: Markdown, or CSV text for [`FeatureKind::Csv`].
    pub example: &'static str,
    /// ADF node and mark types the construct produces.
    pub adf: &'static [&'static str],
    /// A note on lossy or optional behaviour, if any.
    pub note: Option<&'static str>,
}

const fn feature(
    name: &'static str,
    kind: FeatureKind,
    syntax: &'static str,
    example: &'static str,
    adf: &'static [&'static str],
) -> Feature {
    Feature {
        name,
        kind,
        extension: None,
        syntax,
        example,
        adf,
        note: None,
    }
}

const fn extension(
    name: &'static str,
    syntax: &'static str,
    example: &'static str,
    adf: &'static [&'static str],
) -> Feature {
    Feature {
        name,
        kind: FeatureKind::Extension,
        extension: Some(name),
        syntax,
        example,
        adf,
        note: None,
    }
}

const fn with_note(mut feature: Feature, note: &'static str) -> Feature {
    feature.note = Some(note);
    feature
}

use FeatureKind::{CommonMark, Csv, Gfm};

/// Every construct of `docs/syntax.md`, in documentation order.
pub const FEATURES: &[Feature] = &[
    feature(
        "headings",
        CommonMark,
        "# H1 … ###### H6, setext === / ---",
        "# Release notes\n\n## Fixes\n",
        &["heading"],
    ),
    feature(
        "paragraphs",
        CommonMark,
        "text separated by blank lines; a single newline is a space",
        "First paragraph,\nstill the first.\n\nSecond paragraph.\n",
        &["paragraph", "text"],
    ),
    feature(
        "hard-breaks",
        CommonMark,
        "two trailing spaces, \\ at line end, or <br>",
        "first line\\\nsecond line\n",
        &["hardBreak"],
    ),
    feature(
        "emphasis",
        CommonMark,
        "*em* _em_ **strong** __strong__ ***both***",
        "Some *emphasis*, some **strong** text and ***both***.\n",
        &["em", "strong"],
    ),
    with_note(
        feature(
            "inline-code",
            CommonMark,
            "`code` (any number of backticks)",
            "Run `adf --help` to list the commands.\n",
            &["code"],
        ),
        "code only combines with link; other marks around it are dropped (W_MARK_DROPPED)",
    ),
    with_note(
        feature(
            "links",
            CommonMark,
            "[text](https://example.com \"title\"), reference links, <https://…>",
            "See [the docs](https://example.com/docs \"Docs\") or <https://example.com>.\n",
            &["link"],
        ),
        "only http(s), ftp(s), mailto:, tel:, /paths and #anchors become links (W_UNSAFE_URL)",
    ),
    with_note(
        feature(
            "images",
            CommonMark,
            "![alt](https://example.com/a.png)",
            "![Architecture diagram](https://example.com/diagram.png)\n",
            &["link", "mediaSingle", "media"],
        ),
        "a link by default (W_IMAGE_AS_LINK); --images external embeds by URL",
    ),
    feature(
        "bullet-lists",
        CommonMark,
        "- item, * item, + item; nest by indenting",
        "- one\n- two\n  - nested\n",
        &["bulletList", "listItem"],
    ),
    feature(
        "ordered-lists",
        CommonMark,
        "1. item, 1) item; the first number is kept",
        "3. three\n4. four\n",
        &["orderedList", "listItem"],
    ),
    feature(
        "code-blocks",
        CommonMark,
        "```lang fenced, or indented by four spaces",
        "```rust\nfn main() {}\n```\n",
        &["codeBlock"],
    ),
    feature(
        "blockquotes",
        CommonMark,
        "> quote",
        "> Quoted text.\n",
        &["blockquote"],
    ),
    feature(
        "rules",
        CommonMark,
        "--- *** ___",
        "Above\n\n---\n\nBelow\n",
        &["rule"],
    ),
    feature(
        "inline-html",
        CommonMark,
        "<u> <sub> <sup> <s> <kbd> <br> <span style=\"color: #RRGGBB\">",
        "H<sub>2</sub>O, x<sup>2</sup>, <u>underlined</u>, <span style=\"color: #FF5630\">red</span>\n",
        &[
            "underline",
            "subsup",
            "strike",
            "textColor",
            "backgroundColor",
        ],
    ),
    feature(
        "escapes",
        CommonMark,
        "\\* and entities like &amp; &copy;",
        "\\*not emphasis\\* &amp; &copy;\n",
        &["text"],
    ),
    feature(
        "tables",
        Gfm,
        "| a | b |",
        "| Name | Value |\n|:-----|------:|\n| a    | 1     |\n",
        &[
            "table",
            "tableRow",
            "tableHeader",
            "tableCell",
            "paragraph",
            "alignment",
        ],
    ),
    feature(
        "task-lists",
        Gfm,
        "- [ ] task, - [x] done",
        "- [ ] write the docs\n- [x] ship it\n",
        &["taskList", "taskItem"],
    ),
    feature(
        "strikethrough",
        Gfm,
        "~~strike~~ ~strike~",
        "~~obsolete~~ text\n",
        &["strike"],
    ),
    feature(
        "autolinks",
        Gfm,
        "bare https://example.com and www.example.com",
        "Visit https://example.com or www.example.com.\n",
        &["link"],
    ),
    with_note(
        feature(
            "footnotes",
            Gfm,
            "text[^1] … [^1]: note",
            "A claim[^1].\n\n[^1]: The source.\n",
            &["subsup", "rule", "orderedList"],
        ),
        "ADF has no footnotes: superscript numbers plus a list at the end (W_FOOTNOTE_FLATTENED)",
    ),
    extension(
        "alerts",
        "> [!NOTE] / [!TIP] / [!IMPORTANT] / [!WARNING] / [!CAUTION]",
        "> [!WARNING]\n> Back up the database first.\n",
        &["panel"],
    ),
    extension(
        "details",
        "<details><summary>Title</summary> … </details>",
        "<details>\n<summary>Logs</summary>\n\nMarkdown content.\n\n</details>\n",
        &["expand", "nestedExpand"],
    ),
    extension(
        "cells",
        "::: cell … :::",
        "::: cell\nA framed **box**.\n:::\n",
        &["table", "tableRow", "tableCell"],
    ),
    extension(
        "raw-adf",
        "```adf with a block node or an array of block nodes",
        "```adf\n{\"type\": \"panel\", \"attrs\": {\"panelType\": \"success\"}, \"content\": [{\"type\": \"paragraph\"}]}\n```\n",
        &[],
    ),
    extension(
        "mention",
        "[@Display Name](adf:mention/ACCOUNT_ID)",
        "Thanks [@Jane Doe](adf:mention/5b10ac8d82e05b22cc7d4ef5)!\n",
        &["mention"],
    ),
    extension(
        "status",
        "[TEXT](adf:status/neutral|purple|blue|red|yellow|green)",
        "Build: [DONE](adf:status/green)\n",
        &["status"],
    ),
    extension(
        "date",
        "[YYYY-MM-DD](adf:date)",
        "Due [2026-10-19](adf:date)\n",
        &["date"],
    ),
    extension(
        "emoji",
        "[😄](adf:emoji/:smile:)",
        "Nice work [😄](adf:emoji/:smile:)\n",
        &["emoji"],
    ),
    extension(
        "card",
        "[https://…](adf:card)",
        "[https://example.atlassian.net/browse/PROJ-1](adf:card)\n",
        &["inlineCard"],
    ),
    with_note(
        extension(
            "media",
            "![alt](adf:media/UUID?collection=&width=&height=&layout=&size=)",
            "![shot.png](adf:media/11111111-2222-3333-4444-555555555555?collection=contentId-123456&width=1512&height=945&layout=wide&size=720px)\n",
            &["mediaSingle", "media"],
        ),
        "references a file already uploaded to Jira or Confluence; adf never uploads. --images drop removes it (W_IMAGE_DROPPED); a bad argument keeps the alt text (W_INVALID_EXTENSION_ARG)",
    ),
    // The link form of the `media` extension: attached files by name. Only this item belongs to
    // the link form; the image form has its own entry, which comes first.
    with_note(
        Feature {
            name: "attachment",
            kind: FeatureKind::Extension,
            extension: Some("media"),
            syntax: "[file name](adf:attachment/UUID?collection=)",
            example: "[report.pdf](adf:attachment/66666666-7777-8888-9999-000000000000) [trace.zip](adf:attachment/66666666-7777-8888-9999-000000000002?collection=contentId-123456)\n\nSee [log.txt](adf:attachment/66666666-7777-8888-9999-000000000001) for details.\n",
            adf: &["mediaGroup", "media", "mediaInline"],
            note: None,
        },
        "references files already attached to a Jira issue or Confluence page; adf never uploads. A paragraph of only attachment links is a mediaGroup (not in a list item), a link amid text is a mediaInline. The link text is the file name. --images drop removes the links (W_IMAGE_DROPPED); a bad argument keeps the link text (W_INVALID_EXTENSION_ARG); the image form ![x](adf:attachment/UUID) is not an attachment (W_UNKNOWN_EXTENSION)",
    ),
    with_note(
        extension(
            "csv",
            "```csv or ```tsv fence, optionally followed by key=value options (delimiter, header, max-rows, align); the body is the CSV text",
            "```csv align=numbers\nItem,Qty,Notes\nWidget,3,\"Blue, small\"\nGadget,12,\n```\n",
            &[
                "table",
                "tableRow",
                "tableHeader",
                "tableCell",
                "paragraph",
                "hardBreak",
                "link",
                "alignment",
            ],
        ),
        "one-way: ADF to Markdown writes a GFM table. A table in a list item or quote is hoisted like a GFM table (W_BLOCK_HOISTED); a bad option is E_CSV_OPTION, a bad body E_INPUT_CSV; --extensions without csv keeps the fence a code block (W_EXTENSION_DISABLED); to show CSV as code use another language word such as text",
    ),
    // The include form of the `csv` extension (issue #13, phase 3). Only this item belongs to it;
    // the fence form of the same extension has its own entry, which comes first.
    with_note(
        Feature {
            name: "csv-include",
            kind: FeatureKind::Extension,
            extension: Some("csv"),
            syntax: "![alt](adf:csv/PATH?delimiter=&header=&max-rows=&align=) with --include-root DIR",
            example: "![Inventory](adf:csv/inventory.csv)\n",
            adf: &[
                "table",
                "tableRow",
                "tableHeader",
                "tableCell",
                "paragraph",
                "hardBreak",
                "link",
                "alignment",
            ],
            note: None,
        },
        "off unless --include-root DIR is given (the example below shows the disabled form: nothing is read, W_INCLUDE_DISABLED); PATH is relative to DIR, uses / and is %-decoded once; absolute paths, .., backslashes, ':' and symlinks out of DIR are refused (E_INCLUDE); the Markdown decides what is read, so DIR must hold only data you are willing to publish; at most 16 files, 1 MiB each and 4 MiB in all per document",
    ),
    with_note(
        feature(
            "csv-tables",
            Csv,
            "adf convert --from csv FILE (header row, --delimiter, --no-header, --max-rows, --align-numbers); --from adf --to csv writes one table back",
            "Name,Qty,Notes\nWidget,3,\"Blue, small\"\nGadget,12,https://example.com/gadget\n",
            &[
                "table",
                "tableRow",
                "tableHeader",
                "tableCell",
                "paragraph",
                "hardBreak",
                "link",
                "alignment",
            ],
        ),
        "cell text is never parsed as Markdown; bare http(s) URLs become links; --max-rows leaves rows out (W_CSV_TRUNCATED); --to csv repeats merged cells (W_CSV_MERGED_CELLS) and --csv-safe guards against spreadsheet formulas",
    ),
];

/// Looks a feature up by name (case-insensitive).
pub fn feature_by_name(name: &str) -> Option<&'static Feature> {
    FEATURES
        .iter()
        .find(|feature| feature.name.eq_ignore_ascii_case(name.trim()))
}

/// The process exit codes and their meaning (`docs/cli-contract.md#exit-codes`).
pub const EXIT_CODES: &[(u8, &str)] = &[
    (0, "ok (warnings may have been printed)"),
    (1, "internal error (a bug in adf)"),
    (2, "usage error"),
    (3, "input or I/O error"),
    (4, "validation failed"),
    (5, "lossy conversion refused (--strict or --html error)"),
];

/// One envelope as described by `capabilities`.
#[derive(Debug, Clone, Serialize)]
pub struct EnvelopeInfo {
    pub name: &'static str,
    /// The JSON shape, with `DOC` standing for the document.
    pub shape: &'static str,
    /// Where to send it.
    pub endpoint: &'static str,
    /// The target used by `-t auto`.
    pub default_target: &'static str,
    /// Whether `--field NAME` is required.
    pub requires_field: bool,
}

/// The envelopes of `-e/--envelope`, in documentation order.
pub fn envelopes() -> Vec<EnvelopeInfo> {
    ENVELOPE_NAMES
        .iter()
        .map(|&name| {
            let envelope = match name {
                "jira-comment" => Envelope::JiraComment,
                "jira-description" => Envelope::JiraDescription,
                "jira-field" => Envelope::JiraField(String::new()),
                "confluence" => Envelope::Confluence,
                _ => Envelope::Doc,
            };
            let (shape, endpoint) = match envelope {
                Envelope::Doc => (
                    "DOC",
                    "acli --body-adf / --description-file, or embedding in your own JSON",
                ),
                Envelope::JiraComment => (
                    "{\"body\":DOC}",
                    "POST /rest/api/3/issue/{key}/comment, PUT /rest/api/3/issue/{key}/comment/{id}",
                ),
                Envelope::JiraDescription => (
                    "{\"fields\":{\"description\":DOC}}",
                    "PUT /rest/api/3/issue/{key}",
                ),
                Envelope::JiraField(_) => (
                    "{\"fields\":{\"<NAME>\":DOC}}",
                    "PUT /rest/api/3/issue/{key} (rich-text custom fields; needs --field NAME)",
                ),
                Envelope::Confluence => (
                    "{\"representation\":\"atlas_doc_format\",\"value\":\"<DOC as a JSON string>\"}",
                    "body of POST /wiki/api/v2/pages and POST /wiki/api/v2/footer-comments",
                ),
            };
            EnvelopeInfo {
                name,
                shape,
                endpoint,
                default_target: envelope.default_target().as_str(),
                requires_field: matches!(envelope, Envelope::JiraField(_)),
            }
        })
        .collect()
}

/// Whether extension `name` is on in `extensions`.
pub fn extension_enabled(extensions: &Extensions, name: &str) -> bool {
    match name {
        "alerts" => extensions.alerts,
        "details" => extensions.details,
        "cells" => extensions.cells,
        "raw-adf" => extensions.raw_adf,
        "mention" => extensions.mention,
        "status" => extensions.status,
        "date" => extensions.date,
        "emoji" => extensions.emoji,
        "card" => extensions.card,
        "media" => extensions.media,
        "csv" => extensions.csv,
        _ => false,
    }
}

/// What to do about a diagnostic, for `adf explain CODE`.
#[allow(clippy::too_many_lines)] // one flat arm per diagnostic code
pub fn advice(code: Code) -> &'static str {
    match code {
        Code::Internal => {
            "This is a bug in adf. Please open an issue at https://github.com/Yornik/adf-converter/issues with the command and a minimal input."
        }
        Code::Usage => {
            "Fix the command line; `adf --help` and `adf COMMAND --help` list the options."
        }
        Code::InputRead => {
            "Check that the file exists and is readable, or pipe the input into stdin."
        }
        Code::InputEncoding => {
            "Convert the input to UTF-8 (for example `iconv -f latin1 -t utf-8`); the location gives the first invalid byte."
        }
        Code::InputTooLarge => {
            "Split the input into smaller documents (the limits are 1 MiB of Markdown or CSV and 4 MiB of ADF), split very large tables (at most 100,000 cells) or paragraphs, or shorten long link URLs."
        }
        Code::InputTooDeep => {
            "Flatten the nesting: Markdown may nest 64 containers (lists, quotes, cells, expands), ADF input 512 levels of JSON arrays and objects."
        }
        Code::InputJson => "Fix the JSON syntax at the reported line and column.",
        Code::InputNotAdf => {
            "Pass an ADF document or one of the envelopes from `adf explain envelopes`; for Jira issues pick the field with --field NAME."
        }
        Code::RawAdf => {
            "Fix the JSON in the ```adf block: it must be one block node, or an array of block nodes, that is valid where the block stands."
        }
        Code::OutputWrite => {
            "Check that the output directory exists and is writable, and that there is free space."
        }
        Code::Schema => {
            "Fix the node at the reported JSON Pointer; `adf schema adf` prints the schema."
        }
        Code::Target => {
            "Remove the node, mark or attribute, or pick the right product with -t jira|confluence."
        }
        Code::Invariant => {
            "Fix the node at the reported JSON Pointer; Jira and Confluence reject these documents even though the schema allows them."
        }
        Code::Lossy => {
            "Change the input so the warnings disappear, or drop --strict (or --html error) to accept the lossy result."
        }
        Code::MarkDropped => {
            "Move the formatting outside the code span or extension link, or accept the drop."
        }
        Code::HeadingDemoted => {
            "Move the heading out of the list item or quote if it should stay a heading."
        }
        Code::BlockHoisted => {
            "Move the block out of its container, or accept that it now follows the container."
        }
        Code::BlockFlattened => "Avoid nesting this container, or accept the flattened result.",
        Code::HtmlAsText => "Replace the HTML with Markdown, or use --html drop to remove it.",
        Code::HtmlDropped => "Replace the HTML with Markdown if the content matters.",
        Code::ImageAsLink => {
            "Upload the image to Jira/Confluence and reference it there, or use --images external to embed it by URL."
        }
        Code::ImageDropped => "Use --images link (the default) to keep a link to the image.",
        Code::UnsafeUrl => {
            "Use an http(s), ftp(s), mailto: or tel: URL, a /root-relative path or a #anchor."
        }
        Code::ControlChar => "Remove control characters from the input.",
        Code::TableCellDropped => "Give every row the same number of cells as the header row.",
        Code::FootnoteFlattened => {
            "Inline the footnote text if superscript numbers are not good enough."
        }
        Code::TaskNumberingLost => "Use - [ ] instead of 1. [ ] for task lists.",
        Code::FrontMatterDropped => "Remove the front matter from the input if it is not needed.",
        Code::UnknownExtension => {
            "Use one of the adf: link kinds mention, status, date, emoji, card or attachment ([file name](adf:attachment/<uuid>)), or the adf:media image form ![alt](adf:media/<uuid>) (`adf capabilities` lists them)."
        }
        Code::InvalidExtensionArg => {
            "Fix the argument of the adf: link, the adf:attachment link or the adf:media image; `adf examples NAME` shows the syntax."
        }
        Code::ExtensionDisabled => {
            "Enable the extension with --extensions, or write plain Markdown."
        }
        Code::NodeNotInTarget => "Pick the right product with -t, or avoid the construct.",
        Code::SizeLimit => "Split the content into several comments, or shorten it.",
        Code::EmptyDocument => "Add content; Jira rejects empty comment bodies.",
        Code::AdfPassthrough => {
            "Nothing to do for round trips; converting the Markdown back restores the node (this includes code blocks whose language is csv or tsv, which a csv fence would turn into a table). Use --md-flavor plain for readable text instead."
        }
        Code::AttrDropped => {
            "Nothing to do unless the attribute matters; Markdown cannot express it."
        }
        Code::InputSchema => {
            "The input ADF is not schema-valid; `adf validate` on the input shows the details."
        }
        Code::InputCsv => {
            "Close the quote that was opened at the reported line and column, and check that --delimiter matches the file; a CSV with only empty rows has nothing to convert."
        }
        Code::InputNoTable => {
            "The message says how many tables the document has; pass --table N with N in that range, or convert a document that holds a table."
        }
        Code::CsvOption => {
            "Fix the option named in the message. Allowed: delimiter (comma, tab, semicolon, pipe or one character), header (true or false), max-rows (a whole number of at least 1) and align (numbers or none), each at most once."
        }
        Code::Include => {
            "Use a relative path to a regular file inside the directory given with --include-root (no .., no absolute path, no symlink out of it), and keep within the file size, cell and per-document include budgets; the message says which rule failed."
        }
        Code::IncludeDisabled => {
            "Includes are off by default, because the Markdown decides which files are read. Only if you chose the include yourself, pass --include-root DIR for a directory that holds nothing but data you are willing to publish; otherwise convert the CSV with --from csv and paste the table."
        }
        Code::CsvTruncated => {
            "Raise or drop --max-rows, or attach the full file and say the table is a preview."
        }
        Code::CsvMergedCells => {
            "Expected for tables with merged cells; each covered cell repeats the merged cell's text."
        }
    }
}

/// One diagnostic code as described by `capabilities`.
#[derive(Debug, Clone, Serialize)]
pub struct CodeInfo {
    pub code: Code,
    pub severity: Severity,
    pub exit_code: u8,
    pub summary: &'static str,
}

/// Every diagnostic code, in documentation order.
pub fn codes() -> Vec<CodeInfo> {
    Code::ALL
        .iter()
        .map(|&code| CodeInfo {
            code,
            severity: code.severity(),
            exit_code: code.exit_code(),
            summary: code.summary(),
        })
        .collect()
}

/// A feature as described by `capabilities`.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureInfo {
    pub name: &'static str,
    pub kind: FeatureKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<&'static str>,
    pub syntax: &'static str,
    pub adf: &'static [&'static str],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
}

/// An extension as described by `capabilities`.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionInfo {
    pub name: &'static str,
    pub syntax: &'static str,
    pub adf: &'static [&'static str],
    /// Whether it is on without `--extensions`.
    pub default: bool,
}

/// The limits as described by `capabilities`.
#[derive(Debug, Clone, Serialize)]
pub struct LimitsInfo {
    /// Input size, for ADF (JSON) input.
    pub max_input_bytes: usize,
    /// Markdown and CSV input size.
    pub max_markdown_bytes: usize,
    /// `*` and `_` characters per paragraph of Markdown input.
    pub max_emphasis_delimiters: usize,
    /// Container levels in Markdown input.
    pub max_nesting_depth: usize,
    /// Levels of JSON arrays/objects in ADF input.
    pub max_json_depth: usize,
    /// Table cells in Markdown and CSV input, counting the cells short rows are padded with.
    pub max_table_cells: usize,
    /// Output size in either direction.
    pub max_output_bytes: usize,
    pub jira_field_char_limit: usize,
}

/// The `adf capabilities --json` document (`schemas/capabilities.v1.json`).
#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub schema_version: u32,
    pub version: &'static str,
    pub adf_schema: &'static str,
    pub directions: &'static [&'static str],
    pub envelopes: Vec<EnvelopeInfo>,
    pub targets: &'static [&'static str],
    pub features: Vec<FeatureInfo>,
    pub extensions: Vec<ExtensionInfo>,
    /// Exit code (as a string key) → meaning.
    pub exit_codes: serde_json::Map<String, serde_json::Value>,
    pub diagnostic_codes: Vec<CodeInfo>,
    pub limits: LimitsInfo,
}

/// The vendored schema version without the trailing newline of `schemas/adf/VERSION`.
pub const ADF_SCHEMA: &str = crate::ADF_SCHEMA_VERSION;

/// Builds the capabilities document for this build.
pub fn capabilities() -> Capabilities {
    let defaults = Extensions::default();
    Capabilities {
        schema_version: 1,
        version: env!("CARGO_PKG_VERSION"),
        adf_schema: ADF_SCHEMA,
        directions: &["md->adf", "adf->md", "csv->adf", "adf->csv"],
        envelopes: envelopes(),
        targets: &["jira", "confluence"],
        features: FEATURES
            .iter()
            .map(|feature| FeatureInfo {
                name: feature.name,
                kind: feature.kind,
                extension: feature.extension,
                syntax: feature.syntax,
                adf: feature.adf,
                note: feature.note,
            })
            .collect(),
        extensions: Extensions::NAMES
            .iter()
            .map(|&name| {
                let feature = FEATURES
                    .iter()
                    .find(|feature| feature.extension == Some(name));
                ExtensionInfo {
                    name,
                    syntax: feature.map_or("", |feature| feature.syntax),
                    adf: feature.map_or(&[], |feature| feature.adf),
                    default: extension_enabled(&defaults, name),
                }
            })
            .collect(),
        exit_codes: EXIT_CODES
            .iter()
            .map(|(code, meaning)| (code.to_string(), serde_json::Value::from(*meaning)))
            .collect(),
        diagnostic_codes: codes(),
        limits: LimitsInfo {
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_markdown_bytes: DEFAULT_MAX_MARKDOWN_BYTES,
            max_emphasis_delimiters: MAX_EMPHASIS_DELIMITERS,
            max_nesting_depth: DEFAULT_MAX_DEPTH,
            max_json_depth: DEFAULT_MAX_DEPTH * JSON_LEVELS_PER_CONTAINER,
            max_table_cells: MAX_TABLE_CELLS,
            max_output_bytes: MAX_OUTPUT_BYTES,
            jira_field_char_limit: JIRA_FIELD_CHAR_LIMIT,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_names_are_unique_kebab_case() {
        let mut seen = std::collections::BTreeSet::new();
        for feature in FEATURES {
            assert!(seen.insert(feature.name), "duplicate {}", feature.name);
            assert!(
                feature
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-'),
                "{}",
                feature.name
            );
            assert!(feature.example.ends_with('\n'), "{}", feature.name);
            assert_eq!(
                feature.kind == FeatureKind::Extension,
                feature.extension.is_some(),
                "{}",
                feature.name
            );
        }
    }

    #[test]
    fn every_extension_has_a_feature() {
        for name in Extensions::NAMES {
            assert!(
                FEATURES.iter().any(|f| f.extension == Some(*name)),
                "{name}"
            );
            assert!(extension_enabled(&Extensions::all(), name));
            assert!(!extension_enabled(&Extensions::none(), name));
        }
        for feature in FEATURES {
            if let Some(name) = feature.extension {
                assert!(Extensions::NAMES.contains(&name), "{name}");
            }
        }
    }

    #[test]
    fn capabilities_list_all_four_directions() {
        assert_eq!(
            capabilities().directions,
            ["md->adf", "adf->md", "csv->adf", "adf->csv"]
        );
        let csv: Vec<&str> = FEATURES
            .iter()
            .filter(|feature| feature.kind == FeatureKind::Csv)
            .map(|feature| feature.name)
            .collect();
        assert_eq!(csv, ["csv-tables"]);
        assert_eq!(FeatureKind::Csv.as_str(), "csv");
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert_eq!(feature_by_name("Tables").unwrap().name, "tables");
        assert!(feature_by_name("nope").is_none());
    }

    #[test]
    fn capabilities_cover_codes_and_envelopes() {
        let caps = capabilities();
        assert_eq!(caps.diagnostic_codes.len(), Code::ALL.len());
        assert_eq!(caps.envelopes.len(), ENVELOPE_NAMES.len());
        assert_eq!(caps.extensions.len(), Extensions::NAMES.len());
        assert_eq!(caps.adf_schema, "@atlaskit/adf-schema@57.7.18");
        assert_eq!(caps.exit_codes.len(), 6);
        let confluence = caps
            .envelopes
            .iter()
            .find(|e| e.name == "confluence")
            .unwrap();
        assert_eq!(confluence.default_target, "confluence");
        let field = caps
            .envelopes
            .iter()
            .find(|e| e.name == "jira-field")
            .unwrap();
        assert!(field.requires_field);
        let json = serde_json::to_value(&caps).unwrap();
        assert_eq!(json["limits"]["max_input_bytes"], 4_194_304);
        assert_eq!(json["exit_codes"]["4"], "validation failed");
    }

    #[test]
    fn advice_for_every_code() {
        for &code in Code::ALL {
            let text = advice(code);
            assert!(text.ends_with('.'), "{code}: {text}");
        }
    }
}
