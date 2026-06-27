# ahkbuild

`ahkbuild` is a module-aware command-line build tool for AutoHotkey v2.1+. Bundle a script and its
`#Import` graph into a single `.ahk` file, or a standalone Windows `.exe` - no separate AutoHotkey
install required to run it.

## Installation

```shell
cargo install --path crates/cli
```

Or build from source and run from the repo root:

```shell
cargo build --release
./target/release/ahkbuild --help
```
