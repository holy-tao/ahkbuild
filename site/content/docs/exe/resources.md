---
title: Embedded resources
weight: 4
---

# Embedded resources

`ahkbuild` provides a few ways to embed resources in your final executable.

Resource embedding is the final step in the `.exe` bundling process.

## [`FileInstall`]

[`FileInstall`] allows you to embed resources in the bundled `.exe` file and extract them later at runtime to
the user's file system.

`ahkbuild` does not have the same limitations as `Ahk2Exe` regarding the strict requirements for
[*Source*](https://www.autohotkey.com/docs/alpha/lib/FileInstall.htm#Parameters):

> This parameter must be a quoted literal string (not a variable or any other expression), and must be
> listed to the right of the function name FileInstall (that is, not on a continuation line beneath it).

The first argument to [`FileInstall`] can physically located anywhere in the source file, the only
requirement is that it is a constant string literal known at build time. Thus, simple expressions which can
be [shaken out]({{< relref "docs/bundling/tree-shaking" >}}) are allowed so long as they resolve to a string
literal:

```autohotkey
; embed either lib64.bin or lib32.bin
FileInstall("assets/lib" (A_PtrSize * 8) ".bin", "./assets/lib.bin")
```

```autohotkey
; use a different resource when compiled vs running as a script
FileInstall(
     A_IsCompiled ? "assets/config.release.ini" : "assets/config.dev.ini",
     "assets/config.ini"
)
```

Similarly, [foldable constants]({{< relref "docs/bundling/constant-folding" >}}) are also allowed (or
shakeable expressions which resolve to foldable constants -- you get the idea):

```autohotkey
;@ahkbuild-const
LOGO_PATH := "C:/SourceCode/Images/Shared/Logo.png"

FileInstall(LOGO_PATH, A_ProgramFiles "/" A_ScriptName "/logo.png")
```

[`FileInstall`] calls are also eligible to be shaken out themselves. Nothing is embedded for dead calls.

[`FileInstall`]: https://www.autohotkey.com/docs/alpha/lib/FileInstall.htm

## Icons

### The main Icon

The main icon of your executable is specified by path under `exe.icon`:

```json
{
  "exe": {
    "icon": "path/to/icon.ico"
  }
}
```

This icon replaces the default AutoHotkey icon if set.

### Other Icons

Icons are embedded separately from [extras](#extra-resources) to allow for ergonomic loading via
[`LoadPicture`]. They're specified in the `resources.icons` section of the `ahkbuid.json`:

```json
{
  "resources": {
    "icons": [
      { "path": "assets/icon1.ico", "id": 300 }
    ]
  }
}
```

You are required to choose the resource ID of your icons explicitly. At runtime, load these from
the executable with [`LoadPicture`] or any other builtin that supports the `"IconN"` convention:

```autohotkey
icon := LoadPicture(A_IsCompiled ? "Icon-300" : "assets/icon1.ico")
```

These icons *can* overwrite resources that already exist in the executable. You can use this section, for
example, to set the paused and suspended tray icons for your final program.

## Extra Resources

The `ahkbuild` [config]({{< relref "docs/reference/config#resources" >}}) allows you embed arbitrary
resources to embed into an executable. It is also possible to embed [icons](#icons)
with this configuration section, see the documentation above.

> [!IMPORTANT]
> **Icons cannot be embedded with `resources.extra`** - you must use [`resources.icons`](#icons) instead.

The `resources.extra` section is roughly equivalent to `Ahk2Exe`'s [`;@Ahk2Exe-AddResource`] directive.

### Configuration

Every resource in the extra block specifies a name, type, and path:

```jsonc
{
     "resources": {
          "extra": [
               { "name": "HELP", "type": "RT_HTML", "path": "assets/help.html" }   
          ]
     }
}
```

All object keys are required:

| Key | Description |
| --- | --- |
| `name` | the name of the resource, used to retrieve it later at runtime. This must be an all-caps ASCII string. |
| `type` | the all-caps name of a [resource type], or a bare integer to use instead. The `RT_` prefix is optional, and if a string, `type` is case-insensitive. This `"RT_HTML"`, `"html"`, and `23` are equivalent. |
| `path` | the path, relative to the directory containing the configuration file, of the resource to embed. The resource's bytes are copied into the .exe file verbatim, no consideration is given to their contents or the file extension. |

[`;@Ahk2Exe-AddResource`]: https://www.autohotkey.com/docs/alpha/misc/Ahk2ExeDirectives.htm#AddResource
[resource type]: https://learn.microsoft.com/en-us/windows/win32/menurc/resource-types

### Retrieving resources

Unlike [`FileInstall`] or loading embedded icons via [`LoadPicture`], there is no built-in way to retrieve
resources embedded this way.

[`LoadPicture`]: https://www.autohotkey.com/docs/alpha/lib/LoadPicture.htm

<!-- TODO get a working example of DllCall-ing FindResource / LoadResource to extract an embedded resource-->
