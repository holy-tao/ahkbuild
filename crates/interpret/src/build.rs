use anyhow::Result;
use std::path::Path;

#[cfg(target_os = "windows")]
use anyhow::Context;
#[cfg(target_os = "windows")]
use std::process::Command;

use crate::{AhkVersion, Bitness};

/// Compile a single AutoHotkey bitness from source and place the exe into `dest`.
/// Requires Windows with Visual Studio Build Tools (MSBuild) and git.
pub fn compile_from_source(version: &AhkVersion, bitness: &Bitness, dest: &Path) -> Result<()> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (version, bitness, dest);
        anyhow::bail!("compile-from-source requires Windows (MSBuild + Visual Studio)");
    }

    #[cfg(target_os = "windows")]
    compile_windows(version, bitness, dest)
}

#[cfg(target_os = "windows")]
fn compile_windows(version: &AhkVersion, bitness: &Bitness, dest: &Path) -> Result<()> {
    let msbuild = find_msbuild()?;

    let temp_dir = std::env::temp_dir().join(format!("ahk-src-{}", version.canonical()));
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir)
            .with_context(|| format!("removing stale temp dir {}", temp_dir.display()))?;
    }

    let tag = format!("v{}", version.canonical());
    let temp_str = temp_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("temp dir path is not valid UTF-8"))?;

    tracing::info!(%tag, "cloning AutoHotkey/AutoHotkey");
    let status = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            &tag,
            "https://github.com/AutoHotkey/AutoHotkey",
            temp_str,
        ])
        .status()
        .context("running git clone")?;
    anyhow::ensure!(status.success(), "git clone failed (exit {})", status);

    let (platform, exe_name) = match bitness {
        Bitness::X32 => ("Win32", "AutoHotkey32.exe"),
        Bitness::X64 => ("x64", "AutoHotkey64.exe"),
    };

    tracing::info!(platform, "building Release configuration");
    let status = Command::new(&msbuild)
        .args([
            "AutoHotkeyx.sln",
            "/p:Configuration=Release",
            &format!("/p:Platform={}", platform),
            "/nologo",
            "/verbosity:minimal",
            "/m",
        ])
        .current_dir(&temp_dir)
        .status()
        .with_context(|| format!("running MSBuild for Release|{}", platform))?;
    anyhow::ensure!(
        status.success(),
        "MSBuild failed for Release|{} (exit {})",
        platform,
        status
    );

    std::fs::create_dir_all(dest)?;
    let src = temp_dir.join("bin").join(exe_name);
    let dst = dest.join(exe_name);
    std::fs::copy(&src, &dst)
        .with_context(|| format!("copying {} -> {}", src.display(), dst.display()))?;

    std::fs::remove_dir_all(&temp_dir).ok();

    tracing::info!(
        version = %version.canonical(),
        platform,
        dest = %dest.display(),
        "built AutoHotkey from source",
    );
    Ok(())
}

#[cfg(target_os = "windows")]
fn find_msbuild() -> Result<String> {
    let prog_files_x86 =
        std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| "C:\\Program Files (x86)".into());
    let vswhere = format!(
        "{}\\Microsoft Visual Studio\\Installer\\vswhere.exe",
        prog_files_x86
    );

    if !Path::new(&vswhere).exists() {
        anyhow::bail!(
            "vswhere.exe not found at '{}'. Install Visual Studio Build Tools 2022.",
            vswhere
        );
    }

    let output = Command::new(&vswhere)
        .args([
            "-latest",
            "-requires",
            "Microsoft.Component.MSBuild",
            "-find",
            "MSBuild\\**\\Bin\\MSBuild.exe",
        ])
        .output()
        .context("running vswhere")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let path = stdout
        .lines()
        .next()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "MSBuild not found via vswhere - install the 'Desktop development with C++' workload"
            )
        })?
        .trim()
        .to_string();

    Ok(path)
}
