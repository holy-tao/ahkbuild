//! Build-time constant folding and branch resolution.
//!
//! Given a linked [`Program`] and the set of [`Constants`] known at build time (today
//! `A_IsCompiled` and `A_PtrSize`), [`fold`] computes, without mutating the IR:
//!
//! 1. **Literal substitution** ([`FoldResult::literals`]) - every built-in identifier node
//!    whose value is known, so the emitter can rewrite its text (`A_IsCompiled` -> `0`/`1`).
//! 2. **Branch resolution** ([`FoldResult::branches`]) - every `if` / ternary whose condition
//!    evaluates to a build-time constant, recorded as which arm survives. Tree-shaking uses
//!    this to skip the dead arm during reachability; the emitter deletes it.
//!
//! The evaluator is total and conservative: any expression it cannot prove constant yields
//! `None` and is left untouched, so over-keeping (a branch left intact) is always safe.
//! Logical operators short-circuit (`false && Foo()` folds to `false` without evaluating the
//! call), which is exactly what makes discarding a folded condition's subtree sound - every
//! non-constant part was guaranteed never to execute.

use std::collections::HashMap;

use ahkbuild_ir::node::LiteralKind;
use ahkbuild_ir::{children, NodeId, NodeKind, Program};

/// Build-time constant inputs. Each is `None` when the value is not known for the current
/// target (e.g. `ptr_size` for a `.ahk` bundle with no bitness-pinned `#Requires`), in which
/// case the corresponding built-in is left as-is. Extensible: add fields as more build-time
/// constants become foldable.
#[derive(Clone, Copy, Debug, Default)]
pub struct Constants {
    /// Value of `A_IsCompiled` (false for `.ahk`, true for `.exe`).
    pub is_compiled: Option<bool>,
    /// Value of `A_PtrSize` in bytes - 4 (32-bit) or 8 (64-bit).
    pub ptr_size: Option<u8>,
}

/// A folded constant value.
///
/// There is no boolean variant: in AHK `true`/`false` are simply the integers `1`/`0` and
/// arithmetic applies to them (`true + 1 == 2`), so booleans fold to [`ConstValue::Int`].
#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    Int(i64),
    Float(f64),
    Str(String),
}

/// Which arm of a constant-conditioned branch survives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Branch {
    /// The condition is truthy: keep the `then` arm (for ternaries, the `?` operand).
    Then,
    /// The condition is falsey and an `else` arm exists: keep it (ternary `:` operand).
    Else,
    /// The condition is falsey and there is no `else` arm: the whole statement is dead.
    Dead,
}

/// What [`fold`] found: constant (sub)expressions to substitute and branches to resolve.
#[derive(Debug, Default)]
pub struct FoldResult {
    /// **Maximal** constant (sub)expressions that contain a known build-time built-in, mapped to
    /// their value - the emitter rewrites each one's span to the rendered literal. "Maximal"
    /// means the largest such expression: `A_PtrSize * 8` is recorded as `64`, not the inner
    /// `A_PtrSize` as `8`. Pure author constants (`2 + 2`, `true`) are *not* recorded - only
    /// expressions that fold *because of* a build-time constant.
    pub literals: HashMap<NodeId, ConstValue>,
    /// `IfStmt` / `TernaryExpr` nodes whose condition is build-time constant.
    pub branches: HashMap<NodeId, Branch>,
}

impl FoldResult {
    /// Whether nothing was folded.
    pub fn is_empty(&self) -> bool {
        self.literals.is_empty() && self.branches.is_empty()
    }
}

