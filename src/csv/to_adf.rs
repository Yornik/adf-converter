//! CSV → ADF: records from the reader become the rows of one table.

use crate::Conversion;
use crate::adf::model::{
    Align, AlignedParagraph, CellKind, Doc, DocBlock, Inline, NonEmpty, Table, TableCell,
    TableCellChild, TableRow, Text,
};
use crate::diagnostics::{Code, Diagnostic, Diagnostics};
use crate::limits::{MAX_OUTPUT_BYTES, MAX_TABLE_CELLS};
use crate::options::CsvToAdfOptions;

use super::numbers::is_number;
use super::parse::{Positions, Reader, Record, normalize};
use super::text;

/// Estimated bytes of JSON for a cell without content, and for the alignment mark.
const CELL_JSON_BYTES: usize = 60;
const ALIGN_JSON_BYTES: usize = 80;

/// Converts CSV text to an ADF document holding one table.
pub(super) fn convert(csv: &str, opts: &CsvToAdfOptions) -> Result<Conversion<Doc>, Diagnostics> {
    let limit = opts
        .limits
        .max_markdown_bytes
        .min(opts.limits.max_input_bytes);
    if csv.len() > limit {
        return Err(Diagnostics::single(
            Diagnostic::new(
                Code::InputTooLarge,
                format!(
                    "The CSV input is {} bytes, larger than the {limit}-byte limit",
                    csv.len()
                ),
            )
            .with_hint("Split the file, or keep fewer rows"),
        ));
    }
    let text = normalize(csv);
    let mut positions = Positions::new(&text);
    let mut builder = Builder::new(opts);
    let mut reader = Reader::new(&text, opts.delimiter.as_char());
    loop {
        match reader.next_record() {
            Ok(Some(record)) => {
                if let Err(error) = builder.accept(&record, &mut positions) {
                    return Err(failure(error, &builder));
                }
            }
            Ok(None) => break,
            Err(open) => {
                let (line, column) = positions.line_col(open);
                return Err(failure(
                    Diagnostic::new(
                        Code::InputCsv,
                        format!("The quoted field that starts at line {line}, column {column} is never closed"),
                    )
                    .at(positions.location(open))
                    .with_hint("Close the field with a double quote, or double a quote that is part of the text (\"\")"),
                    &builder,
                ));
            }
        }
    }
    builder.finish(&mut positions)
}

/// An error plus the warnings collected so far.
fn failure(error: Diagnostic, builder: &Builder<'_>) -> Diagnostics {
    let mut all = vec![error];
    all.extend(builder.warnings.iter().cloned());
    Diagnostics(all)
}

/// One cell: its inline content.
type Cell = Vec<Inline>;

/// Whether the cells of a column look like numbers: at least one non-empty data cell, and every
/// non-empty data cell matches.
#[derive(Debug, Clone, Copy)]
struct Column {
    seen_value: bool,
    all_numbers: bool,
}

impl Default for Column {
    fn default() -> Self {
        Self {
            seen_value: false,
            all_numbers: true,
        }
    }
}

impl Column {
    fn is_numeric(self) -> bool {
        self.seen_value && self.all_numbers
    }
}

/// Collects the records of one input.
#[derive(Debug)]
struct Builder<'o> {
    opts: &'o CsvToAdfOptions,
    /// The header row, once read (or never, without a header).
    header: Option<Vec<Cell>>,
    /// The kept data rows.
    rows: Vec<Vec<Cell>>,
    /// The widest record seen, whether kept or not.
    width: usize,
    columns: Vec<Column>,
    /// Data rows beyond `max_rows`: where the first one starts, and how many there were.
    dropped: Option<(usize, usize)>,
    estimated_bytes: usize,
    warnings: Vec<Diagnostic>,
}

impl<'o> Builder<'o> {
    fn new(opts: &'o CsvToAdfOptions) -> Self {
        Self {
            opts,
            header: None,
            rows: Vec::new(),
            width: 0,
            columns: Vec::new(),
            dropped: None,
            estimated_bytes: 0,
            warnings: Vec::new(),
        }
    }

