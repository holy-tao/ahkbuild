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
fn reexport_chain_is_kept_not_trimmed() {
    // A imports a barrel B (`#Import export ...`) that re-exports from C, which itself
    // re-exports a sibling's named export. Even though nothing in B or C *references* those
    // names locally, the re-exports are public surface and must survive — both the re-export
    // import directives (never dropped) and the re-exported target declarations (kept live).
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Barrel as Lib\nLib.Thing()\n");
    write(tmp.path(), "Barrel.ahk", "#Import export Core {*}\n");
    write(tmp.path(), "Core.ahk", "#Import export Thing\n");
    write(
        tmp.path(),
        "Thing.ahk",
        "export Thing() {\n    return Helper()\n}\nHelper() {\n}\n",
    );

    let (p, r) = run(&main);

    // The re-exported `Thing` (and its transitive `Helper`) survive.
    assert!(dead_names(&p, &r).is_empty(), "dead: {:?}", dead_names(&p, &r));
    // The re-export directives are never dropped (the chain must hold at runtime).
    assert!(
        r.dropped_imports.is_empty(),
        "re-exports must not be dropped: {:?}",
        r.dropped_imports
    );
    // No module is removed — every link in the barrel chain is needed.
    assert!(r.dead_modules.is_empty(), "dead modules: {:?}", r.dead_modules);
}

#[test]
fn reference_inside_parentheses_keeps_its_import() {
    // A reference that only appears inside parentheses (here a `!(x is Query)` type check)
    // must still mark its import used. Parenthesized expressions previously lowered to Opaque
    // and hid the reference, wrongly dropping `#Import Query`.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import QC {Check}\nCheck(0)\n");
    write(
        tmp.path(),
        "QC.ahk",
        "#Import Query\nexport Check(o) {\n    if (!(o is Query))\n        return 0\n    return 1\n}\n",
    );
    write(tmp.path(), "Query.ahk", "export default class Query {\n}\n");

    let (p, r) = run(&main);
    assert!(
        r.dropped_imports.is_empty(),
        "`#Import Query` is used inside parens and must be kept: {:?}",
        r.dropped_imports
    );
    assert!(r.dead_modules.is_empty(), "Query module must survive");
    assert!(dead_names(&p, &r).is_empty(), "dead: {:?}", dead_names(&p, &r));
}

#[test]
fn unused_import_does_not_load_its_module_even_with_static_new() {
    // The regression this whole change targets: a module reached only through an *unused*
    // import must not be loaded, so its `static __New` class never becomes a root. Importing
    // is the "use"; without it, the target's body never runs and the module shakes out whole.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Lib {Thing}\nx := 1\n");
    write(
        tmp.path(),
        "Lib.ahk",
        "export class Thing {\n}\n\nclass Boot {\n    static __New() {\n    }\n}\n",
    );
    let (p, r) = run(&main);

    // `Thing` is never referenced -> Lib is not loaded -> Boot's static __New is not a root.
    assert_eq!(r.dropped_imports.len(), 1, "the unused import is dropped");
    assert_eq!(
        r.dead_modules.len(),
        1,
        "Lib is removed despite its static __New class"
    );
    // Sanity: the dropped node is the import directive, not Boot.
    assert!(matches!(
        p.arena[r.dropped_imports[0]].kind,
        NodeKind::ImportDirective(_)
    ));
}

#[test]
fn used_import_loads_module_and_its_static_new_runs() {
    // The flip side: once the import is taken, the whole target body runs, so a sibling
    // `static __New` class in the loaded module is kept even though nothing references it.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Lib {Thing}\nThing()\n");
    write(
        tmp.path(),
        "Lib.ahk",
        "export Thing() {\n}\n\nclass Boot {\n    static __New() {\n    }\n}\n",
    );
    let (p, r) = run(&main);

    assert!(r.dropped_imports.is_empty(), "the import is used");
    assert!(r.dead_modules.is_empty(), "Lib is loaded");
    // Boot stays live via its static __New now that Lib's body runs.
    assert!(dead_names(&p, &r).is_empty(), "dead: {:?}", dead_names(&p, &r));
}

#[test]
fn side_effect_import_loads_its_module() {
    // A pure side-effect import (`#Import "path"`, no binding) binds nothing but still runs
    // the target's body, so it must load the module and is never dropped.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import \"side.ahk\"\nx := 1\n");
    write(
        tmp.path(),
        "side.ahk",
        "class Boot {\n    static __New() {\n    }\n}\n",
    );
    let (p, r) = run(&main);

    assert!(
        r.dropped_imports.is_empty(),
        "a side-effect import is never dropped: {:?}",
        r.dropped_imports
    );
    assert!(
        r.dead_modules.is_empty(),
        "side.ahk must stay loaded: {:?}",
        r.dead_modules
    );
    assert!(dead_names(&p, &r).is_empty(), "dead: {:?}", dead_names(&p, &r));
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
