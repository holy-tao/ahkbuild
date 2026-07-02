//! The per-project link-farm at `<project>/.ahkbuild/modules/`.
//!
//! For each dependency `Name`, a directory junction (Windows) / symlink (Unix) `Name` points at
//! the dependency's tree - its store directory, or the local `path` source, plus any `subdir`. The
//! interpreter and editors resolve `#Import Name` here via `AhkImportPath`. The farm is rebuilt from
//! scratch on every restore, so it always reflects the current manifest + lock.

use std::path::{Path, PathBuf};

use ahkbuild_config::{BuildConfig, DependencySource};
use anyhow::{Context, Result};

use crate::lock::Lockfile;
use crate::store::{self, store_path};

/// `<project>/.ahkbuild/`.
pub fn ahkbuild_dir(project_root: &Path) -> PathBuf {
    project_root.join(".ahkbuild")
}

/// `<project>/.ahkbuild/modules/` - the directory `AhkImportPath` should point at.
pub fn modules_dir(project_root: &Path) -> PathBuf {
    ahkbuild_dir(project_root).join("modules")
}

/// Rebuild the link-farm from the manifest + lock. Clears any stale links first, then creates one
/// junction/symlink per dependency. Also writes `.ahkbuild/.gitignore` so the generated tree is not
/// committed.
pub fn materialize(project_root: &Path, config: &BuildConfig, lock: &Lockfile) -> Result<usize> {
    let ahkbuild = ahkbuild_dir(project_root);
    std::fs::create_dir_all(&ahkbuild)
        .with_context(|| format!("creating {}", ahkbuild.display()))?;
    write_gitignore(&ahkbuild)?;

    let modules = modules_dir(project_root);
    clear_dir(&modules)?;
    std::fs::create_dir_all(&modules).with_context(|| format!("creating {}", modules.display()))?;

    let mut count = 0;
    for (name, spec) in &config.dependencies {
        let base = match &spec.source {
            DependencySource::Path { path } => path.clone(),
            _ => {
                let entry = lock.get(name).ok_or_else(|| {
                    anyhow::anyhow!("dependency {name:?} is missing from the lockfile; run `ahkbuild package restore`")
                })?;
                store_path(entry.content_hash()?)?
            }
        };
        let target = match &spec.subdir {
            Some(sub) => base.join(sub),
            None => base,
        };
        anyhow::ensure!(
            target.is_dir(),
            "dependency {name:?} target {} does not exist",
            target.display()
        );
        // The farm exposes each dependency under its import name (the `alias`, or the key) so
        // `#Import <name>` resolves here via `AhkImportPath`.
        let import_name = spec.import_name(name);
        let link = modules.join(import_name);
        tracing::debug!(name = %import_name, target = %target.display(), "linking dependency");
        make_link(&link, &target)
            .with_context(|| format!("linking {} -> {}", link.display(), target.display()))?;
        count += 1;
    }
    Ok(count)
}

fn write_gitignore(ahkbuild_dir: &Path) -> Result<()> {
    // A `*` inside the generated dir ignores everything within it (including itself), so the store
    // links and gitignore never land in the user's repo.
    let path = ahkbuild_dir.join(".gitignore");
    if !path.exists() {
        std::fs::write(&path, "*\n").with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Remove every child of `dir` without following links into their targets, then remove `dir`.
fn clear_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let md = std::fs::symlink_metadata(&path)?;
        if md.file_type().is_symlink() {
            // A symlink/junction: remove the link itself, never its target.
            remove_link(&path)?;
        } else if md.is_dir() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing {}", path.display()))?;
        } else {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn remove_link(path: &Path) -> Result<()> {
    // Directory junctions and directory symlinks are removed with remove_dir (the reparse point),
    // which does not touch the target.
    std::fs::remove_dir(path)
        .or_else(|_| std::fs::remove_file(path))
        .with_context(|| format!("removing link {}", path.display()))
}

#[cfg(not(windows))]
fn remove_link(path: &Path) -> Result<()> {
    std::fs::remove_file(path)
        .or_else(|_| std::fs::remove_dir(path))
        .with_context(|| format!("removing link {}", path.display()))
}

/// Create a directory link `link` -> `target`, falling back to a recursive copy where the platform
/// forbids unprivileged links.
#[cfg(windows)]
fn make_link(link: &Path, target: &Path) -> Result<()> {
    match junction::create(target, link) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!(error = %e, "junction failed; copying instead");
            store::copy_dir(target, link)
        }
    }
}

#[cfg(not(windows))]
fn make_link(link: &Path, target: &Path) -> Result<()> {
    match std::os::unix::fs::symlink(target, link) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!(error = %e, "symlink failed; copying instead");
            store::copy_dir(target, link)
        }
    }
}
