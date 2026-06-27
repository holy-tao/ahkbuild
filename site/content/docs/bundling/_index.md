---
title: Bundling
weight: 2
bookCollapseSection: false
---

# Bundling

Bundling is the process of packaging a script into a single file for distribution. `ahkbuild` can bundle
scripts into two *targets*:

- `.ahk` -- A single, standalone `.ahk` file
- `.exe` -- A "compiled" script, which is distributed with its interpreter and does not require AutoHotkey
  to be separately installed on a user's machine.

In both cases `ahkbuild` will perform a series of optimization passes aimed at reducing the final size of
bundled file.

> [!NOTE]
> Bundling is not likely to provide noticeable performance improvements for your actual script, though it
> may reduce overall memory consumption.

`ahkbuild` is ultimately aimed at producing `exe` bundles.

Bundling is done with the `ahkbuild bundle` command:

```bash
ahkbuild bundle ahk myScript.ahk output.ahk
ahkbuild bundle exe
```
