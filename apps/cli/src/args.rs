//! Normalize Cargo's extra subcommand argument and apply explicit CLI overrides.

use std::{ffi::OsString, path::PathBuf};

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use rusteward_core::diagnostic::Severity;
use rusteward_workspace::config::{Settings, parse_rustfmt_override};

#[derive(Debug, Parser)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "global command options are independent switches exposed by clap"
)]
#[command(
    name = "cargo-dev",
    bin_name = "cargo dev",
    version,
    about = "Personal Rust formatting and lint workflows"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
    /// Cargo manifest used to locate the target workspace.
    #[arg(long, global = true)]
    pub manifest_path: Option<PathBuf>,
    /// Replace the automatically discovered workspace configuration.
    #[arg(long, global = true, conflicts_with = "no_config")]
    pub config: Option<PathBuf>,
    /// Use built-in defaults and CLI overrides without reading config files.
    #[arg(long, global = true)]
    pub no_config: bool,
    /// Require Cargo.lock to be current for metadata and Clippy.
    #[arg(long, global = true)]
    pub locked: bool,
    /// Prevent Cargo from accessing the network.
    #[arg(long, global = true)]
    pub offline: bool,
    /// Print one versioned JSON result with structured diagnostics and separate child logs.
    #[arg(long, global = true)]
    pub json: bool,
    /// Hide successful summaries; findings are still printed.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,
    /// Show full compiler diagnostics and workflow logs (-v); add per-file logs with -vv.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
    #[command(flatten)]
    pub overrides: Overrides,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Apply rustfmt and declaration spacing to authored Rust source files.
    Format {
        /// Compare the full pipeline without modifying source files.
        #[arg(long)]
        check: bool,
        /// Show final unified diffs; combine with --check for a read-only operation.
        #[arg(long)]
        diff: bool,
    },
    /// Check source policies and run Clippy with -D warnings.
    Lint,
    /// Check formatting and run lint, collecting both sets of findings.
    Check {
        #[arg(long)]
        diff: bool,
    },
    /// Initialize configuration or inspect the fully merged settings.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Create an annotated configuration without overwriting an existing file.
    Init {
        /// Create the configuration in the platform's user config directory.
        #[arg(long, conflicts_with = "config")]
        global: bool,
    },
    /// Print settings after all configuration layers and CLI overrides.
    Show,
}

#[derive(Debug, Default, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "configuration overrides represent independent command-line switches"
)]
pub struct Overrides {
    /// Append an exclusion glob relative to the workspace root (repeatable).
    #[arg(long, global = true)]
    pub exclude: Vec<String>,
    /// Set a rule to off, warning, or error: lines, mod-rs, inline-tests.
    #[arg(long, global = true, value_name = "RULE=LEVEL")]
    pub rule: Vec<String>,
    /// Warn only when a file exceeds this many effective code lines.
    #[arg(long, global = true)]
    pub line_warning: Option<usize>,
    /// Fail only when a file exceeds this many effective code lines.
    #[arg(long, global = true)]
    pub line_error: Option<usize>,
    /// Treat custom rule warnings as a failed check without changing their labels.
    #[arg(long, global = true)]
    pub deny_warnings: bool,
    /// Run custom policies without invoking Clippy.
    #[arg(long, global = true)]
    pub skip_clippy: bool,
    /// Run rustfmt without the declaration-spacing step.
    #[arg(long, global = true)]
    pub no_spacing: bool,
    /// Override a rustfmt option; TOML values or bare enum names are accepted.
    #[arg(long, global = true, value_name = "KEY=VALUE")]
    pub rustfmt_option: Vec<String>,
    /// Select a rustup toolchain for rustfmt; defaults to the project's active toolchain.
    #[arg(long, global = true)]
    pub rustfmt_toolchain: Option<String>,
    /// Select a rustup toolchain for Clippy.
    #[arg(long, global = true)]
    pub clippy_toolchain: Option<String>,
    /// Enable features for Clippy; repeat or separate feature names with commas.
    #[arg(short = 'F', long, global = true, value_delimiter = ',')]
    pub features: Vec<String>,
    #[arg(long, global = true)]
    pub all_features: bool,
    #[arg(long, global = true)]
    pub no_default_features: bool,
    /// Set Clippy's --all-targets selection; accepts --all-targets=false.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "true", require_equals = true)]
    pub all_targets: Option<bool>,
    /// Append a Clippy/rustc flag after -D warnings (repeatable).
    #[arg(long, global = true, allow_hyphen_values = true)]
    pub clippy_arg: Vec<String>,
}

impl Overrides {
    /// Arrays of excludes/features/Clippy flags append; rustfmt values replace by key.
    ///
    /// # Errors
    /// Returns an error if a rule name, severity, or rustfmt override is invalid.
    pub fn apply(&self, settings: &mut Settings) -> Result<()> {
        settings.scan.exclude.extend(self.exclude.iter().cloned());
        if let Some(warn) = self.line_warning {
            settings.rules.lines.warn = warn;
        }
        if let Some(error) = self.line_error {
            settings.rules.lines.error = error;
        }
        for argument in &self.rule {
            let Some((rule, level)) = argument.split_once('=') else {
                bail!("expected --rule RULE=LEVEL");
            };
            let level = match level {
                "off" => Severity::Off,
                "warning" => Severity::Warning,
                "error" => Severity::Error,
                _ => bail!("invalid rule level {level:?}; expected off, warning, or error"),
            };
            match rule {
                "lines" => {
                    settings.rules.lines.warn_level = level;
                    settings.rules.lines.error_level = level;
                },
                "mod-rs" => settings.rules.mod_rs = level,
                "inline-tests" => settings.rules.inline_tests = level,
                _ => bail!("unknown rule {rule:?}; expected lines, mod-rs, or inline-tests"),
            }
        }
        if self.deny_warnings {
            settings.lint.deny_warnings = true;
        }
        if self.skip_clippy {
            settings.lint.clippy = false;
        }
        if self.no_spacing {
            settings.format.spacing = false;
        }
        for argument in &self.rustfmt_option {
            let (key, value) = parse_rustfmt_override(argument)?;
            settings.format.rustfmt.insert(key, value);
        }
        if let Some(toolchain) = &self.rustfmt_toolchain {
            settings.format.toolchain = Some(toolchain.clone());
        }
        if let Some(toolchain) = &self.clippy_toolchain {
            settings.lint.toolchain = Some(toolchain.clone());
        }
        if let Some(all_targets) = self.all_targets {
            settings.lint.all_targets = all_targets;
        }
        if self.all_features {
            settings.lint.all_features = true;
        }
        if self.no_default_features {
            settings.lint.no_default_features = true;
        }
        settings.lint.features.extend(self.features.iter().cloned());
        settings
            .lint
            .clippy_args
            .extend(self.clippy_arg.iter().cloned());
        Ok(())
    }
}

/// Cargo invokes cargo-dev with "dev" as its first argument; direct execution omits it.
pub fn normalize(arguments: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut arguments: Vec<_> = arguments.into_iter().collect();
    if arguments.get(1).is_some_and(|argument| argument == "dev") {
        arguments.remove(1);
    }
    arguments
}

#[cfg(test)]
mod tests;
