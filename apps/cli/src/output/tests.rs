use std::path::Path;

use clap::Parser;
use rusteward_workspace::{
    report::{
        ClippyReport, CompilerDetails, Diagnostic, DiagnosticLevel, DiagnosticSource, Phase,
        PhaseReport, PhaseStatus, Report,
    },
    workflow::Outcome,
};
use serde_json::{Value, json};

use super::{Presentation, write_error, write_report};
use crate::args::Cli;

fn finding(source: DiagnosticSource, severity: DiagnosticLevel) -> Diagnostic {
    Diagnostic {
        source,
        rule: Some("sample-rule".into()),
        severity,
        message: "something needs attention".into(),
        path: Some("src/lib.rs".into()),
        line: Some(3),
        column: Some(7),
        snippet: None,
        compiler: None,
    }
}

fn child_logs(success: bool) -> ClippyReport {
    ClippyReport {
        success,
        exit_code: Some(if success { 0 } else { 101 }),
        build_success: Some(success),
        output: "custom build output\n".into(),
        stderr: "Cargo build context\n".into(),
    }
}

fn phase(phase: Phase, status: PhaseStatus) -> PhaseReport {
    PhaseReport {
        phase,
        status,
        elapsed_ms: if matches!(status, PhaseStatus::Skipped | PhaseStatus::NotRun) {
            None
        } else {
            Some(1234)
        },
    }
}

fn outcome(report: Report) -> Outcome {
    Outcome {
        report,
        error: None,
    }
}

