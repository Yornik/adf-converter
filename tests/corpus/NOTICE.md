# Third-party test corpora

The files in this directory are used **only by the test suite** (`tests/corpus.rs`). They are not
compiled into `adf`, not shipped in release archives, and not part of the published crate (see
`include` in `Cargo.toml`). They are vendored byte-for-byte; do not edit them. To update one,
download the new version, verify it as described below, replace the file, and update this notice.

## `commonmark-0.31.2.json`

- **What:** the 652 examples of the CommonMark Spec, version 0.31.2 (2024-01-28), as published
  by the CommonMark project.
- **Source:** <https://spec.commonmark.org/0.31.2/spec.json>
- **SHA-256:** `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`
- **Verified:** every example (`markdown`, `html`, `section`, `start_line`, `end_line`) matches the
  examples extracted from `spec.txt` at tag `0.31.2` of
  <https://github.com/commonmark/commonmark-spec> (sha256 of that `spec.txt`:
  `257c41ad946f7a1414a499aca402a1aa8fdac3678532266611348c1cf54f4b80`), using the extraction rules
  of the project's `test/spec_tests.py`.
- **Copyright:** © John MacFarlane.
- **Licence:** [Creative Commons Attribution-ShareAlike 4.0 International (CC BY-SA 4.0)](https://creativecommons.org/licenses/by-sa/4.0/).

## `gfm-0.29-spec.txt`

- **What:** the GitHub Flavored Markdown Spec, version 0.29-gfm (2019-04-06), in its source form.
  The tests extract its 672 examples (including the `table`, `strikethrough`, `autolink`,
  `tagfilter` and task-list examples).
- **Source:** `test/spec.txt` of <https://github.com/github/cmark-gfm> at tag `0.29.0.gfm.13`
  (commit `587a12bb54d95ac37241377e6ddc93ea0e45439b`):
  <https://raw.githubusercontent.com/github/cmark-gfm/587a12bb54d95ac37241377e6ddc93ea0e45439b/test/spec.txt>.
  The rendered version is <https://github.github.com/gfm/>.
- **SHA-256:** `7d8e5814befec287ac116786d81ff14e0adc9b13295b4494649e995408fd871c`
- **Copyright:** © John MacFarlane (CommonMark Spec), with the GitHub Flavored Markdown
  extensions by GitHub, Inc. (cmark-gfm's `COPYING`: "The CommonMark spec (test/spec.txt) is
  Copyright (C) 2014-15 John MacFarlane. Released under the Creative Commons CC-BY-SA 4.0
  license").
- **Licence:** [Creative Commons Attribution-ShareAlike 4.0 International (CC BY-SA 4.0)](https://creativecommons.org/licenses/by-sa/4.0/).

No changes were made to either file. The test harness converts the `→` characters in examples to
tabs at run time, as the upstream test runners do.
