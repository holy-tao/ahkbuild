//! Arbitrary resource embedding for the `.exe` backend (config `resources.extra`).
//!
//! Analogous to Ahk2Exe's `;@Ahk2Exe-AddResource` directive, but driven by the project's
//! `ahkbuild.json` rather than source comments: each entry names a file on disk, a resource type,
//! and the name to file it under.
//!
//! Icons still go through `exe.icon` (the `RT_GROUP_ICON` + `RT_ICON` split in `icon.rs`); a raw
//! `.ico` filed directly under `RT_GROUP_ICON` would be a malformed group, so that is rejected here.

use std::collections::HashSet;

use anyhow::{bail, Context, Result};

use ahkbuild_config::{BuildConfig, ResourceType};

const RT_ICON: u16 = 3;
const RT_RCDATA: u16 = 10;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;

/// A resource name: an integer id (`#N` in config) or a string name (always uppercased, matching
/// Ahk2Exe and the case-insensitive Win32 `FindResource` lookup).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResName {
    Id(u16),
    Name(String),
}

/// A resolved extra resource ready to inject via `UpdateResource`.
#[derive(Debug, Clone)]
pub struct EmbeddedResource {
    /// Numeric resource type (`RT_*`).
    pub type_id: u16,
    /// Resource name (integer id or uppercased string).
    pub name: ResName,
    /// Raw file bytes to embed.
    pub data: Vec<u8>,
}

/// Resolve and read every `resources.extra` entry. Paths are already absolute (resolved against the
/// config dir at load time).
///
/// `reserved_rcdata` is the set of uppercased `RT_RCDATA` string names the emitter already owns -
/// the embedded script modules and `FileInstall` files - so an extra resource cannot silently
/// clobber one (extras are written last, so the collision would otherwise win unnoticed).
///
/// Errors on an unknown type name, an icon type (steer to `exe.icon`), a collision with a resource
/// the emitter itself owns, a duplicate `(type, name)` pair, or an unreadable file.
pub fn collect(
    config: &BuildConfig,
    reserved_rcdata: &HashSet<String>,
) -> Result<Vec<EmbeddedResource>> {
    let mut out: Vec<EmbeddedResource> = Vec::new();
    for entry in &config.resources.extra {
        let type_id = resolve_type(&entry.resource_type)?;
        // Icons need the group/image split (see icon.rs); raw `.ico` bytes filed as RT_GROUP_ICON
        // (or RT_ICON) would be a broken resource.
        if type_id == RT_GROUP_ICON || type_id == RT_ICON {
            bail!(
                "resource {:?}: icon embedding via `resources.extra` is not supported - use \
                 `exe.icon` to set the application icon",
                entry.name
            );
        }

        let name = resolve_name(&entry.name);
        // Guard the resources the module/version emitter writes itself.
        if type_id == RT_RCDATA {
            if name == ResName::Id(1) {
                bail!(
                    "resource {:?}: RT_RCDATA #1 is reserved for the entry module",
                    entry.name
                );
            }
            if let ResName::Name(n) = &name {
                if reserved_rcdata.contains(n) {
                    bail!(
                        "resource {:?}: RT_RCDATA name {n:?} collides with an embedded script \
                         module or FileInstall file",
                        entry.name
                    );
                }
            }
        }
        if type_id == RT_VERSION {
            bail!(
                "resource {:?}: embed version info via `exe.version`/`exe.name`/etc., not \
                 `resources.extra`",
                entry.name
            );
        }
        if out.iter().any(|r| r.type_id == type_id && r.name == name) {
            bail!("duplicate resource: type {type_id}, name {name:?}");
        }

        let data = std::fs::read(&entry.path)
            .with_context(|| format!("reading resource file {}", entry.path.display()))?;
        out.push(EmbeddedResource {
            type_id,
            name,
            data,
        });
    }
    Ok(out)
}

/// Resolve a config resource type to its numeric id.
fn resolve_type(ty: &ResourceType) -> Result<u16> {
    match ty {
        ResourceType::Raw(n) => Ok(*n),
        ResourceType::Named(s) => named_type(s).ok_or_else(|| {
            anyhow::anyhow!("unknown resource type {s:?} (use a numeric id or a known RT_* name)")
        }),
    }
}

