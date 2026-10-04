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

By default, each finding uses `path:line:column: severity[rule]: message`, followed by available
help and replacement suggestions. Diagnostics without a primary source location show their producer
and message without invented coordinates. Suggestions include their replacement ranges and
applicability; Rusteward does not apply them.

| Mode               | Behavior                                                                                           |
| ------------------ | -------------------------------------------------------------------------------------------------- |
| Default            | Compact diagnostics, repair hints, and a combined summary. Successful Cargo build logs are hidden. |
| `--quiet` / `-q`   | Keep findings and failure context; hide successful summaries and default workflow logs.            |
| `--verbose` / `-v` | Show full compiler-rendered diagnostics, captured Cargo logs, and workflow-stage logs.             |
| `-vv`              | Add per-file workflow logs to verbose output.                                                      |
| `--json`           | Write one structured result to stdout, including diagnostics, diffs, and child logs.               |

Text findings, summaries, and logs go to stderr; unified diffs and configuration results go to
stdout. Failed Clippy invocations retain their non-diagnostic output in every text mode so Cargo
failures before compilation remain visible. If the process fails without any diagnostic or captured
text, Rusteward reports its available process/build status. Compiler output is captured without
color. `RUST_LOG` explicitly overrides the workflow log filter, including the default quiet filter;
it does not change the diagnostic presentation mode. Quiet and verbose flags conflict.

### JSON output

Every JSON command result includes `schema_version: 1`, `success`, and `exit_code`. Completed
format/lint/check results also include `workspace`, `summary` (warning/error counts), and `report`.
Quiet and verbose flags do not remove information from that JSON result. Workflow logs remain on
stderr.

`report.diagnostics` contains objects with these common fields:

| Field                    | Meaning                                                                                                                                            |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `source`                 | `rusteward`, `clippy`, or `rustc`.                                                                                                                 |
| `rule`                   | The Rusteward rule ID, Clippy lint name, or rustc error code; `null` when absent.                                                                  |
| `severity`               | The original diagnostic level, such as `warning`, `error`, `note`, or `help`.                                                                      |
| `message`                | The diagnostic message as data.                                                                                                                    |
| `path`, `line`, `column` | The first primary location, with one-based coordinates; `null` when absent. Absolute workspace paths are shortened relative to the workspace root. |
| `compiler`               | Present for Clippy/rustc findings: package ID, Cargo target, all spans, nested children, optional explanation, and rendered text.                  |

Compiler spans retain primary/secondary locations, source excerpts, macro expansion context,
replacement text, and suggestion applicability. Child diagnostics remain nested so multipart repair
suggestions retain their grouping.

`report.clippy` records `success`, the actual process `exit_code`, Cargo's optional `build_success`,
non-protocol stdout in `output`, and Cargo logs in `stderr`. Compiler JSON records are decoded into
diagnostics instead of embedded in an escaped stdout string. Artifact and build-script protocol
records do not appear in the report. Other stdout, including unrelated JSON, is preserved. Malformed
known Cargo records or a successful process with no Cargo build result are operational failures
(exit code 2), with captured output included in the error context.

This schema replaces the earlier unversioned output: consumers should read the unified diagnostic
array instead of parsing `report.clippy.stdout`. `config show` adds `workspace`, `sources`, and
`settings` to the outcome envelope; `config init` adds `path`. Operational failures add an `error`
string. Help, version output, and argument-parsing errors use the standard CLI text presentation.

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
