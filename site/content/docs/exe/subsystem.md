---
title: Subsystem (gui/console)
weight: 5
---

# Subsystem

Every Windows executable declares a [subsystem] that tells the loader whether it is a GUI program or a
console program. `ahkbuild` sets it from[ ]`exe.subsystem`]({{< relref "/docs/reference/config#exe" >}}):

> [!NOTE]
> AutoHotkey uses the gui subsystem by default.

```json
{
  "exe": {
    "subsystem": "gui"
  }
}
```

| Value | Meaning |
| --- | --- |
| `"gui"` (default) | A windowed program. No console is created; the exe runs detached from any terminal that launched it. |
| `"console"` | A console program. Windows allocates a console at launch, giving the script working `stdin`, `stdout`, and `stderr`. |

[subsystem]: https://learn.microsoft.com/en-us/cpp/build/reference/subsystem-specify-subsystem

## When to use `console`

Choose `"console"` for command-line tools - anything that reads from standard input or prints to
standard output, e.g. with `FileOpen("*", "w")` or `FileAppend(text, "*")`. With the default `gui`
subsystem those handles aren't connected to a terminal, so output goes nowhere and a launching shell
returns immediately instead of waiting for the program to finish.

Choose `"gui"` (or just omit the field) for normal GUI scripts, tray apps, and hotkey scripts. A GUI
exe started from a console returns the prompt straight away rather than blocking it.

> [!NOTE]
> The subsystem is a property of the exe, not of how it draws. A `gui` exe can still open a console
> on demand (e.g. with `DllCall("AllocConsole")`), and a `console` exe can still create GUI windows.
> The setting only controls what Windows does *at launch*.
