//! Cargo subcommand entry point; policy and workflows live in the libraries.

mod args;
mod output;

use std::{fs, io::Write, path::Path, process::ExitCode};

use anyhow::{Context, Result};
use rusteward_workspace::{config, discovery, execution::Executor, format, process, workflow};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::args::{Cli, Command, ConfigCommand};

fn main() -> ExitCode {
    let cli = args::parse(std::env::args_os()).unwrap_or_else(|error| error.exit());
    let presentation = output::Presentation::detect(&cli);
    let default_filter = match cli.verbose {
        0 if cli.quiet => "off",
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(presentation.color)
        .without_time()
        .with_target(cli.verbose > 1)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)),
        )
        .init();
    match run(&cli, presentation) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            if let Err(write_error) = output::error(&cli, &error, presentation) {
                eprintln!("cargo dev: cannot write error output: {write_error:#}");
            }
            ExitCode::from(2)
        },
    }
}

/// Completed workflows retain their report on failure; setup errors have no workflow report.
fn run(cli: &Cli, presentation: output::Presentation) -> Result<u8> {
    if let Command::Config {
        command: ConfigCommand::Init { global: true },
    } = &cli.command
    {
        let path =
            config::user_path().context("cannot determine the user configuration directory")?;
        initialize(&path, cli)?;
        return Ok(0);
    }
    let cargo = process::CargoOptions {
        manifest_path: cli.manifest_path.clone(),
        locked: cli.locked,
        offline: cli.offline,
    };
    let workspace = discovery::locate(&cargo)?;
    if let Command::Config {
        command: ConfigCommand::Init { global: false },
    } = &cli.command
    {
        let path = cli
            .config
            .clone()
            .unwrap_or_else(|| workspace.root.join(config::FILE_NAME));
        initialize(&path, cli)?;
        return Ok(0);
    }
    let loaded = config::load(&workspace.root, cli.config.as_deref(), cli.no_config)?;
    let mut settings = loaded.settings;
    cli.overrides.apply(&mut settings)?;
    config::validate(&settings)?;
    discovery::exclusions(&settings.scan.exclude)?;
    if let Command::Config {
        command: ConfigCommand::Show,
    } = &cli.command
    {
        if cli.json {
            let mut result = output::envelope(true, 0);
            result["workspace"] = serde_json::to_value(&workspace.root)?;
            result["sources"] = serde_json::to_value(&loaded.sources)?;
            result["settings"] = serde_json::to_value(&settings)?;
            writeln!(std::io::stdout().lock(), "{result}")?;
        } else {
            writeln!(
                std::io::stdout().lock(),
                "{}",
                toml::to_string_pretty(&settings)?
            )?;
        }
        return Ok(0);
    }
    let sources = discovery::sources(&workspace, &settings.scan)?;
    output::workspace(cli, &workspace.root, sources.len())?;
    let executor = Executor::new(&settings.execution, sources.len())?;
    let command = match &cli.command {
        Command::Format { check, diff } => workflow::Command::Format(format::Options {
            check: *check,
            diff: *diff,
        }),
        Command::Lint => workflow::Command::Lint,
        Command::Check { diff } => workflow::Command::Check { diff: *diff },
        Command::Config { .. } => unreachable!("configuration commands returned above"),
    };
    let outcome = workflow::run(
        &workspace,
        &sources,
        &settings,
        &executor,
        &cargo,
        command,
        &mut |event| output::progress(cli, event, presentation),
    );
    output::render(cli, &workspace.root, &outcome, presentation)?;
    Ok(outcome.exit_code())
}

/// Never overwrite an existing configuration, including when the path is supplied explicitly.
fn initialize(path: &Path, cli: &Cli) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "cannot create {}; existing configuration files are never overwritten",
                path.display()
            )
        })?;
    file.write_all(include_bytes!("../../../examples/rusteward.toml"))?;
    info!(path = %path.display(), "initialized configuration");
    if cli.json {
        let mut result = output::envelope(true, 0);
        result["path"] = serde_json::to_value(path)?;
        writeln!(std::io::stdout().lock(), "{result}")?;
    } else if !cli.quiet {
        writeln!(std::io::stdout().lock(), "Created {}", path.display())?;
    }
    Ok(())
}
