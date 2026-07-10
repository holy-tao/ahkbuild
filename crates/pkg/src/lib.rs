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
mod index;
mod list;
mod lock;
mod source;
mod store;
mod trust;

use std::collections::BTreeSet;
use std::path::Path;

use ahkbuild_config::{BuildConfig, DependencySource};
use anyhow::{bail, Context, Result};

pub use farm::{ahkbuild_dir, modules_dir};
pub use index::{list_store, prune, PruneReport, RemovedEntry, StorePackage};
pub use list::{list, PackageStatus};
pub use lock::{Lockfile, LOCKFILE_NAME, LOCK_VERSION};
pub use trust::{resolve_trust, FileRule, ResolvedTrust};

use lock::LockEntry;

/// A dependency that failed [`verify`], with a human-readable reason.
#[derive(Debug, Clone)]
pub struct VerifyIssue {
    pub name: String,
    pub problem: String,
}

/// What a [`verify`] found.
#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    /// Dependencies whose store contents matched the lock (plus `path` deps that exist on disk).
    pub verified: usize,
    /// Dependencies that are missing, unpinned, or whose store contents drifted from the lock.
    pub issues: Vec<VerifyIssue>,
}

impl VerifyReport {
    /// Whether every dependency checked out.
    pub fn ok(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Check, offline, that every dependency is pinned and its store contents still hash to the lock's
/// checksum. Never fetches - use `restore` to repair anything this flags. `path` deps only need to
/// exist on disk.
pub fn verify(config: &BuildConfig, project_root: &Path) -> Result<VerifyReport> {
    let lock = Lockfile::load(project_root)?.unwrap_or_default();
    let mut report = VerifyReport::default();

    for (name, spec) in &config.dependencies {
        let issue = |problem: String| VerifyIssue {
            name: name.clone(),
            problem,
        };
        match &spec.source {
            DependencySource::Path { path } => {
                if path.exists() {
                    report.verified += 1;
                } else {
                    report.issues.push(issue(format!(
                        "path source {} does not exist",
                        path.display()
                    )));
                }
            }
            src => {
                let sid = source::source_id(src);
                let entry = match lock.get(name) {
                    Some(e) if e.source == sid => e,
                    Some(_) => {
                        report.issues.push(issue(
                            "lockfile is out of date (manifest source changed); run \
                             `ahkbuild package restore`"
                                .into(),
                        ));
                        continue;
                    }
                    None => {
                        report.issues.push(issue(
                            "not present in the lockfile; run `ahkbuild package restore`".into(),
                        ));
                        continue;
                    }
                };
                let hash = entry.content_hash()?;
                let dir = store::store_path(hash)?;
                if !dir.exists() {
                    report.issues.push(issue(
                        "missing from the store; run `ahkbuild package restore`".into(),
                    ));
                    continue;
                }
                let got = store::hash_tree(&dir)?;
                if got != hash {
                    report.issues.push(issue(format!(
                        "store contents do not match the lock checksum\n    expected sha256:{hash}\n    \
                         got      sha256:{got}"
                    )));
                    continue;
                }
                report.verified += 1;
            }
        }
    }
    Ok(report)
}

/// Forget removed dependencies: drop their lockfile entries and unlink them from the farm. `deps` is
/// `(manifest name, import name)` pairs, captured before the manifest edit. Used by `package remove`.
pub fn forget(project_root: &Path, deps: &[(String, String)]) -> Result<()> {
    if let Some(mut lock) = Lockfile::load(project_root)? {
        let names: BTreeSet<&str> = deps.iter().map(|(n, _)| n.as_str()).collect();
        let before = lock.packages.len();
        lock.packages.retain(|e| !names.contains(e.name.as_str()));
        if lock.packages.len() != before {
            if lock.packages.is_empty() {
                // Preserve the invariant that a project with nothing pinned has no lockfile.
                std::fs::remove_file(project_root.join(LOCKFILE_NAME)).ok();
            } else {
                lock.normalized().save(project_root)?;
            }
        }
    }
    for (_, import_name) in deps {
        farm::unlink(project_root, import_name)?;
    }
    Ok(())
}

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

/// What a [`update`] did. Each re-resolved dependency reports its old and new pinned revision.
#[derive(Debug, Clone, Default)]
pub struct UpdateReport {
    /// Dependencies re-resolved against their manifest selector (may or may not have moved).
    pub changes: Vec<PackageChange>,
    /// Dependencies named on the command line that cannot be updated (pinned by the manifest, or a
    /// `path` source). Empty when updating everything.
    pub skipped: Vec<String>,
}

/// A single dependency's revision change from an [`update`].
#[derive(Debug, Clone)]
pub struct PackageChange {
    pub name: String,
    /// The previously pinned revision, or `None` if it was not yet locked.
    pub from: Option<String>,
    /// The freshly resolved revision.
    pub to: String,
    /// Whether `to` differs from `from`.
    pub moved: bool,
}

/// How the driver decides whether to reuse an existing lock entry or re-resolve it.
enum Refresh {
    /// Reuse a pinned entry whenever the manifest source is unchanged (normal restore).
    Reuse,
    /// Additionally force re-resolution of these dependency names (used by `update`).
    Force(BTreeSet<String>),
}

impl Refresh {
    fn forces(&self, name: &str) -> bool {
        match self {
            Refresh::Reuse => false,
            Refresh::Force(names) => names.contains(name),
        }
    }
}

/// The old and new lockfiles from a driver run, plus the restore-shaped report.
struct Driven {
    old: Lockfile,
    new: Lockfile,
    report: RestoreReport,
}

/// Resolve, fetch, lock, and materialize every dependency in `config` for the project rooted at
/// `project_root` (the directory containing `ahkbuild.json`).
pub fn restore(
    config: &BuildConfig,
    project_root: &Path,
    opts: RestoreOptions,
) -> Result<RestoreReport> {
    Ok(drive(config, project_root, &Refresh::Reuse, opts.locked)?.report)
}

/// Re-resolve the floating (git/gist) dependencies in `names` (or all of them when `names` is empty)
/// to their current remote revision, rewrite the lock, and rebuild the farm. Manifest-pinned sources
/// (`rev`, `tarball`, `release`) and `path` sources cannot move; when named explicitly they are
/// reported as `skipped`, and otherwise silently left in place.
pub fn update(config: &BuildConfig, project_root: &Path, names: &[String]) -> Result<UpdateReport> {
    for n in names {
        if !config.dependencies.contains_key(n) {
            bail!("no dependency named {n:?} in ahkbuild.json");
        }
    }

    let select_all = names.is_empty();
    let mut force = BTreeSet::new();
    let mut skipped = Vec::new();
    for (name, spec) in &config.dependencies {
        if !select_all && !names.iter().any(|n| n == name) {
            continue;
        }
        if source::is_updatable(&spec.source) {
            force.insert(name.clone());
        } else if !select_all {
            // Explicitly named but pinned by the manifest (or a path dep): report, don't touch.
            skipped.push(name.clone());
        }
    }

    let driven = drive(config, project_root, &Refresh::Force(force.clone()), false)?;

    let mut changes = Vec::new();
    for name in &force {
        // Every forced name is a non-path dep, so it is present in the new lock.
        if let Some(to) = driven.new.get(name).map(|e| e.resolved.clone()) {
            let from = driven.old.get(name).map(|e| e.resolved.clone());
            let moved = from.as_deref() != Some(to.as_str());
            changes.push(PackageChange {
                name: name.clone(),
                from,
                to,
                moved,
            });
        }
    }
    Ok(UpdateReport { changes, skipped })
}

/// The shared resolve/lock/materialize driver behind [`restore`] and [`update`].
fn drive(
    config: &BuildConfig,
    project_root: &Path,
    refresh: &Refresh,
    locked: bool,
) -> Result<Driven> {
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
                // Reuse the pinned revision when the manifest source is unchanged and this name is
                // not being force-refreshed.
                let entry = match old.get(name) {
                    Some(e) if e.source == sid && !refresh.forces(name) => {
                        tracing::debug!(resolved = %e.resolved, "reusing pinned lock entry");
                        e.clone()
                    }
                    _ => {
                        if locked {
                            bail!(
                                "ahkbuild.lock is out of date for {name:?}\n\
                                 hint: run `ahkbuild package restore` and commit the updated lockfile"
                            );
                        }
                        tracing::debug!(source = %sid, "resolving dependency");
                        let fresh = fetch::fetch_fresh(src)?;
                        let hash = store::hash_tree(&fresh.dir)?;
                        let final_dir = store::populate(&hash, &fresh.dir)?;
                        seal_store(&hash, &final_dir);
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

                // Ensure the store holds this revision with matching contents; re-fetch the pinned
                // id if it is missing or has drifted from the lock checksum.
                ensure_store(
                    src,
                    name,
                    entry.content_hash()?,
                    &entry.resolved,
                    &mut report,
                )?;

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
    if locked {
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

    // Record this project and its store entries in the global index for `list --global` / `prune`.
    // Best-effort: the restore has already succeeded, so an index write failure only warns.
    if let Err(e) = index::record(project_root, &new_lock) {
        tracing::warn!(error = %e, "failed to update the store index");
    }

    Ok(Driven {
        old,
        new: new_lock,
        report,
    })
}

/// Ensure the content-addressed store holds `hash` with contents that hash back to it, re-fetching
/// the pinned `resolved` revision of `src` when the entry is missing or has drifted from the lock.
/// `name` is used only for diagnostics; `report.fetched` is bumped whenever a fetch was needed.
fn ensure_store(
    src: &DependencySource,
    name: &str,
    hash: &str,
    resolved: &str,
    report: &mut RestoreReport,
) -> Result<()> {
    let dir = store::store_path(hash)?;
    if dir.exists() {
        // Fast path: the store is content-addressed and immutable, so if the tree's cheap metadata
        // fingerprint (file count, total size, newest mtime) still matches the seal recorded when we
        // last confirmed it hashes to `hash`, trust it without re-reading and re-hashing every file.
        let stat = store::stat_tree(&dir)?;
        if index::seal_matches(hash, &stat)? {
            tracing::debug!(%hash, "store seal fresh; skipping full re-hash");
            return Ok(());
        }
        // No seal yet, or the metadata moved: fall back to the authoritative content hash, and reseal
        // on a match so subsequent restores are fast again.
        if store::hash_tree(&dir)? == hash {
            seal_store(hash, &dir);
            return Ok(());
        }
        // Contents drifted from the pinned checksum. `store::populate` treats an existing store dir
        // as authoritative, so the corrupt copy must be removed before the good one can replace it.
        tracing::info!(%resolved, "store contents drifted from the lock; refetching pinned revision");
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("removing drifted store entry {}", dir.display()))?;
    } else {
        tracing::info!(%resolved, "store miss; fetching pinned revision");
    }

    let staged = fetch::fetch_pinned(src, resolved)?;
    let got = store::hash_tree(&staged.dir)?;
    if got != hash {
        bail!(
            "content hash mismatch for {name:?}\n  expected: sha256:{hash}\n  got:      sha256:{got}"
        );
    }
    let final_dir = store::populate(hash, &staged.dir)?;
    seal_store(hash, &final_dir);
    report.fetched += 1;
    Ok(())
}

/// Record the seal (metadata fingerprint) for a freshly hashed/populated store directory so later
/// restores can skip the full re-hash. Best-effort: the store stays authoritative, so a failed seal
/// write only means the next restore re-hashes and reseals.
fn seal_store(hash: &str, dir: &Path) {
    match store::stat_tree(dir) {
        Ok(stat) => {
            if let Err(e) = index::write_seal(hash, &stat) {
                tracing::debug!(error = %e, "could not record store seal");
            }
        }
        Err(e) => tracing::debug!(error = %e, "could not stat store tree to seal it"),
    }
}
