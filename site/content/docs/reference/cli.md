---
title: CLI
weight: 1
---

# CLI reference

The `ahkbuild` binary has three subcommands:

- [`ahkbuild preprocess`](#ahkbuild-preprocess)
- [`ahkbuild bundle ahk`](#ahkbuild-bundle-ahk)
- [`ahkbuild bundle exe`](#ahkbuild-bundle-exe)
- [`ahkbuild interpreter`](#ahkbuild-interpreter)

The global `-d` / `--debug` flag (repeatable) raises log verbosity for any of them.

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
