//! The `ahkbuild package` subcommands: restore, list, update, prune, add, remove, verify, and trust.

use std::path::Path;

use ahkbuild_pkg::{
    PackageStatus, PruneReport, RestoreOptions, RestoreReport, StorePackage, UpdateReport,
    VerifyReport,
};
use anyhow::{bail, Result};
use serde_json::{Map, Value};

/// The source and option flags for `package add`, mirroring a manifest dependency's shape.
#[derive(Default)]
pub(crate) struct AddSpec {
    pub git: Option<String>,
    pub gist: Option<String>,
    pub tarball: Option<String>,
    pub release: Option<String>,
    pub path: Option<String>,
    pub tag: Option<String>,
    pub branch: Option<String>,
    pub rev: Option<String>,
    pub asset: Option<String>,
    pub sha256: Option<String>,
    pub subdir: Option<String>,
    pub alias: Option<String>,
}

pub(crate) fn restore(config_path: Option<&Path>, locked: bool) -> Result<()> {
    let (config, project_root) = crate::config_util::load(config_path)?;
    let report = ahkbuild_pkg::restore(&config, &project_root, RestoreOptions { locked })?;
    report_summary(&report);
    Ok(())
}

fn report_summary(report: &RestoreReport) {
    tracing::info!(
        restored = report.restored,
        fetched = report.fetched,
        lock_changed = report.lock_changed,
        "restore complete"
    );
    let noun = if report.restored == 1 {
        "dependency"
    } else {
        "dependencies"
    };
    println!(
        "Restored {} {noun} ({} fetched{}).",
        report.restored,
        report.fetched,
        if report.lock_changed {
            ", lockfile updated"
        } else {
            ""
        }
    );
}

pub(crate) fn list(config_path: Option<&Path>) -> Result<()> {
    let (config, project_root) = crate::config_util::load(config_path)?;
    let statuses = ahkbuild_pkg::list(&config, &project_root)?;
    if statuses.is_empty() {
        println!("No dependencies declared in ahkbuild.json.");
        return Ok(());
    }

    let noun = if statuses.len() == 1 {
        "dependency"
    } else {
        "dependencies"
    };
    println!("{} {noun}:", statuses.len());
    for s in &statuses {
        println!();
        if s.import_name == s.name {
            println!("  {}", s.name);
        } else {
            println!("  {}  (imported as {})", s.name, s.import_name);
        }
        println!("    source:  {}", s.source);
        println!("    status:  {}", status_line(s));
    }
    Ok(())
}

pub(crate) fn update(config_path: Option<&Path>, names: &[String]) -> Result<()> {
    let (config, project_root) = crate::config_util::load(config_path)?;
    let report = ahkbuild_pkg::update(&config, &project_root, names)?;
    report_update(&report);
    Ok(())
}

fn report_update(report: &UpdateReport) {
    let moved: Vec<_> = report.changes.iter().filter(|c| c.moved).collect();
    for c in &moved {
        let from = c
            .from
            .as_deref()
            .map(short_rev)
            .unwrap_or_else(|| "(new)".to_string());
        println!("  {}: {} -> {}", c.name, from, short_rev(&c.to));
    }
    for name in &report.skipped {
        println!("  {name}: pinned by ahkbuild.json, not updated");
    }

    let n = moved.len();
    if n == 0 {
        println!("Everything is already up to date.");
    } else {
        let noun = if n == 1 { "dependency" } else { "dependencies" };
        println!("Updated {n} {noun}.");
    }
}

/// A one-line "pin - present - linked" summary for a dependency.
fn status_line(s: &PackageStatus) -> String {
    let pin = if s.local {
        "local".to_string()
    } else {
        match &s.resolved {
            Some(r) => format!("pinned {}", short_rev(r)),
            None => "unpinned (run `ahkbuild package restore`)".to_string(),
        }
    };
    let present = match (s.local, s.present) {
        (true, true) => "path exists",
        (true, false) => "path missing",
        (false, true) => "in store",
        (false, false) => "not fetched",
    };
    let link = if s.linked { "linked" } else { "not linked" };
    format!("{pin} - {present} - {link}")
}

