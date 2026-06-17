//! Output emitters that consume the linker's backend-neutral [`BundlePlan`].
//!
//! This crate is the home for every emission backend. The single-`.ahk` emitter
//! ([`emit_ahk`]) lives here today; the planned `.exe` emitter (RCDATA injection, asset
//! embedding, resource naming) will land as a separate, dependency-heavy sibling crate so it
//! can pull in PE/Win32 machinery without weighing down this portable, text-only path or the
//! `link` crate that produces the plan.
//!
//! Emission is span-level, not a re-serialization of the IR: each file starts from its
//! original source text and the emitter splices in [`Edit`](patch::Edit)s for the nodes it
//! needs to change (see [`patch`]). Edits are keyed by [`FileId`] — a group spans several
//! files once `#Include` is in play — and the producers run today are import redirection,
//! module renaming, tree-shaking deletions, and `#Include` splicing. A group is emitted by
//! [`expand`]ing its primary file, recursively pasting each included file's emitted text over
//! its `#Include` directive (the future `.exe` emitter instead keeps the directive and emits
//! each file once as a resource).

pub mod patch;

use std::collections::{HashMap, HashSet};

use ahkbuild_ir::node::ImportSource;
use ahkbuild_ir::{FileId, GroupId, NodeId, NodeKind, Program, Span};
use ahkbuild_link::{BundlePlan, IncludeSplice};
use ahkbuild_shake::ShakeResult;

use patch::{apply_edits, Edit};

