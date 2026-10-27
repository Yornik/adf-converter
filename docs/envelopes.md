# Envelopes

An ADF document on its own is `{"type":"doc","version":1,"content":[…]}`. Each Jira and
Confluence endpoint expects that document at a different place in its request body. `-e` /
`--envelope` wraps the document for the endpoint you call, so the output can be posted as-is.

The normative definition is [cli-contract.md#envelopes](cli-contract.md#envelopes); this page
adds examples and the endpoints. For complete `curl` and acli commands, see
[posting.md](posting.md).

All examples convert this Markdown:

```markdown
Hello **world**
```

Examples are shown compact. On a terminal, `adf` prints pretty JSON instead; `--compact` and
`--pretty` choose explicitly. The content is the same either way.

## Choosing an envelope

| You want to… | Envelope | Endpoint |
|---|---|---|
| pass the document to acli or embed it in your own JSON | `doc` (default) | acli `--body-adf`, `--description-file` |
| add or update a Jira comment | `jira-comment` | `POST /rest/api/3/issue/{key}/comment`, `PUT /rest/api/3/issue/{key}/comment/{id}` |
| set a Jira issue description | `jira-description` | `PUT /rest/api/3/issue/{key}` |
| set a rich-text (multi-line) Jira custom field | `jira-field --field customfield_NNNNN` | `PUT /rest/api/3/issue/{key}` |
| create or update a Confluence page or comment | `confluence` | the `body` of `POST /wiki/api/v2/pages`, `PUT /wiki/api/v2/pages/{id}`, `POST /wiki/api/v2/footer-comments` |

`--from csv` (a CSV or TSV file converted to a table) takes `-e` like Markdown input.
`--from adf --to csv` reads any of these envelopes, detected exactly as for `--from adf`, and
writes plain CSV text, so `-e` is not allowed with it.

`-t/--target auto` (the default) validates against Confluence's rules for `-e confluence` and
against Jira's rules for everything else. Use `-t confluence` to produce a bare `doc` for
Confluence.

## `doc` (default)

```sh
adf convert hello.md
```

```json
{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Hello "},{"type":"text","text":"world","marks":[{"type":"strong"}]}]}]}
```

The bare document. Use it with acli (`jira workitem comment update --body-adf FILE`,
`jira workitem edit --description-file FILE`), with tools that build their own request body, or
to embed the document in a larger JSON request with `jq`.

## `jira-comment`

```sh
adf convert hello.md -e jira-comment
```

```json
{"body":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Hello "},{"type":"text","text":"world","marks":[{"type":"strong"}]}]}]}}
```

The request body for:

- `POST /rest/api/3/issue/{issueIdOrKey}/comment`: add a comment (response `201`);
- `PUT /rest/api/3/issue/{issueIdOrKey}/comment/{id}`: replace a comment (response `200`).

To restrict who can see the comment, add a `visibility` object to the file before posting, for
example with `jq '. + {visibility: {type: "role", value: "Administrators"}}'`. Without it, the
comment is visible to everyone who can see the issue.

## `jira-description`

```sh
adf convert hello.md -e jira-description
```

```json
{"fields":{"description":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Hello "},{"type":"text","text":"world","marks":[{"type":"strong"}]}]}]}}}
```

The request body for `PUT /rest/api/3/issue/{issueIdOrKey}` (response `204`). To create an
issue, merge the other required fields into `fields` and `POST /rest/api/3/issue`; see
[posting.md](posting.md#create-an-issue-with-a-description).

## `jira-field`

```sh
adf convert hello.md -e jira-field --field customfield_10010
```

```json
{"fields":{"customfield_10010":{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"Hello "},{"type":"text","text":"world","marks":[{"type":"strong"}]}]}]}}}
```

Like `jira-description`, for any field that takes ADF: rich-text (multi-line, "Paragraph")
custom fields and `environment`. `--field` is required. Single-line text fields take a plain
string, not ADF, so they don't work with this envelope.

## `confluence`

```sh
adf convert hello.md -e confluence
```

```json
{"representation":"atlas_doc_format","value":"{\"type\":\"doc\",\"version\":1,\"content\":[{\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":\"Hello \"},{\"type\":\"text\",\"text\":\"world\",\"marks\":[{\"type\":\"strong\"}]}]}]}"}
```

This is the value of the `body` field in Confluence's v2 API, not a complete request. Wrap it
with the page or comment fields, for example:

```sh
adf convert page.md -e confluence -o body.json &&
  jq --arg space "$SPACE_ID" --arg title "Release notes" \
    '{spaceId: $space, status: "current", title: $title, body: .}' body.json > page.json
```

**String encoding.** Confluence requires `value` to be a *string* that contains the document
serialized as JSON, not a nested object; an object is rejected with `400`. `adf` writes the
document as compact JSON into that string, so every `"` in the document appears as `\"`. With
`--pretty`, only the outer object is indented; the string itself stays compact. Don't
re-serialize `value` yourself, or it gets encoded twice.

## Reading: `--from adf`

`adf convert --from adf` and `adf validate` accept a bare document or any of the shapes above,
plus what the Jira and Confluence APIs return when you read content. The document is found by
checking, in order:

| # | Input shape | Typical source | Document at (JSON Pointer) |
|---|---|---|---|
| 1 | `{"type":"doc",…}` | a bare document, `-e doc` output | `` (the whole input) |
| 2 | `{"body": DOC}` | `-e jira-comment` output, `GET /rest/api/3/issue/{key}/comment/{id}` | `/body` |
| 3 | `{"fields": {…}}` | `-e jira-description` / `-e jira-field` output, `GET /rest/api/3/issue/{key}` | `/fields/description`, or `/fields/<NAME>` with `--field NAME` |
| 4 | `{"representation":"atlas_doc_format","value":"…"}` | `-e confluence` output | `/value` (the string is parsed as JSON) |
| 5 | `{"body":{"atlas_doc_format":{"value":"…"}}}` | `GET /wiki/api/v2/pages/{id}?body-format=atlas_doc_format`, footer comments | `/body/atlas_doc_format/value` |

Anything else fails with `E_INPUT_NOT_ADF` (exit 3). That includes a Jira field that is `null`
(Jira returns `null` for an empty description) and list responses such as
`GET …/issue/{key}/comment`, which wrap comments in a `comments` array. Pick one item first, for
example `jq '.comments[0]' comments.json`.

Diagnostics for ADF input locate problems with a JSON Pointer into your input, starting at the
document's position: `/body/content/0` is the first block of a comment. In a Confluence body the
document is a JSON string, and the segments after `/value` point into the parsed string:
`/value/content/0` is `.value | fromjson | .content[0]` in jq. `adf validate --json` reports the
detected envelope in its `envelope` field.

`adf convert --from adf` does not accept `-e` (exit 2): the input is unwrapped automatically,
and the output is Markdown. With `--to csv` the input is unwrapped the same way and the output is
plain CSV text of one table (`--table N`); `-e` is not accepted there either. Warning pointers for
`--to csv` start at the document's position, as above.
