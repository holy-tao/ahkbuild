//! Constant-fold emission: a folded `if`/ternary collapses to its surviving arm, and a known
//! build-time constant is substituted in surviving code.

use std::fs;
use std::path::PathBuf;

use ahkbuild_fold::{fold, Constants};
use ahkbuild_emit::EmitOptions;
use ahkbuild_link::{link_entry, SearchPath};

fn write(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn emit(src: &str, consts: Constants) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", src);
    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let f = fold(&out.program, &consts);
    ahkbuild_emit::emit_ahk(&out.program, &out.plan, None, Some(&f), &EmitOptions::default())
}

#[test]
fn if_collapses_to_live_arm() {
    let ahk = emit(
        "if A_IsCompiled {\n    Compiled()\n} else {\n    Script()\n}\n",
        Constants { is_compiled: Some(false), ptr_size: None },
    );
    assert!(ahk.contains("Script()"), "{ahk}");
    assert!(!ahk.contains("Compiled()"), "{ahk}");
    assert!(!ahk.contains("A_IsCompiled"), "{ahk}");
    assert!(!ahk.contains("else"), "{ahk}");
    // The redundant block braces are stripped along with the scaffolding.
    assert!(!ahk.contains('{'), "{ahk}");
    assert!(!ahk.contains('}'), "{ahk}");
}

#[test]
fn dead_if_without_else_is_removed() {
    let ahk = emit(
        "if A_IsCompiled {\n    Compiled()\n}\nAlways()\n",
        Constants { is_compiled: Some(false), ptr_size: None },
    );
    assert!(!ahk.contains("Compiled()"), "{ahk}");
    assert!(!ahk.contains("A_IsCompiled"), "{ahk}");
    assert!(ahk.contains("Always()"), "{ahk}");
}

#[test]
fn constant_is_substituted_in_live_code() {
    let ahk = emit(
        "x := A_PtrSize\n",
        Constants { is_compiled: None, ptr_size: Some(8) },
    );
    assert!(ahk.contains("x := 8"), "{ahk}");
    assert!(!ahk.contains("A_PtrSize"), "{ahk}");
}

#[test]
fn maximal_constant_expression_emits_one_value() {
    // The whole `A_PtrSize * 8` folds to `64`, not `8 * 8`; string concat folds too.
    let ahk = emit(
        "x := A_PtrSize * 8\ny := \"lib\" . A_PtrSize\n",
        Constants { is_compiled: None, ptr_size: Some(8) },
    );
    assert!(ahk.contains("x := 64"), "{ahk}");
    assert!(!ahk.contains('*'), "{ahk}");
    assert!(ahk.contains("y := \"lib8\""), "{ahk}");
}

#[test]
fn command_style_arg_keeps_its_separator_space() {
    // A command-style call argument lowers with its leading space inside the span; substituting
    // must preserve it (`MsgBox A_PtrSize` -> `MsgBox 8`, not `MsgBox8`).
    let ahk = emit(
        "MsgBox A_PtrSize\n",
        Constants { is_compiled: None, ptr_size: Some(8) },
    );
    assert!(ahk.contains("MsgBox 8"), "{ahk}");
    assert!(!ahk.contains("MsgBox8"), "{ahk}");
}

#[test]
fn ternary_unwraps_to_the_taken_arm() {
    let ahk = emit(
        "x := A_IsCompiled ? \"exe\" : \"script\"\n",
        Constants { is_compiled: Some(true), ptr_size: None },
    );
    assert!(ahk.contains("\"exe\""), "{ahk}");
    assert!(!ahk.contains("\"script\""), "{ahk}");
    assert!(!ahk.contains('?'), "{ahk}");
}
