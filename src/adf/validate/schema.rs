//! The JSON-schema layer of validation (`E_SCHEMA`).
//!
//! The verdict comes from the vendored draft-04 schema compiled as a whole
//! ([`jsonschema::Validator::is_valid`], which is fast). Reporting *why* a document is invalid is
//! harder: the schema expresses "which children may appear here" as large `anyOf`s, and
//! `iter_errors` on nested `anyOf`s both produces unhelpful messages ("… is not valid under any of
//! the schemas listed in the 'anyOf' keyword", echoing the whole subtree) and takes time
//! exponential in the nesting depth (eight nested lists with one bad text node take a minute).
//!
//! So when the document is invalid, an [`Explainer`] walks it node by node instead:
//!
//! - every definition is also compiled *shallow*, with the `items` of its `content` removed, so
//!   validating a node only checks the node itself (type, attributes, marks, child count);
//! - the walker checks each child against the definitions its parent allows at that position,
//!   memoizing `(node, definition)` results, which makes the walk linear in the document size;
//! - an `anyOf` failure is reported as the most specific cause: "`heading` is not allowed inside
//!   `blockquote`", or the errors of the one definition whose `type` matches.
//!
//! The walker implements exactly the schema's semantics for the constructs the vendored schema
//! uses (`$ref`, `allOf`, `anyOf` of node definitions, single and tuple `items` under `content`);
//! a unit test cross-checks it against the whole-schema verdict.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::sync::LazyLock;

use jsonschema::error::ValidationErrorKind;
use jsonschema::{Draft, ValidationError, Validator};
use serde_json::{Map, Value};

/// One schema problem, located relative to the document root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Issue {
    pub pointer: String,
    pub message: String,
    pub hint: Option<String>,
}

type DefId = usize;

/// Constraints on the children (`content` items) of a node definition.
#[derive(Debug, Clone)]
enum ContentRule {
    /// Every child must match one of these definitions.
    Each(Vec<DefId>),
    /// Child `i` must match one of `items[i]`; children past the tuple are unconstrained
    /// (draft-04 tuple semantics without `additionalItems`; counts are checked by `maxItems`).
    Tuple(Vec<Vec<DefId>>),
}

impl ContentRule {
    fn candidates(&self, index: usize) -> Option<&[DefId]> {
        match self {
            Self::Each(defs) => Some(defs),
            Self::Tuple(items) => items.get(index).map(Vec::as_slice),
        }
    }
}

#[derive(Debug)]
struct Def {
    name: String,
    /// The `type` values the definition accepts (resolved through `allOf`); empty for unions and
    /// non-node definitions.
    types: Vec<String>,
    content: Vec<ContentRule>,
    /// The shallow validator (children not checked).
    shallow: Option<Validator>,
}

/// Node types that only exist in the experimental `stage-0.json` schema.
const STAGE0_NODES: &[&str] = &["multiBodiedExtension", "extensionFrame", "bodiedRule"];

/// The compiled schema plus the per-definition data the explainer needs.
#[derive(Debug)]
pub(crate) struct Engine {
    full: Validator,
    defs: Vec<Def>,
    root: DefId,
    known_types: BTreeSet<String>,
}

static ENGINE: LazyLock<Result<Engine, String>> =
    LazyLock::new(|| Engine::new(crate::ADF_SCHEMA_JSON));

/// The engine for the vendored schema, compiled on first use.
pub(crate) fn engine() -> Result<&'static Engine, &'static str> {
    ENGINE.as_ref().map_err(String::as_str)
}

impl Engine {
    /// Compiles `schema_text` (a draft-04 schema whose root is `{"$ref": "#/definitions/…"}`).
    pub(crate) fn new(schema_text: &str) -> Result<Self, String> {
        let schema: Value = serde_json::from_str(schema_text)
            .map_err(|err| format!("the ADF schema is not valid JSON: {err}"))?;
        let full = jsonschema::draft4::new(&schema)
            .map_err(|err| format!("the ADF schema does not compile: {err}"))?;
        let definitions = schema
            .get("definitions")
            .and_then(Value::as_object)
            .ok_or("the ADF schema has no definitions")?;
        let root_name = schema
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(ref_name)
            .ok_or("the ADF schema root is not a $ref to a definition")?;

        let ids: BTreeMap<&str, DefId> = definitions
            .keys()
            .enumerate()
            .map(|(id, name)| (name.as_str(), id))
            .collect();
        let resolver = Resolver {
            definitions,
            ids: &ids,
        };

        let shallow_schema = shallow(&schema, definitions);
        let map = jsonschema::options()
            .with_draft(Draft::Draft4)
            .build_map(&shallow_schema)
            .map_err(|err| format!("the shallow ADF schema does not compile: {err}"))?;

        let mut defs = Vec::with_capacity(definitions.len());
        let mut known_types = BTreeSet::new();
        for (name, def) in definitions {
            let types = resolver.types(def, 0);
            known_types.extend(types.iter().cloned());
            let content = if types.is_empty() {
                Vec::new()
            } else {
                resolver.content_rules(def, 0)?
            };
            defs.push(Def {
                name: name.clone(),
                types,
                content,
                shallow: map.get(&format!("#/definitions/{name}")).cloned(),
            });
        }
        let root = *ids
            .get(root_name)
            .ok_or("the ADF schema root definition is missing")?;
        Ok(Self {
            full,
            defs,
            root,
            known_types,
        })
    }

    /// Validates a document. Returns no issues if and only if the whole schema accepts it.
    pub(crate) fn check(&self, doc: &Value) -> Vec<Issue> {
        if self.full.is_valid(doc) {
            return Vec::new();
        }
        let mut explainer = Explainer {
            engine: self,
            memo: HashMap::new(),
            issues: Vec::new(),
        };
        explainer.explain(doc, &[self.root], "", None, 0);
        let mut issues = explainer.issues;
        if issues.is_empty() {
            // The walker always finds a cause for the constructs the vendored schema uses; keep a
            // located error anyway so an invalid document can never pass.
            issues.push(Issue {
                pointer: String::new(),
                message: "the document does not match the ADF schema".to_owned(),
                hint: None,
            });
        }
        let mut seen = BTreeSet::new();
        issues.retain(|issue| seen.insert((issue.pointer.clone(), issue.message.clone())));
        issues
    }
}

