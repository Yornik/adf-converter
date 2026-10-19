# Troubleshooting: diagnostic codes and fixes

Every diagnostic has a stable `code`. Errors (`E_…`) stop the command with the exit code shown;
warnings (`W_…`) describe something that was repaired or lost, and the command still succeeds
(exit 0), unless `--strict` is set (exit 5). `adf explain <CODE>` prints the same information for
the installed version, and `adf explain codes` lists all codes.

Locations: Markdown input has `line` and `column` (1-based); ADF input has a JSON `pointer` such
as `/body/content/2`; invalid UTF-8 has a `byte_offset`.

## Exit codes

| Exit | Meaning | Retry? | What to do |
|---|---|---|---|
| 0 | Success; warnings may have been printed | | Read the warnings |
| 1 | Internal error (`E_INTERNAL`) | no | A bug in `adf`; see below |
| 2 | Usage error (`E_USAGE`) | after fixing the command | Check `adf <command> --help` |
| 3 | Input or I/O problem | no | Fix the input file, path or permissions |
| 4 | Validation failed (`validate` only) | no | Regenerate the ADF from Markdown |
| 5 | Lossy conversion refused (`E_LOSSY`) | no | Fix the Markdown, or drop `--strict` if the user accepts the loss |

## Errors

| Code | Exit | Meaning | Fix |
|---|---|---|---|
| `E_INTERNAL` | 1 | A bug in `adf`. Its own output failed its self-check, or it crashed. | Don't retry. Simplify the construct near the reported location, and tell the user; a minimal invented input reproducing it belongs in a bug report. |
| `E_USAGE` | 2 | Invalid command line. | Common causes: no subcommand (`adf file.md` instead of `adf convert file.md`), `-e` together with `--from adf`, `-e jira-field` without `--field`, an unknown `--extensions` name. |
| `E_INPUT_READ` | 3 | The input file or stdin could not be read. | Check the path and permissions. Pass `-` to read stdin. |
| `E_INPUT_ENCODING` | 3 | The input is not valid UTF-8; `byte_offset` points at the first bad byte. | Save the file as UTF-8 (for example `iconv -f WINDOWS-1252 -t UTF-8 in.md > out.md`). |
| `E_INPUT_TOO_LARGE` | 3 | The input is larger than 4 MiB, or its tables would have more than 100,000 cells in total (short rows are padded to the header width, so a wide header with many short rows counts fully). | Split the content; Jira would reject a body that large anyway. Attach big logs and data tables as files. |
| `E_INPUT_TOO_DEEP` | 3 | Lists, quotes, boxes or expands nest deeper than 64 levels (for ADF input: the JSON nests deeper than 512 levels). | Flatten the structure. |
| `E_INPUT_JSON` | 3 | `--from adf` input is not valid JSON. `adf validate` reads input that starts with `{` as JSON unless you pass `--from md`. | Make sure the file is the API's response body, not an empty file or an HTML error page; fetch with `curl --fail-with-body -o FILE`. For Markdown that starts with `{` (such as `{code}`), use `adf validate --from md`. |
| `E_INPUT_NOT_ADF` | 3 | The JSON is not an ADF document or a known envelope around one. | Fetch the field (`?fields=description`), pass `--field NAME` for custom fields, pick one item from list responses (`jq '.comments[0]'`), and request Confluence pages with `?body-format=atlas_doc_format`. A `null` field is empty: there is nothing to convert. |
| `E_RAW_ADF` | 3 | An ```` ```adf ```` block is not valid JSON, or not valid ADF where it is placed. | Fix the JSON, use a block node (not an inline node) that is allowed in that container, or replace the block with Markdown. The location points at the fence. |
| `E_OUTPUT_WRITE` | 3 | The `-o` file could not be written. | Check that the directory exists and is writable, and that there is disk space. Nothing was written. |
| `E_SCHEMA` | 4 | The ADF document does not match the ADF JSON schema, for example an empty text node, or bold on inline code; `pointer` shows where. | Don't patch the JSON. Regenerate it from Markdown with `adf convert`, or convert it to Markdown (`--from adf`) and back. |
| `E_TARGET` | 4 | The document uses a node, mark or attribute the target product does not accept (for example `fontSize`, layouts or extensions in Jira). | Remove the construct, or check that `-t`/`-e` match where you post. Regenerate from Markdown. |
| `E_INVARIANT` | 4 | The document breaks a rule Jira or Confluence enforce beyond the schema: a list item that doesn't start with a paragraph, duplicate task `localId`s, a ragged table, an empty `marks` array. | Regenerate from Markdown; `adf convert` always produces documents that satisfy these rules. |
| `E_LOSSY` | 5 | `--strict` is set (or `--html error`) and the conversion would lose something. | Read the warnings reported with it and fix the Markdown so they go away. Drop `--strict` only if the user accepts the loss. |

## Warnings: Markdown → ADF

| Code | Meaning | Fix |
|---|---|---|
| `W_MARK_DROPPED` | Formatting ADF doesn't allow there was dropped: bold, italic or strike on inline code, or any formatting around a mention, status, date, emoji or card. | Remove the formatting around the code span or extension (`` **`x`** `` → `` `x` ``), or format the surrounding words instead. |
| `W_HEADING_DEMOTED` | A heading inside a list item or quote became a bold paragraph. | Move the heading out of the list or quote, or write **bold** text on purpose. |
| `W_BLOCK_HOISTED` | A block was moved out of a container that cannot hold it, such as a table, rule or quote in a list, a rule or table in a quote, or a table in a box cell. The container is split around it. | End the list or quote before the block, then continue it after. Keep tables at the top level or in `<details>`. |
| `W_BLOCK_FLATTENED` | A nested container was merged: a quote in a quote, an alert or `<details>` where a panel or expand cannot go, or a nested `::: cell`. | Avoid nested quotes; put alerts and `<details>` at the top level, in `<details>` or in a box cell; don't nest boxes. |
| `W_HTML_AS_TEXT` | HTML without an ADF equivalent was kept as literal text. | Use Markdown or a supported tag (see [syntax.md](syntax.md#inline-html)). Escape a literal `<` as `\<`. Use `--html drop` to remove HTML instead. |
| `W_HTML_DROPPED` | HTML was removed because of `--html drop`. | Expected with that option. Rewrite as Markdown if the content matters. |
| `W_IMAGE_AS_LINK` | An image became a link, because ADF images need media uploaded to Jira or Confluence. | Usually fine for new content. `--images external` embeds by URL instead (rendering not guaranteed); otherwise attach the file in Jira. When you post back Markdown from `adf convert --from adf`, always use `--images external`: there the warning means an embedded image is about to become a link. |
| `W_IMAGE_DROPPED` | An image was removed because of `--images drop`. | Expected with that option. |
| `W_UNSAFE_URL` | A link target is not an allowed URL and became plain text. | Use an absolute `https://` URL, `mailto:`, `tel:`, `/path` or `#anchor`. Relative paths (`docs/x.md`), `javascript:` and `data:` are never linked. |
| `W_CONTROL_CHAR` | Control characters were removed from text. | Usually terminal colour codes or pasted binary. Strip them from the source (for example, rerun the command that produced the log with colours off). |
| `W_TABLE_CELL_DROPPED` | A table row had more cells than the header; the extra cells were dropped. | Add header columns or remove cells. Write a literal pipe as `\|`, also inside code spans. |
| `W_FOOTNOTE_FLATTENED` | Footnotes became superscript numbers and a numbered list at the end. | Usually fine. Or put the note in parentheses in the text. |
| `W_TASK_NUMBERING_LOST` | An ordered task list (`1. [ ]`) became an unordered task list. | Use `- [ ]`, or put the numbers in the item text. |
| `W_FRONT_MATTER_DROPPED` | Leading YAML front matter (`---` … `---`) was removed. | Expected for files from a docs site. Remove it from the input to silence the warning. |
| `W_UNKNOWN_EXTENSION` | An `adf:` link names an unknown extension; the label was kept as text. | Use `mention`, `status`, `date`, `emoji` or `card` (`adf capabilities --json` lists them); check for typos such as `adf:mentions/`. |
| `W_INVALID_EXTENSION_ARG` | An `adf:` extension has an invalid argument; the label was kept as text. | Status colours: `neutral`, `purple`, `blue`, `red`, `yellow`, `green`, and a non-empty label. Dates: label `YYYY-MM-DD`. Mentions: an account ID in the path. Cards: the label must be an allowed URL. |
| `W_EXTENSION_DISABLED` | Extension syntax was found, but `--extensions` turned that extension off, so it was converted as plain Markdown. | Leave `--extensions` at its default (`all`) or include the extension's name. |
| `W_NODE_NOT_IN_TARGET` | Reserved for a construct the target product doesn't support being replaced by a fallback; current versions never emit it. | Check that `-e`/`-t` match where you post (`-e confluence` for Confluence). Otherwise avoid the construct for that product. |
| `W_SIZE_LIMIT` | The output is larger than Jira's 32,767-character field limit and may be rejected (`CONTENT_LIMIT_EXCEEDED`). `adf validate` reports it for ADF input too. | Split it into several comments, shorten it, or attach long logs as files. |
| `W_EMPTY_DOCUMENT` | The document is empty; Jira rejects empty comment bodies. `adf validate` reports it for ADF input too. | Check that the input file isn't empty or only front matter and comments. |