/// Fold build-time constants across `program` under the known `consts`.
pub fn fold(program: &Program, consts: &Constants) -> FoldResult {
    let ev = Evaluator { program, consts };
    let mut result = FoldResult::default();

    // (A) Maximal constant substitution: walk each module top-down, recording the largest
    // expressions that fold thanks to a build-time built-in (so `A_PtrSize * 8` is `64`, not
    // `8 * 8`) and skipping their now-subsumed children.
    for m in program.modules() {
        if let NodeKind::Module(module) = &program.arena[m].kind {
            for &stmt in &module.body {
                ev.collect_substitutions(stmt, &mut result.literals);
            }
        }
    }

    // (C) Branch resolution: any `if`/ternary whose condition folds to a build-time constant.
    for (id, node) in program.arena.iter() {
        match &node.kind {
            NodeKind::IfStmt {
                condition,
                else_body,
                ..
            } => {
                if let Some(v) = ev.eval(*condition) {
                    let branch = if truthy(&v) {
                        Branch::Then
                    } else if else_body.is_some() {
                        Branch::Else
                    } else {
                        Branch::Dead
                    };
                    result.branches.insert(id, branch);
                }
            }
            NodeKind::TernaryExpr { condition, .. } => {
                if let Some(v) = ev.eval(*condition) {
                    let branch = if truthy(&v) {
                        Branch::Then
                    } else {
                        Branch::Else
                    };
                    result.branches.insert(id, branch);
                }
            }
            _ => {}
        }
    }

    result
}

/// Derive `A_PtrSize` (in bytes) from a bitness-pinned `#Requires` in the entry group, e.g.
/// `#Requires AutoHotkey v2.1-alpha.22 32-bit` -> `Some(4)`. Returns `None` when no entry
/// `#Requires` pins the bitness. The CLI uses this only when no explicit `--bitness` was given.
pub fn ptr_size_from_requires(program: &Program) -> Option<u8> {
    let entry = program.groups.first()?;
    for &module in &entry.modules {
        let NodeKind::Module(m) = &program.arena[module].kind else {
            continue;
        };
        for &stmt in &m.body {
            if let NodeKind::Directive { kind, .. } = &program.arena[stmt].kind {
                if kind.eq_ignore_ascii_case("requires") {
                    let text = program.text(stmt).to_ascii_lowercase();
                    if text.contains("32-bit") {
                        return Some(4);
                    }
                    if text.contains("64-bit") {
                        return Some(8);
                    }
                }
            }
        }
    }
    None
}

/// AHK truthiness: `0`, `0.0`, the empty string, and any string that is numerically zero are
/// false; everything else is true.
fn truthy(v: &ConstValue) -> bool {
    match v {
        ConstValue::Int(i) => *i != 0,
        ConstValue::Float(f) => *f != 0.0,
        ConstValue::Str(s) => match parse_number(s) {
            Some(n) => n != 0.0,
            None => !s.trim().is_empty(),
        },
    }
}

/// The recursive constant evaluator over one program's IR.
struct Evaluator<'a> {
    program: &'a Program,
    consts: &'a Constants,
}

