//! Present workflow state, diagnostic blocks, and captured logs through one output policy.

use std::{
    io::{self, IsTerminal, Write},
    path::Path,
};

use anyhow::Result;
use rusteward_workspace::{
    report::{
        Diagnostic, DiagnosticLevel, DiagnosticSource, Phase, PhaseReport, PhaseStatus, Report,
    },
    workflow::{Event, Outcome},
};

use crate::args::{Cli, Command};

/// Runtime capabilities are explicit inputs to the pure presentation functions.
#[derive(Clone, Copy, Default)]
pub struct Presentation {
    pub color: bool,
    pub live: bool,
}

impl Presentation {
    pub fn detect(cli: &Cli) -> Self {
        Self {
            color: !cli.json
                && cli.color.enabled(
                    io::stderr().is_terminal(),
                    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()),
                ),
            live: true,
        }
    }
}

pub fn render(cli: &Cli, root: &Path, outcome: &Outcome, style: Presentation) -> Result<()> {
    write_report(
        cli,
        root,
        outcome,
        style,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
}

/// Announce the selected workspace once, before file processing starts.
pub fn workspace(cli: &Cli, root: &Path, files: usize) -> Result<()> {
    if !cli.quiet && !cli.json {
        let name = root
            .file_name()
            .unwrap_or(root.as_os_str())
            .to_string_lossy();
        let mut stderr = io::stderr().lock();
        writeln!(
            stderr,
            "{} {name} · {files} Rust source files\n",
            action_name(&cli.command)
        )?;
        stderr.flush()?;
    }
    Ok(())
}

/// Phase events are emitted on the calling thread, never by parallel file workers.
pub fn progress(cli: &Cli, event: Event<'_>, style: Presentation) -> Result<()> {
    if cli.quiet || cli.json {
        return Ok(());
    }
    let mut stderr = io::stderr().lock();
    match event {
        Event::Started(phase) => {
            styled(
                &mut stderr,
                style,
                "36",
                &format!("{}...", phase_name(phase)),
            )?;
            writeln!(stderr)?;
        },
        Event::Finished(phase) => write_phase(&mut stderr, phase, style)?,
    }
    stderr.flush()?;
    Ok(())
}

/// All JSON commands share a versioned outcome envelope.
pub fn envelope(success: bool, exit_code: u8) -> serde_json::Value {
    serde_json::json!({"schema_version": 1, "success": success, "exit_code": exit_code})
}

/// Errors before a workflow has a report still follow the same output policy.
pub fn error(cli: &Cli, error: &anyhow::Error, style: Presentation) -> Result<()> {
    write_error(
        cli,
        error,
        style,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
}

fn write_error(
    cli: &Cli,
    error: &anyhow::Error,
    style: Presentation,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<()> {
    let message = format!("{error:#}");
    if cli.json {
        let mut result = envelope(false, 2);
        result["error"] = message.into();
        writeln!(stdout, "{result}")?;
    } else {
        write_block(stderr, &format!("error: {message}"), style, "31")?;
        styled(
            stderr,
            style,
            "31",
            &format!("{} aborted", command_name(&cli.command)),
        )?;
        writeln!(stderr)?;
    }
    Ok(())
}

/// JSON always contains the complete report; text verbosity only selects log presentation.
fn write_report(
    cli: &Cli,
    root: &Path,
    outcome: &Outcome,
    style: Presentation,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<()> {
    let report = &outcome.report;
    let exit_code = outcome.exit_code();
    if cli.json {
        let mut result = envelope(exit_code == 0, exit_code);
        result["workspace"] = serde_json::to_value(root)?;
        result["summary"] = serde_json::to_value(report.summary())?;
        result["report"] = serde_json::to_value(report)?;
        if let Some(error) = &outcome.error {
            result["error"] = format!("{error:#}").into();
        }
        writeln!(stdout, "{result}")?;
        return Ok(());
    }
    if !cli.quiet {
        for phase in &report.phases {
            if !style.live || phase.status == PhaseStatus::NotRun {
                write_phase(stderr, phase, style)?;
            }
        }
    }
    if !report.phases.is_empty() && !cli.quiet {
        writeln!(stderr)?;
    }
    for diagnostic in &report.diagnostics {
        write_diagnostic(stderr, root, diagnostic, style)?;
    }
    if let Some(clippy) = &report.clippy {
        let explained = report.diagnostics.iter().any(|diagnostic| {
            diagnostic.source != DiagnosticSource::Rusteward && diagnostic.is_error()
        });
        let show_logs =
            cli.verbose > 0 || outcome.error.is_some() || (!clippy.success && !explained);
        if show_logs {
            write_log(stderr, "Cargo stdout", &clippy.output)?;
            write_log(stderr, "Cargo stderr", &clippy.stderr)?;
        }
        if !clippy.success && (!explained || outcome.error.is_some()) {
            writeln!(
                stderr,
                "Clippy failed · process exit: {} · Cargo build: {}\n",
                clippy
                    .exit_code
                    .map_or_else(|| "unavailable".into(), |code| code.to_string()),
                clippy
                    .build_success
                    .map_or("unavailable", |success| if success {
                        "passed"
                    } else {
                        "failed"
                    })
            )?;
        } else if !clippy.success && cli.verbose == 0 && !cli.quiet {
            writeln!(stderr, "Cargo logs are available with -v or --json.\n")?;
        }
    }
    if let Some(error) = &outcome.error {
        write_block(stderr, &format!("error: {error:#}"), style, "31")?;
    }
    for diff in &report.diffs {
        stdout.write_all(diff.as_bytes())?;
    }
    // Flush data before the final outcome when callers combine the two streams.
    stdout.flush()?;
    if !cli.quiet || exit_code != 0 {
        write_summary(stderr, cli, report, exit_code, style)?;
    }
    Ok(())
}

fn write_phase(writer: &mut impl Write, phase: &PhaseReport, style: Presentation) -> Result<()> {
    let (label, color) = match phase.status {
        PhaseStatus::NotRun => ("not run", "2"),
        PhaseStatus::Running => ("running", "36"),
        PhaseStatus::Passed => ("passed", "32"),
        PhaseStatus::Failed => ("failed", "31"),
        PhaseStatus::Skipped => ("skipped", "2"),
        PhaseStatus::Error => ("error", "31"),
    };
    write!(writer, "{:<13} ", phase_name(phase.phase))?;
    styled(writer, style, color, label)?;
    if let Some(elapsed) = phase.elapsed_ms {
        write!(
            writer,
            " · {}.{:02}s",
            elapsed / 1000,
            (elapsed % 1000) / 10
        )?;
    }
    writeln!(writer)?;
    Ok(())
}

fn write_summary(
    writer: &mut impl Write,
    cli: &Cli,
    report: &Report,
    exit_code: u8,
    style: Presentation,
) -> Result<()> {
    let status = match exit_code {
        0 => "passed",
        1 => "failed",
        _ => "aborted",
    };
    styled(
        writer,
        style,
        if exit_code == 0 { "32" } else { "31" },
        &format!("{} {status}", command_name(&cli.command)),
    )?;
    match &cli.command {
        Command::Format { check: false, .. } if exit_code != 2 => {
            if report.changed == 0 {
                write!(writer, " · no files changed")?;
            } else {
                write!(writer, " · {} formatted", counted(report.changed, "file"))?;
            }
        },
        Command::Format { check: true, .. } | Command::Check { .. } if report.changed > 0 => {
            write!(
                writer,
                " · {} {} formatting",
                counted(report.changed, "file"),
                if report.changed == 1 { "needs" } else { "need" }
            )?;
        },
        _ => {},
    }
    let summary = report.summary();
    if summary.errors > 0 {
        write!(writer, " · {}", counted(summary.errors, "error"))?;
    }
    if summary.warnings > 0 {
        write!(writer, " · {}", counted(summary.warnings, "warning"))?;
    }
    if report.skipped > 0 {
        write!(writer, " · {} skipped", counted(report.skipped, "file"))?;
    }
    if report.clippy.as_ref().is_some_and(|clippy| !clippy.success) {
        write!(writer, " · Clippy failed")?;
    }
    writeln!(writer)?;
    Ok(())
}

fn write_diagnostic(
    writer: &mut impl Write,
    root: &Path,
    diagnostic: &Diagnostic,
    style: Presentation,
) -> Result<()> {
    let color = severity_color(diagnostic.severity);
    if let Some(rendered) = diagnostic
        .compiler
        .as_ref()
        .and_then(|compiler| compiler.rendered.as_deref())
        .filter(|rendered| !rendered.trim().is_empty())
    {
        return write_block(writer, rendered, style, color);
    }
    let code = diagnostic
        .rule
        .as_ref()
        .map_or_else(String::new, |rule| format!("[{rule}]"));
    styled(
        writer,
        style,
        color,
        &format!(
            "{}{code}: {}",
            level_name(diagnostic.severity),
            diagnostic.message
        ),
    )?;
    writeln!(writer)?;
    if let Some(path) = &diagnostic.path {
        write!(writer, "  --> {}", relative_path(root, path).display())?;
        if let Some(line) = diagnostic.line {
            write!(writer, ":{line}")?;
            if let Some(column) = diagnostic.column {
                write!(writer, ":{column}")?;
            }
        }
        writeln!(writer)?;
        if let (Some(line), Some(source)) = (diagnostic.line, &diagnostic.snippet) {
            let width = line.to_string().len().max(3);
            writeln!(
                writer,
                "{:width$} |\n{line:>width$} | {source}\n{:width$} |",
                "", ""
            )?;
        }
    } else {
        writeln!(writer, "  = source: {}", source_name(diagnostic.source))?;
    }
    if let Some(compiler) = &diagnostic.compiler {
        write_spans(writer, root, &compiler.spans, 2, diagnostic.path.is_some())?;
        if compiler
            .spans
            .iter()
            .any(|span| span.suggested_replacement.is_some())
        {
            writeln!(writer, "  = help: suggested edits")?;
            write_edits(writer, root, &compiler.spans, 4)?;
        }
        write_children(writer, root, &compiler.children, 2)?;
    }
    writeln!(writer)?;
    Ok(())
}

/// A child owns its complete edit group; multipart replacements are never flattened.
fn write_children(
    writer: &mut impl Write,
    root: &Path,
    children: &[cargo_metadata::diagnostic::Diagnostic],
    indent: usize,
) -> Result<()> {
    for child in children {
        writeln!(
            writer,
            "{:indent$}= {}: {}",
            "",
            level_name(child.level),
            child.message
        )?;
        write_spans(writer, root, &child.spans, indent + 2, false)?;
        write_edits(writer, root, &child.spans, indent + 2)?;
        write_children(writer, root, &child.children, indent + 2)?;
    }
    Ok(())
}

fn write_spans(
    writer: &mut impl Write,
    root: &Path,
    spans: &[cargo_metadata::diagnostic::DiagnosticSpan],
    indent: usize,
    mut primary_shown: bool,
) -> Result<()> {
    for span in spans {
        if span.suggested_replacement.is_some() {
            continue;
        }
        if span.is_primary && !primary_shown {
            writeln!(
                writer,
                "{:indent$}--> {}:{}:{}",
                "",
                relative_path(root, Path::new(&span.file_name)).display(),
                span.line_start,
                span.column_start
            )?;
        } else if !span.is_primary {
            writeln!(
                writer,
                "{:indent$}::: {}:{}:{}",
                "",
                relative_path(root, Path::new(&span.file_name)).display(),
                span.line_start,
                span.column_start
            )?;
        }
        // Only the first primary location is already supplied by the diagnostic header.
        if span.is_primary {
            primary_shown = false;
        }
        for (offset, line) in span.text.iter().enumerate() {
            writeln!(
                writer,
                "{:indent$}{} | {}",
                "",
                span.line_start + offset,
                line.text
            )?;
        }
        if let Some(label) = &span.label {
            writeln!(writer, "{:indent$}= {label}", "")?;
        }
    }
    Ok(())
}

fn write_edits(
    writer: &mut impl Write,
    root: &Path,
    spans: &[cargo_metadata::diagnostic::DiagnosticSpan],
    indent: usize,
) -> Result<()> {
    for span in spans {
        if let Some(replacement) = &span.suggested_replacement {
            writeln!(
                writer,
                "{:indent$}--> {}:{}:{}–{}:{}",
                "",
                relative_path(root, Path::new(&span.file_name)).display(),
                span.line_start,
                span.column_start,
                span.line_end,
                span.column_end
            )?;
            if replacement.is_empty() {
                writeln!(writer, "{:indent$}remove this range", "")?;
            } else {
                for line in replacement.lines() {
                    writeln!(writer, "{:indent$}+ {line}", "")?;
                }
            }
        }
    }
    Ok(())
}

/// Keep arbitrary child output recognizable and separate from compiler diagnostics.
fn write_log(writer: &mut impl Write, label: &str, text: &str) -> Result<()> {
    if !text.trim().is_empty() {
        writeln!(writer, "{label}:")?;
        for line in text.lines() {
            writeln!(writer, "  {line}")?;
        }
        writeln!(writer)?;
    }
    Ok(())
}

/// Preserve compiler text, styling only the heading and normalizing block separation.
fn write_block(
    writer: &mut impl Write,
    text: &str,
    style: Presentation,
    color: &str,
) -> Result<()> {
    let text = text.trim_end_matches(['\r', '\n']);
    let (heading, body) = text
        .split_once('\n')
        .map_or((text, None), |(heading, body)| (heading, Some(body)));
    styled(writer, style, color, heading)?;
    writeln!(writer)?;
    if let Some(body) = body {
        writeln!(writer, "{body}")?;
    }
    writeln!(writer)?;
    Ok(())
}

fn styled(writer: &mut impl Write, style: Presentation, color: &str, text: &str) -> Result<()> {
    if style.color {
        write!(writer, "\x1b[{color}m{text}\x1b[0m")?;
    } else {
        write!(writer, "{text}")?;
    }
    Ok(())
}

fn relative_path<'a>(root: &Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

fn counted(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Format { check: true, .. } => "Format check",
        Command::Format { check: false, .. } => "Format",
        Command::Lint => "Lint",
        Command::Check { .. } => "Check",
        Command::Config { .. } => "Configuration",
    }
}

fn action_name(command: &Command) -> &'static str {
    match command {
        Command::Format { check: false, .. } => "Formatting",
        Command::Lint => "Linting",
        _ => "Checking",
    }
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Formatting => "Formatting",
        Phase::SourceRules => "Source rules",
        Phase::Clippy => "Clippy",
    }
}

fn source_name(source: DiagnosticSource) -> &'static str {
    match source {
        DiagnosticSource::Rusteward => "rusteward",
        DiagnosticSource::Clippy => "clippy",
        DiagnosticSource::Rustc => "rustc",
    }
}

fn severity_color(level: DiagnosticLevel) -> &'static str {
    match level {
        DiagnosticLevel::Error | DiagnosticLevel::Ice => "31",
        DiagnosticLevel::Warning => "33",
        _ => "36",
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
