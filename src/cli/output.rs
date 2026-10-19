//! The only place that writes to stdout, stderr and output files.
//!
//! Results are buffered by the commands and written in one go, so a failed conversion never
//! leaves partial output (important for `adf … | curl --data-binary @-`). Files are written
//! atomically: a temporary file in the same directory, then a rename.

use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

/// Writes `bytes` to stdout and flushes.
pub(crate) fn stdout(bytes: &[u8]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(bytes)?;
    out.flush()
}

/// Writes `text` to stderr. Errors are ignored: there is nowhere left to report them.
pub(crate) fn stderr(text: &str) {
    let mut err = io::stderr().lock();
    let _ = err.write_all(text.as_bytes());
    let _ = err.flush();
}

pub(crate) fn stdin_is_terminal() -> bool {
    io::stdin().is_terminal()
}

pub(crate) fn stdout_is_terminal() -> bool {
    io::stdout().is_terminal()
}

pub(crate) fn stderr_is_terminal() -> bool {
    io::stderr().is_terminal()
}

/// Whether the `NO_COLOR` convention asks for no colour (set and not empty).
pub(crate) fn no_color_env() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

/// Writes `bytes` to `path` atomically: the data goes to a new temporary file next to `path`,
/// which is then renamed over it. On any error the temporary file is removed and `path` is left
/// untouched.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            "is a directory",
        ));
    }
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a file path",
        ));
    };
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut attempt = 0u32;
    let (temp, mut file) = loop {
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(name);
        temp_name.push(format!(".{}.{attempt}.tmp", std::process::id()));
        let temp = dir.join(temp_name);
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (temp, file),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => {
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// A short, platform-independent description of an I/O error (the OS message differs between
/// platforms and languages).
#[allow(clippy::wildcard_enum_match_arm)] // `ErrorKind` is non-exhaustive; the rest use its text
pub(crate) fn describe_io_error(err: &io::Error) -> String {
    match err.kind() {
        io::ErrorKind::NotFound => "no such file or directory".to_owned(),
        io::ErrorKind::PermissionDenied => "permission denied".to_owned(),
        io::ErrorKind::IsADirectory => "is a directory".to_owned(),
        io::ErrorKind::NotADirectory => "a parent is not a directory".to_owned(),
        io::ErrorKind::InvalidInput => "not a file path".to_owned(),
        io::ErrorKind::StorageFull => "no space left on the device".to_owned(),
        io::ErrorKind::ReadOnlyFilesystem => "read-only file system".to_owned(),
        kind => kind.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("adf-output-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_and_replaces_atomically() {
        let dir = temp_dir("replace");
        let path = dir.join("out.json");
        write_atomic(&path, b"one").unwrap();
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(entries(&dir), ["out.json"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failures_leave_nothing_behind() {
        let dir = temp_dir("fail");
        let sub = dir.join("sub");
        fs::create_dir(&sub).unwrap();
        let err = write_atomic(&sub, b"x").unwrap_err();
        assert_eq!(describe_io_error(&err), "is a directory");
        let err = write_atomic(&dir.join("missing").join("out.json"), b"x").unwrap_err();
        assert_eq!(describe_io_error(&err), "no such file or directory");
        assert_eq!(entries(&dir), ["sub"]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
