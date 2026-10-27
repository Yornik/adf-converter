//! The rectangular grid of an ADF table: `colspan` and `rowspan` expanded, so that every
//! position says which cell covers it.
//!
//! Cells are placed the way HTML does it: a cell goes to the first position of its row that no
//! earlier cell covers. A span that would run into a position another cell already covers is cut
//! short there (nothing is overwritten), and a `rowspan` past the last row is cut at the table's
//! end. Every position is written at most once, so the work is linear in the size of the grid,
//! which is capped at [`MAX_TABLE_CELLS`].

use crate::adf::reader::Node;
use crate::diagnostics::{Code, Diagnostic, Location};
use crate::limits::MAX_TABLE_CELLS;

/// A table as a grid.
#[derive(Debug)]
pub(super) struct Grid<'r, 'a> {
    /// The cells, in document order.
    pub cells: Vec<&'r Node<'a>>,
    /// Per row, per column: the index into `cells` of the cell covering the position, or `None`
    /// where no cell does. Rows may be shorter than `width`.
    pub rows: Vec<Vec<Option<usize>>>,
    /// The widest row.
    pub width: usize,
    /// Some cell covers more than one position.
    pub merged: bool,
}

impl Grid<'_, '_> {
    /// The index of the cell covering `(row, column)`.
    pub(super) fn cell_at(&self, row: usize, column: usize) -> Option<usize> {
        self.rows.get(row)?.get(column).copied().flatten()
    }
}

fn is_cell(node: &Node<'_>) -> bool {
    matches!(node.ty, "tableCell" | "tableHeader")
}

/// A `colspan` or `rowspan` attribute: a whole number of at least 1 (1 when it is missing or not
/// such a number).
fn span(cell: &Node<'_>, name: &str) -> usize {
    cell.attr(name)
        .and_then(serde_json::Value::as_u64)
        .map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX).max(1))
}

/// Builds the grid of `table`'s rows. Fails with `E_INPUT_TOO_LARGE` when it would hold more than
/// [`MAX_TABLE_CELLS`] positions.
pub(super) fn build<'r, 'a>(table: &'r Node<'a>) -> Result<Grid<'r, 'a>, Diagnostic> {
    let row_nodes: Vec<&Node<'a>> = table
        .content
        .iter()
        .filter(|n| n.ty == "tableRow")
        .collect();
    let mut grid = Grid {
        cells: Vec::new(),
        rows: vec![Vec::new(); row_nodes.len()],
        width: 0,
        merged: false,
    };
    let max_width = MAX_TABLE_CELLS / row_nodes.len().max(1);
    for (row, row_node) in row_nodes.iter().enumerate() {
        let mut column = 0;
        for cell in row_node.content.iter().filter(|n| is_cell(n)) {
            while grid.is_covered(row, column) {
                column += 1;
            }
            let columns =
                grid.free_columns(row, column, span(cell, "colspan"), max_width, table)?;
            let rows = grid.free_rows(row, column, columns, span(cell, "rowspan"));
            grid.place(cell, row, column, columns, rows);
            column += columns;
        }
    }
    grid.width = grid.rows.iter().map(Vec::len).max().unwrap_or(0);
    Ok(grid)
}

impl<'r, 'a> Grid<'r, 'a> {
    fn is_covered(&self, row: usize, column: usize) -> bool {
        self.cell_at(row, column).is_some()
    }

    /// How many columns a cell at `(row, column)` covers: its `colspan`, cut short at the first
    /// position of its row that is already covered. Fails when that reaches past `max_width`.
    fn free_columns(
        &self,
        row: usize,
        column: usize,
        colspan: usize,
        max_width: usize,
        table: &Node<'_>,
    ) -> Result<usize, Diagnostic> {
        let too_large = || {
            Diagnostic::new(
                Code::InputTooLarge,
                format!("The table would expand to more than {MAX_TABLE_CELLS} cells"),
            )
            .at(Location::pointer(table.pointer()))
            .with_hint("A colspan or rowspan is far larger than the table")
        };
        if column >= max_width {
            return Err(too_large());
        }
        let mut columns = 1;
        while columns < colspan && !self.is_covered(row, column + columns) {
            columns += 1;
            if column + columns > max_width {
                return Err(too_large());
            }
        }
        Ok(columns)
    }

