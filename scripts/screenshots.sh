#!/usr/bin/env bash
# Regenerates the terminal screenshots in docs/assets/ from the real binary.
#
# Usage:
#   scripts/screenshots.sh [ADF_BINARY]
#
# ADF_BINARY defaults to <target-dir>/debug/adf, built first (<target-dir> is $CARGO_TARGET_DIR
# or target/). Needs freeze (https://github.com/charmbracelet/freeze), tested with v0.2.2; other
# versions may render slightly different SVG. Install it with
# `brew install charmbracelet/tap/freeze` or `go install github.com/charmbracelet/freeze@v0.2.2`.
#
# Every image is written twice, NAME-dark.svg and NAME-light.svg; the README shows the one that
# matches the reader's GitHub theme. The images are SVG text, so they stay small and sharp. They
# only hold invented content. Run this after an intentional change to the top of `--help`, to the
# text diagnostics or to the `adf validate` summary line; update the alt texts in README.md if what
# the images show changed, and commit the result. An image that does not show what it should is not
# written to docs/assets/.
set -euo pipefail

die() {
  printf 'screenshots.sh: error: %s\n' "$*" >&2
  exit 1
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/docs/assets"

command -v freeze >/dev/null 2>&1 || die "freeze not found: brew install charmbracelet/tap/freeze"

adf="${1:-}"
if [[ -z "$adf" ]]; then
  (cd "$root" && cargo build --locked --quiet)
  # cargo runs in $root, so a relative CARGO_TARGET_DIR is taken from there.
  target_dir="${CARGO_TARGET_DIR:-target}"
  [[ "$target_dir" == /* ]] || target_dir="$root/$target_dir"
  adf="$target_dir/debug/adf"
fi
# Checked before the path is made absolute: a missing directory must stop the script too. Without
# this, freeze would quietly run whatever other `adf` is on PATH.
[[ -f "$adf" && -x "$adf" ]] || die "not an executable file: $adf"
adf="$(cd "$(dirname "$adf")" && pwd)/$(basename "$adf")"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/bin" "$work/shots" "$out"
ln -s "$adf" "$work/bin/adf"

cat >"$work/notes.md" <<'EOF'
# Release notes

The **upload fix** shipped. See the ![chart](https://example.com/chart.png) and `adf validate`.

- [x] fixed
- [ ] announce
EOF

# freeze v0.2.2 draws terminal colours from one fixed palette on any background, and on a light
# background they are too pale to read (1.4:1 to 2.5:1 on white). The light images swap each of
# them for a darker shade with at least 4.5:1 on the white window (WCAG AA): GitHub's light-theme
# green and "attention" yellow, and a dark cyan. A colour missing here stops the script (see
# lighten).
light_colours=(
  "#31BB71=#1a7f37" # green: commands and flags
  "#04D7D7=#0e7490" # cyan: placeholders
  "#D3E561=#9a6700" # bold yellow: the warning severity
)
# The default text colour of each variant. The dark one is freeze's; the light one is GitHub's
# light-theme text colour (15.8:1 on white), set by lighten.
dark_text="#c4c4c4"
light_text="#1f2328"

# Sets the text colours of the light image in place. Only the <tspan> fills and the default fill
# of the text <g> are touched, so the window buttons keep their colours. The github theme sets no
# text colour, and freeze v0.2.2 writes that unset value as the invalid fill "#-00001" (renderers
# ignore it and draw black); it becomes light_text.
lighten() {
  local svg="$1" pair colour
  local edits=(-e "s/(<g [^>]* fill=\")#-00001\"/\\1${light_text}\"/")
  for pair in "${light_colours[@]}"; do
    edits+=(-e "s/(<tspan[^>]* fill=\")${pair%%=*}\"/\\1${pair#*=}\"/g")
  done
  sed -E "${edits[@]}" "$svg" >"$svg.tmp"
  mv "$svg.tmp" "$svg"
  if grep -q 'fill="#-' "$svg"; then
    die "$(basename "$svg") still has an unset colour (fill=\"#-...\"); check that freeze is v0.2.2"
  fi
  while read -r colour; do
    case " ${light_colours[*]#*=} " in
      *" $colour "*) ;;
      *) die "$(basename "$svg") uses text colour $colour, which has no entry in light_colours" ;;
    esac
  done < <(grep -o '<tspan[^>]* fill="#[0-9A-Fa-f]*' "$svg" | grep -o '#[0-9A-Fa-f]*$' | sort -u)
}

# Same look for every image: a window and a monospace stack every OS has, all fonts about 0.6 em
# wide (the width freeze sizes the window for; SF Mono is wider and would be clipped). The dark
# window is freeze's default theme; the light one uses the `github` theme, which sets the white
# background but no text colour (lighten sets them), and a thin border so it stands out on a white
# page. The two variables adf reads are cleared, so the images do not depend on the caller's
# environment.
# usage: shot NAME EXPECTED_TEXT FREEZE_ARGS...
shot() {
  local name="$1" expected="$2"
  shift 2
  local variant svg look text
  for variant in dark light; do
    svg="$work/shots/$name-$variant.svg"
    if [[ "$variant" == dark ]]; then
      look=(--background "#1e1e2e")
      text="$dark_text"
    else
      # The theme sets the white background; --background has no effect with it.
      look=(--theme github --border.width 1 --border.color "#d0d7de")
      text="$light_text"
    fi
    (
      cd "$work"
      # freeze reads stdin instead of running --execute when stdin is a pipe or a socket (an
      # agent, CI, make); with stdin on /dev/null it runs the command from any shell. It also
      # kills an --execute command after 10 s by default, and the first run of a fresh build can
      # be slower.
      env -u NO_COLOR -u ADF_ERROR_FORMAT PATH="$work/bin:$PATH" freeze "$@" \
        --output "$svg" \
        --execute.timeout 30s \
        --window --padding 20 --margin 0 --border.radius 8 "${look[@]}" \
        --font.size 14 \
        --font.family "Menlo, Consolas, 'DejaVu Sans Mono', 'Liberation Mono', monospace" \
        </dev/null
    )
    # Stop instead of replacing a good image with a wrong one: the image must show the expected
    # text, in colour (an old `adf` or a leaked NO_COLOR gives none).
    [[ -s "$svg" ]] || die "freeze wrote no $name-$variant.svg; check that freeze is v0.2.2"
    grep -qF -- "$expected" "$svg" ||
      die "$name-$variant.svg does not show \"$expected\"; check that freeze is v0.2.2"
    grep -q '<tspan[^>]* fill=' "$svg" ||
      die "$name-$variant.svg has no colour; check the adf binary"
    if [[ "$variant" == light ]]; then
      lighten "$svg"
    fi
    grep -q "<g [^>]* fill=\"$text\"" "$svg" ||
      die "$name-$variant.svg does not use default text colour $text; check that freeze is v0.2.2"
  done
  # Both variants passed: only now replace the committed pair.
  for variant in dark light; do
    mv "$work/shots/$name-$variant.svg" "$out/$name-$variant.svg"
  done
}

# The top of `adf --help`: usage and the command list, up to the blank line after it. The help is
# wrapped at a fixed width (term_width in src/cli/args.rs), so the piped text has the same lines.
help_end="$(env -u ADF_ERROR_FORMAT "$adf" --help |
  awk '/^Commands:/ { c = 1; next } c && /^$/ { print NR - 1; exit }')"
[[ -n "$help_end" ]] || die "no command list in adf --help"
shot help "Commands:" --execute "adf --help" --lines "1,$help_end"
# A lossy step reported as a warning, with its code, position and hint.
shot validate "[W_IMAGE_AS_LINK]" --execute "adf validate --from md notes.md" --wrap 100
