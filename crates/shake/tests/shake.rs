//! Reachability tests: link a (multi-file) project from a temp dir, tree-shake it, and
//! assert the exact set of dead declaration names / dropped imports.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use ahkbuild_ir::{NodeKind, Program};
use ahkbuild_link::{link_entry, SearchPath};
use ahkbuild_shake::{shake, ShakeResult};

fn write(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, contents).unwrap();
    path
}

/// Link `main` (plus any sibling files already written) and tree-shake the result.
fn run(main: &std::path::Path) -> (Program, ShakeResult) {
    let out = link_entry(main, &SearchPath::from_dirs([])).unwrap();
    let result = shake(&out.program, &out.plan, None);
    (out.program, result)
}

/// The declared name of a dead node (recursing through `export`), lowercased.
fn decl_name(p: &Program, node: ahkbuild_ir::NodeId) -> Option<String> {
    let text = |s| p.span_text(s).trim().to_ascii_lowercase();
    match &p.arena[node].kind {
        NodeKind::Function(f) => f.name.map(text),
        NodeKind::ClassDecl(t) | NodeKind::StructDecl(t) => t.name.map(text),
        NodeKind::ExportDecl { decl, .. } => decl_name(p, *decl),
        _ => None,
    }
}

/// The set of dead declaration names (ignoring non-named dead nodes like dead-module bodies).
fn dead_names(p: &Program, r: &ShakeResult) -> HashSet<String> {
    r.dead.iter().filter_map(|&n| decl_name(p, n)).collect()
}

fn names(strs: &[&str]) -> HashSet<String> {
    strs.iter().map(|s| s.to_ascii_lowercase()).collect()
}

/// The name of a dead *class member* (method/property/field), lowercased. Methods carry an
/// `owner`, which cleanly separates them from dead top-level functions; properties and fields
/// only ever appear as members.
fn member_name(p: &Program, node: ahkbuild_ir::NodeId) -> Option<String> {
    let text = |s| p.span_text(s).trim().to_ascii_lowercase();
    match &p.arena[node].kind {
        NodeKind::Function(f) if f.owner.is_some() => f.name.map(text),
        NodeKind::Property(pr) => pr.name.map(text),
        NodeKind::Field(fl) => fl.name.map(text),
        NodeKind::TypedProperty(tp) => tp.name.map(text),
        _ => None,
    }
}

/// The set of pruned class-member names.
fn dead_member_names(p: &Program, r: &ShakeResult) -> HashSet<String> {
    r.dead.iter().filter_map(|&n| member_name(p, n)).collect()
}

/// Whether any dead node's source text contains `needle` (used to spot pruned `DefineProp`s).
fn any_dead_text_contains(p: &Program, r: &ShakeResult, needle: &str) -> bool {
    r.dead.iter().any(|&n| p.text(n).contains(needle))
}

// ---------------------------------------------------------------------------
// Member-level pruning (ported from build/tests/memberpruning.test.ahk)
// ---------------------------------------------------------------------------

#[test]
fn unreferenced_method_is_pruned_referenced_one_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\no.Used()\n\nclass C {\n    Used() {\n    }\n    Unused() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Unused"]));
}

#[test]
fn unreferenced_property_is_pruned() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\nx := o.ReadIt\n\nclass C {\n    ReadIt => 1\n    Hidden => 2\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Hidden"]));
}

#[test]
fn unreferenced_static_method_is_pruned() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "C.DoStatic()\n\nclass C {\n    static DoStatic() {\n    }\n    static Hidden() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Hidden"]));
}

#[test]
fn protected_meta_members_are_never_pruned() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\no.Used()\n\nclass C {\n    __New() {\n    }\n    __Delete() {\n    }\n    __Item[k] {\n        get => k\n    }\n    Used() {\n    }\n    Unused() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    // Only the ordinary unreferenced method shakes out; the meta-members stay.
    assert_eq!(dead_member_names(&p, &r), names(&["Unused"]));
}

#[test]
fn keep_directive_pins_an_unreferenced_member() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\no.Used()\n\nclass C {\n    Used() {\n    }\n    ;@AhkBuild-Keep\n    Kept() {\n    }\n    Unused() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Unused"]));
}

