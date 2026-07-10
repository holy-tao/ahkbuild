//! The `ahkbuild.trust.json` file: out-of-band trust for immutable packages.
//!
//! The static-analysis passes (tree-shaking / member-pruning) conservatively "blow up" a module
//! that contains a dynamic construct (`%deref%`, dynamic member access / call). First-party code
//! can vouch for such a construct in-source with a `;@ahkbuild-safe` directive, but packages live
//! in an immutable, content-addressed store and cannot be annotated without forking them. This file
//! is the out-of-source equivalent: it names package files (or whole packages) whose dynamic
//! constructs the project author has vetted as safe.
//!
//! It lives at the project root beside `ahkbuild.json` / `ahkbuild.lock` and is committed. Each
//! entry records the package `checksum` it was vouched against, so a trust entry is silently
//! invalidated when the pinned bytes change (an upgrade forces re-vouching). Resolution to on-disk
//! paths lives in `crates/pkg`; this module only defines and (de)serializes the file.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The trust file name, written beside `ahkbuild.json`.
pub const TRUST_NAME: &str = "ahkbuild.trust.json";

/// Current trust-file schema version.
pub const TRUST_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustFile {
    pub version: u32,
    #[serde(default)]
    pub trust: Vec<TrustEntry>,
}

impl Default for TrustFile {
    fn default() -> Self {
        TrustFile {
            version: TRUST_VERSION,
            trust: Vec::new(),
        }
    }
}

/// One vouched-for package (or a set of files within it). `package` is the manifest dependency key
/// (matching `BuildConfig::dependencies` and the lockfile's `LockEntry::name`); `checksum` is the
/// `sha256:<hex>` the entry was vouched against - resolution ignores the entry if the pinned
/// package's current checksum differs. `files` are package-relative paths (POSIX-style); an empty
/// list or `["*"]` trusts the whole package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrustEntry {
    pub package: String,
    pub checksum: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl TrustEntry {
    /// Whether this entry trusts the whole package rather than a specific list of files.
    pub fn is_whole_package(&self) -> bool {
        self.files.is_empty() || self.files.iter().any(|f| f == "*")
    }
}

impl TrustFile {
    /// Load `ahkbuild.trust.json` from `root`, or `None` if it does not exist.
    pub fn load(root: &Path) -> Result<Option<TrustFile>> {
        let path = root.join(TRUST_NAME);
        if !path.is_file() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let tf: TrustFile =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(tf))
    }

    /// Write `ahkbuild.trust.json` to `root` (pretty JSON, trailing newline).
    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(TRUST_NAME);
        let mut s = serde_json::to_string_pretty(self).context("serializing trust file")?;
        s.push('\n');
        std::fs::write(&path, s).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    /// The trust entry for `package`, if any.
    pub fn get(&self, package: &str) -> Option<&TrustEntry> {
        self.trust.iter().find(|e| e.package == package)
    }

    /// A normalized copy (version pinned, entries sorted by package then files) for stable
    /// equality / diffing and deterministic serialization.
    pub fn normalized(&self) -> TrustFile {
        let mut trust = self.trust.clone();
        for e in &mut trust {
            e.files.sort();
        }
        trust.sort_by(|a, b| {
            a.package
                .cmp(&b.package)
                .then_with(|| a.files.cmp(&b.files))
        });
        TrustFile {
            version: TRUST_VERSION,
            trust,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let tf = TrustFile {
            version: TRUST_VERSION,
            trust: vec![TrustEntry {
                package: "SomeLib".into(),
                checksum: "sha256:deadbeef".into(),
                files: vec!["src/dynamic.ahk".into()],
                reason: Some("vetted".into()),
            }],
        };
        tf.save(tmp.path()).unwrap();
        let back = TrustFile::load(tmp.path()).unwrap().unwrap();
        assert_eq!(tf, back);
        assert!(!back.trust[0].is_whole_package());
    }

    #[test]
    fn missing_file_loads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(TrustFile::load(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn whole_package_when_files_empty_or_star() {
        let entry = |files: Vec<String>| TrustEntry {
            package: "p".into(),
            checksum: "sha256:1".into(),
            files,
            reason: None,
        };
        assert!(entry(vec![]).is_whole_package());
        assert!(entry(vec!["*".into()]).is_whole_package());
        assert!(!entry(vec!["a.ahk".into()]).is_whole_package());
    }

    #[test]
    fn normalized_sorts_and_pins_version() {
        let tf = TrustFile {
            version: 0,
            trust: vec![
                TrustEntry {
                    package: "b".into(),
                    checksum: "sha256:1".into(),
                    files: vec!["z.ahk".into(), "a.ahk".into()],
                    reason: None,
                },
                TrustEntry {
                    package: "a".into(),
                    checksum: "sha256:2".into(),
                    files: vec![],
                    reason: None,
                },
            ],
        }
        .normalized();
        assert_eq!(tf.version, TRUST_VERSION);
        assert_eq!(tf.trust[0].package, "a");
        assert_eq!(tf.trust[1].files, vec!["a.ahk", "z.ahk"]);
    }

    #[test]
    fn rejects_unknown_fields() {
        let json = r#"{"version":1,"trust":[{"package":"p","checksum":"sha256:1","oops":true}]}"#;
        assert!(serde_json::from_str::<TrustFile>(json).is_err());
    }
}
