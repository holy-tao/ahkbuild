//! `ahkbuild package restore`: resolve and pin dependencies, populate the store, and build the
//! per-project link-farm.

use std::path::Path;

use ahkbuild_pkg::{RestoreOptions, RestoreReport};
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
