//! `ahkbuild package restore`: resolve and pin dependencies, populate the store, and build the
//! per-project link-farm.

use std::path::Path;

use ahkbuild_pkg::{PackageStatus, RestoreOptions, RestoreReport};
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
