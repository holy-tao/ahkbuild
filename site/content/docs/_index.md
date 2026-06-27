---
title: Documentation
bookFlatSection: false
weight: 1
---

# Documentation

`ahkbuild` bundles an AutoHotkey v2.1 script and its `#Import` graph into a single `.ahk` file or a
standalone Windows `.exe`. These docs cover everything from your first build to the design rationale
behind the bundler.

New here? Start with [Getting started]({{< relref "/docs/getting-started" >}}), or jump straight to
[Your first exe]({{< relref "/docs/getting-started/quickstart-exe" >}}).

## Sections

- **[Getting started]({{< relref "/docs/getting-started" >}})** - install `ahkbuild` and build your
  first executable.
- **[Bundling]({{< relref "/docs/bundling" >}})** - what the bundler does to your code:
  preprocessing, [tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}}),
  [constant folding]({{< relref "/docs/bundling/constant-folding" >}}), and
  [build directives]({{< relref "/docs/bundling/directives" >}}).
- **[Exe target]({{< relref "/docs/exe" >}})** - building a standalone `.exe`: icons, version info,
  manifests, embedded resources, build scripts, and interpreter management.
- **[Reference]({{< relref "/docs/reference/cli" >}})** - the [CLI]({{< relref "/docs/reference/cli" >}})
  and the [`ahkbuild.json`]({{< relref "/docs/reference/config" >}}) schema.
- **[Coming from Ahk2Exe]({{< relref "/docs/coming-from-ahk2exe" >}})** - a directive-by-directive
  mapping for anyone migrating an existing project.
- **[Internals]({{< relref "/docs/internals" >}})** - design rationale and contributor docs; not
  needed to use the tool.

> [!NOTE]
> Bundling to a single `.ahk` file works on any platform. Building an `.exe` currently requires
> Windows - see [the exe target]({{< relref "/docs/exe" >}}).
