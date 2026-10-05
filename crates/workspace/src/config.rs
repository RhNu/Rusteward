//! Layer user and workspace settings over the built-in personal defaults.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use rusteward_core::rules::Rules;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

pub const FILE_NAME: &str = "rusteward.toml";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Settings {
    pub version: u32,
    pub execution: ExecutionSettings,
    pub scan: ScanSettings,
    pub format: FormatSettings,
    pub lint: LintSettings,
    pub rules: Rules,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            execution: ExecutionSettings::default(),
            scan: ScanSettings::default(),
            format: FormatSettings::default(),
            lint: LintSettings::default(),
            rules: Rules::default(),
        }
    }
}

/// Bound Rusteward's file processing independently of Cargo's worker selection.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ExecutionSettings {
    /// Zero selects available CPUs and file count; one processes files serially.
    pub jobs: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct ScanSettings {
    pub exclude: Vec<String>,
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self {
            exclude: vec!["**/vendor/**".into(), "**/dist/**".into()],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct FormatSettings {
    pub spacing: bool,
    /// An optional rustup toolchain; absent means the target project's active toolchain.
    pub toolchain: Option<String>,
    /// Values retain native rustfmt TOML types until command-line serialization.
    pub rustfmt: BTreeMap<String, toml::Value>,
}

impl Default for FormatSettings {
    fn default() -> Self {
        Self {
            spacing: true,
            toolchain: None,
            rustfmt: rustfmt_defaults(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Cargo feature selection and Clippy execution are independent user options"
)]
pub struct LintSettings {
    pub clippy: bool,
    pub deny_warnings: bool,
    pub all_targets: bool,
    pub all_features: bool,
    pub no_default_features: bool,
    pub features: Vec<String>,
    pub toolchain: Option<String>,
    /// Native Clippy lint names map to levels before extra rustc flags are applied.
    pub clippy_lints: BTreeMap<String, ClippyLevel>,
    /// Clippy configuration retains native TOML values for its configuration file.
    pub clippy_config: BTreeMap<String, toml::Value>,
    /// Extra Clippy/rustc lint flags follow the managed default policy.
    pub clippy_args: Vec<String>,
}

impl Default for LintSettings {
    fn default() -> Self {
        let defaults = clippy_defaults();
        Self {
            clippy: true,
            deny_warnings: false,
            all_targets: true,
            all_features: false,
            no_default_features: false,
            features: Vec::new(),
            toolchain: None,
            clippy_lints: defaults.lints,
            clippy_config: defaults.config,
            clippy_args: Vec::new(),
        }
    }
}

/// Rustc lint levels accepted by the managed Clippy profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClippyLevel {
    Allow,
    Warn,
    Deny,
    Forbid,
}

impl ClippyLevel {
    /// Return the rustc flag corresponding to this lint level.
    pub const fn flag(self) -> &'static str {
        match self {
            Self::Allow => "-A",
            Self::Warn => "-W",
            Self::Deny => "-D",
            Self::Forbid => "-F",
        }
    }
}

/// Parse a TOML layer, deeply replacing scalars and arrays while merging tables.
///
/// # Errors
/// Returns an error when the layer is not valid TOML.
pub fn overlay(base: &mut toml::Value, source: &str) -> Result<()> {
    let layer: toml::Value = toml::from_str(source).context("invalid configuration TOML")?;
    merge(base, layer);
    Ok(())
}

fn merge(base: &mut toml::Value, layer: toml::Value) {
    match (base, layer) {
        (toml::Value::Table(base), toml::Value::Table(layer)) => {
            for (key, value) in layer {
                if let Some(existing) = base.get_mut(&key) {
                    merge(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        },
        (base, layer) => *base = layer,
    }
}

/// Reject ambiguous policies, unsafe option names, and unsupported rustfmt CLI values.
///
/// # Errors
/// Returns an error for unsupported configuration versions or invalid setting values.
pub fn validate(settings: &Settings) -> Result<()> {
    ensure!(
        settings.version == 1,
        "unsupported configuration version {}; expected 1",
        settings.version
    );
    ensure!(
        settings.rules.lines.warn <= settings.rules.lines.error,
        "rules.lines.warn must not exceed rules.lines.error"
    );
    for (key, value) in &settings.format.rustfmt {
        ensure!(
            !key.is_empty()
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
            "invalid rustfmt option name {key:?}"
        );
        ensure!(
            !matches!(key.as_str(), "edition" | "skip_children" | "emit_mode"),
            "rustfmt option {key} is managed by rusteward to preserve per-file processing"
        );
        rustfmt_value(value).with_context(|| format!("invalid rustfmt option {key}"))?;
    }
    for toolchain in [&settings.format.toolchain, &settings.lint.toolchain]
        .into_iter()
        .flatten()
    {
        ensure!(
            !toolchain.is_empty()
                && !toolchain.starts_with('-')
                && !toolchain.chars().any(char::is_whitespace),
            "invalid toolchain name {toolchain:?}"
        );
    }
    for key in settings.lint.clippy_lints.keys() {
        ensure!(
            native_option_name(key, b'_'),
            "invalid Clippy lint name {key:?}; use its unqualified underscore name"
        );
    }
    for key in settings.lint.clippy_config.keys() {
        ensure!(
            native_option_name(key, b'-'),
            "invalid Clippy configuration name {key:?}; use its hyphenated name"
        );
    }
    Ok(())
}

fn native_option_name(name: &str, separator: u8) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == separator)
}

/// Serialize managed Clippy parameters without converting their native TOML types.
///
/// # Errors
/// Returns an error when TOML cannot represent the provided parameters.
pub fn clippy_config_toml(config: &BTreeMap<String, toml::Value>) -> Result<String> {
    toml::to_string(config).context("cannot serialize Clippy configuration")
}

/// Encode scalars and attempt bracketed arrays; rustfmt validates option-specific support.
///
/// # Errors
/// Returns an error for unsupported types or strings that cannot be represented on the CLI.
pub fn rustfmt_value(value: &toml::Value) -> Result<String> {
    match value {
        toml::Value::String(value) => {
            ensure!(
                !value.contains([',', '\n', '\r']),
                "rustfmt CLI string values cannot contain commas or newlines"
            );
            Ok(value.clone())
        },
        toml::Value::Boolean(_) | toml::Value::Integer(_) | toml::Value::Float(_) => {
            Ok(value.to_string())
        },
        toml::Value::Array(values) => {
            let values = values
                .iter()
                .map(rustfmt_value)
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("[{}]", values.join(",")))
        },
        _ => bail!("expected a string, number, boolean, or array"),
    }
}

/// CLI overrides accept native TOML literals or bare rustfmt enum names.
///
/// # Errors
/// Returns an error when the argument is missing `=` or a nonempty value.
pub fn parse_rustfmt_override(argument: &str) -> Result<(String, toml::Value)> {
    let (key, raw) = argument
        .split_once('=')
        .context("expected a rustfmt KEY=VALUE override")?;
    ensure!(!raw.is_empty(), "rustfmt override value cannot be empty");
    let parsed: Result<toml::Table, _> = toml::from_str(&format!("value = {raw}"));
    let value = parsed
        .ok()
        .and_then(|mut table| table.remove("value"))
        .unwrap_or_else(|| toml::Value::String(raw.into()));
    Ok((key.into(), value))
}

/// Use the platform's standard configuration directory without creating it.
pub fn user_path() -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join("rusteward").join("config.toml"))
}