#[test]
fn member_names_match_across_unrelated_classes() {
    // The name table is global: `.Foo` anywhere keeps `Foo` in every class. `B.Foo` survives
    // even though only `a.Foo()` is called; both `Bar`s shake out.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "a := A()\na.Foo()\nb := B()\n\nclass A {\n    Foo() {\n    }\n    Bar() {\n    }\n}\n\nclass B {\n    Foo() {\n    }\n    Bar() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Bar"]));
}

#[test]
fn fully_dynamic_member_access_disables_pruning() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\nn := \"A\"\no.%n%()\n\nclass C {\n    A() {\n    }\n    B() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    // The table blows up, so every member is kept.
    assert!(
        dead_member_names(&p, &r).is_empty(),
        "{:?}",
        dead_member_names(&p, &r)
    );
}

#[test]
fn dynamic_prefix_keeps_matching_members() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\ns := \"Name\"\no.Get%s%()\n\nclass C {\n    GetName() {\n    }\n    GetAge() {\n    }\n    SetName() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["SetName"]));
}

#[test]
fn dynamic_suffix_keeps_matching_members() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\nk := \"Click\"\no.%k%Handler()\n\nclass C {\n    ClickHandler() {\n    }\n    KeyHandler() {\n    }\n    Other() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Other"]));
}

#[test]
fn dynamic_inner_string_literal_keeps_exact_member() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\no.%\"Target\"%()\n\nclass C {\n    Target() {\n    }\n    Other() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Other"]));
}

#[test]
fn resolves_to_directive_keeps_named_members() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\nn := \"Alpha\"\n;@AhkBuild-ResolvesTo Alpha, Beta\no.%n%()\n\nclass C {\n    Alpha() {\n    }\n    Beta() {\n    }\n    Gamma() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Gamma"]));
}

#[test]
fn objbindmethod_literal_keeps_named_member() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\ncb := ObjBindMethod(o, \"Bound\")\n\nclass C {\n    Bound() {\n    }\n    Unbound() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Unbound"]));
}

#[test]
fn objbindmethod_non_literal_disables_pruning() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\nm := \"Bound\"\ncb := ObjBindMethod(o, m)\n\nclass C {\n    Bound() {\n    }\n    Unbound() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert!(
        dead_member_names(&p, &r).is_empty(),
        "{:?}",
        dead_member_names(&p, &r)
    );
}

#[test]
fn defineprop_with_unreferenced_name_is_pruned() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\no.Used()\n\nclass C {\n    __New() {\n        this.DefineProp(\"Unused\", {Get: (*) => 1})\n    }\n    Used() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert!(
        any_dead_text_contains(&p, &r, "DefineProp(\"Unused\""),
        "the DefineProp call should be pruned; dead spans: {:?}",
        r.dead.iter().map(|&n| p.text(n)).collect::<Vec<_>>()
    );
}

#[test]
fn defineprop_with_referenced_name_survives() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\nx := o.Kept\n\nclass C {\n    __New() {\n        this.DefineProp(\"Kept\", {Get: (*) => 1})\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert!(
        !any_dead_text_contains(&p, &r, "DefineProp"),
        "the DefineProp defines a referenced name and must be kept"
    );
}

#[test]
fn unreferenced_constant_field_is_pruned_but_side_effecting_one_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "o := C()\no.Used()\n\nclass C {\n    config := 0\n    live := Compute()\n    Used() {\n    }\n}\n\nCompute() {\n    return 1\n}\n",
    );
    let (p, r) = run(&main);
    // `config := 0` (constant) shakes out; `live := Compute()` is kept (its initializer has
    // side effects) and keeps `Compute` reachable.
    assert_eq!(dead_member_names(&p, &r), names(&["config"]));
    assert!(
        !dead_names(&p, &r).contains("compute"),
        "Compute is referenced by a kept field initializer"
    );
}

#[test]
fn struct_instance_members_are_never_pruned() {
    // Pruning a struct's instance field would change its binary layout / identity, so every
    // non-static struct member is kept regardless of references. Only `static` struct members
    // are prunable. Here `y` (unreferenced instance field) survives, while the unreferenced
    // static field `Scale` and static method `Unused` shake out.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "v := Pt()\nx := v.X\n\nstruct Pt {\n    X: Int := 0\n    Y: Int := 0\n    static Scale := 2\n    static Unused() {\n    }\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_member_names(&p, &r), names(&["Scale", "Unused"]));
}

