//! Parses rendered inline Markdown back with pulldown-cmark (the parser behind Markdown → ADF)
//! and reduces it to styled text runs, so the renderer can check that what it wrote means what
//! it intended. This is how delimiter flanking, escaping and code-span padding are verified
//! without re-implementing CommonMark's delimiter algorithm.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::inline::{InlineKind, LinkTarget, Style, SubSup};

/// Styled text, or a hard break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Run {
    Text(String, Style),
    Break,
}

/// Appends text, merging it into the previous run when the style is the same.
pub(super) fn push_run(runs: &mut Vec<Run>, text: &str, style: &Style) {
    if text.is_empty() {
        return;
    }
    if let Some(Run::Text(previous, previous_style)) = runs.last_mut()
        && previous_style == style
    {
        previous.push_str(text);
        return;
    }
    runs.push(Run::Text(text.to_owned(), style.clone()));
}

/// The CommonMark/GFM options Markdown → ADF parses with.
pub(super) fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM
}

/// Parses `md` as the inline content of `kind` and returns its runs, or `None` when it does not
/// parse as exactly one such container holding only constructs this renderer writes.
pub(super) fn parse_inline(md: &str, kind: InlineKind) -> Option<Vec<Run>> {
    let source = match kind {
        InlineKind::Block => md.to_owned(),
        InlineKind::Heading(level) => format!("{} {md}", "#".repeat(usize::from(level))),
        InlineKind::TableCell => format!("| {md} |\n| --- |\n"),
    };
    let mut events = Parser::new_ext(&source, options());
    let opened = match kind {
        InlineKind::Block => matches!(events.next(), Some(Event::Start(Tag::Paragraph))),
        InlineKind::Heading(_) => matches!(events.next(), Some(Event::Start(Tag::Heading { .. }))),
        InlineKind::TableCell => {
            matches!(events.next(), Some(Event::Start(Tag::Table(_))))
                && matches!(events.next(), Some(Event::Start(Tag::TableHead)))
                && matches!(events.next(), Some(Event::Start(Tag::TableCell)))
        }
    };
    if !opened {
        return None;
    }
    let mut state = State::default();
    let mut runs = Vec::new();
    loop {
        let event = events.next()?;
        if is_container_end(&event, kind) {
            break;
        }
        state.apply(event, &mut runs)?;
    }
    if !state.is_closed() {
        return None;
    }
    let rest_ok = match kind {
        InlineKind::Block | InlineKind::Heading(_) => events.next().is_none(),
        InlineKind::TableCell => {
            matches!(events.next(), Some(Event::End(TagEnd::TableHead)))
                && matches!(events.next(), Some(Event::End(TagEnd::Table)))
                && events.next().is_none()
        }
    };
    rest_ok.then_some(runs)
}

fn is_container_end(event: &Event<'_>, kind: InlineKind) -> bool {
    match kind {
        InlineKind::Block => matches!(event, Event::End(TagEnd::Paragraph)),
        InlineKind::Heading(_) => matches!(event, Event::End(TagEnd::Heading(_))),
        InlineKind::TableCell => matches!(event, Event::End(TagEnd::TableCell)),
    }
}

#[derive(Debug, Default)]
struct State {
    em: u32,
    strong: u32,
    strike: u32,
    underline: u32,
    subsup: Vec<SubSup>,
    spans: Vec<SpanKind>,
    links: Vec<LinkTarget>,
}

#[derive(Debug, Clone)]
enum SpanKind {
    Color(String),
    Bg(String),
}

impl State {
    fn style(&self) -> Style {
        let color = self.spans.iter().rev().find_map(|s| match s {
            SpanKind::Color(c) => Some(c.clone()),
            SpanKind::Bg(_) => None,
        });
        let bg = self.spans.iter().rev().find_map(|s| match s {
            SpanKind::Bg(c) => Some(c.clone()),
            SpanKind::Color(_) => None,
        });
        Style {
            link: self.links.last().cloned(),
            em: self.em > 0,
            strong: self.strong > 0,
            strike: self.strike > 0,
            underline: self.underline > 0,
            subsup: self.subsup.last().copied(),
            color,
            bg,
            code: false,
        }
    }

    fn is_closed(&self) -> bool {
        self.em == 0
            && self.strong == 0
            && self.strike == 0
            && self.underline == 0
            && self.subsup.is_empty()
            && self.spans.is_empty()
            && self.links.is_empty()
    }

    /// Applies one inline event; `None` for anything this renderer never writes.
    fn apply(&mut self, event: Event<'_>, runs: &mut Vec<Run>) -> Option<()> {
        match event {
            Event::Text(text) => push_run(runs, &text, &self.style()),
            Event::Code(text) => {
                let style = Style {
                    code: true,
                    ..self.style()
                };
                push_run(runs, &text, &style);
            }
            Event::HardBreak => runs.push(Run::Break),
            Event::InlineHtml(html) => self.html(&html, runs)?,
            Event::Start(tag) => self.start(tag)?,
            Event::End(tag) => self.end(tag)?,
            Event::SoftBreak
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::Html(_)
            | Event::FootnoteReference(_)
            | Event::Rule
            | Event::TaskListMarker(_) => return None,
        }
        Some(())
    }

