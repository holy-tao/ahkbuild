---
title: Constant folding
weight: 3
---

# Constant folding & branch shaking

Constant folding evaluates expressions whose value is known at build time and bakes the result into
the bundle. Its main job is to resolve the build-configuration guards `A_IsCompiled` and `A_PtrSize` so
that branches which can't run in this build are pruned before [tree-shaking]({{< relref "/docs/bundling/tree-shaking" >}})
gets to them.

```autohotkey
if (A_IsCompiled)
    path := A_ScriptFullPath
else
    path := RunCodeFromSource()   ; only needed when running from source
```

When you bundle this to an `.exe`, [`A_IsCompiled`] is known to be [`true`], so the `if` statement collapses
to just `path := A_ScriptFullPath`. The `else` arm is gone and `RunCodeFromSource` may be shaken out if no
other code references it.

## Build-time constants

Folding only happens for values the bundler can prove to be constant or which you
[explicitly identify](#the-ahkbuild-const-directive) as constant. Two built-ins are supplied by
configuration:

| Constant | Set by |
| --- | --- |
| [`A_IsCompiled`] | [`--compiled true\|false`]({{< relref "/docs/reference/cli" >}}). **Off by default for the `.ahk` target** (a bundled `.ahk` might later be compiled by ahk2exe, which would flip it). The [`.exe` target]({{< relref "/docs/exe" >}}) defaults it to `true`. |
| [`A_PtrSize`] | [`--bitness 32\|64`]({{< relref "/docs/reference/cli" >}}), or inferred from a bitness-pinned `#Requires` (e.g. `#Requires AutoHotkey v2.1-alpha.3 64-bit` ⇒ `8`). The `.exe` target derives it from the target bitness. |

If a constant isn't known for a build, the built-in is left untouched. If neither is known, the fold
pass doesn't run at all and the output is byte-for-byte unchanged.

[`A_IsCompiled`]: https://www.autohotkey.com/docs/alpha/Variables.htm#IsCompiled
[`A_PtrSize`]: https://www.autohotkey.com/docs/alpha/Variables.htm#PtrSize

## What's folded

Folding records the **largest** constant subexpression it can, then substitutes its value:

```autohotkey
size := A_PtrSize * 8              ; size := 64
lib  := "lib" (A_PtrSize * 8)      ; "lib64"
```

The evaluator handles literals, the constants `true`/`false` (which are just `1`/`0` in AHK),
comparisons, arithmetic, bitwise and logical operators, and [string concatenation](#string-folding). Anything
it can't prove is constant is left alone. Floats are not substituted because the bundler may produce a different
string representation than the AutoHotkey interpreter would.

Note that **Logical operators [short-circuit]**, so `A_IsCompiled && Setup()` folds to `false` when
[`A_IsCompiled`] is false *without* evaluating `Setup()`.

[short-circuit]: https://www.autohotkey.com/docs/alpha/Functions.htm#ShortCircuit

### String folding

`ahkbuild` does not always fold string literals. Since the goal is to ultimately reduce the size of the bundle,
strings are evaluated before folding to see if replacements would reduce the size of the bundle or increase it. A
very long string like a help text literal might *increase* the size of the bundle if it were replaced inline at
every read site, as might a moderately-sized string which is read many times.

## User-defined constants

AHK has no `const` keyword, but most "constants" are names assigned once and never changed. The
bundler detects these and folds their read sites too:

```autohotkey
static FLAG := 0x40000000      ; assigned once and never reassigned
DllCall("...", "uint", FLAG)   ; FLAG folds to 0x40000000
```

Detection is conservative - a name is only folded when it is provably single-assignment with a
constant value. Getter-only fat-arrow properties (`static Value => 42`) are also folded, but only when
accessed as `ClassName.Value`. A bare `obj.Value` can't be proven to mean a member of a particular class.

Detection runs to a fixpoint, so constants defined from other constants resolve (`B := A + 1`).

### The `;@ahkbuild-const` directive

When you know a value is constant but the bundler can't prove it, you can tell it explicitly. The directive
goes on the declaration and skips static analysis:

```autohotkey
;@ahkbuild-const
LOGO := "C:/assets/logo.png"
```
