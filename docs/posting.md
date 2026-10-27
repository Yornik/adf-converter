# Posting to Jira and Confluence

`adf` only converts: it never makes network requests and never reads credentials. This page shows
how to send its output with `curl` (or acli) and how to read content back as Markdown.

The examples target **Jira Cloud** (REST API v3) and **Confluence Cloud** (REST API v2). Jira
Data Center and Server do not use ADF.

## Before you start

The recipes read these environment variables. You set them; `adf` itself ignores them.

```sh
export JIRA_BASE_URL=https://your-site.atlassian.net          # no trailing slash
export CONFLUENCE_BASE_URL="$JIRA_BASE_URL/wiki"              # only for the Confluence recipes
export JIRA_EMAIL=you@example.com
read -rs JIRA_API_TOKEN && export JIRA_API_TOKEN                # paste the token; it is not echoed
```

- Create an API token at <https://id.atlassian.com/manage-profile/security/api-tokens>. It
  authenticates as you, with HTTP basic auth (`-u "$JIRA_EMAIL:$JIRA_API_TOKEN"`).
- With a classic (unscoped) token, Jira and Confluence on the same Atlassian Cloud site share the
  site URL and the token, so `CONFLUENCE_BASE_URL` is the site URL plus `/wiki`.
- **Scoped API tokens** go through Atlassian's API gateway instead of your site URL, with a
  different base URL per product:
  - `JIRA_BASE_URL=https://api.atlassian.com/ex/jira/<cloudId>`;
  - `CONFLUENCE_BASE_URL=https://api.atlassian.com/ex/confluence/<cloudId>/wiki`.

  A scoped token is created for one product, so the Confluence recipes need a token made for
  Confluence. These recipes have not been tested with scoped tokens.
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
read or give to an LLM. Leave the default (`roundtrip`) if you want to edit the Markdown and post
it back; see [Edit and post back](#edit-and-post-back).

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

### Edit and post back

Convert with the default `roundtrip` flavour, edit the Markdown, and convert it back with
`--images external`:

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o issue.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123?fields=description" &&
  adf convert --from adf issue.json -o description.md
# Edit description.md. Leave ```adf blocks unchanged: they hold nodes Markdown cannot express.
adf convert description.md -e jira-description --images external -o description.json &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @description.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123"
```

- **`--images external`**: roundtrip Markdown writes an embedded external image as
  `![alt](url)`. With the default `--images link`, it would come back as a text link
  (`W_IMAGE_AS_LINK`). With `--images external`, images you add are embedded by URL too.
- Uploaded images and screenshots come back as `![alt](adf:media/<id>?…)` lines when `adf` can
  express them; leave those lines unchanged (see
  [Attaching files and screenshots](#attaching-files-and-screenshots)).
- Use the envelope of the place you post to: `-e jira-comment` for a comment, `-e confluence` for
  a Confluence page (see [Update a page](#update-a-page)).
- `W_ATTR_DROPPED` from `--from adf` means that posting back loses something Markdown cannot
  express: column widths, cell colours, alignment, or the anchors of Confluence inline comments,
  for example. Check before you overwrite such content.

## Confluence

Confluence takes the document as a JSON *string* inside `body` (see
[envelopes.md](envelopes.md#confluence)). `-e confluence` produces that `body` value; wrap it
with `jq`.

### Create a page

Pages are created in a space by its numeric ID, not its key. Look the ID up once:

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  "$CONFLUENCE_BASE_URL/api/v2/spaces?keys=DOCS" | jq -r '.results[0].id'
```

```sh
adf convert page.md -e confluence -o body.json &&
  jq --arg space "$SPACE_ID" --arg title "Release notes" \
    '{spaceId: $space, status: "current", title: $title, body: .}' body.json > page.json &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @page.json \
    "$CONFLUENCE_BASE_URL/api/v2/pages"
```

Add `parentId` to place the page under another page.

### Update a page

An update must send the page's `id`, `status`, `title`, `body` and the next `version.number`:

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o current.json \
    "$CONFLUENCE_BASE_URL/api/v2/pages/$PAGE_ID" &&
  adf convert page.md -e confluence -o body.json &&
  jq --slurpfile cur current.json \
    '{id: $cur[0].id, status: "current", title: $cur[0].title, body: .,
      version: {number: ($cur[0].version.number + 1)}}' body.json > update.json &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @update.json \
    "$CONFLUENCE_BASE_URL/api/v2/pages/$PAGE_ID"
