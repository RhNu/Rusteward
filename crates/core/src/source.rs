//! Normalize parser input while retaining byte boundaries in the authored source.

use std::borrow::Cow;

use ra_ap_syntax::{Edition, Parse, SourceFile};

/// Rust normalizes CRLF once before tokenization; the parser expects that input already prepared.
pub(crate) struct SourceText<'a> {
    text: Cow<'a, str>,
    removed_crs: Vec<usize>,
}

impl<'a> SourceText<'a> {
    /// Borrow LF inputs and record only the collapsed CRLF boundaries for changed inputs.
    pub(crate) fn new(source: &'a str) -> Self {
        let mut text = String::new();
        let mut removed_crs = Vec::new();
        let mut start = 0;
        for (offset, _) in source.match_indices("\r\n") {
            if removed_crs.is_empty() {
                text.reserve(source.len());
            }
            text.push_str(&source[start..offset]);
            removed_crs.push(text.len());
            // Keep the LF for the next chunk, without revisiting newly adjacent CR/LF pairs.
            start = offset + 1;
        }
        let text = if removed_crs.is_empty() {
            Cow::Borrowed(source)
        } else {
            text.push_str(&source[start..]);
            Cow::Owned(text)
        };
        Self { text, removed_crs }
    }

    /// Parse the normalized view without altering the original source or its newline style.
    pub(crate) fn parse(&self, edition: Edition) -> Parse<SourceFile> {
        SourceFile::parse(&self.text, edition)
    }

    /// Map a parser boundary back before the original CR when it precedes a collapsed newline.
    pub(crate) fn original_offset(&self, offset: usize) -> usize {
        // A boundary before LF must include its CR in the original gap, while a boundary after
        // LF must skip both bytes. Counting only earlier removals preserves both boundaries.
        offset
            + self
                .removed_crs
                .partition_point(|position| *position < offset)
    }
}

#[cfg(test)]
mod tests;