    /// Takes one record: skips blank ones, checks the limits, and keeps or counts the rest.
    fn accept(&mut self, record: &Record, positions: &mut Positions<'_>) -> Result<(), Diagnostic> {
        if record.is_blank() {
            return Ok(());
        }
        let is_header = self.opts.header && self.header.is_none();
        let keep = is_header || self.opts.max_rows.is_none_or(|max| self.rows.len() < max);
        self.width = self.width.max(record.fields.len());
        let kept_rows = self.rows.len() + usize::from(keep && !is_header);
        // The table has one header row and the kept data rows.
        if self.width.saturating_mul(kept_rows + 1) > MAX_TABLE_CELLS {
            return Err(self.too_large(record, kept_rows + 1, positions));
        }
        if !keep {
            self.dropped = Some(match self.dropped {
                Some((first, count)) => (first, count + 1),
                None => (record.start, 1),
            });
            return Ok(());
        }
        let cells = self.cells(record, is_header, positions);
        if self.estimated_bytes > MAX_OUTPUT_BYTES {
            return Err(Diagnostic::new(
                Code::InputTooLarge,
                format!(
                    "The ADF output would be larger than the {} MiB limit",
                    MAX_OUTPUT_BYTES / (1024 * 1024)
                ),
            )
            .at(positions.location(record.start))
            .with_hint("Use --max-rows to keep fewer rows, or split the file"));
        }
        if is_header {
            self.header = Some(cells);
        } else {
            self.rows.push(cells);
        }
        Ok(())
    }

    fn too_large(&self, record: &Record, rows: usize, positions: &mut Positions<'_>) -> Diagnostic {
        Diagnostic::new(
            Code::InputTooLarge,
            format!(
                "The table would have {} columns and {rows} rows, more than the limit of {MAX_TABLE_CELLS} cells",
                self.width
            ),
        )
        .at(positions.location(record.start))
        .with_hint("Use --max-rows to keep fewer rows, or split the file")
    }

    /// Converts the fields of a kept record; tracks numeric columns for data rows.
    fn cells(
        &mut self,
        record: &Record,
        is_header: bool,
        positions: &mut Positions<'_>,
    ) -> Vec<Cell> {
        let mut cells = Vec::with_capacity(record.fields.len());
        for (index, field) in record.fields.iter().enumerate() {
            let (clean, removed) = text::clean(&field.text);
            if removed {
                self.warnings.push(
                    Diagnostic::new(
                        Code::ControlChar,
                        "Control characters were removed from a cell",
                    )
                    .at(positions.location(field.start)),
                );
            }
            if self.opts.align_numbers && !is_header {
                self.track_column(index, &clean);
            }
            let inlines = text::inlines(&clean);
            self.estimated_bytes = self
                .estimated_bytes
                .saturating_add(CELL_JSON_BYTES + ALIGN_JSON_BYTES)
                .saturating_add(text::estimated_json_bytes(&inlines));
            cells.push(inlines);
        }
        cells
    }

    fn track_column(&mut self, index: usize, clean: &str) {
        if self.columns.len() <= index {
            self.columns.resize(index + 1, Column::default());
        }
        if clean.is_empty() {
            return;
        }
        if let Some(column) = self.columns.get_mut(index) {
            column.seen_value = true;
            column.all_numbers &= is_number(clean);
        }
    }

    /// Builds the document; adds `W_CSV_TRUNCATED` when rows were left out.
    fn finish(mut self, positions: &mut Positions<'_>) -> Result<Conversion<Doc>, Diagnostics> {
        if self.header.is_none() && self.rows.is_empty() && self.dropped.is_none() {
            return Err(Diagnostics::single(
                Diagnostic::new(Code::InputCsv, "The CSV has no rows")
                    .with_hint("Pass at least one row that is not blank"),
            ));
        }
        if let Some((first, count)) = self.dropped {
            let kept = self.rows.len();
            self.warnings.push(
                Diagnostic::new(
                    Code::CsvTruncated,
                    format!("Kept {kept} of {} data rows", kept + count),
                )
                .at(positions.location(first))
                .with_hint("Remove --max-rows to keep every row"),
            );
        }
        let table = self.table();
        Ok(Conversion {
            output: Doc {
                content: vec![DocBlock::Table(table)],
            },
            warnings: self.warnings,
        })
    }

