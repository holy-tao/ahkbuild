//! User-defined constant detection.
//!
//! Build-time folding ([`crate::fold`]) only knows `A_IsCompiled` / `A_PtrSize`. This pass adds
//! *user* constants: names assigned exactly once and never reassigned, getter-only fat-arrow
//! properties, and anything explicitly marked `;@ahkbuild-const`. It produces a map from each
//! **read-site** node ([`Identifier`] or static [`MemberAccess`]) to its constant value; the
//! evaluator then resolves those reads like any other constant, so the existing maximal-
//! substitution and branch passes pick them up unchanged.
//!
//! [`Identifier`]: ahkbuild_ir::NodeKind::Identifier
//! [`MemberAccess`]: ahkbuild_ir::NodeKind::MemberAccess

use std::collections::{HashMap, HashSet};

use ahkbuild_ir::node::{LiteralKind, NodeKind, VarDecl, VarScope};
use ahkbuild_ir::{children, NodeId, Program};

use crate::{ConstValue, Constants, Evaluator};

const DIRECTIVE: &str = "ahkbuild-const";

/// Detect user constants in `program` and return each read-site node's value plus the declaration
/// statements that become dead once every read folds.
pub fn collect(program: &Program, consts: &Constants) -> UserConsts {
    let mut b = Builder::new(program);
    for m in program.modules() {
        let s = b.new_scope(None, true);
        b.walk(m, s, m);
    }
    let binds = b.analyze();
    let members = Members::collect(program);

    let mut cands = b.var_candidates(&binds);
    cands.extend(members.candidates());

    // Resolve constants until a pass produces no additional changes.
    let mut result = UserConsts::default();
    let mut pending: Vec<Candidate> = cands;
    loop {
        let mut resolved = Vec::new();
        {
            let ev = Evaluator {
                program,
                consts,
                user_consts: &result.reads,
            };
            for (i, c) in pending.iter().enumerate() {
                if let Some(v) = ev.eval(c.expr) {
                    resolved.push((i, v));
                }
            }
        }
        if resolved.is_empty() {
            break;
        }
        // Apply in reverse index order so swap_remove keeps the remaining indices valid.
        for (i, v) in resolved.iter().rev() {
            if worth_substituting(
                program,
                v,
                &pending[*i].reads,
                !pending[*i].removable.is_empty(),
            ) {
                for &site in &pending[*i].reads {
                    result.reads.insert(site, v.clone());
                }
                // Every read folded, so the declaration is now dead (when it is safe to delete).
                result
                    .dead_consts
                    .extend(pending[*i].removable.iter().copied());
            }
            pending.swap_remove(*i);
        }
    }
    result
}

/// A constant to resolve
struct Candidate {
    /// The expression to resolve
    expr: NodeId,
    /// Nodes to replace when folding `expr`
    reads: Vec<NodeId>,
    /// Statement(s) to delete once `expr` folds (the constant's whole declaration). Empty for
    /// getter-property candidates and for variable bindings that aren't safe to delete.
    removable: Vec<NodeId>,
}

/// What [`collect`] found: the read-site -> value map (fed to the evaluator) and the set of
/// declaration statements whose every read folded and that are safe to delete.
#[derive(Default)]
pub struct UserConsts {
    pub reads: HashMap<NodeId, ConstValue>,
    pub dead_consts: HashSet<NodeId>,
}

/// One function-like scope (module / function / arrow body).
#[derive(Default)]
struct Scope {
    parent: Option<usize>,
    is_module: bool,
    /// Parameter names.
    params: HashSet<String>,
    /// Explicit `local` / `static` declarations.
    locals: HashSet<String>,
    /// Explicit `global` declarations (they alias the module scope).
    globals: HashSet<String>,
    /// Names written anywhere in this scope's own region (excluding nested functions).
    assigned: HashSet<String>,
    /// Names this scope *binds* - params, explicit locals, and assume-local assignments that no
    /// enclosing function already binds. Filled by [`Builder::resolve_bindings`].
    bound: HashSet<String>,
    /// Whether an un-pinnable dynamic write (`%expr% := …`) occurs in this scope's region.
    dynamic_write: bool,
    /// Whether a dynamic *read* (`%expr%`) occurs in this scope's region. It could read any
    /// visible name, so a binding it can reach must not have its declaration deleted (folding the
    /// static reads is still safe). Conservative: any deref read sets this, pinned or not.
    dynamic_read: bool,
}

/// One occurrence of a name, before it is resolved to a binding.
struct Occurrence {
    scope: usize,
    name: String,
    /// For a read: the `Identifier` node (the substitution site). For a definer: the `:=` RHS.
    node: NodeId,
    /// The enclosing statement (a module/block body element). Used to delete a fully-folded
    /// constant's whole declaration.
    stmt: NodeId,
    kind: OccKind,
}

enum OccKind {
    Read,
    Definer,
    Disqualify,
}

/// Resolution result for one (binding scope, name): its definer(s), reads, and gating flags.
#[derive(Default)]
struct Binding {
    definers: Vec<NodeId>,
    /// The statement node enclosing each definer, in lockstep with `definers`.
    definer_stmts: Vec<NodeId>,
    reads: Vec<NodeId>,
    disqualified: bool,
    /// Reachable by an un-pinnable dynamic write.
    tainted: bool,
    /// Reachable by a dynamic read - blocks deleting the declaration (folding is still fine).
    read_tainted: bool,
    /// Marked `;@ahkbuild-const` - fold on the author's word.
    directive: bool,
}

