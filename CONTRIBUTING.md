# Contributing

Thanks for helping. adf-converter is a free-time project, so small, focused pull requests with a
test are the easiest to review and merge.

Before you start:

- **Bugs in Jira or Confluence** (rendering, API errors that `adf validate` doesn't explain) are
  Atlassian's to fix; report them to [Atlassian Support](https://support.atlassian.com/). This
  project is not affiliated with Atlassian.
- **Security problems:** follow [SECURITY.md](SECURITY.md). Don't open a public issue.
- **Bigger changes** (new syntax, new flags, anything in
  [docs/cli-contract.md](docs/cli-contract.md)): open an issue first, so we can agree on the
  design before you write code.
- Coding agents: [AGENTS.md](AGENTS.md) has the same rules in compact form.

## Never commit private content

Fixtures, examples, issues and commit messages must contain only content you invented or that is
public and freely licensed. **Never** use real Jira or Confluence content from an employer, a
customer or anyone else's site, not even "anonymised". If you need a realistic document, write a
synthetic one about a fictional service with the same *structure*.

## Development setup

1. Install Rust with [rustup](https://rustup.rs/). The repository pins the toolchain in
   `rust-toolchain.toml` (Rust 1.99.1 with clippy and rustfmt); rustup installs it on the first
   `cargo` command.
2. Clone the repository and run the checks:

   ```sh
   git clone https://github.com/Yornik/adf-converter.git
   cd adf-converter
   scripts/check.sh
   ```

Optional tools:

- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) (`cargo install --locked cargo-deny`)
  for the licence and advisory checks; `scripts/check.sh` runs it when it is installed.
- [uv](https://docs.astral.sh/uv/) to run the skill validator and the workflow linter:
  `uvx --from skills-ref==0.1.1 agentskills validate skills/markdown-to-adf` (the `skills-ref`
  package installs a command named `agentskills`) and `uvx zizmor@1.31.1 .github/workflows` (the
  version CI uses).
- [jq](https://jqlang.org/) and uv for `scripts/contract.sh`, and
  [shellcheck](https://www.shellcheck.net/) and [actionlint](https://github.com/rhysd/actionlint)
  (CI uses 1.7.12) for the other CI checks below.
- A nightly toolchain and `cargo-fuzz` for the fuzz targets; see `fuzz/README.md`.

`scripts/check.sh` runs the core checks of CI:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
cargo deny --locked check          # only if cargo-deny is installed
```

CI also tests on Linux, macOS and Windows, and runs these jobs, which `scripts/check.sh` does not
(all of them feed the required `ci-ok` check):

```sh
# contract: converts every fixture with the release binary and checks the results with an
# independent JSON Schema validator
cargo build --locked --release && scripts/contract.sh target/release/adf
# workflows: lints the scripts and workflows
shellcheck --external-sources scripts/*.sh
actionlint
uvx zizmor@1.31.1 .github/workflows
```

## Workflow

1. Fork the repository (or create a branch if you have write access). Never push to `main`.
2. For a behaviour change, write the fixture or test first (see below), then change the code.
3. Run `scripts/check.sh` until it passes.
4. Update the docs if behaviour changed: [docs/syntax.md](docs/syntax.md) for Markdown ⇄ ADF
   mapping, [docs/cli-contract.md](docs/cli-contract.md) for commands, flags, codes and JSON, and
   the skill references in `skills/markdown-to-adf/references/`.
5. Open a pull request and fill in the template.

## Commit messages and PR titles

This project uses [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/).
Pull requests are **squash-merged**, and the PR title becomes the commit message on `main`, so
the PR title is what counts (a CI check enforces the format). Commits inside your branch can be
anything, but conventional messages make review easier. One exception: if the PR has a single
commit, GitHub may use that commit's message instead, so the check requires its first line to
equal the PR title.

[release-please](https://github.com/googleapis/release-please) reads these messages on `main`. It
keeps a release PR open that bumps the version and updates `CHANGELOG.md`; merging that PR tags
the release and publishes the binaries. release-please creates `CHANGELOG.md` with the first
release; don't edit it by hand.

| Type | Example | Effect on the next release |
|---|---|---|
| `fix:` | `fix: keep the link mark on inline code in table cells` | patch (0.4.1 → 0.4.2), listed under Bug Fixes |
| `feat:` | `feat: add --images drop` | minor (0.4.1 → 0.5.0), listed under Features |
| `perf:` | `perf: avoid re-parsing raw adf blocks` | patch, listed under Performance Improvements |
| `revert:` | `revert: feat: add --images drop` | patch, listed under Reverts |
| `docs:` | `docs: fix the Confluence recipe` | patch, listed under Documentation (`docs/` and the skill ship in every release zip) |
| `deps:` | `deps: update rust crate serde to v1.0.230` | patch, listed under Dependencies (Renovate's type for crates in the shipped binary and for `Cargo.lock` maintenance) |
| `feat!:`, `fix!:` (any type with `!`) | `feat!: rename the jira-field envelope` | major (1.4.0 → 2.0.0), listed under Breaking Changes |
| `test:`, `refactor:`, `build:`, `ci:`, `chore:`, `style:` | `test: cover tables in box cells` | no release on their own, not in the changelog |

- Use the imperative mood and lower case after the colon: `fix: reject …`, not `Fixed …`.
- A scope is optional: `feat(md): …`, `fix(validate): …`.
- Mark a breaking change with `!` in the **PR title**, and explain it in the PR description. The
  squash commit keeps only the title, so a `BREAKING CHANGE:` footer in the description is not
  seen by release-please.
- While the version is below 1.0.0, a breaking change bumps the minor version if
  `release-please-config.json` sets `bump-minor-pre-major`, and the major version otherwise.
- Anything that changes the [CLI contract](docs/cli-contract.md) in an incompatible way is
  breaking, including a renamed diagnostic code or a changed JSON field.

## Tests and fixtures

Most behaviour is tested with **golden fixtures** in `tests/fixtures/<area>/`:

| File | Who writes it | Content |
|---|---|---|
| `<case>.md` | you | the Markdown input |
| `<case>.opts.json` | you, optional | non-default options, e.g. `{"softbreak":"hardbreak"}` (format in `tests/fixtures/README.md`) |
| `<case>.adf.json` | **blessed**, never by hand | the expected ADF document |
| `<case>.diag.jsonl` | **blessed**, never by hand | the expected warnings, one JSON object per line |

To add or change a fixture:

```sh
$EDITOR tests/fixtures/gfm/table-escaped-pipe.md
ADF_BLESS=1 cargo test --locked --test golden     # writes .adf.json and .diag.jsonl
git diff tests/fixtures/                          # read every change
cargo test --locked --all-features                # everything still passes
```

- Blessing rewrites the expected files from the current code. Review the diff as carefully as
  code: a wrong golden hides a bug. Explain changed goldens in the PR description.
- A fixture must convert successfully (exit 0) with the default envelope. Error cases and
  envelope cases are CLI tests in `tests/cli/`; bless those with
  `TRYCMD=overwrite cargo test --locked --test cli`.
- Every lossy conversion needs a `W_` code and a fixture whose `.diag.jsonl` shows it.
- Every ADF document the tests produce is validated against the vendored schema and Jira's
  rules, and must survive Markdown → ADF → Markdown → ADF unchanged.
- Don't delete, skip or loosen a failing test to make CI pass. If you think a test is wrong, say
  so in the PR.
- Tests never use the network.

## AI-assisted contributions

AI-assisted pull requests are welcome. Say so in the PR description (the template has a
checkbox), and make sure you understand and have reviewed every line you submit. Coding agents
read [AGENTS.md](AGENTS.md).

## Licence

By contributing, you agree that your contribution is licensed under the [MIT License](LICENSE),
like the rest of the project.
