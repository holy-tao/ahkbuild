//! The content-addressed package store at `~/.ahkbuild/packages/<sha256>/`. Keyed by a
//! deterministic hash of the fetched tree, so identical content dedupes across projects and
//! versions.

use anyhow::{Context, Result};
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
    rels.sort();

    let mut hasher = Sha256::new();
    for rel in &rels {
        let full = dir.join(rel);
        let bytes = std::fs::read(&full).with_context(|| format!("reading {}", full.display()))?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        hasher.update(rel_str.as_bytes());
        hasher.update([0u8]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hex(&hasher.finalize()))
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
