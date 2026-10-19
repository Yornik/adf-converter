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
| ```` ```python ```` … ```` ``` ```` | code block with language | Contents are never formatted. |
| `> quote` | quote | Quotes hold paragraphs, lists and code only. |
| `---` on its own line (blank line before it) | horizontal rule | Directly under a text line, `---` makes that line a heading instead. |
| `![alt](https://…/x.png)` | a link to the image | `W_IMAGE_AS_LINK`. Images need uploaded media in Jira; `--images external` embeds by URL. |
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
- Tables only work at the top level or inside `<details>`. Inside a list, quote, panel or box they
  are moved out (`W_BLOCK_HOISTED`).

## Extensions

All are on by default (`--extensions all`). `--extensions none` or a list such as
`--extensions alerts,details` turns them off; their syntax is then plain Markdown
(`W_EXTENSION_DISABLED`). Names: `alerts`, `details`, `cells`, `raw-adf`, `mention`, `status`,
`date`, `emoji`, `card`.

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

A panel may contain paragraphs, headings, lists, task lists, code and rules. Use panels at the top
level, in `<details>` or in a box cell; elsewhere they become a plain quote (`W_BLOCK_FLATTENED`).

### Expand (`details`)

```markdown
<details>
<summary>Full logs</summary>

Markdown content here, separated from the tags by blank lines.

</details>
```

The blank lines matter, as on GitHub. The summary is plain text. A `<details>` inside another
`<details>` or a box cell becomes a nested expand; inside a list, quote or panel it is flattened to
a bold title followed by the content.

### Box cells (`cells`)

```markdown
::: cell
Any Markdown: paragraphs, **lists**, code, alerts, `<details>`.
:::
```

A framed box (a 1×1 table), common in Jira descriptions. The `:::` lines must start at column 0.
Boxes cannot be nested, and tables inside a box are moved after it.

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
`adf:foo/x` too (`W_UNKNOWN_EXTENSION`).

## Inline HTML

| Write | Result |
|---|---|
| `<u>x</u>`, `<ins>x</ins>` | underline |
| `<s>`, `<del>`, `<strike>` | strike |
| `<b>`, `<strong>`, `<i>`, `<em>` | bold, italic |
| `<sub>x</sub>`, `<sup>x</sup>` | subscript, superscript |
| `<span style="color: #FF5630">x</span>` | text colour (6-digit hex only) |
| `<span style="background-color: #FFF0B3">x</span>` | highlight (6-digit hex only) |
| `<kbd>x</kbd>` | inline code |
| `<br>` | line break |
| `<!-- comment -->` | removed silently |

Other HTML is kept as literal text (`W_HTML_AS_TEXT`); `--html drop` removes it
(`W_HTML_DROPPED`) and `--html error` fails (exit 5).

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

- `--md-flavor roundtrip` (default): converting the Markdown again gives the same document.
  Nodes without a Markdown form become ```` ```adf ```` blocks (`W_ADF_PASSTHROUGH`); keep them
  unchanged when editing.
- `--md-flavor plain`: compact Markdown for reading. Mentions become `@Name`, statuses `[TEXT]`,
  dates `YYYY-MM-DD`, emoji their character, cards bare URLs, and nodes without a Markdown form
  are dropped.