struct Builder<'a> {
    program: &'a Program,
    scopes: Vec<Scope>,
    occurrences: Vec<Occurrence>,
    /// The scope each visited node belongs to (used to place a directive's binding).
    node_scope: HashMap<NodeId, usize>,
}

impl<'a> Builder<'a> {
    fn new(program: &'a Program) -> Self {
        Builder {
            program,
            scopes: Vec::new(),
            occurrences: Vec::new(),
            node_scope: HashMap::new(),
        }
    }

    fn new_scope(&mut self, parent: Option<usize>, is_module: bool) -> usize {
        let idx = self.scopes.len();
        self.scopes.push(Scope {
            parent,
            is_module,
            ..Default::default()
        });
        idx
    }

    fn name(&self, id: NodeId) -> String {
        self.program.text(id).trim().to_ascii_lowercase()
    }

    fn read(&mut self, scope: usize, id: NodeId, stmt: NodeId) {
        let name = self.name(id);
        if !name.is_empty() {
            self.occurrences.push(Occurrence {
                scope,
                name,
                node: id,
                stmt,
                kind: OccKind::Read,
            });
        }
    }

    /// Record a write of `name` in `scope`: a `:=` definer carries its `rhs`; anything else
    /// disqualifies. `stmt` is the enclosing statement (used to delete a folded constant's decl).
    fn record_write(
        &mut self,
        scope: usize,
        name: String,
        rhs: NodeId,
        is_definer: bool,
        stmt: NodeId,
    ) {
        if name.is_empty() {
            return;
        }
        self.scopes[scope].assigned.insert(name.clone());
        let kind = if is_definer {
            OccKind::Definer
        } else {
            OccKind::Disqualify
        };
        self.occurrences.push(Occurrence {
            scope,
            name,
            node: rhs,
            stmt,
            kind,
        });
    }

    /// Register a `local`/`static`/`global` declaration's name in `scope`.
    fn declare_var(&mut self, v: &VarDecl, scope: usize) {
        let Some(s) = v.name else { return };
        let n = self.program.span_text(s).trim().to_ascii_lowercase();
        match v.scope {
            VarScope::Global => self.scopes[scope].globals.insert(n),
            VarScope::Local | VarScope::Static => self.scopes[scope].locals.insert(n),
        };
    }

    /// Walk `id` as an expression/statement in read position under `scope`. `stmt` is the
    /// enclosing statement; it resets to each child when descending a block or module body.
    fn walk(&mut self, id: NodeId, scope: usize, stmt: NodeId) {
        self.node_scope.insert(id, scope);
        let kind = self.program.arena[id].kind.clone();
        match kind {
            NodeKind::Function(f) => {
                let child = self.new_scope(Some(scope), false);
                for p in &f.params {
                    if let Some(s) = p.name {
                        let n = self.program.span_text(s).trim().to_ascii_lowercase();
                        self.scopes[child].params.insert(n);
                    }
                    if let Some(d) = p.default {
                        self.walk(d, child, d);
                    }
                }
                if let Some(body) = f.body {
                    self.walk(body, child, body);
                }
            }
            NodeKind::FatArrow { params, body } => {
                let child = self.new_scope(Some(scope), false);
                for p in &params {
                    if let Some(s) = p.name {
                        let n = self.program.span_text(s).trim().to_ascii_lowercase();
                        self.scopes[child].params.insert(n);
                    }
                    if let Some(d) = p.default {
                        self.walk(d, child, d);
                    }
                }
                self.walk(body, child, body);
            }
            NodeKind::VarDecl(v) => {
                self.declare_var(&v, scope);
                if let (Some(name), Some(init)) = (v.name, v.initializer) {
                    let n = self.program.span_text(name).trim().to_ascii_lowercase();
                    self.record_write(scope, n, init, true, stmt);
                }
                if let Some(init) = v.initializer {
                    self.walk(init, scope, stmt);
                }
            }
            NodeKind::BinaryExpr { left, op, right } => {
                let op = self.program.span_text(op).trim().to_string();
                match assign_kind(&op) {
                    Some(is_definer) => {
                        self.assign_target(left, right, is_definer, scope, stmt);
                        self.walk(right, scope, stmt);
                    }
                    None => {
                        self.walk(left, scope, stmt);
                        self.walk(right, scope, stmt);
                    }
                }
            }
            NodeKind::UnaryExpr { operand, op, .. } => {
                let op = self.program.span_text(op).trim().to_string();
                if matches!(op.as_str(), "++" | "--")
                    && matches!(self.program.arena[operand].kind, NodeKind::Identifier)
                {
                    let n = self.name(operand);
                    self.record_write(scope, n, operand, false, stmt);
                } else {
                    self.walk(operand, scope, stmt);
                }
            }
            NodeKind::VarRefExpr { operand } => {
                if matches!(self.program.arena[operand].kind, NodeKind::Identifier) {
                    let n = self.name(operand);
                    self.record_write(scope, n, operand, false, stmt);
                } else {
                    self.walk(operand, scope, stmt);
                }
            }
            NodeKind::MemberAccess {
                object,
                member,
                is_dynamic,
            } => {
                self.walk(object, scope, stmt);
                if is_dynamic {
                    self.walk(member, scope, stmt);
                }
            }
            // A dynamic read (`%expr%`) could read any visible name. It doesn't disqualify
            // folding (the static reads still fold), but it blocks deleting a reachable
            // declaration, so poison the scope for removal.
            NodeKind::DerefExpr { .. } | NodeKind::DynamicIdentifier { .. } => {
                self.scopes[scope].dynamic_read = true;
                for c in children(&self.program.arena[id].kind) {
                    self.walk(c, scope, stmt);
                }
            }
            NodeKind::ObjectLiteral { members } => {
                // Keys are member names, not variable reads; only values reference variables.
                for m in &members {
                    if let Some(v) = m.value {
                        self.walk(v, scope, stmt);
                    }
                }
            }
            NodeKind::Identifier => self.read(scope, id, stmt),
            _ => {
                // Block/module bodies introduce fresh statements; everything else inherits `stmt`.
                let descends = matches!(
                    self.program.arena[id].kind,
                    NodeKind::Block { .. } | NodeKind::Module(_)
                );
                for c in children(&self.program.arena[id].kind) {
                    let next = if descends { c } else { stmt };
                    self.walk(c, scope, next);
                }
            }
        }
    }

