//! Command-line arguments (clap derive). The contract is `docs/cli-contract.md#commands`.
//!
//! The doc comments on arguments are the `--help` text, written for terminals, not rustdoc.
#![allow(clippy::doc_markdown)]

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::options::{Extensions, HtmlPolicy, ImagePolicy, MdFlavor, SoftBreak, Target};

const AFTER_HELP: &str = "\
Examples:
  adf convert notes.md -e jira-comment -o comment.json   Markdown to a Jira comment body
  adf convert - < notes.md                               read Markdown from stdin
  adf convert --from adf issue.json --md-flavor plain    a Jira issue (GET response) as Markdown
  adf validate comment.json                              check ADF before posting it
  adf validate --from md notes.md --strict               would the conversion lose anything?
  adf capabilities --json                                what this build supports

Exit codes: 0 ok, 1 internal error, 2 usage, 3 input/IO, 4 validation failed, 5 lossy (--strict).
Diagnostics go to stderr: text on a terminal, JSON lines otherwise (--error-format).

Unofficial community tool; not affiliated with or endorsed by Atlassian. Jira and Confluence are
trademarks of Atlassian.";

/// Convert Markdown to Atlassian Document Format (ADF) JSON for Jira and Confluence, and back.
// `bin_name` is fixed so that usage lines say `adf` whatever argv[0] is (`adf.exe` on Windows, a
// full path, a renamed copy): `--help` and `E_USAGE` hints are the same on every platform.
#[derive(Debug, Parser)]
#[command(
    name = "adf",
    bin_name = "adf",
    version,
    after_help = AFTER_HELP,
    term_width = 100,
    subcommand_required = true
)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Command,
}

/// Options shared by every command.
#[derive(Debug, Args)]
#[command(next_help_heading = "Global options")]
pub(crate) struct GlobalArgs {
    /// How diagnostics are printed on stderr [default: auto] [env: ADF_ERROR_FORMAT]
    // Not clap's `env`: clap would validate the variable as if it were the flag, so an empty or
    // invalid value broke every command (even with the flag given) and blamed the flag. `None`
    // means "not given"; `report::env_error_format` resolves the variable.
    #[arg(long, value_enum, global = true, value_name = "FORMAT")]
    pub error_format: Option<ErrorFormat>,

    /// Do not print warnings (errors are always printed; exit codes are unchanged)
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Never use colour (also: the NO_COLOR environment variable)
    #[arg(long, global = true)]
    pub no_color: bool,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Convert Markdown to ADF JSON, or ADF JSON to Markdown
    Convert(ConvertArgs),
    /// Check ADF (or a Markdown conversion) without writing output
    Validate(ValidateArgs),
    /// Describe what this build supports
    Capabilities {
        /// Print the machine-readable document (schemas/capabilities.v1.json)
        #[arg(long)]
        json: bool,
    },
    /// Show Markdown → ADF examples, one per feature
    Examples {
        /// Only this feature (see `adf capabilities`)
        feature: Option<String>,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Print a JSON Schema
    Schema {
        /// Which schema
        #[arg(value_enum)]
        name: SchemaName,
    },
    /// Explain a topic or a diagnostic code
    Explain {
        /// exit-codes, envelopes, syntax, codes, or a code such as W_IMAGE_AS_LINK
        topic: String,
    },
    /// Print a shell completion script
    Completions {
        /// The shell
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print version information
    Version {
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
}

/// `adf convert`.
#[derive(Debug, Args)]
pub(crate) struct ConvertArgs {
    /// Input file, or `-` for stdin (the default)
    #[arg(value_name = "INPUT")]
    pub input: Option<PathBuf>,

    /// Write the result to PATH (atomically; nothing is written on failure)
    #[arg(short, long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Input format: Markdown, or ADF JSON (a document or any envelope)
    #[arg(long, value_enum, default_value_t = ConvertFrom::Md)]
    pub from: ConvertFrom,

    #[command(flatten)]
    pub envelope: EnvelopeArgs,

    /// Product whose rules the ADF must follow; auto: confluence for -e confluence, else jira
    #[arg(short, long, value_enum, default_value_t = TargetArg::Auto)]
    pub target: TargetArg,

    /// Pretty-print JSON (the default on a terminal)
    #[arg(long, conflicts_with = "compact")]
    pub pretty: bool,

    /// Print compact JSON (the default in pipes and files)
    #[arg(long)]
    pub compact: bool,

    /// Fail with E_LOSSY (exit 5) instead of writing a result that has warnings
    #[arg(long)]
    pub strict: bool,

    #[command(flatten)]
    pub markdown: MarkdownArgs,

    /// Markdown flavour of ADF → Markdown output
    #[arg(
        long,
        value_enum,
        default_value_t = FlavorArg::Roundtrip,
        value_name = "FLAVOR",
        help_heading = "ADF → Markdown options"
    )]
    pub md_flavor: FlavorArg,
}

/// `adf validate`.
#[derive(Debug, Args)]
pub(crate) struct ValidateArgs {
    /// Input file, or `-` for stdin (the default)
    #[arg(value_name = "INPUT")]
    pub input: Option<PathBuf>,

