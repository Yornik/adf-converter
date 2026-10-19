//! `adf convert`: Markdown → ADF (with the self-check and the envelope) and ADF → Markdown.

use std::path::Path;

use serde_json::Value;

use super::Outcome;
use super::args::{ConvertArgs, ConvertFrom, EnvelopeArg, MarkdownArgs, TargetArg};
use super::input::{self, Input};
use super::output;
use crate::Conversion;
use crate::adf::envelope::{self, Envelope};
use crate::adf::model::{
    BlockquoteChild, BulletList, Doc, DocBlock, ExpandChild, ListItem, ListItemChild,
    NestedExpandChild, OrderedList, PanelChild, Table, TableCellChild,
};
use crate::diagnostics::{self, Code, Diagnostic, Diagnostics, omitted_count, reported_count};
use crate::limits::Limits;
use crate::options::{AdfToMdOptions, MdToAdfOptions, Target};

const LOSSY_CONVERSION: &str = "--strict: the conversion is lossy ({codes}); nothing was written";

/// Runs `adf convert`.
pub(crate) fn run(args: &ConvertArgs) -> Outcome {
    match args.from {
        ConvertFrom::Md => markdown_to_adf(args),
        ConvertFrom::Adf => adf_to_markdown(args),
    }
}

fn markdown_to_adf(args: &ConvertArgs) -> Outcome {
    let envelope = match parse_envelope(args.envelope.envelope, args.envelope.field.as_deref()) {
        Ok(envelope) => envelope,
        Err(outcome) => return *outcome,
    };
    let target = resolve_target(args.target, &envelope);
    let input = match read_input(args.input.as_deref(), "convert") {
        Ok(input) => input,
        Err(outcome) => return *outcome,
    };
    let opts = markdown_options(&args.markdown, target);
    let converted = match markdown_pipeline(&input.text, &opts) {
        Ok(converted) => converted,
        Err(diagnostics) => return Outcome::failed(diagnostics, Some(input.label)),
    };
    if args.strict && !converted.warnings.is_empty() {
        return Outcome::failed(
            strict_failure(converted.warnings, LOSSY_CONVERSION),
            Some(input.label),
        );
    }
    let wrapped = envelope::wrap(converted.doc, &envelope);
    let pretty = pretty_output(args, args.output.as_deref());
    Outcome {
        stdout: json_bytes(&wrapped, pretty),
        output_file: args.output.clone(),
        diagnostics: converted.warnings,
        label: Some(input.label),
        ..Outcome::default()
    }
}

fn adf_to_markdown(args: &ConvertArgs) -> Outcome {
    if args.envelope.envelope.is_some() {
        return Outcome::usage(
            "-e/--envelope cannot be used with --from adf; the envelope is detected from the input",
            "Drop -e; use --field NAME to pick a Jira field",
        );
    }
    let input = match read_input(args.input.as_deref(), "convert") {
        Ok(input) => input,
        Err(outcome) => return *outcome,
    };
    let unwrapped = match parse_and_unwrap(&input.text, args.envelope.field.as_deref()) {
        Ok(unwrapped) => unwrapped,
        Err(diagnostic) => return Outcome::failed(vec![diagnostic], Some(input.label)),
    };
    let opts = AdfToMdOptions {
        flavor: args.md_flavor.into(),
        limits: Limits::default(),
    };
    let conversion = match crate::adf_to_md(&unwrapped.doc, &opts) {
        Ok(conversion) => conversion,
        Err(diagnostics) => {
            return Outcome::failed(
                prefix_pointers(diagnostics.0, &unwrapped.pointer),
                Some(input.label),
            );
        }
    };
    let warnings = prefix_pointers(conversion.warnings, &unwrapped.pointer);
    if args.strict && !warnings.is_empty() {
        return Outcome::failed(
            strict_failure(warnings, LOSSY_CONVERSION),
            Some(input.label),
        );
    }
    Outcome {
        stdout: single_trailing_newline(conversion.output).into_bytes(),
        output_file: args.output.clone(),
        diagnostics: warnings,
        label: Some(input.label),
        ..Outcome::default()
    }
}

