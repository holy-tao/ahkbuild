---
title: Internals
weight: 9
bookCollapseSection: true
---

# Internals

This section documents *how* `ahkbuild` works and, more importantly, *why* it works that way. None
of it is needed to use the tool - the [exe target]({{< relref "/docs/exe" >}}) and
[bundling]({{< relref "/docs/bundling" >}}) pages cover that. It exists for contributors, and for
anyone debugging a surprising result who wants to understand what the bundler did to their script.

Much of the hard-won knowledge here is about working *with* the AutoHotkey interpreter binary rather
than against it: the interpreter is a real, shipped PE file whose resources, manifest, and icon
groups the runtime depends on, and the safest edits are the smallest ones.

- [Architecture]({{< relref "/docs/internals/architecture" >}}) - the crate layout and the
  pass pipeline, including the fixpoint driver.
- [PE manipulation]({{< relref "/docs/internals/pe-manipulation" >}}) - why resources are injected
  with the Win32 `UpdateResource` API instead of a pure-Rust PE library, and the exe emit flow.
- [Manifest namespaces]({{< relref "/docs/internals/manifest-namespaces" >}}) - the XML-namespace
  gotcha behind the surgical manifest edits.
- [Icon internals]({{< relref "/docs/internals/icon-internals" >}}) - how the interpreter's
  built-in icon groups are preserved and how `LoadPicture` addressing was reverse-engineered.
- [Preprocessing]({{< relref "/docs/internals/preprocessing" >}}) - the pure-text phase that runs
  before parsing, and why it exists.
