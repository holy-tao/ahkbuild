//! In-place edits to `ahkbuild.json` for `ahkbuild package add` / `remove`.
//!
//! Edits go through a `serde_json::Value` round-trip (with the `preserve_order` feature) so only the
//! `dependencies` table changes and the rest of the user's file - key order, formatting choices we
//! can preserve - is left as-is. Every added dependency is validated by deserializing it as a
//! [`DependencySpec`], so the same manifest rules that `load` enforces reject a bad entry here too.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

use crate::DependencySpec;

/// Add dependency `name` with the given source object (e.g. `{"git": "...", "tag": "v1"}`) to the
/// `dependencies` table of the manifest at `path`. Fails if `name` already exists or if the source
/// object is not a valid dependency spec.
pub fn add_dependency(path: &Path, name: &str, spec: Value) -> Result<()> {
    // Validate the source object against the real manifest rules before touching the file.
    serde_json::from_value::<DependencySpec>(spec.clone())
        .with_context(|| format!("invalid dependency spec for {name:?}"))?;

    let mut root = read_object(path)?;
    let deps = dependencies_mut(&mut root);
    if deps.contains_key(name) {
        bail!(
            "dependency {name:?} already exists in {}\n\
             hint: remove it first (`ahkbuild package remove {name}`) or edit ahkbuild.json directly",
            path.display()
        );
    }
    deps.insert(name.to_string(), spec);
    write_object(path, &root)
}

/// Remove dependency `name` from the manifest at `path`. Returns whether it was present.
pub fn remove_dependency(path: &Path, name: &str) -> Result<bool> {
    let mut root = read_object(path)?;
    let existed = dependencies_mut(&mut root).remove(name).is_some();
    if existed {
        write_object(path, &root)?;
    }
    Ok(existed)
}

fn read_object(path: &Path) -> Result<Map<String, Value>> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    match serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))? {
        Value::Object(map) => Ok(map),
        _ => bail!("{} is not a JSON object", path.display()),
    }
}

/// Borrow the `dependencies` object, inserting an empty one if the manifest has none yet.
fn dependencies_mut(root: &mut Map<String, Value>) -> &mut Map<String, Value> {
    root.entry("dependencies")
        .or_insert_with(|| Value::Object(Map::new()));
    // `or_insert_with` guarantees the key exists; coerce it to an object (replacing a non-object, so
    // a malformed manifest cannot wedge the edit).
    let slot = root.get_mut("dependencies").expect("just inserted");
    if !slot.is_object() {
        *slot = Value::Object(Map::new());
    }
    slot.as_object_mut().expect("coerced to object")
}

fn write_object(path: &Path, root: &Map<String, Value>) -> Result<()> {
    let mut s = serde_json::to_string_pretty(&Value::Object(root.clone()))
        .context("serializing ahkbuild.json")?;
    s.push('\n');
    std::fs::write(path, s).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DependencySource, GitSelector};

    fn write(path: &Path, contents: &str) {
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn add_appends_and_preserves_key_order() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ahkbuild.json");
        write(
            &path,
            r#"{
  "entry": "main.ahk",
  "interpreter": {
    "version": "2.1-alpha.27"
  },
  "dependencies": {
    "Existing": {
      "git": "https://github.com/x/existing.git"
    }
  }
}"#,
        );

        let spec = serde_json::json!({"git": "https://github.com/x/y.git", "tag": "v1.0.0"});
        add_dependency(&path, "Widget", spec).unwrap();

        // The added dependency parses, and the top-level order (entry before interpreter) is intact.
        let cfg = crate::load(&path).unwrap();
        assert_eq!(cfg.dependencies.len(), 2);
        assert!(matches!(
            &cfg.dependencies["Widget"].source,
            DependencySource::Git { selector: GitSelector::Tag(t), .. } if t == "v1.0.0"
        ));
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.find("\"entry\"").unwrap() < raw.find("\"interpreter\"").unwrap());
    }

    #[test]
    fn add_creates_dependencies_table_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ahkbuild.json");
        write(&path, r#"{"interpreter": {"version": "2.1-alpha.27"}}"#);

        add_dependency(&path, "Local", serde_json::json!({"path": "vendor/local"})).unwrap();
        let cfg = crate::load(&path).unwrap();
        assert!(matches!(
            &cfg.dependencies["Local"].source,
            DependencySource::Path { .. }
        ));
    }

    #[test]
    fn add_rejects_a_duplicate_name() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ahkbuild.json");
        write(
            &path,
            r#"{"interpreter": {"version": "2.1-alpha.27"},
                "dependencies": {"Widget": {"git": "u"}}}"#,
        );
        let err = add_dependency(&path, "Widget", serde_json::json!({"git": "u2"})).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    #[test]
    fn add_rejects_an_invalid_spec() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ahkbuild.json");
        write(&path, r#"{"interpreter": {"version": "2.1-alpha.27"}}"#);
        // Two conflicting sources: the manifest deserializer must reject it, leaving the file untouched.
        let err =
            add_dependency(&path, "X", serde_json::json!({"git": "u", "path": "p"})).unwrap_err();
        assert!(err.to_string().contains("invalid dependency spec"), "{err}");
        let cfg = crate::load(&path).unwrap();
        assert!(cfg.dependencies.is_empty());
    }

    #[test]
    fn remove_reports_presence() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ahkbuild.json");
        write(
            &path,
            r#"{"interpreter": {"version": "2.1-alpha.27"},
                "dependencies": {"Widget": {"git": "u"}}}"#,
        );
        assert!(remove_dependency(&path, "Widget").unwrap());
        assert!(!remove_dependency(&path, "Widget").unwrap()); // already gone
        let cfg = crate::load(&path).unwrap();
        assert!(cfg.dependencies.is_empty());
    }
}
