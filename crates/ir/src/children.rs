//! Exhaustive child-node iteration over the IR.
//!
//! [`children`] returns every direct child [`NodeId`] of a node via a `match` on
//! [`NodeKind`] with no catch-all — adding a variant fails to compile until handled here.
//! Analyses (reachability, constant folding, …) share this one walk instead of each
//! re-deriving it and risking a missed edge. It mirrors the recursion in
//! [`crate::print`]; the completeness test below keeps the two in lockstep.
//!
//! References carried as raw [`Span`](ahkbuild_syntax::Span)s rather than child nodes — an
//! [`Identifier`](NodeKind::Identifier)'s own text, [`TypeDecl::superclass`], a
//! [`CatchClause`](crate::node::CatchClause)'s `error_types`, and `Goto`/`Break`/`Continue`
//! labels — are deliberately *not* returned here; name-resolving callers handle those out
//! of band.
//!
//! [`TypeDecl::superclass`]: crate::node::TypeDecl::superclass

use crate::arena::{Arena, NodeId};
use crate::node::{NodeKind, Param};

/// Every direct child [`NodeId`] of `kind`, roughly in source order.
pub fn children(kind: &NodeKind) -> Vec<NodeId> {
    let mut out = Vec::new();
    collect(kind, &mut out);
    out
}

/// [`children`] of the node `id` stored in `arena`.
pub fn child_ids(arena: &Arena, id: NodeId) -> Vec<NodeId> {
    children(&arena[id].kind)
}

/// Parameter default-value expressions are children too (closures/defaults can reference
/// other declarations); `print.rs` reaches them through its `params()` helper.
fn push_param_defaults(params: &[Param], out: &mut Vec<NodeId>) {
    out.extend(params.iter().filter_map(|p| p.default));
}

