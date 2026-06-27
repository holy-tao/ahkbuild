---
title: Your first exe
weight: 2
---

# Your first exe

This walkthrough takes you from an empty folder to a running `.exe`. It assumes you've already
[installed `ahkbuild`]({{< relref "/docs/getting-started/installation" >}}) and are on Windows (the
[exe target]({{< relref "/docs/exe" >}}) requires it).

## 1. Write a script

Create `main.ahk`:

```autohotkey
#Requires AutoHotkey v2.1-alpha.30

MsgBox("Hello from a bundled exe!")
```

## 2. Add a config file

`ahkbuild bundle exe` reads its settings from an
[`ahkbuild.json`]({{< relref "/docs/reference/config" >}}) in your project. The only required field
is the [interpreter]({{< relref "/docs/exe/interpreters" >}}) version. Create `ahkbuild.json` next to
your script:

```json
{
  "entry": "main.ahk",
  "interpreter": { "version": "2.1-alpha.30" }
}
```

## 3. (Optional) Install the interpreter

`ahkbuild` builds your exe from a cached copy of the AutoHotkey interpreter. `bundle exe` downloads
the one you configured automatically if it isn't cached yet, so this step is optional - but you can
fetch it ahead of time:

```bash
ahkbuild interpreter install 2.1-alpha.30
```

See [Interpreter management]({{< relref "/docs/exe/interpreters" >}}) for how the cache works.

## 4. Bundle

From the project folder, run:

```bash
ahkbuild bundle exe
```

`ahkbuild` discovers `ahkbuild.json` by walking up from the current directory, resolves the
`#Import` graph, [tree-shakes]({{< relref "/docs/bundling/tree-shaking" >}}) dead code, and writes
the executable. With no `exe.name` configured, the output is named after the entry script -
`main.exe`.

## 5. Run it

```bash
./main.exe
```

You should see your message box. That's a complete build.

## Next steps

Flesh out `ahkbuild.json` to make a real release. A fuller config:

```json
{
  "entry": "main.ahk",
  "interpreter": { "version": "2.1-alpha.30", "bitness": 64 },
  "exe": {
    "name": "MyApp",
    "version": "1.0.0.0",
    "description": "My application",
    "icon": "assets/icon.ico"
  }
}
```

From here:

- [`ahkbuild.json` reference]({{< relref "/docs/reference/config" >}}) - every configuration field.
- [Version info]({{< relref "/docs/exe/version-info" >}}) - product name, version, and metadata.
- [Embedded resources]({{< relref "/docs/exe/resources" >}}) - icons, `FileInstall`, and arbitrary
  resources.
- [Manifest]({{< relref "/docs/exe/manifest" >}}) - UAC elevation and DPI awareness.
- [Build scripts]({{< relref "/docs/exe/build-scripts" >}}) - run codegen, compression, or signing
  around the build.
