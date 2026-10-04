//! Shared diagnostic policy, independent of process execution and presentation.

use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Off,
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Off => "off",
            Self::Warning => "warning",
            Self::Error => "error",
        })
    }
}

/// A diagnostic location uses one-based lines and columns in the original source.
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub rule: &'static str,
    pub severity: Severity,
    pub message: String,
}

/// Decide the exit outcome without changing warning labels in the report.
pub fn has_failures(diagnostics: &[Diagnostic], deny_warnings: bool) -> bool {
    diagnostics.iter().any(|diagnostic| {
        diagnostic.severity == Severity::Error
            || (deny_warnings && diagnostic.severity == Severity::Warning)
    })
}

#[cfg(test)]
mod tests;
