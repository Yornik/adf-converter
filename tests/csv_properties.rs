//! Property tests (proptest) for the CSV conversions:
//!
//! - `csv_to_adf` never panics on arbitrary text or on hostile, grammar-built CSV; it returns
//!   valid ADF (for both targets) or only documented errors (`E_INPUT_CSV`, `E_INPUT_TOO_LARGE`),
//!   is deterministic, and its output size is bounded by `K * input + C`;
//! - `adf_to_csv` never panics on ADF-shaped JSON with arbitrary spans and content, or on any
//!   JSON, and returns CSV that ends with a line feed or only documented errors;
//! - the round trip `csv → adf → csv` gives the normalised input for rectangular tables
//!   (normalisation: blank records dropped, cells trimmed, CRLF and CR → LF, control characters
//!   removed).
//!
//! 256 cases per property by default; set `PROPTEST_CASES` to change it. Failures are persisted
//! to `proptest-regressions/csv_properties.txt`, which is committed.

// Test helpers report failures by panicking.
#![allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing
)]

mod support;

use adf_converter::{
    AdfToCsvOptions, Code, CsvDelimiter, CsvToAdfOptions, Diagnostic, Target, adf_to_csv,
    csv_to_adf,
};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use serde_json::{Map, Value, json};

/// Output bound `K * input bytes + C` (compact JSON). The largest legitimate expansion per input
/// byte is an empty, aligned cell (`,` → about 110 bytes of JSON); a generated header adds
/// `Column N` per column.
const SIZE_K: usize = 256;
const SIZE_C: usize = 8192;

fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "proptest-regressions/csv_properties.txt",
        ))),
        ..ProptestConfig::default()
    }
}

// ---------------------------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------------------------

/// Fragments that matter to a CSV reader, a Markdown reader or a spreadsheet.
const TOKENS: &[&str] = &[
    "a",
    "b c",
    "x",
    "1",
    "2.50",
    "-3",
    "12%",
    " ",
    "  ",
    "\t",
    "\n",
    "\r\n",
    "\r",
    ",",
    ";",
    "|",
    "\"",
    "\"\"",
    "é",
    "日本語",
    "😀",
    "\0",
    "\u{1}",
    "\u{7f}",
    "\u{85}",
    "\u{a0}",
    "\u{feff}",
    "\u{2028}",
    "=1+1",
    "+x",
    "-",
    "@u",
    "*",
    "_",
    "`",
    "<b>",
    "[x](y)",
    "![i](z)",
    "\\",
    "&amp;",
    "#",
    "https://example.com/a",
    "http://x.org/p.",
    "(https://e.com/q)",
    "www.x.com",
    "a@b.co",
    "ftp://f.org",
    "https://",
];

/// A string made of [`TOKENS`].
fn soup(max_tokens: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(TOKENS), 0..max_tokens).prop_map(|t| t.concat())
}

fn delimiter() -> impl Strategy<Value = CsvDelimiter> {
    prop::sample::select(&["comma", "tab", "semicolon", "pipe", "|", "x"][..])
        .prop_map(|name| CsvDelimiter::parse(name).unwrap())
}

fn options() -> impl Strategy<Value = CsvToAdfOptions> {
    (
        delimiter(),
        any::<bool>(),
        prop::option::of(0usize..5),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(delimiter, header, max_rows, align_numbers, confluence)| CsvToAdfOptions {
                target: if confluence {
                    Target::Confluence
                } else {
                    Target::Jira
                },
                delimiter,
                header,
                max_rows,
                align_numbers,
                ..CsvToAdfOptions::default()
            },
        )
}

/// A field as written: quoted (properly escaped), or raw (anything, even stray quotes).
fn raw_field() -> impl Strategy<Value = String> {
    (soup(4), any::<bool>()).prop_map(|(text, quoted)| {
        if quoted {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text
        }
    })
}

/// Records of fields with every kind of line ending; sometimes cut short or with a stray quote.
fn csv_text() -> impl Strategy<Value = String> {
    let record = (
        prop::collection::vec(raw_field(), 0..5),
        prop::sample::select(&["\n", "\r\n", "\r", "", "\n\n"][..]),
    )
        .prop_map(|(fields, eol)| format!("{}{eol}", fields.join(",")));
    (
        prop::collection::vec(record, 0..6),
        any::<bool>(),
        prop::option::of(Just("\"")),
    )
        .prop_map(|(records, bom, tail)| {
            let mut out = String::new();
            if bom {
                out.push('\u{feff}');
            }
            out.push_str(&records.concat());
            out.push_str(tail.unwrap_or_default());
            out
        })
}

// ---------------------------------------------------------------------------------------------
// CSV → ADF
// ---------------------------------------------------------------------------------------------

fn codes(diagnostics: &[Diagnostic]) -> Vec<Code> {
    diagnostics.iter().map(|d| d.code).collect()
}

