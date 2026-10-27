# Markdown syntax cheat sheet for `adf`

What to write so Jira and Confluence show what you mean. The normative mapping is
[docs/syntax.md](https://github.com/Yornik/adf-converter/blob/main/docs/syntax.md) (also in
`docs/` of every release zip). `adf capabilities --json` and `adf examples <feature>` show what
the installed version supports.

## Standard Markdown

| Write | Result | Notes |
|---|---|---|
| `# Title` … `###### Title` | heading 1–6 | Not inside lists or quotes (becomes bold text, `W_HEADING_DEMOTED`). |
| Blank line between paragraphs | separate paragraphs | |
| Line break inside a paragraph | a space | Use `--softbreak hardbreak` to keep line breaks, or end the line with `\`. |
| Line ending in `\` or two spaces, or `<br>` | line break | |
| `*em*`, `**strong**`, `***both***`, `~~strike~~` | italic, bold, strike | |
| `` `code` `` | inline code | Inline code can only be combined with a link. `` **`x`** `` loses the bold (`W_MARK_DROPPED`). |
| `[text](https://example.com "title")` | link | Allowed: `http`, `https`, `ftp`, `ftps`, `mailto:`, `tel:`, `/path`, `#anchor`. Anything else (relative paths, `javascript:`) stays plain text (`W_UNSAFE_URL`). |
| `<https://x.example>`, bare `https://…` or `www.…` | link | Write `https\://x` to keep a URL as text. |
| `- item` / `* item` / `+ item` | bullet list | Indent sub-items to line up with the item's text: 2 spaces under `- `, 3 under `1. `. |
| `1. item` | numbered list | Starts at the number you write. |
| `- [ ] todo` / `- [x] done` | task list | Use `-`, not `1.` (`W_TASK_NUMBERING_LOST`). |
| ```` ```python ```` … ```` ``` ```` | code block with language | Contents are never formatted. Except ```` ```csv ```` and ```` ```tsv ````: those become tables ([below](#csv-tables-in-markdown-csv)). |
| `> quote` | quote | Quotes hold paragraphs, lists and code only. |
| `---` on its own line (blank line before it) | horizontal rule | Directly under a text line, `---` makes that line a heading instead. |
| `![alt](https://…/x.png)` | a link to the image | `W_IMAGE_AS_LINK`. Images need uploaded media in Jira; `--images external` embeds by URL, without the image's title or a link around it (`W_MARK_DROPPED`). For a file you uploaded, use `![alt](adf:media/<id>)` ([below](#file-media-media)). |
| `[^1]` and `[^1]: note` | superscript number + numbered list at the end | `W_FOOTNOTE_FLATTENED`. |
| `\*`, `&amp;` | literal `*`, `&` | Backslash escapes and entities work as usual. |

### Tables

```markdown
| Step | Owner | Status |
|:-----|:-----:|-------:|
| Migrate | **DBA** | `done` |
| Verify | QA | [IN PROGRESS](adf:status/blue) |
```

- The first row is the header. `:---:` centres a column, `---:` right-aligns it.
- Every row gets the header's number of cells: missing cells are added empty, extra cells are
  dropped (`W_TABLE_CELL_DROPPED`). Write a literal pipe as `\|`, also inside code.
- A document's tables may have at most 100,000 cells in total, counting the added empty cells;
  more fails with `E_INPUT_TOO_LARGE`.
- `<br>` makes a line break inside a cell. Put lists or several paragraphs in a box cell instead
  (below).
- Building a table from CSV data (`adf convert data.csv --from csv`, or a ```` ```csv ```` fence
  inside Markdown: [below](#csv-tables-in-markdown-csv)), getting one out as CSV
  (`--from adf --to csv`), big tables: see [tables.md](tables.md).
- Tables only work at the top level or inside `<details>`. Inside a list, quote, panel or box they
  are moved out (`W_BLOCK_HOISTED`).

## Extensions

All are on by default (`--extensions all`). `--extensions none` or a list such as
`--extensions alerts,details` turns them off; their syntax is then plain Markdown
(`W_EXTENSION_DISABLED`). Names: `alerts`, `details`, `cells`, `raw-adf`, `mention`, `status`,
`date`, `emoji`, `card`, `media`, `csv`.

### Panels (`alerts`)

```markdown
> [!WARNING]
> Back up the database first.
```

| Alert | Panel |
|---|---|
| `[!NOTE]` | info (blue) |
| `[!TIP]` | success (green) |
| `[!IMPORTANT]` | note (purple) |
| `[!WARNING]` | warning (yellow) |
| `[!CAUTION]` | error (red) |

A panel may contain paragraphs, headings, lists, task lists, code and rules. Panels go at the top
level, in `<details>` or in a box cell. An alert under a list item (a numbered step, a task) or
inside another alert is moved after it and still becomes a panel (`W_BLOCK_HOISTED`); inside a
quote it becomes plain quote text (`W_BLOCK_FLATTENED`).

### Expand (`details`)

```markdown
<details>
<summary>Full logs</summary>

Markdown content here, separated from the tags by blank lines.

</details>
```

The blank lines matter, as on GitHub. The summary is plain text: tags in it are removed, and
unrecognised ones such as `<code>` or `<img>` are reported (`W_HTML_DROPPED`). A `<details>` inside another
`<details>` or a box cell becomes a nested expand; inside a list, quote or panel it is flattened to
a bold title followed by the content.

### Box cells (`cells`)

```markdown
::: cell
Any Markdown: paragraphs, **lists**, code, alerts, `<details>`.
:::
```

A framed box (a 1×1 table), common in Jira descriptions. The `:::` lines must start at column 0.
Boxes cannot be nested. A table inside a box is moved out of it, and the box is split around it:
a box, the table, then a second box with the rest (`W_BLOCK_HOISTED`).

### Raw ADF (`raw-adf`)

````markdown
```adf
{"type": "panel", "attrs": {"panelType": "success"}, "content": [{"type": "paragraph"}]}
```
````

The escape hatch for a block node Markdown cannot express: one JSON object or an array of block
nodes, spliced in after validation (`E_RAW_ADF`, exit 3, if it is invalid there). Prefer Markdown
whenever it can express the content.

### Inline extensions (`adf:` links)

| Write | Result | Rules |
|---|---|---|
| `[@Jane Doe](adf:mention/5b10ac8d82e05b22cc7d4ef5)` | mention | The path is the Atlassian **account ID** (from `GET /rest/api/3/user/search?query=…`); the label is the display text. |
| `[DONE](adf:status/green)` | status lozenge | Colours: `neutral` (default), `purple`, `blue`, `red`, `yellow`, `green`. The label must not be empty. |
| `[2026-10-19](adf:date)` | date | The label must be `YYYY-MM-DD`. |
| `[😄](adf:emoji/:smile:)` | emoji | The path is the short name. |
| `[https://site.atlassian.net/browse/PROJ-1](adf:card)` | smart link card | The label must be an allowed URL. |

These nodes cannot be bold or italic; formatting around them is dropped (`W_MARK_DROPPED`). A bad
argument keeps the label as text (`W_INVALID_EXTENSION_ARG`); an unknown kind such as
`adf:foo/x` too (`W_UNKNOWN_EXTENSION`). A line break inside a label becomes a space, so a label
may wrap across lines even with `--softbreak hardbreak`.

### CSV tables in Markdown (`csv`)

`````markdown
```csv align=numbers
Item,Qty,Notes
Widget,3,"Blue, small"
Gadget,12,
```
`````

A ```` ```csv ```` or ```` ```tsv ```` fence (backticks or tildes) becomes a table, in the middle of
a comment or description. The body is the CSV text; cell text is never Markdown. Options after
the word, as `key=value` with no spaces: `delimiter` (`comma`, `tab`, `semicolon`, `pipe` or one
character; `tsv` defaults to `tab`), `header` (`true` or `false`), `max-rows` (at least 1) and
`align` (`numbers` or `none`). A bad option is `E_CSV_OPTION`, an unterminated quote or empty body
`E_INPUT_CSV` (exit 3). The table is placed like any table (top level or `<details>`; elsewhere it
is moved out with `W_BLOCK_HOISTED`) and counts toward the 100,000 cells of all tables. Needs
`adf` 0.3.0 or later. **Behaviour change:** a ```` ```csv ```` fence used to be a code block; to
show CSV as code, write ```` ```text ````, or leave `csv` out of `--extensions`. Converting back
writes a GFM table, never a fence. More in [tables.md](tables.md#csv-in-the-middle-of-a-markdown-document).

### File media (`media`)

An image that is already uploaded to Jira or Confluence, referenced by its **media id** (a UUID;
not the numeric attachment id). `adf` never uploads and never looks ids up; see
[posting.md](posting.md#attaching-files-and-screenshots) for how to get the id.

```markdown
![shot.png](adf:media/11111111-2222-3333-4444-555555555555?width=1512&height=945)
![arch.png](adf:media/11111111-2222-3333-4444-555555555555?collection=contentId-123456&width=1280&height=720&layout=wide&size=720px)
```

| Parameter | Values | Default |
|---|---|---|
| id (after `adf:media/`) | a UUID | required |
| `collection` | up to 64 of `A-Za-z0-9_-` | empty (Jira); Confluence: from the upload, e.g. `contentId-123456` |
| `width`, `height` | file size in pixels, 1 to 100000 | none; give both |
| `layout` | `wide`, `full-width`, `center`, `wrap-right`, `wrap-left`, `align-end`, `align-start` | `center` |
| `size` | displayed width: `N%` (1 to 100) or `Npx` (1 to 100000) | none |

- Put it on its own line. Media is a block: in the middle of a paragraph it splits the paragraph,
  and after text in a heading or GFM table cell it follows that text, without a warning. It stays
  inside list items, quotes, panels, expands and cells. Only in a task item (the task list is
  split around it) or an `adf:` label is it moved out, with `W_BLOCK_HOISTED`.
- An unknown or repeated key, an invalid value, an empty value (except `collection=`), or a bad id or collection keeps the alt
  text as plain text (`W_INVALID_EXTENSION_ARG`). With `--extensions` lacking `media`, the alt text
  stays too (`W_EXTENSION_DISABLED`). `--images drop` removes the image (`W_IMAGE_DROPPED`);
  `--images link` and `--images external` do not affect it.
- An image title, or a link around the image, is dropped (`W_MARK_DROPPED`).
- `adf` cannot tell whether the id exists. A wrong id fails when you post.
- Only images. Other files (PDFs, logs) use the `adf:attachment` link form below.

### Attached files (`media`)

A reference to a file that is already attached, by its name: the link text is the file name, the
path is the same **media id** as for images (a UUID, not the numeric attachment id), and
`?collection=…` is optional. Same extension (`media`) as the image form. `adf` never uploads or
looks ids up. Needs `adf` 0.4.0 or later: older versions keep the link text as plain text and
report `W_UNKNOWN_EXTENSION` (exit 0), so check `adf --version` first, and note that those versions
also write such files as ```` ```adf ```` blocks when reading ADF.

```markdown
[report.pdf](adf:attachment/66666666-7777-8888-9999-000000000000) [trace.zip](adf:attachment/66666666-7777-8888-9999-000000000002?collection=contentId-123456)

See [log.txt](adf:attachment/66666666-7777-8888-9999-000000000001) for details.
```

- A paragraph of **only** attachment links (apart by spaces or line breaks) is one `mediaGroup`
  (a block of file cards). A link with text around it, or two links with nothing between them, is a
  `mediaInline` (a file inside the line).
- In a list item, a heading, a task item and a cell of a GFM table the links are always inline:
  a list item cannot hold a `mediaGroup`. For a group in a table, use a `::: cell` box.
- The link text must have a visible character (white space or only invisible characters is empty).
  Only the key `collection` exists; `width`, `height`, `layout`
  and `size` are for images. A bad id, a bad or repeated key or an empty link text keeps the link
  text as plain text (`W_INVALID_EXTENSION_ARG`). With `--extensions` lacking `media` the text stays
  too (`W_EXTENSION_DISABLED`). `--images drop` removes the links (`W_IMAGE_DROPPED`);
  `--images link` and `--images external` do not affect them. Formatting around the link or in
  the link text is dropped (`W_MARK_DROPPED`).
- `![x](adf:attachment/<id>)` (the image form) is not an attachment: the alt text stays and
  `W_UNKNOWN_EXTENSION` says to use the link form.
- `adf` accepts both nodes for Jira and Confluence. Whether the products *render* a `mediaGroup`
  or `mediaInline` written through the API is not verified: say so, and check the result after
  posting.

### CSV include (`csv`, needs `--include-root`)

```markdown
![Q1 numbers](adf:csv/q1.csv)
![Q1 numbers](adf:csv/reports/q1.csv?delimiter=semicolon&header=true&max-rows=50&align=numbers)
```

`adf` 0.3.0 or later replaces the image by the table of a CSV file. **Off unless you pass
`--include-root DIR`**: the Markdown then decides which files under `DIR` are read and published,
so use a scratch directory with a copy of just the data, never a home directory or repository, and
never for Markdown built from untrusted text. Details and the safe recipe: see
[tables.md](tables.md#a-csv-table-in-the-middle-of-a-document-include).

- The path is relative to `DIR`, uses `/`, and is `%`-decoded once; `..`, a leading `/`, `\`, `:`,
  control characters and symbolic links out of `DIR` are `E_INCLUDE`. Options: `delimiter`,
  `header`, `max-rows`, `align` (`E_CSV_OPTION` for a bad one).
- No root: the image becomes its alt text (`W_INCLUDE_DISABLED`). Without `csv` in `--extensions`:
  `W_EXTENSION_DISABLED`. The link form `[x](adf:csv/…)` is not an include (`W_UNKNOWN_EXTENSION`).
- The table is placed like a Markdown table (moved out of lists and quotes with
  `W_BLOCK_HOISTED`), and is one-way: ADF → Markdown writes a normal table.

## Inline HTML

| Write | Result |
|---|---|
| `<u>x</u>`, `<ins>x</ins>` | underline |
| `<s>`, `<del>`, `<strike>` | strike |
| `<b>`, `<strong>`, `<i>`, `<em>` | bold, italic |
| `<sub>x</sub>`, `<sup>x</sup>` | subscript, superscript (not both: the inner one wins, `W_MARK_DROPPED`) |
| `<span style="color: #FF5630">x</span>` | text colour (6-digit hex only) |
| `<span style="background-color: #FFF0B3">x</span>` | highlight (6-digit hex only) |
| `<kbd>x</kbd>` | inline code |
| `<br>` | line break |
| `<!-- comment -->` | removed silently |

Other HTML is kept as literal text (`W_HTML_AS_TEXT`); `--html drop` removes it
(`W_HTML_DROPPED`), with the content of `<script>` and `<style>` elements, and `--html error`
fails (exit 5).

## Where blocks may go

| Container | May contain |
|---|---|
| top level | everything |
| list item | paragraphs, lists, task lists, code |
| quote | paragraphs, lists, code |
| panel (`> [!NOTE]`) | paragraphs, headings, lists, task lists, code, rules |
| box cell (`::: cell`), table cell | everything except tables (`<details>` becomes a nested expand) |
| `<details>` | everything (`<details>` inside becomes a nested expand) |

Anything else is repaired: headings become bold text, nested quotes are merged, and other blocks
are moved out of the container, which is split around them. Every repair is reported with a
warning, so `adf validate --from md` shows what to restructure.

## ADF → Markdown

`adf convert --from adf` writes the same syntax back:

- `--md-flavor roundtrip` (default): converting the Markdown again **with `--images external`**
  gives the same document (without it, embedded external images become links). Nodes without a
  Markdown form become ```` ```adf ```` blocks (`W_ADF_PASSTHROUGH`); keep them unchanged when
  editing. Attributes and marks without a Markdown form (column widths, cell colours, alignment,
  Confluence inline-comment anchors) are dropped with `W_ATTR_DROPPED`.
- A file image that `adf` can express is written as an `![alt](adf:media/<id>?…)` line (leave it
  unchanged when you edit); captions and marks stay ```` ```adf ```` blocks. A `mediaGroup` and a
  `mediaInline` with a file name are written as `[name](adf:attachment/<id>?…)` links (leave them
  unchanged too); one without a file name, with other attributes (`localId` too), and a paragraph
  of only inline files stay ```` ```adf ```` blocks. A table cell or list item holding only an image (no text before it) turns the whole
  table or list into one ```` ```adf ```` block.
- `--md-flavor plain`: compact Markdown for reading. Mentions become `@Name`, statuses `[TEXT]`,
  dates `YYYY-MM-DD`, emoji their character, cards bare URLs, external images `![alt](url)`
  with their caption, uploaded images `[image: alt]`, attachments `[attachment: name]` (or
  `[attachment]` without a name), and nodes without a Markdown form are dropped (media is never
  dropped silently: it leaves a marker, also in table cells and inline). An inline marker is
  inserted as is, with no spaces added around it. Merged table cells leave the columns they span empty, so every
  value stays under its header.

Text, code or titles that Markdown cannot carry unchanged (control characters such as terminal
colour codes, or an expand title with extra spaces) become ```` ```adf ```` blocks in roundtrip
mode; plain mode shows control characters in code as U+FFFD. Markdown output is limited to
64 MiB (`E_INPUT_TOO_LARGE`).
