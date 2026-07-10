//! End-to-end: a user constant whose every read folds away has its declaration shaken out of the
//! emitted bundle - both variable/static constants and getter-only properties.

use std::fs;
use std::path::PathBuf;

use ahkbuild_emit::EmitOptions;
use ahkbuild_fold::{fold, Constants};
use ahkbuild_link::{link_entry, SearchPath};
use ahkbuild_shake::{shake, TrustSet};

/// Run the full fold + tree-shake + emit pipeline (no build-time constants, so only user
/// constants drive folding).
fn bundle(src: &str) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let main: PathBuf = tmp.path().join("main.ahk");
    fs::write(&main, src).unwrap();
    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let f = fold(&out.program, &Constants::default());
    let s = shake(&out.program, &out.plan, Some(&f), &TrustSet::default());
    ahkbuild_emit::emit_ahk(
        &out.program,
        &out.plan,
        Some(&s),
        Some(&f),
        &EmitOptions::default(),
    )
}

#[test]
fn folded_variable_constant_declaration_is_removed() {
    let ahk = bundle("x := 42\nMsgBox(x)\n");
    assert!(ahk.contains("MsgBox(42)"), "{ahk}");
    assert!(
        !ahk.contains("x := 42"),
        "declaration should be shaken out: {ahk}"
    );
}

#[test]
fn folded_static_constant_in_function_is_removed() {
    let ahk =
        bundle("Greet() {\n  static PREFIX := \"Hi, \"\n  return PREFIX \"world\"\n}\nGreet()\n");
    assert!(ahk.contains("\"Hi, world\""), "{ahk}");
    assert!(
        !ahk.contains("PREFIX"),
        "declaration should be shaken out: {ahk}"
    );
}

#[test]
fn getter_only_property_is_pruned_when_all_accesses_fold() {
    let ahk = bundle("class Consts {\n  static Value => 42\n}\nMsgBox(Consts.Value)\n");
    assert!(ahk.contains("MsgBox(42)"), "{ahk}");
    assert!(
        !ahk.contains("Value =>"),
        "getter-const member should be pruned: {ahk}"
    );
}

#[test]
fn dynamic_read_keeps_the_constant_declaration() {
    // `%name%` could read FLAG at runtime, so its declaration must survive even though the static
    // `return FLAG` still folds to `1`.
    let ahk = bundle("f(name) {\n  FLAG := 1\n  y := %name%\n  return FLAG\n}\nf(\"FLAG\")\n");
    assert!(ahk.contains("FLAG := 1"), "declaration must survive: {ahk}");
}
