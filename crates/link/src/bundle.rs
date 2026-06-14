//! Backend-neutral bundle plan and the single-`.ahk` emitter.
//!
//! The linker produces a [`BundlePlan`]: an ordered list of module units (one per group)
//! and the module name each gets in the output. This is deliberately backend-agnostic — the
//! `.ahk` emitter here turns each non-entry unit into a `#Module Name` block, and a future
//! `.exe` emitter would turn the same units into RCDATA resources named `Name`.
//!
//! v1 emits each group's whole original source text (a group is currently one file). It does
//! not yet strip comments, fold constants, or drop tree-shaken nodes — those become a
//! node/span-level emission pass layered on top. It also does not rename same-name modules
//! across groups (the linker warns; see the runtime probes for why that only matters here,
//! not at runtime).

use ahkbuild_ir::{GroupId, Program};

/// An ordered, backend-neutral description of how groups become output modules.
#[derive(Clone, Debug)]
pub struct BundlePlan {
    /// Units in emission order. `units[0]` is the entry group.
    pub units: Vec<BundleUnit>,
}

/// One group's placement in the bundle.
#[derive(Clone, Debug)]
pub struct BundleUnit {
    pub group: GroupId,
    /// The module name this group takes in the output: `None` for the entry group (it stays
    /// the implicit `__Main`), `Some(name)` for an imported group — emitted as `#Module
    /// name` (`.ahk`) or a resource named `name` (`.exe`).
    pub module_name: Option<String>,
}

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
