//! Module-aware tree-shaking: dead-code elimination by reachability.
//!
//! Given a linked [`Program`] + [`BundlePlan`], [`shake`] finds the declarations no live
//! code can reach and reports them as a [`ShakeResult`] for the emitter to delete. It never
//! mutates the IR — like every other pass, removal is expressed as span edits at emit time.
//!
//! It works at **declaration / whole-module** granularity with conservative, name-based,
//! module-scoped resolution: a top-level function/class is dead if nothing references its
//! name; an `#Import` is dropped if its bound name is never used; a module with no surviving
//! content is removed entirely. Over-keeping is safe; under-keeping would drop live code, so
//! anything ambiguous (dynamic references, namespace member access, wildcard imports) keeps
//! more.
//!
//! On top of that, **per-member pruning** trims a live class down to the members live code can
//! reach: a program-wide [member-name table](members) records every member name any access
//! could resolve to (static `obj.Foo`, dynamic `obj.Get%x%`, reflection-builtin string args),
//! and a live class keeps only its matching (or protected, or `;@AhkBuild-Keep`-marked)
//! members. [`defineprop`] additionally drops standalone `DefineProp("Name", …)` statements
//! whose name nothing references. A fully-dynamic member access disables member pruning
//! program-wide (classes kept whole).

mod defineprop;
mod members;
mod reach;
mod resolve;

use std::collections::HashSet;

use ahkbuild_fold::FoldResult;
use ahkbuild_ir::{NodeId, NodeKind, Program};
use ahkbuild_link::BundlePlan;

/// What tree-shaking found to remove: dead declaration statements, unreferenced class members,
/// pruned `DefineProp` calls, unused `#Import` directives, and whole modules. The emitter turns
/// each into a deletion edit (and skips emitting a fully-dead group). Overlapping spans (a
/// pruned member or `DefineProp` inside an already-dead class/module) are resolved by the
/// emitter — the outer deletion wins — so listing both is harmless.
#[derive(Debug, Default)]
pub struct ShakeResult {
    /// Spans to delete: top-level declaration statements (and, for inline dead modules, their
    /// body statements and `#Module` header), plus unreferenced class members and pruned
    /// standalone `DefineProp` calls.
    pub dead: HashSet<NodeId>,
    /// `#Import` directive nodes whose bound name is never referenced — safe to drop (the
    /// target module still auto-executes).
    pub dropped_imports: Vec<NodeId>,
    /// Module nodes with zero surviving content. If *every* module of a group is dead, the
    /// emitter omits the whole unit; otherwise the module's spans are deleted via `dead`.
    pub dead_modules: Vec<NodeId>,
}

impl ShakeResult {
    /// Whether anything will be removed.
    pub fn is_empty(&self) -> bool {
        self.dead.is_empty() && self.dropped_imports.is_empty() && self.dead_modules.is_empty()
    }
}

/// Run tree-shaking over a linked program.
///
/// When `fold` is `Some`, branches whose conditions folded to a build-time constant are
/// shaken at the arm level: reachability descends only into the surviving arm, so any
/// declaration reachable *only* from a dropped arm shakes out with everything else.
pub fn shake(program: &Program, plan: &BundlePlan, fold: Option<&FoldResult>) -> ShakeResult {
    let resolved = resolve::resolve(program, plan);

    // Build the program-wide member-name table, then prune standalone `DefineProp` calls whose
    // property names it never matches (this also strips those calls' descriptor referencers,
    // so a name used only inside a pruned call stops counting). Both run before marking.
    let mut table = members::collect(program, fold);
    let mut dead_defineprops = defineprop::prune(program, &mut table);

    // The entry module (main script's `__Main`) is the program; never remove it.
    let entry = program
        .groups
        .first()
        .and_then(|g| g.modules.first().copied());

    let mut reach = reach::mark(program, &resolved, &table, &dead_defineprops, fold);
    let mut result = assemble_result(program, &resolved, &reach, entry, &dead_defineprops);

    // As long as the name table isn't blown, run the tree-shaking algorithm until it produces
    // no additional dead nodes. It's guaranteed to converge because reference counts can only
    // ever decrease (put another way, the dead set can only ever grow).
    if !table.is_blown() {
        loop {
            let before = table.referencer_count();
            for &n in &result.dead {
                table.remove_descendant_referencers(n, program);
            }
            // Re-prune: a `DefineProp` name that now matches nothing becomes prunable too.
            // `prune` re-scans, so its result is a superset of the old; union it in and note any
            // genuinely new pruning (`insert` is true only for a not-yet-seen call).
            let mut grew = false;
            for n in defineprop::prune(program, &mut table) {
                grew |= dead_defineprops.insert(n);
            }

            // Fixpoint: the table stopped shrinking and no new `DefineProp` pruned.
            if table.referencer_count() == before && !grew {
                break;
            }
            reach = reach::mark(program, &resolved, &table, &dead_defineprops, fold);
            result = assemble_result(program, &resolved, &reach, entry, &dead_defineprops);
        }
    }

    result
}

/// Assemble a [`ShakeResult`] from one marking pass: pruned `DefineProp` calls and dead members
/// are deleted by their own spans, whole-dead modules are removed entirely (header + body), and
/// every unreferenced declaration in a surviving module is dropped. Pure in its inputs, so the
/// fixpoint loop can call it once per marking round.
fn assemble_result(
    program: &Program,
    resolved: &resolve::Resolved,
    reach: &reach::Reachability,
    entry: Option<NodeId>,
    dead_defineprops: &HashSet<NodeId>,
) -> ShakeResult {
    let mut result = ShakeResult::default();
    result.dead.extend(dead_defineprops.iter().copied());
    result.dead.extend(reach.dead_members.iter().copied());

    for &mref in &resolved.modules {
        let NodeKind::Module(module) = &program.arena[mref.module].kind else {
            continue;
        };
        let decls = &resolved.decls[&mref];
        let droppable: HashSet<NodeId> =
            resolved.imports[&mref].droppable.iter().copied().collect();

        // Walk the body to find what survives and which imports are unused.
        let mut has_live = false;
        let mut has_kept_import = false;
        let mut local_dropped = Vec::new();
        for &stmt in &module.body {
            if matches!(program.arena[stmt].kind, NodeKind::ImportDirective(_)) {
                if droppable.contains(&stmt) && !reach.used_imports.contains(&stmt) {
                    local_dropped.push(stmt);
                } else {
                    has_kept_import = true;
                }
            } else if reach.live.contains(&stmt) {
                has_live = true;
            }
        }

        // A module whose group was never loaded never runs at all: it's dead in its entirety,
        // imports and all (a non-droppable import inside it must not keep it alive).
        let group_loaded = reach.loaded.contains(&mref.group);
        let is_entry = Some(mref.module) == entry;
        if !is_entry && (!group_loaded || (!has_live && !has_kept_import)) {
            // Nothing in this module survives - remove it whole.
            result.dead_modules.push(mref.module);
            result.dead.insert(mref.module); // its `#Module` header span
            result.dead.extend(module.body.iter().copied());
        } else {
            for &d in &decls.all {
                if !reach.live.contains(&d) {
                    result.dead.insert(d);
                }
            }
            result.dropped_imports.extend(local_dropped);
        }
    }

    result
}
