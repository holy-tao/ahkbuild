//! Orchestration for `bundle exe`, called from `main.rs`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use ahkbuild_interpret::{AhkVersion, Bitness};

use crate::scripts::{run_scripts, ScriptContext, Stage};

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

    tracing::info!(path = %interp.display(), "resolved interpreter");

    // 4. Link
    let script_dir = entry
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let builtins = ahkbuild_link::Builtins::detect(script_dir);
    let search = ahkbuild_link::SearchPath::from_env(&builtins);
    let linked = ahkbuild_link::link_entry(entry, &search)?;

    tracing::info!(
        file = %entry.display(),
        groups = linked.program.groups.len(),
        warnings = linked.warnings.len(),
        "linked",
    );
    for w in &linked.warnings {
        tracing::warn!("{w}");
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
    let converged = ahkbuild_pipeline::converge(linked, consts, tree_shake)?;

    // 6. Determine output path and the build-script context. The config directory is the project
    //    root: relative paths in argv resolve against it, and scripts run with it as their cwd.
    let out_path = resolve_output(output, &config, entry);
    let config_dir = config_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let defines = config.defines_env()?;
    let ctx = ScriptContext {
        target: "exe",
        output: &out_path,
        entry,
        interpreter: &interp,
        bitness: config.interpreter.bitness.clone(),
        subsystem: config.exe.subsystem,
        version: config.exe.version.as_deref(),
        config_dir: &config_dir,
        defines: &defines,
    };

    // 7. Pre-bundle scripts (e.g. codegen) run before emit; AHKBUILD_OUTPUT points at the
    //    not-yet-written exe so a script can pre-stage siblings.
    run_scripts(Stage::Pre, &config.scripts.pre_bundle, &ctx)?;

    // 8. Emit: inject each module as RCDATA into a copy of the interpreter PE, stamp metadata, and
    //    write the standalone exe. `keep_comments` is honored inside the exe emitter's render step.
    let _ = keep_comments; // exe emit always minifies; comment retention is not yet plumbed through
    ahkbuild_emit_exe::bundle_exe(
        &interp,
        &converged.program,
        &converged.plan,
        converged.round.shake.as_ref(),
        converged.round.fold.as_ref(),
        &config,
        &out_path,
    )?;

    tracing::info!(path = %out_path.display(), "wrote exe");

    // 9. Post-bundle scripts (e.g. code signing, UPX/MPRESS compression) run on the finished exe.
    run_scripts(Stage::Post, &config.scripts.post_bundle, &ctx)?;

    Ok(())
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
