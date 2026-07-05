---
title: Build directives
weight: 4
---

# Build directives

Build directives are special comments that steer `ahkbuild`'s static analysis from inside your
source. Syntactically, these are comments, so a script carrying them still runs unchanged under a normal
AutoHotkey install.

Directives are case-insensitive and attach to the statements that directly follow them.

| Directive | Purpose | Documented on |
| --- | --- | --- |
| `;@AhkBuild-Keep` | Force a statement to survive [tree-shaking](#ahkbuild-keep). | This page |
| `;@AhkBuild-ResolvesTo` | Name the targets of a [dynamic member access](#ahkbuild-resolvesto). | This page |
| `;@AhkBuild-Safe` | Vouch that a [dynamic reference](#ahkbuild-safe) won't defeat analysis. | This page |
| `;@ahkbuild-const` | Assert a value is a [build-time constant]({{< relref "/docs/bundling/constant-folding#the-ahkbuild-const-directive" >}}). | [Constant folding]({{< relref "/docs/bundling/constant-folding" >}}) |
| `;@AhkBuild-IgnoreBegin` / `End` | Strip a region of source before parsing. | [Preprocessing]({{< relref "/docs/bundling/preprocessing" >}}) |

## `;@AhkBuild-Keep`

Prevents the statement that follows from being [tree-shaken]({{< relref "/docs/bundling/tree-shaking" >}})
out, no matter what reachability analysis concludes. This is an escape hatch for when code could be reachable
from a mechanism that the bundler can't see.

```autohotkey
;@AhkBuild-Keep
Unreferenced() => LogError("Kept even though nothing calls it")
```

> [!NOTE]
> `;@AhkBuild-Keep` only keeps the marked statement itself - it does **not** change analysis of what
> that statement references. In the example above, if `LogError` is never referenced anywhere else,
> it is still pruned. Mark anything that must come along too.

## `;@AhkBuild-ResolvesTo`

A fully [dynamic member access]({{< relref "/docs/bundling/tree-shaking#dynamic-access" >}}) with no
constant parts like `obj.%myVar%` normally forces the bundler to give up on member pruning and keep
every class whole. This directive lets you name the members the expression could resolve to, so
pruning can continue:

```autohotkey
;@AhkBuild-ResolvesTo One Two Three
return myObj.%myVar%
```

The argument is a list of names separated by whitespace or commas; wrap a name in quotes to keep it
whole. Only those members (plus any referenced the normal way) are kept alive; the rest can still be
pruned.

> [!NOTE]
> The directive applies only when the access is **fully dynamic**. If the expression already has an
> extractable constant part (`obj.Get%x%`, `ObjBindMethod(obj, "On" name)`), the bundler uses that
> instead and the directive is ignored.

## `;@AhkBuild-Safe`

When you know a dynamic reference is safe - it will only ever resolve to code that survives on its own,
as is common when building or reflecting over types at runtime - you can promise this to the bundler by
marking the statement `;@AhkBuild-Safe`. This prevents the reference from blowing up member pruning.

```autohotkey
cls := BuildClass(defn)
;@AhkBuild-Safe
cls.%member% := handler
```

Unlike [`;@AhkBuild-ResolvesTo`](#ahkbuild-resolvesto), it does not name any members - it asserts the
reference contributes no reachability edges at all, so analysis continues as if it wasn't there.

> [!WARNING]
> This is a promise, not a hint. If the reference actually reaches code that nothing else keeps, that
> code shakes out and the bundle breaks at runtime. Use [`;@AhkBuild-ResolvesTo`](#ahkbuild-resolvesto)
> instead when you *can* enumerate the targets, and [`;@AhkBuild-Keep`](#ahkbuild-keep) to pin anything
> the reference relies on.

Without the directive, the bundler logs a warning naming the file and line whenever a dynamic reference
disables pruning, so you can find the accesses worth annotating. Run with `-v`/`-vv` (or
`AHKBUILD_LOG=ahkbuild_shake=debug`) to trace every shake decision.