#[test]
fn unused_function_is_dead_used_one_is_kept_transitively() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "Greet()\n\nGreet() {\n    Helper()\n}\n\nHelper() {\n}\n\nDead() {\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_names(&p, &r), names(&["Dead"]));
}

#[test]
fn referenced_class_is_kept_whole_unreferenced_is_dead() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "a := Animal()\na.Speak()\n\nclass Animal {\n    Speak() {\n    }\n}\n\nclass Unused {\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_names(&p, &r), names(&["Unused"]));
}

#[test]
fn static_new_class_is_a_root_even_if_unreferenced() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "class Boot {\n    static __New() {\n        Side()\n    }\n}\n\nSide() {\n}\n",
    );
    let (p, r) = run(&main);
    // Boot is kept (static __New runs at load); Side is reachable from it.
    assert!(dead_names(&p, &r).is_empty(), "{:?}", dead_names(&p, &r));
}

#[test]
fn superclass_reference_keeps_the_base() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "x := Derived()\n\nclass Base {\n}\n\nclass Derived extends Base {\n}\n\nclass Lonely {\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_names(&p, &r), names(&["Lonely"]));
}

#[test]
fn catch_error_type_keeps_the_class() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "try {\n    Risky()\n} catch MyError as e {\n}\n\nclass MyError extends Error {\n}\n\nRisky() {\n}\n",
    );
    let (p, r) = run(&main);
    assert!(dead_names(&p, &r).is_empty(), "{:?}", dead_names(&p, &r));
}

#[test]
fn dynamic_call_blows_up_and_keeps_everything() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "name := \"Foo\"\n%name%()\n\nFoo() {\n}\n\nBar() {\n}\n",
    );
    let (p, r) = run(&main);
    // The dynamic call defeats name resolution, so both functions are conservatively kept.
    assert!(dead_names(&p, &r).is_empty(), "{:?}", dead_names(&p, &r));
}

#[test]
fn selective_import_drops_the_unreferenced_export() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Lib {Used}\nUsed()\n");
    write(
        tmp.path(),
        "Lib.ahk",
        "export Used() {\n}\n\nexport Unused() {\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_names(&p, &r), names(&["Unused"]));
    assert!(r.dropped_imports.is_empty(), "the import is used");
}

#[test]
fn path_qualified_import_keeps_only_referenced_submodule_export() {
    // A path-qualified import binds a sub-module's export. Reachability must resolve against
    // that *sub-module's* declarations (not the group's primary `__Main`): the referenced
    // export survives and its unreferenced sibling shakes out.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import \"Thing:Inner\" {Q}\nQ()\n");
    write(
        tmp.path(),
        "Thing.ahk",
        "P := 1\n#Module Inner\nexport Q() {\n}\n\nexport Z() {\n}\n",
    );
    let (p, r) = run(&main);
    assert_eq!(dead_names(&p, &r), names(&["Z"]));
    assert!(r.dropped_imports.is_empty(), "the import is used");
}

#[test]
fn reexport_chain_is_kept_not_trimmed() {
    // A imports a barrel B (`#Import export ...`) that re-exports from C, which itself
    // re-exports a sibling's named export. Even though nothing in B or C *references* those
    // names locally, the re-exports are public surface and must survive — both the re-export
    // import directives (never dropped) and the re-exported target declarations (kept live).
    let tmp = tempfile::tempdir().unwrap();
    let main = write(
        tmp.path(),
        "main.ahk",
        "#Import Barrel as Lib\nLib.Thing()\n",
    );
    write(tmp.path(), "Barrel.ahk", "#Import export Core {*}\n");
    write(tmp.path(), "Core.ahk", "#Import export Thing\n");
    write(
        tmp.path(),
        "Thing.ahk",
        "export Thing() {\n    return Helper()\n}\nHelper() {\n}\n",
    );

    let (p, r) = run(&main);

    // The re-exported `Thing` (and its transitive `Helper`) survive.
    assert!(
        dead_names(&p, &r).is_empty(),
        "dead: {:?}",
        dead_names(&p, &r)
    );
    // The re-export directives are never dropped (the chain must hold at runtime).
    assert!(
        r.dropped_imports.is_empty(),
        "re-exports must not be dropped: {:?}",
        r.dropped_imports
    );
    // No module is removed — every link in the barrel chain is needed.
    assert!(
        r.dead_modules.is_empty(),
        "dead modules: {:?}",
        r.dead_modules
    );
}

