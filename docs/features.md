# Features and configuration

This reference describes Rusteward's user-visible behavior. See the [README](../README.md) for
installation and quick start, and [Code architecture](architecture.md) for implementation and
extension boundaries.

## Commands

| Command                          | Behavior                                                                                         |
| -------------------------------- | ------------------------------------------------------------------------------------------------ |
| `cargo dev format`               | Run rustfmt, add declaration spacing, and write the final text.                                  |
| `cargo dev format --check`       | Compare the complete formatting result without writing source files.                             |
| `cargo dev format --diff`        | Show unified diffs of the final result and apply changes. Add `--check` to make it read-only.    |
| `cargo dev lint`                 | Run custom source rules and the strict managed Clippy profile for all workspace targets.         |
| `cargo dev lint --skip-clippy`   | Run custom source rules without compiling the target project.                                    |
| `cargo dev check`                | Check formatting, then collect lint findings even when formatting differs. Supports `--diff`.    |
| `cargo dev config init`          | Create an annotated `rusteward.toml` at the workspace root without overwriting an existing file. |
| `cargo dev config init --global` | Create an annotated configuration in the platform's user configuration directory.                |
| `cargo dev config show`          | Show the effective settings after configuration layers and CLI overrides.                        |

All commands support `--manifest-path PATH`. Invocation from a project subdirectory still uses its
Cargo workspace as the configuration and scan boundary. `config init --config PATH` creates the
configuration at the supplied path.

`--locked` and `--offline` apply to both Cargo metadata and Clippy. Without them, Cargo follows its
normal behavior: even a formatting check may create or update a lockfile while resolving metadata,
although it does not write source files.

## Formatting

Formatting has two ordered stages: rustfmt, then optional declaration spacing. `format --check`
compares the original source with the result of both stages, so running rustfmt alone does not
necessarily produce the final accepted format.

Each file is sent to rustfmt independently through stdin with its owning package's edition.
Rusteward supplies an empty temporary rustfmt configuration and passes its effective profile through
CLI overrides. Standalone `rustfmt.toml` files and user rustfmt configurations do not participate;
`cargo dev config show` exposes the profile Rusteward uses. Excluded child modules are not
recursively formatted.

### Declaration spacing

Spacing is enabled by default. For each pair of adjacent sibling declarations, Rusteward requires at
least one blank line if **either declaration** is a function, `struct`, `enum`, `impl`, or `trait`.

| Adjacent declarations                             | Spacing added when missing? |
| ------------------------------------------------- | --------------------------- |
| Function followed by function                     | Yes.                        |
| Import followed by struct                         | Yes.                        |
| Enum followed by constant                         | Yes.                        |
| Type alias followed by method in an impl or trait | Yes.                        |
| Import followed by import                         | No.                         |
| Constant followed by constant or type alias       | No.                         |
| Module declaration followed by module declaration | No.                         |

The condition is symmetric: a constant before a function and a function before a constant both
require separation. Imports, constants, type aliases, module declarations, and macro declarations do
not require it on their own, but are separated from a selected neighbor. Visibility and attributes
do not change the declaration kind.

The following examples isolate the spacing stage; rustfmt may make additional changes earlier in the
full pipeline. For example, spacing changes this input:

```rust
use std::path::Path;
const LIMIT: usize = 10;
fn within_limit(path: &Path) -> bool {
    path.components().count() <= LIMIT
}
type Label = String;
struct Entry {
    path: Label,
    enabled: bool,
}
```

Into:

```rust
use std::path::Path;
const LIMIT: usize = 10;

fn within_limit(path: &Path) -> bool {
    path.components().count() <= LIMIT
}

type Label = String;

struct Entry {
    path: Label,
    enabled: bool,
}
```

The import and constant stay together. The function is separated on both sides, and the type alias
is separated from the following struct. Struct fields remain compact.

#### Where spacing applies

The rule applies to declarations at file scope, inside inline modules, and inside `impl` and `trait`
bodies. Trait methods require separation even when they have no body. Associated constants and types
stay together unless a selected declaration is their neighbor.

Before the spacing stage:

