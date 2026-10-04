//! Build commands as argument vectors and keep child process failures explicit.

use std::{
    any::Any,
    ffi::OsString,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result, anyhow};
use tracing::debug;

#[derive(Clone, Debug, Default)]
pub struct CargoOptions {
    pub manifest_path: Option<std::path::PathBuf>,
    pub locked: bool,
    pub offline: bool,
}

/// Honor Cargo's supplied executable unless a specific rustup toolchain was requested.
pub fn cargo(toolchain: Option<&str>) -> Command {
    if let Some(toolchain) = toolchain {
        let mut command = Command::new("rustup");
        command.args(["run", toolchain, "cargo"]);
        command
    } else {
        Command::new(std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
    }
}

pub fn rustfmt(toolchain: Option<&str>) -> Command {
    if let Some(toolchain) = toolchain {
        let mut command = Command::new("rustup");
        command.args(["run", toolchain, "rustfmt"]);
        command
    } else {
        Command::new("rustfmt")
    }
}

/// Drain output while supplying stdin so large sources cannot deadlock on full pipes.
///
/// # Errors
/// Returns an error when the child cannot start, its output cannot be collected, its input writer
/// panics, or input cannot be delivered to a successful child.
pub fn with_input(mut command: Command, source: &str, path: &Path) -> Result<Output> {
    debug!(program = ?command.get_program(), path = %path.display(), "running rustfmt");
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context(
            "cannot start rustfmt; install the rustfmt component for the selected toolchain",
        )?;
    let mut stdin = child.stdin.take().context("rustfmt stdin was not piped")?;
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(source.as_bytes()));
        let output = child
            .wait_with_output()
            .context("cannot collect rustfmt output")?;
        let write_result = writer
            .join()
            .map_err(|payload| writer_panic(payload.as_ref()))?;
        if output.status.success() {
            write_result.context("cannot send source to rustfmt")?;
        }
        Ok(output)
    })
}

/// Thread joins return type-erased panic payloads; preserve their message when one is available.
fn writer_panic(payload: &(dyn Any + Send)) -> anyhow::Error {
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload");
    anyhow!("rustfmt input writer panicked: {message}")
}

#[cfg(test)]
mod tests;
