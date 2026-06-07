//! A deterministic pretty-printer for the IR tree, used by snapshot tests and the CLI
//! `--ir` flag.
//!
//! This is the canonical *exhaustive* walker: the `match` on [`NodeKind`] below has no
//! catch-all arm, so adding a variant fails to compile here until it's handled — exactly
//! the guarantee the Rust rewrite exists to get.

use std::fmt::Write;

use ahkbuild_syntax::Span;

use crate::arena::NodeId;
use crate::node::*;
use crate::program::Program;

/// Render `program` as an indented IR tree.
pub fn print_program(program: &Program) -> String {
    let mut p = Printer {
        program,
        out: String::new(),
    };
    // Groups are printed transparently (no header) so single-group output is unchanged.
    for group in &program.groups {
        for &module in &group.modules {
            p.emit("", module, 0);
        }
    }
    p.out
}

struct Printer<'a> {
    program: &'a Program,
    out: String,
}

impl Printer<'_> {
    fn emit(&mut self, label: &str, id: NodeId, depth: usize) {
        let node = &self.program.arena[id];
        let indent = "  ".repeat(depth);
        let label = if label.is_empty() {
            String::new()
        } else {
            format!("{label}: ")
        };
        let span = node.span;
        let head = format!("{indent}{label}{}", self.kind_summary(id));
        let _ = writeln!(self.out, "{head} [{}, {}]", span.start, span.end);
        self.emit_children(&node.kind, depth + 1);
    }

    /// A one-line label for the node: its variant name plus a small distinguishing detail
    /// (name, operator, snippet) but never its full multi-line text.
    fn kind_summary(&self, id: NodeId) -> String {
        let node = &self.program.arena[id];
        let prog = self.program;
        let snippet = |span: Span| -> String {
            let t = prog.span_text(span);
            let one = t.replace(['\n', '\r', '\t'], " ");
            let trimmed = one.trim();
            if trimmed.len() > 48 {
                format!("{}…", &trimmed[..48])
            } else if t.contains('\n') {
                format!("{trimmed}…")
            } else {
                trimmed.to_string()
            }
        };
        let name_of = |s: &Option<Span>| -> String {
            s.map(|sp| prog.span_text(sp).to_string()).unwrap_or_default()
        };

        match &node.kind {
            NodeKind::Block { body } => format!("Block ({} stmts)", body.len()),
            NodeKind::ExpressionStatement { .. } => "ExpressionStatement".into(),
            NodeKind::Opaque => format!("Opaque \"{}\"", snippet(node.span)),
            NodeKind::Module(m) => format!(
                "Module \"{}\"{}",
                m.name,
                if m.is_main() { " (implicit)" } else { "" }
            ),
            NodeKind::ImportDirective(d) => format!("ImportDirective {}", self.import_summary(d)),
            NodeKind::ExportDecl { default, .. } => {
                format!("ExportDecl{}", if *default { " default" } else { "" })
            }
            NodeKind::Function(f) => {
                let mut s = String::from("Function");
                if f.is_expression {
                    s.push_str(" (expr)");
                }
                let n = name_of(&f.name);
                if !n.is_empty() {
                    let _ = write!(s, " \"{n}\"");
                }
                if f.is_static {
                    s.push_str(" static");
                }
                if f.is_arrow {
                    s.push_str(" =>");
                }
                if f.is_variadic {
                    s.push_str(" variadic");
                }
                s
            }
            NodeKind::ClassDecl(t) => {
                format!("ClassDecl \"{}\"{}", name_of(&t.name), self.extends(t))
            }
            NodeKind::StructDecl(t) => {
                format!("StructDecl \"{}\"{}", name_of(&t.name), self.extends(t))
            }
            NodeKind::Property(p) => format!(
                "Property \"{}\"{}",
                name_of(&p.name),
                if p.is_static { " static" } else { "" }
            ),
            NodeKind::Field(f) => format!(
                "Field \"{}\"{}",
                name_of(&f.name),
                if f.is_static { " static" } else { "" }
            ),
            NodeKind::TypedProperty(t) => format!("TypedProperty \"{}\"", name_of(&t.name)),
            NodeKind::TypeSpecifier { .. } => "TypeSpecifier".into(),
            NodeKind::VarDecl(v) => {
                format!("VarDecl {:?} \"{}\"", v.scope, name_of(&v.name))
            }
            NodeKind::BinaryExpr { op, .. } => {
                format!("BinaryExpr \"{}\"", prog.span_text(*op).trim())
            }
            NodeKind::UnaryExpr { op, prefix, .. } => format!(
                "UnaryExpr \"{}\" ({})",
                prog.span_text(*op).trim(),
                if *prefix { "prefix" } else { "postfix" }
            ),
            NodeKind::TernaryExpr { .. } => "TernaryExpr".into(),
            NodeKind::CallExpr(c) => {
                let mut s = String::from("CallExpr");
                if c.is_command_style {
                    s.push_str(" command");
                }
                if c.is_dynamic {
                    s.push_str(" dynamic");
                }
                s
            }
            NodeKind::Literal { kind } => format!("Literal {:?} \"{}\"", kind, snippet(node.span)),
            NodeKind::Identifier => format!("Identifier \"{}\"", snippet(node.span)),
            NodeKind::DynamicIdentifier { .. } => "DynamicIdentifier".into(),
            NodeKind::MemberAccess { is_dynamic, .. } => {
                format!("MemberAccess{}", if *is_dynamic { " dynamic" } else { "" })
            }
            NodeKind::IndexAccess { .. } => "IndexAccess".into(),
            NodeKind::ArrayLiteral { elements } => format!("ArrayLiteral ({})", elements.len()),
            NodeKind::ObjectLiteral { members } => format!("ObjectLiteral ({})", members.len()),
            NodeKind::DerefExpr { .. } => "DerefExpr".into(),
            NodeKind::VarRefExpr { .. } => "VarRefExpr".into(),
            NodeKind::FatArrow { .. } => "FatArrow".into(),
            NodeKind::IfStmt { .. } => "IfStmt".into(),
            NodeKind::WhileStmt { .. } => "WhileStmt".into(),
            NodeKind::ForStmt(_) => "ForStmt".into(),
            NodeKind::LoopStmt(l) => format!("LoopStmt {:?}", l.kind),
            NodeKind::SwitchStmt { .. } => "SwitchStmt".into(),
            NodeKind::CaseClause { is_default, .. } => {
                format!("CaseClause{}", if *is_default { " default" } else { "" })
            }
            NodeKind::TryStmt(_) => "TryStmt".into(),
            NodeKind::CatchClause(c) => {
                let types: Vec<&str> = c.error_types.iter().map(|s| prog.span_text(*s)).collect();
                format!("CatchClause [{}]", types.join(", "))
            }
            NodeKind::ReturnStmt { .. } => "ReturnStmt".into(),
            NodeKind::BreakStmt { label } => format!("BreakStmt {}", name_of(label)),
            NodeKind::ContinueStmt { label } => format!("ContinueStmt {}", name_of(label)),
            NodeKind::ThrowStmt { .. } => "ThrowStmt".into(),
            NodeKind::GotoStmt { label } => format!("GotoStmt {}", name_of(label)),
            NodeKind::Hotkey { trigger, .. } => format!("Hotkey \"{}\"", name_of(trigger)),
            NodeKind::Hotstring { trigger, .. } => format!("Hotstring \"{}\"", name_of(trigger)),
            NodeKind::Directive { kind, .. } => format!("Directive #{kind}"),
            NodeKind::Label { name } => format!("Label \"{}\"", name_of(name)),
        }
    }

    fn extends(&self, t: &TypeDecl) -> String {
        match &t.superclass {
            Some(s) => format!(" extends {}", self.program.span_text(*s)),
            None => String::new(),
        }
    }

    fn import_summary(&self, d: &ImportDirective) -> String {
        let prog = self.program;
        let source = match &d.source {
            ImportSource::Name(s) => prog.span_text(*s).to_string(),
            ImportSource::Path(s) => prog.span_text(*s).to_string(),
        };
        let binding = match &d.binding {
            ImportBinding::Whole => String::new(),
            ImportBinding::Alias(a) => format!(" as {}", prog.span_text(*a)),
            ImportBinding::Selective { wildcard, names } => {
                let mut parts: Vec<String> = Vec::new();
                if *wildcard {
                    parts.push("*".into());
                }
                for n in names {
                    match &n.alias {
                        Some(a) => {
                            parts.push(format!("{} as {}", prog.span_text(n.name), prog.span_text(*a)))
                        }
                        None => parts.push(prog.span_text(n.name).to_string()),
                    }
                }
                format!(" {{{}}}", parts.join(", "))
            }
        };
        format!("{source}{binding}")
    }

    /// Recurse into every child node. Exhaustive `match` — no catch-all.
    fn emit_children(&mut self, kind: &NodeKind, depth: usize) {
        match kind {
            NodeKind::Block { body } => self.list("", body, depth),
            NodeKind::ExpressionStatement { expr } => self.emit("expr", *expr, depth),
            NodeKind::Opaque => {}
            NodeKind::Module(m) => self.list("", &m.body, depth),
            NodeKind::ImportDirective(_) => {}
            NodeKind::ExportDecl { decl, .. } => self.emit("decl", *decl, depth),
            NodeKind::Function(f) => {
                self.params(&f.params, depth);
                if let Some(b) = f.body {
                    self.emit("body", b, depth);
                }
            }
            NodeKind::ClassDecl(t) | NodeKind::StructDecl(t) => {
                self.list("static field", &t.static_fields, depth);
                self.list("field", &t.instance_fields, depth);
                self.list("typed field", &t.typed_fields, depth);
                self.list("prop", &t.properties, depth);
                self.list("method", &t.methods, depth);
                self.list("nested", &t.nested, depth);
            }
            NodeKind::Property(p) => {
                if let Some(g) = p.getter {
                    self.emit("get", g, depth);
                }
                if let Some(s) = p.setter {
                    self.emit("set", s, depth);
                }
            }
            NodeKind::Field(f) => {
                if let Some(i) = f.initializer {
                    self.emit("init", i, depth);
                }
            }
            NodeKind::TypedProperty(t) => {
                self.emit("type", t.type_spec, depth);
                if let Some(i) = t.initializer {
                    self.emit("init", i, depth);
                }
            }
            NodeKind::TypeSpecifier { type_expr } => self.emit("type", *type_expr, depth),
            NodeKind::VarDecl(v) => {
                if let Some(i) = v.initializer {
                    self.emit("init", i, depth);
                }
            }
            NodeKind::BinaryExpr { left, right, .. } => {
                self.emit("left", *left, depth);
                self.emit("right", *right, depth);
            }
            NodeKind::UnaryExpr { operand, .. } => self.emit("operand", *operand, depth),
            NodeKind::TernaryExpr {
                condition,
                then_branch,
                else_branch,
            } => {
                self.emit("cond", *condition, depth);
                self.emit("then", *then_branch, depth);
                self.emit("else", *else_branch, depth);
            }
            NodeKind::CallExpr(c) => {
                self.emit("callee", c.callee, depth);
                self.list("arg", &c.args, depth);
            }
            NodeKind::Literal { .. } => {}
            NodeKind::Identifier => {}
            NodeKind::DynamicIdentifier { parts } => self.list("part", parts, depth),
            NodeKind::MemberAccess { object, member, .. } => {
                self.emit("object", *object, depth);
                self.emit("member", *member, depth);
            }
            NodeKind::IndexAccess { object, args } => {
                self.emit("object", *object, depth);
                self.list("arg", args, depth);
            }
            NodeKind::ArrayLiteral { elements } => self.list("", elements, depth),
            NodeKind::ObjectLiteral { members } => {
                for m in members {
                    self.emit("key", m.key, depth);
                    if let Some(v) = m.value {
                        self.emit("value", v, depth);
                    }
                }
            }
            NodeKind::DerefExpr { inner } => self.emit("inner", *inner, depth),
            NodeKind::VarRefExpr { operand } => self.emit("operand", *operand, depth),
            NodeKind::FatArrow { params, body } => {
                self.params(params, depth);
                self.emit("body", *body, depth);
            }
            NodeKind::IfStmt {
                condition,
                then_body,
                else_body,
            } => {
                self.emit("cond", *condition, depth);
                self.emit("then", *then_body, depth);
                if let Some(e) = else_body {
                    self.emit("else", *e, depth);
                }
            }
            NodeKind::WhileStmt { condition, body } => {
                self.emit("cond", *condition, depth);
                self.emit("body", *body, depth);
            }
            NodeKind::ForStmt(f) => {
                self.list("iter", &f.iterators, depth);
                if let Some(i) = f.iterable {
                    self.emit("in", i, depth);
                }
                if let Some(b) = f.body {
                    self.emit("body", b, depth);
                }
                if let Some(e) = f.else_body {
                    self.emit("else", e, depth);
                }
            }
            NodeKind::LoopStmt(l) => {
                if let Some(h) = l.head {
                    self.emit("head", h, depth);
                }
                if let Some(b) = l.body {
                    self.emit("body", b, depth);
                }
                if let Some(u) = l.until {
                    self.emit("until", u, depth);
                }
            }
            NodeKind::SwitchStmt {
                discriminant,
                cases,
            } => {
                if let Some(d) = discriminant {
                    self.emit("on", *d, depth);
                }
                self.list("case", cases, depth);
            }
            NodeKind::CaseClause { values, body, .. } => {
                self.list("value", values, depth);
                self.list("", body, depth);
            }
            NodeKind::TryStmt(t) => {
                self.emit("try", t.try_body, depth);
                self.list("catch", &t.catches, depth);
                if let Some(e) = t.else_body {
                    self.emit("else", e, depth);
                }
                if let Some(f) = t.finally_body {
                    self.emit("finally", f, depth);
                }
            }
            NodeKind::CatchClause(c) => self.emit("body", c.body, depth),
            NodeKind::ReturnStmt { value } => {
                if let Some(v) = value {
                    self.emit("value", *v, depth);
                }
            }
            NodeKind::BreakStmt { .. }
            | NodeKind::ContinueStmt { .. }
            | NodeKind::GotoStmt { .. } => {}
            NodeKind::ThrowStmt { value } => {
                if let Some(v) = value {
                    self.emit("value", *v, depth);
                }
            }
            NodeKind::Hotkey { body, .. } => {
                if let Some(b) = body {
                    self.emit("body", *b, depth);
                }
            }
            NodeKind::Hotstring { replacement, .. } => {
                if let Some(r) = replacement {
                    self.emit("replacement", *r, depth);
                }
            }
            NodeKind::Directive { expression, .. } => {
                if let Some(e) = expression {
                    self.emit("expr", *e, depth);
                }
            }
            NodeKind::Label { .. } => {}
        }
    }

    fn list(&mut self, label: &str, ids: &[NodeId], depth: usize) {
        for &id in ids {
            self.emit(label, id, depth);
        }
    }

    fn params(&mut self, params: &[Param], depth: usize) {
        for p in params {
            let name = p
                .name
                .map(|s| self.program.span_text(s).to_string())
                .unwrap_or_default();
            let mut flags = String::new();
            if p.by_ref {
                flags.push_str(" &");
            }
            if p.variadic {
                flags.push('*');
            }
            if p.optional {
                flags.push('?');
            }
            let indent = "  ".repeat(depth);
            let _ = writeln!(self.out, "{indent}param: \"{name}\"{flags}");
            if let Some(d) = p.default {
                self.emit("default", d, depth + 1);
            }
        }
    }
}