impl Evaluator<'_> {
    /// The value of a substitutable built-in identifier (`A_IsCompiled`, `A_PtrSize`), or
    /// `None` if `id` is not one or its value is unknown for the current target. The language
    /// constants `true`/`false` are *not* returned here - they already read as literals, so
    /// there is nothing to rewrite.
    fn substitutable_builtin(&self, id: NodeId) -> Option<ConstValue> {
        let name = self.program.text(id).trim();
        if name.eq_ignore_ascii_case("A_IsCompiled") {
            return self.consts.is_compiled.map(|b| ConstValue::Int(b as i64));
        }
        if name.eq_ignore_ascii_case("A_PtrSize") {
            return self.consts.ptr_size.map(|n| ConstValue::Int(n as i64));
        }
        None
    }

    /// Record the maximal constant substitutions under `id`: the largest subexpressions that
    /// contain a known built-in and evaluate to a renderable constant. Recurses top-down,
    /// stopping at each recorded node (its children are subsumed). `if`/ternary nodes are never
    /// recorded - branch resolution handles those - but are descended so substitutions inside
    /// their arms are still found. A `Float` result is left unrecorded (we keep descending so
    /// the built-ins inside still get leaf-substituted) to avoid number-formatting drift.
    fn collect_substitutions(&self, id: NodeId, out: &mut HashMap<NodeId, ConstValue>) {
        let is_branch = matches!(
            self.program.arena[id].kind,
            NodeKind::IfStmt { .. } | NodeKind::TernaryExpr { .. }
        );
        if !is_branch {
            if let Some(v) = self.eval(id) {
                if !matches!(v, ConstValue::Float(_)) {
                    out.insert(id, v);
                    return;
                }
            }
        }
        // For static member access the property name is not a value expression - only recurse
        // into the object. Dynamic access (%expr%) does evaluate the member, so descend both.
        if let NodeKind::MemberAccess {
            object,
            member,
            is_dynamic,
        } = &self.program.arena[id].kind
        {
            self.collect_substitutions(*object, out);
            if *is_dynamic {
                self.collect_substitutions(*member, out);
            }
            return;
        }
        for c in children(&self.program.arena[id].kind) {
            self.collect_substitutions(c, out);
        }
    }

    /// Resolve an identifier to a constant value: the language constants `true`/`false`, plus
    /// any substitutable built-in known for the target.
    fn ident_value(&self, id: NodeId) -> Option<ConstValue> {
        let name = self.program.text(id).trim();
        if name.eq_ignore_ascii_case("true") {
            return Some(ConstValue::Int(1));
        }
        if name.eq_ignore_ascii_case("false") {
            return Some(ConstValue::Int(0));
        }
        self.substitutable_builtin(id)
    }

    /// Evaluate `id` to a constant value, or `None` if it is not build-time constant.
    fn eval(&self, id: NodeId) -> Option<ConstValue> {
        match &self.program.arena[id].kind {
            NodeKind::Literal { kind } => self.eval_literal(id, *kind),
            NodeKind::Identifier => self.ident_value(id),
            NodeKind::UnaryExpr {
                op,
                operand,
                prefix,
            } => {
                if !prefix {
                    return None; // post-fix `x++`/`x--` are assignments, not pure.
                }
                self.eval_unary(self.program.span_text(*op).trim(), *operand)
            }
            NodeKind::BinaryExpr { left, op, right } => {
                self.eval_binary(self.program.span_text(*op).trim(), *left, *right)
            }
            NodeKind::TernaryExpr {
                condition,
                then_branch,
                else_branch,
            } => {
                let c = self.eval(*condition)?;
                self.eval(if truthy(&c) {
                    *then_branch
                } else {
                    *else_branch
                })
            }
            _ => None,
        }
    }

    fn eval_literal(&self, id: NodeId, kind: LiteralKind) -> Option<ConstValue> {
        let text = self.program.text(id).trim();
        match kind {
            LiteralKind::Integer => parse_int(text).map(ConstValue::Int),
            LiteralKind::Float => text.parse::<f64>().ok().map(ConstValue::Float),
            LiteralKind::String => parse_string(text).map(ConstValue::Str),
            LiteralKind::Boolean => Some(ConstValue::Int(text.eq_ignore_ascii_case("true") as i64)),
        }
    }

    fn eval_unary(&self, op: &str, operand: NodeId) -> Option<ConstValue> {
        match op {
            "!" | "not" => Some(ConstValue::Int(!truthy(&self.eval(operand)?) as i64)),
            "-" => match self.eval(operand)? {
                ConstValue::Int(i) => Some(ConstValue::Int(-i)),
                ConstValue::Float(f) => Some(ConstValue::Float(-f)),
                ConstValue::Str(_) => None,
            },
            "+" => match self.eval(operand)? {
                v @ (ConstValue::Int(_) | ConstValue::Float(_)) => Some(v),
                ConstValue::Str(_) => None,
            },
            "~" => match self.eval(operand)? {
                ConstValue::Int(i) => Some(ConstValue::Int(!i)),
                _ => None,
            },
            _ => None,
        }
    }

    fn eval_binary(&self, op: &str, left: NodeId, right: NodeId) -> Option<ConstValue> {
        // Logical operators short-circuit: only the left operand must be constant. AHK's `&&`
        // and `||` yield 1/0, not the operand value.
        match op.to_ascii_lowercase().as_str() {
            "&&" | "and" => {
                let l = self.eval(left)?;
                return Some(ConstValue::Int(if !truthy(&l) {
                    0
                } else {
                    truthy(&self.eval(right)?) as i64
                }));
            }
            "||" | "or" => {
                let l = self.eval(left)?;
                return Some(ConstValue::Int(if truthy(&l) {
                    1
                } else {
                    truthy(&self.eval(right)?) as i64
                }));
            }
            _ => {}
        }

        let l = self.eval(left)?;
        let r = self.eval(right)?;

        // String concatenation, explicit (`.`) or implicit (adjacency, lowered with an empty
        // operator span). A float operand bails to stay exact.
        if op == "." || op.is_empty() {
            return Some(ConstValue::Str(concat(&l, &r)?));
        }

        // Comparisons and arithmetic only fold over numbers - this side-steps AHK's
        // string/number coercion quirks, which never matter for build-config guards.
        let (a, b) = (as_number(&l)?, as_number(&r)?);
        let both_int = matches!((&l, &r), (ConstValue::Int(_), ConstValue::Int(_)));

        let num = |x: f64| {
            if both_int && x.fract() == 0.0 {
                ConstValue::Int(x as i64)
            } else {
                ConstValue::Float(x)
            }
        };
        let boolean = |b: bool| ConstValue::Int(b as i64);

        Some(match op {
            "+" => num(a + b),
            "-" => num(a - b),
            "*" => num(a * b),
            "/" => ConstValue::Float(a / b), // AHK `/` is always true (float) division.
            "//" if both_int => ConstValue::Int((a as i64).div_euclid(b as i64)),
            "**" => ConstValue::Float(a.powf(b)),
            "=" | "==" => boolean(a == b),
            "!=" | "<>" => boolean(a != b),
            "<" => boolean(a < b),
            ">" => boolean(a > b),
            "<=" => boolean(a <= b),
            ">=" => boolean(a >= b),
            _ => return None,
        })
    }
}

