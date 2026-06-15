//! Output emitters that consume the linker's backend-neutral [`BundlePlan`].
//!
//! This crate is the home for every emission backend. The single-`.ahk` emitter
//! ([`emit_ahk`]) lives here today; the planned `.exe` emitter (RCDATA injection, asset
//! embedding, resource naming) will land as a separate, dependency-heavy sibling crate so it
//! can pull in PE/Win32 machinery without weighing down this portable, text-only path or the
//! `link` crate that produces the plan.
//!
//! `emit_ahk` emits each group's whole original source text (a group is currently one file).
//! It does not yet strip comments, fold constants, or drop tree-shaken nodes — those become a
//! node/span-level emission pass layered on top. It also does not rename same-name modules
//! across groups (the linker warns; see the runtime probes for why that only matters here,
//! not at runtime).

use ahkbuild_ir::Program;
use ahkbuild_link::BundlePlan;

/// Emit a single self-contained `.ahk` bundle: the entry group's source, then each imported
/// group wrapped in a `#Module Name` block so the entry's `#Import Name` re-targets to it
/// (in-file modules take precedence over the filesystem).
pub fn emit_ahk(program: &Program, plan: &BundlePlan) -> String {
    let mut out = String::new();
    for unit in &plan.units {
        let group = &program.groups[unit.group.0 as usize];
        let text = &program.sources.file(group.file).text;
        match &unit.module_name {
            None => out.push_str(text),
            Some(name) => {
                // Blank-line separation, then the module header on its own line.
                if !out.is_empty() {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push('\n');
                }
                out.push_str("#Module ");
                out.push_str(name);
                out.push('\n');
                out.push_str(text);
            }
        }
    }
    out
}
