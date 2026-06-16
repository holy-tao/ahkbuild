//! The resolution foundation: per-module declaration tables, import-binding tables, and
//! the seed roots — everything reachability needs to follow name references.
//!
//! Resolution is deliberately **conservative and name-based** (no lexical-scope/shadowing
//! analysis): a reference is any identifier/superclass/catch-type whose *text* matches a
//! module-global declaration name (case-insensitively, as AHK is). Over-matching keeps a few
//! extra symbols (safe); the thing we must never do is under-match and drop live code.
//!
//! Module identity is `(GroupId, module NodeId)`. Because the linker only bundles
//! import-reachable modules and **every module body auto-executes at startup** (see
//! `docs/Modules.md`), all modules' auto-execute statements are unconditional roots — there
//! is no separate "module becomes reachable when imported" step. Import bindings still matter
//! for resolving cross-module *name* references.

use std::collections::HashMap;

use ahkbuild_ir::node::{ImportBinding, ImportSource};
use ahkbuild_ir::{GroupId, NodeId, NodeKind, Program, Span};
use ahkbuild_link::BundlePlan;

/// A module within the program: a `#Module` block (or the implicit `__Main`) in some group.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ModuleRef {
    pub group: GroupId,
    pub module: NodeId,
}

/// Module-global declarations of one module, keyed by lowercased name.
#[derive(Default)]
pub struct DeclTable {
    /// Lowercased declared name -> the top-level statement node(s) declaring it. A name may
    /// map to several nodes (a reopened module re-declares it); reachability marks them all.
    /// The stored node is the *outer* statement — an `ExportDecl` for `export ...`, so the
    /// sweep deletes `export` + decl together.
    pub by_name: HashMap<String, Vec<NodeId>>,
    /// Every declaration statement node in this module (for whole-module-dead detection and
    /// the "mark all of a namespace-imported module's decls" conservative edge).
    pub all: Vec<NodeId>,
}

/// What an imported name refers to in the target module.
#[derive(Clone, Debug)]
pub enum Origin {
    /// `#Import X` / `as Z`: the bound name is the whole module namespace; a reference marks
    /// *all* of the target's declarations live (member access may be dynamic).
    Namespace,
    /// `#Import X {a as b}`: the bound name `b` refers to target declaration `a`.
    Name(String),
}

/// One resolved import binding: the directive node, the target module, and what it binds.
#[derive(Clone, Debug)]
pub struct ImportTarget {
    pub node: NodeId,
    pub target: ModuleRef,
    pub origin: Origin,
}

/// Import bindings visible in one module.
#[derive(Default)]
pub struct ImportTable {
    /// Lowercased bound local name -> target. Referencing the name marks the import "used".
    pub by_name: HashMap<String, ImportTarget>,
    /// Resolved file-import directive nodes that bind at least one name — candidates to drop
    /// if none of their names are referenced. (Wildcard, re-export, quoted side-effect, and
    /// in-group/builtin imports are never listed; they are always kept.)
    pub droppable: Vec<NodeId>,
    /// `(import node, target)` for wildcard `{*}` imports: their target's decls are kept
    /// wholesale (we can't cheaply tell which unqualified names came from the wildcard).
    pub wildcards: Vec<(NodeId, ModuleRef)>,
    /// `#Import export ...` re-exports: the bound names are part of *this* module's public
    /// surface, so the import is never dropped and the re-exported target declarations are
    /// kept live (a consumer of this module may reference them through it — resolving that
    /// transitively is v2 work). `Origin::Namespace` re-exports the whole target.
    pub reexports: Vec<(ModuleRef, Origin)>,
    /// Pure side-effect imports (`#Import "path"` with no name binding) resolved to a bundled
    /// group. They bind nothing but still run the target's body, so when this module's group is
    /// loaded they unconditionally load their target (and are never dropped). `(node, target)`.
    pub side_effects: Vec<(NodeId, ModuleRef)>,
}