fn render(
    outcome: &Outcome,
    command: &[&str],
    flags: &[&str],
    style: Presentation,
) -> (String, String) {
    let cli = Cli::try_parse_from(
        ["cargo-dev"]
            .into_iter()
            .chain(command.iter().copied())
            .chain(flags.iter().copied()),
    )
    .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    write_report(
        &cli,
        Path::new("project"),
        outcome,
        style,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    (
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

fn text(outcome: &Outcome, flags: &[&str]) -> (String, String) {
    render(outcome, &["lint"], flags, Presentation::default())
}

fn edit(replacement: &str, line: usize) -> Value {
    json!({
        "file_name": "src/lib.rs", "byte_start": 4, "byte_end": 5,
        "line_start": line, "line_end": line, "column_start": 7, "column_end": 8,
        "is_primary": false, "text": [], "label": null,
        "suggested_replacement": replacement, "suggestion_applicability": "MachineApplicable", "expansion": null
    })
}

fn compiler_finding(rendered: Option<&str>) -> Diagnostic {
    let mut diagnostic = finding(DiagnosticSource::Clippy, DiagnosticLevel::Error);
    diagnostic.compiler = Some(CompilerDetails {
        package_id: serde_json::from_value(json!("path+file:///project#sample@0.1.0")).unwrap(),
        target: serde_json::from_value(json!({
            "name": "sample", "kind": ["lib"], "crate_types": ["lib"], "src_path": "/project/src/lib.rs",
            "edition": "2024", "doc": true, "doctest": true, "test": true
        })).unwrap(),
        spans: vec![],
        children: vec![serde_json::from_value(json!({
            "message": "replace both pieces", "code": null, "level": "help",
            "spans": [edit("assert_eq", 3), edit("items,\n    []", 4)],
            "children": [{"message": "edits belong to one suggestion", "code": null, "level": "note", "spans": [], "children": [], "rendered": null}],
            "rendered": null
        })).unwrap()],
        explanation: None, rendered: rendered.map(str::to_owned),
    });
    diagnostic
}

#[test]
fn native_compiler_context_is_complete_and_emitted_once_in_every_text_mode() {
    let rendered = "error: use a more descriptive assertion\n  --> src/lib.rs:3:7\n   |\n 3 | assert!(items.is_empty());\n   | ^^^^^^^^^^^^^^^^^^^^^^^^^\n   = help: use assert_eq!(items, [])\n\n";
    let outcome = outcome(Report {
        diagnostics: vec![compiler_finding(Some(rendered))],
        ..Report::default()
    });
    for flags in [vec![], vec!["-q"], vec!["-v"], vec!["-vv"]] {
        let (stdout, stderr) = text(&outcome, &flags);
        assert_eq!(stdout, "");
        assert!(stderr.contains(rendered));
        assert_eq!(
            stderr
                .matches("error: use a more descriptive assertion")
                .count(),
            1
        );
        assert!(!stderr.contains("something needs attention"));
        assert!(!stderr.contains("replace both pieces"));
    }
}

#[test]
fn compiler_errors_explain_failed_processes_and_verbose_adds_labelled_logs() {
    let outcome = outcome(Report {
        diagnostics: vec![compiler_finding(Some("error: compiler finding\n"))],
        clippy: Some(child_logs(false)),
        ..Report::default()
    });
    for flags in [vec![], vec!["-q"], vec!["-v"], vec!["-vv"]] {
        let (_, stderr) = text(&outcome, &flags);
        assert_eq!(stderr.matches("error: compiler finding").count(), 1);
        assert!(stderr.contains("Lint failed"));
        if flags.contains(&"-v") || flags.contains(&"-vv") {
            assert!(stderr.contains("Cargo stdout:\n  custom build output\n\n"));
            assert!(stderr.contains("Cargo stderr:\n  Cargo build context\n\n"));
        } else {
            assert!(!stderr.contains("custom build output"));
            assert!(!stderr.contains("Cargo build context"));
        }
    }
}

#[test]
fn successful_child_logs_are_hidden_until_verbose_is_requested() {
    let outcome = outcome(Report {
        clippy: Some(child_logs(true)),
        ..Report::default()
    });
    let (_, default) = text(&outcome, &[]);
    assert!(!default.contains("custom build output"));
    assert!(!default.contains("Cargo build context"));
    let (_, verbose) = text(&outcome, &["-v"]);
    assert!(verbose.contains("Cargo stdout:"));
    assert!(verbose.contains("Cargo stderr:"));
}

#[test]
fn quiet_process_failure_without_diagnostics_keeps_logs_and_exit_status() {
    let outcome = outcome(Report {
        clippy: Some(child_logs(false)),
        ..Report::default()
    });
    let (stdout, stderr) = text(&outcome, &["-q"]);
    assert_eq!(stdout, "");
    assert!(stderr.contains("Cargo stdout:\n  custom build output"));
    assert!(stderr.contains("Cargo stderr:\n  Cargo build context"));
    assert!(stderr.contains("Clippy failed · process exit: 101 · Cargo build: failed"));
    assert!(stderr.contains("Lint failed"));
}

#[test]
fn failed_process_with_no_output_still_reports_available_exit_information() {
    let outcome = outcome(Report {
        clippy: Some(ClippyReport {
            success: false,
            exit_code: None,
            build_success: None,
            output: String::new(),
            stderr: String::new(),
        }),
        ..Report::default()
    });
    for flags in [vec![], vec!["-q"]] {
        let (_, stderr) = text(&outcome, &flags);
        assert!(
            stderr.contains("Clippy failed · process exit: unavailable · Cargo build: unavailable")
        );
        assert!(stderr.contains("Lint failed"));
        assert!(!stderr.contains("Cargo stdout:"));
        assert!(!stderr.contains("Cargo stderr:"));
    }
}

#[test]
fn custom_findings_show_original_source_with_blank_block_separation() {
    let mut first = finding(DiagnosticSource::Rusteward, DiagnosticLevel::Warning);
    first.snippet = Some("    original_call(é);".into());
    let mut second = finding(DiagnosticSource::Rusteward, DiagnosticLevel::Error);
    second.message = "second finding".into();
    let outcome = outcome(Report {
        diagnostics: vec![first, second],
        ..Report::default()
    });
    let (_, stderr) = text(&outcome, &[]);
    assert!(
        stderr.contains("warning[sample-rule]: something needs attention\n  --> src/lib.rs:3:7")
    );
    assert!(stderr.contains("  3 |     original_call(é);"));
    assert!(stderr.contains("   |\n\nerror[sample-rule]: second finding"));
    assert!(stderr.contains("\n\nLint failed · 1 error · 1 warning\n"));
}

#[test]
fn source_less_errors_identify_the_producer_without_inventing_coordinates() {
    let mut diagnostic = finding(DiagnosticSource::Rustc, DiagnosticLevel::Error);
    diagnostic.path = None;
    diagnostic.line = None;
    diagnostic.column = None;
    diagnostic.rule = None;
    let outcome = outcome(Report {
        diagnostics: vec![diagnostic],
        ..Report::default()
    });
    let (_, stderr) = text(&outcome, &[]);
    assert!(stderr.starts_with("error: something needs attention\n  = source: rustc\n\n"));
    assert!(!stderr.contains("-->"));
    assert!(stderr.contains("Lint failed · 1 error"));
}

#[test]
fn fallback_groups_multipart_suggestions_and_preserves_real_newlines() {
    for rendered in [None, Some("  \n")] {
        let outcome = outcome(Report {
            diagnostics: vec![compiler_finding(rendered)],
            ..Report::default()
        });
        for flags in [vec![], vec!["-q"], vec!["-v"]] {
            let (_, stderr) = text(&outcome, &flags);
            assert_eq!(stderr.matches("help: replace both pieces").count(), 1);
            let heading = stderr.find("help: replace both pieces").unwrap();
            let first = stderr.find("+ assert_eq").unwrap();
            let second = stderr.find("+ items,").unwrap();
            let note = stderr.find("note: edits belong to one suggestion").unwrap();
            assert!(heading < first && first < second && second < note);
            assert!(stderr.contains("+ items,\n    +     []\n"));
            assert!(!stderr.contains("items,\\n"));
            assert!(!stderr.contains("MachineApplicable"));
        }
    }
}

#[test]
fn json_is_a_single_mode_independent_result_with_findings_phases_and_logs() {
    let mut diagnostic = compiler_finding(Some("error: compiler context\n"));
    diagnostic.snippet = Some("original source".into());
    let outcome = outcome(Report {
        files: 4,
        phases: vec![
            phase(Phase::SourceRules, PhaseStatus::Passed),
            phase(Phase::Clippy, PhaseStatus::Failed),
        ],
        diagnostics: vec![diagnostic],
        diffs: vec!["a diff\n".into()],
        clippy: Some(child_logs(false)),
        ..Report::default()
    });
    let mut results = Vec::new();
    for flags in [
        vec!["--json"],
        vec!["--json", "-q"],
        vec!["--json", "-vv"],
        vec!["--json", "--color=always"],
    ] {
        let (stdout, stderr) = text(&outcome, &flags);
        assert_eq!(stderr, "");
        assert_eq!(stdout.lines().count(), 1);
        results.push(serde_json::from_str::<Value>(&stdout).unwrap());
    }
    assert!(results.windows(2).all(|pair| pair[0] == pair[1]));
    let result = &results[0];
    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["success"], false);
    assert_eq!(result["exit_code"], 1);
    assert_eq!(result["summary"]["errors"], 1);
    assert_eq!(result["report"]["phases"][1]["status"], "failed");
    assert_eq!(result["report"]["phases"][1]["elapsed_ms"], 1234);
    assert_eq!(
        result["report"]["diagnostics"][0]["snippet"],
        "original source"
    );
    assert_eq!(
        result["report"]["diagnostics"][0]["compiler"]["children"][0]["spans"][1]["suggested_replacement"],
        "items,\n    []"
    );
    assert_eq!(
        result["report"]["clippy"]["output"],
        "custom build output\n"
    );
    assert_eq!(
        result["report"]["clippy"]["stderr"],
        "Cargo build context\n"
    );
    assert_eq!(result["report"]["diffs"][0], "a diff\n");
}

#[test]
fn operational_failure_json_preserves_partial_results_and_returns_exit_two() {
    let outcome = Outcome {
        report: Report {
            changed: 2,
            phases: vec![
                phase(Phase::Formatting, PhaseStatus::Failed),
                phase(Phase::Clippy, PhaseStatus::Error),
            ],
            diagnostics: vec![finding(DiagnosticSource::Rusteward, DiagnosticLevel::Error)],
            clippy: Some(child_logs(false)),
            ..Report::default()
        },
        error: Some(
            anyhow::anyhow!("malformed compiler record").context("cannot decode Clippy output"),
        ),
    };
    for flags in [vec!["--json"], vec!["--json", "-q"], vec!["--json", "-v"]] {
        let (stdout, stderr) = text(&outcome, &flags);
        assert_eq!(stderr, "");
        assert_eq!(stdout.lines().count(), 1);
        let result: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(result["exit_code"], 2);
        assert_eq!(result["success"], false);
        assert_eq!(
            result["error"],
            "cannot decode Clippy output: malformed compiler record"
        );
        assert_eq!(result["report"]["changed"], 2);
        assert_eq!(result["report"]["diagnostics"].as_array().unwrap().len(), 1);
        assert_eq!(result["report"]["phases"][1]["status"], "error");
        assert_eq!(
            result["report"]["clippy"]["stderr"],
            "Cargo build context\n"
        );
    }
}

#[test]
fn operational_failure_expands_child_context_even_with_a_compiler_finding() {
    let outcome = Outcome {
        report: Report {
            diagnostics: vec![compiler_finding(Some("error: compiler finding\n"))],
            clippy: Some(child_logs(false)),
            ..Report::default()
        },
        error: Some(anyhow::anyhow!("compiler protocol failed")),
    };
    let (_, stderr) = text(&outcome, &["-q"]);
    assert!(stderr.contains("Cargo stdout:\n  custom build output"));
    assert!(stderr.contains("Cargo stderr:\n  Cargo build context"));
    assert_eq!(stderr.matches("error: compiler protocol failed").count(), 1);
    assert!(stderr.contains("Lint aborted"));
}

#[test]
fn text_data_uses_stdout_while_diagnostics_and_outcomes_use_stderr() {
    let outcome = outcome(Report {
        diagnostics: vec![finding(
            DiagnosticSource::Rusteward,
            DiagnosticLevel::Warning,
        )],
        diffs: vec!["first diff\n".into(), "second diff\n".into()],
        ..Report::default()
    });
    for flags in [vec![], vec!["-q"], vec!["-v"]] {
        let (stdout, stderr) = text(&outcome, &flags);
        assert_eq!(stdout, "first diff\nsecond diff\n");
        assert!(stderr.contains("warning[sample-rule]"));
        assert!(!stderr.contains("first diff"));
        assert_eq!(stderr.contains("Lint passed"), !flags.contains(&"-q"));
    }
}

#[test]
fn quiet_success_has_no_output_without_findings_or_data() {
    assert_eq!(
        text(&outcome(Report::default()), &["-q"]),
        (String::new(), String::new())
    );
}

#[test]
fn summaries_distinguish_applied_formatting_from_files_that_need_formatting() {
    let applied = outcome(Report {
        changed: 2,
        ..Report::default()
    });
    let (_, stderr) = render(&applied, &["format"], &[], Presentation::default());
    assert!(stderr.contains("Format passed · 2 files formatted"));
    let checked = outcome(Report {
        changed: 2,
        phases: vec![phase(Phase::Formatting, PhaseStatus::Failed)],
        ..Report::default()
    });
    for (command, summary) in [
        (
            vec!["format", "--check"],
            "Format check failed · 2 files need formatting",
        ),
        (vec!["check"], "Check failed · 2 files need formatting"),
    ] {
        let (_, stderr) = render(&checked, &command, &[], Presentation::default());
        assert!(stderr.contains(summary));
        assert!(!stderr.contains("files formatted"));
    }
    let single = outcome(Report {
        changed: 1,
        ..Report::default()
    });
    let (_, stderr) = render(
        &single,
        &["format", "--check"],
        &[],
        Presentation::default(),
    );
    assert!(stderr.contains("1 file needs formatting"));
}

#[test]
fn aborted_formatting_does_not_claim_a_known_number_of_written_files() {
    for changed in [0, 2] {
        let outcome = Outcome {
            report: Report {
                changed,
                phases: vec![phase(Phase::Formatting, PhaseStatus::Error)],
                ..Report::default()
            },
            error: Some(anyhow::anyhow!("cannot replace selected file")),
        };
        let (_, stderr) = render(&outcome, &["format"], &[], Presentation::default());
        assert!(stderr.contains("Format aborted"));
        assert!(!stderr.contains("no files changed"));
        assert!(!stderr.contains("files formatted"));
    }
}

#[test]
fn plain_and_colored_output_preserve_diagnostic_content() {
    let outcome = outcome(Report {
        diagnostics: vec![finding(DiagnosticSource::Rusteward, DiagnosticLevel::Error)],
        ..Report::default()
    });
    let (_, plain) = text(&outcome, &[]);
    let (_, colored) = render(
        &outcome,
        &["lint"],
        &[],
        Presentation {
            color: true,
            live: false,
        },
    );
    assert!(!plain.contains('\u{1b}'));
    assert!(colored.contains("\u{1b}[31merror[sample-rule]: something needs attention\u{1b}[0m"));
    assert!(colored.contains("  --> src/lib.rs:3:7"));
    assert!(colored.contains("\u{1b}[31mLint failed\u{1b}[0m"));
}

#[test]
fn offline_phases_report_skips_errors_and_unstarted_stages() {
    let outcome = Outcome {
        report: Report {
            phases: vec![
                phase(Phase::Formatting, PhaseStatus::Error),
                phase(Phase::SourceRules, PhaseStatus::NotRun),
                phase(Phase::Clippy, PhaseStatus::Skipped),
            ],
            ..Report::default()
        },
        error: Some(anyhow::anyhow!("cannot start formatter")),
    };
    let (_, stderr) = text(&outcome, &[]);
    assert!(stderr.contains("Formatting    error · 1.23s"));
    assert!(stderr.contains("Source rules  not run"));
    assert!(stderr.contains("Clippy        skipped"));
}

#[test]
fn live_final_report_repeats_only_stages_that_never_emitted_completion() {
    let outcome = Outcome {
        report: Report {
            phases: vec![
                phase(Phase::Formatting, PhaseStatus::Passed),
                phase(Phase::SourceRules, PhaseStatus::Error),
                phase(Phase::Clippy, PhaseStatus::NotRun),
            ],
            ..Report::default()
        },
        error: Some(anyhow::anyhow!("cannot inspect source")),
    };
    let (_, stderr) = render(
        &outcome,
        &["check"],
        &[],
        Presentation {
            color: false,
            live: true,
        },
    );
    assert!(!stderr.contains("Formatting    passed"));
    assert!(!stderr.contains("Source rules  error"));
    assert!(stderr.contains("Clippy        not run"));
    assert!(stderr.contains("Check aborted"));
}

#[test]
fn errors_before_report_creation_keep_one_error_chain_and_json_exit_two() {
    let error = anyhow::anyhow!("bad setting").context("configuration failed");
    for flags in [vec![], vec!["-q"], vec!["--json"], vec!["--json", "-v"]] {
        let cli = Cli::try_parse_from(
            ["cargo-dev", "check"]
                .into_iter()
                .chain(flags.iter().copied()),
        )
        .unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        write_error(
            &cli,
            &error,
            Presentation::default(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        if flags.contains(&"--json") {
            assert_eq!(stderr, [] as [u8; 0]);
            assert_eq!(
                String::from_utf8(stdout.clone()).unwrap().lines().count(),
                1
            );
            let result: Value = serde_json::from_slice(&stdout).unwrap();
            assert_eq!(result["schema_version"], 1);
            assert_eq!(result["exit_code"], 2);
            assert_eq!(result["success"], false);
            assert_eq!(result["error"], "configuration failed: bad setting");
        } else {
            assert_eq!(stdout, [] as [u8; 0]);
            let stderr = String::from_utf8(stderr).unwrap();
            assert_eq!(stderr.matches("configuration failed").count(), 1);
            assert!(stderr.contains("bad setting"));
            assert!(stderr.contains("Check aborted"));
        }
    }
}
