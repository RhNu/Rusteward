use std::path::{Path, PathBuf};

use cargo_metadata::CompilerMessage;
use rusteward_core::diagnostic::{Diagnostic as SourceDiagnostic, Severity};
use serde_json::{Value, json};

use super::{ClippyReport, Diagnostic, DiagnosticLevel, DiagnosticSource, Report};

fn custom(severity: Severity) -> SourceDiagnostic {
    SourceDiagnostic {
        path: "src/lib.rs".into(),
        line: 4,
        column: 8,
        rule: "synthetic-rule",
        severity,
        message: "source finding".into(),
    }
}

fn compiler_message(level: &str, code: Option<&str>) -> Value {
    json!({
        "package_id": "path+file:///project#sample@0.1.0",
        "target": {
            "name": "sample", "kind": ["lib"], "crate_types": ["lib"],
            "src_path": "/project/src/lib.rs", "edition": "2024",
            "doc": true, "doctest": true, "test": true
        },
        "message": {
            "message": "compiler finding",
            "code": code.map(|code| json!({"code": code, "explanation": "why this occurs"})),
            "level": level, "spans": [], "children": [], "rendered": "rendered finding\n"
        }
    })
}

fn compiler(value: Value) -> Diagnostic {
    let message: CompilerMessage = serde_json::from_value(value).unwrap();
    Diagnostic::compiler(message, Path::new("/project"))
}

fn span(file: &str, line: usize, column: usize, primary: bool) -> Value {
    json!({
        "file_name": file, "byte_start": 2, "byte_end": 3,
        "line_start": line, "line_end": line,
        "column_start": column, "column_end": column + 1,
        "is_primary": primary,
        "text": [{"text": "let x;", "highlight_start": column, "highlight_end": column + 1}],
        "label": "location", "suggested_replacement": null,
        "suggestion_applicability": null, "expansion": null
    })
}

fn clippy(success: bool) -> ClippyReport {
    ClippyReport {
        success,
        exit_code: Some(if success { 0 } else { 101 }),
        build_success: Some(success),
        output: "unrelated stdout".into(),
        stderr: "process stderr".into(),
    }
}

#[test]
fn converts_enabled_custom_findings_and_omits_disabled_rules() {
    let mut report = Report::default();
    report.extend_custom(
        [
            custom(Severity::Off),
            custom(Severity::Warning),
            custom(Severity::Error),
        ],
        "one\ntwo\nthree\n    original();\n",
    );
    assert_eq!(report.diagnostics.len(), 2);
    let warning = &report.diagnostics[0];
    assert_eq!(warning.source, DiagnosticSource::Rusteward);
    assert_eq!(warning.rule.as_deref(), Some("synthetic-rule"));
    assert_eq!(warning.severity, DiagnosticLevel::Warning);
    assert_eq!(warning.message, "source finding");
    assert_eq!(warning.path.as_deref(), Some(Path::new("src/lib.rs")));
    assert_eq!(warning.line, Some(4));
    assert_eq!(warning.column, Some(8));
    assert!(warning.compiler.is_none());
    assert_eq!(warning.snippet.as_deref(), Some("    original();"));
    assert_eq!(report.diagnostics[1].severity, DiagnosticLevel::Error);
}

#[test]
fn deny_warnings_applies_to_custom_findings_without_changing_their_severity() {
    let mut report = Report::default();
    report.extend_custom([custom(Severity::Warning)], "");
    assert!(!report.failed(false));
    assert!(report.failed(true));
    assert_eq!(report.diagnostics[0].severity, DiagnosticLevel::Warning);
}

#[test]
fn compiler_warning_overrides_are_respected_for_rustc_and_clippy() {
    let report = Report {
        diagnostics: vec![
            compiler(compiler_message("warning", Some("clippy::synthetic"))),
            compiler(compiler_message("warning", Some("unused_variables"))),
        ],
        clippy: Some(clippy(true)),
        ..Report::default()
    };
    assert!(!report.failed(false));
    assert!(!report.failed(true));
    assert_eq!(report.summary().warnings, 2);
}

#[test]
fn compiler_errors_fail_even_when_the_process_reports_success() {
    for level in ["error", "error: internal compiler error"] {
        let report = Report {
            diagnostics: vec![compiler(compiler_message(level, None))],
            clippy: Some(clippy(true)),
            ..Report::default()
        };
        assert!(report.failed(false), "{level}");
    }
    let mut report = Report::default();
    report.extend_custom([custom(Severity::Error)], "");
    assert!(report.failed(false));
}

#[test]
fn process_failure_fails_without_compiler_diagnostics_or_a_build_result() {
    let report = Report {
        clippy: Some(ClippyReport {
            build_success: None,
            ..clippy(false)
        }),
        ..Report::default()
    };
    assert!(report.failed(false));
    assert!(report.failed(true));
    assert_eq!(report.summary().errors, 0);
    assert_eq!(report.summary().warnings, 0);
}

