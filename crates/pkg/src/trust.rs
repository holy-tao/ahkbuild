//! Resolve the project's `ahkbuild.trust.json` entries to on-disk roots.
//!
//! Each [`TrustEntry`] names a package (by manifest key) whose dynamic constructs the author has
//! vetted. This module turns that into concrete canonical filesystem locations the shaker can match
//! a source node against.

use std::path::{Path, PathBuf};

use ahkbuild_config::{BuildConfig, DependencySource, TrustFile};
use anyhow::Result;

use crate::farm::modules_dir;
use crate::lock::Lockfile;
use crate::source::archive_kind;
use crate::store::store_path;

/// Which files within a trusted package are covered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRule {
    /// The whole package is trusted (any file under a resolved root).
    All,
    /// Only these exact (canonical) file paths are trusted.
    Paths(Vec<PathBuf>),
}

/// A resolved trust entry: the canonical on-disk roots a package's files may live under, and which
/// of them are covered. Two roots are recorded per package - the content store directory and the
/// link-farm directory - because a farm that fell back to copying (no symlink privilege) leaves the
/// files under the farm dir instead of resolving into the store.
#[derive(Debug, Clone)]
pub struct ResolvedTrust {
    pub package: String,
    pub roots: Vec<PathBuf>,
    pub files: FileRule,
}

/// Resolve every entry in `file` against `config` + the project lockfile, dropping (with a warning)
/// any entry that is stale, unpinned, unknown, or backed by a `path` source.
pub fn resolve_trust(
    config: &BuildConfig,
    project_root: &Path,
    file: &TrustFile,
) -> Result<Vec<ResolvedTrust>> {
    let lock = Lockfile::load(project_root)?.unwrap_or_default();
    let modules = modules_dir(project_root);

    let mut out = Vec::new();
    for entry in &file.trust {
        let name = entry.package.as_str();
        let Some(spec) = config.dependencies.get(name) else {
            tracing::warn!(
                package = name,
                "trust entry names an unknown dependency; ignoring"
            );
            continue;
        };
        if matches!(spec.source, DependencySource::Path { .. }) {
            tracing::warn!(
                package = name,
                "trust entry targets a `path` dependency; use in-source `;@ahkbuild-safe` instead",
            );
            continue;
        }
        let Some(lock_entry) = lock.get(name) else {
            tracing::warn!(
                package = name,
                "trust entry names a dependency missing from the lockfile; run `ahkbuild package restore`",
            );
            continue;
        };
        if lock_entry.checksum != entry.checksum {
            tracing::warn!(
                package = name,
                vouched = %entry.checksum,
                current = %lock_entry.checksum,
                "trust entry is stale (pinned package changed); re-run `ahkbuild package trust`",
            );
            continue;
        }

        let hash = lock_entry.content_hash()?;
        let store = store_path(hash)?;
        let import_name = spec.import_name(name);

        // A single-file release asset is exposed as one `.ahk` file, not a tree. Its roots are that
        // file (in the store, and its farm link); file-list granularity does not apply.
        if let DependencySource::GithubRelease { asset, .. } = &spec.source {
            if archive_kind(asset).is_none() {
                let roots = vec![
                    canon(store.join(asset)),
                    canon(modules.join(format!("{import_name}.ahk"))),
                ];
                out.push(ResolvedTrust {
                    package: entry.package.clone(),
                    roots,
                    files: FileRule::All,
                });
                continue;
            }
        }

        // Directory dependency: the farm links `modules/<import>` at `store/<subdir>`, so both
        // candidate roots point at the same subtree (a junction resolves to the store; a copy
        // fallback keeps the files under the farm dir).
        let store_root = match &spec.subdir {
            Some(sub) => store.join(sub),
            None => store,
        };
        let roots = vec![canon(store_root), canon(modules.join(import_name))];

        let files = if entry.is_whole_package() {
            FileRule::All
        } else {
            let mut paths = Vec::new();
            for rel in &entry.files {
                let rel = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
                for root in &roots {
                    paths.push(canon(root.join(&rel)));
                }
            }
            FileRule::Paths(paths)
        };

        out.push(ResolvedTrust {
            package: entry.package.clone(),
            roots,
            files,
        });
    }
    Ok(out)
}

