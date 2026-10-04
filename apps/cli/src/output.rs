//! Render deterministic diagnostics and keep JSON stdout free from child process logs.

use std::{
    io::{self, Write},
    path::Path,
};

use anyhow::Result;
use rusteward_workspace::report::{Diagnostic, DiagnosticLevel, DiagnosticSource, Report};

use crate::args::Cli;

pub fn render(cli: &Cli, root: &Path, report: &Report, failed: bool) -> Result<()> {
    write_report(
        cli,
        root,
        report,
        failed,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
}

/// All JSON commands share a versioned outcome envelope.
pub fn envelope(success: bool, exit_code: u8) -> serde_json::Value {
    serde_json::json!({"schema_version": 1, "success": success, "exit_code": exit_code})
}

/// Present operational errors once, using the same JSON outcome as completed checks.
pub fn error(cli: &Cli, error: &anyhow::Error) -> Result<()> {
    write_error(
        cli.json,
        error,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
}

fn write_error(
    json: bool,
    error: &anyhow::Error,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<()> {
    let message = format!("{error:#}");
    if json {
        let mut result = envelope(false, 2);
        result["error"] = message.into();
        writeln!(stdout, "{result}")?;
    } else {
        writeln!(stderr, "cargo dev: {message}")?;
    }
    Ok(())
}

/// Keep rendering independent of terminal state so output modes can be verified with pure inputs.
fn write_report(
    cli: &Cli,
    root: &Path,
    report: &Report,
    failed: bool,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<()> {
    if cli.json {
        let mut result = envelope(!failed, u8::from(failed));
        result["workspace"] = serde_json::to_value(root)?;
        result["summary"] = serde_json::to_value(report.summary())?;
        result["report"] = serde_json::to_value(report)?;
        writeln!(stdout, "{result}")?;
        return Ok(());
    }
    for diagnostic in &report.diagnostics {
        write_diagnostic(stderr, diagnostic, cli.verbose > 0)?;
    }
    if let Some(clippy) = &report.clippy
        && (cli.verbose > 0 || !clippy.success)
    {
        write_text(stderr, &clippy.output)?;
        write_text(stderr, &clippy.stderr)?;
        if !clippy.success
            && clippy.output.trim().is_empty()
            && clippy.stderr.trim().is_empty()
            && !report.diagnostics.iter().any(|diagnostic| {
                diagnostic.source != DiagnosticSource::Rusteward && diagnostic.is_error()
            })
        {
            writeln!(
                stderr,
                "Clippy failed (process exit code: {:?}, Cargo build success: {:?})",
                clippy.exit_code, clippy.build_success
            )?;
        }
    }
    for diff in &report.diffs {
        stdout.write_all(diff.as_bytes())?;
    }
    if !cli.quiet || failed {
        let summary = report.summary();
        let clippy = match &report.clippy {
            None => "skipped",
            Some(result) if result.success => "passed",
            _ => "failed",
        };
        writeln!(
            stderr,
            "{}: {} files, {} changed, {} skipped, {} warnings, {} errors; Clippy {clippy}",
            if failed { "failed" } else { "passed" },
            report.files,
            report.changed,
            report.skipped,
            summary.warnings,
            summary.errors
        )?;
    }
    Ok(())
}

fn write_diagnostic(
    writer: &mut impl Write,
    diagnostic: &Diagnostic,
    detailed: bool,
) -> Result<()> {
    if detailed
        && let Some(rendered) = diagnostic
            .compiler
            .as_ref()
            .and_then(|compiler| compiler.rendered.as_deref())
        && !rendered.trim().is_empty()
    {
        return write_text(writer, rendered);
    }
    if let Some(path) = &diagnostic.path {
        write!(writer, "{}", path.display())?;
        if let (Some(line), Some(column)) = (diagnostic.line, diagnostic.column) {
            write!(writer, ":{line}:{column}")?;
        }
        write!(writer, ": ")?;
    } else {
        write!(writer, "{}: ", source_name(diagnostic.source))?;
    }
    write!(writer, "{}", level_name(diagnostic.severity))?;
    if let Some(rule) = &diagnostic.rule {
        write!(writer, "[{rule}]")?;
    }
    // Keep the compact header on one line; multiline context belongs to the detailed view.
    writeln!(
        writer,
        ": {}",
        diagnostic.message.lines().collect::<Vec<_>>().join(" ")
    )?;
    if let Some(compiler) = &diagnostic.compiler {
        write_suggestions(writer, &compiler.spans)?;
        write_children(writer, &compiler.children, detailed)?;
    }
    Ok(())
}

fn write_children(
    writer: &mut impl Write,
    children: &[cargo_metadata::diagnostic::Diagnostic],
    detailed: bool,
) -> Result<()> {
    for child in children {
        if detailed
            || child.level == DiagnosticLevel::Help
            || child
                .spans
                .iter()
                .any(|span| span.suggested_replacement.is_some())
        {
            writeln!(writer, "  {}: {}", level_name(child.level), child.message)?;
            write_suggestions(writer, &child.spans)?;
        }
        write_children(writer, &child.children, detailed)?;
    }
    Ok(())
}

fn write_suggestions(
    writer: &mut impl Write,
    spans: &[cargo_metadata::diagnostic::DiagnosticSpan],
) -> Result<()> {
    for span in spans {
        if let Some(replacement) = &span.suggested_replacement {
            write!(
                writer,
                "    {}:{}:{}-{}:{}: replace with {replacement:?}",
                span.file_name, span.line_start, span.column_start, span.line_end, span.column_end
            )?;
            if let Some(applicability) = &span.suggestion_applicability {
                write!(writer, " ({applicability:?})")?;
            }
            writeln!(writer)?;
        }
    }
    Ok(())
}

fn write_text(writer: &mut impl Write, text: &str) -> Result<()> {
    if !text.is_empty() {
        writer.write_all(text.as_bytes())?;
        if !text.ends_with('\n') {
            writeln!(writer)?;
        }
    }
    Ok(())
}

fn source_name(source: DiagnosticSource) -> &'static str {
    match source {
        DiagnosticSource::Rusteward => "rusteward",
        DiagnosticSource::Clippy => "clippy",
        DiagnosticSource::Rustc => "rustc",
    }
}

fn level_name(level: DiagnosticLevel) -> &'static str {
    match level {
        DiagnosticLevel::Error => "error",
        DiagnosticLevel::Warning => "warning",
        DiagnosticLevel::Note => "note",
        DiagnosticLevel::Help => "help",
        DiagnosticLevel::FailureNote => "failure-note",
        DiagnosticLevel::Ice => "error: internal compiler error",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests;
