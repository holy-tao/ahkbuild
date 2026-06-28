use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Result};

/// An AutoHotkey interpreter version.
///
/// These are *almost* SemVer, but not quite, so we have our own type:
/// - v2.0 releases use `major.minor.patch` (e.g. `2.0.26`).
/// - v2.1 pre-releases use `major.minor-alpha.N` with no patch (e.g. `2.1-alpha.27`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AhkVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: Option<u32>,
    pub pre: Option<AhkPrerelease>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AhkPrerelease {
    Alpha(u32),
}

impl AhkVersion {
    /// The string used as the cache directory name and in URLs.
    /// Matches the canonical AHK release tag exactly (e.g. `2.1-alpha.27`, `2.0.26`).
    pub fn canonical(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for AhkVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)?;
        if let Some(patch) = self.patch {
            write!(f, ".{}", patch)?;
        }
        if let Some(pre) = &self.pre {
            match pre {
                AhkPrerelease::Alpha(n) => write!(f, "-alpha.{}", n)?,
            }
        }
        Ok(())
    }
}

impl FromStr for AhkVersion {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        // Split on '-' first to separate prerelease (e.g. "2.1-alpha.27" -> "2.1" + "alpha.27")
        let (version_part, pre_part) = match s.split_once('-') {
            Some((v, p)) => (v, Some(p)),
            None => (s, None),
        };

        let mut numeric = version_part.splitn(3, '.');
        let major = numeric
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("expected a major version"))?;
        let minor = numeric
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| anyhow::anyhow!("expected a minor version"))?;
        let patch = numeric.next().and_then(|s| s.parse().ok());

        let pre = match pre_part {
            Some(p) => {
                if let Some(n) = p.strip_prefix("alpha.") {
                    let n: u32 = n
                        .parse()
                        .map_err(|_| anyhow::anyhow!("expected a numeric prerelease version"))?;
                    Some(AhkPrerelease::Alpha(n))
                } else {
                    bail!("unrecognised AHK prerelease tag: {:?}", p);
                }
            }
            None => None,
        };

        Ok(AhkVersion {
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl PartialOrd for AhkVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AhkVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare major, then minor, then patch (None < Some), then prerelease.
        // A prerelease is older than the same version without one (alpha < release),
        // matching semver convention.
        (self.major, self.minor, self.patch, PreOrd(&self.pre)).cmp(&(
            other.major,
            other.minor,
            other.patch,
            PreOrd(&other.pre),
        ))
    }
}

/// Newtype so we can impose the ordering: None (no prerelease = stable) > Some (prerelease).
struct PreOrd<'a>(&'a Option<AhkPrerelease>);

impl PartialEq for PreOrd<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for PreOrd<'_> {}

impl PartialOrd for PreOrd<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PreOrd<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self.0, other.0) {
            (None, None) => std::cmp::Ordering::Equal,
            // stable > prerelease
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(a), Some(b)) => match (a, b) {
                (AhkPrerelease::Alpha(a), AhkPrerelease::Alpha(b)) => a.cmp(b),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_v2_0() {
        let v: AhkVersion = "2.0.26".parse().unwrap();
        assert_eq!(
            v,
            AhkVersion {
                major: 2,
                minor: 0,
                patch: Some(26),
                pre: None
            }
        );
        assert_eq!(v.to_string(), "2.0.26");
    }

    #[test]
    fn parse_v2_1_alpha() {
        let v: AhkVersion = "2.1-alpha.27".parse().unwrap();
        assert_eq!(
            v,
            AhkVersion {
                major: 2,
                minor: 1,
                patch: None,
                pre: Some(AhkPrerelease::Alpha(27))
            }
        );
        assert_eq!(v.to_string(), "2.1-alpha.27");
    }

    #[test]
    fn ordering() {
        let v = |s: &str| s.parse::<AhkVersion>().unwrap();
        assert!(v("2.0.26") > v("2.0.18"));
        assert!(v("2.1-alpha.27") > v("2.1-alpha.10"));
        // v2.0 stable is a lower minor than v2.1 alpha
        assert!(v("2.1-alpha.1") > v("2.0.26"));
    }
}
