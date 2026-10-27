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
| ```` ```lang ```` fenced / indented code | `codeBlock` `{language}` | `language` is the first word of the info string, omitted if empty. One trailing newline is stripped. Content is never parsed as Markdown. An empty block has no `content` but keeps its `language`: ```` ``` ```` → `{"type":"codeBlock"}`, ```` ```rust ```` → `{"type":"codeBlock","attrs":{"language":"rust"}}`. |
| `> quote` | `blockquote` | See [Container rules](#container-rules). |
| `---` / `***` / `___` | `rule` | |
| GFM table | `table` > `tableRow` > `tableHeader`/`tableCell` > `paragraph` | See [Tables](#tables). |
| `> [!NOTE]` etc. | `panel` | ADF extension `alerts`; see [Panels](#panels-github-alerts). |
| `<details><summary>T</summary>` … `</details>` | `expand` / `nestedExpand` `{title}` | ADF extension `details`. |
| `::: cell` … `:::` | 1×1 `table` used as a box | ADF extension `cells`. |
| ```` ```adf ```` with ADF JSON | the JSON, spliced in | ADF extension `raw-adf`. |
| `[^1]` footnotes | superscript number + `rule` + numbered list at the end | ADF has no footnotes; `W_FOOTNOTE_FLATTENED`. Labels match case-insensitively, with Unicode case folding (`[^SS]` refers to `[^ß]`), as in GFM. |
| Leading `---` YAML front matter | removed | `W_FRONT_MATTER_DROPPED`, located at the removed lines. Only when it reads as YAML: the first line inside is `key: value`, and every other non-blank line before the closing `---` (or `...`) is a key, an indented line or a `- ` item. Otherwise (prose, headings, fences, tables) the `---` is a rule and the lines are Markdown. |
| HTML block | see [HTML](#html) | |

### Inline

| Markdown | ADF marks / nodes | Notes |
|---|---|---|
| `*em*` / `_em_` | `em` | |
| `**strong**` / `__strong__` | `strong` | |
| `***both***` | `em` + `strong` | |
| `~~strike~~` / `~strike~` | `strike` | |
| `` `code` `` (any number of backticks) | `code` | `code` combines **only** with `link`. Other marks around code are dropped: `W_MARK_DROPPED`. Empty code spans are skipped. |
| `[text](url "title")`, reference links | `link` `{href, title}` | `title` only if non-empty. `href` must pass the [URL allowlist](#url-allowlist); otherwise the text is kept without a link, `W_UNSAFE_URL`. Empty link text → the URL is used as text. Link text of only spaces (`[ ](url)`) is kept, even at the edge of a block. |
| `<https://x>`, `<a@b.c>` | `link` | Email → `mailto:`. |
| Bare `https://x`, `www.x.com` (GFM) | `link` | `www.` gets `http://`. Only URLs that appear literally in the source are linked, so `https\://x` stays text. |
| `![alt](url)` | text `alt` (or the URL) with a `link` mark | `W_IMAGE_AS_LINK`. `--images external` → block `mediaSingle` with external `media`, after the paragraph, heading or cell text (which keeps one space where the image was); external media has no title and cannot be linked, so an image title or a link around the image is dropped with `W_MARK_DROPPED`. `--images drop` → removed, `W_IMAGE_DROPPED`. Alt text is plain: line breaks (`<br>` too) become spaces. `![alt](adf:media/UUID)` is not a URL image: it references a file already uploaded to Jira or Confluence, see [File media](#file-media-adfmedia-images). |
| `<u>` / `<ins>` | `underline` | |
| `<s>` / `<del>` / `<strike>` | `strike` | |
| `<strong>` / `<b>`, `<em>` / `<i>` | `strong`, `em` | Used by ADF → Markdown when `**`/`*` would not parse. |
| `<sub>`, `<sup>` | `subsup` `{type: sub/sup}` | Text takes one `subsup`: `<sup>` inside `<sub>` (or the reverse) keeps the inner one, `W_MARK_DROPPED`. |
| `<span style="color: #RRGGBB">` | `textColor` | Only 6-digit hex. Any other `<span>` is [unrecognised HTML](#html); its `</span>` does not end a recognised span around it. |
| `<span style="background-color: #RRGGBB">` | `backgroundColor` | Only 6-digit hex. |
| `<kbd>` | `code` | |
| `<br>` | `hardBreak` | |
| `<!-- comment -->` | removed | Silent: comments are invisible in rendered Markdown. |
| Backslash escapes, entities (`&amp;`, `&#32;`) | decoded to literal text | A decoded `\n` becomes a `hardBreak`. |
| Control characters (except tab) | removed | `W_CONTROL_CHAR`, also in attributes: link and image titles, code block languages, `<summary>` titles, alt text and `adf:` labels. |

Adjacent text with the same marks is merged. Marks are emitted in Atlassian's canonical order:
link, em, strong, textColor, backgroundColor, strike, subsup, underline, code.

### ADF extensions

All extensions are on by default. `--extensions none` turns them off;
`--extensions alerts,details` keeps only the listed ones. Names: `alerts`, `details`, `cells`,
`raw-adf`, `mention`, `status`, `date`, `emoji`, `card`, `media`. When an extension is off, its
syntax is converted as plain Markdown, and `W_EXTENSION_DISABLED` is reported where the result
differs.

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

A panel can contain paragraphs, headings, lists, task lists, code blocks and rules. A table, a
quote or another alert inside one is [hoisted](#container-rules): the panel is split around it
(`W_BLOCK_HOISTED`). A `<details>` block inside one is flattened to a bold title and its content,
inside the panel (`W_BLOCK_FLATTENED`; see [Expand](#expand-details)). Panels can only be placed
in the document, an expand, a nested expand, or a table cell. An alert in a list item, a task item
or another panel is hoisted (`W_BLOCK_HOISTED`) and becomes a panel where it lands, so a warning
under a numbered step becomes a panel between the parts of the list. An alert inside a quote, or
hoisted into one, is merged into the quote as plain content, with `W_BLOCK_FLATTENED`. An empty
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
- The summary is plain text (tags inside it are removed, `<br>` is a space, entities are
  decoded). Unrecognised tags in it (`<code>`, `<img>`) are reported with `W_HTML_DROPPED`, or
  fail the conversion with `--html error`. A `<summary>` outside `<details>` is ordinary
  [HTML](#html).
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
  `tableCell` holding the content. A table inside a box is [hoisted](#container-rules): the box
  is closed before the table and reopened after it, so a box holding a paragraph, a table and
  another paragraph becomes a box, the table, and a second box, with `W_BLOCK_HOISTED`.
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
  are not, the conversion fails with `E_RAW_ADF` (exit 3) and the fence's location. That includes
  a task or decision `localId` that an earlier ```` ```adf ```` block already uses; generated task
  list IDs never reuse one.
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
- The label is plain text: a line break in it (a hard break, `<br>`, or a soft break with
  `--softbreak hardbreak`) is a space. An image in it (an `adf:media` image, or an external one
  with `--images external`) is moved after the node, with `W_BLOCK_HOISTED`.

#### File media (`adf:media` images)

Jira and Confluence store an uploaded screenshot or image as a reference to a Media Services file
(a `media` node of type `file`), not as a URL. `adf` is offline: it never uploads a file and never
looks an id up. You upload the file with `curl`, acli or your own code, learn its media id, and
write that id here. [Attaching files and screenshots](posting.md#attaching-files-and-screenshots)
shows the steps.

```markdown
![shot.png](adf:media/11111111-2222-3333-4444-555555555555)
![shot.png](adf:media/11111111-2222-3333-4444-555555555555?collection=contentId-123456&width=1512&height=945&layout=wide&size=720px)
```

The label is the alt text. The path after `adf:media/` is the id, optionally followed by `?` and
`&`-separated `key=value` pairs. The scheme and the kind are case-insensitive, like the other
`adf:` links. Values are used as written: there is no percent-decoding.

| Part | Rules | ADF |
|---|---|---|
| id | A UUID: 8-4-4-4-12 hexadecimal digits, case-insensitive. Written to ADF exactly as given. | `media` `attrs.id` |
| `collection` | 0 to 64 characters from `A-Za-z0-9_-`. Default: empty; `collection=` (empty) is accepted and means the default, the value Jira itself writes. Examples: `contentId-123456` (Confluence page attachment), `comment-container-123456` (Confluence comment attachment uploaded in the editor). | `media` `attrs.collection`, always present, `""` when not given |
| `width`, `height` | Integers 1 to 100000: the original size of the file in pixels. Either may be given alone. | `media` `attrs.width`, `attrs.height` |
| `layout` | `wide`, `full-width`, `center` (default), `wrap-right`, `wrap-left`, `align-end` or `align-start`. | `mediaSingle` `attrs.layout` |
| `size` | `N%` with N from 1 to 100, or `Npx` with N from 1 to 100000. The displayed width, not the size of the file. | `mediaSingle` `attrs.width` = N; for `px` also `attrs.widthType` = `pixel` |
| alt text | The label, as plain text (same rules as for external images; empty: no `alt`). | `media` `attrs.alt` |

The example above gives:

```json
{"type":"mediaSingle","attrs":{"layout":"wide","width":720,"widthType":"pixel"},"content":[{"type":"media","attrs":{"type":"file","id":"11111111-2222-3333-4444-555555555555","collection":"contentId-123456","alt":"shot.png","width":1512,"height":945}}]}
```

Without parameters the result is `mediaSingle` `{layout: "center"}` holding `media` `{type: "file",
id, collection: "", alt}`: `type`, `id` and `collection` always come first, then `alt`, `width`
and `height` when present.

- **Give `width` and `height`.** Atlassian's `media` documentation says that both must be set
  inside a `mediaSingle`, or the media does not display (that page dates from 2020 and has not
  been re-checked against the products; see
  [maintainers.md](maintainers.md#optional-checking-rendering-on-a-personal-jira-site)). Neither
  upload API returns a file's dimensions, so you have to read them from the file. `adf` does not
  warn when they are missing.
- **Invalid input.** An unknown key, a duplicate key, an invalid value, an empty value (except
  `collection=`), a bad id or a bad collection keeps the label as plain text and reports `W_INVALID_EXTENSION_ARG`, located at
  the image. The message names the argument that is wrong. The whole image falls back, not just
  the argument.
- **`--extensions` without `media`.** The image is not recognised as media: the label is kept as
  plain text, with `W_EXTENSION_DISABLED`.
- **`--images`.** `--images drop` removes the image with `W_IMAGE_DROPPED`. `--images link` (the
  default) and `--images external` do not affect these images: they are not URLs, so they never
  become links.
- **Title and link.** Media nodes have neither a title nor a link. An image title, or a link
  around the image, is dropped with `W_MARK_DROPPED`, as for external images.
- **Placement.** `mediaSingle` is a block, placed like an external image. In the middle of a
  paragraph (also in a list item, quote, panel, expand or `::: cell` box) it splits the paragraph:
  the text before it, the image, the text after it, without a warning. After text in a heading or
  a GFM table cell it follows the heading or the cell's text. It stays inside list items, quotes,
  panels, expands and table cells, because they all allow media. Only two places cannot hold it,
  and there it is moved out with `W_BLOCK_HOISTED`: a task item (the task list is closed after
  that item, the image follows, and the remaining items start a new task list) and an `adf:`
  link label (the image follows the node).
- **Not checked.** `adf` cannot know whether the id exists, belongs to this site or issue, or
  which collection the product expects. A made-up id converts without a warning; the product
  rejects or fails to render it when you post.
- Only the image form exists in this version. Links to files (`mediaGroup`, `mediaInline`) are
  not written from Markdown; use [raw ADF](#raw-adf-adf) for them.

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
- `adf` has no CSV input or output. The Agent Skill ships `scripts/csv2md.py` (CSV or TSV → Markdown
  table) and a `jq` recipe (table → CSV); see
  [`skills/markdown-to-adf/references/tables.md`](../skills/markdown-to-adf/references/tables.md).
- `\|` is a literal pipe, including inside code spans.

### Task lists

- `- [ ] x` → `TODO`; `- [x] x` / `- [X] x` → `DONE`.
- A task item holds inline content only. Several paragraphs are joined with `hardBreak`.
- A nested task list stays nested (`taskList` inside `taskList`, after its item).
- Other blocks under a task item (a plain sublist, code, a table) are hoisted out of the task list,
  with `W_BLOCK_HOISTED`: the list is closed after that item, the block follows it, and the items
  after that start a new `taskList`. So are images: `adf:media` images, and external ones
  (`--images external`). The task list is closed after the item that held the image, the image
  follows it, and the items after that item start a new `taskList`. The spaces around a moved
  image collapse to one.
- A list mixing checkbox and plain items is split into consecutive `taskList` and
  `bulletList`/`orderedList` runs.
- An ordered task list (`1. [ ] x`) becomes a `taskList`, with `W_TASK_NUMBERING_LOST`.
- `localId`s are deterministic: the same input always gives the same IDs.

### HTML

| `--html` | Unrecognised HTML (block or inline) |
|---|---|
| `text` (default) | Kept as literal text, `W_HTML_AS_TEXT`. |
| `drop` | Removed, `W_HTML_DROPPED`. The text between the tags stays, except inside `<script>` and `<style>`, which are removed with their content up to the closing tag (browsers never show it). |
| `error` | Conversion fails, `E_LOSSY`. |

The tags listed in [Inline](#inline), `<details>`/`<summary>` (when the `details` extension is
on) and comments are always recognised.

### Container rules

ADF restricts which blocks may appear where. When a block is not allowed in its container:

1. **Demote**: a heading in a list item or quote → bold paragraph (`W_HEADING_DEMOTED`).
2. **Flatten**:
   - a quote inside a quote → merged into the outer quote;
   - an alert in a quote, or `<details>` where an expand is not allowed → converted as
     described above.

   Both report `W_BLOCK_FLATTENED`.
3. **Convert**: an expand inside an expand or a table cell becomes a `nestedExpand`, without a
   warning.
4. **Hoist**: for anything else (a table in a list, a rule in a quote, a quote in a list item, an
   alert in a list item or a panel, a table in a box cell…):
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
`javascript:`, `data:`, relative paths, protocol-relative `//host`, and `/\host`, which browsers
also read as another host) becomes plain text, with `W_UNSAFE_URL`. This is a strict subset of
Atlassian's own `isSafeUrl`.

### Rules the converter always enforces

The schema alone does not catch everything Jira rejects. These rules hold for all output, whether
`adf validate` reports a break of them as `E_SCHEMA` (for example an empty text node, or `code`
with `strong`), `E_TARGET` or `E_INVARIANT`:

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

`adf validate` and ```` ```adf ```` blocks check ADF that comes from elsewhere against the rules
Jira and Confluence themselves enforce, which are looser in two places. A list item may also
start with a `codeBlock` or a `mediaSingle` (only a nested list first is rejected). Several
`annotation` marks with different `id`s may share a text node (overlapping inline comments). In
that input the URL allowlist also applies to the `data.url` of smart links in the JSON-LD form and
to the `url` of external `media`.

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
| `--md-flavor roundtrip` (default) | Uses the extension syntax above, so `md → adf` with `--images external` gives the same document back (see the [round-trip guarantee](#round-trip-guarantee)). Nodes Markdown cannot express become ```` ```adf ```` blocks (`W_ADF_PASSTHROUGH`). |
| `--md-flavor plain` | Compact, readable Markdown for people and LLMs. Drops what Markdown cannot express, with no `adf` blocks. Mentions become `@Name`, statuses `[TEXT]`, dates `YYYY-MM-DD`, emoji their character, panels alerts, expands `<details>`, cards bare URLs (only allowed URLs, escaped like text), external images `![alt](url)` with their caption on the next line, uploaded images `[image: alt]`, attachments `[attachment: name]` or `[attachment]` (see [Plain flavour: media](#plain-flavour-media)). Merged table cells keep their text in their first column and leave the columns they span empty. Rows shorter than the header are not padded (GFM readers pad them). Control characters in code become U+FFFD (CR LF and a lone CR in code blocks become a line break). Trailing spaces are dropped. Plain mode is lossy by design and reports no warnings for it, but it does not drop media silently. |

### Mapping (roundtrip)

| ADF | Markdown |
|---|---|
| `heading` | ATX `#`…`######`; `hardBreak` inside → `<br>` |
| `paragraph` | text; `hardBreak` → `\` + newline; leading/trailing spaces → `&#32;` (tabs `&#9;`), also inside marks at the edges of a paragraph, heading, task item or table cell, and at the start of a line; U+FEFF starting the document → `&#65279;` |
| marks | `*em*`, `**strong**`, `~~strike~~`, `` `code` ``, `[t](href "title")`, `<u>`, `<sub>`, `<sup>`, `<span style="color: #hex">`, `<span style="background-color: #hex">`. When `*`/`**`/`~~` would not parse (whitespace or punctuation at the edge, inside a word), the HTML form (`<em>`, `<strong>`, `<del>`) is used. |
| text | Markdown-special characters are escaped so they stay literal. `://` in unlinked text is written `\://` so it is not autolinked. Text holding control characters other than tab and line feed → ```` ```adf ```` block (`W_ADF_PASSTHROUGH`), because Markdown → ADF removes them; the JSON escapes them, so they never reach a terminal raw. |
| `bulletList`, `orderedList {order}` | `-` / `N.` lists; nesting by indentation; an item whose first paragraph is empty → `-` alone on its line, or `- ![alt](url)` when an external image follows it. A blank line separates a list that cannot interrupt a paragraph (`N.` with `N` ≠ 1, an empty first item) from a paragraph line before it. |
| `taskList`, `taskItem` | `- [ ]` / `- [x]` |
| `codeBlock {language}` | fenced code, with a fence longer than any backtick run in the content; `\` and `&` in the language are escaped. Language `adf` → ```` ```adf ```` block with the node's JSON (`W_ADF_PASSTHROUGH`), because a fence with that language is raw ADF. Code or a language holding control characters other than tab and line feed (CR included) → ```` ```adf ```` block. |
| `blockquote` | `>`; a quote holding one empty paragraph → `>` alone |
| `rule` | `---` (`***` as the first block, where `---` could open front matter) |
| `panel` info / success / note / warning / error | `> [!NOTE]` / `[!TIP]` / `[!IMPORTANT]` / `[!WARNING]` / `[!CAUTION]` |
| `expand`, `nestedExpand` | `<details><summary>title</summary>` … `</details>`. A title that Markdown → ADF would change (whitespace at the ends, tabs, line breaks, repeated spaces, control characters) → ```` ```adf ```` block. |
| `table` 1×1 with a single `tableCell` and default attrs | `::: cell` … `:::` |
| `table` with a header row, one paragraph per cell (optionally followed by images: external, or file images that fit the [read-back rule](#reading-file-media-back), written after the cell's text), no spans, consistent column alignment | GFM table (a cell or list item that holds only an image is the exception, see [below](#reading-file-media-back)) |
| `mention`, `status`, `date`, `emoji`, `inlineCard` | the `adf:` link forms above |
| `mediaSingle` with external `media` | `![alt](url)` |
| `mediaSingle` with one Media Services file that fits [the read-back rule](#reading-file-media-back) | `![alt](adf:media/UUID?…)` |
| anything else: other tables, other Media Services files (`mediaGroup`, `mediaInline`, captions, marks), `decisionList`, layouts, extensions, unknown nodes | ```` ```adf ```` block with the node's JSON (`W_ADF_PASSTHROUGH`), pretty-printed in its key order for the first 16 JSON levels and compact below |

In the roundtrip flavour, attributes and marks that Markdown cannot express are dropped and
reported with `W_ATTR_DROPPED`, whose `pointer` locates the node or the mark. This covers:
- **attributes** without a Markdown form, for example cell `colwidth`, `background` and
  `valign`, table `width`/`layout`/`displayMode`, a numbered column, media size, panel icons,
  mention `accessLevel`, emoji `id`, codeBlock `uniqueId`, and link attributes other than `href`
  and `title` (such as Confluence's `__confluenceMetadata`);
- **block marks**: `alignment` (except on the paragraphs of a GFM table's cells), `indentation`,
  `breakout`;
- **text marks** without a Markdown form, notably Confluence's `annotation` marks, which anchor
  inline comments: the text is kept, so posting the Markdown back detaches those comments;
- marks with a value Markdown cannot carry: a link whose URL fails the
  [URL allowlist](#url-allowlist) (the text is kept), a `textColor`/`backgroundColor` that is not
  `#RRGGBB`, a `subsup` without a valid `type`.

Carriage returns in text and code blocks (Windows CR LF line endings, or a lone CR) are written as
line breaks, with `W_CONTROL_CHAR`. Other control characters can't be written in Markdown: in
roundtrip the node becomes a ```` ```adf ```` block, and in plain they become U+FFFD.

Attributes whose value carries no information (`null`, `""`, `false`, or the default, such as
mention `userType: "DEFAULT"`) are dropped silently, and so are `localId`s, because they are editor
bookkeeping. The plain flavour drops all of these without warnings.

### Reading file media back

In the roundtrip flavour, a `mediaSingle` becomes an `![alt](adf:media/UUID?…)` line exactly when
`adf` can write every attribute it holds, so that [converting it back](#round-trip-guarantee)
gives the same document. All of these must hold:

- it has exactly one child, a `media` node of `type` `file`, and no `caption`;
- neither node has `marks`;
- `media` has only these attributes: `type`, `id`, `collection`, `alt`, `width`, `height`,
  `localId`. `id` is a UUID, `collection` is present and uses the characters `A-Za-z0-9_-` (at
  most 64, and it may be `""`), `width` and `height` are absent or integers from 1 to 100000, and
  `alt` is absent or a string. A present `alt` has no control characters or line breaks and no
  leading or trailing whitespace (Unicode whitespace counts, so a whitespace-only `alt` fails
  too); an empty `alt` is fine and is dropped;
- `mediaSingle` has only these attributes: `layout`, `width`, `widthType`, `localId`. `layout` is
  absent or one of the layouts listed [above](#file-media-adfmedia-images). `width` and
  `widthType` are either both absent, or `width` is an integer from 1 to 100 and `widthType` is
  absent or `percentage` (written `size=N%`), or `width` is an integer from 1 to 100000 and
  `widthType` is `pixel` (written `size=Npx`).

The query parameters are written in the order `collection`, `width`, `height`, `layout`, `size`.
Defaults are left out: an empty `collection` and the `center` layout. `localId`s are dropped
silently, as everywhere. A `mediaSingle` without `layout` is read as `center`, so it comes back
with `"layout":"center"`.

Everything else keeps its raw ```` ```adf ```` block with `W_ADF_PASSTHROUGH`: a file
`mediaSingle` with a caption, with marks (Confluence's `border`, for example), with other
attributes (`occurrenceKey`, for example) or with an id that is not a UUID; a `media` of type
`link`; a `mediaGroup`; and a paragraph holding a `mediaInline`. A lone external `mediaSingle`
keeps its `![alt](url)` form.

**Limitation.** A file image is written inside a table cell or list item only after a paragraph
(`| text![alt](adf:media/…) |`, `- text` followed by the image line), because `| ![alt](…) |` reads
back as an empty paragraph followed by the image. A table cell whose only child is a
`mediaSingle`, or a list item that starts with one, is therefore not a GFM shape: the whole table
or list becomes one ```` ```adf ```` block (`W_ADF_PASSTHROUGH`). The `::: cell` box of a 1×1
table is not affected.

### Plain flavour: media

`--md-flavor plain` has no syntax for Media Services files, because plain Markdown is for reading.
It still never loses a media node without a trace. Each of these leaves a marker in the text,
without a warning, wherever the node is, including in table cells and inline:

| ADF | Plain Markdown |
|---|---|
| `mediaSingle` with a file | `[image: alt]` (`[image]` when there is no alt) |
| a file in a `mediaGroup` | `[attachment: alt]`, or `[attachment]` when there is no alt; one marker per file, each on its own line |
| `mediaInline` | the same marker as a `mediaGroup` file, inline in its paragraph, inserted as is: no spaces are added around it, so only the spaces already in the neighbouring text separate it (`Log: ` + chip + ` attached.` gives `Log: [attachment] attached.`, but text directly next to the chip runs into it) |
| `mediaSingle` with external media | `![alt](url)` (the caption goes on the next line) |

Markers are not read back: converting plain Markdown to ADF gives text, not media. Use the
roundtrip flavour to edit and post content that holds media.

### Round-trip guarantee

For a document `x` that contains only constructs with a roundtrip mapping above:

```
md_to_adf(adf_to_md(x)) == normalize(x)
```

where `normalize`:
- removes all `localId` attributes, except on task lists and items, which are compared by
  position;
- splits `\n` inside text nodes into `hardBreak`s;
- gives a file `mediaSingle` without a `layout` the layout `center`;
- drops an empty file `alt` and an explicit `widthType` of `percentage` (the default), both
  silently;
- drops every attribute and mark that `adf_to_md` drops, as described above (each one is
  reported with `W_ATTR_DROPPED`, so a conversion without that warning drops nothing but
  `localId`s and empty or default attributes).

`md_to_adf` here runs for the document's target, with all ADF extensions on (the roundtrip
Markdown is written with them) and with `--images external` (`![alt](url)` stands for an external
image; `![alt](adf:media/…)` stands for a file and does not depend on `--images`, except that
`--images drop` removes it). To edit converted content and post it back, convert the Markdown with the same options:
`adf convert FILE.md --images external` plus the envelope (`-e jira-description`,
`-e confluence`, …). With the default `--images link`, each external image becomes a text link
(`W_IMAGE_AS_LINK`).

Every document produced by `md → adf` (with any options) satisfies this. The test suite checks it
with goldens and property tests.
