//! Source-level guards for hazards that tests cannot safely trigger.

// Test helpers outside `#[test]` functions: failing loudly is the point.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `jsonschema`'s error-collecting APIs explore every branch of the ADF schema's nested `anyOf`s:
/// time and memory grow exponentially with nesting depth (a 200-byte raw `adf` block reached tens
/// of GB and crashed a developer machine). Outside the shallow, per-node validators in
/// `src/adf/validate/schema.rs`, use `is_valid` for the verdict and `crate::adf::validate` for
/// the messages.
#[test]
fn no_exponential_jsonschema_apis_outside_the_shallow_validator() {
    let mut files = Vec::new();
    rust_files(Path::new("src"), &mut files);
    let allowed = Path::new("src/adf/validate/schema.rs");
    let mut offenders = Vec::new();
    for file in &files {
        if file == allowed {
            continue;
        }
        let text = fs::read_to_string(file).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if [
                ".iter_errors(",
                "jsonschema::validate(",
                "jsonschema::evaluate(",
            ]
            .iter()
            .any(|api| code.contains(api))
            {
                offenders.push(format!("{}:{}: {}", file.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "exponential jsonschema APIs used:\n{}",
        offenders.join("\n")
    );
}
