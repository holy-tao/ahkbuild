---
title: ahkbuild
type: docs
bookToc: false
---

# ahkbuild

`ahkbuild` is a module-aware command-line build tool for AutoHotkey v2.1+. Bundle a script and its
`#Import` graph into a single `.ahk` file, or a standalone Windows `.exe` - no separate AutoHotkey
install required to run it.

It is declarative and configuration-based, built for reproducible project builds and CI/CD pipelines
rather than a right-click-to-compile workflow.

## Why ahkbuild

- **Reproducible, project-oriented builds.** One [`ahkbuild.json`]({{< relref "/docs/reference/config" >}})
  describes the whole build. Clone the repo, run a single command, and get the same output every time.
- **Module-aware.** Resolves the full `#Import` graph across files into one artifact.
- **Optimizing.** [Tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}}) drops unused code and
  [constant folding]({{< relref "/docs/bundling/constant-folding" >}}) resolves build-time branches, making
  large libraries practical.
- **Centralized resources.** Icons, version info, the application manifest, and embedded files are
  all declared in config, not scattered through source comments.
- **Pipeline-friendly.** [Pre- and post-bundle build scripts]({{< relref "/docs/exe/build-scripts" >}})
  hook in codegen, compression, or code signing, and the interpreter is
  [fetched and cached]({{< relref "/docs/exe/interpreters" >}}) automatically.

## Install

```bash
cargo install --path crates/cli
```

See [Installation]({{< relref "/docs/getting-started/installation" >}}) for the full instructions.

## Quick Start

Drop an `ahkbuild.json` next to your script:

```json
{
  "entry": "main.ahk",
  "interpreter": { "version": "2.1-alpha.30" }
}
```

Then build a standalone executable:

```bash
ahkbuild bundle exe
```

That's it - see [Your first exe]({{< relref "/docs/getting-started/quickstart-exe" >}}) for the full
walkthrough.

> [!NOTE]
> Bundling to a single `.ahk` file works anywhere; building an `.exe` currently requires Windows.