fn collect(kind: &NodeKind, out: &mut Vec<NodeId>) {
    match kind {
        NodeKind::Block { body } => out.extend(body.iter().copied()),
        NodeKind::ExpressionStatement { expr } => out.push(*expr),
        NodeKind::Opaque => {}
        NodeKind::ExpressionSequence { exprs } => out.extend(exprs.iter().copied()),
        NodeKind::Module(m) => out.extend(m.body.iter().copied()),
        NodeKind::ImportDirective(_) => {}
        NodeKind::IncludeDirective(_) => {}
        NodeKind::ExportDecl { decl, .. } => out.push(*decl),
        NodeKind::Function(f) => {
            push_param_defaults(&f.params, out);
            out.extend(f.body);
        }
        NodeKind::ClassDecl(t) | NodeKind::StructDecl(t) => {
            out.extend(t.static_fields.iter().copied());
            out.extend(t.instance_fields.iter().copied());
            out.extend(t.typed_fields.iter().copied());
            out.extend(t.properties.iter().copied());
            out.extend(t.methods.iter().copied());
            out.extend(t.nested.iter().copied());
        }
        NodeKind::Property(p) => {
            out.extend(p.getter);
            out.extend(p.setter);
        }
        NodeKind::Field(f) => out.extend(f.initializer),
        NodeKind::TypedProperty(t) => {
            out.push(t.type_spec);
            out.extend(t.initializer);
        }
        NodeKind::TypeSpecifier { type_expr } => out.push(*type_expr),
        NodeKind::VarDecl(v) => out.extend(v.initializer),
        NodeKind::BinaryExpr { left, right, .. } => {
            out.push(*left);
            out.push(*right);
        }
        NodeKind::UnaryExpr { operand, .. } => out.push(*operand),
        NodeKind::TernaryExpr {
            condition,
            then_branch,
            else_branch,
        } => {
            out.push(*condition);
            out.push(*then_branch);
            out.push(*else_branch);
        }
        NodeKind::CallExpr(c) => {
            out.push(c.callee);
            out.extend(c.args.iter().copied());
        }
        NodeKind::Literal { .. } => {}
        NodeKind::Identifier => {}
        NodeKind::DynamicIdentifier { parts } => out.extend(parts.iter().copied()),
        NodeKind::MemberAccess { object, member, .. } => {
            out.push(*object);
            out.push(*member);
        }
        NodeKind::IndexAccess { object, args } => {
            out.push(*object);
            out.extend(args.iter().copied());
        }
        NodeKind::ArrayLiteral { elements } => out.extend(elements.iter().copied()),
        NodeKind::ObjectLiteral { members } => {
            for m in members {
                out.push(m.key);
                out.extend(m.value);
            }
        }
        NodeKind::DerefExpr { inner } => out.push(*inner),
        NodeKind::VarRefExpr { operand } => out.push(*operand),
        NodeKind::FatArrow { params, body } => {
            push_param_defaults(params, out);
            out.push(*body);
        }
        NodeKind::IfStmt {
            condition,
            then_body,
            else_body,
        } => {
            out.push(*condition);
            out.push(*then_body);
            out.extend(*else_body);
        }
        NodeKind::WhileStmt { condition, body } => {
            out.push(*condition);
            out.push(*body);
        }
        NodeKind::ForStmt(f) => {
            out.extend(f.iterators.iter().copied());
            out.extend(f.iterable);
            out.extend(f.body);
            out.extend(f.else_body);
        }
        NodeKind::LoopStmt(l) => {
            out.extend(l.head);
            out.extend(l.body);
            out.extend(l.until);
        }
        NodeKind::SwitchStmt {
            discriminant,
            cases,
        } => {
            out.extend(*discriminant);
            out.extend(cases.iter().copied());
        }
        NodeKind::CaseClause { values, body, .. } => {
            out.extend(values.iter().copied());
            out.extend(body.iter().copied());
        }
        NodeKind::TryStmt(t) => {
            out.push(t.try_body);
            out.extend(t.catches.iter().copied());
            out.extend(t.else_body);
            out.extend(t.finally_body);
        }
        NodeKind::CatchClause(c) => out.push(c.body),
        NodeKind::ReturnStmt { value } => out.extend(*value),
        NodeKind::BreakStmt { .. } | NodeKind::ContinueStmt { .. } | NodeKind::GotoStmt { .. } => {}
        NodeKind::ThrowStmt { value } => out.extend(*value),
        NodeKind::Hotkey { body, .. } => out.extend(*body),
        NodeKind::Hotstring { replacement, .. } => out.extend(*replacement),
        NodeKind::Directive { expression, .. } => out.extend(*expression),
        NodeKind::Label { .. } => {}
        NodeKind::Comment => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::lower;
    use std::collections::HashSet;

    fn program(src: &str) -> crate::Program {
        let tree = ahkbuild_syntax::parse(src).expect("tree");
        assert!(!tree.root_node().has_error(), "parse error in fixture");
        lower(&tree, src)
    }

    /// Every node is either a module root or reached as some node's child — i.e. `children`
    /// covers the whole tree. A missing child edge would orphan a subtree and show up here.
    #[test]
    fn children_reach_every_node() {
        // Exercise a broad spread of variants: classes/structs with members, control flow,
        // calls/args, object/array literals, fat arrows with defaults, try/catch, etc.
        let p = program(
            r#"
global G := 1
Greet(who := "world") => MsgBox("hi " who)
class Animal extends Base {
    static Kind := "?"
    name := ""
    static __New() {
        this.Kind := "animal"
    }
    Speak(n) {
        loop n {
            if (n > 0)
                MsgBox(this.name)
            else
                continue
        }
    }
    Tag => this.name
}
struct Pt {
    x: Int := 0
}
arr := [1, 2, 3]
obj := {a: 1, b: G}
try {
    Greet()
} catch Error as e {
    throw e
}
cb := (x, y := 2) => x + y
#HotIf G
^a::MsgBox("hotkey")
"#,
        );

        let mut referenced: HashSet<NodeId> = HashSet::new();
        for (_, node) in p.arena.iter() {
            for c in children(&node.kind) {
                referenced.insert(c);
            }
        }
        let roots: HashSet<NodeId> = p
            .groups
            .iter()
            .flat_map(|g| g.modules.iter().copied())
            .collect();

        for (id, node) in p.arena.iter() {
            assert!(
                roots.contains(&id) || referenced.contains(&id),
                "node {id:?} ({:?}) is not a module root nor any node's child — \
                 children() is missing an edge",
                std::mem::discriminant(&node.kind),
            );
        }
    }

    #[test]
    fn parenthesized_expression_surfaces_inner_reference() {
        // The grammar's `_parenthesized_expression` is hidden, so naively reading a field
        // could yield the `(` token and lower to Opaque, hiding references inside. Both a
        // parenthesized operand of `!` and a parenthesized binary operand must expose their
        // inner identifiers as real nodes (else reachability under-marks and drops live code).
        let p = program("r := !(x is Query)\n");
        // Collect identifier texts reachable by walking children from the module root.
        let mut seen = Vec::new();
        let mut stack = vec![p.groups[0].modules[0]];
        while let Some(n) = stack.pop() {
            if matches!(p.arena[n].kind, NodeKind::Identifier) {
                seen.push(p.text(n).trim().to_string());
            }
            stack.extend(children(&p.arena[n].kind));
        }
        assert!(
            seen.iter().any(|s| s == "Query"),
            "the parenthesized `Query` reference must be reachable, saw: {seen:?}"
        );
    }

    #[test]
    fn call_children_are_callee_then_args() {
        let p = program("Foo(a, b)\n");
        // The top-level call lowers straight to a CallExpr (no ExpressionStatement wrapper).
        let main = p.groups[0].modules[0];
        let NodeKind::Module(m) = &p.arena[main].kind else {
            panic!()
        };
        let call = m.body[0];
        assert!(matches!(p.arena[call].kind, NodeKind::CallExpr(_)));
        let kids = children(&p.arena[call].kind);
        // callee + 2 args, callee first
        assert_eq!(kids.len(), 3, "callee + 2 args");
        assert!(matches!(p.arena[kids[0]].kind, NodeKind::Identifier));
    }
}
