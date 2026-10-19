//! ADF → Markdown (`adf convert --from adf`). Mapping and round-trip rules: `docs/syntax.md#adf--markdown`.
//!
//! The input is read leniently ([`crate::adf::reader`]) and rendered in one of two flavours:
//!
//! - **roundtrip** (default): the exact inverse of Markdown → ADF, so that
//!   `md_to_adf(adf_to_md(x)) == normalize(x)`. Whatever Markdown cannot express is written as a
//!   ```` ```adf ```` block holding the node's JSON (`W_ADF_PASSTHROUGH`) at the nearest position
//!   that can hold a block; attributes without a Markdown form are dropped with `W_ATTR_DROPPED`;
//!   `localId`s are dropped silently.
//! - **plain**: compact Markdown for people and LLMs. Lossy by design, without warnings.
//!
//! Inline content is verified by parsing it back with the same CommonMark parser the other
//! direction uses (see `to_md/verify.rs`), so delimiters, escapes and code spans always mean
//! what the ADF says.

mod block;
mod date;
mod escape;
mod inline;
mod passthrough;
mod table;
mod verify;

#[cfg(test)]
mod roundtrip_tests;
#[cfg(test)]
mod tests;

use serde_json::Value;

use crate::Conversion;
use crate::adf::reader::{self, Node};
use crate::diagnostics::{Code, Diagnostic, Diagnostics, Location};
use crate::options::{AdfToMdOptions, MdFlavor, Target};

/// At most this many `W_INPUT_SCHEMA` warnings are reported; the rest are summarised.
const MAX_SCHEMA_WARNINGS: usize = 20;

/// Converts a bare `doc` to Markdown. Warnings use pointers relative to the document root.
pub fn to_markdown(doc: &Value, opts: &AdfToMdOptions) -> Result<Conversion<String>, Diagnostics> {
    let root = reader::read_doc(doc, &opts.limits).map_err(Diagnostics::single)?;
    let mut warnings = schema_warnings(doc);
    let mut cx = Ctx {
        roundtrip: opts.flavor == MdFlavor::Roundtrip,
        max_depth: opts.limits.max_json_depth(),
        depth: 0,
        warnings: Vec::new(),
    };
    let output = block::render_doc(&root, &mut cx).map_err(|error| {
        let mut all = std::mem::take(&mut warnings);
        all.append(&mut cx.warnings);
        all.push(error);
        Diagnostics(all)
    })?;
    warnings.append(&mut cx.warnings);
    Ok(Conversion { output, warnings })
}

/// Schema problems in the input, as warnings: the conversion is lenient, but the user should
/// know the document would not validate.
fn schema_warnings(doc: &Value) -> Vec<Diagnostic> {
    let errors: Vec<Diagnostic> = crate::validate_adf(doc, Target::default())
        .into_iter()
        .filter(|d| d.code == Code::Schema)
        .collect();
    // The validator lists the first problems and summarises the rest; count those too.
    let total = crate::diagnostics::reported_count(&errors);
    let mut out: Vec<Diagnostic> = errors
        .into_iter()
        .filter(|d| crate::diagnostics::omitted_count(d).is_none())
        .take(MAX_SCHEMA_WARNINGS)
        .map(|d| Diagnostic {
            code: Code::InputSchema,
            message: format!("The input does not match the ADF schema: {}", d.message),
            location: d.location,
            hint: Some("The document was converted anyway; check the output".to_owned()),
        })
        .collect();
    if total > MAX_SCHEMA_WARNINGS {
        out.push(Diagnostic::new(
            Code::InputSchema,
            format!(
                "{} more schema problems in the input were not listed",
                total - MAX_SCHEMA_WARNINGS
            ),
        ));
    }
    out
}

/// Conversion state shared by the renderers.
#[derive(Debug)]
struct Ctx {
    roundtrip: bool,
    max_depth: usize,
    depth: usize,
    warnings: Vec<Diagnostic>,
}

