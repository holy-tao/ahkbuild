//! Offline tests for `package list` and `package update`, using local `path` dependencies so they
//! never touch the network or the shared `~/.ahkbuild` store.

use ahkbuild_pkg::{list, restore, update, RestoreOptions};

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn project(root: &std::path::Path) -> ahkbuild_config::BuildConfig {
    write(&root.join("dep").join("Widget.ahk"), "#Module Widget\n");
    write(
        &root.join("ahkbuild.json"),
        r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "dependencies": {"Widget": {"path": "dep", "alias": "W"}}
        }"#,
    );
    ahkbuild_config::load(&root.join("ahkbuild.json")).unwrap()
}

#[test]
fn list_reports_a_path_dependency() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config = project(root);
    restore(&config, root, RestoreOptions::default()).unwrap();

    let statuses = list(&config, root).unwrap();
    assert_eq!(statuses.len(), 1);
    let s = &statuses[0];
    assert_eq!(s.name, "Widget");
    assert_eq!(s.import_name, "W"); // linked and imported under the alias
    assert!(s.local);
    assert_eq!(s.resolved, None); // path deps are never locked
    assert!(s.present, "the path source exists on disk");
    assert!(s.linked, "restore linked it into the farm");
    assert!(s.source.starts_with("path "));
}

#[test]
fn update_all_skips_nothing_and_changes_nothing_for_path_only() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config = project(root);

    // Updating everything: a path dep is not updatable but was not named, so it is silently ignored.
    let report = update(&config, root, &[]).unwrap();
    assert!(report.changes.is_empty());
    assert!(report.skipped.is_empty());
    // The driver still (re)builds the farm.
    assert!(ahkbuild_pkg::modules_dir(root).join("W").exists());
}

#[test]
fn update_named_path_dependency_is_reported_as_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config = project(root);

    let report = update(&config, root, &["Widget".to_string()]).unwrap();
    assert!(report.changes.is_empty());
    assert_eq!(report.skipped, vec!["Widget".to_string()]);
}

#[test]
fn update_unknown_dependency_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let config = project(root);

    let err = update(&config, root, &["Nope".to_string()]).unwrap_err();
    assert!(err.to_string().contains("Nope"), "{err}");
}
