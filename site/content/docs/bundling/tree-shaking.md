---
title: Tree-shaking
weight: 2
---

# Tree-shaking

[Tree-shaking] removes code your program never uses from the bundle. If you `#Import` a large library
but only call one function from it, tree-shaking drops the rest - functions, classes, even whole
modules that nothing reachable refers to.

It is on by default for both [targets]({{< relref "/docs/bundling" >}}). Disable it for a build with
[`--no-tree-shake`]({{< relref "/docs/reference/cli" >}}).

```autohotkey
#Import "BigLib"   ; exports Used() and a hundred other things

Used()
```

Only `Used()` (and whatever *it* needs) is included in the bundle; the rest of `BigLib` is dropped.

[tree-shaking]: https://en.wikipedia.org/wiki/Tree_shaking

## It is conservative by design

AutoHotkey is dynamically typed, so the bundler cannot always tell what a piece of code refers to.
Tree-shaking errs on the side of **keeping** anything that *might* be reached - a bundle may still
contain code you think is dead, but it will never remove code that could actually run.

Removal is **name-based**, not type-based. The bundler builds a table of every name the program
could reference, then walks outward from the [entry points](#entry-points), marking everything it
can reach. At the end of this process, anything that isn't marked is dropped. There is no type
inference: if the name `.ToString` appears *anywhere*, every class keeps its `ToString` member,
even on unrelated types.

## Removal Candidates

`ahkbuild`'s tree-shaking algorithm can remove:

- **Functions** - a top-level function nothing live calls.
- **Classes** - a class that is unreferenced by live code is removed.
- **Class / struct members** - individual methods, properties, and fields whose name never appears
  in any member access in the program.
  - Struct *fields* are always kept, as removing them would change the layout and break ABI compatibility.
- **`#Import` directives** - if an import's bound name is never used, the directive is dropped. A
  module reached *only* through unused imports is never loaded and shakes out entirely.
- **Modules** - a `#Module` block or imported file where nothing survives.
- **Labels** - an unreferenced label (those in the auto-execute section are always kept).
- **Dead branches** - an `if` / ternary arm that [constant folding]({{< relref "/docs/bundling/constant-folding" >}})
  proved can never run, plus any declarations only that arm used.

## Reachability

These are the entry points and patterns that keep code in the bundle. If something you expected to
be removed survives, it is almost always one of these.

### Entry points

Reachability starts from code that runs without being called:

| Always live | Why |
| --- | --- |
| Top-level statements | The auto-execute section runs at startup. |
| Hotkey & hotstring bodies | User-triggered; they must be preserved. |
| [`#HotIf`](https://www.autohotkey.com/docs/v2/lib/_HotIf.htm) expressions | Evaluated at runtime to decide hotkey context. |
| Classes with `static __New()` | Run automatically when the class is declared. |

Top-level function and class declarations are **not** entry points - they survive only if reachable
code refers to them.

### Dynamic access

A [dereference] or [dynamic reference] with *no constant parts* will defeat static analysis entirely. If the
bundler encounters any of these, member pruning is disabled altogether.

A member access with no constant part like `obj.%someVar%` means *any* member could be the target, so
the bundler **disables member pruning** and keeps every class whole. The same applies to the
[reflection functions](#reflection-functions) below when called with a non-literal name.

Partial names still help: `obj.Get%suffix%` keeps every member *starting with* `Get`, and
`obj.%prefix%Handler` keeps every member *ending with* `Handler`. Dynamic names containing string literals
(or expressions which fold to string literals) are also supported. That said, any dynamic expression is
liable to over-keep live code.

If a dynamic access is known to resolve to a limited set of constant values, use the `;@ahkbuild-resolves-to`
directive to enumerate them.

[dereference]: https://www.autohotkey.com/docs/alpha/Variables.htm#deref
[dynamic reference]: https://www.autohotkey.com/docs/alpha/Language.htm#dynamic-variables

### Reflection functions

These take a member name as a string, so the bundler reads their literal arguments and keeps the
named member alive:

- [`ObjBindMethod`](https://www.autohotkey.com/docs/v2/lib/ObjBindMethod.htm)
- [`GetOwnPropDesc`](https://www.autohotkey.com/docs/v2/lib/Object.htm#GetOwnPropDesc)
- [`GetMethod`](https://www.autohotkey.com/docs/v2/lib/GetMethod.htm)

`ObjBindMethod(obj, "Refresh")` is constant and thus keeps `Refresh`. `ObjBindMethod(obj, name)`, a variable
keeps *everything* (see [above](#dynamic-access)). `ObjBindMethod(obj, "On" event)` binds the prefix `On`.

### Protected meta-functions

These are invoked implicitly by the runtime, so they are **never** pruned from a live class even if
their name never appears anywhere:

- [`__New`](https://www.autohotkey.com/docs/v2/Objects.htm#Custom_NewDelete)
- [`__Delete`](https://www.autohotkey.com/docs/v2/Objects.htm#Custom_NewDelete)
- [`__Call`](https://www.autohotkey.com/docs/v2/Objects.htm#Meta_Functions)
- [`__Get`](https://www.autohotkey.com/docs/v2/Objects.htm#Meta_Functions)
- [`__Set`](https://www.autohotkey.com/docs/v2/Objects.htm#Meta_Functions)
- [`__Item`](https://www.autohotkey.com/docs/v2/Objects.htm#__Item)
- [`__Enum`](https://www.autohotkey.com/docs/v2/Objects.htm#__Enum)
- [`Call`](https://www.autohotkey.com/docs/v2/Functions.htm#DynCall)

In v2.1-alpha versions:

- [`__value`](https://www.autohotkey.com/docs/alpha/Structs.htm#abstract)
- [`__Ref`](https://www.autohotkey.com/docs/alpha/lib/Struct.htm#__Ref)

`Call` is protected because invoking any non-function object implicitly calls its `.Call()`.

## `DefineProp`

A [`DefineProp`](https://www.autohotkey.com/docs/v2/lib/Object.htm#DefineProp) call with a string
literal name is treated like any other member: if that property name is never referenced elsewhere in
the program, the whole call is removed - and anything only its descriptor referenced becomes dead
too. Calls are kept when the name is not a literal, is a [protected meta-function](#protected-meta-functions),
or the call is chained / embedded in a larger expression rather than a standalone statement.

## Forcing code to be kept

You can prevent code from being pruned with the [`;@AhkBuild-Keep`]({{< relref "/docs/bundling/directives" >}})
Code marked this way is never removed regardless of what static analysis says.
