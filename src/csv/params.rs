//! The options of an embedded table: the info string of a ```` ```csv ```` fence and the query of
//! an `adf:csv/PATH?…` include take the same keys (issue #13, phases 2 and 3).
//!
//! | key         | value                                                   |
//! |-------------|---------------------------------------------------------|
//! | `delimiter` | `comma`, `tab`, `semicolon`, `pipe` or one ASCII character |
//! | `header`    | `true` (default) or `false`                              |
//! | `max-rows`  | a whole number of at least 1                             |
//! | `align`     | `numbers` (right-align numeric columns) or `none`        |

use crate::Conversion;
use crate::adf::model::{DocBlock, Table};
use crate::diagnostics::{Code, Diagnostic, Diagnostics};
use crate::limits::Limits;
use crate::options::{CsvDelimiter, CsvToAdfOptions, Target};

/// Options given with an embedded table; `None` keeps the default of [`CsvToAdfOptions`] (or, for
/// a `tsv` fence, a tab).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CsvParams {
    pub delimiter: Option<CsvDelimiter>,
    pub header: Option<bool>,
    pub max_rows: Option<usize>,
    pub align_numbers: Option<bool>,
}

impl CsvParams {
    /// Parses `key`/`value` pairs. An unknown key, a repeated key or an invalid value is an error
    /// whose message names the key; the caller turns it into `E_CSV_OPTION` at its own location.
    pub(crate) fn from_pairs<'a>(
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, String> {
        let mut params = Self::default();
        let mut seen: Vec<&str> = Vec::new();
        for (key, value) in pairs {
            if seen.contains(&key) {
                return Err(format!("the option '{key}' is given twice"));
            }
            seen.push(key);
            match key {
                "delimiter" => {
                    params.delimiter = Some(
                        CsvDelimiter::parse(value).map_err(|why| format!("delimiter: {why}"))?,
                    );
                }
                "header" => params.header = Some(parse_bool(key, value)?),
                "max-rows" => match value.parse::<usize>() {
                    Ok(n) if n >= 1 => params.max_rows = Some(n),
                    _ => return Err("max-rows: expected a whole number of at least 1".to_owned()),
                },
                "align" => match value {
                    "numbers" => params.align_numbers = Some(true),
                    "none" => params.align_numbers = Some(false),
                    _ => return Err("align: expected numbers or none".to_owned()),
                },
                other => {
                    let shown: String = other.escape_debug().take(20).collect();
                    return Err(format!(
                        "unknown option '{shown}'; expected delimiter, header, max-rows or align"
                    ));
                }
            }
        }
        Ok(params)
    }

    /// The conversion options: the defaults, then `default_delimiter` (a `tsv` fence), then these
    /// parameters.
    pub(crate) fn options(
        &self,
        target: Target,
        limits: &Limits,
        default_delimiter: Option<CsvDelimiter>,
    ) -> CsvToAdfOptions {
        let base = CsvToAdfOptions::default();
        CsvToAdfOptions {
            target,
            delimiter: self
                .delimiter
                .or(default_delimiter)
                .unwrap_or(base.delimiter),
            header: self.header.unwrap_or(base.header),
            max_rows: self.max_rows,
            align_numbers: self.align_numbers.unwrap_or(base.align_numbers),
            limits: limits.clone(),
        }
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{key}: expected true or false")),
    }
}

/// Converts CSV text to the one table it makes, for embedding in a larger document. Same rules,
/// limits and diagnostics as [`crate::csv::to_adf`]; the locations are relative to `csv`.
pub(crate) fn embedded_table(
    csv: &str,
    opts: &CsvToAdfOptions,
) -> Result<Conversion<Table>, Diagnostics> {
    let Conversion { output, warnings } = super::to_adf(csv, opts)?;
    let mut tables = output.content.into_iter().filter_map(|block| match block {
        DocBlock::Table(table) => Some(table),
        _ => None,
    });
    match (tables.next(), tables.next()) {
        (Some(table), None) => Ok(Conversion {
            output: table,
            warnings,
        }),
        _ => Err(Diagnostics::single(Diagnostic::new(
            Code::Internal,
            "CSV conversion did not produce exactly one table",
        ))),
    }
}

/// Moves a diagnostic from CSV-relative lines to the lines of the Markdown that holds the CSV:
/// `first_line` is the Markdown line of the CSV's first line.
pub(crate) fn shift_lines(diagnostic: &mut Diagnostic, first_line: u32) {
    if let Some(location) = diagnostic.location.as_mut() {
        let shift = first_line.saturating_sub(1);
        location.line = location.line.map(|line| line.saturating_add(shift));
        location.end_line = location.end_line.map(|line| line.saturating_add(shift));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pairs: &[(&str, &str)]) -> Result<CsvParams, String> {
        CsvParams::from_pairs(pairs.iter().copied())
    }

    #[test]
    fn every_key_parses() {
        let p = parse(&[
            ("delimiter", "tab"),
            ("header", "false"),
            ("max-rows", "50"),
            ("align", "numbers"),
        ])
        .unwrap();
        assert_eq!(p.delimiter, Some(CsvDelimiter::TAB));
        assert_eq!(p.header, Some(false));
        assert_eq!(p.max_rows, Some(50));
        assert_eq!(p.align_numbers, Some(true));
        assert_eq!(parse(&[]).unwrap(), CsvParams::default());
    }

    #[test]
    fn bad_options_name_the_key() {
        for (pairs, needle) in [
            (vec![("delimiter", "ab")], "delimiter"),
            (vec![("header", "yes")], "header"),
            (vec![("max-rows", "0")], "max-rows"),
            (vec![("max-rows", "x")], "max-rows"),
            (vec![("align", "left")], "align"),
            (vec![("colour", "red")], "colour"),
            (vec![("header", "true"), ("header", "false")], "twice"),
        ] {
            let message = parse(&pairs).unwrap_err();
            assert!(message.contains(needle), "{pairs:?}: {message}");
        }
    }

    #[test]
    fn a_tsv_default_loses_to_an_explicit_delimiter() {
        let limits = Limits::default();
        let tsv = Some(CsvDelimiter::TAB);
        let none = CsvParams::default();
        assert_eq!(
            none.options(Target::Jira, &limits, tsv).delimiter,
            CsvDelimiter::TAB
        );
        let pipe = parse(&[("delimiter", "pipe")]).unwrap();
        assert_eq!(
            pipe.options(Target::Jira, &limits, tsv).delimiter.as_char(),
            '|'
        );
        assert_eq!(
            none.options(Target::Jira, &limits, None).delimiter,
            CsvDelimiter::COMMA
        );
    }

    #[test]
    fn an_embedded_table_is_one_table() {
        let table = embedded_table("a,b\n1,2\n", &CsvToAdfOptions::default()).unwrap();
        assert_eq!(table.output.rows.len(), 2);
        assert!(embedded_table("", &CsvToAdfOptions::default()).is_err());
    }

    #[test]
    fn lines_shift_by_the_line_the_csv_starts_on() {
        let mut d = Diagnostic::new(Code::InputCsv, "x").at(crate::Location {
            line: Some(3),
            column: Some(5),
            ..crate::Location::default()
        });
        shift_lines(&mut d, 10);
        let location = d.location.unwrap();
        assert_eq!((location.line, location.column), (Some(12), Some(5)));
    }
}
