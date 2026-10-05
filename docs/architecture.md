# Code architecture

Rusteward is a local development tool distributed as `cargo-dev`. It does not depend on an xtask in
the project being checked. Each invocation resolves a target Cargo workspace; the tool's own build
directory does not determine that target.

User-visible behavior, settings, and examples are documented in
[Features and configuration](features.md). Installation and contributor commands are in the
[README](../README.md).

## Crate boundaries

The dependency direction is `apps/cli` → `crates/workspace` → `crates/core`.

| Crate                                       | Responsibility                                                                                                                                               | Main modules                                                                          |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------- |
| `rusteward-core` in `crates/core`           | Pure transformations, lexical counting, source policies, diagnostics, and failure policy. Accepts source text, path values, editions, and rules without I/O. | `spacing`, `lines`, `rules`, `diagnostic`                                             |
| `rusteward-workspace` in `crates/workspace` | Configuration layers, Cargo metadata, deterministic scanning, bounded file execution, subprocesses, formatting writes, and workflow reports.                 | `config`, `discovery`, `execution`, `process`, `format`, `lint`, `workflow`, `report` |
| `rusteward` in `apps/cli`                   | The `cargo-dev` binary: argument normalization and parsing, tracing setup, text or JSON presentation, and exit status.                                       | `args`, `output`, `main`                                                              |

The core crate must not discover projects, read files, run commands, or render terminal output.
Workspace workflows produce reports rather than presentation text. The CLI applies overrides to the
workspace settings model instead of maintaining a second configuration model. Core transformations
and inspections remain synchronous and pure; file concurrency belongs to the workspace crate.

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

### File executor

`execution::Executor` owns a local Rayon thread pool for bounded file processing. The CLI creates
one executor per invocation from `[execution].jobs` after CLI overrides and the discovered file
count; format and lint reuse it. Automatic selection (`0`) uses
`std::thread::available_parallelism`, falling back to one when the query fails, and caps the result
at the file count. An explicit positive limit is also capped at the file count. Zero files need no
workers, and one worker takes the serial path without creating a pool.

The executor maps independent file tasks over the ordered source list and collects their results in
input order. Each task returns its own result; workers do not mutate a shared `Report`. Sequential
aggregation preserves diagnostic and diff ordering, combines counts, and selects the first
operational error in source order after in-flight parallel tasks finish. The worker budget limits
simultaneous file tasks and rustfmt processes, not total threads or processes. Cargo Clippy retains
Cargo's own build job settings.

Info tracing events record the effective worker count and elapsed phase time; debug events record
per-file duration. Parallel events can arrive in completion order while reports retain source order.

### Format

Each workspace format task reads a source, skips generated files, invokes rustfmt with stdin,
applies optional declaration spacing, and compares the final text against the original. These tasks
run through the executor. Differences can become unified diffs, check diagnostics, or staged writes.
A check observes the complete pipeline without writing authored source files.

All formatting finishes before source writes begin. The workflow re-reads every pending source and
checks it against the original before any replacement, reporting an explicit conflict if an editor
changed it. Each replacement uses a temporary file in the source directory, preserves permissions,
syncs its contents, and persists it over the original. Replacement is atomic per file; multiple
files do not form a transaction and are not rolled back after a later I/O failure.

`process::with_input` writes stdin on a scoped thread while collecting stdout and stderr, avoiding
pipe deadlocks for large inputs. Child commands use argument vectors rather than shell
interpolation, preserving paths with spaces or non-ASCII text.

### Lint

Custom source reads and independent inspections run through the executor, with results merged in
source order before optional Clippy execution. Custom rule errors do not suppress Clippy, so the
report can include both sets of findings. Clippy's Cargo options precede `--`; `-D warnings`,
configured group levels, individual lint levels, and user `clippy-args` follow it in that order.
Group flags must precede individual lint overrides so a selected exception is not overwritten by its
group. Cargo configuration continues to control the target platform and build environment.

The single Cargo invocation includes `--keep-going` to continue independent compilation units after
failures; units that depend on a failed unit remain blocked.

Clippy runs with Cargo's JSON message format and color disabled. `lint::messages` is a pure decoder
of captured stdout: it strictly validates recognized Cargo protocol records, collects compiler
messages and the optional final build result, discards artifact/build-script records, and preserves
unrelated text verbatim. A small lexical check recognizes declared protocol reasons even in
truncated JSON because `cargo_metadata::Message::parse_stream` otherwise silently treats malformed
records as ordinary text. Duplicate final build results are rejected. A successful process must
include a build result; an unsuccessful process may fail before Cargo emits any protocol records.
The process exit status and Cargo's build result both participate in Clippy success. `lint::inspect`
handles custom source rules; `lint::clippy` handles the managed compiler invocation. Clippy captures
raw output in the report before decoding it, so protocol failures retain inspectable logs without
repeating them in an error string.

### Check and reporting

`workflow::run` selects Formatting, Source rules, and Clippy phases from a CLI-independent command.
The phases run sequentially using the same executor. Each selected phase starts as not run and moves
through running to passed, failed, or error. Disabled Clippy becomes skipped without execution.
Stage failure is evaluated against only that stage's new diagnostics and process status, including
the custom warning policy. Ordinary failed checks allow later stages to continue.

