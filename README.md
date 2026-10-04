# Rusteward

A personal Rust development toolbox, distributed as `cargo-dev` and invoked through `cargo dev`. It
combines rustfmt with declaration spacing, checks source policies, and runs Clippy for local
development and CI.

Requires Rust with rustfmt and Clippy installed for the target project's toolchain.

## Installation

Install directly from GitHub:

```
cargo install --git https://github.com/RhNu/Rusteward.git --locked rusteward
```

To update an existing installation to the latest repository revision:

```
cargo install --git https://github.com/RhNu/Rusteward.git --locked --force rusteward
```

Alternatively, install from a local checkout's repository root:

```
cargo install --path apps/cli --locked
```

The tool is not currently published on crates.io. To install from another directory, use
`cargo install --path /path/to/rusteward/apps/cli --locked`.

## Quick start

Run these commands in a Cargo project:

```
cargo dev format
cargo dev format --check --diff
cargo dev lint
cargo dev check --locked
```

Use `--quiet` to keep findings while hiding successful summaries, `-v` for full compiler context and
workflow logs, or `--json` for versioned structured results. Output modes and the JSON diagnostic
fields are described in [Diagnostics and CI](docs/features.md#diagnostics-and-ci).

| Command                 | Purpose                                                                           |
| ----------------------- | --------------------------------------------------------------------------------- |
| `format`                | Apply rustfmt and declaration spacing.                                            |
| `format --check --diff` | Show differences from the final formatting result without writing source files.   |
| `lint`                  | Check custom source policies and run the strict built-in Clippy profile.          |
| `check`                 | Check formatting and collect lint results.                                        |
| `config init`           | Create an annotated workspace configuration without overwriting an existing file. |
| `config show`           | Show the effective configuration after layers and CLI overrides.                  |

This repository provides a `dev` Cargo alias, so the same commands work before installation. Other
projects do not need an alias: Cargo discovers `cargo-dev` on PATH or in its bin directory. An
existing `dev` alias takes precedence. Direct invocation, such as `cargo-dev format`, is also
supported.

Use `--manifest-path PATH` to select a project explicitly. Commands operate on its Cargo workspace
even when invoked from a member's subdirectory.

## Documentation

- [Features and configuration](docs/features.md): command behavior, declaration spacing with
  examples, source rules, scanning, configuration, diagnostics, and CI.
- [Code architecture](docs/architecture.md): crate responsibilities, dependencies, workflow
  implementation, and extension boundaries.
- [Configuration example](examples/rusteward.toml): supported settings with comments.

## Development

Format Rust source with the workspace's built-in profile:

```
cargo dev format --no-config
```

Verify pure logic and lint the workspace:

```
cargo test --workspace --lib --bins --locked
cargo dev lint --no-config --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --package rusteward --bin cargo-dev --locked
```

`cargo dev lint --no-config --locked` checks this workspace with Rusteward's own default profile,
including `all`, `pedantic`, and selected `restriction` and `nursery` lints. Production code cannot
use `unwrap`; test functions and `#[cfg(test)]` code can. `expect` remains available. See
[Clippy options](docs/features.md#clippy-options) for the profile and configuration behavior.

Unit tests cover lexical line counting, declaration spacing, source policies, configuration merging,
CLI parsing, command construction, compiler-message decoding, report/output behavior, and diffs.
Installation, external workspaces, subprocess interactions, and CI platform compatibility require
manual verification.

### Markdown formatting

Write documentation in English. Install the pinned local
[Prettier](https://prettier.io/docs/install) dependency once, then format and check Markdown from
the repository root:

```
npm ci
npm run format:docs
npm run check:docs
```

Node.js and npm are needed for documentation formatting. The shared `.prettierrc.json` wraps prose
at 100 columns, uses LF line endings, and preserves code block contents. The scripts target Markdown
and ignore build output and dependencies.

For VS Code, install the recommended
[Prettier extension](https://github.com/prettier/prettier-vscode). The workspace selects it for
Markdown and enables formatting on save. After `npm ci`, the extension uses the same local Prettier
version and configuration as the CLI. Run `npm run format:docs` and `npm run check:docs` after
documentation edits, including edits made outside VS Code.

The declaration-spacing logic originated in Coffer's `apps/xtask`; effective line counting
originated in Folio's `modules/apps/xtask`. Their behavior tests have been retained and extended.
