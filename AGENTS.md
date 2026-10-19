# adf-converter

Rust CLI `adf` that converts Markdown to Atlassian Document Format (ADF) JSON for Jira and
Confluence, and ADF back to Markdown. Pure converter: no network I/O, no credentials. Independent
project, not affiliated with Atlassian.

Stack: Rust 1.99 (pinned in `rust-toolchain.toml`), edition 2024, pulldown-cmark 0.13 (+ linkify
for bare URLs), serde/serde_json (`preserve_order`), clap 4 (+ clap_complete), jsonschema `=0.59`
with no default features, and the vendored ADF JSON schema `@atlaskit/adf-schema` 57.7.18.

## Commands

```sh
scripts/check.sh                                   # fmt check, clippy, tests, rustdoc, cargo-deny (if installed)
cargo build --locked
cargo test --locked --all-features
ADF_BLESS=1 cargo test --locked --test golden      # rewrite fixture goldens; then review `git diff tests/`
TRYCMD=overwrite cargo test --locked --test cli    # rewrite CLI case output; then review `git diff tests/cli/`
cargo run --locked -- convert tests/fixtures/<area>/<case>.md -e jira-comment
cargo fmt --all
cargo clippy --locked --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
cargo deny --locked check
uvx --from skills-ref==0.1.1 agentskills validate skills/markdown-to-adf   # the skills-ref package's CLI is `agentskills`
uvx zizmor .github/workflows
```

## Layout

Markdown → ADF:
`src/md/` (prepass: BOM/CRLF, front matter, `::: cell` → pulldown-cmark events with byte offsets
→ tree builder and repair passes) → strict model `src/adf/model.rs` → `src/adf/validate.rs`
(embedded schema + `src/adf/targets.rs` + invariants) → `src/adf/envelope.rs` → output in
`src/cli.rs`.

ADF → Markdown: `src/adf/envelope.rs` (unwrap) → `src/adf/reader.rs` (lenient) →
`src/adf/to_md.rs`.

- `src/diagnostics.rs`: the `Code` enum, the single source of every `E_`/`W_` code.
- `src/options.rs`, `src/limits.rs`: options mirroring the CLI flags; size and depth limits.
- `src/catalog.rs`: features and examples behind `adf capabilities` and `adf examples`.
- `src/adf/ids.rs`: deterministic task-list `localId`s.
- `docs/cli-contract.md`, `docs/syntax.md`: normative. Code that disagrees with them is a bug.
- `schemas/adf/full.json`: vendored, unmodified (see `PROVENANCE`). `schemas/*.v1.json`: JSON
  Schemas of this tool's own JSON output.
- `tests/fixtures/<area>/<case>.md` with blessed `<case>.adf.json`, optional `<case>.diag.jsonl`
  and `<case>.opts.json`; harnesses in `tests/*.rs`, helpers in `tests/support/mod.rs`, CLI cases
  in `tests/cli/`, fuzz targets in `fuzz/` (separate workspace).
- `skills/markdown-to-adf/`: the Agent Skill for people and agents who *use* `adf`. It is not for
  developing this repository.

## Contract invariants (ask before changing)

The CLI is a public contract, versioned with semver. Ask the maintainer before changing any of:
command and flag names, defaults, exit codes, `E_`/`W_` codes and their meaning, JSON shapes
(`schema_version`, diagnostics, `validate --json`, `capabilities --json`, `version --json`),
envelopes and input detection, stdout/stderr routing, byte-for-byte determinism, the environment
variables read (`ADF_ERROR_FORMAT`, `NO_COLOR`), and the limits.

A change you were asked to make must update, in the same PR:
- `docs/cli-contract.md` (and `docs/syntax.md` for mapping changes);
- `schemas/*.v1.json` if a JSON shape changes;
- `skills/markdown-to-adf/references/` if users see the difference;
- the commit type in the PR title: `feat:` for additions, `feat!:` (or another type with `!`) for
  breaking changes. release-please builds the changelog and the version from it.

Add new codes at the end of their group in `src/diagnostics.rs`. Never rename or reuse a code.

## Adding a Markdown feature

1. Write the fixture first: `tests/fixtures/<area>/<case>.md`, plus `<case>.opts.json` for
   non-default options. Fixtures must convert with exit 0; error cases belong in `tests/cli/`.
