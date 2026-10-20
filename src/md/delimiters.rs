//! Emphasis-delimiter budget: a linear pre-scan that refuses Markdown whose paragraphs hold more
//! than [`MAX_EMPHASIS_DELIMITERS`](crate::limits::MAX_EMPHASIS_DELIMITERS) `*` and `_`
//! characters, before pulldown-cmark sees it.
//!
//! pulldown-cmark 0.13.4 resolves emphasis in quadratic time for some delimiter patterns: a
//! paragraph of `*a_` repeated makes every `_` rescan all the `*` openers before it
//! (pulldown-cmark issue #1154, fixed upstream but not released). 1 MiB of it would take
//! minutes. Real documents never have tens of thousands of delimiters in one paragraph, so the
//! scan simply counts them.
//!
//! A *block* is a run of lines between blank lines (only spaces, tabs and `>`). Emphasis never
//! spans a blank line, so no paragraph is larger than its block. Every `*` and `_` counts, in
//! code too: a line-by-line scan cannot tell code, lists or tables apart reliably, and anything
//! it could be fooled about would let the slow case through. Over-counting only means a
//! clearer error on input that is pathological anyway.
//!
//! The scan is `O(input length)` and allocates nothing.

use crate::diagnostics::{Code, Diagnostic};

use super::cx::Locator;

/// Checks the normalised Markdown `text`. On failure, returns the byte offset of the start of the
/// first block with more than `limit` delimiters.
pub(crate) fn check(text: &str, limit: usize) -> Result<(), usize> {
    let mut block_start = 0;
    let mut count = 0usize;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let blank = line
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t' | b'>' | b'\n' | b'\r'));
        if blank {
            count = 0;
            block_start = offset + line.len();
        } else {
            count += line.bytes().filter(|&b| b == b'*' || b == b'_').count();
            if count > limit {
                return Err(block_start);
            }
        }
        offset += line.len();
    }
    Ok(())
}

/// The `E_INPUT_TOO_LARGE` diagnostic for a block starting at `offset` in `text`.
pub(crate) fn diagnostic(text: &str, offset: usize, limit: usize) -> Diagnostic {
    let location = Locator::new(text).location(&(offset..offset));
    Diagnostic::new(
        Code::InputTooLarge,
        format!("A paragraph holds more than {limit} `*` and `_` characters"),
    )
    .at(location)
    .with_hint("Split the text with blank lines, or put long code or data in a fenced code block in its own paragraph; such input would take minutes to parse")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_per_block_and_locates_the_block() {
        assert_eq!(check("a_b *c*\n\nd", 3), Ok(()));
        assert_eq!(check("a_b *c* e_\n\nd", 3), Err(0));
        // Blank lines (also inside quotes) reset the count.
        assert_eq!(check("**\n> \n**\n\n**", 2), Ok(()));
        assert_eq!(check("x\n\n**\n**\n*", 4), Err(3));
        // Code counts too; the scan cannot tell code apart reliably.
        assert_eq!(check("```\n___\n```\n", 2), Err(0));
    }

    #[test]
    fn exactly_at_the_limit_passes() {
        let text = "*a_".repeat(10);
        assert_eq!(check(&text, 20), Ok(()));
        assert_eq!(check(&format!("{text}*"), 20), Err(0));
    }
}
