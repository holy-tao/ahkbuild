//! Integration tests for the module-graph linker: real files in a temp dir, resolved via
//! the search path. (AhkImportPath *parsing* is unit-tested in `search.rs`.)

use std::fs;
use std::path::PathBuf;

use ahkbuild_ir::NodeKind;
use ahkbuild_link::{link_entry, SearchPath};

/// Write `name` -> `contents` under `dir`, creating parent dirs.
fn write(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

/// Count distinct module names across all groups (for assertions).
fn module_names(p: &ahkbuild_ir::Program) -> Vec<String> {
    let mut out = Vec::new();
    for g in &p.groups {
        for &m in &g.modules {
            if let NodeKind::Module(module) = &p.arena[m].kind {
                out.push(module.name.clone());
            }
        }
    }
    out
}

#[test]
fn imports_resolve_into_separate_groups() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Helper\nMain := 1\n");
    write(tmp.path(), "Helper.ahk", "Helper := 2\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert_eq!(out.program.groups.len(), 2, "entry + Helper");
    assert!(
        out.warnings.is_empty(),
        "unexpected warnings: {:?}",
        out.warnings
    );
}

#[test]
fn resolves_via_fixed_search_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("proj");
    let lib = tmp.path().join("lib");
    let main = write(&proj, "main.ahk", "#Import Widget\n");
    write(&lib, "Widget.ahk", "Widget := 1\n");

    // Widget is not in proj/; it must be found via the fixed search dir `lib/`.
    let out = link_entry(&main, &SearchPath::from_dirs([lib])).unwrap();
    assert_eq!(out.program.groups.len(), 2);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn directory_module_resolves_via_init_file() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Pkg\n");
    // `Pkg` is a directory whose __Init.ahk is the module entry.
    write(tmp.path(), "Pkg/__Init.ahk", "PkgVal := 1\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert_eq!(out.program.groups.len(), 2, "entry + Pkg/__Init.ahk");
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn shared_import_is_loaded_once() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import A\n#Import B\n");
    write(tmp.path(), "A.ahk", "#Import Shared\n");
    write(tmp.path(), "B.ahk", "#Import Shared\n");
    write(tmp.path(), "Shared.ahk", "S := 1\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    // main, A, B, Shared — Shared deduped despite two importers.
    assert_eq!(out.program.groups.len(), 4);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn same_name_modules_across_groups_get_unique_output_names() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import A\n#Import B\n");
    write(tmp.path(), "A.ahk", "#Module Helper\nx := 1\n");
    write(tmp.path(), "B.ahk", "#Module Helper\ny := 2\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert_eq!(out.program.groups.len(), 3);
    // The merge hazard is now resolved by renaming, so no warning fires.
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    // Both A and B still carry a `Helper` sub-module in the IR (distinct groups)...
    let helpers = module_names(&out.program)
        .iter()
        .filter(|n| *n == "Helper")
        .count();
    assert_eq!(helpers, 2);
    // ...but they are assigned distinct, program-unique output names.
    let assigned: Vec<&String> = out.plan.module_names.values().collect();
    assert!(assigned.iter().any(|n| *n == "Helper"));
    assert!(assigned.iter().any(|n| *n == "Helper_2"));
}

#[test]
fn path_qualified_import_resolves_to_submodule() {
    let tmp = tempfile::tempdir().unwrap();
    // The entry path-qualifies a sub-module of Thing's group.
    let main = write(
        tmp.path(),
        "main.ahk",
        "#Import \"Thing:Inner\" as I\nI.Q()\n",
    );
    write(
        tmp.path(),
        "Thing.ahk",
        "P := 1\n#Module Inner\nexport Q() {\n    return 2\n}\n",
    );

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert_eq!(out.program.groups.len(), 2, "entry + Thing");
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);

    // The import resolved to the *Inner* sub-module node, not Thing's primary `__Main`.
    let ri = out
        .plan
        .resolved_imports
        .iter()
        .find(|ri| ri.group.0 == 1)
        .expect("a resolved import into Thing's group");
    let NodeKind::Module(m) = &out.program.arena[ri.module].kind else {
        panic!("import target is not a module node");
    };
    assert_eq!(m.name, "Inner");
}

#[test]
fn missing_submodule_warns() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import \"Thing:Nope\" as N\n");
    write(tmp.path(), "Thing.ahk", "P := 1\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert!(
        out.warnings.iter().any(|w| w.contains("Nope")),
        "expected a missing-sub-module warning, got: {:?}",
        out.warnings
    );
}

#[test]
fn in_group_submodule_import_is_not_a_file() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Thing\n");
    // Thing's primary module imports its own `#Module Inner` sub-module. That `#Import
    // Inner` must NOT be resolved as a filesystem import (no Inner.ahk exists).
    write(
        tmp.path(),
        "Thing.ahk",
        "#Import Inner\nP := 1\n#Module Inner\nQ := 2\n",
    );

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert_eq!(
        out.program.groups.len(),
        2,
        "entry + Thing (Inner is in-group)"
    );
    assert!(
        out.warnings.is_empty(),
        "in-group import should not warn: {:?}",
        out.warnings
    );
}

