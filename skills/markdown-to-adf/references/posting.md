# Posting and reading recipes

`adf` converts only. Post its output with `curl` (or acli). Everything here targets Jira Cloud
(REST API v3) and Confluence Cloud (REST API v2); Jira Data Center and Server don't use ADF.
More background: [docs/posting.md](https://github.com/Yornik/adf-converter/blob/main/docs/posting.md)
and [docs/envelopes.md](https://github.com/Yornik/adf-converter/blob/main/docs/envelopes.md).

## Install adf

Only with the user's agreement. Pick the target for the machine (`uname -sm`):

| `uname -sm` | Target |
|---|---|
| `Darwin arm64` | `aarch64-apple-darwin` |
| `Darwin x86_64` | `x86_64-apple-darwin` |
| `Linux x86_64` | `x86_64-unknown-linux-musl` |
| `Linux aarch64` | `aarch64-unknown-linux-musl` |
| Windows | `x86_64-pc-windows-msvc` or `aarch64-pc-windows-msvc` |

```sh
target=aarch64-apple-darwin
dir=$(mktemp -d) && cd "$dir"
gh release download -R Yornik/adf-converter -p "adf-converter-v*-$target.zip" -p SHA256SUMS
shasum -a 256 -c SHA256SUMS --ignore-missing    # Linux: sha256sum -c SHA256SUMS --ignore-missing
unzip -q adf-converter-v*-"$target".zip
mkdir -p ~/.local/bin && cp adf-converter-v*-"$target"/adf ~/.local/bin/
~/.local/bin/adf --version
```

Without `gh`, download the zip and `SHA256SUMS` from
<https://github.com/Yornik/adf-converter/releases/latest>. With Rust installed:
`cargo install --git https://github.com/Yornik/adf-converter --locked adf-converter`.

## Credentials

The recipes expect these environment variables. They are the user's; `adf` doesn't read them.

| Variable | Example |
|---|---|
| `JIRA_BASE_URL` | `https://your-site.atlassian.net` (no trailing slash) |
| `CONFLUENCE_BASE_URL` | Confluence recipes only: `https://your-site.atlassian.net/wiki` |
| `JIRA_EMAIL` | the Atlassian account email |
| `JIRA_API_TOKEN` | an API token from <https://id.atlassian.com/manage-profile/security/api-tokens> |

- Check that they are set without printing them:
  `for v in JIRA_BASE_URL JIRA_EMAIL JIRA_API_TOKEN; do printenv "$v" >/dev/null || echo "$v is not set"; done`
  (add `CONFLUENCE_BASE_URL` for Confluence).
- If they are missing, ask the user to set them. Never ask the user to paste a token into the
  chat, and never write a token into a file or a command you print.
- With a classic token, Confluence on the same site uses the same token, and
  `CONFLUENCE_BASE_URL` is the site URL plus `/wiki`. If only `JIRA_BASE_URL` is set, use
  `"$JIRA_BASE_URL/wiki"`.
- Scoped API tokens use Atlassian's gateway instead of the site URL:
  `https://api.atlassian.com/ex/jira/<cloudId>` for Jira and
  `https://api.atlassian.com/ex/confluence/<cloudId>/wiki` for Confluence. A scoped token is
  created for one product, so Confluence needs its own.

## The pattern

```sh
work=$(mktemp -d) && echo "$work"    # once; note the path
# 1. write $work/x.md   2. adf validate --from md -e ENVELOPE --json "$work/x.md"
adf convert "$work/x.md" -e ENVELOPE -o "$work/x.json" &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @"$work/x.json" URL
```

- The recipes use `"$work"` for the scratch directory. If your shell does not keep variables
  between commands (many agent tools start a new shell for each one), start every command with
  `work=<the printed path>;`, or write the path out.
- `-o` writes atomically and only on success; `&&` stops curl when `adf` fails.
- Never `adf … | curl --data-binary @-`: a failed conversion still sends an (empty) request.
- `--fail-with-body` makes HTTP errors fail the command and still shows Jira's message.
- The content can also be a CSV or TSV file: `adf convert data.csv --from csv -e ENVELOPE -o
  "$work/x.json"` replaces step 1, and everything after it is the same. See
  [tables.md](tables.md) (needs adf 0.3.0 or later).

## Jira

| Task | Envelope | Request |
|---|---|---|
| Add a comment | `-e jira-comment` | `POST $JIRA_BASE_URL/rest/api/3/issue/KEY/comment` → `201`, response has the comment `id` |
| Replace a comment | `-e jira-comment` | `PUT $JIRA_BASE_URL/rest/api/3/issue/KEY/comment/ID` → `200` |
| Replace the description | `-e jira-description` | `PUT $JIRA_BASE_URL/rest/api/3/issue/KEY` → `204` |
| Set a rich-text custom field | `-e jira-field --field customfield_NNNNN` | `PUT $JIRA_BASE_URL/rest/api/3/issue/KEY` → `204` |

`PUT` needs `-X PUT` in curl. Examples:

```sh
# Add a comment
adf convert "$work/c.md" -e jira-comment -o "$work/c.json" &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @"$work/c.json" \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/comment"

# Replace the description
adf convert "$work/d.md" -e jira-description -o "$work/d.json" &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @"$work/d.json" \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123"

# Create an issue with a description
adf convert "$work/d.md" -e jira-description -o "$work/d.json" &&
  jq '.fields += {project: {key: "PROJ"}, issuetype: {name: "Task"}, summary: "Short summary"}' \
    "$work/d.json" > "$work/issue.json" &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @"$work/issue.json" \
    "$JIRA_BASE_URL/rest/api/3/issue"
```

Restrict a comment's visibility by adding it before posting:
`jq '. + {visibility: {type: "role", value: "Administrators"}}'`.

## Confluence

`-e confluence` produces the `body` value (`{"representation":"atlas_doc_format","value":"<the
document as a JSON string>"}`). Wrap it with `jq`; never re-encode `value`.

```sh
# Create a page (spaceId is numeric: GET $CONFLUENCE_BASE_URL/api/v2/spaces?keys=KEY → .results[0].id)
adf convert "$work/p.md" -e confluence -o "$work/body.json" &&
  jq --arg space "$SPACE_ID" --arg title "Page title" \
    '{spaceId: $space, status: "current", title: $title, body: .}' "$work/body.json" > "$work/page.json" &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @"$work/page.json" \
    "$CONFLUENCE_BASE_URL/api/v2/pages"

# Update a page: send id, status, title, body and the next version number
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o "$work/current.json" \
    "$CONFLUENCE_BASE_URL/api/v2/pages/$PAGE_ID" &&
  adf convert "$work/p.md" -e confluence -o "$work/body.json" &&
  jq --slurpfile cur "$work/current.json" \
    '{id: $cur[0].id, status: "current", title: $cur[0].title, body: .,
      version: {number: ($cur[0].version.number + 1)}}' "$work/body.json" > "$work/update.json" &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @"$work/update.json" \
    "$CONFLUENCE_BASE_URL/api/v2/pages/$PAGE_ID"

# Add a footer comment
adf convert "$work/c.md" -e confluence -o "$work/body.json" &&
  jq --arg page "$PAGE_ID" '{pageId: $page, body: .}' "$work/body.json" > "$work/comment.json" &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @"$work/comment.json" \
    "$CONFLUENCE_BASE_URL/api/v2/footer-comments"
```

## Reading content as Markdown

Fetch to a file, then convert. `--md-flavor plain` is compact for reading and summarising; the
default `roundtrip` flavour is for editing and posting back.

| Content | Fetch | Convert |
|---|---|---|
| Jira description | `GET /rest/api/3/issue/KEY?fields=description` | `adf convert --from adf issue.json --md-flavor plain` |
| Jira custom field | `GET /rest/api/3/issue/KEY?fields=customfield_NNNNN` | add `--field customfield_NNNNN` |
| One Jira comment | `GET /rest/api/3/issue/KEY/comment/ID` | `adf convert --from adf comment.json --md-flavor plain` |
| All Jira comments | `GET /rest/api/3/issue/KEY/comment` | `jq -c '.comments[]'`, then convert each line from stdin (`-`) |
| Confluence page | `GET /wiki/api/v2/pages/ID?body-format=atlas_doc_format` | `adf convert --from adf page.json --md-flavor plain` |

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o "$work/comments.json" \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/comment" &&
  jq -c '.comments[]' "$work/comments.json" | while IFS= read -r c; do
    printf '%s\n' "$c" | adf convert --from adf - --md-flavor plain
  done
```

An empty Jira field is `null` and gives `E_INPUT_NOT_ADF` (exit 3): there is nothing to convert.

## Editing and posting back

```sh
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o "$work/issue.json" \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123?fields=description" &&
  adf convert --from adf "$work/issue.json" -o "$work/d.md"
# edit $work/d.md, leaving ```adf blocks unchanged, then:
adf convert "$work/d.md" -e jira-description --images external -o "$work/d.json" &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @"$work/d.json" \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123"
```

- Use the default `roundtrip` flavour (no `--md-flavor`) for the Markdown you edit.
- Convert back with `--images external`. Roundtrip Markdown writes embedded external images as
  `![alt](url)`; without the flag they come back as text links (`W_IMAGE_AS_LINK`).
- Use the envelope of the destination: `-e jira-comment` for a comment, `-e confluence` (wrapped
  as in [Confluence](#confluence)) for a page.
- If `--from adf` reported `W_ATTR_DROPPED`, posting back loses those details (column widths, cell
  colours, alignment, Confluence inline-comment anchors). Tell the user before overwriting.

## Attaching files and screenshots

An uploaded image is a reference to a media-service file, not a URL. `adf` never uploads or
downloads and never looks ids up: you do the API calls, then write
`![alt](adf:media/<media id>?width=W&height=H)` in the Markdown (syntax:
[syntax.md](syntax.md#file-media-media)), or `[file name](adf:attachment/<media id>)` for a file
that is not a picture (`adf` 0.4.0 or later). Steps are labelled **documented** (Atlassian API
reference), **community-reported** (not documented, may break) or **unverified**; say so when you
tell the user what you did. The full recipe with sources is
[docs/posting.md](https://github.com/Yornik/adf-converter/blob/main/docs/posting.md#attaching-files-and-screenshots).

The media id is **not** the numeric attachment id. Only upload files the user asked you to.

```sh
# Jira: 1. upload (documented). The response is an array; .[0].id is the ATTACHMENT id.
curl --fail-with-body -sS -X POST -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  -H 'X-Atlassian-Token: no-check' -F 'file=@shot.png' \
  "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/attachments" -o "$work/att.json"

# 2. media id (community-reported, undocumented): do not follow the redirect and never print it,
#    it can hold a short-lived token. Accept HTTP 302 or 303; the id is the second path segment
#    of https://api.media.atlassian.com/file/<media id>/binary?...
att_id=$(jq -r '.[0].id' "$work/att.json")
out=$(curl -sS -o /dev/null -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -w '%{http_code} %{redirect_url}' \
  "$JIRA_BASE_URL/rest/api/3/attachment/content/$att_id")
media_id=$(printf '%s\n' "${out#* }" |
  sed -n 's|^https://api\.media\.atlassian\.com/file/\([0-9A-Fa-f-]\{36\}\)/.*|\1|p')
test -n "$media_id" || echo "no media id (HTTP ${out%% *})" >&2
```

3. Write the line (Jira: no `collection`), pre-flight and post as in the sections above:
   `![shot.png](adf:media/$media_id?width=1512&height=945)` (use an unquoted heredoc so the shell
   fills in `$media_id`). Read the pixel size from the file: `file shot.png` prints it, and so do
   `sips -g pixelWidth -g pixelHeight shot.png` (macOS) and `identify -format '%w %h' shot.png`
   (ImageMagick). Attach the file to the same issue that holds the comment or description.

```sh
# Confluence: 1. upload to the page with v1 (documented); header value is nocheck.
curl --fail-with-body -sS -X POST -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
  -H 'X-Atlassian-Token: nocheck' -F 'file=@shot.png' \
  "$CONFLUENCE_BASE_URL/rest/api/content/$PAGE_ID/child/attachment" -o "$work/att.json"
# 2. media id and collection (community-reported): the v2 attachment `fileId`, never the `att…` id.
jq -r '.results[0].extensions | "\(.fileId) \(.collectionName)"' "$work/att.json"
```

3. Write `![shot.png](adf:media/<fileId>?collection=<collectionName>&width=W&height=H)` and post
   with `-e confluence` as above.

Pitfalls:
- Wrong id (attachment id, made-up or from another issue): `adf` accepts any UUID; Jira answers
  `400` `ATTACHMENT_VALIDATION_ERROR` (community-reported). A non-UUID gives
  `W_INVALID_EXTENSION_ARG` and the alt text.
- Confluence: attaching a file that is already on the page to a comment on that page is reported
  to remove it from the page (CONFCLOUD-84716, open), so give every upload a unique name. Files
  uploaded through REST are reported not to render in Confluence comments (CONFCLOUD-84916;
  editor uploads use the collection `comment-container-<pageId>`). The v1 attachment endpoints may
  be retired (CONFCLOUD-84981 reports 410 Gone for the comment one), so check the upload in the
  sandbox run before relying on it.
- Media is a block. In the middle of a paragraph it splits the paragraph, and after text in a
  heading it follows the heading, both without a warning; it stays inside list items, quotes,
  panels, expands and table cells; only in task items and `adf:` labels is it moved out
  (`W_BLOCK_HOISTED`). Put it on its own line. A table cell or list item holding only an image
  makes the whole table or list a raw ```` ```adf ```` block when read back. Give `width` and
  `height`.
- JSM request comments are plain strings, not ADF; use `POST /rest/api/3/issue/{key}/comment`.
- Other files (PDF, logs, archives): write `[name](adf:attachment/<media id>)` (`adf` 0.4.0 or
  later) with the media id and collection found as above ([syntax.md](syntax.md#attached-files-media)). A paragraph of only
  such links is a `mediaGroup`, a link amid text a `mediaInline`. `adf` validates both nodes for
  Jira and Confluence, but whether the products render them when written through the API is
  **unverified**: tell the user, and check the result.

Reading: the roundtrip flavour writes `![alt](adf:media/<id>?…)` lines and
`[name](adf:attachment/<id>?…)` links; the plain flavour writes
markers such as `[image: alt]` and `[attachment: name]`. To fetch a file, list the issue's
attachments (`GET /rest/api/3/issue/KEY?fields=attachment`, match `.filename` to the alt text,
download `GET /rest/api/3/attachment/content/ID` with `curl -L -o FILE`), or on Confluence list
`GET /wiki/api/v2/pages/ID/attachments` and select the entry whose `fileId` equals the id. These
are your calls; `adf` does not make them. If the alt text does not identify the attachment, repeat
the redirect lookup for each candidate and compare (community-reported).

## acli

acli takes a bare document: convert without `-e`.

```sh
# Description (acli documents --description-file as "plain text or ADF")
adf convert "$work/d.md" -o "$work/d.json" &&
  acli jira workitem edit --key PROJ-123 --description-file "$work/d.json" --yes

# Replace an existing comment
adf convert "$work/c.md" -o "$work/c.json" &&
  acli jira workitem comment update --key PROJ-123 --id 10001 --body-adf "$work/c.json"
```

- `acli jira workitem comment create` has no `--body-adf`, and ADF passed to `--body` is posted
  as plain text (atlassian/homebrew-acli#45, open as of acli v1.3.15). To add a formatted
  comment, use the REST recipe.
- These acli behaviours come from Atlassian's acli reference and that issue; they were not tested
  against a live site. Check the result after posting.
