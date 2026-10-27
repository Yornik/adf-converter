//! ADF → CSV: picks one table of a document, expands it to a grid and writes the grid.

use crate::Conversion;
use crate::adf::reader::{Node, read_doc};
use crate::diagnostics::{Code, Diagnostic, Diagnostics, Location};
use crate::limits::MAX_OUTPUT_BYTES;
use crate::options::AdfToCsvOptions;

use super::cell_text::cell_text;
use super::grid::{self, Grid};
use super::write;

/// Converts table number `opts.table` of a bare `doc` to CSV text.
pub(super) fn convert(
    doc: &serde_json::Value,
    opts: &AdfToCsvOptions,
) -> Result<Conversion<String>, Diagnostics> {
    let root = read_doc(doc, &opts.limits).map_err(Diagnostics::single)?;
    let tables = find_tables(&root);
    let table = opts
        .table
        .checked_sub(1)
        .and_then(|index| tables.get(index))
        .copied()
        .ok_or_else(|| Diagnostics::single(no_table(tables.len(), opts.table)))?;
    let grid = grid::build(table).map_err(Diagnostics::single)?;
    let mut warnings = Vec::new();
    if grid.merged {
        warnings.push(
            Diagnostic::new(
                Code::CsvMergedCells,
                "The table has merged cells; their text is repeated in every cell they cover",
            )
            .at(Location::pointer(table.pointer())),
        );
    }
    let fields = fields(&grid, opts, &mut warnings);
    let output = write_grid(&grid, &fields, opts).map_err(|error| {
        let mut all = vec![error];
        all.extend(warnings.iter().cloned());
        Diagnostics(all)
    })?;
    Ok(Conversion { output, warnings })
}

/// Every `table` node of the document, in document order. Walks with an explicit stack.
fn find_tables<'r, 'a>(root: &'r Node<'a>) -> Vec<&'r Node<'a>> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.ty == "table" {
            found.push(node);
        }
        stack.extend(node.content.iter().rev());
    }
    found
}

fn no_table(count: usize, wanted: usize) -> Diagnostic {
    let (message, hint) = match count {
        0 => (
            "The document has no table".to_owned(),
            "Pass an ADF document that contains a table".to_owned(),
        ),
        1 => (
            format!("The document has 1 table, so table {wanted} does not exist"),
            "Use --table 1 (the default)".to_owned(),
        ),
        n => (
            format!("The document has {n} tables, so table {wanted} does not exist"),
            format!("Use --table with a number from 1 to {n}"),
        ),
    };
    Diagnostic::new(Code::InputNoTable, message).with_hint(hint)
}

/// The written form of each cell, indexed like `grid.cells`. Control characters are removed
/// (`W_CONTROL_CHAR` at the cell).
fn fields(
    grid: &Grid<'_, '_>,
    opts: &AdfToCsvOptions,
    warnings: &mut Vec<Diagnostic>,
) -> Vec<String> {
    let delimiter = opts.delimiter.as_char();
    grid.cells
        .iter()
        .map(|cell| {
            let (text, removed) = cell_text(cell);
            if removed {
                warnings.push(
                    Diagnostic::new(
                        Code::ControlChar,
                        "Control characters were removed from a cell",
                    )
                    .at(Location::pointer(cell.pointer())),
                );
            }
            write::field(&text, delimiter, opts.csv_safe)
        })
        .collect()
}

