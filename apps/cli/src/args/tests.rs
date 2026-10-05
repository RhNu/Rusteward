use std::ffi::OsString;

use clap::Parser;
use rusteward_core::diagnostic::Severity;
use rusteward_workspace::config::Settings;

use super::{Cli, Command, Overrides, normalize};

#[test]
fn jobs_are_global_before_after_and_between_nested_commands() {
    for arguments in [
        vec!["cargo-dev", "--jobs", "3", "format"],
        vec!["cargo-dev", "lint", "-j", "3"],
        vec!["cargo-dev", "config", "--jobs=3", "show"],
        vec!["cargo-dev", "config", "show", "-j3"],
    ] {
        let cli = Cli::try_parse_from(arguments).unwrap();
        assert_eq!(cli.overrides.jobs, Some(3));
        let mut settings = Settings::default();
        cli.overrides.apply(&mut settings).unwrap();
        assert_eq!(settings.execution.jobs, 3);
    }
}

#[test]
fn explicit_jobs_override_configured_value_and_absence_preserves_it() {
    let mut settings: Settings = toml::from_str("[execution]\njobs = 5").unwrap();
    Overrides::default().apply(&mut settings).unwrap();
    assert_eq!(settings.execution.jobs, 5);
    for (jobs, expected) in [("1", 1), ("0", 0), ("4", 4)] {
        let cli = Cli::try_parse_from(["cargo-dev", "check", "--jobs", jobs]).unwrap();
        cli.overrides.apply(&mut settings).unwrap();
        assert_eq!(settings.execution.jobs, expected);
    }
}

#[test]
fn jobs_reject_negative_and_noninteger_values() {
    for value in ["-1", "1.5", "many"] {
        let argument = format!("--jobs={value}");
        assert!(Cli::try_parse_from(["cargo-dev", "lint", &argument]).is_err());
    }
}

#[test]
fn direct_and_cargo_invocations_share_the_command_surface() {
    for arguments in [
        vec!["cargo-dev", "format", "--check"],
        vec!["cargo-dev", "dev", "format", "--check"],
    ] {
        let cli =
            Cli::try_parse_from(normalize(arguments.into_iter().map(OsString::from))).unwrap();
        assert!(matches!(cli.command, Command::Format { check: true, .. }));
    }
}

#[test]
fn global_flags_after_commands_apply_to_effective_settings() {
    let cli = Cli::try_parse_from([
        "cargo-dev",
        "lint",
        "--rule",
        "inline-tests=error",
        "--rustfmt-option",
        "max_width=90",
        "--skip-clippy",
        "--all-targets=false",
    ])
    .unwrap();
    let mut settings = Settings::default();
    cli.overrides.apply(&mut settings).unwrap();
    assert_eq!(settings.rules.inline_tests, Severity::Error);
    assert_eq!(settings.format.rustfmt["max_width"].as_integer(), Some(90));
    assert!(!settings.lint.clippy);
    assert!(!settings.lint.all_targets);
}

#[test]
fn rejects_conflicting_config_modes_and_unknown_rules() {
    assert!(
        Cli::try_parse_from(["cargo-dev", "lint", "--config", "a.toml", "--no-config"]).is_err()
    );
    let cli = Cli::try_parse_from(["cargo-dev", "lint", "--rule", "unknown=off"]).unwrap();
    assert!(cli.overrides.apply(&mut Settings::default()).is_err());
}
