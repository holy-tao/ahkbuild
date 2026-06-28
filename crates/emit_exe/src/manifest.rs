//! Surgical edits to the interpreter's application manifest (`RT_MANIFEST`).
//!
//! Like the version-info path, this **never regenerates the manifest from scratch**: the
//! interpreter's shipped manifest carries the `comctl32 v6` dependency and `supportedOS`/DPI
//! declarations the runtime relies on, so we read its bytes (read-only, via `editpe`), apply only
//! the elements the config sets, and write the result back via Win32 `UpdateResource`. The edits
//! are minimal string surgery rather than a DOM round-trip, for the same reason we avoid editpe's
//! PE rebuild: a reformat risks dropping/reordering something the runtime depends on.

use std::path::Path;

use anyhow::{Context, Result};

use ahkbuild_config::{BuildConfig, ManifestConfig};

/// Build the edited manifest bytes from the interpreter's manifest overlaid with `config.exe.manifest`.
///
/// Returns `Ok(None)` when no manifest override is configured (the emitter then leaves the
/// interpreter's manifest untouched) or when the interpreter has no manifest to edit.
pub fn build_manifest_bytes(interpreter: &Path, config: &BuildConfig) -> Result<Option<Vec<u8>>> {
    let m: &ManifestConfig = &config.exe.manifest;
    if m.is_empty() {
        return Ok(None);
    }

    let Some(raw) = read_manifest(interpreter)? else {
        anyhow::bail!(
            "interpreter {} has no application manifest to edit",
            interpreter.display()
        );
    };
    let mut xml = String::from_utf8(raw).context("interpreter manifest is not valid UTF-8")?;

    if let Some(uac) = m.uac {
        set_uac_level(&mut xml, uac.as_str()).context("setting manifest UAC level")?;
    }
    if let Some(v) = m.dpi_aware {
        set_windows_setting(&mut xml, "dpiAware", bool_str(v)).context("setting dpiAware")?;
    }
    if let Some(v) = &m.dpi_awareness {
        set_windows_setting(&mut xml, "dpiAwareness", v).context("setting dpiAwareness")?;
    }
    if let Some(v) = m.long_path_aware {
        set_windows_setting(&mut xml, "longPathAware", bool_str(v))
            .context("setting longPathAware")?;
    }
    if let Some(v) = m.gdi_scaling {
        set_windows_setting(&mut xml, "gdiScaling", bool_str(v)).context("setting gdiScaling")?;
    }

    Ok(Some(xml.into_bytes()))
}

fn bool_str(b: bool) -> &'static str {
    if b {
        "true"
    } else {
        "false"
    }
}

/// Read the raw `RT_MANIFEST` (type 24, name id 1) resource bytes from the interpreter PE, using
/// `editpe` purely as a reader. Returns `None` if the PE has no manifest resource.
fn read_manifest(interpreter: &Path) -> Result<Option<Vec<u8>>> {
    use editpe::{ResourceEntry, ResourceEntryName};

    const RT_MANIFEST: u32 = 24;

    let image = editpe::Image::parse_file(interpreter)
        .with_context(|| format!("reading interpreter PE {}", interpreter.display()))?;
    let Some(res) = image.resource_directory() else {
        return Ok(None);
    };

    let root = res.root();
    let Some(ResourceEntry::Table(by_name)) = root.get(ResourceEntryName::ID(RT_MANIFEST)) else {
        return Ok(None);
    };
    // First (and, for an exe, only) manifest -> its language sub-table -> the data blob.
    let Some(name) = by_name.entries().first().map(|n| (*n).clone()) else {
        return Ok(None);
    };
    let Some(ResourceEntry::Table(by_lang)) = by_name.get(name) else {
        return Ok(None);
    };
    let Some(lang) = by_lang.entries().first().map(|n| (*n).clone()) else {
        return Ok(None);
    };
    let Some(ResourceEntry::Data(data)) = by_lang.get(lang) else {
        return Ok(None);
    };
    Ok(Some(data.data().to_vec()))
}

// ---------------------------------------------------------------------------
// Minimal, namespace-aware XML surgery
//
// The manifest is small, well-formed, ASCII XML we already know the shape of. These helpers match
// elements by *local* name (ignoring any `prefix:`) and never quote `>` inside attribute values,
// which the manifest never does. They are deliberately not a general XML editor.
// ---------------------------------------------------------------------------

/// Set the `level` attribute of the `requestedExecutionLevel` element. Errors if the element is
/// missing (every AHK interpreter ships one, so its absence means an unexpected manifest).
fn set_uac_level(xml: &mut String, level: &str) -> Result<()> {
    let tag = find_open_tag(xml, "requestedExecutionLevel")
        .context("manifest has no <requestedExecutionLevel> element")?;
    set_attr(xml, &tag, "level", level);
    Ok(())
}

