# ahkbuild
Build tools for AHK - Work in progress.

These are all command line tools, written in AutoHotkey. Is it the correct tool for the job? No.

There's two tools right now - a preprocessor and a bundler

### Dependencies

The only external dependency that isn't included as a submodule is the tree-sitter binaries:
- [tree-sitter-autohotkey](https://github.com/holy-tao/tree-sitter-autohotkey) for the actual grammar
  - This dll, as well as the tree-sitter runtime, must be present in the ./bin/ directory

## Directives

The preprocessor and build scripts have the following directives

### Preprocessor

Directive | Arguments | Description
----------|-----------|-------------
`;@AhkBuild-IgnoreBegin/End` | *none* | Code phyically present between these directives is ignored. The behavior of these directives is identical to that of the [`@Ahk2Exe-IgnoreBegin/End`](https://www.autohotkey.com/docs/v2/misc/Ahk2ExeDirectives.htm#IgnoreKeep) directives, which the preprocessor also respects.

### Bundler

#### Tree-shaking directives

Directive | Arguments | Applies To | Description
----------|-----------|------------|--------------
`;@AhkBuild-Keep` | *none* | All statements | Prevents the statement that follows from ever being pruned. This has no effect on static analysis.