    /// Classify an assignment `target <op> rhs` (`is_definer` = the plain `:=` form). `stmt` is the
    /// enclosing statement.
    fn assign_target(
        &mut self,
        target: NodeId,
        rhs: NodeId,
        is_definer: bool,
        scope: usize,
        stmt: NodeId,
    ) {
        match &self.program.arena[target].kind {
            NodeKind::Identifier => {
                let n = self.name(target);
                self.record_write(scope, n, rhs, is_definer, stmt);
            }
            // `static x := …` / `local x := …` / `global x := …` lower the *target* to a `VarDecl`.
            NodeKind::VarDecl(v) => {
                let v = v.clone();
                self.declare_var(&v, scope);
                if let Some(s) = v.name {
                    let n = self.program.span_text(s).trim().to_ascii_lowercase();
                    self.record_write(scope, n, rhs, is_definer, stmt);
                }
            }
            NodeKind::DynamicIdentifier { .. } | NodeKind::DerefExpr { .. } => {
                match self.const_target_name(target) {
                    // A pinned dynamic write (`%"Foo"% := …`): an ordinary write to `Foo`.
                    Some(n) => self.record_write(scope, n, target, false, stmt),
                    // Un-pinnable: it could write anything visible here, so poison the scope.
                    None => self.scopes[scope].dynamic_write = true,
                }
            }
            // `obj.M := …` / `arr[i] := …` write a member/element, not a variable; the object is
            // still read. Member assignments are handled by the member-constant pass.
            _ => self.walk(target, scope, stmt),
        }
    }

    /// If `id` is a dynamic name made entirely of constant text (`%"Foo"%`, `pre%"x"%`), the
    /// resolved name; otherwise `None` (an un-pinnable target).
    fn const_target_name(&self, id: NodeId) -> Option<String> {
        match &self.program.arena[id].kind {
            NodeKind::DerefExpr { inner } => string_literal(self.program, *inner),
            NodeKind::DynamicIdentifier { parts } => {
                let mut s = String::new();
                for &p in parts {
                    match &self.program.arena[p].kind {
                        NodeKind::DerefExpr { inner } => {
                            s.push_str(&string_literal(self.program, *inner)?)
                        }
                        // A literal text chunk between derefs.
                        _ => s.push_str(self.program.text(p).trim()),
                    }
                }
                (!s.is_empty()).then(|| s.to_ascii_lowercase())
            }
            _ => None,
        }
    }

    /// Resolve every occurrence to its binding and return the per-binding tallies.
    fn analyze(&mut self) -> HashMap<(usize, String), Binding> {
        self.resolve_bindings();
        let tainted = self.tainted_scopes();
        let read_tainted = self.read_tainted_scopes();
        let mut binds: HashMap<(usize, String), Binding> = HashMap::new();
        for occ in &self.occurrences {
            let Some(bscope) = self.resolve(&occ.name, occ.scope) else {
                continue;
            };
            let b = binds.entry((bscope, occ.name.clone())).or_default();
            match occ.kind {
                OccKind::Read => b.reads.push(occ.node),
                OccKind::Definer => {
                    b.definers.push(occ.node);
                    b.definer_stmts.push(occ.stmt);
                }
                OccKind::Disqualify => b.disqualified = true,
            }
            if tainted.contains(&bscope) {
                b.tainted = true;
            }
            if read_tainted.contains(&bscope) {
                b.read_tainted = true;
            }
        }
        self.apply_directives(&mut binds);
        binds
    }

    /// Fill [`Scope::bound`]. Processed in index order - a parent always precedes its children, so
    /// each scope's enclosing-function bounds are ready when it is reached.
    fn resolve_bindings(&mut self) {
        for t in 0..self.scopes.len() {
            let mut bound: HashSet<String> = self.scopes[t].params.clone();
            bound.extend(self.scopes[t].locals.iter().cloned());
            let assigned: Vec<String> = self.scopes[t].assigned.iter().cloned().collect();
            for n in assigned {
                // A `global`-declared name aliases the module; a name an enclosing function binds
                // is captured, not a new local. Everything else is assume-local to `t`.
                if self.scopes[t].globals.contains(&n) || self.captured_by_enclosing_fn(t, &n) {
                    continue;
                }
                bound.insert(n);
            }
            self.scopes[t].bound = bound;
        }
    }

