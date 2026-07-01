//! Logging/diagnostics setup for the `ahkbuild` CLI.

use std::fs::File;
use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

/// The env var that controls per-target filtering, e.g. `AHKBUILD_LOG=ahkbuild_link=debug`.
/// Falls back to `RUST_LOG` so the conventional knob also works.
const ENV_FILTER_VAR: &str = "AHKBUILD_LOG";

/// Map a `-v` repeat count (and `--quiet`) to a default level for the console.
fn console_level(verbosity: u8, quiet: bool) -> LevelFilter {
    if quiet {
        return LevelFilter::ERROR;
    }
    match verbosity {
        0 => LevelFilter::WARN,
        1 => LevelFilter::INFO,
        2 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    }
}

/// Build the console env filter. An explicit `AHKBUILD_LOG` (or, failing that, `RUST_LOG`)
/// takes precedence for fine-grained per-target control; otherwise we fall back to the
/// single level derived from `-v`/`--quiet`.
fn console_filter(verbosity: u8, quiet: bool) -> EnvFilter {
    let from_env = std::env::var(ENV_FILTER_VAR)
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok()
        .filter(|s| !s.is_empty());
    match from_env {
        Some(directives) => EnvFilter::new(directives),
        None => EnvFilter::new(console_level(verbosity, quiet).to_string()),
    }
}

/// Install the global tracing subscriber. Call once, before any command runs.
///
/// `verbosity` is the `-v` repeat count, `quiet` is `--quiet`, and `log_file`, when set,
/// adds a second sink that captures a full debug-level, timestamped, ANSI-free trail
/// regardless of how quiet the console is.
pub fn init(verbosity: u8, quiet: bool, log_file: Option<&Path>) -> Result<()> {
    let stderr_layer = fmt::layer()
        .with_writer(io::stderr)
        .without_time()
        // The originating target (crate/module) is noise at the default level; surface it
        // only once the user has asked for detail (`-vv`+).
        .with_target(verbosity >= 2)
        .with_filter(console_filter(verbosity, quiet));

    let file_layer = log_file
        .map(|path| -> Result<_> {
            let file = File::create(path)
                .with_context(|| format!("opening log file {}", path.display()))?;
            // The file is a diagnostic record: keep timestamps and targets, drop ANSI, and
            // capture at least debug detail no matter the console verbosity.
            Ok(fmt::layer()
                .with_writer(file)
                .with_ansi(false)
                .with_target(true)
                .with_filter(LevelFilter::DEBUG))
        })
        .transpose()?;

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();

    Ok(())
}