/// Extracts `name` from `#/definitions/name`.
fn ref_name(reference: &str) -> Option<&str> {
    reference.strip_prefix("#/definitions/")
}

/// Builds the shallow schema: the `items` under every `content` property are removed (including
/// those of definitions that are only used as `content`, like `table_cell_content`), so that
/// validating a node never descends into its children.
fn shallow(schema: &Value, definitions: &Map<String, Value>) -> Value {
    fn strip(def: &mut Value, content_defs: &mut BTreeSet<String>) {
        let Some(object) = def.as_object_mut() else {
            return;
        };
        if let Some(content) = object
            .get_mut("properties")
            .and_then(|properties| properties.get_mut("content"))
            .and_then(Value::as_object_mut)
        {
            if let Some(name) = content
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(ref_name)
            {
                content_defs.insert(name.to_owned());
            }
            content.remove("items");
            content.remove("additionalItems");
        }
        if let Some(Value::Array(members)) = object.get_mut("allOf") {
            for member in members {
                strip(member, content_defs);
            }
        }
    }

    let mut shallow = schema.clone();
    let mut content_defs = BTreeSet::new();
    if let Some(defs) = shallow
        .get_mut("definitions")
        .and_then(Value::as_object_mut)
    {
        for def in defs.values_mut() {
            strip(def, &mut content_defs);
        }
        for name in &content_defs {
            if let Some(def) = defs.get_mut(name).and_then(Value::as_object_mut) {
                def.remove("items");
                def.remove("additionalItems");
            }
        }
    }
    debug_assert_eq!(
        definitions.len(),
        shallow
            .get("definitions")
            .and_then(Value::as_object)
            .map_or(0, Map::len)
    );
    shallow
}

/// Resolves `$ref`s, unions and `allOf` chains of the raw schema.
struct Resolver<'a> {
    definitions: &'a Map<String, Value>,
    ids: &'a BTreeMap<&'a str, DefId>,
}

/// Guards the resolver against reference cycles in a (future) schema.
const MAX_RESOLVE_DEPTH: usize = 16;

impl Resolver<'_> {
    fn definition(&self, reference: &str) -> Option<&Value> {
        self.definitions.get(ref_name(reference)?)
    }

    /// The `type` enum of a definition, following `allOf` members.
    fn types(&self, def: &Value, depth: usize) -> Vec<String> {
        if depth > MAX_RESOLVE_DEPTH {
            return Vec::new();
        }
        if let Some(values) = def
            .pointer("/properties/type/enum")
            .and_then(Value::as_array)
        {
            return values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
        }
        def.get("allOf")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|member| member.get("$ref").and_then(Value::as_str))
            .filter_map(|reference| self.definition(reference))
            .map(|target| self.types(target, depth + 1))
            .find(|types| !types.is_empty())
            .unwrap_or_default()
    }

    /// All `content` constraints of a definition (its own and those of its `allOf` members).
    fn content_rules(&self, def: &Value, depth: usize) -> Result<Vec<ContentRule>, String> {
        if depth > MAX_RESOLVE_DEPTH {
            return Err("the ADF schema nests allOf too deeply".to_owned());
        }
        let mut rules = Vec::new();
        if let Some(content) = def.pointer("/properties/content") {
            let content = match content.get("$ref").and_then(Value::as_str) {
                Some(reference) => self
                    .definition(reference)
                    .ok_or_else(|| format!("unknown $ref {reference}"))?,
                None => content,
            };
            if content.get("additionalItems").is_some() {
                return Err("additionalItems under content is not supported".to_owned());
            }
            match content.get("items") {
                None => {}
                Some(Value::Array(tuple)) => {
                    let items = tuple
                        .iter()
                        .map(|item| self.flatten(item, depth))
                        .collect::<Result<Vec<_>, _>>()?;
                    rules.push(ContentRule::Tuple(items));
                }
                Some(item) => rules.push(ContentRule::Each(self.flatten(item, depth)?)),
            }
        }
        for member in def
            .get("allOf")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let member = match member.get("$ref").and_then(Value::as_str) {
                Some(reference) => self
                    .definition(reference)
                    .ok_or_else(|| format!("unknown $ref {reference}"))?,
                None => member,
            };
            rules.extend(self.content_rules(member, depth + 1)?);
        }
        Ok(rules)
    }

    /// The node definitions an `items` schema allows: a `$ref` (unions are expanded) or an
    /// `anyOf` of them.
    fn flatten(&self, schema: &Value, depth: usize) -> Result<Vec<DefId>, String> {
        if depth > MAX_RESOLVE_DEPTH {
            return Err("the ADF schema nests unions too deeply".to_owned());
        }
        let mut out = Vec::new();
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let name =
                ref_name(reference).ok_or_else(|| format!("unsupported $ref {reference}"))?;
            let target = self
                .definitions
                .get(name)
                .ok_or_else(|| format!("unknown $ref {reference}"))?;
            if is_union(target) {
                out.extend(self.flatten(target, depth + 1)?);
            } else {
                out.push(
                    *self
                        .ids
                        .get(name)
                        .ok_or_else(|| format!("unknown $ref {reference}"))?,
                );
            }
        } else if let Some(members) = schema.get("anyOf").and_then(Value::as_array) {
            for member in members {
                out.extend(self.flatten(member, depth + 1)?);
            }
        } else {
            return Err(format!("unsupported content items schema {schema}"));
        }
        Ok(out)
    }
}

/// A definition that is nothing but an `anyOf` (e.g. `inline_node`, `block_content`).
fn is_union(def: &Value) -> bool {
    def.as_object()
        .is_some_and(|object| object.len() == 1 && object.contains_key("anyOf"))
}

/// Walks an invalid document and records the most specific causes.
struct Explainer<'e> {
    engine: &'e Engine,
    /// `(node address, definition)` → valid. Addresses are stable: the document is borrowed for
    /// the whole walk.
    memo: HashMap<(usize, DefId), bool>,
    issues: Vec<Issue>,
}

/// The walker never recurses deeper than this; `validate` rejects deeper documents up front.
const MAX_WALK_DEPTH: usize = 1024;

