//! The `adf` command line. Contract: `docs/cli-contract.md`.
//!
//! OWNER: stream S3. The signature of `run` is frozen.

use std::process::ExitCode;

/// Runs the CLI with `std::env::args_os()` and returns the process exit code.
pub fn run() -> ExitCode {
    ExitCode::from(1)
}
