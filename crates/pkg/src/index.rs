//! A global metadata index for the content-addressed store, at `~/.ahkbuild/packages/index.json`.
//!
//! Store directories are named by opaque content hashes, so on their own they cannot be described or
//! safely garbage-collected. The index records, per hash, the human-facing source/name/size that
//! `package list --global` prints, and the set of project roots that have restored into the store so
//! `package prune` can drop any entry no live lockfile references.
//!
//! References are never trusted from recorded state - `prune`/`list --global` re-derive the live set
//! by reading each known project's `ahkbuild.lock`, so a dropped dependency or deleted project is
//! reflected immediately. The index only supplies the project list and the display metadata.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::lock::Lockfile;
use crate::store::store_root;

/// Current index schema version.
const INDEX_VERSION: u32 = 1;

/// The persisted index: known projects plus per-hash display metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreIndex {
    pub version: u32,
    /// Canonical project roots (each holding an `ahkbuild.json`) that have restored into this store.
    #[serde(default)]
    pub projects: BTreeSet<PathBuf>,
    /// Display metadata keyed by content hash (the store directory name).
    #[serde(default)]
    pub packages: BTreeMap<String, IndexEntry>,
}

impl Default for StoreIndex {
    fn default() -> Self {
        StoreIndex {
            version: INDEX_VERSION,
            projects: BTreeSet::new(),
            packages: BTreeMap::new(),
        }
    }
}

/// Display metadata for one stored tree. Content is deduplicated by hash, so a single entry may have
/// been fetched under several `names`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    /// The manifest source identity (`source_id`) last seen for this content.
    pub source: String,
    /// The resolved revision last seen (a commit SHA, or a URL).
    pub resolved: String,
    /// Import/manifest names this content has been locked under.
    #[serde(default)]
    pub names: BTreeSet<String>,
    /// Size of the stored tree in bytes.
    #[serde(default)]
    pub size: u64,
}

/// One store entry as reported by `package list --global`.
#[derive(Debug, Clone)]
pub struct StorePackage {
    pub hash: String,
    pub names: Vec<String>,
    /// The recorded source identity, or `None` for a store directory the index has no record of.
    pub source: Option<String>,
    pub resolved: Option<String>,
    pub size: u64,
    /// How many known projects currently reference this hash in their lockfile.
    pub refs: usize,
}

/// What a [`prune`] removed.
#[derive(Debug, Clone, Default)]
pub struct PruneReport {
    pub removed: Vec<RemovedEntry>,
    /// Total bytes reclaimed (0 for a dry run, which removes nothing).
    pub freed: u64,
    pub dry_run: bool,
}

/// One store entry a [`prune`] dropped (or would drop, for a dry run).
#[derive(Debug, Clone)]
pub struct RemovedEntry {
    pub hash: String,
    pub names: Vec<String>,
    pub size: u64,
}

impl StoreIndex {
    fn load_from(root: &Path) -> Result<StoreIndex> {
        let path = root.join("index.json");
        if !path.is_file() {
            return Ok(StoreIndex::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        // A corrupt or future-version index should not wedge the tool: start fresh and let the next
        // restore repopulate it (the store itself is the source of truth).
        match serde_json::from_str(&raw) {
            Ok(idx) => Ok(idx),
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "ignoring unreadable store index");
                Ok(StoreIndex::default())
            }
        }
    }

