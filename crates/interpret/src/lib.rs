//! Libraries for managing interpreters. Ahkbuild maintains a cache of interpreters
//! at ~/.ahkbuild/interpreters/<version>/ - each version directory contains one or more
//! of AutoHotkey32.exe and AutoHotkey64.exe.
//!
//! Users manage these with the `ahkbuild interpret` cli command.

mod build;
mod github;
mod version;

pub use version::AhkVersion;

use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(clap::ValueEnum, Debug, Clone, Eq, PartialEq)]
pub enum Bitness {
    X32,
    X64,
}

impl Bitness {
    fn exe_name(&self) -> &'static str {
        match self {
            Bitness::X32 => "AutoHotkey32.exe",
            Bitness::X64 => "AutoHotkey64.exe",
        }
    }
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot locate home directory"))
}

fn cache_dir(version: &AhkVersion) -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".ahkbuild")
        .join("interpreters")
        .join(version.canonical()))
}

/// Install an AHK interpreter with the given version and bitness.
/// Returns the path to the installed interpreter.
pub fn install(version: &AhkVersion, bitness: &Bitness) -> Result<PathBuf> {
    let dir = cache_dir(&version)?;
    let exe = dir.join(bitness.exe_name());

    // 1. Check cache
    if exe.exists() {
        eprintln!("Using cached interpreter: {}", exe.display());
        return Ok(exe);
    }

    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating cache dir {}", dir.display()))?;

    // v2.1+ alphas have no GitHub releases - must compile from source.
    // v2.0.x is available from autohotkey.com and GitHub releases.
    let is_v21_plus = version.major >= 2 && version.minor >= 1;

    // 2. Try autohotkey.com (fast on user machines; often blocked in CI by Cloudflare)
    let ahkcom_url = format!(
        "https://www.autohotkey.com/download/{}.{}/AutoHotkey_{}.zip",
        version.major,
        version.minor,
        version.canonical()
    );
    eprintln!("Trying autohotkey.com: {}", ahkcom_url);
    match download_and_extract(&ahkcom_url, &dir) {
        Ok(()) if exe.exists() => {
            eprintln!("Installed from autohotkey.com");
            return Ok(exe);
        }
        Ok(()) => {
            eprintln!(
                "Download succeeded but {} not found in zip, trying next source",
                bitness.exe_name()
            );
        }
        Err(e) => {
            eprintln!("autohotkey.com failed ({}), trying next source", e);
        }
    }

    // 3. Try GitHub releases (v2.0 only; v2.1 alphas are tagged but have no formal release)
    if !is_v21_plus {
        eprintln!("Trying GitHub releases...");
        match github::release_zip_url(&version) {
            Ok(url) => match download_and_extract(&url, &dir) {
                Ok(()) if exe.exists() => {
                    eprintln!("Installed from GitHub releases");
                    return Ok(exe);
                }
                Ok(()) => eprintln!(
                    "GitHub download succeeded but {} not found in zip",
                    bitness.exe_name()
                ),
                Err(e) => eprintln!("GitHub download failed: {}", e),
            },
            Err(e) => eprintln!("GitHub release lookup failed: {}", e),
        }
    }

    // 4. Compile from source (required for v2.1 in CI; slow but reliable)
    eprintln!("Building AutoHotkey {} from source...", version.canonical());
    build::compile_from_source(&version, &bitness, &dir)
        .context("failed to compile AutoHotkey from source")?;

    if exe.exists() {
        Ok(exe)
    } else {
        anyhow::bail!(
            "source build completed but {} was not found at {}",
            bitness.exe_name(),
            dir.display()
        )
    }
}

fn download_and_extract(url: &str, dest: &Path) -> Result<()> {
    let resp = ureq::get(url)
        .set("User-Agent", "ahkbuild")
        .call()
        .with_context(|| format!("GET {}", url))?;

    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .context("reading response body")?;

    extract_ahk_zip(&buf, dest)
}

fn extract_ahk_zip(bytes: &[u8], dest: &Path) -> Result<()> {
    use std::io::Cursor;

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("reading zip archive")?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        let basename = Path::new(&name)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("")
            .to_string();

        if basename == "AutoHotkey32.exe" || basename == "AutoHotkey64.exe" {
            let out_path = dest.join(&basename);
            let mut out = std::fs::File::create(&out_path)
                .with_context(|| format!("creating {}", out_path.display()))?;
            std::io::copy(&mut file, &mut out)
                .with_context(|| format!("extracting {}", basename))?;
        }
    }

    Ok(())
}
