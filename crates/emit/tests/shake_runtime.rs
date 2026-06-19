//! Runtime-equivalence gate for tree-shaking (the real correctness test).
//!
//! Bundles a multi-module project both with and without tree-shaking, runs each on the real
//! v2.1 interpreter, and asserts identical stdout / exit code. Any under-marking (dropping
//! live code) shows up as a behavior difference. A negative check confirms shaking actually
//! happened (smaller output, known-dead names gone) so the test can't pass trivially.
//!
//! `#[ignore]` by default — it needs the interpreter, which is environment-specific (like the
//! `tests/fixtures/probes/runtime` harness). Run with:
//!     cargo test -p ahkbuild-emit --test shake_runtime -- --ignored
//! Override the interpreter path with the `AHK_V21` environment variable.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ahkbuild_link::{link_entry, SearchPath};

fn interpreter() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("AHK_V21") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let default = PathBuf::from(r"C:\Program Files\AutoHotkey\v2.1\AutoHotkey64.exe");
    default.exists().then_some(default)
}

/// Run a script and return `(exit code, stdout with normalized newlines)`.
fn run(ahk: &Path, script: &Path) -> (i32, String) {
    let out = Command::new(ahk)
        .arg("/ErrorStdOut")
        .arg(script)
        .output()
        .expect("spawn interpreter");
    let stdout = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
    (out.status.code().unwrap_or(-1), stdout)
}

#[test]
#[ignore = "requires the v2.1 interpreter (set AHK_V21 or install to the default path)"]
fn shaken_bundle_matches_unshaken_on_interpreter() {
    let Some(ahk) = interpreter() else {
        eprintln!("skipping: v2.1 interpreter not found");
        return;
    };

    let src = tempfile::tempdir().unwrap();
    let w = |name: &str, body: &str| {
        let p = src.path().join(name);
        fs::write(&p, body).unwrap();
        p
    };
    // Entry imports one of two exports from Util (so `Sub` is dead) and never references the
    // Side binding (so the import is dropped — but Side still auto-executes its `static __New`
    // side effect). Dead: NeverCalled, UnusedClass, Util.Sub.
    let main = w(
        "main.ahk",
        "#Import Util {Add}\n\
         #Import Side\n\
         \n\
         global Result := Add(2, 3)\n\
         FileAppend(\"result=\" Result \"`n\", \"*\")\n\
         Greet()\n\
         \n\
         Greet() {\n    FileAppend(\"greet=\" Banner() \"`n\", \"*\")\n}\n\
         Banner() {\n    return \"hello\"\n}\n\
         NeverCalled() {\n    return \"dead\"\n}\n\
         class UnusedClass {\n    Method() {\n    }\n}\n",
    );
    w(
        "Util.ahk",
        "export Add(a, b) {\n    return a + b\n}\n\
         export Sub(a, b) {\n    return a - b\n}\n",
    );
    w(
        "Side.ahk",
        "class Boot {\n    static __New() {\n        FileAppend(\"boot=ran`n\", \"*\")\n    }\n}\n",
    );

    let linked = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let shaken = ahkbuild_shake::shake(&linked.program, &linked.plan, None);
    let opts = ahkbuild_emit::EmitOptions::default();
    let unshaken_src = ahkbuild_emit::emit_ahk(&linked.program, &linked.plan, None, None, &opts);
    let shaken_src =
        ahkbuild_emit::emit_ahk(&linked.program, &linked.plan, Some(&shaken), None, &opts);

    // Write the bundles to an isolated dir (no source files), so they must resolve in-process.
    let bundles = tempfile::tempdir().unwrap();
    let unshaken_path = bundles.path().join("unshaken.ahk");
    let shaken_path = bundles.path().join("shaken.ahk");
    fs::write(&unshaken_path, &unshaken_src).unwrap();
    fs::write(&shaken_path, &shaken_src).unwrap();

    let (u_code, u_out) = run(&ahk, &unshaken_path);
    let (s_code, s_out) = run(&ahk, &shaken_path);

    assert_eq!(u_code, 0, "unshaken bundle errored: {u_out}");
    assert_eq!(
        s_out, u_out,
        "tree-shaking changed runtime behavior (under-marking?)\nunshaken: {u_out:?}\nshaken: {s_out:?}"
    );
    assert_eq!(s_code, u_code, "exit codes differ");

    // Prove shaking actually happened.
    assert!(
        shaken_src.len() < unshaken_src.len(),
        "shaken output should be smaller"
    );
    for dead in ["NeverCalled", "UnusedClass", "Sub"] {
        assert!(
            !shaken_src.contains(dead),
            "dead symbol {dead:?} survived shaking:\n{shaken_src}"
        );
    }
}
