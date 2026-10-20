//! Release and CI configuration that can be checked offline: the assumptions that
//! `release-please-config.json`, `.github/workflows/release.yml` and `.github/workflows/ci.yml`
//! rely on, and that a reviewer would otherwise only notice after a release went wrong.

// Test helpers report failures by panicking.
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

/// The contents of a repository file, or `None` if it does not exist.
fn read_optional(relative: &str) -> Option<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => panic!("cannot read {}: {e}", path.display()),
    }
}

fn read(relative: &str) -> String {
    read_optional(relative).unwrap_or_else(|| panic!("{relative} does not exist"))
}

/// The string value of the first `key = "value"` line in the `[package]` table of a manifest.
fn package_value(manifest: &str, key: &str) -> String {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package
            && let Some(rest) = line.strip_prefix(key)
            && let Some(value) = rest.trim_start().strip_prefix('=')
        {
            return value.trim().trim_matches('"').to_owned();
        }
    }
    panic!("no {key} in the [package] table");
}

/// The version a `Cargo.lock` records for `package`.
fn locked_version(lock: &str, package: &str) -> Option<String> {
    let name_line = format!("name = \"{package}\"");
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line == name_line {
            let version = lines.next()?;
            return Some(
                version
                    .strip_prefix("version = ")?
                    .trim_matches('"')
                    .to_owned(),
            );
        }
    }
    None
}

/// Whether release-please's changelog updater finds a version header: its default
/// `versionHeaderRegex` is `\n###? v?[0-9[]`. Without one, it puts the new entry *above* all
/// existing content, so an intro paragraph ends up under the first release, for good.
fn has_release_please_version_header(changelog: &str) -> bool {
    changelog.split('\n').skip(1).any(|line| {
        ["###", "##"].iter().any(|hashes| {
            line.strip_prefix(hashes)
                .and_then(|rest| rest.strip_prefix(' '))
                .map(|rest| rest.strip_prefix('v').unwrap_or(rest))
                .and_then(|rest| rest.chars().next())
                .is_some_and(|c| c.is_ascii_digit() || c == '[')
        })
    })
}

#[test]
fn the_version_header_check_matches_release_please() {
    assert!(has_release_please_version_header(
        "# Changelog\n\n## 0.1.0 (2026-10-20)\n"
    ));
    assert!(has_release_please_version_header(
        "# Changelog\n\n## [0.1.1](https://example.invalid) (2026-10-27)\n"
    ));
    assert!(has_release_please_version_header(
        "# Changelog\n### v1.0.0\n"
    ));
    assert!(!has_release_please_version_header(
        "All notable changes are recorded here.\nDon't edit it by hand.\n"
    ));
    assert!(!has_release_please_version_header(
        "# Changelog\n## Changelog\n"
    ));
    assert!(!has_release_please_version_header("## 0.1.0\n"));
    assert!(!has_release_please_version_header("intro\n#### 1.0\n"));
}

#[test]
fn changelog_is_absent_or_has_a_release_please_version_header() {
    if let Some(changelog) = read_optional("CHANGELOG.md") {
        assert!(
            has_release_please_version_header(&changelog),
            "CHANGELOG.md has no version header that release-please recognises, so the next \
             release would move all of its content below the new entry. Delete the file \
             (release-please creates it with the first release), or keep any text between \
             `# Changelog` and the first version header."
        );
    }
}

#[test]
fn fuzz_lock_records_the_crate_version() {
    let version = package_value(&read("Cargo.toml"), "version");
    let fuzz_version = locked_version(&read("fuzz/Cargo.lock"), "adf-converter")
        .expect("fuzz/Cargo.lock has no adf-converter entry");
    assert_eq!(
        fuzz_version, version,
        "fuzz/Cargo.lock records adf-converter {fuzz_version}, Cargo.toml says {version}. \
         release-please bumps both through `extra-files` in release-please-config.json; to fix \
         it by hand, run `cargo metadata --manifest-path fuzz/Cargo.toml --format-version 1 \
         >/dev/null`."
    );
}

