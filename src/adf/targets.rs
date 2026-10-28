//! Per-product rules: which nodes, marks and attributes Jira and Confluence accept beyond the
//! schema (`E_TARGET`).
//!
//! The vendored schema is the union of everything the Atlassian editors know. Jira rejects a part
//! of it: Confluence-only constructs (layouts, extensions/macros, placeholders), constructs that
//! were added to the schema recently while the server's schema version is unknown (rule C27 in the
//! research: `fontSize`, codeBlock `wrap`/`hideLineNumbers`, cell `valign`, hex status colours), and
//! nodes that are linked from Atlassian's structure page but not documented anywhere (their pages
//! return 404). Confluence accepts far more; its list only holds the undocumented nodes.

use serde_json::{Map, Value};

use crate::diagnostics::{Code, Diagnostic, Location};
use crate::options::Target;

const LAYOUTS: &str = "layouts are a Confluence feature";
const EXTENSIONS: &str = "extensions (macros) only work with an app installed in the product";
const UNDOCUMENTED: &str =
    "it is linked from Atlassian's ADF structure page but not documented for the REST APIs";
const STAGE0: &str = "it is an experimental (stage-0) ADF node";

/// Nodes Jira does not accept, with the reason.
const JIRA_NODES: &[(&str, &str)] = &[
    ("layoutSection", LAYOUTS),
    ("layoutColumn", LAYOUTS),
    ("extension", EXTENSIONS),
    ("bodiedExtension", EXTENSIONS),
    ("inlineExtension", EXTENSIONS),
    ("multiBodiedExtension", EXTENSIONS),
    ("extensionFrame", EXTENSIONS),
    ("syncBlock", UNDOCUMENTED),
    ("bodiedSyncBlock", UNDOCUMENTED),
    ("blockTaskItem", UNDOCUMENTED),
    (
        "placeholder",
        "placeholders are a Confluence template feature",
    ),
    ("bodiedRule", STAGE0),
];

/// Marks Jira does not accept, with the reason.
const JIRA_MARKS: &[(&str, &str)] = &[
    (
        "fontSize",
        "it was added to the ADF schema in 52.9.0 (May 2026) and Jira may not accept it yet",
    ),
    ("breakout", "it only applies to Confluence page layouts"),
    (
        "indentation",
        "it is an editor-only mark that Jira does not document",
    ),
    ("dataConsumer", "it only applies to Confluence extensions"),
    ("fragment", "it only applies to Confluence extensions"),
    ("border", UNDOCUMENTED),
];

/// Attributes Jira does not accept: (node types, attribute, reason).
const JIRA_ATTRS: &[(&[&str], &str, &str)] = &[
    (
        &["codeBlock"],
        "wrap",
        "it was added to the ADF schema in 53.0.0 (June 2026) and Jira may not accept it yet",
    ),
    (
        &["codeBlock"],
        "hideLineNumbers",
        "it was added to the ADF schema in 53.0.0 (June 2026) and Jira may not accept it yet",
    ),
    (
        &["tableCell", "tableHeader"],
        "valign",
        "it was added to the ADF schema in 56.0.0 (June 2026) and Jira may not accept it yet",
    ),
];

/// Nodes Confluence does not accept, with the reason.
const CONFLUENCE_NODES: &[(&str, &str)] = &[
    ("syncBlock", UNDOCUMENTED),
    ("bodiedSyncBlock", UNDOCUMENTED),
    ("blockTaskItem", UNDOCUMENTED),
];

/// The product name used in messages.
fn product(target: Target) -> &'static str {
    match target {
        Target::Jira => "Jira",
        Target::Confluence => "Confluence",
    }
}

fn blocked_nodes(target: Target) -> &'static [(&'static str, &'static str)] {
    match target {
        Target::Jira => JIRA_NODES,
        Target::Confluence => CONFLUENCE_NODES,
    }
}

