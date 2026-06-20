//! `ahkbuild` CLI. Parses an AHK file and, with `--ir`, lowers it to the IR and prints
//! the IR tree — used to eyeball lowering against real v2.1 module sources.

use std::fs;
use std::path::{Path, PathBuf};

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
    };

    result
}

/// Bundle into a single .ahk file
fn bundle_ahk(
    input: &Path,
    output: &Option<PathBuf>,
    tree_shake: bool,
    compiled: Option<bool>,
    bitness: Option<u8>,
    emit_options: &ahkbuild_emit::EmitOptions,
) -> Result<()> {
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

    // Build-time constants. `A_PtrSize` is taken from `--bitness`, else from a bitness-pinned
    // `#Requires` (a certainty when present). `A_IsCompiled` is folded only when `--compiled`
    // is given — for `ahk` we assume nothing, since a bundle may later be compiled with ahk2exe.
    let ptr_size = match bitness {
        Some(32) => Some(4),
        Some(64) => Some(8),
        Some(other) => anyhow::bail!("invalid --bitness {other}; expected 32 or 64"),
        None => ahkbuild_fold::ptr_size_from_requires(&out.program),
    };
    let consts = ahkbuild_fold::Constants {
        is_compiled: compiled,
        ptr_size,
    };

    // Hand off to the fixpoint driver, which runs constant folding and tree-shaking (when
    // `tree_shake` is set; `--no-tree-shake` opts out for a faithful bundle) to a fixpoint and
    // emits the final bundle. Pure-constant conditions (`if 2 + 2 == 4`) fold regardless of the
    // flags; `A_IsCompiled` folds only when `--compiled` made its value known.
    let bundled = ahkbuild_pipeline::bundle_ahk(out, consts, tree_shake, emit_options)?;

    match output {
        Some(path) => {
            fs::write(path, bundled)?;
        }
        None => {
            print!("{}", bundled);
        }
    }
    Ok(())
}