    /// Whether a function scope strictly enclosing `scope` binds `name` (so a write here captures
    /// it). The module boundary blocks capture: module globals never reach into a function.
    fn captured_by_enclosing_fn(&self, scope: usize, name: &str) -> bool {
        let mut a = self.scopes[scope].parent;
        while let Some(p) = a {
            if self.scopes[p].is_module {
                break;
            }
            if self.scopes[p].bound.contains(name) {
                return true;
            }
            a = self.scopes[p].parent;
        }
        false
    }

    /// Resolve a use of `name` in `scope` to the scope that binds it, climbing the closure chain.
    fn resolve(&self, name: &str, scope: usize) -> Option<usize> {
        if self.scopes[scope].globals.contains(name) {
            return self.module_of(scope);
        }
        let mut t = scope;
        loop {
            if self.scopes[t].bound.contains(name) {
                return Some(t);
            }
            match self.scopes[t].parent {
                // A function only captures from an *enclosing function*, never the module globals.
                Some(p) if !self.scopes[p].is_module => t = p,
                _ => return None,
            }
        }
    }

    fn module_of(&self, mut scope: usize) -> Option<usize> {
        loop {
            if self.scopes[scope].is_module {
                return Some(scope);
            }
            scope = self.scopes[scope].parent?;
        }
    }

    /// Scopes from which an un-pinnable dynamic write could reach a binding (the writing scope and
    /// every scope enclosing it).
    fn tainted_scopes(&self) -> HashSet<usize> {
        let mut out = HashSet::new();
        for s in 0..self.scopes.len() {
            if !self.scopes[s].dynamic_write {
                continue;
            }
            let mut a = Some(s);
            while let Some(t) = a {
                out.insert(t);
                a = self.scopes[t].parent;
            }
        }
        out
    }

    /// Scopes a dynamic read could reach (the reading scope and every scope enclosing it). A
    /// binding here keeps its declaration even when every static read folds.
    fn read_tainted_scopes(&self) -> HashSet<usize> {
        let mut out = HashSet::new();
        for s in 0..self.scopes.len() {
            if !self.scopes[s].dynamic_read {
                continue;
            }
            let mut a = Some(s);
            while let Some(t) = a {
                out.insert(t);
                a = self.scopes[t].parent;
            }
        }
        out
    }

    /// Whether `stmt` is a plain `name := <expr>` / `local|static name := <expr>` declaration whose
    /// RHS is exactly `expr`, so deleting the whole statement removes only this binding. Rejects
    /// `export`-wrapped statements (public API, may be read outside the bundle) and any nested or
    /// compound assignment.
    fn is_removable_stmt(&self, stmt: NodeId, expr: NodeId) -> bool {
        // The statement may be a bare assignment/declaration or wrapped in an `ExpressionStatement`;
        // an `ExportDecl` (or block, etc.) unwraps to itself and is rejected below.
        let inner = match &self.program.arena[stmt].kind {
            NodeKind::ExpressionStatement { expr } => *expr,
            _ => stmt,
        };
        match &self.program.arena[inner].kind {
            NodeKind::BinaryExpr { left, op, right } => {
                *right == expr
                    && assign_kind(self.program.span_text(*op).trim()) == Some(true)
                    && matches!(
                        self.program.arena[*left].kind,
                        NodeKind::Identifier | NodeKind::VarDecl(_)
                    )
            }
            NodeKind::VarDecl(v) => v.initializer == Some(expr),
            _ => false,
        }
    }

    /// Mark bindings named by a `;@ahkbuild-const` directive as trusted constants.
    fn apply_directives(&self, binds: &mut HashMap<(usize, String), Binding>) {
        let stmts = self
            .program
            .directives
            .keys()
            .filter(|&s| self.program.has_directive(*s, DIRECTIVE));
        for stmt in stmts {
            if let Some(key) = self.directive_binding(*stmt) {
                if let Some(b) = binds.get_mut(&key) {
                    b.directive = true;
                }
            }
        }
    }

    /// The (binding scope, name) defined by a directive-carrying statement.
    fn directive_binding(&self, stmt: NodeId) -> Option<(usize, String)> {
        let inner = match &self.program.arena[stmt].kind {
            NodeKind::ExportDecl { decl, .. } => *decl,
            NodeKind::ExpressionStatement { expr } => *expr,
            _ => stmt,
        };
        let name = match &self.program.arena[inner].kind {
            NodeKind::BinaryExpr { left, op, .. }
                if assign_kind(self.program.span_text(*op).trim()).is_some() =>
            {
                match &self.program.arena[*left].kind {
                    NodeKind::Identifier => self.name(*left),
                    NodeKind::VarDecl(v) => {
                        self.program.span_text(v.name?).trim().to_ascii_lowercase()
                    }
                    _ => return None,
                }
            }
            NodeKind::VarDecl(v) => self.program.span_text(v.name?).trim().to_ascii_lowercase(),
            _ => return None,
        };
        let scope = *self
            .node_scope
            .get(&inner)
            .or_else(|| self.node_scope.get(&stmt))?;
        Some((self.resolve(&name, scope)?, name))
    }