The runner emits stage-boundary events to an observer on the calling thread. These events contain
state and timing data; terminal output belongs to the CLI. File workers never render progress. An
operational failure stops the workflow, marks the current phase as error, and leaves later phases
not run. `Outcome` retains the partial `Report` alongside the optional error. Its exit status gives
operational failures precedence over failed checks. Configuration and discovery failures occur
before the runner and have no partial workflow report.

`Report` carries phase status and timing, source counts, changed and skipped counts, unified
diagnostics, optional diffs, and Clippy process/build status with separate non-protocol output.
Reports merge without double-counting the shared source set. The workspace report model adapts pure
core diagnostics and Cargo compiler messages into common source, code, level, message, and optional
primary-location fields. Custom findings capture their original source line while that text is
already available; the CLI does not re-read files to render diagnostics. Compiler details retain
package/target context, every span, nested diagnostics, grouped suggestions, explanations, and
rendered text. Cargo types stay in the workspace and presentation layers; core does not depend on
the compiler protocol. Disabled core diagnostics are omitted during conversion.

Summary counts use top-level diagnostics across all producers; child notes and suggestions do not
increase error totals. Failure policy combines error/ICE diagnostics, phase failures, and Clippy's
status. Denying custom warnings affects only Rusteward diagnostics, allowing explicit Clippy flags
to control compiler warning policy without relabeling findings. The CLI maps success, completed
check failure, and operational failure to exit codes 0, 1, and 2.

Libraries emit tracing events; the CLI owns filters and writes logs to stderr. Operational errors
have one presentation path instead of appearing again as tracing errors. JSON command results use a
common versioned outcome envelope, and workflow results expose phases, diagnostics, summaries,
diffs, and child logs as separate fields, including completed results on abort. JSON report contents
are independent of presentation flags.

Text mode presents live phase boundaries, complete compiler-rendered diagnostics, and a
command-specific final result. Custom findings use a compatible heading/location layout and captured
source snippets. The fallback compiler presenter keeps child notes and edit groups nested instead of
flattening replacement spans. Captured Cargo logs are labelled sections shown in verbose mode, on
unexplained process failures, or on operational compiler failures. Compiler errors that already
explain a failed build suppress default Cargo log replay. Quiet mode keeps full diagnostics and
necessary failure information while hiding progress and successful summaries. Runtime terminal and
color capabilities are explicit presentation inputs; the CLI applies styling after collecting
unstyled compiler output. Explicit `RUST_LOG` filters override the tracing default. Argument
parsing, help, and version presentation remain owned by clap.

The CLI's report/error writers accept output sinks so unit tests can verify presentation using
synthetic reports without launching processes or interacting with a terminal.

## GitHub Actions distribution

The root `action.yml` exposes a Node.js 24 installation Action independently of the Rust CLI.
`action/src/model.mjs` owns pure platform, input, revision, asset, checksum, and manifest decisions;
`action/src/install.mjs` owns GitHub API access, release polling, downloading, extraction,
filesystem validation, caches, PATH registration, outputs, and installation logs. These
responsibilities stay outside the Rust core and workspace crates.

The installer resolves its source revision once to a full commit SHA. Local tool-cache and GitHub
Actions cache entries are keyed by that SHA and target, and are validated before reuse. Disabling
the cache input skips GitHub Actions cache operations but retains runner-local reuse. Downloads use
the matching public `ci-<SHA>` prerelease and validate the archive checksum, embedded manifest, and
binary hash. Incomplete releases are retried within the configured timeout; invalid packages fail.
There is no older-release or source-build fallback. The Action does not run Rusteward or alter the
consumer's Rust toolchain.

`action/src/package.mjs` packages the binary and provenance manifest with checksums;
`action/src/publish.mjs` publishes the complete platform set. `.github/workflows/ci.yml` verifies
changes on pushes, pull requests, and manual dispatch. Publishing is restricted to main after
verification and all native builds succeed: Windows x64 on Windows 2022, and Linux x64/ARM64 on
Ubuntu 22.04. Linux artifacts use GNU libc and require glibc 2.35 or newer.

Publication is restricted to `RhNu/Rusteward`. The publisher uploads all archives and checksums to a
draft before making the prerelease visible, allowing interrupted draft uploads to be retried.
Already-published packages are retained rather than replaced; an incomplete published release is an
error. Publication jobs for the same commit run serially, so a push and manual dispatch cannot
modify the same draft concurrently.

`action/scripts/build.mjs` bundles the installer into the committed `action/dist/index.cjs` with
dependency license notices, so consumer workflows need no npm install. Generated artifacts use LF
line endings for reproducible checks across platforms. `action/scripts/check.mjs` checks JavaScript
and YAML syntax and bundle freshness. Pure Action unit tests cover installation and packaging
decisions; actual release publication, hosted installation, cache persistence, and performance
remain manual verification boundaries. Installer code selection in `uses` is independent of binary
source selection through the `ref` input. User-facing inputs and outputs belong in the
[feature reference](features.md#github-actions-installation).

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
generation, CLI parsing, worker-count resolution, and diffs. Filesystem behavior, actual file and
subprocess parallelism, parallel report ordering, installation, external workspace discovery, editor
conflicts, and performance require manual QA. Contributor commands and the Markdown formatting
workflow are documented in the [README](../README.md#development).
