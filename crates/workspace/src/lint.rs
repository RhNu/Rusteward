//! Combine authored-source policies with the selected Cargo Clippy invocation.

use std::{ffi::OsString, fs, path::Path, process::Command};

use anyhow::{Context, Result};
use rusteward_core::rules::inspect;
use tracing::{debug, info};

use crate::{
    config::{LintSettings, Settings, clippy_config_toml},
    discovery::{Source, Workspace},
    process::{self, CargoOptions},
    report::{ClippyReport, Report},
};

/// Isolate Clippy's parameter discovery and apply groups before individual lint overrides.
pub fn command(
    workspace: &Workspace,
    settings: &LintSettings,
    options: &CargoOptions,
    configuration_directory: &Path,
) -> Command {
    let mut arguments: Vec<OsString> = [
        "clippy",
        "--workspace",
        "--message-format=json",
        "--color=never",
        "--manifest-path",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    arguments.push(workspace.manifest.as_os_str().into());
    if options.locked {
        arguments.push("--locked".into());
    }
    if options.offline {
        arguments.push("--offline".into());
    }
    if settings.all_targets {
        arguments.push("--all-targets".into());
    }
    if settings.all_features {
        arguments.push("--all-features".into());
    }
    if settings.no_default_features {
        arguments.push("--no-default-features".into());
    }
    if !settings.features.is_empty() {
        arguments.push("--features".into());
        arguments.push(settings.features.join(",").into());
    }
    arguments.extend(["--", "-D", "warnings"].into_iter().map(OsString::from));
    arguments.extend(lint_flags(settings));
    arguments.extend(settings.clippy_args.iter().map(OsString::from));
    let mut command = process::cargo(settings.toolchain.as_deref());
    command
        .current_dir(&workspace.root)
        .env("CLIPPY_CONF_DIR", configuration_directory)
        .args(arguments);
    command
}

/// Group flags must precede specific flags even when a map sorts their names differently.
fn lint_flags(settings: &LintSettings) -> Vec<OsString> {
    const GROUPS: &[&str] = &[
        "all",
        "correctness",
        "suspicious",
        "style",
        "complexity",
        "perf",
        "pedantic",
        "restriction",
        "nursery",
        "cargo",
        "deprecated",
    ];
    let mut flags = Vec::new();
    for group in GROUPS {
        if let Some(level) = settings.clippy_lints.get(*group) {
            flags.extend([
                OsString::from(level.flag()),
                format!("clippy::{group}").into(),
            ]);
        }
    }
    for (name, level) in &settings.clippy_lints {
        if !GROUPS.contains(&name.as_str()) {
            flags.extend([
                OsString::from(level.flag()),
                format!("clippy::{name}").into(),
            ]);
        }
    }
    flags
}

/// Collect custom diagnostics even if Clippy later reports compilation or lint failures.
///
/// # Errors
/// Returns an error when source reads, Clippy configuration or execution, or compiler-message
/// decoding fails.
pub fn run(
    workspace: &Workspace,
    sources: &[Source],
    settings: &Settings,
    options: &CargoOptions,
) -> Result<Report> {
    let mut report = Report {
        files: sources.len(),
        ..Report::default()
    };
    for file in sources {
        let source = fs::read_to_string(&file.path)
            .with_context(|| format!("cannot read {}", file.path.display()))?;
        if rusteward_core::is_generated(&source) {
            report.skipped += 1;
            debug!(path = %file.path.display(), "skipping generated source");
            continue;
        }
        let path = file
            .path
            .strip_prefix(&workspace.root)
            .unwrap_or(&file.path);
        let diagnostics = inspect(path, &source, file.edition, &settings.rules);
        debug!(path = %path.display(), diagnostics = diagnostics.len(), "inspected source rules");
        report.extend_custom(diagnostics);
    }
    if settings.lint.clippy {
        let configuration_directory =
            tempfile::tempdir().context("cannot create isolated Clippy configuration directory")?;
        let configuration = clippy_config_toml(&settings.lint.clippy_config)?;
        fs::write(
            configuration_directory.path().join("clippy.toml"),
            configuration,
        )
        .context("cannot write isolated Clippy configuration")?;
        let mut invocation = command(
            workspace,
            &settings.lint,
            options,
            configuration_directory.path(),
        );
        info!(toolchain = ?settings.lint.toolchain, lints = settings.lint.clippy_lints.len(), parameters = settings.lint.clippy_config.len(), "running cargo clippy with managed profile");
        debug!(arguments = ?invocation.get_args().collect::<Vec<_>>(), configuration = %configuration_directory.path().display(), "constructed Clippy invocation");
        let output = invocation.output().context(
            "cannot start cargo clippy; install the clippy component for the selected toolchain",
        )?;
        let (parsed, success) = messages::parse(&output.stdout, &workspace.root)
            .and_then(|parsed| {
                let success = parsed.success(output.status.success())?;
                Ok((parsed, success))
            })
            .with_context(|| {
                format!(
                    "cannot interpret Clippy output ({}); stdout:\n{}\nstderr:\n{}",
                    output.status,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            })?;
        debug!(status = %output.status, build_success = ?parsed.build_success, diagnostics = parsed.diagnostics.len(), "collected Clippy results");
        report.diagnostics.extend(parsed.diagnostics);
        report.clippy = Some(ClippyReport {
            success,
            exit_code: output.status.code(),
            build_success: parsed.build_success,
            output: parsed.output,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    info!(
        files = report.files,
        findings = report.diagnostics.len(),
        "lint workflow complete"
    );
    Ok(report)
}

#[cfg(test)]
mod tests;

mod messages;
