---
title: ahkbuild.json
weight: 2
---

# `ahkbuild.json` reference

The project configuration file for the [exe target]({{< relref "/docs/exe" >}}).
It is discovered by walking up from the current directory, or pointed at
explicitly with `--config`. A few fields can be overridden per-invocation by a
[CLI flag]({{< relref "/docs/reference/cli" >}}).

Relative paths in the file resolve against the **config file's directory** (the
project root), not the current working directory.

## Minimal config

Only the `interpreter` block (with a `version`) is required. The entry script
may come from the config or from `--input`:

```json
{
  "entry": "src/main.ahk",
  "interpreter": { "version": "2.1-alpha.27" }
}
```

## Full example

```json
{
  "entry": "src/main.ahk",
  "interpreter": {
    "version": "2.1-alpha.27",
    "bitness": 64
  },
  "exe": {
    "name": "MyApp",
    "version": "1.2.3.0",
    "description": "My application",
    "copyright": "Copyright 2026 Example",
    "company": "Example, LLC",
    "trademarks": "MyApp is a trademark of Example, LLC",
    "comments": "Built with ahkbuild",
    "icon": "assets/icon.ico",
    "subsystem": "gui",
    "manifest": {
      "uac": "requireAdministrator",
      "dpiAwareness": "PerMonitorV2",
      "longPathAware": true,
      "gdiScaling": true
    }
  },
  "resources": {
    "icons": [
      { "path": "assets/extra.ico", "id": 300 }
    ],
    "extra": [
      { "name": "HELP", "type": "RT_HTML", "path": "assets/help.html" },
      { "name": "ABOUT", "type": 23, "path": "assets/about.html" }
    ]
  },
  "scripts": {
    "pre-bundle": [ ["${AHK}", "scripts/codegen.ahk"] ],
    "post-bundle": [ ["signtool", "sign", "/fd", "SHA256", "${AHKBUILD_OUTPUT}"] ]
  },
  "defines": {
    "MODE": "release"
  }
}
```

