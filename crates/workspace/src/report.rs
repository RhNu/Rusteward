//! Workflow outcomes shared by text and structured CLI renderers.

use std::path::{Path, PathBuf};

pub use cargo_metadata::diagnostic::DiagnosticLevel;
use cargo_metadata::{CompilerMessage, PackageId, Target, diagnostic};
use rusteward_core::diagnostic::{Diagnostic as SourceDiagnostic, Severity};
use serde::Serialize;

/// Identify the producer independently of a diagnostic's optional rule code.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSource {
    Rusteward,
    Clippy,
    Rustc,
}

/// Common fields make every finding directly consumable without parsing rendered text.
#[derive(Debug, Serialize)]
pub struct Diagnostic {
    pub source: DiagnosticSource,
    pub rule: Option<String>,
    pub severity: DiagnosticLevel,
    pub message: String,
    pub path: Option<PathBuf>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiler: Option<CompilerDetails>,
}

/// Retain compilation context, all locations, and nested suggestions without flattening edits.
#[derive(Debug, Serialize)]
pub struct CompilerDetails {
    pub package_id: PackageId,
    pub target: Target,
    pub spans: Vec<diagnostic::DiagnosticSpan>,
    pub children: Vec<diagnostic::Diagnostic>,
    pub explanation: Option<String>,
    pub rendered: Option<String>,
}

impl Diagnostic {
    fn custom(diagnostic: SourceDiagnostic) -> Option<Self> {
        let severity = match diagnostic.severity {
            Severity::Off => return None,
            Severity::Warning => DiagnosticLevel::Warning,
            Severity::Error => DiagnosticLevel::Error,
        };
        Some(Self {
            source: DiagnosticSource::Rusteward,
            rule: Some(diagnostic.rule.into()),
            severity,
            message: diagnostic.message,
            path: Some(diagnostic.path),
            line: Some(diagnostic.line),
            column: Some(diagnostic.column),
            compiler: None,
        })
    }

    /// Use the primary span for navigation; preserve secondary and macro spans in full.
    pub(crate) fn compiler(message: CompilerMessage, root: &Path) -> Self {
        let diagnostic = message.message;
        let rule = diagnostic.code.as_ref().map(|code| code.code.clone());
        let source = if rule
            .as_deref()
            .is_some_and(|rule| rule.starts_with("clippy::"))
        {
            DiagnosticSource::Clippy
        } else {
            DiagnosticSource::Rustc
        };
        let span = diagnostic.spans.iter().find(|span| span.is_primary);
        let path = span.map(|span| {
            let path = Path::new(&span.file_name);
            path.strip_prefix(root).unwrap_or(path).to_path_buf()
        });
        Self {
            source,
            rule,
            severity: diagnostic.level,
            message: diagnostic.message,
            path,
            line: span.map(|span| span.line_start),
            column: span.map(|span| span.column_start),
            compiler: Some(CompilerDetails {
                package_id: message.package_id,
                target: message.target,
                explanation: diagnostic.code.and_then(|code| code.explanation),
                spans: diagnostic.spans,
                children: diagnostic.children,
                rendered: diagnostic.rendered,
            }),
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self.severity, DiagnosticLevel::Error | DiagnosticLevel::Ice)
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub files: usize,
    pub changed: usize,
    pub skipped: usize,
    pub diagnostics: Vec<Diagnostic>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diffs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clippy: Option<ClippyReport>,
}

/// Process status remains authoritative even when a failure has no compiler diagnostic.
#[derive(Debug, Serialize)]
pub struct ClippyReport {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub build_success: Option<bool>,
    pub output: String,
    pub stderr: String,
}

/// Count top-level diagnostics only; notes and suggestions never inflate error totals.
#[derive(Debug, Default, Serialize)]
pub struct Summary {
    pub warnings: usize,
    pub errors: usize,
}

impl Report {
    pub fn extend_custom(&mut self, diagnostics: impl IntoIterator<Item = SourceDiagnostic>) {
        self.diagnostics
            .extend(diagnostics.into_iter().filter_map(Diagnostic::custom));
    }

    pub fn summary(&self) -> Summary {
        Summary {
            warnings: self
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == DiagnosticLevel::Warning)
                .count(),
            errors: self
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .count(),
        }
    }

    pub fn failed(&self, deny_warnings: bool) -> bool {
        self.diagnostics.iter().any(|diagnostic| {
            diagnostic.is_error()
                || (deny_warnings
                    && diagnostic.source == DiagnosticSource::Rusteward
                    && diagnostic.severity == DiagnosticLevel::Warning)
        }) || self.clippy.as_ref().is_some_and(|clippy| !clippy.success)
    }

    /// Both workflows scan the same inputs; aggregate findings without double-counting files.
    pub fn append(&mut self, other: Self) {
        self.files = self.files.max(other.files);
        self.changed += other.changed;
        self.skipped = self.skipped.max(other.skipped);
        self.diagnostics.extend(other.diagnostics);
        self.diffs.extend(other.diffs);
        self.clippy = other.clippy.or(self.clippy.take());
    }
}

#[cfg(test)]
mod tests;
