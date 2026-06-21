use anyhow::{Context, Result};

use crate::AhkVersion;

/// Returns the browser download URL for the AutoHotkey zip in the GitHub release for `version`.
pub fn release_zip_url(version: &AhkVersion) -> Result<String> {
    let tag = format!("v{}", version.canonical());
    let url = format!(
        "https://api.github.com/repos/AutoHotkey/AutoHotkey/releases/tags/{}",
        tag
    );

    let mut req = ureq::get(&url)
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .set("User-Agent", "ahkbuild");

    // Use GitHub token if we have it
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.set("Authorization", &format!("Bearer {}", token));
    }

    let json: serde_json::Value = req
        .call()
        .with_context(|| format!("GET {}", url))?
        .into_json()
        .context("parsing GitHub release JSON")?;

    let assets = json["assets"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no 'assets' array in GitHub release response"))?;

    for asset in assets {
        let name = asset["name"].as_str().unwrap_or("");
        if name.starts_with("AutoHotkey_") && name.ends_with(".zip") {
            let download_url = asset["browser_download_url"].as_str().ok_or_else(|| {
                anyhow::anyhow!("missing browser_download_url for asset {}", name)
            })?;
            return Ok(download_url.to_string());
        }
    }

    anyhow::bail!("no AutoHotkey zip asset found in GitHub release {}", tag)
}
