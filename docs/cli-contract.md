# CLI contract

This is the stable interface of `adf` for scripts and AI agents. Everything here follows semantic
versioning:
- **additions** (new flags, codes, JSON fields) are minor changes;
- **renames, removals and behaviour changes** are breaking.

Machine-readable JSON documents carry `"schema_version": 1`. Their JSON Schemas ship in
`schemas/*.v1.json` and are printed by `adf schema …`.

## Commands

```
adf convert [INPUT|-] [-o PATH] [--from md|adf] [-e ENVELOPE] [--field NAME]
            [-t auto|jira|confluence] [--pretty|--compact] [--strict]
            [--softbreak space|hardbreak] [--html text|drop|error]
            [--images link|external|drop] [--extensions all|none|LIST]
            [--md-flavor roundtrip|plain] [--error-format auto|text|json] [-q] [--no-color]
adf validate [INPUT|-] [--from md|adf] [-e ENVELOPE] [--field NAME] [-t …] [--strict] [--json]
adf capabilities [--json]
adf examples [FEATURE] [--json]
adf schema adf|diagnostic|capabilities|validate-report
adf explain exit-codes|envelopes|syntax|codes|<CODE>
adf completions bash|zsh|fish|powershell|elvish
adf version [--json]            (also: adf --version)
```

There is no default subcommand: `adf FILE` is a usage error (`E_USAGE`, exit 2).

### `convert`

- **Input.** `INPUT` is a path, or `-` / omitted for stdin. If stdin is a terminal and no input
  is given, a short usage message is printed to stderr and the exit code is 2.
