# ahkbuild
Module-aware build tools for AutoHotkey v2.0 / v2.1


> [!IMPORTANT]
> Work in progress. Expect bugs (and pleas report them!)

## Bundler

### Tree-shaking directives

#### `;@AhkBuild-Keep`
```autohotkey
;@AhkBuild-Keep
Unreferenced() => LogError("This code is unreachable!")
```

Prevents the statement that follows from ever being pruned, regardless of reachability analysis or name references. This has no effect on static analysis. If a statement that is would otherwrise be pruned is kept this way, the names it references may be pruned, and executing it may still be unsafe. In the statement above, if `LogError` is not referenced anywhere else, it would still be pruned, even though `Unreferenced` is kept.

#### `;@AhkBuild-ResolvesTo`

```autohotkey
;@AhkBuild-ResolvesTo One Two Three
return myObj.%myVar%
```

Specify a space-delimited list of values that a fully dynamic [dereference](https://www.autohotkey.com/docs/v2/Variables.htm#deref) or reflection-like method (e.g. `HasMethod`) could resolve to. This prevents member pruning from aborting when the expression is encountered. If the dereference is *not* fully dynamic, this directive is ignored.