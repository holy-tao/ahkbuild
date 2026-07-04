//! The `ahkbuild.lock` file: the pinned, immutable identity of every non-`path` dependency. It,
//! not the manifest, is the reproducibility guarantee - `package restore` reads it to fetch exact
//! revisions and verify their content.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The lockfile name, written beside `ahkbuild.json`.
pub const LOCKFILE_NAME: &str = "ahkbuild.lock";

/// Current lockfile schema version.
pub const LOCK_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lockfile {
    pub version: u32,
    #[serde(default, rename = "package")]
    pub packages: Vec<LockEntry>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Lockfile {
            version: LOCK_VERSION,
            packages: Vec::new(),
        }
    }
}

/// One pinned dependency. `source` is the manifest source identity (see [`crate::source::source_id`],
/// excludes `subdir`); `resolved` is the immutable revision (git/gist commit SHA, or the tarball
/// URL); `checksum` is `sha256:<hex>` over the fetched tree and also names the store directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockEntry {
    pub name: String,
    pub source: String,
    pub resolved: String,
    pub checksum: String,
}

impl LockEntry {
    /// The bare content hash (the `sha256:` prefix stripped), which names the store directory.
    pub fn content_hash(&self) -> Result<&str> {
        self.checksum
            .strip_prefix("sha256:")
            .with_context(|| format!("lock entry {:?} has a malformed checksum", self.name))
    }
}

impl Lockfile {
    /// Load `ahkbuild.lock` from `root`, or `None` if it does not exist.
    pub fn load(root: &Path) -> Result<Option<Lockfile>> {
        let path = root.join(LOCKFILE_NAME);
        if !path.is_file() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let lf: Lockfile =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(lf))
    }

    /// Write `ahkbuild.lock` to `root` (pretty JSON, trailing newline).
    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(LOCKFILE_NAME);
        let mut s = serde_json::to_string_pretty(self).context("serializing lockfile")?;
        s.push('\n');
        std::fs::write(&path, s).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&LockEntry> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// A normalized copy (version pinned, packages sorted by name) for stable equality/diffing.
    pub fn normalized(&self) -> Lockfile {
        let mut packages = self.packages.clone();
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Lockfile {
            version: LOCK_VERSION,
            packages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let lf = Lockfile {
            version: LOCK_VERSION,
            packages: vec![LockEntry {
                name: "Widget".into(),
                source: "git+u?tag=v1".into(),
                resolved: "abc123".into(),
                checksum: "sha256:deadbeef".into(),
            }],
        };
        lf.save(tmp.path()).unwrap();
        let back = Lockfile::load(tmp.path()).unwrap().unwrap();
        assert_eq!(lf, back);
        assert_eq!(back.packages[0].content_hash().unwrap(), "deadbeef");
    }

    #[test]
    fn missing_lockfile_loads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(Lockfile::load(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn normalized_sorts_and_pins_version() {
        let lf = Lockfile {
            version: 0,
            packages: vec![
                LockEntry {
                    name: "b".into(),
                    source: "s".into(),
                    resolved: "r".into(),
                    checksum: "sha256:1".into(),
                },
                LockEntry {
                    name: "a".into(),
                    source: "s".into(),
                    resolved: "r".into(),
                    checksum: "sha256:2".into(),
                },
            ],
        }
        .normalized();
        assert_eq!(lf.version, LOCK_VERSION);
        assert_eq!(lf.packages[0].name, "a");
    }
}
