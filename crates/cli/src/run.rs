//! `ahkbuild run`: launch an entry script under the project's configured interpreter, with
//! `AhkImportPath` pointed at the dependency link-farm so `#Import Name` resolves.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ahkbuild_interpret::{AhkVersion, Bitness};
use anyhow::{bail, Context, Result};

use crate::scripts::child_stdin;

/// The interpreter's default `AhkImportPath` list, used as the base when the environment does not
/// already set one. Kept in sync with `ahkbuild_link`'s default list.
const DEFAULT_IMPORT_PATH: &str = r"%A_ScriptDir%;%A_MyDocuments%\AutoHotkey;%A_AhkPath%\..";

pub(crate) fn run(
    config_path: Option<&Path>,
    entry_override: Option<&Path>,
    validate_only: &bool,
    interpreter_version: Option<AhkVersion>,
    bitness: Option<Bitness>,
    args: &[String],
) -> Result<()> {
    let (mut config, project_root) = crate::config_util::load(config_path)?;
    config.merge_cli(
        entry_override.map(PathBuf::from),
        interpreter_version,
        bitness,
    );

    let entry = config.entry.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "no entry point specified\n\
             hint: add \"entry\" to ahkbuild.json or pass a script to `ahkbuild run`"
        )
    })?;

    // Make sure dependencies are pinned and the link-farm exists before we launch.
    let report = ahkbuild_pkg::restore(
        &config,
        &project_root,
        ahkbuild_pkg::RestoreOptions::default(),
    )
    .context("restoring dependencies")?;
    tracing::info!(restored = report.restored, "dependencies ready");

    // Resolve the interpreter (auto-installs if not cached).
    let interp =
        ahkbuild_interpret::install(&config.interpreter.version, &config.interpreter.bitness)
            .with_context(|| {
                format!(
                    "could not resolve interpreter {} ({})",
                    config.interpreter.version,
                    match config.interpreter.bitness {
                        Bitness::X32 => "x32",
                        Bitness::X64 => "x64",
                    },
                )
            })?;

    // Prepend the link-farm to the interpreter's search list so `#Import Name` resolves to a
    // dependency. Anything already on `AhkImportPath` (or the default list) still applies after it.
    let modules = ahkbuild_pkg::modules_dir(&project_root);
    let base = std::env::var("AhkImportPath").unwrap_or_else(|_| DEFAULT_IMPORT_PATH.to_string());
    let import_path = format!("{};{}", modules.display(), base);

    tracing::info!(
        interpreter = %interp.display(),
        entry = %entry.display(),
        "running",
    );

    let mut interpereter_args = vec!["/ErrorStdOut=Utf-8"];
    if *validate_only {
        interpereter_args.push("/Validate")
    }

    // Pipe + relay stdout/stderr rather than inheriting, for the same GUI-subsystem handle reason
    // as build scripts (see `scripts::run_one`).
    let mut child = Command::new(&interp)
        .args(interpereter_args)
        .arg(&entry)
        .args(args)
        .current_dir(&project_root)
        .env("AhkImportPath", &import_path)
        .stdin(child_stdin())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("launching {}", interp.display()))?;

    let mut child_out = child.stdout.take().expect("stdout piped");
    let mut child_err = child.stderr.take().expect("stderr piped");
    let relay_out = std::thread::spawn(move || {
        let _ = std::io::copy(&mut child_out, &mut std::io::stdout());
    });
    let relay_err = std::thread::spawn(move || {
        let _ = std::io::copy(&mut child_err, &mut std::io::stderr());
    });

    let status = child.wait().context("waiting on interpreter")?;
    let _ = relay_out.join();
    let _ = relay_err.join();

    if !status.success() {
        match status.code() {
            Some(code) => bail!("script exited with status {code}"),
            None => bail!("script terminated by signal"),
        }
    }
    Ok(())
}
