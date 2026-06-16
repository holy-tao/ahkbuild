//! Worklist reachability: mark every declaration that live code can reach.
//!
//! Reachability is **group-loaded**: a group's modules auto-execute (and so contribute seed
//! roots) only once the group is *loaded* — the entry group, or the target of a taken import.
//! Loading starts at the entry group and spreads through imports that are actually taken: a
//! name/namespace import is taken when its bound name is referenced from live code, while
//! side-effect, wildcard and re-export imports are taken unconditionally (they always run the
//! target's body). A module reached only through an unused import is therefore never loaded,
//! its roots are never seeded, and it shakes out whole.
//!
//! With the loaded groups seeded, the marker drains a worklist of `(node, owning module)`
//! pairs. Walking a live node's whole subtree, each name reference is resolved against the
//! *owning module's* tables — so a declaration pulled in from another module resolves its own
//! references in its own module's namespace. The `live` set doubles as the visited guard.

use std::collections::HashSet;

use ahkbuild_ir::{children, GroupId, NodeId, NodeKind, Program, Span};

use crate::resolve::{ModuleRef, Origin, Resolved};

/// The outcome of marking: which nodes are live, which imports were used, and which groups
/// were loaded (a group never loaded is dead in its entirety).
pub struct Reachability {
    pub live: HashSet<NodeId>,
    pub used_imports: HashSet<NodeId>,
    pub loaded: HashSet<GroupId>,
}

/// Mark all reachable declarations, starting by loading the entry group.
pub fn mark(program: &Program, resolved: &Resolved) -> Reachability {
    let mut m = Marker {
        program,
        resolved,
        live: HashSet::new(),
        used_imports: HashSet::new(),
        loaded: HashSet::new(),
        blown: HashSet::new(),
        worklist: Vec::new(),
    };

    // The entry group (the main script) always runs; everything else loads transitively.
    if let Some(entry) = program.groups.first() {
        m.load_group(entry.id);
    }

    m.run();
    Reachability {
        live: m.live,
        used_imports: m.used_imports,
        loaded: m.loaded,
    }
}

struct Marker<'a> {
    program: &'a Program,
    resolved: &'a Resolved,
    live: HashSet<NodeId>,
    used_imports: HashSet<NodeId>,
    /// Groups whose bodies are known to run. Seeded from the entry group and grown as taken
    /// imports load their targets. Doubles as the visited guard for `load_group`.
    loaded: HashSet<GroupId>,
    /// Modules whose member/name resolution has been defeated by a dynamic construct; their
    /// declarations (and imports) are all kept. Tracked so we blow up each module only once.
    blown: HashSet<ModuleRef>,
    worklist: Vec<(NodeId, ModuleRef)>,
}

impl Marker<'_> {
    /// Load a group: seed every one of its modules' roots, and follow the imports that are
    /// taken unconditionally (side-effect, wildcard, re-export), loading their targets too.
    /// Idempotent — the `loaded` set guards against reprocessing and import cycles.
    fn load_group(&mut self, gid: GroupId) {
        if !self.loaded.insert(gid) {
            return;
        }
        let Some(group) = self.program.groups.iter().find(|g| g.id == gid) else {
            return;
        };
        for module_id in group.modules.clone() {
            let mref = ModuleRef {
                group: gid,
                module: module_id,
            };
            // This module auto-executes now that its group is loaded — seed its roots.
            let roots = self.resolved.roots.get(&mref).cloned().unwrap_or_default();
            for r in roots {
                self.worklist.push((r, mref));
            }
            // Follow the always-taken imports (clone out first so the arena borrow ends).
            let (side, wild, reex) = match self.resolved.imports.get(&mref) {
                Some(imp) => (
                    imp.side_effects.clone(),
                    imp.wildcards.clone(),
                    imp.reexports.clone(),
                ),
                None => (Vec::new(), Vec::new(), Vec::new()),
            };
            // Side-effect imports run the target body but bind nothing.
            for (node, target) in side {
                self.used_imports.insert(node);
                self.load_group(target.group);
            }
            // Wildcard imports keep the whole target (can't tell which unqualified names they
            // pull in); seed its declarations and mark the import used.
            for (node, target) in wild {
                self.used_imports.insert(node);
                self.enqueue_all_decls(target);
                self.load_group(target.group);
            }
            // Re-exports (`#Import export ...`) are public surface: keep the re-exported target
            // declarations live (a consumer may reach them through this module).
            for (target, origin) in reex {
                match origin {
                    Origin::Namespace => self.enqueue_all_decls(target),
                    Origin::Name(name) => self.enqueue_named_decl(target, &name),
                }
                self.load_group(target.group);
            }
        }
    }

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

    /// Resolve a referenced name within `mref`: mark matching local declarations live, and —
    /// if the name is an import binding — take that import, loading its target module (the
    /// reference is the "use" that makes the import worth keeping) and pulling in its decls.
    fn reference(&mut self, name: &str, mref: ModuleRef) {
        // Extract owned data under the immutable borrow first, then mutate.
        let locals: Vec<NodeId> = self
            .resolved
            .decls
            .get(&mref)
            .and_then(|tbl| tbl.by_name.get(name))
            .cloned()
            .unwrap_or_default();
        for d in locals {
            self.worklist.push((d, mref));
        }

        let binding = self
            .resolved
            .imports
            .get(&mref)
            .and_then(|imports| imports.by_name.get(name))
            .map(|t| (t.node, t.target, t.origin.clone()));
        if let Some((node, target, origin)) = binding {
            self.used_imports.insert(node);
            match origin {
                // `X` (namespace): member access may be dynamic -> keep all target decls.
                Origin::Namespace => self.enqueue_all_decls(target),
                // `{a}` (selective): just the named export `a` in the target.
                Origin::Name(origin) => self.enqueue_named_decl(target, &origin),
            }
            // Taking the import runs the target's body, so its group loads.
            self.load_group(target.group);
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
        let targets: Vec<(NodeId, ModuleRef)> = match self.resolved.imports.get(&m) {
            Some(imports) => imports
                .by_name
                .values()
                .map(|t| (t.node, t.target))
                .chain(imports.wildcards.iter().copied())
                .chain(imports.side_effects.iter().copied())
                .collect(),
            None => Vec::new(),
        };
        for (node, target) in targets {
            self.used_imports.insert(node);
            self.enqueue_all_decls(target);
            self.load_group(target.group);
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