## Warnings: ADF → Markdown

| Code | Meaning | Fix |
|---|---|---|
| `W_ADF_PASSTHROUGH` | A node without a Markdown equivalent was written as a ```` ```adf ```` block. | Expected in `roundtrip` mode. Keep the block unchanged when you edit and post the Markdown back. Use `--md-flavor plain` for reading. |
| `W_ATTR_DROPPED` | Attributes or marks without a Markdown form were dropped, such as column widths, cell colours, table layout, panel icons, text alignment and indentation, or Confluence `annotation` marks (the anchors of inline comments). | Fine for reading. If you post the Markdown back, those details are lost (for inline comments, their anchors): tell the user first. |
| `W_INPUT_SCHEMA` | The input ADF doesn't match the vendored schema (often newer Jira features); it was converted anyway. | Check the output for missing content. If it keeps happening with ordinary Jira content, the `adf` version may be outdated. |

## HTTP errors after posting

| Response | Likely cause | Fix |
|---|---|---|
| `400` `INVALID_INPUT`, "not valid Atlassian Document Format" | Wrong envelope for the endpoint, or ADF that didn't come from `adf convert`. | `adf validate` the file `adf convert` wrote (for Confluence `body.json`, not the `jq`-wrapped request); match the envelope to the endpoint. |
| `400` "Operation value must be an Atlassian Document" | A string (Markdown, wiki markup) was sent to an ADF field. | Send `adf` output. |
| `400` "Comment body can not be empty" | An empty body, usually from piping a failed `adf` into curl. | Convert to a file with `-o`, then post with `&&`. |
| `400` "Invalid message" (Confluence) | `value` was an object, not a string. | Use `-e confluence` and don't re-encode `value`. |
| `400` `CONTENT_LIMIT_EXCEEDED` | Body too large. | See `W_SIZE_LIMIT`. |
| `401`, `403` | Wrong email or token, a scoped token on the site URL, or missing permission. | Ask the user to check the credentials and permissions. Never print the token. |
| `404` | Wrong issue key, comment ID, page ID or site, or no permission to see it. | Check the URL and IDs. |