    /// The header (generated without one), padded to the width, and the data rows.
    fn table(&mut self) -> Table {
        let width = self.width;
        let header = self
            .header
            .take()
            .unwrap_or_else(|| generated_header(width));
        let aligned: Vec<bool> = (0..width)
            .map(|i| self.opts.align_numbers && self.columns.get(i).is_some_and(|c| c.is_numeric()))
            .collect();
        let mut rows = vec![build_row(header, CellKind::Header, &aligned)];
        rows.extend(
            std::mem::take(&mut self.rows)
                .into_iter()
                .map(|cells| build_row(cells, CellKind::Data, &aligned)),
        );
        Table {
            rows: NonEmpty::from_vec(rows).unwrap_or_else(|| {
                NonEmpty::new(build_row(Vec::new(), CellKind::Header, &aligned))
            }),
        }
    }
}

/// `Column 1`, `Column 2`, …
fn generated_header(width: usize) -> Vec<Cell> {
    (1..=width)
        .map(|i| {
            Text::plain(format!("Column {i}"))
                .map(Inline::Text)
                .into_iter()
                .collect()
        })
        .collect()
}

/// A row of `aligned.len()` cells: `cells` padded with empty ones. Numeric columns are aligned
/// to the end.
fn build_row(cells: Vec<Cell>, kind: CellKind, aligned: &[bool]) -> TableRow {
    let mut cells = cells.into_iter();
    let built: Vec<TableCell> = aligned
        .iter()
        .map(|&right| {
            let content = cells.next().unwrap_or_default();
            TableCell {
                kind,
                content: NonEmpty::new(TableCellChild::Paragraph(AlignedParagraph {
                    content,
                    align: right.then_some(Align::End),
                })),
            }
        })
        .collect();
    TableRow {
        cells: NonEmpty::from_vec(built).unwrap_or_else(|| NonEmpty::new(TableCell::empty(kind))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::CsvDelimiter;
    use serde_json::{Value, json};

    fn convert_ok(csv: &str, opts: &CsvToAdfOptions) -> (Value, Vec<Diagnostic>) {
        let conversion = convert(csv, opts).unwrap();
        (conversion.output.to_value().unwrap(), conversion.warnings)
    }

    fn error(csv: &str, opts: &CsvToAdfOptions) -> Diagnostic {
        convert(csv, opts).unwrap_err().0.remove(0)
    }

    /// The text of every cell, row by row, with hard breaks as `\n`.
    fn texts(doc: &Value) -> Vec<Vec<String>> {
        let rows = doc["content"][0]["content"].as_array().unwrap();
        rows.iter()
            .map(|row| {
                row["content"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|cell| {
                        let paragraphs = cell["content"].as_array().unwrap();
                        assert_eq!(paragraphs.len(), 1);
                        paragraphs[0]["content"]
                            .as_array()
                            .map(|inlines| {
                                inlines
                                    .iter()
                                    .map(|i| i["text"].as_str().unwrap_or("\n"))
                                    .collect::<String>()
                            })
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .collect()
    }

    fn grid(csv: &str) -> Vec<Vec<String>> {
        texts(&convert_ok(csv, &CsvToAdfOptions::default()).0)
    }

    fn strs(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|r| r.iter().map(|c| (*c).to_owned()).collect())
            .collect()
    }

    #[test]
    fn header_and_data_cells() {
        let (doc, warnings) = convert_ok("a,b\n1,2\n", &CsvToAdfOptions::default());
        assert_eq!(warnings.len(), 0);
        let rows = doc["content"][0]["content"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["content"][0]["type"], "tableHeader");
        assert_eq!(rows[1]["content"][1]["type"], "tableCell");
        assert_eq!(doc["content"][0]["attrs"]["isNumberColumnEnabled"], false);
        assert_eq!(doc["content"][0]["attrs"]["layout"], "default");
        assert_eq!(doc["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn cell_text_is_trimmed_and_empty_cells_have_an_empty_paragraph() {
        let (doc, _) = convert_ok(" a ,\"\t b \n\" ,\n", &CsvToAdfOptions::default());
        assert_eq!(texts(&doc), strs(&[&["a", "b", ""]]));
        let cell = &doc["content"][0]["content"][0]["content"][2];
        assert_eq!(cell["content"], json!([{"type": "paragraph"}]));
    }

    #[test]
    fn short_rows_are_padded_and_no_cell_is_dropped() {
        assert_eq!(
            grid("a,b,c\n1\n1,2,3,4\n"),
            strs(&[
                &["a", "b", "c", ""],
                &["1", "", "", ""],
                &["1", "2", "3", "4"]
            ])
        );
    }

    #[test]
    fn blank_records_are_skipped() {
        assert_eq!(
            grid("\n a ,b\n\n , \n\t\n1,2\n\n"),
            strs(&[&["a", "b"], &["1", "2"]])
        );
    }

    #[test]
    fn no_rows_is_an_error() {
        for csv in ["", "\n", " \n,\n\t,  \n", "\u{feff}", "\r\n"] {
            let e = error(csv, &CsvToAdfOptions::default());
            assert_eq!(e.code, Code::InputCsv, "{csv:?}");
            assert!(e.message.contains("no rows"));
        }
    }

    #[test]
    fn an_unterminated_quote_is_located_at_the_opening_quote() {
        let e = error("a,b\nc,\"d\ne,f\n", &CsvToAdfOptions::default());
        assert_eq!(e.code, Code::InputCsv);
        let at = e.location.unwrap();
        assert_eq!((at.line, at.column), (Some(2), Some(3)));
        // Columns count characters, and a BOM and CRLF do not shift anything.
        let e = error("\u{feff}é\"x\r\né,\"y", &CsvToAdfOptions::default());
        let at = e.location.unwrap();
        assert_eq!((at.line, at.column), (Some(2), Some(3)));
    }

    #[test]
    fn warnings_before_the_error_are_returned_with_it() {
        let diagnostics = convert("a\n\u{1}b\n\"x", &CsvToAdfOptions::default()).unwrap_err();
        assert_eq!(diagnostics.0[0].code, Code::InputCsv);
        assert_eq!(diagnostics.0[1].code, Code::ControlChar);
    }

    #[test]
    fn no_header_generates_column_names_and_keeps_every_record() {
        let opts = CsvToAdfOptions {
            header: false,
            ..CsvToAdfOptions::default()
        };
        let (doc, _) = convert_ok("a,b\n1,2,3\n", &opts);
        assert_eq!(
            texts(&doc),
            strs(&[
                &["Column 1", "Column 2", "Column 3"],
                &["a", "b", ""],
                &["1", "2", "3"]
            ])
        );
        let first = &doc["content"][0]["content"][0]["content"][0];
        assert_eq!(first["type"], "tableHeader");
    }

    #[test]
    fn a_header_only_input_is_a_header_only_table() {
        assert_eq!(grid("a,b\n"), strs(&[&["a", "b"]]));
    }

    #[test]
    fn line_breaks_in_a_cell_are_hard_breaks() {
        let (doc, _) = convert_ok("\"a\r\nb\rc\"\n", &CsvToAdfOptions::default());
        let content = &doc["content"][0]["content"][0]["content"][0]["content"][0]["content"];
        assert_eq!(
            *content,
            json!([
                {"type": "text", "text": "a"}, {"type": "hardBreak"},
                {"type": "text", "text": "b"}, {"type": "hardBreak"},
                {"type": "text", "text": "c"}
            ])
        );
    }

    #[test]
    fn control_characters_are_removed_once_per_cell_at_its_position() {
        let (doc, warnings) = convert_ok(
            "ok,x\n\u{1}a\u{2},\"b\n\0c\"\n",
            &CsvToAdfOptions::default(),
        );
        assert_eq!(texts(&doc), strs(&[&["ok", "x"], &["a", "b\nc"]]));
        let places: Vec<_> = warnings
            .iter()
            .map(|w| {
                let at = w.location.as_ref().unwrap();
                (w.code, at.line.unwrap(), at.column.unwrap())
            })
            .collect();
        assert_eq!(
            places,
            [(Code::ControlChar, 2, 1), (Code::ControlChar, 2, 5)]
        );
    }

    #[test]
    fn a_control_character_only_cell_becomes_empty_with_a_warning() {
        let (doc, warnings) = convert_ok("a,b\n\0,\u{7f}\n", &CsvToAdfOptions::default());
        assert_eq!(texts(&doc), strs(&[&["a", "b"], &["", ""]]));
        assert_eq!(warnings.len(), 2);
    }

    #[test]
    fn other_delimiters() {
        for (name, csv) in [
            ("tab", "a\tb\n1\t2,3\n"),
            ("semicolon", "a;b\n1;2,3\n"),
            ("pipe", "a|b\n1|2,3\n"),
        ] {
            let opts = CsvToAdfOptions {
                delimiter: CsvDelimiter::parse(name).unwrap(),
                ..CsvToAdfOptions::default()
            };
            assert_eq!(
                texts(&convert_ok(csv, &opts).0),
                strs(&[&["a", "b"], &["1", "2,3"]]),
                "{name}"
            );
        }
    }

    fn numeric_alignment(csv: &str) -> Vec<bool> {
        let opts = CsvToAdfOptions {
            align_numbers: true,
            ..CsvToAdfOptions::default()
        };
        let (doc, _) = convert_ok(csv, &opts);
        let header = doc["content"][0]["content"][0]["content"]
            .as_array()
            .unwrap();
        header
            .iter()
            .map(|cell| cell["content"][0]["marks"][0]["attrs"]["align"] == "end")
            .collect()
    }

    #[test]
    fn numeric_columns_are_right_aligned_header_included() {
        assert_eq!(
            numeric_alignment("name,qty,price\nx,3,\"1,250.50\"\ny,10,9.99\n"),
            [false, true, true]
        );
    }

    #[test]
    fn numeric_columns_need_a_value_and_only_numbers() {
        // Empty column, mixed column, header-only table, and empty cells that do not count.
        assert_eq!(
            numeric_alignment("a,b,c\n,1x,1\n,2,\n"),
            [false, false, true]
        );
        assert_eq!(numeric_alignment("a,b\n"), [false, false]);
        assert_eq!(numeric_alignment("1,2\n3,4\n"), [true, true]);
    }

    #[test]
    fn every_row_of_an_aligned_column_is_aligned_including_padding() {
        let opts = CsvToAdfOptions {
            align_numbers: true,
            ..CsvToAdfOptions::default()
        };
        let (doc, _) = convert_ok("a,n\nx\ny,2\n", &opts);
        for row in doc["content"][0]["content"].as_array().unwrap() {
            assert_eq!(
                row["content"][1]["content"][0]["marks"][0]["attrs"]["align"],
                "end"
            );
            assert!(row["content"][0]["content"][0].get("marks").is_none());
        }
    }

    #[test]
    fn rows_dropped_by_max_rows_do_not_count_for_numbers() {
        let opts = CsvToAdfOptions {
            align_numbers: true,
            max_rows: Some(1),
            ..CsvToAdfOptions::default()
        };
        let (doc, _) = convert_ok("n\n1\nx\n", &opts);
        assert_eq!(
            doc["content"][0]["content"][0]["content"][0]["content"][0]["marks"][0]["attrs"]["align"],
            "end"
        );
    }

    #[test]
    fn max_rows_keeps_the_first_rows_and_warns_at_the_first_dropped_record() {
        let opts = CsvToAdfOptions {
            max_rows: Some(2),
            ..CsvToAdfOptions::default()
        };
        let (doc, warnings) = convert_ok("h\n1\n2\n\n3\n4\n", &opts);
        assert_eq!(texts(&doc), strs(&[&["h"], &["1"], &["2"]]));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, Code::CsvTruncated);
        assert_eq!(warnings[0].message, "Kept 2 of 4 data rows");
        let at = warnings[0].location.as_ref().unwrap();
        assert_eq!((at.line, at.column), (Some(5), Some(1)));
    }

    #[test]
    fn max_rows_that_keeps_everything_is_silent() {
        let opts = CsvToAdfOptions {
            max_rows: Some(2),
            ..CsvToAdfOptions::default()
        };
        let (doc, warnings) = convert_ok("h\n1\n2\n", &opts);
        assert_eq!(texts(&doc).len(), 3);
        assert_eq!(warnings.len(), 0);
    }

    #[test]
    fn max_rows_zero_gives_a_header_only_table() {
        let opts = CsvToAdfOptions {
            max_rows: Some(0),
            ..CsvToAdfOptions::default()
        };
        let (doc, warnings) = convert_ok("h,i\n1,2\n", &opts);
        assert_eq!(texts(&doc), strs(&[&["h", "i"]]));
        assert_eq!(warnings[0].message, "Kept 0 of 1 data rows");
    }

    #[test]
    fn width_counts_dropped_rows() {
        let opts = CsvToAdfOptions {
            max_rows: Some(1),
            ..CsvToAdfOptions::default()
        };
        let (doc, _) = convert_ok("a\n1\n2,3,4\n", &opts);
        assert_eq!(texts(&doc), strs(&[&["a", "", ""], &["1", "", ""]]));
    }

    #[test]
    fn max_rows_without_a_header_keeps_records() {
        let opts = CsvToAdfOptions {
            header: false,
            max_rows: Some(1),
            ..CsvToAdfOptions::default()
        };
        let (doc, warnings) = convert_ok("a\nb\nc\n", &opts);
        assert_eq!(texts(&doc), strs(&[&["Column 1"], &["a"]]));
        assert_eq!(warnings[0].message, "Kept 1 of 3 data rows");
    }

    #[test]
    fn truncation_is_reported_after_control_character_warnings() {
        let opts = CsvToAdfOptions {
            max_rows: Some(1),
            ..CsvToAdfOptions::default()
        };
        let (_, warnings) = convert_ok("h\n\u{1}a\n\u{1}b\n", &opts);
        let codes: Vec<_> = warnings.iter().map(|w| w.code).collect();
        assert_eq!(codes, [Code::ControlChar, Code::CsvTruncated]);
    }

    #[test]
    fn the_input_size_limit_is_inclusive() {
        let mut opts = CsvToAdfOptions::default();
        opts.limits.max_markdown_bytes = 7;
        assert!(convert("a,b\n1,2", &opts).is_ok());
        let e = error("a,b\n1,22", &opts);
        assert_eq!(e.code, Code::InputTooLarge);
        // The smaller of the two limits applies.
        opts.limits.max_markdown_bytes = 100;
        opts.limits.max_input_bytes = 7;
        assert_eq!(error("a,b\n1,22", &opts).code, Code::InputTooLarge);
    }

    fn wide(columns: usize, rows: usize) -> String {
        let line = vec!["x"; columns].join(",");
        vec![line; rows].join("\n")
    }

    #[test]
    fn the_cell_limit_is_inclusive_and_checked_on_the_record_that_crosses_it() {
        // 100,000 cells: ok. One more row: error at that row.
        let opts = CsvToAdfOptions::default();
        assert!(convert(&wide(1000, 100), &opts).is_ok());
        let e = error(&wide(1000, 101), &opts);
        assert_eq!(e.code, Code::InputTooLarge);
        assert_eq!(e.location.unwrap().line, Some(101));
        // One column too many on a single row.
        assert!(convert(&wide(100_000, 1), &opts).is_ok());
        let e = error(&wide(100_001, 1), &opts);
        assert_eq!(e.code, Code::InputTooLarge);
        assert_eq!(e.location.unwrap().line, Some(1));
    }

    #[test]
    fn without_a_header_the_generated_header_counts() {
        let opts = CsvToAdfOptions {
            header: false,
            ..CsvToAdfOptions::default()
        };
        assert!(convert(&wide(1000, 99), &opts).is_ok());
        assert_eq!(error(&wide(1000, 100), &opts).code, Code::InputTooLarge);
    }

    #[test]
    fn a_later_wider_row_can_cross_the_limit() {
        let mut csv = wide(1, 1000);
        csv.push('\n');
        csv.push_str(&wide(101, 1));
        let e = error(&csv, &CsvToAdfOptions::default());
        assert_eq!(e.code, Code::InputTooLarge);
        assert_eq!(e.location.unwrap().line, Some(1001));
    }

    #[test]
    fn dropped_rows_count_for_the_width_but_not_the_row_count() {
        let opts = CsvToAdfOptions {
            max_rows: Some(0),
            ..CsvToAdfOptions::default()
        };
        // Header + 1 row would be fine, 500,000 dropped rows never cross the limit.
        let csv = format!("a\n{}", "1\n".repeat(50_000));
        assert!(convert(&csv, &opts).is_ok());
        // A dropped row that is wide enough crosses it with the header row alone.
        let csv = format!("a\n1\n{}", wide(100_001, 1));
        assert_eq!(error(&csv, &opts).code, Code::InputTooLarge);
    }

    #[test]
    fn blank_wide_records_do_not_count() {
        let csv = format!("a\n{}\n1\n", ",".repeat(200_000 / 2));
        let (doc, _) = convert_ok(&csv, &CsvToAdfOptions::default());
        assert_eq!(texts(&doc), strs(&[&["a"], &["1"]]));
    }
}
