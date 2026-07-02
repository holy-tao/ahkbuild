//! Offline restore tests using a local `path` dependency, so they never touch the network or the
//! shared `~/.ahkbuild` store.

use ahkbuild_pkg::{modules_dir, restore, RestoreOptions};

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

#[test]
fn path_dependency_materializes_link_farm() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // A local dependency exposing a module file.
    write(&root.join("dep").join("Widget.ahk"), "#Module Widget\n");

    write(
        &root.join("ahkbuild.json"),
        r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "dependencies": {"Widget": {"path": "dep"}}
        }"#,
    );

    let config = ahkbuild_config::load(&root.join("ahkbuild.json")).unwrap();
    let report = restore(&config, root, RestoreOptions::default()).unwrap();
    assert_eq!(report.restored, 1);
    assert_eq!(report.fetched, 0);

    // The module file resolves through the link-farm under its logical name.
    let linked = modules_dir(root).join("Widget").join("Widget.ahk");
    assert!(
        linked.is_file(),
        "{} should resolve through the link-farm",
        linked.display()
    );

    // A pure `path` project stays lockfile-free and the generated tree is gitignored.
    assert!(!root.join("ahkbuild.lock").exists());
    assert!(root.join(".ahkbuild").join(".gitignore").exists());
}

#[test]
fn subdir_selects_the_module_root() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // The module lives in a sub-directory of the dependency tree.
    write(
        &root.join("dep").join("src").join("Widget.ahk"),
        "#Module Widget\n",
    );
    write(&root.join("dep").join("README.md"), "ignore me\n");

    write(
        &root.join("ahkbuild.json"),
        r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "dependencies": {"Widget": {"path": "dep", "subdir": "src"}}
        }"#,
    );

    let config = ahkbuild_config::load(&root.join("ahkbuild.json")).unwrap();
    restore(&config, root, RestoreOptions::default()).unwrap();

    let linked = modules_dir(root).join("Widget").join("Widget.ahk");
    assert!(linked.is_file(), "{} should resolve", linked.display());
    // The README, above the subdir, is not exposed under the logical name.
    assert!(!modules_dir(root).join("Widget").join("README.md").exists());
}

#[test]
fn alias_links_under_the_import_name() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // A package whose canonical name (`yaml.ahk`) is not a valid AHK identifier.
    write(&root.join("dep").join("yaml.ahk"), "#Module yaml\n");

    write(
        &root.join("ahkbuild.json"),
        r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "dependencies": {"yaml.ahk": {"path": "dep", "alias": "yaml"}}
        }"#,
    );

    let config = ahkbuild_config::load(&root.join("ahkbuild.json")).unwrap();
    restore(&config, root, RestoreOptions::default()).unwrap();

    // The farm exposes the tree under the alias, not the `yaml.ahk` key, so `#Import yaml` resolves.
    let linked = modules_dir(root).join("yaml").join("yaml.ahk");
    assert!(linked.is_file(), "{} should resolve", linked.display());
    assert!(!modules_dir(root).join("yaml.ahk").exists());
}

#[test]
fn restore_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(&root.join("dep").join("Widget.ahk"), "#Module Widget\n");
    write(
        &root.join("ahkbuild.json"),
        r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "dependencies": {"Widget": {"path": "dep"}}
        }"#,
    );
    let config = ahkbuild_config::load(&root.join("ahkbuild.json")).unwrap();

    restore(&config, root, RestoreOptions::default()).unwrap();
    // A second restore rebuilds the farm cleanly rather than erroring on the existing link.
    let report = restore(&config, root, RestoreOptions::default()).unwrap();
    assert_eq!(report.restored, 1);
    let linked = modules_dir(root).join("Widget").join("Widget.ahk");
    assert!(linked.is_file());
}
