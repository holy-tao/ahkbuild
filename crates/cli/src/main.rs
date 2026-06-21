//! `ahkbuild` CLI. Parses an AHK file and, with `--ir`, lowers it to the IR and prints
//! the IR tree — used to eyeball lowering against real v2.1 module sources.

use std::fs;
use std::path::PathBuf;

use anyhow::Context;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

use ahkbuild_interpret::AhkVersion;

mod bundle;

use bundle::bundle_ahk;

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

#[derive(ValueEnum, Debug, Clone, Eq, PartialEq)]
enum BundleTarget {
    Ahk,
    Exe,
}

#[derive(ValueEnum, Debug, Clone, Eq, PartialEq)]
enum Bitness {
    X32,
    X64,
}

#[derive(Subcommand, Debug, Clone, Eq, PartialEq)]
enum InterpreterCommand {
    /// Install an interpreter
    Install {
        /// The AHK version to install (e.g. '2.1-alpha.16' or '2.0.24')
        version: AhkVersion,

        /// The bitness of the interpreter to install. If unspecified, both are cached
        #[arg(long)]
        bitness: Option<Bitness>,
    },
    /// List the interpreters ahkbuild knows about
    List,
    /// Remove cached interpreters
    Prune {
        /// The AHK version to remove
        #[arg(long)]
        version: Option<AhkVersion>,

        /// The bitness of the interpreter to remove. If unspecified, both are removed
        #[arg(long)]
        bitness: Option<Bitness>,
    },
}

#[derive(Subcommand)]
enum Commands {
    /// Run the preprocessor over a file and emit the output
    Preprocess {
        /// The file to preprocess.
        input: PathBuf,

        /// The output file - leave blank to print to stdout.
        output: Option<PathBuf>,
    },
    /// Bundle a script into a single script or a .exe file
    Bundle {
        /// The output format to bundle to
        format: BundleTarget,

        /// The file to bundle.
        input: PathBuf,

        /// The output file - leave blank to print to stdout.
        output: Option<PathBuf>,

        /// Disable tree-shaking (dead-code elimination); emit a byte-faithful bundle.
        #[arg(long)]
        no_tree_shake: bool,

        /// Keep comments in the bundle. By default comments are stripped.
        #[arg(long)]
        keep_comments: bool,

        /// Override `A_IsCompiled` to fold build-time branches (e.g. `--compiled false`). Off by
        /// default for `ahk` (a bundle may later be compiled with ahk2exe).
        #[arg(long)]
        compiled: Option<bool>,

        /// Target bitness (32 or 64) used to fold `A_PtrSize`. Defaults from a bitness-pinned
        /// `#Requires` when present.
        #[arg(long)]
        bitness: Option<u8>,

        /// Lower to IR and print the IR tree.
        #[cfg(debug_assertions)]
        #[arg(long)]
        ir: bool,

        /// Parse the main file and print the sexp
        #[cfg(debug_assertions)]
        #[arg(long)]
        sexp: bool,
    },
    /// Manage ahkbuild managed interpreters
    Interpreter {
        #[command(subcommand)]
        command: InterpreterCommand,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Preprocess { input, output } => {
            let raw = std::fs::read_to_string(input)
                .with_context(|| format!("reading {}", input.display()))?;
            let processed = ahkbuild_preprocess::run(&input.to_string_lossy(), &raw)?;
            match output {
                Some(path) => {
                    fs::write(path, processed)?;
                }
                None => {
                    print!("{}", processed);
                }
            }
            Ok(())
        }
        Commands::Bundle {
            format,
            input,
            output,
            no_tree_shake,
            keep_comments,
            compiled,
            bitness,
            #[cfg(debug_assertions)]
            ir,
            #[cfg(debug_assertions)]
            sexp,
        } => {
            // If we've got one of the diagnostic flags, do that and exit
            #[cfg(debug_assertions)]
            if *ir || *sexp {
                let raw = std::fs::read_to_string(input)
                    .with_context(|| format!("reading {}", input.display()))?;
                let source = ahkbuild_preprocess::run(&input.to_string_lossy(), &raw)?;

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

            // Backend-neutral emit knobs, shared by both targets (comments stripped by default).
            // `.exe` output isn't read by humans, so it gets the aggressive whitespace pass.
            let emit_options = ahkbuild_emit::EmitOptions {
                strip_comments: !keep_comments,
                whitespace: match format {
                    BundleTarget::Exe => ahkbuild_emit::WsLevel::Minify,
                    BundleTarget::Ahk => ahkbuild_emit::WsLevel::Readable,
                },
            };

            // Otherwise run the appropriate bundler
            match format {
                BundleTarget::Exe => {
                    todo!("EXE bundling is not yet supported");
                }
                BundleTarget::Ahk => bundle_ahk(
                    input,
                    output,
                    !no_tree_shake,
                    *compiled,
                    *bitness,
                    &emit_options,
                ),
            }
        }
        Commands::Interpreter { command } => match command {
            InterpreterCommand::Install { version, bitness } => {
                todo!("Not implemented: install {:?} {:?}", version, bitness);
            }
            InterpreterCommand::List => {
                todo!("Not implemented: list")
            }
            InterpreterCommand::Prune { version, bitness } => {
                todo!("Not implemented: prune {:?} {:?}", version, bitness)
            }
        },
    };

    result
}
