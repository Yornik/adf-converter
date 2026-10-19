# Posting to Jira and Confluence

`adf` only converts: it never makes network requests and never reads credentials. This page shows
how to send its output with `curl` (or acli) and how to read content back as Markdown.

The examples target **Jira Cloud** (REST API v3) and **Confluence Cloud** (REST API v2). Jira
Data Center and Server do not use ADF.

## Before you start

The recipes read three environment variables. You set them; `adf` itself ignores them.

```sh
export JIRA_BASE_URL=https://your-site.atlassian.net    # no trailing slash
export JIRA_EMAIL=you@example.com
read -rs JIRA_API_TOKEN && export JIRA_API_TOKEN          # paste the token; it is not echoed
```

- Create an API token at <https://id.atlassian.com/manage-profile/security/api-tokens>. It
  authenticates as you, with HTTP basic auth (`-u "$JIRA_EMAIL:$JIRA_API_TOKEN"`).
- Jira and Confluence on the same Atlassian Cloud site share the URL and the token, so the
  Confluence recipes use `$JIRA_BASE_URL/wiki/…`.
- **Scoped API tokens** go through Atlassian's API gateway instead of your site URL:
  `https://api.atlassian.com/ex/jira/<cloudId>` for Jira (set `JIRA_BASE_URL` to it) and
  `https://api.atlassian.com/ex/confluence/<cloudId>` for Confluence. These recipes have not been
  tested with scoped tokens.
- Never print the token, paste it into a command line that ends up in your shell history, or
  commit it. On a shared machine, prefer a netrc file (`curl --netrc-file FILE`) over `-u`, so the
  token does not appear in the process list.

### Always convert to a file, then post

```sh
adf convert comment.md -e jira-comment -o comment.json &&
  curl … --data-binary @comment.json …
```

Don't pipe `adf` straight into `curl` (`adf … | curl --data-binary @- …`). Both programs start at
the same time; if `adf` fails, it writes nothing, and curl still sends the request with an empty
body. `set -o pipefail` only changes the exit status afterwards. With `-o FILE &&`, curl runs only if
the conversion succeeded, and `-o` writes the file atomically, so it never contains a partial
document.

Every recipe uses `curl --fail-with-body` (curl 7.76 or later): an HTTP error gives a non-zero
exit status and still prints Jira's error message. Add `adf validate --from md FILE` before
converting if you want to see warnings first; with `--strict`, any warning stops the conversion
(exit 5).

## Jira

Replace `PROJ-123` with your issue key.

### Add a comment

```sh
adf convert comment.md -e jira-comment -o comment.json &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @comment.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/comment"
```

The response (`201 Created`) is the new comment; its `id` is what you need to update it later
(`… | jq -r .id`, or add `-o created.json` and read it from the file).

To restrict visibility, add it to the body before posting:

```sh
jq '. + {visibility: {type: "role", value: "Administrators"}}' comment.json > restricted.json
```

### Update a comment

```sh
adf convert comment.md -e jira-comment -o comment.json &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @comment.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/comment/10001"
```

### Replace the description

```sh
adf convert description.md -e jira-description -o description.json &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @description.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123"
```

A successful update returns `204 No Content`.

### Set a rich-text custom field

Find the field ID (`customfield_NNNNN`) with `GET /rest/api/3/field`. Only multi-line
("Paragraph") text fields take ADF.

```sh
adf convert notes.md -e jira-field --field customfield_10010 -o field.json &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @field.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123"
```

### Create an issue with a description

Merge the other required fields into the envelope with `jq`:

```sh
adf convert description.md -e jira-description -o description.json &&
  jq '.fields += {project: {key: "PROJ"}, issuetype: {name: "Task"}, summary: "Short summary"}' \
    description.json > issue.json &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @issue.json \
    "$JIRA_BASE_URL/rest/api/3/issue"
```

### Read an issue or comments as Markdown

