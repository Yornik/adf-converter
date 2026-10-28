//! Property tests (proptest) for untrusted input:
//!
//! - `md_to_adf` never panics, and returns valid ADF or only documented errors;
//! - it is deterministic;
//! - its output size is bounded by `K * input + C`;
//! - every successful result satisfies the round-trip guarantee;
//! - `adf_to_md` never panics on arbitrary ADF-shaped JSON.
//!
//! Inputs come from a grammar-based Markdown generator (random nesting of lists, quotes, tables,
//! code, emphasis, links, alerts, details, box cells, raw ADF, `adf:` links, HTML, escapes and
//! Unicode) and from a "token soup" of Markdown-significant fragments. 256 cases per property by
//! default; set `PROPTEST_CASES` to change it. Failures are persisted to
//! `proptest-regressions/properties.txt`, which is committed.
//!
//! The deterministic tests at the end cover pathological inputs and limits.

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::time::{Duration, Instant};

use adf_converter::limits::{DEFAULT_MAX_MARKDOWN_BYTES, MAX_TABLE_CELLS};
use adf_converter::{
    AdfToMdOptions, Code, Diagnostics, Extensions, HtmlPolicy, ImagePolicy, Limits, MdFlavor,
    MdToAdfOptions, SoftBreak, Target,
};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use serde_json::{Value, json};

/// Output bound `K * input bytes + C` (compact JSON). The largest legitimate expansion per input
/// byte is an empty, aligned table cell (`|` → about 110 bytes of JSON); `K` leaves room for
/// that. GFM row padding is the only super-linear construct (see the padded-table test).
const SIZE_K: usize = 256;
const SIZE_C: usize = 8192;

fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "proptest-regressions/properties.txt",
        ))),
        ..ProptestConfig::default()
    }
}

// ---------------------------------------------------------------------------------------------
// Markdown grammar
// ---------------------------------------------------------------------------------------------

