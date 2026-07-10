//! Branch-shaking: a declaration reachable only from a folded-away `if`/ternary arm must
//! shake out, while the surviving arm's references stay live.

use std::fs;
use std::path::PathBuf;

use ahkbuild_fold::{fold, Constants};
use ahkbuild_ir::{NodeId, NodeKind, Program};
use ahkbuild_link::{link_entry, SearchPath};
use ahkbuild_shake::{shake, TrustSet};

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

/// The set of dead class-*member* names (a method carries an `owner`, separating it from a dead
/// top-level function of the same name).
fn dead_members(src: &str, consts: Constants) -> Vec<String> {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", src);
    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let f = fold(&out.program, &consts);
    let result = shake(&out.program, &out.plan, Some(&f), &TrustSet::default());
    result
        .dead
        .iter()
        .filter_map(|&n| match &out.program.arena[n].kind {
            NodeKind::Function(f) if f.owner.is_some() => f
                .name
                .map(|s| out.program.span_text(s).trim().to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// Link `src`, fold under `consts`, shake, and return the set of dead function names.
fn dead_fns(src: &str, consts: Constants) -> Vec<String> {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", src);
    let out = link_entry(&main, &SearchPath::from_dirs([])).unwrap();
    let f = fold(&out.program, &consts);
    let result = shake(&out.program, &out.plan, Some(&f), &TrustSet::default());
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
    let dead = dead_fns(
        SRC,
        Constants {
            is_compiled: Some(false),
            ptr_size: None,
        },
    );
    assert!(dead.contains(&"compiled".to_string()), "dead: {dead:?}");
    assert!(!dead.contains(&"script".to_string()), "dead: {dead:?}");
}

#[test]
fn then_arm_kept_when_compiled() {
    let dead = dead_fns(
        SRC,
        Constants {
            is_compiled: Some(true),
            ptr_size: None,
        },
    );
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

/// A member named *only* inside a folded-away branch arm must prune: the member-name table has
/// to honor constant folding, or the unreachable reference keeps the member alive. This is the
/// `A_PtrSize = 4 ? this._LoadLib32Bit() : this._LoadLib64Bit()` pattern (e.g. the JSON lib),
/// where a 64-bit target should shake `_LoadLib32Bit` out in a single pass.
const MEMBER_SRC: &str = "\
Lib.Load()
class Lib {
    static Load() {
        return A_PtrSize = 4 ? this.Load32() : this.Load64()
    }
    static Load32() {
        return 32
    }
    static Load64() {
        return 64
    }
}
";

#[test]
fn member_only_in_dead_arm_is_pruned() {
    // 64-bit: `A_PtrSize = 4` is false, so `this.Load32()` is unreachable and `Load32` prunes.
    let dead = dead_members(
        MEMBER_SRC,
        Constants {
            is_compiled: None,
            ptr_size: Some(8),
        },
    );
    assert!(dead.contains(&"load32".to_string()), "dead: {dead:?}");
    assert!(!dead.contains(&"load64".to_string()), "dead: {dead:?}");
}

#[test]
fn member_in_live_arm_is_kept() {
    // 32-bit: the live arm calls `this.Load32()`, so it stays; `Load64` prunes instead.
    let dead = dead_members(
        MEMBER_SRC,
        Constants {
            is_compiled: None,
            ptr_size: Some(4),
        },
    );
    assert!(dead.contains(&"load64".to_string()), "dead: {dead:?}");
    assert!(!dead.contains(&"load32".to_string()), "dead: {dead:?}");
}

#[test]
fn member_kept_when_branch_unfolded() {
    // No bitness known -> the ternary does not fold, so both helpers stay live.
    let dead = dead_members(MEMBER_SRC, Constants::default());
    assert!(!dead.contains(&"load32".to_string()), "dead: {dead:?}");
    assert!(!dead.contains(&"load64".to_string()), "dead: {dead:?}");
}
