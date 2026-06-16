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
