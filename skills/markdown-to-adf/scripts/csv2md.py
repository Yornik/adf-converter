#!/usr/bin/env python3
"""Turn a CSV (or TSV) file into a GitHub-flavoured Markdown table that `adf convert` understands.

FALLBACK for adf before 0.3.0. From 0.3.0 on, `adf convert data.csv --from csv` reads CSV and TSV
natively (and `--from adf --to csv` writes it), with the same rules as this script; use that and
see references/tables.md. Check with `adf --version`. This helper only exists for older versions
and for putting a CSV table between other Markdown in one comment. It uses only the Python
standard library:

    python3 csv2md.py data.csv > table.md
    python3 csv2md.py --delimiter tab --max-rows 50 --align-numbers data.tsv > table.md
    some-command | python3 csv2md.py - > table.md

The first row is the header (`--no-header` invents "Column 1", "Column 2", ...). Cells are escaped
so that nothing in the data is read as Markdown: `|` and `\\`, emphasis and code characters, links,
HTML and entities. Line breaks inside a cell become `<br>`. Rows shorter than the longest row are
padded with empty cells, so no data is dropped. Blank lines are skipped.

Exit codes: 0 ok, 2 usage, 3 bad input (not UTF-8, no rows, too many cells).
"""
import argparse
import csv
import re
import sys

# adf refuses documents whose tables expand to more cells than this (E_INPUT_TOO_LARGE).
MAX_CELLS = 100_000

DELIMITERS = {"comma": ",", ",": ",", "tab": "\t", "\\t": "\t", "semicolon": ";", ";": ";",
              "pipe": "|", "|": "|"}

# Characters that can start Markdown syntax inside a table cell.
ESCAPE = re.compile(r"([\\`*_\[\]<>&~|])")
NUMBER = re.compile(r"^[+-]?(\d[\d,. _]*)?\d%?$")


URL = re.compile(r"(https?://[^\s<>|]+)")


def cell(text):
    """One cell as Markdown inline text. Bare http(s) URLs stay as they are, so they become links."""
    text = text.replace("\r\n", "\n").replace("\r", "\n").strip()
    parts = URL.split(text)  # odd items are URLs
    text = "".join(part if i % 2 else ESCAPE.sub(r"\\\1", part) for i, part in enumerate(parts))
    return text.replace("\n", "<br>")


def main(argv):
    parser = argparse.ArgumentParser(description="CSV to a Markdown table for adf convert.")
    parser.add_argument("file", help="CSV file, or - for standard input")
    parser.add_argument("--delimiter", default="comma",
                        help="comma (default), tab, semicolon, pipe, or one character")
    parser.add_argument("--no-header", action="store_true",
                        help="the first row is data; use Column 1, Column 2, ... as the header")
    parser.add_argument("--max-rows", type=int, default=0, metavar="N",
                        help="keep only the first N data rows (0 = all) and say so on stderr")
    parser.add_argument("--align-numbers", action="store_true",
                        help="right-align columns whose cells are all numbers")
    args = parser.parse_args(argv)

    delimiter = DELIMITERS.get(args.delimiter, args.delimiter)
    if len(delimiter) != 1:
        parser.error("--delimiter must be comma, tab, semicolon, pipe or one character")
    try:
        if args.file == "-":
            sys.stdin.reconfigure(encoding="utf-8-sig", newline="")
            rows = list(csv.reader(sys.stdin, delimiter=delimiter))
        else:
            with open(args.file, newline="", encoding="utf-8-sig") as handle:
                rows = list(csv.reader(handle, delimiter=delimiter))
    except (OSError, UnicodeDecodeError, csv.Error) as error:
        print(f"csv2md: {error}", file=sys.stderr)
        return 3

    rows = [row for row in rows if any(c.strip() for c in row)]
    if not rows:
        print("csv2md: the input has no rows", file=sys.stderr)
        return 3
    width = max(len(row) for row in rows)
    if args.no_header:
        header, body = [f"Column {i + 1}" for i in range(width)], rows
    else:
        header, body = rows[0], rows[1:]
    if args.max_rows and len(body) > args.max_rows:
        print(f"csv2md: kept {args.max_rows} of {len(body)} data rows", file=sys.stderr)
        body = body[:args.max_rows]
    if width * (len(body) + 1) > MAX_CELLS:
        print(f"csv2md: {width} columns x {len(body) + 1} rows is more than {MAX_CELLS} cells; "
              "use --max-rows and attach the full file instead", file=sys.stderr)
        return 3

    def pad(row):
        return list(row) + [""] * (width - len(row))

    header, body = pad(header), [pad(row) for row in body]
    right = [
        args.align_numbers and body and all(
            not row[i].strip() or NUMBER.match(row[i].strip()) for row in body)
        and any(row[i].strip() for row in body)
        for i in range(width)
    ]
    out = ["| " + " | ".join(cell(c) for c in header) + " |",
           "| " + " | ".join("---:" if r else "---" for r in right) + " |"]
    out += ["| " + " | ".join(cell(c) for c in row) + " |" for row in body]
    sys.stdout.write("\n".join(out) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
