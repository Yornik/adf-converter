# Tables, CSV and spreadsheets

`adf` turns a Markdown table into an ADF table and back. It has **no CSV input or output of its
own**; use `scripts/csv2md.py` (CSV → table) and the `jq` recipe below (table → CSV).

## Write a table

```markdown
| Step | Owner | Status |
|:-----|:-----:|-------:|
| Migrate | **DBA** | `done` |
| Verify | QA | [IN PROGRESS](adf:status/blue) |
```

- The first row is the header. `:---:` centres a column, `---:` right-aligns it.
- Cells hold inline Markdown only (bold, code, links, `adf:` links, `<br>` for a line break).
  A literal pipe is `\|`, also inside code.
- Every row gets the header's number of cells: missing cells are added empty, extra cells are
  dropped (`W_TABLE_CELL_DROPPED`).
- A document's tables may have at most **100,000 cells** in total (counting added empty cells), and
  Markdown input at most 1 MiB; more fails with `E_INPUT_TOO_LARGE`.
- Jira stores at most about **32,767 characters** per comment or description. A table of a few
  hundred rows can exceed that (`W_SIZE_LIMIT`): post a preview and attach the file instead (below).
- Tables work at the top level and inside `<details>`. Inside a list, quote, panel or `::: cell`
  box they are moved out (`W_BLOCK_HOISTED`).
- **Lists, several paragraphs, code blocks or images in one cell** do not fit a GFM table. Put that
  content in a `::: cell` … `:::` box (a 1×1 table), or split the table. Merged cells and header
  columns cannot be written in Markdown at all.

## CSV or TSV → table

```sh
python3 scripts/csv2md.py data.csv > table.md                 # comma separated, UTF-8
python3 scripts/csv2md.py --delimiter tab data.tsv > table.md  # also: semicolon, pipe, one character
python3 scripts/csv2md.py --max-rows 50 --align-numbers data.csv > table.md
some-command | python3 scripts/csv2md.py - > table.md
```

(`scripts/` is next to this skill's `SKILL.md`; in the release zip it is
`skills/markdown-to-adf/scripts/`.) The script uses only the Python standard library.

- The first row is the header; `--no-header` invents `Column 1`, `Column 2`, …
- Everything in a cell is escaped, so data like `a|b`, `*x*`, `<b>`, `C:\temp` or `[x](y)` stays
  literal; line breaks in a cell become `<br>`; bare `http(s)://` URLs stay clickable.
- Short rows are padded with empty cells; blank lines are skipped.
- `--max-rows N` keeps the first N data rows and prints `kept N of M data rows` on stderr. Tell the
  reader the table is a preview.
- Exit 3 means the file is not UTF-8, has no rows, or is above the 100,000-cell limit. Convert other
  encodings first (`iconv -f latin1 -t utf-8`).

Then convert and post as usual (`adf convert table.md -e jira-comment -o body.json`, see
[posting.md](posting.md)). Check first with `adf validate --from md table.md`.

### Big tables: preview in the text, full file attached

1. Post the first rows with `--max-rows 25` and a sentence such as "First 25 of 1,832 rows; full
   file attached: `report.csv`."
2. Upload the CSV to the same issue (Jira: `POST /rest/api/3/issue/{key}/attachments`, see
   "Attaching files" in [posting.md](posting.md#attaching-files-and-screenshots)). It then shows
   under the issue's attachments. Embedding a non-image file *inside* the comment text needs the
   `adf:attachment` link form, which `adf` does not have yet.

## Table → CSV

Fetch the content as ADF, then pick the table with `jq`. This prints the first table of a document
(also inside `{"body": …}`, `{"fields": {"description": …}}` and comment envelopes) as CSV; cell text
with several paragraphs or line breaks is joined with spaces:

```sh
jq -r '[.. | objects | select(.type == "table")] | first
       | .content[] | [.content[] | [.. | .text? // empty] | join(" ")] | @csv' issue.json > table.csv
```

Use `... | .[1]` instead of `first` for the second table, or drop `first` and loop for all of them.
The text of mentions, statuses, dates and emoji is not a `text` node and is skipped; read the
Markdown (below) when that matters.

## Read or edit a table in Markdown

`adf convert --from adf page.json` writes a GFM table when the ADF table has a header row, exactly
one paragraph per cell (images may follow it), no merged cells and consistent column alignment.
Anything else (merged cells, a header column, lists in cells) is written as a raw ```` ```adf ````
block with `W_ADF_PASSTHROUGH`: edit the Markdown around it and leave the block alone, or replace it
with a table you write. `--md-flavor plain` always writes a table (merged cells keep their text in
the first column) but cannot be posted back faithfully.
A table cell that holds only an image, or a list item that has no leading paragraph, is also
written as a raw block (the whole table or list).
