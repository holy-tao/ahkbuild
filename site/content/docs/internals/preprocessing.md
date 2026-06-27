---
title: Preprocessing
weight: 5
---

# Preprocessing

Preprocessing is a pure-text phase that runs *before* the source is ever parsed. The user-facing
behaviour - continuation-section collapsing and ignore regions - is documented under
[Bundling > Preprocessing]({{< relref "/docs/bundling/preprocessing" >}}); this page is about why it
exists and why it runs where it does.

## Why a text phase at all

Everything downstream of preprocessing works on a tree-sitter CST. The problem is that the
[`tree-sitter-autohotkey`] grammar cannot reliably parse [continuation section]s, and AHK's
continuation sections can rejoin text in ways that are not lexically obvious. Consider:

```autohotkey
( JoinB
Msg
ox "Hello, World!"
)
```

That is a single call to `MsgBox "Hello, World!"`, but no grammar can see that until the section is
joined. Multiline string literals are worse: until the section is collapsed, it is genuinely
ambiguous whether a given position on a line is *inside* a string or not - which is exactly the
question every later pass needs answered before it can decide whether a span is safe to delete or
fold.

So preprocessing collapses continuation sections to a single physical line first,
**unconditionally**. After that, the CST faithfully reflects the code, and every downstream pass can
treat string boundaries as known.

## Why text and not an AST transform

The phase is implemented as text in / text out (the `preprocess` crate), deliberately:

- It runs before parsing, so there is no tree to transform yet - the whole point is to make the text
  *parseable*.
- It can be run and inspected standalone with `ahkbuild preprocess source.ahk [out.ahk]`, which is
  invaluable when a bundle result is surprising.
- Ignore regions (`;@AhkBuild-IgnoreBegin` / `End`, and the Ahk2Exe equivalents) are a pure textual
  cut - the ignored span is literally removed before parsing, so it never has to be valid code.

> [!NOTE]
> Because this phase rewrites source text, it can change the physical line numbers of later
> statements, which affects [`A_LineNumber`] and the line numbers in error messages. This is the
> price of collapsing continuations before parse, and is called out for users on the
> [Preprocessing]({{< relref "/docs/bundling/preprocessing" >}}) page.

[`tree-sitter-autohotkey`]: https://github.com/holy-tao/tree-sitter-autohotkey
[continuation section]: https://www.autohotkey.com/docs/alpha/Scripts.htm#continuation
[`A_LineNumber`]: https://www.autohotkey.com/docs/alpha/Variables.htm#LineNumber