/// The whole resolution result for a program.
pub struct Resolved {
    pub modules: Vec<ModuleRef>,
    pub decls: HashMap<ModuleRef, DeclTable>,
    pub imports: HashMap<ModuleRef, ImportTable>,
    /// Per-module seed roots: every always-live statement of that module (auto-execute code,
    /// plus `static __New` classes whose construction has load-time side effects). A module's
    /// roots are only seeded into the worklist once its *group* is loaded — the entry group, or
    /// the target of a taken import (see `reach`). A module imported but never used therefore
    /// never has its roots seeded, so it can be shaken out whole.
    pub roots: HashMap<ModuleRef, Vec<NodeId>>,
}

/// Build the resolution tables for a linked program.
pub fn resolve(program: &Program, plan: &BundlePlan) -> Resolved {
    let import_group: HashMap<NodeId, GroupId> = plan
        .resolved_imports
        .iter()
        .map(|ri| (ri.node, ri.group))
        .collect();

    // group -> its primary module (modules[0], the `__Main` / entry module of that file),
    // which is what an `#Import` of the group's file resolves to.
    let group_primary: HashMap<GroupId, ModuleRef> = program
        .groups
        .iter()
        .filter_map(|g| {
            g.modules.first().map(|&m| {
                (
                    g.id,
                    ModuleRef {
                        group: g.id,
                        module: m,
                    },
                )
            })
        })
        .collect();

    let mut resolved = Resolved {
        modules: Vec::new(),
        decls: HashMap::new(),
        imports: HashMap::new(),
        roots: HashMap::new(),
    };

    for group in &program.groups {
        for &module_id in &group.modules {
            let mref = ModuleRef {
                group: group.id,
                module: module_id,
            };
            resolved.modules.push(mref);

            let NodeKind::Module(module) = &program.arena[module_id].kind else {
                continue;
            };

            let mut decls = DeclTable::default();
            let mut imports = ImportTable::default();
            let mut roots = Vec::new();

            for &stmt in &module.body {
                match classify(program, stmt) {
                    TopLevel::Import => collect_import(
                        program,
                        stmt,
                        &import_group,
                        &group_primary,
                        &mut imports,
                    ),
                    TopLevel::Root => roots.push(stmt),
                    TopLevel::Decl { name, also_root } => {
                        decls.all.push(stmt);
                        decls.by_name.entry(name).or_default().push(stmt);
                        if also_root {
                            roots.push(stmt);
                        }
                    }
                }
            }

            resolved.decls.insert(mref, decls);
            resolved.imports.insert(mref, imports);
            resolved.roots.insert(mref, roots);
        }
    }

    resolved
}

/// How a top-level statement participates in reachability.
enum TopLevel {
    /// An `#Import` directive — recorded in the import table, not a root.
    Import,
    /// Always-live auto-execute code (and `export`ed/global vars whose initializer runs).
    Root,
    /// A declaration: live only if referenced. `also_root` for a class with `static __New`.
    Decl { name: String, also_root: bool },
}

fn classify(program: &Program, stmt: NodeId) -> TopLevel {
    match &program.arena[stmt].kind {
        NodeKind::ImportDirective(_) => TopLevel::Import,
        NodeKind::Function(f) => match f.name {
            Some(span) => TopLevel::Decl {
                name: ident(program, span),
                also_root: false,
            },
            // An anonymous top-level function is unusual; keep it (root) to be safe.
            None => TopLevel::Root,
        },
        NodeKind::ClassDecl(t) | NodeKind::StructDecl(t) => decl_for_type(program, stmt, t),
        NodeKind::ExportDecl { decl, .. } => classify_export(program, stmt, *decl),
        // Everything else — expression statements, assignments, `VarDecl` (its initializer
        // runs at startup), control flow, hotkeys/hotstrings, directives, labels, blocks — is
        // auto-execute code and always kept.
        _ => TopLevel::Root,
    }
}

/// Classify an `export ...` statement by its inner declaration, but key/store it under the
/// outer `ExportDecl` node (`stmt`) so deletion removes the `export` keyword too.
fn classify_export(program: &Program, stmt: NodeId, inner: NodeId) -> TopLevel {
    match &program.arena[inner].kind {
        NodeKind::Function(f) => match f.name {
            Some(span) => TopLevel::Decl {
                name: ident(program, span),
                also_root: false,
            },
            None => TopLevel::Root,
        },
        NodeKind::ClassDecl(t) | NodeKind::StructDecl(t) => decl_for_type(program, stmt, t),
        // `export global X := ...` — the initializer runs at startup, so keep it.
        _ => TopLevel::Root,
    }
}