impl Explainer<'_> {
    fn def(&self, id: DefId) -> Option<&Def> {
        self.engine.defs.get(id)
    }

    fn type_matches(&self, node: &Value, id: DefId) -> bool {
        let Some(ty) = node.get("type").and_then(Value::as_str) else {
            return false;
        };
        self.def(id)
            .is_some_and(|def| def.types.iter().any(|t| t == ty))
    }

    fn shallow_ok(&self, node: &Value, id: DefId) -> bool {
        self.def(id)
            .and_then(|def| def.shallow.as_ref())
            .is_some_and(|validator| validator.is_valid(node))
    }

    /// Whether `node` is valid against definition `id`, including its subtree.
    fn valid(&mut self, node: &Value, id: DefId, depth: usize) -> bool {
        if depth > MAX_WALK_DEPTH {
            return true;
        }
        let key = (std::ptr::from_ref(node) as usize, id);
        if let Some(&known) = self.memo.get(&key) {
            return known;
        }
        let ok = self.shallow_ok(node, id) && self.children_ok(node, id, depth);
        self.memo.insert(key, ok);
        ok
    }

    fn children_ok(&mut self, node: &Value, id: DefId, depth: usize) -> bool {
        let engine = self.engine;
        let Some(children) = node.get("content").and_then(Value::as_array) else {
            return true;
        };
        let Some(def) = engine.defs.get(id) else {
            return false;
        };
        for rule in &def.content {
            for (index, child) in children.iter().enumerate() {
                if let Some(candidates) = rule.candidates(index)
                    && !self.valid_any(child, candidates, depth + 1)
                {
                    return false;
                }
            }
        }
        true
    }

    fn valid_any(&mut self, node: &Value, candidates: &[DefId], depth: usize) -> bool {
        candidates
            .iter()
            .any(|&id| self.type_matches(node, id) && self.valid(node, id, depth))
    }

    fn issue(&mut self, pointer: String, message: String, hint: Option<String>) {
        self.issues.push(Issue {
            pointer,
            message,
            hint,
        });
    }

    /// Records why `node` (at `pointer`, a child of a `parent` node) matches none of
    /// `candidates`.
    fn explain(
        &mut self,
        node: &Value,
        candidates: &[DefId],
        pointer: &str,
        parent: Option<&str>,
        depth: usize,
    ) {
        if depth > MAX_WALK_DEPTH || self.valid_any(node, candidates, depth) {
            return;
        }
        let engine = self.engine;
        let Some(object) = node.as_object() else {
            let what = if parent.is_some() {
                "an ADF node"
            } else {
                "the document"
            };
            self.issue(
                pointer.to_owned(),
                format!("expected {what} (a JSON object), found {}", kind(node)),
                None,
            );
            return;
        };
        let ty = match object.get("type") {
            Some(Value::String(ty)) => ty.as_str(),
            Some(other) => {
                self.issue(
                    format!("{pointer}/type"),
                    format!("`type` must be a string, found {}", kind(other)),
                    None,
                );
                return;
            }
            None => {
                self.issue(
                    pointer.to_owned(),
                    "this node has no `type`".to_owned(),
                    Some("Every ADF node needs a \"type\", for example \"paragraph\"".to_owned()),
                );
                return;
            }
        };
        let typed: Vec<DefId> = candidates
            .iter()
            .copied()
            .filter(|&id| self.type_matches(node, id))
            .collect();
        if typed.is_empty() {
            self.not_allowed(pointer, ty, candidates, parent);
            return;
        }
        if ty == "text"
            && let Some(message) = code_mark_conflict(object)
        {
            self.issue(
                format!("{pointer}/marks"),
                message,
                Some("Split the text into separate nodes, or drop the other marks".to_owned()),
            );
            if object.get("text").and_then(Value::as_str) == Some("") {
                self.issue(
                    format!("{pointer}/text"),
                    "text nodes must not be empty".to_owned(),
                    Some("Remove the node or give it some text".to_owned()),
                );
            }
            return;
        }

        let chosen = if let Some(&id) = typed.iter().find(|&&id| self.shallow_ok(node, id)) {
            id
        } else {
            let mut best: Option<(DefId, Vec<Issue>)> = None;
            for &id in &typed {
                let found = self.shallow_issues(node, id, ty, pointer);
                if best.as_ref().is_none_or(|(_, b)| found.len() < b.len()) {
                    best = Some((id, found));
                }
            }
            let Some((id, found)) = best else {
                return;
            };
            self.issues.extend(found);
            id
        };

        let Some(children) = object.get("content").and_then(Value::as_array) else {
            return;
        };
        let Some(def) = engine.defs.get(chosen) else {
            return;
        };
        for rule in &def.content {
            for (index, child) in children.iter().enumerate() {
                if let Some(child_candidates) = rule.candidates(index) {
                    let child_pointer = format!("{pointer}/content/{index}");
                    self.explain(child, child_candidates, &child_pointer, Some(ty), depth + 1);
                }
            }
        }
    }

    /// `ty` is not among the types `candidates` allow.
    fn not_allowed(&mut self, pointer: &str, ty: &str, candidates: &[DefId], parent: Option<&str>) {
        let shown = code(ty);
        if !self.engine.known_types.contains(ty) {
            let message = if ty.is_empty() {
                "this node has an empty `type`".to_owned()
            } else if STAGE0_NODES.contains(&ty) {
                format!(
                    "{shown} is an experimental (stage-0) ADF node that is not in the published schema"
                )
            } else {
                format!("unknown node type {shown}")
            };
            self.issue(pointer.to_owned(), message, None);
            return;
        }
        let mut allowed: Vec<&str> = Vec::new();
        for &id in candidates {
            if let Some(def) = self.def(id) {
                for t in &def.types {
                    if !allowed.contains(&t.as_str()) {
                        allowed.push(t);
                    }
                }
            }
        }
        let (message, hint) = match parent {
            None => (
                format!("the root node must be `doc`, found {shown}"),
                Some(
                    "An ADF document is {\"type\":\"doc\",\"version\":1,\"content\":[…]}"
                        .to_owned(),
                ),
            ),
            Some(parent) => {
                let hint = if ty == "text" || ty == "hardBreak" {
                    format!(
                        "Inline content must be wrapped in a paragraph; allowed here: {}",
                        allowed.join(", ")
                    )
                } else {
                    format!("Allowed here: {}", allowed.join(", "))
                };
                (
                    format!("{shown} is not allowed inside `{parent}`"),
                    Some(hint),
                )
            }
        };
        self.issue(pointer.to_owned(), message, hint);
    }

    /// The node's own problems against definition `id`, as issues.
    fn shallow_issues(&self, node: &Value, id: DefId, ty: &str, pointer: &str) -> Vec<Issue> {
        let Some(def) = self.def(id) else {
            return Vec::new();
        };
        let Some(validator) = def.shallow.as_ref() else {
            return vec![Issue {
                pointer: pointer.to_owned(),
                message: format!(
                    "`{ty}` does not match the ADF schema definition `{}`",
                    def.name
                ),
                hint: None,
            }];
        };
        let mut found = Vec::new();
        for error in validator.iter_errors(node) {
            reduce(&error, node, ty, pointer, &mut found, 0);
        }
        found
    }
}

