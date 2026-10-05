use std::path::Path;

use clap::Parser;
use rusteward_workspace::report::{
    ClippyReport, CompilerDetails, Diagnostic, DiagnosticLevel, DiagnosticSource, Report,
};
use serde_json::{Value, json};

use super::{write_error, write_report};
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
        compiler: None,
    }
}

fn child_logs(success: bool) -> ClippyReport {
    ClippyReport {
        success,
        exit_code: Some(if success { 0 } else { 101 }),
        build_success: Some(success),
        output: "custom build output".into(),
        stderr: "Cargo build context".into(),
    }
}

fn render(report: &Report, flags: &[&str], failed: bool) -> (String, String) {
    let cli = Cli::try_parse_from(
        ["cargo-dev", "lint"]
            .into_iter()
            .chain(flags.iter().copied()),
    )
    .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    write_report(
        &cli,
        Path::new("project"),
        report,
        failed,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    (
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn compact_output_unifies_diagnostics_and_counts_without_success_logs() {
    let report = Report {
        diagnostics: vec![
            finding(DiagnosticSource::Rusteward, DiagnosticLevel::Warning),
            finding(DiagnosticSource::Clippy, DiagnosticLevel::Error),
        ],
        clippy: Some(child_logs(true)),
        ..Report::default()
    };
    let (stdout, stderr) = render(&report, &[], true);
    assert_eq!(stdout, "");
    assert!(stderr.contains("src/lib.rs:3:7: warning[sample-rule]: something needs attention"));
    assert!(stderr.contains("src/lib.rs:3:7: error[sample-rule]: something needs attention"));
    assert!(stderr.contains("1 warnings, 1 errors"));
    assert!(!stderr.contains("custom build output"));
    assert!(!stderr.contains("Cargo build context"));
}

#[test]
fn quiet_keeps_findings_and_failure_context_but_hides_success_summary() {
    let mut report = Report {
        diagnostics: vec![finding(
            DiagnosticSource::Rusteward,
            DiagnosticLevel::Warning,
        )],
        clippy: Some(child_logs(true)),
        ..Report::default()
    };
    let (_, stderr) = render(&report, &["--quiet"], false);
    assert!(stderr.contains("something needs attention"));
    assert!(!stderr.contains("files,"));
    assert!(!stderr.contains("Cargo build context"));
    report.clippy = Some(child_logs(false));
    let (_, stderr) = render(&report, &["--quiet"], true);
    assert!(stderr.contains("custom build output\nCargo build context\n"));
    assert!(stderr.contains("Clippy failed"));
}

#[test]
fn process_failures_without_output_remain_visible_in_quiet_mode() {
    let report = Report {
        clippy: Some(ClippyReport {
            success: false,
            exit_code: None,
            build_success: None,
            output: String::new(),
            stderr: String::new(),
        }),
        ..Report::default()
    };
    let (_, stderr) = render(&report, &["-q"], true);
    assert!(stderr.contains("Clippy failed (process exit code:"));
    assert!(stderr.contains("failed: 0 files"));
}

#[test]
fn locationless_errors_do_not_invent_a_file_or_coordinates() {
    let mut diagnostic = finding(DiagnosticSource::Rustc, DiagnosticLevel::Error);
    diagnostic.path = None;
    diagnostic.line = None;
    diagnostic.column = None;
    diagnostic.rule = None;
    let report = Report {
        diagnostics: vec![diagnostic],
        ..Report::default()
    };
    let (_, stderr) = render(&report, &[], true);
    assert!(stderr.starts_with("rustc: error: something needs attention\n"));
    assert!(stderr.contains("0 warnings, 1 errors"));
}

fn compiler_details() -> CompilerDetails {
    let span = json!({
        "file_name":"src/lib.rs", "byte_start":4, "byte_end":5,
        "line_start":3, "line_end":3, "column_start":7, "column_end":8,
        "is_primary":false, "text":[], "label":null,
        "suggested_replacement":"é", "suggestion_applicability":"MaybeIncorrect",
        "expansion":null
    });
    CompilerDetails {
        package_id: serde_json::from_value(json!("path+file:///project#sample@0.1.0")).unwrap(),
        target: serde_json::from_value(json!({
            "name":"sample", "kind":["lib"], "crate_types":["lib"],
            "src_path":"/project/src/lib.rs", "edition":"2024",
            "doc":true, "doctest":true, "test":true
        }))
        .unwrap(),
        spans: vec![],
        children: vec![
            serde_json::from_value(json!({
                "message":"try this replacement", "code":null, "level":"help",
                "spans":[span], "children":[], "rendered":null
            }))
            .unwrap(),
        ],
        explanation: None,
        rendered: Some("error: full compiler context\n  --> src/lib.rs:3:7".into()),
    }
}

#[test]
fn compact_shows_suggestions_and_verbose_uses_complete_rendered_context_once() {
    let mut diagnostic = finding(DiagnosticSource::Clippy, DiagnosticLevel::Error);
    diagnostic.compiler = Some(compiler_details());
    let report = Report {
        diagnostics: vec![diagnostic],
        clippy: Some(child_logs(true)),
        ..Report::default()
    };
    let (_, compact) = render(&report, &[], true);
    assert!(compact.contains("help: try this replacement"));
    assert!(compact.contains("src/lib.rs:3:7-3:8: replace with \"é\" (MaybeIncorrect)"));
    assert!(!compact.contains("full compiler context"));
    for flag in ["-v", "-vv"] {
        let (_, detailed) = render(&report, &[flag], true);
        assert_eq!(detailed.matches("full compiler context").count(), 1);
        assert!(!detailed.contains("something needs attention"));
        assert!(detailed.contains("src/lib.rs:3:7\ncustom build output\n"));
        assert!(detailed.contains("Cargo build context\n"));
    }
}

#[test]
fn verbose_falls_back_to_structured_children_when_rendered_text_is_missing() {
    let mut diagnostic = finding(DiagnosticSource::Rustc, DiagnosticLevel::Error);
    let mut details = compiler_details();
    details.rendered = None;
    details.children[0].level = DiagnosticLevel::Note;
    details.children[0].spans.clear();
    diagnostic.compiler = Some(details);
    let report = Report {
        diagnostics: vec![diagnostic],
        ..Report::default()
    };
    let (_, compact) = render(&report, &[], true);
    let (_, detailed) = render(&report, &["-v"], true);
    assert!(!compact.contains("try this replacement"));
    assert!(detailed.contains("note: try this replacement"));
}

#[test]
fn json_is_one_versioned_result_with_structured_findings_and_separate_logs() {
    let mut diagnostic = finding(DiagnosticSource::Clippy, DiagnosticLevel::Error);
    diagnostic.compiler = Some(compiler_details());
    let report = Report {
        diagnostics: vec![diagnostic],
        diffs: vec!["a diff\n".into()],
        clippy: Some(child_logs(false)),
        ..Report::default()
    };
    for flags in [vec!["--json"], vec!["--json", "-q"], vec!["--json", "-vv"]] {
        let (stdout, stderr) = render(&report, &flags, true);
        assert_eq!(stderr, "");
        assert_eq!(stdout.lines().count(), 1);
        let result: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(result["schema_version"], 1);
        assert_eq!(result["success"], false);
        assert_eq!(result["exit_code"], 1);
        assert_eq!(result["summary"]["errors"], 1);
        let diagnostic = &result["report"]["diagnostics"][0];
        assert_eq!(diagnostic["source"], "clippy");
        assert_eq!(diagnostic["path"], "src/lib.rs");
        assert_eq!(
            diagnostic["compiler"]["children"][0]["spans"][0]["suggested_replacement"],
            "é"
        );
        assert_eq!(result["report"]["clippy"]["output"], "custom build output");
        assert_eq!(result["report"]["diffs"][0], "a diff\n");
    }
}

#[test]
fn text_diffs_use_stdout_while_findings_and_summaries_use_stderr() {
    let report = Report {
        diffs: vec!["first diff\n".into(), "second diff\n".into()],
        ..Report::default()
    };
    let (stdout, stderr) = render(&report, &[], false);
    assert_eq!(stdout, "first diff\nsecond diff\n");
    assert!(stderr.contains("passed:"));
}

#[test]
fn operational_errors_use_exit_two_and_preserve_the_error_chain_once() {
    let error = anyhow::anyhow!("bad setting").context("configuration failed");
    for json in [false, true] {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        write_error(json, &error, &mut stdout, &mut stderr).unwrap();
        if json {
            assert_eq!(stderr, [] as [u8; 0]);
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
        }
    }
}