## Top-level fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| [`entry`](#entry) | string (path) | No¹ | Entry script to bundle. |
| [`interpreter`](#interpreter) | object | **Yes** | Which AHK interpreter to build against. |
| [`exe`](#exe) | object | No | Output executable metadata and manifest. |
| [`resources`](#resources) | object | No | Extra icons and embedded resources. |
| [`scripts`](#scripts) | object | No | Pre/post-bundle hooks. |
| [`defines`](#defines) | object | No | User build variables. |
| [`dependencies`](#dependencies) | object | No | Module dependencies, keyed by import name. |

¹ `entry` is optional in the file but must be supplied somehow - either here or
via `--input`. It is an error if neither is present.

## `entry`

Path to the script that is the root of the `#Import` graph.

```json
{ "entry": "src/main.ahk" }
```

Overridden by `--input`.

## `interpreter`

The interpreter the exe is built from. Required.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `version` | string | - (**required**) | AHK version, e.g. `"2.1-alpha.27"` or `"2.0.26"`. Selects the cached interpreter; auto-installed if missing. See [Interpreter management]({{< relref "/docs/exe/interpreters" >}}). The version string must exactly match either an [Autohotkey download](https://www.autohotkey.com/download/) or the tag of an AutoHotkey GitHub release. |
| `bitness` | `32` \| `64` | `64` | Target architecture. Also folds `A_PtrSize`. |

`version` is overridden by `--interpreter-version`; `bitness` by `--bitness`.

```json
{ "interpreter": { "version": "2.1-alpha.27", "bitness": 64 } }
```

## `exe`

Output executable metadata. Every field is optional. Version-info strings left
unset keep the interpreter's default value.

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `name` | string | - | `InternalName` / `OriginalFilename`; also the default output filename (`<name>.exe`). |
| `version` | string | `"0.0.0.0"` | Four-part `W.X.Y.Z` version (`FileVersion` / `ProductVersion`). |
| `description` | string | - | `FileDescription`. |
| `copyright` | string | - | `LegalCopyright`. |
| `company` | string | - | `CompanyName`. |
| `trademarks` | string | - | `LegalTrademarks`. |
| `comments` | string | - | `Comments`. |
| `icon` | string (path) | - | `.ico` replacing the primary application icon. See [Icons]({{< relref "/docs/exe/resources#icons" >}}). |
| `subsystem` | `"gui"` \| `"console"` | `"gui"` | [PE subsystem] for the final executable. `console` allocates a console for stdin/stdout. See [Subsystem]({{< relref "/docs/exe/subsystem" >}}). |
| `manifest` | object | `{}` | Application-manifest overrides (below). |

See [Version info]({{< relref "/docs/exe/version-info" >}}) for the metadata
fields in context.

[PE subsystem]: https://learn.microsoft.com/de-de/cpp/build/reference/subsystem-specify-subsystem?view=msvc-180

### `exe.manifest`

Values here override or are merged into the [application manifest] shipped with the
interpreter. All fields are optional and omitting any field (or the entire object)
leaves the interpreter's default in place.

| Field | Type | Manifest element | Description |
| --- | --- | --- | --- |
| `uac` | `"asInvoker"` \| `"highestAvailable"` \| `"requireAdministrator"` | `<requestedExecutionLevel level>` | [UAC] elevation requested at launch by the OS - this can prevent the need to relaunch the script as an administrator (see [`A_IsAdmin`]). |
| `dpiAware` | bool | `<dpiAware>` | Legacy DPI-awareness flag. |
| `dpiAwareness` | string | `<dpiAwareness>` | Modern DPI mode, e.g. `"PerMonitorV2"`, `"system"`. |
| `longPathAware` | bool | `<longPathAware>` | Opt into paths longer than `MAX_PATH`. |
| `gdiScaling` | bool | `<gdiScaling>` | GDI bitmap scaling under DPI virtualization. |

[application manifest]: https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests
[UAC]: https://learn.microsoft.com/en-us/windows/win32/secauthz/user-account-control
[`A_IsAdmin`]: https://www.autohotkey.com/docs/alpha/Variables.htm#RequireAdmin

## `resources`

Additional arbitrary resources embedded into the exe.

Scripts can retrieve these at runtime with [`LoadResource`].

[`LoadResource`]: https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-loadresource

### `resources.icons`

Extra application icons. Each icon here is filed as a new `RT_GROUP_ICON` under an **explicit
resource id** you choose. A script [loads](https://www.autohotkey.com/docs/alpha/lib/LoadPicture.htm) one by id:

```ahk
hIcon := LoadPicture(A_ScriptFullPath, "Icon-300", &type)  ; resource id 300
```

| Field | Type | Description |
| --- | --- | --- |
| `path` | string (path) | The `.ico` file. |
| `id` | integer (`u16`) | `RT_GROUP_ICON` resource id. Must not collide with a built-in interpreter icon group or another configured icon. |

For the primary/taskbar icon use [`exe.icon`](#exe), not this list. See [Icons]({{< relref "/docs/exe/resources#icons" >}}).

### `resources.extra`

Arbitrary resources embedded by name, type, and path - the equivalent of Ahk2Exe's [`;@Ahk2Exe-AddResource`]. Read back at runtime with
`FindResource` / `LoadResource`.

| Field | Type | Description |
| --- | --- | --- |
| `name` | string | Resource name. `#N` is an integer id; anything else is a string name. |
| `type` | string \| integer | A standard `RT_*` constant (case-insensitive, `RT_` prefix optional - `"RT_HTML"` and `"html"` are equivalent) or a raw integer [resource type]. Icon types are rejected - use `resources.icons`. |
| `path` | string (path) | The file whose bytes are embedded verbatim. |

See [Embedded resources]({{< relref "/docs/exe/resources" >}}) for naming rules and collision errors.

[`;@Ahk2Exe-AddResource`]: https://www.autohotkey.com/docs/alpha/misc/Ahk2ExeDirectives.htm#AddResource
[resource type]: https://learn.microsoft.com/en-us/windows/win32/menurc/resource-types

## `scripts`

Out-of-process commands run before and after bundling. Each entry is an **argv array**. Three shapes are accepted, all normalized to one command:

```jsonc
{
  "scripts": {
    "pre-bundle": [
      "./generate.exe",                 // bare string: one executable, no args
      ["${AHK}", "scripts/codegen.ahk"] // array: explicit argv
    ],
    "post-bundle": [
      { "command": ["upx", "--best", "${AHKBUILD_OUTPUT}"] } // object: options slot
    ]
  }
}
```

| Field | Type | Description |
| --- | --- | --- |
| `pre-bundle` | array of commands | Run before emit, in order. |
| `post-bundle` | array of commands | Run on the finished exe, in order. |

A command must have at least one token. `${NAME}` tokens are substituted by ahkbuild (the `${AHK}` interpreter path, `${AHKBUILD_*}` vars, and `defines`).
A non-zero exit aborts the build. See [Build scripts]({{< relref "/docs/exe/build-scripts" >}}) for the full token and environment tables.

## `defines`

A map of build variables exposed to build scripts (as environment variables and `${NAME}` tokens).

> [!NOTE]
> These variables are reserved to drive future conditional compilation directives in the preprocessor.

```json
{ 
  "defines": { 
    "DEBUG": 1,
    "RATIO": 1.5,
    "FLAG": true,
    "MODE": "release"
  }
}
```

Values may be strings, numbers, or booleans, but are always flattened to strings (`1`, `1.5`, `true`,
`release`). Must match `[A-Za-z_][A-Za-z0-9_]*` and may **not** start with `AHKBUILD_` (that prefix is
reserved for future use).

## `dependencies`

Module dependencies, keyed by the **logical import name** written in `#Import Name`. Each value
names exactly one source. Resolved, pinned to `ahkbuild.lock`, and materialized into the
`.ahkbuild/modules/` link-farm by [`ahkbuild package restore`]({{< relref "/docs/reference/cli#ahkbuild-package" >}}).
See [Dependencies]({{< relref "/docs/dependencies" >}}) for the full model.

```json
{
  "dependencies": {
    "GuiEnhancer": { "git": "https://github.com/nperovic/GuiEnhancerKit.git", "tag": "v1.0.3" },
    "gistCode":    { "gist": "a1b2c3d4e5f6", "rev": "deadbeef" },
    "Rapid":       { "tarball": "https://example.com/rapid.zip", "sha256": "…", "subdir": "src" },
    "MyLocal":     { "path": "../shared/MyLocal" }
  }
}
```

Each value sets **exactly one** source key (`git`, `gist`, `tarball`, or `path`); setting zero or
more than one is an error.

| Source key | Companion fields | Description |
| --- | --- | --- |
| `git` | one of `tag` / `branch` / `rev` (optional) | A `.git` clone URL for any forge. No selector uses the default branch HEAD. Pinned to a commit SHA in the lock. |
| `gist` | `rev` (optional) | A gist id (gists are git repos). `rev` pins a commit; latest HEAD otherwise. |
| `tarball` | `sha256` (**required**) | A `.zip` or `.tar.gz` URL. `sha256` of the archive bytes is verified on download. |
| `path` | - | A local directory (relative paths resolve against the project root). Not reproducible, so **excluded from the lockfile**. |

| Common field | Type | Description |
| --- | --- | --- |
| `subdir` | string | Sub-directory within the fetched tree that holds the module, when it is not the tree root. |

`sha256` is rejected on non-tarball sources; `tag`/`branch`/`rev` are rejected on `tarball`/`path`;
`git` accepts at most one selector.
