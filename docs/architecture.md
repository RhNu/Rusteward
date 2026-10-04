# Code architecture

Rusteward is a local development tool distributed as `cargo-dev`. It does not depend on an xtask in
the project being checked. Each invocation resolves a target Cargo workspace; the tool's own build
directory does not determine that target.

User-visible behavior, settings, and examples are documented in
[Features and configuration](features.md). Installation and contributor commands are in the
[README](../README.md).

## Crate boundaries

The dependency direction is `apps/cli` → `crates/workspace` → `crates/core`.

| Crate                                       | Responsibility                                                                                                                                               | Main modules                                                 |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| `rusteward-core` in `crates/core`           | Pure transformations, lexical counting, source policies, diagnostics, and failure policy. Accepts source text, path values, editions, and rules without I/O. | `spacing`, `lines`, `rules`, `diagnostic`                    |
| `rusteward-workspace` in `crates/workspace` | Configuration layers, Cargo metadata, deterministic scanning, subprocesses, formatting writes, and workflow reports.                                         | `config`, `discovery`, `process`, `format`, `lint`, `report` |
| `rusteward` in `apps/cli`                   | The `cargo-dev` binary: argument normalization and parsing, tracing setup, text or JSON presentation, and exit status.                                       | `args`, `output`, `main`                                     |

The core crate must not discover projects, read files, run commands, or render terminal output.
Workspace workflows produce reports rather than presentation text. The CLI applies overrides to the
workspace settings model instead of maintaining a second configuration model.

Syntax analysis uses rust-analyzer's `ra_ap_syntax`; line counting uses its published rustc lexer.
The parser version and compatible Unicode tables are pinned to prevent lexical and syntactic
dependencies from diverging. `Cargo.lock` fixes the complete build dependency set.

## Configuration ownership

Workspace configuration defaults belong to `config`; source rule defaults and line thresholds belong
to core. The authoritative built-in rustfmt profile is
`crates/workspace/src/config/rustfmt-defaults.toml`, embedded by the workspace crate. Cargo aliases
only select the executable and do not duplicate that profile.

The authoritative built-in Clippy profile is `crates/workspace/src/config/clippy-defaults.toml`,
embedded by the same crate. It separates lint levels from Clippy configuration parameters. Workspace
manifests and Cargo aliases do not duplicate its rule catalogue;
`cargo dev lint --no-config --locked` uses the actual built-in profile for self-verification.

Configuration is represented as typed `Settings`. Loading starts from defaults, merges user and
project TOML layers, and records their paths. Tables merge recursively; scalar and array values
replace the previous layer. CLI overrides then update the same model, appending exclusion globs,
feature names, and Clippy arguments. An explicit configuration path replaces the discovered project
layer.

Deserialization rejects unknown Rusteward fields in each loaded layer, so a later replacement cannot
hide a typo. After CLI overrides, validation checks the version, threshold ordering, toolchain
names, rustfmt value encoding, Clippy lint names and parameter keys, and exclusion globs. Rusteward
field names use kebab-case; rustfmt and Clippy lint tables retain native snake_case, while Clippy
parameter keys retain native kebab-case.

Rustfmt options are passed as individual stable CLI overrides. An empty temporary configuration
isolates rustfmt's ambient configuration discovery. The workflow controls `edition`,
`skip_children`, and the emission mode for each file. Option support remains rustfmt's
responsibility: even a warning with a successful process status prevents the workflow from claiming
that the requested profile was applied.

Clippy lint levels become rustc flags. Clippy configuration parameters retain native TOML values and
are serialized to a temporary `clippy.toml`. The child process receives that directory through
`CLIPPY_CONF_DIR`; the directory remains alive until Clippy exits. This isolates ambient and
per-package Clippy configuration discovery in the same way that formatting isolates rustfmt
configuration. Clippy validates parameter support and value types. Cargo still supplies each
package's edition, minimum Rust version, and build configuration.

## Workspace discovery

`discovery::locate` uses Cargo metadata without dependency details to obtain the workspace root,
target directory, member package paths, and editions. Cargo metadata receives the manifest, locked,
and offline options independently of the selected Clippy toolchain.

Scanning covers the workspace tree and external member directories. The longest matching member
directory determines a file's edition; unmatched workspace files use edition 2024. Ordered
collections deduplicate and sort source paths before execution.

Traversal never follows symlinks. Built-in directory exclusions and configured globs prune
directories; the same glob set filters files. External members use paths relative to the workspace
root when possible, falling back to absolute paths across volumes. Scan and read errors propagate
rather than yielding a partial successful report.

Generated-file detection is a shared pure helper: an `@generated` marker in the first five lines
excludes the file from formatting and custom rules. Discovery itself returns paths without reading
their contents.

## Pure source analysis

### Declaration spacing

`spacing::separate_declarations` consumes already formatted Rust and returns the transformed text,
original coordinates of inserted lines, and an optional skip reason. It parses with the file's
edition, then visits source, module item, and associated item lists. Adjacent sibling nodes
determine where separation is needed; macro token trees and skipped containers remain opaque.

The transformation inspects trivia tokens after the preceding declaration, including leading trivia
attached to the next item by the parser. This handles the pitfall where a trailing comment is
represented as part of the next declaration: the insertion stays after that trailing comment and
before standalone comments or attributes belonging to the next item.

Edits contain only newline insertions. Their offsets are sorted and deduplicated, then applied in
reverse order to preserve byte offsets. The inserted newline follows the gap's LF or CRLF
convention. Existing separation is retained, making the transformation idempotent. A required gap
without a newline is an error because rustfmt is expected to have separated sibling declarations
first.

