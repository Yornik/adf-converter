//! Shared checks for the fuzz targets. A panic here is a finding: it means the converter broke a
//! documented guarantee (`docs/syntax.md`, `docs/cli-contract.md`).
//!
//! `normalize` and the invariant checks are the same code the integration tests use
//! (`tests/support/`), included by path so there is one definition of each.

#![forbid(unsafe_code)]
// Panicking is how a fuzz target reports a finding; every check here may panic by design.
#![allow(clippy::missing_panics_doc, clippy::must_use_candidate)]

#[path = "../../tests/support/invariants.rs"]
#[allow(dead_code)]
mod invariants;
#[path = "../../tests/support/normalize.rs"]
mod normalize;

use adf_converter::{
    AdfToMdOptions, Code, Diagnostics, Extensions, HtmlPolicy, ImagePolicy, MdFlavor,
    MdToAdfOptions, SoftBreak, Target,
};
use serde_json::Value;

/// Largest input the targets convert; larger inputs only slow the fuzzer down.
pub const MAX_INPUT: usize = 64 * 1024;

/// Options chosen by one input byte, so the fuzzer explores every policy.
pub fn md_options(selector: u8) -> MdToAdfOptions {
    let extensions = match selector >> 6 {
        0 | 1 => Extensions::all(),
        2 => Extensions::none(),
        _ => Extensions::parse("alerts,cells,mention").unwrap_or_default(),
    };
    MdToAdfOptions {
        target: if selector & 1 == 0 {
            Target::Jira
        } else {
            Target::Confluence
        },
        softbreak: if selector & 2 == 0 {
            SoftBreak::Space
        } else {
            SoftBreak::HardBreak
        },
        html: match (selector >> 2) & 3 {
            0 | 1 => HtmlPolicy::Text,
            2 => HtmlPolicy::Drop,
            _ => HtmlPolicy::Error,
        },
        images: match (selector >> 4) & 3 {
            0 | 1 => ImagePolicy::Link,
            2 => ImagePolicy::External,
            _ => ImagePolicy::Drop,
        },
        extensions,
        ..MdToAdfOptions::default()
    }
}

/// Splits fuzzer bytes into an options selector and Markdown text.
pub fn split_input(data: &[u8]) -> Option<(MdToAdfOptions, String)> {
    let (&selector, rest) = data.split_first()?;
    if rest.len() > MAX_INPUT {
        return None;
    }
    Some((
        md_options(selector),
        String::from_utf8_lossy(rest).into_owned(),
    ))
}

/// Errors `md_to_adf` may return: limits, raw ADF, `--html error`.
fn assert_documented_md_errors(diagnostics: &Diagnostics, options: &MdToAdfOptions) {
    assert!(
        diagnostics.has_errors(),
        "Err without an error: {diagnostics}"
    );
    for d in diagnostics.iter().filter(|d| d.is_error()) {
        let allowed = matches!(
            d.code,
            Code::InputTooLarge | Code::InputTooDeep | Code::RawAdf
        ) || (d.code == Code::Lossy && options.html == HtmlPolicy::Error);
        assert!(
            allowed,
            "undocumented md_to_adf error {}: {}",
            d.code, d.message
        );
    }
}

/// Every problem found by the crate's validator and the independent invariant checks.
fn assert_valid(doc: &Value, target: Target) {
    let errors: Vec<String> = adf_converter::validate_adf(doc, target)
        .into_iter()
        .filter(adf_converter::Diagnostic::is_error)
        .map(|d| format!("{}: {}", d.code, d.message))
        .chain(invariants::invariant_errors(doc))
        .collect();
    assert!(errors.is_empty(), "invalid ADF: {errors:#?}\n{doc}");
}

/// Converts Markdown and checks the result: valid ADF or a documented error, deterministic.
/// Returns the document on success.
pub fn check_md_to_adf(markdown: &str, options: &MdToAdfOptions) -> Option<Value> {
    let first = adf_converter::md_to_adf(markdown, options);
    let second = adf_converter::md_to_adf(markdown, options);
    assert!(first == second, "md_to_adf is not deterministic");
    match first {
        Ok(conversion) => {
            let doc = conversion.output.to_value().expect("Doc serializes");
            assert_valid(&doc, options.target);
            Some(doc)
        }
        Err(diagnostics) => {
            assert_documented_md_errors(&diagnostics, options);
            None
        }
    }
}

/// `md_to_adf(adf_to_md(x)) == normalize(x)` for a document produced by `md_to_adf`.
pub fn check_round_trip(x: &Value, options: &MdToAdfOptions) {
    let markdown = adf_converter::adf_to_md(x, &AdfToMdOptions::default())
        .unwrap_or_else(|d| panic!("adf_to_md failed on converter output: {d}"))
        .output;
    let y = check_md_to_adf(&markdown, options)
        .unwrap_or_else(|| panic!("the Markdown from adf_to_md does not convert:\n{markdown}"));
    let (expected, actual) = (normalize::normalize(x), normalize::normalize(&y));
    assert!(
        expected == actual,
        "round trip changed the document\nmarkdown:\n{markdown}\nexpected: {expected}\nactual:   {actual}"
    );
}

/// ADF → Markdown on arbitrary JSON: no panic, only input errors (exit code 3), and any Markdown
/// produced converts back without an undocumented error.
pub fn check_adf_to_md(value: &Value) {
    for flavor in [MdFlavor::Roundtrip, MdFlavor::Plain] {
        let options = AdfToMdOptions {
            flavor,
            ..AdfToMdOptions::default()
        };
        match adf_converter::adf_to_md(value, &options) {
            Ok(conversion) => {
                let back = MdToAdfOptions::default();
                if let Err(d) = adf_converter::md_to_adf(&conversion.output, &back) {
                    assert_documented_md_errors(&d, &back);
                }
            }
            Err(diagnostics) => {
                for d in diagnostics.iter().filter(|d| d.is_error()) {
                    assert_eq!(
                        d.code.exit_code(),
                        3,
                        "unexpected adf_to_md error {}: {}",
                        d.code,
                        d.message
                    );
                }
            }
        }
    }
}
