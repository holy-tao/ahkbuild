//! The global member-name table: every member name that any live code could reference.
//!
//! Member-level pruning is **name-based, not type-based** (type inference is infeasible for
//! dynamically-typed AHK). We scan the whole program for every member name that could be
//! accessed at runtime — static `obj.Foo`, dynamic `obj.Get%x%` / `obj.%"lit"%`, and string
//! arguments to reflection builtins like `ObjBindMethod` — and record exact names plus
//! prefix/suffix patterns for the dynamic forms. A live class then keeps only the members
//! whose names this table could match (plus protected meta-members); the rest shake out.
//!
//! If any member access is *fully* dynamic with no extractable constant (`obj.%v%`), the
//! table "blows up": member pruning is disabled program-wide and every live class is kept
//! whole. Over-keeping is safe; under-keeping would drop live code. This is a global switch,
//! distinct from `reach`'s per-module name blow-up.
//!
//! Ports `build/treeshake.ahk` (`_CollectMemberNames` and friends) and
//! `build/membernametable.ahk`.

use std::collections::HashMap;

use ahkbuild_fold::{Branch, FoldResult};
use ahkbuild_ir::node::{CallExpr, LiteralKind};
use ahkbuild_ir::{children, NodeId, NodeKind, Program};

/// Meta-functions the AHK runtime can invoke implicitly (without explicit user code), so they
/// are never pruned from a live class. Includes the v2.1 additions `__Ref` and `__Value`.
const PROTECTED: &[&str] = &[
    "__new", "__delete", "__call", "__get", "__set", "__item", "__enum", "call", "__ref",
    "__value", "__init", "tostring",
];

/// Whether `name` (any case) is a protected meta-member that must never be pruned.
pub fn is_protected(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    PROTECTED.contains(&n.as_str())
}

/// Reflection builtins that take a member name as a string argument. Returns the 0-based index
/// of the name argument *within `CallExpr.args`* (which excludes the callee). The legacy
/// 1-based indices (`ObjBindMethod` 2, `GetOwnPropDesc` 1, `GetMethod` 2) become 0-based here.
fn reflection_arg_index(callee: &str) -> Option<usize> {
    match callee.trim().to_ascii_lowercase().as_str() {
        "objbindmethod" => Some(1),
        "getownpropdesc" => Some(0),
        "getmethod" => Some(1),
        _ => None,
    }
}

/// Every member name potentially referenced in the program, for per-member pruning.
#[derive(Default)]
pub struct MemberNameTable {
    /// Lowercased exact member name -> the nodes that reference it.
    exact: HashMap<String, Vec<NodeId>>,
    /// Lowercased prefix -> referencing nodes. A member matches if its name starts with this.
    prefixes: HashMap<String, Vec<NodeId>>,
    /// Lowercased suffix -> referencing nodes. A member matches if its name ends with this.
    suffixes: HashMap<String, Vec<NodeId>>,
    /// A fully-dynamic member access defeated analysis: keep every member of every live class.
    blown: bool,
}

impl MemberNameTable {
    pub fn is_blown(&self) -> bool {
        self.blown
    }

    fn blow_up(&mut self) {
        self.blown = true;
    }

    fn add_exact(&mut self, name: &str, by: NodeId) {
        self.exact.entry(normalize(name)).or_default().push(by);
    }

    fn add_prefix(&mut self, prefix: &str, by: NodeId) {
        let p = normalize(prefix);
        if !p.is_empty() {
            self.prefixes.entry(p).or_default().push(by);
        }
    }

    fn add_suffix(&mut self, suffix: &str, by: NodeId) {
        let s = normalize(suffix);
        if !s.is_empty() {
            self.suffixes.entry(s).or_default().push(by);
        }
    }

    /// The nodes that could reference member `name`, or `None` if nothing can. A blown table
    /// or a prefix/suffix hit returns `Some(empty)` — the member is referenced, but by a
    /// pattern with no single referencing node. Ports `MemberNameTable.Matches`.
    pub fn matches(&self, name: &str) -> Option<Vec<NodeId>> {
        if self.blown {
            return Some(Vec::new());
        }
        let key = normalize(name);
        if let Some(nodes) = self.exact.get(&key) {
            return Some(nodes.clone());
        }
        for prefix in self.prefixes.keys() {
            if key.starts_with(prefix.as_str()) {
                return Some(Vec::new());
            }
        }
        for suffix in self.suffixes.keys() {
            if key.ends_with(suffix.as_str()) {
                return Some(Vec::new());
            }
        }
        None
    }