#[test]
fn counts_top_level_warning_error_and_ice_without_counting_child_messages() {
    let mut warning = compiler_message("warning", None);
    warning["message"]["children"] = json!([{
        "message": "child error context", "code": null, "level": "error",
        "spans": [], "children": [], "rendered": null
    }]);
    let mut report = Report {
        diagnostics: vec![
            compiler(warning),
            compiler(compiler_message("error", None)),
            compiler(compiler_message("error: internal compiler error", None)),
            compiler(compiler_message("note", None)),
            compiler(compiler_message("help", None)),
            compiler(compiler_message("failure-note", None)),
        ],
        ..Report::default()
    };
    report.extend_custom([custom(Severity::Warning)], "");
    assert_eq!(report.summary().warnings, 2);
    assert_eq!(report.summary().errors, 2);
}

#[test]
fn appends_findings_and_changes_without_double_counting_scanned_sources() {
    let mut report = Report {
        files: 8,
        changed: 2,
        skipped: 3,
        diffs: vec!["first diff".into()],
        clippy: Some(clippy(true)),
        ..Report::default()
    };
    report.extend_custom([custom(Severity::Warning)], "");
    report.append(Report {
        files: 6,
        changed: 1,
        skipped: 4,
        diagnostics: vec![compiler(compiler_message("error", None))],
        diffs: vec!["second diff".into()],
        ..Report::default()
    });
    assert_eq!(report.files, 8);
    assert_eq!(report.changed, 3);
    assert_eq!(report.skipped, 4);
    assert_eq!(report.diagnostics.len(), 2);
    assert_eq!(report.diagnostics[0].source, DiagnosticSource::Rusteward);
    assert_eq!(report.diagnostics[1].source, DiagnosticSource::Rustc);
    assert_eq!(report.diffs, ["first diff", "second diff"]);
    assert!(report.clippy.as_ref().unwrap().success);
    report.append(Report {
        files: 9,
        clippy: Some(clippy(false)),
        ..Report::default()
    });
    assert_eq!(report.files, 9);
    assert!(!report.clippy.as_ref().unwrap().success);
}

#[test]
fn selects_first_primary_location_and_retains_compiler_context_and_multipart_suggestions() {
    let mut value = compiler_message("warning", Some("clippy::synthetic"));
    let secondary = span("/project/src/secondary.rs", 2, 3, false);
    let mut primary = span("/project/src/lib.rs", 7, 9, true);
    primary["expansion"] = json!({
        "span": span("/project/src/macros.rs", 5, 1, false),
        "macro_decl_name": "example!",
        "def_site_span": null
    });
    let later_primary = span("/project/src/later.rs", 10, 4, true);
    value["message"]["spans"] = json!([secondary, primary, later_primary]);
    let mut first_edit = span("/project/src/lib.rs", 7, 9, false);
    first_edit["suggested_replacement"] = json!("first replacement");
    first_edit["suggestion_applicability"] = json!("MachineApplicable");
    let mut second_edit = span("/project/src/lib.rs", 8, 2, false);
    second_edit["suggested_replacement"] = json!("second replacement");
    second_edit["suggestion_applicability"] = json!("MachineApplicable");
    value["message"]["children"] = json!([{
        "message": "replace both pieces", "code": null, "level": "help",
        "spans": [first_edit, second_edit], "children": [], "rendered": null
    }]);
    let diagnostic = compiler(value);
    assert_eq!(diagnostic.source, DiagnosticSource::Clippy);
    assert_eq!(diagnostic.rule.as_deref(), Some("clippy::synthetic"));
    assert_eq!(diagnostic.path, Some(PathBuf::from("src/lib.rs")));
    assert_eq!(diagnostic.line, Some(7));
    assert_eq!(diagnostic.column, Some(9));
    let details = diagnostic.compiler.unwrap();
    assert_eq!(details.package_id.repr, "path+file:///project#sample@0.1.0");
    assert_eq!(details.target.name, "sample");
    assert_eq!(details.target.src_path.as_str(), "/project/src/lib.rs");
    assert_eq!(details.explanation.as_deref(), Some("why this occurs"));
    assert_eq!(details.rendered.as_deref(), Some("rendered finding\n"));
    assert_eq!(details.spans.len(), 3);
    assert_eq!(details.spans[0].file_name, "/project/src/secondary.rs");
    assert_eq!(details.spans[1].text[0].text, "let x;");
    assert_eq!(
        details.spans[1].expansion.as_ref().unwrap().macro_decl_name,
        "example!"
    );
    assert_eq!(details.children.len(), 1);
    let suggestion = &details.children[0];
    assert_eq!(suggestion.message, "replace both pieces");
    assert_eq!(suggestion.level, DiagnosticLevel::Help);
    assert_eq!(suggestion.spans.len(), 2);
    assert_eq!(
        suggestion.spans[0].suggested_replacement.as_deref(),
        Some("first replacement")
    );
    assert_eq!(
        suggestion.spans[1].suggested_replacement.as_deref(),
        Some("second replacement")
    );
    assert_eq!(suggestion.spans[1].line_start, 8);
}

