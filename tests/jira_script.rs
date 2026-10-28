//! `skills/markdown-to-adf/scripts/jira.sh` with a fake `curl`: no network, and `JIRA_EMAIL` is
//! always set so that `jira me` is never run. Unix only (the script is bash).
#![cfg(unix)]
#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TOKEN: &str = "dummy-token-for-tests";

const FAKE_CURL: &str = r#"#!/usr/bin/env bash
# Logs the arguments, reads the --config from stdin, answers with a canned document.
echo "ARGS: $*" >> "$FAKE_LOG"
config=$(cat)
printf '%s' "$config" | grep -q '^user = ' && echo "CONFIG: user" >> "$FAKE_LOG"
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in -o) out="$2"; shift 2 ;; http*) url="$1"; shift ;; *) shift ;; esac
done
case "$url" in
  */comment) body='{"id":"10001","comments":[]}' ;;
  *) body='{"fields":{"summary":"Demo","description":{"version":1,"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hello"}]}]}}}' ;;
esac
[ -n "$out" ] && printf '%s' "$body" > "$out"
exit 0
"#;

struct Sandbox {
    dir: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("adf-jira-script-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).unwrap();
        let curl = dir.join("bin/curl");
        fs::write(&curl, FAKE_CURL).unwrap();
        fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.join("curl.log")).unwrap_or_default()
    }

    fn run(&self, args: &[&str], token: &str) -> Output {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/markdown-to-adf/scripts/jira.sh");
        let adf_dir = Path::new(env!("CARGO_BIN_EXE_adf"))
            .parent()
            .unwrap()
            .to_owned();
        let path = format!(
            "{}:{}:/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin",
            self.dir.join("bin").display(),
            adf_dir.display()
        );
        Command::new("bash")
            .arg(script)
            .args(args)
            .env_clear()
            .env("PATH", path)
            .env("HOME", &self.dir)
            .env("TMPDIR", &self.dir)
            .env("JIRA_BASE_URL", "https://example.invalid/")
            .env("JIRA_EMAIL", "tester@example.invalid")
            .env("JIRA_API_TOKEN", token)
            .env("FAKE_LOG", self.dir.join("curl.log"))
            .output()
            .unwrap()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn reads_comments_and_replaces_descriptions_without_the_token_on_the_command_line() {
    if Command::new("jq").arg("--version").output().is_err() {
        eprintln!("jq is not installed; skipping");
        return;
    }
    let sandbox = Sandbox::new("flow");
    let md = sandbox.dir.join("x.md");
    fs::write(&md, "# Hi\n\nbody\n").unwrap();
    let md = md.to_str().unwrap();

    let read = sandbox.run(&["read", "DEMO-1"], TOKEN);
    assert!(read.status.success(), "{}", text(&read.stderr));
    assert!(text(&read.stdout).contains("hello"));
    let comment = sandbox.run(&["comment", "DEMO-1", md], TOKEN);
    assert!(comment.status.success(), "{}", text(&comment.stderr));
    assert!(text(&comment.stdout).contains("comment 10001 added to DEMO-1"));
    let describe = sandbox.run(&["describe", "DEMO-1", md], TOKEN);
    assert!(describe.status.success(), "{}", text(&describe.stderr));

    let log = sandbox.log();
    assert!(
        !log.contains(TOKEN),
        "the token reached curl's arguments:\n{log}"
    );
    assert_eq!(log.matches("CONFIG: user").count(), 3, "{log}");
    assert!(
        log.contains("https://example.invalid/rest/api/3/issue/DEMO-1/comment"),
        "{log}"
    );
    assert!(log.contains("-X PUT"), "{log}");
    for output in [&read, &comment, &describe] {
        assert!(!text(&output.stdout).contains(TOKEN) && !text(&output.stderr).contains(TOKEN));
    }
}

#[test]
fn refuses_bad_input_before_calling_curl() {
    let sandbox = Sandbox::new("refuse");
    let bad_key = sandbox.run(&["read", "not-a-key"], TOKEN);
    assert_eq!(bad_key.status.code(), Some(1));
    assert!(text(&bad_key.stderr).contains("not an issue key"));
    let no_token = sandbox.run(&["read", "DEMO-1"], "");
    assert_eq!(no_token.status.code(), Some(1));
    assert!(text(&no_token.stderr).contains("JIRA_API_TOKEN is not set"));
    let missing = sandbox.run(&["comment", "DEMO-1", "/nonexistent/file.md"], TOKEN);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        sandbox.log().is_empty(),
        "curl must not have run:\n{}",
        sandbox.log()
    );
}

#[test]
fn a_failed_conversion_posts_nothing() {
    let sandbox = Sandbox::new("noconvert");
    let json = sandbox.dir.join("bad.md");
    // Strict mode is not used by the script, so make the conversion fail on an oversized input.
    fs::write(&json, "a".repeat(2 * 1024 * 1024)).unwrap();
    let out = sandbox.run(&["comment", "DEMO-1", json.to_str().unwrap()], TOKEN);
    assert!(!out.status.success());
    assert!(
        sandbox.log().is_empty(),
        "nothing may be posted:\n{}",
        sandbox.log()
    );
}
