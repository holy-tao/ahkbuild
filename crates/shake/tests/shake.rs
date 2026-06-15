//! Reachability tests: link a (multi-file) project from a temp dir, tree-shake it, and
//! assert the exact set of dead declaration names / dropped imports.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use ahkbuild_ir::{NodeKind, Program};
use ahkbuild_link::{link_entry, SearchPath};
use ahkbuild_shake::{shake, ShakeResult};

fn write(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

/// Link `main` (plus any sibling files already written) and tree-shake the result.
fn run(main: &std::path::Path) -> (Program, ShakeResult) {
    let out = link_entry(main, &SearchPath::from_dirs([])).unwrap();
    let result = shake(&out.program, &out.plan);
    (out.program, result)
}

/// The declared name of a dead node (recursing through `export`), lowercased.
fn decl_name(p: &Program, node: ahkbuild_ir::NodeId) -> Option<String> {
    let text = |s| p.span_text(s).trim().to_ascii_lowercase();
    match &p.arena[node].kind {
        NodeKind::Function(f) => f.name.map(text),
        NodeKind::ClassDecl(t) | NodeKind::StructDecl(t) => t.name.map(text),
        NodeKind::ExportDecl { decl, .. } => decl_name(p, *decl),
        _ => None,
    }
}

/// The set of dead declaration names (ignoring non-named dead nodes like dead-module bodies).
fn dead_names(p: &Program, r: &ShakeResult) -> HashSet<String> {
    r.dead.iter().filter_map(|&n| decl_name(p, n)).collect()
}

fn names(strs: &[&str]) -> HashSet<String> {
    strs.iter().map(|s| s.to_ascii_lowercase()).collect()
}

#[test]
fn unused_function_is_dead_used_one_is_kept_transitively() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "Greet()\n\nGreet() {\n    Helper()\n}\n\nHelper() {\n}\n\nDead() {\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_names(&p, &r), names(&["Dead"]));
}

#[test]
fn referenced_class_is_kept_whole_unreferenced_is_dead() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "a := Animal()\na.Speak()\n\nclass Animal {\n    Speak() {\n    }\n}\n\nclass Unused {\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_names(&p, &r), names(&["Unused"]));
}

#[test]
fn static_new_class_is_a_root_even_if_unreferenced() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "class Boot {\n    static __New() {\n        Side()\n    }\n}\n\nSide() {\n}\n",
    );
    let (p, r) = run(&main);
    // Boot is kept (static __New runs at load); Side is reachable from it.
    assert!(dead_names(&p, &r).is_empty(), "{:?}", dead_names(&p, &r));
}

#[test]
fn superclass_reference_keeps_the_base() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "x := Derived()\n\nclass Base {\n}\n\nclass Derived extends Base {\n}\n\nclass Lonely {\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_names(&p, &r), names(&["Lonely"]));
}

#[test]
fn catch_error_type_keeps_the_class() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "try {\n    Risky()\n} catch MyError as e {\n}\n\nclass MyError extends Error {\n}\n\nRisky() {\n}\n",
    );
    let (p, r) = run(&main);
    assert!(dead_names(&p, &r).is_empty(), "{:?}", dead_names(&p, &r));
}

#[test]
fn dynamic_call_blows_up_and_keeps_everything() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "name := \"Foo\"\n%name%()\n\nFoo() {\n}\n\nBar() {\n}\n",
    );
    let (p, r) = run(&main);
    // The dynamic call defeats name resolution, so both functions are conservatively kept.
    assert!(dead_names(&p, &r).is_empty(), "{:?}", dead_names(&p, &r));
}

#[test]
fn selective_import_drops_the_unreferenced_export() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Lib {Used}\nUsed()\n");
    write(
        tmp.path(),
        "Lib.ahk",
        "export Used() {\n}\n\nexport Unused() {\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_names(&p, &r), names(&["Unused"]));
    assert!(r.dropped_imports.is_empty(), "the import is used");
}

#[test]
fn unreferenced_import_is_dropped_and_its_module_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Lib\nx := 1\n");
    write(tmp.path(), "Lib.ahk", "export Used() {\n}\n");
    let (p, r) = run(&main);

    // `Lib` is never referenced -> the import is dropped and the whole module is dead.
    assert_eq!(r.dropped_imports.len(), 1, "the unused import is dropped");
    assert!(
        matches!(
            p.arena[r.dropped_imports[0]].kind,
            NodeKind::ImportDirective(_)
        ),
        "dropped node is an import directive"
    );
    assert_eq!(r.dead_modules.len(), 1, "Lib module is removed");
}
