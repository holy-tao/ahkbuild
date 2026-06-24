//! The `.exe` emit backend: assemble a standalone Windows executable by embedding each AHK module
//! as a Win32 `RT_RCDATA` resource inside a copy of the interpreter binary.
//!
//! This is the PE-aware sibling of [`ahkbuild_emit`]. The text of each module is produced by
//! [`ahkbuild_emit::emit_exe_modules`] (the portable span-edit path, with cross-module `#Import`s
//! already rewritten to `"*<name>"`); this crate handles the binary side: injecting those modules
//! as resources and stamping version metadata.
//!
//! Conventions (AHK v2.1, see `docs/EXE_BUNDLING.md`):
//! - the entry module is `RT_RCDATA` integer id `1`, which the interpreter auto-runs (`*#1`);
//! - every other module is a named `RT_RCDATA` resource loaded via `#Import "*<name>"`.

// NOTE: **PE editing uses the Win32 `UpdateResource` API** (`BeginUpdateResource` /
// `UpdateResource` / `EndUpdateResource`), exactly as Ahk2Exe does. Tried `editpe` for a pure
// Rust approach, hoping that it could be cross-platform, but it corrupts the final executable
// in such a way that it can no longer run scripts. So we only use it to read existing info
// from the interpereter. **This means exe bundling requires windows**

mod fileinstall;
mod icon;
mod resource;

use std::path::Path;

use anyhow::{Context, Result};

use ahkbuild_config::BuildConfig;
use ahkbuild_emit::{emit_exe_modules, EmitModule, EmitOptions, ResourceName, WsLevel};
use ahkbuild_fold::FoldResult;
use ahkbuild_ir::Program;
use ahkbuild_link::BundlePlan;
use ahkbuild_shake::ShakeResult;

use icon::{IconIdAllocator, IconResources};
use resource::{EmbeddedResource, ResName};

/// US-English language id (0x0409). Embedded resources are filed under this language, matching
/// Ahk2Exe; the Win32 resource loader resolves them via the standard fallback.
const LANG_EN_US: u16 = 0x0409;

/// Assemble a standalone `.exe` from a linked, optimized program and the interpreter binary.
///
/// `program` / `plan` / `shake` / `fold` are the converged pipeline outputs (identical to the
/// `.ahk` path); `config` supplies exe metadata (version, name). Copies the interpreter to
/// `output` and injects the script modules and metadata via Win32 `UpdateResource`.
pub fn bundle_exe(
    interpreter: &Path,
    program: &Program,
    plan: &BundlePlan,
    shake: Option<&ShakeResult>,
    fold: Option<&FoldResult>,
    config: &BuildConfig,
    output: &Path,
) -> Result<()> {
    // 1. Render each module to its final (minified, post-fold/shake) source text.
    let options = EmitOptions {
        strip_comments: true,
        whitespace: WsLevel::Minify,
    };
    let modules = emit_exe_modules(program, plan, shake, fold, &options);
    anyhow::ensure!(!modules.is_empty(), "no modules to embed");

    // 2. Build the version-info resource bytes from the interpreter's existing one (read-only use
    //    of editpe) overlaid with the project's metadata.
    let version = build_version_bytes(interpreter, config).context("building version info")?;

    // 3. Detect live `FileInstall` calls and read each file's bytes for embedding.
    let installs = fileinstall::scan(program, shake, fold).context("scanning FileInstall calls")?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(installs.len());
    for fi in &installs {
        let data = std::fs::read(&fi.source).with_context(|| {
            format!(
                "reading FileInstall source {} (embedded as resource {})",
                fi.source.display(),
                fi.name
            )
        })?;
        files.push((fi.name.clone(), data));
    }

    // 4. Build the icon resources: `exe.icon` replaces the interpreter's primary group, and each
    //    `resources.icons` entry adds a new group under its explicit id. All share one image-id
    //    allocator so nothing clobbers a built-in (tray/Gui) icon.
    let icons = build_icons(interpreter, config).context("building icons")?;

    // 5. Collect the project's generic extra resources (config `resources.extra`). Pass the
    //    RT_RCDATA names the emitter already owns (modules + FileInstall files) so an extra
    //    resource can't silently clobber one.
    let reserved_rcdata: std::collections::HashSet<String> = modules
        .iter()
        .filter_map(|m| match &m.resource {
            ResourceName::Named(n) => Some(n.clone()),
            ResourceName::Entry => None,
        })
        .chain(files.iter().map(|(name, _)| name.clone()))
        .collect();
    let extras =
        resource::collect(config, &reserved_rcdata).context("collecting extra resources")?;

    // 6. Copy the interpreter to the output path, then inject resources into the copy.
    std::fs::copy(interpreter, output).with_context(|| {
        format!(
            "copying interpreter {} -> {}",
            interpreter.display(),
            output.display()
        )
    })?;

    write_resources(
        output,
        &modules,
        version.as_deref(),
        &files,
        &icons,
        &extras,
    )
}