/// The text of a `code` + other-mark conflict on a text node, if there is one.
fn code_mark_conflict(text: &Map<String, Value>) -> Option<String> {
    let marks = text.get("marks")?.as_array()?;
    let types: Vec<&str> = marks
        .iter()
        .filter_map(|mark| mark.get("type").and_then(Value::as_str))
        .collect();
    if !types.contains(&"code") {
        return None;
    }
    let mut others: Vec<String> = Vec::new();
    for ty in types {
        if !matches!(ty, "code" | "link" | "annotation") {
            let shown = code(ty);
            if !others.contains(&shown) {
                others.push(shown);
            }
        }
    }
    (!others.is_empty()).then(|| {
        format!(
            "the `code` mark can only be combined with `link`, but this text also has {}",
            others.join(", ")
        )
    })
}

/// Turns one jsonschema error (from a shallow validator, so it never concerns children) into
/// issues, following the most relevant `anyOf` branch.
fn reduce(
    error: &ValidationError<'_>,
    node: &Value,
    ty: &str,
    pointer: &str,
    out: &mut Vec<Issue>,
    depth: usize,
) {
    let path = error.instance_path().as_str();
    let at = format!("{pointer}{path}");
    let instance: &Value = error.instance();
    if let ValidationErrorKind::AnyOf { context } | ValidationErrorKind::OneOfNotValid { context } =
        error.kind()
    {
        let type_path = format!("{path}/type");
        let relevant: Vec<&Vec<ValidationError<'static>>> = context
            .iter()
            .filter(|branch| {
                !branch.iter().any(|e| {
                    e.instance_path().as_str() == type_path
                        && matches!(
                            e.kind(),
                            ValidationErrorKind::Enum { .. } | ValidationErrorKind::Constant { .. }
                        )
                })
            })
            .collect();
        let best = relevant.into_iter().min_by_key(|branch| branch.len());
        match best {
            Some(branch) if depth < 8 && !branch.is_empty() => {
                for inner in branch {
                    reduce(inner, node, ty, pointer, out, depth + 1);
                }
            }
            _ => {
                let message = match instance.get("type").and_then(Value::as_str) {
                    Some(inner) if path.contains("/marks/") => {
                        format!("the {} mark is not allowed on `{ty}` here", code(inner))
                    }
                    Some(inner) => format!("{} is not allowed here", code(inner)),
                    None => format!(
                        "{} does not match any allowed form",
                        subject(path, node, ty)
                    ),
                };
                out.push(Issue {
                    pointer: at,
                    message,
                    hint: None,
                });
            }
        }
        return;
    }
    let (message, hint) = describe(error.kind(), path, instance, node, ty);
    out.push(Issue {
        pointer: at,
        message,
        hint,
    });
}

/// A message (and hint) for a non-`anyOf` error at `path` (relative to the node).
#[allow(clippy::wildcard_enum_match_arm)] // the remaining keywords are handled by `describe_limit`
fn describe(
    error_kind: &ValidationErrorKind,
    path: &str,
    instance: &Value,
    node: &Value,
    ty: &str,
) -> (String, Option<String>) {
    let subject = subject(path, node, ty);
    if instance.is_null()
        && matches!(
            error_kind,
            ValidationErrorKind::Type { .. }
                | ValidationErrorKind::Enum { .. }
                | ValidationErrorKind::Constant { .. }
        )
    {
        return (
            format!("{subject} must not be null"),
            Some("Omit the property instead of setting it to null".to_owned()),
        );
    }
    match error_kind {
        ValidationErrorKind::Required { property } => {
            let property = property.as_str().map_or_else(|| property.to_string(), code);
            let (owner, what) = owner(path, node, ty);
            (
                format!("{owner} is missing required {what} {property}"),
                None,
            )
        }
        ValidationErrorKind::AdditionalProperties { unexpected } => (
            unknown_keys(unexpected, path, node, ty),
            Some(CLOSED_HINT.to_owned()),
        ),
        ValidationErrorKind::Type { kind } => (
            format!(
                "{subject} must be {}, found {}",
                type_name(kind),
                self::kind(instance)
            ),
            None,
        ),
        ValidationErrorKind::Enum { options } => (
            format!(
                "{subject} has an invalid value {}; expected {}",
                preview(instance),
                options
                    .as_array()
                    .map_or_else(|| options.to_string(), |values| one_of(values))
            ),
            None,
        ),
        ValidationErrorKind::Constant { expected_value } => (
            format!("{subject} must be {}", preview(expected_value)),
            None,
        ),
        ValidationErrorKind::Pattern { pattern } => {
            (describe_pattern(&subject, pattern, instance), None)
        }
        other => describe_limit(other, &subject, path, instance, ty),
    }
}

const CLOSED_HINT: &str = "ADF objects are closed: remove keys the schema does not define";

/// The message for `additionalProperties` errors.
fn unknown_keys(unexpected: &[String], path: &str, node: &Value, ty: &str) -> String {
    match (path, unexpected) {
        ("", [only]) if only == "content" => format!("`{ty}` cannot have content"),
        ("", [only]) if only == "marks" => format!("`{ty}` cannot have marks"),
        ("", [only]) if only == "attrs" => format!("`{ty}` cannot have attributes"),
        _ => {
            let mut names: Vec<String> = unexpected.iter().take(5).map(|name| code(name)).collect();
            if unexpected.len() > names.len() {
                names.push("…".to_owned());
            }
            let (owner, what) = owner(path, node, ty);
            let what = match what {
                "property" => "properties",
                "attribute" => "attributes",
                _ => "keys",
            };
            format!("{owner} has unknown {what}: {}", names.join(", "))
        }
    }
}

