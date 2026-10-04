//! Decode Cargo records while preserving unrelated stdout exactly as emitted.

use std::{path::Path, str};

use anyhow::{Context, Result, bail};
use cargo_metadata::Message;

use crate::report::Diagnostic;

/// Compiler findings, ordinary output, and Cargo's optional final build result.
#[derive(Debug, Default)]
pub(super) struct ParsedOutput {
    pub diagnostics: Vec<Diagnostic>,
    pub output: String,
    pub build_success: Option<bool>,
}

impl ParsedOutput {
    /// Missing build results are valid only when Cargo failed before completing a build.
    pub(super) fn success(&self, process_success: bool) -> Result<bool> {
        anyhow::ensure!(
            !process_success || self.build_success.is_some(),
            "Clippy exited successfully without a Cargo build result"
        );
        Ok(process_success && self.build_success == Some(true))
    }
}

/// Validate known protocol records; Cargo may also print arbitrary nonprotocol text.
pub(super) fn parse(stdout: &[u8], root: &Path) -> Result<ParsedOutput> {
    let stdout = str::from_utf8(stdout).context("Cargo Clippy stdout is not valid UTF-8")?;
    let mut parsed = ParsedOutput::default();
    for (index, line) in stdout.split_inclusive('\n').enumerate() {
        if !declares_protocol_reason(line) {
            parsed.output.push_str(line);
            continue;
        }
        // parse_stream deliberately turns malformed records into text, which would hide
        // a broken compiler payload or build result. Deserialize known records strictly.
        let message: Message = serde_json::from_str(line).with_context(|| {
            format!(
                "malformed Cargo Clippy message on stdout line {}",
                index + 1
            )
        })?;
        match message {
            Message::CompilerMessage(message) => {
                parsed.diagnostics.push(Diagnostic::compiler(message, root));
            },
            Message::BuildFinished(message) => {
                if parsed.build_success.replace(message.success).is_some() {
                    bail!(
                        "duplicate Cargo build-finished message on stdout line {}",
                        index + 1
                    );
                }
            },
            Message::CompilerArtifact(_) | Message::BuildScriptExecuted(_) => {},
            _ => parsed.output.push_str(line),
        }
    }
    Ok(parsed)
}

/// Recognize a top-level reason even if its record is truncated after that field.
/// Only inspect object keys, so nested JSON and strings mentioning Cargo remain text.
fn declares_protocol_reason(line: &str) -> bool {
    let line = line.trim_start();
    if !line.starts_with('{') {
        return false;
    }
    let bytes = line.as_bytes();
    let mut depth = 1usize;
    let mut offset = 1usize;
    let mut field_start = true;
    while offset < bytes.len() {
        match bytes[offset] {
            b'"' => {
                let start = offset;
                offset += 1;
                while offset < bytes.len() {
                    match bytes[offset] {
                        b'\\' => offset += 2,
                        b'"' => break,
                        _ => offset += 1,
                    }
                }
                if offset >= bytes.len() {
                    return false;
                }
                offset += 1;
                if depth == 1
                    && field_start
                    && serde_json::from_str::<String>(&line[start..offset])
                        .is_ok_and(|key| key == "reason")
                    && let Some(value) = line[offset..].trim_start().strip_prefix(':')
                    && let Some(Ok(reason)) = serde_json::Deserializer::from_str(value)
                        .into_iter::<String>()
                        .next()
                    && matches!(
                        reason.as_str(),
                        "compiler-message"
                            | "compiler-artifact"
                            | "build-script-executed"
                            | "build-finished"
                    )
                {
                    return true;
                }
                field_start = false;
            },
            b'{' | b'[' => {
                depth += 1;
                field_start = false;
                offset += 1;
            },
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return false;
                }
                offset += 1;
            },
            b',' => {
                field_start = depth == 1;
                offset += 1;
            },
            byte if byte.is_ascii_whitespace() => offset += 1,
            _ => {
                field_start = false;
                offset += 1;
            },
        }
    }
    false
}

#[cfg(test)]
mod tests;
