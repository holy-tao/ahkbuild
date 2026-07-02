---
title: CLI
weight: 1
---

# CLI reference

- [`ahkbuild preprocess`](#ahkbuild-preprocess)
- [`ahkbuild bundle ahk`](#ahkbuild-bundle-ahk)
- [`ahkbuild bundle exe`](#ahkbuild-bundle-exe)
- [`ahkbuild interpreter`](#ahkbuild-interpreter)
- [`ahkbuild package`](#ahkbuild-package)
- [`ahkbuild run`](#ahkbuild-run)
- [Global Flags](#global-flags)
  - [Logging](#logging)

## `ahkbuild preprocess`

Runs the [preprocessor]({{< relref "/docs/bundling/preprocessing" >}}) over a single file and emits
the result. Primarilly for debugging.

```bash
ahkbuild preprocess <input> [output]
```

| Argument | Description |
| --- | --- |
| `input` | The file to preprocess. |
| `output` | Where to write the result. Printed to `stdout` if omitted. |

## `ahkbuild bundle ahk`

Bundles a script and its `#Import` graph into a single self-contained `.ahk` file.

```bash
ahkbuild bundle ahk <input> [output] [flags]
```

| Argument | Description |
| --- | --- |
| `input` | The entry script to bundle. |
| `output` | Output file. Printed to `stdout` if omitted. |

| Flag | Description |
| --- | --- |
| `--no-tree-shake` | Disable [tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}}); emit a byte-faithful bundle. |
| `--keep-comments` | Keep comments in the output. Comments are stripped by default. |
| `--compiled <true\|false>` | Override [`A_IsCompiled`]({{< relref "/docs/bundling/constant-folding" >}}) for branch folding. Off by default for the `.ahk` target. |
| `--bitness <32\|64>` | Target bitness used to fold [`A_PtrSize`]({{< relref "/docs/bundling/constant-folding" >}}). Defaults from a bitness-pinned `#Requires` when present. |

## `ahkbuild bundle exe`

Bundles a script into a standalone Windows `.exe`. Most settings come from
[`ahkbuild.json`]({{< relref "/docs/reference/config" >}}); the flags below override the
corresponding config fields per-invocation. See the [exe target]({{< relref "/docs/exe" >}}).

```bash
ahkbuild bundle exe [flags]
```

| Flag | Overrides | Description |
| --- | --- | --- |
| `--config <path>` | - | Path to `ahkbuild.json`. Discovered by walking up from the cwd if omitted. |
| `--input <path>` | `entry` | Entry script. |
| `--output`, `-o <path>` | - | Output file. Defaults to `<exe.name>.exe`, else `<entry-stem>.exe`. |
| `--interpreter-version <v>` | `interpreter.version` | AHK version to build against. |
| `--bitness <32\|64>` | `interpreter.bitness` | Target architecture. |
| `--no-tree-shake` | - | Disable [tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}}). |
| `--keep-comments` | - | Keep comments in the embedded scripts. |

> [!NOTE]
> `bundle exe` requires Windows. See [PE manipulation]({{< relref "/docs/internals/pe-manipulation" >}})
> for why.

## `ahkbuild interpreter`

Manages the cached AutoHotkey [interpreters]({{< relref "/docs/exe/interpreters" >}}) under
`~/.ahkbuild/interpreters/`.

```bash
ahkbuild interpreter install <version> [--bitness 32|64]
ahkbuild interpreter list
ahkbuild interpreter prune [--version <v>] [--bitness 32|64]
```

| Subcommand | Description |
| --- | --- |
| `install <version>` | Download or build an interpreter into the cache. `--bitness` limits it to one architecture; both are cached otherwise. |
| `list` | Show the cached versions and their bitnesses. |
| `prune` | Remove cached interpreters. `--version` and `--bitness` narrow what is removed; everything is removed if both are omitted. |

## `ahkbuild package`

Manages [module dependencies]({{< relref "/docs/dependencies" >}}) declared in
[`ahkbuild.json`]({{< relref "/docs/reference/config#dependencies" >}}): resolves them to an
`ahkbuild.lock`, populates the content-addressed store under `~/.ahkbuild/packages/`, and builds the
per-project `.ahkbuild/modules/` link-farm.

```bash
ahkbuild package restore [--config <path>] [--locked]
```

| Subcommand | Description |
| --- | --- |
| `restore` | Resolve, pin, and fetch dependencies, then (re)build the link-farm. |

| Flag | Description |
| --- | --- |
| `--config <path>` | Path to `ahkbuild.json`. Discovered by walking up from the cwd if omitted. |
| `--locked` | CI mode: fail if `ahkbuild.lock` is missing or would change, instead of updating it. The store may still be populated from the existing lock. |

## `ahkbuild run`

Runs an entry script under the project's configured interpreter with dependencies resolved:
restores dependencies, resolves (and auto-installs) the interpreter, points
[`AhkImportPath`]({{< relref "/docs/dependencies" >}}) at the link-farm, and launches the script.

```bash
ahkbuild run [entry] [flags] [-- <script args>]
```

| Argument | Description |
| --- | --- |
| `entry` | Entry script. Overrides the `entry` field in `ahkbuild.json`. |
| `args` | Everything after `--` is passed through to the script. |

| Flag | Overrides | Description |
| --- | --- | --- |
| `--config <path>` | - | Path to `ahkbuild.json`. Discovered by walking up from the cwd if omitted. |
| `--interpreter-version <v>` | `interpreter.version` | AHK version to run under. |
| `--bitness <32\|64>` | `interpreter.bitness` | Target architecture. |
| `--validate` | - | Load the script, but do not execute it. Runs the interpreter with [`/Validate`], so can be used to check for load-time errors. |

[`/Validate`]: https://www.autohotkey.com/docs/alpha/Scripts.htm#validate

## Global Flags

### Logging

| Flag | Description |
| --- | --- |
| `-v`, `--verbose` | Raise verbosity. Repeatable: `-v` = info, `-vv` = debug (also shows each log's source), `-vvv` = trace. Default shows warnings and errors only. |
| `-q`, `--quiet` | Suppress everything except errors. Takes precedence over `-v`. |
| `--log-file <path>` | Additionally write a timestamped, debug-level log to `<path>` (ANSI-free), regardless of console verbosity. None by default. |

For fine-grained, per-module control, set the `AHKBUILD_LOG` environment variable (or the
conventional `RUST_LOG`) to a [`tracing` env-filter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
directive. When set, it overrides the `-v` / `--quiet` level for the console:

```bash
# Debug-level logs from the linker only; everything else stays quiet.
AHKBUILD_LOG=ahkbuild_link=debug ahkbuild bundle ahk app.ahk

# Trace everything.
AHKBUILD_LOG=trace ahkbuild bundle exe
```
