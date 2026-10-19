@AGENTS.md

## Claude Code

- Run `scripts/check.sh` before reporting a task as done, and report what failed, if anything.
- After an intentional output change, bless (`ADF_BLESS=1 cargo test --locked --test golden`,
  `TRYCMD=overwrite cargo test --locked --test cli`), then review `git diff` of the blessed files
  and explain each change.
- Never edit blessed files (`tests/fixtures/**/*.adf.json`, `*.diag.jsonl`, trycmd expected
  output) by hand.
- `skills/markdown-to-adf/` is for consumers of `adf`. Don't copy it into `.claude/skills/`, and
  don't follow it when developing this repository.
- Personal overrides go in `CLAUDE.local.md` (git-ignored).
