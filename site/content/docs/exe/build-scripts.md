---
title: Build scripts
weight: 7
---

# Build scripts

Build scripts are arbitrary commands run before or after bundling happens. Build scripts can be anything.
but are the natural place for things like code generation, compression, or signing.

> [!IMPORTANT]
> Build scripts currently only run for the **exe target** and are ignored when bundling to
> a single .ahk file.

Scripts are defined in [config]({{< relref "/docs/reference/config" >}}) under `scripts`:

```json
{
  "scripts": {
    "pre-bundle": [ 
      ["${AHK}", "/ErrorStdOut=UTF-8", "scripts/codegen.ahk"],
      ["${AHK}", "/ErrorStdOut=UTF-8", "/Validate", "${AHKBUILD_ENTRY}"]
    ],
    "post-bundle": [
      ["signtool", "sign", "/fd", "SHA256", "${AHKBUILD_OUTPUT}"]
    ]
  }
}
```

## Script running

Build scripts are run serially in the order defined. Any script that returns a non-zero exit code aborts the
build, as does any failure to launch a script. Note that there is no timeout for these scripts, a script that
hangs (for example, an Autohotkey script that shows a `MsgBox`) will hang the build.

The current working directory of any build script is always the directory containing the `ahkbuild.json`
file.

## Build script tokens

Build scripts commands can include tokens using the format `${TOKEN_NAME}`. Several of these are supplied by
ahkbuild, but they can also be [defined explicitly]({{< relref "/docs/reference/config" >}}) in config. All
build tokens are available to build scripts as environment variables:

```autohotkey
output := EnvGet("AHKBUILD_OUTPUT")
```

The token `${AHK}` contains the full path to the configured AutoHotkey interpreter, this can be used to run
Autohotkey scripts during the build process. `AHK` is only valid as the first token in the array. The other
tokens are valid anywhere:

| Token | Values | Description |
| --- | --- | --- |
| `AHKBUILD_STAGE` | `"pre"` \| `"post"` | Whether the script is running before or the .exe file has been created |
| `AHKBUILD_TARGET` | `"ahk"` \| `"exe"` | Whether ahkbuild is bundling for .ahk or an executable - because build scripts aren't invoked during .ahk bundling, this is always `.exe` in practice, but is reserved for future use |
| `AHKBUILD_INTERPRETER` | String (path) | Full path to the interpreter being used for bundling |
| `AHKBUILD_BITNESS` | `32` \| `64` | Bitness of the interpreter being used. Note that because Autohotkey build scripts are run with the interpreter being used for bundling, they can also check `A_PtrSize` |
| `AHKBUILD_SUBSYSTEM` | `"gui"` \| `"console"` | The exe subsystem |
| `AHKBUILD_CONFIG_DIR` | String (path) | Full path to the directory containing the ahkbuild configuration file |
| `AHKBUILD_EXE` | String (path) | Full path to the `ahkbuild.exe` doing the bundling - this can be used to re-run the bundler, if necessary |
| `AHKBUILD_VERSION` | String | The version of `ahkbuild.exe` doing the bundling |

Variables can also be defined manually using the ahkbuild.json's [`defines`]({{< relref "/docs/reference/config#defines" >}}) section:

```json
{
  "defines": {
    "CUSTOM": "Any value"
  }
}
```

These values are similarly available as environment variables to build scripts. These are also reserved for
use in future preprocessor directives.s