const PUNCTUATION: &[&str] = &[
    "*", "_", "`", "~", "[", "]", "(", ")", "<", ">", "#", "|", "\\", "&", "!", ":", "@", "-", "+",
    ".", "\"", "'", "{", "}", "^", "=", "$", "%",
];
const UNICODE: &[&str] = &[
    "é",
    "ß",
    "漢字",
    "עברית",
    "😄",
    "👩\u{200d}💻",
    "✍\u{fe0f}",
    "→",
    "\u{a0}",
    "\u{200b}",
];
const URLS: &[&str] = &[
    "https://example.com",
    "http://example.org/a?b=1&c=2",
    "/browse/PROJ-1",
    "#anchor",
    "mailto:a@example.com",
    "tel:+3120",
    "ftp://files.example.com/x",
    "javascript:alert(1)",
    "data:text/html,x",
    "relative/path",
    "//protocol.relative",
    "",
    "<https://example.com/a b>",
    "https://example.com \"Title\"",
];
const ADF_LINKS: &[&str] = &[
    "[@Jane](adf:mention/abc123)",
    "[@](adf:mention/)",
    "[DONE](adf:status/green)",
    "[X](adf:status/orange)",
    "[](adf:status)",
    "[2026-10-19](adf:date)",
    "[19-10-2026](adf:date)",
    "[😄](adf:emoji/:smile:)",
    "[](adf:emoji/tada)",
    "[https://example.com/x](adf:card)",
    "[javascript:x](adf:card)",
    "[label](adf:unknown/thing)",
];
/// `adf:media` images: valid ones (every parameter, case variants, boundaries) and invalid ones
/// (every failure family), with and without alt text, titles and links.
const MEDIA_IMAGES: &[&str] = &[
    "![shot.png](adf:media/11111111-2222-3333-4444-555555555555)",
    "![](adf:media/11111111-2222-3333-4444-555555555555)",
    "![a *b*](ADF:Media/AAAAAAAA-bbbb-2222-CCCC-333344445555)",
    "![shot](adf:media/11111111-2222-3333-4444-555555555555?collection=contentId-123456&width=1512&height=945&layout=wide&size=720px)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?size=50%&layout=wrap-left)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?width=1&height=100000&size=100%)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?size=100000px&collection=)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555 \"Title\")",
    "[![s](adf:media/11111111-2222-3333-4444-555555555555)](https://example.com)",
    "![s](adf:media)",
    "![s](adf:media/)",
    "![s](adf:media/not-a-uuid)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?width=0)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?width=1&width=2)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?layout=left)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?size=101%)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?size=5)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?colour=red)",
    "![s](adf:media/11111111-2222-3333-4444-555555555555?collection=a%20b)",
    "![s\u{7}](adf:media/11111111-2222-3333-4444-555555555555?width=99999999999999999999)",
    "![s ![n](adf:media/11111111-2222-3333-4444-555555555555) t](https://example.com/i.png)",
];
/// `adf:attachment` links: valid ones (with and without a collection, case variants, file names
/// that need escaping) and invalid ones (every failure family).
const ATTACHMENT_LINKS: &[&str] = &[
    "[report.pdf](adf:attachment/66666666-7777-8888-9999-000000000000)",
    "[trace.zip](adf:attachment/66666666-7777-8888-9999-000000000002?collection=contentId-123456)",
    "[log.txt](ADF:Attachment/AAAAAAAA-bbbb-2222-CCCC-333344445555?collection=)",
    "[a \\[1\\] \\*b\\*.txt](adf:attachment/66666666-7777-8888-9999-000000000003 \"Title\")",
    "[**bold** `code` name](adf:attachment/66666666-7777-8888-9999-000000000004)",
    "**[bold.pdf](adf:attachment/66666666-7777-8888-9999-000000000005)**",
    "[ pad ](adf:attachment/66666666-7777-8888-9999-000000000006)",
    "[a\u{7}b](adf:attachment/66666666-7777-8888-9999-000000000007)",
    "[](adf:attachment/66666666-7777-8888-9999-000000000008)",
    "[ ](adf:attachment/66666666-7777-8888-9999-000000000008)",
    "[x](adf:attachment)",
    "[x](adf:attachment/)",
    "[x](adf:attachment/not-a-uuid)",
    "[x](adf:attachment/66666666-7777-8888-9999-000000000009?)",
    "[x](adf:attachment/66666666-7777-8888-9999-000000000009?collection=a%20b)",
    "[x](adf:attachment/66666666-7777-8888-9999-000000000009?width=1)",
    "[x](adf:attachment/66666666-7777-8888-9999-000000000009?collection=a&collection=b)",
    "[![i](https://example.com/i.png) x](adf:attachment/66666666-7777-8888-9999-000000000009)",
    "![x](adf:attachment/66666666-7777-8888-9999-000000000009)",
];
const INLINE_HTML: &[(&str, &str)] = &[
    ("<u>", "</u>"),
    ("<ins>", "</ins>"),
    ("<s>", "</s>"),
    ("<del>", "</del>"),
    ("<b>", "</b>"),
    ("<strong>", "</strong>"),
    ("<i>", "</i>"),
    ("<em>", "</em>"),
    ("<sub>", "</sub>"),
    ("<sup>", "</sup>"),
    ("<kbd>", "</kbd>"),
    ("<span style=\"color: #ff5630\">", "</span>"),
    ("<span style=\"background-color: #e3fcef\">", "</span>"),
    ("<span style=\"color: red\">", "</span>"),
    ("<abbr>", "</abbr>"),
    ("<u>", ""),
];
const INLINE_LEAVES: &[&str] = &[
    "<br>",
    "<!-- comment -->",
    "&amp;",
    "&#32;",
    "&#10;",
    "&copy;",
    "&bogus;",
    "\\*",
    "\\_",
    "\\`",
    "\\\\",
    "https\\://example.com",
    "https://example.com/bare",
    "www.example.com",
    "<https://example.com/auto>",
    "<a@example.com>",
    "![alt](https://example.com/i.png)",
    "![](https://example.com/i.png)",
    "[^1]",
    "[^note]",
    "\u{7}",
    "\t",
];
const RAW_ADF: &[&str] = &[
    r#"{"type":"panel","attrs":{"panelType":"success"},"content":[{"type":"paragraph"}]}"#,
    r#"[{"type":"rule"},{"type":"paragraph","content":[{"type":"text","text":"raw"}]}]"#,
    r#"{"type":"codeBlock","attrs":{"language":"text"},"content":[{"type":"text","text":"x"}]}"#,
    r#"{"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"code"},{"type":"strong"}]}]}"#,
    r#"{"type":"bogus"}"#,
    "{not json",
];

fn pick(items: &'static [&'static str]) -> impl Strategy<Value = String> {
    prop::sample::select(items).prop_map(str::to_owned)
}

fn word() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => "[a-zA-Z0-9]{1,8}",
        2 => pick(PUNCTUATION),
        1 => pick(UNICODE),
    ]
}

fn inline_leaf() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => word(),
        2 => "`[a-z `*_]{0,6}`",
        1 => "``[a-z`]{1,4}``",
        2 => pick(INLINE_LEAVES),
        2 => pick(ADF_LINKS),
        2 => pick(MEDIA_IMAGES),
        2 => pick(ATTACHMENT_LINKS),
    ]
}

fn inline() -> impl Strategy<Value = String> {
    inline_leaf().prop_recursive(3, 24, 4, |inner| {
        let seq = prop::collection::vec(inner, 1..4).prop_map(|v| v.join(" "));
        prop_oneof![
            seq.clone().prop_map(|s| format!("*{s}*")),
            seq.clone().prop_map(|s| format!("_{s}_")),
            seq.clone().prop_map(|s| format!("**{s}**")),
            seq.clone().prop_map(|s| format!("***{s}***")),
            seq.clone().prop_map(|s| format!("~~{s}~~")),
            (seq.clone(), pick(URLS)).prop_map(|(s, u)| format!("[{s}]({u})")),
            (seq.clone(), prop::sample::select(INLINE_HTML))
                .prop_map(|(s, (open, close))| format!("{open}{s}{close}")),
            seq.clone()
                .prop_map(|s| format!("{s}\\\nafter a hard break")),
            seq,
        ]
    })
}

