//! Spec corpora (vendored, nothing is downloaded at test time; see `tests/corpus/NOTICE.md`):
//!
//! - all 652 examples of the CommonMark Spec 0.31.2;
//! - all 672 examples of the GFM Spec 0.29 (tables, strikethrough, autolinks, task lists, …).
//!
//! Every example must convert without an error (warnings are fine) to valid ADF, ADF → Markdown
//! must not fail on the result, and the round-trip guarantee must hold.

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

mod support;

use std::fs;

use adf_converter::{MdFlavor, MdToAdfOptions, Target};
use serde::Deserialize;

/// Failures beyond this many are counted but not printed.
const MAX_REPORTED: usize = 40;

#[derive(Debug, Clone, Deserialize)]
struct Example {
    markdown: String,
    #[serde(rename = "example")]
    number: u32,
    section: String,
    #[serde(default)]
    start_line: u32,
}

fn commonmark_examples() -> Vec<Example> {
    let path = support::repo_path("tests/corpus/commonmark-0.31.2.json");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("commonmark-0.31.2.json is a list of examples")
}

/// Extracts the examples from a CommonMark-style `spec.txt`, like the upstream `spec_tests.py`:
/// an example starts with a line of 32 backticks followed by ` example` (and optional extension
/// names), its Markdown ends at a line holding `.`, and `→` stands for a tab.
fn spec_txt_examples(text: &str) -> Vec<(Example, String)> {
    let fence = "`".repeat(32);
    let mut examples = Vec::new();
    let mut section = String::new();
    let mut markdown = String::new();
    let mut extensions = String::new();
    let mut state = 0; // 0: prose, 1: markdown, 2: expected HTML
    let mut start_line = 0;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if state == 0 {
            if let Some(rest) = trimmed.strip_prefix(&format!("{fence} example")) {
                state = 1;
                rest.trim().clone_into(&mut extensions);
                start_line = u32::try_from(index + 1).unwrap_or(u32::MAX);
            } else if trimmed.starts_with('#') {
                let title = trimmed.trim_start_matches('#');
                if title.starts_with(' ') {
                    title.trim().clone_into(&mut section);
                }
            }
        } else if state == 1 && trimmed == "." {
            state = 2;
        } else if state == 1 {
            markdown.push_str(&line.replace('→', "\t"));
        } else if trimmed == fence {
            state = 0;
            let example = Example {
                markdown: std::mem::take(&mut markdown),
                number: u32::try_from(examples.len() + 1).unwrap_or(u32::MAX),
                section: section.clone(),
                start_line,
            };
            examples.push((example, std::mem::take(&mut extensions)));
        }
    }
    examples
}

fn gfm_examples() -> Vec<(Example, String)> {
    let path = support::repo_path("tests/corpus/gfm-0.29-spec.txt");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    spec_txt_examples(&text)
}

/// Converts one example and checks the result; returns a failure report.
fn check_example(example: &Example, round_trip: bool) -> Result<(), String> {
    let options = MdToAdfOptions::default();
    let (doc, _) = support::md_to_value(&example.markdown, &options)?;
    let problems = support::adf_problems(&doc, Target::Jira);
    if !problems.is_empty() {
        return Err(format!("invalid ADF:\n  {}", problems.join("\n  ")));
    }
    if round_trip {
        support::check_round_trip(&doc, &options)
    } else {
        support::adf_to_markdown(&doc, MdFlavor::Roundtrip)?;
        support::adf_to_markdown(&doc, MdFlavor::Plain)?;
        Ok(())
    }
}

fn run_corpus(name: &str, examples: &[Example], round_trip: bool) {
    let mut failures = Vec::new();
    for example in examples {
        if let Err(e) = check_example(example, round_trip) {
            failures.push(format!(
                "{name} example {} ({}, spec line {}):\n    input: {:?}\n    {}",
                example.number,
                example.section,
                example.start_line,
                example.markdown,
                e.replace('\n', "\n    ")
            ));
        }
    }
    let total = failures.len();
    failures.truncate(MAX_REPORTED);
    assert!(
        total == 0,
        "{total} of {} {name} examples failed{}:\n\n{}\n",
        examples.len(),
        if total > MAX_REPORTED {
            format!(" (first {MAX_REPORTED} shown)")
        } else {
            String::new()
        },
        failures.join("\n\n")
    );
}

#[test]
fn commonmark_examples_convert_to_valid_adf() {
    run_corpus("CommonMark 0.31.2", &commonmark_examples(), false);
}

#[test]
fn commonmark_examples_round_trip() {
    run_corpus("CommonMark 0.31.2", &commonmark_examples(), true);
}

#[test]
fn gfm_examples_convert_to_valid_adf() {
    let examples: Vec<Example> = gfm_examples().into_iter().map(|(e, _)| e).collect();
    run_corpus("GFM 0.29", &examples, false);
}

#[test]
fn gfm_examples_round_trip() {
    let examples: Vec<Example> = gfm_examples().into_iter().map(|(e, _)| e).collect();
    run_corpus("GFM 0.29", &examples, true);
}

/// The vendored files are complete and were not modified (structure checks; the SHA-256 sums are
/// recorded in `tests/corpus/NOTICE.md`).
#[test]
fn corpus_files_are_intact() {
    let commonmark = commonmark_examples();
    assert_eq!(commonmark.len(), 652, "CommonMark 0.31.2 has 652 examples");
    for (i, example) in commonmark.iter().enumerate() {
        assert_eq!(
            example.number as usize,
            i + 1,
            "examples are numbered 1..=652"
        );
    }
    let sections: std::collections::BTreeSet<&str> =
        commonmark.iter().map(|e| e.section.as_str()).collect();
    assert_eq!(
        sections.len(),
        26,
        "CommonMark 0.31.2 examples come from 26 sections"
    );

    let gfm = gfm_examples();
    assert_eq!(gfm.len(), 672, "GFM 0.29 has 672 examples");
    let count = |kind: &str| gfm.iter().filter(|(_, ext)| ext == kind).count();
    assert_eq!(count("table"), 8);
    assert_eq!(count("strikethrough"), 2);
    assert_eq!(count("autolink"), 11);
    assert_eq!(count("tagfilter"), 1);
    assert_eq!(count("disabled"), 2, "the two task-list examples");
    assert!(
        gfm.iter().any(|(e, _)| e.markdown.contains('\t')),
        "→ is converted to a tab"
    );
}

#[test]
fn spec_txt_parser_extracts_examples() {
    let fence = "`".repeat(32);
    let text = format!(
        "# Title\n\n## Tabs\n\n{fence} example\n→foo\n.\n<pre>foo</pre>\n{fence}\n\n\
         ## Tables\n\n{fence} example table\n| a |\n| - |\n.\n<table/>\n{fence}\n"
    );
    let examples = spec_txt_examples(&text);
    assert_eq!(examples.len(), 2);
    let (first, ext) = &examples[0];
    assert_eq!(
        (
            first.markdown.as_str(),
            first.section.as_str(),
            ext.as_str()
        ),
        ("\tfoo\n", "Tabs", "")
    );
    assert_eq!(first.start_line, 5);
    let (second, ext) = &examples[1];
    assert_eq!(
        (
            second.markdown.as_str(),
            second.section.as_str(),
            ext.as_str()
        ),
        ("| a |\n| - |\n", "Tables", "table")
    );
}