/// Set a `<windowsSettings>` child element (`dpiAware`, `dpiAwareness`, `longPathAware`,
/// `gdiScaling`) to `value`, replacing its text if it already exists or inserting it into the
/// `windowsSettings` element otherwise. Errors if the manifest has no `windowsSettings` element.
fn set_windows_setting(xml: &mut String, element: &str, value: &str) -> Result<()> {
    if let Some(tag) = find_open_tag(xml, element) {
        set_text(xml, &tag, element, value);
        Ok(())
    } else {
        let container = find_open_tag(xml, "windowsSettings").with_context(|| {
            format!("manifest has no <windowsSettings> element to add <{element}> to")
        })?;
        let close = find_close_tag(xml, "windowsSettings", container.gt + 1)
            .context("manifest <windowsSettings> element is not closed")?;
        // A newly inserted element inherits <windowsSettings>'s default namespace (the 2016
        // WindowsSettings ns). Elements that live in a *different* namespace must declare it
        // explicitly, or SxS rejects them as "not registered".
        let fragment = match windows_setting_ns(element) {
            Some(ns) => format!(
                r#"<{element} xmlns="{ns}">{}</{element}>"#,
                escape_text(value)
            ),
            None => format!("<{element}>{}</{element}>", escape_text(value)),
        };
        xml.insert_str(close, &fragment);
        Ok(())
    }
}

/// The XML namespace a `<windowsSettings>` child must declare when it is *not* the container's
/// default (2016 WindowsSettings) namespace. Returns `None` for elements that belong to the 2016
/// namespace and so need no explicit `xmlns` on insertion.
fn windows_setting_ns(element: &str) -> Option<&'static str> {
    match element {
        "dpiAware" => Some("http://schemas.microsoft.com/SMI/2005/WindowsSettings"),
        "gdiScaling" => Some("http://schemas.microsoft.com/SMI/2017/WindowsSettings"),
        // dpiAwareness and longPathAware are in the default (2016) namespace.
        _ => None,
    }
}

/// A located element open tag: `<` at `lt`, name ends at `name_end`, `>` at `gt`.
struct OpenTag {
    lt: usize,
    name_end: usize,
    gt: usize,
    self_closing: bool,
}

/// Find the first element whose *local* name (after any `prefix:`) equals `local`. Skips comments,
/// processing instructions, and close tags.
fn find_open_tag(xml: &str, local: &str) -> Option<OpenTag> {
    let mut i = 0;
    while let Some(rel) = xml[i..].find('<') {
        let lt = i + rel;
        let next = xml[lt + 1..].chars().next();
        if matches!(next, Some('/') | Some('!') | Some('?') | None) {
            i = lt + 1;
            continue;
        }
        if let Some(name_end) = matched_name_end(xml, lt + 1, local) {
            let gt = name_end + xml[name_end..].find('>')?;
            let self_closing = xml.as_bytes()[gt - 1] == b'/';
            return Some(OpenTag {
                lt,
                name_end,
                gt,
                self_closing,
            });
        }
        i = lt + 1;
    }
    None
}

/// If the tag name starting at `start` has local name `local`, return the index just past the
/// qualified name; otherwise `None`. The name ends at whitespace, `>`, or `/`.
fn matched_name_end(xml: &str, start: usize, local: &str) -> Option<usize> {
    let rel = xml[start..].find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
    let end = start + rel;
    let qname = &xml[start..end];
    let name = qname.rsplit(':').next().unwrap_or(qname);
    (name == local).then_some(end)
}

/// Find the close tag `</...local>` at or after `from`, returning the byte index of its `<`.
fn find_close_tag(xml: &str, local: &str, from: usize) -> Option<usize> {
    let mut i = from;
    while let Some(rel) = xml[i..].find("</") {
        let lt = i + rel;
        let name_start = lt + 2;
        let rel2 = xml[name_start..].find(|c: char| c.is_whitespace() || c == '>')?;
        let name_end = name_start + rel2;
        let qname = &xml[name_start..name_end];
        let name = qname.rsplit(':').next().unwrap_or(qname);
        if name == local {
            return Some(lt);
        }
        i = lt + 2;
    }
    None
}

/// Replace (or insert) an attribute's value within an element's open tag.
fn set_attr(xml: &mut String, tag: &OpenTag, attr: &str, value: &str) {
    let open = &xml[tag.name_end..tag.gt];
    // Look for ` attr="` (a leading boundary char avoids matching a longer attribute name).
    if let Some(pos) = find_attr_value(open, attr) {
        let val_start = tag.name_end + pos;
        let val_end = val_start + xml[val_start..].find('"').unwrap_or(0);
        xml.replace_range(val_start..val_end, &escape_attr(value));
    } else {
        // Insert before the `>` (or `/>`) of the open tag.
        let insert_at = if tag.self_closing { tag.gt - 1 } else { tag.gt };
        let frag = format!(" {attr}=\"{}\"", escape_attr(value));
        xml.insert_str(insert_at, &frag);
    }
}