/// The message for `pattern` errors (in ADF: colours and status colours).
fn describe_pattern(subject: &str, pattern: &str, instance: &Value) -> String {
    if pattern.contains("#[0-9a-fA-F]{6}") && pattern.contains("neutral") {
        format!(
            "{subject} is {}; expected neutral, purple, blue, red, yellow, green or #RRGGBB",
            preview(instance)
        )
    } else if pattern.contains("[0-9a-fA-F]") {
        format!(
            "{subject} must be a hex colour like #FF5630, found {}",
            preview(instance)
        )
    } else {
        format!(
            "{subject} {} does not match the pattern {}",
            preview(instance),
            code(pattern)
        )
    }
}

/// Messages for length, count and range limits, and a generic message for the keywords the ADF
/// schema does not use.
#[allow(clippy::wildcard_enum_match_arm)] // a generic message is right for any other keyword
fn describe_limit(
    error_kind: &ValidationErrorKind,
    subject: &str,
    path: &str,
    instance: &Value,
    ty: &str,
) -> (String, Option<String>) {
    let message = match error_kind {
        ValidationErrorKind::MinLength { .. } if path == "/text" => {
            return (
                "text nodes must not be empty".to_owned(),
                Some("Remove the node or give it some text".to_owned()),
            );
        }
        ValidationErrorKind::MinLength { limit: 1 } => format!("{subject} must not be empty"),
        ValidationErrorKind::MinLength { limit } => {
            format!("{subject} must be at least {limit} characters long")
        }
        ValidationErrorKind::MaxLength { limit } => {
            format!("{subject} must be at most {limit} characters long")
        }
        ValidationErrorKind::MaxItems { limit: 0 } if path == "/marks" => {
            return (
                format!("`{ty}` cannot have marks here"),
                Some("Remove the marks; this position only allows unmarked nodes".to_owned()),
            );
        }
        ValidationErrorKind::MinItems { limit } if path == "/content" => {
            format!("`{ty}` must contain at least {}", plural(*limit, "node"))
        }
        ValidationErrorKind::MaxItems { limit } if path == "/content" => {
            format!("`{ty}` can contain at most {}", plural(*limit, "node"))
        }
        ValidationErrorKind::MinItems { limit } => {
            format!("{subject} must have at least {}", plural(*limit, "item"))
        }
        ValidationErrorKind::MaxItems { limit } => {
            format!("{subject} must have at most {}", plural(*limit, "item"))
        }
        ValidationErrorKind::Minimum { limit }
        | ValidationErrorKind::ExclusiveMinimum { limit } => {
            format!(
                "{subject} is too small: {} (minimum {limit})",
                preview(instance)
            )
        }
        ValidationErrorKind::Maximum { limit }
        | ValidationErrorKind::ExclusiveMaximum { limit } => {
            format!(
                "{subject} is too large: {} (maximum {limit})",
                preview(instance)
            )
        }
        other => format!("{subject} is invalid ({} constraint)", other.keyword()),
    };
    (message, None)
}

/// The object an object-level error at `path` belongs to, and whether its keys are attributes or
/// properties: `("`heading`", "attribute")` for `/attrs`.
fn owner(path: &str, node: &Value, ty: &str) -> (String, &'static str) {
    let segments: Vec<&str> = path.split('/').skip(1).collect();
    match segments.as_slice() {
        [] => (format!("`{ty}`"), "property"),
        ["attrs"] => (format!("`{ty}`"), "attribute"),
        ["marks", _] => (subject(path, node, ty), "property"),
        ["marks", index, "attrs"] => (subject(&format!("/marks/{index}"), node, ty), "attribute"),
        _ => (subject(path, node, ty), "key"),
    }
}

/// Describes the value at `path` (relative to a node of type `ty`) for messages.
fn subject(path: &str, node: &Value, ty: &str) -> String {
    let segments: Vec<String> = path
        .split('/')
        .skip(1)
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    let mark_type = |index: &str| {
        node.get("marks")
            .and_then(|marks| marks.get(index.parse::<usize>().ok()?))
            .and_then(|mark| mark.get("type"))
            .and_then(Value::as_str)
            .map(code)
    };
    let seg: Vec<&str> = segments.iter().map(String::as_str).collect();
    match seg.as_slice() {
        [] => format!("`{ty}`"),
        ["attrs"] => format!("the attributes of `{ty}`"),
        ["attrs", name, rest @ ..] => {
            let mut name = (*name).to_owned();
            for part in rest {
                let _ = write!(name, "/{part}");
            }
            format!("attribute {} of `{ty}`", code(&name))
        }
        ["marks"] => format!("the marks of `{ty}`"),
        ["marks", index] => match mark_type(index) {
            Some(mark) => format!("the {mark} mark"),
            None => format!("mark {index} of `{ty}`"),
        },
        ["marks", index, "attrs"] => match mark_type(index) {
            Some(mark) => format!("the attributes of the {mark} mark"),
            None => format!("the attributes of mark {index}"),
        },
        ["marks", index, "attrs", name, ..] => match mark_type(index) {
            Some(mark) => format!("attribute {} of the {mark} mark", code(name)),
            None => format!("attribute {} of mark {index}", code(name)),
        },
        ["marks", index, "type"] => format!("the type of mark {index}"),
        ["content"] => format!("the content of `{ty}`"),
        ["text"] => "the text".to_owned(),
        [key] => format!("property {} of `{ty}`", code(key)),
        _ => format!("{} in `{ty}`", code(path)),
    }
}

