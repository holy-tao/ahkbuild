---
title: Trusting packages
weight: 5
---

# Trusting packages

[Tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}}) is conservative about
[dynamic access]({{< relref "/docs/bundling/tree-shaking#dynamic-access" >}}): a `%deref%`, dynamic
member access, or dynamic call with no constant parts makes the bundler keep the whole module (or
disable member pruning) rather than risk dropping code the dynamic reference might reach.

For your own code you promise the reference is safe in-source with
[`;@AhkBuild-Safe`]({{< relref "/docs/bundling/directives#ahkbuild-safe" >}}). But you can't edit a
dependency: packages live in an immutable, content-addressed store, so annotating one would mean
forking it. `ahkbuild.trust.json` is the out-of-source equivalent - it lets you vouch for a
dependency's dynamic code from *your* project.

> [!TIP]
> Directives are for code you own; the trust file is for code you don't. If you can enumerate the
> targets of a dynamic access, prefer
> [`;@AhkBuild-ResolvesTo`]({{< relref "/docs/bundling/directives#ahkbuild-resolvesto" >}}) instead -
> it keeps only the named members alive rather than the whole module.

## The trust file

`ahkbuild.trust.json` lives at the project root, beside `ahkbuild.json` and `ahkbuild.lock`, and is
committed - it is trust data you author, not generated state.

```json
{
  "version": 1,
  "trust": [
    {
      "package": "SomeLib",
      "checksum": "sha256:abc123…",
      "files": ["src/dynamic.ahk"],
      "reason": "vetted: %deref% only indexes a fixed internal method table"
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `package` | The dependency's manifest key (the same name used in `ahkbuild.json` and the lockfile). |
| `checksum` | The `sha256:…` the entry was vouched against. Required. |
| `files` | Package-relative paths to trust. Omit it (or use `["*"]`) to trust the whole package. |
| `reason` | Optional note explaining why the dynamic code is safe. |

## Adding an entry

The easiest way to add trust is the CLI, which fills in the current checksum for you:

```shell
ahkbuild package trust SomeLib src/dynamic.ahk --reason "vetted: fixed method table"
```

With no files, the whole package is trusted:

```shell
ahkbuild package trust SomeLib
```

The dependency must already be pinned (run [`ahkbuild package restore`]({{< relref "/docs/reference/cli#ahkbuild-package" >}})
first). `path` dependencies are mutable, so they can't be trusted this way - annotate their dynamic
code in-source with `;@AhkBuild-Safe` instead.

## Version scoping

The recorded `checksum` binds the trust to the exact bytes you reviewed. When a package moves - an
[`update`]({{< relref "/docs/reference/cli#updating-packages" >}}), a re-resolve - its checksum
changes and the trust entry no longer matches. The bundler then ignores it (with a warning) and
falls back to the conservative behavior until you vet and re-trust the package:

```shell
ahkbuild package trust SomeLib src/dynamic.ahk   # re-records the new checksum
```

## Granularity

Trust is per-package and per-file. Trusting a package (or a file within it) tells the bundler that
*every* dynamic construct in those files is safe - it is not scoped to a single line or expression.
Keep the `files` list as narrow as you can so the promise covers only the code you actually reviewed.

> [!WARNING]
> A trust entry is a promise, exactly like [`;@AhkBuild-Safe`]({{< relref "/docs/bundling/directives#ahkbuild-safe" >}}).
> If a trusted file's dynamic code reaches code that isn't reached by anything else, the code will
> still be shaken out and your bundle will break at runtime. Only trust packages that you actually, uhh,
> trust.
