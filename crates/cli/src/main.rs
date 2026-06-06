//! `ahkbuild` CLI. Parses an AHK file and, with `--ir`, lowers it to the IR and prints
//! the IR tree — used to eyeball lowering against real v2.1 module sources.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "ahkbuild",
    about = "AutoHotkey v2.1 module-aware bundler (WIP)"
)]
struct Cli {
    /// AHK source file to parse.
    file: PathBuf,

    /// Print the full s-expression parse tree.
    #[arg(long)]
    sexp: bool,

    /// Lower to IR and print the IR tree.
    #[arg(long)]
    ir: bool,
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

    if cli.ir {
        let program = ahkbuild_ir::lower(&tree, &source);
        print!("{}", ahkbuild_ir::print_program(&program));
    }

    Ok(())
}