    /// Input format; auto: ADF when the input starts with `{`, else Markdown
    #[arg(long, value_enum, default_value_t = ValidateFrom::Auto)]
    pub from: ValidateFrom,

    #[command(flatten)]
    pub envelope: EnvelopeArgs,

    /// Product whose rules to check; auto: confluence for Confluence envelopes, else jira
    #[arg(short, long, value_enum, default_value_t = TargetArg::Auto)]
    pub target: TargetArg,

    /// Treat warnings as failures (E_LOSSY, exit 5)
    #[arg(long)]
    pub strict: bool,

    /// Write a JSON report to stdout (schemas/validate-report.v1.json)
    #[arg(long)]
    pub json: bool,

    #[command(flatten)]
    pub markdown: MarkdownArgs,
}

/// `-e` and `--field`.
#[derive(Debug, Args)]
pub(crate) struct EnvelopeArgs {
    /// Wrap the document for an API (Markdown input only; ADF input is detected)
    #[arg(short, long, value_enum, value_name = "ENVELOPE")]
    pub envelope: Option<EnvelopeArg>,

    /// Field name: for -e jira-field, or which `fields` entry to read from ADF input
    /// (default description)
    #[arg(long, value_name = "NAME")]
    pub field: Option<String>,
}

/// Markdown → ADF options.
#[derive(Debug, Args)]
#[command(next_help_heading = "Markdown options")]
pub(crate) struct MarkdownArgs {
    /// What a newline inside a paragraph becomes
    #[arg(long, value_enum, default_value_t = SoftBreakArg::Space, value_name = "MODE")]
    pub softbreak: SoftBreakArg,

    /// What happens to HTML without an ADF equivalent
    #[arg(long, value_enum, default_value_t = HtmlArg::Text, value_name = "POLICY")]
    pub html: HtmlArg,

    /// What happens to images
    #[arg(long, value_enum, default_value_t = ImagesArg::Link, value_name = "POLICY")]
    pub images: ImagesArg,

