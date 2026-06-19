//! Branch-shaking: a declaration reachable only from a folded-away `if`/ternary arm must
//! shake out, while the surviving arm's references stay live.

use std::fs;
use std::path::PathBuf;

use ahkbuild_fold::{fold, Constants};
use ahkbuild_ir::{NodeId, NodeKind, Program};
use ahkbuild_link::{link_entry, SearchPath};
use ahkbuild_shake::shake;

fn write(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).unwrap();
    path
}

fn fn_name(p: &Program, node: NodeId) -> Option<String> {
    match &p.arena[node].kind {
        NodeKind::Function(f) => f.name.map(|s| p.span_text(s).trim().to_ascii_lowercase()),
        _ => None,
    }
}

/// Link `src`, fold under `consts`, shake, and return the set of dead function names.
fn dead_fns(src: &str, consts: Constants) -> Vec<String> {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", src);
    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let f = fold(&out.program, &consts);
    let result = shake(&out.program, &out.plan, Some(&f));
    result
        .dead
        .iter()
        .filter_map(|&n| fn_name(&out.program, n))
        .collect()
}

const SRC: &str = "\
if A_IsCompiled {
    Compiled()
} else {
    Script()
}
Compiled() {
    return 1
}
Script() {
    return 2
}
";

#[test]
fn else_arm_kept_when_not_compiled() {
    let dead = dead_fns(SRC, Constants { is_compiled: Some(false), ptr_size: None });
    assert!(dead.contains(&"compiled".to_string()), "dead: {dead:?}");
    assert!(!dead.contains(&"script".to_string()), "dead: {dead:?}");
}

#[test]
fn then_arm_kept_when_compiled() {
    let dead = dead_fns(SRC, Constants { is_compiled: Some(true), ptr_size: None });
    assert!(dead.contains(&"script".to_string()), "dead: {dead:?}");
    assert!(!dead.contains(&"compiled".to_string()), "dead: {dead:?}");
}

#[test]
fn unfolded_branch_keeps_both_arms() {
    // No constant supplied -> the condition does not fold, both functions stay live.
    let dead = dead_fns(SRC, Constants::default());
    assert!(!dead.contains(&"compiled".to_string()), "dead: {dead:?}");
    assert!(!dead.contains(&"script".to_string()), "dead: {dead:?}");
}