/// Parses `-e`/`--field` into an envelope (usage error on failure).
pub(crate) fn parse_envelope(
    envelope: Option<EnvelopeArg>,
    field: Option<&str>,
) -> Result<Envelope, Box<Outcome>> {
    let name = envelope.map_or("doc", EnvelopeArg::name);
    Envelope::parse(name, field).map_err(|message| {
        Box::new(Outcome::usage(
            &message,
            "See `adf explain envelopes` for the envelopes and their fields",
        ))
    })
}

/// The target for `-t`: explicit, or implied by the envelope.
pub(crate) fn resolve_target(target: TargetArg, envelope: &Envelope) -> Target {
    target
        .explicit()
        .unwrap_or_else(|| envelope.default_target())
}

/// Markdown → ADF options from the flags.
pub(crate) fn markdown_options(markdown: &MarkdownArgs, target: Target) -> MdToAdfOptions {
    MdToAdfOptions {
        target,
        softbreak: markdown.softbreak.into(),
        html: markdown.html.into(),
        images: markdown.images.into(),
        extensions: markdown.extensions,
        limits: Limits::default(),
    }
}

/// Reads the input; with no input argument and a terminal on stdin, this is a usage error
/// instead of a silent wait.
pub(crate) fn read_input(path: Option<&Path>, command: &str) -> Result<Input, Box<Outcome>> {
    if path.is_none() && output::stdin_is_terminal() {
        return Err(Box::new(Outcome::usage(
            "no input: pass a file name, or pipe the input into stdin",
            &format!("Usage: adf {command} [INPUT|-] [OPTIONS]; see `adf {command} --help`"),
        )));
    }
    input::read(path, &Limits::default())
        .map_err(|diagnostic| Box::new(Outcome::failed(vec![diagnostic], Some(input::label(path)))))
}

/// Parses JSON input and finds the document in it.
pub(crate) fn parse_and_unwrap(
    text: &str,
    field: Option<&str>,
) -> Result<envelope::Unwrapped, Diagnostic> {
    let value = input::parse_json(text)?;
    envelope::unwrap(&value, field)
}

/// Prefixes every pointer with `prefix` (the document's position inside its envelope).
pub(crate) fn prefix_pointers(diagnostics: Vec<Diagnostic>, prefix: &str) -> Vec<Diagnostic> {
    if prefix.is_empty() {
        return diagnostics;
    }
    diagnostics
        .into_iter()
        .map(|mut diagnostic| {
            if let Some(pointer) = diagnostic
                .location
                .as_mut()
                .and_then(|location| location.pointer.as_mut())
            {
                *pointer = format!("{prefix}{pointer}");
            }
            diagnostic
        })
        .collect()
}

/// The diagnostics for `--strict` with warnings: the warnings, then `E_LOSSY` (the only error,
/// so it sets the exit code). `template` is the message, with `{codes}` replaced by the warning
/// codes.
pub(crate) fn strict_failure(mut warnings: Vec<Diagnostic>, template: &str) -> Vec<Diagnostic> {
    let mut codes: Vec<&str> = Vec::new();
    for warning in &warnings {
        if !codes.contains(&warning.code.as_str()) {
            codes.push(warning.code.as_str());
        }
    }
    let lossy = Diagnostic::new(Code::Lossy, template.replace("{codes}", &codes.join(", ")))
        .with_hint("Fix the input so the warnings disappear, or drop --strict");
    warnings.push(lossy);
    warnings
}

/// A successful Markdown conversion.
#[derive(Debug)]
pub(crate) struct Converted {
    /// The bare document.
    pub doc: Value,
    /// Conversion warnings plus `W_SIZE_LIMIT` / `W_EMPTY_DOCUMENT` from the self-check.
    pub warnings: Vec<Diagnostic>,
}