/// Concatenate two values into a string (AHK's `.`), or `None` if either is a float - whose
/// AHK string form we don't reproduce exactly, so folding it would risk drift.
fn concat(l: &ConstValue, r: &ConstValue) -> Option<String> {
    Some(format!("{}{}", concat_operand(l)?, concat_operand(r)?))
}

fn concat_operand(v: &ConstValue) -> Option<String> {
    match v {
        ConstValue::Int(i) => Some(i.to_string()),
        ConstValue::Str(s) => Some(s.clone()),
        ConstValue::Float(_) => None,
    }
}

/// Numeric view of a value: `Int`/`Float` directly, a string only if it parses as a number.
fn as_number(v: &ConstValue) -> Option<f64> {
    match v {
        ConstValue::Int(i) => Some(*i as f64),
        ConstValue::Float(f) => Some(*f),
        ConstValue::Str(s) => parse_number(s),
    }
}

/// Parse a numeric string (decimal or `0x` hex, integer or float).
fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if let Some(i) = parse_int(t) {
        return Some(i as f64);
    }
    t.parse::<f64>().ok()
}

/// Parse an integer literal: `0x`-prefixed hex or plain decimal (underscores allowed).
fn parse_int(t: &str) -> Option<i64> {
    let t = t.trim().replace('_', "");
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()
    } else if let Some(hex) = t.strip_prefix("-0x").or_else(|| t.strip_prefix("-0X")) {
        i64::from_str_radix(hex, 16).ok().map(|n| -n)
    } else {
        t.parse::<i64>().ok()
    }
}