    fn start(&mut self, tag: Tag<'_>) -> Option<()> {
        match tag {
            Tag::Emphasis => self.em += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link {
                dest_url, title, ..
            } => self.links.push(LinkTarget {
                href: dest_url.to_string(),
                title: (!title.is_empty()).then(|| title.to_string()),
            }),
            Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::BlockQuote(_)
            | Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::List(_)
            | Tag::Item
            | Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Image { .. }
            | Tag::MetadataBlock(_) => return None,
        }
        Some(())
    }

    fn end(&mut self, tag: TagEnd) -> Option<()> {
        match tag {
            TagEnd::Emphasis => self.em = self.em.checked_sub(1)?,
            TagEnd::Strong => self.strong = self.strong.checked_sub(1)?,
            TagEnd::Strikethrough => self.strike = self.strike.checked_sub(1)?,
            TagEnd::Link => {
                self.links.pop()?;
            }
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Table
            | TagEnd::TableHead
            | TagEnd::TableRow
            | TagEnd::TableCell
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Image
            | TagEnd::MetadataBlock(_) => return None,
        }
        Some(())
    }

    /// The inline HTML tags this renderer writes.
    fn html(&mut self, tag: &str, runs: &mut Vec<Run>) -> Option<()> {
        match tag {
            "<br>" => runs.push(Run::Break),
            "<em>" => self.em += 1,
            "</em>" => self.em = self.em.checked_sub(1)?,
            "<strong>" => self.strong += 1,
            "</strong>" => self.strong = self.strong.checked_sub(1)?,
            "<del>" => self.strike += 1,
            "</del>" => self.strike = self.strike.checked_sub(1)?,
            "<u>" => self.underline += 1,
            "</u>" => self.underline = self.underline.checked_sub(1)?,
            "<sub>" => self.subsup.push(SubSup::Sub),
            "<sup>" => self.subsup.push(SubSup::Sup),
            "</sub>" => {
                if self.subsup.pop()? != SubSup::Sub {
                    return None;
                }
            }
            "</sup>" => {
                if self.subsup.pop()? != SubSup::Sup {
                    return None;
                }
            }
            "</span>" => {
                self.spans.pop()?;
            }
            other => {
                let color = |prefix: &str| {
                    other
                        .strip_prefix(prefix)
                        .and_then(|rest| rest.strip_suffix("\">"))
                        .map(str::to_owned)
                };
                let span = if let Some(c) = color("<span style=\"color: ") {
                    SpanKind::Color(c)
                } else {
                    SpanKind::Bg(color("<span style=\"background-color: ")?)
                };
                self.spans.push(span);
            }
        }
        Some(())
    }
}

/// `None` when the runs are equal. Otherwise the character positions whose style differs, or an
/// empty list when the text itself differs (positions are then meaningless).
pub(super) fn first_differences(intended: &[Run], parsed: &[Run]) -> Option<Vec<usize>> {
    if intended == parsed {
        return None;
    }
    let a = flatten(intended);
    let b = flatten(parsed);
    if a.len() != b.len() || a.iter().zip(&b).any(|(x, y)| x.0 != y.0) {
        return Some(Vec::new());
    }
    Some(
        a.iter()
            .zip(&b)
            .enumerate()
            .filter(|(_, (x, y))| x.1 != y.1)
            .map(|(i, _)| i)
            .collect(),
    )
}

fn flatten(runs: &[Run]) -> Vec<(char, Option<&Style>)> {
    let mut out = Vec::new();
    for run in runs {
        match run {
            Run::Text(text, style) => out.extend(text.chars().map(|c| (c, Some(style)))),
            Run::Break => out.push(('\n', None)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Run {
        Run::Text(text.into(), Style::default())
    }

    #[test]
    fn parses_paragraph_runs() {
        let runs = parse_inline("a **b** <u>c</u>\\\n`d`", InlineKind::Block).unwrap();
        assert_eq!(
            runs,
            vec![
                plain("a "),
                Run::Text(
                    "b".into(),
                    Style {
                        strong: true,
                        ..Style::default()
                    }
                ),
                plain(" "),
                Run::Text(
                    "c".into(),
                    Style {
                        underline: true,
                        ..Style::default()
                    }
                ),
                Run::Break,
                Run::Text(
                    "d".into(),
                    Style {
                        code: true,
                        ..Style::default()
                    }
                ),
            ]
        );
    }

    #[test]
    fn rejects_other_structures() {
        assert!(parse_inline("- x", InlineKind::Block).is_none());
        assert!(parse_inline("a\nb", InlineKind::Block).is_none());
        assert!(parse_inline("<b>x</b>", InlineKind::Block).is_none());
        assert!(parse_inline("x\n\ny", InlineKind::Block).is_none());
        assert!(parse_inline("a | b", InlineKind::TableCell).is_none());
    }

    #[test]
    fn heading_and_cell_wrappers() {
        assert_eq!(
            parse_inline("a<br>b", InlineKind::Heading(2)).unwrap(),
            vec![plain("a"), Run::Break, plain("b")]
        );
        assert_eq!(
            parse_inline(r"a\|b", InlineKind::TableCell).unwrap(),
            vec![plain("a|b")]
        );
    }

    #[test]
    fn differences_are_localised() {
        let strong = Style {
            strong: true,
            ..Style::default()
        };
        let intended = vec![plain("a"), Run::Text("b".into(), strong)];
        assert_eq!(first_differences(&intended, &intended), None);
        assert_eq!(first_differences(&intended, &[plain("ab")]), Some(vec![1]));
        assert_eq!(first_differences(&intended, &[plain("a**b")]), Some(vec![]));
    }
}
