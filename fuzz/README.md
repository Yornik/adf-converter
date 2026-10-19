# Fuzzing

Coverage-guided fuzz targets for `adf`, using [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz)
(libFuzzer). This directory is its own Cargo workspace, so `cargo build` and `cargo test` in the
repository root never build it.

| Target | Input | A finding (panic) means |
|---|---|---|
| `md_to_adf` | bytes: an options byte, then Markdown | a panic, a stack overflow, invalid ADF, an undocumented error, or non-deterministic output |
| `adf_to_md` | bytes parsed as JSON (any value, usually ADF-shaped) | a panic, an error that is not an input error (exit code 3), or Markdown that cannot be converted back |
| `roundtrip` | bytes: an options byte, then Markdown | everything `md_to_adf` checks, plus `md_to_adf(adf_to_md(x)) != normalize(x)` for an ADF document `x` it produced |

The first input byte selects the options (target, `--softbreak`, `--html`, `--images`,
`--extensions`), so one corpus covers every policy. "Valid ADF" is checked with
`adf_converter::validate_adf` and the independent invariant checks in
`tests/support/invariants.rs`; `normalize` is `tests/support/normalize.rs`. Both are shared with
the integration tests (see `src/lib.rs`).

## Running

Fuzzing needs a nightly toolchain and cargo-fuzz:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz --locked
```

From the repository root:

```sh
# Seed the corpora from the test fixtures (optional, but finds bugs much faster).
mkdir -p fuzz/corpus/md_to_adf fuzz/corpus/roundtrip
for f in tests/fixtures/*/*.md; do
  printf '\000' | cat - "$f" > "fuzz/corpus/md_to_adf/$(basename "$f")"
done
cp fuzz/corpus/md_to_adf/* fuzz/corpus/roundtrip/

cargo +nightly fuzz run md_to_adf -- -max_total_time=600 -dict="$PWD/fuzz/dict/markdown.dict"
cargo +nightly fuzz run roundtrip -- -max_total_time=600 -dict="$PWD/fuzz/dict/markdown.dict"
cargo +nightly fuzz run adf_to_md -- -max_total_time=600 -dict="$PWD/fuzz/dict/adf.dict"
```

Useful libFuzzer flags: `-max_len=65536` (longer inputs), `-jobs=N -workers=N` (parallel runs),
`-timeout=10` (report slow inputs). `fuzz/corpus/`, `fuzz/artifacts/`, `fuzz/coverage/` and
`fuzz/target/` are git-ignored.

To check that the targets still compile without nightly (CI's lint job runs this on every PR):

```sh
cargo check --manifest-path fuzz/Cargo.toml --bins --lib
```

## When a target finds something

1. Reproduce it: `cargo +nightly fuzz run md_to_adf fuzz/artifacts/md_to_adf/crash-…`.
2. Minimize it: `cargo +nightly fuzz tmin md_to_adf fuzz/artifacts/md_to_adf/crash-…`.
3. Turn the minimized input into a permanent test: a fixture under `tests/fixtures/` if it
   converts successfully once fixed (drop the first byte; add a `<case>.opts.json` if the
   options byte mattered), or a deterministic test in `tests/properties.rs` / a CLI case otherwise.
4. Fix the bug, bless the fixture (`ADF_BLESS=1 cargo test --locked --test golden`), and keep the
   test.

Never commit fuzz inputs built from private documents.
