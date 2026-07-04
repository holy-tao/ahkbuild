//! Fetching a dependency source into a staging directory.
//!
//! `git`/`gist` are real `git` checkouts (any forge, gists included); `tarball` is a checksummed
//! `.zip` or `.tar.gz` download. Each fetch returns the staged directory plus the immutable
//! `resolved` id (a commit SHA for git/gist, the URL for a tarball).

use std::path::PathBuf;
use std::process::Command;

use ahkbuild_config::{DependencySource, GitSelector};
use anyhow::{bail, Context, Result};

use crate::source::{archive_kind, gist_url, release_asset_url, ArchiveKind};
use crate::store::fresh_temp;

/// A staged fetch: the tree lives in `dir`, pinned to the immutable `resolved` id.
pub struct Fetched {
    pub resolved: String,
    pub dir: PathBuf,
}

/// Resolve a source at its manifest selector (tag/branch/rev/default, or a tarball URL) and stage
/// it. Used when the lockfile has no matching entry.
pub fn fetch_fresh(src: &DependencySource) -> Result<Fetched> {
    match src {
        DependencySource::Git { url, selector } => clone_at_selector(url, selector),
        DependencySource::Gist { id, rev } => {
            let url = gist_url(id);
            match rev {
                Some(r) => clone_at_selector(&url, &GitSelector::Rev(r.clone())),
                None => clone_at_selector(&url, &GitSelector::Default),
            }
        }
        DependencySource::Tarball { url, sha256 } => fetch_tarball(url, sha256),
        DependencySource::GithubRelease {
            repo,
            tag,
            asset,
            sha256,
        } => fetch_release(repo, tag, asset, sha256),
        DependencySource::Path { .. } => bail!("path dependencies are not fetched"),
    }
}

/// Re-stage a source at an already-pinned `resolved` id (repopulating a missing store entry from
/// the lockfile). For git/gist this checks out the exact commit SHA; for a tarball or release asset
/// it re-downloads and re-verifies (the URL is immutable, so `resolved` is not needed).
pub fn fetch_pinned(src: &DependencySource, resolved: &str) -> Result<Fetched> {
    match src {
        DependencySource::Git { url, .. } => {
            clone_at_selector(url, &GitSelector::Rev(resolved.into()))
        }
        DependencySource::Gist { id, .. } => {
            let url = gist_url(id);
            clone_at_selector(&url, &GitSelector::Rev(resolved.into()))
        }
        DependencySource::Tarball { url, sha256 } => fetch_tarball(url, sha256),
        DependencySource::GithubRelease {
            repo,
            tag,
            asset,
            sha256,
        } => fetch_release(repo, tag, asset, sha256),
        DependencySource::Path { .. } => bail!("path dependencies are not fetched"),
    }
}

fn clone_at_selector(url: &str, selector: &GitSelector) -> Result<Fetched> {
    let dir = fresh_temp()?;
    let dir_str = dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("temp path is not valid UTF-8"))?;

    match selector {
        // A tag or branch can be fetched shallowly by name.
        GitSelector::Tag(name) | GitSelector::Branch(name) => {
            git(&[
                "clone", "--quiet", "--depth", "1", "--branch", name, url, dir_str,
            ])?;
        }
        // The default branch HEAD, shallow.
        GitSelector::Default => {
            git(&["clone", "--quiet", "--depth", "1", url, dir_str])?;
        }
        // An arbitrary commit needs history, then a checkout.
        GitSelector::Rev(rev) => {
            git(&["clone", "--quiet", url, dir_str])?;
            git_in(&dir, &["checkout", "--quiet", rev])
                .with_context(|| format!("checking out {rev}"))?;
        }
    }

    let sha = git_capture(&dir, &["rev-parse", "HEAD"])?
        .trim()
        .to_string();

    tracing::info!(%sha, "Cloned repository {url} at ");

    // Drop VCS metadata so the tree hashes identically to a tarball of the same content.
    std::fs::remove_dir_all(dir.join(".git")).ok();

    Ok(Fetched { resolved: sha, dir })
}

fn fetch_tarball(url: &str, sha256: &str) -> Result<Fetched> {
    let bytes = download_verified(url, sha256, "tarball")?;
    let kind = archive_kind(url).ok_or_else(|| {
        anyhow::anyhow!("unsupported tarball extension for {url}: expected .zip, .tar.gz, or .tgz")
    })?;

    let dir = fresh_temp()?;
    extract_archive(kind, &bytes, &dir)?;

    Ok(Fetched {
        resolved: url.to_string(),
        dir,
    })
}