/// Strip a quoted string literal's delimiters and un-escape any quoted delimiters
fn parse_string(t: &str) -> Option<String> {
    let t = t.trim();
    let q = t.chars().next()?;
    if (q != '"' && q != '\'') || t.chars().count() < 2 || !t.ends_with(q) {
        return None;
    }
    let inner = &t[q.len_utf8()..t.len() - q.len_utf8()];
    let escaped = format!("`{q}");
    Some(inner.replace(&escaped, &q.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(src: &str) -> Program {
        let tree = ahkbuild_syntax::parse(src).expect("tree");
        assert!(!tree.root_node().has_error(), "parse error in fixture");
        ahkbuild_ir::lower(&tree, src)
    }

    fn consts(is_compiled: Option<bool>, ptr_size: Option<u8>) -> Constants {
        Constants {
            is_compiled,
            ptr_size,
        }
    }

    /// Find the `Branch` for the program's single `if` statement.
    fn if_branch(src: &str, c: &Constants) -> Option<Branch> {
        let p = program(src);
        let r = fold(&p, c);
        let found = p
            .arena
            .iter()
            .find(|(_, n)| matches!(n.kind, NodeKind::IfStmt { .. }))
            .and_then(|(id, _)| r.branches.get(&id).copied());
        found
    }

    #[test]
    fn a_is_compiled_picks_the_else_arm_when_false() {
        let b = if_branch(
            "if A_IsCompiled {\n  Foo()\n} else {\n  Bar()\n}\n",
            &consts(Some(false), None),
        );
        assert_eq!(b, Some(Branch::Else));
    }

    #[test]
    fn a_is_compiled_picks_the_then_arm_when_true() {
        let b = if_branch(
            "if A_IsCompiled {\n  Foo()\n} else {\n  Bar()\n}\n",
            &consts(Some(true), None),
        );
        assert_eq!(b, Some(Branch::Then));
    }

    #[test]
    fn falsey_if_without_else_is_dead() {
        let b = if_branch(
            "if A_IsCompiled {\n  Foo()\n}\n",
            &consts(Some(false), None),
        );
        assert_eq!(b, Some(Branch::Dead));
    }

    #[test]
    fn unset_constant_does_not_fold() {
        let b = if_branch("if A_IsCompiled {\n  Foo()\n}\n", &consts(None, None));
        assert_eq!(b, None);
    }

    #[test]
    fn ptrsize_comparison_folds() {
        // `A_PtrSize = 8` is a constant true on a 64-bit target.
        let b = if_branch(
            "if (A_PtrSize = 8) {\n  Wide()\n} else {\n  Narrow()\n}\n",
            &consts(None, Some(8)),
        );
        assert_eq!(b, Some(Branch::Then));
        let b = if_branch(
            "if (A_PtrSize = 8) {\n  Wide()\n} else {\n  Narrow()\n}\n",
            &consts(None, Some(4)),
        );
        assert_eq!(b, Some(Branch::Else));
    }

    #[test]
    fn short_circuit_discards_non_constant_rhs() {
        // `A_IsCompiled && Foo()` folds to false via short-circuit without Foo being constant.
        let b = if_branch(
            "if (A_IsCompiled && Foo()) {\n  X()\n}\n",
            &consts(Some(false), None),
        );
        assert_eq!(b, Some(Branch::Dead));
    }

    #[test]
    fn non_constant_condition_yields_nothing() {
        assert_eq!(
            if_branch("if Foo() {\n  X()\n}\n", &consts(Some(false), Some(8))),
            None
        );
    }

    #[test]
    fn boolean_arithmetic_is_numeric() {
        // `true + 1` is 2, which is truthy.
        let b = if_branch("if (true + 1) {\n  X()\n}\n", &consts(None, None));
        assert_eq!(b, Some(Branch::Then));
    }

    #[test]
    fn substitutable_builtins_are_recorded() {
        let p = program("MsgBox(A_IsCompiled)\nMsgBox(A_PtrSize)\n");
        let r = fold(&p, &consts(Some(true), Some(8)));
        let vals: Vec<_> = r.literals.values().cloned().collect();
        assert!(vals.contains(&ConstValue::Int(1)), "A_IsCompiled -> 1");
        assert!(vals.contains(&ConstValue::Int(8)), "A_PtrSize -> 8");
    }

    /// The values the program's substitutions resolve to.
    fn subs(src: &str, c: &Constants) -> Vec<ConstValue> {
        let p = program(src);
        fold(&p, c).literals.values().cloned().collect()
    }

    #[test]
    fn maximal_expression_folds_to_one_value() {
        // The whole `A_PtrSize * 8` is recorded as `64`, not the inner `A_PtrSize` as `8`.
        let v = subs("x := A_PtrSize * 8\n", &consts(None, Some(8)));
        assert_eq!(v, vec![ConstValue::Int(64)]);
    }

    #[test]
    fn string_concat_with_builtin_folds() {
        let v = subs("x := \"lib\" . A_PtrSize\n", &consts(None, Some(8)));
        assert_eq!(v, vec![ConstValue::Str("lib8".to_string())]);
    }

    #[test]
    fn implicit_concat_with_builtin_folds() {
        // Adjacency (no `.`) concatenates just like the explicit form.
        let v = subs("x := \"lib\" A_PtrSize\n", &consts(None, Some(8)));
        assert_eq!(v, vec![ConstValue::Str("lib8".to_string())]);
    }

    #[test]
    fn concat_through_parenthesized_operand_folds() {
        // A parenthesized operand surfaces its inner node; the wrapping parens must not pollute
        // the operator span and break concat folding (`"v" . (A_PtrSize ? "a" : "b")`).
        let v = subs(
            "x := \"v\" . (A_PtrSize == 8 ? \"a\" : \"b\")\n",
            &consts(None, Some(8)),
        );
        assert_eq!(v, vec![ConstValue::Str("va".to_string())]);
    }

    #[test]
    fn float_result_falls_back_to_leaf_substitution() {
        // `A_PtrSize / 3` is a float we won't render; only the inner `A_PtrSize` is recorded.
        let v = subs("x := A_PtrSize / 3\n", &consts(None, Some(8)));
        assert_eq!(v.len(), 2, "expected two substitutions, got {:?}", v);
        assert!(v.contains(&ConstValue::Int(8)), "A_PtrSize -> 8 missing");
        assert!(v.contains(&ConstValue::Int(3)), "literal 3 missing");
    }

    #[test]
    fn ternary_resolves_to_an_arm() {
        let p = program("x := A_IsCompiled ? 1 : 2\n");
        let r = fold(&p, &consts(Some(false), None));
        let ternary = p
            .arena
            .iter()
            .find(|(_, n)| matches!(n.kind, NodeKind::TernaryExpr { .. }))
            .map(|(id, _)| id)
            .unwrap();
        assert_eq!(r.branches.get(&ternary), Some(&Branch::Else));
    }

    #[test]
    fn property_name_true_false_not_substituted() {
        // `MyObj.True` - `True` is a property name, not the boolean constant; must not fold.
        let v = subs("x := MyObj.True\n", &consts(None, None));
        assert!(
            v.is_empty(),
            "property name True must not be substituted, got {v:?}"
        );
        let v = subs("x := MyObj.False\n", &consts(None, None));
        assert!(
            v.is_empty(),
            "property name False must not be substituted, got {v:?}"
        );
    }

    #[test]
    fn ptr_size_from_requires_reads_bitness() {
        let p = program("#Requires AutoHotkey v2.1-alpha.22 32-bit\nMsgBox(1)\n");
        assert_eq!(ptr_size_from_requires(&p), Some(4));
        let p = program("#Requires AutoHotkey v2.1-alpha.22 64-bit\n");
        assert_eq!(ptr_size_from_requires(&p), Some(8));
        let p = program("#Requires AutoHotkey v2.1-alpha.22\n");
        assert_eq!(ptr_size_from_requires(&p), None);
    }
}
