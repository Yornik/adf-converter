---
name: markdown-to-adf
description: Converts Markdown to Atlassian Document Format (ADF) JSON, and ADF back to Markdown, with the offline `adf` CLI, so Jira comments, Jira issue descriptions, rich-text custom fields and Confluence pages render with real headings, lists, tables, code blocks, panels, mentions and status lozenges. Use when posting, editing or updating Jira or Confluence comments, descriptions or pages from Markdown through the REST API, curl or acli; when reading a Jira issue, comment or Confluence page as Markdown; or when the user mentions ADF, Atlassian Document Format, or formatted Jira comments.
license: MIT
compatibility: Requires the adf CLI (adf-converter 0.1.0 or later, https://github.com/Yornik/adf-converter) on PATH. adf works offline; posting needs curl (or acli), jq for some recipes, network access and Atlassian credentials in environment variables.
metadata:
  repository: https://github.com/Yornik/adf-converter
  min-cli-version: "0.1.0"
---

# Markdown ⇄ ADF for Jira and Confluence

Jira Cloud (REST API v3) and Confluence Cloud store rich text as ADF JSON. Write Markdown, let
`adf` turn it into valid ADF, and post that. `adf` never touches the network; you post its output
with `curl` or acli.

## Rules

- **Never hand-write ADF JSON**, and never patch `adf`'s output by hand. Change the Markdown and
  convert again.
- **Never send Markdown or Jira wiki markup (`h2.`, `{code}`) as plain text** to an ADF field.
- **Convert to a file, then post**: `adf convert … -o FILE && curl … --data-binary @FILE`. Never
  pipe `adf` into `curl`: if `adf` fails, curl still posts an empty body.
- **Exit codes 3, 4 and 5 mean "fix the input"**; running the same command again won't help.
  2 means fix the command line; 1 is a bug in `adf`. See [Exit codes](#exit-codes).
- **Never print, log or write credentials** (`$JIRA_API_TOKEN`) to files you create. Check that a
  variable is set with `test -n "$JIRA_API_TOKEN"`, never by echoing it.
- Post only where the user asked. Don't comment on many issues (JQL, loops) unless asked to.
- Write Markdown to a file with a quoted heredoc (`<<'EOF'`), not as a shell-quoted string.

## 1. Check the tool

```sh
adf --version
```

`adf` must be 0.1.0 or later. If it is missing, tell the user and offer to install it from the
latest release (see [references/posting.md](references/posting.md#install-adf)), or with
`cargo install --git https://github.com/Yornik/adf-converter --locked`. Don't install software
without the user's agreement.

Use `adf capabilities --json` to see what this version supports, `adf examples` for Markdown →
ADF pairs, and `adf explain <CODE>` for any diagnostic code.

## 2. Post Markdown to Jira or Confluence

**Write the content** to a file in a scratch directory:

```sh
work=$(mktemp -d)
cat > "$work/comment.md" <<'EOF'
## Deploy finished

- [x] Database migrated
- [ ] Smoke tests

> [!WARNING]
> Rollback window closes at 18:00 UTC.
EOF
```

**Pre-flight it** with the envelope you will use:

```sh
adf validate --from md -e jira-comment --json "$work/comment.md"
```

The report on stdout lists `warnings` (and `errors`), each with a `code`, a `message`, a
`location` (`line`, `column`) and often a `hint`. Fix the Markdown for every warning that loses
something the user cares about, and run it again. Some warnings are usually fine, such as
`W_IMAGE_AS_LINK`. [references/troubleshooting.md](references/troubleshooting.md) explains every
code.

**Convert** with the envelope that matches the endpoint:

| Destination | Convert | Then |
|---|---|---|
| New Jira comment | `adf convert c.md -e jira-comment -o c.json` | `POST /rest/api/3/issue/{key}/comment` |
| Edit a Jira comment | `adf convert c.md -e jira-comment -o c.json` | `PUT /rest/api/3/issue/{key}/comment/{id}` |
| Jira description | `adf convert d.md -e jira-description -o d.json` | `PUT /rest/api/3/issue/{key}` |
| Jira rich-text custom field | `adf convert f.md -e jira-field --field customfield_10010 -o f.json` | `PUT /rest/api/3/issue/{key}` |
| Confluence page or footer comment | `adf convert p.md -e confluence -o body.json` | wrap as `body` with `jq`, then `POST /wiki/api/v2/pages` or `/footer-comments` |
| acli | `adf convert d.md -o d.json` (bare document) | `acli jira workitem edit --description-file`, `comment update --body-adf` |

**Post it.** For a Jira comment:

```sh
adf convert "$work/comment.md" -e jira-comment -o "$work/comment.json" &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @"$work/comment.json" \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/comment"
```

All recipes (descriptions, custom fields, new issues, Confluence pages and comments, acli) are in
[references/posting.md](references/posting.md).

**Check the result.** Look at the HTTP status and tell the user what was posted where. A `400`
from Jira rarely says why; run `adf validate` on the file you sent and check the envelope.

Avoid `acli jira workitem comment create` for formatted comments: it posts ADF as plain text
(raw JSON in the comment). Use the REST recipe instead.

## 3. Read Jira or Confluence content as Markdown

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o "$work/issue.json" \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123?fields=description" &&
  adf convert --from adf "$work/issue.json" --md-flavor plain
```

`--from adf` finds the document in a bare ADF document, a Jira issue response
(`fields.description`, or `--field NAME`), a Jira comment (`body`), and a Confluence page fetched
with `?body-format=atlas_doc_format`. Use `--md-flavor plain` to read or summarise; it is compact
and drops what Markdown cannot show.

## 4. Edit existing content

1. Fetch it and convert with the default flavour, `roundtrip`:
   `adf convert --from adf "$work/issue.json" -o "$work/description.md"`.
2. Edit the Markdown. Leave any ```` ```adf ```` blocks unchanged: they hold nodes Markdown cannot
   express and are restored exactly.
3. Convert and `PUT` it back as in section 2.

Roundtrip warnings such as `W_ATTR_DROPPED` (column widths, cell colours) mean those details
are lost when you post back. Tell the user before overwriting such content.

## Markdown that `adf` understands

CommonMark and GitHub Flavored Markdown (headings, emphasis, lists, task lists, code, quotes,
tables, links), plus:

| Write | Get in Jira/Confluence |
|---|---|
| `> [!NOTE]` / `[!TIP]` / `[!IMPORTANT]` / `[!WARNING]` / `[!CAUTION]` | info / success / note / warning / error panel |
| `<details><summary>Title</summary>` + blank line + content + blank line + `</details>` | expand section |
| `::: cell` … `:::` | content in a framed box (1×1 table) |
| `[@Jane Doe](adf:mention/<account-id>)` | mention (needs the Atlassian account ID) |
| `[IN REVIEW](adf:status/blue)` | status lozenge (neutral, purple, blue, red, yellow, green) |
| `[2026-10-19](adf:date)` | date |
| `[😄](adf:emoji/:smile:)` | emoji |
| `[https://site.atlassian.net/browse/PROJ-1](adf:card)` | smart link card |

Keep tables, headings, rules, panels and `<details>` at the top level, not inside lists or
quotes; `adf` moves them out otherwise (`W_BLOCK_HOISTED`, `W_HEADING_DEMOTED`). The full
cheat sheet is [references/syntax.md](references/syntax.md).

## Exit codes

| Code | Meaning | What to do |
|---|---|---|
| 0 | Success; warnings may have been printed | Read the warnings |
| 1 | `E_INTERNAL`, a bug in `adf` | Don't retry; simplify the input or tell the user |
| 2 | `E_USAGE`, invalid command line | Fix the command (`adf <command> --help`) |
| 3 | Input or I/O problem | Fix the input file or path |
| 4 | ADF validation failed (`validate` only) | Regenerate the ADF from Markdown |
| 5 | Lossy conversion refused (`--strict`, `--html error`) | Fix the Markdown, or drop `--strict` if the user accepts the loss |

When stderr is not a terminal, diagnostics are JSON lines (`--error-format json` forces it):

```json
{"schema_version":1,"severity":"warning","code":"W_HEADING_DEMOTED","message":"…","location":{"file":"comment.md","line":4,"column":3}}
```
