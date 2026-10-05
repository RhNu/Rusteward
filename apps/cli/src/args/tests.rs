use std::ffi::OsString;

use clap::Parser;
use rusteward_core::diagnostic::Severity;
use rusteward_workspace::config::Settings;

use super::{Cli, Color, Command, Overrides, parse};

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
        let cli = parse(arguments.into_iter().map(OsString::from)).unwrap();
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
    for arguments in [
        vec!["cargo-dev", "lint", "--config", "a.toml", "--no-config"],
        vec!["cargo-dev", "--config", "a.toml", "check", "--no-config"],
        vec!["cargo-dev", "--no-config", "format", "--config", "a.toml"],
    ] {
        assert_eq!(
            parse(arguments.into_iter().map(OsString::from))
                .unwrap_err()
                .kind(),
            clap::error::ErrorKind::ArgumentConflict
        );
    }
    let cli = Cli::try_parse_from(["cargo-dev", "lint", "--rule", "unknown=off"]).unwrap();
    assert!(cli.overrides.apply(&mut Settings::default()).is_err());
}

#[test]
fn color_options_are_global_and_accept_only_the_supported_modes() {
    for arguments in [
        vec!["cargo-dev", "--color", "always", "format"],
        vec!["cargo-dev", "lint", "--color=always"],
        vec!["cargo-dev", "config", "--color", "always", "show"],
        vec!["cargo-dev", "config", "show", "--color=always"],
    ] {
        assert_eq!(Cli::try_parse_from(arguments).unwrap().color, Color::Always);
    }
    for (argument, expected) in [
        ("--color=auto", Color::Auto),
        ("--color=never", Color::Never),
    ] {
        assert_eq!(
            Cli::try_parse_from(["cargo-dev", "check", argument])
                .unwrap()
                .color,
            expected
        );
    }
    assert_eq!(
        Cli::try_parse_from(["cargo-dev", "check"]).unwrap().color,
        Color::Auto
    );
    assert!(Cli::try_parse_from(["cargo-dev", "check", "--color=sometimes"]).is_err());
}

#[test]
fn quiet_and_verbose_are_mutually_exclusive_across_command_boundaries() {
    for arguments in [
        vec!["cargo-dev", "lint", "-q", "-v"],
        vec!["cargo-dev", "--quiet", "check", "-vv"],
        vec!["cargo-dev", "-v", "config", "show", "--quiet"],
    ] {
        let result = parse(arguments.iter().copied().map(OsString::from));
        assert_eq!(
            result.unwrap_err().kind(),
            clap::error::ErrorKind::ArgumentConflict
        );
    }
    let cli = Cli::try_parse_from(["cargo-dev", "--json", "lint", "-q"]).unwrap();
    assert!(cli.json && cli.quiet);
    let cli = Cli::try_parse_from(["cargo-dev", "--json", "lint", "-vv"]).unwrap();
    assert!(cli.json);
    assert_eq!(cli.verbose, 2);
}

#[test]
fn automatic_color_uses_terminal_capability_and_explicit_modes_override_no_color() {
    for (terminal, no_color, expected) in [
        (false, false, false),
        (false, true, false),
        (true, false, true),
        (true, true, false),
    ] {
        assert_eq!(Color::Auto.enabled(terminal, no_color), expected);
        assert!(Color::Always.enabled(terminal, no_color));
        assert!(!Color::Never.enabled(terminal, no_color));
    }
}
