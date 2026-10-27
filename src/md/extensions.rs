//! ADF extensions to Markdown: `adf:` inline links and ```` ```adf ```` raw blocks.

use serde_json::{Value, json};

use crate::adf::model::{
    ALLOWED_URL_FORMS, AttachmentFile, Date, Emoji, Href, Inline, InlineCard, MediaInline, Mention,
    NonEmptyString, Status, StatusColor,
};
use crate::diagnostics::{Code, Diagnostic};
use crate::options::{Extensions, Target};

use super::media;

/// The kinds of `adf:` link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkExt {
    Mention,
    Status,
    Date,
    Emoji,
    Card,
    Attachment,
}

impl LinkExt {
    /// The kind: the word after `adf:`.
    fn name(self) -> &'static str {
        match self {
            Self::Mention => "mention",
            Self::Status => "status",
            Self::Date => "date",
            Self::Emoji => "emoji",
            Self::Card => "card",
            Self::Attachment => "attachment",
        }
    }

    /// The `--extensions` name that switches the kind on (`adf:attachment` shares `media`).
    fn extension(self) -> &'static str {
        match self {
            Self::Attachment => "media",
            Self::Mention | Self::Status | Self::Date | Self::Emoji | Self::Card => self.name(),
        }
    }

    fn enabled(self, ext: &Extensions) -> bool {
        match self {
            Self::Mention => ext.mention,
            Self::Status => ext.status,
            Self::Date => ext.date,
            Self::Emoji => ext.emoji,
            Self::Card => ext.card,
            Self::Attachment => ext.media,
        }
    }
}

/// Splits `adf:KIND[/ARG]` (scheme and kind are case-insensitive). `None` if not an `adf:` URL.
pub(crate) fn split_adf_url(url: &str) -> Option<(String, Option<&str>)> {
    let scheme = url.get(..4)?;
    if !scheme.eq_ignore_ascii_case("adf:") {
        return None;
    }
    let rest = url.get(4..)?;
    Some(match rest.split_once('/') {
        Some((kind, arg)) => (kind.to_ascii_lowercase(), Some(arg)),
        None => (rest.to_ascii_lowercase(), None),
    })
}

/// Why an `adf:` link stays plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fallback {
    pub(crate) code: Code,
    pub(crate) message: String,
    /// What to write instead, when the message alone does not say.
    pub(crate) hint: Option<&'static str>,
}

impl Fallback {
    fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
        }
    }

    fn with_hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
        self
    }
}

/// Builds the inline node for an `adf:` link with the given (plain-text) label.
pub(crate) fn build_link_node(
    kind: &str,
    arg: Option<&str>,
    label: &str,
    ext: &Extensions,
) -> Result<Inline, Fallback> {
    let which = match kind {
        "mention" => LinkExt::Mention,
        "status" => LinkExt::Status,
        "date" => LinkExt::Date,
        "emoji" => LinkExt::Emoji,
        "card" => LinkExt::Card,
        "attachment" => LinkExt::Attachment,
        "media" => {
            return Err(Fallback::new(
                Code::UnknownExtension,
                "adf:media is an image form, not a link kind; write ![alt](adf:media/<uuid>) instead",
            ));
        }
        "csv" => {
            return Err(Fallback::new(
                Code::UnknownExtension,
                "adf:csv is an image form, not a link kind; write ![alt](adf:csv/PATH) instead",
            ));
        }
        other => {
            return Err(Fallback::new(
                Code::UnknownExtension,
                format!(
                    "Unknown adf: link kind '{}'; expected mention, status, date, emoji, card or attachment",
                    truncate(other, 40)
                ),
            ));
        }
    };
    if !which.enabled(ext) {
        return Err(Fallback::new(
            Code::ExtensionDisabled,
            format!(
                "adf:{} link found, but the '{}' extension is disabled; kept as text",
                which.name(),
                which.extension()
            ),
        ));
    }
    match which {
        LinkExt::Mention => mention(arg, label),
        LinkExt::Status => status(arg, label),
        LinkExt::Date => date(arg, label),
        LinkExt::Emoji => emoji(arg, label),
        LinkExt::Card => card(arg, label),
        LinkExt::Attachment => attachment(arg, label),
    }
}

