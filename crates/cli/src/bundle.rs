//! Orchestration code for bundling, called from `main.rs`
//!

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Bundle into a single .ahk file
pub(crate) fn bundle_ahk(
    input: &Path,
    output: &Option<PathBuf>,
    tree_shake: bool,
    compiled: Option<bool>,
    bitness: Option<u8>,
    emit_options: &ahkbuild_emit::EmitOptions,
) -> Result<()> {
    let script_dir = input
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let builtins = ahkbuild_link::Builtins::detect(script_dir);
    let search = ahkbuild_link::SearchPath::from_env(&builtins);

    // Link modules
    let out = ahkbuild_link::link_entry(input, &search)?;

    tracing::info!(
        file = %input.display(),
        groups = out.program.groups.len(),
        warnings = out.warnings.len(),
        "linked",
    );

    for w in &out.warnings {
        tracing::warn!("{w}");
    }

    // Build-time constants. `A_PtrSize` is taken from `--bitness`, else from a bitness-pinned
    // `#Requires` (a certainty when present). `A_IsCompiled` is folded only when `--compiled`
    // is given — for `ahk` we assume nothing, since a bundle may later be compiled with ahk2exe.
    let ptr_size = match bitness {
        Some(32) => Some(4),
        Some(64) => Some(8),
        Some(other) => anyhow::bail!("invalid --bitness {other}; expected 32 or 64"),
        None => ahkbuild_fold::ptr_size_from_requires(&out.program),
    };
    let consts = ahkbuild_fold::Constants {
        is_compiled: compiled,
        ptr_size,
    };

    // Out-of-band package trust from `ahkbuild.trust.json`, if this script sits in a project. The
    // `ahk` bundle has no config of its own, so discover one by walking up from the input's dir;
    // no project (or no trust file) means empty trust and the usual conservative behavior.
    let trust = load_trust_for(input)?;

    // Hand off to the fixpoint driver, which runs constant folding and tree-shaking (when
    // `tree_shake` is set; `--no-tree-shake` opts out for a faithful bundle) to a fixpoint and
    // emits the final bundle. Pure-constant conditions (`if 2 + 2 == 4`) fold regardless of the
    // flags; `A_IsCompiled` folds only when `--compiled` made its value known.
    let bundled = ahkbuild_pipeline::bundle_ahk(out, consts, trust, tree_shake, emit_options)?;

    match output {
        Some(path) => {
            fs::write(path, bundled)?;
        }
        None => {
            print!("{}", bundled);
        }
    }
    Ok(())
}

/// Discover the project's `ahkbuild.json` by walking up from `input`'s directory and build its
/// tree-shaking trust set. Returns an empty set when the script is not inside a project (or the
/// project has no `ahkbuild.trust.json`), preserving the default conservative behavior.
fn load_trust_for(input: &Path) -> Result<ahkbuild_shake::TrustSet> {
    let Some(config_file) = ahkbuild_config::find_config(input)? else {
        return Ok(ahkbuild_shake::TrustSet::default());
    };
    let config = ahkbuild_config::load(&config_file)?;
    let root = config_file
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    crate::config_util::load_trust(&config, &root)
}
