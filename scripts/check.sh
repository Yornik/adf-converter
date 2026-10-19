#!/usr/bin/env bash
# Runs the same checks as CI's lint, test and supply-chain jobs. Run it before every push:
#
#   scripts/check.sh
#
# cargo-deny is optional locally (`cargo install --locked cargo-deny`); CI always runs it.
set -euo pipefail

cd "$(dirname "$0")/.."

step() {
  printf '\n==> %s\n' "$*"
}

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --locked --all-targets --all-features -- -D warnings"
cargo clippy --locked --all-targets --all-features -- -D warnings

step "cargo test --locked --all-features"
cargo test --locked --all-features

step 'RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps'
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps

# `cargo deny` also finds cargo-deny in $CARGO_HOME/bin when that is not on PATH.
if cargo deny --version >/dev/null 2>&1; then
  step "cargo deny --locked check"
  cargo deny --locked check
else
  step "cargo deny --locked check (skipped: cargo-deny is not installed; CI runs it)"
fi

printf '\nAll checks passed.\n'
