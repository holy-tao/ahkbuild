//! `ahkbuild` CLI. Currently a parsing spike (step 0): parse an AHK file and report
//! the tree, so we can validate the grammar against real v2.1 module sources before
//! designing the IR.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;

#[derive(Parser)]
#[command(name = "ahkbuild", about = "AutoHotkey v2.1 module-aware bundler (WIP)")]
struct Cli {
    /// AHK source file to parse.
    file: PathBuf,

    /// Print the full s-expression parse tree.
    #[arg(long)]
    sexp: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let source = std::fs::read_to_string(&cli.file)
        .with_context(|| format!("reading {}", cli.file.display()))?;

    let tree = ahkbuild_syntax::parse(&source).context("parser returned no tree")?;
    let root = tree.root_node();

    if cli.sexp {
        println!("{}", root.to_sexp());
    }

    eprintln!(
        "parsed {} ({} bytes, {} top-level nodes), has_error={}",
        cli.file.display(),
        source.len(),
        root.named_child_count(),
        root.has_error(),
    );

    if root.has_error() {
        bail!("parse tree contains ERROR/MISSING nodes");
    }
    Ok(())
}
