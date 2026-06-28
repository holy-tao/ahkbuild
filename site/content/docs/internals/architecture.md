---
title: Architecture
weight: 1
---

# Architecture

`ahkbuild` is a Rust workspace. The build is a pipeline of small, mostly-pure passes that turn a
source script and its `#Import` graph into either a single `.ahk` file or a standalone `.exe`:

```text
Source -> Preprocess -> Link (CST + IR per file) -> [Fold] -> [Shake] -> Emit -> Output
```

The passes up to and including emit are shared between the two output targets; only the final emit
step differs (see [PE manipulation]({{< relref "/docs/internals/pe-manipulation" >}}) for the exe
side).

## Crates

| Crate | Responsibility |
| --- | --- |
| `syntax` | Thin wrapper around the [`tree-sitter-autohotkey`] grammar; exposes `parse()`. |
| `preprocess` | The pure-text [preprocessing]({{< relref "/docs/internals/preprocessing" >}}) phase: resolves continuation sections and ignore regions before parsing. |
| `ir` | Lowers a tree-sitter CST into the IR (`lower()`); defines the IR node types, the arena, and the `Program` / `Group` / `Lowering` types. |
| `link` | The module-graph linker. Walks the `#Import` graph breadth-first, resolves `#Include`, and assembles a multi-group `Program`. Produces a `BundlePlan` (emission order, module names, resolved imports). |
| `fold` | Build-time [constant folding]({{< relref "/docs/bundling/constant-folding" >}}) and branch resolution. Consumes a `Program` + known `Constants` (`A_IsCompiled`, `A_PtrSize`); produces a `FoldResult`. |
| `shake` | [Tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}}) (dead-code elimination). Consumes a `Program` + `BundlePlan` + optional `FoldResult`; produces a `ShakeResult`. |
| `emit` | Emits the final `.ahk` bundle from a `Program`, `BundlePlan`, and optional `ShakeResult` / `FoldResult`. Also renders per-module source for the exe target. |
| `emit_exe` | The binary side of the exe target: resource injection, version info, manifest, icons, and subsystem patching. Kept separate so the Win32 / PE dependencies stay out of the portable text path. |
| `config` | Parses [`ahkbuild.json`]({{< relref "/docs/reference/config" >}}) into a `BuildConfig`. |
| `interpret` | [Interpreter management]({{< relref "/docs/exe/interpreters" >}}): the `~/.ahkbuild/interpreters/` cache, downloads, and source builds. |
| `pipeline` | The fixpoint build driver that runs the passes to completion (below). |
| `cli` | The `ahkbuild` binary; subcommands `preprocess`, `bundle ahk`, `bundle exe`, `interpret`. Also runs [build scripts]({{< relref "/docs/exe/build-scripts" >}}). |

[`tree-sitter-autohotkey`]: https://github.com/holy-tao/tree-sitter-autohotkey

## The fixpoint driver

The passes are not run just once. Each optimization pass is a pure function of the IR plus side
tables, and the passes compose: tree-shaking can expose more dead code for folding, and inlining
(planned) will expose new folding and shaking opportunities. The `pipeline` crate runs them to a
**fixpoint** with two nested loops, split by whether a pass *removes/substitutes* or *adds*:

- **Inner loop** - the subtractive / substitutive passes (`fold`, `shake`) express their results as
  side tables over the original source spans. They iterate until those tables stop growing.
- **Outer loop** - additive / structural change (inlining) can't be expressed as a span edit on the
  original text, so it is applied by **materializing** the current edits to text, re-parsing,
  re-lowering, and re-running the inner loop on the new tree.

Because the subtractive passes only ever annotate spans, the common case is cheap: parse and lower
once, then iterate side tables to a fixpoint and emit. The expensive re-parse only happens when a
structural pass actually changed the tree.
