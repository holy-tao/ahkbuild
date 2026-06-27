---
title: Manifest (UAC, DPI)
weight: 3
---

# Application manifest

Every Windows executable carries an embedded [application manifest] - an XML blob the OS reads
*before* the program runs to decide things like whether to prompt for administrator rights and how
the program handles high-DPI displays. `ahkbuild` lets you set the most common manifest options
from the [`exe.manifest`]({{< relref "/docs/reference/config#exemanifest" >}}) section of your
config:

```json
{
  "exe": {
    "manifest": {
      "uac": "requireAdministrator",
      "dpiAwareness": "PerMonitorV2",
      "longPathAware": true,
      "gdiScaling": true
    }
  }
}
```

Every field is optional. Omitting a field - or the entire `manifest` block - leaves the
interpreter's default value in place; `ahkbuild` only ever edits the settings you mention specifically.

[application manifest]: https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests

## Fields

| Field | Type | Description |
| --- | --- | --- |
| `uac` | `"asInvoker"` \| `"highestAvailable"` \| `"requireAdministrator"` | The elevation level requested at launch. See [UAC elevation](#uac-elevation). |
| [`dpiAware`] | bool | Legacy system-DPI-aware flag. Prefer `dpiAwareness` on modern systems. |
| [`dpiAwareness`] | string | Modern DPI mode, e.g. `"PerMonitorV2"` or `"system"`. |
| [`longPathAware`] | bool | Opt into file paths longer than `MAX_PATH` (260 characters). |
| `gdiScaling` | bool | Let the system bitmap-scale GDI drawing under DPI virtualization. |

[`longPathAware`]: https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation?tabs=registry
[`dpiAware`]: https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests#dpiaware
[`dpiAwareness`]: https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests#dpiawareness

## UAC elevation

`uac` controls the [`<requestedExecutionLevel>`](https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests#trustinfo)
the OS honors when the exe is launched:

- `"asInvoker"` - The script runs with the same rights as the process that started it. This is the default
  if no `uac` is provided.
- `"highestAvailable"` - Request the highest rights the user can grant. An administrator is prompted to
  elevate while non-admins are not.
- `"requireAdministrator"` - Always run elevated. A non-administrators get a credential prompt.

> [!NOTE]
> This is the OS elevating your program *at launch*, which is different from the typical pattern of
> [relaunching](https://www.autohotkey.com/docs/alpha/lib/_RequireAdmin.htm) with administrator rights.
> A manifest `requireAdministrator` avoids that relaunch entirely. You can check the result at runtime with
> [`A_IsAdmin`](https://www.autohotkey.com/docs/alpha/Variables.htm#RequireAdmin).

## DPI and display

- `dpiAware` / `dpiAwareness` declare how your GUI handles displays scaled above 100%. Without
  them, Windows bitmap-stretches the window and text looks blurry. `"PerMonitorV2"` is the modern
  choice for GUIs that lay out their own controls.
- `longPathAware` opts the process into the extended path limit on Windows 10 1607+.
- `gdiScaling` asks the system to scale legacy GDI output more cleanly under DPI virtualization.
