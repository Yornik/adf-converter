# Regression fixture sources

Each fixture in this directory is a minimal Markdown input that broke another Markdown → ADF
tool, or that the issue identified as a cause of Jira `400 INVALID_INPUT` errors. Fixture names
follow the issue: `<repository>-<number>.md`. Comments inside fixtures would be stripped by the
converter, so the sources are listed here instead.

Inputs were reduced to the construct that failed; they contain no content from the issues beyond
short example snippets.

| Fixture | Issue | What went wrong elsewhere |
|---|---|---|
| `marklassian-10.md` | <https://github.com/jamsinclair/marklassian/issues/10> | The Confluence v2 API rejected pages with task lists (target `confluence`). The issue (still open) suspects `taskList` itself, but Confluence accepts it. The likely cause is that marklassian puts nested lists, and the paragraphs of loose items, inside `taskItem`, which allows only inline content. Here a nested task list becomes a sibling task list, a plain sub-list is moved after the task list, and further paragraphs are joined with hard breaks. |
| `marklassian-11.md` | <https://github.com/jamsinclair/marklassian/issues/11> | Random `localId`s made the output non-reproducible; ours are derived from the input. |
| `marklassian-12.md` | <https://github.com/jamsinclair/marklassian/issues/12> (fix: [#13](https://github.com/jamsinclair/marklassian/pull/13)) | A soft line break before emphasis was dropped, gluing words together. |
| `extended-markdown-adf-parser-5.md` | <https://github.com/JeromeErasmus/extended-markdown-adf-parser/issues/5> | Inline code inside bold produced `code` + `strong` marks (invalid ADF). |
| `extended-markdown-adf-parser-7.md` | <https://github.com/JeromeErasmus/extended-markdown-adf-parser/issues/7> (fix: [#9](https://github.com/JeromeErasmus/extended-markdown-adf-parser/pull/9)) | Table cells held text nodes directly instead of paragraphs; Jira returned 400. |
| `extended-markdown-adf-parser-8.md` | <https://github.com/JeromeErasmus/extended-markdown-adf-parser/issues/8> (fix: [#10](https://github.com/JeromeErasmus/extended-markdown-adf-parser/pull/10)) | Checkbox state was lost; task lists became bullet lists. |
| `md-to-adf-22.md` | <https://github.com/b-yond-infinite-network/md-to-adf/issues/22> | An ordered list after a paragraph line was split into one list per item. |
| `md-to-adf-28.md` | <https://github.com/b-yond-infinite-network/md-to-adf/issues/28> | Only one link per paragraph was recognised. |
| `jira-cli-407.md` | <https://github.com/ankitpokhrel/jira-cli/issues/407> | `@` mentions were posted as plain text. |
| `jira-cli-568.md` | <https://github.com/ankitpokhrel/jira-cli/issues/568> | Emphasis inside list items and nested lists was escaped or mangled. |
| `jira-cli-716.md` | <https://github.com/ankitpokhrel/jira-cli/issues/716> | A heading followed by a fenced code block rendered badly. |
| `jira-cli-843.md` | <https://github.com/ankitpokhrel/jira-cli/issues/843> | Dashes, parentheses and underscores in code blocks were shown with backslashes. |
| `jira-cli-941.md` | <https://github.com/ankitpokhrel/jira-cli/issues/941> | Triple underscores in link text became emphasis. |
| `jira-cli-974.md` | <https://github.com/ankitpokhrel/jira-cli/issues/974> | A code block holding a URL with `{{…}}` parameters, inside a quote, truncated the content. |
| `jira-cli-1005.md` | <https://github.com/ankitpokhrel/jira-cli/issues/1005> | Editing a description dropped embedded media (and Jira then deleted the attachment). Here the media node is carried by a raw ```` ```adf ```` block and must survive the round trip. |
| `atlassian-mcp-server-136.md` | <https://github.com/atlassian/atlassian-mcp-server/issues/136> | Mentions were downgraded to plain text. |
| `atlassian-mcp-server-229.md` | <https://github.com/atlassian/atlassian-mcp-server/issues/229> | Markdown checkboxes were written as literal `[ ]` text. |
| `aide-11.md` | <https://github.com/rlcurrall/aide/issues/11> | Inline content in table cells, `title: null` on links and images, `language: null` on untagged fences (all 400), formatting not applied inside strong, ordered lists renumbered from 1. |
| `mmatczuk-jira-mcp-32.md` | <https://github.com/mmatczuk/jira-mcp/issues/32> | A blockquote inside a list item was emitted as a `listItem` child (400). |
| `owenoclee-jt-15.md` | <https://github.com/owenoclee/jt/pull/15> | Code spans with formatting, and headings, rules and quotes inside quotes and list items, were rejected by Jira. |
| `orekgames-track-cli-222.md` | <https://github.com/OrekGames/track-cli/issues/222> | Nested bullet lists produced a `listItem` starting with a list, and the parent text leaked into the child (400). |
| `rhecosystemappeng-sdlc-plugins-206.md` | <https://github.com/RHEcosystemAppEng/sdlc-plugins/pull/206> | Task lists without `localId`s were rejected (400). |
| `tingyiy-atlassian-mcp-server-5.md` | <https://github.com/tingyiy/atlassian-mcp-server/pull/5> | Bold- or italic-wrapped inline code made every write fail (400); `code` + `link` is fine. |
| `zious11-jira-cli-571.md` | <https://github.com/Zious11/jira-cli/issues/571> | Strong wrapping inline code produced invalid ADF (400). |
| `prowler-11474.md` | <https://github.com/prowler-cloud/prowler/pull/11474> | Empty fields produced empty text nodes (400). |
| `sectile-803.md` | <https://github.com/sebastienferry/sectile/issues/803> | Inline code inside bold in list items was rejected (400). |
| `sectile-252.md` | <https://github.com/sebastienferry/sectile/issues/252> | A list item continued after a blank line was split into three blocks. |

## Findings from reviewing the predecessor script

The `predecessor-review-*.md` fixtures reproduce findings F1–F21 from the review of the
line-based script this project replaces (that script is not public). Each input is a generic
Markdown snippet:

| Fixture | Finding |
|---|---|
| `predecessor-review-f1.md` | F1: ``` ``a`b`` ``` produced an empty text node (400). |
| `predecessor-review-f2.md` | F2: fenced code was parsed as Markdown. |
| `predecessor-review-f4-f5-f6.md` | F4–F6: every line became a paragraph, no hard breaks, no nesting or continuation lines, lists split by blank lines, start numbers lost. |
| `predecessor-review-f8-f10.md` | F8–F10: silently deleted delimiters, empty paragraphs, wrong inline precedence. |
| `predecessor-review-f11-f13.md` | F11–F13: alignment separators, escaped pipes, empty rows and tables without outer pipes. |
| `predecessor-review-f15.md` | F15: only `- ` and `N. ` started list items; task lists became text. |
| `predecessor-review-f20.md` | F20: `###  spaced ###` kept the leading space and closing hashes. |
