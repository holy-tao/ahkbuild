//! Module dependency declarations (`ahkbuild.json` -> `dependencies`). Resolution, locking, and
//! materialization live in `crates/pkg`; this module only defines and validates the manifest shape.

use std::path::PathBuf;

use serde::Deserialize;

/// A single module dependency. Exactly one source kind is set; `subdir` optionally points at the
/// module root inside the fetched tree when it is not the repository/archive root.
#[derive(Debug, Clone, PartialEq)]
pub struct DependencySpec {
    pub source: DependencySource,
    /// Sub-directory within the fetched tree that holds the module (`#Import Name` maps here).
    pub subdir: Option<String>,
    /// Local import name for the dependency, overriding the manifest key. The key is the canonical
    /// package name (often the repo/tarball name, e.g. `yaml.ahk`), which may not be a valid AHK
    /// identifier; `alias` lets the project import it under a clean unquoted name (`#Import yaml`).
    /// When set it must be a valid AHK identifier; when unset the key itself is the import name.
    pub alias: Option<String>,
}

impl DependencySpec {
    /// The name this dependency is imported under (`#Import <name>`) and exposed as in the
    /// link-farm: the `alias` when set, otherwise the manifest `key`.
    pub fn import_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.alias.as_deref().unwrap_or(key)
    }
}

/// Validate that `alias` is a usable AHK module identifier
/// See: https://www.autohotkey.com/docs/alpha/Concepts.htm#names
fn validate_alias(alias: &str) -> Result<(), String> {
    let is_start = |c: char| c.is_ascii_alphabetic() || c == '_' || !c.is_ascii();
    let is_part = |c: char| c.is_ascii_alphanumeric() || c == '_' || !c.is_ascii();
    let mut chars = alias.chars();
    match chars.next() {
        Some(c) if is_start(c) => {}
        _ => {
            return Err(format!(
                "dependency alias {alias:?} is not a valid AHK identifier: it must start with a \
                 letter, underscore, or non-ASCII character"
            ))
        }
    }
    if !chars.all(is_part) {
        return Err(format!(
            "dependency alias {alias:?} is not a valid AHK identifier: only letters, digits, \
             underscore, and non-ASCII characters are allowed"
        ));
    }
    Ok(())
}

/// Where a dependency's bytes come from. `git` is a real clone of a `.git` URL (any forge);
/// `gist` is the same mechanism against a gist; `tarball` is a checksummed archive; `path` is a
/// local directory (not reproducible, so excluded from the lockfile).
#[derive(Debug, Clone, PartialEq)]
pub enum DependencySource {
    Git { url: String, selector: GitSelector },
    Gist { id: String, rev: Option<String> },
    Tarball { url: String, sha256: String },
    Path { path: PathBuf },
}

/// The revision selector for a `git` source. `Default` means the remote's default branch HEAD,
/// resolved to a commit SHA at lock time.
#[derive(Debug, Clone, PartialEq)]
pub enum GitSelector {
    Tag(String),
    Branch(String),
    Rev(String),
    Default,
}