The precise spacing policy and its exceptions are described in the
[feature reference](features.md#declaration-spacing).

### Effective line counting

`lines::code_lines` tokenizes complete Rust tokens after removing a BOM and shebang. It counts each
physical line containing a non-whitespace character in a code token once. Comment and whitespace
tokens do not contribute. Tokenization keeps comment markers inside strings from being interpreted
as comments, and malformed input remains countable.

### Source policies

`rules::inspect` returns shared `Diagnostic` values. The line rule has independently enabled tiers,
with the higher tier taking precedence when exceeded. The filename rule uses the supplied path.
Inline-test detection traverses syntax modules without macro expansion or conditional-compilation
evaluation.

Syntax errors become error diagnostics when inline-test inspection parses a file. Disabling that
rule avoids parsing for the filename and line-count checks. Diagnostic positions use one-based lines
and character columns derived from UTF-8 byte offsets.

## Workflow execution

### Format

The workspace format workflow reads each source, skips generated files, invokes rustfmt with stdin,
applies optional declaration spacing, and compares the final text against the original. Differences
can become unified diffs, check diagnostics, or staged writes. A check observes the complete
pipeline without writing authored source files.

All formatting finishes before source writes begin. The workflow re-reads every pending source and
checks it against the original before any replacement, reporting an explicit conflict if an editor
changed it. Each replacement uses a temporary file in the source directory, preserves permissions,
syncs its contents, and persists it over the original. Replacement is atomic per file; multiple
files do not form a transaction and are not rolled back after a later I/O failure.

`process::with_input` writes stdin on a scoped thread while collecting stdout and stderr, avoiding
pipe deadlocks for large inputs. Child commands use argument vectors rather than shell
interpolation, preserving paths with spaces or non-ASCII text.

### Lint

Custom source inspection completes before optional Clippy execution. Custom rule errors do not
suppress Clippy, so the report can include both sets of findings. Clippy's Cargo options precede
`--`; `-D warnings`, configured group levels, individual lint levels, and user `clippy-args` follow
it in that order. Group flags must precede individual lint overrides so a selected exception is not
overwritten by its group. Cargo configuration continues to control the target platform and build
environment.

Clippy runs with Cargo's JSON message format and color disabled. `lint::messages` is a pure decoder
of captured stdout: it strictly validates recognized Cargo protocol records, collects compiler
messages and the optional final build result, discards artifact/build-script records, and preserves
unrelated text verbatim. A small lexical check recognizes declared protocol reasons even in
truncated JSON because `cargo_metadata::Message::parse_stream` otherwise silently treats malformed
records as ordinary text. Duplicate final build results are rejected. A successful process must
include a build result; an unsuccessful process may fail before Cargo emits any protocol records.
The process exit status and Cargo's build result both participate in Clippy success.

### Check and reporting

`check` runs read-only formatting, then lint, and merges their reports without double-counting the
shared source set. Ordinary formatting differences allow lint to continue. Configuration, I/O,
process-start, and formatting failures abort the operation.

`Report` carries source counts, changed and skipped counts, unified diagnostics, optional diffs, and
Clippy process/build status with separate non-protocol output. The workspace report model adapts
pure core diagnostics and Cargo compiler messages into common source, code, level, message, and
optional primary-location fields. Compiler details retain package/target context, every span, nested
diagnostics, grouped suggestions, explanations, and rendered text. Cargo types stay in the workspace
and presentation layers; core does not depend on the compiler protocol. Disabled core diagnostics
are omitted during conversion.

Summary counts use top-level diagnostics across all producers; child notes and suggestions do not
increase error totals. Failure policy combines error/ICE diagnostics with Clippy's status. Denying
custom warnings affects only Rusteward diagnostics, allowing explicit Clippy flags to control
compiler warning policy without relabeling findings. The CLI maps success, completed check failure,
and operational failure to exit codes 0, 1, and 2.

Libraries emit tracing events; the CLI owns filters and writes logs to stderr. Operational errors
have one presentation path instead of appearing again as tracing errors. JSON command results use a
common versioned outcome envelope, and check results expose diagnostics, summaries, diffs, and child
logs as separate fields. JSON report contents are independent of quiet/verbose presentation. Text
mode shows compact diagnostics and help by default, full rendered compiler context in verbose mode,
and captured child logs on verbosity or process failure. Quiet mode suppresses successful summaries
and default tracing. Explicit `RUST_LOG` filters override that tracing default. Argument parsing,
help, and version presentation remain owned by clap.

The CLI's report/error writers accept output sinks so unit tests can verify presentation using
synthetic reports without launching processes or interacting with a terminal.

## Extension and verification boundaries

Add source transformations or policies as pure core functions with behavioral unit tests using
minimal synthetic source. Reuse the shared diagnostic and severity types. Keep scanning,
configuration discovery, filesystem changes, and subprocess execution in the workspace crate; keep
argument parsing and presentation in the CLI.

Extend existing settings and override paths when exposing a new option, and update the
[feature reference](features.md) and [configuration example](../examples/rusteward.toml). Clippy
policies belong in `clippy-lints`, parameters belong in `clippy-config`, and additional rustc flags
can use `clippy-args`. A new crate is warranted only when an independent responsibility, such as
semantic analysis, requires it.

Automated verification is limited to pure logic: transformations, rules, configuration merging and
validation, argument construction, compiler-message decoding/conversion, report policies, output
generation, CLI parsing, and diffs. Filesystem behavior, actual subprocess execution, installation,
external workspace discovery, and editor integration require manual QA. Contributor commands and the
Markdown formatting workflow are documented in the [README](../README.md#development).