/// The records of the grid, each ending with a line feed. A table without rows is one empty
/// record, so that the output still ends with a line feed.
fn write_grid(
    grid: &Grid<'_, '_>,
    fields: &[String],
    opts: &AdfToCsvOptions,
) -> Result<String, Diagnostic> {
    let delimiter = opts.delimiter.as_char();
    let mut out = String::new();
    for row in 0..grid.rows.len() {
        for column in 0..grid.width {
            if column > 0 {
                out.push(delimiter);
            }
            let field = grid
                .cell_at(row, column)
                .and_then(|index| fields.get(index))
                .map_or("", String::as_str);
            out.push_str(field);
            if out.len() > MAX_OUTPUT_BYTES {
                return Err(too_large());
            }
        }
        out.push('\n');
    }
    if grid.rows.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

fn too_large() -> Diagnostic {
    Diagnostic::new(
        Code::InputTooLarge,
        format!(
            "The CSV output would be larger than the {} MiB limit",
            MAX_OUTPUT_BYTES / (1024 * 1024)
        ),
    )
    .with_hint("Merged cells repeat their text in every cell they cover; split the table")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::CsvDelimiter;
    use serde_json::{Value, json};

    fn cell(kind: &str, text: &str) -> Value {
        json!({"type": kind, "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]})
    }

    fn spanned(text: &str, colspan: u64, rowspan: u64) -> Value {
        let mut cell = cell("tableCell", text);
        cell["attrs"] = json!({"colspan": colspan, "rowspan": rowspan});
        cell
    }

    fn table(rows: &[Vec<Value>]) -> Value {
        json!({"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"},
               "content": rows.iter().map(|r| json!({"type": "tableRow", "content": r})).collect::<Vec<_>>()})
    }

    fn doc(blocks: &[Value]) -> Value {
        json!({"type": "doc", "version": 1, "content": blocks})
    }

    fn simple() -> Value {
        doc(&[table(&[
            vec![cell("tableHeader", "a"), cell("tableHeader", "b")],
            vec![cell("tableCell", "1"), cell("tableCell", "2")],
        ])])
    }

    fn run(doc: &Value, opts: &AdfToCsvOptions) -> Conversion<String> {
        convert(doc, opts).unwrap()
    }

    fn error(doc: &Value, opts: &AdfToCsvOptions) -> Diagnostic {
        match convert(doc, opts) {
            Err(diagnostics) => diagnostics.0.into_iter().next().unwrap(),
            Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn a_simple_table() {
        let got = run(&simple(), &AdfToCsvOptions::default());
        assert_eq!(got.output, "a,b\n1,2\n");
        assert_eq!(got.warnings.len(), 0);
    }

    #[test]
    fn tables_are_numbered_in_document_order_depth_first() {
        let nested = json!({"type": "expand", "attrs": {"title": "x"}, "content": [
            table(&[vec![cell("tableCell", "inner")]])
        ]});
        let doc = doc(&[
            table(&[vec![cell("tableCell", "first")]]),
            nested,
            table(&[vec![cell("tableCell", "third")]]),
        ]);
        for (n, expected) in [(1, "first\n"), (2, "inner\n"), (3, "third\n")] {
            let opts = AdfToCsvOptions {
                table: n,
                ..AdfToCsvOptions::default()
            };
            assert_eq!(run(&doc, &opts).output, expected, "table {n}");
        }
    }

    #[test]
    fn a_missing_table_says_how_many_there_are() {
        let none = doc(&[json!({"type": "paragraph"})]);
        let e = error(&none, &AdfToCsvOptions::default());
        assert_eq!(e.code, Code::InputNoTable);
        assert!(e.message.contains("no table"));
        let opts = |table| AdfToCsvOptions {
            table,
            ..AdfToCsvOptions::default()
        };
        let e = error(&simple(), &opts(2));
        assert_eq!(e.code, Code::InputNoTable);
        assert!(e.message.contains("1 table"), "{}", e.message);
        let two = doc(&[
            table(&[vec![cell("tableCell", "a")]]),
            table(&[vec![cell("tableCell", "b")]]),
        ]);
        let e = error(&two, &opts(3));
        assert!(e.message.contains("2 tables"), "{}", e.message);
        let e = error(&two, &opts(0));
        assert_eq!(e.code, Code::InputNoTable);
    }

    #[test]
    fn input_that_is_not_a_document_fails() {
        let e = error(&json!({"type": "paragraph"}), &AdfToCsvOptions::default());
        assert_eq!(e.code, Code::InputNotAdf);
    }

    #[test]
    fn deep_documents_fail_without_recursing_forever() {
        let mut node = json!({"type": "paragraph"});
        for _ in 0..600 {
            node = json!({"type": "blockquote", "content": [node]});
        }
        let doc = doc(&[node]);
        let e = error(&doc, &AdfToCsvOptions::default());
        assert_eq!(e.code, Code::InputTooDeep);
        std::mem::forget(doc);
    }

    #[test]
    fn merged_cells_repeat_their_text_with_one_warning_at_the_table() {
        let doc = doc(&[
            json!({"type": "paragraph"}),
            table(&[
                vec![spanned("wide", 2, 1), cell("tableHeader", "c")],
                vec![
                    spanned("tall", 1, 2),
                    cell("tableCell", "x"),
                    cell("tableCell", "y"),
                ],
                vec![cell("tableCell", "z"), cell("tableCell", "w")],
            ]),
        ]);
        let got = run(&doc, &AdfToCsvOptions::default());
        assert_eq!(got.output, "wide,wide,c\ntall,x,y\ntall,z,w\n");
        assert_eq!(got.warnings.len(), 1);
        assert_eq!(got.warnings[0].code, Code::CsvMergedCells);
        let at = got.warnings[0].location.as_ref().unwrap();
        assert_eq!(at.pointer.as_deref(), Some("/content/1"));
    }

    #[test]
    fn ragged_rows_are_padded_with_empty_fields() {
        let doc = doc(&[table(&[
            vec![
                cell("tableCell", "a"),
                cell("tableCell", "b"),
                cell("tableCell", "c"),
            ],
            vec![cell("tableCell", "d")],
            vec![],
        ])]);
        assert_eq!(
            run(&doc, &AdfToCsvOptions::default()).output,
            "a,b,c\nd,,\n,,\n"
        );
    }

    #[test]
    fn quoting_and_the_delimiter() {
        let doc = doc(&[table(&[vec![
            cell("tableCell", "a,b"),
            cell("tableCell", "say \"hi\""),
            cell("tableCell", " pad "),
            cell("tableCell", "x;y"),
            cell("tableCell", ""),
        ]])]);
        assert_eq!(
            run(&doc, &AdfToCsvOptions::default()).output,
            "\"a,b\",\"say \"\"hi\"\"\",\" pad \",x;y,\n"
        );
        let semicolon = AdfToCsvOptions {
            delimiter: CsvDelimiter::parse("semicolon").unwrap(),
            ..AdfToCsvOptions::default()
        };
        assert_eq!(
            run(&doc, &semicolon).output,
            "a,b;\"say \"\"hi\"\"\";\" pad \";\"x;y\";\n"
        );
    }

    #[test]
    fn csv_safe_prefixes_formulas_only_when_asked() {
        let doc = doc(&[table(&[vec![
            cell("tableCell", "=1+1"),
            cell("tableCell", "-5"),
            cell("tableCell", "ok"),
        ]])]);
        assert_eq!(
            run(&doc, &AdfToCsvOptions::default()).output,
            "=1+1,-5,ok\n"
        );
        let safe = AdfToCsvOptions {
            csv_safe: true,
            ..AdfToCsvOptions::default()
        };
        assert_eq!(run(&doc, &safe).output, "'=1+1,'-5,ok\n");
    }

    #[test]
    fn control_characters_are_removed_with_a_warning_at_the_cell() {
        let doc = doc(&[table(&[vec![
            cell("tableCell", "a\u{1}b"),
            cell("tableCell", "c"),
        ]])]);
        let got = run(&doc, &AdfToCsvOptions::default());
        assert_eq!(got.output, "ab,c\n");
        assert_eq!(got.warnings.len(), 1);
        assert_eq!(got.warnings[0].code, Code::ControlChar);
        assert_eq!(
            got.warnings[0]
                .location
                .as_ref()
                .unwrap()
                .pointer
                .as_deref(),
            Some("/content/0/content/0/content/0")
        );
    }

    #[test]
    fn a_table_without_rows_is_one_empty_record() {
        let doc = doc(&[table(&[])]);
        assert_eq!(run(&doc, &AdfToCsvOptions::default()).output, "\n");
    }

    #[test]
    fn cells_without_content_are_empty_fields() {
        let doc = doc(&[table(&[vec![
            json!({"type": "tableCell"}),
            json!({"type": "tableHeader", "content": []}),
        ]])]);
        assert_eq!(run(&doc, &AdfToCsvOptions::default()).output, ",\n");
    }

    #[test]
    fn the_output_always_ends_with_a_line_feed_and_has_no_bom() {
        let out = run(&simple(), &AdfToCsvOptions::default()).output;
        assert!(out.ends_with('\n'));
        assert!(!out.starts_with('\u{feff}'));
    }

    #[test]
    fn oversized_grids_fail() {
        let doc = doc(&[table(&[vec![spanned("a", 100_001, 1)]])]);
        assert_eq!(
            error(&doc, &AdfToCsvOptions::default()).code,
            Code::InputTooLarge
        );
    }

    #[test]
    fn repeated_text_is_bounded_by_the_output_budget() {
        // Only a huge merged cell can reach the cap.
        let big = "x".repeat(1 << 19);
        let mut rows = vec![vec![spanned(&big, 100, 1000)]];
        rows.extend((1..1000).map(|_| vec![]));
        let doc = doc(&[table(&rows)]);
        // 100 x 1000 = 100,000 positions x 512 KiB would be 50 GiB: it must stop at 64 MiB.
        let e = error(&doc, &AdfToCsvOptions::default());
        assert_eq!(e.code, Code::InputTooLarge);
        assert!(e.message.contains("64 MiB"));
    }
}
