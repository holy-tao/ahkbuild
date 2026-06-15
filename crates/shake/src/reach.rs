//! Worklist reachability: mark every declaration that live code can reach.
//!
//! Seeds from the roots (`resolve` already collected every module's auto-execute statements
//! and `static __New` classes), then drains a worklist of `(node, owning module)` pairs.
//! Walking a live node's whole subtree, each name reference is resolved against the *owning
//! module's* tables — so a declaration pulled in from another module resolves its own
//! references in its own module's namespace. The `live` set doubles as the visited guard.

use std::collections::HashSet;

use ahkbuild_ir::{children, NodeId, NodeKind, Program, Span};

use crate::resolve::{ModuleRef, Origin, Resolved};

/// The outcome of marking: which nodes are live, and which imports were actually used.
pub struct Reachability {
    pub live: HashSet<NodeId>,
    pub used_imports: HashSet<NodeId>,
}

/// Mark all reachable declarations starting from the seed roots.
pub fn mark(program: &Program, resolved: &Resolved) -> Reachability {
    let mut m = Marker {
        program,
        resolved,
        live: HashSet::new(),
        used_imports: HashSet::new(),
        blown: HashSet::new(),
        worklist: Vec::new(),
    };

    for &(node, mref) in &resolved.roots {
        m.worklist.push((node, mref));
    }
    // Wildcard imports keep their whole target (can't tell which unqualified names they pull
    // in); seed those targets' declarations and mark the import used.
    for imports in resolved.imports.values() {
        for &(import_node, target) in &imports.wildcards {
            m.used_imports.insert(import_node);
            m.enqueue_all_decls(target);
        }
        // Re-exports (`#Import export ...`) are public surface: keep the re-exported target
        // declarations live unconditionally (a consumer may reach them through this module).
        for (target, origin) in &imports.reexports {
            match origin {
                Origin::Namespace => m.enqueue_all_decls(*target),
                Origin::Name(name) => m.enqueue_named_decl(*target, name),
            }
        }
    }

    m.run();
    Reachability {
        live: m.live,
        used_imports: m.used_imports,
    }
}

struct Marker<'a> {
    program: &'a Program,
    resolved: &'a Resolved,
    live: HashSet<NodeId>,
    used_imports: HashSet<NodeId>,
    /// Modules whose member/name resolution has been defeated by a dynamic construct; their
    /// declarations (and imports) are all kept. Tracked so we blow up each module only once.
    blown: HashSet<ModuleRef>,
    worklist: Vec<(NodeId, ModuleRef)>,
}

impl Marker<'_> {
    fn run(&mut self) {
        while let Some((id, mref)) = self.worklist.pop() {
            if !self.live.insert(id) {
                continue;
            }
            self.walk(id, mref);
        }
    }

    /// Walk `id`'s subtree, resolving each node's outgoing name edges against `mref`.
    fn walk(&mut self, id: NodeId, mref: ModuleRef) {
        let program = self.program;
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            self.resolve_edges(n, mref);
            stack.extend(children(&program.arena[n].kind));
        }
    }

    /// Push every outgoing reference of node `n` (resolved in module `mref`) onto the worklist.
    fn resolve_edges(&mut self, n: NodeId, mref: ModuleRef) {
        let program = self.program;
        // Extract owned edge data first so the arena borrow ends before we mutate `self`.
        enum Edge {
            Name(String),
            BlowUp,
            None,
        }
        let edge = match &program.arena[n].kind {
            NodeKind::Identifier => Edge::Name(ident(program, program.arena[n].span)),
            NodeKind::ClassDecl(t) | NodeKind::StructDecl(t) => match t.superclass {
                // Dotted `Base.Inner` — the head segment names the referenced class.
                Some(s) => {
                    let full = ident(program, s);
                    let head = full.split('.').next().unwrap_or(&full).to_string();
                    Edge::Name(head)
                }
                None => Edge::None,
            },
            NodeKind::CatchClause(c) => {
                for et in c.error_types.clone() {
                    let name = ident(program, et);
                    self.reference(&name, mref);
                }
                Edge::None
            }
            // Dynamic constructs hide their target name -> conservatively keep the whole module.
            NodeKind::DynamicIdentifier { .. } | NodeKind::DerefExpr { .. } => Edge::BlowUp,
            NodeKind::MemberAccess { is_dynamic, .. } if *is_dynamic => Edge::BlowUp,
            NodeKind::CallExpr(c) if c.is_dynamic => Edge::BlowUp,
            _ => Edge::None,
        };
        match edge {
            Edge::Name(name) => self.reference(&name, mref),
            Edge::BlowUp => self.blow_up(mref),
            Edge::None => {}
        }
    }

    /// Resolve a referenced name within `mref`: mark matching local declarations live, and
    /// follow import bindings into their target module.
    fn reference(&mut self, name: &str, mref: ModuleRef) {
        let resolved = self.resolved;
        if let Some(tbl) = resolved.decls.get(&mref) {
            if let Some(nodes) = tbl.by_name.get(name) {
                for &d in nodes {
                    self.worklist.push((d, mref));
                }
            }
        }
        if let Some(imports) = resolved.imports.get(&mref) {
            if let Some(t) = imports.by_name.get(name) {
                self.used_imports.insert(t.node);
                match &t.origin {
                    // `X` (namespace): member access may be dynamic -> keep all target decls.
                    Origin::Namespace => self.enqueue_all_decls(t.target),
                    // `{a}` (selective): just the named export `a` in the target.
                    Origin::Name(origin) => {
                        let target = t.target;
                        if let Some(tt) = resolved.decls.get(&target) {
                            if let Some(nodes) = tt.by_name.get(origin) {
                                for &d in nodes {
                                    self.worklist.push((d, target));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Conservatively keep every declaration of `m` (a dynamic reference could resolve to any
    /// of them) and treat all of `m`'s imports as used — a `%val%` deref can name an imported
    /// symbol just as easily as a local one.
    fn blow_up(&mut self, m: ModuleRef) {
        if !self.blown.insert(m) {
            return;
        }
        self.enqueue_all_decls(m);
        let resolved = self.resolved;
        if let Some(imports) = resolved.imports.get(&m) {
            let targets: Vec<(NodeId, ModuleRef)> = imports
                .by_name
                .values()
                .map(|t| (t.node, t.target))
                .chain(imports.wildcards.iter().copied())
                .collect();
            for (node, target) in targets {
                self.used_imports.insert(node);
                self.enqueue_all_decls(target);
            }
        }
    }

    fn enqueue_all_decls(&mut self, m: ModuleRef) {
        if let Some(tbl) = self.resolved.decls.get(&m) {
            for &d in &tbl.all {
                self.worklist.push((d, m));
            }
        }
    }

    fn enqueue_named_decl(&mut self, m: ModuleRef, name: &str) {
        if let Some(tbl) = self.resolved.decls.get(&m) {
            if let Some(nodes) = tbl.by_name.get(name) {
                for &d in nodes {
                    self.worklist.push((d, m));
                }
            }
        }
    }
}

fn ident(program: &Program, span: Span) -> String {
    program.span_text(span).trim().to_ascii_lowercase()
}
