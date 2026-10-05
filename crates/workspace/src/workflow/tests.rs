use anyhow::{Result, bail};
use rusteward_core::diagnostic::{Diagnostic, Severity};

use super::{Event, execute};
use crate::report::{ClippyReport, Phase, PhaseReport, PhaseStatus, Report};

fn pending() -> Report {
    Report {
        files: 3,
        phases: [Phase::Formatting, Phase::SourceRules, Phase::Clippy]
            .into_iter()
            .map(|phase| PhaseReport {
                phase,
                status: PhaseStatus::NotRun,
                elapsed_ms: None,
            })
            .collect(),
        ..Report::default()
    }
}

fn finding(report: &mut Report, severity: Severity) {
    report.extend_custom(
        [Diagnostic {
            path: "src/lib.rs".into(),
            line: 1,
            column: 1,
            rule: "synthetic-rule",
            severity,
            message: "synthetic finding".into(),
        }],
        "original source",
    );
}

fn statuses(report: &Report) -> Vec<PhaseStatus> {
    report.phases.iter().map(|phase| phase.status).collect()
}

#[test]
fn continues_after_failed_checks_and_scopes_each_stage_to_its_own_findings() {
    let mut events = Vec::new();
    let outcome = execute(
        pending(),
        false,
        true,
        &mut |event| {
            events.push(match event {
                Event::Started(phase) => (phase, PhaseStatus::Running),
                Event::Finished(report) => (report.phase, report.status),
            });
            Ok(())
        },
        |phase, report| {
            if phase == Phase::Formatting {
                finding(report, Severity::Error);
                report.changed = 1;
                report.diffs.push("source diff".into());
            }
            Ok(())
        },
    );
    assert_eq!(outcome.exit_code(), 1);
    assert!(outcome.error.is_none());
    assert_eq!(
        statuses(&outcome.report),
        [
            PhaseStatus::Failed,
            PhaseStatus::Passed,
            PhaseStatus::Passed
        ]
    );
    assert_eq!(
        events,
        [
            (Phase::Formatting, PhaseStatus::Running),
            (Phase::Formatting, PhaseStatus::Failed),
            (Phase::SourceRules, PhaseStatus::Running),
            (Phase::SourceRules, PhaseStatus::Passed),
            (Phase::Clippy, PhaseStatus::Running),
            (Phase::Clippy, PhaseStatus::Passed),
        ]
    );
    assert_eq!(outcome.report.changed, 1);
    assert_eq!(outcome.report.diffs, ["source diff"]);
    assert!(
        outcome
            .report
            .phases
            .iter()
            .all(|phase| phase.elapsed_ms.is_some())
    );
}

#[test]
fn abort_retains_completed_results_and_leaves_remaining_stages_not_run() {
    let outcome = execute(pending(), false, true, &mut |_| Ok(()), |phase, report| {
        match phase {
            Phase::Formatting => {
                finding(report, Severity::Error);
                report.changed = 2;
            },
            Phase::SourceRules => bail!("cannot read selected source"),
            Phase::Clippy => panic!("aborted workflow must not run Clippy"),
        }
        Ok(())
    });
    assert_eq!(outcome.exit_code(), 2);
    assert!(outcome.error.unwrap().to_string().contains("cannot read"));
    assert_eq!(outcome.report.changed, 2);
    assert_eq!(outcome.report.diagnostics.len(), 1);
    assert_eq!(
        statuses(&outcome.report),
        [PhaseStatus::Failed, PhaseStatus::Error, PhaseStatus::NotRun]
    );
    assert!(outcome.report.phases[2].elapsed_ms.is_none());
}

#[test]
fn denied_custom_warnings_fail_only_the_source_rules_stage() {
    for deny_warnings in [false, true] {
        let outcome = execute(
            pending(),
            deny_warnings,
            true,
            &mut |_| Ok(()),
            |phase, report| {
                if phase == Phase::SourceRules {
                    finding(report, Severity::Warning);
                }
                Ok(())
            },
        );
        assert_eq!(outcome.exit_code(), u8::from(deny_warnings));
        assert_eq!(
            outcome.report.phases[1].status,
            if deny_warnings {
                PhaseStatus::Failed
            } else {
                PhaseStatus::Passed
            }
        );
        assert_eq!(outcome.report.phases[2].status, PhaseStatus::Passed);
    }
}

#[test]
fn disabled_clippy_emits_a_skipped_result_without_execution() {
    let mut skipped = 0;
    let outcome = execute(
        pending(),
        false,
        false,
        &mut |event| {
            if let Event::Finished(report) = event
                && report.status == PhaseStatus::Skipped
            {
                skipped += 1;
            }
            Ok(())
        },
        |phase, _| {
            assert_ne!(phase, Phase::Clippy);
            Ok(())
        },
    );
    assert_eq!(outcome.exit_code(), 0);
    assert_eq!(skipped, 1);
    assert_eq!(outcome.report.phases[2].status, PhaseStatus::Skipped);
    assert!(outcome.report.phases[2].elapsed_ms.is_none());
}

#[test]
fn clippy_process_failure_without_diagnostics_fails_its_stage() {
    let outcome = execute(pending(), false, true, &mut |_| Ok(()), |phase, report| {
        if phase == Phase::Clippy {
            report.clippy = Some(ClippyReport {
                success: false,
                exit_code: Some(101),
                build_success: None,
                output: String::new(),
                stderr: "Cargo failed".into(),
            });
        }
        Ok(())
    });
    assert_eq!(outcome.exit_code(), 1);
    assert_eq!(outcome.report.summary().errors, 0);
    assert_eq!(outcome.report.phases[2].status, PhaseStatus::Failed);
}

#[test]
fn observer_failure_stops_operations_and_reports_an_operational_error() {
    let outcome = execute(
        pending(),
        false,
        true,
        &mut |_| -> Result<()> { bail!("output stream closed") },
        |_, _| panic!("failed start notification must prevent execution"),
    );
    assert_eq!(outcome.exit_code(), 2);
    assert_eq!(
        statuses(&outcome.report),
        [PhaseStatus::Error, PhaseStatus::NotRun, PhaseStatus::NotRun]
    );
}

#[test]
fn completion_notification_failure_retains_the_operation_results() {
    let outcome = execute(
        pending(),
        false,
        true,
        &mut |event| {
            if matches!(event, Event::Finished(_)) {
                bail!("cannot write completion")
            }
            Ok(())
        },
        |_, report| {
            finding(report, Severity::Error);
            Ok(())
        },
    );
    assert_eq!(outcome.exit_code(), 2);
    assert_eq!(outcome.report.diagnostics.len(), 1);
    assert_eq!(outcome.report.phases[0].status, PhaseStatus::Error);
    assert_eq!(outcome.report.phases[1].status, PhaseStatus::NotRun);
}
