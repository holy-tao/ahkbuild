---
title: Icon internals
weight: 4
---

# Icon internals

Replacing an exe's icon sounds trivial - swap one `.ico` for another - but the AHK interpreter ships
with several **built-in icon groups** the runtime uses for tray state (running, paused, suspended)
and for Gui windows. Clobber the wrong one and a running script loses its tray icon. This page covers
how [`exe.icon`]({{< relref "/docs/exe/resources#icons" >}}) and
[`resources.icons`]({{< relref "/docs/exe/resources#icons" >}}) avoid that.

## Built-in groups

Windows icons come in two resource types: `RT_GROUP_ICON` (a directory) and `RT_ICON` (the
individual images each group points at). On alpha.22 and alpha.30, the interpreter's
`RT_GROUP_ICON` ids are `159, 160, 206, 207, 208`, with `RT_ICON` image ids running up to `27`.

The **primary** group - the one Windows shows as the application icon, and the one Ahk2Exe replaces -
is `159`, the *lowest-numbered* group. `ahkbuild` therefore targets the lowest-numbered group rather
than a hard-coded `159`, so it adapts if the interpreter's layout shifts in a future version.

`exe.icon` replaces only that primary group; every other built-in group is left untouched. Group
159's member `RT_ICON` ids are `1..8`; the replacement reuses those ids and mints any extra images it
needs *above* the highest existing `RT_ICON` id, so no built-in (tray/Gui) images are ever overwritten.

## Why explicit IDs?

[`resources.icons`]({{< relref "/docs/exe/resources#icons" >}}) makes you choose a resource id for
each extra icon, and you load it by the **negative** form (`LoadPicture(.., "Icon-300")`). This avoids
assuming a particular icon layout for the icons in the AHK binary.

Empirically, against `2.1-alpha.30`, the positive ordinals `Icon1..Icon5` map to the five built-in
groups (`159, 160, 206, 207, 208`), and `Icon6+` fail until more groups are added. In other words,
**`IconN` indexes groups, not the individual `RT_ICON` images** (which run `1..27`).

Because the built-ins occupy the low ordinals - and the primary group *must* stay lowest so Windows
shows `exe.icon` as the app icon - there is no way to expose an appended icon as a clean `Icon1` /
`Icon2`. So, like Ahk2Exe's `;@Ahk2Exe-AddResource icon.ico, 160`, each icon gets an explicit
resource id and scripts load it by the negative form. A resource id is stable across interpreter
versions; a positive ordinal would shift the moment the built-in group count changed.

## Validation and the shared allocator

At bundle time each configured id is checked: it must not collide with a built-in group (that would
overwrite a tray/Gui icon) or with another configured icon, and an id *below* the primary group is
warned about (it would hijack the application icon).

All the new `RT_ICON` images - both the `exe.icon` replacement and every `resources.icons` entry -
are minted from **one shared id allocator** that starts above the interpreter's highest existing
image id. That single allocator is what guarantees nothing ever clobbers a built-in image, no matter
how many icons you add.