`adf convert --from adf` unwraps Jira's responses itself (see
[envelopes.md](envelopes.md#reading---from-adf)). Use `--md-flavor plain` for compact text to
read or give to an LLM; leave the default (`roundtrip`) if you want to edit the Markdown and post
it back without losing anything.

```sh
# Description
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o issue.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123?fields=description" &&
  adf convert --from adf issue.json --md-flavor plain

# A rich-text custom field
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o issue.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123?fields=customfield_10010" &&
  adf convert --from adf issue.json --field customfield_10010 --md-flavor plain

# One comment
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o comment.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/comment/10001" &&
  adf convert --from adf comment.json --md-flavor plain

# All comments (the list response wraps them in a "comments" array)
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o comments.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/comment" &&
  jq -c '.comments[]' comments.json | while IFS= read -r c; do
    printf '%s\n' "$c" | adf convert --from adf - --md-flavor plain
  done
```

An empty description is `null` in Jira's response; `adf` reports that as `E_INPUT_NOT_ADF`
(exit 3).

## Confluence

Confluence takes the document as a JSON *string* inside `body` (see
[envelopes.md](envelopes.md#confluence)). `-e confluence` produces that `body` value; wrap it
with `jq`.

### Create a page

Pages are created in a space by its numeric ID, not its key. Look the ID up once:

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  "$JIRA_BASE_URL/wiki/api/v2/spaces?keys=DOCS" | jq -r '.results[0].id'
```

```sh
adf convert page.md -e confluence -o body.json &&
  jq --arg space "$SPACE_ID" --arg title "Release notes" \
    '{spaceId: $space, status: "current", title: $title, body: .}' body.json > page.json &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @page.json \
    "$JIRA_BASE_URL/wiki/api/v2/pages"
```

Add `parentId` to place the page under another page.

### Update a page

An update must send the page's `id`, `status`, `title`, `body` and the next `version.number`:

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o current.json \
    "$JIRA_BASE_URL/wiki/api/v2/pages/$PAGE_ID" &&
  adf convert page.md -e confluence -o body.json &&
  jq --slurpfile cur current.json \
    '{id: $cur[0].id, status: "current", title: $cur[0].title, body: .,
      version: {number: ($cur[0].version.number + 1)}}' body.json > update.json &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @update.json \
    "$JIRA_BASE_URL/wiki/api/v2/pages/$PAGE_ID"
```

### Add a footer comment

```sh
adf convert comment.md -e confluence -o body.json &&
  jq --arg page "$PAGE_ID" '{pageId: $page, body: .}' body.json > comment.json &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @comment.json \
    "$JIRA_BASE_URL/wiki/api/v2/footer-comments"
```

### Read a page as Markdown

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o page.json \
    "$JIRA_BASE_URL/wiki/api/v2/pages/$PAGE_ID?body-format=atlas_doc_format" &&
  adf convert --from adf page.json --md-flavor plain
```

## acli (Atlassian CLI)

acli takes a bare ADF document, so use the default envelope (`doc`, no `-e`).

**What was checked.** On 2026-10-19 this project read Atlassian's acli command reference
(`jira workitem comment create`, `comment update`, `edit`, `view`) and changelog; the latest acli
then was v1.3.15-stable (2026-03-25). These facts come from those pages and from
[atlassian/homebrew-acli#45](https://github.com/atlassian/homebrew-acli/issues/45):

- `acli jira workitem comment create` has **no** `--body-adf` flag. Its `--body` help says "plain
  text or Atlassian Document Format (ADF)", but ADF passed there is posted as plain text, so the
  comment shows raw JSON (homebrew-acli#45, open since 2026-02-09; no fix in the changelog).
- `acli jira workitem comment update` has `--body-adf`: "Body in Atlassian Document Format (JSON
  file)".
- `acli jira workitem edit` has `--description-file`: "Read the description in plain text or
  Atlassian Document Format (ADF) from the file".

**What was not checked.** This project has not run acli against a Jira site. How acli tells ADF
apart from plain text, and the shape of its `--json` output, are undocumented. A community report
says a heading sent through `comment create --body-file` was rejected with `INVALID_INPUT`
(<https://community.developer.atlassian.com/t/96638>). Check the result after posting.

### Set a description

```sh
adf convert description.md -o description.json &&
  acli jira workitem edit --key PROJ-123 --description-file description.json --yes
```

### Update a comment

```sh
adf convert comment.md -o comment.json &&
  acli jira workitem comment update --key PROJ-123 --id 10001 --body-adf comment.json
```

### Add a comment

Use the [REST recipe](#add-a-comment). With acli only, the workaround from homebrew-acli#45 is
to create a placeholder comment and then replace it:

```sh
acli jira workitem comment create --key PROJ-123 --body "placeholder" --json
# Read the new comment's ID from the JSON output, then:
adf convert comment.md -o comment.json &&
  acli jira workitem comment update --key PROJ-123 --id 10001 --body-adf comment.json
```

Watchers are notified about the placeholder.

## jira-cli

[jira-cli](https://github.com/ankitpokhrel/jira-cli) (third-party, `ankitpokhrel/jira-cli`) has
its own Markdown support: its README (v1.7.0) documents Markdown and Jira wiki markup for
descriptions and comments, but no way to send a prepared ADF document. Use the REST recipes above
when you want `adf`'s output. `adf convert --from adf` also reads what jira-cli fetched, if you
save the raw API response.

## When Jira or Confluence says no

| Response | Likely cause | What to do |
|---|---|---|
| `400` with `INVALID_INPUT` or "not valid Atlassian Document Format" | The body is not valid ADF for this endpoint, or it is the wrong envelope. | Run `adf validate FILE`; check the envelope against [envelopes.md](envelopes.md). |
| `400` "Operation value must be an Atlassian Document" | A string or wiki markup was sent to an ADF field. | Send `adf` output instead. |
| `400` "Comment body can not be empty" | An empty body, often from piping a failed `adf` into curl. | Convert to a file first. |
| `400` "Invalid message" (Confluence) | `value` was an object instead of a string. | Use `-e confluence`. |
| `400` `CONTENT_LIMIT_EXCEEDED`, or a size error | The body is too large. | Watch for `W_SIZE_LIMIT`; split the content. |
| `401` / `403` | Wrong email or token, a scoped token on the site URL, or missing permissions. | Check the token and the base URL. |
| `404` | Wrong issue key, page ID or site, or no permission to see it. | Check the URL. |