/// Emit a single self-contained `.ahk` bundle: the entry group's source, then each imported
/// group wrapped in a `#Module Name` block, with every `#Include` spliced inline. Resolved
/// `#Import`s are rewritten to name the in-file module, every module gets a program-unique
/// name, and each included file's text is pasted over its directive (deduped repeats deleted),
/// so the bundle resolves entirely in-process.
///
/// Pass a [`ShakeResult`] to also delete dead declarations and unused imports and omit
/// fully-dead groups; pass `None` for a byte-faithful bundle.
pub fn emit_ahk(program: &Program, plan: &BundlePlan, shake: Option<&ShakeResult>) -> String {
    // Imports the shaker dropped must not also be rewritten — they're being deleted.
    let dropped: HashSet<NodeId> = shake
        .map(|s| s.dropped_imports.iter().copied().collect())
        .unwrap_or_default();
    let dead_nodes: HashSet<NodeId> = shake
        .map(|s| s.dead.iter().copied().collect())
        .unwrap_or_default();
    let dead_groups = shake
        .map(|s| fully_dead_groups(program, s))
        .unwrap_or_default();

    // Rewrite edits (import redirects + module renames) are always applied — they produce the
    // same text for every copy of a file. Tree-shaking deletions are kept separate so they can
    // be suppressed for a file spliced in more than once (see `multiply_spliced`).
    let mut rewrites = import_edits(program, plan, &dropped);
    add_rename_edits(program, plan, &mut rewrites);
    let mut deletions: HashMap<FileId, Vec<Edit>> = HashMap::new();
    if let Some(s) = shake {
        add_deletion_edits(program, s, &mut deletions);
    }

    let includes = includes_by_file(program, plan, &dead_nodes);
    let multiply = multiply_spliced(plan);

    let mut out = String::new();
    for (i, unit) in plan.units.iter().enumerate() {
        // A group whose every module is dead is omitted entirely (its importer's `#Import`
        // is in `dropped_imports`, so nothing dangles).
        if dead_groups.contains(&unit.group) {
            continue;
        }
        let group = &program.groups[unit.group.0 as usize];
        let mut stack = Vec::new();
        let text = expand(
            program,
            group.file,
            &rewrites,
            &deletions,
            &includes,
            &multiply,
            &mut stack,
        );

        // The entry group's primary module stays the implicit `__Main` (no header). Every
        // imported group's primary needs a synthesized `#Module Name` header before its text;
        // any in-source `#Module` sub-modules are already in `text`, renamed in place.
        let header = if i == 0 {
            None
        } else {
            group
                .modules
                .first()
                .and_then(|m| plan.module_names.get(&(unit.group, *m)))
        };
        match header {
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

/// Emit one file's text: its own rewrite (and, unless multiply-spliced, deletion) edits, plus
/// an edit per `#Include` directive that pastes the recursively-expanded text of the included
/// file (or deletes a deduped repeat). `stack` guards against include cycles defensively (the
/// linker already rejects them).
fn expand(
    program: &Program,
    file: FileId,
    rewrites: &HashMap<FileId, Vec<Edit>>,
    deletions: &HashMap<FileId, Vec<Edit>>,
    includes: &HashMap<FileId, Vec<(Span, IncludeSplice)>>,
    multiply: &HashSet<FileId>,
    stack: &mut Vec<FileId>,
) -> String {
    let source = program.sources.file(file);
    let mut edits: Vec<Edit> = rewrites.get(&file).cloned().unwrap_or_default();
    if !multiply.contains(&file) {
        if let Some(d) = deletions.get(&file) {
            edits.extend(d.iter().cloned());
        }
    }
    if let Some(list) = includes.get(&file) {
        stack.push(file);
        for (span, splice) in list {
            match splice {
                IncludeSplice::First(inc) => {
                    let content = if stack.contains(inc) {
                        String::new()
                    } else {
                        expand(program, *inc, rewrites, deletions, includes, multiply, stack)
                    };
                    edits.push(Edit::new(*span, content));
                }
                IncludeSplice::Dedup => edits.push(Edit::new(*span, "")),
                IncludeSplice::Missing => {}
            }
        }
        stack.pop();
    }
    apply_edits(&source.text, source.base, &edits)
}

/// Group `#Include` directives by the file they appear in, as `(directive span, splice)`. A
/// directive whose node is dead (its module was shaken out) is skipped — the deletion edit
/// removes the whole line, so its content must not also be spliced.
fn includes_by_file(
    program: &Program,
    plan: &BundlePlan,
    dead_nodes: &HashSet<NodeId>,
) -> HashMap<FileId, Vec<(Span, IncludeSplice)>> {
    let mut out: HashMap<FileId, Vec<(Span, IncludeSplice)>> = HashMap::new();
    for ri in &plan.resolved_includes {
        if dead_nodes.contains(&ri.node) {
            continue;
        }
        let span = program.arena[ri.node].span;
        let file = program.sources.file_at(span.start).id;
        out.entry(file).or_default().push((span, ri.splice));
    }
    out
}

/// Files spliced in more than once (via `#IncludeAgain`, or included into two modules). Their
/// shared spans can't carry per-copy tree-shaking deletions safely, so they are emitted whole
/// (rewrites only). A conservative over-keep.
fn multiply_spliced(plan: &BundlePlan) -> HashSet<FileId> {
    let mut count: HashMap<FileId, usize> = HashMap::new();
    for ri in &plan.resolved_includes {
        if let IncludeSplice::First(f) = ri.splice {
            *count.entry(f).or_insert(0) += 1;
        }
    }
    count
        .into_iter()
        .filter(|&(_, c)| c > 1)
        .map(|(f, _)| f)
        .collect()
}

/// Rename each written `#Module` directive whose assigned output name differs from its source
/// text, so colliding sub-module names across groups stay distinct in the flat output. Keyed
/// by the file the name span falls in.
fn add_rename_edits(program: &Program, plan: &BundlePlan, edits: &mut HashMap<FileId, Vec<Edit>>) {
    for group in &program.groups {
        for &mid in &group.modules {
            let NodeKind::Module(module) = &program.arena[mid].kind else {
                continue;
            };
            // Only written `#Module Name` directives have a name span to rewrite (the implicit
            // `__Main` primary has none).
            let Some(span) = module.name_span else {
                continue;
            };
            let Some(name) = plan.module_names.get(&(group.id, mid)) else {
                continue;
            };
            if program.span_text(span) == name {
                continue;
            }
            let file = program.sources.file_at(span.start).id;
            edits.entry(file).or_default().push(Edit::new(span, name));
        }
    }
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

/// Add deletion edits for every dead node and dropped import, keyed by the file whose text the
/// span falls in. (Whitespace left behind is a known cosmetic follow-up; orphaned `;@`
/// directive comments on a deleted node are harmless and left for now.)
fn add_deletion_edits(program: &Program, shake: &ShakeResult, edits: &mut HashMap<FileId, Vec<Edit>>) {
    let mut delete = |span: Span| {
        if span.is_empty() {
            return;
        }
        let file = program.sources.file_at(span.start).id;
        edits.entry(file).or_default().push(Edit::new(span, ""));
    };
    for &node in shake.dead.iter().chain(&shake.dropped_imports) {
        delete(program.arena[node].span);
    }
}

/// Build the per-file source edits that redirect each resolved `#Import` to its target group's
/// in-file module name. Keyed by the file whose text the edit lands in. Imports in `dropped`
/// are skipped — they're being deleted, not redirected.
fn import_edits(
    program: &Program,
    plan: &BundlePlan,
    dropped: &HashSet<NodeId>,
) -> HashMap<FileId, Vec<Edit>> {
    let mut edits: HashMap<FileId, Vec<Edit>> = HashMap::new();
    for ri in &plan.resolved_imports {
        if dropped.contains(&ri.node) {
            continue;
        }
        // The output name of the specific target module this import resolves to.
        let Some(target) = plan.module_names.get(&(ri.group, ri.module)) else {
            continue;
        };
        let target = target.as_str();
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
        let file = program.sources.file_at(spec_span.start).id;
        edits
            .entry(file)
            .or_default()
            .push(Edit::new(spec_span, target));
    }
    edits
}
