//! `ahkbuild` CLI. Parses an AHK file and, with `--ir`, lowers it to the IR and prints
//! the IR tree - used to eyeball lowering against real v2.1 module sources.

use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};

use ahkbuild_interpret::{AhkVersion, Bitness};

mod bundle;
mod bundle_exe;
mod config_util;
mod logging;
mod package;
mod run;
mod scripts;

use bundle::bundle_ahk;
use bundle_exe::bundle_exe;

#[derive(Parser)]
#[command(
    name = "ahkbuild",
    about = "AutoHotkey v2.1 module-aware bundler (WIP)"
)]
struct Cli {
    /// Increase log verbosity (repeatable): `-v` info, `-vv` debug, `-vvv` trace. Overridden
    /// by the AHKBUILD_LOG (or RUST_LOG) env filter when set.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Silence all diagnostics except errors. Takes precedence over `-v`.
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Also write a timestamped, debug-level log to this file (in addition to stderr).
    #[arg(long, value_name = "PATH", global = true)]
    log_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
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
        /// The AHK version to remove. If unspecified, all versions are removed
        #[arg(long)]
        version: Option<AhkVersion>,

        /// The bitness of the interpreter to remove. If unspecified, both are removed
        #[arg(long)]
        bitness: Option<Bitness>,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum BundleCommand {
    /// Bundle to a single self-contained .ahk file
    Ahk {
        /// The entry script to bundle.
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
        /// default (a bundle may later be compiled with ahk2exe).
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

        /// Parse the main file and print the sexp.
        #[cfg(debug_assertions)]
        #[arg(long)]
        sexp: bool,
    },
    /// Bundle to a standalone Windows .exe
    Exe {
        /// Path to ahkbuild.json. If omitted, the file is discovered by walking up from cwd.
        #[arg(long)]
        config: Option<PathBuf>,

        /// Entry script. Overrides the `entry` field in ahkbuild.json.
        #[arg(long)]
        input: Option<PathBuf>,

        /// Output file. Defaults to `<exe.name>.exe` from config, or `<entry-stem>.exe`.
        #[arg(long, short)]
        output: Option<PathBuf>,

        /// Interpreter version. Overrides `interpreter.version` in ahkbuild.json.
        #[arg(long)]
        interpreter_version: Option<AhkVersion>,

        /// Target bitness (32 or 64). Overrides `interpreter.bitness` in ahkbuild.json.
        #[arg(long)]
        bitness: Option<u8>,

        /// Disable tree-shaking.
        #[arg(long)]
        no_tree_shake: bool,

        /// Keep comments in the embedded scripts.
        #[arg(long)]
        keep_comments: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum PackageCommand {
    /// Resolve, pin, and fetch dependencies, then build the per-project link-farm
    Restore {
        /// Path to ahkbuild.json. If omitted, the file is discovered by walking up from cwd.
        #[arg(long)]
        config: Option<PathBuf>,

        /// CI mode: fail if the lockfile is missing or would change, instead of updating it.
        #[arg(long)]
        locked: bool,
    },
    /// List declared dependencies with their pinned revision and fetch/link status
    List {
        /// List packages in the global store instead of this project's dependencies. Ignores `--config`.
        #[arg(long)]
        global: bool,

        /// Path to ahkbuild.json. If omitted, the file is discovered by walking up from cwd.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Re-resolve floating (git/gist) dependencies to their latest revision and update the lock
    Update {
        /// Dependencies to update. If omitted, every updatable dependency is refreshed.
        names: Vec<String>,

        /// Path to ahkbuild.json. If omitted, the file is discovered by walking up from cwd.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Remove unreferenced global packages
    Prune {
        /// Report what would be removed without deleting anything.
        #[arg(long)]
        dry_run: bool,

        /// Also remove store directories not in the index.
        #[arg(long)]
        include_untracked: bool,
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
    /// Bundle a script into a .ahk file or standalone .exe
    Bundle {
        #[command(subcommand)]
        cmd: BundleCommand,
    },
    /// Manage ahkbuild managed interpreters
    Interpreter {
        #[command(subcommand)]
        command: InterpreterCommand,
    },
    /// Manage module dependencies (`ahkbuild.json` -> `ahkbuild.lock`)
    Package {
        #[command(subcommand)]
        cmd: PackageCommand,
    },
    /// Run an entry script under the configured interpreter, with dependencies resolved
    Run {
        /// Entry script. Overrides the `entry` field in ahkbuild.json.
        entry: Option<PathBuf>,

        /// Load the script, but do not execute it.
        #[arg(long)]
        validate: bool,

        /// Path to ahkbuild.json. If omitted, the file is discovered by walking up from cwd.
        #[arg(long)]
        config: Option<PathBuf>,

        /// Interpreter version. Overrides `interpreter.version` in ahkbuild.json.
        #[arg(long)]
        interpreter_version: Option<AhkVersion>,

        /// Target bitness (32 or 64). Overrides `interpreter.bitness` in ahkbuild.json.
        #[arg(long)]
        bitness: Option<u8>,

        /// Arguments passed through to the script (everything after `--`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    logging::init(cli.verbose, cli.quiet, cli.log_file.as_deref())?;

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
        Commands::Bundle { cmd } => match cmd {
            BundleCommand::Ahk {
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
                #[cfg(debug_assertions)]
                if *ir || *sexp {
                    let raw = std::fs::read_to_string(input)
                        .with_context(|| format!("reading {}", input.display()))?;
                    let source = ahkbuild_preprocess::run(&input.to_string_lossy(), &raw)?;
                    let tree =
                        ahkbuild_syntax::parse(&source).context("parser returned no tree")?;
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

                let emit_options = ahkbuild_emit::EmitOptions {
                    strip_comments: !keep_comments,
                    whitespace: ahkbuild_emit::WsLevel::Readable,
                };
                bundle_ahk(
                    input,
                    output,
                    !no_tree_shake,
                    *compiled,
                    *bitness,
                    &emit_options,
                )
            }
            BundleCommand::Exe {
                config,
                input,
                output,
                interpreter_version,
                bitness,
                no_tree_shake,
                keep_comments,
            } => {
                let bitness_enum = match bitness {
                    Some(32) => Some(Bitness::X32),
                    Some(64) => Some(Bitness::X64),
                    Some(other) => anyhow::bail!("invalid --bitness {other}; expected 32 or 64"),
                    None => None,
                };
                bundle_exe(
                    config.as_deref(),
                    input.as_deref(),
                    output.as_deref(),
                    interpreter_version.clone(),
                    bitness_enum,
                    !no_tree_shake,
                    *keep_comments,
                )
            }
        },
        Commands::Interpreter { command } => match command {
            InterpreterCommand::Install { version, bitness } => {
                let targets = match bitness {
                    Some(b) => vec![b.clone()],
                    None => vec![Bitness::X32, Bitness::X64],
                };
                for b in targets {
                    let path = ahkbuild_interpret::install(version, &b)?;
                    println!("{}", path.display());
                }
                Ok(())
            }
            InterpreterCommand::List => {
                let entries = ahkbuild_interpret::list()?;
                if entries.is_empty() {
                    println!("No cached interpreters.");
                } else {
                    for e in entries {
                        let bits: Vec<&str> = e
                            .bitnesses
                            .iter()
                            .map(|b| match b {
                                Bitness::X32 => "x32",
                                Bitness::X64 => "x64",
                            })
                            .collect();
                        println!(
                            "{:<20} {}  ({})",
                            e.version,
                            bits.join("  "),
                            e.dir.display()
                        );
                    }
                }
                Ok(())
            }
            InterpreterCommand::Prune { version, bitness } => {
                let n = ahkbuild_interpret::prune(version.as_ref(), bitness.as_ref())?;
                match n {
                    0 => println!("Nothing to remove."),
                    1 => println!("Removed 1 entry."),
                    _ => println!("Removed {} entries.", n),
                }
                Ok(())
            }
        },
        Commands::Package { cmd } => match cmd {
            PackageCommand::Restore { config, locked } => {
                package::restore(config.as_deref(), *locked)
            }
            PackageCommand::List { global, config } => {
                if *global {
                    package::list_global()
                } else {
                    package::list(config.as_deref())
                }
            }
            PackageCommand::Update { names, config } => package::update(config.as_deref(), names),
            PackageCommand::Prune {
                dry_run,
                include_untracked,
            } => package::prune(*dry_run, *include_untracked),
        },
        Commands::Run {
            entry,
            validate,
            config,
            interpreter_version,
            bitness,
            args,
        } => {
            let bitness_enum = match bitness {
                Some(32) => Some(Bitness::X32),
                Some(64) => Some(Bitness::X64),
                Some(other) => anyhow::bail!("invalid --bitness {other}; expected 32 or 64"),
                None => None,
            };
            run::run(
                config.as_deref(),
                entry.as_deref(),
                validate,
                interpreter_version.clone(),
                bitness_enum,
                args,
            )
        }
    };

    result
}
