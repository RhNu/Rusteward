//! Track each workflow stage and retain completed results when an operation fails.

use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{debug, info};

use crate::{
    config::Settings,
    discovery::{Source, Workspace},
    execution::Executor,
    format, lint,
    process::CargoOptions,
    report::{DiagnosticLevel, DiagnosticSource, Phase, PhaseReport, PhaseStatus, Report},
};

/// Select workflow stages and formatting behavior without carrying CLI arguments.
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Format(format::Options),
    Lint,
    Check { diff: bool },
}

/// Observers receive stage boundaries without imposing a presentation format.
#[derive(Clone, Copy, Debug)]
pub enum Event<'a> {
    Started(Phase),
    Finished(&'a PhaseReport),
}

/// Keep the partial report beside an optional operational failure.
#[derive(Debug)]
pub struct Outcome {
    pub report: Report,
    pub error: Option<anyhow::Error>,
}

impl Outcome {
    /// Operational failures take precedence over unsuccessful checks.
    pub fn exit_code(&self) -> u8 {
        if self.error.is_some() {
            2
        } else {
            u8::from(self.report.failed(false))
        }
    }
}

/// Execute the selected stages, keeping earlier results on an operational error.
/// Aborting a stage marks it as errored and leaves subsequent stages unstarted.
pub fn run(
    workspace: &Workspace,
    sources: &[Source],
    settings: &Settings,
    executor: &Executor,
    cargo: &CargoOptions,
    command: Command,
    observer: &mut impl FnMut(Event<'_>) -> Result<()>,
) -> Outcome {
    let phases = match command {
        Command::Format(_) => vec![Phase::Formatting],
        Command::Lint => vec![Phase::SourceRules, Phase::Clippy],
        Command::Check { .. } => vec![Phase::Formatting, Phase::SourceRules, Phase::Clippy],
    };
    let report = Report {
        files: sources.len(),
        phases: phases
            .into_iter()
            .map(|phase| PhaseReport {
                phase,
                status: PhaseStatus::NotRun,
                elapsed_ms: None,
            })
            .collect(),
        ..Report::default()
    };
    execute(
        report,
        settings.lint.deny_warnings,
        settings.lint.clippy,
        observer,
        |phase, report| {
            match phase {
                Phase::Formatting => {
                    let options = match command {
                        Command::Format(options) => options,
                        Command::Check { diff } => format::Options { check: true, diff },
                        Command::Lint => unreachable!("lint has no formatting stage"),
                    };
                    report.append(format::run(
                        workspace, sources, settings, executor, options,
                    )?);
                },
                Phase::SourceRules => {
                    report.append(lint::inspect(workspace, sources, settings, executor)?);
                },
                Phase::Clippy => lint::clippy(workspace, settings, cargo, report)?,
            }
            Ok(())
        },
    )
}

/// Evaluate a stage against only its new findings so previous failures cannot taint later stages.
fn stage_failed(
    report: &Report,
    phase: Phase,
    diagnostic_start: usize,
    deny_warnings: bool,
) -> bool {
    report.diagnostics[diagnostic_start..]
        .iter()
        .any(|diagnostic| {
            diagnostic.is_error()
                || (deny_warnings
                    && diagnostic.source == DiagnosticSource::Rusteward
                    && diagnostic.severity == DiagnosticLevel::Warning)
        })
        || (phase == Phase::Clippy && report.clippy.as_ref().is_some_and(|clippy| !clippy.success))
}

/// The operation boundary also permits pure tests of abort and continuation behavior.
fn execute(
    mut report: Report,
    deny_warnings: bool,
    clippy_enabled: bool,
    observer: &mut impl FnMut(Event<'_>) -> Result<()>,
    mut operation: impl FnMut(Phase, &mut Report) -> Result<()>,
) -> Outcome {
    for index in 0..report.phases.len() {
        let phase = report.phases[index].phase;
        if phase == Phase::Clippy && !clippy_enabled {
            report.phases[index].status = PhaseStatus::Skipped;
            info!(?phase, "workflow stage skipped");
            if let Err(error) = observer(Event::Finished(&report.phases[index])) {
                report.phases[index].status = PhaseStatus::Error;
                return Outcome {
                    report,
                    error: Some(error.context("cannot report skipped workflow stage")),
                };
            }
            continue;
        }
        report.phases[index].status = PhaseStatus::Running;
        let started = Instant::now();
        info!(?phase, "workflow stage started");
        let diagnostic_start = report.diagnostics.len();
        let result = observer(Event::Started(phase))
            .context("cannot report workflow stage start")
            .and_then(|()| operation(phase, &mut report));
        report.phases[index].elapsed_ms = Some(started.elapsed().as_millis());
        report.phases[index].status = if result.is_err() {
            PhaseStatus::Error
        } else if stage_failed(&report, phase, diagnostic_start, deny_warnings) {
            PhaseStatus::Failed
        } else {
            PhaseStatus::Passed
        };
        info!(?phase, status = ?report.phases[index].status, elapsed_ms = ?report.phases[index].elapsed_ms, "workflow stage finished");
        let presentation = observer(Event::Finished(&report.phases[index]))
            .context("cannot report workflow stage completion");
        if let Err(error) = result.and(presentation) {
            report.phases[index].status = PhaseStatus::Error;
            debug!(?phase, status = ?report.phases[index].status, "workflow aborted");
            return Outcome {
                report,
                error: Some(error),
            };
        }
    }
    Outcome {
        report,
        error: None,
    }
}

#[cfg(test)]
mod tests;
