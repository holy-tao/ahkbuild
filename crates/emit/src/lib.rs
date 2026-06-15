//! Output emitters that consume the linker's backend-neutral [`BundlePlan`].
//!
//! This crate is the home for every emission backend. The single-`.ahk` emitter
//! ([`emit_ahk`]) lives here today; the planned `.exe` emitter (RCDATA injection, asset
//! embedding, resource naming) will land as a separate, dependency-heavy sibling crate so it
//! can pull in PE/Win32 machinery without weighing down this portable, text-only path or the
//! `link` crate that produces the plan.
//!
//! Emission is span-level, not a re-serialization of the IR: each group starts from its
//! original source text and the emitter splices in [`Edit`](patch::Edit)s for the nodes it
//! needs to change (see [`patch`]). Today the only rewrite is import redirection; comment
//! stripping, constant folding and tree-shaking will plug in as further edit producers.

pub mod patch;

use std::collections::HashMap;

use ahkbuild_ir::node::ImportSource;
use ahkbuild_ir::{FileId, GroupId, NodeKind, Program};
use ahkbuild_link::BundlePlan;

use patch::{apply_edits, Edit};

/// Emit a single self-contained `.ahk` bundle: the entry group's source, then each imported
/// group wrapped in a `#Module Name` block. Every resolved `#Import` is rewritten to name the
/// in-file module instead of a file/path, so the bundle resolves entirely in-process
/// (in-file modules take precedence over the filesystem).
///
/// What it does *not* do yet: rename same-name modules that collide once merged into one file
/// (the linker warns), strip comments, fold constants, or drop tree-shaken nodes. Those are
/// future [`Edit`] producers over the same per-group text.
pub fn emit_ahk(program: &Program, plan: &BundlePlan) -> String {
    let edits = import_edits(program, plan);

    let mut out = String::new();
    for unit in &plan.units {
        let group = &program.groups[unit.group.0 as usize];
        let file = program.sources.file(group.file);
        let group_edits = edits.get(&unit.group).map(Vec::as_slice).unwrap_or(&[]);
        let text = apply_edits(&file.text, file.base, group_edits);

        match &unit.module_name {
            None => out.push_str(&text),
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
                out.push_str(&text);
            }
        }
    }
    out
}

/// Build the per-group source edits that redirect each resolved `#Import` to its target
/// group's in-file module name. Keyed by the *importing* group (the one whose text the edit
/// lands in), since edits are applied per group.
fn import_edits(program: &Program, plan: &BundlePlan) -> HashMap<GroupId, Vec<Edit>> {
    // group -> assigned module name (only non-entry groups have one).
    let name_by_group: HashMap<GroupId, &str> = plan
        .units
        .iter()
        .filter_map(|u| u.module_name.as_deref().map(|n| (u.group, n)))
        .collect();
    // file -> group (the layout is one file per group today, so this is 1:1).
    let group_by_file: HashMap<FileId, GroupId> =
        program.groups.iter().map(|g| (g.file, g.id)).collect();

    let mut edits: HashMap<GroupId, Vec<Edit>> = HashMap::new();
    for ri in &plan.resolved_imports {
        let Some(&target) = name_by_group.get(&ri.group) else {
            continue;
        };
        // The span of the import's source spec — a bare name or a quoted path/string.
        let NodeKind::ImportDirective(directive) = &program.arena[ri.node].kind else {
            continue;
        };
        let spec_span = match &directive.source {
            ImportSource::Name(s) | ImportSource::Path(s) => *s,
        };
        // Already spelled exactly as the target module name: nothing to rewrite.
        if program.span_text(spec_span) == target {
            continue;
        }
        let importing_file = program.sources.file_at(spec_span.start).id;
        let Some(&importing_group) = group_by_file.get(&importing_file) else {
            continue;
        };
        edits
            .entry(importing_group)
            .or_default()
            .push(Edit::new(spec_span, target));
    }
    edits
}
