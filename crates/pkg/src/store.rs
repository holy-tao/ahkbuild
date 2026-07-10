//! The content-addressed package store at `~/.ahkbuild/packages/<sha256>/`. Keyed by a
//! deterministic hash of the fetched tree, so identical content dedupes across projects and
//! versions.

use anyhow::{Context, Result};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// `~/.ahkbuild/packages/`.
pub fn store_root() -> Result<PathBuf> {
    Ok(ahkbuild_interpret::ahkbuild_root()?.join("packages"))
}

/// The store directory for a given content hash (hex, no `sha256:` prefix).
pub fn store_path(content_hash: &str) -> Result<PathBuf> {
    Ok(store_root()?.join(content_hash))
}

/// A scratch directory for staging a fetch before it is content-hashed and moved into the store.
pub fn fresh_temp() -> Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = store_root()?
        .join(".tmp")
        .join(format!("{}-{}-{}", std::process::id(), nanos, n));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).ok();
    }
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating temp dir {}", dir.display()))?;
    Ok(dir)
}

/// Hash a directory tree deterministically. Files are visited in sorted relative-path order; each
/// contributes its normalized relative path, byte length, and bytes. Returns lowercase hex sha256.
/// A `.git` directory is skipped so a git checkout hashes the same as the equivalent tarball.
pub fn hash_tree(dir: &Path) -> Result<String> {
    let mut rels = Vec::new();
    collect_files(dir, PathBuf::new(), &mut rels)?;
    tracing::trace!("Hashing {} files in {}", rels.len(), dir.display());

    rels.sort();

    // Reads dominate the cost (tens of thousands of file opens, especially on Windows), while the
    // SHA feed order must stay fixed to keep the hash stable. So parallelize the reads but update the
    // hasher sequentially in sorted order. Files are read one bounded chunk at a time, keeping peak
    // memory proportional to `CHUNK` rather than the whole tree.
    const CHUNK: usize = 512;
    let mut progress = Progress::new(rels.len());
    let mut hasher = Sha256::new();
    for chunk in rels.chunks(CHUNK) {
        let datas: Vec<Result<Vec<u8>>> = chunk
            .par_iter()
            .map(|rel| {
                let full = dir.join(rel);
                std::fs::read(&full).with_context(|| format!("reading {}", full.display()))
            })
            .collect();
        for (rel, data) in chunk.iter().zip(datas) {
            let bytes = data?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            hasher.update(rel_str.as_bytes());
            hasher.update([0u8]);
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
        progress.advance(chunk.len());
    }
    progress.finish();
    Ok(hex(&hasher.finalize()))
}

/// A throttled, single-line `% done` indicator for the potentially minutes-long tree hash. It writes
/// to stderr only when attached to a terminal (so pipes and log capture stay clean) and only for
/// trees large enough that the wait is noticeable - small packages hash instantly and stay silent.
struct Progress {
    total: usize,
    done: usize,
    enabled: bool,
    last: std::time::Instant,
}

impl Progress {
    fn new(total: usize) -> Self {
        use std::io::IsTerminal;
        // Below this the hash is sub-second; a progress line would just flicker.
        let enabled = total >= 4000 && std::io::stderr().is_terminal();
        Progress {
            total,
            done: 0,
            enabled,
            last: std::time::Instant::now(),
        }
    }

    fn advance(&mut self, n: usize) {
        self.done += n;
        if !self.enabled {
            return;
        }
        // Cap redraws at ~10/s so the terminal is not spammed on fast trees.
        if self.done < self.total && self.last.elapsed() < std::time::Duration::from_millis(100) {
            return;
        }
        self.last = std::time::Instant::now();
        let pct = self.done * 100 / self.total.max(1);
        // `\x1b[2K` clears the line, `\r` returns to column 0; no newline so the line updates in place.
        eprint!("\r\x1b[2K  hashing {} files… {pct:>3}%", self.total);
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }

    fn finish(&self) {
        if self.enabled {
            eprint!("\r\x1b[2K");
            let _ = std::io::Write::flush(&mut std::io::stderr());
        }
    }
}

/// A cheap metadata fingerprint of a tree: file count, total byte size, and the newest file mtime.
/// It reads only directory metadata - never file contents - so it stays fast on very large trees, and
/// is used to decide whether a store directory has changed since it was last confirmed to match its
/// content hash. Recorded per hash as the "seal" fields of an index entry; see [`crate::index`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeStat {
    pub files: u64,
    pub total_size: u64,
    /// Newest file mtime across the tree, in nanoseconds since the Unix epoch (0 if unavailable).
    pub max_mtime_nanos: u64,
}

