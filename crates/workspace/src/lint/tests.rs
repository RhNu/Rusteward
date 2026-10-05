use std::{collections::BTreeMap, ffi::OsString, path::Path};

use rusteward_core::{Edition, rules::Rules};

use super::{command, decode_output, inspect_source};
use crate::{
    config::{ClippyLevel, LintSettings},
    discovery::{Source, Workspace},
    process::CargoOptions,
    report::Report,
};

#[test]
fn generated_source_has_no_findings_even_with_invalid_syntax() {
    let file = Source {
        path: "project/src/mod.rs".into(),
        edition: Edition::Edition2024,
    };
    let result = inspect_source(
        &file,
        Path::new("project"),
        "// @generated\nfn {",
        &Rules::default(),
    );
    assert!(result.skipped);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn malformed_clippy_protocol_retains_raw_process_output_in_the_report() {
    let stdout = b"{\"reason\":\"compiler-message\",\"message\":broken}\n";
    let stderr = b"Cargo execution context\n";
    let mut report = Report::default();
    let error = decode_output(
        stdout,
        stderr,
        Path::new("project"),
        false,
        Some(101),
        &mut report,
    )
    .unwrap_err();
    assert!(error.to_string().contains("malformed Cargo Clippy message"));
    let clippy = report.clippy.unwrap();
    assert!(!clippy.success);
    assert_eq!(clippy.exit_code, Some(101));
    assert_eq!(clippy.output, String::from_utf8_lossy(stdout));
    assert_eq!(clippy.stderr, String::from_utf8_lossy(stderr));
}

#[test]
fn source_findings_use_relative_paths_and_original_coordinates() {
    let file = Source {
        path: "project/src/lib.rs".into(),
        edition: Edition::Edition2024,
    };
    let result = inspect_source(
        &file,
        Path::new("project"),
        "const HEADER: () = ();\nmod tests {}\n",
        &Rules::default(),
    );
    assert!(!result.skipped);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(
        result.diagnostics[0].path.as_deref(),
        Some(Path::new("src/lib.rs"))
    );
    assert_eq!(
        (result.diagnostics[0].line, result.diagnostics[0].column),
        (Some(2), Some(1))
    );
    assert_eq!(
        result.diagnostics[0].snippet.as_deref(),
        Some("mod tests {}")
    );
}

#[test]
fn separates_cargo_selection_from_rustc_flags() {
    let workspace = Workspace {
        root: "project".into(),
        manifest: "project/Cargo.toml".into(),
        target: "project/target".into(),
        packages: vec![],
    };
    let settings = LintSettings {
        all_features: true,
        features: vec!["extras".into()],
        clippy_args: vec!["-W".into(), "clippy::pedantic".into()],
        clippy_lints: BTreeMap::new(),
        ..LintSettings::default()
    };
    let invocation = command(
        &workspace,
        &settings,
        &CargoOptions {
            locked: true,
            ..CargoOptions::default()
        },
        Path::new("isolated-profile"),
    );
    let arguments: Vec<_> = invocation.get_args().map(OsString::from).collect();
    let separator = arguments.iter().position(|arg| arg == "--").unwrap();
    assert!(arguments[..separator].contains(&OsString::from("--message-format=json")));
    assert!(arguments[..separator].contains(&OsString::from("--color=never")));
    assert!(arguments[..separator].contains(&OsString::from("--all-features")));
    assert!(arguments[..separator].contains(&OsString::from("--locked")));
    assert_eq!(
        &arguments[separator + 1..],
        &["-D", "warnings", "-W", "clippy::pedantic"].map(OsString::from)
    );
    assert_eq!(
        invocation
            .get_envs()
            .find(|(key, _)| *key == "CLIPPY_CONF_DIR")
            .unwrap()
            .1,
        Some(Path::new("isolated-profile").as_os_str())
    );
}

#[test]
fn individual_policy_overrides_groups_and_raw_flags_are_last() {
    let settings = LintSettings {
        clippy_lints: BTreeMap::from([
            ("all".into(), ClippyLevel::Warn),
            ("pedantic".into(), ClippyLevel::Deny),
            ("must_use_candidate".into(), ClippyLevel::Allow),
            ("dbg_macro".into(), ClippyLevel::Forbid),
        ]),
        clippy_args: vec!["-A".into(), "clippy::similar_names".into()],
        ..LintSettings::default()
    };
    let workspace = Workspace {
        root: "project".into(),
        manifest: "project/Cargo.toml".into(),
        target: "project/target".into(),
        packages: vec![],
    };
    let invocation = command(
        &workspace,
        &settings,
        &CargoOptions::default(),
        Path::new("profile"),
    );
    let arguments: Vec<_> = invocation.get_args().collect();
    let separator = arguments.iter().position(|arg| *arg == "--").unwrap();
    let lint_arguments: Vec<_> = arguments[separator + 1..]
        .iter()
        .map(|arg| arg.to_str().unwrap())
        .collect();
    assert_eq!(
        lint_arguments,
        [
            "-D",
            "warnings",
            "-W",
            "clippy::all",
            "-D",
            "clippy::pedantic",
            "-F",
            "clippy::dbg_macro",
            "-A",
            "clippy::must_use_candidate",
            "-A",
            "clippy::similar_names",
        ]
    );
}
