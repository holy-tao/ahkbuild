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
//! needs to change (see [`patch`]). Two producers run today: import redirection and
//! tree-shaking deletions ([`ShakeResult`]); comment stripping and constant folding will plug
//! in the same way.

pub mod patch;

use std::collections::{HashMap, HashSet};

use ahkbuild_ir::node::ImportSource;
use ahkbuild_ir::{FileId, GroupId, NodeId, NodeKind, Program, Span};
use ahkbuild_link::BundlePlan;
use ahkbuild_shake::ShakeResult;

use patch::{apply_edits, Edit};

/// Emit a single self-contained `.ahk` bundle: the entry group's source, then each imported
/// group wrapped in a `#Module Name` block. Every resolved `#Import` is rewritten to name the
/// in-file module instead of a file/path, so the bundle resolves entirely in-process
/// (in-file modules take precedence over the filesystem).
///
/// Pass a [`ShakeResult`] to also delete dead declarations and unused imports and omit
/// fully-dead groups; pass `None` for a byte-faithful bundle. It does *not* yet rename
/// same-name modules that collide once merged into one file (the linker warns), strip
/// comments, or fold constants — future [`Edit`] producers over the same per-group text.
pub fn emit_ahk(program: &Program, plan: &BundlePlan, shake: Option<&ShakeResult>) -> String {
    // Imports the shaker dropped must not also be rewritten — they're being deleted.
    let dropped: HashSet<NodeId> = shake
        .map(|s| s.dropped_imports.iter().copied().collect())
        .unwrap_or_default();
    let dead_groups = shake
        .map(|s| fully_dead_groups(program, s))
        .unwrap_or_default();

    let mut edits = import_edits(program, plan, &dropped);
    if let Some(s) = shake {
        add_deletion_edits(program, s, &mut edits);
    }

    let mut out = String::new();
    for unit in &plan.units {
        // A group whose every module is dead is omitted entirely (its importer's `#Import`
        // is in `dropped_imports`, so nothing dangles).
        if dead_groups.contains(&unit.group) {
            continue;
        }
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

/// Groups whose every module the shaker marked dead — omit the whole unit.
fn fully_dead_groups(program: &Program, shake: &ShakeResult) -> HashSet<GroupId> {
    let dead: HashSet<NodeId> = shake.dead_modules.iter().copied().collect();
    program
        .groups
        .iter()
        .filter(|g| !g.modules.is_empty() && g.modules.iter().all(|m| dead.contains(m)))
        .map(|g| g.id)
        .collect()
}

/// Add deletion edits for every dead node and dropped import, keyed by the group whose text
/// the span falls in. (Whitespace left behind is a known cosmetic follow-up; orphaned
/// `;@` directive comments on a deleted node are harmless and left for now.)
fn add_deletion_edits(program: &Program, shake: &ShakeResult, edits: &mut HashMap<GroupId, Vec<Edit>>) {
    let group_by_file: HashMap<FileId, GroupId> =
        program.groups.iter().map(|g| (g.file, g.id)).collect();
    let delete = |span: Span, edits: &mut HashMap<GroupId, Vec<Edit>>| {
        if span.is_empty() {
            return;
        }
        if let Some(&g) = group_by_file.get(&program.sources.file_at(span.start).id) {
            edits.entry(g).or_default().push(Edit::new(span, ""));
        }
    };
    for &node in shake.dead.iter().chain(&shake.dropped_imports) {
        delete(program.arena[node].span, edits);
    }
}

/// Build the per-group source edits that redirect each resolved `#Import` to its target
/// group's in-file module name. Keyed by the *importing* group (the one whose text the edit
/// lands in), since edits are applied per group. Imports in `dropped` are skipped — they're
/// being deleted, not redirected.
fn import_edits(
    program: &Program,
    plan: &BundlePlan,
    dropped: &HashSet<NodeId>,
) -> HashMap<GroupId, Vec<Edit>> {
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
        if dropped.contains(&ri.node) {
            continue;
        }
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
