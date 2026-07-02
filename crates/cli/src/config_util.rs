//! Shared config discovery/loading for the config-driven subcommands (`bundle exe`, `run`,
//! `package restore`).

use std::path::{Path, PathBuf};

use ahkbuild_config::BuildConfig;
use anyhow::{Context, Result};

/// Locate `ahkbuild.json` (explicit `--config`, else discovered by walking up from cwd), load it,
/// and return it alongside the project root (the config file's directory).
pub(crate) fn load(config_path: Option<&Path>) -> Result<(BuildConfig, PathBuf)> {
    let config_file = match config_path {
        Some(p) => p.to_path_buf(),
        None => {
            let cwd = std::env::current_dir().context("getting current directory")?;
            ahkbuild_config::find_config(&cwd)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "no ahkbuild.json found in {} or any parent directory\n\
                     hint: create an ahkbuild.json at the project root, or use --config",
                    cwd.display()
                )
            })?
        }
    };

    let config = ahkbuild_config::load(&config_file)?;
    let root = config_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok((config, root))
}
