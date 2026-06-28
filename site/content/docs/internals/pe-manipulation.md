---
title: PE manipulation
weight: 2
---

# PE manipulation

Building an `.exe` means taking the AutoHotkey [interpreter]({{< relref "/docs/exe/interpreters" >}})
binary - a real, shipped Windows PE file - and adding your script and metadata to it without
breaking anything else inside it. This turns out to be the single most delicate part of the whole
tool, and the reason `bundle exe` is currently **Windows-only**.

## Why `UpdateResource`, not a pure-Rust PE library

Resources are injected with the Win32 [`UpdateResource`] family of APIs
([`BeginUpdateResource`] / `UpdateResource` / `EndUpdateResource`) - exactly as Ahk2Exe does. The
emitter copies the interpreter to the output path, opens an update handle, writes each resource, and
commits.

Two pure-Rust resource editors were evaluated and both fell short:

- [`pelite`] - its resource support is **read-only**. It can parse the resource directory but cannot
  rebuild it, so it can't add a script.
- [`editpe`] - *can* rebuild the resource directory, but its rebuilt `.rsrc` section **corrupts the
  AHK interpreter**. An editpe round-trip of `AutoHotkey64.exe` with *no other changes* produces a
  binary that can no longer run scripts.
  - It may be possible to use [`editpe`] as a an editor with more investigation.

`UpdateResource` performs a minimal **in-place** edit and preserves everything else, so it Just
Works. The lesson that runs through this whole subsystem: the interpreter is load-bearing, and the
smallest possible edit is the safe one.

> [!NOTE]
> `editpe` is still used - but only as a **reader**. It parses the interpreter's existing
> `VS_VERSION_INFO` and `RT_MANIFEST` so those can be overlaid and rebuilt in memory, but the PE
> itself is never serialized by editpe. The new bytes are written back through `UpdateResource`.

Because `UpdateResource` is a live OS API, `bundle exe` requires Windows. The
cross-platform-from-Linux goal is deferred behind a future pure-Rust resource writer (a hand-rolled
`.rsrc` builder, or a fixed `editpe`) - but correctness on Windows came first. Bundling to a single
[`.ahk` file]({{< relref "/docs/bundling" >}}) has none of this and runs everywhere.

[`UpdateResource`]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-updateresourcew
[`BeginUpdateResource`]: https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-beginupdateresourcew
[`pelite`]: https://docs.rs/pelite
[`editpe`]: https://docs.rs/editpe

## Exe emission

`emit_exe::bundle_exe` is the entry point. The module source text is rendered by
`ahkbuild_emit::emit_exe_modules` (per-group emit, imports rewritten); this crate handles the binary
side:

1. **Emit modules** - each group is rendered to minified source. The entry group is the entry
   resource; others are named after their module, uppercased (Win32 resource names match
   case-insensitively), and emitted with **no synthetic `#Module` header** - the resource *is* the
   module, so a header would demote the body to an empty default sub-module.
2. **Import binding** - a bare `#Import Name` is rewritten to `#Import "*NAME" as Name`. The `as`
   re-adds the default-export binding the unquoted form provided, which a quoted spec would
   otherwise drop.
3. **Version bytes** - read the interpreter's `VS_VERSION_INFO` (via editpe), overlay
   [`config.exe`]({{< relref "/docs/exe/version-info" >}}), rebuild the bytes.
4. **Manifest bytes** - if any [`exe.manifest`]({{< relref "/docs/exe/manifest" >}}) field is set,
   read the interpreter's `RT_MANIFEST` and apply the configured edits (see
   [Manifest namespaces]({{< relref "/docs/internals/manifest-namespaces" >}})); skipped otherwise.
5. **Copy + UpdateResource** - copy the interpreter to the output, then `BeginUpdateResource` /
   `UpdateResource` (one `RT_RCDATA` per module, plus `FileInstall` files,
   [icons]({{< relref "/docs/internals/icon-internals" >}}), `RT_VERSION`, `RT_MANIFEST`, and
   [extra resources]({{< relref "/docs/exe/resources" >}})) / `EndUpdateResource`. Encoding is UTF-8
   without BOM, language `0x0409`, integer id `1` for the entry - all matching Ahk2Exe.
6. **Subsystem** - patch the PE optional-header `Subsystem` field in place (a 2-byte edit:
   `2` = GUI, `3` = console). Done last, since `UpdateResource` never touches it. See
   [Subsystem]({{< relref "/docs/exe/subsystem" >}}).

Code signing is intentionally **not** part of this flow - it's naturally a
[post-bundle build script]({{< relref "/docs/exe/build-scripts" >}}) (`signtool`, `osslsigncode`).