/// Markdown → ADF → self-check. On failure returns every diagnostic (errors and warnings); the
/// first error determines the exit code.
///
/// Errors found by the self-check inside nodes spliced from ```` ```adf ```` blocks are the
/// user's (`E_RAW_ADF`, exit 3); anywhere else they are a bug in this tool (`E_INTERNAL`).
pub(crate) fn markdown_pipeline(
    text: &str,
    opts: &MdToAdfOptions,
) -> Result<Converted, Vec<Diagnostic>> {
    self_checked(crate::md_to_adf(text, opts), opts.target)
}

/// The self-check half of [`markdown_pipeline`], given the converter's result.
fn self_checked(
    converted: Result<Conversion<Doc>, Diagnostics>,
    target: Target,
) -> Result<Converted, Vec<Diagnostic>> {
    let conversion = converted.map_err(|diagnostics| diagnostics.0)?;
    let raw = raw_pointers(&conversion.output);
    let mut warnings = conversion.warnings;
    let doc = match conversion.output.to_value() {
        Ok(doc) => doc,
        Err(err) => {
            let mut all = vec![Diagnostic::new(
                Code::Internal,
                format!("the ADF document could not be serialized: {err}"),
            )];
            all.append(&mut warnings);
            return Err(all);
        }
    };
    let (errors, checks): (Vec<Diagnostic>, Vec<Diagnostic>) = crate::validate_adf(&doc, target)
        .into_iter()
        .partition(Diagnostic::is_error);
    if !errors.is_empty() {
        let mut all = self_check_failure(errors, &raw, target);
        all.append(&mut warnings);
        return Err(all);
    }
    for check in checks {
        if !warnings.iter().any(|warning| warning.code == check.code) {
            warnings.push(check);
        }
    }
    Ok(Converted { doc, warnings })
}

