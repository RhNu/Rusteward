//! Enforce separation around selected declarations without rewriting source tokens.

use ra_ap_syntax::{AstNode, Edition, SyntaxElement, SyntaxError, SyntaxKind, SyntaxNode, ast};

use crate::{rules::location, source::SourceText};

/// A pure formatting result, with original line numbers suitable for check diagnostics.
pub struct Spacing {
    pub text: String,
    pub missing_lines: Vec<usize>,
    pub skip_reason: Option<&'static str>,
}

/// Insert blank lines where either adjacent item is a function, struct, enum, impl or trait.
/// Imports, constants and other items are otherwise untouched. Input should already
/// have been formatted by rustfmt, which puts sibling declarations on separate lines.
///
/// # Errors
/// Returns an error if the source cannot be parsed or adjacent declarations share a line.
pub fn separate_declarations(source: &str, edition: Edition) -> Result<Spacing, String> {
    if crate::is_generated(source) {
        return Ok(unchanged(source, Some("generated file")));
    }
    let input = SourceText::new(source);
    let parsed = input.parse(edition);
    let errors = parsed.errors();
    if !errors.is_empty() {
        return Err(parse_error_message(source, &input, &errors));
    }
    let root = parsed.syntax_node();
    if has_skip(&root) {
        return Ok(unchanged(source, Some("rustfmt::skip")));
    }
    let tokens: Vec<_> = root
        .descendants_with_tokens()
        .filter_map(SyntaxElement::into_token)
        .collect();
    let line_starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let mut edits = Vec::new();
    for list in root.descendants().filter(|node| {
        matches!(
            node.kind(),
            SyntaxKind::SOURCE_FILE | SyntaxKind::ITEM_LIST | SyntaxKind::ASSOC_ITEM_LIST
        )
    }) {
        if list.ancestors().any(|node| {
            has_skip(&node)
                || matches!(
                    node.kind(),
                    SyntaxKind::MACRO_RULES | SyntaxKind::MACRO_CALL | SyntaxKind::TOKEN_TREE
                )
        }) {
            continue;
        }
        let items: Vec<_> = list
            .children()
            .filter(|node| {
                ast::Item::can_cast(node.kind()) || ast::AssocItem::can_cast(node.kind())
            })
            .collect();
        for pair in items.windows(2) {
            if !pair.iter().any(|item| separated_kind(item.kind())) {
                continue;
            }
            let end = usize::from(pair[0].text_range().end());
            let next_end = usize::from(pair[1].text_range().end());
            let first =
                tokens.partition_point(|token| usize::from(token.text_range().start()) < end);
            // The parser can attach a previous declaration's trailing comment to the
            // next item. Inspect its leading trivia too: insert after that trailing
            // comment, but before the next item's standalone comments and attributes.
            let gap = tokens[first..]
                .iter()
                .take_while(|token| {
                    usize::from(token.text_range().end()) <= next_end
                        && matches!(token.kind(), SyntaxKind::WHITESPACE | SyntaxKind::COMMENT)
                })
                .find(|token| {
                    token.kind() == SyntaxKind::WHITESPACE && token.text().contains('\n')
                });
            let Some(gap) = gap else {
                let original_end = input.original_offset(end);
                let line = line_starts.partition_point(|start| *start <= original_end);
                return Err(format!(
                    "declarations share a line near line {line}; run rustfmt first"
                ));
            };
            if gap.text().bytes().filter(|byte| *byte == b'\n').count() >= 2 {
                continue;
            }
            let offset = input.original_offset(usize::from(gap.text_range().start()));
            let gap_end = input.original_offset(usize::from(gap.text_range().end()));
            let newline = if source[offset..gap_end].contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let line = line_starts.partition_point(|start| *start <= offset) + 1;
            edits.push((offset, newline, line));
        }
    }
    edits.sort_unstable_by_key(|edit| edit.0);
    edits.dedup_by_key(|edit| edit.0);
    let missing_lines = edits.iter().map(|edit| edit.2).collect();
    let mut text = source.to_owned();
    for (offset, newline, _) in edits.into_iter().rev() {
        text.insert_str(offset, newline);
    }
    Ok(Spacing {
        text,
        missing_lines,
        skip_reason: None,
    })
}

/// Describe all parse failures at original source positions rather than normalized byte offsets.
fn parse_error_message(source: &str, input: &SourceText<'_>, errors: &[SyntaxError]) -> String {
    let messages: Vec<_> = errors
        .iter()
        .map(|error| {
            let (line, column) = location(
                source,
                input.original_offset(usize::from(error.range().start())),
            );
            format!("{line}:{column}: {error}")
        })
        .collect();
    format!("cannot parse Rust source: {}", messages.join("; "))
}

/// Identify declarations that require separation from either neighboring item.
fn separated_kind(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::FN
            | SyntaxKind::STRUCT
            | SyntaxKind::ENUM
            | SyntaxKind::IMPL
            | SyntaxKind::TRAIT
    )
}

/// Respect explicit skips and conservatively preserve conditional skip regions too.
fn has_skip(node: &SyntaxNode) -> bool {
    node.children().filter_map(ast::Attr::cast).any(|attr| {
        attr.meta()
            .into_iter()
            .flat_map(ast::Meta::skip_cfg_attrs)
            .any(|meta| {
                meta.as_simple_path().is_some_and(|path| {
                    let name: String = path
                        .syntax()
                        .descendants_with_tokens()
                        .filter_map(SyntaxElement::into_token)
                        .filter(|token| {
                            !matches!(token.kind(), SyntaxKind::WHITESPACE | SyntaxKind::COMMENT)
                        })
                        .map(|token| token.text().to_owned())
                        .collect();
                    name == "rustfmt::skip"
                })
            })
    })
}

/// Preserve skipped inputs verbatim and report why the task did not inspect them.
fn unchanged(source: &str, skip_reason: Option<&'static str>) -> Spacing {
    Spacing {
        text: source.to_owned(),
        missing_lines: Vec::new(),
        skip_reason,
    }
}

#[cfg(test)]
mod tests;