/// Within an open tag's text, find the byte offset of an attribute's value (just past the opening
/// quote). Matches `attr="`, requiring a non-name boundary before `attr` so `level` doesn't match
/// inside `uiAccessLevel`.
fn find_attr_value(open: &str, attr: &str) -> Option<usize> {
    let needle = format!("{attr}=\"");
    let mut i = 0;
    while let Some(rel) = open[i..].find(&needle) {
        let at = i + rel;
        let prev_ok = at == 0
            || open[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace());
        if prev_ok {
            return Some(at + needle.len());
        }
        i = at + 1;
    }
    None
}

/// Set an element's text content, converting a self-closing tag to an open/close pair if needed.
fn set_text(xml: &mut String, tag: &OpenTag, local: &str, value: &str) {
    let escaped = escape_text(value);
    if tag.self_closing {
        // `<E .../>` -> `<E ...>value</E>`. Drop the `/`, then append text + close tag.
        let close = format!(">{escaped}</{}>", &xml[tag.lt + 1..tag.name_end]);
        // tag.gt-1 is the `/`; replace `/>` (gt-1..=gt) with the new tail.
        xml.replace_range(tag.gt - 1..tag.gt + 1, &close);
    } else if let Some(close) = find_close_tag(xml, local, tag.gt + 1) {
        xml.replace_range(tag.gt + 1..close, &escaped);
    }
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed manifest that mirrors the AHK interpreter's shape (prefixed v3:, child default ns).
    const MANIFEST: &str = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0" xmlns:v3="urn:schemas-microsoft-com:asm.v3">"#,
        r#"<v3:application><v3:windowsSettings xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">"#,
        r#"<dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true</dpiAware>"#,
        r#"<dpiAwareness>PerMonitorV2</dpiAwareness>"#,
        r#"<longPathAware>true</longPathAware>"#,
        r#"</v3:windowsSettings></v3:application>"#,
        r#"<v3:trustInfo><v3:security><v3:requestedPrivileges>"#,
        r#"<v3:requestedExecutionLevel level="asInvoker" uiAccess="false" />"#,
        r#"</v3:requestedPrivileges></v3:security></v3:trustInfo></assembly>"#,
    );

    #[test]
    fn sets_uac_level() {
        let mut xml = MANIFEST.to_string();
        set_uac_level(&mut xml, "requireAdministrator").unwrap();
        assert!(xml.contains(r#"level="requireAdministrator""#));
        assert!(!xml.contains(r#"level="asInvoker""#));
        // Other attributes survive.
        assert!(xml.contains(r#"uiAccess="false""#));
    }

    #[test]
    fn replaces_existing_setting_text() {
        let mut xml = MANIFEST.to_string();
        set_windows_setting(&mut xml, "dpiAwareness", "system").unwrap();
        assert!(xml.contains("<dpiAwareness>system</dpiAwareness>"));
        assert!(!xml.contains("PerMonitorV2"));
    }

    #[test]
    fn dpi_aware_match_does_not_clobber_dpi_awareness() {
        let mut xml = MANIFEST.to_string();
        set_windows_setting(&mut xml, "dpiAware", "false").unwrap();
        // dpiAware updated, dpiAwareness untouched (prefix collision avoided).
        assert!(xml.contains(">false</dpiAware>"));
        assert!(xml.contains("<dpiAwareness>PerMonitorV2</dpiAwareness>"));
    }

    #[test]
    fn inserts_missing_setting_into_windows_settings() {
        let mut xml = MANIFEST.to_string();
        assert!(!xml.contains("gdiScaling"));
        set_windows_setting(&mut xml, "gdiScaling", "true").unwrap();
        // gdiScaling lives in the 2017 namespace, not the container's default 2016 ns; it must
        // declare it explicitly or SxS rejects the manifest as "gdiScaling is not registered".
        assert!(xml.contains(
            r#"<gdiScaling xmlns="http://schemas.microsoft.com/SMI/2017/WindowsSettings">true</gdiScaling>"#
        ));
        // Inserted before the closing windowsSettings tag.
        let g = xml.find("gdiScaling").unwrap();
        let close = xml.find("</v3:windowsSettings>").unwrap();
        assert!(g < close);
    }

    #[test]
    fn converts_self_closing_setting() {
        let mut xml = r#"<v3:windowsSettings><longPathAware/></v3:windowsSettings>"#.to_string();
        set_windows_setting(&mut xml, "longPathAware", "true").unwrap();
        assert!(xml.contains("<longPathAware>true</longPathAware>"));
    }
}