#[test]
fn relative_import_resolves_against_importing_files_own_dir() {
    // The key A_ScriptDir-vs-importer-dir distinction: a module file in a *subdirectory*
    // imports a sibling that exists ONLY in that subdir, not in the entry dir. If we
    // (wrongly) resolved relative to A_ScriptDir (the entry dir) it would fail; the
    // "importing file's dir is searched first" rule must find it.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Pkg\n");
    write(tmp.path(), "Pkg/__Init.ahk", "#Import Helper\nP := 1\n");
    write(tmp.path(), "Pkg/Helper.ahk", "H := 1\n"); // sibling of __Init.ahk, only in Pkg/
                                                     // Deliberately NO tmp/Helper.ahk in the entry dir.

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert_eq!(
        out.program.groups.len(),
        3,
        "main + Pkg/__Init + Pkg/Helper"
    );
    assert!(
        out.warnings.is_empty(),
        "Helper should resolve against Pkg/, not the entry dir: {:?}",
        out.warnings
    );
}

#[test]
fn embedded_resource_import_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import \"*WIDGET\" as Widget\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    // No file to resolve for a `*RESNAME` import: just the entry group, no warnings.
    assert_eq!(out.program.groups.len(), 1);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

// ----------------------------------------------------------------------------------------
// #Include
// ----------------------------------------------------------------------------------------

use ahkbuild_link::IncludeSplice;

/// Names of every identifier-bearing declaration reachable from the entry group's modules, by
/// walking the IR — used to confirm `#Include`d declarations were spliced into the group.
fn entry_decl_names(p: &ahkbuild_ir::Program) -> Vec<String> {
    let mut out = Vec::new();
    for &m in &p.groups[0].modules {
        let NodeKind::Module(module) = &p.arena[m].kind else {
            continue;
        };
        for &stmt in &module.body {
            if let NodeKind::Function(f) = &p.arena[stmt].kind {
                if let Some(n) = f.name {
                    out.push(p.span_text(n).to_string());
                }
            }
        }
    }
    out
}

#[test]
fn include_splices_into_one_group_transitively() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "#Include lib.ahk\nMainFn() {\n  return 1\n}\n",
    );
    write(
        tmp.path(),
        "lib.ahk",
        "#Include deep.ahk\nLibFn() {\n  return 2\n}\n",
    );
    write(tmp.path(), "deep.ahk", "DeepFn() {\n  return 3\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    // #Include does not create groups: everything is in the entry group.
    assert_eq!(out.program.groups.len(), 1, "includes stay in one group");
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);

    let names = entry_decl_names(&out.program);
    for want in ["MainFn", "LibFn", "DeepFn"] {
        assert!(
            names.contains(&want.to_string()),
            "missing {want} in {names:?}"
        );
    }

    // Two First splices recorded (main->lib, lib->deep), none deduped/missing.
    let firsts = out
        .plan
        .resolved_includes
        .iter()
        .filter(|ri| matches!(ri.splice, IncludeSplice::First(_)))
        .count();
    assert_eq!(firsts, 2, "{:?}", out.plan.resolved_includes);
}

#[test]
fn include_dedup_and_again() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "#Include util.ahk\n#Include util.ahk\n#IncludeAgain util.ahk\n",
    );
    write(tmp.path(), "util.ahk", "UtilFn() {\n  return 1\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);

    let mut firsts = 0;
    let mut dedups = 0;
    for ri in &out.plan.resolved_includes {
        match ri.splice {
            IncludeSplice::First(_) => firsts += 1,
            IncludeSplice::Dedup => dedups += 1,
            IncludeSplice::Missing => {}
        }
    }
    // First plain include splices; the second is deduped; #IncludeAgain splices again.
    assert_eq!((firsts, dedups), (2, 1), "{:?}", out.plan.resolved_includes);
}

#[test]
fn include_missing_with_star_i_is_a_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Include *i nope.ahk\nX := 1\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert_eq!(out.warnings.len(), 1, "one ignored-missing warning");
    assert!(out
        .plan
        .resolved_includes
        .iter()
        .any(|ri| matches!(ri.splice, IncludeSplice::Missing)));
}

#[test]
fn include_missing_without_star_i_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Include nope.ahk\n");
    assert!(link_entry(&main, &SearchPath::from_dirs([])).is_err());
}

#[test]
fn include_cycle_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#IncludeAgain b.ahk\n");
    write(tmp.path(), "b.ahk", "#IncludeAgain main.ahk\n");
    assert!(link_entry(&main, &SearchPath::from_dirs([])).is_err());
}

#[test]
fn include_lib_resolves_with_underscore_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    // `<MyPrefix_Func>` is not found directly; the prefix file `MyPrefix.ahk` in the local
    // Lib folder is the fallback.
    let main = write(tmp.path(), "main.ahk", "#Include <MyPrefix_Func>\n");
    write(
        tmp.path(),
        "Lib/MyPrefix.ahk",
        "PrefixFn() {\n  return 1\n}\n",
    );

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert!(entry_decl_names(&out.program).contains(&"PrefixFn".to_string()));
}
