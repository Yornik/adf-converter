//! The `adf` command-line tool. All logic lives in the library; see `adf_converter::cli`.

fn main() -> std::process::ExitCode {
    adf_converter::cli::run()
}