/// Canonicalize best-effort: junctions/symlinks resolve to the store, and both sides of a later
/// prefix match go through the same normalization.
fn canon(p: PathBuf) -> PathBuf {
    std::fs::canonicalize(&p).unwrap_or(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::{LockEntry, Lockfile};
    use ahkbuild_config::{TrustEntry, TrustFile};

    /// A one-dependency config (`SomeLib`, a git source) built from a minimal manifest.
    fn config_with_somelib() -> BuildConfig {
        serde_json::from_str(
            r#"{
                "interpreter": { "version": "2.1-alpha.27" },
                "dependencies": { "SomeLib": { "git": "https://example.com/x.git", "tag": "v1" } }
            }"#,
        )
        .unwrap()
    }

    /// Write an `ahkbuild.lock` pinning `SomeLib` to `checksum` under `root`.
    fn write_lock(root: &Path, checksum: &str) {
        Lockfile {
            version: crate::lock::LOCK_VERSION,
            packages: vec![LockEntry {
                name: "SomeLib".into(),
                source: "git+https://example.com/x.git?tag=v1".into(),
                resolved: "abc123".into(),
                checksum: checksum.into(),
            }],
        }
        .save(root)
        .unwrap();
    }

    fn entry(checksum: &str, files: Vec<String>) -> TrustFile {
        TrustFile {
            version: 1,
            trust: vec![TrustEntry {
                package: "SomeLib".into(),
                checksum: checksum.into(),
                files,
                reason: None,
            }],
        }
    }

    #[test]
    fn keeps_a_matching_entry() {
        let tmp = tempfile::tempdir().unwrap();
        write_lock(tmp.path(), "sha256:abc");
        let config = config_with_somelib();

        let resolved = resolve_trust(&config, tmp.path(), &entry("sha256:abc", vec![])).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].package, "SomeLib");
        assert_eq!(resolved[0].files, FileRule::All);
        // Two candidate roots: the store dir and the farm link.
        assert_eq!(resolved[0].roots.len(), 2);
    }

    #[test]
    fn per_file_entry_yields_paths_rule() {
        let tmp = tempfile::tempdir().unwrap();
        write_lock(tmp.path(), "sha256:abc");
        let config = config_with_somelib();

        let tf = entry("sha256:abc", vec!["src/dyn.ahk".into()]);
        let resolved = resolve_trust(&config, tmp.path(), &tf).unwrap();
        match &resolved[0].files {
            // One relative path joined onto each of the two roots.
            FileRule::Paths(p) => assert_eq!(p.len(), 2),
            FileRule::All => panic!("expected a per-file rule"),
        }
    }

    #[test]
    fn drops_a_stale_checksum() {
        let tmp = tempfile::tempdir().unwrap();
        write_lock(tmp.path(), "sha256:abc");
        let config = config_with_somelib();

        // The entry was vouched against a different checksum than the lock now pins.
        let resolved = resolve_trust(&config, tmp.path(), &entry("sha256:OLD", vec![])).unwrap();
        assert!(resolved.is_empty());
    }

    #[test]
    fn drops_unknown_and_unpinned_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let config = config_with_somelib();

        // No lockfile at all -> the package is unpinned, so the entry is skipped.
        let resolved = resolve_trust(&config, tmp.path(), &entry("sha256:abc", vec![])).unwrap();
        assert!(resolved.is_empty());

        // A trust entry naming a dependency the manifest does not declare is skipped too.
        write_lock(tmp.path(), "sha256:abc");
        let mut tf = entry("sha256:abc", vec![]);
        tf.trust[0].package = "NotADep".into();
        assert!(resolve_trust(&config, tmp.path(), &tf).unwrap().is_empty());
    }
}
