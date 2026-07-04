//! Source identity helpers shared by resolution and locking.

use ahkbuild_config::{DependencySource, GitSelector};

/// A stable identity string for a dependency source. Captures everything that determines the
/// resolved revision and the fetched tree - url, selector, checksum - but **not** `subdir`, which
/// only affects which junction the tree is exposed under (the link-farm is rebuilt every restore,
/// so a `subdir` change is picked up without re-resolving). Used to detect when the manifest has
/// drifted from the lockfile.
pub fn source_id(src: &DependencySource) -> String {
    match src {
        DependencySource::Git { url, selector } => {
            let mut s = format!("git+{url}");
            match selector {
                GitSelector::Tag(t) => s.push_str(&format!("?tag={t}")),
                GitSelector::Branch(b) => s.push_str(&format!("?branch={b}")),
                GitSelector::Rev(r) => s.push_str(&format!("?rev={r}")),
                GitSelector::Default => {}
            }
            s
        }
        DependencySource::Gist { id, rev } => match rev {
            Some(r) => format!("gist+{id}?rev={r}"),
            None => format!("gist+{id}"),
        },
        DependencySource::Tarball { url, sha256 } => format!("tarball+{url}?sha256={sha256}"),
        DependencySource::GithubRelease {
            repo,
            tag,
            asset,
            sha256,
        } => format!("release+{repo}@{tag}/{asset}?sha256={sha256}"),
        // `path` deps are never locked; this is only a stable placeholder.
        DependencySource::Path { .. } => "path".to_string(),
    }
}

/// The `.git` clone URL for a gist id.
pub fn gist_url(id: &str) -> String {
    format!("https://gist.github.com/{id}.git")
}

/// The direct download URL for a GitHub release asset.
pub fn release_asset_url(repo: &str, tag: &str, asset: &str) -> String {
    format!("https://github.com/{repo}/releases/download/{tag}/{asset}")
}

/// The archive format implied by a file name's extension. `None` means the file is not a recognized
/// archive and should be treated as a single file. Shared by tarball/release fetching (to decide
/// whether to extract) and the link-farm (to decide whether a release asset is a directory tree or
/// a single `.ahk` module file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
}

/// Classify `name` (a URL or bare file name) by its archive extension.
pub fn archive_kind(name: &str) -> Option<ArchiveKind> {
    if name.ends_with(".zip") {
        Some(ArchiveKind::Zip)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some(ArchiveKind::TarGz)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_id_is_stable_per_kind() {
        assert_eq!(
            source_id(&DependencySource::Git {
                url: "https://example.com/x.git".into(),
                selector: GitSelector::Tag("v1".into()),
            }),
            "git+https://example.com/x.git?tag=v1"
        );
        assert_eq!(
            source_id(&DependencySource::Git {
                url: "u".into(),
                selector: GitSelector::Default,
            }),
            "git+u"
        );
        assert_eq!(
            source_id(&DependencySource::Gist {
                id: "abc".into(),
                rev: Some("deadbeef".into()),
            }),
            "gist+abc?rev=deadbeef"
        );
        assert_eq!(
            source_id(&DependencySource::Tarball {
                url: "u".into(),
                sha256: "ff".into(),
            }),
            "tarball+u?sha256=ff"
        );
        assert_eq!(
            source_id(&DependencySource::GithubRelease {
                repo: "holy-tao/YAML".into(),
                tag: "v0.5.0".into(),
                asset: "YAML64.ahk".into(),
                sha256: "ff".into(),
            }),
            "release+holy-tao/YAML@v0.5.0/YAML64.ahk?sha256=ff"
        );
    }

    #[test]
    fn release_asset_url_is_the_github_download_path() {
        assert_eq!(
            release_asset_url("holy-tao/YAML", "v0.5.0", "YAML64.ahk"),
            "https://github.com/holy-tao/YAML/releases/download/v0.5.0/YAML64.ahk"
        );
    }

    #[test]
    fn archive_kind_classifies_by_extension() {
        assert_eq!(archive_kind("x.zip"), Some(ArchiveKind::Zip));
        assert_eq!(archive_kind("x.tar.gz"), Some(ArchiveKind::TarGz));
        assert_eq!(archive_kind("x.tgz"), Some(ArchiveKind::TarGz));
        assert_eq!(archive_kind("YAML64.ahk"), None);
    }
}
