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
  - [Updating Packages](#updating-packages)
  - [Pruning Packages](#pruning-packages)
  - [Adding and Removing Packages](#adding-and-removing-packages)
  - [Verifying Packages](#verifying-packages)
  - [Trusting Packages](#trusting-packages)
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
ahkbuild package list    [--global] [--config <path>]
ahkbuild package update  [names...] [--config <path>]
ahkbuild package prune   [--dry-run] [--include-untracked]
ahkbuild package add     <name> <--git|--gist|--tarball|--release|--path <value>> [selectors] [--config <path>]
ahkbuild package remove  <names...> [--config <path>]
ahkbuild package verify  [--config <path>]
ahkbuild package trust   <package> [files...] [--reason <text>] [--config <path>]
```

| Subcommand | Description |
| --- | --- |
| `restore` | Resolve, pin, and fetch dependencies, then (re)build the link-farm. |
| `list` | Show each declared dependency with its source, pinned revision, and whether it is fetched into the store and linked into the farm. With `--global`, list the shared store (`~/.ahkbuild/packages/`) instead. |
| `update [names...]` | Re-resolve floating dependencies to their latest remote revision and rewrite the lock. With no names, every updatable dependency is refreshed, otherwise, only the named dependencies are updated. |
| `prune` | Remove unused packages from the global store. |
| `add <name>` | Add a dependency to `ahkbuild.json`. Edits the manifest only; run `restore` to fetch it. |
| `remove <names...>` | Remove dependencies from `ahkbuild.json`, drop their lock entries, and unlink them from the farm. |
| `verify` | Check, offline, that every dependency is pinned and its stored contents still match the lock checksum. |
| `trust <package> [files...]` | Vouch that a dependency's dynamic code is safe so tree-shaking keeps it narrowly. Records the package's current lock checksum in `ahkbuild.trust.json`. With no files, the whole package is trusted. |

| Flag | Description |
| --- | --- |
| `--config <path>` | Path to `ahkbuild.json`. Discovered by walking up from the cwd if omitted. Ignored by `list --global` and `prune`. |
| `--global` | (`list` only) List the shared package store instead of this project's dependencies. |
| `--locked` | (`restore` only) CI mode: fail if `ahkbuild.lock` is missing or would change, instead of updating it. The store may still be populated from the existing lock. |
| `--dry-run` | (`prune` only) Report what would be removed without deleting anything. |
| `--include-untracked` | (`prune` only) Also remove store directories the index has no record of (fetched before the index existed, or by a project not restored since). A project that still needs one but has not been re-restored would lose it, so restore your projects first. |
| `--reason <text>` | (`trust` only) A note recorded with the trust entry explaining why the dynamic code is safe. |

### Updating Packages

Only packages whose sources' revisions "float" are updated:

- `git` packages with any selector but (`rev`)
- `gist` packages with any selecto rbut (`rev`) (recall that gists are just git repositories)

In both cases, `ahkbuild` will update the revision in the lockfile to the latest on the specified branch (or the
repository's default branch).

`tarball`, `release`, and packages pinned to specific revisions are not updated by the `update` command. If you
explicitly name one of these, it is reported as skipped.

### Pruning Packages

The store is shared across every project on the machine. To garbage-collect it safely, ahkbuild keeps
a metadata index (`~/.ahkbuild/packages/index.json`) recording, per stored tree:

- the source/name it was fetched under, and,
- which project root(s) have restored it.

`prune` re-derives the set of live entries by reading every known project's lockfile, then removes the rest. By
default, directories not in the index ("orphaned") are ignored, pass `--include-untracked` to also delete these.

### Adding and Removing Packages

`add` and `remove` edit the `dependencies` table of `ahkbuild.json` for you, preserving the rest of the file's key
order and formatting. Neither touches the network.

`add` takes the package name followed by exactly one source flag and any selectors, mirroring the
[manifest shape]({{< relref "/docs/reference/config#dependencies" >}}):

```bash
ahkbuild package add GuiEnhancerKit --git https://github.com/nperov/GuiEnhancerKit.git --tag v1.0.3
ahkbuild package add cJson --gist 5f2f6f0f... --rev deadbeef
ahkbuild package add Rapid --tarball https://example.com/rapid.zip --sha256 <hash> --subdir src
ahkbuild package add YAML64.ahk --release holy-tao/YAML --tag v0.5.0 --asset YAML64.ahk --sha256 <hash> --alias YAML
ahkbuild package add MyLocal --path ../shared/MyLocal
```

The source object is validated against the same rules `ahkbuild.json` parsing enforces, so a bad
combination (two sources, a `release` missing its `asset`, an invalid `alias`, …) is rejected and the
file is left untouched. `add` only writes the manifest; run `restore` afterwards to fetch and link.
There is no registry, so it does not resolve versions or discover the latest tag - you supply the
source explicitly.

`remove` deletes each named dependency from the manifest, drops its `ahkbuild.lock` entry, and unlinks
it from `.ahkbuild/modules/`. The store copy is left in place (other projects may share it); reclaim it
later with `prune`.

### Verifying Packages

`verify` checks, without fetching, that the project is in a consistent, reproducible state: every
non-`path` dependency is pinned in the lock, present in the store, and its stored tree still hashes to
the lock's checksum (`path` dependencies only need to exist on disk). It exits non-zero if anything is
missing, unpinned, or has drifted - useful in CI alongside `restore --locked`. Run `restore` to repair
whatever it flags.

### Trusting Packages

`trust` records that a dependency's dynamic constructs (`%deref%`, dynamic member access/calls) are
safe, so [tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}}) does not conservatively keep the
whole module. It is the out-of-source equivalent of the
[`;@AhkBuild-Safe`]({{< relref "/docs/bundling/directives#ahkbuild-safe" >}}) directive, for packages
you can't edit. See [trusting packages]({{< relref "/docs/bundling/trust" >}}) for the full model.

```bash
ahkbuild package trust SomeLib src/dynamic.ahk --reason "vetted: fixed method table"
ahkbuild package trust SomeLib                 # trust the whole package
```

The dependency must already be pinned (`restore` first). `trust` writes the package's current lock
checksum into `ahkbuild.trust.json`, so a later `update` that moves the package invalidates the entry
(the bundler warns and ignores it) until you re-run `trust` to re-vouch. `path` dependencies are
mutable and can't be trusted this way - annotate their code in-source with `;@AhkBuild-Safe` instead.

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