/// Assemble every icon resource to inject: `exe.icon` (replacing the interpreter's primary group)
/// and each `resources.icons` entry (a new `RT_GROUP_ICON` under its explicit id). All images draw
/// `RT_ICON` ids from one shared [`IconIdAllocator`] minted above the interpreter's highest icon id,
/// so nothing clobbers a built-in (tray/Gui) image. Returns the group writes in injection order.
///
/// Errors if a `resources.icons` id collides with one of the interpreter's built-in icon groups
/// (that would overwrite a tray/Gui icon) or with another configured icon.
fn build_icons(interpreter: &Path, config: &BuildConfig) -> Result<Vec<IconResources>> {
    if config.exe.icon.is_none() && config.resources.icons.is_empty() {
        return Ok(Vec::new());
    }

    let image = editpe::Image::parse_file(interpreter)
        .with_context(|| format!("reading interpreter PE {}", interpreter.display()))?;
    let layout = image
        .resource_directory()
        .and_then(read_icon_layout)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "interpreter {} has no icon group to replace",
                interpreter.display()
            )
        })?;

    let mut alloc = IconIdAllocator::new(layout.max_icon_id);
    let mut out = Vec::new();

    // Primary icon: replace the lowest-numbered group, recycling its member image ids.
    if let Some(ico_path) = config.exe.icon.as_deref() {
        let ico_bytes = std::fs::read(ico_path)
            .with_context(|| format!("reading icon file {}", ico_path.display()))?;
        eprintln!(
            "icon: replacing primary group {} ({} existing image(s))",
            layout.group_id,
            layout.member_ids.len()
        );
        out.push(
            icon::build_icon_resources(&ico_bytes, layout.group_id, &layout.member_ids, &mut alloc)
                .with_context(|| {
                    format!("building primary icon from {}", ico_path.display())
                })?,
        );
    }

    // Additional icons: each becomes a brand-new group under its configured id. The id must miss
    // every built-in group (else we'd overwrite a tray/Gui icon) and every other configured id.
    let builtin: std::collections::HashSet<u16> = layout.group_ids.iter().copied().collect();
    let mut used = std::collections::HashSet::new();
    for ic in &config.resources.icons {
        if builtin.contains(&ic.id) {
            let mut ids = layout.group_ids.clone();
            ids.sort_unstable();
            anyhow::bail!(
                "resources.icons: id {} collides with a built-in interpreter icon group (ids {:?}); \
                 pick an id outside that set",
                ic.id,
                ids
            );
        }
        if !used.insert(ic.id) {
            anyhow::bail!("resources.icons: duplicate icon id {}", ic.id);
        }
        if ic.id < layout.group_id {
            eprintln!(
                "icon: warning: id {} is below the primary group {}; it will become the icon \
                 Windows shows for the .exe, overriding exe.icon",
                ic.id, layout.group_id
            );
        }

        let ico_bytes = std::fs::read(&ic.path)
            .with_context(|| format!("reading icon file {}", ic.path.display()))?;
        eprintln!(
            "icon: adding group {} (load via LoadPicture(A_ScriptFullPath, \"Icon-{}\"))",
            ic.id, ic.id
        );
        out.push(
            icon::build_icon_resources(&ico_bytes, ic.id, &[], &mut alloc)
                .with_context(|| format!("building icon from {}", ic.path.display()))?,
        );
    }

    Ok(out)
}