fn lookup<'a>(table: &'a [(&'a str, &'a str)], name: &str) -> Option<&'a str> {
    table
        .iter()
        .find(|(blocked, _)| *blocked == name)
        .map(|(_, reason)| *reason)
}

/// Node types the target rejects (for documentation and `capabilities`).
pub fn rejected_nodes(target: Target) -> impl Iterator<Item = &'static str> {
    blocked_nodes(target).iter().map(|(name, _)| *name)
}

/// Mark types the target rejects.
pub fn rejected_marks(target: Target) -> impl Iterator<Item = &'static str> {
    let marks: &[(&str, &str)] = match target {
        Target::Jira => JIRA_MARKS,
        Target::Confluence => &[],
    };
    marks.iter().map(|(name, _)| *name)
}

/// Checks one node (type `ty`, at `pointer`) and its own marks and attributes. Returns `true` if
/// the node type itself is rejected; callers then skip target checks inside it, so a rejected
/// layout reports once instead of once per column.
pub(crate) fn check_node(
    target: Target,
    ty: &str,
    node: &Map<String, Value>,
    pointer: &str,
    out: &mut Vec<Diagnostic>,
) -> bool {
    if let Some(reason) = lookup(blocked_nodes(target), ty) {
        out.push(
            Diagnostic::new(
                Code::Target,
                format!("`{ty}` is not accepted by {}: {reason}", product(target)),
            )
            .at(Location::pointer(pointer))
            .with_hint(match target {
                Target::Jira => {
                    "Remove the node, or validate with -t confluence if the document is for Confluence"
                }
                Target::Confluence => "Remove the node",
            }),
        );
        return true;
    }
    if target == Target::Jira {
        check_jira_attrs(ty, node, pointer, out);
    }
    false
}

fn check_jira_attrs(ty: &str, node: &Map<String, Value>, pointer: &str, out: &mut Vec<Diagnostic>) {
    let Some(attrs) = node.get("attrs").and_then(Value::as_object) else {
        return;
    };
    for (types, attr, reason) in JIRA_ATTRS {
        if types.contains(&ty) && attrs.contains_key(*attr) {
            out.push(
                Diagnostic::new(
                    Code::Target,
                    format!("`{ty}` attribute `{attr}` is not accepted by Jira: {reason}"),
                )
                .at(Location::pointer(format!("{pointer}/attrs/{attr}")))
                .with_hint("Remove the attribute"),
            );
        }
    }
    if ty == "status"
        && let Some(color) = attrs.get("color").and_then(Value::as_str)
        && color.starts_with('#')
    {
        out.push(
            Diagnostic::new(
                Code::Target,
                "hex status colours are not accepted by Jira: they were added to the ADF schema in 57.5.0 (September 2026)",
            )
            .at(Location::pointer(format!("{pointer}/attrs/color")))
            .with_hint("Use one of neutral, purple, blue, red, yellow, green"),
        );
    }
}