```rust
trait Store {
    type Key;
    const CAPACITY: usize;
    fn get(&self, key: &Self::Key);
    fn clear(&mut self);
}
```

After:

```rust
trait Store {
    type Key;
    const CAPACITY: usize;

    fn get(&self, key: &Self::Key);

    fn clear(&mut self);
}
```

Spacing does not add blank lines between struct fields, enum variants, local statements or
declarations in a function block, or tokens inside a macro body or invocation. An inline module's
sibling declarations can receive spacing even when the module is nested inside a function. The rule
does not add separation just after an opening brace or just before a closing brace.

Macros are opaque. For example, the two functions inside this macro remain together, while the
function after the macro is separated:

```rust
macro_rules! handlers {
    () => {
        fn first() {}
        fn second() {}
    };
}

fn outside() {}
```

#### Comments, attributes, and existing blank lines

Trailing comments stay with the previous declaration. Standalone comments, documentation comments,
and stacked attributes stay with the following declaration; the blank line is inserted before that
group.

Before:

```rust
fn first() {} // Keep this trailing comment here.
/// Documentation for the next function.
#[must_use]
fn second() -> bool {
    true
}
```

After:

```rust
fn first() {} // Keep this trailing comment here.

/// Documentation for the next function.
#[must_use]
fn second() -> bool {
    true
}
```

Nested block comments and string contents are preserved. The spacing stage only inserts missing
newlines: it does not remove existing blank lines or rewrite tokens. Running it again on the same
result adds nothing. Inserted newlines use the existing gap's LF or CRLF style. These guarantees
describe the spacing stage; rustfmt still controls the preceding formatting stage.

#### Skips and disabling spacing

- An `@generated` marker in the first five lines skips the whole file for formatting and custom
  rules.
- File-level `#![rustfmt::skip]` skips declaration spacing throughout the file.
- A skipped module, impl, or trait keeps its internal spacing. A blank line can still be inserted
  between the skipped container and an adjacent selected declaration.
- `cfg_attr(..., rustfmt::skip)` is respected conservatively by the spacing stage without evaluating
  its condition. Other conditional-compilation gates do not restrict the source scan to enabled
  items.
- Invalid Rust, or sibling declarations that still share a line where separation is required, causes
  a spacing error instead of a partial formatting result.

To run rustfmt without declaration spacing:

```text
cargo dev format --no-spacing
```

Or configure:

```toml
[format]
spacing = false
```

`rustfmt::skip` handling above describes the spacing stage; rustfmt interprets its own attributes
during the earlier stage.

### Applying changes

Rusteward finishes formatting all selected files before writing any source changes. It rechecks
every pending file against the original input to detect intervening edits, then replaces each file
atomically using a temporary file in the same directory, retaining permissions.

The write is atomic per file, not across the entire workspace. If a later I/O operation fails, files
already replaced can remain modified.

## Custom source rules

| Rule           | Default behavior                                                                                      |
| -------------- | ----------------------------------------------------------------------------------------------------- |
| `lines`        | Warn above 650 effective code lines; error above 1200. Equality with a threshold does not trigger it. |
| `mod-rs`       | Error for a file named `mod.rs`; prefer `<module>.rs` with a `<module>/` child directory.             |
| `inline-tests` | Warn for an inline module named `tests` or an inline module directly annotated with `#[cfg(test)]`.   |

Effective code lines are physical lines containing Rust code, counted lexically. Blank lines,
comments, BOM, and shebang are excluded. Nonempty code lines inside multiline strings count, while
blank string lines do not. Comment markers inside strings, nested block comments, Unicode, and CRLF
are handled as Rust tokens.

Use external unit-test modules:

```rust
#[cfg(test)]
mod tests;
```

Place crate-root tests in `src/tests.rs`; other module tests belong in `<module>/tests.rs`. External
module declarations do not trigger `inline-tests`. Detection uses syntax modules, without expanding
macros or evaluating complex conditional-compilation expressions. When enabled, this inspection also
reports syntax errors as errors. Disabling `inline-tests` leaves the filename and lexical line-count
rules active without requiring a syntax parse.