/// The interpreter's icon layout: the primary group to replace, its member `RT_ICON` ids (to
/// reuse), every existing `RT_GROUP_ICON` id (so an additional icon can't clobber a built-in
/// group), and the highest `RT_ICON` id in the PE (to mint fresh image ids above).
struct IconLayout {
    group_id: u16,
    member_ids: Vec<u16>,
    group_ids: Vec<u16>,
    max_icon_id: u16,
}

/// Read the [`IconLayout`] from an interpreter's resource directory. The primary group is the
/// lowest-numbered `RT_GROUP_ICON` id (the one Windows shows as the application icon). Returns
/// `None` if the PE has no group-icon resources.
fn read_icon_layout(dir: &editpe::ResourceDirectory) -> Option<IconLayout> {
    use editpe::{ResourceEntry, ResourceEntryName};

    const RT_ICON: u32 = 3;
    const RT_GROUP_ICON: u32 = 14;

    let root = dir.root();
    let groups = match root.get(ResourceEntryName::ID(RT_GROUP_ICON))? {
        ResourceEntry::Table(t) => t,
        _ => return None,
    };

    // All existing group ids; the primary group is the lowest numeric one.
    let group_ids: Vec<u16> = groups
        .entries()
        .into_iter()
        .filter_map(|n| match n {
            ResourceEntryName::ID(id) => u16::try_from(*id).ok(),
            _ => None,
        })
        .collect();
    let group_id = *group_ids.iter().min()?;

    // The group's icon directory lives under a language sub-table; read its first data entry and
    // parse the 14-byte GRPICONDIRENTRY records for their member RT_ICON ids.
    let mut member_ids = Vec::new();
    if let Some(ResourceEntry::Table(lang)) = groups.get(ResourceEntryName::ID(group_id as u32)) {
        if let Some(first) = lang.entries().first().map(|n| (*n).clone()) {
            if let Some(ResourceEntry::Data(data)) = lang.get(first) {
                let bytes = data.data();
                if bytes.len() >= 6 {
                    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
                    for i in 0..count {
                        let id_off = 6 + i * 14 + 12;
                        if let Some(slice) = bytes.get(id_off..id_off + 2) {
                            member_ids.push(u16::from_le_bytes([slice[0], slice[1]]));
                        }
                    }
                }
            }
        }
    }

    // Highest RT_ICON id anywhere in the PE (so minted ids never collide with a built-in icon).
    let max_icon_id = match root.get(ResourceEntryName::ID(RT_ICON)) {
        Some(ResourceEntry::Table(t)) => t
            .entries()
            .into_iter()
            .filter_map(|n| match n {
                ResourceEntryName::ID(id) => u16::try_from(*id).ok(),
                _ => None,
            })
            .max()
            .unwrap_or(0),
        _ => 0,
    };

    Some(IconLayout {
        group_id,
        member_ids,
        group_ids,
        max_icon_id,
    })
}