/// One line of inline content (never empty, never starting with block syntax by accident more
/// often than the generator intends).
fn line() -> impl Strategy<Value = String> {
    prop::collection::vec(inline(), 1..5).prop_map(|v| v.join(" "))
}

/// A Markdown block, rendered later. Containers hold more blocks.
#[derive(Debug, Clone)]
enum Block {
    Paragraph(Vec<String>),
    Heading(usize, String),
    Fenced(String, String, String),
    Indented(String),
    Rule(String),
    Table(Vec<String>, Vec<String>, Vec<Vec<String>>),
    Html(String),
    RawAdf(String),
    FootnoteDef(String),
    Quote(Vec<Block>),
    Alert(String, Vec<Block>),
    List(ListKind, bool, Vec<Vec<Block>>),
    Details(Option<String>, Vec<Block>),
    Cell(Vec<Block>),
}

#[derive(Debug, Clone, Copy)]
enum ListKind {
    Bullet,
    Ordered(usize),
    Task,
}

fn leaf_block() -> impl Strategy<Value = Block> {
    let cell = prop_oneof![3 => line(), 1 => Just(String::new()), 1 => Just("a \\| b".to_owned())];
    prop_oneof![
        6 => prop::collection::vec(line(), 1..3).prop_map(Block::Paragraph),
        2 => (1..=6_usize, line()).prop_map(|(n, t)| Block::Heading(n, t)),
        2 => (
            prop::sample::select(&["```", "~~~", "````"][..]),
            prop::sample::select(&["", "rust", "adf", "sh extra words"][..]),
            "[a-z *`#>|\n-]{0,40}",
        )
            .prop_map(|(f, l, b)| Block::Fenced(f.to_owned(), l.to_owned(), b)),
        1 => "[a-z *]{1,20}".prop_map(Block::Indented),
        1 => pick(&["---", "***", "___", "- - -"]).prop_map(Block::Rule),
        2 => (1..4_usize)
            .prop_flat_map(move |columns| (
                prop::collection::vec(cell.clone(), columns),
                prop::collection::vec(pick(&["---", ":---", ":---:", "---:"]), columns),
                // Rows may be short (padded) or long (cells dropped).
                prop::collection::vec(prop::collection::vec(cell.clone(), 0..columns + 2), 0..4),
            ))
            .prop_map(|(h, a, r)| Block::Table(h, a, r)),
        1 => pick(&["<div>\nblock html\n</div>", "<!-- block comment -->", "<script>x</script>", "<br>"])
            .prop_map(Block::Html),
        1 => pick(RAW_ADF).prop_map(Block::RawAdf),
        1 => line().prop_map(Block::FootnoteDef),
    ]
}

fn block() -> impl Strategy<Value = Block> {
    leaf_block().prop_recursive(4, 32, 4, |inner| {
        let children = prop::collection::vec(inner, 1..4);
        prop_oneof![
            children.clone().prop_map(Block::Quote),
            (
                pick(&["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION", "note"]),
                children.clone()
            )
                .prop_map(|(k, c)| Block::Alert(k, c)),
            (
                prop_oneof![
                    Just(ListKind::Bullet),
                    (0..20_usize).prop_map(ListKind::Ordered),
                    Just(ListKind::Task)
                ],
                any::<bool>(),
                prop::collection::vec(
                    prop_oneof![6 => children.clone(), 1 => Just(Vec::new())],
                    1..4,
                ),
            )
                .prop_map(|(kind, loose, items)| Block::List(kind, loose, items)),
            (prop::option::of(line()), children.clone()).prop_map(|(s, c)| Block::Details(s, c)),
            children.prop_map(Block::Cell),
        ]
    })
}

fn render_blocks(blocks: &[Block]) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.extend(render_block(block));
    }
    lines
}

