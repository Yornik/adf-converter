# Tables, CSV and spreadsheets

`adf` turns a CSV or TSV file into an ADF table (`--from csv`), a table back into CSV
(`--from adf --to csv`), and a Markdown table into an ADF table and back. CSV needs `adf` 0.3.0 or
later (`adf --version`); for older versions see
[Before adf 0.3.0](#before-adf-030-the-csv2mdpy-helper).

## CSV or TSV → table (native)

```sh
adf convert data.csv --from csv -e jira-comment -o body.json
adf convert data.tsv --from csv --delimiter tab -e jira-comment -o body.json
adf convert data.csv --from csv --max-rows 25 --align-numbers -e jira-comment -o body.json
adf convert data.csv --from csv -e confluence -o body.json        # Confluence page or comment
adf validate --from csv data.csv                                  # dry run: lists the warnings
```

Then post `body.json` as in [posting.md](posting.md). The envelope flag (`-e`) works as for
Markdown; the output is the whole document, a single table.

- The first row is the header. `--no-header` invents `Column 1`, `Column 2`, … and treats every
  row as data.
- `--delimiter` takes `comma` (the default), `tab`, `semicolon`, `pipe` or one character. `adf`
  never guesses it: a semicolon file read as comma data is one wide column, so look at the first
  line (`head -1 data.csv`) and pass `--delimiter semicolon`. Spreadsheets in many European
  locales export semicolons.
- The file must be UTF-8 (a BOM is fine). Convert other encodings first
  (`iconv -f latin1 -t utf-8 in.csv > data.csv`); otherwise `E_INPUT_ENCODING`.
- Quotes follow RFC 4180: a field that starts with `"` may hold the delimiter, line breaks and
  doubled quotes (`""`). A quote that is never closed fails with `E_INPUT_CSV` and names the line
  and column where it opened.
- **Cell text is never Markdown.** `*x*`, `<b>`, `[x](y)`, `|` and `C:\temp` stay literal; no
  escaping needed. Line breaks inside a cell become line breaks in the table cell. Bare `http://`
  and `https://` URLs become links.
- Blank lines are skipped. Short rows are padded with empty cells; no cell is dropped. Cell text is
  trimmed.
- `--align-numbers` right-aligns a column when every value in it is a number (`1,250.50`, `-3`,
  `12%`), the header too.
- Limits: 1 MiB of CSV text and **100,000 cells** in the table (`E_INPUT_TOO_LARGE`, with the
  record where it was crossed). `--max-rows` lifts the cell limit, not the 1 MiB text limit: for a bigger file, cut it first (for example `head -n 101 big.csv`), and attach the full file.
- Control characters in the data are removed with `W_CONTROL_CHAR`.

### Big tables: preview in the text, full file attached

Jira stores at most about **32,767 characters** per comment or description. A table of a few
hundred rows can exceed that (`W_SIZE_LIMIT`). Post a preview and attach the file:

1. Convert the first rows: `adf convert data.csv --from csv --max-rows 25 -e jira-comment -o
   body.json`. It prints `W_CSV_TRUNCATED` ("Kept 25 of 1832 data rows"): expected. Add a sentence
   to the comment, such as "First 25 of 1,832 rows; full file attached: `report.csv`." Say so
   whenever you truncate; never present a preview as the whole table.
2. Upload the CSV to the same issue (Jira: `POST /rest/api/3/issue/{key}/attachments`, see
   "Attaching files" in [posting.md](posting.md#attaching-files-and-screenshots)). It then shows
   under the issue's attachments. To show the file by name *inside* the comment text, get its media
   id as in [posting.md](posting.md#attaching-files-and-screenshots) and write
   `[report.csv](adf:attachment/<media-id>)` (see
   [syntax.md](syntax.md#attached-files-media); rendering in the products is not verified).

`adf validate --from csv --max-rows 25 data.csv` shows the warnings without writing anything. With
`--strict`, `W_CSV_TRUNCATED` is an error (exit 5), so leave `--strict` off for previews.

## Table → CSV (native)

```sh
adf convert issue.json --from adf --to csv -o table.csv                   # the first table
adf convert page.json --from adf --to csv --table 2 -o second.csv         # the second one
adf convert issue.json --from adf --to csv --delimiter tab -o table.tsv
adf convert issue.json --from adf --to csv --csv-safe -o for-excel.csv
```

The input is whatever you fetched: a bare ADF document, a Jira issue (`fields.description`, or
`--field NAME`), a Jira comment, or a Confluence page fetched with `?body-format=atlas_doc_format`.
Do not pass `-e` (a usage error: the output is CSV, not JSON).

- `--table N` picks the Nth table in document order (default 1). With fewer tables, or none, you
  get `E_INPUT_NO_TABLE` and the message says how many there are; try `--table 1`, `2`, … or read
  the page as Markdown first (section 3 of `SKILL.md`).
- The output is UTF-8, LF line endings, always ends with a line feed; fields are quoted only when
  needed.
- **Cell text** is flat text: paragraphs, list items and other blocks of a cell are joined with one
  space, line breaks inside a paragraph stay as line breaks, and marks are ignored. Mentions give
  their name, statuses their text, dates `YYYY-MM-DD`, smart links their URL, images and files a
  marker such as `[image: shot.png]`.
- **Merged cells repeat.** CSV has no merged cells, so the text of a merged cell is written in
  every cell it covers, and `W_CSV_MERGED_CELLS` says so. Tell the user when that happens, because
  a column then has duplicate values.
- **Spreadsheets run formulas.** A cell that starts with `=`, `+`, `-` or `@` is a formula to
  Excel, LibreOffice and Google Sheets, and Jira text is untrusted. When the CSV goes to a
  spreadsheet or to a person who will open it in one, **always add `--csv-safe`**: it prefixes
  such cells with `'` (so a negative number `-5` shows as text `'-5`). Leave it off when a program
  reads the CSV and the values must stay exact.

## Write a table in Markdown

```markdown
| Step | Owner | Status |
|:-----|:-----:|-------:|
| Migrate | **DBA** | `done` |
| Verify | QA | [IN PROGRESS](adf:status/blue) |
```

Use this when you want formatting in cells (bold, code, links, status lozenges), which CSV cannot
carry.

- The first row is the header. `:---:` centres a column, `---:` right-aligns it.
- Cells hold inline Markdown only (bold, code, links, `adf:` links, `<br>` for a line break).
  A literal pipe is `\|`, also inside code.
- Every row gets the header's number of cells: missing cells are added empty, extra cells are
  dropped (`W_TABLE_CELL_DROPPED`).
- A document's tables may have at most **100,000 cells** in total (counting added empty cells), and
  Markdown input at most 1 MiB; more fails with `E_INPUT_TOO_LARGE`.
- Tables work at the top level and inside `<details>`. Inside a list, quote, panel or `::: cell`
  box they are moved out (`W_BLOCK_HOISTED`).
- **Lists, several paragraphs, code blocks or images in one cell** do not fit a GFM table. Put that
  content in a `::: cell` … `:::` box (a 1×1 table), or split the table. Merged cells and header
  columns cannot be written in Markdown at all.

Do not turn CSV into a Markdown table yourself: `--from csv` does not go through Markdown, so
nothing in the data needs escaping. `--from csv` makes a document that is only the table. To put
a CSV table between paragraphs of one comment, use a `csv` fence
([next section](#csv-in-the-middle-of-a-markdown-document)), include the file
([after it](#a-csv-table-in-the-middle-of-a-document-include)), use the `csv2md.py` route
([below](#before-adf-030-the-csv2mdpy-helper)), or post the table as its own comment.

## CSV in the middle of a Markdown document

A ```` ```csv ```` or ```` ```tsv ```` fence in Markdown becomes a table (adf 0.3.0 or later), so
one comment can hold text, a CSV table and more text:

`````markdown
Results of the run:

```csv align=numbers
Step,Seconds
Build,41
Test,128
```

All steps passed.
`````

- The word after the fence is exactly `csv` or `tsv` (lowercase). Options follow as `key=value`
  words, no spaces: `delimiter=semicolon` (also `comma`, `tab`, `pipe`, one character; `tsv`
  defaults to `tab`, an explicit `delimiter=` wins), `header=false`, `max-rows=25` (the rest is
  left out with `W_CSV_TRUNCATED`), `align=numbers`. Each at most once.
- The same reading rules and limits as `--from csv`: UTF-8, quotes, `""`, blank lines skipped, cells
  are text, never Markdown. The cells count toward the 100,000 cells of all tables in the document.
- Errors exit 3 and write nothing: `E_CSV_OPTION` (bad option; located at the fence line),
  `E_INPUT_CSV` (unterminated quote, or no rows; located at the Markdown line), `E_INPUT_TOO_LARGE`.
- Like any table, it is moved out of a list item, quote, panel or `::: cell` box
  (`W_BLOCK_HOISTED`); put the fence at the top level or in `<details>`.
- **Behaviour change.** A ```` ```csv ```` fence used to be a code block. To show CSV as code, use
  another language word, such as ```` ```text ````, or pass `--extensions` with a list that leaves
  out `csv` (the fence then stays code, with `W_EXTENSION_DISABLED`).
- One way: converting the ADF back to Markdown writes a GFM table, not a fence. A Jira code block
  whose language is `csv` or `tsv` comes back as a ```` ```adf ```` block (`W_ADF_PASSTHROUGH`); leave it alone.

## A CSV table in the middle of a document (include)

Needs `adf` 0.3.0 or later.

```markdown
Results of the run:

![Q1 numbers](adf:csv/q1.csv?delimiter=semicolon&max-rows=50&align=numbers)

Details follow.
```

```sh
work=$(mktemp -d) && mkdir "$work/data" && cp q1.csv "$work/data/"
adf convert comment.md -e jira-comment --include-root "$work/data" -o "$work/comment.json"
```

The image is replaced by the table of `q1.csv`, with the options of `--from csv` as a query
(`delimiter`, `header`, `max-rows`, `align`; percent-encode `&`, `=` and `%` in values). The path
is relative to the root, uses `/`, and is `%`-decoded once (`my%20file.csv`, or
`<adf:csv/my file.csv>`).

**Security: the Markdown decides what is read.** Includes are off unless you pass
`--include-root DIR`. Then every `![](adf:csv/PATH)` in the Markdown reads a file under `DIR`, and
the table is part of what you post.

- **Use a scratch directory that holds only a copy of the data to include**, as above. Never a
  home directory, a repository, `/`, `~/Downloads`, or a directory with credentials, configuration
  or anything you would not publish.
- **Do not enable it for Markdown built from untrusted text** (an issue, a web page, an e-mail, a
  tool's output): that text can contain an include line for any file under the root.
- Do not add the flag unless the user asked for a CSV in the middle of a document, and tell them
  which directory you used.
- `adf validate --from md --include-root DIR --json doc.md` lists in `includes` which files would
  be read, without writing ADF. Check it before you post.

Paths with `..`, a leading `/`, a backslash, `:` or control characters, symbolic links that lead
out of the directory, and anything that is not a regular file are refused with `E_INCLUDE` (exit
3, nothing written); do not try to work around it. Limits: 16 files per document, 1 MiB per file,
4 MiB in all; the table counts toward the 100,000-cell limit. Without the flag, the image becomes
its alt text with `W_INCLUDE_DISABLED`. An included table is one-way: reading the posted content
back (`--from adf`) gives an ordinary Markdown table.

## Read or edit a table in Markdown

`adf convert --from adf page.json` writes a GFM table when the ADF table has a header row, exactly
one paragraph per cell (images may follow it), no merged cells and consistent column alignment.
Anything else (merged cells, a header column, lists in cells) is written as a raw ```` ```adf ````
block with `W_ADF_PASSTHROUGH`: edit the Markdown around it and leave the block alone, or replace it
with a table you write. `--md-flavor plain` always writes a table (merged cells keep their text in
the first column) but cannot be posted back faithfully.
A table cell that holds only an image, or a list item that has no leading paragraph, is also
written as a raw block (the whole table or list).

## Before adf 0.3.0: the `csv2md.py` helper

`adf` before 0.3.0 has no `--from csv` and no `--to csv`. Check with `adf --version`; if you can,
upgrade (see [posting.md](posting.md#install-adf)) and use the commands above. Otherwise:

**CSV or TSV → table** with `scripts/csv2md.py` (next to this skill's `SKILL.md`; in the release zip
it is `skills/markdown-to-adf/scripts/`). It uses only the Python standard library and needs
`python3`:

```sh
python3 scripts/csv2md.py data.csv > table.md                 # comma separated, UTF-8
python3 scripts/csv2md.py --delimiter tab data.tsv > table.md  # also: semicolon, pipe, one character
python3 scripts/csv2md.py --max-rows 50 --align-numbers data.csv > table.md
some-command | python3 scripts/csv2md.py - > table.md
adf validate --from md table.md && adf convert table.md -e jira-comment -o body.json
```

It behaves like the native command in the normal cases: the same header rules, `Column 1`… with `--no-header`,
escaping so that nothing in a cell is read as Markdown, line breaks as `<br>`, bare `http(s)://`
URLs stay links, short rows padded, blank lines skipped, `--max-rows N` (prints `kept N of M data
rows` on stderr instead of a warning) and `--align-numbers`. Exit 3 means the file is not UTF-8,
has no rows, or is above the 100,000-cell limit. Because the table goes through Markdown, it can
sit between other Markdown in the same comment; from adf 0.3.0 a `csv` fence or an `adf:csv` include
does that natively, so this route is only for older versions. Two differences: an unterminated quote is not an error (the rest of the file ends up in one cell, so
check the row count), and `--max-rows 0` means "all rows".

**Table → CSV** with `jq`. Fetch the content as ADF, then print the first table of a document (also
inside `{"body": …}`, `{"fields": {"description": …}}` and comment envelopes) as CSV; cell text with
several paragraphs or line breaks is joined with spaces:

```sh
jq -r '[.. | objects | select(.type == "table")] | first // empty
       | .content[] | [.content[] | [.. | .text? // empty] | join(" ")] | @csv' issue.json > table.csv
```

Use `... | .[1]` instead of `first` for the second table. Mentions, statuses and emoji appear
through their `text` attribute, dates are skipped, a document without a table prints nothing, merged cells are not repeated, and nothing guards against
spreadsheet formulas: prefer `--to csv --csv-safe` when you can.
