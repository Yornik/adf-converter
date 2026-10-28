# Maintainer guide

Owner-side tasks that contributors and agents cannot do: repository settings, releases, and
checks against a live Jira site. Agents working on this repository must not change any of these
settings; they belong to the owner.

## One-time repository settings

State as read with read-only `gh api` calls on 2026-10-19:

| Setting | Now | Target |
|---|---|---|
| Merge methods | merge, squash and rebase allowed | squash only |
| Squash commit title | `COMMIT_OR_PR_TITLE` | the PR title |
| Delete head branches after merge | off | on |
| Require actions pinned to a full-length commit SHA | off | on |
| Default `GITHUB_TOKEN` permissions | read | read (keep) |
| Allow GitHub Actions to create and approve pull requests | on | on, unless `RELEASE_PLEASE_TOKEN` is set (see below) |
| Rulesets | none | branch ruleset and tag ruleset |
| `release` deployment environment (used by the publish job) | does not exist | exists, deployment branches limited to `main`; optionally you as required reviewer |
| Private vulnerability reporting | off | on |
| Renovate GitHub App | not installed | installed on this repository (see [Renovate](#renovate)) |
| Dependabot alerts / security updates | off / off | on / off (Renovate opens the security PRs) |
| CodeQL | not configured | default setup |
| Immutable releases | off | on, once a release has gone through the draft flow |
| Secret scanning and push protection | on | keep |

The commands below make the same changes as the UI. Run them yourself, after reading them.

### Pull requests

**Settings → General → Pull Requests**: allow squash merging only, set the default commit
message to "Pull request title", and turn on "Automatically delete head branches".
release-please reads the squashed commit message, so the PR title must be a Conventional Commit;
`pr-title.yml` checks that. A title-only message also keeps the PR template's checklist out of
the history, and out of release-please's input; breaking changes are marked with `!` in the
title.

```sh
gh repo edit Yornik/adf-converter --enable-squash-merge --enable-merge-commit=false \
  --enable-rebase-merge=false --delete-branch-on-merge
gh api -X PATCH repos/Yornik/adf-converter \
  -f squash_merge_commit_title=PR_TITLE -f squash_merge_commit_message=BLANK
```

### Actions

**Settings → Actions → General**:

- turn on "Require actions to be pinned to a full-length commit SHA";
- keep "Workflow permissions" at "Read repository contents and packages permissions"; each
  workflow job asks for what it needs;
- "Allow GitHub Actions to create and approve pull requests": release-please opens its release PR
  with `GITHUB_TOKEN` unless `RELEASE_PLEASE_TOKEN` is set, and that needs this setting **on**.
  Turn it off only after you add `RELEASE_PLEASE_TOKEN`. With 0 required approvals, an approval
  from a workflow gates nothing anyway.

```sh
gh api -X PUT repos/Yornik/adf-converter/actions/permissions \
  -F enabled=true -f allowed_actions=all -F sha_pinning_required=true
```

### Rulesets

**Settings → Rules → Rulesets.** A branch ruleset for `main`:

- target: the default branch; enforcement: active;
- restrict deletions, block force pushes, require linear history;
- require a pull request: 0 approvals, require conversation resolution, allowed merge method
  "Squash". Leave "Require review from Code Owners" and "Require approval of the most recent
  reviewable push" off: with one maintainer, either one blocks every merge;
- require status checks: `ci-ok` (from `ci.yml`, which aggregates every CI job) and
  `conventional-title` (from `pr-title.yml`);
- bypass list: the Admin role, "For pull requests only", so you can merge a release PR whose
  checks did not run (see [Releases](#releases)).

A tag ruleset for `v*`: block updates, deletions and force pushes, so a published release tag
never moves. Don't restrict tag *creation*, or release-please cannot create the release tag.

The same rulesets as API calls. `actor_id: 5` is the built-in Admin repository role; check it in
the UI after creating the rulesets.

```sh
gh api -X POST repos/Yornik/adf-converter/rulesets --input - <<'EOF'
{
  "name": "main",
  "target": "branch",
  "enforcement": "active",
  "conditions": {"ref_name": {"include": ["~DEFAULT_BRANCH"], "exclude": []}},
  "bypass_actors": [{"actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "pull_request"}],
  "rules": [
    {"type": "deletion"},
    {"type": "non_fast_forward"},
    {"type": "required_linear_history"},
    {"type": "pull_request", "parameters": {
      "required_approving_review_count": 0,
      "dismiss_stale_reviews_on_push": false,
      "require_code_owner_review": false,
      "require_last_push_approval": false,
      "required_review_thread_resolution": true,
      "allowed_merge_methods": ["squash"]}},
    {"type": "required_status_checks", "parameters": {
      "strict_required_status_checks_policy": false,
      "required_status_checks": [{"context": "ci-ok"}, {"context": "conventional-title"}]}}
  ]
}
EOF
gh api -X POST repos/Yornik/adf-converter/rulesets --input - <<'EOF'
{
  "name": "release tags",
  "target": "tag",
  "enforcement": "active",
  "conditions": {"ref_name": {"include": ["refs/tags/v*"], "exclude": []}},
  "bypass_actors": [{"actor_id": 5, "actor_type": "RepositoryRole", "bypass_mode": "always"}],
  "rules": [{"type": "update"}, {"type": "deletion"}, {"type": "non_fast_forward"}]
}
EOF
```

The `ci-ok` and `conventional-title` checks must have run at least once (open any PR) before
GitHub suggests them in the UI. If you rename either job, update the ruleset.

### The `release` environment

The `publish` job in `release.yml` is the only job with `contents: write`, `id-token: write` and
`attestations: write`, and it runs in the deployment environment `release`. If that environment
does not exist, the first release run creates it **without any protection**, so create it
before the first release.

**Settings → Environments → New environment** `release`:

- "Deployment branches and tags": **Selected branches and tags**, with the branch rule `main`.
  Use a branch rule, not a tag pattern such as `v*`: the publish job runs in the workflow run
  started by the push to `main` (its ref is `refs/heads/main`), not in a tag push, so a tag-only
  rule would block every release.
- Optional: "Required reviewers" with yourself, so the publish job waits for your approval after
  the build. Leave "Prevent self-review" off, or you cannot approve your own release.
- No environment secrets are needed; the job uses only `GITHUB_TOKEN`.

```sh
gh api -X PUT repos/Yornik/adf-converter/environments/release --input - <<'EOF'
{"deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true}}
EOF
gh api -X POST repos/Yornik/adf-converter/environments/release/deployment-branch-policies \
  -f name=main -f type=branch
gh api repos/Yornik/adf-converter/environments/release/deployment-branch-policies   # check
```

To add yourself as a required reviewer from the command line, add
`"reviewers": [{"type": "User", "id": <your user id>}]` to the `PUT` body (`gh api user --jq .id`
prints the ID).

### Renovate

Install the [Renovate GitHub App](https://github.com/apps/renovate) for this repository only
(**Only select repositories** → `adf-converter`). The configuration is already in
`.github/renovate.json`, so Renovate skips its onboarding PR and opens a **Dependency Dashboard**
issue instead. It merges its own PRs after all checks pass (`platformAutomerge` is off, so this
does not depend on GitHub auto-merge); the branch ruleset's required checks (`ci-ok`,
`conventional-title`) apply to it like to any other PR, so it needs no bypass.

### Security features

**Settings → Advanced Security** (or "Code security"): turn on private vulnerability reporting,
Dependabot alerts (Renovate reads them to open security PRs; leave Dependabot security updates
off so the same fix is not proposed twice), and CodeQL **default setup** (languages: Rust and
GitHub Actions). [SECURITY.md](../SECURITY.md) points reporters at private vulnerability
reporting, so enable it before the first release.

```sh
gh api -X PUT repos/Yornik/adf-converter/private-vulnerability-reporting
gh api -X PUT repos/Yornik/adf-converter/vulnerability-alerts
gh api -X PUT repos/Yornik/adf-converter/automated-security-fixes
gh api -X PATCH repos/Yornik/adf-converter/code-scanning/default-setup -f state=configured
```

Turn on CodeQL before you add a code-scanning rule to a ruleset, or merges block while there are
no results yet.

### Immutable releases

**Settings → General → Releases**: "Enable release immutability". Once a release is published,
its tag and assets can no longer change, and `gh release verify` works. Immutability starts when
a release is *published*, so `release.yml` must upload every asset while the release is still a
draft. Check that the publish job does draft → upload → publish (and that one release has gone
through it) before you enable this.

```sh
gh api -X PUT repos/Yornik/adf-converter/immutable-releases
```

### Optional: `RELEASE_PLEASE_TOKEN`

Pull requests and tags created with `GITHUB_TOKEN` do not trigger other workflows. Without a
separate token, CI does not run on the release PR, and you merge it with your admin bypass after
checking that `main` is green. Alternatively, close and reopen the release PR: the `reopened`
event comes from you, so CI runs.

To get CI on release PRs automatically, add a repository secret `RELEASE_PLEASE_TOKEN`. Either:

- a **fine-grained personal access token** limited to this repository, with "Contents: Read and
  write" and "Pull requests: Read and write" permissions and an expiry date (renew it before it
  expires); or
- a **GitHub App** installed only on this repository with the same permissions; generate the token
  in the workflow with `actions/create-github-app-token`.

`release.yml` uses the secret when it exists and falls back to `GITHUB_TOKEN` otherwise. After
adding it, you can turn off "Allow GitHub Actions to create and approve pull requests".

## Releases

Releases are cut by [release-please](https://github.com/googleapis/release-please) from the
Conventional Commit messages on `main` (see
[CONTRIBUTING.md](../CONTRIBUTING.md#commit-messages-and-pr-titles) for which types release).

1. Merge pull requests as usual. After each push to `main`, the release-please job in
   `release.yml` opens or updates a release PR (`chore(main): release X.Y.Z`) that bumps the
   version in `Cargo.toml` and `Cargo.lock` and adds the changelog entry.
2. Review the release PR: the version number, the changelog wording, and whether
   `THIRD_PARTY_NOTICES.md` still matches `Cargo.lock` (Renovate updates change crate versions;
   regenerate the list in a normal PR as described in that file). The PR must also bump the
   `adf-converter` version in `fuzz/Cargo.lock` (the `extra-files` entry in
   `release-please-config.json`; its jsonpath needs `@.name.value`, because release-please's TOML
   parser wraps every value in an object).
3. Check that the `Release` workflow is green on the latest `main` commit: every push to `main`
   dry-runs the six release builds, smoke tests and packaging, and a failure there would
   otherwise only show up after the tag exists. Then get CI to run on the release PR (see
   [`RELEASE_PLEASE_TOKEN`](#optional-release_please_token)), and squash-merge it.
4. The next release-please run creates the tag `vX.Y.Z` and the GitHub release. `release.yml`
   then builds the six zips, checks that the tag matches the `Cargo.toml` version, smoke-tests
   each binary on its native runner, uploads the zips and `SHA256SUMS`, creates the build
   provenance attestation, and publishes the release. If the
   [`release` environment](#the-release-environment) has a required reviewer, approve the
   publish job in the Actions tab first.
5. Verify the published release (below).

If the build or publish job fails, re-run the failed jobs from the Actions tab within 30 days
(the build artifacts are kept that long; publish fails, and publishes nothing, unless all six
zips are there). If the problem needs a code change, merge a `fix:` PR; release-please then
proposes the next patch version. Never delete or move a published tag.

### Verifying a release

```sh
tag=vX.Y.Z
dir=$(mktemp -d) && cd "$dir"
gh release view "$tag" -R Yornik/adf-converter            # 6 zips + SHA256SUMS, not a draft
gh release download "$tag" -R Yornik/adf-converter
shasum -a 256 -c SHA256SUMS                                 # Linux: sha256sum -c SHA256SUMS
for zip in adf-converter-*.zip; do gh attestation verify "$zip" --repo Yornik/adf-converter; done
gh release verify "$tag" -R Yornik/adf-converter          # once immutable releases are on
unzip -l "adf-converter-$tag-aarch64-apple-darwin.zip"
```

Each zip must contain one directory `adf-converter-vX.Y.Z-<target>/` with `adf` (`adf.exe` on
Windows), `README.md`, `LICENSE`, `THIRD_PARTY_NOTICES.md`, `CHANGELOG.md`, `docs/`,
`skills/markdown-to-adf/` and `completions/`. On your own machine, unzip the matching archive and
run `./adf --version` and `printf '# Hi\n\n- [ ] task\n' | ./adf convert - | jq .type`.

## Optional: checking rendering on a personal Jira site

The JSON schema and the converter's rules say what Jira *accepts*. How Jira and Confluence
*render* some constructs has not been checked yet, for example expands in comments, external
images, and the exact size limit. To check them:

- Use a **personal** Jira Cloud site (the free plan is enough) with a throwaway project. Never use
  an employer's or customer's site, and never run this from CI.
- Post the extension fixtures as comments on one test issue:

  ```sh
  for f in tests/fixtures/extensions/*.md; do
    adf convert "$f" -e jira-comment -o /tmp/adf-check.json &&
      curl --fail-with-body -sS -o /dev/null -u "$JIRA_EMAIL:$JIRA_API_TOKEN" \
        -H 'Content-Type: application/json' --data-binary @/tmp/adf-check.json \
        "$JIRA_BASE_URL/rest/api/3/issue/SANDBOX-1/comment" && echo "posted $f"
  done
  ```

- Check in the browser: all five panel types, expand and nested expand in comments and in a
  description, status, date, emoji, mention and smart-link rendering, task lists, table
  alignment, `--images external`, and a document just over the 32,767-character size warning.
- Read each comment back with `adf convert --from adf` and compare it with the fixture.
- Record the results in an issue. If a construct is rejected or renders badly, open a `fix:` PR
  with a fixture; if it is confirmed, update its entry in `src/catalog.rs`.

### Sandbox checklist: file media

The `media` extension and the recipe in [posting.md](posting.md#attaching-files-and-screenshots)
rest on facts that come from Atlassian's API reference, from Atlassian-staff tickets and from
community reports, not from this project's own runs. Run the checklist once on a **personal**
Jira Cloud and Confluence Cloud site, then replace each "not run" below with the date, the result
and what you saw. Use invented content only: a screenshot you made of nothing private (a generated
test pattern is enough), a throwaway project and space, and no employer or customer data. Never
paste ids, URLs, tokens or response bodies from a real site into the repository; describe what
happened instead.

For each step, upload the test image with the recipe, write the `adf:media` line (also try the
same node as a raw ```` ```adf ```` block; for files that are not pictures, write the
`adf:attachment` link), convert with `adf convert … -e jira-comment` or
`-e confluence`, post, and look at the result in the browser. Then read it back with
`adf convert --from adf` (roundtrip flavour) and check that the `adf:media` line, the
`adf:attachment` link or the fence matches what you wrote.

| # | Question | How to try | Result |
|---|---|---|---|
| 1 | Does the redirect lookup (`GET /rest/api/3/attachment/content/{id}` without following redirects) still return the media id? Which status (`302` or `303`), and does it need authentication? | Step 2 of the Jira recipe, with and without `-u`. | not run |
| 2 | Is `width`/`height` required on a file `media` in a Jira comment? Which `layout` and pixel `size` does Jira honour? | Post the same image with both, with only one, and with neither; vary `layout` and `size=…px`/`%`. | not run |
| 3 | Does Jira accept a `mediaGroup`, a `mediaInline` and a caption in a comment and in a description, and does it render them? What does real `mediaInline` JSON look like? | Attach a text file in the Jira editor, read the comment back, and post each node: `[name](adf:attachment/<media id>)` alone on a line (a `mediaGroup`) and inside a sentence (a `mediaInline`), and a raw ADF block for the caption. | not run |
| 4 | Confluence comment media: which collection works, and does API-written file media render? Does external media render in Jira and Confluence comments? | Footer comment with the page's `contentId-<pageId>` and with `comment-container-<pageId>`; an editor-attached file read back; `--images external` in both products. | not run |
| 5 | Which API lists a Confluence comment's attachments? (CONFCLOUD-84981, a suggestion in Gathering Interest, says the v1 `content/{commentId}/child/attachment` endpoint is deprecated and returns 410 Gone, and that there is no supported API path and no workaround. v1 attachment endpoints may be retired, which would also end the page upload in the recipe.) | Call it for a comment with an editor-attached file, and repeat the page upload of the Confluence recipe. | not run |
| 6 | Do `__fileName`, `__fileSize` and similar attributes ever appear in REST readbacks? | Read back media posted from the editor, from the API and from a page copy. | not run |
| 7 | Do the Rovo MCP upload and comment tools exist, and do they preserve media nodes? | Only if you use that server; read the tool descriptions first. | not run |
| 8 | Are media nodes accepted in tables, panels, lists and expands, as `adf` places them? | Post `adf:media` lines and `adf:attachment` links inside each container, after `adf validate`, and check the result renders. `adf` keeps media inside list items, quotes, panels, expands and table cells; it moves it out only of task items and `adf:` labels (`W_BLOCK_HOISTED`). A `mediaGroup` (a paragraph of only attachment links) goes in every container except a list item, where the links stay inline `mediaInline`s; a GFM table cell holds inline files only (use a `::: cell` box for a group). | not run |

When the result changes what the docs say:
- Update [posting.md](posting.md#attaching-files-and-screenshots) and the skill's
  `references/posting.md`: remove the "community-reported" or "unverified" label only for what you
  confirmed, and keep it for the rest.
- If a construct is rejected or renders badly, open a `fix:` PR with a fixture; if `adf` should
  warn about something (for example media without `width`/`height`), open an issue first, because
  a new diagnostic code is a contract change.
- Keep the invented UUIDs (`11111111-2222-3333-4444-555555555555`) in all docs and fixtures.

## Routine maintenance

- **Renovate PRs** (crates, `Cargo.lock` maintenance, GitHub Actions, the Rust toolchain, pinned
  CI tools): Renovate proposes updates on Monday mornings, once a release is at least 7 days old
  (security fixes skip the wait), and squash-merges each PR itself as soon as every CI check is
  green. Its titles are Conventional Commits: `deps:` (shipped crates, lock maintenance) releases a
  patch version; `test(deps)`, `chore(deps)` (fuzz workspace), `ci(deps)` and `build(deps)` (Rust
  toolchain) do not. The **Dependency Dashboard** issue lists pending, waiting and failed updates;
  check it now and then. A toolchain update moves `rust-toolchain.toml`, `rust-version` and the
  README's "With Rust … or later" line together (this project targets the latest stable Rust).
  If an automerged update breaks something CI does not cover, revert it with a `revert:` PR and
  add a test.
- **ADF schema drift.** The scheduled CI run validates all goldens against the latest published
  `@atlaskit/adf-schema`; a failure there means the schema changed. To update the vendored copy,
  replace `schemas/adf/full.json` with the new published file (unmodified), update
  `schemas/adf/VERSION` and `schemas/adf/PROVENANCE` (version, source URL, sha256, date), check
  newly added nodes and attributes against `src/adf/targets.rs`, run the full test suite, and
  update the version in `README.md`, `THIRD_PARTY_NOTICES.md` and the docs.
- **Notices.** Regenerate the crate list in `THIRD_PARTY_NOTICES.md` when `Cargo.lock` changes.
