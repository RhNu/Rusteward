use super::{Diagnostic, Severity, has_failures};

#[test]
fn warning_policy_changes_failure_without_relabeling() {
    let diagnostics = [Diagnostic {
        path: "src/a.rs".into(),
        line: 1,
        column: 1,
        rule: "inline-tests",
        severity: Severity::Warning,
        message: "move tests".into(),
    }];
    assert!(!has_failures(&diagnostics, false));
    assert!(has_failures(&diagnostics, true));
    assert_eq!(diagnostics[0].severity, Severity::Warning);
    assert!(!has_failures(&[], true));
}