/// Fetch a GitHub release asset. An archive asset (.zip/.tar.gz/.tgz) is extracted like a tarball;
/// any other asset is staged as the sole file `<asset>` in a fresh directory, which the link-farm
/// exposes as `modules/<import name>.ahk`.
fn fetch_release(repo: &str, tag: &str, asset: &str, sha256: &str) -> Result<Fetched> {
    let url = release_asset_url(repo, tag, asset);
    let bytes = download_verified(&url, sha256, "release asset")?;

    let dir = fresh_temp()?;
    match archive_kind(asset) {
        Some(kind) => extract_archive(kind, &bytes, &dir)?,
        None => {
            let file = dir.join(asset);
            std::fs::write(&file, &bytes).with_context(|| format!("writing {}", file.display()))?;
        }
    }

    Ok(Fetched { resolved: url, dir })
}

fn extract_archive(kind: ArchiveKind, bytes: &[u8], dest: &std::path::Path) -> Result<()> {
    match kind {
        ArchiveKind::Zip => extract_zip(bytes, dest),
        ArchiveKind::TarGz => extract_tar_gz(bytes, dest),
    }
}

/// Download `url` and verify its bytes hash to `sha256` (case-insensitive hex). `what` names the
/// kind of download for error messages.
fn download_verified(url: &str, sha256: &str, what: &str) -> Result<Vec<u8>> {
    tracing::debug!(%url, "downloading {what}");
    let resp = ureq::get(url)
        .set("User-Agent", "ahkbuild")
        .call()
        .with_context(|| format!("GET {url}"))?;
    tracing::trace!("{:?}", resp); // TODO log headers too
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader(), &mut bytes)
        .with_context(|| format!("reading {what} body"))?;

    let got = {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(&bytes);
        let mut s = String::with_capacity(digest.len() * 2);
        for b in digest {
            s.push_str(&format!("{b:02x}"));
        }
        s
    };
    if !got.eq_ignore_ascii_case(sha256) {
        bail!("{what} checksum mismatch for {url}\n  expected sha256: {sha256}\n  got:      sha256: {got}");
    }
    Ok(bytes)
}

fn extract_zip(bytes: &[u8], dest: &std::path::Path) -> Result<()> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).context("reading zip archive")?;
    archive.extract(dest).context("extracting zip")?;
    normalize_single_root(dest)?;
    Ok(())
}

fn extract_tar_gz(bytes: &[u8], dest: &std::path::Path) -> Result<()> {
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
    let mut archive = tar::Archive::new(gz);
    archive.unpack(dest).context("extracting tar.gz")?;
    normalize_single_root(dest)?;
    Ok(())
}

/// Codeload-style archives wrap everything in a single top-level `repo-<sha>/` directory. If `dest`
/// contains exactly one entry and it is a directory, hoist its contents up one level so `#Import`
/// names resolve without a spurious path segment.
fn normalize_single_root(dest: &std::path::Path) -> Result<()> {
    let entries: Vec<_> = std::fs::read_dir(dest)
        .with_context(|| format!("reading {}", dest.display()))?
        .collect::<std::result::Result<_, _>>()?;
    if entries.len() != 1 || !entries[0].file_type()?.is_dir() {
        return Ok(());
    }
    let root = entries[0].path();
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        std::fs::rename(entry.path(), to)?;
    }
    std::fs::remove_dir_all(&root).ok();
    Ok(())
}

fn git(args: &[&str]) -> Result<()> {
    tracing::debug!(cmd = %format!("git {}", args.join(" ")), "running git");
    let status = Command::new("git")
        .args(args)
        .status()
        .context("running git (is it installed and on PATH?)")?;
    anyhow::ensure!(status.success(), "git {:?} failed (exit {})", args, status);
    Ok(())
}

fn git_in(dir: &std::path::Path, args: &[&str]) -> Result<()> {
    tracing::debug!(cmd = %format!("git {}", args.join(" ")), dir = %dir.display(), "running git");
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .context("running git")?;
    anyhow::ensure!(status.success(), "git {:?} failed (exit {})", args, status);
    Ok(())
}

fn git_capture(dir: &std::path::Path, args: &[&str]) -> Result<String> {
    tracing::debug!(cmd = %format!("git {}", args.join(" ")), dir = %dir.display(), "running git");
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .context("running git")?;
    anyhow::ensure!(
        out.status.success(),
        "git {:?} failed (exit {})",
        args,
        out.status
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