fn decl_for_type(program: &Program, outer: NodeId, t: &ahkbuild_ir::node::TypeDecl) -> TopLevel {
    let name = t
        .name
        .map(|s| ident(program, s))
        .unwrap_or_else(|| format!("<anon@{}>", outer.0));
    TopLevel::Decl {
        name,
        also_root: has_static_new(program, t),
    }
}

/// A class/struct with a `static __New` runs construction logic at load time — a root.
/// Recurses into nested types (mirrors the AHK `_HasStaticNew`).
fn has_static_new(program: &Program, t: &ahkbuild_ir::node::TypeDecl) -> bool {
    for &m in &t.methods {
        if let NodeKind::Function(f) = &program.arena[m].kind {
            if f.is_static && f.name.is_some_and(|s| ident(program, s) == "__new") {
                return true;
            }
        }
    }
    t.nested.iter().any(|&n| {
        matches!(&program.arena[n].kind, NodeKind::ClassDecl(nt) | NodeKind::StructDecl(nt) if has_static_new(program, nt))
    })
}

/// Record one `#Import` directive into a module's import table.
fn collect_import(
    program: &Program,
    node: NodeId,
    import_group: &HashMap<NodeId, GroupId>,
    group_primary: &HashMap<GroupId, ModuleRef>,
    imports: &mut ImportTable,
) {
    let NodeKind::ImportDirective(d) = &program.arena[node].kind else {
        return;
    };
    // Only imports the linker resolved to a bundled file create cross-module name edges.
    // In-group `#Module` refs, the builtin `AHK`, embedded `*RES`, and unresolved imports
    // are left untouched (never dropped, no decl edges).
    let Some(target) = import_group
        .get(&node)
        .and_then(|g| group_primary.get(g))
        .copied()
    else {
        return;
    };

    let reexport = d.reexport;
    let mut binds_name = false;
    match &d.binding {
        ImportBinding::Whole => {
            // `#Import X` binds the namespace `X`; `#Import "path"` (quoted) binds nothing.
            if let ImportSource::Name(s) = &d.source {
                let local = ident(program, *s);
                imports.by_name.insert(
                    local,
                    ImportTarget {
                        node,
                        target,
                        origin: Origin::Namespace,
                    },
                );
                binds_name = true;
            } else if !reexport {
                // `#Import "path"` (quoted, no name): a pure side-effect import — it runs the
                // target's body but binds nothing, so it always loads its target and is kept.
                imports.side_effects.push((node, target));
            }
            if reexport {
                imports.reexports.push((target, Origin::Namespace));
            }
        }
        ImportBinding::Alias(a) => {
            imports.by_name.insert(
                ident(program, *a),
                ImportTarget {
                    node,
                    target,
                    origin: Origin::Namespace,
                },
            );
            binds_name = true;
            if reexport {
                imports.reexports.push((target, Origin::Namespace));
            }
        }
        ImportBinding::Selective { wildcard, names } => {
            for n in names {
                let origin = ident(program, n.name);
                let local = n.alias.map(|a| ident(program, a)).unwrap_or_else(|| origin.clone());
                if reexport {
                    imports.reexports.push((target, Origin::Name(origin.clone())));
                }
                imports.by_name.insert(
                    local,
                    ImportTarget {
                        node,
                        target,
                        origin: Origin::Name(origin),
                    },
                );
                binds_name = true;
            }
            if *wildcard {
                // `{*}` (or `{*, Extra}`): keep the whole target — can't tell which
                // unqualified names it pulls in.
                imports.wildcards.push((node, target));
                if reexport {
                    imports.reexports.push((target, Origin::Namespace));
                }
            }
        }
    }

    // A re-export is part of this module's public surface, so it is never dropped (even if
    // unreferenced locally); only ordinary name-binding imports are drop candidates.
    if binds_name && !reexport {
        imports.droppable.push(node);
    }
}

/// The lowercased identity text of a name span (AHK names are case-insensitive).
fn ident(program: &Program, span: Span) -> String {
    program.span_text(span).trim().to_ascii_lowercase()
}
