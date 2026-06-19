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
