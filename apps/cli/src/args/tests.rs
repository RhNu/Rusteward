use std::ffi::OsString;

use clap::Parser;
use rusteward_core::diagnostic::Severity;
use rusteward_workspace::config::Settings;

use super::{Cli, Command, normalize};

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