#[test]
fn release_please_bumps_the_fuzz_lock() {
    let config: serde_json::Value =
        serde_json::from_str(&read("release-please-config.json")).unwrap();
    let extra_files = config["packages"]["."]["extra-files"]
        .as_array()
        .expect("release-please-config.json: packages.\".\" has no extra-files");
    // release-please parses TOML with a parser that wraps every value in an object, so the
    // filter must compare `@.name.value`; `@.name == '…'` silently matches nothing.
    let expected = serde_json::json!({
        "type": "toml",
        "path": "fuzz/Cargo.lock",
        "jsonpath": "$.package[?(@.name.value=='adf-converter')].version",
    });
    assert!(
        extra_files.contains(&expected),
        "release-please-config.json: extra-files must contain {expected}"
    );
}

/// The `- target:` entries of the build matrix in `release.yml`.
fn build_matrix_targets(workflow: &str) -> Vec<String> {
    workflow
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- target: "))
        .map(str::to_owned)
        .collect()
}

/// The `targets=( … )` list in the publish job's SHA256SUMS step.
fn publish_targets(workflow: &str) -> Vec<String> {
    let targets: Vec<String> = workflow
        .lines()
        .skip_while(|line| line.trim() != "targets=(")
        .skip(1)
        .take_while(|line| line.trim() != ")")
        .map(|line| line.trim().to_owned())
        .collect();
    assert!(
        !targets.is_empty(),
        "release.yml: no `targets=(` list in the publish job"
    );
    targets
}

#[test]
fn publish_requires_exactly_the_build_matrix_targets() {
    let workflow = read(".github/workflows/release.yml");
    let matrix = build_matrix_targets(&workflow);
    assert_eq!(matrix.len(), 6, "release.yml: expected six build targets");
    assert_eq!(
        publish_targets(&workflow),
        matrix,
        "release.yml: the publish job's target list must equal the build matrix, or every \
         release fails (a target missing from the matrix) or publishes without a check (a \
         target missing from the list)"
    );
}

#[test]
fn msrv_is_declared_once_and_matches_the_readme() {
    let manifest = read("Cargo.toml");
    // The msrv job in ci.yml reads it with `sed -n 's/^rust-version = "\([0-9.]*\)"$/\1/p'`.
    let declared: Vec<&str> = manifest
        .lines()
        .filter_map(|line| line.strip_prefix("rust-version = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .collect();
    assert_eq!(
        declared.len(),
        1,
        "Cargo.toml: expected exactly one `rust-version = \"X.Y\"` line, found {declared:?}"
    );
    let msrv = declared.first().copied().unwrap_or_default();
    assert!(
        !msrv.is_empty() && msrv.split('.').all(|part| part.parse::<u32>().is_ok()),
        "Cargo.toml: rust-version {msrv:?} is not a plain version number"
    );
    let readme = read("README.md");
    let promise = format!("With Rust {msrv} or later");
    assert!(
        readme.contains(&promise),
        "README.md must say \"{promise}\" (the rust-version in Cargo.toml)"
    );
    assert_eq!(
        package_value(&read("fuzz/Cargo.toml"), "rust-version"),
        msrv,
        "fuzz/Cargo.toml: rust-version must match Cargo.toml"
    );
}

#[test]
fn notices_crate_count_matches_the_table() {
    let notices = read("THIRD_PARTY_NOTICES.md");
    let section = notices
        .split("\n## Rust crates\n")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .expect("THIRD_PARTY_NOTICES.md has no `## Rust crates` section");
    let stated: usize = section
        .split("built from the following ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .expect("THIRD_PARTY_NOTICES.md: no \"built from the following N crates\"");
    let rows = section
        .lines()
        .filter(|line| line.starts_with("| "))
        .filter(|line| !line.starts_with("| Crate |"))
        .count();
    assert_eq!(
        rows, stated,
        "THIRD_PARTY_NOTICES.md says {stated} crates but its table has {rows} rows"
    );
}