#[test]
fn reference_inside_parentheses_keeps_its_import() {
    // A reference that only appears inside parentheses (here a `!(x is Query)` type check)
    // must still mark its import used. Parenthesized expressions previously lowered to Opaque
    // and hid the reference, wrongly dropping `#Import Query`.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import QC {Check}\nCheck(0)\n");
    write(
        tmp.path(),
        "QC.ahk",
        "#Import Query\nexport Check(o) {\n    if (!(o is Query))\n        return 0\n    return 1\n}\n",
    );
    write(tmp.path(), "Query.ahk", "export default class Query {\n}\n");

    let (p, r) = run(&main);
    assert!(
        r.dropped_imports.is_empty(),
        "`#Import Query` is used inside parens and must be kept: {:?}",
        r.dropped_imports
    );
    assert!(r.dead_modules.is_empty(), "Query module must survive");
    assert!(
        dead_names(&p, &r).is_empty(),
        "dead: {:?}",
        dead_names(&p, &r)
    );
}

#[test]
fn unused_import_does_not_load_its_module_even_with_static_new() {
    // The regression this whole change targets: a module reached only through an *unused*
    // import must not be loaded, so its `static __New` class never becomes a root. Importing
    // is the "use"; without it, the target's body never runs and the module shakes out whole.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Lib {Thing}\nx := 1\n");
    write(
        tmp.path(),
        "Lib.ahk",
        "export class Thing {\n}\n\nclass Boot {\n    static __New() {\n    }\n}\n",
    );
    let (p, r) = run(&main);

    // `Thing` is never referenced -> Lib is not loaded -> Boot's static __New is not a root.
    assert_eq!(r.dropped_imports.len(), 1, "the unused import is dropped");
    assert_eq!(
        r.dead_modules.len(),
        1,
        "Lib is removed despite its static __New class"
    );
    // Sanity: the dropped node is the import directive, not Boot.
    assert!(matches!(
        p.arena[r.dropped_imports[0]].kind,
        NodeKind::ImportDirective(_)
    ));
}

#[test]
fn used_import_loads_module_and_its_static_new_runs() {
    // The flip side: once the import is taken, the whole target body runs, so a sibling
    // `static __New` class in the loaded module is kept even though nothing references it.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Lib {Thing}\nThing()\n");
    write(
        tmp.path(),
        "Lib.ahk",
        "export Thing() {\n}\n\nclass Boot {\n    static __New() {\n    }\n}\n",
    );
    let (p, r) = run(&main);

    assert!(r.dropped_imports.is_empty(), "the import is used");
    assert!(r.dead_modules.is_empty(), "Lib is loaded");
    // Boot stays live via its static __New now that Lib's body runs.
    assert!(
        dead_names(&p, &r).is_empty(),
        "dead: {:?}",
        dead_names(&p, &r)
    );
}

#[test]
fn side_effect_import_loads_its_module() {
    // A pure side-effect import (`#Import "path"`, no binding) binds nothing but still runs
    // the target's body, so it must load the module and is never dropped.
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import \"side.ahk\"\nx := 1\n");
    write(
        tmp.path(),
        "side.ahk",
        "class Boot {\n    static __New() {\n    }\n}\n",
    );
    let (p, r) = run(&main);

    assert!(
        r.dropped_imports.is_empty(),
        "a side-effect import is never dropped: {:?}",
        r.dropped_imports
    );
    assert!(
        r.dead_modules.is_empty(),
        "side.ahk must stay loaded: {:?}",
        r.dead_modules
    );
    assert!(
        dead_names(&p, &r).is_empty(),
        "dead: {:?}",
        dead_names(&p, &r)
    );
}

#[test]
fn unreferenced_import_is_dropped_and_its_module_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let main = write(tmp.path(), "main.ahk", "#Import Lib\nx := 1\n");
    write(tmp.path(), "Lib.ahk", "export Used() {\n}\n");
    let (p, r) = run(&main);

    // `Lib` is never referenced -> the import is dropped and the whole module is dead.
    assert_eq!(r.dropped_imports.len(), 1, "the unused import is dropped");
    assert!(
        matches!(
            p.arena[r.dropped_imports[0]].kind,
            NodeKind::ImportDirective(_)
        ),
        "dropped node is an import directive"
    );
    assert_eq!(r.dead_modules.len(), 1, "Lib module is removed");
}