    /// ADF extensions to recognise: all, none, or a comma-separated list
    /// (alerts,details,cells,raw-adf,mention,status,date,emoji,card,media)
    #[arg(long, default_value = "all", value_name = "LIST", value_parser = parse_extensions)]
    pub extensions: Extensions,
}

fn parse_extensions(spec: &str) -> Result<Extensions, String> {
    Extensions::parse(spec)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ErrorFormat {
    /// Text on a terminal, JSON lines otherwise
    Auto,
    /// `file:line:col: severity[CODE]: message`
    Text,
    /// One JSON object per line (schemas/diagnostic.v1.json)
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ConvertFrom {
    /// Markdown → ADF
    Md,
    /// ADF → Markdown
    Adf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ValidateFrom {
    /// ADF when the input starts with `{`, Markdown otherwise
    Auto,
    /// A dry run of `adf convert`
    Md,
    /// Check an ADF document or envelope
    Adf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum EnvelopeArg {
    /// The bare document
    Doc,
    /// {"body": DOC}
    JiraComment,
    /// {"fields": {"description": DOC}}
    JiraDescription,
    /// {"fields": {"<NAME>": DOC}} (needs --field NAME)
    JiraField,
    /// {"representation": "atlas_doc_format", "value": "<DOC as a JSON string>"}
    Confluence,
}

impl EnvelopeArg {
    pub fn name(self) -> &'static str {
        match self {
            Self::Doc => "doc",
            Self::JiraComment => "jira-comment",
            Self::JiraDescription => "jira-description",
            Self::JiraField => "jira-field",
            Self::Confluence => "confluence",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TargetArg {
    /// Follow the envelope
    Auto,
    /// Jira Cloud
    Jira,
    /// Confluence Cloud
    Confluence,
}

impl TargetArg {
    /// The explicit target, or `None` for `auto`.
    pub fn explicit(self) -> Option<Target> {
        match self {
            Self::Auto => None,
            Self::Jira => Some(Target::Jira),
            Self::Confluence => Some(Target::Confluence),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum SoftBreakArg {
    /// A space (CommonMark)
    Space,
    /// A hardBreak node (as GitHub comments render it)
    #[value(name = "hardbreak")]
    HardBreak,
}

impl From<SoftBreakArg> for SoftBreak {
    fn from(value: SoftBreakArg) -> Self {
        match value {
            SoftBreakArg::Space => Self::Space,
            SoftBreakArg::HardBreak => Self::HardBreak,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum HtmlArg {
    /// Keep it as literal text (W_HTML_AS_TEXT)
    Text,
    /// Remove it (W_HTML_DROPPED)
    Drop,
    /// Fail with E_LOSSY
    Error,
}

impl From<HtmlArg> for HtmlPolicy {
    fn from(value: HtmlArg) -> Self {
        match value {
            HtmlArg::Text => Self::Text,
            HtmlArg::Drop => Self::Drop,
            HtmlArg::Error => Self::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ImagesArg {
    /// A link to the image (W_IMAGE_AS_LINK)
    Link,
    /// An external mediaSingle (Jira rendering unverified)
    External,
    /// Remove it (W_IMAGE_DROPPED)
    Drop,
}

impl From<ImagesArg> for ImagePolicy {
    fn from(value: ImagesArg) -> Self {
        match value {
            ImagesArg::Link => Self::Link,
            ImagesArg::External => Self::External,
            ImagesArg::Drop => Self::Drop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum FlavorArg {
    /// Lossless: extension syntax and ```adf blocks
    Roundtrip,
    /// Compact, readable Markdown for people and LLMs
    Plain,
}

impl From<FlavorArg> for MdFlavor {
    fn from(value: FlavorArg) -> Self {
        match value {
            FlavorArg::Roundtrip => Self::Roundtrip,
            FlavorArg::Plain => Self::Plain,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum SchemaName {
    /// The vendored ADF schema (@atlaskit/adf-schema, draft-04)
    Adf,
    /// One stderr diagnostic line
    Diagnostic,
    /// The `adf capabilities --json` document
    Capabilities,
    /// The `adf validate --json` report
    ValidateReport,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn envelope_names_match_the_library() {
        let names: Vec<&str> = EnvelopeArg::value_variants()
            .iter()
            .map(|v| v.name())
            .collect();
        assert_eq!(names, crate::adf::envelope::ENVELOPE_NAMES);
        for variant in EnvelopeArg::value_variants() {
            assert_eq!(
                variant.to_possible_value().unwrap().get_name(),
                variant.name()
            );
        }
    }

    #[test]
    fn parses_convert_options() {
        let cli = Cli::try_parse_from([
            "adf",
            "convert",
            "in.md",
            "-o",
            "out.json",
            "-e",
            "jira-field",
            "--field",
            "customfield_1",
            "-t",
            "confluence",
            "--softbreak",
            "hardbreak",
            "--extensions",
            "alerts,raw-adf",
            "--error-format",
            "json",
            "-q",
        ])
        .unwrap();
        assert!(cli.global.quiet);
        assert_eq!(cli.global.error_format, Some(ErrorFormat::Json));
        let Command::Convert(args) = cli.command else {
            panic!("expected convert");
        };
        assert_eq!(args.envelope.envelope, Some(EnvelopeArg::JiraField));
        assert_eq!(args.target, TargetArg::Confluence);
        assert_eq!(args.markdown.softbreak, SoftBreakArg::HardBreak);
        assert!(args.markdown.extensions.alerts && !args.markdown.extensions.details);
    }

    /// Windows starts the binary as `…\adf.exe`; usage lines must still say `adf`.
    #[test]
    fn usage_names_the_binary_adf_whatever_argv0_is() {
        for argv0 in [r"C:\tools\adf.exe", "/usr/local/bin/adf-0.1", "adf.exe"] {
            let help = Cli::try_parse_from([argv0, "--help"])
                .unwrap_err()
                .render()
                .to_string();
            assert!(help.contains("Usage: adf [OPTIONS] <COMMAND>"), "{help}");
            let usage = Cli::try_parse_from([argv0, "convert", "--bogus"])
                .unwrap_err()
                .render()
                .to_string();
            assert!(usage.contains("Usage: adf convert [OPTIONS]"), "{usage}");
            assert!(
                !usage.contains(".exe") && !usage.contains("adf-0.1"),
                "{usage}"
            );
        }
    }

    #[test]
    fn rejects_conflicts_and_bad_values() {
        for argv in [
            vec!["adf"],
            vec!["adf", "notes.md"],
            vec!["adf", "convert", "--pretty", "--compact"],
            vec!["adf", "convert", "-t", "bitbucket"],
            vec!["adf", "convert", "--extensions", "alerts,bogus"],
            vec!["adf", "schema", "nope"],
        ] {
            assert!(Cli::try_parse_from(&argv).is_err(), "{argv:?}");
        }
    }
}
