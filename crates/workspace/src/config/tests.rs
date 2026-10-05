use std::collections::BTreeMap;

use super::{
    ClippyLevel, Settings, clippy_config_toml, overlay, parse_rustfmt_override, rustfmt_value,
    validate,
};

#[test]
fn execution_defaults_to_automatic_jobs() {
    assert_eq!(Settings::default().execution.jobs, 0);
    for source in ["", "[execution]"] {
        let settings: Settings = toml::from_str(source).unwrap();
        assert_eq!(settings.execution.jobs, 0);
    }
}

#[test]
fn execution_layers_replace_jobs_and_preserve_unrelated_settings() {
    let mut base = toml::Value::try_from(Settings::default()).unwrap();
    overlay(
        &mut base,
        "[execution]\njobs = 8\n[format]\nspacing = false",
    )
    .unwrap();
    for jobs in [2, 1, 0] {
        overlay(&mut base, &format!("[execution]\njobs = {jobs}")).unwrap();
        let settings: Settings = base.clone().try_into().unwrap();
        assert_eq!(settings.execution.jobs, jobs);
        assert!(!settings.format.spacing);
        validate(&settings).unwrap();
    }
    overlay(&mut base, "[lint]\nclippy = false").unwrap();
    let settings: Settings = base.try_into().unwrap();
    assert_eq!(settings.execution.jobs, 0);
    assert!(!settings.lint.clippy);
}

#[test]
fn execution_rejects_negative_noninteger_and_unknown_jobs_settings() {
    for value in ["-1", "1.5", "'2'", "true"] {
        let source = format!("[execution]\njobs = {value}");
        assert!(
            toml::from_str::<Settings>(&source).is_err(),
            "accepted {value}"
        );
    }
    assert!(toml::from_str::<Settings>("[execution]\nworkers = 2").is_err());
}

#[test]
fn layers_merge_options_and_replace_arrays() {
    let mut base = toml::Value::try_from(Settings::default()).unwrap();
    overlay(
        &mut base,
        "[scan]\nexclude = ['old/**']\n[format.rustfmt]\nmax_width = 90\n",
    )
    .unwrap();
    overlay(
        &mut base,
        "[scan]\nexclude = ['new/**']\n[format.rustfmt]\nmax_width = 110\n",
    )
    .unwrap();
    let settings: Settings = base.try_into().unwrap();
    assert_eq!(settings.scan.exclude, ["new/**"]);
    assert_eq!(settings.format.rustfmt["max_width"].as_integer(), Some(110));
    assert!(settings.format.rustfmt.contains_key("imports_granularity"));
    validate(&settings).unwrap();
}

#[test]
fn rejects_unknown_settings_and_invalid_thresholds() {
    assert!(toml::from_str::<Settings>("[lint]\nclpipy = true").is_err());
    assert!(toml::from_str::<Settings>("[rules]\ninline-tests = 'warn'").is_err());
    let mut settings = Settings::default();
    settings.rules.lines.warn = 20;
    settings.rules.lines.error = 10;
    assert!(validate(&settings).is_err());
}

#[test]
fn rustfmt_overrides_preserve_native_values() {
    for (argument, expected) in [
        ("max_width=90", "90"),
        ("fn_single_line=false", "false"),
        ("hex_literal_case=Upper", "Upper"),
    ] {
        let (_, value) = parse_rustfmt_override(argument).unwrap();
        assert_eq!(rustfmt_value(&value).unwrap(), expected);
    }
    assert!(parse_rustfmt_override("max_width").is_err());
    assert!(rustfmt_value(&toml::Value::String("a,b".into())).is_err());
    assert!(rustfmt_value(&toml::Value::Table(toml::Table::new())).is_err());
}

#[test]
fn clippy_layers_override_individual_options_and_keep_other_defaults() {
    let defaults = Settings::default();
    let mut base = toml::Value::try_from(&defaults).unwrap();
    overlay(
        &mut base,
        r#"
[lint.clippy-lints]
unwrap_used = "deny"
additional_lint = "forbid"
[lint.clippy-config]
allow-unwrap-in-tests = false
additional-parameter = ["first", "second"]
"#,
    )
    .unwrap();
    let settings: Settings = base.try_into().unwrap();
    assert_eq!(settings.lint.clippy_lints["unwrap_used"], ClippyLevel::Deny);
    assert_eq!(
        settings.lint.clippy_lints["additional_lint"],
        ClippyLevel::Forbid
    );
    for (name, level) in &defaults.lint.clippy_lints {
        if name != "unwrap_used" {
            assert_eq!(settings.lint.clippy_lints.get(name), Some(level));
        }
    }
    assert_eq!(
        settings.lint.clippy_config["allow-unwrap-in-tests"].as_bool(),
        Some(false)
    );
    for (name, value) in &defaults.lint.clippy_config {
        if name != "allow-unwrap-in-tests" {
            assert_eq!(settings.lint.clippy_config.get(name), Some(value));
        }
    }
    assert_eq!(settings.format.rustfmt, defaults.format.rustfmt);
    validate(&settings).unwrap();
}

#[test]
fn clippy_levels_encode_rustc_flags_and_reject_unknown_levels() {
    for (name, flag) in [
        ("allow", "-A"),
        ("warn", "-W"),
        ("deny", "-D"),
        ("forbid", "-F"),
    ] {
        let source = format!("[lint.clippy-lints]\nexample = '{name}'");
        let settings: Settings = toml::from_str(&source).unwrap();
        assert_eq!(settings.lint.clippy_lints["example"].flag(), flag);
    }
    assert!(toml::from_str::<Settings>("[lint.clippy-lints]\nexample = 'error'").is_err());
}

#[test]
fn clippy_names_require_their_native_spelling() {
    for name in [
        "",
        "clippy::unwrap_used",
        "unwrap-used",
        "-Dwarnings",
        "lint name",
        "Uppercase",
    ] {
        let mut settings = Settings::default();
        settings
            .lint
            .clippy_lints
            .insert(name.into(), ClippyLevel::Warn);
        assert!(validate(&settings).is_err(), "accepted lint name {name:?}");
    }
    for name in [
        "",
        "allow_unwrap_in_tests",
        "--config",
        "parameter name",
        "Uppercase",
    ] {
        let mut settings = Settings::default();
        settings
            .lint
            .clippy_config
            .insert(name.into(), toml::Value::Boolean(true));
        assert!(
            validate(&settings).is_err(),
            "accepted parameter name {name:?}"
        );
    }
}

#[test]
fn clippy_parameters_serialize_with_native_toml_types() {
    let input: BTreeMap<String, toml::Value> = toml::from_str(
        r#"
allow-unwrap-in-tests = false
too-many-arguments-threshold = 9
literal-representation-threshold = 0.75
documentation-valid-idents = ["Rusteward", ".."]
disallowed-methods = [{ path = "std::env::set_var", reason = "Prefer explicit command environment" }]
custom-message = "first\nsecond, quoted \"value\""
"#,
    )
    .unwrap();
    let serialized = clippy_config_toml(&input).unwrap();
    let output: BTreeMap<String, toml::Value> = toml::from_str(&serialized).unwrap();
    assert_eq!(output, input);
}
