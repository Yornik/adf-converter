//! The RFC 4180 reader: one linear pass over the text, no regular expressions, lenient like
//! Python's `csv` module.
//!
//! The input is first normalised ([`normalize`]): a leading BOM is dropped and CRLF and lone CR
//! become LF. That is exactly what the format asks for: outside quotes all three end a record,
//! inside quotes they are kept as LF. After that only `\n` is a line break, and byte offsets in
//! the normalised text map to line and column numbers like the original (a BOM is not counted,
//! like editors do).
//!
//! The delimiter, the quote and the line break are ASCII, so the reader scans bytes and cuts the
//! text only at those; every slice is valid UTF-8.

use std::borrow::Cow;

use crate::diagnostics::Location;

const QUOTE: u8 = b'"';
const LF: u8 = b'\n';

/// Drops a leading BOM and turns CRLF and lone CR into LF.
pub(super) fn normalize(input: &str) -> Cow<'_, str> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if !input.contains('\r') {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// One field: its text and where it starts (byte offset in the normalised text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Field {
    pub text: String,
    pub start: usize,
}

/// One record: its fields and where it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Record {
    pub fields: Vec<Field>,
    pub start: usize,
}

impl Record {
    /// Every field is empty or only whitespace (this includes a blank line).
    pub(super) fn is_blank(&self) -> bool {
        self.fields
            .iter()
            .all(|field| field.text.chars().all(char::is_whitespace))
    }
}

/// Splits normalised text into records.
#[derive(Debug)]
pub(super) struct Reader<'a> {
    text: &'a str,
    pos: usize,
    delimiter: u8,
}

impl<'a> Reader<'a> {
    /// `text` must be normalised (see [`normalize`]); `delimiter` is an ASCII character other
    /// than a quote or a line break (a non-ASCII `delimiter` never matches).
    pub(super) fn new(text: &'a str, delimiter: char) -> Self {
        Self {
            text,
            pos: 0,
            // 0xFF never occurs in UTF-8, so a non-ASCII delimiter matches nothing.
            delimiter: u8::try_from(delimiter)
                .ok()
                .filter(u8::is_ascii)
                .unwrap_or(0xFF),
        }
    }

    /// The next record, `Ok(None)` at the end of the input, or `Err(offset)` of the opening quote
    /// of a quoted field that is still open at the end of the input.
    pub(super) fn next_record(&mut self) -> Result<Option<Record>, usize> {
        if self.pos >= self.text.len() {
            return Ok(None);
        }
        let start = self.pos;
        let mut fields = Vec::new();
        loop {
            fields.push(self.read_field()?);
            match self.byte(self.pos) {
                Some(b) if b == self.delimiter => self.pos += 1,
                Some(_) => {
                    self.pos += 1;
                    break;
                }
                None => break,
            }
        }
        Ok(Some(Record { fields, start }))
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.text.as_bytes().get(at).copied()
    }

    /// The text between `from` and `to`.
    fn slice(&self, from: usize, to: usize) -> &'a str {
        self.text.get(from..to).unwrap_or_default()
    }

    /// Offset of the next byte that ends a field (the delimiter or a line break), or the end.
    fn field_end(&self, from: usize) -> usize {
        let delimiter = self.delimiter;
        let bytes = self.text.as_bytes().get(from..).unwrap_or_default();
        bytes
            .iter()
            .position(|&b| b == delimiter || b == LF)
            .map_or(self.text.len(), |i| from + i)
    }

    /// Reads one field; leaves `pos` at the delimiter or line break that ends it, or at the end.
    fn read_field(&mut self) -> Result<Field, usize> {
        let start = self.pos;
        let text = if self.byte(start) == Some(QUOTE) {
            self.read_quoted(start)?
        } else {
            let end = self.field_end(start);
            self.pos = end;
            self.slice(start, end).to_owned()
        };
        Ok(Field { text, start })
    }

    /// A field that starts with a quote: it runs to the next quote that is not doubled; the
    /// characters after the closing quote up to the delimiter or line break are appended as they
    /// are.
    fn read_quoted(&mut self, open: usize) -> Result<String, usize> {
        let mut out = String::new();
        let mut from = open + 1;
        loop {
            let bytes = self.text.as_bytes().get(from..).unwrap_or_default();
            let Some(offset) = bytes.iter().position(|&b| b == QUOTE) else {
                return Err(open);
            };
            let quote = from + offset;
            out.push_str(self.slice(from, quote));
            if self.byte(quote + 1) == Some(QUOTE) {
                out.push('"');
                from = quote + 2;
            } else {
                from = quote + 1;
                break;
            }
        }
        let end = self.field_end(from);
        out.push_str(self.slice(from, end));
        self.pos = end;
        Ok(out)
    }
}

