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
fn same_name_modules_across_groups_warn() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import A\n#Import B\n");
    write(tmp.path(), "A.ahk", "#Module Helper\nx := 1\n");
    write(tmp.path(), "B.ahk", "#Module Helper\ny := 2\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    assert_eq!(out.program.groups.len(), 3);
    // Both A and B carry a `Helper` sub-module -> the merge-hazard warning fires.
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("Helper") && w.contains("groups")),
        "expected a same-name warning, got: {:?}",
        out.warnings
    );
    // The two Helpers are kept as distinct modules in distinct groups.
    let helpers = module_names(&out.program)
        .iter()
        .filter(|n| *n == "Helper")
        .count();
    assert_eq!(helpers, 2);
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
fn bundle_wraps_imports_in_module_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Greeter\nGreeter.Hello()\n");
    write(tmp.path(), "Greeter.ahk", "export Hello() {\n    x := 1\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_link::emit_ahk(&out.program, &out.plan);

    // Entry content is preserved verbatim; the import is wrapped in a #Module block so the
    // entry's `#Import Greeter` re-targets to it.
    assert!(ahk.contains("#Import Greeter"), "{ahk}");
    assert!(ahk.contains("Greeter.Hello()"), "{ahk}");
    assert!(ahk.contains("\n#Module Greeter\n"), "{ahk}");
    assert!(ahk.contains("export Hello()"), "{ahk}");
    // Entry body precedes the appended module block.
    assert!(ahk.find("Greeter.Hello()").unwrap() < ahk.find("#Module Greeter").unwrap());
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
