# ahkbuild
Build tools for AHK - Work in progress.

These are all command line tools, written in AutoHotkey. Is it the correct tool for the job? No.

There's two tools right now - a preprocessor and a bundler

### Dependencies

The only external dependency that isn't included as a submodule is the tree-sitter binaries:
- [tree-sitter-autohotkey](https://github.com/holy-tao/tree-sitter-autohotkey) for the actual grammar
  - This dll, as well as the tree-sitter runtime, must be present in the ./bin/ directory

# Directives

The preprocessor and build scripts have the following directives. Directives apply at the expression level. If there is more than one statement in an expression which a directive could apply to, it applies to all of them.

## Preprocessor

#### `@AhkBuld-IgnoreBegin` / `;@AhkBuild-IgnoreEnd`

```autohotkey
;@AhkBuild-IgnoreBegin
OutputDebug("About to do something. Inputs: " String(inputs) "`n")
;@AhkBuild-IgnoreEnd
```

Code phyically present between these directives is ignored and not copied to the final processed script. The behavior of these directives is identical to that of the [`@Ahk2Exe-IgnoreBegin/End`](https://www.autohotkey.com/docs/v2/misc/Ahk2ExeDirectives.htm#IgnoreKeep) directives, which the preprocessor also respects.

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