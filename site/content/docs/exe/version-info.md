---
title: Version info
weight: 1
---

# Version info & metadata

Every Windows executable carries a block of version metadata - the strings you see on the **Details** tab of
a file's Properties dialog, and the fields installers and antivirus tools read to identify a program.
`ahkbuild` writes these from the `exe` section of your [config]({{< relref "/docs/reference/config#exe" >}}).

```json
{
  "exe": {
    "name": "MyApp",
    "version": "1.2.3.0",
    "description": "My application",
    "copyright": "Copyright 2026 Example",
    "company": "Example, LLC",
    "trademarks": "MyApp is a trademark of Example, LLC",
    "comments": "Built with ahkbuild"
  }
}
```

Every field is optional. Any string you leave unset keeps the value already present in the
[interpreter]({{< relref "/docs/exe/interpreters" >}}) you build against, rather than being blanked
out.

## Fields

| Config field | Version-info field(s) | Description |
| --- | --- | --- |
| `name` | `InternalName`, `OriginalFilename` | The program's internal name. Also the default output filename - `bundle exe` writes `<name>.exe` unless you pass `--output`. |
| `version` | `FileVersion`, `ProductVersion` | A four-part `W.X.Y.Z` version number. Defaults to `"0.0.0.0"` if omitted. |
| `description` | `FileDescription` | A short description. This is the text Task Manager and the taskbar show for the process. |
| `copyright` | `LegalCopyright` | Copyright notice. |
| `company` | `CompanyName` | Publisher / author. |
| `trademarks` | `LegalTrademarks` | Trademark notice. |
| `comments` | `Comments` | Free-form comments. |

> [!NOTE]
> `version` must be four dot-separated numbers (`W.X.Y.Z`). This is a Windows requirement for the
> numeric `FileVersion` / `ProductVersion`, not an ahkbuild convention - a value like `"1.2"` or
> `"1.2.3"` is not a complete version. Pad with zeros (`"1.2.0.0"`).