/// Turns self-check errors into `E_INTERNAL` (our bug) and `E_RAW_ADF` (inside raw blocks).
fn self_check_failure(errors: Vec<Diagnostic>, raw: &[String], target: Target) -> Vec<Diagnostic> {
    let in_raw = |diagnostic: &Diagnostic| {
        diagnostic
            .location
            .as_ref()
            .and_then(|location| location.pointer.as_deref())
            .is_some_and(|pointer| {
                raw.iter().any(|root| {
                    pointer == root
                        || pointer
                            .strip_prefix(root.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                })
            })
    };
    // A document too deep to check, or a validator failure, is reported as it is.
    let (mut out, errors): (Vec<Diagnostic>, Vec<Diagnostic>) = errors
        .into_iter()
        .partition(|error| matches!(error.code, Code::InputTooDeep | Code::Internal));
    // Errors the validator counted but did not list (it lists at most 100 per code) have no
    // location; they are ours unless every listed error lies in a raw block.
    let (summaries, errors): (Vec<Diagnostic>, Vec<Diagnostic>) = errors
        .into_iter()
        .partition(|error| omitted_count(error).is_some());
    let unlisted = reported_count(&summaries);
    let (raw_errors, own_errors): (Vec<Diagnostic>, Vec<Diagnostic>) =
        errors.into_iter().partition(in_raw);
    let raw_unlisted = if own_errors.is_empty() { unlisted } else { 0 };
    let own_unlisted = unlisted - raw_unlisted;
    if !own_errors.is_empty() {
        let mut details: Vec<String> = own_errors
            .iter()
            .take(5)
            .map(|error| {
                let at = error
                    .location
                    .as_ref()
                    .and_then(|location| location.pointer.as_deref())
                    .map(|pointer| format!(" at {pointer}"))
                    .unwrap_or_default();
                format!("{}{at}: {}", error.code, error.message)
            })
            .collect();
        let more = own_errors.len() - details.len() + own_unlisted;
        if more > 0 {
            details.push(format!("and {more} more"));
        }
        out.push(
            Diagnostic::new(
                Code::Internal,
                "adf produced ADF that fails its own validation; this is a bug",
            )
            .with_hint(format!(
                "{}. Please report it at https://github.com/Yornik/adf-converter/issues with the input",
                details.join("; ")
            )),
        );
    }
    for error in raw_errors {
        let message = format!(
            "an ```adf block is not valid for {}: {}",
            target.as_str(),
            error.message
        );
        let mut diagnostic = Diagnostic::new(Code::RawAdf, message);
        diagnostic.location = error.location;
        diagnostic.hint = error.hint;
        out.push(diagnostic);
    }
    if raw_unlisted > 0 {
        out.push(diagnostics::omitted(Code::RawAdf, raw_unlisted));
    }
    out
}

/// JSON Pointers of the nodes spliced verbatim from ```` ```adf ```` blocks.
fn raw_pointers(doc: &Doc) -> Vec<String> {
    let mut walk = RawWalk { out: Vec::new() };
    for (index, block) in doc.content.iter().enumerate() {
        walk.doc_block(block, &format!("/content/{index}"), 0);
    }
    walk.out
}

/// Walks the typed model looking for `Raw` nodes. The model is built by this crate with bounded
/// nesting; the depth check is a safety net.
struct RawWalk {
    out: Vec<String>,
}

const MAX_MODEL_DEPTH: usize = 512;

impl RawWalk {
    fn doc_block(&mut self, block: &DocBlock, pointer: &str, depth: usize) {
        match block {
            DocBlock::Raw(_) => self.out.push(pointer.to_owned()),
            DocBlock::BulletList(list) => self.bullet_list(list, pointer, depth),
            DocBlock::OrderedList(list) => self.ordered_list(list, pointer, depth),
            DocBlock::Blockquote(quote) => {
                for (index, child) in quote.content.iter().enumerate() {
                    self.blockquote_child(child, &format!("{pointer}/content/{index}"), depth + 1);
                }
            }
            DocBlock::Panel(panel) => {
                for (index, child) in panel.content.iter().enumerate() {
                    self.panel_child(child, &format!("{pointer}/content/{index}"), depth + 1);
                }
            }
            DocBlock::Table(table) => self.table(table, pointer, depth),
            DocBlock::Expand(expand) => {
                for (index, child) in expand.content.iter().enumerate() {
                    self.expand_child(child, &format!("{pointer}/content/{index}"), depth + 1);
                }
            }
            DocBlock::Paragraph(_)
            | DocBlock::Heading(_)
            | DocBlock::TaskList(_)
            | DocBlock::CodeBlock(_)
            | DocBlock::Rule(_)
            | DocBlock::MediaSingle(_) => {}
        }
    }

    fn bullet_list(&mut self, list: &BulletList, pointer: &str, depth: usize) {
        self.items(list.items.as_slice(), pointer, depth);
    }

    fn ordered_list(&mut self, list: &OrderedList, pointer: &str, depth: usize) {
        self.items(list.items.as_slice(), pointer, depth);
    }

    fn items(&mut self, items: &[ListItem], pointer: &str, depth: usize) {
        if depth > MAX_MODEL_DEPTH {
            return;
        }
        for (index, item) in items.iter().enumerate() {
            let item_pointer = format!("{pointer}/content/{index}");
            // Child 0 is the leading paragraph.
            for (rest, child) in item.rest.iter().enumerate() {
                let child_pointer = format!("{item_pointer}/content/{}", rest + 1);
                match child {
                    ListItemChild::Raw(_) => self.out.push(child_pointer),
                    ListItemChild::BulletList(list) => {
                        self.bullet_list(list, &child_pointer, depth + 2);
                    }
                    ListItemChild::OrderedList(list) => {
                        self.ordered_list(list, &child_pointer, depth + 2);
                    }
                    ListItemChild::Paragraph(_)
                    | ListItemChild::TaskList(_)
                    | ListItemChild::CodeBlock(_)
                    | ListItemChild::MediaSingle(_) => {}
                }
            }
        }
    }

    fn blockquote_child(&mut self, child: &BlockquoteChild, pointer: &str, depth: usize) {
        match child {
            BlockquoteChild::Raw(_) => self.out.push(pointer.to_owned()),
            BlockquoteChild::BulletList(list) => self.bullet_list(list, pointer, depth),
            BlockquoteChild::OrderedList(list) => self.ordered_list(list, pointer, depth),
            BlockquoteChild::Paragraph(_)
            | BlockquoteChild::CodeBlock(_)
            | BlockquoteChild::MediaSingle(_) => {}
        }
    }

    fn panel_child(&mut self, child: &PanelChild, pointer: &str, depth: usize) {
        match child {
            PanelChild::Raw(_) => self.out.push(pointer.to_owned()),
            PanelChild::BulletList(list) => self.bullet_list(list, pointer, depth),
            PanelChild::OrderedList(list) => self.ordered_list(list, pointer, depth),
            PanelChild::Paragraph(_)
            | PanelChild::Heading(_)
            | PanelChild::TaskList(_)
            | PanelChild::CodeBlock(_)
            | PanelChild::Rule(_)
            | PanelChild::MediaSingle(_) => {}
        }
    }

    fn table(&mut self, table: &Table, pointer: &str, depth: usize) {
        if depth > MAX_MODEL_DEPTH {
            return;
        }
        for (r, row) in table.rows.iter().enumerate() {
            for (c, cell) in row.cells.iter().enumerate() {
                for (index, child) in cell.content.iter().enumerate() {
                    let child_pointer =
                        format!("{pointer}/content/{r}/content/{c}/content/{index}");
                    self.table_cell_child(child, &child_pointer, depth + 3);
                }
            }
        }
    }

    fn table_cell_child(&mut self, child: &TableCellChild, pointer: &str, depth: usize) {
        match child {
            TableCellChild::Raw(_) => self.out.push(pointer.to_owned()),
            TableCellChild::BulletList(list) => self.bullet_list(list, pointer, depth),
            TableCellChild::OrderedList(list) => self.ordered_list(list, pointer, depth),
            TableCellChild::Blockquote(quote) => {
                for (index, child) in quote.content.iter().enumerate() {
                    self.blockquote_child(child, &format!("{pointer}/content/{index}"), depth + 1);
                }
            }
            TableCellChild::Panel(panel) => {
                for (index, child) in panel.content.iter().enumerate() {
                    self.panel_child(child, &format!("{pointer}/content/{index}"), depth + 1);
                }
            }
            TableCellChild::NestedExpand(expand) => {
                for (index, child) in expand.content.iter().enumerate() {
                    self.nested_expand_child(
                        child,
                        &format!("{pointer}/content/{index}"),
                        depth + 1,
                    );
                }
            }
            TableCellChild::Paragraph(_)
            | TableCellChild::Heading(_)
            | TableCellChild::TaskList(_)
            | TableCellChild::CodeBlock(_)
            | TableCellChild::Rule(_)
            | TableCellChild::MediaSingle(_) => {}
        }
    }

    fn expand_child(&mut self, child: &ExpandChild, pointer: &str, depth: usize) {
        match child {
            ExpandChild::Raw(_) => self.out.push(pointer.to_owned()),
            ExpandChild::BulletList(list) => self.bullet_list(list, pointer, depth),
            ExpandChild::OrderedList(list) => self.ordered_list(list, pointer, depth),
            ExpandChild::Blockquote(quote) => {
                for (index, child) in quote.content.iter().enumerate() {
                    self.blockquote_child(child, &format!("{pointer}/content/{index}"), depth + 1);
                }
            }
            ExpandChild::Panel(panel) => {
                for (index, child) in panel.content.iter().enumerate() {
                    self.panel_child(child, &format!("{pointer}/content/{index}"), depth + 1);
                }
            }
            ExpandChild::Table(table) => self.table(table, pointer, depth),
            ExpandChild::NestedExpand(expand) => {
                for (index, child) in expand.content.iter().enumerate() {
                    self.nested_expand_child(
                        child,
                        &format!("{pointer}/content/{index}"),
                        depth + 1,
                    );
                }
            }
            ExpandChild::Paragraph(_)
            | ExpandChild::Heading(_)
            | ExpandChild::TaskList(_)
            | ExpandChild::CodeBlock(_)
            | ExpandChild::Rule(_)
            | ExpandChild::MediaSingle(_) => {}
        }
    }

    fn nested_expand_child(&mut self, child: &NestedExpandChild, pointer: &str, depth: usize) {
        match child {
            NestedExpandChild::Raw(_) => self.out.push(pointer.to_owned()),
            NestedExpandChild::BulletList(list) => self.bullet_list(list, pointer, depth),
            NestedExpandChild::OrderedList(list) => self.ordered_list(list, pointer, depth),
            NestedExpandChild::Blockquote(quote) => {
                for (index, child) in quote.content.iter().enumerate() {
                    self.blockquote_child(child, &format!("{pointer}/content/{index}"), depth + 1);
                }
            }
            NestedExpandChild::Panel(panel) => {
                for (index, child) in panel.content.iter().enumerate() {
                    self.panel_child(child, &format!("{pointer}/content/{index}"), depth + 1);
                }
            }
            NestedExpandChild::Paragraph(_)
            | NestedExpandChild::Heading(_)
            | NestedExpandChild::TaskList(_)
            | NestedExpandChild::CodeBlock(_)
            | NestedExpandChild::Rule(_)
            | NestedExpandChild::MediaSingle(_) => {}
        }
    }
}

/// Pretty JSON for `--pretty`, compact for `--compact`, files and pipes; pretty on a terminal.
fn pretty_output(args: &ConvertArgs, output_file: Option<&Path>) -> bool {
    if args.pretty {
        true
    } else if args.compact || output_file.is_some_and(|path| !is_stdout(path)) {
        false
    } else {
        output::stdout_is_terminal()
    }
}

/// `-o -` means stdout.
pub(crate) fn is_stdout(path: &Path) -> bool {
    path.as_os_str() == "-"
}

/// Serializes JSON output with a trailing newline.
pub(crate) fn json_bytes(value: &Value, pretty: bool) -> Vec<u8> {
    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    };
    // Serializing a `Value` cannot fail (keys are strings, numbers are finite).
    let mut bytes = text.unwrap_or_default().into_bytes();
    bytes.push(b'\n');
    bytes
}