fn prefixed(lines: Vec<String>, first: &str, rest: &str) -> Vec<String> {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let prefix = if i == 0 { first } else { rest };
            if line.is_empty() {
                prefix.trim_end().to_owned()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect()
}

/// Inline content for single-line contexts (headings, table cells, summaries): a generated hard
/// break would otherwise end the construct.
fn one_line(text: &str) -> String {
    text.replace("\\\n", " ").replace('\n', " ")
}

fn render_block(block: &Block) -> Vec<String> {
    match block {
        Block::Paragraph(lines) => lines
            .iter()
            .flat_map(|l| l.split('\n'))
            .map(str::to_owned)
            .collect(),
        Block::Heading(level, text) => vec![format!("{} {}", "#".repeat(*level), one_line(text))],
        Block::Fenced(fence, lang, body) => {
            let mut lines = vec![format!("{fence}{lang}")];
            lines.extend(body.lines().map(str::to_owned));
            lines.push(fence.clone());
            lines
        }
        Block::Indented(body) => vec![format!("    {body}")],
        Block::Rule(rule) => vec![rule.clone()],
        Block::Table(header, align, rows) => {
            let row = |cells: &[String]| {
                let cells: Vec<String> = cells.iter().map(|c| one_line(c)).collect();
                format!("| {} |", cells.join(" | "))
            };
            let mut lines = vec![row(header), format!("|{}|", align.join("|"))];
            lines.extend(rows.iter().map(|r| row(r)));
            lines
        }
        Block::Html(html) => html.lines().map(str::to_owned).collect(),
        Block::RawAdf(json) => vec!["```adf".to_owned(), json.clone(), "```".to_owned()],
        Block::FootnoteDef(text) => vec![format!("[^note]: {}", one_line(text))],
        Block::Quote(children) => prefixed(render_blocks(children), "> ", "> "),
        Block::Alert(kind, children) => {
            let mut lines = vec![format!("> [!{kind}]")];
            lines.extend(prefixed(render_blocks(children), "> ", "> "));
            lines
        }
        Block::List(kind, loose, items) => render_list(*kind, *loose, items),
        Block::Details(summary, children) => {
            let mut lines = vec!["<details>".to_owned()];
            if let Some(summary) = summary {
                lines.push(format!("<summary>{}</summary>", one_line(summary)));
            }
            lines.push(String::new());
            lines.extend(render_blocks(children));
            lines.push(String::new());
            lines.push("</details>".to_owned());
            lines
        }
        Block::Cell(children) => {
            let mut lines = vec!["::: cell".to_owned()];
            lines.extend(render_blocks(children));
            lines.push(":::".to_owned());
            lines
        }
    }
}

fn render_list(kind: ListKind, loose: bool, items: &[Vec<Block>]) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 && loose {
            lines.push(String::new());
        }
        let marker = match kind {
            ListKind::Bullet => "- ".to_owned(),
            ListKind::Ordered(start) => format!("{}. ", start + i),
            ListKind::Task if i % 2 == 0 => "- [ ] ".to_owned(),
            ListKind::Task => "- [x] ".to_owned(),
        };
        // Continuation lines are indented to the item's content column ("- " for task items).
        let indent = match kind {
            ListKind::Task => "  ".to_owned(),
            ListKind::Bullet | ListKind::Ordered(_) => " ".repeat(marker.len()),
        };
        let body = render_blocks(item);
        if body.is_empty() {
            lines.push(marker.trim_end().to_owned());
        } else {
            lines.extend(prefixed(body, &marker, &indent));
        }
    }
    lines
}

fn markdown_document() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(block(), 0..6),
        prop::bool::weighted(0.05),
    )
        .prop_map(|(blocks, front_matter)| {
            let mut text = if front_matter {
                "---\ntitle: generated\n---\n\n".to_owned()
            } else {
                String::new()
            };
            text.push_str(&render_blocks(&blocks).join("\n"));
            text.push('\n');
            text
        })
}

/// Markdown-significant fragments in random order: finds parser edge cases a grammar misses.
fn token_soup() -> impl Strategy<Value = String> {
    const TOKENS: &[&str] = &[
        "a",
        "word",
        " ",
        "\n",
        "\n\n",
        "\r\n",
        "\t",
        "*",
        "**",
        "_",
        "__",
        "~~",
        "`",
        "```",
        "~~~",
        "> ",
        "- ",
        "* ",
        "+ ",
        "1. ",
        "2) ",
        "- [ ] ",
        "- [x] ",
        "#",
        "## ",
        "|",
        "| a |",
        "|---|",
        ":---:",
        "---",
        "***",
        "[",
        "]",
        "(",
        ")",
        "](",
        "![",
        "<",
        ">",
        "</",
        "!",
        "\\",
        "&amp;",
        "&#",
        ";",
        "\"",
        "'",
        "{",
        "}",
        "::: cell\n",
        ":::\n",
        "[!NOTE]",
        "[!WARNING]",
        "<details>",
        "</details>",
        "<summary>",
        "</summary>",
        "<u>",
        "</u>",
        "<br>",
        "<!--",
        "-->",
        "```adf\n",
        "adf:mention/x",
        "adf:media/11111111-2222-3333-4444-555555555555",
        "adf:attachment/66666666-7777-8888-9999-000000000000",
        "?collection=contentId-1",
        "?width=1&layout=wide",
        "&size=50%",
        "adf:status/green",
        "adf:date",
        "https://",
        "www.",
        "@",
        "[^1]",
        "[^1]: ",
        "é",
        "😄",
        "\u{0}",
        "\u{7}",
        "\u{1b}",
        "\u{feff}",
        "\u{2028}",
        "    ",
        "\\\n",
        "  \n",
    ];
    prop::collection::vec(prop::sample::select(TOKENS), 0..200).prop_map(|t| t.concat())
}

fn options() -> impl Strategy<Value = MdToAdfOptions> {
    (
        prop_oneof![Just(Target::Jira), Just(Target::Confluence)],
        prop_oneof![Just(SoftBreak::Space), Just(SoftBreak::HardBreak)],
        prop_oneof![
            4 => Just(HtmlPolicy::Text),
            1 => Just(HtmlPolicy::Drop),
            1 => Just(HtmlPolicy::Error)
        ],
        prop_oneof![
            4 => Just(ImagePolicy::Link),
            1 => Just(ImagePolicy::External),
            1 => Just(ImagePolicy::Drop)
        ],
        prop_oneof![
            4 => Just(Extensions::all()),
            1 => Just(Extensions::none()),
            1 => prop::sample::subsequence(Extensions::NAMES, 0..=Extensions::NAMES.len())
                .prop_map(|names| Extensions::parse(&names.join(",")).expect("valid names")),
        ],
    )
        .prop_map(
            |(target, softbreak, html, images, extensions)| MdToAdfOptions {
                target,
                softbreak,
                html,
                images,
                extensions,
                ..MdToAdfOptions::default()
            },
        )
}

