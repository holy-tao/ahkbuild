//! `ahkbuild package list`: report each declared dependency's source, pinned revision, and whether
//! it is currently fetched into the store and linked into the project farm. Pure inspection - reads
//! the manifest, lock, store, and farm, and mutates nothing.

use std::path::Path;

use ahkbuild_config::{BuildConfig, DependencySource, GitSelector};

use crate::farm::modules_dir;
use crate::lock::Lockfile;
use crate::source::archive_kind;
use crate::store::store_path;

/// One dependency's declared source and materialization status.
#[derive(Debug, Clone)]
pub struct PackageStatus {
    /// Manifest key (the canonical package name).
    pub name: String,
    /// Import name (`#Import <name>`): the `alias` when set, otherwise the key.
    pub import_name: String,
    /// Human-readable source, e.g. `git https://… @ tag v1.0.3`.
    pub source: String,
    /// A local `path` source (never locked or stored).
    pub local: bool,
    /// The pinned revision from the lock (a commit SHA, or a URL); `None` for a `path` dep or one
    /// not yet resolved into the lock.
    pub resolved: Option<String>,
    /// Whether the dependency's tree is present (in the content store, or on disk for a `path` dep).
    pub present: bool,
    /// Whether the dependency is currently linked into `.ahkbuild/modules/`.
    pub linked: bool,
}

/// Collect the status of every declared dependency, in manifest (name-sorted) order.
pub fn list(config: &BuildConfig, project_root: &Path) -> anyhow::Result<Vec<PackageStatus>> {
    let lock = Lockfile::load(project_root)?.unwrap_or_default();
    let modules = modules_dir(project_root);

    let mut out = Vec::with_capacity(config.dependencies.len());
    for (name, spec) in &config.dependencies {
        let import_name = spec.import_name(name).to_string();
        let entry = lock.get(name);

        let (local, resolved, present) = match &spec.source {
            DependencySource::Path { path } => (true, None, path.exists()),
            _ => {
                let resolved = entry.map(|e| e.resolved.clone());
                let present = match entry.and_then(|e| e.content_hash().ok()) {
                    Some(hash) => store_path(hash)?.exists(),
                    None => false,
                };
                (false, resolved, present)
            }
        };

        out.push(PackageStatus {
            name: name.clone(),
            source: describe_source(&spec.source),
            local,
            resolved,
            present,
            linked: is_linked(&modules, &import_name, &spec.source),
            import_name,
        });
    }
    Ok(out)
}

/// Whether the farm holds a link for this dependency. A single-file release asset links as
/// `<import name>.ahk`; every other source links as a directory named `<import name>`.
fn is_linked(modules: &Path, import_name: &str, source: &DependencySource) -> bool {
    if let DependencySource::GithubRelease { asset, .. } = source {
        if archive_kind(asset).is_none() {
            return present(&modules.join(format!("{import_name}.ahk")));
        }
    }
    present(&modules.join(import_name))
}

/// Whether a path exists as a link or file, without following (a live junction and a dangling one
/// both count as "linked").
fn present(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

fn describe_source(src: &DependencySource) -> String {
    match src {
        DependencySource::Git { url, selector } => {
            let sel = match selector {
                GitSelector::Tag(t) => format!("tag {t}"),
                GitSelector::Branch(b) => format!("branch {b}"),
                GitSelector::Rev(r) => format!("rev {r}"),
                GitSelector::Default => "default branch".to_string(),
            };
            format!("git {url} @ {sel}")
        }
        DependencySource::Gist { id, rev } => match rev {
            Some(r) => format!("gist {id} @ rev {r}"),
            None => format!("gist {id}"),
        },
        DependencySource::Tarball { url, .. } => format!("tarball {url}"),
        DependencySource::GithubRelease {
            repo, tag, asset, ..
        } => format!("release {repo}@{tag}/{asset}"),
        DependencySource::Path { path } => format!("path {}", path.display()),
    }
}