pub struct Loaded {
    pub settings: Settings,
    pub sources: Vec<PathBuf>,
}

/// An explicit path replaces automatic project configuration; --no-config disables both layers.
///
/// # Errors
/// Returns an error if a configuration layer cannot be read or parsed as valid settings.
pub fn load(root: &Path, explicit: Option<&Path>, no_config: bool) -> Result<Loaded> {
    let mut value = toml::Value::try_from(Settings::default())?;
    let mut sources = Vec::new();
    if !no_config {
        if let Some(path) = user_path().filter(|path| path.exists()) {
            read_layer(&path, &mut value, &mut sources)?;
        }
        let project = explicit.map_or_else(|| root.join(FILE_NAME), Path::to_path_buf);
        if explicit.is_some() || project.exists() {
            read_layer(&project, &mut value, &mut sources)?;
        }
    }
    let settings: Settings = value
        .try_into()
        .context("invalid rusteward configuration")?;
    // Each complete configuration is validated after CLI overrides have been applied.
    Ok(Loaded { settings, sources })
}

fn read_layer(path: &Path, value: &mut toml::Value, sources: &mut Vec<PathBuf>) -> Result<()> {
    debug!(path = %path.display(), "reading configuration layer");
    let source =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    overlay(value, &source).with_context(|| format!("cannot load {}", path.display()))?;
    // Detect unknown names per layer, so a later replacement cannot hide a typo.
    let _: Settings = value
        .clone()
        .try_into()
        .with_context(|| format!("invalid settings in {}", path.display()))?;
    sources.push(path.into());
    info!(path = %path.display(), "loaded configuration layer");
    Ok(())
}

/// Preserve the Coffer profile as typed built-in options, independent of Cargo aliases.
fn rustfmt_defaults() -> BTreeMap<String, toml::Value> {
    let source = include_str!("config/rustfmt-defaults.toml");
    toml::from_str(source).expect("built-in rustfmt defaults must be valid TOML")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClippyDefaults {
    lints: BTreeMap<String, ClippyLevel>,
    config: BTreeMap<String, toml::Value>,
}

/// Read the sole authoritative built-in lint and parameter profile.
fn clippy_defaults() -> ClippyDefaults {
    toml::from_str(include_str!("config/clippy-defaults.toml"))
        .expect("built-in Clippy defaults must be valid TOML")
}

#[cfg(test)]
mod tests;