impl<'de> Deserialize<'de> for DependencySpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            // Source kinds (exactly one).
            git: Option<String>,
            gist: Option<String>,
            tarball: Option<String>,
            path: Option<PathBuf>,
            // git/gist revision selectors.
            tag: Option<String>,
            branch: Option<String>,
            rev: Option<String>,
            // tarball integrity.
            sha256: Option<String>,
            // Common.
            subdir: Option<String>,
            alias: Option<String>,
        }

        let r = Repr::deserialize(d)?;

        if let Some(alias) = &r.alias {
            validate_alias(alias).map_err(D::Error::custom)?;
        }

        // Exactly one source key must be present.
        let kinds = [
            ("git", r.git.is_some()),
            ("gist", r.gist.is_some()),
            ("tarball", r.tarball.is_some()),
            ("path", r.path.is_some()),
        ];
        let present: Vec<&str> = kinds.iter().filter(|(_, v)| *v).map(|(k, _)| *k).collect();
        let kind = match present.as_slice() {
            [k] => *k,
            [] => {
                return Err(D::Error::custom(
                    "dependency must set one source: \"git\", \"gist\", \"tarball\", or \"path\"",
                ))
            }
            many => {
                return Err(D::Error::custom(format!(
                    "dependency sets conflicting sources ({}); use exactly one of git/gist/tarball/path",
                    many.join(", ")
                )))
            }
        };

        // Reject selectors/fields that don't belong to the chosen source.
        let git_selectors = r.tag.is_some() || r.branch.is_some() || r.rev.is_some();
        let reject = |cond: bool, msg: &str| {
            if cond {
                Err(D::Error::custom(msg))
            } else {
                Ok(())
            }
        };

        let source = match kind {
            "git" => {
                reject(
                    r.sha256.is_some(),
                    "\"sha256\" is only valid for a tarball source",
                )?;
                let selector = match (r.tag, r.branch, r.rev) {
                    (Some(t), None, None) => GitSelector::Tag(t),
                    (None, Some(b), None) => GitSelector::Branch(b),
                    (None, None, Some(rv)) => GitSelector::Rev(rv),
                    (None, None, None) => GitSelector::Default,
                    _ => {
                        return Err(D::Error::custom(
                            "git source accepts at most one of \"tag\", \"branch\", or \"rev\"",
                        ))
                    }
                };
                DependencySource::Git {
                    url: r.git.unwrap(),
                    selector,
                }
            }
            "gist" => {
                reject(
                    r.sha256.is_some(),
                    "\"sha256\" is only valid for a tarball source",
                )?;
                reject(
                    r.tag.is_some() || r.branch.is_some(),
                    "gist source accepts only \"rev\" (not \"tag\"/\"branch\")",
                )?;
                DependencySource::Gist {
                    id: r.gist.unwrap(),
                    rev: r.rev,
                }
            }
            "tarball" => {
                reject(
                    git_selectors,
                    "\"tag\"/\"branch\"/\"rev\" are not valid for a tarball source",
                )?;
                let sha256 = r
                    .sha256
                    .ok_or_else(|| D::Error::custom("tarball source requires \"sha256\""))?;
                DependencySource::Tarball {
                    url: r.tarball.unwrap(),
                    sha256,
                }
            }
            "path" => {
                reject(
                    git_selectors || r.sha256.is_some(),
                    "a path source takes no revision selector or checksum",
                )?;
                DependencySource::Path {
                    path: r.path.unwrap(),
                }
            }
            _ => unreachable!("kind is one of the matched keys"),
        };

        Ok(DependencySpec {
            source,
            subdir: r.subdir,
            alias: r.alias,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BuildConfig;

    fn parse(json: &str) -> BuildConfig {
        serde_json::from_str(json).expect("parse failed")
    }

    #[test]
    fn dependencies_parse_each_source_kind() {
        let c = parse(
            r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "dependencies": {
                "GuiEnhancer": {"git": "https://github.com/x/y.git", "tag": "v1.0.3"},
                "OnGit":       {"git": "https://gitlab.com/x/y.git"},
                "cJson":       {"gist": "abc123", "rev": "deadbeef"},
                "Rapid":       {"tarball": "https://e.com/r.zip", "sha256": "ff", "subdir": "src"},
                "MyLocal":     {"path": "../shared/MyLocal"}
            }
        }"#,
        );
        assert_eq!(c.dependencies.len(), 5);
        assert_eq!(
            c.dependencies["GuiEnhancer"].source,
            DependencySource::Git {
                url: "https://github.com/x/y.git".into(),
                selector: GitSelector::Tag("v1.0.3".into()),
            }
        );
        assert!(matches!(
            c.dependencies["OnGit"].source,
            DependencySource::Git {
                selector: GitSelector::Default,
                ..
            }
        ));
        assert_eq!(
            c.dependencies["cJson"].source,
            DependencySource::Gist {
                id: "abc123".into(),
                rev: Some("deadbeef".into()),
            }
        );
        assert_eq!(c.dependencies["Rapid"].subdir.as_deref(), Some("src"));
        assert!(matches!(
            c.dependencies["Rapid"].source,
            DependencySource::Tarball { .. }
        ));
    }

    #[test]
    fn dependency_requires_a_source() {
        assert!(serde_json::from_str::<BuildConfig>(
            r#"{"interpreter": {"version": "2.1-alpha.27"}, "dependencies": {"X": {"tag": "v1"}}}"#,
        )
        .is_err());
    }

    #[test]
    fn dependency_rejects_conflicting_sources() {
        assert!(serde_json::from_str::<BuildConfig>(
            r#"{"interpreter": {"version": "2.1-alpha.27"},
                "dependencies": {"X": {"git": "u", "path": "p"}}}"#,
        )
        .is_err());
    }

    #[test]
    fn git_rejects_multiple_selectors() {
        assert!(serde_json::from_str::<BuildConfig>(
            r#"{"interpreter": {"version": "2.1-alpha.27"},
                "dependencies": {"X": {"git": "u", "tag": "t", "branch": "b"}}}"#,
        )
        .is_err());
    }

    #[test]
    fn alias_overrides_import_name() {
        let c = parse(
            r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "dependencies": {
                "yaml.ahk": {"git": "https://github.com/x/yaml.ahk.git", "alias": "yaml"},
                "Plain":    {"git": "https://github.com/x/y.git"}
            }
        }"#,
        );
        // Aliased key imports under the alias; an un-aliased key imports under the key itself.
        assert_eq!(c.dependencies["yaml.ahk"].alias.as_deref(), Some("yaml"));
        assert_eq!(c.dependencies["yaml.ahk"].import_name("yaml.ahk"), "yaml");
        assert_eq!(c.dependencies["Plain"].alias, None);
        assert_eq!(c.dependencies["Plain"].import_name("Plain"), "Plain");
    }

    #[test]
    fn invalid_alias_is_rejected() {
        // A dot makes it an invalid AHK identifier (and would escape the link-farm component).
        assert!(serde_json::from_str::<BuildConfig>(
            r#"{"interpreter": {"version": "2.1-alpha.27"},
                "dependencies": {"X": {"git": "u", "alias": "not.valid"}}}"#,
        )
        .is_err());
        // Leading digit is also rejected.
        assert!(serde_json::from_str::<BuildConfig>(
            r#"{"interpreter": {"version": "2.1-alpha.27"},
                "dependencies": {"X": {"git": "u", "alias": "1yaml"}}}"#,
        )
        .is_err());
    }

    #[test]
    fn tarball_requires_sha256() {
        assert!(serde_json::from_str::<BuildConfig>(
            r#"{"interpreter": {"version": "2.1-alpha.27"},
                "dependencies": {"X": {"tarball": "u"}}}"#,
        )
        .is_err());
    }
}
