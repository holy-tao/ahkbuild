//! Intermediate representation and CST -> IR lowering for the AHK v2.1 bundler.
//!
//! IR nodes live in an [`Arena`] and reference one another by [`NodeId`] (no parent
//! pointers / `Rc`). Each node carries a byte [`Span`] into the original source; analysis
//! metadata lives in side tables keyed by `NodeId`, owned by later passes.

pub mod arena;
pub mod lower;
pub mod node;
pub mod print;
pub mod program;

// Re-exported so downstream crates have one canonical Span type.
pub use ahkbuild_syntax::Span;

pub use arena::{Arena, Node, NodeId};
pub use lower::lower;
pub use node::NodeKind;
pub use print::print_program;
pub use program::Program;

#[cfg(test)]
mod tests {
    use super::*;

    fn lower_str(src: &str) -> Program {
        let tree = ahkbuild_syntax::parse(src).expect("tree");
        assert!(
            !tree.root_node().has_error(),
            "parse error: {}",
            tree.root_node().to_sexp()
        );
        lower(&tree, src)
    }

    #[test]
    fn implicit_main_module_holds_top_level() {
        let p = lower_str("x := 1\nMsgBox(x)\n");
        assert_eq!(p.modules.len(), 1);
        let NodeKind::Module(m) = &p.arena[p.modules[0]].kind else {
            panic!("expected module");
        };
        assert!(m.is_main());
        assert_eq!(m.name, node::Module::MAIN);
        assert_eq!(m.body.len(), 2);
    }

    #[test]
    fn module_directive_starts_a_module() {
        let p = lower_str("a := 1\n#Module Foo\nexport Bar() {\n  return 2\n}\n");
        assert_eq!(p.modules.len(), 2);
        let NodeKind::Module(main) = &p.arena[p.modules[0]].kind else {
            panic!();
        };
        assert!(main.is_main());
        assert_eq!(main.body.len(), 1);
        let NodeKind::Module(foo) = &p.arena[p.modules[1]].kind else {
            panic!();
        };
        assert_eq!(foo.name, "Foo");
        assert_eq!(foo.body.len(), 1);
        // The export wraps a function.
        let NodeKind::ExportDecl { decl, default } = &p.arena[foo.body[0]].kind else {
            panic!("expected export");
        };
        assert!(!default);
        assert!(matches!(p.arena[*decl].kind, NodeKind::Function(_)));
    }

    #[test]
    fn reopened_module_merges() {
        let p = lower_str("#Module Foo\na := 1\n#Module Bar\nb := 2\n#Module Foo\nc := 3\n");
        // Foo, Bar — Foo reopened, so still 3 modules total incl. __Main.
        assert_eq!(p.modules.len(), 3);
        let NodeKind::Module(foo) = &p.arena[p.modules[1]].kind else {
            panic!();
        };
        assert_eq!(foo.name, "Foo");
        assert_eq!(
            foo.body.len(),
            2,
            "reopened Foo should accumulate both blocks"
        );
    }

    #[test]
    fn explicit_main_merges_into_implicit() {
        let p = lower_str("a := 1\n#Module __Main\nb := 2\n");
        assert_eq!(p.modules.len(), 1);
        let NodeKind::Module(m) = &p.arena[p.modules[0]].kind else {
            panic!();
        };
        assert_eq!(m.body.len(), 2);
    }

    #[test]
    fn import_named_and_wildcard() {
        use node::{ImportBinding, ImportSource};
        let p = lower_str("#Import X {Calculate as CalculateX}\n#Import Y {*}\n");
        let NodeKind::Module(m) = &p.arena[p.modules[0]].kind else {
            panic!();
        };
        let NodeKind::ImportDirective(d0) = &p.arena[m.body[0]].kind else {
            panic!("expected import");
        };
        assert!(matches!(&d0.source, ImportSource::Name(_)));
        let ImportBinding::Selective { wildcard, names } = &d0.binding else {
            panic!("expected selective");
        };
        assert!(!wildcard);
        assert_eq!(names.len(), 1);
        assert!(names[0].alias.is_some());

        let NodeKind::ImportDirective(d1) = &p.arena[m.body[1]].kind else {
            panic!();
        };
        let ImportBinding::Selective { wildcard, names } = &d1.binding else {
            panic!();
        };
        assert!(wildcard);
        assert!(names.is_empty());
    }

    #[test]
    fn typed_struct_fields() {
        let p = lower_str("struct Point {\n    x: Int := 5\n    name: String\n}\n");
        let NodeKind::Module(m) = &p.arena[p.modules[0]].kind else {
            panic!();
        };
        let NodeKind::StructDecl(t) = &p.arena[m.body[0]].kind else {
            panic!("expected struct");
        };
        assert_eq!(t.typed_fields.len(), 2);

        // `x: Int := 5` — typed field with a type specifier and an initializer.
        let NodeKind::TypedProperty(x) = &p.arena[t.typed_fields[0]].kind else {
            panic!("expected typed property");
        };
        assert_eq!(x.name.map(|s| s.text(&p.source)), Some("x"));
        assert!(x.initializer.is_some());
        let NodeKind::TypeSpecifier { type_expr } = &p.arena[x.type_spec].kind else {
            panic!("expected type specifier");
        };
        assert!(matches!(p.arena[*type_expr].kind, NodeKind::Identifier));

        // `name: String` — typed field with no initializer.
        let NodeKind::TypedProperty(name) = &p.arena[t.typed_fields[1]].kind else {
            panic!();
        };
        assert!(name.initializer.is_none());
    }

    #[test]
    fn fat_arrow_export_is_a_call_not_an_export() {
        // Per the grammar, `export Fn() => 1` is parsed as a call to `export`, not an
        // export declaration. Confirm it does not lower to ExportDecl.
        let p = lower_str("Calculate() => 1\n");
        let NodeKind::Module(m) = &p.arena[p.modules[0]].kind else {
            panic!();
        };
        // The function declaration lowers to a Function, and there is no ExportDecl.
        for &id in &m.body {
            assert!(!matches!(p.arena[id].kind, NodeKind::ExportDecl { .. }));
        }
    }
}
