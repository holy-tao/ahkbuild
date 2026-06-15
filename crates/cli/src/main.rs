//! `ahkbuild` CLI. Parses an AHK file and, with `--ir`, lowers it to the IR and prints
//! the IR tree — used to eyeball lowering against real v2.1 module sources.

use std::fs;
use std::path::{Path, PathBuf};

#[cfg(debug_assertions)]
use anyhow::Context;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "ahkbuild",
    about = "AutoHotkey v2.1 module-aware bundler (WIP)"
)]
struct Cli {
    #[arg(short, long, action = clap::ArgAction::Count)]
    debug: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(ValueEnum, Debug, Clone)]
enum BundleTarget {
    Ahk,
    Exe,
}

#[derive(Subcommand)]
enum Commands {
    /// Bundle a script into a single script or a .exe file
    Bundle {
        /// The output format to bundle to
        format: BundleTarget,

        /// The file to bundle.
        input: PathBuf,

        /// The output file - leave blank to print to stdout.
        output: Option<PathBuf>,

        /// Lower to IR and print the IR tree.
        #[cfg(debug_assertions)]
        #[arg(long)]
        ir: bool,

        /// Parse the main file and print the sexp
        #[cfg(debug_assertions)]
        #[arg(long)]
        sexp: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Bundle {
            format,
            input,
            output,
            #[cfg(debug_assertions)]
            ir,
            #[cfg(debug_assertions)]
            sexp,
        } => {
            // If we've got one of the diagnostic flags, do that and exit
            #[cfg(debug_assertions)]
            if *ir || *sexp {
                let source = std::fs::read_to_string(input)
                    .with_context(|| format!("reading {}", input.display()))?;

                let tree = ahkbuild_syntax::parse(&source).context("parser returned no tree")?;
                let root = tree.root_node();

                if *ir {
                    let program = ahkbuild_ir::lower(&tree, &source);
                    print!("{}", ahkbuild_ir::print_program(&program));
                }

                if *sexp {
                    print!("{}", &root.to_sexp());
                }

                return Ok(());
            }

            // Otherwise run the appropriate bundler
            match format {
                BundleTarget::Exe => {
                    todo!("EXE bundling is not yet supported");
                }
                BundleTarget::Ahk => bundle_ahk(input, output),
            }
        }
    };

    match result {
        Ok(()) => Ok(()),
        Err(err) => {
            eprint!("{}", err);
            Err(err)
        }
    }
}

/// Bundle into a single .ahk file
fn bundle_ahk(input: &Path, output: &Option<PathBuf>) -> Result<()> {
    let script_dir = input
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let builtins = ahkbuild_link::Builtins::detect(script_dir);
    let search = ahkbuild_link::SearchPath::from_env(&builtins);

    // Link modules
    let out = ahkbuild_link::link_entry(input, &search)?;

    eprintln!(
        "linked {} ({} groups, {} warnings)",
        input.display(),
        out.program.groups.len(),
        out.warnings.len(),
    );

    for w in &out.warnings {
        eprintln!("warning: {w}");
    }

    let bundled = ahkbuild_emit::emit_ahk(&out.program, &out.plan);

    match output {
        Some(path) => {
            fs::write(path, bundled)?;
            Ok(())
        }
        None => {
            print!("{}", bundled);
            Ok(())
        }
    }
}