/// Markdown output always ends with exactly one newline.
fn single_trailing_newline(mut markdown: String) -> String {
    let trimmed = markdown.trim_end_matches(['\n', '\r']).len();
    markdown.truncate(trimmed);
    markdown.push('\n');
    markdown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adf::model::{AlignedParagraph, Paragraph, RawNode};
    use crate::diagnostics::Location;
    use serde_json::json;

    #[test]
    fn newlines() {
        assert_eq!(single_trailing_newline("a\n\n\n".into()), "a\n");
        assert_eq!(single_trailing_newline("a".into()), "a\n");
        assert_eq!(single_trailing_newline(String::new()), "\n");
    }

    #[test]
    fn pointers_are_prefixed() {
        let diagnostics = vec![
            Diagnostic::new(Code::AdfPassthrough, "x").at(Location::pointer("/content/0")),
            Diagnostic::new(Code::AttrDropped, "y"),
        ];
        let out = prefix_pointers(diagnostics, "/body");
        assert_eq!(
            out[0].location.as_ref().unwrap().pointer.as_deref(),
            Some("/body/content/0")
        );
        assert_eq!(out[1].location, None);
    }

    #[test]
    fn strict_failure_lists_codes_once() {
        let warnings = vec![
            Diagnostic::new(Code::ImageAsLink, "a"),
            Diagnostic::new(Code::ImageAsLink, "b"),
            Diagnostic::new(Code::HtmlAsText, "c"),
        ];
        let out = strict_failure(warnings, LOSSY_CONVERSION);
        assert_eq!(out[3].code, Code::Lossy);
        assert_eq!(
            out[3].message,
            "--strict: the conversion is lossy (W_IMAGE_AS_LINK, W_HTML_AS_TEXT); nothing was written"
        );
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn raw_nodes_are_found_everywhere() {
        let raw = || RawNode(json!({"type":"rule"}));
        let doc = Doc {
            content: vec![
                DocBlock::Paragraph(AlignedParagraph::default()),
                DocBlock::Raw(raw()),
                DocBlock::BulletList(BulletList {
                    items: crate::adf::model::NonEmpty::new(ListItem {
                        first: Paragraph::default(),
                        rest: vec![ListItemChild::Raw(raw())],
                    }),
                }),
            ],
        };
        assert_eq!(
            raw_pointers(&doc),
            ["/content/1", "/content/2/content/0/content/1"]
        );
    }

    #[test]
    fn self_check_errors_are_attributed() {
        let raw = vec!["/content/1".to_owned()];
        let errors = vec![
            Diagnostic::new(Code::Target, "layout").at(Location::pointer("/content/1")),
            Diagnostic::new(Code::Schema, "bad").at(Location::pointer("/content/10")),
            Diagnostic::new(Code::Schema, "inner").at(Location::pointer("/content/1/content/0")),
        ];
        let out = self_check_failure(errors, &raw, Target::Jira);
        let codes: Vec<Code> = out.iter().map(|d| d.code).collect();
        assert_eq!(codes, [Code::Internal, Code::RawAdf, Code::RawAdf]);
        assert!(
            out[0]
                .hint
                .as_deref()
                .unwrap()
                .contains("E_SCHEMA at /content/10: bad")
        );
        let deep = vec![Diagnostic::new(Code::InputTooDeep, "deep").at(Location::pointer(""))];
        let out = self_check_failure(deep, &raw, Target::Jira);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, Code::InputTooDeep);
    }

    /// The validator lists at most 100 errors per code and summarises the rest without a
    /// location; the summary must not be mistaken for a bug in adf.
    #[test]
    fn self_check_attributes_unlisted_errors() {
        let raw = vec!["/content/1".to_owned()];
        let inner = |i: usize| {
            Diagnostic::new(Code::Schema, "inner")
                .at(Location::pointer(format!("/content/1/content/{i}")))
        };
        let errors = vec![inner(0), inner(1), diagnostics::omitted(Code::Schema, 7)];
        let out = self_check_failure(errors, &raw, Target::Jira);
        assert_eq!(codes(&out), [Code::RawAdf, Code::RawAdf, Code::RawAdf]);
        assert_eq!(omitted_count(&out[2]), Some(7));
        // With an error of our own among the listed ones, the rest count towards the bug report.
        let own = Diagnostic::new(Code::Schema, "bad").at(Location::pointer("/content/0"));
        let errors = vec![own, inner(0), diagnostics::omitted(Code::Schema, 7)];
        let out = self_check_failure(errors, &raw, Target::Jira);
        assert_eq!(codes(&out), [Code::Internal, Code::RawAdf]);
        let hint = out[0].hint.as_deref().unwrap();
        assert!(hint.contains("and 7 more"), "{hint}");
    }

    /// A successful converter result, shaped like `md_to_adf`'s.
    #[allow(clippy::unnecessary_wraps)]
    fn converted(
        content: Vec<DocBlock>,
        warnings: Vec<Diagnostic>,
    ) -> Result<Conversion<Doc>, Diagnostics> {
        Ok(Conversion {
            output: Doc { content },
            warnings,
        })
    }

    fn paragraph(text: &str) -> DocBlock {
        DocBlock::Paragraph(AlignedParagraph {
            content: vec![crate::adf::model::Inline::Text(
                crate::adf::model::Text::plain(text).unwrap(),
            )],
            align: None,
        })
    }

    fn codes(diagnostics: &[Diagnostic]) -> Vec<Code> {
        diagnostics.iter().map(|d| d.code).collect()
    }

    #[test]
    fn self_check_passes_valid_output_through() {
        let warning = Diagnostic::new(Code::ImageAsLink, "image");
        let out =
            self_checked(converted(vec![paragraph("x")], vec![warning]), Target::Jira).unwrap();
        assert_eq!(codes(&out.warnings), [Code::ImageAsLink]);
        assert_eq!(out.doc["content"][0]["type"], "paragraph");
    }

    #[test]
    fn self_check_adds_document_warnings_once() {
        let out = self_checked(converted(Vec::new(), Vec::new()), Target::Jira).unwrap();
        assert_eq!(codes(&out.warnings), [Code::EmptyDocument]);
        let already = Diagnostic::new(Code::EmptyDocument, "from the converter");
        let out = self_checked(converted(Vec::new(), vec![already]), Target::Jira).unwrap();
        assert_eq!(codes(&out.warnings), [Code::EmptyDocument]);
        assert_eq!(out.warnings[0].message, "from the converter");
    }

    #[test]
    fn self_check_blames_raw_blocks_on_the_user() {
        let layout = RawNode(json!({"type":"layoutSection","content":[
            {"type":"layoutColumn","attrs":{"width":50},"content":[{"type":"paragraph"}]},
            {"type":"layoutColumn","attrs":{"width":50},"content":[{"type":"paragraph"}]}]}));
        let result = self_checked(
            converted(
                vec![paragraph("x"), DocBlock::Raw(layout.clone())],
                Vec::new(),
            ),
            Target::Jira,
        );
        let errors = result.unwrap_err();
        assert_eq!(codes(&errors), [Code::RawAdf]);
        assert_eq!(
            errors[0].location.as_ref().unwrap().pointer.as_deref(),
            Some("/content/1")
        );
        assert!(
            errors[0].message.contains("layoutSection"),
            "{}",
            errors[0].message
        );
        // Confluence accepts layouts.
        let out = self_checked(
            converted(vec![DocBlock::Raw(layout)], Vec::new()),
            Target::Confluence,
        )
        .unwrap();
        assert_eq!(out.doc["content"][0]["type"], "layoutSection");
    }

    #[test]
    fn self_check_reports_invalid_output_as_a_bug() {
        use crate::adf::model::{TaskItem, TaskList, TaskState};
        // The typed model cannot express most mistakes, but it can repeat a localId.
        let item = TaskItem {
            local_id: "same".into(),
            state: TaskState::Todo,
            content: Vec::new(),
        };
        let list = DocBlock::TaskList(TaskList {
            local_id: "same".into(),
            first: item,
            rest: Vec::new(),
        });
        let errors = self_checked(converted(vec![list], Vec::new()), Target::Jira).unwrap_err();
        assert_eq!(codes(&errors), [Code::Internal]);
        assert!(
            errors[0]
                .hint
                .as_deref()
                .unwrap()
                .contains("E_INVARIANT at /content/0/content/0/attrs/localId"),
            "{:?}",
            errors[0].hint
        );
    }

    #[test]
    fn converter_failures_keep_every_diagnostic() {
        let warning = Diagnostic::new(Code::ImageAsLink, "image");
        let failed: Result<Conversion<Doc>, Diagnostics> = Err(Diagnostics(vec![
            warning,
            Diagnostic::new(Code::Lossy, "html"),
        ]));
        let errors = self_checked(failed, Target::Jira).unwrap_err();
        assert_eq!(codes(&errors), [Code::ImageAsLink, Code::Lossy]);
    }

    #[test]
    fn json_output() {
        assert_eq!(json_bytes(&json!({"a":1}), false), b"{\"a\":1}\n");
        assert_eq!(json_bytes(&json!({"a":1}), true), b"{\n  \"a\": 1\n}\n");
    }
}
