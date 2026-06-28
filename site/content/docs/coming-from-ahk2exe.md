---
title: Coming from Ahk2Exe
weight: 5
---

# Coming from Ahk2Exe

`ahkbuild` is not, and does not try to be, a drop-in replacement for [`Ahk2Exe`]. It is a
config-based bundler designed for projects, not a right-click-to-compile tool: everything Ahk2Exe
expresses as `;@Ahk2Exe-*` comments scattered through the source, `ahkbuild` reads from one
[`ahkbuild.json`]({{< relref "/docs/reference/config" >}}). It supports the same output features as
Ahk2Exe, plus several it doesn't have.

The primary goal of `ahkbuild` is ergonomics and first-class support for ci/cd or build pipelines. In a properly
configured project, simply clone the repository and run

```bash
ahkbuild bundle exe
```

[`Ahk2Exe`]: https://www.autohotkey.com/docs/alpha/misc/Ahk2ExeDirectives.htm

## Directive mapping

Most Ahk2Exe directives have a direct equivalent in config or a build directive.

### Executable metadata

All of these live under [`exe`]({{< relref "/docs/reference/config#exe" >}}). See
[Version info]({{< relref "/docs/exe/version-info" >}}).

| Ahk2Exe directive | `ahkbuild` |
| --- | --- |
| `;@Ahk2Exe-SetVersion`, `-SetProductVersion` | `exe.version` (sets both `FileVersion` and `ProductVersion`) |
| `;@Ahk2Exe-SetName`, `-SetInternalName`, `-SetProductName`, `-SetOrigFilename` | `exe.name` (sets `ProductName`, `InternalName`, and `OriginalFilename`) |
| `;@Ahk2Exe-SetDescription` | `exe.description` |
| `;@Ahk2Exe-SetCopyright` | `exe.copyright` |
| `;@Ahk2Exe-SetCompanyName` | `exe.company` |
| `;@Ahk2Exe-SetLegalTrademarks` | `exe.trademarks` |
| `;@Ahk2Exe-SetComments` | `exe.comments` |

### Icons and resources

| Ahk2Exe directive | `ahkbuild` |
| --- | --- |
| `;@Ahk2Exe-SetMainIcon` | [`exe.icon`]({{< relref "/docs/reference/config#exe" >}}) |
| `;@Ahk2Exe-AddResource icon.ico, N` | [`resources.icons`]({{< relref "/docs/exe/resources#icons" >}}) (`{ "path": ..., "id": N }`) |
| `;@Ahk2Exe-AddResource file, ...` | [`resources.extra`]({{< relref "/docs/exe/resources#extra-resources" >}}) |

> [!NOTE]
> Unlike Ahk2Exe, `ahkbuild` does **not** infer a resource's type from its file extension - you
> declare the [`type`]({{< relref "/docs/reference/config#resourcesextra" >}}) explicitly. See
> [Embedded resources]({{< relref "/docs/exe/resources" >}}).

### Build process

| Ahk2Exe directive | `ahkbuild` |
| --- | --- |
| `;@Ahk2Exe-ConsoleApp` | [`exe.subsystem: "console"`]({{< relref "/docs/exe/subsystem" >}}) |
| `;@Ahk2Exe-PostExec` | A [post-bundle build script]({{< relref "/docs/exe/build-scripts" >}}) |
| `;@Ahk2Exe-Bin` / `-ExeName` | [`interpreter`]({{< relref "/docs/exe/interpreters" >}}) version/bitness + `--output` |
| `;@Ahk2Exe-Obey`, `-Let`, `-Cont`, `-If` | No equivalent - use [build scripts]({{< relref "/docs/exe/build-scripts" >}}) and [`defines`]({{< relref "/docs/reference/config#defines" >}}) for build-time logic |

### Ignoring source

| Ahk2Exe directive | `ahkbuild` |
| --- | --- |
| `;@Ahk2Exe-IgnoreBegin` / `-IgnoreEnd` | Honored directly, as is [`;@AhkBuild-IgnoreBegin` / `-IgnoreEnd`]({{< relref "/docs/bundling/preprocessing" >}}) |
| `;@Ahk2Exe-Keep` (`IgnoreKeep`) | **Not** honored - guard the code with [`A_IsCompiled`]({{< relref "/docs/bundling/constant-folding" >}}) and let [tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}}) drop the dead arm |

## What `ahkbuild` adds

Things Ahk2Exe has no equivalent for:

- **Declarative project config.** One [`ahkbuild.json`]({{< relref "/docs/reference/config" >}})
  instead of directives spread across source files.
- **Application manifest control.** [UAC elevation, DPI awareness, long-path and GDI
  scaling]({{< relref "/docs/exe/manifest" >}}) without hand-editing a base file.
- **Build scripts.** Arbitrary [pre- and post-bundle commands]({{< relref "/docs/exe/build-scripts" >}})
  with token substitution - codegen, compression, signing.
- **Interpreter management.** Automatic [download and caching]({{< relref "/docs/exe/interpreters" >}})
  of the exact interpreter version and bitness you target.
- **Module-aware bundling.** Resolves the `#Import` graph, then
  [tree-shakes]({{< relref "/docs/bundling/tree-shaking" >}}) dead code and
  [folds build-time constants]({{< relref "/docs/bundling/constant-folding" >}}).
