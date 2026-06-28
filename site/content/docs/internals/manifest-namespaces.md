---
title: Manifest namespaces
weight: 3
---

# Manifest namespaces

The [application manifest]({{< relref "/docs/exe/manifest" >}}) edits look simple from the
[config]({{< relref "/docs/reference/config#exemanifest" >}}) side - set a UAC level, flip a DPI
flag - but the manifest is XML, and XML namespaces make it a minefield. This page is the *why*
behind the surgical-edit approach.

## Why not regenerate the manifest

`ahkbuild` never rebuilds the manifest from scratch, and never does a full DOM parse-and-reserialize
round-trip. The interpreter's shipped manifest carries declarations the runtime relies on - the
`comctl32 v6` dependency that gives you themed common controls, and the `supportedOS` / DPI entries -
and dropping or reordering any of them risks breaking themed controls or changing behavior.

Thus, `manifest.rs` reads the manifest bytes (read-only, via editpe), applies **minimal string surgery**
for only the fields you configured, and writes the result back through the same `UpdateResource` pass
as everything else, under the manifest's existing language (`0x0409` on alpha.30) so it *replaces*
rather than *duplicates*.

## The namespace gotcha

`<windowsSettings>` declares the 2016 WindowsSettings namespace as its default. The trap: several of
its children live in *other* namespaces and must declare those explicitly, or the side-by-side (SxS)
loader **rejects the exe at launch** with `<element> is not registered` and the generic *"the
application has failed to start because its side-by-side configuration is incorrect"*. That's a hard
launch failure, not a warning.

Each settings element belongs to a specific namespace year:

| Element | Namespace |
| --- | --- |
| `dpiAware` | 2005 |
| `dpiAwareness` | 2016 (the `<windowsSettings>` default) |
| `longPathAware` | 2016 (default) |
| `gdiScaling` | 2017 |

The interpreter already includes `dpiAware`, `dpiAwareness`, and `longPathAware`, so for those
`ahkbuild` only needs to *edit existing text* - the namespace is already declared correctly.
`gdiScaling`, however, is typically **inserted** fresh, so the emitter attaches its 2017 `xmlns`
when it writes the element.