    /// Turn qualifying bindings into [`Candidate`]s. Each candidate also carries the declaration
    /// statement(s) safe to delete once it folds (empty when removal isn't provable).
    fn var_candidates(&self, binds: &HashMap<(usize, String), Binding>) -> Vec<Candidate> {
        let mut out = Vec::new();
        for b in binds.values() {
            if b.reads.is_empty() || b.definers.is_empty() {
                continue;
            }
            // A trusted directive folds on the first definer, skipping every safety check. Removal
            // follows the same trust: every `:=` write is deletable (the structural export guard in
            // `is_removable_stmt` still applies); any compound write is left as a dead store.
            if b.directive {
                let removable = b
                    .definer_stmts
                    .iter()
                    .zip(&b.definers)
                    .filter(|(&stmt, &expr)| self.is_removable_stmt(stmt, expr))
                    .map(|(&stmt, _)| stmt)
                    .collect();
                out.push(Candidate {
                    expr: b.definers[0],
                    reads: b.reads.clone(),
                    removable,
                });
                continue;
            }
            if b.tainted || b.disqualified || b.definers.len() != 1 {
                continue;
            }
            // Removable only when no dynamic read could reach it and the declaration is a lone,
            // deletable statement (not exported, not a nested/compound assignment).
            let removable =
                if !b.read_tainted && self.is_removable_stmt(b.definer_stmts[0], b.definers[0]) {
                    vec![b.definer_stmts[0]]
                } else {
                    Vec::new()
                };
            out.push(Candidate {
                expr: b.definers[0],
                reads: b.reads.clone(),
                removable,
            });
        }
        out
    }
}

/// Strings longer than this (in emitted bytes) are weighed by the cost model; anything shorter
/// always folds. Short strings cost almost nothing to duplicate and keep simple concats (`"v" .
/// X`) and the occasional string-valued branch foldable.
const SHORT_STRING: usize = 24;

/// Whether substituting `value` into every site in `reads` is worth the bytes it adds.
///
/// Folding rewrites each read's variable name to the rendered literal, and when `removable` also
/// deletes the declaration. For numbers that is always a win (a literal is a handful of bytes and
/// it unlocks branch shaking). A *string* literal, though, is copied verbatim into every read
/// site, so a long one read in several places can grow the bundle for no benefit.
///
/// Fold a long string if, with a rendered length `lit` and read names totalling `names` bytes,
/// `reads*lit - names` bytes and removal of the declaration recovers ~ `lit` (that is, if it
/// would actually shrink the size of the bundle).
fn worth_substituting(
    program: &Program,
    value: &ConstValue,
    reads: &[NodeId],
    removable: bool,
) -> bool {
    let ConstValue::Str(s) = value else {
        return true;
    };
    let lit = emitted_str_len(s);
    if lit <= SHORT_STRING {
        return true;
    }
    let names: usize = reads.iter().map(|&r| program.text(r).trim().len()).sum();
    let added = reads.len().saturating_mul(lit).saturating_sub(names);
    let saved = if removable { lit } else { 0 };
    added <= saved
}

/// Emitted byte length of a string constant, mirroring emit's `render_const`: the content wrapped
/// in quotes, with each interior `"` re-escaped to the two-byte `` `" ``.
fn emitted_str_len(s: &str) -> usize {
    s.len() + s.matches('"').count() + 2
}

/// `Some(true)` for the plain definer `:=`; `Some(false)` for a compound assignment; `None` for
/// a non-assignment operator.
fn assign_kind(op: &str) -> Option<bool> {
    match op {
        ":=" => Some(true),
        "+=" | "-=" | "*=" | "/=" | "//=" | ".=" | "|=" | "&=" | "^=" | ">>=" | "<<=" | ">>>="
        | "??=" => Some(false),
        _ => None,
    }
}

/// The contents of a string literal node, lowercased, or `None` if it is not a string literal.
fn string_literal(program: &Program, id: NodeId) -> Option<String> {
    if !matches!(
        program.arena[id].kind,
        NodeKind::Literal {
            kind: LiteralKind::String
        }
    ) {
        return None;
    }
    let t = program.text(id).trim();
    let bytes = t.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        Some(t[1..t.len() - 1].to_ascii_lowercase())
    } else {
        None
    }
}

/// Getter-only fat-arrow property constants and the structural facts that gate them.
struct Members<'a> {
    program: &'a Program,
    /// Lowercased class name -> its getter-only-arrow properties (member name -> getter body).
    classes: HashMap<String, HashMap<String, NodeId>>,
    /// Member names that must never fold (field/setter/method, assignment, or a `DefineProp`).
    blocked: HashSet<String>,
    /// A dynamic-named `DefineProp` is present: no property may fold.
    defineprop_dynamic: bool,
}

impl<'a> Members<'a> {
    fn collect(program: &'a Program) -> Self {
        let mut m = Members {
            program,
            classes: HashMap::new(),
            blocked: HashSet::new(),
            defineprop_dynamic: false,
        };
        m.scan();
        m
    }

