//! `![alt](adf:csv/PATH?options)` includes (issue #13, phase 3): the security-sensitive part.
//!
//! The Markdown is hostile: it decides which files are read. These tests build a sandbox in the
//! temp directory with a root and, next to it, an `outside` directory holding a canary file that
//! must never be read. Every refusal is checked for:
//!
//! - the error (`E_INCLUDE`, exit 3, no output) and the rule it names;
//! - no leak: neither the canary's content nor any resolved path (the sandbox, the root, the
//!   outside directory) appears in a diagnostic;
//! - no read: the list of files read (`md_to_adf_with_includes`) stays empty.
//!
//! Every test uses its own directory (`std::env::temp_dir()` and a unique name), removed on drop.

// Test helpers report failures by panicking.
#![allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing
)]

mod support;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use adf_converter::md::IncludedFile;
use adf_converter::{
    Code, Conversion, Diagnostic, Diagnostics, Extensions, IncludeOptions, MdToAdfOptions,
    md_to_adf_with_includes,
};
use serde_json::Value;

/// A unique directory with `root/` and `outside/` (holding the canary), removed on drop.
struct Sandbox {
    base: PathBuf,
    root: PathBuf,
    outside: PathBuf,
}

const CANARY: &str = "CANARY-SECRET-VALUE";

impl Sandbox {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!("adf-include-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let root = base.join("root");
        let outside = base.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(
            outside.join("canary.csv"),
            format!("{CANARY},x\n{CANARY}2,y\n"),
        )
        .unwrap();
        fs::write(base.join("canary.csv"), format!("{CANARY}-base,x\n1,2\n")).unwrap();
        // A well-formed file the tests include when they need one to succeed.
        fs::write(root.join("ok.csv"), "a,b\n1,2\n").unwrap();
        Self {
            base,
            root,
            outside,
        }
    }

    fn options(&self) -> MdToAdfOptions {
        MdToAdfOptions {
            include: Some(IncludeOptions::new(&self.root)),
            ..MdToAdfOptions::default()
        }
    }

    fn convert(&self, markdown: &str) -> Outcome {
        convert(markdown, &self.options())
    }

