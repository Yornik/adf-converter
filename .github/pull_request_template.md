<!--
PR title: a Conventional Commit, for example "fix: keep the link mark on inline code".
The title becomes the squash commit on main, and release-please builds the changelog and the next
version from it. Mark breaking changes with "!", for example "feat!: rename the jira-field envelope".
-->

## Summary

<!-- What changes and why. Link the issue, for example "Closes #123". -->

## Checklist

- [ ] The PR title is a Conventional Commit (`feat:`, `fix:`, `docs:`, …; `!` for a breaking change).
- [ ] `scripts/check.sh` passes locally.
- [ ] Behaviour changes come with fixtures or tests. Blessed files were regenerated with
      `ADF_BLESS=1` / `TRYCMD=overwrite` (never edited by hand), and I reviewed their diff.
- [ ] If behaviour changed: `docs/syntax.md`, `docs/cli-contract.md` and
      `skills/markdown-to-adf/references/` are updated.
- [ ] No private content: no employer, customer or other real Jira/Confluence data in fixtures,
      docs, screenshots or commit messages.
- [ ] AI assistance is disclosed below.

## AI assistance

<!-- Say whether AI tools helped and how, for example "Claude Code drafted the parser change; I
reviewed and tested it", or write "None". -->