```

### Add a footer comment

```sh
adf convert comment.md -e confluence -o body.json &&
  jq --arg page "$PAGE_ID" '{pageId: $page, body: .}' body.json > comment.json &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @comment.json \
    "$CONFLUENCE_BASE_URL/api/v2/footer-comments"
```

### Read a page as Markdown

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o page.json \
    "$CONFLUENCE_BASE_URL/api/v2/pages/$PAGE_ID?body-format=atlas_doc_format" &&
  adf convert --from adf page.json --md-flavor plain
```

## Attaching files and screenshots

Images and files in Jira and Confluence are not URLs. A screenshot in a comment is a reference to a
file in Atlassian's media service (an ADF `media` node of type `file`, with an `id` and a
`collection`). To embed one, you upload the file, find out its **media id**, write that id into
your Markdown as `![alt](adf:media/<id>?…)` (see
[File media in syntax.md](syntax.md#file-media-adfmedia-images)), and post as usual.

`adf` never uploads and never downloads, and it does not call any API to find an id either.
Uploading, looking up ids and fetching files are steps for you (or your agent) and `curl`. Every
step below is labelled:

- **Documented**: described in Atlassian's API reference.
- **Community-reported**: seen working by other people or tools, not documented by Atlassian, and
  not yet checked by this project. These steps can stop working without notice.
- **Unverified**: nobody has confirmed it.

[maintainers.md](maintainers.md#sandbox-checklist-file-media) lists what still has to be tried on a
real site.

### Write the line

```markdown
Build failed on the second step:

![build.png](adf:media/11111111-2222-3333-4444-555555555555?width=1512&height=945)
```

- Jira: leave `collection` out (the collection is `""`).
- Confluence: `collection` is what the upload returned, for example `contentId-123456` (see
  [below](#confluence-upload-and-media-id)).
- Add `width` and `height`, the file's size in pixels. Read them from the file (for example
  `file shot.png`, or `identify -format '%w %h' shot.png`); neither upload API returns them.
  Atlassian's `media` documentation says both must be set inside a `mediaSingle`, or the media
  does not display. Whether the products still enforce that is unverified.
- Add `layout=wide`, `size=720px` or `size=50%` to control how it is shown
  ([all parameters](syntax.md#file-media-adfmedia-images)).
- `adf` checks the shape of the id, not that it exists. A wrong id converts without a warning and
  then fails when you post, or shows nothing.
- Media is a block. If you write the line inside a list item, a quote or a table cell, `adf` moves
  it out where the container does not allow it (`W_BLOCK_HOISTED`). Put it on its own line,
  outside lists and tables.

### Jira: upload and media id

**1. Upload** (documented). The multipart part is named `file`, and the header turns off Jira's
XSRF check. The response is an array with one entry per file.

```sh
curl --fail-with-body -sS -X POST -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  -H 'X-Atlassian-Token: no-check' -F 'file=@shot.png' \
  "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/attachments" -o attachment.json &&
  jq -r '.[0] | "\(.id) \(.filename) \(.mimeType) \(.size)"' attachment.json
```

`id` is a short number such as `10042`, the **attachment id**. It is not the media id. The response
contains no media id and no image dimensions.

**2. Find the media id** (community-reported, undocumented). Request the attachment's content
without following redirects. Jira answers with a redirect (`303` according to the API reference;
people also report `302`) to `https://api.media.atlassian.com/file/<media id>/binary?...`. The media
id is the second path segment.

```sh
att_id=$(jq -r '.[0].id' attachment.json)
out=$(curl -sS -o /dev/null -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  -w '%{http_code} %{redirect_url}' \
  "$JIRA_BASE_URL/rest/api/3/attachment/content/$att_id")
status=${out%% *}
media_id=$(printf '%s\n' "${out#* }" |
  sed -n 's|^https://api\.media\.atlassian\.com/file/\([0-9A-Fa-f-]\{36\}\)/.*|\1|p')
test -n "$media_id" && echo "$media_id" || echo "no media id found (HTTP $status)" >&2
```

- Do not add `-L`, and do not print or save the redirect URL: it can carry a short-lived access
  token for the media service. Keep only the id.
- If the status is `200` or the URL has another shape, the trick does not work (any more). See
  [Pitfalls](#pitfalls) for ways around media ids.

**3. Write, convert, check, post.** The media id goes into the Markdown. The unquoted heredoc lets
the shell fill in `$media_id`.

```sh
cat > comment.md <<EOF
Build failed on the second step:

![shot.png](adf:media/$media_id?width=1512&height=945)
EOF
adf validate --from md -e jira-comment comment.md &&
  adf convert comment.md -e jira-comment -o comment.json &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @comment.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/comment"
```

For a description, use `-e jira-description` and the `PUT` from
[Replace the description](#replace-the-description). Attach the file to the same issue that holds
the comment or description.

### Confluence: upload and media id

Confluence v2 has no upload endpoint
([CONFCLOUD-77196](https://jira.atlassian.com/browse/CONFCLOUD-77196)), so uploading uses the v1
API. The page must exist first: create it, upload, then update it ([create](#create-a-page),
[update](#update-a-page)).

**1. Upload** (documented). The header value is `nocheck` here, not `no-check`.

```sh
curl --fail-with-body -sS -X POST -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  -H 'X-Atlassian-Token: nocheck' -F 'file=@shot.png' \
  "$CONFLUENCE_BASE_URL/rest/api/content/$PAGE_ID/child/attachment" -o attachment.json
```

**2. Read the media id and the collection** (community-reported). The response lists the new
attachment in `results[0]`. Three independent tools use `extensions.fileId` as the media `id` and
`extensions.collectionName` (`contentId-<pageId>`) as the `collection`. The OpenAPI description
does not mention these fields.

```sh
jq -r '.results[0].extensions | "\(.fileId) \(.collectionName)"' attachment.json
```

The v2 API agrees that the id is not the attachment id: an attachment's `fileId` is "the ID
referenced in `atlas_doc_format` bodies and is distinct from the attachment ID" (the attachment
`id` looks like `att123456`). Use `fileId`, never `id` (documented).

**3. Write, convert, check, post.**

```markdown
![shot.png](adf:media/22222222-3333-4444-5555-666666666666?collection=contentId-123456&width=1512&height=945)
```

Convert with `-e confluence` and post as in [Update a page](#update-a-page) or
[Add a footer comment](#add-a-footer-comment).

**Comments are different** (Atlassian-staff ticket
[CONFCLOUD-84916](https://jira.atlassian.com/browse/CONFCLOUD-84916), open). A file attached in the
editor of a comment uses the collection `comment-container-<pageId>`. A file uploaded through the
REST API gets `contentId-<pageId>`, and the ticket reports that such a file does not render when
embedded in a comment. So embedding a REST-uploaded file in a Confluence comment is unreliable. Two
options, both unverified: put the screenshot on the page itself, or attach it in the comment editor
and read the comment back with `adf convert --from adf`, which shows the media id and collection
to reuse.

### Other files (PDF, logs, archives)

This version has no Markdown syntax for links to files (no `adf:attachment`). The products show
them as a `mediaGroup` (a block of file chips) or as `mediaInline` (inside a paragraph). You can
splice a `mediaGroup` in with a raw ```` ```adf ```` block, using the same id and collection as
above. `adf` accepts it; whether the product renders it in a comment or page has not been checked.

````markdown
```adf
{"type":"mediaGroup","content":[{"type":"media","attrs":{"type":"file","id":"66666666-7777-8888-9999-000000000000","collection":""}}]}
```
````

### Pitfalls

- **The media id is not the attachment id.** `adf:media/10042` is not a UUID: `adf` keeps the
  alt text and reports `W_INVALID_EXTENSION_ARG`. A made-up UUID, or a UUID from another issue,
  passes `adf` and is reported to fail in Jira with `400` `ATTACHMENT_VALIDATION_ERROR`
  (community-reported).
- **Same file name twice on Confluence.** Uploading a file with the name of an existing
  attachment can reassign it and leave the earlier media id unresolved in the page
  ([CONFCLOUD-84732](https://jira.atlassian.com/browse/CONFCLOUD-84732),
  [CONFCLOUD-84716](https://jira.atlassian.com/browse/CONFCLOUD-84716)). Give every upload a
  unique name.
- **Jira ignores the collection.** Reported readbacks of Jira content show `collection: ""`
  whatever was sent (community-reported). Leave it out for Jira.
- **Jira Service Management.** The comment bodies of the JSM request API are plain strings, not
  ADF ([API reference](https://dac-static.atlassian.com/cloud/jira/service-desk/swagger.v3.json)).
  Use the core `POST /rest/api/3/issue/{key}/comment` shown above.
- **Private comments.** The API reference says attachments added in a private comment get the
  comment's restriction. Whether the redirect lookup works for them is unverified.
- **Inline comments on Confluence** need `inlineCommentProperties` next to the body, which `adf`
  does not write.
- **Without media ids.** Two workarounds are reported but not checked: Jira REST API v2 accepts
  wiki markup such as `!shot.png|thumbnail!` for attachments of the issue, and `--images external`
  can embed an image from a URL that every reader's browser can load without signing in.
- **Other tools.** acli has no attachment upload command. The Rovo MCP server lists upload tools
  for Jira and Confluence ([supported
  tools](https://support.atlassian.com/atlassian-rovo-mcp-server/docs/supported-tools/)) without
  parameter descriptions; whether they return media ids or keep media nodes is unverified.

### Reading content that holds media

Converting an issue, comment or page with the default (roundtrip) flavour writes a file image as
a line that you can read, edit and post back unchanged:

```sh
adf convert --from adf comment.json
# ![shot.png](adf:media/11111111-2222-3333-4444-555555555555?width=1512&height=945)
```

What `adf` cannot write that way stays a ```` ```adf ```` block with `W_ADF_PASSTHROUGH`, or becomes
a marker in the plain flavour (`[image: shot.png]`, `[attachment: log.txt]`; see
[syntax.md](syntax.md#plain-flavour-media)). `adf` only shows you the media id. Fetching the file is
a separate step that you run yourself:

- **Jira** (the calls are documented; matching a media id to an attachment is not). List the
  issue's attachments, pick the one that belongs to the media id, and download it. The `alt` is
  often the file name, so match on it:

  ```sh
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
      "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123?fields=attachment" |
    jq -r --arg name "shot.png" '.fields.attachment[] | select(.filename == $name) | .id'
  curl --fail-with-body -sS -L -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o shot.png \
    "$JIRA_BASE_URL/rest/api/3/attachment/content/10042"
  ```

  If two attachments share the name, or the media has no `alt`, run the redirect lookup from
  [step 2](#jira-upload-and-media-id) for each candidate attachment and compare the result with
  the id in the Markdown (community-reported).
- **Confluence** (documented calls; the matching is yours). List the page's attachments and select
  the one whose `fileId` equals the id in the Markdown. Its `downloadLink` is a path for
  downloading the file with the same credentials; whether it needs the `/wiki` prefix on your site
  is unverified:

  ```sh
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
      "$CONFLUENCE_BASE_URL/api/v2/pages/$PAGE_ID/attachments" |
    jq -r --arg id "22222222-3333-4444-5555-666666666666" \
      '.results[] | select(.fileId == $id) | "\(.title) \(.downloadLink)"'
  ```

Sources for the community-reported steps:
[developer community 74338](https://community.developer.atlassian.com/t/74338) and
[an Atlassian community answer](https://community.atlassian.com/t/5/Jira-questions/How-can-I-add-an-image-in-a-comment/qaq-p/952016)
(the redirect lookup, 2023 to 2024),
[developer community 94822](https://community.developer.atlassian.com/t/94822) (shapes seen in
real Jira data), and the Confluence tickets linked above.

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
| `400` with `INVALID_INPUT` or "not valid Atlassian Document Format" | The body is not valid ADF for this endpoint, or it is the wrong envelope. | Run `adf validate` on the file `adf convert` wrote (for Confluence, `body.json`: `adf` does not read the request that `jq` builds around it); check the envelope against [envelopes.md](envelopes.md). |
| `400` "Operation value must be an Atlassian Document" | A string or wiki markup was sent to an ADF field. | Send `adf` output instead. |
| `400` "Comment body can not be empty" | An empty body, often from piping a failed `adf` into curl. | Convert to a file first. |
| `400` "Invalid message" (Confluence) | `value` was an object instead of a string. | Use `-e confluence`. |
| `400` `CONTENT_LIMIT_EXCEEDED`, or a size error | The body is too large. | Watch for `W_SIZE_LIMIT`; split the content. |
| `400` `ATTACHMENT_VALIDATION_ERROR` (community-reported) | The id in an `adf:media` image is not the media id of a file attached to this issue (an attachment id, a made-up UUID, or one from another issue). | See [Attaching files and screenshots](#attaching-files-and-screenshots). |
| `401` / `403` | Wrong email or token, a scoped token on the site URL, or missing permissions. | Check the token and the base URL. |
| `404` | Wrong issue key, page ID or site, or no permission to see it. | Check the URL. |
