//! Prepare the complete rustfmt plus declaration-spacing result before replacing sources.

use std::{fs, io::Write, path::Path, process::Command};

use anyhow::{Context, Result, ensure};
use rusteward_core::{
    Edition,
    diagnostic::{Diagnostic, Severity},
    rules::location,
    spacing::separate_declarations,
};
use similar::TextDiff;
use tempfile::NamedTempFile;
use tracing::{debug, info};

use crate::{
    config::{FormatSettings, Settings, rustfmt_value},
    discovery::{Source, Workspace},
    process,
    report::Report,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub check: bool,
    pub diff: bool,
}

struct Change<'a> {
    source: &'a Source,
    original: String,
    formatted: String,
}

/// Explicit CLI options override an empty config so ambient rustfmt files cannot alter the profile.
///
/// # Errors
/// Returns an error when a configured rustfmt value cannot be encoded as a CLI argument.
pub fn command(
    root: &Path,
    empty_config: &Path,
    edition: Edition,
    settings: &FormatSettings,
) -> Result<Command> {
    let mut command = process::rustfmt(settings.toolchain.as_deref());
    let edition = edition.to_string();
    command
        .current_dir(root)
        .args(["--emit", "stdout", "--edition", &edition])
        .arg("--config-path")
        .arg(empty_config)
        .args(["--config", "skip_children=true"]);
    for (key, value) in &settings.rustfmt {
        command
            .arg("--config")
            .arg(format!("{key}={}", rustfmt_value(value)?));
    }
    Ok(command)
}

/// A check compares the final pipeline output and never writes authored source files.
///
/// # Errors
/// Returns an error for source I/O failures, rejected rustfmt settings or subprocess failures,
/// spacing errors, or conflicts with intervening source edits.
pub fn run(
    workspace: &Workspace,
    sources: &[Source],
    settings: &Settings,
    options: Options,
) -> Result<Report> {
    let mut report = Report {
        files: sources.len(),
        ..Report::default()
    };
    let mut changes = Vec::new();
    let empty_config =
        NamedTempFile::new().context("cannot create isolated rustfmt configuration")?;
    info!(
        files = sources.len(),
        check = options.check,
        "starting format workflow"
    );
    for source in sources {
        let original = fs::read_to_string(&source.path)
            .with_context(|| format!("cannot read {}", source.path.display()))?;
        if rusteward_core::is_generated(&original) {
            report.skipped += 1;
            debug!(path = %source.path.display(), "skipping generated source");
            continue;
        }
        let invocation = command(
            &workspace.root,
            empty_config.path(),
            source.edition,
            &settings.format,
        )?;
        let output = process::with_input(invocation, &original, &source.path)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        ensure!(
            output.status.success(),
            "rustfmt failed for {} ({}): {}",
            source.path.display(),
            output.status,
            stderr.trim()
        );
        // rustfmt can accept unknown settings with a warning; never claim those settings were applied.
        ensure!(
            stderr.trim().is_empty(),
            "rustfmt reported a warning for {}: {}",
            source.path.display(),
            stderr.trim()
        );
        let mut formatted =
            String::from_utf8(output.stdout).context("rustfmt emitted non-UTF-8 output")?;
        if settings.format.spacing {
            let spacing = separate_declarations(&formatted, source.edition)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("cannot space {}", source.path.display()))?;
            if let Some(reason) = spacing.skip_reason {
                report.skipped += 1;
                debug!(path = %source.path.display(), reason, "skipping declaration spacing");
            }
            formatted = spacing.text;
        }
        if original == formatted {
            continue;
        }
        let path = source
            .path
            .strip_prefix(&workspace.root)
            .unwrap_or(&source.path);
        if options.diff {
            report.diffs.push(diff(path, &original, &formatted));
        }
        if options.check {
            let (line, column) = first_difference(&original, &formatted);
            report.extend_custom([Diagnostic {
                path: path.into(), line, column, rule: "format", severity: Severity::Error,
                message: "source differs from the rustfmt plus declaration-spacing result; run cargo dev format".into(),
            }]);
        }
        changes.push(Change {
            source,
            original,
            formatted,
        });
    }
    report.changed = changes.len();
    if !options.check {
        // Check all originals before any replacement, so editor changes cause an explicit conflict.
        for change in &changes {
            let current = fs::read_to_string(&change.source.path)?;
            ensure!(
                current == change.original,
                "{} changed during formatting; retry with the updated source",
                change.source.path.display()
            );
        }
        for change in &changes {
            replace(&change.source.path, &change.formatted)?;
            info!(path = %change.source.path.display(), "formatted source file");
        }
    }
    info!(
        changed = report.changed,
        skipped = report.skipped,
        "format workflow complete"
    );
    Ok(report)
}

/// Replace each file atomically while retaining its permissions.
fn replace(path: &Path, text: &str) -> Result<()> {
    let parent = path.parent().context("source file has no parent")?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("cannot stage {}", path.display()))?;
    temporary
        .as_file()
        .set_permissions(fs::metadata(path)?.permissions())?;
    temporary.write_all(text.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

/// Report the first character mismatch using the original file's coordinates.
pub fn first_difference(original: &str, formatted: &str) -> (usize, usize) {
    let offset = original
        .chars()
        .zip(formatted.chars())
        .take_while(|(left, right)| left == right)
        .map(|(character, _)| character.len_utf8())
        .sum();
    location(original, offset)
}

pub fn diff(path: &Path, original: &str, formatted: &str) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    TextDiff::from_lines(original, formatted)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

#[cfg(test)]
mod tests;
