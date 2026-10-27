//! The argument of an `adf:media` image or an `adf:attachment` link: `<id>` or
//! `<id>?key=value&key=value`.
//!
//! The id is a UUID. The keys of an image are `collection`, `width`, `height`, `layout` and
//! `size`; an attachment takes `collection` only. Each key is allowed at most once, in any order.
//! There is no percent-decoding. Every failure is one sentence that says which part is wrong, for
//! `W_INVALID_EXTENSION_ARG`.

use crate::adf::model::{
    FileMedia, MediaCollection, MediaDimension, MediaId, MediaLayout, MediaSize,
};

use super::extensions::truncate;

/// Which `adf:` form an argument belongs to: decides the name in messages and the keys allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    /// `![alt](adf:media/<arg>)`.
    Image,
    /// `[file name](adf:attachment/<arg>)`.
    Attachment,
}

impl Form {
    fn name(self) -> &'static str {
        match self {
            Self::Image => "adf:media",
            Self::Attachment => "adf:attachment",
        }
    }

    fn keys(self) -> &'static str {
        match self {
            Self::Image => "collection, width, height, layout or size",
            Self::Attachment => "collection",
        }
    }
}

/// What `adf:attachment/<arg>` stands for, before the link text (the file name) is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub(crate) id: MediaId,
    pub(crate) collection: MediaCollection,
}

/// The syntax, shown as the hint of every invalid attachment argument.
pub(crate) const ATTACHMENT_HINT: &str = "Write [file name](adf:attachment/<uuid>?collection=<c>); the file name is the link text and \
     the collection is optional";

/// What `adf:media/<arg>` stands for, before the alt text is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileImage {
    pub(crate) file: FileMedia,
    pub(crate) layout: MediaLayout,
    pub(crate) size: Option<MediaSize>,
}

/// The syntax, shown as the hint of every invalid argument.
pub(crate) const SYNTAX_HINT: &str = "Write ![alt](adf:media/<uuid>?collection=<c>&width=<n>&height=<n>&layout=<l>&size=<n>% or <n>px); \
     all parameters are optional";

/// Parses the text after `adf:media/` (`None` when there was no `/`).
pub(crate) fn parse(arg: Option<&str>) -> Result<FileImage, String> {
    let (id, params) = split(Form::Image, arg)?;
    Ok(FileImage {
        file: FileMedia {
            id,
            collection: params.collection.unwrap_or_default(),
            width: params.width,
            height: params.height,
        },
        layout: params.layout.unwrap_or_default(),
        size: params.size,
    })
}

/// Parses the text after `adf:attachment/` (`None` when there was no `/`): a UUID and, optionally,
/// `?collection=<c>`. The id and the collection follow the same rules as for `adf:media`.
pub(crate) fn parse_attachment(arg: Option<&str>) -> Result<Attachment, String> {
    let (id, params) = split(Form::Attachment, arg)?;
    Ok(Attachment {
        id,
        collection: params.collection.unwrap_or_default(),
    })
}

/// The id and the query parameters of an argument.
fn split(form: Form, arg: Option<&str>) -> Result<(MediaId, Params), String> {
    let name = form.name();
    let arg = arg.unwrap_or_default();
    let (id, query) = match arg.split_once('?') {
        Some((id, query)) => (id, Some(query)),
        None => (arg, None),
    };
    if id.is_empty() {
        return Err(format!("{name} needs the file's media ID: {name}/<uuid>"));
    }
    let id = MediaId::parse(id).ok_or_else(|| {
        format!(
            "{name} id '{}' is not a UUID (8-4-4-4-12 hex digits)",
            shown(id)
        )
    })?;
    let mut params = Params::default();
    if let Some(query) = query {
        for pair in query.split('&') {
            params.add(form, pair)?;
        }
    }
    Ok((id, params))
}

/// A value for a message: escaped, so a control character cannot reach it, and short.
fn shown(text: &str) -> String {
    truncate(&text.escape_debug().to_string(), 40)
}

const PIXELS_HINT: &str = "use an integer from 1 to 100000";

#[derive(Debug, Default)]
struct Params {
    collection: Option<MediaCollection>,
    width: Option<MediaDimension>,
    height: Option<MediaDimension>,
    layout: Option<MediaLayout>,
    size: Option<MediaSize>,
}

