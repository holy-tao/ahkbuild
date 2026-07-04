//! Module dependency management for ahkbuild: resolve the `dependencies` manifest to a pinned
//! `ahkbuild.lock`, populate a content-addressed store at `~/.ahkbuild/packages/`, and materialize
//! a per-project link-farm at `<project>/.ahkbuild/modules/` that the interpreter and editors
//! resolve `#Import Name` through via `AhkImportPath`.
//!
//! The flow is registry-less: dependencies point directly at sources (git/gist/tarball/path) and
//! the lockfile pins each to an immutable revision + content hash. See the crate-level docs in the
//! design notes for the rationale.

mod farm;
mod fetch;
mod list;
mod lock;
mod source;
mod store;

use std::path::Path;

use ahkbuild_config::{BuildConfig, DependencySource};
use anyhow::{bail, Result};

pub use farm::{ahkbuild_dir, modules_dir};
pub use list::{list, PackageStatus};
pub use lock::{Lockfile, LOCKFILE_NAME, LOCK_VERSION};

use lock::LockEntry;

/// Options controlling a [`restore`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RestoreOptions {
    /// CI mode: fail if the lockfile is missing or would change, instead of updating it. The store
    /// may still be populated from the existing lock (fetching cached revisions is allowed).
    pub locked: bool,
}

/// What a [`restore`] did.
#[derive(Debug, Clone, Copy, Default)]
pub struct RestoreReport {
    /// Dependencies materialized into the link-farm.
    pub restored: usize,
    /// Store entries newly fetched this run.
    pub fetched: usize,
    /// Whether the lockfile changed (and was rewritten, unless `locked`).
    pub lock_changed: bool,
}

/// Resolve, fetch, lock, and materialize every dependency in `config` for the project rooted at
/// `project_root` (the directory containing `ahkbuild.json`).
pub fn restore(
    config: &BuildConfig,
    project_root: &Path,
    opts: RestoreOptions,
) -> Result<RestoreReport> {
    let old = Lockfile::load(project_root)?
        .unwrap_or_default()
        .normalized();
    let mut report = RestoreReport::default();
    let mut entries: Vec<LockEntry> = Vec::new();

    // `dependencies` is a BTreeMap, so iteration (and thus the resulting lock) is name-sorted.
    for (name, spec) in &config.dependencies {
        match &spec.source {
            // Path deps are local and non-reproducible: never locked, linked straight through.
            DependencySource::Path { .. } => {}
            src => {
                let _span = tracing::info_span!("dependency", name = %name).entered();
                let sid = source::source_id(src);
                // Reuse the pinned revision when the manifest source is unchanged.
                let entry = match old.get(name) {
                    Some(e) if e.source == sid => {
                        tracing::debug!(resolved = %e.resolved, "reusing pinned lock entry");
                        e.clone()
                    }
                    _ => {
                        if opts.locked {
                            bail!(
                                "ahkbuild.lock is out of date for {name:?}\n\
                                 hint: run `ahkbuild package restore` and commit the updated lockfile"
                            );
                        }
                        tracing::debug!(source = %sid, "resolving dependency");
                        let fresh = fetch::fetch_fresh(src)?;
                        let hash = store::hash_tree(&fresh.dir)?;
                        store::populate(&hash, &fresh.dir)?;
                        report.fetched += 1;
                        tracing::info!(resolved = %fresh.resolved, checksum = %hash, "fetched dependency");
                        LockEntry {
                            name: name.clone(),
                            source: sid,
                            resolved: fresh.resolved,
                            checksum: format!("sha256:{hash}"),
                        }
                    }
                };

                // Ensure the store holds this revision; repopulate from the pinned id if not.
                let hash = entry.content_hash()?.to_string();
                if !store::store_path(&hash)?.exists() {
                    tracing::info!(resolved = %entry.resolved, "store miss; fetching pinned revision");
                    let staged = fetch::fetch_pinned(src, &entry.resolved)?;
                    let got = store::hash_tree(&staged.dir)?;
                    if got != hash {
                        bail!(
                            "content hash mismatch for {name:?}\n  expected: sha256:{hash}\n  got:      sha256:{got}"
                        );
                    }
                    store::populate(&hash, &staged.dir)?;
                    report.fetched += 1;
                }

                entries.push(entry);
            }
        }
    }

    let new_lock = Lockfile {
        version: LOCK_VERSION,
        packages: entries,
    }
    .normalized();

    report.lock_changed = new_lock != old;
    if opts.locked {
        if report.lock_changed {
            bail!(
                "ahkbuild.lock is out of date\n\
                 hint: run `ahkbuild package restore` and commit the updated lockfile"
            );
        }
    } else if report.lock_changed {
        new_lock.save(project_root)?;
    }

    report.restored = farm::materialize(project_root, config, &new_lock)?;
    Ok(report)
}
