# Syntax: Markdown ⇄ ADF

This is the **normative** mapping between Markdown and Atlassian Document Format (ADF) used by
`adf`. Tests, `adf capabilities`, and the agent skill follow this document. If the code and this
document disagree, that's a bug.

- Input Markdown is [CommonMark 0.31](https://spec.commonmark.org/0.31.2/) with the
  [GitHub Flavored Markdown](https://github.github.com/gfm/) extensions (tables, task lists,
  strikethrough, autolinks, footnotes, alerts), plus the **ADF extensions** described below.
- ADF output always validates against the vendored ADF JSON schema
  (`schemas/adf/full.json`, `@atlaskit/adf-schema` 57.7.18) **and** against the extra rules Jira
  enforces (see [Rules](#rules-the-converter-always-enforces)).
- Every lossy step is reported with a `W_…` warning code (see [cli-contract.md](cli-contract.md)).
  Under `--strict`, any warning makes the conversion fail with `E_LOSSY` (exit 5).

## Markdown → ADF

### Blocks

| Markdown | ADF | Notes |
|---|---|---|
| `# H1` … `###### H6`, setext `===`/`---` | `heading` `{level: 1–6}` | In a list item or quote (where ADF forbids headings) → paragraph with all text **bold**, `W_HEADING_DEMOTED`. |
| Paragraph | `paragraph` | Never created for blank lines. |
| Soft line break (newline inside a paragraph) | a space | `--softbreak hardbreak` → `hardBreak`. |
| Hard break (two trailing spaces, `\` at line end, `<br>`) | `hardBreak` | A trailing hard break at the end of a paragraph is dropped. |
| `- item` / `* item` / `+ item` | `bulletList` > `listItem` | Tight and loose lists produce the same ADF; a loose item holds several paragraphs. |
| `1. item` / `1) item` | `orderedList` `{order: N}` | `order` is always emitted, equal to the start number. |
| Nested lists | `listItem` > [`paragraph`, `bulletList` …] | A list item **always starts with a paragraph**; an empty one is inserted if needed. |
| `- [ ] task` / `- [x] task` | `taskList` > `taskItem` `{state: TODO/DONE}` | Deterministic `localId`s (UUIDv8 from a hash of the input). See [Task lists](#task-lists). |
| ```` ```lang ```` fenced / indented code | `codeBlock` `{language}` | `language` is the first word of the info string, omitted if empty. One trailing newline is stripped. Content is never parsed as Markdown. Empty block → `{"type":"codeBlock"}`. |
| `> quote` | `blockquote` | See [Container rules](#container-rules). |
| `---` / `***` / `___` | `rule` | |
| GFM table | `table` > `tableRow` > `tableHeader`/`tableCell` > `paragraph` | See [Tables](#tables). |
| `> [!NOTE]` etc. | `panel` | ADF extension `alerts`; see [Panels](#panels-github-alerts). |
| `<details><summary>T</summary>` … `</details>` | `expand` / `nestedExpand` `{title}` | ADF extension `details`. |
| `::: cell` … `:::` | 1×1 `table` used as a box | ADF extension `cells`. |
| ```` ```adf ```` with ADF JSON | the JSON, spliced in | ADF extension `raw-adf`. |
| `[^1]` footnotes | superscript number + `rule` + numbered list at the end | ADF has no footnotes; `W_FOOTNOTE_FLATTENED`. |
| Leading `---` YAML front matter | removed | `W_FRONT_MATTER_DROPPED`. |
| HTML block | see [HTML](#html) | |

### Inline

| Markdown | ADF marks / nodes | Notes |
|---|---|---|
| `*em*` / `_em_` | `em` | |
| `**strong**` / `__strong__` | `strong` | |
| `***both***` | `em` + `strong` | |
| `~~strike~~` / `~strike~` | `strike` | |
| `` `code` `` (any number of backticks) | `code` | `code` combines **only** with `link`. Other marks around code are dropped: `W_MARK_DROPPED`. Empty code spans are skipped. |
| `[text](url "title")`, reference links | `link` `{href, title}` | `title` only if non-empty. `href` must pass the [URL allowlist](#url-allowlist); otherwise the text is kept without a link, `W_UNSAFE_URL`. Empty link text → the URL is used as text. |
| `<https://x>`, `<a@b.c>` | `link` | Email → `mailto:`. |
| Bare `https://x`, `www.x.com` (GFM) | `link` | `www.` gets `http://`. Only URLs that appear literally in the source are linked, so `https\://x` stays text. |
| `![alt](url)` | text `alt` (or the URL) with a `link` mark | `W_IMAGE_AS_LINK`. `--images external` → block `mediaSingle` with external `media`; an image title and a link around the image are dropped, `W_MARK_DROPPED`. `--images drop` → removed, `W_IMAGE_DROPPED`. |
| `<u>` / `<ins>` | `underline` | |
| `<s>` / `<del>` / `<strike>` | `strike` | |
| `<strong>` / `<b>`, `<em>` / `<i>` | `strong`, `em` | Used by ADF → Markdown when `**`/`*` would not parse. |
| `<sub>`, `<sup>` | `subsup` `{type: sub/sup}` | |
| `<span style="color: #RRGGBB">` | `textColor` | Only 6-digit hex. |
| `<span style="background-color: #RRGGBB">` | `backgroundColor` | Only 6-digit hex. |
| `<kbd>` | `code` | |
| `<br>` | `hardBreak` | |
| `<!-- comment -->` | removed | Silent: comments are invisible in rendered Markdown. |
| Backslash escapes, entities (`&amp;`, `&#32;`) | decoded to literal text | A decoded `\n` becomes a `hardBreak`. |
| Control characters (except tab) | removed | `W_CONTROL_CHAR`. |

Adjacent text with the same marks is merged. Marks are emitted in Atlassian's canonical order:
link, em, strong, textColor, backgroundColor, strike, subsup, underline, code.

### ADF extensions

All extensions are on by default. `--extensions none` turns them off;
`--extensions alerts,details` keeps only the listed ones. Names: `alerts`, `details`, `cells`,
`raw-adf`, `mention`, `status`, `date`, `emoji`, `card`. When an extension is off, its syntax is
converted as plain Markdown, and `W_EXTENSION_DISABLED` is reported where the result differs.

#### Panels (GitHub alerts)

GitHub alert syntax: the first line of a blockquote is `[!KIND]` (case-insensitive).

```markdown
> [!WARNING]
> Back up the database first.
```

| Alert | `panelType` |
|---|---|
| `[!NOTE]` | `info` |
| `[!TIP]` | `success` |
| `[!IMPORTANT]` | `note` |
| `[!WARNING]` | `warning` |
| `[!CAUTION]` | `error` |

A panel can contain paragraphs, headings, lists, task lists, code blocks and rules. A table,
quote, panel or details block inside one is [hoisted](#container-rules). Panels can only be placed
in the document, an expand, a nested expand, or a table cell. Anywhere else (a list item, a quote,
another panel), the alert is converted as a plain blockquote, with `W_BLOCK_FLATTENED`. An empty
alert holds one empty paragraph.

#### Expand (`<details>`)

```markdown
<details>
<summary>Logs</summary>

Markdown content, separated from the tags by blank lines (as on GitHub).

</details>
```

- In the document → `expand` `{title}`. Inside an `expand` or a table cell → `nestedExpand`.
  `attrs` is always emitted, as `{}` when there is no `<summary>`.
- The summary is plain text (tags inside it are removed, entities decoded).
- In containers that cannot hold an expand (list item, quote, panel, nested expand), the block is
  flattened: the title becomes a bold paragraph followed by the content, with `W_BLOCK_FLATTENED`.
- An empty body becomes one empty paragraph.

#### Box cells (`::: cell`)

Jira descriptions often use a 1×1 table as a framed box. Write one with a fenced container:

```markdown
::: cell
Any Markdown: paragraphs, **lists**, code, alerts, `<details>`…
:::
```

- The opening line is three or more `:` characters, optional spaces, and the word `cell`. The
  closing line is the same number of `:` characters. An unclosed box ends at the end of the input.
- Recognised only at the start of a line with no indentation, and not inside fenced code.
- Result: `table` `{isNumberColumnEnabled: false, layout: "default"}` with one row and one
  `tableCell` holding the content. Tables inside a box are hoisted after it.
- Boxes cannot be nested. An inner `::: cell` line is kept as text, with `W_BLOCK_FLATTENED`.

#### Raw ADF (```` ```adf ````)

The escape hatch for anything Markdown cannot express:

````markdown
```adf
{"type": "panel", "attrs": {"panelType": "success"}, "content": [{"type": "paragraph"}]}
```
````

- The content is one JSON object (a block node) or an array of block nodes.
- The nodes are spliced in unchanged, after checking they are valid ADF in that position. If they
  are not, the conversion fails with `E_RAW_ADF` (exit 3) and the fence's location.
- Inline nodes are not supported here.

#### Inline extensions (`adf:` links)

These are ordinary Markdown links with an `adf:` URL, so other Markdown renderers still show the
label.

| Markdown | ADF | Rules |
|---|---|---|
| `[@Jane Doe](adf:mention/5b10ac8d82e05b22cc7d4ef5)` | `mention` `{id, text: "@Jane Doe"}` | The path is the Atlassian account ID. The label is the display text. |
| `[DONE](adf:status/green)` | `status` `{text: "DONE", color: "green"}` | Colours: `neutral` (default when there is no `/color`), `purple`, `blue`, `red`, `yellow`, `green`. The label must not be empty. |
| `[2026-10-19](adf:date)` | `date` `{timestamp: "<ms at 00:00 UTC>"}` | The label must be `YYYY-MM-DD`. |
| `[😄](adf:emoji/:smile:)` | `emoji` `{shortName: ":smile:", text: "😄"}` | The path is the short name (colons optional; they are added). An empty label omits `text`. |
| `[https://x.atlassian.net/browse/A-1](adf:card)` | `inlineCard` `{url}` | The label must pass the [URL allowlist](#url-allowlist). |

- An invalid argument (bad colour, bad date, empty status) keeps the label as plain text, with
  `W_INVALID_EXTENSION_ARG`.
- An unknown kind (`adf:foo/…`) keeps the label as plain text, with `W_UNKNOWN_EXTENSION`.
- These nodes cannot carry marks. Formatting around them (for example `**[@A](adf:mention/x)**`)
  is dropped, with `W_MARK_DROPPED`.

### Tables

- The GFM header row becomes `tableHeader` cells, and the body rows become `tableCell` cells.
- Each cell holds one paragraph with the cell's inline content (empty cell: one empty
  paragraph). `<br>` inside a cell becomes `hardBreak`.
- Column alignment becomes an `alignment` mark on each cell's paragraph: `:---:` → `center`,
  `---:` → `end`, left or none → no mark.
- Rows are padded with empty cells to the header width. Extra cells are dropped, with
  `W_TABLE_CELL_DROPPED`.
- All tables of a document together may have at most 100,000 cells, padding included; more fails
  with `E_INPUT_TOO_LARGE` (see the [limits](cli-contract.md#limits)).
- Tables are only valid in the document and in an expand. Elsewhere they are
  [hoisted](#container-rules).
- `\|` is a literal pipe, including inside code spans.

### Task lists

- `- [ ] x` → `TODO`; `- [x] x` / `- [X] x` → `DONE`.
- A task item holds inline content only. Several paragraphs are joined with `hardBreak`.
- A nested task list stays nested (`taskList` inside `taskList`, after its item).
- Other blocks under a task item (a plain sublist, code, a table) are hoisted after the task list,
  with `W_BLOCK_HOISTED`. So are external images (`--images external`); the spaces around a moved
  image collapse to one.
- A list mixing checkbox and plain items is split into consecutive `taskList` and
  `bulletList`/`orderedList` runs.
- An ordered task list (`1. [ ] x`) becomes a `taskList`, with `W_TASK_NUMBERING_LOST`.
- `localId`s are deterministic: the same input always gives the same IDs.

### HTML

| `--html` | Unrecognised HTML (block or inline) |
|---|---|
| `text` (default) | Kept as literal text, `W_HTML_AS_TEXT`. |
| `drop` | Removed, `W_HTML_DROPPED`. |
| `error` | Conversion fails, `E_LOSSY`. |

The tags listed in [Inline](#inline), `<details>`/`<summary>` (when the `details` extension is
on) and comments are always recognised.

### Container rules

ADF restricts which blocks may appear where. When a block is not allowed in its container:

1. **Demote**: a heading in a list item or quote → bold paragraph (`W_HEADING_DEMOTED`).
2. **Flatten**:
   - a quote inside a quote → merged into the outer quote;
   - an alert or `<details>` where a panel or expand is not allowed → converted as described
     above.

   Both report `W_BLOCK_FLATTENED`.
3. **Convert**: an expand inside an expand or a table cell becomes a `nestedExpand`, without a
   warning.
4. **Hoist**: for anything else (a table in a list, a rule in a quote, a quote in a list item, a
   panel in a list, a table in a box cell…):
   - the container is closed before the block;
   - the block is placed in the nearest ancestor that allows it;
   - the container is reopened afterwards for the remaining content. A reopened ordered list
     continues its numbering.

   Reports `W_BLOCK_HOISTED`.

Allowed children, from the schema:

| Container | May contain |
|---|---|
| document | paragraph, heading, lists, task list, code, quote, rule, panel, table, expand, media |
| list item | paragraph (first), lists, task list, code, media |
| quote | paragraph, lists, code, media |
| panel | paragraph, heading, lists, task list, code, rule, media |
| table cell | paragraph, heading, lists, task list, code, quote, panel, rule, nested expand, media |
| expand | everything a document may contain, except expand (→ nested expand) |
| nested expand | paragraph, heading, lists, task list, code, quote, panel, rule, media |

### URL allowlist

Links and smart links must use `http://`, `https://`, `ftp://`, `ftps://`, `mailto:`, `tel:`, a
root-relative path (`/browse/PROJ-1`) or an anchor (`#section`). Everything else (for example
`javascript:`, `data:`, relative paths, protocol-relative `//host`) becomes plain text, with
`W_UNSAFE_URL`. This is a strict subset of Atlassian's own `isSafeUrl`.

### Rules the converter always enforces

The schema alone does not catch everything Jira rejects. These rules hold for all output:

- no empty text nodes;
- no `"marks": []` and no `null` attributes;
- `code` never combined with any mark except `link`;
- marks are unique per text node;
- a list item starts with a paragraph;
- task lists and task items have unique `localId`s;
- table rows are rectangular, and cells hold block content;
- integer `level` / `order`;
- named status colours only;
- none of these, which are recently added or product-specific:
  - `fontSize`
  - codeBlock `wrap`/`hideLineNumbers`
  - cell `valign`
  - layouts
  - extensions
  - stage-0 nodes

## ADF → Markdown

`adf convert --from adf` accepts:
- a bare document;
- any envelope `adf` produces;
- Jira `GET /issue` responses (`fields.description`, or `--field NAME`);
- Jira comment objects (`body`);
- Confluence `atlas_doc_format` bodies (where the document is a JSON string).

### Flavours

| Flavour | Purpose |
|---|---|
| `--md-flavor roundtrip` (default) | Uses the extension syntax above, so `md → adf` gives the same document back. Nodes Markdown cannot express become ```` ```adf ```` blocks (`W_ADF_PASSTHROUGH`). |
| `--md-flavor plain` | Compact, readable Markdown for people and LLMs. Drops what Markdown cannot express, with no `adf` blocks. Mentions become `@Name`, statuses `[TEXT]`, dates `YYYY-MM-DD`, emoji their character, panels alerts, expands `<details>`, cards bare URLs. Plain mode is lossy by design and reports no warnings for it. |

### Mapping (roundtrip)

| ADF | Markdown |
|---|---|
| `heading` | ATX `#`…`######`; `hardBreak` inside → `<br>` |
| `paragraph` | text; `hardBreak` → `\` + newline; leading/trailing spaces → `&#32;` (tabs `&#9;`), also inside marks at the edges of a paragraph, heading, task item or table cell, and at the start of a line; U+FEFF starting the document → `&#65279;` |
| marks | `*em*`, `**strong**`, `~~strike~~`, `` `code` ``, `[t](href "title")`, `<u>`, `<sub>`, `<sup>`, `<span style="color: #hex">`, `<span style="background-color: #hex">`. When `*`/`**`/`~~` would not parse (whitespace or punctuation at the edge, inside a word), the HTML form (`<em>`, `<strong>`, `<del>`) is used. |
| text | Markdown-special characters are escaped so they stay literal. `://` in unlinked text is written `\://` so it is not autolinked. |
| `bulletList`, `orderedList {order}` | `-` / `N.` lists; nesting by indentation; an item whose first paragraph is empty → `-` alone on its line, or `- ![alt](url)` when an external image follows it. A blank line separates a list that cannot interrupt a paragraph (`N.` with `N` ≠ 1, an empty first item) from a paragraph line before it. |
| `taskList`, `taskItem` | `- [ ]` / `- [x]` |
| `codeBlock {language}` | fenced code, with a fence longer than any backtick run in the content; `\` and `&` in the language are escaped. Language `adf` → ```` ```adf ```` block with the node's JSON (`W_ADF_PASSTHROUGH`), because a fence with that language is raw ADF. |
| `blockquote` | `>`; a quote holding one empty paragraph → `>` alone |
| `rule` | `---` (`***` as the first block, where `---` could open front matter) |
| `panel` info / success / note / warning / error | `> [!NOTE]` / `[!TIP]` / `[!IMPORTANT]` / `[!WARNING]` / `[!CAUTION]` |
| `expand`, `nestedExpand` | `<details><summary>title</summary>` … `</details>` |
| `table` 1×1 with a single `tableCell` and default attrs | `::: cell` … `:::` |
| `table` with a header row, one paragraph per cell (optionally followed by external images, written after the cell's text), no spans, consistent column alignment | GFM table |
| `mention`, `status`, `date`, `emoji`, `inlineCard` | the `adf:` link forms above |
| `mediaSingle` with external `media` | `![alt](url)` |
| anything else: other tables, Media Services files, `decisionList`, layouts, extensions, unknown nodes | ```` ```adf ```` block with the node's JSON (`W_ADF_PASSTHROUGH`) |

Attributes with no Markdown form are dropped with `W_ATTR_DROPPED`: cell `colwidth` and
`background`, table `width`/`layout`/`displayMode`, a numbered column, media size, panel icons.
`localId`s are always dropped silently, because they are editor bookkeeping.

### Round-trip guarantee

For a document `x` that contains only constructs with a roundtrip mapping above:

```
md_to_adf(adf_to_md(x)) == normalize(x)
```

where `normalize`:
- removes all `localId` attributes, except on task lists and items, which are compared by
  position;
- splits `\n` inside text nodes into `hardBreak`s;
- drops the attributes listed under `W_ATTR_DROPPED`.

`md_to_adf` here runs for the document's target, with all ADF extensions on (the roundtrip
Markdown is written with them) and with `--images external` (`![alt](url)` stands for an external
image).

Every document produced by `md → adf` (with any options) satisfies this. The test suite checks it
with goldens and property tests.
