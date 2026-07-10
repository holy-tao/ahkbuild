//! Shared config discovery/loading for the config-driven subcommands (`bundle exe`, `run`,
//! `package restore`).

use std::path::{Path, PathBuf};

use ahkbuild_config::{BuildConfig, TrustFile};
use ahkbuild_pkg::{resolve_trust, FileRule};
use ahkbuild_shake::TrustSet;
use anyhow::{Context, Result};

/// Locate `ahkbuild.json` (explicit `--config`, else discovered by walking up from cwd). Returns the
/// config file path; use [`project_root`] for its directory.
pub(crate) fn locate(config_path: Option<&Path>) -> Result<PathBuf> {
    match config_path {
        Some(p) => Ok(p.to_path_buf()),
        None => {
            let cwd = std::env::current_dir().context("getting current directory")?;
            ahkbuild_config::find_config(&cwd)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "no ahkbuild.json found in {} or any parent directory\n\
                     hint: create an ahkbuild.json at the project root, or use --config",
                    cwd.display()
                )
            })
        }
    }
}

/// The project root (directory) for a config file path.
pub(crate) fn project_root(config_file: &Path) -> PathBuf {
    config_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Locate and load `ahkbuild.json`, returning it alongside the project root (its directory).
pub(crate) fn load(config_path: Option<&Path>) -> Result<(BuildConfig, PathBuf)> {
    let config_file = locate(config_path)?;
    let config = ahkbuild_config::load(&config_file)?;
    let root = project_root(&config_file);
    Ok((config, root))
}

/// Build the tree-shaking [`TrustSet`] for a project from its `ahkbuild.trust.json` (absent = empty
/// trust, i.e. unchanged conservative behavior). Stale/unknown entries are dropped with a warning
/// by [`resolve_trust`].
pub(crate) fn load_trust(config: &BuildConfig, project_root: &Path) -> Result<TrustSet> {
    let Some(file) = TrustFile::load(project_root)? else {
        return Ok(TrustSet::default());
    };
    let resolved = resolve_trust(config, project_root, &file)?;
    let mut trust = TrustSet::default();
    for r in resolved {
        let files = match r.files {
            FileRule::All => None,
            FileRule::Paths(paths) => Some(paths),
        };
        trust.trust_package(r.roots, files);
    }
    Ok(trust)
}