    fn scan(&mut self) {
        for (_id, node) in self.program.arena.iter() {
            match &node.kind {
                NodeKind::ClassDecl(t) | NodeKind::StructDecl(t) => {
                    let Some(cn) = t
                        .name
                        .map(|s| self.program.span_text(s).trim().to_ascii_lowercase())
                    else {
                        continue;
                    };
                    let mut props = HashMap::new();
                    for &p in &t.properties {
                        let NodeKind::Property(prop) = &self.program.arena[p].kind else {
                            continue;
                        };
                        let Some(mn) = prop
                            .name
                            .map(|s| self.program.span_text(s).trim().to_ascii_lowercase())
                        else {
                            continue;
                        };
                        let getter_const =
                            prop.is_getter_only && prop.is_arrow_getter && prop.setter.is_none();
                        if let (true, Some(body)) =
                            (getter_const, getter_body(self.program, prop.getter))
                        {
                            props.insert(mn, body);
                        } else {
                            // A non-const property of this name: never foldable.
                            self.blocked.insert(mn);
                        }
                    }
                    // Fields named like a member also block folding it.
                    for &f in t.static_fields.iter().chain(&t.instance_fields) {
                        if let NodeKind::Field(field) = &self.program.arena[f].kind {
                            if let Some(s) = field.name {
                                self.blocked
                                    .insert(self.program.span_text(s).trim().to_ascii_lowercase());
                            }
                        }
                    }
                    self.classes.entry(cn).or_default().extend(props);
                }
                NodeKind::BinaryExpr { left, op, .. }
                    if assign_kind(self.program.span_text(*op).trim()).is_some() =>
                {
                    // `obj.M := …` (static or dynamic) blocks member `M`.
                    if let NodeKind::MemberAccess { member, .. } = &self.program.arena[*left].kind {
                        self.blocked
                            .insert(self.program.text(*member).trim().to_ascii_lowercase());
                    }
                }
                NodeKind::CallExpr(c) => self.scan_defineprop(c),
                _ => {}
            }
        }
    }

    /// Note the member name a `DefineProp` call targets (or that it is un-pinnable).
    fn scan_defineprop(&mut self, c: &ahkbuild_ir::node::CallExpr) {
        let NodeKind::MemberAccess {
            member, is_dynamic, ..
        } = &self.program.arena[c.callee].kind
        else {
            return;
        };
        if *is_dynamic
            || !self
                .program
                .text(*member)
                .trim()
                .eq_ignore_ascii_case("defineprop")
        {
            return;
        }
        match c.args.first() {
            Some(&arg) => match string_literal(self.program, arg) {
                Some(name) => {
                    self.blocked.insert(name);
                }
                None => self.defineprop_dynamic = true,
            },
            None => self.defineprop_dynamic = true,
        }
    }

    /// Build a candidate per getter body, routing each `ClassName.M` access to its value.
    ///
    /// Only the **class-name** form folds: the object must be an identifier naming the exact class
    /// that defines the getter-const. A bare `obj.M` cannot be folded - without the object's static
    /// type, we can't prove that `M` is the m we're looking for.
    fn candidates(&self) -> Vec<Candidate> {
        if self.defineprop_dynamic {
            return Vec::new();
        }
        let mut by_body: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for (id, node) in self.program.arena.iter() {
            let NodeKind::MemberAccess {
                object,
                member,
                is_dynamic,
            } = &node.kind
            else {
                continue;
            };
            if *is_dynamic {
                continue;
            }
            let NodeKind::Identifier = &self.program.arena[*object].kind else {
                continue;
            };
            let mname = self.program.text(*member).trim().to_ascii_lowercase();
            if self.blocked.contains(&mname) {
                continue;
            }
            let cn = self.program.text(*object).trim().to_ascii_lowercase();
            if let Some(&body) = self.classes.get(&cn).and_then(|p| p.get(&mname)) {
                by_body.entry(body).or_default().push(id);
            }
        }
        by_body
            .into_iter()
            // Getter-only properties shake out via shake's member pruning (folded accesses stop
            // counting as references), not via statement deletion - so no `removable` here.
            .map(|(expr, reads)| Candidate {
                expr,
                reads,
                removable: Vec::new(),
            })
            .collect()
    }
}