/// Map a standard `RT_*` resource-type name (case-insensitive, `RT_` prefix optional) to its id.
fn named_type(s: &str) -> Option<u16> {
    let up = s.trim().to_ascii_uppercase();
    let key = up.strip_prefix("RT_").unwrap_or(&up);
    Some(match key {
        "CURSOR" => 1,
        "BITMAP" => 2,
        "ICON" => 3,
        "MENU" => 4,
        "DIALOG" => 5,
        "STRING" => 6,
        "FONTDIR" => 7,
        "FONT" => 8,
        "ACCELERATOR" => 9,
        "RCDATA" => 10,
        "MESSAGETABLE" => 11,
        "GROUP_CURSOR" => 12,
        "GROUP_ICON" => 14,
        "VERSION" => 16,
        "DLGINCLUDE" => 17,
        "PLUGPLAY" => 19,
        "VXD" => 20,
        "ANICURSOR" => 21,
        "ANIICON" => 22,
        "HTML" => 23,
        "MANIFEST" => 24,
        _ => return None,
    })
}

/// Resolve a config resource name: `#N` is an integer id, anything else is an uppercased string.
fn resolve_name(name: &str) -> ResName {
    let t = name.trim();
    if let Some(digits) = t.strip_prefix('#') {
        if let Ok(id) = digits.parse::<u16>() {
            return ResName::Id(id);
        }
    }
    ResName::Name(t.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `BuildConfig` whose `resources.extra` array is the given JSON object list.
    fn cfg(extra_json: &str) -> BuildConfig {
        let json = format!(
            r#"{{"interpreter":{{"version":"2.1-alpha.27"}},"resources":{{"extra":[{extra_json}]}}}}"#
        );
        serde_json::from_str(&json).expect("parse config")
    }

    /// An empty reserved-name set (the common case in these unit tests).
    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn maps_named_and_raw_types() {
        assert_eq!(named_type("RT_HTML"), Some(23));
        assert_eq!(named_type("html"), Some(23));
        assert_eq!(named_type("RCDATA"), Some(10));
        assert_eq!(named_type("nope"), None);
        assert_eq!(resolve_type(&ResourceType::Raw(42)).unwrap(), 42);
    }

    #[test]
    fn resolves_names() {
        assert_eq!(resolve_name("#7"), ResName::Id(7));
        assert_eq!(resolve_name("help"), ResName::Name("HELP".into()));
        // A non-numeric `#...` is a string name, not an id.
        assert_eq!(resolve_name("#x"), ResName::Name("#X".into()));
    }

    #[test]
    fn collects_and_reads_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("help.html");
        std::fs::write(&p, b"<html>").unwrap();
        let c = cfg(&format!(
            r#"{{"name":"help","type":"RT_HTML","path":{p:?}}}"#
        ));
        let got = collect(&c, &none()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].type_id, 23);
        assert_eq!(got[0].name, ResName::Name("HELP".into()));
        assert_eq!(got[0].data, b"<html>");
    }

    #[test]
    fn rejects_icon_type() {
        let c = cfg(r#"{"name":"APP","type":"RT_GROUP_ICON","path":"x.ico"}"#);
        let err = collect(&c, &none()).unwrap_err().to_string();
        assert!(err.contains("exe.icon"), "{err}");
    }

    #[test]
    fn rejects_reserved_entry_id() {
        let c = cfg(r##"{"name":"#1","type":"RT_RCDATA","path":"x.bin"}"##);
        let err = collect(&c, &none()).unwrap_err().to_string();
        assert!(err.contains("entry module"), "{err}");
    }

    #[test]
    fn rejects_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.bin");
        std::fs::write(&p, b"x").unwrap();
        // "A" and "a" both uppercase to "A"; 10 and "RCDATA" both resolve to type 10.
        let c = cfg(&format!(
            r#"{{"name":"A","type":10,"path":{p:?}}},{{"name":"a","type":"RCDATA","path":{p:?}}}"#
        ));
        let err = collect(&c, &none()).unwrap_err().to_string();
        assert!(err.contains("duplicate"), "{err}");
    }

    #[test]
    fn rejects_collision_with_embedded_module() {
        // An extra RT_RCDATA whose name matches an embedded script module (or FileInstall file)
        // would clobber it, since extras are written last.
        let c = cfg(r#"{"name":"greeter","type":"RT_RCDATA","path":"x.bin"}"#);
        let reserved = HashSet::from(["GREETER".to_string()]);
        let err = collect(&c, &reserved).unwrap_err().to_string();
        assert!(err.contains("collides"), "{err}");
    }
}
