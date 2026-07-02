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
        // `path` deps are never locked; this is only a stable placeholder.
        DependencySource::Path { .. } => "path".to_string(),
    }
}

/// The `.git` clone URL for a gist id.
pub fn gist_url(id: &str) -> String {
    format!("https://gist.github.com/{id}.git")
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
    }
}