Each rule accepts `off`, `warning`, or `error`. The two line-count tiers have independent thresholds
and severities. An exceeded enabled upper tier takes precedence; disabling it still allows the lower
tier to report findings. `--rule lines=LEVEL` sets both severities together.

Custom warnings do not fail checks by default. `--deny-warnings` makes them count as failures while
keeping their warning labels.

## Scan scope

Rusteward scans the whole workspace and external member directories for Rust files, including tests,
examples, `build.rs`, and source behind disabled features. Files under member directories use that
member's Rust edition; other workspace files use edition 2024.

The scanner skips symlinks, Cargo's target directory, and child directories named `target`, `.git`,
`node_modules`, or `.folio`. The default exclusion globs also skip `vendor` and `dist`. Generated
files are detected by an `@generated` marker in the first five lines and skipped by formatting and
custom checks.

The scanner does not automatically use `.gitignore`, allowing Git-ignored authored source to remain
subject to policy checks. Exclusion globs match paths relative to the workspace root, using `/`
separators. External members can be excluded with patterns such as `../shared/**`; members on
another volume require absolute globs. An unreadable source or failed directory traversal aborts the
operation.

Exclusions affect formatting and custom rules. Clippy's compilation scope is determined by Cargo and
the feature and target options, independently of scan globs.

## File concurrency

Formatting and custom source inspection process independent files concurrently. Configure the limit
with `--jobs N` (or `-j N`), which overrides `[execution].jobs`:

```toml
[execution]
jobs = 4
```

The default, `0`, selects the available parallelism reported by the operating system, capped by the
number of selected files. If that query fails, automatic selection falls back to one worker. `1`
processes files serially without a worker pool. A positive value caps simultaneous file tasks at the
smaller of that value and the file count. No workers are needed for an empty source set.

For formatting, a file task includes reading the source, running its rustfmt process, and applying
declaration spacing. For custom rules, it includes reading and inspecting the source. The limit
therefore caps simultaneous rustfmt processes as well as file tasks; it does not cap the total
number of operating-system processes or threads. `check` still finishes the formatting phase before
starting lint, and both phases reuse the same worker pool.

Discovery order determines report and diff order regardless of task completion order. If multiple
file tasks fail operationally, Rusteward finishes the in-flight parallel work and reports the first
failure in discovery order. Formatting must succeed for every file before source writes can begin;
conflict checks and atomic per-file replacements remain sequential.

This setting controls Rusteward's file processing only. It is not passed to Cargo Clippy. Configure
Clippy compilation parallelism with Cargo's `build.jobs` or `CARGO_BUILD_JOBS` independently.
Workflow logs include the effective worker count and phase duration; per-file durations appear in
debug logs (`-vv`).

## Configuration

Settings have the following precedence, from lowest to highest:

1. Built-in defaults.
2. `rusteward/config.toml` in the platform's user configuration directory, typically
   `%APPDATA%\rusteward\config.toml` on Windows.
3. `rusteward.toml` in the Cargo workspace root. `--config PATH` replaces this layer.
4. CLI overrides.

`--no-config` skips user and workspace files, retaining defaults and CLI overrides. Tables merge
recursively across files; scalars and arrays replace the preceding values. CLI `--exclude`,
`--features`, and `--clippy-arg` append to the merged arrays. Unknown Rusteward fields, invalid
severity names or globs, and reversed line-count thresholds are rejected.

`cargo dev config init` creates the annotated [configuration example](../examples/rusteward.toml).
Only specify values you want to override:

```toml
version = 1

[format.rustfmt]
max_width = 110
fn_single_line = false
imports_granularity = "Crate"

[rules]
mod-rs = "error"
inline-tests = "warning"

[rules.lines]
warn = 650
error = 1200
warn-level = "warning"
error-level = "error"
```

### Rustfmt options

`[format.rustfmt]` retains native rustfmt underscore keys and TOML values. Entries merge by key with
the built-in profile and become individual `--config KEY=VALUE` arguments. Booleans, numbers, and
strings are passed directly; arrays are encoded as bracketed lists, with support determined by the
selected rustfmt version.