/// `name` in backticks, escaped and shortened so input text cannot flood or garble a terminal.
pub(crate) fn code(name: &str) -> String {
    let mut out = String::from("`");
    for (count, c) in name.chars().enumerate() {
        if count == 40 {
            out.push('…');
            break;
        }
        if c.is_control() || c == '`' {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out.push('`');
    out
}

/// A short rendering of a value for messages: scalars as JSON (at most 40 characters), containers
/// by kind only.
pub(crate) fn preview(value: &Value) -> String {
    match value {
        Value::Array(_) | Value::Object(_) => kind(value).to_owned(),
        Value::String(text) if text.chars().count() > 40 => {
            let short: String = text.chars().take(40).collect();
            let quoted = Value::from(short).to_string();
            format!("{}…\"", quoted.get(..quoted.len() - 1).unwrap_or(&quoted))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.to_string(),
    }
}

/// "a string", "an object", … for messages.
pub(crate) fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn type_name(kind: &jsonschema::error::TypeKind) -> String {
    match kind {
        jsonschema::error::TypeKind::Single(ty) => article(&ty.to_string()),
        jsonschema::error::TypeKind::Multiple(types) => {
            let names: Vec<String> = types.iter().map(|ty| ty.to_string()).collect();
            format!("one of {}", names.join(", "))
        }
    }
}

fn article(name: &str) -> String {
    match name {
        "object" | "array" | "integer" => format!("an {name}"),
        "null" => "null".to_owned(),
        other => format!("a {other}"),
    }
}

fn one_of(values: &[Value]) -> String {
    let shown: Vec<String> = values.iter().take(12).map(preview).collect();
    match shown.as_slice() {
        [only] => only.clone(),
        _ => format!("one of {}", shown.join(", ")),
    }
}

fn plural(count: u64, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine() -> &'static Engine {
        super::engine().unwrap()
    }

    #[allow(clippy::needless_pass_by_value)]
    fn doc(content: Value) -> Value {
        json!({"type":"doc","version":1,"content":content})
    }

    #[allow(clippy::needless_pass_by_value)]
    fn para(inline: Value) -> Value {
        json!({"type":"paragraph","content":[inline]})
    }

    fn issues(value: &Value) -> Vec<(String, String)> {
        engine()
            .check(value)
            .into_iter()
            .map(|issue| (issue.pointer, issue.message))
            .collect()
    }

    #[test]
    fn compiles_every_definition() {
        let engine = engine();
        let schema: Value = serde_json::from_str(crate::ADF_SCHEMA_JSON).unwrap();
        assert_eq!(
            engine.defs.len(),
            schema["definitions"].as_object().unwrap().len()
        );
        assert_eq!(engine.defs[engine.root].name, "doc_node");
        for def in &engine.defs {
            assert!(def.shallow.is_some(), "{}", def.name);
        }
        assert!(engine.known_types.contains("paragraph"));
        assert!(engine.known_types.contains("mediaSingle"));
        assert!(!engine.known_types.contains("extensionFrame"));
    }

    #[test]
    fn valid_documents_have_no_issues() {
        for value in [
            doc(json!([])),
            doc(json!([para(
                json!({"type":"text","text":"hi","marks":[{"type":"strong"}]})
            )])),
            doc(
                json!([{"type":"mediaSingle","attrs":{"layout":"center"},"content":[
                {"type":"media","attrs":{"type":"external","url":"https://example.com/a.png"}}]}]),
            ),
        ] {
            assert!(issues(&value).is_empty(), "{value}");
        }
    }

    #[test]
    fn code_with_strong() {
        let value = doc(json!([para(
            json!({"type":"text","text":"x","marks":[{"type":"code"},{"type":"strong"}]})
        )]));
        let found = issues(&value);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "/content/0/content/0/marks");
        assert!(found[0].1.contains("`strong`"), "{}", found[0].1);
    }

    #[test]
    fn empty_text() {
        let value = doc(json!([para(json!({"type":"text","text":""}))]));
        assert_eq!(
            issues(&value),
            [(
                "/content/0/content/0/text".to_owned(),
                "text nodes must not be empty".to_owned()
            )]
        );
    }

    #[test]
    fn heading_in_blockquote() {
        let value =
            doc(json!([{"type":"blockquote","content":[{"type":"heading","attrs":{"level":1}}]}]));
        let found = engine().check(&value);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].pointer, "/content/0/content/0");
        assert_eq!(
            found[0].message,
            "`heading` is not allowed inside `blockquote`"
        );
        assert!(found[0].hint.as_deref().unwrap().contains("paragraph"));
    }

    #[test]
    fn table_in_list_item() {
        let value = doc(
            json!([{"type":"bulletList","content":[{"type":"listItem","content":[
                {"type":"paragraph"},
                {"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph"}]}]}]}
            ]}]}]),
        );
        assert_eq!(
            issues(&value),
            [(
                "/content/0/content/0/content/1".to_owned(),
                "`table` is not allowed inside `listItem`".to_owned()
            )]
        );
    }

    #[test]
    fn task_item_without_local_id() {
        let value = doc(
            json!([{"type":"taskList","attrs":{"localId":"l"},"content":[
            {"type":"taskItem","attrs":{"state":"TODO"}}]}]),
        );
        assert_eq!(
            issues(&value),
            [(
                "/content/0/content/0/attrs".to_owned(),
                "`taskItem` is missing required attribute `localId`".to_owned()
            )]
        );
    }

    #[test]
    fn inline_in_cell_and_unknown_types() {
        let value = doc(json!([
            {"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"text","text":"x"}]}]}]},
            {"type":"bogus"},
            {"type":"extensionFrame","content":[]},
            "text",
            {"content":[]}
        ]));
        let found = engine().check(&value);
        let pairs: Vec<(&str, &str)> = found
            .iter()
            .map(|issue| (issue.pointer.as_str(), issue.message.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                (
                    "/content/0/content/0/content/0/content/0",
                    "`text` is not allowed inside `tableCell`"
                ),
                ("/content/1", "unknown node type `bogus`"),
                (
                    "/content/2",
                    "`extensionFrame` is an experimental (stage-0) ADF node that is not in the published schema"
                ),
                (
                    "/content/3",
                    "expected an ADF node (a JSON object), found a string"
                ),
                ("/content/4", "this node has no `type`"),
            ]
        );
        assert!(
            found[0]
                .hint
                .as_deref()
                .unwrap()
                .starts_with("Inline content must be wrapped")
        );
    }

    #[test]
    fn attribute_problems() {
        let value = doc(json!([
            {"type":"heading","attrs":{"level":7},"content":[]},
            {"type":"codeBlock","attrs":{"language":null}},
            {"type":"panel","attrs":{"panelType":"shiny"},"content":[{"type":"paragraph"}]},
            {"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"link","attrs":{}}]}]},
            {"type":"paragraph","content":[{"type":"text","text":"x","marks":[{"type":"textColor","attrs":{"color":"red"}}]}]},
            {"type":"rule","content":[]},
            {"type":"doc"}
        ]));
        let found = issues(&value);
        let expected = [
            (
                "/content/0/attrs/level",
                "attribute `level` of `heading` is too large: 7 (maximum 6)",
            ),
            (
                "/content/1/attrs/language",
                "attribute `language` of `codeBlock` must not be null",
            ),
            (
                "/content/2/attrs/panelType",
                "attribute `panelType` of `panel` has an invalid value \"shiny\"; expected one of \"info\", \"note\", \"tip\", \"warning\", \"error\", \"success\", \"custom\"",
            ),
            (
                "/content/3/content/0/marks/0/attrs",
                "the `link` mark is missing required attribute `href`",
            ),
            (
                "/content/4/content/0/marks/0/attrs/color",
                "attribute `color` of the `textColor` mark must be a hex colour like #FF5630, found \"red\"",
            ),
            ("/content/5", "`rule` cannot have content"),
            ("/content/6", "`doc` is not allowed inside `doc`"),
        ];
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(p, m)| ((*p).to_owned(), (*m).to_owned()))
            .collect();
        assert_eq!(found, expected);
    }

    #[test]
    fn root_problems() {
        assert_eq!(
            issues(&json!([])),
            [(
                String::new(),
                "expected the document (a JSON object), found an array".to_owned()
            )]
        );
        assert_eq!(
            issues(&json!({"type":"paragraph"})),
            [(
                String::new(),
                "the root node must be `doc`, found `paragraph`".to_owned()
            )]
        );
        assert_eq!(
            issues(&json!({"type":"doc","version":2,"content":[]})),
            [(
                "/version".to_owned(),
                "property `version` of `doc` has an invalid value 2; expected 1".to_owned()
            )]
        );
    }

    #[test]
    fn unknown_keys() {
        let value = json!({"type":"doc","version":1,"extra":true,"content":[
            {"type":"heading","attrs":{"level":1,"color":"red"}},
            {"type":"paragraph","content":[{"type":"text","text":"x","marks":[
                {"type":"link","attrs":{"href":"https://a.b","target":"_blank"}}]}]},
            {"type":""}
        ]});
        assert_eq!(
            issues(&value),
            [
                (
                    String::new(),
                    "`doc` has unknown properties: `extra`".to_owned()
                ),
                (
                    "/content/0/attrs".to_owned(),
                    "`heading` has unknown attributes: `color`".to_owned()
                ),
                (
                    "/content/1/content/0/marks/0/attrs".to_owned(),
                    "the `link` mark has unknown attributes: `target`".to_owned()
                ),
                (
                    "/content/2".to_owned(),
                    "this node has an empty `type`".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn messages_do_not_echo_input() {
        let long = "x".repeat(1000);
        let value = doc(
            json!([{"type": long.clone()}, {"type":"panel","attrs":{"panelType": long.clone()},"content":[{"type":"paragraph"}]}]),
        );
        for (_, message) in issues(&value) {
            assert!(message.len() < 300, "{message}");
            assert!(!message.contains(&long));
        }
        let value = doc(json!([{"type":"\u{1b}[31mred"}]));
        let found = issues(&value);
        assert!(!found[0].1.contains('\u{1b}'), "{}", found[0].1);
    }

    /// Nested invalid documents used to take exponential time in `iter_errors`.
    #[test]
    fn deep_invalid_documents_are_fast() {
        let mut node = json!({"type":"paragraph","content":[{"type":"text","text":""}]});
        for _ in 0..60 {
            node = json!({"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph"}, node]}]});
        }
        let value = doc(json!([node]));
        let start = std::time::Instant::now();
        let found = issues(&value);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].0.ends_with("/content/1/content/0/text"),
            "{}",
            found[0].0
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }

    /// The walker must agree with the whole-schema verdict.
    #[test]
    fn walker_agrees_with_full_schema() {
        let engine = engine();
        let samples = [
            doc(json!([])),
            doc(json!([para(json!({"type":"text","text":"a"}))])),
            doc(
                json!([{"type":"mediaSingle","attrs":{"layout":"center"},"content":[
                {"type":"media","attrs":{"type":"external","url":"u"}},
                {"type":"caption","content":[{"type":"text","text":"c"}]}]}]),
            ),
            doc(json!([{"type":"mediaSingle","content":[
                {"type":"media","attrs":{"type":"external","url":"u"}},
                {"type":"media","attrs":{"type":"external","url":"u"}}]}])),
            doc(json!([{"type":"mediaSingle","attrs":{"layout":"center"},"content":[]}])),
            doc(json!([{"type":"layoutSection","content":[
                {"type":"layoutColumn","attrs":{"width":50},"content":[{"type":"paragraph"}]},
                {"type":"layoutColumn","attrs":{"width":50},"content":[{"type":"expand","content":[{"type":"paragraph"}]}]}]}])),
            doc(json!([{"type":"layoutSection","content":[
                {"type":"layoutColumn","attrs":{"width":50},"content":[{"type":"paragraph"}]}]}])),
            doc(
                json!([{"type":"expand","marks":[{"type":"breakout","attrs":{"mode":"wide"}}],"content":[{"type":"paragraph"}]}]),
            ),
            doc(
                json!([{"type":"table","content":[{"type":"tableRow","content":[{"type":"tableHeader","content":[
                {"type":"nestedExpand","attrs":{},"content":[{"type":"paragraph"}]}]}]}]}]),
            ),
            doc(
                json!([{"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","content":[
                {"type":"expand","content":[{"type":"paragraph"}]}]}]}]}]),
            ),
            doc(
                json!([{"type":"taskList","attrs":{"localId":"a"},"content":[
                {"type":"blockTaskItem","attrs":{"localId":"b","state":"DONE"},"content":[{"type":"paragraph"},{"type":"heading","attrs":{"level":1}}]}]}]),
            ),
            doc(
                json!([{"type":"paragraph","marks":[{"type":"alignment","attrs":{"align":"center"}}]}]),
            ),
            doc(
                json!([{"type":"bulletList","content":[{"type":"listItem","content":[
                {"type":"paragraph","marks":[{"type":"alignment","attrs":{"align":"center"}}]}]}]}]),
            ),
            doc(
                json!([{"type":"codeBlock","content":[{"type":"text","text":"x","marks":[{"type":"strong"}]}]}]),
            ),
            doc(
                json!([{"type":"decisionList","attrs":{"localId":"d"},"content":[
                {"type":"decisionItem","attrs":{"localId":"e","state":"DECIDED"},"content":[{"type":"hardBreak"}]}]}]),
            ),
        ];
        for value in samples {
            let verdict = engine.full.is_valid(&value);
            let mut explainer = Explainer {
                engine,
                memo: HashMap::new(),
                issues: Vec::new(),
            };
            let walked = explainer.valid_any(&value, &[engine.root], 0);
            assert_eq!(walked, verdict, "{value}");
            explainer.explain(&value, &[engine.root], "", None, 0);
            assert_eq!(
                explainer.issues.is_empty(),
                verdict,
                "{value}: {:?}",
                explainer.issues
            );
        }
    }

    mod generated {
        use super::*;
        use proptest::prelude::*;

        /// Node types the generator draws from: valid ones in every container, misplaced ones,
        /// and a few that are only valid in some positions.
        const TYPES: &[&str] = &[
            "paragraph",
            "heading",
            "bulletList",
            "orderedList",
            "listItem",
            "taskList",
            "taskItem",
            "blockquote",
            "panel",
            "table",
            "tableRow",
            "tableCell",
            "tableHeader",
            "expand",
            "nestedExpand",
            "codeBlock",
            "mediaSingle",
            "media",
            "caption",
            "rule",
            "layoutSection",
            "layoutColumn",
            "decisionList",
            "decisionItem",
            "text",
            "hardBreak",
            "mention",
            "status",
            "bogus",
        ];

        fn attrs_for(ty: &str, pick: u8) -> Option<Value> {
            let variant = pick % 3;
            Some(match (ty, variant) {
                ("heading", 0) => json!({"level": 2}),
                ("heading", 1) => json!({"level": 9}),
                ("orderedList", 0) => json!({"order": 3}),
                ("taskList", 0 | 1) => json!({"localId": "t"}),
                ("taskItem", 0 | 1) => json!({"localId": "i", "state": "TODO"}),
                ("decisionList", 0 | 1) => json!({"localId": "d"}),
                ("decisionItem", 0 | 1) => json!({"localId": "e", "state": "DECIDED"}),
                ("panel", 0) => json!({"panelType": "info"}),
                ("panel", 1) => json!({"panelType": "shiny"}),
                ("nestedExpand" | "expand", 0 | 1) => json!({"title": "t"}),
                ("mediaSingle", 0) => json!({"layout": "center"}),
                ("media", 0 | 1) => json!({"type": "external", "url": "https://a.b/c.png"}),
                ("layoutColumn", 0 | 1) => json!({"width": 50}),
                ("mention", 0 | 1) => json!({"id": "abc"}),
                ("status", 0) => json!({"text": "OK", "color": "green"}),
                ("codeBlock", 0) => json!({"language": "rust"}),
                ("tableCell" | "tableHeader", 0) => json!({"colspan": 1}),
                (_, 2) => json!({"bogus": null}),
                _ => return None,
            })
        }

        fn marks_for(ty: &str, pick: u8) -> Option<Value> {
            match (ty, pick % 6) {
                ("text", 0) => Some(json!([{"type": "strong"}])),
                ("text", 1) => Some(
                    json!([{"type": "code"}, {"type": "link", "attrs": {"href": "https://a.b"}}]),
                ),
                ("text", 2) => Some(json!([{"type": "code"}, {"type": "em"}])),
                ("paragraph" | "heading", 3) => {
                    Some(json!([{"type": "alignment", "attrs": {"align": "center"}}]))
                }
                (_, 4) => Some(json!([])),
                _ => None,
            }
        }

        fn node(ty: &str, pick: u8, content: Option<Vec<Value>>) -> Value {
            let mut object = serde_json::Map::new();
            object.insert("type".into(), Value::from(ty));
            if ty == "text" {
                object.insert(
                    "text".into(),
                    Value::from(if pick.is_multiple_of(7) { "" } else { "x" }),
                );
            }
            if let Some(attrs) = attrs_for(ty, pick) {
                object.insert("attrs".into(), attrs);
            }
            if let Some(content) = content {
                object.insert("content".into(), Value::Array(content));
            }
            if let Some(marks) = marks_for(ty, pick / 3) {
                object.insert("marks".into(), marks);
            }
            Value::Object(object)
        }

        fn tree() -> impl Strategy<Value = Value> {
            let leaf = (0..TYPES.len(), any::<u8>())
                .prop_map(|(index, pick)| node(TYPES[index], pick, None));
            leaf.prop_recursive(5, 48, 4, |inner| {
                (
                    0..TYPES.len(),
                    any::<u8>(),
                    prop::collection::vec(inner, 0..4),
                )
                    .prop_map(|(index, pick, children)| node(TYPES[index], pick, Some(children)))
            })
        }

        /// Guards against a vacuous property: the generator must produce both verdicts.
        #[test]
        fn generator_produces_valid_and_invalid_documents() {
            use proptest::strategy::ValueTree;
            use proptest::test_runner::TestRunner;
            let engine = super::engine();
            let strategy = prop::collection::vec(tree(), 0..4);
            let mut runner = TestRunner::deterministic();
            let (mut valid, mut invalid) = (0, 0);
            for _ in 0..2000 {
                let children = strategy.new_tree(&mut runner).unwrap().current();
                if engine.full.is_valid(&doc(Value::Array(children))) {
                    valid += 1;
                } else {
                    invalid += 1;
                }
            }
            assert!(
                valid > 100 && invalid > 100,
                "valid {valid}, invalid {invalid}"
            );
        }

        proptest! {
            #![proptest_config(ProptestConfig {
                cases: 2048,
                failure_persistence: None,
                ..ProptestConfig::default()
            })]

            /// The node-by-node walk accepts exactly what the whole schema accepts, and always
            /// explains a rejection.
            #[test]
            fn walker_matches_the_schema(children in prop::collection::vec(tree(), 0..4)) {
                let engine = super::engine();
                let value = doc(Value::Array(children));
                let verdict = engine.full.is_valid(&value);
                let mut explainer = Explainer { engine, memo: HashMap::new(), issues: Vec::new() };
                prop_assert_eq!(explainer.valid_any(&value, &[engine.root], 0), verdict, "{}", value);
                explainer.explain(&value, &[engine.root], "", None, 0);
                prop_assert_eq!(explainer.issues.is_empty(), verdict, "{} {:?}", value, explainer.issues);
                for issue in &explainer.issues {
                    prop_assert!(value.pointer(&issue.pointer).is_some(), "{} {}", value, issue.pointer);
                }
            }
        }
    }
}
