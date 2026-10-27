#!/usr/bin/env bash
# Independent contract checks on a built `adf` binary (CI job `contract` in ci.yml).
#
# Usage:
#   cargo build --locked --release && scripts/contract.sh [ADF_BINARY]
#
# ADF_BINARY defaults to target/release/adf (adf.exe on Windows). Requires jq and uv (uvx).
#
# 1. Converts every tests/fixtures/**/*.md (except README.md) with the binary, passing the
#    options from <case>.opts.json as flags ({"softbreak":"hardbreak"} → --softbreak hardbreak;
#    array values are joined with commas). Every conversion must exit 0.
# 2. Validates every blessed tests/fixtures/**/*.adf.json and every fresh output against the
#    vendored schemas/adf/full.json with check-jsonschema, a validator that shares no code with
#    adf (Python `jsonschema` instead of the Rust `jsonschema` crate).
# 3. Validates adf's own JSON outputs (`capabilities --json`, `validate --json`, JSON-lines
#    diagnostics) against the schemas printed by `adf schema …`.
#
# The ADF goldens are byte-compared by tests/golden.rs; this script only checks validity, so it
# also catches a golden that was blessed while the schema check was broken.
set -euo pipefail

# Pinned on purpose: a new validator release must not change CI results unannounced.
CHECK_JSONSCHEMA_VERSION=0.38.2

die() {
  printf 'contract.sh: error: %s\n' "$*" >&2
  exit 1
}
step() {
  printf '\n==> %s\n' "$*" >&2
}
check_jsonschema() {
  uvx --from "check-jsonschema==$CHECK_JSONSCHEMA_VERSION" check-jsonschema "$@"
}

cd "$(dirname "$0")/.."

adf=${1:-target/release/adf}
if [[ ! -f $adf && -f $adf.exe ]]; then
  adf=$adf.exe
fi
[[ -x $adf ]] || die "no executable adf binary at $adf (run: cargo build --locked --release)"
command -v jq >/dev/null 2>&1 || die "jq is required"
command -v uvx >/dev/null 2>&1 || die "uvx (https://docs.astral.sh/uv/) is required"

