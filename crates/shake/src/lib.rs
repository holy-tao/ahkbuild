//! Module-aware tree-shaking: dead-code elimination by reachability.
//!
//! Given a linked [`Program`] + [`BundlePlan`], [`shake`] finds the declarations no live
//! code can reach and reports them as a [`ShakeResult`] for the emitter to delete. It never
//! mutates the IR — like every other pass, removal is expressed as span edits at emit time.
//!
//! v1 works at **declaration / whole-module** granularity with conservative, name-based,
//! module-scoped resolution: a top-level function/class is dead if nothing references its
//! name; an `#Import` is dropped if its bound name is never used; a module with no surviving
//! content is removed entirely. Over-keeping is safe; under-keeping would drop live code, so
//! anything ambiguous (dynamic references, namespace member access, wildcard imports) keeps
//! more. Per-member pruning, the member-name table, dynamic-member extraction, reflection
//! functions, `DefineProp` pruning, and the protected-member set are deferred to v2; live
//! classes are kept **whole**.

mod reach;
mod resolve;

use std::collections::HashSet;

use ahkbuild_ir::{NodeId, NodeKind, Program};
use ahkbuild_link::BundlePlan;

/// What tree-shaking found to remove. All granularity is top-level: declaration statements,
/// `#Import` directives, and whole modules. The emitter turns each into a deletion edit (and
/// skips emitting a fully-dead group).
#[derive(Debug, Default)]
pub struct ShakeResult {
    /// Top-level declaration statements (and, for inline dead modules, their body statements
    /// and `#Module` header) whose spans should be deleted.
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
pub fn shake(program: &Program, plan: &BundlePlan) -> ShakeResult {
    let resolved = resolve::resolve(program, plan);
    let reach = reach::mark(program, &resolved);

    let mut result = ShakeResult::default();

    // The entry module (main script's `__Main`) is the program; never remove it.
    let entry = program
        .groups
        .first()
        .and_then(|g| g.modules.first().copied());

    for &mref in &resolved.modules {
        let NodeKind::Module(module) = &program.arena[mref.module].kind else {
            continue;
        };
        let decls = &resolved.decls[&mref];
        let droppable: HashSet<NodeId> = resolved.imports[&mref].droppable.iter().copied().collect();

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
            // Nothing in this module survives — remove it whole.
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
