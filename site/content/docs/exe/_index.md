---
title: Exe target
weight: 3
bookCollapseSection: false
---

# The exe target

`ahkbuild bundle exe` produces a standalone Windows executable from your script and its `#Import` graph. The
output is a real `.exe` that runs without a separately installed AutoHotkey, with your metadata, icon, and
resources baked in.

> [!IMPORTANT]
> Building an `.exe` currently **requires Windows**. The bundler injects resources through the
> Win32 resource-update API, which has no cross-platform equivalent yet. Bundling to a single
> [`.ahk` file]({{< relref "/docs/bundling" >}}) works everywhere.

This process is often referred to as "compiling" a script, though no actual code compilation is going on.

## What is this?

A "compiled" AutoHotkey script is just the AutoHotkey **interpreter binary** with your script embedded
inside it as a resource. When the exe starts, the interpreter notices it carries an embedded script and runs
that instead of looking for one on disk. `ahkbuild` produces one by copying the configured
[interpreter]({{< relref "/docs/exe/interpreters" >}}), embedding your code, and patching in metadata.

> [!NOTE]
> Running a bundled `.exe` is not fundamentally different from running a script from source via a standalone
> install of AutoHotkey. Bundling **will not improve performance**[^1] and does **not** provide any meaningful
> obfuscation of your code. You can inspect the contents of a bundled script trivially with 7-zip.

How the script is embedded depends on the AutoHotkey version:

- **v2.0** - the whole program is concatenated into a single script (exactly the output of
  [`bundle ahk`]({{< relref "/docs/bundling" >}})) and embedded as one resource.
- **v2.1** - each module is embedded as its own resource under its module name, and `#Import` directives are
  rewritten to point at the embedded modules. There is no single-file concatenation step, so no module-name mangling is needed.

Bundling also allows you to [embed arbitrary resources]({{< relref "docs/exe/resources">}}) including icons
and files for easier distribution.

[^1]: Although tree-shaking and constant folding may reduce the total size of your script and thus its memory
footprint.

### The exe bundling process

Everything up to emission is shared with the `.ahk` target.

[Preprocessing]({{< relref "/docs/bundling/preprocessing" >}}),
[constant folding]({{< relref "/docs/bundling/constant-folding" >}}), and
[tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}}) all behave the same. Only the final
step differs: instead of writing one text file, the emitter writes each module into the copied
interpreter and layers your icon, version info, manifest, and extra resources on top.

> [!NOTE]
> Because the exe target builds a *compiled* script, `A_IsCompiled` folds to `true` and
> `A_PtrSize` folds to the [target bitness]({{< relref "/docs/exe/interpreters" >}}). Branches
> guarded by these are resolved at build time - see
> [constant folding]({{< relref "/docs/bundling/constant-folding" >}}).

## Features

Each part of the exe build has its own page:

- [Version info]({{< relref "/docs/exe/version-info" >}}) - product name, version, copyright, and
  other metadata shown in the file's Properties dialog.
- [Manifest]({{< relref "/docs/exe/manifest" >}}) - UAC elevation and DPI awareness.
- [Embedded resources]({{< relref "/docs/exe/resources" >}}) - `FileInstall`, icons, and arbitrary
  resources.
- [Subsystem]({{< relref "/docs/exe/subsystem" >}}) - GUI vs. console.
- [Interpreter management]({{< relref "/docs/exe/interpreters" >}}) - how the interpreter binary is
  acquired and cached.
- [Build scripts]({{< relref "/docs/exe/build-scripts" >}}) - pre- and post-bundle commands.

Bundling is configured through [`ahkbuild.json`]({{< relref "/docs/reference/config" >}}).
