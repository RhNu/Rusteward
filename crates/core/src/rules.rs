//! Apply configurable source policies using physical tokens and Rust syntax.

use std::path::Path;

use ra_ap_syntax::{AstNode, Edition, SourceFile, SyntaxElement, SyntaxKind, ast, ast::HasName};
use serde::{Deserialize, Serialize};

use crate::{
    diagnostic::{Diagnostic, Severity},
    lines::{ERROR_LINES, Level, WARN_LINES, code_lines},
};

/// Independent threshold tiers; disabling one tier leaves the other active.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct LineRule {
    pub warn: usize,
    pub error: usize,
    pub warn_level: Severity,
    pub error_level: Severity,
}

impl Default for LineRule {
    fn default() -> Self {
        Self {
            warn: WARN_LINES,
            error: ERROR_LINES,
            warn_level: Severity::Warning,
            error_level: Severity::Error,
        }
    }
}

/// Personal source policies shared by all authored-source workflows.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Rules {
    pub lines: LineRule,
    pub mod_rs: Severity,
    pub inline_tests: Severity,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            lines: LineRule::default(),
            mod_rs: Severity::Error,
            inline_tests: Severity::Warning,
        }
    }
}

/// Inspect disabled-feature code too; macro token trees remain opaque.
pub fn inspect(path: &Path, source: &str, edition: Edition, rules: &Rules) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if crate::is_generated(source) {
        return diagnostics;
    }
    if path.file_name().is_some_and(|name| name == "mod.rs") {
        push(
            &mut diagnostics,
            path,
            1,
            1,
            "mod-rs",
            rules.mod_rs,
            "use <module>.rs with a <module>/ child directory instead of mod.rs".into(),
        );
    }
    inspect_lines(path, source, &rules.lines, &mut diagnostics);
    if rules.inline_tests != Severity::Off {
        inspect_inline_tests(path, source, edition, rules.inline_tests, &mut diagnostics);
    }
    diagnostics
}

/// Apply the independently configurable warning and error thresholds.
fn inspect_lines(path: &Path, source: &str, limits: &LineRule, diagnostics: &mut Vec<Diagnostic>) {
    let lines = code_lines(source);
    // Disabling the upper tier still leaves the independent lower tier active.
    let band = Level::for_lines(
        lines,
        if limits.warn_level == Severity::Off {
            usize::MAX
        } else {
            limits.warn
        },
        if limits.error_level == Severity::Off {
            usize::MAX
        } else {
            limits.error
        },
    );
    let breach = match band {
        Level::Warning => Some((limits.warn_level, limits.warn)),
        Level::Error => Some((limits.error_level, limits.error)),
        Level::Ok => None,
    };
    if let Some((severity, threshold)) = breach {
        push(
            diagnostics,
            path,
            1,
            1,
            "lines",
            severity,
            format!("{lines} code lines exceed {threshold}; split this file by responsibility"),
        );
    }
}

/// Report parse failures and embedded test modules while leaving macro bodies opaque.
fn inspect_inline_tests(
    path: &Path,
    source: &str,
    edition: Edition,
    severity: Severity,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let parsed = SourceFile::parse(source, edition);
    for error in parsed.errors() {
        let (line, column) = location(source, usize::from(error.range().start()));
        push(
            diagnostics,
            path,
            line,
            column,
            "syntax",
            Severity::Error,
            error.to_string(),
        );
    }
    for module in parsed
        .syntax_node()
        .descendants()
        .filter_map(ast::Module::cast)
    {
        if module.item_list().is_none() {
            continue;
        }
        let named_tests = module.name().is_some_and(|name| name.text() == "tests");
        let test_gate = module
            .syntax()
            .children()
            .filter_map(ast::Attr::cast)
            .any(|attr| {
                let tokens: String = attr
                    .syntax()
                    .descendants_with_tokens()
                    .filter_map(SyntaxElement::into_token)
                    .filter(|token| {
                        !matches!(token.kind(), SyntaxKind::WHITESPACE | SyntaxKind::COMMENT)
                    })
                    .map(|token| token.text().to_owned())
                    .collect();
                tokens == "#[cfg(test)]"
            });
        if named_tests || test_gate {
            let (line, column) =
                location(source, usize::from(module.syntax().text_range().start()));
            push(
                diagnostics,
                path,
                line,
                column,
                "inline-tests",
                severity,
                "move unit tests to a child file and declare #[cfg(test)] mod tests; in the parent"
                    .into(),
            );
        }
    }
}

/// Translate UTF-8 byte offsets into character columns for human-readable reports.
pub fn location(source: &str, offset: usize) -> (usize, usize) {
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (line, column)
}

/// Disabled rules never emit diagnostics.
fn push(
    diagnostics: &mut Vec<Diagnostic>,
    path: &Path,
    line: usize,
    column: usize,
    rule: &'static str,
    severity: Severity,
    message: String,
) {
    if severity != Severity::Off {
        diagnostics.push(Diagnostic {
            path: path.into(),
            line,
            column,
            rule,
            severity,
            message,
        });
    }
}

#[cfg(test)]
mod tests;