/// Checks one mark (type `ty`, at `pointer`, e.g. `/content/0/marks/1`).
pub(crate) fn check_mark(target: Target, ty: &str, pointer: &str, out: &mut Vec<Diagnostic>) {
    if target != Target::Jira {
        return;
    }
    if let Some(reason) = lookup(JIRA_MARKS, ty) {
        out.push(
            Diagnostic::new(
                Code::Target,
                format!("the `{ty}` mark is not accepted by Jira: {reason}"),
            )
            .at(Location::pointer(pointer))
            .with_hint("Remove the mark"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(value: &Value) -> &Map<String, Value> {
        value.as_object().unwrap()
    }

    #[test]
    fn jira_rejects_confluence_nodes() {
        let mut out = Vec::new();
        let layout = json!({"type":"layoutSection","content":[]});
        assert!(check_node(
            Target::Jira,
            "layoutSection",
            node(&layout),
            "/content/0",
            &mut out
        ));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, Code::Target);
        assert_eq!(
            out[0].location.as_ref().unwrap().pointer.as_deref(),
            Some("/content/0")
        );
        assert!(
            out[0].message.contains("layoutSection"),
            "{}",
            out[0].message
        );

        let mut out = Vec::new();
        assert!(!check_node(
            Target::Confluence,
            "layoutSection",
            node(&layout),
            "/content/0",
            &mut out
        ));
        assert_eq!(out, Vec::<Diagnostic>::new());
    }

    #[test]
    fn jira_rejects_recent_attributes() {
        let mut out = Vec::new();
        let code = json!({"type":"codeBlock","attrs":{"language":"rust","wrap":true,"hideLineNumbers":false}});
        check_node(
            Target::Jira,
            "codeBlock",
            node(&code),
            "/content/2",
            &mut out,
        );
        let pointers: Vec<_> = out
            .iter()
            .map(|d| d.location.as_ref().unwrap().pointer.clone().unwrap())
            .collect();
        assert_eq!(
            pointers,
            ["/content/2/attrs/wrap", "/content/2/attrs/hideLineNumbers"]
        );

        let mut out = Vec::new();
        let cell = json!({"type":"tableHeader","attrs":{"valign":"top"},"content":[]});
        check_node(Target::Jira, "tableHeader", node(&cell), "/c", &mut out);
        assert_eq!(out.len(), 1);

        let mut out = Vec::new();
        check_node(Target::Confluence, "codeBlock", node(&code), "/c", &mut out);
        assert_eq!(out, Vec::<Diagnostic>::new());
    }

    #[test]
    fn status_colours() {
        let mut out = Vec::new();
        let hex = json!({"type":"status","attrs":{"text":"OK","color":"#00ff00"}});
        let named = json!({"type":"status","attrs":{"text":"OK","color":"green"}});
        check_node(Target::Jira, "status", node(&named), "/s", &mut out);
        assert_eq!(out, Vec::<Diagnostic>::new());
        check_node(Target::Jira, "status", node(&hex), "/s", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].location.as_ref().unwrap().pointer.as_deref(),
            Some("/s/attrs/color")
        );
    }

    #[test]
    fn marks() {
        let mut out = Vec::new();
        check_mark(Target::Jira, "fontSize", "/content/0/marks/0", &mut out);
        check_mark(Target::Jira, "strong", "/content/0/marks/1", &mut out);
        check_mark(
            Target::Confluence,
            "fontSize",
            "/content/0/marks/0",
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("fontSize"));
    }

    /// The target lists name what a product rejects; file media is on neither list. (Whether the
    /// products *render* `mediaGroup` and `mediaInline` written through the API is not verified.)
    #[test]
    fn file_media_nodes_are_not_rejected_by_either_product() {
        let group = json!({"type":"mediaGroup","content":[{"type":"media","attrs":{"type":"file","id":"66666666-7777-8888-9999-000000000000","collection":"","alt":"a.pdf"}}]});
        let inline = json!({"type":"mediaInline","attrs":{"type":"file","id":"66666666-7777-8888-9999-000000000000","collection":"","alt":"a.pdf"}});
        for target in [Target::Jira, Target::Confluence] {
            for (ty, value) in [("mediaGroup", &group), ("mediaInline", &inline)] {
                let mut out = Vec::new();
                assert!(!check_node(target, ty, node(value), "/n", &mut out));
                assert_eq!(out, Vec::<Diagnostic>::new());
                assert!(!rejected_nodes(target).any(|n| n == ty));
            }
        }
    }

    #[test]
    fn listings() {
        assert!(rejected_nodes(Target::Jira).any(|n| n == "extension"));
        assert!(!rejected_nodes(Target::Confluence).any(|n| n == "extension"));
        assert!(rejected_marks(Target::Jira).any(|m| m == "fontSize"));
        assert_eq!(rejected_marks(Target::Confluence).count(), 0);
    }
}
