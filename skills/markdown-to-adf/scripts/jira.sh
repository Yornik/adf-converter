#!/usr/bin/env bash
# Read and post Jira Cloud content as Markdown with `adf`, using JIRA_API_TOKEN.
#
#   jira.sh read     KEY [--comments]   description (and comments) as Markdown on stdout
#   jira.sh comment  KEY FILE.md        add FILE.md as a formatted comment
#   jira.sh describe KEY FILE.md        replace the description with FILE.md
#
# Needs: adf, curl, jq, and
#   JIRA_BASE_URL   https://your-site.atlassian.net (no trailing slash)
#   JIRA_API_TOKEN  an API token from https://id.atlassian.com/manage-profile/security/api-tokens
# The account email is JIRA_EMAIL if set, else the one `jira me` prints (ankitpokhrel/jira-cli).
#
# `adf` itself never reads credentials; this script is the only place that does. The token is
# handed to curl on stdin (not on the command line, so `ps` does not show it) and is never printed.
# Nothing is posted unless `adf` succeeded: the document is written to a file first.
set -euo pipefail

usage() {
  sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

die() {
  printf 'jira.sh: %s\n' "$*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is not installed or not on PATH"
}

# The account email: JIRA_EMAIL, else `jira me`.
account_email() {
  local email="${JIRA_EMAIL:-}"
  if [ -z "$email" ] && command -v jira >/dev/null 2>&1; then
    email=$(jira me 2>/dev/null | head -n 1 | tr -d '[:space:]') || email=""
  fi
  case "$email" in
    *@*) printf '%s' "$email" ;;
    *) die "no account email: set JIRA_EMAIL, or log in with jira-cli so that 'jira me' prints it" ;;
  esac
}

check_env() {
  [ -n "${JIRA_API_TOKEN:-}" ] || die "JIRA_API_TOKEN is not set (never paste a token into a chat or a file)"
  [ -n "${JIRA_BASE_URL:-}" ] || die "JIRA_BASE_URL is not set, e.g. https://your-site.atlassian.net"
  case "$JIRA_BASE_URL" in
    https://*) ;;
    *) die "JIRA_BASE_URL must start with https://" ;;
  esac
  JIRA_BASE_URL="${JIRA_BASE_URL%/}"
  email=$(account_email)
}

check_key() {
  printf '%s' "$1" | grep -Eq '^[A-Z][A-Z0-9_]*-[0-9]+$' || die "'$1' is not an issue key like PROJ-123"
}

# curl with basic auth, the credentials on stdin. Usage: jcurl ARGS...
jcurl() {
  printf 'user = "%s:%s"\n' "$email" "$JIRA_API_TOKEN" |
    curl --fail-with-body -sS --config - "$@"
}

work=""
cleanup() {
  if [ -n "$work" ] && [ -d "$work" ]; then
    find "$work" -type f -delete
    rmdir "$work"
  fi
}
trap cleanup EXIT

main() {
  [ $# -ge 2 ] || usage
  local action="$1" key="$2"
  need adf
  need curl
  need jq
  check_key "$key"
  check_env
  work=$(mktemp -d)
  case "$action" in
    read)
      jcurl -o "$work/issue.json" "$JIRA_BASE_URL/rest/api/3/issue/$key?fields=summary,description"
      printf '# %s\n\n' "$(jq -r '.fields.summary // ""' "$work/issue.json")"
      if jq -e '.fields.description != null' "$work/issue.json" >/dev/null; then
        adf convert --from adf "$work/issue.json" --md-flavor plain
      else
        printf '(no description)\n'
      fi
      if [ "${3:-}" = "--comments" ]; then
        jcurl -o "$work/comments.json" "$JIRA_BASE_URL/rest/api/3/issue/$key/comment"
        jq -c '.comments[]' "$work/comments.json" | while IFS= read -r c; do
          printf '\n---\n\n'
          printf '%s\n' "$c" | jq -r '"**" + .author.displayName + "** (" + .created + ")"'
          printf '%s\n' "$c" | adf convert --from adf - --md-flavor plain
        done
      fi
      ;;
    comment | describe)
      [ $# -ge 3 ] || usage
      local file="$3"
      [ -f "$file" ] || die "$file is not a file"
      if [ "$action" = comment ]; then
        adf convert "$file" -e jira-comment -o "$work/body.json"
        jcurl -X POST -H 'Content-Type: application/json' --data-binary @"$work/body.json" \
          -o "$work/response.json" "$JIRA_BASE_URL/rest/api/3/issue/$key/comment"
        printf 'comment %s added to %s\n' "$(jq -r '.id' "$work/response.json")" "$key"
      else
        adf convert "$file" -e jira-description -o "$work/body.json"
        jcurl -X PUT -H 'Content-Type: application/json' --data-binary @"$work/body.json" \
          "$JIRA_BASE_URL/rest/api/3/issue/$key"
        printf 'description of %s replaced\n' "$key"
      fi
      ;;
    *) usage ;;
  esac
}

main "$@"