fn invalid(message: impl Into<String>) -> Fallback {
    Fallback::new(Code::InvalidExtensionArg, message)
}

/// The label as optional display text.
fn label_text(label: &str) -> Option<String> {
    (!label.is_empty()).then(|| label.to_owned())
}

fn mention(arg: Option<&str>, label: &str) -> Result<Inline, Fallback> {
    let id = arg
        .map(str::trim)
        .filter(|id| !id.chars().any(char::is_control))
        .and_then(NonEmptyString::new)
        .ok_or_else(|| invalid("adf:mention needs an account ID: adf:mention/<accountId>"))?;
    Ok(Inline::Mention(Mention {
        id,
        text: label_text(label),
    }))
}

fn status(arg: Option<&str>, label: &str) -> Result<Inline, Fallback> {
    let color = match arg.map(str::trim).filter(|c| !c.is_empty()) {
        None => StatusColor::Neutral,
        Some(c) => StatusColor::parse(&c.to_ascii_lowercase()).ok_or_else(|| {
            invalid(format!(
                "Unknown status colour '{}'; expected neutral, purple, blue, red, yellow or green",
                truncate(c, 40)
            ))
        })?,
    };
    // The label is kept verbatim (round trip); it only has to contain something visible.
    let text = NonEmptyString::new(label)
        .filter(|t| !t.as_str().trim().is_empty())
        .ok_or_else(|| invalid("adf:status needs a non-empty label"))?;
    Ok(Inline::Status(Status { text, color }))
}

fn date(arg: Option<&str>, label: &str) -> Result<Inline, Fallback> {
    if arg.is_some_and(|a| !a.is_empty()) {
        return Err(invalid(
            "adf:date takes no argument; write the date as the label: [YYYY-MM-DD](adf:date)",
        ));
    }
    let ms = parse_date_ms(label.trim()).ok_or_else(|| {
        invalid(format!(
            "'{}' is not a valid date; adf:date labels must be YYYY-MM-DD",
            truncate(label, 40)
        ))
    })?;
    let timestamp =
        NonEmptyString::new(ms.to_string()).ok_or_else(|| invalid("empty timestamp"))?;
    Ok(Inline::Date(Date { timestamp }))
}

fn emoji(arg: Option<&str>, label: &str) -> Result<Inline, Fallback> {
    let raw = arg.map(str::trim).unwrap_or_default();
    let name = raw.strip_prefix(':').unwrap_or(raw);
    let name = name.strip_suffix(':').unwrap_or(name);
    if name.is_empty()
        || name
            .chars()
            .any(|c| c == ':' || c.is_whitespace() || c.is_control())
    {
        return Err(invalid(
            "adf:emoji needs a short name such as adf:emoji/:smile:",
        ));
    }
    let short_name = NonEmptyString::new(format!(":{name}:"))
        .ok_or_else(|| invalid("empty emoji short name"))?;
    Ok(Inline::Emoji(Emoji {
        short_name,
        id: None,
        text: label_text(label),
    }))
}

fn card(arg: Option<&str>, label: &str) -> Result<Inline, Fallback> {
    if arg.is_some_and(|a| !a.is_empty()) {
        return Err(invalid(
            "adf:card takes no argument; write the URL as the label: [https://…](adf:card)",
        ));
    }
    let url = Href::parse(label).ok_or_else(|| {
        invalid(format!(
            "adf:card labels must be an allowed URL ({ALLOWED_URL_FORMS})"
        ))
    })?;
    Ok(Inline::InlineCard(InlineCard { url }))
}

/// `[file name](adf:attachment/<uuid>?collection=<c>)`. The link text is the file name.
fn attachment(arg: Option<&str>, label: &str) -> Result<Inline, Fallback> {
    let hinted = |message: String| invalid(message).with_hint(media::ATTACHMENT_HINT);
    let parsed = media::parse_attachment(arg).map_err(hinted)?;
    let alt = NonEmptyString::new(label.trim())
        .ok_or_else(|| hinted("adf:attachment needs the file name as the link text".to_owned()))?;
    Ok(Inline::MediaInline(MediaInline {
        file: AttachmentFile {
            id: parsed.id,
            collection: parsed.collection,
            alt,
        },
    }))
}

