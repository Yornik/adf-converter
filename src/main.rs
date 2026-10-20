//! The `adf` command-line tool. All logic lives in the library; see `adf_converter::cli`.
//!
//! A Rust panic would exit with code 101 and print a message that may quote input. Instead, the
//! panic hook prints an `E_INTERNAL` diagnostic and the unwind is caught here, so a bug always
//! exits with the documented code 1 (the release profile keeps `panic = "unwind"` for this).

use std::process::ExitCode;

fn main() -> ExitCode {
    adf_converter::cli::install_panic_hook();
    std::panic::catch_unwind(adf_converter::cli::run).unwrap_or_else(|_| ExitCode::from(1))
}
