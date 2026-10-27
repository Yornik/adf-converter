# CSV fixtures

Fixtures for the CSV conversions (issue #13). They are run by `tests/csv.rs`.

```
tests/csv/<case>.csv                  CSV input (hand-written)
tests/csv/<case>.opts.json            optional: target, delimiter, header, max_rows, align_numbers
tests/csv/<case>.adf.json             expected ADF document (blessed, never edited by hand)
tests/csv/<case>.diag.jsonl           expected warnings, if any (blessed)

tests/csv/to-csv/<case>.adf.json      ADF input: a bare doc (hand-written)
tests/csv/to-csv/<case>.opts.json     optional: delimiter, table, csv_safe
tests/csv/to-csv/<case>.csv           expected CSV (blessed, never edited by hand)
tests/csv/to-csv/<case>.diag.jsonl    expected warnings, if any (blessed)
```

For every CSV input the test checks that the output is valid ADF (the vendored schema, `validate_adf`
for Jira and Confluence, and the independent invariants), that two runs are byte-equal, and that
CRLF, CR, LF and BOM variants of the input give the same document and warnings.

`bom.csv`, `crlf.csv`, `lone-cr.csv` and `control-chars.csv` must be stored verbatim (see
`.gitattributes` in this directory).

Bless with `ADF_BLESS=1 cargo test --locked --test csv`, then review `git diff tests/csv`.
