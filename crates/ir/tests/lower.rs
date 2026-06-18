//! Snapshot tests for CST -> IR lowering. Review changes with `cargo insta review`.

use std::path::PathBuf;

use ahkbuild_ir::{lower, print_program};

/// Parse + lower a source string and render the IR tree.
fn ir(src: &str) -> String {
    let tree = ahkbuild_syntax::parse(src).expect("parser returned a tree");
    assert!(
        !tree.root_node().has_error(),
        "parse error: {}",
        tree.root_node().to_sexp()
    );
    let program = lower(&tree, src);
    print_program(&program)
}

/// Load a fixture from the repo-root `tests/fixtures` directory.
fn fixture(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

#[test]
fn modules_demo() {
    insta::assert_snapshot!(ir(&fixture("modules_demo.ahk")));
}

#[test]
fn export_global() {
    insta::assert_snapshot!(ir(&fixture("probes/p5_export_global.ahk")));
}

#[test]
fn export_default_class() {
    insta::assert_snapshot!(ir(&fixture("probes/p7_export_default_class.ahk")));
}

#[test]
fn import_wildcard_plus_named() {
    // `#Import Y {*, Extra}` — the wildcard-plus-named form fixed upstream in the grammar.
    insta::assert_snapshot!(ir(&fixture("probes/p9_import_brace_wildcard_named.ahk")));
}

#[test]
fn typed_struct() {
    // v2.1 typed fields: `name: Type := value`, including a value-less one.
    insta::assert_snapshot!(ir("\
struct Point {
    x: Int := 0
    y: Int := 0
    name: String
}
"));
}

#[test]
fn comments_are_lowered() {
    // Comments in the structural positions that have a home in the IR tree (and so appear in
    // the printed program): top level, a function body, a class body, a method body, and a
    // `case` body. Comments in slots without an IR home (param lists, object literals, etc.)
    // are lowered as unparented `Comment` nodes for stripping and don't print here; the emit
    // crate's `comments_are_stripped` test exercises those.
    insta::assert_snapshot!(ir("\
; top-level comment
x := 1  ; trailing comment
Fn() {
    ; in a function body
    y := 2
}
class C {
    ; in a class body
    M() {
        ; in a method body
        z := 3
    }
}
switch x {
    case 1:
        ; in a case body
        w := 4
}
"));
}

#[test]
fn class_with_members() {
    insta::assert_snapshot!(ir("\
class Point extends Base {
    static Origin := 0
    x := 1
    y := 2
    Dist => this.x + this.y
    Move(dx, dy) {
        this.x += dx
        this.y += dy
    }
    Name {
        get => this._name
        set => this._name := value
    }
}
"));
}
