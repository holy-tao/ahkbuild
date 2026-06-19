# ahkbuild

Module-aware build tools for AutoHotkey v2.1

> [!IMPORTANT]
> Work in progress. Expect bugs (and please report them!)

`ahkbuild` bundles a v2.1 script and all of its `#Import` dependencies into a
single `.ahk` file (`.exe` output is planned). It resolves the `#Import` graph
across files, lowers each file to an IR, tree-shakes dead code by reachability,
and emits a clean bundle. Written in Rust.

## Installation

```shell
cargo install --path crates/cli
```

Or build from source and run from the repo root:

```shell
cargo build --release
./target/release/ahkbuild --help
```

## Usage

### Preprocess

Run the preprocessor over a source file (resolves continuation sections) and
emit the result. Mostly useful for debugging.

```shell
ahkbuild preprocess source.ahk [out.ahk]
```

If `out.ahk` is omitted the result is printed to stdout.

### Bundle

Bundle a script and its `#Import` graph into a single file.

```shell
ahkbuild bundle ahk <input.ahk> [out.ahk] [--no-tree-shake] [--keep-comments]
```

| Flag | Effect |
| --- | --- |
| `--no-tree-shake` | Disable dead-code elimination; emit a byte-faithful bundle |
| `--keep-comments` | Preserve comments (stripped by default) |

If `out.ahk` is omitted the bundle is printed to stdout.

`.exe` bundling (`ahkbuild bundle exe`) is not yet implemented.

## Build directives

Directives are special comments embedded in the source file that control
`ahkbuild`'s static analysis.

### `;@AhkBuild-Keep`

```autohotkey
;@AhkBuild-Keep
Unreferenced() => LogError("This code is unreachable!")
```

Prevents the statement that follows from ever being pruned, regardless of
reachability analysis or name references. This has no effect on static analysis.
If a statement that would otherwise be pruned is kept this way, the names it
references may still be pruned — in the example above, if `LogError` is not
referenced anywhere else it would still be removed, even though `Unreferenced`
is kept.

### `;@AhkBuild-ResolvesTo`

```autohotkey
;@AhkBuild-ResolvesTo One Two Three
return myObj.%myVar%
```

Specify a space-delimited list of values that a fully dynamic
[dereference](https://www.autohotkey.com/docs/v2/Variables.htm#deref) or
reflection-like method (e.g. `HasMethod`) could resolve to. This prevents
member pruning from aborting when the expression is encountered. If the
dereference is *not* fully dynamic, this directive is ignored.

## Documentation

See `docs/` for design documents:

- [docs/PREPROCESSING.md](docs/PREPROCESSING.md) — preprocessing pipeline
- [docs/TREE_SHAKING.md](docs/TREE_SHAKING.md) — dead-code elimination algorithm