- **`--from md`** (default): Markdown → ADF.
- **`--from adf`**: ADF → Markdown. The input may be a bare document or any
  [envelope](#envelopes). `-e` is not allowed with `--from adf` (exit 2).
- **`-o PATH`**: write to a file, atomically (temporary file in the same directory, then rename).
  Nothing is written if the conversion fails.
- **Output format.** Pretty JSON when stdout is a terminal, compact otherwise. Files are compact
  unless `--pretty` is given. `--pretty` and `--compact` override. Markdown output always ends with
  exactly one newline.
- **`--strict`**: any warning fails the conversion with `E_LOSSY` (exit 5), and nothing is
  written.
- **`-t/--target`**: which product's rules to validate against. `auto` (default) is `confluence`
  for `-e confluence` and `jira` otherwise.
- **Self-check.** Output is checked before it is written (schema, target rules, invariants). If
  `adf`'s own output fails that check, it reports `E_INTERNAL` (exit 1), because that is a bug.

### `validate`

- **`--from adf`** (default for `validate` when the input parses as JSON): checks the document
  against the vendored ADF schema, the target rules and the invariants. Every problem is reported
  with a JSON Pointer. Exit code 4 if anything fails.
- **`--from md`**: a dry run of `convert`. It reports every warning the conversion would produce.
  Exit code 0, or 5 with `--strict` if there are warnings.
- **`--json`**: writes a report to stdout:

  ```json
  {"schema_version":1,"valid":false,"from":"adf","target":"jira","envelope":"jira-comment",
   "adf_schema":"@atlaskit/adf-schema@57.7.18","errors":[<diagnostic>…],"warnings":[<diagnostic>…]}
  ```

  `envelope` is the detected (adf) or requested (md) envelope.
- Without `--json`, diagnostics go to stderr, followed by a one-line summary.

### `capabilities --json`

Describes what this build supports, so agents can discover features instead of guessing:

```json
{"schema_version":1,"version":"0.1.0","adf_schema":"@atlaskit/adf-schema@57.7.18",
 "directions":["md->adf","adf->md"],
 "envelopes":[{"name":"jira-comment","shape":"{\"body\":DOC}","endpoint":"POST /rest/api/3/issue/{key}/comment"}…],
 "targets":["jira","confluence"],
 "features":[{"name":"tables","kind":"gfm","syntax":"| a | b |","adf":["table","tableRow","tableHeader","tableCell"]}…],
 "extensions":[{"name":"alerts","syntax":"> [!NOTE]","adf":["panel"],"default":true}…],
 "exit_codes":{"0":"ok","1":"internal error",…},
 "diagnostic_codes":[{"code":"W_IMAGE_AS_LINK","severity":"warning","summary":"…"}…],
 "limits":{"max_input_bytes":4194304,"max_nesting_depth":64,"jira_field_char_limit":32767}}
```

Without `--json`, a readable summary is printed.

### `examples [FEATURE] [--json]`

Prints curated Markdown → ADF examples, one per feature, taken from the test fixtures. With
`FEATURE`, prints only that feature.

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
{"schema_version":1,"severity":"warning","code":"W_IMAGE_AS_LINK","message":"Image converted to a link; ADF images need uploaded media","location":{"file":"comment.md","line":12,"column":3,"end_line":12,"end_column":41},"hint":"Use --images external to embed by URL, or attach the file in Jira","docs":"https://github.com/Yornik/adf-converter/blob/main/docs/cli-contract.md#w_image_as_link"}
```

- Errors have `"severity":"error"` and an extra `"exit_code"`.
- `location` fields are optional:
  - `file`: `"-"` for stdin;
  - `line`, `column`: 1-based, columns count characters;
  - `end_line`, `end_column`;
  - `byte_offset`: invalid UTF-8;
  - `pointer`: a JSON Pointer into the ADF document.
- `hint` is optional.

**Text**: `comment.md:12:3: warning[W_IMAGE_AS_LINK]: message (hint: …)`. For ADF locations:
`doc.json#/content/0: error[E_SCHEMA]: …`.

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
| <a id="e_input_too_large"></a>`E_INPUT_TOO_LARGE` | 3 | The input is larger than the size limit (4 MiB), or its Markdown tables would expand to more than 100,000 cells (see [Limits](#limits)). |
| <a id="e_input_too_deep"></a>`E_INPUT_TOO_DEEP` | 3 | The input nests deeper than the nesting limit (64 container levels; 512 JSON levels for ADF input). |
| <a id="e_input_json"></a>`E_INPUT_JSON` | 3 | The input is not valid JSON. |
| <a id="e_input_not_adf"></a>`E_INPUT_NOT_ADF` | 3 | The JSON input is not an ADF document or a known envelope around one. |
| <a id="e_raw_adf"></a>`E_RAW_ADF` | 3 | An ```` ```adf ```` block is not valid JSON, or not valid ADF where it is placed. |
| <a id="e_output_write"></a>`E_OUTPUT_WRITE` | 3 | The output could not be written. |
| <a id="e_schema"></a>`E_SCHEMA` | 4 | The ADF document does not match the ADF JSON schema. |
| <a id="e_target"></a>`E_TARGET` | 4 | The ADF document uses a node, mark or attribute the target product does not accept. |
| <a id="e_invariant"></a>`E_INVARIANT` | 4 | The ADF document breaks a rule Jira/Confluence enforce beyond the JSON schema. |
| <a id="e_lossy"></a>`E_LOSSY` | 5 | The conversion would lose information and `--strict` (or `--html error`) is set. |

### Warnings: Markdown → ADF

| Code | Meaning |
|---|---|
| <a id="w_mark_dropped"></a>`W_MARK_DROPPED` | A text mark was dropped because ADF does not allow it there (for example bold on inline code). |
| <a id="w_heading_demoted"></a>`W_HEADING_DEMOTED` | A heading inside a list item or quote became a bold paragraph. |
| <a id="w_block_hoisted"></a>`W_BLOCK_HOISTED` | A block was moved out of a container that cannot hold it (for example a table inside a list). |
| <a id="w_block_flattened"></a>`W_BLOCK_FLATTENED` | A nested container was merged into its parent (for example a quote inside a quote). |
| <a id="w_html_as_text"></a>`W_HTML_AS_TEXT` | Raw HTML without an ADF equivalent was kept as literal text. |
| <a id="w_html_dropped"></a>`W_HTML_DROPPED` | Raw HTML was dropped (`--html drop`). |
| <a id="w_image_as_link"></a>`W_IMAGE_AS_LINK` | An image became a link; ADF images need media uploaded to Jira/Confluence. |
| <a id="w_image_dropped"></a>`W_IMAGE_DROPPED` | An image was dropped (`--images drop`). |
| <a id="w_unsafe_url"></a>`W_UNSAFE_URL` | A link target is not an allowed URL and became plain text. |
| <a id="w_control_char"></a>`W_CONTROL_CHAR` | Control characters were removed from text. |
| <a id="w_table_cell_dropped"></a>`W_TABLE_CELL_DROPPED` | Table cells beyond the header row's width were dropped. |
| <a id="w_footnote_flattened"></a>`W_FOOTNOTE_FLATTENED` | Footnotes became superscript numbers plus a numbered list at the end. |
| <a id="w_task_numbering_lost"></a>`W_TASK_NUMBERING_LOST` | An ordered task list lost its numbering; ADF task lists are unordered. |
| <a id="w_front_matter_dropped"></a>`W_FRONT_MATTER_DROPPED` | Leading YAML front matter was removed. |
| <a id="w_unknown_extension"></a>`W_UNKNOWN_EXTENSION` | An `adf:` link names an unknown extension and was kept as plain text. |
| <a id="w_invalid_extension_arg"></a>`W_INVALID_EXTENSION_ARG` | An `adf:` extension has an invalid argument and was kept as plain text. |
| <a id="w_extension_disabled"></a>`W_EXTENSION_DISABLED` | Extension syntax was found but the extension is disabled, so it was converted as plain Markdown. |
| <a id="w_node_not_in_target"></a>`W_NODE_NOT_IN_TARGET` | A construct is not supported by the target product and was replaced by a fallback. |
| <a id="w_size_limit"></a>`W_SIZE_LIMIT` | The output is larger than Jira's 32,767-character field limit and may be rejected. |
| <a id="w_empty_document"></a>`W_EMPTY_DOCUMENT` | The document is empty; Jira rejects empty comment bodies. |

### Warnings: ADF → Markdown

| Code | Meaning |
|---|---|
| <a id="w_adf_passthrough"></a>`W_ADF_PASSTHROUGH` | An ADF node without a Markdown equivalent was written as a raw ```` ```adf ```` block. |
| <a id="w_attr_dropped"></a>`W_ATTR_DROPPED` | ADF attributes without a Markdown equivalent were dropped (for example column widths or cell colours). |
| <a id="w_input_schema"></a>`W_INPUT_SCHEMA` | The input ADF does not match the schema; it was converted anyway. |

## Environment variables

| Variable | Effect |
|---|---|
| `ADF_ERROR_FORMAT` | Default for `--error-format` (`auto`, `text`, `json`). |
| `NO_COLOR` | Disables colour. |

No other environment variables are read. `adf` never reads config files and never makes network
requests.

## Limits

| Limit | Value | Error |
|---|---|---|
| Input size | 4 MiB | `E_INPUT_TOO_LARGE` |
| Table cells in Markdown input: all tables of the document together, counting the empty cells that pad short rows to the header width (see below) | 100,000 | `E_INPUT_TOO_LARGE` |
| Nesting depth | Markdown input: 64 container levels (lists, quotes, cells, expands); ADF input: 512 levels of JSON arrays/objects (8 × the container limit, so everything `adf` writes can be read back) | `E_INPUT_TOO_DEEP` |
| Output size warning | 32,767 characters of compact JSON | `W_SIZE_LIMIT` |

GFM pads every table row to the header width, so a few kilobytes of Markdown can describe
millions of cells. `adf` estimates the padded size with a linear scan before parsing, and fails
with `E_INPUT_TOO_LARGE`, located at the header row of the table that crosses the limit, when it
exceeds 100,000 cells (it counts the cells it actually builds as well). The estimate errs on the
high side: a line that directly follows a table without a blank line counts as a row. Table-like
lines in fenced code are not counted, except towards a safety bound of 1,000,000 cells over the
whole input.

## Determinism

The same input, flags and version always produce byte-identical output:
- task-list `localId`s are derived from the input;
- no timestamps, randomness or hash-map ordering reach the output;
- CRLF and LF input produce identical output, and a UTF-8 BOM is ignored.