String values cannot contain commas or newlines because of rustfmt CLI encoding. Tables and dates
are unsupported. Rusteward controls `edition`, `skip_children`, and `emit_mode`, so these cannot be
overridden. `style_edition` is configurable. If rustfmt rejects or warns about an option, formatting
fails explicitly.

`[format].toolchain` selects a rustup toolchain for rustfmt. By default it uses the target project's
active toolchain. `[format].spacing` controls declaration spacing.

### Clippy options

`[lint]` supports `clippy`, `deny-warnings`, `all-targets`, `all-features`, `no-default-features`,
`features`, `toolchain`, `clippy-lints`, `clippy-config`, and `clippy-args`. Defaults enable Clippy
and all targets, keep default features, and leave custom warnings nonfatal. The Clippy toolchain can
be selected independently of rustfmt.

The built-in profile enables `clippy::all` and `clippy::pedantic`, with `clippy::must_use_candidate`
allowed. It also enables these individually selected rules; the `restriction` and `nursery` groups
are not enabled wholesale:

- **Restriction:** `allow_attributes`, `allow_attributes_without_reason`, `as_pointer_underscore`,
  `as_underscore`, `clone_on_ref_ptr`, `dbg_macro`, `exit`, `let_underscore_must_use`,
  `lossy_float_literal`, `map_err_ignore`, `multiple_unsafe_ops_per_block`, `panic_in_result_fn`,
  `precedence_bits`, `suspicious_xor_used_as_pow`, `todo`, `undocumented_unsafe_blocks`,
  `unimplemented`, `unused_result_ok`, and `unwrap_used`.
- **Nursery:** `as_ptr_cast_mut`, `clear_with_drain`, `coerce_container_to_any`,
  `collection_is_never_read`, `debug_assert_with_mut_call`, `derive_partial_eq_without_eq`,
  `fallible_impl_from`, `literal_string_with_formatting_args`, `needless_collect`,
  `path_buf_push_overwrite`, `read_zero_byte_vec`, `redundant_clone`, `search_is_some`,
  `suspicious_operation_groupings`, `trait_duplication_in_bounds`, `type_repetition_in_bounds`,
  `unused_peekable`, `unused_rounding`, and `use_self`.

The authoritative profile is
[`crates/workspace/src/config/clippy-defaults.toml`](../crates/workspace/src/config/clippy-defaults.toml).
Enabled rules have warning levels, and Clippy receives `-D warnings`, so their findings fail the
check. The selected toolchain determines the exact members of `all` and `pedantic`; upgrading Clippy
can introduce new findings.

`unwrap_used` disallows production `unwrap`, including const code. The built-in parameters set
`allow-unwrap-in-tests = true` and `allow-unwrap-in-consts = false`. The test exemption covers test
functions and `#[cfg(test)]` code, including external modules reached through a `#[cfg(test)]`
declaration. Helpers merely located in a `tests/` directory do not automatically receive it.
`expect_used` is not enabled, so `expect` remains available. This profile does not prohibit every
possible source of panic.

`[lint.clippy-lints]` maps unqualified native underscore names to `allow`, `warn`, `deny`, or
`forbid`. Entries merge by key with the built-in profile. Group levels are passed before individual
lint levels, so an individual exception takes precedence over its group. `clippy-args` are appended
last and can override those levels. Local lint attributes also apply; use
`#[expect(clippy::lint_name, reason = "...")]` for an intentional exception that should be reported
when it stops triggering. `forbid` prevents later relaxation.

`[lint.clippy-config]` stores native hyphenated Clippy parameter keys and TOML values, including
arrays and tables. Entries merge with built-in parameters using the normal configuration layering
rules. Clippy validates parameter names and value types for the selected toolchain.

Rusteward writes the effective parameters to a temporary `clippy.toml` and sets `CLIPPY_CONF_DIR`
for its Clippy child process. Standalone `clippy.toml` and `.clippy.toml` files in the target
project, workspace members, and ancestor directories do not participate; an inherited
`CLIPPY_CONF_DIR` is also overridden. Migrate their required parameters into `[lint.clippy-config]`.
This applies one managed parameter profile to the whole checked workspace without changing its
files. `cargo dev config show` exposes that profile, and `--no-config` retains the built-in profile.