impl Params {
    /// Adds one `key=value` pair.
    fn add(&mut self, form: Form, pair: &str) -> Result<(), String> {
        let name = form.name();
        if pair.is_empty() {
            return Err(format!("{name} has an empty parameter"));
        }
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            format!(
                "{name} parameter '{}' has no value; write key=value",
                shown(pair)
            )
        })?;
        // An empty collection is the value Jira itself writes, so `collection=` is allowed.
        if value.is_empty() && key != "collection" {
            return Err(format!(
                "{name} parameter '{}' has an empty value",
                shown(key)
            ));
        }
        let image = form == Form::Image;
        match key {
            "collection" => assign(
                name,
                &mut self.collection,
                key,
                value,
                MediaCollection::parse(value),
                "use up to 64 letters, digits, '-' or '_'",
            ),
            "width" if image => assign(
                name,
                &mut self.width,
                key,
                value,
                dimension(value),
                PIXELS_HINT,
            ),
            "height" if image => assign(
                name,
                &mut self.height,
                key,
                value,
                dimension(value),
                PIXELS_HINT,
            ),
            "layout" if image => assign(
                name,
                &mut self.layout,
                key,
                value,
                MediaLayout::parse(value),
                "use wide, full-width, center, wrap-right, wrap-left, align-end or align-start",
            ),
            "size" if image => assign(
                name,
                &mut self.size,
                key,
                value,
                size(value),
                "use N% (1 to 100) or Npx (1 to 100000)",
            ),
            _ => Err(format!(
                "{name} has an unknown parameter '{}'; expected {}",
                shown(key),
                form.keys()
            )),
        }
    }
}

/// Stores a parsed value, refusing a repeated key and an invalid value.
fn assign<T>(
    name: &str,
    slot: &mut Option<T>,
    key: &str,
    value: &str,
    parsed: Option<T>,
    expected: &str,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{name} repeats the parameter '{key}'"));
    }
    *slot = Some(
        parsed
            .ok_or_else(|| format!("{name} {key} '{}' is not valid; {expected}", shown(value)))?,
    );
    Ok(())
}

