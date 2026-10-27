# CLI contract

This is the stable interface of `adf` for scripts and AI agents. Everything here follows semantic
versioning:
- **additions** (new flags, codes, JSON fields) are minor changes;
- **renames, removals and behaviour changes** are breaking.

The JSON diagnostics and the documents of `validate --json`, `capabilities --json` and
`examples --json` carry `"schema_version": 1`. The first three have JSON Schemas, which ship in
`schemas/*.v1.json` and are printed by `adf schema …`; `examples --json` has none.
`version --json` has neither a `schema_version` nor a schema (its shape is
[below](#version---json)).

## Commands

```
adf convert [INPUT|-] [-o PATH] [--from md|adf|csv] [--to md|csv] [-e ENVELOPE] [--field NAME]
            [-t auto|jira|confluence] [--pretty|--compact] [--strict]
            [--softbreak space|hardbreak] [--html text|drop|error]
            [--images link|external|drop] [--extensions all|none|LIST]
            [--include-root DIR]                                                  (--from md)
            [--md-flavor roundtrip|plain] [--error-format auto|text|json] [-q] [--no-color]
            [--delimiter DELIM] [--no-header] [--max-rows N] [--align-numbers]   (--from csv)
            [--table N] [--csv-safe]                                              (--to csv)
adf validate [INPUT|-] [--from auto|md|adf|csv] [-e ENVELOPE] [--field NAME] [-t …] [--strict]
             [--json] [--softbreak …] [--html …] [--images …] [--extensions …]
             [--include-root DIR]                                                  (--from md)
             [--delimiter DELIM] [--no-header] [--max-rows N] [--align-numbers]   (--from csv)
adf capabilities [--json]
adf examples [FEATURE] [--json]
adf schema adf|diagnostic|capabilities|validate-report
adf explain exit-codes|envelopes|syntax|codes|<CODE>
adf completions bash|zsh|fish|powershell|elvish
adf version [--json]            (also: adf --version)
```

`--error-format`, `-q/--quiet` and `--no-color` are global: every command accepts them (see
[Diagnostics](#diagnostics)). There is no default subcommand: `adf FILE` is a usage error
(`E_USAGE`, exit 2).

### `convert`

- **Input.** `INPUT` is a path, or `-` / omitted for stdin. If stdin is a terminal and no input
  is given, a short usage message is printed to stderr and the exit code is 2.
- **`--from md`** (default): Markdown → ADF.
- **`--from adf`**: ADF → Markdown, or with `--to csv` ADF → CSV. The input may be a bare document
  or any [envelope](#envelopes). `-e` is not allowed with `--from adf` (exit 2).
- **`--from csv`**: CSV or TSV → an ADF document that holds one table; see
  [CSV input](#csv-input---from-csv). The text is read as UTF-8 (a BOM is ignored).
- **`--to md|csv`** (default `md`): the output of `--from adf`. `--to csv` writes one table of the
  document as CSV text; see [CSV output](#csv-output---from-adf---to-csv). `--to csv` is only
  valid with `--from adf`; with `--from md` or `--from csv` it is a usage error (exit 2).
- **`-o PATH`**: write to a file, atomically (temporary file in the same directory, then rename),
  so the directory must be writable. Nothing is written if the conversion fails.
  - Replacing an existing file keeps its permission bits (not its owner or ACLs). A symbolic link
    at `PATH` is replaced by a regular file (the file it pointed to is unchanged), and a hard link
    is detached from the file's other names.
  - An existing `PATH` that is not a regular file, or that lies under `/dev` or `/proc`, is opened
    and appended to directly, without a temporary file: `/dev/null`, `/dev/stdout`, a FIFO, the
    `/dev/fd/N` of a process substitution `>(…)`.
- **Output format.** Pretty JSON when stdout is a terminal, compact otherwise. Files are compact
  unless `--pretty` is given. `--pretty` and `--compact` override. Markdown output always ends with
  exactly one newline, and CSV output with exactly one line feed.
- **`--strict`**: any warning fails the conversion with `E_LOSSY` (exit 5), and nothing is
  written.
- **`-t/--target`**: which product's rules to validate against. `auto` (default) is `confluence`
  for `-e confluence` and `jira` otherwise.
- **Self-check.** Output is checked before it is written (schema, target rules, invariants). If
  `adf`'s own output fails that check, it reports `E_INTERNAL` (exit 1), because that is a bug.
- **`--include-root DIR`** (Markdown input only): allow `![alt](adf:csv/PATH)` images to read CSV
  files under `DIR` and include them as tables; see
  [Including a CSV file](syntax.md#including-a-csv-file). **Off by default**: without the flag
  nothing is read, whatever the Markdown says (`W_INCLUDE_DISABLED`). There is no default
  directory, not even the one of the input file, and no environment variable. `DIR` must exist and
  be a directory (`E_USAGE`, exit 2, checked before any input is read); it is canonicalised once,
  and every path must resolve inside it. The Markdown decides what is read, so `DIR` must hold
  only data you are willing to publish: never a home directory, a repository root or a directory
  with secrets, and do not give the flag to an agent that generates Markdown from untrusted text.
  With `--from adf` or `--from csv` the flag is a usage error (exit 2). Stdin works with a root.
  Includes are bounded: at most 16 files per document, 1 MiB per file, 4 MiB in all
  ([Limits](#limits)). A refused include is `E_INCLUDE` (exit 3) and nothing is written.
- **Options of the other direction** are accepted and have no effect. With `--from adf`: `-t`,
  `--pretty`, `--compact` and the Markdown options (`--softbreak`, `--html`, `--images`,
  `--extensions`). With Markdown input: `--md-flavor`. With CSV input: `--md-flavor` and the
  Markdown options. Likewise `validate` ignores the Markdown options when it checks ADF or CSV.
  `--include-root` is the exception: it is refused (`E_USAGE`) wherever the input is not Markdown.
- **CSV flags** are only valid where they mean something; anywhere else they are a usage error
  (`E_USAGE`, exit 2) whose message names the flag:

  | Flag | Valid with | Value |
  |---|---|---|
  | `--delimiter DELIM` | `--from csv`, `--to csv` | `comma` (default), `tab`, `semicolon`, `pipe`, or one printable ASCII character other than `"` (a literal tab, or the two characters `\t`, is fine; other control characters are refused) |
  | `--no-header`, `--align-numbers` | `--from csv` | |
  | `--max-rows N` | `--from csv` | an integer ≥ 1 |
  | `--table N` | `--to csv` | an integer ≥ 1 (default 1) |
  | `--csv-safe` | `--to csv` | |
  | `--to csv` | `--from adf` | |

  `-e/--envelope` is not allowed with `--to csv` (exit 2): the output is plain CSV, not JSON.

### CSV input (`--from csv`)

```sh
adf convert data.csv --from csv -e jira-comment -o body.json
adf convert data.tsv --from csv --delimiter tab --max-rows 50 --align-numbers -o table.json
```

Converts the CSV (or TSV) text into an ADF document that holds exactly one table. The rules (line
breaks, quotes, header, links, numeric columns, truncation) are in
[syntax.md](syntax.md#csv-to-a-table-reading-rules).

- The pipeline is the one of Markdown input: read the input (UTF-8 check, size limit), convert,
  check the result against the schema and the target rules (an invalid result is `E_INTERNAL`),
  wrap it in the envelope (`-e` is allowed), write it. `-t` selects the validation target, as for
  Markdown.
- Warnings go to stderr: `W_CSV_TRUNCATED` (`--max-rows`) and `W_CONTROL_CHAR`. With `--strict` any warning fails the conversion
  with `E_LOSSY` (exit 5), and nothing is written.
- Errors: `E_INPUT_ENCODING` (not UTF-8), `E_INPUT_TOO_LARGE` (more than 1 MiB of text, or a table
  of more than 100,000 cells), `E_INPUT_CSV` (a quote that is never closed, or no rows), all
  exit 3. See [Limits](#limits).
- `--md-flavor`, `--images`, `--html`, `--softbreak` and `--extensions` are accepted and have no
  effect: cell text is never read as Markdown.

### CSV output (`--from adf --to csv`)

```sh
adf convert issue.json --from adf --to csv -o table.csv
adf convert page.json --from adf --to csv --table 2 --delimiter semicolon --csv-safe
```

Writes one table of the ADF document as CSV text, to stdout or to `-o PATH` (atomically, like any
other output). The rules (which table, merged cells, cell text, quoting, `--csv-safe`) are in
[syntax.md](syntax.md#a-table-to-csv-writing-rules).

- **Input.** A bare document or any [envelope](#envelopes), detected exactly as for `--from adf`
  (`--field NAME` selects the field of a Jira issue response). The output is plain CSV text,
  never wrapped in an envelope. Warning pointers are prefixed with
  the envelope's pointer, as for ADF → Markdown.
- **Which table.** All `table` nodes in document order, nested ones included; `--table N` picks
  the Nth (default 1). A document without tables, or `N` larger than the number of tables, fails
  with `E_INPUT_NO_TABLE` (exit 3); the message says how many tables the document has.
- **Output.** UTF-8 without a BOM, records separated by LF, the text always ends with LF. Stdout
  carries only the CSV.
- **Warnings.** `W_CSV_MERGED_CELLS` (one per table, pointing at the table) and `W_CONTROL_CHAR`.
  With `--strict`, a warning fails the conversion with `E_LOSSY` (exit 5) and nothing is written.
- Errors: `E_INPUT_JSON`, `E_INPUT_NOT_ADF`, `E_INPUT_TOO_DEEP` as for `--from adf`;
  `E_INPUT_TOO_LARGE` when the table would have more than 100,000 cells or the CSV more than
  64 MiB.

### `validate`

- **`--from auto`** (default): ADF if the first character that is not whitespace is `{`,
  Markdown otherwise. Markdown that starts with `{` (Jira wiki `{code}`, a `{{template}}`) is
  therefore read as JSON and fails with `E_INPUT_JSON`; pass `--from md` for it.
- **`--from adf`**: checks the document against the vendored ADF schema, the target rules and the
  invariants. The input may be a bare document or any [envelope](#envelopes); `-e` is not
  allowed (exit 2), because the envelope is detected. Problems are reported with a JSON Pointer,
  at most 100 of each code (see [Diagnostics](#diagnostics)). Exit code 4 if anything fails.
- **`--from md`**: a dry run of `convert`. It reports the warnings the conversion would produce,
  at most 100 of each code. Exit code 0, or 5 with `--strict` if there are warnings.
  `--include-root DIR` works as for `convert` and really reads the included files (it is a dry
  run of the conversion, not of the file access); with `--from adf`, `--from csv`, or `--from auto`
  when the input turns out to be ADF, it is a usage error (exit 2).
- **`--from csv`**: a dry run of `convert --from csv`, with the same CSV flags (`--delimiter`,
  `--no-header`, `--max-rows`, `--align-numbers`) and the same exit codes as `--from md`.
  `--from auto` never chooses CSV: pass `--from csv`.
- **`--json`**: writes a report to stdout:

  ```json
  {"schema_version":1,"valid":false,"from":"adf","target":"jira","envelope":"jira-comment",
   "adf_schema":"@atlaskit/adf-schema@57.7.18","errors":[<diagnostic>…],"warnings":[<diagnostic>…],
   "includes":[{"path":"data/q1.csv","bytes":812}]}
  ```

  `includes` lists the files that `adf:csv` includes read, in document order: the PATH as written
  in the Markdown (before percent-decoding, without the query) and the bytes read. It is always
  present and empty when nothing was read (no `--include-root`, no include, ADF or CSV input);
  after an error it lists the files read before it. The text output is unchanged.

  `envelope` is the detected (adf) or requested (md, csv) envelope, or `null` when the input could
  not be read or holds no ADF document. When the input cannot be read, `from` is `adf` unless
  `--from md` or `--from csv` was given (`from` is then `md` or `csv`).
- Without `--json`, diagnostics go to stderr. With the text format (`--error-format text`, or
  `auto` on a terminal), a one-line summary follows them, for example
  `comment.json: valid ADF for jira, envelope doc: 0 errors, 0 warnings`. Its error and warning
  counts include the diagnostics that were not listed. The JSON format has no summary line, so a
  valid document without warnings prints nothing.

### `capabilities --json`

Describes what this build supports, so agents can discover features instead of guessing. An
abridged example (`…` marks left-out entries and fields; `schemas/capabilities.v1.json` has the
full shape):

```json
{"schema_version":1,"version":"0.1.0","adf_schema":"@atlaskit/adf-schema@57.7.18",
 "directions":["md->adf","adf->md","csv->adf","adf->csv"],
 "envelopes":[{"name":"jira-comment","shape":"{\"body\":DOC}","endpoint":"POST /rest/api/3/issue/{key}/comment, …","default_target":"jira","requires_field":false}…],
 "targets":["jira","confluence"],
 "features":[{"name":"tables","kind":"gfm","syntax":"| a | b |","adf":["table","tableRow","tableHeader","tableCell","paragraph","alignment"]}…],
 "extensions":[{"name":"alerts","syntax":"> [!NOTE] / …","adf":["panel"],"default":true}…],
 "exit_codes":{"0":"ok (warnings may have been printed)","1":"internal error (a bug in adf)",…},
 "diagnostic_codes":[{"code":"W_IMAGE_AS_LINK","severity":"warning","exit_code":0,"summary":"…"}…],
 "limits":{"max_input_bytes":4194304,"max_markdown_bytes":1048576,"max_emphasis_delimiters":20000,"max_nesting_depth":64,"max_json_depth":512,"max_table_cells":100000,"max_output_bytes":67108864,"jira_field_char_limit":32767}}
```

`limits` holds the input size, the Markdown nesting depth and Jira's field size. The other limits
(the table-cell budget and the nesting depth of ADF input) are listed under [Limits](#limits).

Without `--json`, a readable summary is printed.

### `examples [FEATURE] [--json]`

Prints curated Markdown → ADF examples, one per feature. The Markdown of each example is built
into the binary, and the ADF is produced by converting it when the command runs, so it is what
this version outputs. With `FEATURE`, prints only that feature; the full list is long, so agents
should ask for one feature at a time.

With `--json`, each entry has `name`, `kind`, `syntax`, `adf`, `warnings` and, for an extension,
`extension`. The source of the example is `markdown` for every feature except the CSV feature
(`kind` `csv`), which has `csv` instead; read `markdown` or `csv`, whichever is present.

### `schema NAME`

Prints a JSON Schema to stdout:
- `adf`: the vendored `full.json`;
- `diagnostic`, `capabilities`, `validate-report`: the schemas of this tool's own JSON outputs.

### `explain TOPIC`

Prints documentation for a topic, or for one diagnostic code (`adf explain W_IMAGE_AS_LINK`).
Unknown topic: exit 2.

### `version [--json]`

`{"version":"0.1.0","commit":"<git sha or null>","os":"macos","arch":"aarch64","adf_schema":"@atlaskit/adf-schema@57.7.18"}`

## Envelopes

`-e/--envelope` wraps the ADF document for the API you are calling:

| Name | Output | Use with |
|---|---|---|
| `doc` (default) | `{"type":"doc","version":1,"content":[…]}` | acli `--body-adf` / `--description-file`, embedding in your own JSON |
| `jira-comment` | `{"body": DOC}` | `POST /rest/api/3/issue/{key}/comment`, `PUT …/comment/{id}` |
| `jira-description` | `{"fields": {"description": DOC}}` | `PUT /rest/api/3/issue/{key}` |
| `jira-field` | `{"fields": {"<NAME>": DOC}}` (requires `--field NAME`, e.g. `customfield_10010`) | `PUT /rest/api/3/issue/{key}` for rich-text custom fields |
| `confluence` | `{"representation":"atlas_doc_format","value":"<DOC as a JSON string>"}` | the `body` of Confluence v2 `POST /wiki/api/v2/pages`, `/footer-comments` |

**Input detection** with `--from adf`, in order:
1. `{"type":"doc"}`;
2. `{"body": DOC}`;
3. `{"fields": {...}}`: uses `--field NAME`, default `description`;
4. `{"representation":"atlas_doc_format","value":"…"}`;
5. `{"body":{"atlas_doc_format":{"value":"…"}}}`.

Anything else fails with `E_INPUT_NOT_ADF` (exit 3).

`--field NAME` is accepted only where it selects something: with `-e jira-field` for Markdown
input, and with ADF input that has a `fields` object (step 3). Anywhere else, including Markdown
input without `-e` and ADF input detected by steps 1, 2, 4 or 5, it is a usage error (`E_USAGE`,
exit 2) instead of being ignored. An empty name is a usage error too.

## Exit codes

| Code | Meaning | Retry? |
|---|---|---|
| 0 | Success. Warnings may have been printed. | |
| 1 | Internal error (`E_INTERNAL`): a bug in `adf`. Please report it. | no |
| 2 | Usage error (`E_USAGE`). | fix the command |
| 3 | Input or I/O problem (`E_INPUT_*`, `E_RAW_ADF`, `E_OUTPUT_WRITE`). | fix the input |
| 4 | Validation failed (`E_SCHEMA`, `E_TARGET`, `E_INVARIANT`); `validate` only. | fix the document |
| 5 | Lossy conversion refused (`E_LOSSY`): `--strict`, or `--html error`. | fix the input or drop `--strict` |

If several errors occur, the exit code is that of the first error.

## Diagnostics

stdout carries **only** the result. Diagnostics always go to **stderr**.

**Format.** `--error-format` (or the `ADF_ERROR_FORMAT` environment variable) selects it:
- `auto` (default): `text` when stderr is a terminal, `json` otherwise.

**JSON**: one object per line.

```json
{"schema_version":1,"severity":"warning","code":"W_IMAGE_AS_LINK","message":"Image converted to a link; ADF images need media uploaded to Jira or Confluence","location":{"file":"comment.md","line":12,"column":3,"end_line":12,"end_column":41},"hint":"Use --images external to embed it by URL, or attach the file in Jira","docs":"https://github.com/Yornik/adf-converter/blob/main/docs/cli-contract.md#w_image_as_link"}
```

Scripts should match on `code`; `message` and `hint` are written for people.

- Errors have `"severity":"error"` and an extra `"exit_code"`.
- `location` fields are optional:
  - `file`: `"-"` for stdin;
  - `line`, `column`: 1-based, columns count characters;
  - `end_line`, `end_column`;
  - `byte_offset`: invalid UTF-8;
  - `pointer`: a JSON Pointer (RFC 6901) into the input: the document's position in its
    [envelope](#envelopes), then the path inside the document (`/body/content/0`). A Confluence
    body holds the document as a JSON *string* (`/value`, `/body/atlas_doc_format/value`); the
    segments after it address the JSON parsed from that string
    (`/value/content/0/content/0/text`), so a generic resolver must parse the string first.
- `hint` is optional.

**Text**: `comment.md:12:3: warning[W_IMAGE_AS_LINK]: message (hint: …)`. For ADF locations:
`doc.json#/content/0: error[E_SCHEMA]: …`.

**At most 100 per code.** Every command lists at most 100 diagnostics of each code, in document
order, so a document with thousands of problems cannot flood stderr (or an agent's context). The
rest of a code are replaced by one diagnostic of the same code and severity, without a
`pointer`, `line` or `column`, right after the last one listed:
`"message":"150 more E_SCHEMA errors were not listed"`. In a `validate --json` report it is in
`errors` or `warnings` like any other diagnostic. The exit code is unchanged: the first error is
always listed.

**Other switches:**
- `-q/--quiet` suppresses warnings. Errors are always printed, and exit codes are unchanged.
- `NO_COLOR` and `--no-color` disable colour. Colour is only used on a terminal.

## Diagnostic codes

The authoritative list is `src/diagnostics.rs`. It is also printed by `adf explain codes` and
included in `adf capabilities --json`.

### Errors

| Code | Exit | Meaning |
|---|---|---|
| <a id="e_internal"></a>`E_INTERNAL` | 1 | Internal error (a bug in adf); please report it with the input that caused it. |
| <a id="e_usage"></a>`E_USAGE` | 2 | Invalid command-line usage. |
| <a id="e_input_read"></a>`E_INPUT_READ` | 3 | The input file or stdin could not be read. |
| <a id="e_input_encoding"></a>`E_INPUT_ENCODING` | 3 | The input is not valid UTF-8 (location has `byte_offset`). |
| <a id="e_input_too_large"></a>`E_INPUT_TOO_LARGE` | 3 | The input is larger than the size limit (1 MiB of Markdown, 4 MiB of ADF), or its Markdown tables would expand to more than 100,000 cells, or a CSV file (1 MiB of text) would make a table of more than 100,000 cells, or (ADF → Markdown) the Markdown would be larger than 64 MiB or its plain-flavour tables would need more than 100,000 empty cells, or (ADF → CSV) the table would have more than 100,000 cells or the CSV would be larger than 64 MiB (see [Limits](#limits)). |
| <a id="e_input_too_deep"></a>`E_INPUT_TOO_DEEP` | 3 | The input nests deeper than the nesting limit (64 container levels; 512 JSON levels for ADF input). |
| <a id="e_input_json"></a>`E_INPUT_JSON` | 3 | The input is not valid JSON. |
| <a id="e_input_not_adf"></a>`E_INPUT_NOT_ADF` | 3 | The JSON input is not an ADF document or a known envelope around one. |
| <a id="e_raw_adf"></a>`E_RAW_ADF` | 3 | An ```` ```adf ```` block is not valid JSON, or not valid ADF where it is placed. |
| <a id="e_output_write"></a>`E_OUTPUT_WRITE` | 3 | The output could not be written. |
| <a id="e_schema"></a>`E_SCHEMA` | 4 | The ADF document does not match the ADF JSON schema. |
| <a id="e_target"></a>`E_TARGET` | 4 | The ADF document uses a node, mark or attribute the target product does not accept. |
| <a id="e_invariant"></a>`E_INVARIANT` | 4 | The ADF document breaks a rule Jira/Confluence enforce beyond the JSON schema. |
| <a id="e_lossy"></a>`E_LOSSY` | 5 | The conversion would lose information and `--strict` (or `--html error`) is set. |
| <a id="e_input_csv"></a>`E_INPUT_CSV` | 3 | The CSV input is malformed or has no rows: a quoted field is never closed (located at its opening quote, by line and column), or every line is blank. |
| <a id="e_csv_option"></a>`E_CSV_OPTION` | 3 | A ```` ```csv ```` fence or an `adf:csv` include has an unknown, repeated or invalid option (`delimiter`, `header`, `max-rows`, `align`); located at the fence's info string or at the image. |
| <a id="e_include"></a>`E_INCLUDE` | 3 | An `adf:csv` include was refused or failed: an empty, absolute or otherwise invalid PATH (backslash, `:`, `..`, `.`, empty component, control character, bad percent-encoding), a file that resolves outside `--include-root` (a symbolic link out of it), a target that is not a regular file (directory, device, FIFO, socket), a file that is missing or unreadable, an unusable root, or a budget used up (more than 16 includes, or more than 4 MiB read, per document). Located at the image; the message names the rule and the PATH as written, never a resolved path. A file over 1 MiB is `E_INPUT_TOO_LARGE`, one that is not UTF-8 is `E_INPUT_ENCODING`, a CSV problem inside the file is `E_INPUT_CSV`, each naming the PATH. |
| <a id="e_input_no_table"></a>`E_INPUT_NO_TABLE` | 3 | `--to csv`: the ADF document has no table, or fewer tables than the number given with `--table`; the message says how many it has. |

### Warnings: Markdown → ADF

| Code | Meaning |
|---|---|
| <a id="w_mark_dropped"></a>`W_MARK_DROPPED` | A text mark was dropped because ADF does not allow it there (for example bold on inline code, or a subscript around a superscript), or an image title, or a link around an image, was dropped (`--images external`, or an `adf:media` image). |
| <a id="w_heading_demoted"></a>`W_HEADING_DEMOTED` | A heading inside a list item or quote became a bold paragraph. |
| <a id="w_block_hoisted"></a>`W_BLOCK_HOISTED` | A block was moved out of a container that cannot hold it (for example a table inside a list). |
| <a id="w_block_flattened"></a>`W_BLOCK_FLATTENED` | A nested container was merged into its parent (for example a quote inside a quote). |
| <a id="w_html_as_text"></a>`W_HTML_AS_TEXT` | Raw HTML without an ADF equivalent was kept as literal text. |
| <a id="w_html_dropped"></a>`W_HTML_DROPPED` | Raw HTML was dropped (`--html drop`), or an unrecognised tag inside `<summary>` was removed (an expand title is plain text). |
| <a id="w_image_as_link"></a>`W_IMAGE_AS_LINK` | An image became a link; ADF images need media uploaded to Jira/Confluence. |
| <a id="w_image_dropped"></a>`W_IMAGE_DROPPED` | An image was dropped (`--images drop`), including an `adf:media` file image or an `adf:csv` include. |
| <a id="w_unsafe_url"></a>`W_UNSAFE_URL` | A link target is not an allowed URL and became plain text. |
| <a id="w_control_char"></a>`W_CONTROL_CHAR` | Control characters were removed from text. ADF → Markdown also reports it when carriage returns (CR LF or a lone CR) in text or code are written as line breaks. |
| <a id="w_table_cell_dropped"></a>`W_TABLE_CELL_DROPPED` | Table cells beyond the header row's width were dropped. |
| <a id="w_footnote_flattened"></a>`W_FOOTNOTE_FLATTENED` | Footnotes became superscript numbers plus a numbered list at the end. |
| <a id="w_task_numbering_lost"></a>`W_TASK_NUMBERING_LOST` | An ordered task list lost its numbering; ADF task lists are unordered. |
| <a id="w_front_matter_dropped"></a>`W_FRONT_MATTER_DROPPED` | Leading YAML front matter was removed. |
| <a id="w_unknown_extension"></a>`W_UNKNOWN_EXTENSION` | An `adf:` link names an unknown extension and was kept as plain text. |
| <a id="w_invalid_extension_arg"></a>`W_INVALID_EXTENSION_ARG` | An `adf:` extension has an invalid argument and was kept as plain text. For an `adf:media` image (extension `media`): a bad id, collection, `width`, `height`, `layout` or `size`, an unknown or duplicate key, or an empty value; the alt text is kept. |
| <a id="w_extension_disabled"></a>`W_EXTENSION_DISABLED` | Extension syntax was found but the extension is disabled, so it was converted as plain Markdown. |
| <a id="w_include_disabled"></a>`W_INCLUDE_DISABLED` | An `![alt](adf:csv/PATH)` include was found but no include root is set (`--include-root`), so nothing was read and the image became its alt text (the PATH when the alt text is empty). Located at the image. |
| <a id="w_node_not_in_target"></a>`W_NODE_NOT_IN_TARGET` | Reserved: a construct is not supported by the target product and was replaced by a fallback. This version never emits it: Markdown cannot produce a construct a target rejects, and raw ADF that the target rejects fails with `E_RAW_ADF` instead. |

### Warnings: document checks

`convert` (Markdown or CSV → ADF) checks the document it writes, and `validate` checks every document,
from Markdown or ADF input alike:

| Code | Meaning |
|---|---|
| <a id="w_size_limit"></a>`W_SIZE_LIMIT` | The output is larger than Jira's 32,767-character field limit and may be rejected. |
| <a id="w_empty_document"></a>`W_EMPTY_DOCUMENT` | The document is empty; Jira rejects empty comment bodies. |

### Warnings: ADF → Markdown

| Code | Meaning |
|---|---|
| <a id="w_adf_passthrough"></a>`W_ADF_PASSTHROUGH` | An ADF node without a Markdown equivalent was written as a raw ```` ```adf ```` block. A file `mediaSingle` that [syntax.md](syntax.md#reading-file-media-back) can express is written as an `adf:media` image instead. |
| <a id="w_attr_dropped"></a>`W_ATTR_DROPPED` | ADF attributes or marks without a Markdown equivalent were dropped (for example column widths, cell colours, paragraph alignment, or Confluence's inline-comment `annotation` marks; the full rule is in [syntax.md](syntax.md#mapping-roundtrip)). |
| <a id="w_input_schema"></a>`W_INPUT_SCHEMA` | The input ADF does not match the schema; it was converted anyway. |

### Warnings: CSV

| Code | Meaning |
|---|---|
| <a id="w_csv_truncated"></a>`W_CSV_TRUNCATED` | `--from csv`: data rows beyond `--max-rows` were left out of the table. Located at the first row that was left out; the message says `Kept N of M data rows`. |
| <a id="w_csv_merged_cells"></a>`W_CSV_MERGED_CELLS` | `--to csv`: a table with merged cells (`colspan` or `rowspan`) was written by repeating the text of a merged cell in every cell it covers. One warning per table, pointing at the table. |

CSV conversions also report `W_CONTROL_CHAR` (control characters other than tab and line feed were
removed from a cell; once per cell, located at the cell: its line when reading CSV) and, like
Markdown input, the document checks below (`W_SIZE_LIMIT` for a table whose JSON is larger than
Jira's field limit).

ADF → Markdown lists at most 20 `W_INPUT_SCHEMA` warnings and at most 100 of each other code. One
more warning of the same code, without a location, counts the rest.

## Environment variables

| Variable | Effect |
|---|---|
| `ADF_ERROR_FORMAT` | Default for `--error-format` (`auto`, `text`, `json`; ASCII case and surrounding spaces are ignored). Unset or empty means `auto`. `--error-format` overrides it. Any other value is a usage error (`E_USAGE`, exit 2) that names the variable, unless `--error-format` is given. |
| `NO_COLOR` | Disables colour. |

No other environment variables are read. `adf` never reads config files and never makes network
requests. The only files it reads are the input, and, only with `--include-root DIR`, the CSV
files that `![alt](adf:csv/PATH)` images name under `DIR` (see
[Including a CSV file](syntax.md#including-a-csv-file)).

## Limits

| Limit | Value | Error |
|---|---|---|
| Input size | Markdown and CSV (text): 1 MiB; ADF (JSON): 4 MiB | `E_INPUT_TOO_LARGE` |
| `*` and `_` characters in one paragraph of Markdown (a block between blank lines, code included) | 20,000 | `E_INPUT_TOO_LARGE` |
| Table cells in Markdown input: all tables of the document together, counting the empty cells that pad short rows to the header width (see below) | 100,000 | `E_INPUT_TOO_LARGE` |
| Table cells in CSV input: the columns of the widest row × (the data rows + the header row), checked while the file is read, located at the record that crosses the limit | 100,000 | `E_INPUT_TOO_LARGE` |
| Table cells in ADF → CSV: the grid of the selected table after merged cells are expanded | 100,000 | `E_INPUT_TOO_LARGE` |
| Nesting depth | Markdown input: 64 container levels (lists, quotes, cells, expands); ADF input: 512 levels of JSON arrays/objects (8 × the container limit, so everything `adf` writes can be read back) | `E_INPUT_TOO_DEEP` |
| Output size warning | 32,767 characters of compact JSON | `W_SIZE_LIMIT` |
| Output size: the Markdown written by ADF → Markdown, the CSV written by ADF → CSV, and the estimated compact JSON written by Markdown → ADF and CSV → ADF | 64 MiB | `E_INPUT_TOO_LARGE` |
| Empty cells that `--md-flavor plain` adds to keep table columns aligned (merged cells): all tables of the document together | 100,000 | `E_INPUT_TOO_LARGE` |
| `adf:csv` includes (only with `--include-root`): files per document / bytes read per document / bytes per file | 16 / 4 MiB / 1 MiB (the Markdown limit); the table cells and the output are counted with the document's other tables | `E_INCLUDE` (files, total bytes), `E_INPUT_TOO_LARGE` (one file, cells, output) |
| Diagnostics listed per code | 100; the rest are counted in one summary diagnostic of that code ([Diagnostics](#diagnostics)) | |

A CSV file is bounded the same way, but exactly: `adf` counts `width × (rows + 1)` while it reads
the records, where the width is the largest field count seen so far, so the memory it uses never
exceeds the limit. `E_INPUT_TOO_LARGE` is located at the record that crosses it.

The CSV limits are on the CSV, not on the ADF it becomes: a table of 100,000 cells is about 12 MB
of JSON, more than Jira accepts (`W_SIZE_LIMIT`) and more than the 4 MiB ADF input limit, so
`adf` cannot read it back. About 33,000 cells is the practical bound for a CSV → ADF → CSV round
trip. A CSV above 1 MiB is refused before `--max-rows` applies, and input above 4 MiB is refused
by the general read limit first; cut such a file (for example with `head -n 100`) before
converting. Peak memory is a multiple of the input: a 1 MiB cell of line breaks needs about
500 MB.

GFM pads every table row to the header width, so a few kilobytes of Markdown can describe
millions of cells. `adf` estimates the padded size with a linear scan before parsing, and fails
with `E_INPUT_TOO_LARGE`, located at the header row of the table that crosses the limit, when it
exceeds 100,000 cells (it counts the cells it actually builds as well). The estimate errs on the
high side: a line that directly follows a table without a blank line counts as a row. Table-like
lines in fenced code are not counted, except towards a safety bound of 1,000,000 cells over the
whole input.

pulldown-cmark 0.13.4, the Markdown parser, resolves some emphasis patterns in quadratic time
(`*a_` repeated; [pulldown-cmark#1154](https://github.com/pulldown-cmark/pulldown-cmark/issues/1154),
fixed upstream but not released). Until a fixed version is released, `adf` refuses paragraphs with
more than 20,000 `*` and `_` characters before parsing, located at the start of the paragraph.

In Markdown → ADF, every text node of a link repeats the link's URL and title, so a long URL over
link text with many differently formatted runs multiplies. `adf` estimates the size of the JSON as
it builds each text node and fails with `E_INPUT_TOO_LARGE`, located at the text, once the estimate
passes 64 MiB.

In ADF → Markdown, indentation makes the output grow with nesting depth: every line of a list nested
100 levels deep carries hundreds of spaces. `adf` fails with `E_INPUT_TOO_LARGE`, located at the node
it was writing, before the Markdown grows beyond 64 MiB; Markdown → ADF could not read it back anyway.
Raw ```` ```adf ```` blocks are pretty-printed only for their first 16 JSON levels and compact below,
so they stay close to the size of the input.

## Determinism

The same input, flags and version always produce byte-identical output:
- task-list `localId`s are derived from the input;
- no timestamps, randomness or hash-map ordering reach the output;
- CRLF and LF input produce identical output, and a UTF-8 BOM is ignored.