2. Implement it, building output with the typed model in `src/adf/model.rs`.
3. Bless with `ADF_BLESS=1 cargo test --locked --test golden` and review every changed file.
4. Document it in `docs/syntax.md` and `skills/markdown-to-adf/references/syntax.md`.
5. Add it to `src/catalog.rs`, and to `src/adf/targets.rs` if it emits a new node or mark.
6. Give every lossy path a `W_` code, documented in `docs/cli-contract.md` (with its anchor) and in
   `skills/markdown-to-adf/references/troubleshooting.md`, and a fixture with a `.diag.jsonl`.
7. Make ADF → Markdown invert it (or pass it through as a ```` ```adf ```` block) and keep the
   round-trip test green.

## Testing rules

- Never hand-edit blessed files (`*.adf.json`, `*.diag.jsonl`, trycmd output). Bless, then read
  the diff.
- Every ADF the tests produce is validated against `schemas/adf/full.json` and the Jira rules.
- Output is deterministic: the determinism test must pass, including CRLF vs LF and BOM input.
- Never delete, skip, `#[ignore]` or loosen a failing test without asking. Fix the code, or
  report why the test is wrong.
- Tests never touch the network or a real Jira/Confluence site. Fixtures contain only invented
  content.
- Commit `proptest-regressions/` files when proptest creates them.

## Code style

- NEVER use `unwrap`, `expect`, indexing/slicing, `panic!`, `todo!`, `unimplemented!` or `dbg!` in
  `src/`; clippy denies them. Tests may.
- ALWAYS build ADF output from the typed model in `src/adf/model.rs`. NEVER use `json!` or
  hand-assembled `serde_json::Value` for output.
- ALWAYS check the depth limit (`limits.max_depth` in the options) in every recursive walk, or
  walk iteratively. Untrusted input must never overflow the stack.
- NEVER write to stdout or stderr outside the CLI output module; `print!`/`eprint!` are denied.
  stdout carries only the result.
- Report errors as `Diagnostic`/`Diagnostics` with a `Code`, a one-sentence message, a location
  and an optional hint. Don't add `thiserror` error types or `Box<dyn Error>` to the API.
- NEVER iterate a `HashMap` or `HashSet` on an output path. Use `Vec` or `BTreeMap`.
- Write doc comments on public items. Keep functions small. Keep clippy pedantic clean; don't add
  blanket `allow`s.
- Never assume a warning or failing test was there before you; fix what you find.

## Security

- All input is untrusted: agents generate it. Keep the limits (4 MiB input, depth 64) and return
  `E_INPUT_TOO_LARGE`/`E_INPUT_TOO_DEEP` instead of crashing.
- Links and smart links must pass `Href::parse` (http, https, ftp, ftps, mailto, tel, `/path`,
  `#anchor`). Everything else becomes plain text with `W_UNSAFE_URL`.
- Strip control characters (`W_CONTROL_CHAR`). Never echo large chunks of input in messages.
- No network calls, no subprocesses, no config files, and no environment variables beyond those in
  `docs/cli-contract.md`. `unsafe` is forbidden. `deny.toml` bans network and TLS crates.

## Git & PRs

- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `perf:`, `build:`,
  `ci:`, `chore:`). release-please turns them into the changelog and the next version.
- PRs are squash-merged and the PR title becomes the commit message, so the title must be a
  Conventional Commit too (the `conventional-title` check enforces it).
- Never push to `main`. Work on a branch; keep PRs small and focused.
- Fill in the PR template. Disclose AI assistance in the PR description.

## Code Review Rules

Treat these as P0/P1 (block the merge):

- a contract break: a changed flag, default, exit code, diagnostic code, JSON shape or envelope
  without the doc, schema and commit-type updates above;
- non-deterministic output (time, randomness, hash-map order, platform differences);
- ADF that is schema-invalid or that the target product rejects (see `docs/syntax.md` rules);
- a panic, hang, unbounded allocation or stack overflow on any input;
- a lossy conversion without a `W_` code;
- a behaviour change without a fixture;
- an unjustified new dependency, or one that adds network access or C code;
- a workflow permission widened, `pull_request_target` added, or an action not pinned to a full
  commit SHA;
- employer, customer or other private content in fixtures, docs or commits.

## Boundaries

- **Always:** run `scripts/check.sh` before saying you are done; bless and review diffs after an
  intentional output change; keep `docs/` in sync with behaviour.
- **Ask first:** new dependencies or `Cargo.toml` changes; contract changes; edits under
  `.github/`; updating the vendored schema in `schemas/adf/`; deleting or loosening tests.
- **Never:** commit secrets or tokens; hand-edit blessed files or `schemas/adf/full.json`; add
  network I/O; call a real Jira or Confluence site from tests or CI; use employer or customer
  content anywhere in the repository; push to `main`.