/// Abbreviate a 40-hex git/gist commit; leave a URL (tarball/release `resolved`) intact.
fn short_rev(rev: &str) -> String {
    if rev.len() >= 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()) {
        rev[..12].to_string()
    } else {
        rev.to_string()
    }
}

pub(crate) fn list_global() -> Result<()> {
    let packages = ahkbuild_pkg::list_store()?;
    if packages.is_empty() {
        println!("The package store is empty.");
        return Ok(());
    }

    let total: u64 = packages.iter().map(|p| p.size).sum();
    let noun = if packages.len() == 1 {
        "entry"
    } else {
        "entries"
    };
    println!("{} store {noun} ({}):", packages.len(), human_size(total));
    for p in &packages {
        println!();
        print_store_package(p);
    }
    Ok(())
}

fn print_store_package(p: &StorePackage) {
    let title = if p.names.is_empty() {
        "(untracked)".to_string()
    } else {
        p.names.join(", ")
    };
    let orphan = if p.refs == 0 { "  [orphan]" } else { "" };
    println!(
        "  {title}  {}  ({}){orphan}",
        &p.hash[..12],
        human_size(p.size)
    );
    if let Some(src) = &p.source {
        println!("    source:  {src}");
    }
    if let Some(rev) = &p.resolved {
        println!("    rev:     {}", short_rev(rev));
    }
    let projects = if p.refs == 1 { "project" } else { "projects" };
    println!("    used by: {} {projects}", p.refs);
}

pub(crate) fn prune(dry_run: bool, include_untracked: bool) -> Result<()> {
    // Register the project we are standing in (if any) so its live entries are never pruned, even
    // when it has not been restored since the index was introduced.
    let current = std::env::current_dir()
        .ok()
        .and_then(|cwd| ahkbuild_config::find_config(&cwd).ok().flatten())
        .and_then(|cfg| cfg.parent().map(Path::to_path_buf));

    let report = ahkbuild_pkg::prune(current.as_deref(), dry_run, include_untracked)?;
    report_prune(&report);
    Ok(())
}

fn report_prune(report: &PruneReport) {
    for e in &report.removed {
        let title = if e.names.is_empty() {
            "(untracked)".to_string()
        } else {
            e.names.join(", ")
        };
        println!("  {title}  {}  ({})", &e.hash[..12], human_size(e.size));
    }

    let n = report.removed.len();
    if n == 0 {
        println!("Nothing to prune; the store holds no unreferenced packages.");
    } else if report.dry_run {
        let noun = if n == 1 { "entry" } else { "entries" };
        println!(
            "Would remove {n} {noun}, freeing {}. Re-run without --dry-run to apply.",
            human_size(report.freed)
        );
    } else {
        let noun = if n == 1 { "entry" } else { "entries" };
        println!("Removed {n} {noun}, freed {}.", human_size(report.freed));
    }
}

pub(crate) fn add(config_path: Option<&Path>, name: &str, spec: AddSpec) -> Result<()> {
    let config_file = crate::config_util::locate(config_path)?;

    // Build the source object from the set flags; the manifest deserializer (invoked inside
    // `add_dependency`) enforces exactly-one-source and every per-source rule, so we don't re-check.
    let mut obj = Map::new();
    let mut put = |key: &str, val: &Option<String>| {
        if let Some(v) = val {
            obj.insert(key.to_string(), Value::String(v.clone()));
        }
    };
    put("git", &spec.git);
    put("gist", &spec.gist);
    put("tarball", &spec.tarball);
    put("release", &spec.release);
    put("path", &spec.path);
    put("tag", &spec.tag);
    put("branch", &spec.branch);
    put("rev", &spec.rev);
    put("asset", &spec.asset);
    put("sha256", &spec.sha256);
    put("subdir", &spec.subdir);
    put("alias", &spec.alias);

    ahkbuild_config::add_dependency(&config_file, name, Value::Object(obj))?;
    println!(
        "Added {name} to {}. Run `ahkbuild package restore` to fetch it.",
        config_file.display()
    );
    Ok(())
}

