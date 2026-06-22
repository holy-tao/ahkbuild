//! Orchestration for `bundle exe`, called from `main.rs`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use ahkbuild_interpret::{AhkVersion, Bitness};

pub(crate) fn bundle_exe(
    config_path: Option<&Path>,
    input_override: Option<&Path>,
    output: Option<&Path>,
    interpreter_version: Option<AhkVersion>,
    bitness: Option<Bitness>,
    tree_shake: bool,
    keep_comments: bool,
) -> Result<()> {
    // 1. Find and load config
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

    let mut config = ahkbuild_config::load(&config_file)?;

    // 2. Apply CLI overrides
    config.merge_cli(
        input_override.map(PathBuf::from),
        interpreter_version,
        bitness,
    );

    let entry = config.entry.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "no entry point specified\n\
             hint: add \"entry\" to ahkbuild.json or use --input"
        )
    })?;

    // 3. Resolve interpreter (auto-installs if not cached)
    let interp =
        ahkbuild_interpret::install(&config.interpreter.version, &config.interpreter.bitness)
            .with_context(|| {
                format!(
                    "could not resolve interpreter {} ({})\n\
                     hint: run `ahkbuild interpreter install {}`",
                    config.interpreter.version,
                    match config.interpreter.bitness {
                        Bitness::X32 => "x32",
                        Bitness::X64 => "x64",
                    },
                    config.interpreter.version,
                )
            })?;

    eprintln!("interpreter: {}", interp.display());

    // 4. Link
    let script_dir = entry
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let builtins = ahkbuild_link::Builtins::detect(script_dir);
    let search = ahkbuild_link::SearchPath::from_env(&builtins);
    let linked = ahkbuild_link::link_entry(entry, &search)?;

    eprintln!(
        "linked {} ({} groups, {} warnings)",
        entry.display(),
        linked.program.groups.len(),
        linked.warnings.len(),
    );
    for w in &linked.warnings {
        eprintln!("warning: {w}");
    }

    // 5. Fold + shake (A_IsCompiled = true, A_PtrSize from bitness)
    let ptr_size = match config.interpreter.bitness {
        Bitness::X32 => Some(4),
        Bitness::X64 => Some(8),
    };
    let consts = ahkbuild_fold::Constants {
        is_compiled: Some(true),
        ptr_size,
    };
    let emit_options = ahkbuild_emit::EmitOptions {
        strip_comments: !keep_comments,
        whitespace: ahkbuild_emit::WsLevel::Minify,
    };
    let _bundled = ahkbuild_pipeline::bundle_ahk(linked, consts, tree_shake, &emit_options)?;

    // 6. Determine output path
    let out_path = resolve_output(output, &config, entry);
    eprintln!("output: {}", out_path.display());

    // TODO(exe-emit): inject _bundled text (and per-module RCDATA for v2.1) into the interpreter
    // PE binary and write to out_path. See docs/EXE_BUNDLING.md - crates/emit_exe planned.
    anyhow::bail!(
        "PE assembly not yet implemented\n\
         use `ahkbuild bundle ahk` to produce a .ahk bundle in the meantime"
    )
}

fn resolve_output(
    explicit: Option<&Path>,
    config: &ahkbuild_config::BuildConfig,
    entry: &Path,
) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    let stem = config
        .exe
        .name
        .as_deref()
        .or_else(|| entry.file_stem().and_then(|s| s.to_str()))
        .unwrap_or("out");
    PathBuf::from(format!("{stem}.exe"))
}