/// The expression a getter node evaluates: a `=> expr` arrow's body.
fn getter_body(program: &Program, getter: Option<NodeId>) -> Option<NodeId> {
    let g = getter?;
    match &program.arena[g].kind {
        NodeKind::FatArrow { body, .. } => Some(*body),
        NodeKind::Function(f) if f.is_arrow => f.body,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::{fold, ConstValue, Constants};
    use ahkbuild_ir::{NodeKind, Program};

    fn program(src: &str) -> Program {
        let tree = ahkbuild_syntax::parse(src).expect("tree");
        assert!(
            !tree.root_node().has_error(),
            "parse error: {}",
            tree.root_node().to_sexp()
        );
        ahkbuild_ir::lower(&tree, src)
    }

    /// Whether *some* `Identifier` read of `name` folded to a literal.
    fn ident_folds(src: &str, name: &str) -> bool {
        let p = program(src);
        let r = fold(&p, &Constants::default());
        let folds = p.arena.iter().any(|(id, n)| {
            matches!(n.kind, NodeKind::Identifier)
                && p.text(id).trim().eq_ignore_ascii_case(name)
                && r.literals.contains_key(&id)
        });
        folds
    }

    /// The value a `MemberAccess` (e.g. `Consts.Value`) folded to, if any.
    fn member_value(src: &str) -> Option<ConstValue> {
        let p = program(src);
        let r = fold(&p, &Constants::default());
        let value = p
            .arena
            .iter()
            .find(|(id, n)| {
                matches!(n.kind, NodeKind::MemberAccess { .. }) && r.literals.contains_key(id)
            })
            .and_then(|(id, _)| r.literals.get(&id).cloned());
        value
    }

    /// The trimmed text of every declaration statement `fold` marked dead (a fully-folded
    /// user constant safe to delete).
    fn dead_const_texts(src: &str) -> Vec<String> {
        let p = program(src);
        let r = fold(&p, &Constants::default());
        let mut texts: Vec<String> = r
            .dead_consts
            .iter()
            .map(|&id| p.text(id).trim().to_string())
            .collect();
        texts.sort();
        texts
    }

    #[test]
    fn function_static_const_folds_at_read_sites() {
        assert!(ident_folds(
            "f() {\n  static FLAG := 0x1234\n  return FLAG\n}\n",
            "FLAG"
        ));
    }

    #[test]
    fn single_assignment_local_folds() {
        assert!(ident_folds("f() {\n  x := 42\n  return x\n}\n", "x"));
    }

    #[test]
    fn second_assignment_disqualifies() {
        assert!(!ident_folds(
            "f() {\n  x := 1\n  x := 2\n  return x\n}\n",
            "x"
        ));
    }

    #[test]
    fn compound_assignment_disqualifies() {
        assert!(!ident_folds(
            "f() {\n  x := 1\n  x += 1\n  return x\n}\n",
            "x"
        ));
    }

    #[test]
    fn increment_disqualifies() {
        assert!(!ident_folds("f() {\n  x := 1\n  x++\n  return x\n}\n", "x"));
    }

    #[test]
    fn by_ref_disqualifies() {
        assert!(!ident_folds(
            "f() {\n  x := 1\n  Mutate(&x)\n  return x\n}\n",
            "x"
        ));
    }

    #[test]
    fn nested_function_reassignment_disqualifies() {
        // `inner` captures and rewrites `x`, so the outer read must not fold.
        let src = "outer() {\n  x := 1\n  inner() {\n    x := 2\n  }\n  inner()\n  return x\n}\n";
        assert!(!ident_folds(src, "x"));
    }

    #[test]
    fn nested_rebind_still_folds_outer() {
        // `inner` declares its own `x`, so the outer `x` is genuinely single-assignment.
        let src =
            "outer() {\n  x := 7\n  inner() {\n    local x := 2\n    return x\n  }\n  return x + inner()\n}\n";
        assert!(ident_folds(src, "x"));
    }

    #[test]
    fn unresolvable_dynamic_write_bails_scope() {
        // `%name% := …` could write `FLAG`, so nothing in the function folds.
        let src = "f(name) {\n  FLAG := 1\n  %name% := 9\n  return FLAG\n}\n";
        assert!(!ident_folds(src, "FLAG"));
    }

    #[test]
    fn constant_dynamic_write_pins_only_its_target() {
        // `%"OTHER"% := …` is an ordinary write to `OTHER`; `FLAG` is untouched and folds.
        let src = "f() {\n  FLAG := 1\n  OTHER := 0\n  %\"OTHER\"% := 9\n  return FLAG\n}\n";
        assert!(ident_folds(src, "FLAG"));
        assert!(!ident_folds(src, "OTHER"));
    }

    #[test]
    fn chained_constants_resolve_via_fixpoint() {
        // `B` reads `A`, which only folds after `A` resolves; `return B` then folds to 42.
        let src = "f() {\n  A := 41\n  B := A + 1\n  return B\n}\n";
        assert!(ident_folds(src, "B"));
        let p = program(src);
        let r = fold(&p, &Constants::default());
        assert!(r.literals.values().any(|v| *v == ConstValue::Int(42)));
    }

    #[test]
    fn getter_only_property_folds_class_access() {
        let src = "class Consts {\n  static Value => 42\n}\nMsgBox(Consts.Value)\n";
        assert_eq!(member_value(src), Some(ConstValue::Int(42)));
    }

    #[test]
    fn bare_object_access_does_not_fold() {
        // `obj.Value` can't be folded: `obj`'s type is unknown, so `Value` might be another
        // class's member. Only `ClassName.Value` (the defining class) folds.
        let src = "class C {\n  static Value => 42\n}\nf(obj) {\n  return obj.Value\n}\n";
        assert_eq!(member_value(src), None);
    }

    #[test]
    fn member_name_colliding_with_nested_class_is_safe() {
        // `Type.Nil => 192` is a getter, but `Box.Nil` is a nested class - folding `Box.Nil` to
        // 192 (breaking `Box.Nil()`) must not happen. Only `Type.Nil` folds.
        let src = "class Type {\n  static Nil => 192\n}\nclass Box {\n  class Nil {\n  }\n}\nMsgBox(Type.Nil)\nx := Box.Nil()\n";
        let p = program(src);
        let r = fold(&p, &Constants::default());
        let folded: Vec<_> = p
            .arena
            .iter()
            .filter(|(id, n)| {
                matches!(n.kind, NodeKind::MemberAccess { .. }) && r.literals.contains_key(id)
            })
            .map(|(id, _)| p.text(id).trim().to_string())
            .collect();
        assert_eq!(
            folded,
            vec!["Type.Nil"],
            "only the class-name getter access folds"
        );
    }

    #[test]
    fn defineprop_blocks_property_folding() {
        let src = "class Consts {\n  static Value => 42\n}\nConsts.DefineProp(\"Value\", { get: (*) => 24 })\nMsgBox(Consts.Value)\n";
        assert_eq!(member_value(src), None);
    }

    #[test]
    fn directive_folds_despite_reassignment() {
        let src = ";@ahkbuild-const\nMAX := 64\nMAX := 128\nMsgBox(MAX)\n";
        assert!(ident_folds(src, "MAX"));
    }

    #[test]
    fn branch_resolves_on_user_constant() {
        let src = "f() {\n  static DEBUG := 0\n  if DEBUG {\n    Log()\n  }\n}\n";
        let p = program(src);
        let r = fold(&p, &Constants::default());
        let branch = p
            .arena
            .iter()
            .find(|(_, n)| matches!(n.kind, NodeKind::IfStmt { .. }))
            .and_then(|(id, _)| r.branches.get(&id).copied());
        assert_eq!(branch, Some(crate::Branch::Dead));
    }

    #[test]
    fn fully_folded_local_decl_is_removable() {
        assert_eq!(
            dead_const_texts("f() {\n  x := 42\n  return x\n}\n"),
            vec!["x := 42"]
        );
    }

    #[test]
    fn fully_folded_static_decl_is_removable() {
        assert_eq!(
            dead_const_texts("f() {\n  static FLAG := 0x1234\n  return FLAG\n}\n"),
            vec!["static FLAG := 0x1234"]
        );
    }

    #[test]
    fn dynamic_read_blocks_removal_but_not_folding() {
        // `%name%` could read FLAG at runtime, so the declaration must stay even though the static
        // `return FLAG` still folds.
        let src = "f(name) {\n  FLAG := 1\n  y := %name%\n  return FLAG\n}\n";
        assert!(ident_folds(src, "FLAG"));
        assert!(dead_const_texts(src).is_empty());
    }

    #[test]
    fn reassigned_local_is_not_removable() {
        // Two definers: never folded, never removed.
        assert!(dead_const_texts("f() {\n  x := 1\n  x := 2\n  return x\n}\n").is_empty());
    }

    #[test]
    fn exported_const_folds_but_is_not_removable() {
        // The read folds, but an exported constant is public API the bundler can't see all uses of.
        let src = "export FOO := 5\nMsgBox(FOO)\n";
        assert!(ident_folds(src, "FOO"));
        assert!(dead_const_texts(src).is_empty());
    }

    #[test]
    fn directive_const_removable_despite_reassignment() {
        // The author trusts the directive; every `:=` write is deletable.
        let src = ";@ahkbuild-const\nMAX := 64\nMAX := 128\nMsgBox(MAX)\n";
        assert_eq!(dead_const_texts(src), vec!["MAX := 128", "MAX := 64"]);
    }

    #[test]
    fn exported_directive_const_is_not_removable() {
        // The structural export guard still applies even under a trust directive.
        let src = ";@ahkbuild-const\nexport MAX := 64\nMsgBox(MAX)\n";
        assert!(dead_const_texts(src).is_empty());
    }

    // ~37 emitted bytes - safely past `SHORT_STRING`, so the cost model applies.
    const LONG: &str = "\"this is a fairly long constant string\"";

    #[test]
    fn long_string_read_once_and_removable_folds() {
        // One read plus decl deletion is a wash-or-win, so a long string still folds here.
        let src = format!("f() {{\n  msg := {LONG}\n  return msg\n}}\n");
        assert!(ident_folds(&src, "msg"));
        assert_eq!(dead_const_texts(&src), vec![format!("msg := {LONG}")]);
    }

    #[test]
    fn long_string_read_many_times_does_not_fold() {
        // Three copies of a long string would dwarf what deleting the one declaration saves, so
        // the constant is left in place (and therefore not removed).
        let src = format!("f() {{\n  msg := {LONG}\n  A(msg)\n  B(msg)\n  return msg\n}}\n");
        assert!(!ident_folds(&src, "msg"));
        assert!(dead_const_texts(&src).is_empty());
    }

    #[test]
    fn long_string_single_read_but_not_removable_does_not_fold() {
        // The decl can't be deleted (exported), so inlining the long string is pure growth.
        let src = format!("export MSG := {LONG}\nShow(MSG)\n");
        assert!(!ident_folds(&src, "MSG"));
    }

    #[test]
    fn short_string_folds_even_with_many_reads() {
        // Below `SHORT_STRING`, duplication is negligible and folding stays on.
        let src = "f() {\n  s := \"lib\"\n  A(s)\n  B(s)\n  return s\n}\n";
        assert!(ident_folds(src, "s"));
    }

    #[test]
    fn nested_assignment_is_not_removed_wholesale() {
        // `x` folds, but its definer sits inside `y := (x := 5)`; deleting that statement would
        // also drop the assignment to `y`, so it must not be removable.
        let src = "f() {\n  y := (x := 5)\n  return x + y\n}\n";
        assert!(dead_const_texts(src).is_empty());
    }
}
