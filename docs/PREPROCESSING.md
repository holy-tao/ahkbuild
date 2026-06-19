# Preprocessing

Preprocessing is a pure-text modification phase that runs before the source file is parsed.

You can run the preprocessor alone on a source file with the command

```shell
ahkbuild preprocess source.ahk [out.ahk]
```

## Preprocessing Steps

The preprocessor does the following, in the order listed.

### [Continuation Section] Resolution

The preprocessor will resolve [continuation section]s to a single line of code unconditionally before parsing the
source file. This is done because the tree-sitter grammar cannot reliably parse code continuation sections, and
multiline strings make it slightly more difficult to tell whether a position in a line is inside a string literal or
not (and thus whether it is safe to prune).

Thus, for example, this:

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

This allows the parser to correctly identify the section as a call to `MsgBox`. String literals are also collapsed
this way. This behavior cannot be modified or configured.

[continuation section]: https://www.autohotkey.com/docs/alpha/Scripts.htm#continuation

### Ignore Sections

Code between a pair of ignore directives is ignored; it is literally not included in the parsed source. Ignore
directives must be the first statement on their line, but can be followed by any amount of text. `ahkbuild` respects
both [`@Ahk2Exe-Ignore`] (begin / end) and `;@AhkBuild-Ignore`. Note that `;@Ahk2Exe-IgnoreKeep` is not respected, to
conditionally *include* code in a script, use the [`A_IsCompiled`] variable and allow tree-shaking to delete the dead
branch.

```autohotkey
MsgBox "This message appears in both the compiled and uncompiled script"
;@AhkBuild-IgnoreBegin this text is ignored and can be used as a comment
MsgBox "This message does NOT appear in the compiled script"
;@AhkBuild-IgnoreEnd
MsgBox "This message appears in both the compiled and uncompiled script"
```

[`@Ahk2Exe-Ignore`]: https://www.autohotkey.com/docs/v2/misc/Ahk2ExeDirectives.htm#IgnoreKeep
[`A_IsCompiled`]: https://www.autohotkey.com/docs/v2/Variables.htm#IsCompiled