/// Compute a [`TreeStat`] for `dir` from directory metadata alone (no file reads). Skips a top-level
/// `.git`, matching [`hash_tree`], so a sealed checkout and its tarball equivalent fingerprint alike.
pub fn stat_tree(dir: &Path) -> Result<TreeStat> {
    let mut stat = TreeStat::default();
    stat_into(dir, true, &mut stat)?;
    Ok(stat)
}

fn stat_into(dir: &Path, is_root: bool, stat: &mut TreeStat) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        if is_root && entry.file_name() == ".git" {
            continue;
        }
        let ft = entry.file_type()?;
        if ft.is_dir() {
            stat_into(&entry.path(), false, stat)?;
        } else {
            let md = entry.metadata()?;
            stat.files += 1;
            stat.total_size += md.len();
            if let Ok(mtime) = md.modified() {
                let nanos = mtime
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                stat.max_mtime_nanos = stat.max_mtime_nanos.max(nanos);
            }
        }
    }
    Ok(())
}

fn collect_files(base: &Path, rel: PathBuf, out: &mut Vec<PathBuf>) -> Result<()> {
    let dir = base.join(&rel);
    for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        if rel.as_os_str().is_empty() && name == ".git" {
            continue;
        }
        let child_rel = rel.join(&name);
        if entry.file_type()?.is_dir() {
            collect_files(base, child_rel, out)?;
        } else {
            out.push(child_rel);
        }
    }
    Ok(())
}

/// Move a staged tree into the store under `content_hash`. Idempotent: if the entry already exists,
/// the staged copy is discarded. Returns the final store path.
pub fn populate(content_hash: &str, staged: &Path) -> Result<PathBuf> {
    tracing::trace!("Moving staged tree at {} to store", staged.display());
    let dest = store_path(content_hash)?;
    if dest.exists() {
        std::fs::remove_dir_all(staged).ok();
        return Ok(dest);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // Prefer an atomic rename; fall back to copy for cross-volume temp dirs.
    if std::fs::rename(staged, &dest).is_err() {
        copy_dir(staged, &dest)?;
        std::fs::remove_dir_all(staged).ok();
    }
    Ok(dest)
}

/// Recursively copy `from` into `to` (used as a fallback when renaming across volumes, and by the
/// link-farm's copy fallback).
pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
    for entry in std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)
                .with_context(|| format!("copying {} -> {}", src.display(), dst.display()))?;
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-answer test pinning the on-disk hash format. If this value changes, existing lockfiles
    /// and store entries are invalidated - treat a mismatch as an intentional, breaking format change.
    #[test]
    fn hash_tree_is_stable_across_chunk_boundaries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Span more than one read chunk (CHUNK = 512) and include a nested dir so path order and the
        // chunked parallel reads are both exercised.
        std::fs::create_dir(root.join("sub")).unwrap();
        for i in 0..600 {
            let dir = if i % 2 == 0 {
                root.to_path_buf()
            } else {
                root.join("sub")
            };
            std::fs::write(dir.join(format!("f{i:04}.txt")), format!("contents {i}")).unwrap();
        }
        // A `.git` dir must not affect the hash.
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main").unwrap();

        let h1 = hash_tree(root).unwrap();
        let h2 = hash_tree(root).unwrap();
        assert_eq!(h1, h2, "hashing must be deterministic");
        assert_eq!(
            h1, "a43f4c867887c5b6fb810ff6f8b8917fa4938d268abdd0bf053f27c21d18aad7",
            "on-disk hash format changed; see the doc comment on this test"
        );
    }

    #[test]
    fn stat_tree_counts_files_and_bytes_and_skips_git() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), "aaa").unwrap(); // 3 bytes
        std::fs::write(root.join("sub").join("b.txt"), "bbbb").unwrap(); // 4 bytes
                                                                         // A top-level `.git` is ignored, exactly as `hash_tree` ignores it.
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".git").join("HEAD"), "ignored").unwrap();

        let stat = stat_tree(root).unwrap();
        assert_eq!(stat.files, 2);
        assert_eq!(stat.total_size, 7);

        // Adding a file changes the fingerprint, so a seal built from the old stat no longer matches.
        std::fs::write(root.join("c.txt"), "c").unwrap();
        let after = stat_tree(root).unwrap();
        assert_ne!(after, stat, "a new file must change the fingerprint");
        assert_eq!(after.files, 3);
        assert_eq!(after.total_size, 8);
    }
}
