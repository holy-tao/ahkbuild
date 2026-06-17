//! Integration tests for the single-`.ahk` emitter: link a real entry from a temp dir, then
//! check the emitted bundle. (Resolution itself is tested in the `link` crate.)

use std::fs;
use std::path::PathBuf;

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

#[test]
fn bundle_wraps_imports_in_module_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Greeter\nGreeter.Hello()\n");
    write(tmp.path(), "Greeter.ahk", "export Hello() {\n    x := 1\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    // Entry content is preserved verbatim; the import is wrapped in a #Module block so the
    // entry's `#Import Greeter` re-targets to it. The bare name already matches the assigned
    // module name, so it is left untouched (no rewrite).
    assert!(ahk.contains("#Import Greeter"), "{ahk}");
    assert!(ahk.contains("Greeter.Hello()"), "{ahk}");
    assert!(ahk.contains("\n#Module Greeter\n"), "{ahk}");
    assert!(ahk.contains("export Hello()"), "{ahk}");
    // Entry body precedes the appended module block.
    assert!(ahk.find("Greeter.Hello()").unwrap() < ahk.find("#Module Greeter").unwrap());
}

#[test]
fn path_import_is_rewritten_to_in_file_module_name() {
    let tmp = tempfile::tempdir().unwrap();
    // A quoted relative-path import with an alias: the path can't be a `#Module` name and
    // won't resolve from the bundle's location, so it must be rewritten to the assigned name.
    let main = write(tmp.path(), "main.ahk", "#Import \"lib/Greeter\" as G\nG.Hello()\n");
    write(tmp.path(), "lib/Greeter.ahk", "export Hello() {\n    return 1\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    // The path spec is gone; the import now names the in-file module, alias preserved.
    assert!(!ahk.contains("lib/Greeter"), "path should be rewritten: {ahk}");
    assert!(ahk.contains("#Import Greeter as G"), "{ahk}");
    assert!(ahk.contains("\n#Module Greeter\n"), "{ahk}");
}

#[test]
fn assigned_module_name_is_sanitized_to_a_valid_identifier() {
    let tmp = tempfile::tempdir().unwrap();
    // A file stem that is not a legal `#Module` name (leading digit, hyphen) must be coerced.
    let main = write(tmp.path(), "main.ahk", "#Import \"3d-utils\" as U\nU.Val()\n");
    write(tmp.path(), "3d-utils.ahk", "export Val() {\n    return 1\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    assert!(ahk.contains("\n#Module M3d_utils\n"), "{ahk}");
    assert!(ahk.contains("#Import M3d_utils as U"), "{ahk}");
}

#[test]
fn same_stem_in_different_dirs_gets_unique_names() {
    let tmp = tempfile::tempdir().unwrap();
    // Two distinct files share the stem `Util`; each becomes its own group and must get a
    // distinct module name, with both importers redirected accordingly.
    let main = write(
        tmp.path(),
        "main.ahk",
        "#Import \"a/Util\" as A\n#Import \"b/Util\" as B\n",
    );
    write(tmp.path(), "a/Util.ahk", "export V() {\n    return 1\n}\n");
    write(tmp.path(), "b/Util.ahk", "export V() {\n    return 2\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    assert!(ahk.contains("\n#Module Util\n"), "{ahk}");
    assert!(ahk.contains("\n#Module Util_2\n"), "{ahk}");
    assert!(ahk.contains("#Import Util as A"), "{ahk}");
    assert!(ahk.contains("#Import Util_2 as B"), "{ahk}");
}

#[test]
fn colliding_submodules_across_groups_are_renamed() {
    let tmp = tempfile::tempdir().unwrap();
    // Two imported files each define a `#Module Helper`. In the flat single-file output their
    // names would merge; the emitter must give one of them a distinct name.
    let main = write(tmp.path(), "main.ahk", "#Import A\n#Import B\n");
    write(tmp.path(), "A.ahk", "#Module Helper\nx := 1\n");
    write(tmp.path(), "B.ahk", "#Module Helper\ny := 2\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    assert!(ahk.contains("\n#Module Helper\n"), "{ahk}");
    assert!(ahk.contains("\n#Module Helper_2\n"), "{ahk}");
    // The group wrappers themselves are present and distinct.
    assert!(ahk.contains("\n#Module A\n"), "{ahk}");
    assert!(ahk.contains("\n#Module B\n"), "{ahk}");
}

#[test]
fn path_qualified_import_is_rewritten_to_submodule_name() {
    let tmp = tempfile::tempdir().unwrap();
    // A path-qualified import targets Thing's `Inner` sub-module; the spec must be rewritten
    // to that sub-module's output name (here unchanged: `Inner`).
    let main = write(tmp.path(), "main.ahk", "#Import \"Thing:Inner\" as I\nI.Q()\n");
    write(
        tmp.path(),
        "Thing.ahk",
        "P := 1\n#Module Inner\nexport Q() {\n    return 2\n}\n",
    );

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    assert!(!ahk.contains("Thing:Inner"), "path spec should be gone: {ahk}");
    assert!(ahk.contains("#Import Inner as I"), "{ahk}");
    assert!(ahk.contains("\n#Module Thing\n"), "{ahk}");
    assert!(ahk.contains("\n#Module Inner\n"), "{ahk}");
}

#[test]
fn submodule_colliding_with_group_name_is_renamed_and_redirected() {
    let tmp = tempfile::tempdir().unwrap();
    // Foo.ahk's group is named `Foo`; Bar.ahk has an inner `#Module Foo` that collides with
    // it. The inner one must be renamed and the path-qualified importer redirected to it.
    let main = write(
        tmp.path(),
        "main.ahk",
        "#Import Foo\n#Import \"Bar:Foo\" as BF\nBF.X()\n",
    );
    write(tmp.path(), "Foo.ahk", "export Y() {\n    return 1\n}\n");
    write(
        tmp.path(),
        "Bar.ahk",
        "Z := 1\n#Module Foo\nexport X() {\n    return 2\n}\n",
    );

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    // Foo's group keeps `Foo`; Bar's inner `#Module Foo` is renamed to `Foo_2`.
    assert!(ahk.contains("\n#Module Foo\n"), "{ahk}");
    assert!(ahk.contains("\n#Module Foo_2\n"), "{ahk}");
    // The bare import of Foo is untouched; the path-qualified one points at the renamed module.
    assert!(ahk.contains("#Import Foo\n"), "{ahk}");
    assert!(ahk.contains("#Import Foo_2 as BF"), "{ahk}");
}

#[test]
fn include_is_spliced_inline() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "Before()\n#Include lib.ahk\nAfter()\n",
    );
    write(tmp.path(), "lib.ahk", "LibFn() {\n    return 7\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    // The directive is replaced by the included file's text, in place between Before and After.
    assert!(!ahk.contains("#Include"), "directive should be spliced away: {ahk}");
    assert!(ahk.contains("LibFn()"), "{ahk}");
    let before = ahk.find("Before()").unwrap();
    let lib = ahk.find("LibFn()").unwrap();
    let after = ahk.find("After()").unwrap();
    assert!(before < lib && lib < after, "splice out of order: {ahk}");
    // No extra module wrapping — includes stay in the entry group.
    assert!(!ahk.contains("#Module"), "{ahk}");
}

#[test]
fn duplicate_include_is_emitted_once() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "#Include util.ahk\n#Include util.ahk\n",
    );
    write(tmp.path(), "util.ahk", "UtilFn() {\n    return 1\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    // First include splices the body; the deduped repeat emits nothing.
    assert_eq!(ahk.matches("UtilFn()").count(), 1, "{ahk}");
}

#[test]
fn include_again_is_emitted_each_time() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "#Include util.ahk\n#IncludeAgain util.ahk\n",
    );
    write(tmp.path(), "util.ahk", "UtilFn() {\n    return 1\n}\n");

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan, None);

    // #IncludeAgain pastes the content a second time.
    assert_eq!(ahk.matches("UtilFn()").count(), 2, "{ahk}");
}