    /// How many rows a cell covers: its `rowspan`, cut short at the last row and at the first row
    /// where one of the cell's columns is already covered.
    fn free_rows(&self, row: usize, column: usize, columns: usize, rowspan: usize) -> usize {
        let mut rows = 1;
        while rows < rowspan
            && row + rows < self.rows.len()
            && (column..column + columns).all(|c| !self.is_covered(row + rows, c))
        {
            rows += 1;
        }
        rows
    }

    fn place(
        &mut self,
        cell: &'r Node<'a>,
        row: usize,
        column: usize,
        columns: usize,
        rows: usize,
    ) {
        let index = self.cells.len();
        self.cells.push(cell);
        self.merged |= columns > 1 || rows > 1;
        for covered in self.rows.iter_mut().skip(row).take(rows) {
            if covered.len() < column + columns {
                covered.resize(column + columns, None);
            }
            for slot in covered.iter_mut().skip(column).take(columns) {
                *slot = Some(index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adf::reader::read_doc;
    use crate::limits::Limits;
    use serde_json::{Value, json};

    /// A cell with `name` as its text and the given spans.
    fn cell(name: &str, colspan: u64, rowspan: u64) -> Value {
        json!({"type": "tableCell", "attrs": {"colspan": colspan, "rowspan": rowspan},
               "content": [{"type": "paragraph", "content": [{"type": "text", "text": name}]}]})
    }

    fn row(cells: &[Value]) -> Value {
        json!({"type": "tableRow", "content": cells})
    }

    /// The grid as rows of cell names (`.` for positions nobody covers), plus the merged flag.
    fn layout(rows: &[Value]) -> Result<(Vec<String>, bool), Diagnostic> {
        let doc = json!({"type": "doc", "content": [{"type": "table", "content": rows}]});
        let root = read_doc(&doc, &Limits::default()).unwrap();
        let grid = build(&root.content[0])?;
        let names: Vec<String> = grid
            .cells
            .iter()
            .map(|c| c.content[0].content[0].text.unwrap().to_owned())
            .collect();
        let lines = (0..grid.rows.len())
            .map(|r| {
                (0..grid.width)
                    .map(|c| grid.cell_at(r, c).map_or(".", |i| &names[i]))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect();
        Ok((lines, grid.merged))
    }

    fn lines(rows: &[Value]) -> Vec<String> {
        layout(rows).unwrap().0
    }

    #[test]
    fn a_plain_table() {
        let got = layout(&[
            row(&[cell("a", 1, 1), cell("b", 1, 1)]),
            row(&[cell("c", 1, 1), cell("d", 1, 1)]),
        ])
        .unwrap();
        assert_eq!(got, (vec!["ab".to_owned(), "cd".to_owned()], false));
    }

    #[test]
    fn colspan_covers_columns() {
        let got = layout(&[
            row(&[cell("a", 2, 1), cell("b", 1, 1)]),
            row(&[cell("c", 1, 1), cell("d", 1, 1), cell("e", 1, 1)]),
        ])
        .unwrap();
        assert_eq!(got, (vec!["aab".to_owned(), "cde".to_owned()], true));
    }

    #[test]
    fn rowspan_covers_rows_and_pushes_later_cells_right() {
        assert_eq!(
            lines(&[
                row(&[cell("a", 1, 2), cell("b", 1, 1)]),
                row(&[cell("c", 1, 1)]),
            ]),
            ["ab", "ac"]
        );
    }

    #[test]
    fn colspan_and_rowspan_together() {
        assert_eq!(
            lines(&[
                row(&[cell("a", 2, 2), cell("b", 1, 1)]),
                row(&[cell("c", 1, 1)]),
                row(&[cell("d", 1, 1), cell("e", 1, 1), cell("f", 1, 1)]),
            ]),
            ["aab", "aac", "def"]
        );
    }

    #[test]
    fn short_rows_leave_positions_empty() {
        assert_eq!(
            lines(&[
                row(&[cell("a", 1, 1), cell("b", 1, 1), cell("c", 1, 1)]),
                row(&[cell("d", 1, 1)]),
                row(&[]),
            ]),
            ["abc", "d..", "..."]
        );
    }

    #[test]
    fn a_rowspan_past_the_last_row_is_cut() {
        assert_eq!(lines(&[row(&[cell("a", 1, 5), cell("b", 1, 1)])]), ["ab"]);
        let (_, merged) = layout(&[row(&[cell("a", 1, 5)])]).unwrap();
        assert!(!merged, "nothing is actually repeated");
    }

    #[test]
    fn overlapping_spans_never_overwrite() {
        // `x` covers column 2 of rows 0-2. `y` (row 1, column 0) wants 4 columns but is cut at `x`.
        assert_eq!(
            lines(&[
                row(&[cell("a", 1, 1), cell("b", 1, 1), cell("x", 1, 3)]),
                row(&[cell("y", 4, 1)]),
                row(&[cell("c", 1, 1)]),
            ]),
            ["abx", "yyx", "c.x"]
        );
        // Cells of later rows still find their positions free below a short rowspan.
        assert_eq!(
            lines(&[
                row(&[cell("a", 1, 1), cell("b", 1, 1)]),
                row(&[cell("c", 1, 1), cell("d", 1, 2)]),
                row(&[cell("e", 1, 1)]),
            ]),
            ["ab", "cd", "ed"]
        );
    }

    #[test]
    fn a_wide_cell_widens_its_row() {
        assert_eq!(
            lines(&[row(&[cell("a", 3, 1)]), row(&[cell("b", 1, 1)]),]),
            ["aaa", "b.."]
        );
    }

    #[test]
    fn bad_spans_count_as_one() {
        let bad = |value: Value| {
            json!({"type": "tableCell", "attrs": {"colspan": value, "rowspan": "2"},
                   "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]})
        };
        for value in [json!(0), json!(-3), json!("2"), json!(1.5), json!(null)] {
            assert_eq!(lines(&[row(&[bad(value)])]), ["a"]);
        }
    }

    #[test]
    fn other_children_are_skipped_and_empty_tables_are_empty() {
        let rows = vec![
            json!({"type": "paragraph"}),
            row(&[json!({"type": "weird"}), cell("a", 1, 1)]),
        ];
        assert_eq!(lines(&rows), ["a"]);
        assert_eq!(lines(&[]).len(), 0);
    }

    #[test]
    fn the_limit_is_inclusive() {
        // 100,000 positions in one row, and in 100 rows of 1,000.
        assert!(layout(&[row(&[cell("a", 100_000, 1)])]).is_ok());
        let e = layout(&[row(&[cell("a", 100_001, 1)])]).unwrap_err();
        assert_eq!(e.code, Code::InputTooLarge);
        assert_eq!(e.location.unwrap().pointer.as_deref(), Some("/content/0"));
        let mut rows = vec![row(&[cell("a", 1000, 1)])];
        rows.extend((1..100).map(|_| row(&[])));
        assert!(layout(&rows).is_ok());
        rows[0] = row(&[cell("a", 1001, 1)]);
        assert_eq!(layout(&rows).unwrap_err().code, Code::InputTooLarge);
    }

    #[test]
    fn huge_spans_fail_fast() {
        let e = layout(&[row(&[cell("a", u64::MAX, u64::MAX)])]).unwrap_err();
        assert_eq!(e.code, Code::InputTooLarge);
        // A huge rowspan is just cut at the table's end.
        assert_eq!(
            lines(&[row(&[cell("a", 1, u64::MAX)]), row(&[])]),
            ["a", "a"]
        );
    }
}
