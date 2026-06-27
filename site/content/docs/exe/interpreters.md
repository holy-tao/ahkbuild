---
title: Interpreter management
weight: 6
---

# Interpreter management

`ahkbuild` will aquire AutoHotkey interpreters as required by [config]({{< relref "docs/reference/config#interpreter" >}}).
The required interpreter is defined in your configuration file under the `interpreter` section:

```json
{
  "interpreter": {
    "version": "2.0.26",
    "bitness": 64, // defaults to 64 if not specified
  }
}
```

Interpreter version and optionally bitness can also be specified on the command line:

```bash
ahkbuild bundle exe script.ahk --interpreter-version "2.1-alpha.25" --bitness 32
```

## Interpreter aquisition

When bundling an `.exe` file, the configured interpreter is automatically installed if it isn't cached
already. Nothing is done when bundling to an `.ahk` file.

> [!NOTE]
> `ahkbuild` will *not* scan the default AutoHotkey installation folder or your `$PATH` to find AutoHotkey
> executables. It will only ever use its managed interpreters.

`ahkbuild` will try the following, in order, to install an AutoHotkey interpreter:

1. [https://www.autohotkey.com/download/](https://www.autohotkey.com/download/) - AutoHotkey.com's
   CloudFlare config may block this if it thinks you're a bot - this is common in ci/cd contexts
2. AutoHotkey's [GitHub releases](https://github.com/autohotkey/autohotkey/releases). v2.0 interpreters can
   be aquired reliably from here.
3. Compiling from source - `ahkbuild` will check out the repository at the specified version tag and compile
   directly from source. This requires `MSVC` to be installed and discoverable via `vswhere`.

Managed interpreters are saved to `~/.ahkbuild/interpreter/<version>/`. Each folder contains one or
both of `AutoHotkey32.exe` and `AutoHotkey64.exe`.

## Manual interpreter management

You can add or remove cached interpreters using the `ahkbuild interpreter` command:

```bash
# List the installed interpreters
ahkbuild interpreter list
```

```bash
# Install an interpreter
ahkbuild interpreter install 2.1-alpha.30
```

```bash
# Remove interpreters
ahkbuild interpreter prune
```