    /// Total recorded referencer entries across the exact/prefix/suffix maps. Only ever
    /// *decreases* as referencers inside dead code are stripped, so the shake fixpoint uses a
    /// stable count to detect convergence.
    pub fn referencer_count(&self) -> usize {
        let sum = |m: &HashMap<String, Vec<NodeId>>| m.values().map(Vec::len).sum::<usize>();
        sum(&self.exact) + sum(&self.prefixes) + sum(&self.suffixes)
    }

    /// Drop every referencer that lies inside `parent`'s subtree, removing keys left empty.
    /// Used after a `DefineProp` call is pruned so names referenced only inside that call's
    /// descriptor become prunable too. Ports `RemoveDescendantReferencers`.
    pub fn remove_descendant_referencers(&mut self, parent: NodeId, program: &Program) {
        let clean = |map: &mut HashMap<String, Vec<NodeId>>| {
            map.retain(|_, nodes| {
                nodes.retain(|&n| !is_descendant_of(program, n, parent));
                !nodes.is_empty()
            });
        };
        clean(&mut self.exact);
        clean(&mut self.prefixes);
        clean(&mut self.suffixes);
    }
}

/// Scan the whole program and build the member-name table. A member access that `fold` resolved
/// to a literal (e.g. a getter-const `ClassName.Value`) is not counted as a reference, so an
/// otherwise-unreferenced getter-only property can shake out.
pub fn collect(program: &Program, fold: Option<&FoldResult>) -> MemberNameTable {
    let mut table = MemberNameTable::default();
    for module in program.modules() {
        collect_node(program, &mut table, module, module, fold);
    }
    table
}

/// The surviving arm node(s) of an `if`/ternary whose condition folded to a build-time constant
/// (empty for a dead `if` with no `else`), or `None` when `node` is not a resolved branch. Shared
/// with the member-name scan and [`reach`](crate::reach) so both descend into the same arm.
pub(crate) fn surviving_arm(
    program: &Program,
    fold: Option<&FoldResult>,
    node: NodeId,
) -> Option<Vec<NodeId>> {
    let branch = fold?.branches.get(&node).copied()?;
    match &program.arena[node].kind {
        NodeKind::IfStmt {
            then_body,
            else_body,
            ..
        } => Some(match branch {
            Branch::Then => vec![*then_body],
            Branch::Else => else_body.iter().copied().collect(),
            Branch::Dead => Vec::new(),
        }),
        NodeKind::TernaryExpr {
            then_branch,
            else_branch,
            ..
        } => Some(match branch {
            Branch::Then => vec![*then_branch],
            Branch::Else | Branch::Dead => vec![*else_branch],
        }),
        _ => None,
    }
}

/// Walk `node`, recording any member names it references. `stmt` is the nearest enclosing
/// directive-bearing statement (a module/block body element or a class member) - the node a
/// `;@AhkBuild-ResolvesTo` directive would attach to.
fn collect_node(
    program: &Program,
    table: &mut MemberNameTable,
    node: NodeId,
    stmt: NodeId,
    fold: Option<&FoldResult>,
) {
    if table.is_blown() {
        return;
    }
    match &program.arena[node].kind {
        NodeKind::MemberAccess {
            member, is_dynamic, ..
        } => {
            // A member access a user constant folded away is no longer a real reference.
            if !fold.is_some_and(|f| f.folded_reads.contains(&node)) {
                if *is_dynamic {
                    extract_dynamic_member(program, table, *member, stmt);
                } else {
                    let name = program.text(*member).to_string();
                    table.add_exact(&name, node);
                }
            }
        }
        NodeKind::CallExpr(c) => check_reflection_call(program, table, c, node, stmt),
        _ => {}
    }

    // A folded `if`/ternary: only the surviving arm can run, so collect names from it alone (the
    // condition is a build-time constant and the dead arm is removed at emit).
    if let Some(arm) = surviving_arm(program, fold, node) {
        for child in arm {
            collect_node(program, table, child, stmt, fold);
        }
        return;
    }

    // Directives attach to body elements and class members (see `lower::attach_directives`):
    // descending into a block/module/type-decl makes each child its own enclosing statement.
    let descends_to_stmt = matches!(
        &program.arena[node].kind,
        NodeKind::Block { .. }
            | NodeKind::Module(_)
            | NodeKind::ClassDecl(_)
            | NodeKind::StructDecl(_)
    );
    for child in children(&program.arena[node].kind) {
        let next = if descends_to_stmt { child } else { stmt };
        collect_node(program, table, child, next, fold);
    }
}