#[test]
fn unlocated_and_secondary_only_diagnostics_do_not_invent_coordinates() {
    for spans in [json!([]), json!([span("/project/src/lib.rs", 3, 4, false)])] {
        let mut value = compiler_message("error", Some("E0123"));
        value["message"]["spans"] = spans;
        let diagnostic = compiler(value);
        assert_eq!(diagnostic.source, DiagnosticSource::Rustc);
        assert_eq!(diagnostic.rule.as_deref(), Some("E0123"));
        assert_eq!(diagnostic.path, None);
        assert_eq!(diagnostic.line, None);
        assert_eq!(diagnostic.column, None);
    }
}

#[test]
fn preserves_locations_outside_the_workspace() {
    let mut value = compiler_message("error", None);
    value["message"]["spans"] = json!([span("/dependency/src/lib.rs", 6, 2, true)]);
    let diagnostic = compiler(value);
    assert_eq!(
        diagnostic.path,
        Some(PathBuf::from("/dependency/src/lib.rs"))
    );
    assert_eq!(diagnostic.line, Some(6));
}

#[test]
fn serializes_compiler_findings_as_structured_diagnostics() {
    let mut value = compiler_message("warning", Some("clippy::synthetic"));
    value["message"]["spans"] = json!([span("/project/src/lib.rs", 7, 9, true)]);
    let report = Report {
        diagnostics: vec![compiler(value)],
        clippy: Some(clippy(true)),
        ..Report::default()
    };
    let serialized = serde_json::to_value(report).unwrap();
    let diagnostic = &serialized["diagnostics"][0];
    assert_eq!(diagnostic["source"], "clippy");
    assert_eq!(diagnostic["rule"], "clippy::synthetic");
    assert_eq!(diagnostic["severity"], "warning");
    assert_eq!(diagnostic["message"], "compiler finding");
    assert_eq!(diagnostic["line"], 7);
    assert_eq!(diagnostic["column"], 9);
    assert_eq!(
        diagnostic["compiler"]["spans"][0]["file_name"],
        "/project/src/lib.rs"
    );
    assert_eq!(diagnostic["compiler"]["rendered"], "rendered finding\n");
    assert_eq!(serialized["clippy"]["output"], "unrelated stdout");
    assert_eq!(serialized["clippy"]["stderr"], "process stderr");
}

#[test]
fn custom_snippets_preserve_original_unicode_and_empty_lines() {
    let mut diagnostic = custom(Severity::Error);
    diagnostic.line = 2;
    let mut report = Report::default();
    report.extend_custom([diagnostic], "fn café() {}\r\n\r\n");
    assert_eq!(report.diagnostics[0].snippet.as_deref(), Some(""));
    let mut diagnostic = custom(Severity::Warning);
    diagnostic.line = 1;
    report.extend_custom([diagnostic], "fn café() {}\r\n");
    assert_eq!(
        report.diagnostics[1].snippet.as_deref(),
        Some("fn café() {}")
    );
    let mut diagnostic = custom(Severity::Error);
    diagnostic.line = 0;
    report.extend_custom([diagnostic], "original");
    assert!(report.diagnostics[2].snippet.is_none());
    report.extend_custom([custom(Severity::Error)], "one line");
    assert!(report.diagnostics[3].snippet.is_none());
}

#[test]
fn appends_stage_reports_and_treats_operational_stage_errors_as_failure() {
    use super::{Phase, PhaseReport, PhaseStatus};

    let mut report = Report {
        phases: vec![PhaseReport {
            phase: Phase::Formatting,
            status: PhaseStatus::Passed,
            elapsed_ms: Some(4),
        }],
        ..Report::default()
    };
    report.append(Report {
        phases: vec![PhaseReport {
            phase: Phase::SourceRules,
            status: PhaseStatus::Error,
            elapsed_ms: Some(2),
        }],
        ..Report::default()
    });
    assert_eq!(report.phases.len(), 2);
    assert_eq!(report.phases[0].phase, Phase::Formatting);
    assert_eq!(report.phases[1].phase, Phase::SourceRules);
    assert!(report.failed(false));
    let serialized = serde_json::to_value(report).unwrap();
    assert_eq!(serialized["phases"][1]["status"], "error");
    assert_eq!(serialized["phases"][1]["elapsed_ms"], 2);
}