/// `YYYY-MM-DD` → milliseconds since the Unix epoch at 00:00 UTC.
pub(crate) fn parse_date_ms(label: &str) -> Option<i64> {
    let bytes = label.as_bytes();
    if bytes.len() != 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = label.get(range)?;
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let year = digits(0..4)?;
    let month = digits(5..7)?;
    let day = digits(8..10)?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400_000)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses the body of a ```` ```adf ```` block: one JSON object or a non-empty array of objects.
pub(crate) fn parse_raw(body: &str) -> Result<Vec<Value>, String> {
    let max_depth = crate::Limits::default().max_json_depth();
    let value = crate::json::parse_bounded(body, max_depth).map_err(|e| match e {
        crate::json::JsonError::TooDeep { .. } => {
            format!("the JSON nests deeper than {max_depth} levels")
        }
        crate::json::JsonError::Syntax(e) => format!("invalid JSON: {e}"),
    })?;
    let nodes = match value {
        Value::Array(items) => items,
        object @ Value::Object(_) => vec![object],
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            return Err(
                "expected a JSON object (one block node) or an array of block nodes".into(),
            );
        }
    };
    if nodes.is_empty() {
        return Err("the array of nodes is empty".into());
    }
    for (i, node) in nodes.iter().enumerate() {
        if !node.get("type").is_some_and(Value::is_string) {
            return Err(format!("node {i} is not an ADF node (no \"type\" string)"));
        }
    }
    Ok(nodes)
}

/// The container a raw block ends up in, for placement validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawPlace {
    Doc,
    ListItem,
    Blockquote,
    Panel,
    TableCell,
    Expand,
    NestedExpand,
}

impl RawPlace {
    fn describe(self) -> &'static str {
        match self {
            Self::Doc => "the document",
            Self::ListItem => "a list item",
            Self::Blockquote => "a quote",
            Self::Panel => "a panel",
            Self::TableCell => "a table cell",
            Self::Expand => "an expand",
            Self::NestedExpand => "a nested expand",
        }
    }

    /// A minimal valid document holding `nodes` in this container, and the JSON Pointer of the
    /// first raw node inside it.
    fn wrap(self, nodes: &[Value]) -> (Value, String) {
        let nodes = Value::Array(nodes.to_vec());
        let para = json!({"type": "paragraph"});
        let (content, prefix) = match self {
            Self::Doc => (nodes, "/content".to_owned()),
            Self::ListItem => {
                let mut items = vec![para];
                if let Value::Array(raw) = nodes {
                    items.extend(raw);
                }
                (
                    json!([{"type": "bulletList", "content": [{"type": "listItem", "content": items}]}]),
                    "/content/0/content/0/content".to_owned(),
                )
            }
            Self::Blockquote => (
                json!([{"type": "blockquote", "content": nodes}]),
                "/content/0/content".to_owned(),
            ),
            Self::Panel => (
                json!([{"type": "panel", "attrs": {"panelType": "info"}, "content": nodes}]),
                "/content/0/content".to_owned(),
            ),
            Self::TableCell => (
                json!([{"type": "table", "attrs": {"isNumberColumnEnabled": false, "layout": "default"},
                        "content": [{"type": "tableRow", "content": [{"type": "tableCell", "content": nodes}]}]}]),
                "/content/0/content/0/content/0/content".to_owned(),
            ),
            Self::Expand => (
                json!([{"type": "expand", "attrs": {}, "content": nodes}]),
                "/content/0/content".to_owned(),
            ),
            Self::NestedExpand => (
                json!([{"type": "expand", "attrs": {}, "content": [{"type": "nestedExpand", "attrs": {}, "content": nodes}]}]),
                "/content/0/content/0/content".to_owned(),
            ),
        };
        (
            json!({"type": "doc", "version": 1, "content": content}),
            prefix,
        )
    }
}

/// Checks that `nodes` are valid ADF in `place`. The error describes the first problem, with a
/// JSON Pointer relative to the block's own JSON.
///
/// Uses the crate's own validator (schema, target rules and invariants). Never call
/// `jsonschema`'s `iter_errors` on untrusted input: on this schema's nested `anyOf`s its time and
/// memory grow exponentially with nesting depth (a 200-byte block reached tens of GB).
pub(crate) fn validate_raw(nodes: &[Value], place: RawPlace, target: Target) -> Result<(), String> {
    let (doc, prefix) = place.wrap(nodes);
    let pointer = |path: &str| relative_pointer(path, &prefix, place, nodes.len());
    let Some(error) = crate::adf::validate::validate(&doc, target)
        .into_iter()
        .find(Diagnostic::is_error)
    else {
        return Ok(());
    };
    let path = error
        .location
        .as_ref()
        .and_then(|l| l.pointer.clone())
        .unwrap_or_default();
    let Some(relative) = pointer(&path) else {
        // The container itself rejects the nodes: they are not allowed there.
        let types: Vec<&str> = nodes
            .iter()
            .filter_map(|n| n.get("type").and_then(Value::as_str))
            .collect();
        return Err(format!(
            "not valid ADF in {}: {} cannot be placed there",
            place.describe(),
            truncate(&types.join(", "), 80)
        ));
    };
    if error.code == Code::Schema {
        return Err(format!(
            "not valid ADF in {}{}: {}",
            place.describe(),
            at(&relative),
            truncate(&error.message, 200)
        ));
    }
    Err(format!(
        "invalid for {}{}: {} ({})",
        target.as_str(),
        at(&relative),
        truncate(&error.message, 200),
        error.code
    ))
}

/// ` (at /pointer)`, or nothing for the node itself.
fn at(relative: &str) -> String {
    if relative == "/" {
        String::new()
    } else {
        format!(" (at {relative})")
    }
}

/// Converts a JSON Pointer into the wrapper document to one relative to the block's own JSON
/// (`/…` for a single node, `/<index>/…` for an array). `None` if it points outside the nodes.
fn relative_pointer(path: &str, prefix: &str, place: RawPlace, count: usize) -> Option<String> {
    let first_index = usize::from(place == RawPlace::ListItem);
    let rest = path.strip_prefix(prefix)?.strip_prefix('/')?;
    let (index, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let index = index.parse::<usize>().ok()?.checked_sub(first_index)?;
    if index >= count {
        return None;
    }
    Some(match (count, tail.is_empty()) {
        (1, _) => format!("/{tail}"),
        (_, true) => format!("/{index}"),
        (_, false) => format!("/{index}/{tail}"),
    })
}

/// Shortens `s` to at most `max` characters for messages (diagnostics must not echo big input).
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(parse_date_ms("1970-01-01"), Some(0));
        assert_eq!(parse_date_ms("2026-10-19"), Some(1_792_368_000_000));
        assert_eq!(parse_date_ms("2024-02-29"), Some(1_709_164_800_000));
        assert_eq!(parse_date_ms("2000-03-01"), Some(951_868_800_000));
        assert_eq!(parse_date_ms("1969-12-31"), Some(-86_400_000));
        for bad in [
            "2023-02-29",
            "2026-13-01",
            "2026-00-10",
            "2026-1-01",
            "26-10-19",
            "2026/10/19",
            "2026-10-32",
            "1900-02-29",
            "+026-10-19",
            "",
        ] {
            assert_eq!(parse_date_ms(bad), None, "{bad}");
        }
    }

    #[test]
    fn urls() {
        assert_eq!(
            split_adf_url("adf:mention/abc"),
            Some(("mention".into(), Some("abc")))
        );
        assert_eq!(split_adf_url("ADF:Date"), Some(("date".into(), None)));
        assert_eq!(split_adf_url("https://x"), None);
        assert_eq!(split_adf_url("adf"), None);
    }

    #[test]
    fn link_nodes() {
        let ext = Extensions::all();
        let ok = |kind: &str, arg: Option<&str>, label: &str| {
            serde_json::to_value(build_link_node(kind, arg, label, &ext).unwrap()).unwrap()
        };
        assert_eq!(
            ok("mention", Some("5b10"), "@Jane"),
            json!({"type":"mention","attrs":{"id":"5b10","text":"@Jane"}})
        );
        assert_eq!(
            ok("status", None, "TODO"),
            json!({"type":"status","attrs":{"text":"TODO","color":"neutral"}})
        );
        assert_eq!(
            ok("status", Some("Green"), "DONE"),
            json!({"type":"status","attrs":{"text":"DONE","color":"green"}})
        );
        assert_eq!(
            ok("status", Some("blue"), " In progress "),
            json!({"type":"status","attrs":{"text":" In progress ","color":"blue"}})
        );
        assert_eq!(
            ok("emoji", Some("smile"), ""),
            json!({"type":"emoji","attrs":{"shortName":":smile:"}})
        );
        assert_eq!(
            ok("card", None, "https://x.atlassian.net/browse/A-1"),
            json!({"type":"inlineCard","attrs":{"url":"https://x.atlassian.net/browse/A-1"}})
        );
        let err = |kind: &str, arg: Option<&str>, label: &str| {
            build_link_node(kind, arg, label, &ext).unwrap_err().code
        };
        assert_eq!(err("bogus", None, "x"), Code::UnknownExtension);
        assert_eq!(err("status", Some("pink"), "x"), Code::InvalidExtensionArg);
        assert_eq!(err("status", None, " "), Code::InvalidExtensionArg);
        assert_eq!(err("date", None, "tomorrow"), Code::InvalidExtensionArg);
        assert_eq!(err("mention", None, "@x"), Code::InvalidExtensionArg);
        assert_eq!(err("emoji", Some("a b"), "x"), Code::InvalidExtensionArg);
        assert_eq!(err("card", None, "javascript:x"), Code::InvalidExtensionArg);
        let none = Extensions::none();
        assert_eq!(
            build_link_node("mention", Some("x"), "@x", &none)
                .unwrap_err()
                .code,
            Code::ExtensionDisabled
        );
    }

    #[test]
    fn raw_parsing_and_placement() {
        assert!(parse_raw("{\"type\":\"rule\"}").is_ok());
        assert!(parse_raw("[{\"type\":\"rule\"},{\"type\":\"rule\"}]").is_ok());
        assert!(parse_raw("[]").is_err());
        assert!(parse_raw("1").is_err());
        assert!(parse_raw("{\"x\":1}").is_err());
        assert!(parse_raw("{").is_err());
        let rule = vec![json!({"type":"rule"})];
        assert!(validate_raw(&rule, RawPlace::Doc, Target::Jira).is_ok());
        assert!(validate_raw(&rule, RawPlace::Panel, Target::Jira).is_ok());
        let err = validate_raw(&rule, RawPlace::ListItem, Target::Jira).unwrap_err();
        assert!(err.contains("list item"), "{err}");
        assert!(validate_raw(&rule, RawPlace::Blockquote, Target::Jira).is_err());
        let table = vec![
            json!({"type":"table","content":[{"type":"tableRow","content":[{"type":"tableCell","content":[{"type":"paragraph"}]}]}]}),
        ];
        assert!(validate_raw(&table, RawPlace::Doc, Target::Jira).is_ok());
        assert!(validate_raw(&table, RawPlace::TableCell, Target::Jira).is_err());
        assert!(validate_raw(&table, RawPlace::NestedExpand, Target::Jira).is_err());
        assert!(validate_raw(&table, RawPlace::Expand, Target::Jira).is_ok());
        let text = vec![json!({"type":"text","text":"x"})];
        assert!(validate_raw(&text, RawPlace::Doc, Target::Jira).is_err());
        let bad_attr = vec![
            json!({"type":"panel","attrs":{"panelType":"bogus"},"content":[{"type":"paragraph"}]}),
        ];
        let err = validate_raw(&bad_attr, RawPlace::Doc, Target::Jira).unwrap_err();
        assert!(err.starts_with("not valid ADF in the document"), "{err}");
    }
}