For example, allow one naming rule and tighten a Clippy threshold:

```toml
[lint.clippy-lints]
similar_names = "allow"

[lint.clippy-config]
too-many-arguments-threshold = 5
```

Custom rule errors do not prevent Clippy from running. `--skip-clippy` runs only custom checks,
though workspace discovery still uses Cargo metadata. Scan exclusions and generated-file markers do
not exempt code that Cargo compiles from Clippy's checks.

### CLI override examples

```text
cargo dev format --rustfmt-option max_width=110 --rustfmt-option fn_single_line=false
cargo dev format --rustfmt-toolchain stable
cargo dev lint --line-warning 500 --line-error 900
cargo dev lint --rule inline-tests=error --rule mod-rs=warning
cargo dev lint --rule lines=off --exclude "generated/**"
cargo dev check --deny-warnings --all-features
cargo dev lint --features extras,serde --no-default-features
cargo dev lint --all-targets=false --clippy-arg=-A --clippy-arg=clippy::similar_names
```

`--exclude`, `--rule`, `--rustfmt-option`, and `--clippy-arg` can be repeated. `--features` accepts
repeated flags or comma-separated names.

## Diagnostics and CI

| Exit code | Meaning                                                                    |
| --------- | -------------------------------------------------------------------------- |
| `0`       | The operation completed successfully and checks passed.                    |
| `1`       | Formatting, custom policies, or Clippy checks failed.                      |
| `2`       | Configuration, file access, tool startup, or formatting processing failed. |

Formatting, custom policies, Clippy lints, and rustc compilation errors share one diagnostic list.
The summary counts top-level warnings and errors from every source; nested notes and suggestions do
not increase those totals. Counts refer to emitted diagnostics, including findings from different
compilation targets. A failed Clippy process still fails the check even when it produces no compiler
diagnostics. `--deny-warnings` applies to Rusteward rules; Clippy/rustc warning policy follows the
flags passed to Clippy.

### Text output

Human output follows this order: workspace banner, live phase status, diagnostic blocks, optional
captured logs, diffs, and final result. Each phase announces its start immediately and reports its
completion with elapsed time. `check` runs Formatting, Source rules, and Clippy; `format` runs only
Formatting, and `lint` runs Source rules and Clippy. Disabling Clippy reports it as skipped.

A phase reports passed or failed according to its own findings and warning policy. An operational
failure reports error, retains results from completed phases, and marks later phases as not run. The
final result says passed, failed, or aborted, matching exit codes 0, 1, and 2.

Compiler findings use the compiler's complete rendered text in every text mode, including source
excerpts, location markers, notes, and repair suggestions. Rusteward findings use a
`severity[rule]: message` heading, a separate `--> path:line:column` location, and the original
source line when available. Each diagnostic is separated by a blank line. Locationless findings
identify their producer without invented coordinates.

When compiler-rendered text is unavailable, Rusteward presents structured locations, source lines,
and nested notes or help. Multipart edits stay under their owning suggestion; multiline replacements
are printed as actual lines. Applicability metadata remains in JSON. Rusteward does not apply
compiler suggestions.

| Mode               | Behavior                                                                                         |
| ------------------ | ------------------------------------------------------------------------------------------------ |
| Default            | Live phase status, complete diagnostics, and a command-specific summary.                         |
| `--quiet` / `-q`   | Keep complete diagnostics and necessary failure context; hide progress and successful summaries. |
| `--verbose` / `-v` | Add labelled captured Cargo stdout/stderr and workflow logs.                                     |
| `-vv`              | Add per-file debug logs, command arguments, and other execution details.                         |
| `--json`           | Write one complete structured result to stdout; suppress human banners, progress, and summaries. |

Default and quiet output hide captured Cargo logs when compiler errors already explain a failed
Clippy invocation. This keeps Cargo's build progress and trailing failure summaries out of the
diagnostic list. Default output points to `-v` or `--json` for the retained logs. If Clippy fails
without compiler errors, or its output cannot be decoded, every text mode includes labelled captured
output and the available process/build status. Verbose output includes captured logs even after a
successful build.