/// Digits only (no sign, no space), at most `u32::MAX`.
fn digits(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn dimension(text: &str) -> Option<MediaDimension> {
    digits(text).and_then(MediaDimension::new)
}

/// `N%` or `Npx`.
fn size(text: &str) -> Option<MediaSize> {
    if let Some(n) = text.strip_suffix('%') {
        digits(n).and_then(MediaSize::percent)
    } else {
        text.strip_suffix("px")
            .and_then(digits)
            .and_then(MediaSize::pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "11111111-2222-3333-4444-555555555555";

    fn ok(arg: &str) -> FileImage {
        parse(Some(arg)).unwrap_or_else(|e| panic!("{arg}: {e}"))
    }

    fn err(arg: &str) -> String {
        parse(Some(arg)).expect_err(arg)
    }

    #[test]
    fn id_only_uses_the_defaults() {
        let image = ok(ID);
        assert_eq!(image.file.id.as_str(), ID);
        assert!(image.file.collection.is_empty());
        assert_eq!(image.file.width, None);
        assert_eq!(image.file.height, None);
        assert_eq!(image.layout, MediaLayout::Center);
        assert_eq!(image.size, None);
    }

    #[test]
    fn uppercase_uuid_is_kept_as_written() {
        let upper = "ABCDEF01-2222-3333-4444-5555555555AB";
        assert_eq!(ok(upper).file.id.as_str(), upper);
    }

    #[test]
    fn all_parameters() {
        let image = ok(&format!(
            "{ID}?collection=contentId-123456&width=1512&height=945&layout=wide&size=720px"
        ));
        assert_eq!(image.file.collection.as_str(), "contentId-123456");
        assert_eq!(image.file.width, MediaDimension::new(1512));
        assert_eq!(image.file.height, MediaDimension::new(945));
        assert_eq!(image.layout, MediaLayout::Wide);
        assert_eq!(image.size, MediaSize::pixels(720));
    }

    #[test]
    fn key_order_does_not_matter() {
        let a = ok(&format!(
            "{ID}?size=50%&layout=wrap-left&height=2&width=1&collection=c"
        ));
        let b = ok(&format!(
            "{ID}?collection=c&width=1&height=2&layout=wrap-left&size=50%"
        ));
        assert_eq!(a, b);
    }

    #[test]
    fn parameters_can_be_given_alone() {
        assert_eq!(
            ok(&format!("{ID}?width=7")).file.width,
            MediaDimension::new(7)
        );
        assert_eq!(ok(&format!("{ID}?width=7")).file.height, None);
        assert_eq!(ok(&format!("{ID}?height=7")).file.width, None);
        assert_eq!(
            ok(&format!("{ID}?layout=align-end")).layout,
            MediaLayout::AlignEnd
        );
        assert_eq!(ok(&format!("{ID}?size=3%")).size, MediaSize::percent(3));
    }

    #[test]
    fn every_layout_parses() {
        for layout in MediaLayout::ALL {
            let image = ok(&format!("{ID}?layout={}", layout.as_str()));
            assert_eq!(image.layout, layout);
        }
    }

    #[test]
    fn dimension_boundaries() {
        for key in ["width", "height"] {
            for good in ["1", "100000", "0042"] {
                assert!(
                    parse(Some(&format!("{ID}?{key}={good}"))).is_ok(),
                    "{key}={good}"
                );
            }
            for bad in [
                "0",
                "100001",
                "-1",
                "+5",
                "1.5",
                "1e3",
                "ten",
                "99999999999",
                " 5",
                "5 ",
                "5px",
            ] {
                let message = err(&format!("{ID}?{key}={bad}"));
                assert!(message.contains(key), "{message}");
                assert!(message.contains("1 to 100000"), "{message}");
            }
        }
    }

    #[test]
    fn size_forms_and_boundaries() {
        assert_eq!(ok(&format!("{ID}?size=1%")).size, MediaSize::percent(1));
        assert_eq!(ok(&format!("{ID}?size=100%")).size, MediaSize::percent(100));
        assert_eq!(ok(&format!("{ID}?size=1px")).size, MediaSize::pixels(1));
        assert_eq!(
            ok(&format!("{ID}?size=100000px")).size,
            MediaSize::pixels(100_000)
        );
        for bad in [
            "0%",
            "101%",
            "0px",
            "100001px",
            "50",
            "px",
            "%",
            "5 %",
            "5PX",
            "5pt",
            "-5%",
            "1.5%",
            "50%%",
            "99999999999px",
        ] {
            let message = err(&format!("{ID}?size={bad}"));
            assert!(message.contains("size"), "{message}");
        }
    }

    #[test]
    fn collection_charset_and_length() {
        let longest = "a".repeat(64);
        assert_eq!(
            ok(&format!("{ID}?collection={longest}"))
                .file
                .collection
                .as_str(),
            longest
        );
        assert_eq!(
            ok(&format!("{ID}?collection=comment-container-123456"))
                .file
                .collection
                .as_str(),
            "comment-container-123456"
        );
        for bad in [&"a".repeat(65), "a b", "a.b", "a/b", "é", "a%20b", "a;b"] {
            let message = err(&format!("{ID}?collection={bad}"));
            assert!(message.contains("collection"), "{message}");
        }
    }

    #[test]
    fn invalid_layouts() {
        for bad in ["", "Wide", "left", "full_width", "center "] {
            assert!(
                parse(Some(&format!("{ID}?layout={bad}"))).is_err(),
                "{bad:?}"
            );
        }
        let message = err(&format!("{ID}?layout=left"));
        assert!(message.contains("layout 'left'"), "{message}");
        assert!(message.contains("align-start"), "{message}");
    }

    #[test]
    fn missing_or_bad_id() {
        for arg in [None, Some(""), Some("?width=1")] {
            let message = parse(arg).unwrap_err();
            assert!(message.contains("needs the file's media ID"), "{message}");
        }
        for bad in [
            "abc",
            "11111111222233334444555555555555",
            "11111111-2222-3333-4444-55555555555",
            "11111111-2222-3333-4444-55555555555g",
            "11111111-2222-3333-4444-555555555555/extra",
            " 11111111-2222-3333-4444-555555555555",
            "123",
        ] {
            let message = err(bad);
            assert!(message.contains("is not a UUID"), "{message}");
            let message = err(&format!("{bad}?width=1"));
            assert!(message.contains("is not a UUID"), "{message}");
        }
    }

    #[test]
    fn unknown_duplicate_and_empty_parameters() {
        let message = err(&format!("{ID}?colour=red"));
        assert!(message.contains("unknown parameter 'colour'"), "{message}");
        // Keys are case-sensitive.
        assert!(err(&format!("{ID}?Width=1")).contains("unknown parameter 'Width'"));
        for key in [
            "collection=a",
            "width=1",
            "height=1",
            "layout=wide",
            "size=1%",
        ] {
            let name = key.split('=').next().unwrap();
            let message = err(&format!("{ID}?{key}&{key}"));
            assert!(
                message.contains(&format!("repeats the parameter '{name}'")),
                "{message}"
            );
        }
        assert!(err(&format!("{ID}?")).contains("empty parameter"));
        assert!(err(&format!("{ID}?width=1&")).contains("empty parameter"));
        assert!(err(&format!("{ID}?&width=1")).contains("empty parameter"));
        assert!(err(&format!("{ID}?width")).contains("has no value"));
        assert!(err(&format!("{ID}?=1")).contains("unknown parameter ''"));
        for key in ["width", "height", "layout", "size"] {
            let message = err(&format!("{ID}?{key}="));
            assert!(
                message.contains(&format!("'{key}' has an empty value")),
                "{message}"
            );
        }
        // An empty collection is the value Jira writes: allowed, the same as no collection.
        assert_eq!(ok(&format!("{ID}?collection=")), ok(ID));
        assert!(err(&format!("{ID}?collection=&collection=a")).contains("repeats the parameter"));
        // A second `?` ends up inside a value, which then fails its own check.
        assert!(parse(Some(&format!("{ID}?width=1?height=2"))).is_err());
    }

    #[test]
    fn messages_are_short_single_sentences_without_control_characters() {
        let long = "x".repeat(500);
        for arg in [
            long.clone(),
            format!("{ID}?collection={long}"),
            format!("{ID}?{long}=1"),
            format!("{ID}?width=\u{7}\n{long}"),
            format!("{ID}?{long}"),
        ] {
            let message = err(&arg);
            assert!(message.chars().count() < 200, "{message}");
            assert!(!message.chars().any(char::is_control), "{message:?}");
            assert!(!message.contains('\n'));
        }
    }

    // --- adf:attachment ---------------------------------------------------------------------

    fn attachment(arg: &str) -> Attachment {
        parse_attachment(Some(arg)).unwrap_or_else(|e| panic!("{arg}: {e}"))
    }

    fn attachment_err(arg: &str) -> String {
        parse_attachment(Some(arg)).expect_err(arg)
    }

    #[test]
    fn attachment_takes_an_id_and_an_optional_collection() {
        let plain = attachment(ID);
        assert_eq!(plain.id.as_str(), ID);
        assert!(plain.collection.is_empty());
        assert_eq!(
            attachment(&format!("{ID}?collection=contentId-123456"))
                .collection
                .as_str(),
            "contentId-123456"
        );
        // The empty collection Jira writes is the same as none.
        assert_eq!(attachment(&format!("{ID}?collection=")), plain);
        let upper = "ABCDEF01-2222-3333-4444-5555555555AB";
        assert_eq!(attachment(upper).id.as_str(), upper);
    }

    #[test]
    fn attachment_uses_the_collection_and_id_rules_of_the_image_form() {
        for collection in [
            "a",
            "a-b_C9",
            &"a".repeat(64),
            &"a".repeat(65),
            "a b",
            "a.b",
            "é",
        ] {
            let image = parse(Some(&format!("{ID}?collection={collection}")));
            let file = parse_attachment(Some(&format!("{ID}?collection={collection}")));
            assert_eq!(image.is_ok(), file.is_ok(), "{collection:?}");
            if let (Ok(image), Ok(file)) = (image, file) {
                assert_eq!(image.file.collection, file.collection);
            }
        }
        for id in ["abc", "11111111222233334444555555555555", "123", ""] {
            assert_eq!(parse(Some(id)).is_ok(), parse_attachment(Some(id)).is_ok());
        }
    }

    #[test]
    fn attachment_refuses_the_image_parameters() {
        for key in [
            "width=1",
            "height=1",
            "layout=wide",
            "size=50%",
            "colour=red",
        ] {
            let message = attachment_err(&format!("{ID}?{key}"));
            assert!(
                message.starts_with("adf:attachment has an unknown parameter"),
                "{message}"
            );
            assert!(message.ends_with("expected collection"), "{message}");
        }
        // The image form still takes them.
        assert!(parse(Some(&format!("{ID}?width=1"))).is_ok());
    }

    #[test]
    fn attachment_failures_name_the_attachment_form() {
        assert!(
            attachment_err(&format!("{ID}?collection=a&collection=b"))
                .contains("adf:attachment repeats the parameter 'collection'")
        );
        assert!(
            attachment_err("not-a-uuid").contains("adf:attachment id 'not-a-uuid' is not a UUID")
        );
        assert!(
            parse_attachment(None)
                .unwrap_err()
                .contains("adf:attachment needs the file's media ID: adf:attachment/<uuid>")
        );
        assert!(
            attachment_err(&format!("{ID}?")).contains("adf:attachment has an empty parameter")
        );
        assert!(
            attachment_err(&format!("{ID}?collection=a b"))
                .contains("adf:attachment collection 'a b' is not valid")
        );
        assert!(
            attachment_err(&format!("{ID}?collection"))
                .contains("adf:attachment parameter 'collection' has no value")
        );
    }
}