pub(crate) fn remove(config_path: Option<&Path>, names: &[String]) -> Result<()> {
    let config_file = crate::config_util::locate(config_path)?;
    let config = ahkbuild_config::load(&config_file)?;
    let project_root = crate::config_util::project_root(&config_file);

    // Capture (manifest name, import name) before editing so we can also drop the lock entry and
    // unlink the farm. Reject unknown names up front so a typo changes nothing.
    let mut deps = Vec::new();
    let mut missing = Vec::new();
    for name in names {
        match config.dependencies.get(name) {
            Some(spec) => deps.push((name.clone(), spec.import_name(name).to_string())),
            None => missing.push(name.clone()),
        }
    }
    if !missing.is_empty() {
        bail!(
            "no such dependency in ahkbuild.json: {}",
            missing.join(", ")
        );
    }

    for (name, _) in &deps {
        ahkbuild_config::remove_dependency(&config_file, name)?;
    }
    ahkbuild_pkg::forget(&project_root, &deps)?;

    let noun = if deps.len() == 1 {
        "dependency"
    } else {
        "dependencies"
    };
    println!("Removed {} {noun}.", deps.len());
    Ok(())
}

pub(crate) fn verify(config_path: Option<&Path>) -> Result<()> {
    let (config, project_root) = crate::config_util::load(config_path)?;
    let report = ahkbuild_pkg::verify(&config, &project_root)?;
    report_verify(&report)
}

fn report_verify(report: &VerifyReport) -> Result<()> {
    for issue in &report.issues {
        println!("  {}: {}", issue.name, issue.problem);
    }
    if report.ok() {
        let noun = if report.verified == 1 {
            "dependency"
        } else {
            "dependencies"
        };
        println!("All {} {noun} verified.", report.verified);
        Ok(())
    } else {
        let n = report.issues.len();
        let noun = if n == 1 { "problem" } else { "problems" };
        bail!("{n} {noun} found; run `ahkbuild package restore` to repair");
    }
}

/// Record an out-of-band trust entry for a dependency in `ahkbuild.trust.json`, so tree-shaking
/// treats its dynamic constructs (`%deref%`, dynamic member access/calls) as safe instead of
/// conservatively keeping the module whole. The package's current lock checksum is stored with the
/// entry: a later upgrade changes the checksum and silently invalidates the trust, forcing the
/// author to re-vouch. With no `files`, the whole package is trusted.
pub(crate) fn trust(
    config_path: Option<&Path>,
    package: &str,
    files: &[String],
    reason: Option<String>,
) -> Result<()> {
    let (config, project_root) = crate::config_util::load(config_path)?;

    let Some(spec) = config.dependencies.get(package) else {
        bail!("no dependency named {package:?} in ahkbuild.json");
    };
    if matches!(spec.source, ahkbuild_config::DependencySource::Path { .. }) {
        bail!(
            "{package:?} is a `path` dependency; it is mutable, so annotate the dynamic code \
             in-source with `;@ahkbuild-safe` instead of adding a trust entry"
        );
    }

    let lock = ahkbuild_pkg::Lockfile::load(&project_root)?.unwrap_or_default();
    let Some(checksum) = lock.get(package).map(|e| e.checksum.clone()) else {
        bail!("{package:?} is not pinned in ahkbuild.lock; run `ahkbuild package restore` first");
    };

    let mut trust_file = ahkbuild_config::TrustFile::load(&project_root)?.unwrap_or_default();
    // Upsert: a package has at most one entry, so replace any existing one.
    trust_file.trust.retain(|e| e.package != package);
    trust_file.trust.push(ahkbuild_config::TrustEntry {
        package: package.to_string(),
        checksum,
        files: files.to_vec(),
        reason,
    });
    trust_file.normalized().save(&project_root)?;

    let scope = if files.is_empty() {
        "the whole package".to_string()
    } else if files.len() == 1 {
        files[0].clone()
    } else {
        format!("{} files", files.len())
    };
    println!(
        "Trusted {package} ({scope}) in {}.",
        ahkbuild_config::TRUST_NAME
    );
    Ok(())
}

/// Format a byte count as a compact human-readable size (e.g. `4.2 MB`).
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}
