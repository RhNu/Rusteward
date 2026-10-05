//! Combine authored-source policies with the selected Cargo Clippy invocation.

use std::{ffi::OsString, fs, path::Path, process::Command, time::Instant};

use anyhow::{Context, Result};
use rusteward_core::rules::{Rules, inspect as inspect_rules};
use tracing::{debug, info};

use crate::{
    config::{LintSettings, Settings, clippy_config_toml},
    discovery::{Source, Workspace},
    execution::Executor,
    process::{self, CargoOptions},
    report::{ClippyReport, Diagnostic, Report},
};

/// Independent source findings are merged by the caller in discovery order.
struct Inspection {
    skipped: bool,
    diagnostics: Vec<Diagnostic>,
}

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
        // Failed dependencies still block their dependents; continue independent targets.
        "--keep-going",
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

/// Collect custom source diagnostics independently from Cargo execution.
///
/// # Errors
/// Returns an error when a source cannot be read or the executor cannot complete its tasks.
pub fn inspect(
    workspace: &Workspace,
    sources: &[Source],
    settings: &Settings,
    executor: &Executor,
) -> Result<Report> {
    let started = Instant::now();
    let mut report = Report {
        files: sources.len(),
        ..Report::default()
    };
    info!(
        files = sources.len(),
        jobs = executor.jobs(),
        "starting source inspection"
    );
    let results = executor.map(sources, |file| {
        let file_started = Instant::now();
        let result = fs::read_to_string(&file.path)
            .with_context(|| format!("cannot read {}", file.path.display()))
            .map(|source| inspect_source(file, &workspace.root, &source, &settings.rules));
        debug!(path = %file.path.display(), elapsed_ms = file_started.elapsed().as_millis(), success = result.is_ok(), "finished source inspection task");
        result
    })?;
    for result in results {
        report.skipped += usize::from(result.skipped);
        report.diagnostics.extend(result.diagnostics);
    }
    info!(
        files = report.files,
        skipped = report.skipped,
        findings = report.diagnostics.len(),
        elapsed_ms = started.elapsed().as_millis(),
        "source inspection complete"
    );
    Ok(report)
}

/// Run the managed Clippy profile and retain process output before decoding its records.
///
/// # Errors
/// Returns an error for configuration, execution, or compiler-message decoding failures.
pub fn clippy(
    workspace: &Workspace,
    settings: &Settings,
    options: &CargoOptions,
    report: &mut Report,
) -> Result<()> {
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
    let clippy_started = Instant::now();
    let output = invocation.output().context(
        "cannot start cargo clippy; install the clippy component for the selected toolchain",
    )?;
    decode_output(
        &output.stdout,
        &output.stderr,
        &workspace.root,
        output.status.success(),
        output.status.code(),
        report,
    )
    .with_context(|| format!("cannot interpret Clippy output ({})", output.status))?;
    info!(
        success = report.clippy.as_ref().is_some_and(|clippy| clippy.success),
        elapsed_ms = clippy_started.elapsed().as_millis(),
        "Clippy execution complete"
    );
    Ok(())
}

/// Capture raw output first so a protocol failure remains inspectable in the partial report.
fn decode_output(
    stdout: &[u8],
    stderr: &[u8],
    root: &Path,
    process_success: bool,
    exit_code: Option<i32>,
    report: &mut Report,
) -> Result<()> {
    report.clippy = Some(ClippyReport {
        success: false,
        exit_code,
        build_success: None,
        output: String::from_utf8_lossy(stdout).into_owned(),
        stderr: String::from_utf8_lossy(stderr).into_owned(),
    });
    let parsed = messages::parse(stdout, root)?;
    let success = parsed.success(process_success)?;
    debug!(build_success = ?parsed.build_success, diagnostics = parsed.diagnostics.len(), "collected Clippy results");
    report.diagnostics.extend(parsed.diagnostics);
    report.clippy = Some(ClippyReport {
        success,
        exit_code,
        build_success: parsed.build_success,
        output: parsed.output,
        stderr: String::from_utf8_lossy(stderr).into_owned(),
    });
    Ok(())
}

/// Inspect an in-memory source and keep generated-file accounting beside its findings.
fn inspect_source(file: &Source, root: &Path, source: &str, rules: &Rules) -> Inspection {
    if rusteward_core::is_generated(source) {
        debug!(path = %file.path.display(), "skipping generated source");
        return Inspection {
            skipped: true,
            diagnostics: Vec::new(),
        };
    }
    let path = file.path.strip_prefix(root).unwrap_or(&file.path);
    let diagnostics = inspect_rules(path, source, file.edition, rules)
        .into_iter()
        .filter_map(|diagnostic| Diagnostic::custom(diagnostic, source))
        .collect::<Vec<_>>();
    debug!(path = %path.display(), diagnostics = diagnostics.len(), "inspected source rules");
    Inspection {
        skipped: false,
        diagnostics,
    }
}

#[cfg(test)]
mod tests;

mod messages;
