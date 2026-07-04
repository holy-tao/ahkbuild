//! The `ahkbuild package` subcommands: restore, list (project + global store), update, and prune.

use std::path::Path;

use ahkbuild_pkg::{
    PackageStatus, PruneReport, RestoreOptions, RestoreReport, StorePackage, UpdateReport,
};
use anyhow::Result;

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