/// Read the interpreter's `VS_VERSION_INFO`, overlay the project's metadata, and rebuild the raw
/// resource bytes. Returns `None` if the interpreter has no version info to use as a template.
fn build_version_bytes(interpreter: &Path, config: &BuildConfig) -> Result<Option<Vec<u8>>> {
    let image = editpe::Image::parse_file(interpreter)
        .with_context(|| format!("reading interpreter PE {}", interpreter.display()))?;
    let Some(res) = image.resource_directory() else {
        return Ok(None);
    };
    let Some(mut info) = res.get_version_info()? else {
        return Ok(None);
    };

    let exe = &config.exe;
    let version = exe.version.as_deref().unwrap_or("0.0.0.0");
    let (ms, ls) = parse_version(version);
    info.info.file_version.major = ms;
    info.info.file_version.minor = ls;
    info.info.product_version.major = ms;
    info.info.product_version.minor = ls;

    // String table entries. Only overwrite what the config provides; leave the rest as shipped.
    let mut set = |key: &str, value: Option<String>| {
        if let Some(v) = value {
            for table in info.strings.iter_mut() {
                table.strings.insert(key.to_string(), v.clone());
            }
        }
    };
    set("FileVersion", Some(version.to_string()));
    set("ProductVersion", Some(version.to_string()));
    set("ProductName", exe.name.clone());
    set("InternalName", exe.name.clone());
    set(
        "OriginalFilename",
        exe.name.as_ref().map(|n| format!("{n}.exe")),
    );
    set("FileDescription", exe.description.clone());
    set("LegalCopyright", exe.copyright.clone());

    Ok(Some(info.build()))
}

/// Split a dotted version string `W.X.Y.Z` into the two packed `dwFileVersionMS`/`LS` words used
/// by `VS_FIXEDFILEINFO`: `MS = (W << 16) | X`, `LS = (Y << 16) | Z`. Missing parts default to 0.
fn parse_version(s: &str) -> (u32, u32) {
    let mut parts = s.split('.').map(|p| p.trim().parse::<u32>().unwrap_or(0));
    let w = parts.next().unwrap_or(0);
    let x = parts.next().unwrap_or(0);
    let y = parts.next().unwrap_or(0);
    let z = parts.next().unwrap_or(0);
    ((w << 16) | (x & 0xffff), (y << 16) | (z & 0xffff))
}