    fn save_to(&self, root: &Path) -> Result<()> {
        std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
        let path = root.join("index.json");
        let mut s = serde_json::to_string_pretty(self).context("serializing store index")?;
        s.push('\n');
        std::fs::write(&path, s).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

/// Record a completed restore/update: register `project_root` and upsert an [`IndexEntry`] for every
/// locked (non-`path`) dependency. Best-effort - callers log and continue on error rather than fail
/// a restore that has already materialized.
pub fn record(project_root: &Path, lock: &Lockfile) -> Result<()> {
    record_in(&store_root()?, project_root, lock)
}

/// List every present store directory, enriched with index metadata and a live reference count.
pub fn list_store() -> Result<Vec<StorePackage>> {
    list_in(&store_root()?)
}

/// Garbage-collect store entries no known project's lockfile references. `current_project`, when
/// given, is registered first so a freshly-restored project is never considered orphaned.
///
/// By default only *tracked* entries (those the index recorded, so their provenance is known) are
/// removed. With `include_untracked`, store directories the index has no record of are swept too -
/// these predate the index or were left by a project that has not restored since, so a project that
/// still needs them but has not been re-restored would have them deleted.
pub fn prune(
    current_project: Option<&Path>,
    dry_run: bool,
    include_untracked: bool,
) -> Result<PruneReport> {
    prune_in(&store_root()?, current_project, dry_run, include_untracked)
}

fn record_in(root: &Path, project_root: &Path, lock: &Lockfile) -> Result<()> {
    // A project with no locked (non-`path`) dependencies references nothing in the store, so it is
    // irrelevant to `list`/`prune`; skip it rather than accumulate empty project rows.
    if lock.packages.is_empty() {
        return Ok(());
    }
    let mut idx = StoreIndex::load_from(root)?;
    idx.projects.insert(canonical(project_root));
    for e in &lock.packages {
        let hash = e.content_hash()?.to_string();
        let dir = root.join(&hash);
        let size = dir_size(&dir).unwrap_or(0);
        let entry = idx.packages.entry(hash).or_insert_with(|| IndexEntry {
            source: e.source.clone(),
            resolved: e.resolved.clone(),
            names: BTreeSet::new(),
            size,
        });
        entry.source = e.source.clone();
        entry.resolved = e.resolved.clone();
        entry.names.insert(e.name.clone());
        entry.size = size;
    }
    idx.save_to(root)
}

fn list_in(root: &Path) -> Result<Vec<StorePackage>> {
    let idx = StoreIndex::load_from(root)?;
    let refs = reference_counts(&idx.projects)?.counts;

    let mut out = Vec::new();
    for hash in hash_dirs(root)? {
        let entry = idx.packages.get(&hash);
        out.push(StorePackage {
            names: entry
                .map(|e| e.names.iter().cloned().collect())
                .unwrap_or_default(),
            source: entry.map(|e| e.source.clone()),
            resolved: entry.map(|e| e.resolved.clone()),
            size: entry
                .map(|e| e.size)
                .filter(|&s| s != 0)
                .unwrap_or_else(|| dir_size(&root.join(&hash)).unwrap_or(0)),
            refs: refs.get(&hash).copied().unwrap_or(0),
            hash,
        });
    }
    // Name-sorted for stable output; unnamed (untracked) entries sink to the bottom by hash.
    out.sort_by(|a, b| (a.names.first(), &a.hash).cmp(&(b.names.first(), &b.hash)));
    Ok(out)
}

fn prune_in(
    root: &Path,
    current_project: Option<&Path>,
    dry_run: bool,
    include_untracked: bool,
) -> Result<PruneReport> {
    let mut idx = StoreIndex::load_from(root)?;
    if let Some(p) = current_project {
        idx.projects.insert(canonical(p));
    }
    let refs = reference_counts(&idx.projects)?;
    let live = &refs.counts;

    // Removal candidates: tracked entries no known project references. With `include_untracked`,
    // also any store directory the index has no record of and that no lockfile pins.
    let mut candidates: Vec<(String, Vec<String>, u64)> = Vec::new();
    for (hash, entry) in &idx.packages {
        if !live.contains_key(hash) && root.join(hash).exists() {
            let size = if entry.size != 0 {
                entry.size
            } else {
                dir_size(&root.join(hash)).unwrap_or(0)
            };
            candidates.push((hash.clone(), entry.names.iter().cloned().collect(), size));
        }
    }
    if include_untracked {
        for hash in hash_dirs(root)? {
            if !idx.packages.contains_key(&hash) && !live.contains_key(&hash) {
                let size = dir_size(&root.join(&hash)).unwrap_or(0);
                candidates.push((hash, Vec::new(), size));
            }
        }
    }

    let mut report = PruneReport {
        dry_run,
        ..Default::default()
    };
    for (hash, names, size) in candidates {
        report.freed += size;
        if !dry_run {
            std::fs::remove_dir_all(root.join(&hash)).ok();
            idx.packages.remove(&hash);
        }
        report.removed.push(RemovedEntry { hash, names, size });
    }

    if !dry_run {
        // Drop dead projects and index rows whose store directory is gone, then persist.
        for dead in &refs.dead {
            idx.projects.remove(dead);
        }
        idx.packages.retain(|hash, _| root.join(hash).exists());
        idx.save_to(root)?;
    }
    Ok(report)
}

/// Live reference counts and dead projects, derived by reading each known project's lockfile.
struct References {
    /// content hash -> number of projects whose lockfile pins it.
    counts: BTreeMap<String, usize>,
    /// Project roots that no longer exist on disk.
    dead: BTreeSet<PathBuf>,
}

fn reference_counts(projects: &BTreeSet<PathBuf>) -> Result<References> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut dead = BTreeSet::new();
    for root in projects {
        match Lockfile::load(root) {
            Ok(Some(lock)) => {
                for e in &lock.packages {
                    if let Ok(h) = e.content_hash() {
                        *counts.entry(h.to_string()).or_default() += 1;
                    }
                }
            }
            // No lockfile (path-only project, or the directory is gone): contributes no references.
            Ok(None) | Err(_) => {
                if !root.exists() {
                    dead.insert(root.clone());
                }
            }
        }
    }
    Ok(References { counts, dead })
}

/// The 64-hex content-hash directory names directly under the store root (skipping `.tmp`, the
/// `index.json` file, and anything not shaped like a hash).
fn hash_dirs(root: &Path) -> Result<Vec<String>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
            out.push(name);
        }
    }
    Ok(out)
}

/// Total size in bytes of every file under `dir` (0 if it does not exist).
fn dir_size(dir: &Path) -> Result<u64> {
    if !dir.exists() {
        return Ok(0);
    }
    let mut total = 0;
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            total += dir_size(&entry.path())?;
        } else if ft.is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

