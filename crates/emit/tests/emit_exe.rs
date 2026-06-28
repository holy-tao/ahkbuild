//! Integration tests for the per-module `.exe` emitter ([`emit_exe_modules`]): link a real entry
//! from a temp dir, then check each emitted module's resource name and text. The entry group is
//! the auto-run `RT_RCDATA` id 1 ([`ResourceName::Entry`]); imports are rewritten to `"*<name>"`.

use std::fs;
use std::path::PathBuf;

use ahkbuild_emit::{emit_exe_modules, EmitOptions, ResourceName};
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
fn two_modules_emit_entry_plus_named_resource() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Greeter\nGreeter.Hello()\n");
    write(
        tmp.path(),
        "Greeter.ahk",
        "export Hello() {\n    x := 1\n}\n",
    );

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let modules = emit_exe_modules(&out.program, &out.plan, None, None, &EmitOptions::default());

    assert_eq!(modules.len(), 2, "entry + one imported group");

    // Entry is the auto-run resource, emitted without a `#Module` header.
    let entry = &modules[0];
    assert_eq!(entry.resource, ResourceName::Entry);
    assert!(entry.text.contains("Greeter.Hello()"), "{}", entry.text);
    assert!(!entry.text.contains("#Module"), "{}", entry.text);

    // The imported group is a named resource (uppercased, per Win32 lookup). It carries *no*
    // synthetic `#Module` header: the resource itself is the module, so its body is the module's
    // main code directly (a header would demote it to an empty-default sub-module).
    let greeter = &modules[1];
    assert_eq!(greeter.resource, ResourceName::Named("GREETER".into()));
    assert!(!greeter.text.contains("#Module"), "{}", greeter.text);
    assert!(greeter.text.contains("export Hello()"), "{}", greeter.text);
}

#[test]
fn path_import_is_rewritten_to_resource_spec() {
    let tmp = tempfile::tempdir().unwrap();
    // A quoted relative-path import: for the exe backend it must become `#Import "*<name>"` so the
    // interpreter loads the embedded module resource rather than touching the filesystem.
    let main = write(
        tmp.path(),
        "main.ahk",
        "#Import \"lib/Greeter\" as G\nG.Hello()\n",
    );
    write(
        tmp.path(),
        "lib/Greeter.ahk",
        "export Hello() {\n    return 1\n}\n",
    );

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let modules = emit_exe_modules(&out.program, &out.plan, None, None, &EmitOptions::default());

    let entry = &modules[0];
    // The resolved import points at the assigned resource name via the `*` spec, preserving the
    // alias.
    let ResourceName::Named(name) = &modules[1].resource else {
        panic!("second module should be named");
    };
    assert!(
        entry.text.contains(&format!("#Import \"*{name}\"")),
        "import not rewritten to resource spec: {}",
        entry.text
    );
}

#[test]
fn bare_name_import_keeps_its_binding_via_alias() {
    let tmp = tempfile::tempdir().unwrap();
    // A bare-name import binds the module's default export under the module name. Rewriting it to a
    // *quoted* resource spec would drop that binding, so the exe backend must re-add `as Greeter`
    // to keep `Greeter.Hello()` resolving at runtime (docs/lib/_Import.md).
    let main = write(tmp.path(), "main.ahk", "#Import Greeter\nGreeter.Hello()\n");
    write(
        tmp.path(),
        "Greeter.ahk",
        "export Hello() {\n    return 1\n}\n",
    );

    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let modules = emit_exe_modules(&out.program, &out.plan, None, None, &EmitOptions::default());

    assert!(
        modules[0].text.contains("#Import \"*GREETER\" as Greeter"),
        "binding not preserved: {}",
        modules[0].text
    );
}
