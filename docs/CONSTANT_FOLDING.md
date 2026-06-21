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

- **`literals`** - the **maximal** constant subexpressions that fold, mapped to their value. The
  emitter rewrites each one's span to the rendered literal.
  - "Maximal" means the largest such expression: `A_PtrSize * 8` is recorded as `64` (not the
    inner `A_PtrSize` as `8`), and `"lib" . A_PtrSize` as `"lib8"`.
  - A subexpression that evaluates to a **float** is left unrecorded (we fall back to substituting
    the constants inside it) so we never reproduce AHK's number formatting.
- **`branches`** - every `if` / ternary whose condition evaluates to a build-time constant,
  recorded as which arm survives (`Then`, `Else`, or `Dead` - a falsey `if` with no `else`).

## Build-time constants (`Constants`)

| Constant | Source |
| --- | --- |
| `A_IsCompiled` | `--compiled <true\|false>`. **Off by default for `ahk`** - a bundled `.ahk` may later be compiled with ahk2exe, which would flip it. The future `exe` target defaults it to `true`. |
| `A_PtrSize` | `--bitness <32\|64>`, else derived from a bitness-pinned `#Requires` (e.g. `#Requires AutoHotkey v2.1-alpha.3 64-bit` ⇒ `8`), which is a certainty when present. |

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
  **short-circuit** logical (`&& || and or`), and string concatenation - both explicit (`.`)
  and implicit (adjacency, e.g. `"lib" A_PtrSize`). Comparisons and arithmetic fold only over
  numbers, side-stepping AHK's string/number coercion quirks (irrelevant to build-config
  guards). A parenthesized operand surfaces its inner node, so the concat operators (which
  carry no `operator` field) recover their operator span by trimming whitespace and the
  wrapping parens from the gap between operands - implicit concat lands on an empty span.
- **Ternary** - folds the condition, then recurses into the taken arm.

Short-circuiting is what makes branch shaking sound: `A_IsCompiled && Foo()` folds to `false`
(when `A_IsCompiled` is false) **without** evaluating `Foo()`. Because a condition only folds
when every non-constant part was short-circuited away and would never run, tree-shaking can
safely discard the whole condition subtree. We do not need to care whether `Foo` is effectful
or not, because it would never run in the first place.

## User-defined constants

AHK has no `const`, but most "constants" are names assigned once and never reassigned (the
classic `static FLAG := 0x1234` DllCall pattern) or getter-only fat-arrow properties. The
`userconst` module detects these and feeds each **read site** into the evaluator as if it were a
known constant, so the maximal-substitution and branch passes fold them with no extra machinery.
It is run inside `fold` before the two passes above. It also surfaces, for `shake`, the read sites
it folded (`FoldResult.folded_reads`) and the declaration statements that are now dead
(`FoldResult.dead_consts`) - see [Declaration removal](#declaration-removal).

Detection is conservative - it never folds a name it cannot prove is single-assignment with a
constant value:

- **Scope-aware binding.** Every name occurrence is resolved to the scope that *binds* it,
  honouring AHK v2 closures: a nested function captures (reads **and** writes) an enclosing
  *function's* locals, while the module-global scope never captures into a function. A binding
  folds only with exactly one `:=` definer whose value folds.
- **Disqualifiers.** A second assignment, a compound assignment (`+=`, `.=`, …), `++`/`--`, or
  `&name` (taken by reference) - anywhere the binding is visible, including a capturing nested
  function - leaves the name untouched.
- **Dynamic writes.** An un-pinnable `%expr% := …` poisons every binding its scope can see. A
  write whose target is a constant name (`%"Foo"%`, `pre%"x"%`) is treated as an ordinary write to
  that resolved name instead.
- **Getter-only properties.** `static Value => 42` folds **only** a `ClassName.Value` access - the
  object must be an identifier naming the exact class that defines the getter. A bare `obj.Value`
  is *not* folded: without the object's static type, `Value` could resolve to a different class's
  member, a nested class, or a method (e.g. `MsgPack.Nil` is a nested class while
  `MsgPackType.nil => 192` is a getter - folding every `.nil` would break `MsgPack.Nil()`). A
  member is also left alone when blocked anywhere by a field/setter/member-assignment of that name,
  a literal-named `DefineProp` for it, or *any* dynamically-named `DefineProp`.
- **`;@ahkbuild-const` directive.** The explicit escape hatch: placed on a declaration, it folds
  the binding on the author's word, skipping the single-assignment / dynamic / `DefineProp`
  checks. Only the value still needs to fold.

Detection runs to a **fixpoint**, so constants defined in terms of other constants resolve
(`B := A + 1`, or a getter that reads a folded name).

### Declaration removal

Once *every* read of a constant has been folded to a literal, its declaration is dead weight.
`userconst` reports the safe-to-delete declarations in `FoldResult.dead_consts` and `shake` deletes
them (they are auto-execute roots it would otherwise always keep). Removal is conservative on top of
the folding gates above:

- **Structural guard (always).** An `export`ed constant is public API the bundler can't see all
  uses of, so it is never removed - even under a directive. The declaration must also be a lone
  `name := <expr>` / `local|static name := <expr>` statement, never a nested or compound assignment.
- **Provability guards (non-directive).** Removed only when single-assignment *and* no dynamic read
  (`%expr%`) could reach the binding - a dynamic read needs the variable to still exist at runtime,
  even though the static reads fold. A `;@ahkbuild-const` directive bypasses these (author's word);
  all its `:=` writes are deleted.

Getter-only properties take a different route: `shake` discounts their folded accesses
(`FoldResult.folded_reads`) when building its member-name table, so an otherwise-unreferenced
property shakes out through ordinary [per-member pruning](TREE_SHAKING.md#per-member-pruning).

## How the results are used

- **`shake`** (`reach::walk`) descends only into the surviving arm of a resolved branch, so
  declarations reachable only from a dead arm shake out. It also deletes `dead_consts` declarations
  and discounts `folded_reads` member accesses. See [TREE_SHAKING.md](TREE_SHAKING.md).
- **`emit`** produces span edits:
  - *Substitution* (rewrite): replace each `literals` (sub)expression's span with its rendered
    value, trimmed to the expression's non-whitespace extent so a command-style call's separator
    space survives (`MsgBox A_PtrSize` -> `MsgBox 8`, not `MsgBox8`). Skipped when the span is
    already inside a deleted region (a collapsed branch's condition).
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
- **Drop emptied getter-const classes.** A class whose only members were folded-away getter
  constants is left as an empty `class` shell - reachability still sees the class-name identifier in
  the pre-fold IR. Teaching `reach` to skip the subtree of a `folded_reads` access would let it
  shake out too.
- **Exe target defaults.** The build-time constants should default to `compiled=true, bitness=<target>`
  when the bundle target is a .exe.
