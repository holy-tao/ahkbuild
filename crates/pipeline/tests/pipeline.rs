//! Integration tests for the fixpoint driver: link a real entry from a temp dir, then exercise
//! the outer-loop machinery (materialize -> re-parse -> `link_bundle`) and the end-to-end
//! equivalence with a direct single-pass emit.

use std::fs;
use std::path::{Path, PathBuf};

use ahkbuild_emit::{materialize, EmitOptions, InlineEdits};
use ahkbuild_fold::{fold, Constants};
use ahkbuild_ir::NodeKind;
use ahkbuild_link::{link_bundle, link_entry, SearchPath};
use ahkbuild_shake::{shake, TrustSet};

/// Write `name` -> `contents` under `dir`, creating parent dirs.
fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

/// A fixture with an `#Import`, a build-time branch, and dead code - exercises fold + shake.
fn scenario(dir: &Path) -> PathBuf {
    let main = write(
        dir,
        "main.ahk",
        "#Import Lib\n\
         Lib.Used()\n\
         if A_IsCompiled {\n    CompiledOnly()\n} else {\n    NotCompiled()\n}\n\
         Used2()\n\
         Used2() {\n    return 1\n}\n\
         CompiledOnly() {\n    return 2\n}\n\
         NotCompiled() {\n    return 3\n}\n\
         Dead() {\n    return 4\n}\n",
    );
    write(
        dir,
        "Lib.ahk",
        "export Used() {\n    return 1\n}\nexport Unused() {\n    return 2\n}\n",
    );
    main
}

/// With inlining stubbed empty, the driver's outer loop runs once, so its output must be
/// byte-identical to a direct `fold` + `shake` + `emit_ahk`.
#[test]
fn bundle_matches_single_pass_emit() {
    let tmp = tempfile::tempdir().unwrap();
    let main = scenario(tmp.path());
    let consts = Constants {
        is_compiled: Some(false),
        ptr_size: None,
    };
    let opts = EmitOptions::default();

    // Direct, single-pass reference.
    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let f = fold(&out.program, &consts);
    let s = shake(&out.program, &out.plan, Some(&f), &TrustSet::default());
    let direct = ahkbuild_emit::emit_ahk(&out.program, &out.plan, Some(&s), Some(&f), &opts);

    // Through the pipeline (consumes the LinkOutput, so re-link).
    let out2 = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let piped =
        ahkbuild_pipeline::bundle_ahk(out2, consts, TrustSet::default(), true, &opts).unwrap();

    assert_eq!(direct, piped);
    // Sanity: the branch folded and dead code was removed (so the test isn't vacuous).
    assert!(
        !piped.contains("CompiledOnly"),
        "then-arm should be gone:\n{piped}"
    );
    assert!(!piped.contains("Dead"), "dead fn should be gone:\n{piped}");
}

/// A materialized bundle re-parses cleanly and re-lowers (the outer loop's re-entry point).
#[test]
fn materialize_output_reparses() {
    let tmp = tempfile::tempdir().unwrap();
    let main = scenario(tmp.path());
    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();

    let text = materialize(&out.program, &out.plan, None, None, &InlineEdits::default());
    let tree = ahkbuild_syntax::parse(&text).expect("tree");
    assert!(
        !tree.root_node().has_error(),
        "materialized bundle should re-parse cleanly:\n{}\n---\n{text}",
        tree.root_node().to_sexp()
    );
}

/// `link_bundle` re-resolves the now in-group `#Import` of a re-parsed bundle, with an empty
/// include list and identity module names.
#[test]
fn link_bundle_resolves_in_group_imports() {
    let tmp = tempfile::tempdir().unwrap();
    let main = scenario(tmp.path());
    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();

    // Materialize (no shake/fold) -> single self-contained text, then re-parse + re-lower.
    let text = materialize(&out.program, &out.plan, None, None, &InlineEdits::default());
    let tree = ahkbuild_syntax::parse(&text).expect("tree");
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    let program = ahkbuild_ir::lower(&tree, &text);

    let relinked = link_bundle(program);
    assert!(relinked.warnings.is_empty(), "{:?}", relinked.warnings);
    assert!(
        relinked.plan.resolved_includes.is_empty(),
        "a bundle has no unspliced includes"
    );
    // The `#Import Lib` now binds to the in-file `#Module Lib`.
    assert_eq!(relinked.plan.resolved_imports.len(), 1);
    let ri = &relinked.plan.resolved_imports[0];
    let NodeKind::Module(m) = &relinked.program.arena[ri.module].kind else {
        panic!("import should resolve to a module node");
    };
    assert_eq!(m.name, "Lib");
}