Summaries omit zero warning, error, and skip counts. `format` reports files actually formatted;
`format --check` and `check` describe differences as files that need formatting. An aborted `format`
does not claim that no files were changed: file writes are atomic individually and a later write
failure may leave earlier changes in place.

Text progress, findings, summaries, and logs go to stderr; unified diffs and configuration results
go to stdout. Data output is flushed before the final summary. `--quiet` and `--verbose` conflict.
`RUST_LOG` explicitly overrides the workflow log filter, including the quiet default; it does not
change diagnostic presentation. Workflow logs omit timestamps; `-vv` includes their module targets.

`--color auto` is the default: human status and diagnostic headings use color only when stderr is a
terminal and `NO_COLOR` is unset or empty. `--color always` forces styling and `--color never`
disables it. Explicit color choices override `NO_COLOR`. Captured Cargo output and compiler data are
collected without color; JSON output remains unstyled regardless of color mode. Help, version
output, and argument-parsing errors use clap's standard presentation.

### JSON output

Every JSON command result includes `schema_version: 1`, `success`, and `exit_code`. Completed
format/lint/check results also include `workspace`, `summary` (warning/error counts), and `report`.
Quiet, verbose, and color flags do not remove information from that JSON result. Workflow logs
remain on stderr. A workflow that aborts adds an `error` string while preserving its partial
`report`, `summary`, and `workspace`. Failures before workflow execution have only the outcome
envelope and `error`.

`report.phases` contains each selected phase in execution order:

| Field        | Meaning                                                                                       |
| ------------ | --------------------------------------------------------------------------------------------- |
| `phase`      | `formatting`, `source_rules`, or `clippy`.                                                    |
| `status`     | `not_run`, `running`, `passed`, `failed`, `skipped`, or `error`.                              |
| `elapsed_ms` | Elapsed milliseconds, or `null` when the phase never started, including intentional skipping. |

`report.changed` is the number of applied changes for a completed `format`, and the number of files
requiring formatting for `format --check` or `check`. The report retains results from completed
phases on abort; counts do not describe incomplete file processing or partially applied writes.

`report.diagnostics` contains objects with these common fields:

| Field                    | Meaning                                                                                                                                            |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `source`                 | `rusteward`, `clippy`, or `rustc`.                                                                                                                 |
| `rule`                   | The Rusteward rule ID, Clippy lint name, or rustc error code; `null` when absent.                                                                  |
| `severity`               | The original diagnostic level, such as `warning`, `error`, `note`, or `help`.                                                                      |
| `message`                | The diagnostic message as data.                                                                                                                    |
| `path`, `line`, `column` | The first primary location, with one-based coordinates; `null` when absent. Absolute workspace paths are shortened relative to the workspace root. |
| `snippet`                | Present when an original source line is available for a Rusteward finding; absent for compiler findings.                                           |
| `compiler`               | Present for Clippy/rustc findings: package ID, Cargo target, all spans, nested children, optional explanation, and rendered text.                  |

Compiler spans retain primary/secondary locations, source excerpts, macro expansion context,
replacement text, and suggestion applicability. Child diagnostics remain nested so multipart repair
suggestions retain their grouping.

`report.clippy` records `success`, the actual process `exit_code`, Cargo's optional `build_success`,
non-protocol stdout in `output`, and Cargo logs in `stderr`. Compiler JSON records are decoded into
diagnostics instead of embedded in an escaped stdout string. Artifact and build-script protocol
records do not appear in the report. Other stdout, including unrelated JSON, is preserved. Malformed
known Cargo records or a successful process with no Cargo build result are operational failures
(exit code 2), with captured output retained in the partial report's Clippy fields.

The phase and snippet fields extend schema version 1. Consumers should read the unified diagnostic
array instead of parsing child logs. `config show` adds `workspace`, `sources`, and `settings` to
the outcome envelope; `config init` adds `path`.

`check` continues to lint after ordinary formatting differences. An operational error, such as a
rustfmt warning or unreadable file, stops the run instead.

Install `cargo-dev` in CI, then run it in the target project:

```sh
cargo dev check --locked
# Enforce a stricter custom-policy warning policy:
cargo dev check --locked --deny-warnings
# Save structured results:
cargo dev check --locked --json > rusteward-report.json
```

Use the same stable Rust version and rustfmt/Clippy components as local development to keep
formatting reproducible. Installation, subprocess behavior, external workspaces, and CI platform
compatibility need manual verification; pure unit tests alone do not establish those outcomes.
Actual file and subprocess parallelism, editor conflict handling, and performance also require
manual QA.

### GitHub Actions installation

The repository's root Action installs Rusteward and adds its binary directory to PATH. It does not
run checks or configure Rust, rustfmt, or Clippy. Prepare the target project's toolchain in your
workflow, then retain your existing commands:

```yaml
- uses: RhNu/Rusteward@main
- run: cargo dev check --locked
```

`uses: RhNu/Rusteward@main` selects the installer code. The `ref` input independently selects the
Rusteward binary's source revision. Pin the Action to a commit for a stable installer while leaving
`ref: main` to receive new tool builds, or pin both when reproducibility is required:

```yaml
- uses: RhNu/Rusteward@main
  with:
    ref: main
    cache: "true"
    wait-timeout: "180"
- run: cargo dev check --locked
```

The producer publishes main commits only. A branch, tag, or pinned SHA must resolve to a commit with
a published `ci-<SHA>` package; selecting another ref does not trigger a build.

| Input          | Default               | Behavior                                                                             |
| -------------- | --------------------- | ------------------------------------------------------------------------------------ |
| `ref`          | `main`                | Source branch, tag, or full 40-character commit SHA. Resolved once per installation. |
| `token`        | `${{ github.token }}` | GitHub API token used to resolve the revision and locate its release.                |
| `cache`        | `true`                | Enable persistent GitHub Actions caching of the installed binary.                    |
| `wait-timeout` | `180`                 | Seconds to wait for the complete release; an integer from `0` through `900`.         |

The supported targets are:

| Runner environment | Rust target                 | Build environment |
| ------------------ | --------------------------- | ----------------- |
| Windows x64        | `x86_64-pc-windows-msvc`    | Windows 2022      |
| Linux x64          | `x86_64-unknown-linux-gnu`  | Ubuntu 22.04      |
| Linux ARM64        | `aarch64-unknown-linux-gnu` | Ubuntu 22.04      |

Linux requires glibc 2.35 or newer. Unsupported operating systems or architectures fail with an
explicit error; Alpine/musl and macOS are not supported.

The Action resolves the requested revision to a full SHA, then checks the runner's local tool cache
and GitHub Actions cache using that SHA and target. `cache: "false"` disables GitHub Actions cache
restore/save; a validated runner-local installation can still be reused. Cache entries are checked
against the manifest and binary hash before use. Invalid entries are treated as misses, and an
unavailable GitHub Actions cache or cache service failure allows installation to continue. A cache
miss downloads the matching archive from the public `ci-<SHA>` prerelease. It verifies the archive's
SHA256 checksum, embedded source and target manifest, and binary hash before installation. A release
is available only after all three platform builds succeed. The Action waits for that complete
release within `wait-timeout`; a missing release, missing platform asset, or failed validation stops
installation. It never selects an older build or compiles from source as a fallback.

| Output      | Meaning                                             |
| ----------- | --------------------------------------------------- |
| `sha`       | Full source commit SHA of the installed tool.       |
| `target`    | Rust target triple selected for the runner.         |
| `cache-hit` | Whether the installation was restored from a cache. |
| `path`      | Installed binary directory added to PATH.           |

Installation logs include the resolved SHA, platform, cache result, and elapsed time. A new main
commit has its own cache entry and may require waiting for the producer workflow. Precompilation
removes Rusteward compilation from the consumer installation step; actual download time, cache
persistence, and hosted runner compatibility require GitHub Actions verification.

In Rusteward's own checkout, `.cargo/config.toml` defines `dev` through `cargo run`, which takes
precedence over the installed executable. Use `cargo-dev check --locked` there to run the installed
binary directly. Other projects can continue using `cargo dev check --locked` unless their own Cargo
alias overrides `dev`.
