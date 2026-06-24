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

use std::path::Path;

use anyhow::{Context, Result};

use ahkbuild_config::BuildConfig;
use ahkbuild_emit::{emit_exe_modules, EmitModule, EmitOptions, ResourceName, WsLevel};
use ahkbuild_fold::FoldResult;
use ahkbuild_ir::Program;
use ahkbuild_link::BundlePlan;
use ahkbuild_shake::ShakeResult;

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

    if config.exe.icon.is_some() {
        eprintln!(
            "note: exe.icon is set but icon embedding is not yet implemented; ignoring for now"
        );
    }

    // 3. Copy the interpreter to the output path, then inject resources into the copy.
    std::fs::copy(interpreter, output).with_context(|| {
        format!(
            "copying interpreter {} -> {}",
            interpreter.display(),
            output.display()
        )
    })?;

    write_resources(output, &modules, version.as_deref())
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

/// Inject the module scripts (and optional version info) into the PE at `output` via Win32
/// `UpdateResource`. Entry -> `RT_RCDATA` integer id 1; named modules -> `RT_RCDATA` string name.
#[cfg(windows)]
fn write_resources(output: &Path, modules: &[EmitModule], version: Option<&[u8]>) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        BeginUpdateResourceW, EndUpdateResourceW, UpdateResourceW,
    };

    // MAKEINTRESOURCE: a numeric type/name is passed where a wide-string pointer is expected; the
    // loader checks the high word is zero and uses the low word as the integer id. These are
    // address-only "pointers" with no provenance, never dereferenced.
    const RT_RCDATA: *const u16 = std::ptr::without_provenance(10);
    const RT_VERSION: *const u16 = std::ptr::without_provenance(16);
    let int_name = |id: usize| -> *const u16 { std::ptr::without_provenance(id) };

    let wide_path: Vec<u16> = output.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: `wide_path` is a valid NUL-terminated wide string for the duration of the call.
    let handle = unsafe { BeginUpdateResourceW(wide_path.as_ptr(), 0) };
    anyhow::ensure!(
        !handle.is_null(),
        "BeginUpdateResource failed for {}",
        output.display()
    );

    // Names of string-named resources must outlive the update calls.
    let mut name_bufs: Vec<Vec<u16>> = Vec::new();
    let update = |ty: *const u16, name: *const u16, data: &[u8]| -> Result<()> {
        // SAFETY: `handle` is live; `data` outlives the call (UpdateResource copies it).
        let ok = unsafe {
            UpdateResourceW(
                handle,
                ty,
                name,
                LANG_EN_US,
                data.as_ptr() as *const core::ffi::c_void,
                data.len() as u32,
            )
        };
        anyhow::ensure!(ok != 0, "UpdateResource failed");
        Ok(())
    };

    for m in modules {
        let name: *const u16 = match &m.resource {
            ResourceName::Entry => int_name(1),
            ResourceName::Named(n) => {
                let buf: Vec<u16> = n.encode_utf16().chain([0]).collect();
                let ptr = buf.as_ptr();
                name_bufs.push(buf);
                ptr
            }
        };
        update(RT_RCDATA, name, m.text.as_bytes())
            .with_context(|| format!("embedding module {:?}", m.resource))?;
    }

    if let Some(v) = version {
        update(RT_VERSION, int_name(1), v).context("embedding version info")?;
    }

    // SAFETY: `handle` is live; `0` (FALSE) commits the accumulated updates.
    let ok = unsafe { EndUpdateResourceW(handle, 0) };
    anyhow::ensure!(ok != 0, "EndUpdateResource failed");
    drop(name_bufs);
    Ok(())
}

#[cfg(not(windows))]
fn write_resources(_output: &Path, _modules: &[EmitModule], _version: Option<&[u8]>) -> Result<()> {
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