// ---------------------------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------------------------

/// Error codes `md_to_adf` may return (`docs/cli-contract.md`): limits, raw ADF, `--html error`.
fn check_md_errors(diagnostics: &Diagnostics, options: &MdToAdfOptions) -> Result<(), String> {
    if !diagnostics.has_errors() {
        return Err(format!("Err without an error diagnostic: {diagnostics}"));
    }
    for d in diagnostics.iter().filter(|d| d.is_error()) {
        let allowed = matches!(
            d.code,
            Code::InputTooLarge | Code::InputTooDeep | Code::RawAdf
        ) || (d.code == Code::Lossy && options.html == HtmlPolicy::Error);
        if !allowed {
            return Err(format!("undocumented error {}: {}", d.code, d.message));
        }
    }
    Ok(())
}

/// The main property: valid ADF or a documented error, deterministic, bounded, round-trips.
fn check_markdown(markdown: &str, options: &MdToAdfOptions) -> Result<(), String> {
    let first = adf_converter::md_to_adf(markdown, options);
    let second = adf_converter::md_to_adf(markdown, options);
    if first != second {
        return Err("two conversions of the same input differ".to_owned());
    }
    let conversion = match first {
        Ok(conversion) => conversion,
        Err(diagnostics) => return check_md_errors(&diagnostics, options),
    };
    let doc = conversion
        .output
        .to_value()
        .map_err(|e| format!("to_value: {e}"))?;
    let problems = support::adf_problems(&doc, options.target);
    if !problems.is_empty() {
        return Err(format!("invalid ADF:\n  {}", problems.join("\n  ")));
    }
    let size = support::to_compact_json(&doc).len();
    let bound = SIZE_K * markdown.len() + SIZE_C;
    if size > bound {
        return Err(format!(
            "output is {size} bytes for {} input bytes (bound {bound})",
            markdown.len()
        ));
    }
    support::check_round_trip(&doc, options)
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn generated_markdown_converts_to_valid_adf(markdown in markdown_document(), options in options()) {
        if let Err(e) = check_markdown(&markdown, &options) {
            prop_assert!(false, "{}\n\ninput:\n{}", e, markdown);
        }
    }

    #[test]
    fn generated_markdown_with_default_options(markdown in markdown_document()) {
        if let Err(e) = check_markdown(&markdown, &MdToAdfOptions::default()) {
            prop_assert!(false, "{}\n\ninput:\n{}", e, markdown);
        }
    }

    #[test]
    fn token_soup_converts_or_fails_cleanly(markdown in token_soup(), options in options()) {
        if let Err(e) = check_markdown(&markdown, &options) {
            prop_assert!(false, "{}\n\ninput: {:?}", e, markdown);
        }
    }

    #[test]
    fn arbitrary_strings_convert_or_fail_cleanly(markdown in any::<String>()) {
        if let Err(e) = check_markdown(&markdown, &MdToAdfOptions::default()) {
            prop_assert!(false, "{}\n\ninput: {:?}", e, markdown);
        }
    }

    #[test]
    fn adf_to_md_never_panics_on_arbitrary_adf(doc in adf_document(), plain in any::<bool>()) {
        let flavor = if plain { MdFlavor::Plain } else { MdFlavor::Roundtrip };
        let options = AdfToMdOptions { flavor, ..AdfToMdOptions::default() };
        match adf_converter::adf_to_md(&doc, &options) {
            Ok(conversion) => {
                // Whatever Markdown comes out must be safe to convert back.
                let back = adf_converter::md_to_adf(&conversion.output, &MdToAdfOptions::default());
                if let Err(diagnostics) = back {
                    let checked = check_md_errors(&diagnostics, &MdToAdfOptions::default());
                    prop_assert!(checked.is_ok(), "{:?}\n\nMarkdown:\n{}", checked, conversion.output);
                }
            }
            Err(diagnostics) => {
                for d in diagnostics.iter().filter(|d| d.is_error()) {
                    prop_assert_eq!(d.code.exit_code(), 3, "unexpected error {}: {}", d.code, d.message);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Arbitrary ADF-shaped JSON for ADF → Markdown
// ---------------------------------------------------------------------------------------------

const NODE_TYPES: &[&str] = &[
    "paragraph",
    "heading",
    "bulletList",
    "orderedList",
    "listItem",
    "taskList",
    "taskItem",
    "codeBlock",
    "blockquote",
    "rule",
    "panel",
    "table",
    "tableRow",
    "tableHeader",
    "tableCell",
    "expand",
    "nestedExpand",
    "mediaSingle",
    "media",
    "mediaGroup",
    "decisionList",
    "decisionItem",
    "layoutSection",
    "layoutColumn",
    "extension",
    "blockCard",
    "unknownNode",
    "doc",
    "text",
];
const INLINE_TYPES: &[&str] = &[
    "text",
    "hardBreak",
    "mention",
    "emoji",
    "date",
    "status",
    "inlineCard",
    "placeholder",
    "bogus",
];
const MARK_TYPES: &[&str] = &[
    "strong",
    "em",
    "code",
    "strike",
    "underline",
    "link",
    "subsup",
    "textColor",
    "backgroundColor",
    "alignment",
    "fontSize",
    "unknownMark",
];

/// Sets `key` on a JSON object (no-op for other values).
fn set(node: &mut Value, key: &str, value: Value) {
    if let Some(object) = node.as_object_mut() {
        object.insert(key.to_owned(), value);
    }
}

fn json_scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        (-3..300_i64).prop_map(Value::from),
        Just(json!(1.5)),
        "[a-z#:/.@ ]{0,12}".prop_map(Value::from),
        pick(URLS).prop_map(Value::from),
    ]
}

fn attrs() -> impl Strategy<Value = Value> {
    let keys = prop::sample::select(
        &[
            "level",
            "order",
            "localId",
            "state",
            "panelType",
            "language",
            "title",
            "href",
            "url",
            "id",
            "text",
            "color",
            "timestamp",
            "shortName",
            "type",
            "layout",
            "colspan",
            "rowspan",
            "colwidth",
            "background",
            "width",
            "isNumberColumnEnabled",
            "align",
        ][..],
    );
    prop::collection::btree_map(keys, json_scalar(), 0..4)
        .prop_map(|m| Value::Object(m.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()))
}

fn marks() -> impl Strategy<Value = Value> {
    prop::collection::vec(
        (pick(MARK_TYPES), prop::option::of(attrs())).prop_map(|(ty, attrs)| {
            let mut mark = json!({"type": ty});
            if let Some(attrs) = attrs {
                set(&mut mark, "attrs", attrs);
            }
            mark
        }),
        0..3,
    )
    .prop_map(Value::Array)
}

fn inline_node() -> impl Strategy<Value = Value> {
    (
        pick(INLINE_TYPES),
        prop_oneof![
            Just(None),
            "[a-z\n *`]{0,8}".prop_map(Some),
            Just(Some(String::new()))
        ],
        prop::option::of(attrs()),
        prop::option::of(marks()),
    )
        .prop_map(|(ty, text, attrs, marks)| {
            let mut node = json!({"type": ty});
            if let Some(text) = text {
                set(&mut node, "text", Value::from(text));
            }
            if let Some(attrs) = attrs {
                set(&mut node, "attrs", attrs);
            }
            if let Some(marks) = marks {
                set(&mut node, "marks", marks);
            }
            node
        })
}

fn adf_node() -> impl Strategy<Value = Value> {
    inline_node().prop_recursive(6, 64, 5, |inner| {
        (
            pick(NODE_TYPES),
            prop::option::of(attrs()),
            prop_oneof![
                4 => prop::collection::vec(inner, 0..5).prop_map(|v| Some(Value::Array(v))),
                1 => Just(None),
                1 => json_scalar().prop_map(Some),
            ],
        )
            .prop_map(|(ty, attrs, content)| {
                let mut node = json!({"type": ty});
                if let Some(attrs) = attrs {
                    set(&mut node, "attrs", attrs);
                }
                if let Some(content) = content {
                    set(&mut node, "content", content);
                }
                node
            })
    })
}

fn adf_document() -> impl Strategy<Value = Value> {
    prop_oneof![
        8 => prop::collection::vec(adf_node(), 0..6)
            .prop_map(|content| json!({"type": "doc", "version": 1, "content": content})),
        1 => adf_node(),
        1 => json_scalar(),
    ]
}

// ---------------------------------------------------------------------------------------------
// Deterministic robustness tests
// ---------------------------------------------------------------------------------------------

/// `depth` list items, each nested one level deeper than the previous one.
fn nested_list(marker: &str, indent: usize, depth: usize) -> String {
    let mut markdown = String::new();
    for level in 0..depth {
        markdown.push_str(&" ".repeat(indent * level));
        markdown.push_str(marker);
        markdown.push_str("level ");
        markdown.push_str(&level.to_string());
        markdown.push('\n');
    }
    markdown
}

fn error_codes(markdown: &str, options: &MdToAdfOptions) -> Vec<Code> {
    match adf_converter::md_to_adf(markdown, options) {
        Ok(_) => Vec::new(),
        Err(d) => d.iter().filter(|d| d.is_error()).map(|d| d.code).collect(),
    }
}

/// Nesting beyond the limit (64) fails with `E_INPUT_TOO_DEEP` instead of overflowing the stack.
#[test]
fn deep_nesting_fails_with_input_too_deep() {
    let options = MdToAdfOptions::default();
    let cases = [
        ("quotes", "> ".repeat(10_000) + "x\n"),
        ("bullet lists", nested_list("- ", 2, 200)),
        ("ordered lists", nested_list("1. ", 3, 200)),
        ("list markers on one line", "- ".repeat(5_000) + "x\n"),
        (
            "details",
            "<details>\n\n".repeat(200) + "x\n" + &"\n</details>\n".repeat(200),
        ),
    ];
    for (name, markdown) in cases {
        let codes = error_codes(&markdown, &options);
        assert!(
            codes.contains(&Code::InputTooDeep),
            "{name}: expected E_INPUT_TOO_DEEP, got {codes:?}"
        );
    }
}

/// Inputs that are slow or crash naive parsers (after cmark's `pathological_tests.py`) finish
/// with valid ADF or a documented error.
#[test]
fn pathological_inputs_finish() {
    let n = 5_000;
    let cases: Vec<(&str, String)> = vec![
        ("nested strong emphasis", "*a **a ".repeat(n)),
        ("unclosed emphasis", "*a ".repeat(n)),
        ("underscores", "a_ ".repeat(n)),
        ("open brackets", "[".repeat(n)),
        ("close brackets", "]".repeat(n)),
        ("unclosed links", "[a](".repeat(n)),
        (
            "link openers and closers",
            "[a]".repeat(n) + &"(b)".repeat(n),
        ),
        ("angle brackets", "<".repeat(n)),
        ("backticks", "`".repeat(n)),
        ("alternating backticks", "`a``".repeat(n)),
        ("entities", "&#".repeat(n)),
        ("hard breaks", "a\\\n".repeat(n)),
        ("pipes", "|".repeat(n)),
        ("tildes", "~".repeat(n)),
        ("many tasks", "- [ ] t\n".repeat(n)),
        ("many paragraphs", "p\n\n".repeat(n)),
        ("adf links", "[x](adf:status/green)".repeat(n)),
        ("html comments", "<!--".repeat(n)),
        ("cell fences", "::: cell\n".repeat(n)),
        ("raw fences", "```adf\n".repeat(200)),
        ("long line", "x".repeat(200_000)),
    ];
    let mut failures = Vec::new();
    for (name, markdown) in cases {
        match adf_converter::md_to_adf(&markdown, &MdToAdfOptions::default()) {
            Ok(conversion) => {
                let doc = conversion.output.to_value().expect("serializes");
                let problems = support::adf_problems(&doc, Target::Jira);
                if !problems.is_empty() {
                    failures.push(format!("{name}: invalid ADF: {:?}", problems.first()));
                }
            }
            Err(d) => {
                if let Err(e) = check_md_errors(&d, &MdToAdfOptions::default()) {
                    failures.push(format!("{name}: {e}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Upper bound on the compact JSON of one empty, aligned table cell (about 55 bytes in practice).
const CELL_BYTES: usize = 128;

/// A table of `columns` columns and `body_rows` one-character rows, which GFM pads to the
/// header width: `columns × (body_rows + 1)` cells from about `2 × (columns + body_rows)` bytes.
fn padded_table(columns: usize, body_rows: usize) -> String {
    let mut markdown = format!("|{}\n|{}\n", "c|".repeat(columns), "-|".repeat(columns));
    markdown.push_str(&"x\n".repeat(body_rows));
    markdown
}

/// GFM pads short rows to the header width. Without a limit, a 3 KB input (256 columns, 1,000
/// one-character rows) would produce about 14 MB of ADF, and a 4 MiB input about 20 GB. Tables
/// may expand to at most `MAX_TABLE_CELLS` cells: within it, the output stays within the linear
/// bound plus `CELL_BYTES` per allowed cell; beyond it, the conversion fails with
/// `E_INPUT_TOO_LARGE`.
#[test]
fn padded_table_rows_keep_output_bounded() {
    let cases = [
        (256, 1_000),
        (256, 389),
        (100, 999),
        (100, 1_000),
        (1_000, 99),
        (1_000, 100),
        (3, 30_000),
    ];
    for (columns, rows) in cases {
        let markdown = padded_table(columns, rows);
        let cells = columns * (rows + 1);
        match adf_converter::md_to_adf(&markdown, &MdToAdfOptions::default()) {
            Ok(conversion) => {
                assert!(
                    cells <= MAX_TABLE_CELLS,
                    "{columns}×{rows}: {cells} cells were accepted"
                );
                let size =
                    support::to_compact_json(&conversion.output.to_value().expect("serializes"))
                        .len();
                let bound = SIZE_K * markdown.len() + SIZE_C + CELL_BYTES * MAX_TABLE_CELLS;
                assert!(
                    size <= bound,
                    "{columns}×{rows}: {} input bytes produced {size} bytes of ADF (bound {bound})",
                    markdown.len()
                );
            }
            Err(d) => {
                assert!(
                    cells > MAX_TABLE_CELLS,
                    "{columns}×{rows}: {cells} cells were refused: {d}"
                );
                let codes: Vec<Code> = d.iter().filter(|d| d.is_error()).map(|d| d.code).collect();
                assert_eq!(codes, [Code::InputTooLarge], "{columns}×{rows}: {d}");
            }
        }
    }
}

/// Close to 1 MiB of padded tables (about 75 million cells) is refused before the Markdown
/// parser builds them, fast and with `E_INPUT_TOO_LARGE` at the table that crosses the limit;
/// so is a single padded table after 1 MiB of table-free text, which the pre-scan reads in full.
#[test]
fn many_padded_tables_fail_fast() {
    let table = padded_table(256, 1_000);
    let tables = vec![table.as_str(); DEFAULT_MAX_MARKDOWN_BYTES / (table.len() + 1)].join("\n");
    let mut text = "| not | a | table |\n".repeat((DEFAULT_MAX_MARKDOWN_BYTES - table.len()) / 20);
    let text_lines = text.matches('\n').count();
    text.push_str(&table);
    for (name, markdown, line) in [("tables", &tables, 1), ("text", &text, text_lines + 1)] {
        assert!(
            markdown.len() > 1_000_000,
            "{name}: {} bytes",
            markdown.len()
        );
        let start = Instant::now();
        let result = adf_converter::md_to_adf(markdown, &MdToAdfOptions::default());
        let elapsed = start.elapsed();
        let error = result
            .expect_err("refused")
            .iter()
            .find(|d| d.is_error())
            .cloned()
            .expect("an error");
        assert_eq!(error.code, Code::InputTooLarge, "{name}: {error}");
        let location = error.location.expect("located");
        assert_eq!(location.line, Some(u32::try_from(line).unwrap()), "{name}");
        // Release builds take a few milliseconds; debug builds are about 10× slower.
        let budget = if cfg!(debug_assertions) {
            Duration::from_secs(5)
        } else {
            Duration::from_millis(500)
        };
        assert!(elapsed < budget, "{name}: refused after {elapsed:?}");
    }
}

#[test]
fn input_size_limit_is_enforced() {
    let options = MdToAdfOptions {
        limits: Limits {
            max_input_bytes: 1024,
            ..Limits::default()
        },
        ..MdToAdfOptions::default()
    };
    assert_eq!(error_codes(&"a".repeat(1024), &options), Vec::<Code>::new());
    assert_eq!(
        error_codes(&"a".repeat(1025), &options),
        vec![Code::InputTooLarge]
    );
}

#[test]
fn depth_limit_is_configurable() {
    let options = MdToAdfOptions {
        limits: Limits {
            max_depth: 4,
            ..Limits::default()
        },
        ..MdToAdfOptions::default()
    };
    assert_eq!(error_codes("- a\n  - b\n", &options), Vec::<Code>::new());
    let deep = nested_list("- ", 2, 10);
    assert!(error_codes(&deep, &options).contains(&Code::InputTooDeep));
}

/// ADF input deeper than the limit fails cleanly in ADF → Markdown.
#[test]
fn adf_to_md_rejects_deep_json() {
    let mut node = json!({"type": "paragraph", "content": [{"type": "text", "text": "x"}]});
    for _ in 0..300 {
        node = json!({"type": "bulletList", "content": [{"type": "listItem", "content": [
            {"type": "paragraph"}, node]}]});
    }
    let doc = json!({"type": "doc", "version": 1, "content": [node]});
    let result = adf_converter::adf_to_md(&doc, &AdfToMdOptions::default());
    let codes: Vec<Code> = result
        .err()
        .map_or_else(Vec::new, |d| d.iter().map(|d| d.code).collect());
    assert!(
        codes.contains(&Code::InputTooDeep),
        "expected E_INPUT_TOO_DEEP, got {codes:?}"
    );
}

/// Values that are not documents fail with an input error instead of panicking.
#[test]
fn adf_to_md_handles_non_documents() {
    for value in [
        json!(null),
        json!([]),
        json!("doc"),
        json!({}),
        json!({"type": "paragraph"}),
        json!({"type": "doc"}),
        json!({"type": "doc", "version": 1, "content": "x"}),
        json!({"type": "doc", "version": 1, "content": [null, 1, "x", []]}),
    ] {
        for flavor in [MdFlavor::Roundtrip, MdFlavor::Plain] {
            let options = AdfToMdOptions {
                flavor,
                ..AdfToMdOptions::default()
            };
            if let Err(d) = adf_converter::adf_to_md(&value, &options) {
                for e in d.iter().filter(|d| d.is_error()) {
                    assert_eq!(
                        e.code.exit_code(),
                        3,
                        "{value}: unexpected {}: {}",
                        e.code,
                        e.message
                    );
                }
            }
        }
    }
}

/// The generator itself produces the constructs it promises (guards against a vacuous grammar).
#[test]
fn generator_covers_the_grammar() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let mut all = String::new();
    for _ in 0..400 {
        let tree = markdown_document()
            .new_tree(&mut runner)
            .expect("generates");
        all.push_str(&tree.current());
    }
    for needle in [
        "> [!",
        "<details>",
        "::: cell",
        "```adf",
        "| ",
        "- [ ] ",
        "1. ",
        "**",
        "~~",
        "](",
        "adf:",
        "](adf:media/",
        "](adf:attachment/",
        "<u>",
        "&#",
        "\\",
        "    ",
    ] {
        assert!(
            all.contains(needle),
            "the generator never produced {needle:?}"
        );
    }
}
