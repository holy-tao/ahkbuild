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
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan);

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
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan);

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
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan);

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
    let ahk = ahkbuild_emit::emit_ahk(&out.program, &out.plan);

    assert!(ahk.contains("\n#Module Util\n"), "{ahk}");
    assert!(ahk.contains("\n#Module Util_2\n"), "{ahk}");
    assert!(ahk.contains("#Import Util as A"), "{ahk}");
    assert!(ahk.contains("#Import Util_2 as B"), "{ahk}");
}
