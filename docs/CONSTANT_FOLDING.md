# Constant Folding & Branch Shaking

`ahkbuild_fold` evaluates expressions that are knowable at build time and reports what can be
substituted or pruned. It runs between linking and tree-shaking:

```text
Source -> Preprocess -> Link -> [Fold] -> [Tree-Shaking] -> Emit -> Output
```

Like every other pass it never mutates the IR - it returns a side table (`FoldResult`) keyed by
`NodeId` that both `shake` and `emit` consume.

## What it produces

`fold(program, &Constants) -> FoldResult` computes two things:

- **`literals`** - the **maximal** constant subexpressions that fold *because of* a known
  built-in (`A_IsCompiled`, `A_PtrSize`), mapped to their value. The emitter rewrites each one's
  span to the rendered literal. "Maximal" means the largest such expression: `A_PtrSize * 8` is
  recorded as `64` (not the inner `A_PtrSize` as `8`), and `"lib" . A_PtrSize` as `"lib8"`. Pure
  author constants (`2 + 2`, `true`) are *not* rewritten - only expressions whose value the
  build determines. A subexpression that evaluates to a **float** is left unrecorded (we fall
  back to substituting the built-ins inside it) so we never reproduce AHK's number formatting.
- **`branches`** - every `if` / ternary whose condition evaluates to a build-time constant,
  recorded as which arm survives (`Then`, `Else`, or `Dead` - a falsey `if` with no `else`).

## Build-time constants (`Constants`)

| Constant       | Source                                                                 |
| -------------- | ---------------------------------------------------------------------- |
| `A_IsCompiled` | `--compiled <true\|false>`. **Off by default for `ahk`** - a bundled `.ahk` may later be compiled with ahk2exe, which would flip it. The future `exe` target defaults it to `true`. |
| `A_PtrSize`    | `--bitness <32\|64>`, else derived from a bitness-pinned `#Requires` (e.g. `#Requires AutoHotkey v2.1-alpha.3 64-bit` ⇒ `8`), which is a certainty when present. |

Each is `Option`: when unknown for the target, the corresponding built-in is left untouched. If
no constant is known, the fold pass does not run and output is byte-for-byte unchanged.

## The evaluator

`eval(node) -> Option<ConstValue>` is a total, conservative recursion: anything it cannot prove
constant yields `None` and is left alone (over-keeping a branch is always safe). It folds:

- **Literals** - integer (decimal/`0x` hex), float, string, and `Boolean` (-> `Int(1)`/`Int(0)`).
- **Identifiers** - the language constants `true`/`false`, plus any known built-in. In AHK
  `true`/`false` are just `1`/`0` and arithmetic applies (`true + 1 == 2`), so there is no
  separate boolean `ConstValue` variant.
- **Unary** - `!` / `not`, `-`, `+`, `~`.
- **Binary** - comparisons (`= == != <> < > <= >=`), arithmetic (`+ - * / // **`),
  **short-circuit** logical (`&& || and or`), and explicit string concatenation (`.`).
  Comparisons and arithmetic fold only over numbers, side-stepping AHK's string/number coercion
  quirks (irrelevant to build-config guards). Implicit concatenation (adjacency) has an
  unreliable operator span, so only the explicit `.` form folds.
- **Ternary** - folds the condition, then recurses into the taken arm.

Short-circuiting is what makes branch shaking sound: `A_IsCompiled && Foo()` folds to `false`
(when `A_IsCompiled` is false) **without** evaluating `Foo()`. Because a condition only folds
when every non-constant part was short-circuited away and would never run, tree-shaking can
safely discard the whole condition subtree.

## How the results are used

- **`shake`** (`reach::walk`) descends only into the surviving arm of a resolved branch, so
  declarations reachable only from a dead arm shake out. See [TREE_SHAKING.md](TREE_SHAKING.md).
- **`emit`** produces span edits:
  - *Substitution* (rewrite): replace each `literals` (sub)expression's span with its rendered
    value, trimmed to the expression's non-whitespace extent so a command-style call's separator
    space survives (`MsgBox A_PtrSize` -> `MsgBox 8`, not `MsgBox8`). Skipped when the span already
    lies inside a deleted region (a collapsed branch's condition).
  - *Branch collapse* (deletion): delete the scaffolding around the surviving arm - the
    condition and dead arm - leaving the live arm's body in place so its own inner edits still
    apply. A braced block arm has its braces stripped too; a `Dead` `if` is removed whole.

## Future

- **Feed folded strings into reachability.** A constant that folds to a string can become a
  method/property name in a dynamic deref or a reflection call (`GetMethod(obj, "On" . SUFFIX)`).
  Today `shake`'s [member-name table](TREE_SHAKING.md#per-member-pruning) only reads literal
  string arguments; consulting `FoldResult.literals` for folded string values would let it
  resolve more of these instead of falling back to keeping the class whole.
- **Cross-pass fixpoint.** `eval` is a pure function of the IR plus the known constants, so once
  inlining lands it can enrich that table and `fold` can re-run to a fixpoint, with `emit`
  staying a dumb renderer of the final side tables.
