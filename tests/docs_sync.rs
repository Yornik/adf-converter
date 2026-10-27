//! The normative docs and the code agree: every diagnostic code is documented (with an anchor the
//! JSON diagnostics' `docs` URL points to), every extension name is documented, and every exit
//! code is documented.

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::collections::BTreeSet;
use std::fs;

use adf_converter::{Code, Extensions, Severity};

fn read_doc(relative: &str) -> String {
    let path = support::repo_path(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The table row of `docs/cli-contract.md` that documents `code`, if any.
fn code_row(contract: &str, code: Code) -> Option<&str> {
    let anchor = format!("<a id=\"{}\"></a>", code.as_str().to_ascii_lowercase());
    contract
        .lines()
        .find(|line| line.trim_start().starts_with('|') && line.contains(&anchor))
}

/// The cells of a Markdown table row, trimmed.
fn cells(row: &str) -> Vec<&str> {
    row.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

#[test]
fn every_code_is_documented_with_an_anchor() {
    let contract = read_doc("docs/cli-contract.md");
    let mut missing = Vec::new();
    for &code in Code::ALL {
        match code_row(&contract, code) {
            Some(row) if row.contains(&format!("`{}`", code.as_str())) => {}
            Some(row) => missing.push(format!("{code}: anchor row lacks `{code}`: {row}")),
            None => missing.push(format!(
                "{code}: no table row with <a id=\"{}\"></a>",
                code.as_str().to_ascii_lowercase()
            )),
        }
    }
    assert!(
        missing.is_empty(),
        "docs/cli-contract.md is out of date:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn documented_error_exit_codes_match_the_code_table() {
    let contract = read_doc("docs/cli-contract.md");
    let mut wrong = Vec::new();
    for &code in Code::ALL {
        if code.severity() != Severity::Error {
            continue;
        }
        let Some(row) = code_row(&contract, code) else {
            continue; // reported by every_code_is_documented_with_an_anchor
        };
        let exit = cells(row).get(1).copied().unwrap_or_default();
        if exit != code.exit_code().to_string() {
            wrong.push(format!(
                "{code}: documented exit {exit:?}, code says {}",
                code.exit_code()
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "exit codes differ:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn every_documented_code_exists() {
    let contract = read_doc("docs/cli-contract.md");
    let known: BTreeSet<String> = Code::ALL
        .iter()
        .map(|c| c.as_str().to_ascii_lowercase())
        .collect();
    let mut stale = Vec::new();
    for piece in contract.split("<a id=\"").skip(1) {
        let Some(id) = piece.split('"').next() else {
            continue;
        };
        let is_code = id.starts_with("e_") || id.starts_with("w_");
        if is_code && !known.contains(id) {
            stale.push(id.to_owned());
        }
    }
    assert!(
        stale.is_empty(),
        "docs/cli-contract.md documents codes that do not exist: {stale:?}"
    );
}

#[test]
fn warnings_and_errors_are_in_the_right_sections() {
    let contract = read_doc("docs/cli-contract.md");
    let errors_start = contract.find("### Errors").expect("### Errors section");
    let warnings_start = contract.find("### Warnings").expect("### Warnings section");
    assert!(
        errors_start < warnings_start,
        "errors are documented before warnings"
    );
    let mut misplaced = Vec::new();
    for &code in Code::ALL {
        let anchor = format!("<a id=\"{}\"></a>", code.as_str().to_ascii_lowercase());
        let Some(at) = contract.find(&anchor) else {
            continue;
        };
        let in_errors = at > errors_start && at < warnings_start;
        if in_errors != (code.severity() == Severity::Error) {
            misplaced.push(code.as_str());
        }
    }
    assert!(
        misplaced.is_empty(),
        "codes in the wrong section: {misplaced:?}"
    );
}

/// The paragraph (up to the next blank line) that starts at the first occurrence of `marker`.
fn paragraph_from<'a>(doc: &'a str, marker: &str) -> &'a str {
    let start = doc
        .find(marker)
        .unwrap_or_else(|| panic!("no paragraph starting with {marker:?}"));
    let rest = doc.get(start..).unwrap_or_default();
    rest.split("\n\n").next().unwrap_or(rest)
}

/// The names in [`Extensions::NAMES`] that `paragraph` does not list in backticks.
fn unlisted_extension_names(paragraph: &str) -> Vec<&'static str> {
    Extensions::NAMES
        .iter()
        .copied()
        .filter(|name| !paragraph.contains(&format!("`{name}`")))
        .collect()
}

#[test]
fn extension_list_check_notices_a_missing_name() {
    let without_media = "Names: `alerts`, `details`, `cells`, `raw-adf`, `mention`, `status`, \
                         `date`, `emoji`, `card`, `csv`.";
    assert_eq!(unlisted_extension_names(without_media), ["media"]);
    let bare_words = "alerts details cells raw-adf mention status date emoji card media";
    assert_eq!(unlisted_extension_names(bare_words), Extensions::NAMES);
}

#[test]
fn skill_syntax_reference_lists_every_extension_name() {
    let skill = read_doc("skills/markdown-to-adf/references/syntax.md");
    let listed = paragraph_from(&skill, "All are on by default");
    let missing = unlisted_extension_names(listed);
    assert!(
        missing.is_empty(),
        "the extension list in skills/markdown-to-adf/references/syntax.md lacks {missing:?}"
    );
}

#[test]
fn every_extension_name_is_documented() {
    let syntax = read_doc("docs/syntax.md");
    let contract = read_doc("docs/cli-contract.md");
    for name in Extensions::NAMES {
        let parsed = Extensions::parse(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_ne!(parsed, Extensions::none(), "{name} enables nothing");
        assert_ne!(parsed, Extensions::all(), "{name} enables everything");
    }
    let listed = paragraph_from(&syntax, "All extensions are on by default");
    let missing = unlisted_extension_names(listed);
    assert!(
        missing.is_empty(),
        "the extension list in docs/syntax.md lacks {missing:?}"
    );
    assert!(
        contract.contains("--extensions all|none|LIST"),
        "docs/cli-contract.md documents --extensions"
    );
}

#[test]
fn every_exit_code_is_documented() {
    let contract = read_doc("docs/cli-contract.md");
    let section = contract
        .split("## Exit codes")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("docs/cli-contract.md has an '## Exit codes' section");
    let documented: BTreeSet<u8> = section
        .lines()
        .filter(|line| line.trim_start().starts_with('|'))
        .filter_map(|line| cells(line).first().and_then(|c| c.parse().ok()))
        .collect();
    let expected: BTreeSet<u8> = (0..=5).collect();
    assert_eq!(documented, expected, "exit codes in docs/cli-contract.md");
    for &code in Code::ALL {
        assert!(
            documented.contains(&code.exit_code()),
            "{code} uses undocumented exit code {}",
            code.exit_code()
        );
    }
}

#[test]
fn every_code_has_a_summary() {
    for &code in Code::ALL {
        let summary = code.summary();
        assert!(
            !summary.is_empty() && summary.ends_with('.'),
            "{code}: summary should be one sentence ending in a period: {summary:?}"
        );
    }
}