tmp_root=${TMPDIR:-/tmp}
work=$(mktemp -d "${tmp_root%/}/adf-contract.XXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/fresh" "$work/self"

# --- 1. Convert every Markdown fixture ---------------------------------------------------------
step "Converting every Markdown fixture with $adf"
count=0
while IFS= read -r -d '' md; do
  opts="${md%.md}.opts.json"
  flags=()
  if [[ -f $opts ]]; then
    jq -e 'type == "object"' "$opts" >/dev/null || die "$opts must be a JSON object"
    while IFS= read -r flag; do
      flags+=("$flag")
    done < <(jq -r 'to_entries[] | "--\(.key)", (.value | if type == "array" then join(",") else tostring end)' "$opts")
  fi
  rel=${md#tests/fixtures/}
  dest="$work/fresh/$(printf '%s' "${rel%.md}" | tr '/' '_').adf.json"
  status=0
  # ${flags[@]+…} keeps bash < 4.4 happy when there are no flags.
  "$adf" convert "$md" ${flags[@]+"${flags[@]}"} -o "$dest" || status=$?
  if [[ $status -ne 0 ]]; then
    # GitHub annotation when running in Actions, a plain message otherwise.
    printf '::error file=%s::adf convert exited with status %s\n' "$md" "$status"
    die "adf convert $md ${flags[*]-} exited with status $status"
  fi
  count=$((count + 1))
done < <(find tests/fixtures -type f -name '*.md' ! -name README.md -print0 | LC_ALL=C sort -z)
[[ $count -gt 0 ]] || die "no Markdown fixtures found under tests/fixtures"
printf 'Converted %d fixtures.\n' "$count" >&2

# --- 2. Schema-validate goldens and fresh output -----------------------------------------------
step "Validating goldens and fresh output with check-jsonschema $CHECK_JSONSCHEMA_VERSION"
docs=()
while IFS= read -r -d '' f; do
  docs+=("$f")
done < <(find tests/fixtures -type f -name '*.adf.json' -print0 | LC_ALL=C sort -z)
[[ ${#docs[@]} -gt 0 ]] || die "no blessed *.adf.json goldens found under tests/fixtures"
for f in "$work"/fresh/*.adf.json; do
  docs+=("$f")
done
check_jsonschema --schemafile schemas/adf/full.json "${docs[@]}"

# --- 3. adf's own JSON outputs -----------------------------------------------------------------
step "Validating adf's own JSON outputs against 'adf schema …'"
for name in diagnostic capabilities validate-report; do
  "$adf" schema "$name" >"$work/self/$name.schema.json" || die "'adf schema $name' failed"
  jq -e 'type == "object"' "$work/self/$name.schema.json" >/dev/null ||
    die "'adf schema $name' did not print a JSON object"
done

"$adf" capabilities --json >"$work/self/capabilities.json" || die "'adf capabilities --json' failed"
check_jsonschema --schemafile "$work/self/capabilities.schema.json" "$work/self/capabilities.json"

# An empty text node: invalid under the ADF schema, so `validate` must exit 4 with a report.
printf '%s\n' '{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":""}]}]}' \
  >"$work/self/invalid.json"
status=0
"$adf" validate --json "$work/self/invalid.json" >"$work/self/report.json" || status=$?
[[ $status -eq 4 ]] || die "adf validate --json on an invalid document exited with $status, expected 4"
jq -e '.valid == false and (.errors | length) > 0' "$work/self/report.json" >/dev/null ||
  die "adf validate --json did not report the invalid document as invalid"
check_jsonschema --schemafile "$work/self/validate-report.schema.json" "$work/self/report.json"

# An image produces W_IMAGE_AS_LINK; with --error-format json every stderr line is a diagnostic.
printf '![diagram](https://example.com/diagram.png)\n' >"$work/self/image.md"
"$adf" convert "$work/self/image.md" --error-format json -o "$work/self/image.adf.json" \
  2>"$work/self/diagnostics.jsonl" || die "converting a Markdown image failed"
lines=0
while IFS= read -r line; do
  lines=$((lines + 1))
  printf '%s\n' "$line" >"$work/self/diagnostic-$lines.json"
done <"$work/self/diagnostics.jsonl"
[[ $lines -gt 0 ]] || die "converting an image printed no diagnostics (expected W_IMAGE_AS_LINK)"
jq -e -s 'any(.[]; .code == "W_IMAGE_AS_LINK")' "$work/self/diagnostics.jsonl" >/dev/null ||
  die "converting an image did not report W_IMAGE_AS_LINK"
check_jsonschema --schemafile "$work/self/diagnostic.schema.json" "$work"/self/diagnostic-*.json

# The Agent Skill's CSV helper (skills/markdown-to-adf/scripts/csv2md.py) produces the stored table,
# and adf accepts that table without a single warning.
step "Checking the skill's csv2md.py helper"
if command -v python3 >/dev/null 2>&1; then
  python3 skills/markdown-to-adf/scripts/csv2md.py --align-numbers tests/skill/csv2md-sample.csv \
    >"$work/self/csv2md.md" || die "csv2md.py failed on tests/skill/csv2md-sample.csv"
  diff -u tests/skill/csv2md-sample.md "$work/self/csv2md.md" ||
    die "csv2md.py output differs from tests/skill/csv2md-sample.md"
  "$adf" validate --from md --strict "$work/self/csv2md.md" >/dev/null ||
    die "adf reports warnings for the table csv2md.py wrote"
else
  printf 'contract.sh: warning: python3 not found; skipped the csv2md.py check\n' >&2
fi

printf '\nContract checks passed (%d fixtures, %d ADF documents).\n' "$count" "${#docs[@]}" >&2
