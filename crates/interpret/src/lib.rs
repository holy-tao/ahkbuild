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

/// A cached interpreter entry returned by [`list`].
pub struct CachedEntry {
    pub version: AhkVersion,
    pub bitnesses: Vec<Bitness>,
    pub dir: PathBuf,
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot locate home directory"))
}

fn interpreters_root() -> Result<PathBuf> {
    Ok(home_dir()?.join(".ahkbuild").join("interpreters"))
}

fn cache_dir(version: &AhkVersion) -> Result<PathBuf> {
    Ok(interpreters_root()?.join(version.canonical()))
}

/// Return all cached interpreters, sorted by version ascending.
pub fn list() -> Result<Vec<CachedEntry>> {
    let root = interpreters_root()?;
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<CachedEntry> = std::fs::read_dir(&root)
        .with_context(|| format!("reading {}", root.display()))?
        .filter_map(|res| {
            let entry = res.ok()?;
            if !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let name = entry.file_name().into_string().ok()?;
            let version: AhkVersion = name.parse().ok()?;
            let dir = entry.path();

            let mut bitnesses = Vec::new();
            if dir.join("AutoHotkey32.exe").exists() {
                bitnesses.push(Bitness::X32);
            }
            if dir.join("AutoHotkey64.exe").exists() {
                bitnesses.push(Bitness::X64);
            }
            // Skip dirs that contain neither exe (e.g. partially-cleaned entries)
            if bitnesses.is_empty() {
                return None;
            }

            Some(CachedEntry {
                version,
                bitnesses,
                dir,
            })
        })
        .collect();

    entries.sort_by(|a, b| a.version.cmp(&b.version));
    Ok(entries)
}

/// Remove cached interpreter files.
///
/// - `version = None`  -> all versions; `version = Some(v)` -> only that version.
/// - `bitness = None`  -> all bitnesses (removes the whole version dir);
///   `bitness = Some(b)` -> only that exe (removes the version dir if it becomes empty).
pub fn prune(version: Option<&AhkVersion>, bitness: Option<&Bitness>) -> Result<usize> {
    let root = interpreters_root()?;
    if !root.exists() {
        return Ok(0);
    }

    // Collect the version directories to operate on.
    let dirs: Vec<PathBuf> = match version {
        Some(v) => {
            let d = root.join(v.canonical());
            if d.exists() {
                vec![d]
            } else {
                vec![]
            }
        }
        None => std::fs::read_dir(&root)
            .with_context(|| format!("reading {}", root.display()))?
            .filter_map(|res| {
                let e = res.ok()?;
                if e.file_type().ok()?.is_dir() {
                    Some(e.path())
                } else {
                    None
                }
            })
            .collect(),
    };

    let mut removed = 0usize;
    for dir in &dirs {
        match bitness {
            None => {
                std::fs::remove_dir_all(dir)
                    .with_context(|| format!("removing {}", dir.display()))?;
                removed += 1;
            }
            Some(b) => {
                let exe = dir.join(b.exe_name());
                if exe.exists() {
                    std::fs::remove_file(&exe)
                        .with_context(|| format!("removing {}", exe.display()))?;
                    removed += 1;
                }
                // Clean up the version dir if both exes are now gone.
                let is_empty = !dir.join("AutoHotkey32.exe").exists()
                    && !dir.join("AutoHotkey64.exe").exists();
                if is_empty {
                    std::fs::remove_dir_all(dir).ok();
                }
            }
        }
    }

    Ok(removed)
}

/// Install an AHK interpreter with the given version and bitness.
/// Returns the path to the installed interpreter.
pub fn install(version: &AhkVersion, bitness: &Bitness) -> Result<PathBuf> {
    let dir = cache_dir(version)?;
    let exe = dir.join(bitness.exe_name());

    // 1. Check cache
    if exe.exists() {
        tracing::info!(path = %exe.display(), "using cached interpreter");
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
    tracing::info!(url = %ahkcom_url, "trying autohotkey.com");
    match download_and_extract(&ahkcom_url, &dir) {
        Ok(()) if exe.exists() => {
            tracing::info!("installed from autohotkey.com");
            return Ok(exe);
        }
        Ok(()) => {
            tracing::debug!(
                exe = bitness.exe_name(),
                "download succeeded but exe not found in zip, trying next source"
            );
        }
        Err(e) => {
            tracing::debug!(error = %e, "autohotkey.com failed, trying next source");
        }
    }

    // 3. Try GitHub releases (v2.0 only; v2.1 alphas are tagged but have no formal release)
    if !is_v21_plus {
        tracing::info!("trying GitHub releases");
        match github::release_zip_url(version) {
            Ok(url) => match download_and_extract(&url, &dir) {
                Ok(()) if exe.exists() => {
                    tracing::info!("installed from GitHub releases");
                    return Ok(exe);
                }
                Ok(()) => tracing::debug!(
                    exe = bitness.exe_name(),
                    "GitHub download succeeded but exe not found in zip"
                ),
                Err(e) => tracing::debug!(error = %e, "GitHub download failed"),
            },
            Err(e) => tracing::debug!(error = %e, "GitHub release lookup failed"),
        }
    }

    // 4. Compile from source (required for v2.1 in CI; slow but reliable)
    tracing::info!(version = %version.canonical(), "building AutoHotkey from source");
    build::compile_from_source(version, bitness, &dir)
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