/// Checks everything that must hold for any input.
fn check_csv_to_adf(text: &str, options: &CsvToAdfOptions) -> Result<(), TestCaseError> {
    let result = csv_to_adf(text, options);
    prop_assert_eq!(&result, &csv_to_adf(text, options), "not deterministic");
    match result {
        Ok(conversion) => {
            let doc = conversion.output.to_value().unwrap();
            for target in [Target::Jira, Target::Confluence] {
                let problems = support::adf_problems(&doc, target);
                prop_assert!(problems.is_empty(), "invalid ADF: {problems:?}\n{doc}");
            }
            for code in codes(&conversion.warnings) {
                prop_assert!(
                    matches!(code, Code::ControlChar | Code::CsvTruncated),
                    "unexpected warning {code}"
                );
            }
            for warning in &conversion.warnings {
                let at = warning.location.as_ref().unwrap();
                prop_assert!(at.line.is_some() && at.column.is_some());
            }
            let size = support::to_compact_json(&doc).len();
            prop_assert!(
                size <= SIZE_K * text.len() + SIZE_C,
                "{size} bytes of ADF for {} bytes of CSV",
                text.len()
            );
            prop_assert_eq!(doc["content"].as_array().map(Vec::len), Some(1));
            let table = &doc["content"][0];
            prop_assert_eq!(table["type"].as_str(), Some("table"));
            let rows = table["content"].as_array().unwrap();
            let widths: Vec<_> = rows
                .iter()
                .map(|r| r["content"].as_array().unwrap().len())
                .collect();
            prop_assert!(
                widths.windows(2).all(|w| w[0] == w[1]),
                "ragged table {widths:?}"
            );
        }
        Err(diagnostics) => {
            let error = &diagnostics.0[0];
            prop_assert!(
                matches!(error.code, Code::InputCsv | Code::InputTooLarge),
                "unexpected error {error}"
            );
            prop_assert_eq!(error.code.exit_code(), 3);
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn arbitrary_text_never_panics(text in any::<String>(), options in options()) {
        check_csv_to_adf(&text, &options)?;
    }

    #[test]
    fn token_soup_never_panics(text in soup(40), options in options()) {
        check_csv_to_adf(&text, &options)?;
    }

    #[test]
    fn hostile_csv_never_panics(text in csv_text(), options in options()) {
        check_csv_to_adf(&text, &options)?;
    }

    /// The line ending and a leading BOM never change the result.
    #[test]
    fn line_endings_and_boms_do_not_matter(text in csv_text(), options in options()) {
        let lf = text.replace("\r\n", "\n").replace('\r', "\n");
        let body = lf.strip_prefix('\u{feff}').unwrap_or(&lf);
        prop_assume!(!body.starts_with('\u{feff}'));
        let expected = csv_to_adf(body, &options);
        for variant in [
            body.replace('\n', "\r\n"),
            body.replace('\n', "\r"),
            format!("\u{feff}{body}"),
            text.clone(),
        ] {
            prop_assert_eq!(&csv_to_adf(&variant, &options), &expected);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Round trip
// ---------------------------------------------------------------------------------------------

/// What a cell becomes: line breaks as LF, control characters (other than tab and LF) removed,
/// spaces, tabs and line breaks trimmed.
fn normalise_cell(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|&c| c == '\t' || c == '\n' || !c.is_control())
        .collect::<String>()
        .trim_matches([' ', '\t', '\n'])
        .to_owned()
}

/// The writer rules of the contract, written out again.
fn expected_field(text: &str, delimiter: char) -> String {
    let needs_quotes = text.contains(delimiter)
        || text.contains(['"', '\n', '\r'])
        || text.starts_with([' ', '\t'])
        || text.ends_with([' ', '\t']);
    if needs_quotes {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// A CSV file written by a careful program: every cell that needs it is quoted.
fn write_csv(rows: &[Vec<String>], delimiter: char, eol: &str, quote_all: bool) -> String {
    let mut out = String::new();
    for row in rows {
        let fields: Vec<String> = row
            .iter()
            .map(|cell| {
                if quote_all {
                    format!("\"{}\"", cell.replace('"', "\"\""))
                } else {
                    expected_field(cell, delimiter)
                }
            })
            .collect();
        out.push_str(&fields.join(&delimiter.to_string()));
        out.push_str(eol);
    }
    out
}

fn rectangular() -> impl Strategy<Value = Vec<Vec<String>>> {
    (1usize..5, 1usize..6).prop_flat_map(|(rows, columns)| {
        prop::collection::vec(
            prop::collection::vec(soup(4), columns..=columns),
            rows..=rows,
        )
    })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn csv_adf_csv_gives_the_normalised_input(
        rows in rectangular(),
        delimiter in prop::sample::select(&[',', '\t', ';', '|'][..]),
        eol in prop::sample::select(&["\n", "\r\n", "\r"][..]),
        quote_all in any::<bool>(),
    ) {
        // A BOM at the start of the file is not data.
        prop_assume!(!rows[0][0].starts_with('\u{feff}') || quote_all);
        let text = write_csv(&rows, delimiter, eol, quote_all);
        let options = CsvToAdfOptions {
            delimiter: CsvDelimiter::parse(&delimiter.to_string()).unwrap(),
            ..CsvToAdfOptions::default()
        };
        let kept: Vec<Vec<String>> = rows
            .iter()
            .filter(|row| !row.iter().all(|c| c.chars().all(char::is_whitespace)))
            .map(|row| row.iter().map(|c| normalise_cell(c)).collect())
            .collect();
        let result = csv_to_adf(&text, &options);
        if kept.is_empty() {
            let error = &result.unwrap_err().0[0];
            prop_assert_eq!(error.code, Code::InputCsv);
            return Ok(());
        }
        let doc = result.unwrap().output.to_value().unwrap();
        let csv_options = AdfToCsvOptions { delimiter: options.delimiter, ..AdfToCsvOptions::default() };
        let back = adf_to_csv(&doc, &csv_options).unwrap();
        prop_assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        let mut expected = String::new();
        for row in &kept {
            let fields: Vec<String> = row.iter().map(|c| expected_field(c, delimiter)).collect();
            // A lone empty field is written as `""`, so that it is not a blank line.
            if fields.len() == 1 && fields[0].is_empty() {
                expected.push_str("\"\"");
            }
            expected.push_str(&fields.join(&delimiter.to_string()));
            expected.push('\n');
        }
        prop_assert_eq!(back.output, expected);
    }
}

// ---------------------------------------------------------------------------------------------
// ADF → CSV
// ---------------------------------------------------------------------------------------------

fn text_node(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

/// Inline nodes, known and unknown.
fn inline() -> impl Strategy<Value = Value> {
    prop_oneof![
        soup(3)
            .prop_filter("text is not empty", |s| !s.is_empty())
            .prop_map(|text| text_node(&text)),
        Just(json!({"type": "hardBreak"})),
        Just(json!({"type": "mention", "attrs": {"id": "u1", "text": "@Ann"}})),
        Just(json!({"type": "mention", "attrs": {"id": "u2"}})),
        Just(json!({"type": "status", "attrs": {"text": "DONE", "color": "green"}})),
        Just(json!({"type": "date", "attrs": {"timestamp": "1761523200000"}})),
        Just(json!({"type": "date", "attrs": {"timestamp": "soon"}})),
        Just(json!({"type": "emoji", "attrs": {"shortName": ":smile:"}})),
        Just(json!({"type": "inlineCard", "attrs": {"url": "https://example.com/x"}})),
        Just(
            json!({"type": "mediaInline", "attrs": {"id": "1", "collection": "c", "alt": "a.zip"}})
        ),
        Just(json!({"type": "weird", "text": "w", "content": [{"type": "text", "text": "z"}]})),
        Just(json!(42)),
        Just(json!({"type": 7})),
    ]
}

fn paragraph() -> impl Strategy<Value = Value> {
    prop::collection::vec(inline(), 0..4)
        .prop_map(|content| json!({"type": "paragraph", "content": content}))
}

/// Block nodes, nested a few levels.
fn block() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        paragraph(),
        Just(json!({"type": "rule"})),
        Just(json!({"type": "codeBlock", "content": [{"type": "text", "text": "a\nb"}]})),
        Just(
            json!({"type": "mediaSingle", "content": [{"type": "media", "attrs": {"type": "file", "id": "1", "collection": "c", "alt": "i.png"}}]})
        ),
        Just(json!({"type": "blockCard", "attrs": {"url": "https://example.com/b"}})),
    ];
    leaf.prop_recursive(3, 12, 3, |inner| {
        prop::collection::vec(inner, 0..3).prop_flat_map(|content| {
            prop::sample::select(
                &[
                    "blockquote",
                    "panel",
                    "listItem",
                    "bulletList",
                    "nestedExpand",
                    "table",
                ][..],
            )
            .prop_map(move |ty| json!({"type": ty, "content": content.clone()}))
        })
    })
}

fn span() -> impl Strategy<Value = Value> {
    prop_oneof![
        3 => Just(json!(1)),
        2 => Just(json!(2)),
        1 => Just(json!(3)),
        1 => Just(json!(0)),
        1 => Just(json!(-1)),
        1 => Just(json!("2")),
        1 => Just(json!(1000)),
        1 => Just(json!(u64::MAX)),
    ]
}

fn cell() -> impl Strategy<Value = Value> {
    (
        prop::collection::vec(block(), 0..3),
        prop::option::of((span(), span())),
        any::<bool>(),
    )
        .prop_map(|(content, spans, header)| {
            let mut cell = Map::new();
            cell.insert(
                "type".into(),
                json!(if header { "tableHeader" } else { "tableCell" }),
            );
            if let Some((colspan, rowspan)) = spans {
                cell.insert(
                    "attrs".into(),
                    json!({"colspan": colspan, "rowspan": rowspan}),
                );
            }
            cell.insert("content".into(), Value::Array(content));
            Value::Object(cell)
        })
}

fn table() -> impl Strategy<Value = Value> {
    prop::collection::vec(prop::collection::vec(cell(), 0..5), 0..5).prop_map(|rows| {
        let rows: Vec<Value> = rows
            .into_iter()
            .map(|cells| json!({"type": "tableRow", "content": cells}))
            .collect();
        json!({"type": "table", "content": rows})
    })
}

fn document() -> impl Strategy<Value = Value> {
    prop::collection::vec(prop_oneof![3 => table(), 1 => block()], 0..4)
        .prop_map(|content| json!({"type": "doc", "version": 1, "content": content}))
}

/// Any JSON, a few levels deep.
fn any_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        soup(3).prop_map(Value::from),
    ];
    leaf.prop_recursive(4, 32, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::vec(("(type|content|attrs|text)", inner), 0..4)
                .prop_map(|entries| Value::Object(entries.into_iter().collect())),
        ]
    })
}

fn csv_options() -> impl Strategy<Value = AdfToCsvOptions> {
    (delimiter(), 0usize..4, any::<bool>()).prop_map(|(delimiter, table, csv_safe)| {
        AdfToCsvOptions {
            delimiter,
            table,
            csv_safe,
            ..AdfToCsvOptions::default()
        }
    })
}

fn check_adf_to_csv(doc: &Value, options: &AdfToCsvOptions) -> Result<(), TestCaseError> {
    let result = adf_to_csv(doc, options);
    prop_assert_eq!(&result, &adf_to_csv(doc, options), "not deterministic");
    match result {
        Ok(conversion) => {
            let csv = &conversion.output;
            prop_assert!(csv.ends_with('\n'));
            let bom = '\u{feff}';
            prop_assert!(!csv.starts_with(bom));
            prop_assert!(!csv.contains('\r'));
            for warning in &conversion.warnings {
                prop_assert!(
                    matches!(warning.code, Code::CsvMergedCells | Code::ControlChar),
                    "unexpected warning {warning}"
                );
                let pointer = warning.location.as_ref().and_then(|l| l.pointer.as_ref());
                prop_assert!(pointer.is_some_and(|p| p.is_empty() || p.starts_with('/')));
            }
            prop_assert!(
                !csv.chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
            );
            // The output can be read back, or has no rows at all.
            let back = CsvToAdfOptions {
                delimiter: options.delimiter,
                ..CsvToAdfOptions::default()
            };
            if let Err(error) = csv_to_adf(csv, &back) {
                let error = &error.0[0];
                let no_rows = error.code == Code::InputCsv && error.message.contains("no rows");
                prop_assert!(no_rows || error.code == Code::InputTooLarge, "{error}");
            }
        }
        Err(diagnostics) => {
            let error = &diagnostics.0[0];
            prop_assert!(
                matches!(
                    error.code,
                    Code::InputNoTable
                        | Code::InputNotAdf
                        | Code::InputTooDeep
                        | Code::InputTooLarge
                ),
                "unexpected error {error}"
            );
            prop_assert_eq!(error.code.exit_code(), 3);
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn adf_shaped_documents_never_panic(doc in document(), options in csv_options()) {
        check_adf_to_csv(&doc, &options)?;
    }

    #[test]
    fn any_json_never_panics(value in any_json(), options in csv_options()) {
        check_adf_to_csv(&value, &options)?;
    }

    /// A document that is not a table-less accident: asking for table 1 of a document with a
    /// table gives CSV, and for the table after the last gives `E_INPUT_NO_TABLE`.
    #[test]
    fn table_numbers_are_exact(doc in document()) {
        let count = {
            let mut count = 0usize;
            let mut stack = vec![&doc];
            while let Some(node) = stack.pop() {
                if node["type"] == "table" { count += 1; }
                stack.extend(node["content"].as_array().into_iter().flatten());
            }
            count
        };
        let at = |table| adf_to_csv(&doc, &AdfToCsvOptions { table, ..AdfToCsvOptions::default() });
        let past = at(count + 1).unwrap_err();
        prop_assert_eq!(past.0[0].code, Code::InputNoTable);
        if count > 0 {
            let first = at(1);
            prop_assert!(first.is_ok() || matches!(first.as_ref().unwrap_err().0[0].code, Code::InputTooLarge), "{first:?}");
        }
    }
}
