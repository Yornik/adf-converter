# Test fixtures

Each fixture is a Markdown input plus files generated from it. The Markdown → ADF mapping they
test is defined in [`docs/syntax.md`](../../docs/syntax.md).

```
tests/fixtures/<area>/<case>.md          input (hand-written)
tests/fixtures/<area>/<case>.opts.json   optional, non-default options (hand-written)
tests/fixtures/<area>/<case>.adf.json    expected ADF document (blessed, never edited by hand)
tests/fixtures/<area>/<case>.diag.jsonl  expected warnings, if any (blessed, never edited by hand)
```

Files whose name is all upper case (`README.md`, `SOURCES.md`) are documentation, not fixtures.

## Which tests use them

| Test | What it checks for every fixture |
|---|---|
| `tests/golden.rs` | The output is valid ADF and equals `<case>.adf.json`; the warnings equal `<case>.diag.jsonl`. |
| `tests/roundtrip.rs` | `md_to_adf(adf_to_md(x)) == normalize(x)` for `x = md_to_adf(<case>.md)`, and the `plain` flavour works. |
| `tests/determinism.rs` | Two runs give identical bytes; CRLF and BOM variants give the same output. |

`tests/golden.rs` also checks that every fixture converts the same for both targets, and that
some fixtures still contain what they are about (`fixtures_still_test_what_they_say`), so that
blessing cannot silently record a fixture that lost its point.

"Valid ADF" means all three of: the vendored schema `schemas/adf/full.json` (draft-04, checked
directly with `jsonschema`), `adf_converter::validate_adf` for the fixture's target, and the
independent invariant checks in `tests/support/invariants.rs`.

## Rules

- **Every fixture converts successfully** (exit code 0; warnings are fine). Inputs that must fail
  (`E_RAW_ADF`, `--html error` with unknown HTML, `--strict`, limits) are CLI cases in
  `tests/cli/`.
- **Blessed files are never edited by hand.** Change the input or the converter, bless, and review
  the diff. Every change to a blessed file must be explained in the pull request.
- Goldens are always the bare document (`-e doc`); envelopes are covered by the CLI cases.
- No employer, customer or other private content, ever. Real-world-shaped documents are
  invented (see `real-world-synthetic/`).
- Name files in kebab-case after what they test. Regression fixtures are named after the issue
  they come from (`<repository>-<number>.md`) and cited in `regression/SOURCES.md`.
- A fixture that needs CRLF line endings or a BOM must be listed with `-text` in a
  `.gitattributes` file in its directory (see `commonmark/.gitattributes`).
- Check that the input tests what its name and text say. Many editors and AI write tools remove
  trailing spaces, and two of them are a hard break: check with `cat -et`. A fixture whose point
  is easy to lose like this gets a check in `fixtures_still_test_what_they_say`.

## Blessing

```sh
ADF_BLESS=1 cargo test --locked --test golden
git diff tests/fixtures      # review every change
```

Blessing writes `<case>.adf.json` (2-space indented JSON with a trailing newline, keys in output
order), writes `<case>.diag.jsonl` when there are warnings, and deletes a stale `<case>.diag.jsonl`
when there are none. It never blesses invalid ADF: such fixtures still fail. Without `ADF_BLESS=1`,
a missing expected file is a failure that names the fixture.

`<case>.diag.jsonl` holds one compact JSON object per warning, in the order the converter reports
them:

```json
{"code":"W_IMAGE_AS_LINK","line":1,"column":1}
```

`line` and `column` are 1-based (columns count characters). Warnings without a source position
have only `code` (or `code` and `pointer`). Messages are not compared; they may be reworded.

## Options files

`<case>.opts.json` sets non-default `MdToAdfOptions`. Every field is optional; unknown fields are
an error.

```json
{
  "target": "jira",
  "softbreak": "space",
  "html": "text",
  "images": "link",
  "extensions": "all"
}
```

| Field | Values (default first) | CLI flag |
|---|---|---|
| `target` | `"jira"`, `"confluence"` | `-t/--target` |
| `softbreak` | `"space"`, `"hardbreak"` | `--softbreak` |
| `html` | `"text"`, `"drop"`, `"error"` | `--html` |
| `images` | `"link"`, `"external"`, `"drop"` | `--images` |
| `extensions` | `"all"`, `"none"`, a comma-separated list such as `"alerts,details"`, or an array such as `["alerts", "details"]` | `--extensions` |

Extension names: `alerts`, `details`, `cells`, `raw-adf`, `mention`, `status`, `date`, `emoji`,
`card`.

## Areas

| Area | Contents |
|---|---|
| `commonmark/` | Every CommonMark block and inline construct, entities, escapes, Unicode, CRLF and BOM input. |
| `gfm/` | Tables (alignment, escaped pipes, ragged rows), task lists (mixed, nested), strikethrough, extended autolinks (including suppressed `https\://`), footnotes. |
| `extensions/` | Alerts (all five kinds, in containers), `<details>`, `::: cell` boxes, raw ```` ```adf ````, `adf:` links (valid and invalid), extensions disabled through options, the extension nodes for target `confluence`. |
| `html/` | Each HTML policy, each recognised inline tag, comments, unknown tags. |
| `lossy/` | At least one fixture per Markdown → ADF `W_` warning code (`W_NODE_NOT_IN_TARGET` is reserved and never emitted). `W_SIZE_LIMIT` and `W_EMPTY_DOCUMENT` come from the CLI's self-check, so their fixtures have no `.diag.jsonl`; `tests/cli.rs` converts them with the CLI. |
| `schema-gotchas/` | One fixture per Jira `400` cause and per hard ADF rule that Markdown can trigger. |
| `regression/` | Inputs that broke other converters; sources in `regression/SOURCES.md`. |
| `roundtrip/` | Inputs that are hard to write back as Markdown (escaping, marks at word edges, …). |
| `real-world-synthetic/` | Invented documents shaped like real Jira content, including the incident-report acceptance fixture. |
