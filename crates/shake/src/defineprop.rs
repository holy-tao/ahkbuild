//! `DefineProp` pruning: drop standalone `obj.DefineProp("Name", desc)` statements that define
//! a property whose name no live code references.
//!
//! This runs *before* reachability so the references inside a pruned descriptor (e.g. the
//! getter function it names) are never followed — letting that now-orphaned code shake out
//! too. Pruning a call also strips its descriptor's referencers from the member-name table, so
//! a name referenced *only* inside the pruned call stops counting as live. Conservative: only
//! literal-named, standalone, non-chained calls qualify; anything else is kept.
//!
//! Ports `_PruneDefinePropCalls` / `_TryPruneDefineProp` from `build/treeshake.ahk`.

use std::collections::HashSet;

use ahkbuild_ir::node::LiteralKind;
use ahkbuild_ir::{NodeId, NodeKind, Program};

use crate::members::{is_descendant_of, is_protected, MemberNameTable};

/// Find every prunable standalone `DefineProp` call, mark it (return its `NodeId`), and remove
/// its descriptor's referencers from `table`. No-op if the table is blown (member pruning off).
pub fn prune(program: &Program, table: &mut MemberNameTable) -> HashSet<NodeId> {
    let mut pruned = HashSet::new();
    if table.is_blown() {
        return pruned;
    }
    // Standalone calls are the direct elements of a module or block body. Visiting those gives
    // statement-position calls only — a chained/nested `DefineProp` is an arg/callee, not a
    // body element, so it is never reached here (matching the legacy parent-is-block check).
    for (_, node) in program.arena.iter() {
        let body = match &node.kind {
            NodeKind::Module(m) => &m.body,
            NodeKind::Block { body } => body,
            _ => continue,
        };
        for &stmt in body {
            // A statement call may be the body element itself or the expr it wraps.
            let call = match &program.arena[stmt].kind {
                NodeKind::CallExpr(_) => stmt,
                NodeKind::ExpressionStatement { expr }
                    if matches!(program.arena[*expr].kind, NodeKind::CallExpr(_)) =>
                {
                    *expr
                }
                _ => continue,
            };
            if try_prune(program, table, call) {
                pruned.insert(call);
            }
        }
    }
    pruned
}

/// Whether `call` is a prunable standalone `DefineProp` call; on success strips its descriptor's
/// referencers from `table`.
fn try_prune(program: &Program, table: &mut MemberNameTable, call: NodeId) -> bool {
    let NodeKind::CallExpr(c) = &program.arena[call].kind else {
        return false;
    };
    // 1. Callee is a static `.DefineProp` member access.
    let NodeKind::MemberAccess {
        object,
        member,
        is_dynamic,
    } = &program.arena[c.callee].kind
    else {
        return false;
    };
    if *is_dynamic || !program.text(*member).eq_ignore_ascii_case("defineprop") {
        return false;
    }
    // 2. First argument is a string literal (the property name).
    let Some(&name_arg) = c.args.first() else {
        return false;
    };
    let NodeKind::Literal {
        kind: LiteralKind::String,
    } = &program.arena[name_arg].kind
    else {
        return false;
    };
    let prop_name = strip_quotes(program.text(name_arg));

    // 3. Never prune a protected meta-function.
    if is_protected(prop_name) {
        return false;
    }
    // 4. Keep if the name is referenced anywhere outside this DefineProp call itself.
    if let Some(refs) = table.matches(prop_name) {
        // Empty refs = a prefix/suffix (or blown) match: referenced, keep. Otherwise keep
        // unless the sole referencer is this very call's descriptor (a self-reference).
        if refs.is_empty() || refs.len() > 1 || !is_descendant_of(program, refs[0], call) {
            return false;
        }
    }
    // 5. Guard against chained calls (`obj.DefineProp(..).DefineProp(..)`): pruning the outer
    //    would wrongly delete the inner. The object of the callee must not itself be a call.
    if matches!(program.arena[*object].kind, NodeKind::CallExpr(_)) {
        return false;
    }

    table.remove_descendant_referencers(call, program);
    true
}

fn strip_quotes(text: &str) -> &str {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        if (first == b'"' || first == b'\'') && bytes[bytes.len() - 1] == first {
            return &text[1..text.len() - 1];
        }
    }
    text
}