/// Canonicalize a project root for stable identity, stripping Windows' `\\?\` verbatim prefix; falls
/// back to the given path if it cannot be canonicalized (e.g. it no longer exists).
fn canonical(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(c) => strip_verbatim(c),
        Err(_) => p.to_path_buf(),
    }
}

#[cfg(windows)]
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

#[cfg(not(windows))]
fn strip_verbatim(p: PathBuf) -> PathBuf {
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::{LockEntry, LOCK_VERSION};

    /// Create a store directory `<root>/<hash>` holding a file of `size` bytes.
    fn make_store_entry(root: &Path, hash: &str, size: usize) {
        let dir = root.join(hash);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mod.ahk"), vec![b'x'; size]).unwrap();
    }

    fn lock_with(entries: &[(&str, &str)]) -> Lockfile {
        Lockfile {
            version: LOCK_VERSION,
            packages: entries
                .iter()
                .map(|(name, hash)| LockEntry {
                    name: (*name).into(),
                    source: format!("git+{name}"),
                    resolved: "rev".into(),
                    checksum: format!("sha256:{hash}"),
                })
                .collect(),
        }
    }

    // A valid 64-hex hash for a given short label.
    fn hash(seed: char) -> String {
        std::iter::repeat_n(seed, 64).collect()
    }

    #[test]
    fn record_then_list_reports_names_and_refs() {
        let store = tempfile::tempdir().unwrap();
        let proj = tempfile::tempdir().unwrap();
        let (h_a, h_b) = (hash('a'), hash('b'));
        make_store_entry(store.path(), &h_a, 10);
        make_store_entry(store.path(), &h_b, 20);

        let lock = lock_with(&[("Alpha", &h_a), ("Beta", &h_b)]);
        lock.save(proj.path()).unwrap();
        record_in(store.path(), proj.path(), &lock).unwrap();

        let listed = list_in(store.path()).unwrap();
        assert_eq!(listed.len(), 2);
        let alpha = listed.iter().find(|p| p.hash == h_a).unwrap();
        assert_eq!(alpha.names, vec!["Alpha".to_string()]);
        assert_eq!(alpha.refs, 1); // one project references it
        assert_eq!(alpha.source.as_deref(), Some("git+Alpha"));
    }

    #[test]
    fn prune_removes_tracked_entries_a_project_no_longer_references() {
        let store = tempfile::tempdir().unwrap();
        let proj = tempfile::tempdir().unwrap();
        let (h_keep, h_drop) = (hash('a'), hash('b'));
        make_store_entry(store.path(), &h_keep, 10);
        make_store_entry(store.path(), &h_drop, 40);

        // The project first depends on both, so both are tracked in the index...
        let first = lock_with(&[("Alpha", &h_keep), ("Beta", &h_drop)]);
        first.save(proj.path()).unwrap();
        record_in(store.path(), proj.path(), &first).unwrap();
        // ...then drops Beta, leaving h_drop tracked but unreferenced.
        let second = lock_with(&[("Alpha", &h_keep)]);
        second.save(proj.path()).unwrap();

        let report = prune_in(store.path(), None, false, false).unwrap();
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].hash, h_drop);
        assert!(report.freed >= 40);
        assert!(store.path().join(&h_keep).exists(), "referenced entry kept");
        assert!(
            !store.path().join(&h_drop).exists(),
            "dropped entry removed"
        );
    }

    #[test]
    fn default_prune_leaves_untracked_entries_alone() {
        let store = tempfile::tempdir().unwrap();
        let untracked = hash('c');
        make_store_entry(store.path(), &untracked, 40);

        // The index has no record of this directory, so the safe default must not touch it...
        let report = prune_in(store.path(), None, false, false).unwrap();
        assert!(report.removed.is_empty());
        assert!(store.path().join(&untracked).exists());

        // ...but the explicit untracked sweep removes it.
        let report = prune_in(store.path(), None, false, true).unwrap();
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.removed[0].hash, untracked);
        assert!(!store.path().join(&untracked).exists());
    }

    #[test]
    fn prune_dry_run_removes_nothing() {
        let store = tempfile::tempdir().unwrap();
        let untracked = hash('c');
        make_store_entry(store.path(), &untracked, 40);

        let report = prune_in(store.path(), None, true, true).unwrap();
        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.freed, 40);
        assert!(report.dry_run);
        assert!(
            store.path().join(&untracked).exists(),
            "dry run keeps the dir"
        );
    }

    #[test]
    fn prune_registers_the_current_project_so_it_is_not_orphaned() {
        let store = tempfile::tempdir().unwrap();
        let proj = tempfile::tempdir().unwrap();
        let h = hash('a');
        make_store_entry(store.path(), &h, 10);
        // The lock references h and the index records it, but via a project we then "forget" by not
        // passing it as a known project - except as the current project, which keeps it live.
        let lock = lock_with(&[("Alpha", &h)]);
        lock.save(proj.path()).unwrap();
        record_in(store.path(), proj.path(), &lock).unwrap();

        let report = prune_in(store.path(), Some(proj.path()), false, true).unwrap();
        assert!(
            report.removed.is_empty(),
            "current project keeps its entry live"
        );
        assert!(store.path().join(&h).exists());
    }
}
