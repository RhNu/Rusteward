//! Discover a Cargo workspace and deterministically scan its authored Rust files.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use cargo_metadata::{MetadataCommand, Package};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use rusteward_core::Edition;
use tracing::{debug, info};
use walkdir::{DirEntry, WalkDir};

use crate::{config::ScanSettings, process::CargoOptions};

/// Cargo's resolved project context, including parsing editions and generated output paths.
pub struct Workspace {
    pub root: PathBuf,
    pub manifest: PathBuf,
    pub target: PathBuf,
    pub packages: Vec<Package>,
}

#[derive(Clone, Debug)]
/// A discovered file paired with its owning package's Rust edition.
pub struct Source {
    pub path: PathBuf,
    pub edition: Edition,
}

/// Cargo resolves manifests from the invoking directory, including virtual workspaces.
///
/// # Errors
/// Returns an error when Cargo metadata cannot resolve the requested workspace.
pub fn locate(options: &CargoOptions) -> Result<Workspace> {
    let mut command = MetadataCommand::new();
    command.no_deps();
    if let Some(cargo) = std::env::var_os("CARGO") {
        command.cargo_path(PathBuf::from(cargo));
    }
    if let Some(manifest) = &options.manifest_path {
        command.manifest_path(manifest);
    }
    let mut flags = Vec::new();
    if options.locked {
        flags.push("--locked".into());
    }
    if options.offline {
        flags.push("--offline".into());
    }
    command.other_options(flags);
    let metadata = command.exec().context("cannot locate Cargo workspace")?;
    let root = metadata.workspace_root.into_std_path_buf();
    let packages = metadata
        .packages
        .into_iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .collect();
    info!(root = %root.display(), "located Cargo workspace");
    Ok(Workspace {
        manifest: root.join("Cargo.toml"),
        root,
        target: metadata.target_directory.into_std_path_buf(),
        packages,
    })
}

/// Validate glob syntax before either workflow can launch a child process or write sources.
///
/// # Errors
/// Returns an error when an exclusion pattern or the combined glob set is invalid.
pub fn exclusions(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid exclusion glob {pattern:?}"))?;
        builder.add(glob);
    }
    Ok(builder.build()?)
}

/// Treat directory globs as subtree exclusions as well as matching individual files.
pub fn excluded(path: &Path, directory: bool, root: &Path, globs: &GlobSet) -> bool {
    let relative = pathdiff::diff_paths(path, root).unwrap_or_else(|| path.into());
    let normalized = relative.to_string_lossy().replace('\\', "/");
    globs.is_match(&normalized) || (directory && globs.is_match(format!("{normalized}/")))
}

/// Never follow links; propagate every scan error instead of returning a partial success.
///
/// # Errors
/// Returns an error for invalid exclusions, unsupported package editions, or directory scan failures.
pub fn sources(workspace: &Workspace, settings: &ScanSettings) -> Result<Vec<Source>> {
    let globs = exclusions(&settings.exclude)?;
    let mut package_roots = workspace
        .packages
        .iter()
        .map(|package| {
            let root = package
                .manifest_path
                .parent()
                .context("package manifest has no parent")?
                .as_std_path()
                .to_path_buf();
            let edition = parse_edition(&package.edition.to_string())?;
            Ok((root, edition))
        })
        .collect::<Result<Vec<_>>>()?;
    // The most specific member owns files when packages have nested directories.
    package_roots.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    let mut roots = BTreeSet::from([workspace.root.clone()]);
    roots.extend(
        package_roots
            .iter()
            .filter(|(path, _)| !path.starts_with(&workspace.root))
            .map(|(path, _)| path.clone()),
    );
    let mut files = BTreeMap::new();
    for root in roots {
        let entries = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| descend(entry, &workspace.target, &workspace.root, &globs));
        for entry in entries {
            let entry = entry.with_context(|| format!("cannot scan {}", root.display()))?;
            if entry.file_type().is_symlink() {
                debug!(path = %entry.path().display(), "skipping linked path");
                continue;
            }
            if !entry.file_type().is_file()
                || entry
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "rs")
            {
                continue;
            }
            let edition = package_roots
                .iter()
                .find(|(root, _)| entry.path().starts_with(root))
                .map_or(Edition::Edition2024, |(_, edition)| *edition);
            files.insert(entry.into_path(), edition);
        }
    }
    info!(files = files.len(), "discovered Rust source files");
    Ok(files
        .into_iter()
        .map(|(path, edition)| Source { path, edition })
        .collect())
}

fn descend(entry: &DirEntry, target: &Path, root: &Path, globs: &GlobSet) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let directory = entry.file_type().is_dir();
    let builtin = directory
        && (entry.path() == target
            || matches!(
                entry.file_name().to_str(),
                Some(".git" | "target" | "node_modules" | ".folio")
            ));
    let skip = builtin || excluded(entry.path(), directory, root, globs);
    if skip {
        debug!(path = %entry.path().display(), "excluding source path");
    }
    !skip
}

/// Resolve the source parser edition from Cargo's edition string.
///
/// # Errors
/// Returns an error when the edition is unsupported by Rusteward.
pub fn parse_edition(edition: &str) -> Result<Edition> {
    match edition {
        "2015" => Ok(Edition::Edition2015),
        "2018" => Ok(Edition::Edition2018),
        "2021" => Ok(Edition::Edition2021),
        "2024" => Ok(Edition::Edition2024),
        _ => bail!("unsupported Rust edition {edition}"),
    }
}

#[cfg(test)]
mod tests;