/// Inject the script modules, optional version info, `FileInstall` files, and primary icon into
/// the PE at `output` via Win32 `UpdateResource`. Entry -> `RT_RCDATA` integer id 1; named modules
/// and `FileInstall` files -> `RT_RCDATA` string name; icon -> `RT_GROUP_ICON` + `RT_ICON`.
#[cfg(windows)]
fn write_resources(
    output: &Path,
    modules: &[EmitModule],
    version: Option<&[u8]>,
    files: &[(String, Vec<u8>)],
    icons: &[IconResources],
    extras: &[EmbeddedResource],
) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::LibraryLoader::{
        BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW,
    };
    // MAKEINTRESOURCE: a numeric type/name is passed where a wide-string pointer is expected; the
    // loader checks the high word is zero and uses the low word as the integer id. These are
    // address-only "pointers" with no provenance, never dereferenced.
    const RT_RCDATA: *const u16 = std::ptr::without_provenance(10);
    const RT_VERSION: *const u16 = std::ptr::without_provenance(16);
    const RT_ICON: *const u16 = std::ptr::without_provenance(3);
    const RT_GROUP_ICON: *const u16 = std::ptr::without_provenance(14);
    let int_name = |id: usize| -> *const u16 { std::ptr::without_provenance(id) };

    let wide_path: Vec<u16> = output.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide_path` is a valid NUL-terminated wide string for the duration of the call.
    let handle = unsafe { BeginUpdateResourceW(wide_path.as_ptr(), 0) };
    anyhow::ensure!(
        !handle.is_null(),
        "BeginUpdateResource failed (errno={}) for {}",
        // SAFETY: GetLastError just reads the calling thread's last-error code.
        unsafe { GetLastError() },
        output.display()
    );

    // Names of string-named resources must outlive the update calls.
    let mut name_bufs: Vec<Vec<u16>> = Vec::new();
    // `data == None` deletes the resource (pass a null pointer and zero length).
    let update = |ty: *const u16, name: *const u16, data: Option<&[u8]>| -> Result<()> {
        let (ptr, len) = match data {
            Some(d) => (d.as_ptr() as *const core::ffi::c_void, d.len() as u32),
            None => (std::ptr::null(), 0),
        };
        // SAFETY: `handle` is live; `data` outlives the call (UpdateResource copies it).
        let ok = unsafe { UpdateResourceW(handle, ty, name, LANG_EN_US, ptr, len) };
        // SAFETY: GetLastError just reads the calling thread's last-error code.
        anyhow::ensure!(ok != 0, "UpdateResource failed (errno={})", unsafe {
            GetLastError()
        });
        Ok(())
    };
    // Build (and keep alive) a NUL-terminated wide name buffer, returning a pointer to it.
    let wide_name = |bufs: &mut Vec<Vec<u16>>, s: &str| -> *const u16 {
        let buf: Vec<u16> = s.encode_utf16().chain([0]).collect();
        let ptr = buf.as_ptr();
        bufs.push(buf);
        ptr
    };

    for m in modules {
        let name: *const u16 = match &m.resource {
            ResourceName::Entry => int_name(1),
            ResourceName::Named(n) => wide_name(&mut name_bufs, n),
        };
        update(RT_RCDATA, name, Some(m.text.as_bytes()))
            .with_context(|| format!("embedding module {:?}", m.resource))?;
    }

    for (name, data) in files {
        let wname = wide_name(&mut name_bufs, name);
        update(RT_RCDATA, wname, Some(data))
            .with_context(|| format!("embedding FileInstall resource {name}"))?;
    }

    if let Some(v) = version {
        update(RT_VERSION, int_name(1), Some(v)).context("embedding version info")?;
    }

    for icon in icons {
        for (id, data) in &icon.images {
            update(RT_ICON, int_name(*id as usize), Some(data))
                .with_context(|| format!("embedding icon image {id}"))?;
        }
        for id in &icon.stale_image_ids {
            update(RT_ICON, int_name(*id as usize), None)
                .with_context(|| format!("removing stale icon image {id}"))?;
        }
        update(
            RT_GROUP_ICON,
            int_name(icon.group_id as usize),
            Some(&icon.group_data),
        )
        .with_context(|| format!("embedding icon group {}", icon.group_id))?;
    }

    for res in extras {
        let ty: *const u16 = std::ptr::without_provenance(res.type_id as usize);
        let name: *const u16 = match &res.name {
            ResName::Id(id) => int_name(*id as usize),
            ResName::Name(n) => wide_name(&mut name_bufs, n),
        };
        update(ty, name, Some(&res.data)).with_context(|| {
            format!("embedding resource (type {}, {:?})", res.type_id, res.name)
        })?;
    }

    // SAFETY: `handle` is live; `0` (FALSE) commits the accumulated updates.
    let ok = unsafe { EndUpdateResourceW(handle, 0) };
    anyhow::ensure!(
        ok != 0,
        "EndUpdateResource failed (errno={})",
        // SAFETY: GetLAstError just reads the current thread's last error
        unsafe { GetLastError() }
    );
    drop(name_bufs);
    Ok(())
}

#[cfg(not(windows))]
fn write_resources(
    _output: &Path,
    _modules: &[EmitModule],
    _version: Option<&[u8]>,
    _files: &[(String, Vec<u8>)],
    _icons: &[IconResources],
    _extras: &[EmbeddedResource],
) -> Result<()> {
    anyhow::bail!(
        "`bundle exe` currently requires Windows: it injects resources via the Win32 \
         UpdateResource API. Run the build on Windows."
    )
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn version_packs_into_ms_ls_words() {
        assert_eq!(parse_version("1.2.3.4"), ((1 << 16) | 2, (3 << 16) | 4));
        assert_eq!(parse_version("0.0.0.0"), (0, 0));
        assert_eq!(parse_version("2"), ((2 << 16), 0));
    }
}