/// Extract constant parts from a dynamic member access's `member` expression — outer
/// prefix/suffix identifiers and string literals inside `%...%` derefs — or blow up the table
/// if there is no constant to anchor on. Ports `_ExtractDynamicMemberParts`.
fn extract_dynamic_member(
    program: &Program,
    table: &mut MemberNameTable,
    member: NodeId,
    stmt: NodeId,
) {
    let mut has_constant = false;
    let mut outer_prefix = String::new();
    let mut outer_suffix = String::new();
    let mut derefs: Vec<NodeId> = Vec::new();

    match &program.arena[member].kind {
        // A bare `%expr%` member: the whole thing is one deref, no outer parts.
        NodeKind::DerefExpr { .. } => derefs.push(member),
        // `pre%expr%post`: identifier parts are constant text, deref parts are dynamic.
        NodeKind::DynamicIdentifier { parts } => {
            for &part in parts {
                match &program.arena[part].kind {
                    NodeKind::Identifier => {
                        if derefs.is_empty() {
                            outer_prefix.push_str(program.text(part).trim());
                        } else {
                            // Keep only the trailing identifier as the suffix.
                            outer_suffix = program.text(part).trim().to_string();
                        }
                        has_constant = true;
                    }
                    NodeKind::DerefExpr { .. } => {
                        derefs.push(part);
                        outer_suffix.clear();
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    // Inspect each deref's inner expression for string literals we can pin down.
    for &deref in &derefs {
        let NodeKind::DerefExpr { inner } = &program.arena[deref].kind else {
            continue;
        };
        match &program.arena[*inner].kind {
            // `%"literal"%` -> an exact name (with any outer prefix/suffix).
            NodeKind::Literal {
                kind: LiteralKind::String,
            } => {
                let lit = string_literal_text(program.text(*inner));
                let name = format!("{outer_prefix}{lit}{outer_suffix}");
                table.add_exact(&name, deref);
                has_constant = true;
            }
            // `%"pre" . v%` / `%v . "post"%` -> a prefix / suffix pattern.
            NodeKind::BinaryExpr { left, op, right } if is_concat(program, *op) => {
                if let Some(lit) = string_literal(program, *left) {
                    table.add_prefix(&format!("{outer_prefix}{lit}"), deref);
                    has_constant = true;
                }
                if let Some(lit) = string_literal(program, *right) {
                    table.add_suffix(&format!("{lit}{outer_suffix}"), deref);
                    has_constant = true;
                }
            }
            _ => {}
        }
    }

    if !outer_prefix.is_empty() {
        table.add_prefix(&outer_prefix, member);
        has_constant = true;
    }
    if !outer_suffix.is_empty() {
        table.add_suffix(&outer_suffix, member);
        has_constant = true;
    }

    // Last resort: an explicit `;@AhkBuild-ResolvesTo A, B` directive names the targets.
    if let Some(names) = resolves_to(program, stmt) {
        for name in names {
            table.add_exact(&name, member);
        }
        has_constant = true;
    }

    if !has_constant {
        blow_up_or_trust(program, table, stmt, member, "dynamic member access");
    }
}

/// Either blow up the member-name table (disabling per-member pruning program-wide) because a
/// dynamic construct at `at` has no extractable name, or - if the enclosing statement carries
/// `;@ahkbuild-safe` - trust the author that the access is safe and keep pruning. Traces either
/// way so a user can see (`-v` for the suppression, default for the blow-up) what defeated
/// pruning and how to fix it. `what` names the construct for the message.
fn blow_up_or_trust(
    program: &Program,
    table: &mut MemberNameTable,
    stmt: NodeId,
    at: NodeId,
    what: &str,
) {
    if program.has_directive(stmt, "ahkbuild-safe") {
        tracing::debug!(
            at = %program.node_location(at),
            "{what} marked ;@ahkbuild-safe - keeping per-member pruning enabled",
        );
        return;
    }
    tracing::warn!(
        at = %program.node_location(at),
        "{what} with no extractable name disables per-member pruning program-wide (every class \
         kept whole); annotate the statement with ;@ahkbuild-safe or ;@AhkBuild-ResolvesTo to \
         keep pruning",
    );
    table.blow_up();
}

/// If `call` is a reflection builtin (`ObjBindMethod`/`GetMethod`/`GetOwnPropDesc`) with a
/// plain-identifier callee, extract the member name from its string argument. Ports
/// `_CheckReflectionCall`.
fn check_reflection_call(
    program: &Program,
    table: &mut MemberNameTable,
    call: &CallExpr,
    call_node: NodeId,
    stmt: NodeId,
) {
    let NodeKind::Identifier = &program.arena[call.callee].kind else {
        return;
    };
    let Some(idx) = reflection_arg_index(program.text(call.callee)) else {
        return;
    };
    let Some(&arg) = call.args.get(idx) else {
        return;
    };
    extract_string_expr(program, table, arg, call_node, stmt);
}

/// Pull a member name out of a string-valued expression: a literal -> exact; a concat with a
/// literal side -> prefix/suffix; a `;@AhkBuild-ResolvesTo` directive -> exacts; otherwise the
/// name is unknowable and the table blows up. Ports `_ExtractStringExprParts`.
fn extract_string_expr(
    program: &Program,
    table: &mut MemberNameTable,
    expr: NodeId,
    context: NodeId,
    stmt: NodeId,
) {
    if let Some(lit) = string_literal(program, expr) {
        table.add_exact(&lit, context);
        return;
    }
    if let NodeKind::BinaryExpr { left, op, right } = &program.arena[expr].kind {
        if is_concat(program, *op) {
            let mut has_constant = false;
            if let Some(lit) = string_literal(program, *left) {
                table.add_prefix(&lit, expr);
                has_constant = true;
            }
            if let Some(lit) = string_literal(program, *right) {
                table.add_suffix(&lit, expr);
                has_constant = true;
            }
            if has_constant {
                return;
            }
        }
    }
    if let Some(names) = resolves_to(program, stmt) {
        for name in names {
            table.add_exact(&name, expr);
        }
        return;
    }
    blow_up_or_trust(
        program,
        table,
        stmt,
        expr,
        "reflection call with a non-literal name",
    );
}

/// The `;@AhkBuild-ResolvesTo` names attached to statement `stmt`, if any. Directives are keyed
/// by the statement they precede (see `lower`).
fn resolves_to(program: &Program, stmt: NodeId) -> Option<Vec<String>> {
    let args = program.directive_arg(stmt, "ahkbuild-resolvesto")?;
    Some(parse_resolves_to(args))
}

/// Parse a `;@AhkBuild-ResolvesTo` argument string into names: whitespace/comma-separated
/// tokens, with quoted runs kept whole and unquoted. Mirrors `Directives.ParseResolvesToArgs`.
fn parse_resolves_to(args: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut chars = args.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\r' | '\n' | ',' => {
                chars.next();
            }
            '"' | '\'' => {
                let quote = c;
                chars.next();
                let mut s = String::new();
                for ch in chars.by_ref() {
                    if ch == quote {
                        break;
                    }
                    s.push(ch);
                }
                names.push(s);
            }
            _ => {
                let mut s = String::new();
                while let Some(&ch) = chars.peek() {
                    if matches!(ch, ' ' | '\t' | '\r' | '\n' | ',') {
                        break;
                    }
                    s.push(ch);
                    chars.next();
                }
                names.push(s);
            }
        }
    }
    names
}

/// The unquoted text of a string-literal node, or `None` if `expr` isn't a string literal.
fn string_literal(program: &Program, expr: NodeId) -> Option<String> {
    match &program.arena[expr].kind {
        NodeKind::Literal {
            kind: LiteralKind::String,
        } => Some(string_literal_text(program.text(expr)).to_string()),
        _ => None,
    }
}

/// Whether the binary operator span is a concatenation: explicit `.` or an implicit-concat
/// whitespace gap (which trims to empty). Mirrors the legacy `op == "." || op == " "` check.
fn is_concat(program: &Program, op: ahkbuild_ir::Span) -> bool {
    let t = program.span_text(op).trim();
    t == "." || t.is_empty()
}

/// Strip the surrounding quotes from a string-literal's source text.
fn string_literal_text(text: &str) -> &str {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' || first == b'\'') && last == first {
            return &text[1..text.len() - 1];
        }
    }
    text
}

/// `trim`ed, lowercased member-name identity (AHK names are case-insensitive).
fn normalize(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Whether `node` lies within `ancestor`'s subtree (strictly below it).
pub fn is_descendant_of(program: &Program, node: NodeId, ancestor: NodeId) -> bool {
    let mut stack = children(&program.arena[ancestor].kind);
    while let Some(n) = stack.pop() {
        if n == node {
            return true;
        }
        stack.extend(children(&program.arena[n].kind));
    }
    false
}