    fn write(&self, relative: &str, content: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    /// Texts that must not appear anywhere in the output: the canary and every resolved path.
    fn secrets(&self) -> Vec<String> {
        let mut secrets = vec![
            CANARY.to_owned(),
            self.base.display().to_string(),
            self.root.display().to_string(),
            self.outside.display().to_string(),
        ];
        if let Ok(real) = fs::canonicalize(&self.base) {
            secrets.push(real.display().to_string());
        }
        secrets
    }

    /// Fails if a secret appears in any diagnostic or in the document.
    fn assert_no_leak(&self, outcome: &Outcome) {
        let text = outcome.everything();
        for secret in self.secrets() {
            assert!(!text.contains(&secret), "{secret:?} leaked into:\n{text}");
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// The result of one conversion.
struct Outcome {
    result: Result<Conversion<adf_converter::adf::model::Doc>, Diagnostics>,
    read: Vec<IncludedFile>,
}

fn convert(markdown: &str, options: &MdToAdfOptions) -> Outcome {
    let (result, read) = md_to_adf_with_includes(markdown, options);
    Outcome { result, read }
}

impl Outcome {
    fn diagnostics(&self) -> Vec<&Diagnostic> {
        match &self.result {
            Ok(done) => done.warnings.iter().collect(),
            Err(failed) => failed.0.iter().collect(),
        }
    }

    /// Every message, hint and location, and the document as JSON.
    fn everything(&self) -> String {
        let mut text = String::new();
        for diagnostic in self.diagnostics() {
            writeln!(text, "{diagnostic:?}").unwrap();
        }
        if let Ok(done) = &self.result {
            text.push_str(&done.output.to_value().unwrap().to_string());
        }
        text
    }

    fn error(&self) -> &Diagnostic {
        match &self.result {
            Ok(_) => panic!("expected an error, got a document:\n{}", self.everything()),
            Err(failed) => failed
                .0
                .iter()
                .find(|d| d.is_error())
                .expect("an error in the diagnostics"),
        }
    }

    fn doc(&self) -> Value {
        match &self.result {
            Ok(done) => {
                let value = done.output.to_value().unwrap();
                support::assert_valid_adf(&value);
                value
            }
            Err(failed) => panic!("expected a document, got:\n{failed:?}"),
        }
    }

    fn warning_codes(&self) -> Vec<&'static str> {
        match &self.result {
            Ok(done) => done.warnings.iter().map(|w| w.code.as_str()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

fn image(path: &str) -> String {
    format!("![t](<adf:csv/{path}>)\n")
}

/// Asserts `E_INCLUDE` (exit 3) naming `rule`, the PATH as written, nothing read, no leak.
fn assert_refused(sandbox: &Sandbox, path: &str, rule: &str) {
    let outcome = sandbox.convert(&image(path));
    let error = outcome.error();
    assert_eq!(error.code, Code::Include, "{path:?}: {error:?}");
    assert_eq!(error.code.exit_code(), 3);
    assert!(
        error.message.contains(rule),
        "{path:?}: expected {rule:?} in {:?}",
        error.message
    );
    assert!(
        error.message.contains("adf:csv/"),
        "{path:?}: the PATH is named: {:?}",
        error.message
    );
    assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
    assert!(!error.message.chars().any(char::is_control));
    sandbox.assert_no_leak(&outcome);
}

// ---------------------------------------------------------------------------------------------
// Path rules (no file system needed: every one is refused before any file is touched)
// ---------------------------------------------------------------------------------------------

#[test]
fn hostile_paths_are_refused_by_rule() {
    let sandbox = Sandbox::new("rules");
    let cases: &[(&str, &str)] = &[
        // Empty.
        ("", "empty"),
        // Absolute, in the path and percent-encoded (%2F decodes to a leading slash).
        ("/etc/passwd", "absolute"),
        ("%2Fetc/passwd", "absolute"),
        ("%2fetc%2fpasswd", "absolute"),
        // Parent components, plain and percent-encoded.
        ("../outside/canary.csv", "'..'"),
        ("a/../../outside/canary.csv", "'..'"),
        ("sub/..", "'..'"),
        ("..", "'..'"),
        ("%2e%2e%2foutside/canary.csv", "'..'"),
        ("%2E%2E/outside/canary.csv", "'..'"),
        (".%2e/outside/canary.csv", "'..'"),
        ("a/%2e%2e/%2e%2e/outside/canary.csv", "'..'"),
        // Current-directory and empty components.
        ("./ok.csv", "'.'"),
        ("a/./ok.csv", "'.'"),
        (".", "'.'"),
        ("a//ok.csv", "empty component"),
        ("//ok.csv", "absolute"),
        ("ok.csv/", "empty component"),
        ("a%2F%2Fb.csv", "empty component"),
        // Drive letters and ':' (also in later components).
        ("C:/x.csv", "':'"),
        ("C:x.csv", "':'"),
        ("c:/Windows/win.ini", "':'"),
        ("%43%3a/x.csv", "':'"),
        ("sub/ok.csv:stream", "':'"),
        // Backslashes: Windows separators have no meaning here.
        ("sub\\ok.csv", "backslash"),
        ("..\\outside\\canary.csv", "backslash"),
        ("C:\\Windows\\win.ini", "backslash"),
        ("%5coutside%5ccanary.csv", "backslash"),
        ("%5C", "backslash"),
        ("a%5Cb", "backslash"),
        // NUL and control characters, plain and percent-encoded.
        ("ok.csv%00.png", "control character"),
        ("%00", "control character"),
        ("ok%0a.csv", "control character"),
        ("ok%0D.csv", "control character"),
        ("ok%07.csv", "control character"),
        ("ok%1b[31m.csv", "control character"),
        ("ok%7f.csv", "control character"),
        ("ok%c2%85.csv", "control character"),
        ("ok\t.csv", "control character"),
        // Broken percent-encoding.
        ("ok%.csv", "%-escape"),
        ("ok%4", "%-escape"),
        ("ok%zz.csv", "%-escape"),
        ("%ff.csv", "UTF-8"),
    ];
    for (path, rule) in cases {
        assert_refused(&sandbox, path, rule);
    }
}

#[test]
fn a_directory_traversal_that_would_succeed_is_still_refused_as_text() {
    // `root/../outside/canary.csv` exists; the rule fires before the file system is asked.
    let sandbox = Sandbox::new("traversal");
    assert!(sandbox.outside.join("canary.csv").exists());
    assert_refused(&sandbox, "../outside/canary.csv", "'..'");
    assert_refused(&sandbox, "..%2foutside%2fcanary.csv", "'..'");
    assert_refused(&sandbox, "%2e%2e/outside/canary.csv", "'..'");
    assert_refused(&sandbox, "sub/%2e%2e/%2e%2e/canary.csv", "'..'");
}

#[test]
fn percent_decoding_happens_exactly_once() {
    let sandbox = Sandbox::new("decode-once");
    // `%252e%252e%252f` decodes once to the literal text `%2e%2e%2f`: a file name, not a
    // traversal. It does not exist, and nothing outside was read.
    let outcome = sandbox.convert(&image("%252e%252e%252fcanary.csv"));
    assert_eq!(outcome.error().code, Code::Include);
    assert!(outcome.error().message.contains("does not exist"));
    sandbox.assert_no_leak(&outcome);
    // A file whose name holds a literal `%20` is reachable by encoding the `%` once.
    sandbox.write("a%20b.csv", b"h\nliteral\n");
    sandbox.write("a b.csv", b"h\nspace\n");
    let literal = sandbox.convert(&image("a%2520b.csv"));
    assert!(literal.everything().contains("literal"));
    let space = sandbox.convert(&image("a%20b.csv"));
    assert!(space.everything().contains("space"));
}

#[test]
fn messages_are_bounded_and_printable_whatever_the_path() {
    let sandbox = Sandbox::new("bounded");
    let long = format!("{}.csv", "x".repeat(5000));
    let outcome = sandbox.convert(&image(&long));
    let error = outcome.error();
    assert!(error.message.chars().count() < 400, "{}", error.message);
    for path in ["a%00b", "a%1bb", "..%0a"] {
        let outcome = sandbox.convert(&image(path));
        assert!(!outcome.error().message.chars().any(char::is_control));
    }
}

// ---------------------------------------------------------------------------------------------
// The file system: symlinks, special files, missing files
// ---------------------------------------------------------------------------------------------

#[test]
fn a_missing_file_is_refused() {
    let sandbox = Sandbox::new("missing");
    assert_refused(&sandbox, "nothing/here.csv", "does not exist");
    assert_refused(&sandbox, "nothing.csv", "does not exist");
}

#[test]
fn a_missing_or_wrong_root_is_refused_without_naming_it() {
    let sandbox = Sandbox::new("bad-root");
    for root in [
        sandbox.base.join("no-such-dir"),
        sandbox.root.join("ok.csv"),
    ] {
        let options = MdToAdfOptions {
            include: Some(IncludeOptions::new(&root)),
            ..MdToAdfOptions::default()
        };
        let outcome = convert(&image("ok.csv"), &options);
        let error = outcome.error();
        assert_eq!(error.code, Code::Include);
        assert!(error.message.contains("include root"), "{}", error.message);
        sandbox.assert_no_leak(&outcome);
        assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
    }
}

#[test]
fn a_directory_is_not_a_regular_file() {
    let sandbox = Sandbox::new("directory");
    fs::create_dir_all(sandbox.root.join("sub/dir.csv")).unwrap();
    assert_refused(&sandbox, "sub/dir.csv", "not a regular file");
    assert_refused(&sandbox, "sub", "not a regular file");
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_symlink_to_a_file_outside_is_refused() {
        let sandbox = Sandbox::new("link-out-file");
        symlink(
            sandbox.outside.join("canary.csv"),
            sandbox.root.join("leak.csv"),
        )
        .unwrap();
        assert_refused(&sandbox, "leak.csv", "outside the include root");
    }

    #[test]
    fn a_symlink_to_a_relative_path_outside_is_refused() {
        let sandbox = Sandbox::new("link-out-relative");
        symlink("../outside/canary.csv", sandbox.root.join("leak.csv")).unwrap();
        assert_refused(&sandbox, "leak.csv", "outside the include root");
        symlink("../canary.csv", sandbox.root.join("leak2.csv")).unwrap();
        assert_refused(&sandbox, "leak2.csv", "outside the include root");
    }

    #[test]
    fn a_symlinked_directory_to_outside_is_refused() {
        let sandbox = Sandbox::new("link-out-dir");
        symlink(&sandbox.outside, sandbox.root.join("dir")).unwrap();
        assert_refused(&sandbox, "dir/canary.csv", "outside the include root");
        // Through a chain: inside link -> inside link -> outside.
        symlink(sandbox.root.join("dir"), sandbox.root.join("hop")).unwrap();
        assert_refused(&sandbox, "hop/canary.csv", "outside the include root");
    }

    #[test]
    fn a_symlink_to_the_parent_of_the_root_is_refused() {
        let sandbox = Sandbox::new("link-parent");
        symlink(&sandbox.base, sandbox.root.join("up")).unwrap();
        assert_refused(&sandbox, "up/canary.csv", "outside the include root");
        // The canary is also reachable as `up/root/ok.csv` (inside the root again): allowed,
        // because the file really is inside the root.
        let outcome = sandbox.convert(&image("up/root/ok.csv"));
        assert_eq!(outcome.doc()["content"][0]["type"], "table");
    }

    #[test]
    fn a_symlink_inside_the_root_to_a_file_inside_is_allowed() {
        let sandbox = Sandbox::new("link-in");
        sandbox.write("data/real.csv", b"h\nvalue\n");
        symlink("data/real.csv", sandbox.root.join("alias.csv")).unwrap();
        symlink(sandbox.root.join("data"), sandbox.root.join("dirlink")).unwrap();
        for path in ["alias.csv", "dirlink/real.csv", "data/real.csv"] {
            let outcome = sandbox.convert(&image(path));
            assert!(outcome.everything().contains("value"), "{path}");
            assert_eq!(outcome.read.len(), 1);
            assert_eq!(outcome.read[0].path, path);
        }
    }

    #[test]
    fn a_symlinked_root_is_followed_once() {
        let sandbox = Sandbox::new("link-root");
        let link = sandbox.base.join("rootlink");
        symlink(&sandbox.root, &link).unwrap();
        let options = MdToAdfOptions {
            include: Some(IncludeOptions::new(&link)),
            ..MdToAdfOptions::default()
        };
        let outcome = convert(&image("ok.csv"), &options);
        outcome.doc();
        // A link inside the root that leads outside is still refused.
        symlink(
            sandbox.outside.join("canary.csv"),
            sandbox.root.join("leak.csv"),
        )
        .unwrap();
        let outcome = convert(&image("leak.csv"), &options);
        assert_eq!(outcome.error().code, Code::Include);
        sandbox.assert_no_leak(&outcome);
    }

    #[test]
    fn a_symlink_loop_is_refused() {
        let sandbox = Sandbox::new("link-loop");
        symlink("b", sandbox.root.join("a")).unwrap();
        symlink("a", sandbox.root.join("b")).unwrap();
        symlink("self", sandbox.root.join("self")).unwrap();
        for path in ["a", "b", "self", "a/x.csv"] {
            let outcome = sandbox.convert(&image(path));
            assert_eq!(outcome.error().code, Code::Include, "{path}");
            assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
            sandbox.assert_no_leak(&outcome);
        }
    }

    #[test]
    fn a_dangling_symlink_is_refused() {
        let sandbox = Sandbox::new("link-dangling");
        symlink("nowhere.csv", sandbox.root.join("dangling.csv")).unwrap();
        assert_refused(&sandbox, "dangling.csv", "does not exist");
    }

    /// Runs `f` on another thread and fails, rather than hangs, when it takes too long (opening
    /// a FIFO without a writer blocks forever).
    fn within<T: Send + 'static>(seconds: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(f());
        });
        receiver
            .recv_timeout(Duration::from_secs(seconds))
            .expect("the conversion hung (a FIFO was opened?)")
    }

    fn make_fifo(path: &Path) -> bool {
        Command::new("mkfifo")
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[test]
    fn a_fifo_is_refused_before_it_is_opened() {
        let sandbox = Sandbox::new("fifo");
        if !make_fifo(&sandbox.root.join("pipe.csv")) {
            eprintln!("mkfifo is not available; skipping");
            return;
        }
        let options = sandbox.options();
        let outcome = within(10, move || convert(&image("pipe.csv"), &options));
        let error = outcome.error();
        assert_eq!(error.code, Code::Include);
        assert!(
            error.message.contains("not a regular file"),
            "{}",
            error.message
        );
        assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
        // Through a symlink inside the root as well.
        symlink("pipe.csv", sandbox.root.join("via-link.csv")).unwrap();
        let options = sandbox.options();
        let outcome = within(10, move || convert(&image("via-link.csv"), &options));
        assert_eq!(outcome.error().code, Code::Include);
    }

    #[test]
    fn without_an_include_root_nothing_is_opened_not_even_a_fifo() {
        let sandbox = Sandbox::new("fifo-disabled");
        if !make_fifo(&sandbox.root.join("pipe.csv")) {
            eprintln!("mkfifo is not available; skipping");
            return;
        }
        // The default options hold no root, so the PATH must never reach the file system.
        let outcome = within(10, move || {
            convert("![fifo](adf:csv/pipe.csv)\n", &MdToAdfOptions::default())
        });
        assert_eq!(outcome.warning_codes(), ["W_INCLUDE_DISABLED"]);
        assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
    }

    #[test]
    fn an_unreadable_file_is_refused_without_a_path() {
        use std::os::unix::fs::PermissionsExt;
        let sandbox = Sandbox::new("unreadable");
        let path = sandbox.write("secret.csv", b"h\nv\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::File::open(&path).is_ok();
        let outcome = sandbox.convert(&image("secret.csv"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        if readable {
            return; // running as root: permissions do not apply
        }
        assert_eq!(outcome.error().code, Code::Include);
        assert!(outcome.error().message.contains("cannot be read"));
        sandbox.assert_no_leak(&outcome);
    }
}

// ---------------------------------------------------------------------------------------------
// Disabled: no root, no file system
// ---------------------------------------------------------------------------------------------

#[test]
fn without_a_root_the_image_becomes_its_alt_text() {
    let sandbox = Sandbox::new("disabled");
    // The PATH points at a file that would fail loudly if it were read.
    fs::create_dir_all(sandbox.root.join("dir.csv")).unwrap();
    let options = MdToAdfOptions::default();
    let outcome = convert("Before ![The alt text](adf:csv/dir.csv) after.\n", &options);
    let doc = outcome.doc();
    assert_eq!(outcome.warning_codes(), ["W_INCLUDE_DISABLED"]);
    assert_eq!(
        doc["content"][0]["content"][0]["text"],
        "Before The alt text after."
    );
    let warning = outcome.diagnostics()[0];
    let location = warning.location.as_ref().unwrap();
    assert_eq!((location.line, location.column), (Some(1), Some(8)));
    assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);

    // An empty alt text: the PATH.
    let outcome = convert("![](adf:csv/dir.csv?delimiter=tab)\n", &options);
    assert_eq!(outcome.doc()["content"][0]["content"][0]["text"], "dir.csv");
    // Not even a hostile PATH is looked at.
    let outcome = convert("![](adf:csv/../../etc/passwd)\n", &options);
    assert_eq!(outcome.warning_codes(), ["W_INCLUDE_DISABLED"]);
}

#[test]
fn with_the_csv_extension_off_it_is_an_ordinary_extension_warning() {
    let sandbox = Sandbox::new("ext-off");
    let mut options = sandbox.options();
    options.extensions = Extensions::parse("media").unwrap();
    let outcome = convert("![Alt](adf:csv/ok.csv)\n", &options);
    assert_eq!(outcome.warning_codes(), ["W_EXTENSION_DISABLED"]);
    assert_eq!(outcome.doc()["content"][0]["content"][0]["text"], "Alt");
    assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
}

#[test]
fn images_drop_reads_nothing() {
    let sandbox = Sandbox::new("drop");
    let mut options = sandbox.options();
    options.images = adf_converter::ImagePolicy::Drop;
    let outcome = convert("![Alt](adf:csv/ok.csv)\n", &options);
    assert_eq!(outcome.warning_codes(), ["W_IMAGE_DROPPED"]);
    assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
}

#[test]
fn the_link_form_is_not_an_include() {
    let sandbox = Sandbox::new("link-form");
    let outcome = sandbox.convert("[the data](adf:csv/ok.csv)\n");
    assert_eq!(outcome.warning_codes(), ["W_UNKNOWN_EXTENSION"]);
    assert!(
        outcome.diagnostics()[0]
            .message
            .contains("![alt](adf:csv/PATH)")
    );
    assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
}

#[test]
fn an_image_inside_alt_text_never_reads() {
    let sandbox = Sandbox::new("nested");
    let outcome = sandbox.convert("![outer ![inner](adf:csv/../x.csv) text](adf:csv/ok.csv)\n");
    // The outer image is an include; the nested one is only alt text.
    assert_eq!(outcome.read.len(), 1);
    outcome.doc();
}

// ---------------------------------------------------------------------------------------------
// Budgets
// ---------------------------------------------------------------------------------------------

#[test]
fn a_file_over_the_markdown_limit_is_too_large() {
    let sandbox = Sandbox::new("oversize");
    let limit = adf_converter::Limits::default().max_markdown_bytes;
    // One byte over the limit.
    let mut big = b"h\n".to_vec();
    big.resize(limit + 1, b'x');
    sandbox.write("big.csv", &big);
    let outcome = sandbox.convert(&image("big.csv"));
    let error = outcome.error();
    assert_eq!(error.code, Code::InputTooLarge);
    assert_eq!(error.code.exit_code(), 3);
    assert!(
        error.message.contains("adf:csv/big.csv"),
        "{}",
        error.message
    );
    assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
    sandbox.assert_no_leak(&outcome);
}

#[test]
fn a_huge_single_cell_file_at_the_limit_converts() {
    let sandbox = Sandbox::new("huge-cell");
    let limit = adf_converter::Limits::default().max_markdown_bytes;
    let mut data = b"h\n".to_vec();
    data.resize(limit, b'x');
    sandbox.write("cell.csv", &data);
    let outcome = sandbox.convert(&image("cell.csv"));
    let doc = outcome.doc();
    assert_eq!(doc["content"][0]["type"], "table");
    assert_eq!(outcome.read[0].bytes, limit);
}

#[test]
fn the_seventeenth_include_is_refused() {
    let sandbox = Sandbox::new("max-files");
    let sixteen = image("ok.csv").repeat(16);
    let outcome = sandbox.convert(&sixteen);
    outcome.doc();
    assert_eq!(outcome.read.len(), 16);

    let seventeen = image("ok.csv").repeat(17);
    let outcome = sandbox.convert(&seventeen);
    let error = outcome.error();
    assert_eq!(error.code, Code::Include);
    assert!(error.message.contains("at most 16"), "{}", error.message);
    assert_eq!(outcome.read.len(), 16);
    // Located at the seventeenth image.
    assert_eq!(error.location.as_ref().unwrap().line, Some(17));
}

#[test]
fn the_same_file_twice_counts_twice() {
    let sandbox = Sandbox::new("twice");
    let mut options = sandbox.options();
    options.include.as_mut().unwrap().max_files = 2;
    let outcome = convert(&image("ok.csv").repeat(2), &options);
    outcome.doc();
    assert_eq!(outcome.read.len(), 2);
    assert_eq!(outcome.read[0], outcome.read[1]);
    let outcome = convert(&image("ok.csv").repeat(3), &options);
    assert_eq!(outcome.error().code, Code::Include);
}

#[test]
fn the_total_byte_budget_counts_every_read() {
    let sandbox = Sandbox::new("total");
    sandbox.write("hundred.csv", format!("h\n{}\n", "x".repeat(97)).as_bytes());
    let mut options = sandbox.options();
    options.include.as_mut().unwrap().max_total_bytes = 250;
    // 100 + 100 = 200 <= 250.
    let outcome = convert(&image("hundred.csv").repeat(2), &options);
    outcome.doc();
    assert_eq!(outcome.read.iter().map(|f| f.bytes).sum::<usize>(), 200);
    // The third read would make 300.
    let outcome = convert(&image("hundred.csv").repeat(3), &options);
    let error = outcome.error();
    assert_eq!(error.code, Code::Include);
    assert!(error.message.contains("budget"), "{}", error.message);
    assert_eq!(outcome.read.len(), 2);
}

#[test]
fn the_default_total_budget_is_four_mib() {
    let sandbox = Sandbox::new("total-default");
    // 900 KiB per file: four fit in 4 MiB, the fifth does not.
    let mut data = b"h\n".to_vec();
    data.resize(900 * 1024, b'x');
    sandbox.write("nine.csv", &data);
    let outcome = sandbox.convert(&image("nine.csv").repeat(4));
    outcome.doc();
    let outcome = sandbox.convert(&image("nine.csv").repeat(5));
    assert_eq!(outcome.error().code, Code::Include);
    assert_eq!(outcome.read.len(), 4);
}

#[test]
fn included_cells_count_toward_the_document_limit() {
    let sandbox = Sandbox::new("cells");
    // 300 columns x 200 rows = 60,000 cells, about 120 KB.
    let header = vec!["c"; 300].join(",");
    let row = vec!["1"; 300].join(",");
    let mut data = header.clone();
    for _ in 0..199 {
        data.push('\n');
        data.push_str(&row);
    }
    data.push('\n');
    sandbox.write("wide.csv", data.as_bytes());
    let outcome = sandbox.convert(&image("wide.csv"));
    outcome.doc();
    let outcome = sandbox.convert(&image("wide.csv").repeat(2));
    let error = outcome.error();
    assert_eq!(error.code, Code::InputTooLarge);
    assert!(error.message.contains("100000"), "{}", error.message);
}

// ---------------------------------------------------------------------------------------------
// The file's content
// ---------------------------------------------------------------------------------------------

#[test]
fn invalid_utf8_names_the_path() {
    let sandbox = Sandbox::new("utf8");
    sandbox.write("bad.csv", b"a,b\n\xff\xfe,2\n");
    let outcome = sandbox.convert(&image("bad.csv"));
    let error = outcome.error();
    assert_eq!(error.code, Code::InputEncoding);
    assert_eq!(error.code.exit_code(), 3);
    assert!(
        error.message.contains("adf:csv/bad.csv"),
        "{}",
        error.message
    );
    assert!(error.message.contains("UTF-8"));
    sandbox.assert_no_leak(&outcome);
}

#[test]
fn csv_problems_are_located_at_the_image() {
    let sandbox = Sandbox::new("csv-error");
    sandbox.write("quote.csv", b"a,b\n\"never closed,2\n");
    let outcome = sandbox.convert("Intro\n\nText ![t](adf:csv/quote.csv) more\n");
    let error = outcome.error();
    assert_eq!(error.code, Code::InputCsv);
    assert!(
        error.message.starts_with("In quote.csv: "),
        "{}",
        error.message
    );
    assert!(error.message.contains("of the file"), "{}", error.message);
    let location = error.location.as_ref().unwrap();
    assert_eq!((location.line, location.column), (Some(3), Some(6)));
    sandbox.assert_no_leak(&outcome);

    sandbox.write("empty.csv", b"");
    let outcome = sandbox.convert(&image("empty.csv"));
    assert_eq!(outcome.error().code, Code::InputCsv);
    assert!(outcome.error().message.starts_with("In empty.csv: "));
}

#[test]
fn option_errors_are_e_csv_option_at_the_image() {
    let sandbox = Sandbox::new("options");
    for query in [
        "colour=red",
        "delimiter=ab",
        "header=maybe",
        "max-rows=0",
        "align=left",
        "header=true&header=false",
        "header",
        "header=true&&align=none",
        "delimiter=%zz",
    ] {
        let outcome = sandbox.convert(&format!("![t](adf:csv/ok.csv?{query})\n"));
        let error = outcome.error();
        assert_eq!(error.code, Code::CsvOption, "{query}: {error:?}");
        assert_eq!(error.code.exit_code(), 3);
        assert!(error.location.is_some());
        assert_eq!(outcome.read.len(), 0, "files were read: {:?}", outcome.read);
    }
}

#[test]
fn csv_warnings_are_kept_and_located_at_the_image() {
    let sandbox = Sandbox::new("csv-warn");
    sandbox.write("rows.csv", b"h\n1\n2\n3\n");
    let outcome = sandbox.convert("![t](adf:csv/rows.csv?max-rows=1)\n");
    outcome.doc();
    let warning = &outcome.diagnostics()[0];
    assert_eq!(warning.code, Code::CsvTruncated);
    assert!(
        warning.message.starts_with("In rows.csv: "),
        "{}",
        warning.message
    );
    assert_eq!(warning.location.as_ref().unwrap().line, Some(1));

    sandbox.write("ctl.csv", b"h\nbell\x07here\n");
    let outcome = sandbox.convert(&image("ctl.csv"));
    outcome.doc();
    assert!(outcome.warning_codes().contains(&"W_CONTROL_CHAR"));
}

#[test]
fn a_bom_and_crlf_in_the_file_make_the_same_table() {
    let sandbox = Sandbox::new("bom-crlf");
    sandbox.write("lf.csv", b"a,b\n1,2\n");
    sandbox.write("crlf.csv", b"\xef\xbb\xbfa,b\r\n1,2\r\n");
    assert_eq!(
        sandbox.convert(&image("lf.csv")).doc(),
        sandbox.convert(&image("crlf.csv")).doc()
    );
}

// ---------------------------------------------------------------------------------------------
// Behaviour
// ---------------------------------------------------------------------------------------------

#[test]
fn the_report_lists_files_in_document_order() {
    let sandbox = Sandbox::new("report");
    sandbox.write("one.csv", b"h\n1\n");
    sandbox.write("sub/two words.csv", b"h\n22\n");
    let outcome = sandbox.convert(
        "![](adf:csv/sub/two%20words.csv)\n\n![](adf:csv/one.csv?header=false)\n\n![](adf:csv/ok.csv)\n",
    );
    outcome.doc();
    assert_eq!(
        outcome.read,
        [
            IncludedFile {
                path: "sub/two%20words.csv".to_owned(),
                bytes: 5
            },
            IncludedFile {
                path: "one.csv".to_owned(),
                bytes: 4
            },
            IncludedFile {
                path: "ok.csv".to_owned(),
                bytes: 8
            },
        ]
    );
}

#[test]
fn the_output_is_deterministic() {
    let sandbox = Sandbox::new("determinism");
    let markdown = "# T\n\n![a](adf:csv/ok.csv)\n\ntext ![b](adf:csv/ok.csv?header=false) text\n";
    let first = sandbox.convert(markdown).doc();
    for variant in [
        markdown.to_owned(),
        markdown.replace('\n', "\r\n"),
        format!("\u{feff}{markdown}"),
    ] {
        assert_eq!(first, sandbox.convert(&variant).doc());
    }
}

#[test]
fn an_included_table_converts_back_as_a_gfm_table() {
    let sandbox = Sandbox::new("round-trip");
    let first = sandbox.convert(&image("ok.csv")).doc();
    let (markdown, _) =
        support::adf_to_markdown(&first, adf_converter::MdFlavor::Roundtrip).unwrap();
    assert!(markdown.contains("| a"), "{markdown}");
    assert!(!markdown.contains("adf:csv"), "{markdown}");
    support::check_round_trip(&first, &MdToAdfOptions::default()).unwrap();
}

#[test]
fn includes_place_like_gfm_tables() {
    let sandbox = Sandbox::new("placement");
    let outcome = sandbox.convert(
        "- item\n\n  ![](adf:csv/ok.csv)\n\n> quote\n>\n> ![](adf:csv/ok.csv)\n\n- [ ] task ![](adf:csv/ok.csv)\n",
    );
    let doc = outcome.doc();
    let types: Vec<&str> = doc["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "bulletList",
            "table",
            "blockquote",
            "table",
            "taskList",
            "table"
        ]
    );
    assert_eq!(
        outcome.warning_codes(),
        ["W_BLOCK_HOISTED", "W_BLOCK_HOISTED", "W_BLOCK_HOISTED"]
    );
}

// ---------------------------------------------------------------------------------------------
// The CLI
// ---------------------------------------------------------------------------------------------

fn adf(args: &[&str], stdin: &[u8], dir: Option<&Path>) -> std::process::Output {
    use std::io::Write as _;
    let mut command = Command::new(env!("CARGO_BIN_EXE_adf"));
    command
        .args(args)
        .env_remove("ADF_ERROR_FORMAT")
        .env_remove("NO_COLOR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().unwrap();
    let mut pipe = child.stdin.take().unwrap();
    let input = stdin.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = pipe.write_all(&input);
    });
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    output
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn root_arg(sandbox: &Sandbox) -> String {
    sandbox.root.display().to_string()
}

#[test]
fn the_cli_reads_stdin_with_a_root_and_not_without() {
    let sandbox = Sandbox::new("cli-stdin");
    let root = root_arg(&sandbox);
    let with = adf(
        &["convert", "--include-root", &root, "--compact"],
        b"![t](adf:csv/ok.csv)\n",
        None,
    );
    assert_eq!(with.status.code(), Some(0), "{}", text(&with.stderr));
    let doc: Value = serde_json::from_slice(&with.stdout).unwrap();
    assert_eq!(doc["content"][0]["type"], "table");

    let without = adf(&["convert", "--compact"], b"![t](adf:csv/ok.csv)\n", None);
    assert_eq!(without.status.code(), Some(0));
    assert!(text(&without.stderr).contains("W_INCLUDE_DISABLED"));
    assert!(!text(&without.stdout).contains("\"table\""));
}

#[test]
fn the_cli_has_no_default_root_not_even_the_working_directory() {
    let sandbox = Sandbox::new("cli-no-default");
    let output = adf(
        &["convert", "--compact"],
        b"![t](adf:csv/ok.csv)\n",
        Some(&sandbox.root),
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(text(&output.stderr).contains("W_INCLUDE_DISABLED"));
    // Nor the directory of the input file.
    let input = sandbox.root.join("doc.md");
    fs::write(&input, "![t](adf:csv/ok.csv)\n").unwrap();
    let output = adf(
        &["convert", "--compact", &input.display().to_string()],
        b"",
        None,
    );
    assert!(text(&output.stderr).contains("W_INCLUDE_DISABLED"));
    assert!(!text(&output.stdout).contains("\"table\""));
}

#[test]
fn the_cli_refuses_a_hostile_path_with_exit_3_and_no_output() {
    let sandbox = Sandbox::new("cli-hostile");
    let root = root_arg(&sandbox);
    for path in [
        "../outside/canary.csv",
        "/etc/passwd",
        "%2e%2e%2foutside%2fcanary.csv",
    ] {
        let markdown = format!("![t](adf:csv/{path})\n");
        let output = adf(
            &["convert", "--include-root", &root],
            markdown.as_bytes(),
            None,
        );
        assert_eq!(output.status.code(), Some(3), "{path}");
        assert_eq!(output.stdout.len(), 0, "{path}: no output");
        let stderr = text(&output.stderr);
        assert!(stderr.contains("E_INCLUDE"), "{path}: {stderr}");
        for secret in sandbox.secrets() {
            assert!(!stderr.contains(&secret), "{path}: leaked {secret}");
        }
    }
}

#[test]
fn the_cli_refuses_a_bad_root_before_reading_the_input() {
    let sandbox = Sandbox::new("cli-bad-root");
    let missing = sandbox.base.join("nope").display().to_string();
    let file = sandbox.root.join("ok.csv").display().to_string();
    for (root, needle) in [(&missing, "does not exist"), (&file, "not a directory")] {
        for command in ["convert", "validate"] {
            // The input would be invalid UTF-8: the usage error must come first.
            let output = adf(&[command, "--include-root", root], b"\xff\xfe", None);
            assert_eq!(output.status.code(), Some(2), "{command} {root}");
            let stderr = text(&output.stderr);
            assert!(stderr.contains("E_USAGE"), "{stderr}");
            assert!(stderr.contains(needle), "{stderr}");
            assert_eq!(output.stdout.len(), 0, "no output");
        }
    }
}

#[test]
fn the_cli_refuses_the_root_for_input_that_is_not_markdown() {
    let sandbox = Sandbox::new("cli-not-md");
    let root = root_arg(&sandbox);
    let adf_doc = br#"{"type":"doc","version":1,"content":[{"type":"paragraph","content":[{"type":"text","text":"x"}]}]}"#;
    for args in [
        vec!["convert", "--from", "adf", "--include-root", &root],
        vec!["convert", "--from", "csv", "--include-root", &root],
        vec!["validate", "--from", "adf", "--include-root", &root],
        vec!["validate", "--from", "csv", "--include-root", &root],
        // Auto-detection: the input turns out to be ADF.
        vec!["validate", "--include-root", &root],
    ] {
        let output = adf(&args, adf_doc, None);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = text(&output.stderr);
        assert!(stderr.contains("E_USAGE"), "{args:?}: {stderr}");
        assert!(
            stderr.contains("--include-root only applies to Markdown input"),
            "{stderr}"
        );
        assert_eq!(output.stdout.len(), 0, "no output");
    }
    // Auto-detection that finds Markdown works.
    let output = adf(
        &["validate", "--include-root", &root],
        b"![t](adf:csv/ok.csv)\n",
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
}

#[test]
fn validate_json_lists_the_includes() {
    let sandbox = Sandbox::new("cli-report");
    let root = root_arg(&sandbox);
    sandbox.write("sub/two words.csv", b"h\n22\n");
    let markdown = "![](adf:csv/sub/two%20words.csv)\n\n![](adf:csv/ok.csv?header=false)\n";
    let output = adf(
        &["validate", "--json", "--include-root", &root],
        markdown.as_bytes(),
        None,
    );
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(
        report["includes"],
        serde_json::json!([
            {"path": "sub/two%20words.csv", "bytes": 5},
            {"path": "ok.csv", "bytes": 8}
        ])
    );
    assert_matches_report_schema(&report);

    // Always present, empty when nothing was read.
    for (args, input) in [
        (vec!["validate", "--json"], "plain text\n"),
        (vec!["validate", "--json"], "![](adf:csv/ok.csv)\n"),
        (vec!["validate", "--json", "--from", "csv"], "a,b\n1,2\n"),
    ] {
        let output = adf(&args, input.as_bytes(), None);
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["includes"], serde_json::json!([]), "{args:?}");
        assert_matches_report_schema(&report);
    }
    // A failure reports what was read before it.
    let markdown = "![](adf:csv/ok.csv)\n\n![](adf:csv/../outside/canary.csv)\n";
    let output = adf(
        &["validate", "--json", "--include-root", &root],
        markdown.as_bytes(),
        None,
    );
    assert_eq!(output.status.code(), Some(3));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert_eq!(report["includes"][0]["path"], "ok.csv");
    assert_eq!(report["errors"][0]["code"], "E_INCLUDE");
    let rendered = report.to_string();
    for secret in sandbox.secrets() {
        assert!(!rendered.contains(&secret));
    }
    assert_matches_report_schema(&report);
}

fn assert_matches_report_schema(report: &Value) {
    let schema: Value = serde_json::from_str(
        &fs::read_to_string(support::repo_path("schemas/validate-report.v1.json")).unwrap(),
    )
    .unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(
        validator.is_valid(report),
        "report does not match the schema: {report}"
    );
}

#[test]
fn the_cli_output_is_the_same_as_the_library_and_has_no_resolved_path() {
    let sandbox = Sandbox::new("cli-output");
    let root = root_arg(&sandbox);
    let output = adf(
        &["convert", "--compact", "--include-root", &root],
        b"![t](adf:csv/ok.csv)\n",
        None,
    );
    assert_eq!(output.status.code(), Some(0));
    let doc: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(doc, sandbox.convert("![t](adf:csv/ok.csv)\n").doc());
    assert!(!text(&output.stdout).contains(&root));
}
