use std::path::Path;

use cargo_metadata::diagnostic::DiagnosticLevel;
use serde_json::{Value, json};

use super::parse;

/// Minimal compiler record with configurable diagnostic data and no filesystem access.
fn compiler_record(message: &Value) -> Value {
    json!({
        "reason": "compiler-message",
        "package_id": "path+file:///project#sample@0.1.0",
        "target": {
            "name": "sample",
            "kind": ["lib"],
            "crate_types": ["lib"],
            "src_path": "/project/src/lib.rs",
            "edition": "2024",
            "doc": true,
            "doctest": true,
            "test": true
        },
        "message": message
    })
}

fn diagnostic(message: &str, level: &str) -> Value {
    json!({
        "message": message,
        "code": null,
        "level": level,
        "spans": [],
        "children": [],
        "rendered": null
    })
}

#[test]
fn collects_compiler_diagnostics_in_emission_order() {
    let mut warning = diagnostic("a warning", "warning");
    warning["children"] = json!([diagnostic("additional help", "help")]);
    let stdout = format!(
        "{}\n{}\n{{\"reason\":\"build-finished\",\"success\":false}}\n",
        compiler_record(&warning),
        compiler_record(&diagnostic("an error", "error"))
    );
    let parsed = parse(stdout.as_bytes(), Path::new("/project")).unwrap();
    assert_eq!(parsed.diagnostics.len(), 2);
    assert_eq!(parsed.diagnostics[0].message, "a warning");
    assert_eq!(parsed.diagnostics[0].severity, DiagnosticLevel::Warning);
    assert_eq!(parsed.diagnostics[0].path, None);
    let details = parsed.diagnostics[0].compiler.as_ref().unwrap();
    assert_eq!(details.children.len(), 1);
    assert_eq!(details.children[0].message, "additional help");
    assert_eq!(details.children[0].level, DiagnosticLevel::Help);
    assert_eq!(parsed.diagnostics[1].message, "an error");
    assert_eq!(parsed.diagnostics[1].severity, DiagnosticLevel::Error);
    assert_eq!(parsed.build_success, Some(false));
    assert!(parsed.output.is_empty());
}

#[test]
fn retains_arbitrary_text_and_json_with_original_line_endings() {
    let unrelated = concat!(
        "building café 🦀\r\n",
        "{ordinary text\n",
        "{\"reason\":\"custom-tool\",\"success\":true}\n",
        "{\"nested\":{\"reason\":\"build-finished\"}}\n",
        "{\"text\":\"\\\"reason\\\":\\\"compiler-message\\\"\"}\n",
        "[\"compiler-message\", 42]\n",
        "42\n",
        "\r\n"
    );
    let stdout = format!("{unrelated}{{\"reason\":\"build-finished\",\"success\":true}}\r\nfin 🦀");
    let parsed = parse(stdout.as_bytes(), Path::new("/project")).unwrap();
    assert_eq!(parsed.output, format!("{unrelated}fin 🦀"));
    assert_eq!(parsed.build_success, Some(true));
    assert!(parsed.diagnostics.is_empty());
}

#[test]
fn validates_and_discards_non_diagnostic_cargo_records() {
    let target = compiler_record(&diagnostic("unused", "note"))["target"].clone();
    let artifact = json!({
        "reason": "compiler-artifact",
        "package_id": "path+file:///project#sample@0.1.0",
        "target": target,
        "profile": {
            "opt_level": "0", "debuginfo": 2, "debug_assertions": true,
            "overflow_checks": true, "test": false
        },
        "features": [], "filenames": [], "executable": null, "fresh": false
    });
    let build_script = json!({
        "reason": "build-script-executed",
        "package_id": "path+file:///project#sample@0.1.0",
        "linked_libs": [], "linked_paths": [], "cfgs": [], "env": [],
        "out_dir": "/project/target/out"
    });
    let stdout = format!("{artifact}\n{build_script}\n");
    let parsed = parse(stdout.as_bytes(), Path::new("/project")).unwrap();
    assert!(parsed.output.is_empty());
    assert!(parsed.diagnostics.is_empty());
    assert_eq!(parsed.build_success, None);
}

#[test]
fn rejects_malformed_known_protocol_records() {
    for line in [
        r#"{"reason":"compiler-message"}"#,
        r#"{"reason":"compiler-artifact"}"#,
        r#"{"reason":"build-script-executed"}"#,
        r#"{"reason":"build-finished","success":"yes"}"#,
        r#"{"reason":"compiler-message","message":"#,
        r#"{"other":null,"reason":"build-finished","success":true"#,
        r#"{"other":,"reason":"build-finished","success":true}"#,
        r#"{"reason":"build-finished","success":true} trailing"#,
        r#"{"reason":"build-finished","reason":"other","success":true}"#,
        r#"{"re\u0061son":"build-finished","success":null}"#,
    ] {
        let stdout = format!("ordinary line\n{line}");
        let error = parse(stdout.as_bytes(), Path::new("/project")).unwrap_err();
        assert!(
            error.to_string().contains("stdout line 2"),
            "{line}: {error}"
        );
    }
}

#[test]
fn rejects_duplicate_build_results() {
    let stdout = concat!(
        "{\"reason\":\"build-finished\",\"success\":false}\n",
        "{\"reason\":\"build-finished\",\"success\":true}\n"
    );
    let error = parse(stdout.as_bytes(), Path::new("/project")).unwrap_err();
    assert!(error.to_string().contains("duplicate Cargo build-finished"));
}

#[test]
fn permits_empty_output_and_failures_before_any_build_result() {
    let parsed = parse(b"", Path::new("/project")).unwrap();
    assert_eq!(parsed.build_success, None);
    assert!(parsed.output.is_empty());
    let parsed = parse(b"dependency resolution failed", Path::new("/project")).unwrap();
    assert_eq!(parsed.build_success, None);
    assert_eq!(parsed.output, "dependency resolution failed");
}

#[test]
fn accepts_unicode_compiler_messages_and_protocol_without_final_newline() {
    let record = compiler_record(&diagnostic("déjà vu 🦀", "error"));
    let stdout = format!("{record}\r\n{{\"reason\":\"build-finished\",\"success\":false}}");
    let parsed = parse(stdout.as_bytes(), Path::new("/project")).unwrap();
    assert_eq!(parsed.diagnostics[0].message, "déjà vu 🦀");
    assert_eq!(parsed.build_success, Some(false));
    assert!(parsed.output.is_empty());
}

#[test]
fn rejects_invalid_utf8_including_nonprotocol_output() {
    let error = parse(b"output: \xff", Path::new("/project")).unwrap_err();
    assert!(error.to_string().contains("UTF-8"));
}

#[test]
fn successful_process_requires_an_explicit_cargo_build_result() {
    let parsed = parse(b"unexpected tool output", Path::new("/project")).unwrap();
    assert!(parsed.success(true).is_err());
    assert!(!parsed.success(false).unwrap());
}

#[test]
fn both_process_and_build_must_succeed() {
    for (build_success, process_success, expected) in [
        (true, true, true),
        (true, false, false),
        (false, true, false),
        (false, false, false),
    ] {
        let stdout = format!(r#"{{"reason":"build-finished","success":{build_success}}}"#);
        let parsed = parse(stdout.as_bytes(), Path::new("/project")).unwrap();
        assert_eq!(parsed.success(process_success).unwrap(), expected);
    }
}
