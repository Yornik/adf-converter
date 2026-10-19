# adf-converter

Markdown ⇄ Atlassian Document Format (ADF) converter for Jira and Confluence — for humans and AI
agents.

[![CI](https://github.com/Yornik/adf-converter/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Yornik/adf-converter/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/Yornik/adf-converter?sort=semver)](https://github.com/Yornik/adf-converter/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

> [!IMPORTANT]
> adf-converter is an independent, free-time hobby project. It is not affiliated with, endorsed
> by, sponsored by, or supported by Atlassian. Atlassian, Jira and Confluence are trademarks of
> Atlassian. Report problems with Jira or Confluence themselves to
> [Atlassian Support](https://support.atlassian.com/); report problems with this tool
> [here](https://github.com/Yornik/adf-converter/issues).

The command is `adf`. It is a single self-contained binary that works offline: it never makes
network requests and never sees your credentials. You post its output with `curl`, acli or your
own code.

## Why

- **Jira Cloud and Confluence Cloud store rich text as ADF JSON.** The Jira REST API v3 takes
  comments, descriptions and rich-text fields only as ADF, and Confluence's v2 API accepts pages
  and comments as ADF (`atlas_doc_format`). Plain text loses headings, lists, tables and code
  blocks, Markdown sent as plain text shows up as literal asterisks and pipes, and wiki markup
  (`h2.`, `{code}`) only works with the older Jira v2 API.
- **Hand-made ADF is easy to get wrong, and Jira's error messages don't say where.** An empty
  text node, bold on inline code, a heading inside a list item or a task item without a
  `localId` all get a generic `400` error, often just `INVALID_INPUT`. `adf` checks every
  document against the vendored ADF JSON schema *and* the rules Jira enforces beyond it, repairs
  what it can, and reports anything lossy with a code and a `line:column`.
- **Atlassian has no official ADF → Markdown converter.** Reading a ticket into a diff, a review
  or an LLM prompt means wading through deeply nested JSON. `adf convert --from adf` turns
  issues, comments and Confluence pages back into Markdown.

## Install

### Release archive (recommended)

Every [release](https://github.com/Yornik/adf-converter/releases) has one zip per platform plus a
`SHA256SUMS` file:

| Platform | Target |
|---|---|
| macOS, Apple silicon | `aarch64-apple-darwin` |
| macOS, Intel | `x86_64-apple-darwin` |
| Linux x86_64 (static, any distribution) | `x86_64-unknown-linux-musl` |
| Linux ARM64 (static, any distribution) | `aarch64-unknown-linux-musl` |
| Windows x64 | `x86_64-pc-windows-msvc` |
| Windows ARM64 | `aarch64-pc-windows-msvc` |

Each zip `adf-converter-vX.Y.Z-<target>.zip` holds one directory with the `adf` binary (`adf.exe`
on Windows), this README, the licences, `docs/`, the agent skill in `skills/markdown-to-adf/` and
shell completions in `completions/`.

macOS and Linux, using the [GitHub CLI](https://cli.github.com/) in an empty directory:

```sh
target=aarch64-apple-darwin        # pick yours from the table above
gh release download -R Yornik/adf-converter -p "adf-converter-v*-$target.zip" -p SHA256SUMS
shasum -a 256 -c SHA256SUMS --ignore-missing     # Linux: sha256sum -c SHA256SUMS --ignore-missing
unzip -q adf-converter-v*-"$target".zip
mkdir -p ~/.local/bin && cp adf-converter-v*-"$target"/adf ~/.local/bin/
adf --version                      # ~/.local/bin must be on your PATH
```

Windows (PowerShell):

```powershell
gh release download -R Yornik/adf-converter -p "adf-converter-v*-x86_64-pc-windows-msvc.zip" -p SHA256SUMS
Get-FileHash adf-converter-v*-x86_64-pc-windows-msvc.zip -Algorithm SHA256   # compare with SHA256SUMS
Expand-Archive adf-converter-v*-x86_64-pc-windows-msvc.zip -DestinationPath .
# Move adf.exe from the extracted directory into a folder on your PATH, then:
adf --version
```

- **macOS quarantine.** The binaries are not signed or notarized. If you downloaded the zip with a
  browser, Gatekeeper blocks the binary. After checking the checksum, remove the quarantine flag
  with `xattr -d com.apple.quarantine ~/.local/bin/adf`. Files downloaded with `gh` or `curl` are
  not quarantined.
- **Windows SmartScreen.** `adf.exe` is not code-signed, so Windows may say "Windows protected your
  PC" for a browser download. After checking the checksum, choose **More info → Run anyway**, or
  run `Unblock-File .\adf.exe`.

### From source

With Rust 1.99 or later:

```sh
cargo install --git https://github.com/Yornik/adf-converter --locked
```

This installs `adf` into `~/.cargo/bin`. Add `--tag vX.Y.Z` to build a specific release.

## Quick start for humans

```sh
# Markdown → ADF. Pretty JSON on a terminal, compact JSON in pipes and files.
adf convert notes.md

# A ready-to-post Jira comment body, written atomically (nothing is written if it fails).
adf convert notes.md -e jira-comment -o comment.json

# From the clipboard (macOS; on Linux use wl-paste/wl-copy or xclip).
pbpaste | adf convert - -e jira-comment | pbcopy

# ADF → Markdown: a bare document, a Jira issue or comment, or a Confluence page body.
adf convert --from adf issue.json
adf convert --from adf issue.json --md-flavor plain    # compact, for reading and LLMs

# Check before posting.
adf validate --from md notes.md     # lists every lossy step as a warning
adf validate comment.json           # ADF: schema + Jira rules, exit 4 if invalid
adf explain W_IMAGE_AS_LINK         # what a diagnostic code means
```

Warnings go to stderr, the result to stdout. `--strict` turns every warning into a failure (exit
5) and writes nothing. `adf --help` and `adf <command> --help` list every option; the full
interface is in [docs/cli-contract.md](docs/cli-contract.md).

## For AI agents

`adf` is built to be called by agents. Its interface for scripts and agents,
[docs/cli-contract.md](docs/cli-contract.md), follows semantic versioning: new flags, codes and
JSON fields are minor changes; renames, removals and behaviour changes are breaking.

- **Use the skill.** [`skills/markdown-to-adf/`](skills/markdown-to-adf/SKILL.md) is an
  [Agent Skill](https://agentskills.io/) that teaches an agent to write Markdown, pre-flight it,
  convert it and post it safely. Copy the directory into your agent's skills directory (for
  example `~/.claude/skills/` for Claude Code). It is also included in every release zip.
- **Discover, don't guess.** `adf capabilities --json` lists the supported features, extensions,
  envelopes, exit codes, diagnostic codes and the main limits (all limits are in
  [docs/cli-contract.md](docs/cli-contract.md#limits)). `adf examples FEATURE` prints a
  Markdown → ADF pair, and `adf schema diagnostic` prints the JSON Schema of a diagnostic line.
- **Machine-readable diagnostics.** When stderr is not a terminal, every warning and error is
  one JSON object per line, with `code`, `severity`, `message`, `location` (`line`/`column`, or a
  JSON Pointer for ADF input) and an optional `hint`. Force it with `--error-format json`.
- **Exit codes mean something.** 0 is success (warnings allowed). 3, 4 and 5 mean "fix the input",
  so retrying the same command won't help. See [Exit codes](#exit-codes).
- **Never hand-write ADF.** Write Markdown and let `adf` produce the JSON; use a ```` ```adf ````
  block only for the rare node Markdown cannot express.
- **Write to a file, then post.** In `adf convert … | curl --data-binary @- …`, curl still sends a
  request (with an empty body) if `adf` fails. Use `-o FILE &&` instead, as in the recipes below.

## Posting to Jira and Confluence

`adf` only converts. These recipes use `curl` with an
[Atlassian API token](https://id.atlassian.com/manage-profile/security/api-tokens) in environment
variables that you set (`adf` itself does not read them):

```sh
export JIRA_BASE_URL=https://your-site.atlassian.net
export JIRA_EMAIL=you@example.com
read -rs JIRA_API_TOKEN && export JIRA_API_TOKEN    # paste the token; it is not echoed

# Add a comment to PROJ-123.
adf convert comment.md -e jira-comment -o comment.json &&
  curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @comment.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123/comment"

# Replace the description of PROJ-123.
adf convert description.md -e jira-description -o description.json &&
  curl --fail-with-body -sS -X PUT -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
    -H 'Content-Type: application/json' --data-binary @description.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123"

# Read the description back as Markdown.
curl --fail-with-body -sS -u "$JIRA_EMAIL:$JIRA_API_TOKEN" -o issue.json \
    "$JIRA_BASE_URL/rest/api/3/issue/PROJ-123?fields=description" &&
  adf convert --from adf issue.json --md-flavor plain
```

[docs/posting.md](docs/posting.md) has more: updating comments, custom fields, new issues,
Confluence pages and footer comments, acli (and its `comment create` caveat), and scoped tokens.
[docs/envelopes.md](docs/envelopes.md) describes every envelope.

## Supported syntax

CommonMark and GitHub Flavored Markdown, plus a few extensions for ADF-only nodes. The normative
mapping, including every repair rule, is [docs/syntax.md](docs/syntax.md).

| Markdown | ADF |
|---|---|
| `#` … `######` headings, paragraphs, hard breaks | `heading`, `paragraph`, `hardBreak` |
| `*em*`, `**strong**`, `~~strike~~`, `` `code` `` | `em`, `strong`, `strike`, `code` marks |
| `[text](url)`, `<url>`, bare `https://…` and `www.…` | `link` mark (`http`, `https`, `ftp`, `ftps`, `mailto`, `tel`, `/path`, `#anchor` only) |
| `-` / `*` / `+` and `1.` lists, nested | `bulletList`, `orderedList` (`order` always set), `listItem` |
| `- [ ]` / `- [x]` | `taskList`, `taskItem` with deterministic `localId`s |
| Fenced and indented code | `codeBlock` with `language` |
| `>` quotes, `---` rules | `blockquote`, `rule` |
| GFM tables, with column alignment | `table` with `tableHeader` cells and `alignment` marks |
| `<u>`, `<sub>`, `<sup>`, `<br>` | `underline`, `subsup`, `hardBreak` |
| `<span style="color: #rrggbb">`, `<span style="background-color: #rrggbb">` | `textColor`, `backgroundColor` |
| `> [!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]` | `panel` (info, success, note, warning, error) |
| `<details><summary>Title</summary>` … `</details>` | `expand` / `nestedExpand` |
| `::: cell` … `:::` | a 1×1 `table` used as a box |
| ```` ```adf ```` with ADF JSON | spliced in after validation |
| `[@Jane Doe](adf:mention/<account-id>)` | `mention` |
| `[DONE](adf:status/green)` | `status` |
| `[2026-10-19](adf:date)` | `date` |
| `[😄](adf:emoji/:smile:)` | `emoji` |
| `[https://…](adf:card)` | `inlineCard` (smart link) |
| `![alt](url)` | a link (`W_IMAGE_AS_LINK`); `--images external` embeds by URL |
| `[^1]` footnotes | superscript numbers and a list at the end (`W_FOOTNOTE_FLATTENED`) |

All extensions are on by default; `--extensions none` or `--extensions alerts,details` limits
them. ADF → Markdown writes the same syntax, so for these constructs, converting back with
`--images external` gives the same document (with the default `--images link`, embedded external
images become links). Nodes Markdown cannot express become ```` ```adf ```` blocks
(`W_ADF_PASSTHROUGH`); attributes and marks it cannot express, such as column widths or
Confluence inline-comment marks, are dropped (`W_ATTR_DROPPED`).

## Exit codes

| Code | Meaning | Retry? |
|---|---|---|
| 0 | Success. Warnings may have been printed. | |
| 1 | Internal error (`E_INTERNAL`): a bug in `adf`. Please report it. | no |
| 2 | Usage error (`E_USAGE`). | fix the command |
| 3 | Input or I/O problem (`E_INPUT_*`, `E_RAW_ADF`, `E_OUTPUT_WRITE`). | fix the input |
| 4 | Validation failed (`E_SCHEMA`, `E_TARGET`, `E_INVARIANT`); `validate` only. | fix the document |
| 5 | Lossy conversion refused (`E_LOSSY`): `--strict`, or `--html error`. | fix the input or drop `--strict` |

`adf explain exit-codes` prints this table, and `adf explain codes` lists every diagnostic code.

## Verifying downloads

Every release has a `SHA256SUMS` file and a build provenance attestation for each zip, which
[`gh attestation verify`](https://cli.github.com/manual/gh_attestation_verify) checks.

```sh
# Checksums (run in the directory with the zip and SHA256SUMS).
shasum -a 256 -c SHA256SUMS --ignore-missing      # macOS
sha256sum -c SHA256SUMS --ignore-missing          # Linux

# Provenance: the zip was built by this repository's GitHub Actions.
gh attestation verify adf-converter-vX.Y.Z-<target>.zip --repo Yornik/adf-converter
```

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow, commit conventions and fixture rules,
and [AGENTS.md](AGENTS.md) for the rules that coding agents (and humans) follow in this
repository. `scripts/check.sh` runs the main CI checks (formatting, clippy, tests, rustdoc,
cargo-deny) locally.

## Security

`adf` treats all input as untrusted, enforces size and nesting limits, and makes no network
requests. To report a vulnerability, see [SECURITY.md](SECURITY.md); please don't open a public
issue.

## License

[MIT](LICENSE). The vendored ADF JSON schema (`schemas/adf/full.json`, from `@atlaskit/adf-schema`)
is licensed under Apache-2.0. Third-party licences are listed in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Legal

adf-converter is an independent, free-time hobby project maintained by an individual. It is not
affiliated with, endorsed by, sponsored by, or supported by Atlassian. This project is not
affiliated with the author's employer. Please don't contact Atlassian Support about this tool;
open an issue in this repository instead.

Atlassian, Jira, and Confluence are trademarks or registered trademarks of Atlassian Pty Ltd
and/or its affiliates in the United States and other countries. They are used here only to
describe what this tool is compatible with. All other trademarks belong to their respective
owners.

The Atlassian Document Format (ADF) is publicly documented by Atlassian at
<https://developer.atlassian.com/cloud/jira/platform/apis/document/structure/>. This project is
an independent implementation written from that public documentation and contains no Atlassian
logos or other brand assets. `schemas/adf/full.json` is copied unmodified from
`@atlaskit/adf-schema` 57.7.18 (Copyright 2019 Atlassian Pty Ltd) under the Apache License 2.0;
see [`schemas/adf/LICENSE`](schemas/adf/LICENSE) and
[`schemas/adf/PROVENANCE`](schemas/adf/PROVENANCE). Release archives carry the same licence text
in `THIRD_PARTY_NOTICES.md`.

This software is provided "as is" under the MIT License, without warranty of any kind. Check the
generated output before posting it to a Jira or Confluence site.
