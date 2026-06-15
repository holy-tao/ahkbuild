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
    // entry's `#Import Greeter` re-targets to it.
    assert!(ahk.contains("#Import Greeter"), "{ahk}");
    assert!(ahk.contains("Greeter.Hello()"), "{ahk}");
    assert!(ahk.contains("\n#Module Greeter\n"), "{ahk}");
    assert!(ahk.contains("export Hello()"), "{ahk}");
    // Entry body precedes the appended module block.
    assert!(ahk.find("Greeter.Hello()").unwrap() < ahk.find("#Module Greeter").unwrap());
}