/// Why a node was not rendered.
#[derive(Debug)]
enum Fail {
    /// Roundtrip only: the node (or something inside it, at `pointer`) has no Markdown form that
    /// parses back to the same ADF. The nearest container that can hold a ```` ```adf ```` block
    /// writes the node there instead.
    Unrepresentable { pointer: String, reason: String },
    /// Stops the conversion (too deep, internal error).
    Fatal(Diagnostic),
}

impl Fail {
    fn unrepresentable(pointer: &str, reason: &str) -> Self {
        Self::Unrepresentable {
            pointer: pointer.to_owned(),
            reason: reason.to_owned(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::Fatal(Diagnostic::new(Code::Internal, message))
    }
}

impl Ctx {
    /// `W_ATTR_DROPPED` (roundtrip only; plain is lossy by design).
    fn warn_attr_dropped(&mut self, pointer: &str, message: &str) {
        if self.roundtrip {
            self.warnings
                .push(Diagnostic::new(Code::AttrDropped, message).at(Location::pointer(pointer)));
        }
    }

    /// Warns about the attributes of `node` other than `known`, `localId` (always dropped
    /// silently) and attributes whose value carries no information (`null`, `""`, `false`, or a
    /// listed default value).
    fn drop_attrs(&mut self, node: &Node<'_>, known: &[&str], defaults: &[(&str, &str)]) {
        if !self.roundtrip {
            return;
        }
        let dropped: Vec<&str> = node
            .attr_names()
            .filter(|name| *name != "localId" && !known.contains(name))
            .filter(|name| {
                let value = node.attr(name);
                let empty = value.is_none_or(|v| {
                    v.is_null() || v.as_str() == Some("") || v.as_bool() == Some(false)
                });
                let default = defaults
                    .iter()
                    .any(|(n, d)| n == name && value.and_then(Value::as_str) == Some(*d));
                !empty && !default
            })
            .collect();
        if !dropped.is_empty() {
            let message = format!(
                "Attributes of {} without a Markdown form were dropped: {}",
                node.ty,
                dropped.join(", ")
            );
            self.warn_attr_dropped(&node.pointer, &message);
        }
    }

    /// Warns about block marks (`alignment`, `indentation`, `breakout`, …) on a block node.
    fn drop_block_marks(&mut self, node: &Node<'_>) {
        if !node.marks.is_empty() {
            let names: Vec<&str> = node.marks.iter().map(|m| m.ty).collect();
            let message = format!(
                "Marks of {} without a Markdown form were dropped: {}",
                node.ty,
                names.join(", ")
            );
            self.warn_attr_dropped(&format!("{}/marks", node.pointer), &message);
        }
    }

    /// `W_ADF_PASSTHROUGH` for a node written as a raw ```` ```adf ```` block.
    fn warn_passthrough(&mut self, node: &Node<'_>, pointer: &str, reason: &str) {
        let what = if node.ty.is_empty() {
            "a node without a type".to_owned()
        } else {
            format!("the {} node", node.ty)
        };
        let message = if pointer == node.pointer {
            format!("{what} was written as a raw ```adf block: {reason}")
        } else {
            format!("{what} was written as a raw ```adf block: {reason} (at {pointer})")
        };
        let mut message = message;
        if let Some(first) = message.get(..1) {
            message = first.to_uppercase() + message.get(1..).unwrap_or("");
        }
        self.warnings.push(
            Diagnostic::new(Code::AdfPassthrough, message)
                .at(Location::pointer(node.pointer.clone()))
                .with_hint(
                    "Markdown has no equivalent; edit the JSON in the block or leave it as is",
                ),
        );
    }

    /// Enters a container; fails beyond the nesting limit.
    fn enter(&mut self, node: &Node<'_>) -> Result<(), Fail> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return Err(Fail::Fatal(
                Diagnostic::new(
                    Code::InputTooDeep,
                    format!(
                        "The ADF document nests deeper than the limit of {} JSON levels",
                        self.max_depth
                    ),
                )
                .at(Location::pointer(node.pointer.clone())),
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}
