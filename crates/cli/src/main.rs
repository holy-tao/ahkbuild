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

    /// Resolve `#Import`s from this entry file (module-graph linker) instead of treating
    /// the file as a single already-preprocessed script. Combine with `--ir` to print the
    /// linked multi-group IR.
    #[arg(long)]
    link: bool,

    /// Link the entry file and emit a single self-contained `.ahk` bundle to stdout.
    /// Implies `--link`.
    #[arg(long)]
    bundle: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.link || cli.bundle {
        return run_link(&cli);
    }

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

/// Drive the module-graph linker from an entry file.
fn run_link(cli: &Cli) -> Result<()> {
    let script_dir = cli
        .file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let builtins = ahkbuild_link::Builtins::detect(script_dir);
    let search = ahkbuild_link::SearchPath::from_env(&builtins);

    let out = ahkbuild_link::link_entry(&cli.file, &search)?;

    eprintln!(
        "linked {} ({} groups, {} warnings)",
        cli.file.display(),
        out.program.groups.len(),
        out.warnings.len(),
    );
    for w in &out.warnings {
        eprintln!("warning: {w}");
    }

    if cli.bundle {
        print!("{}", ahkbuild_link::emit_ahk(&out.program, &out.plan));
    }

    if cli.ir {
        print!("{}", ahkbuild_ir::print_program(&out.program));
    }

    Ok(())
}
