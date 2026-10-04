# Rusteward

- `crates/core` contains pure source transformations and rules. It must not read files, run
  processes, discover projects, or render terminal output.
- `crates/workspace` owns configuration layers, Cargo metadata, scanning, process execution, and
  workflow reports. `apps/cli` owns argument parsing, logging setup, and presentation.
- The authoritative built-in rustfmt profile is `crates/workspace/src/config/rustfmt-defaults.toml`.
  Pass its values through rustfmt CLI overrides on stable; do not introduce a nightly requirement or
  duplicate the profile in Cargo aliases.
- The authoritative built-in Clippy profile is `crates/workspace/src/config/clippy-defaults.toml`.
  Keep lint levels separate from native Clippy parameters, and do not duplicate the rule catalogue
  in workspace manifests or Cargo aliases.
- Use `cargo dev format --no-config` to format this workspace. Unit verification is
  `cargo test --workspace --lib --bins --locked`; strict-profile self-verification is
  `cargo dev lint --no-config --locked`. Build/lint verification also includes
  `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- Write all documentation in English. `README.md` covers installation, quick start, and contributor
  commands; `docs/features.md` owns user-facing behavior and configuration; `docs/architecture.md`
  owns code architecture and extension boundaries. Keep examples consistent with the supported
  settings.
- After every Markdown documentation edit, run `npm run format:docs`, then `npm run check:docs` from
  the workspace root before finishing. Run `npm ci` first when the pinned local Prettier dependency
  is not installed. CLI and VS Code Markdown formatting use the same local version and
  `.prettierrc.json`; do not rely on an unpinned global formatter.
