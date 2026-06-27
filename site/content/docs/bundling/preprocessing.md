---
title: Preprocessing
weight: 1
---

# Preprocessing

Preprocessing is a pure-text modification phase that runs before the source file is parsed. Preprocessing
consists of two steps:

1. [Continuation Section] Collapsing
2. Code Ignoring

> [!WARNING]
> Because preprocessing modifies the source text of your input file(s), it can change the line numbers at
> which statements appear, which will affect variables like [`A_LineNumber`] and reported line numbers when
> the bundler encounters errors.

You can run the preprocessor alone on a source file with the command.

```shell
ahkbuild preprocess source.ahk [output]
```

This prints the preprocessed script to the specified output file, or `stdout` if `output` is omitted.

[`A_LineNumber`]: https://www.autohotkey.com/docs/alpha/Variables.htm#LineNumber

## Preprocessing steps

### [Continuation Section] Collapsing

The preprocessor will collapse [continuation section]s to a single line of code unconditionally before
parsing the source file. This is done because the tree-sitter grammar cannot reliably parse code
continuation sections, and multiline strings make it slightly more difficult to tell whether a position in a
line is inside a string literal or not (and thus whether it is safe to prune).

Thus, for example, this code snippet:

```autohotkey
( JoinB
Msg
ox "Hello, World!"
)
```

Is collapsed before the source file is parsed into

``` autohotkey
MsgBox "Hello, World!"
```

This allows the parser to correctly identify the section as a call to `MsgBox`. String literals are also
collapsed this way. This behavior cannot be modified or configured. Other forms of continuation, like
[continuation by enclosure], are not collapsed this way.

[continuation section]: https://www.autohotkey.com/docs/alpha/Scripts.htm#continuation
[continuation by enclosure]: https://www.autohotkey.com/docs/alpha/Scripts.htm#continuation-expr

### Code Ignoring (Conditional Compilation)

The file is scanned line-by-line from top to bottom. Any text in between a pair of ignore directives is
ignored; it is literally removed from the source before it is parsed.

```autohotkey
MsgBox "This message appears in both the compiled and uncompiled script"
;@AhkBuild-IgnoreBegin this text is ignored and can be used as a comment
MsgBox "This message does NOT appear in the compiled script"
;@AhkBuild-IgnoreEnd
MsgBox "This message appears in both the compiled and uncompiled script"
```

Ignore directives must be the first statement on their line, but can be followed by any amount of text.
`ahkbuild` respects both [`@Ahk2Exe-Ignore`] (begin / end) and `;@AhkBuild-Ignore`. Note that
`;@Ahk2Exe-IgnoreKeep` is not respected, to conditionally *include* code in a script, use the
[`A_IsCompiled`] variable and allow [tree-shaking]({{< relref "docs/bundling/tree-shaking" >}}) to delete
the dead branch.

[`@Ahk2Exe-Ignore`]: https://www.autohotkey.com/docs/v2/misc/Ahk2ExeDirectives.htm#IgnoreKeep
[`A_IsCompiled`]: https://www.autohotkey.com/docs/v2/Variables.htm#IsCompiled