/// Turns byte offsets into 1-based line and column numbers (columns count characters). Meant for
/// offsets that mostly grow: it resumes from the last answer, so asking for many offsets in
/// order costs one pass over the text.
#[derive(Debug)]
pub(super) struct Positions<'a> {
    text: &'a str,
    offset: usize,
    line: u32,
    column: u32,
}

impl<'a> Positions<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        Self {
            text,
            offset: 0,
            line: 1,
            column: 1,
        }
    }

    /// `(line, column)` of the character at `offset` (clamped to the end of the text).
    pub(super) fn line_col(&mut self, offset: usize) -> (u32, u32) {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        if offset < self.offset {
            self.offset = 0;
            self.line = 1;
            self.column = 1;
        }
        for c in self
            .text
            .get(self.offset..offset)
            .unwrap_or_default()
            .chars()
        {
            if c == '\n' {
                self.line = self.line.saturating_add(1);
                self.column = 1;
            } else {
                self.column = self.column.saturating_add(1);
            }
        }
        self.offset = offset;
        (self.line, self.column)
    }

    /// A location with line and column.
    pub(super) fn location(&mut self, offset: usize) -> Location {
        let (line, column) = self.line_col(offset);
        Location {
            line: Some(line),
            column: Some(column),
            ..Location::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records as lists of field texts, blank records included.
    fn read(input: &str, delimiter: char) -> Result<Vec<Vec<String>>, usize> {
        let text = normalize(input);
        let mut reader = Reader::new(&text, delimiter);
        let mut records = Vec::new();
        while let Some(record) = reader.next_record()? {
            records.push(record.fields.into_iter().map(|f| f.text).collect());
        }
        Ok(records)
    }

    fn rows(input: &str) -> Vec<Vec<String>> {
        read(input, ',').unwrap()
    }

    fn row(fields: &[&str]) -> Vec<String> {
        fields.iter().map(|f| (*f).to_owned()).collect()
    }

    #[test]
    fn simple_records() {
        assert_eq!(rows("a,b\nc,d\n"), [row(&["a", "b"]), row(&["c", "d"])]);
        assert_eq!(rows("a,b\nc,d"), [row(&["a", "b"]), row(&["c", "d"])]);
    }

    #[test]
    fn empty_input_has_no_records() {
        assert_eq!(rows("").len(), 0);
        assert_eq!(rows("\u{feff}").len(), 0);
    }

    #[test]
    fn a_blank_line_is_a_record_with_one_empty_field() {
        assert_eq!(rows("\n"), [row(&[""])]);
        assert_eq!(rows("a\n\nb"), [row(&["a"]), row(&[""]), row(&["b"])]);
    }

    #[test]
    fn empty_fields_and_a_trailing_delimiter() {
        assert_eq!(rows(",\n"), [row(&["", ""])]);
        assert_eq!(rows("a,"), [row(&["a", ""])]);
        assert_eq!(rows("a,,b"), [row(&["a", "", "b"])]);
    }

    #[test]
    fn crlf_cr_and_lf_end_records() {
        let expected = [row(&["a"]), row(&["b"]), row(&["c"]), row(&["d"])];
        assert_eq!(rows("a\r\nb\rc\nd"), expected);
        assert_eq!(rows("a\r\nb\rc\nd\r\n"), expected);
        // CR CR is two line breaks, CR LF is one.
        assert_eq!(rows("a\r\rb"), [row(&["a"]), row(&[""]), row(&["b"])]);
    }

    #[test]
    fn a_bom_is_ignored_once() {
        assert_eq!(rows("\u{feff}a,b"), [row(&["a", "b"])]);
        assert_eq!(rows("\u{feff}\u{feff}a"), [row(&["\u{feff}a"])]);
        // Only a leading BOM; one inside the text is data.
        assert_eq!(rows("a\u{feff}"), [row(&["a\u{feff}"])]);
    }

    #[test]
    fn quoted_fields_keep_delimiters_and_line_breaks() {
        assert_eq!(rows("\"a,b\",c"), [row(&["a,b", "c"])]);
        assert_eq!(rows("\"a\nb\",c"), [row(&["a\nb", "c"])]);
        assert_eq!(rows("\"a\r\nb\rc\",d"), [row(&["a\nb\nc", "d"])]);
    }

    #[test]
    fn doubled_quotes_are_one_literal_quote() {
        assert_eq!(rows("\"say \"\"hi\"\"\",x"), [row(&["say \"hi\"", "x"])]);
        assert_eq!(rows("\"\"\"\""), [row(&["\""])]);
        assert_eq!(rows("\"\""), [row(&[""])]);
        assert_eq!(rows("\"\",a"), [row(&["", "a"])]);
    }

    #[test]
    fn text_after_the_closing_quote_is_appended() {
        assert_eq!(rows("\"a\"b,c"), [row(&["ab", "c"])]);
        assert_eq!(rows("\"a\"b\"c\",d"), [row(&["ab\"c\"", "d"])]);
        assert_eq!(rows("\"a\" ,b"), [row(&["a ", "b"])]);
        assert_eq!(rows("\"a\"b\nc"), [row(&["ab"]), row(&["c"])]);
    }

    #[test]
    fn a_quote_inside_an_unquoted_field_is_literal() {
        assert_eq!(rows("a\"b,c"), [row(&["a\"b", "c"])]);
        assert_eq!(rows("a,b\"\n"), [row(&["a", "b\""])]);
        // A leading space means the field does not start with a quote.
        assert_eq!(rows(" \"a,b\""), [row(&[" \"a", "b\""])]);
    }

    #[test]
    fn a_quote_right_after_the_delimiter_opens_a_field() {
        assert_eq!(rows("a,\"b,c\""), [row(&["a", "b,c"])]);
        assert_eq!(rows("a,\"\"\n"), [row(&["a", ""])]);
    }

    #[test]
    fn an_open_quote_at_the_end_is_an_error_at_the_opening_quote() {
        assert_eq!(read("a,\"b\nc", ','), Err(2));
        assert_eq!(read("\"", ','), Err(0));
        assert_eq!(read("\"a\"\"", ','), Err(0));
        assert_eq!(read("x\n\"a,b\n\n", ','), Err(2));
    }

    #[test]
    fn other_delimiters() {
        assert_eq!(read("a\tb,c\n", '\t').unwrap(), [row(&["a", "b,c"])]);
        assert_eq!(read("a;b,c", ';').unwrap(), [row(&["a", "b,c"])]);
        assert_eq!(read("a|\"b|c\"", '|').unwrap(), [row(&["a", "b|c"])]);
        assert_eq!(read("a,b", ';').unwrap(), [row(&["a,b"])]);
    }

    #[test]
    fn a_non_ascii_delimiter_never_matches() {
        // `CsvDelimiter` only builds ASCII delimiters; the reader still must not panic.
        assert_eq!(read("a,é", '\u{e9}').unwrap(), [row(&["a,é"])]);
    }

    #[test]
    fn unicode_and_nul_pass_through() {
        assert_eq!(rows("é,日本,😀\0"), [row(&["é", "日本", "😀\0"])]);
    }

    #[test]
    fn field_and_record_offsets() {
        let text = normalize("a,\"b\"\nccc,d");
        let mut reader = Reader::new(&text, ',');
        let first = reader.next_record().unwrap().unwrap();
        assert_eq!(first.start, 0);
        assert_eq!(
            first.fields.iter().map(|f| f.start).collect::<Vec<_>>(),
            [0, 2]
        );
        let second = reader.next_record().unwrap().unwrap();
        assert_eq!(second.start, 6);
        assert_eq!(second.fields.last().map(|f| f.start), Some(10));
        assert!(reader.next_record().unwrap().is_none());
    }

    #[test]
    fn blank_records() {
        let blank = |input: &str| {
            let text = normalize(input);
            Reader::new(&text, ',')
                .next_record()
                .unwrap()
                .unwrap()
                .is_blank()
        };
        assert!(blank("\n"));
        assert!(blank(" \t ,  ,\u{a0}\n"));
        assert!(blank("\"  \",\n"));
        assert!(!blank(" ,x\n"));
        assert!(!blank("\0\n"));
        assert!(!blank("\"\n\",x"));
    }

    #[test]
    fn normalize_borrows_when_there_is_nothing_to_do() {
        assert!(matches!(normalize("a\nb"), Cow::Borrowed(_)));
        assert!(matches!(normalize("\u{feff}a"), Cow::Borrowed("a")));
        assert_eq!(normalize("a\r\nb\rc\r"), "a\nb\nc\n");
    }

    #[test]
    fn positions_count_characters_and_lines() {
        let text = "ab\nécd\n\nx";
        let mut positions = Positions::new(text);
        assert_eq!(positions.line_col(0), (1, 1));
        assert_eq!(positions.line_col(1), (1, 2));
        assert_eq!(positions.line_col(3), (2, 1));
        // `é` is two bytes: `c` is the second character.
        assert_eq!(positions.line_col(5), (2, 2));
        assert_eq!(positions.line_col(8), (3, 1));
        assert_eq!(positions.line_col(9), (4, 1));
        // Going back works, and so does asking past the end.
        assert_eq!(positions.line_col(1), (1, 2));
        assert_eq!(positions.line_col(1000), (4, 2));
        // The middle of a character rounds down.
        assert_eq!(positions.line_col(4), (2, 1));
    }
}
