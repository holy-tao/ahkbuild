# Porting Plan: ahkbuild -> Rust (v2.1 module-aware)

Status: planning. Written 2026-06-05. This is a redesign, not a 1:1 port.

## Goal

Rewrite the AHK bundler in Rust, targeting AutoHotkey **v2.1's module system**. The
bundler collapses a multi-file v2.1 module project into a **single self-contained
`.ahk` file**, optionally tree-shaken / constant-folded / inlined.

- **Output is `.ahk`, consumed by the v2.1 interpreter** — not `.exe`. Ahk2Exe does
  not support any v2.1 features today, so it is out of scope as a consumer.
- We still expect Ahk2Exe to gain v2.1 support eventually, which is one reason we
  **preserve `#Module` blocks** in the output rather than flattening to a single
  global namespace (see below).

## Why Rust

- **Sum types + exhaustive `match`** on the IR. The current AHK tree-shaker's worst
  bug class is silently missed reference edges (e.g. forgetting to trace the children
  of one node variant in the reachability walk). Modeling IR nodes as an `enum` with
  no catch-all arm turns that into a compile error. This is the primary motivation.
- Best-in-class tree-sitter support; the grammar repo is already Rust-native.
- Accepted tradeoff: smaller pool of AHK-community contributors who'll touch Rust.

## What carries over from the AHK implementation

| Reusable (the moat) | Discarded / replaced |
|---|---|
| `tree-sitter-autohotkey` grammar — author is the user (holy-tao); already parses v2.1 modules | Hand-rolled `DllCall` tree-sitter bindings (`Lib/tree-sitter/`) -> native `tree-sitter` crate |
| Design docs & algorithms (`docs/TREE_SHAKING.md`, IR design) | YUnit test harness -> Rust `#[test]` / `insta` snapshots |
| IR node taxonomy (~45 types in `build/ir.ahk`) -> maps to a Rust enum | AHK class machinery, extension libs |
| AHK-semantics knowledge (protected meta-functions, reflection funcs, dynamic deref patterns) | |
| Test **fixtures** (`.ahk` inputs / expected outputs under `tests/`) | |
| Two-phase build/resolve structure of `build/irbuilder.ahk` | |

## v2.1 module model (the facts that drive design)

- A module = own global namespace + body (auto-exec section) + exports + unique name.
  Default is the implicit `__Main` module.
- Three entry mechanisms (vs. v2.0's single `#Include`):
  - `#Module Name` — starts/reopens an in-file module; ends at next `#Module` or EOF.
  - `#Import` — loads a module as a **separate namespace**, optionally binding names
    (`as alias`, `{a, b as c}`, wildcard `*`, `from`, quoted vs unquoted).
  - `#Include` — still text paste, **but now once *per module***; `#Module` is
    prohibited in a multiply-`#Include`d file.
- `#Import` resolution: search path = importing file's dir first, then `AhkImportPath`
  env var or default `%A_ScriptDir%;%A_MyDocuments%\AutoHotkey;%A_AhkPath%\..`.
  Lookup order within a dir: `ModuleName` -> `ModuleName\__Init.ahk` -> `ModuleName.ahk`.
- `export` / `export default` gate wildcard imports. Backward-compat parse traps:
  `export MyVar` is a function call (need `export global MyVar`); `export fn() => 1`
  is a call, not an export (fat-arrow can't be exported).
- All modules implicitly import the built-in `AHK` module; names may shadow built-ins,
  reachable via `#Import AHK` + `AHK.MsgBox()`.
- In-file `#Module` definitions take precedence over filesystem modules of the same name.
- Sub-module groups (alpha.21): each file is its own module-name group; cross-group
  reference uses path-qualified `"Path:ModuleName"`.

## Bundler design

### Output model: preserve modules

The output is a single file containing multiple `#Module` blocks plus rewritten
in-file imports. The **interpreter** enforces namespace isolation, so we do **not**
flatten to one global namespace. This avoids v2.0-style identifier mangling, keeps
tree-shaking precision (per-module name tables), and keeps the door open for a future
Ahk2Exe path.

Hedge: keep module structure first-class in the IR and make any future
flatten-to-global pass a **separate optional pass** that does not exist yet. Only
build it if a downstream consumer genuinely can't read `#Module`.

### Preprocessor -> module-graph resolver/linker

The current `preprocess.ahk` is a line-based text splicer with a global
`includeMap` dedup. That model is wrong for modules. The new design:

- **One module-aware pipeline**, with the v2.0 (`#Include`-only) case modeled as a
  single implicit `__Main` module with no imports — exactly how AHK models it. (Not
  two separate preprocessors; that would fork the #Include/comment/constant logic.)
- `#Include` dedup keyed by `(module, file)`, not just file. Enforce "no `#Module` in
  a multiply-included file."
- Implement `#Import` resolution: search path, `__Init.ahk` handling, the various
  binding forms.
- Linker job: walk the import graph, emit each file-module once as a `#Module` block,
  rewrite filesystem imports to in-file bare-name imports.

### Same-name module collisions (v1: warn and defer)

A repeated `#Module Name` **reopens** (merges into) the existing module — it does NOT
hard-error. So pulling two imported files that each define `#Module Helper` into one
output file silently *merges* them. Getting execution order provably correct is
tantamount to interpreting the script (lazy first-reference execution, alpha.21).

- **v1:** detect same-name-across-groups, **warn, emit as-is.**
- **v2 (principled fix):** module-level **rename/alias pass** — rename the second
  group's module and rewrite its import references (incl. path-qualified `"path:Name"`
  and `from` bindings). Cheap because it's module-granularity (few modules), and it
  sidesteps both the merge and the ordering ambiguity.

### Tree-shaking — carries over and gets more precise

The worklist reachability, reflection/dynamic-deref handling, protected meta-functions,
and `DefineProp` pruning all port from `docs/TREE_SHAKING.md`. Module-driven upgrades:

- `MemberNameTable` becomes **per-module** -> kills most cross-type over-approximation
  (the "any `.Foo` anywhere keeps every `.Foo`" problem).
- `import` / `export` are precise reachability edges. A non-exported, unreferenced
  module name is *definitively* dead; exported-but-never-imported is dead program-wide.
- **Whole-module DCE**: a module never imported (and not `__Main`) is entirely dead.

### Constant folding / inlining — later

Mostly unaffected by modules, except cross-module inlining must resolve an inlined
function's free variables in *its* module, not the call site's. Note for later.

## Rust architecture decisions (get these right on day one)

1. **Arena + integer IDs, not parent pointers / `Rc<RefCell<>>`.** Store IR nodes in a
   `Vec<Node>` referenced by `NodeId(u32)` (or `la-arena` / `id_arena`). Same for the
   symbol table and scopes. The AHK IR's free use of `parent` / `children` /
   `resolvedSymbol` references does not translate; decide arena layout before writing
   any node type.
2. **Owned IR with byte `Span { start: u32, end: u32 }`, not borrowed tree-sitter
   nodes.** A `tree_sitter::Node` borrows the tree; holding them long-term fights
   lifetimes. Lower eagerly into owned IR carrying spans; keep source text + tree
   separately for span-slicing at emit time. Preserves the patch-based emission model.
3. **Lean into enum exhaustiveness.** One `enum Node { Function(..), ClassDecl(..),
   ImportDecl(..), ... }`; reachability/reference walks `match` with no catch-all arm,
   so adding a variant forces every walker to handle it. This is the whole point.

## Build order

0. **Tree-sitter spike** (~1 day): drive the grammar via the `tree-sitter` crate, parse
   real v2.1 module files, walk the tree. Retires the highest-uncertainty dependency
   and gives real CST nodes to design the IR against.
1. **IR + lowering**: define IR enums + arena/span model; lower CST -> IR.
2. **Module graph + linker** on the IR.
3. **Tree-shaker** (port the algorithm, now per-module-scoped).
4. **Later**: constant folding, inlining, optional flatten-to-global pass.

Comment/`A_PtrSize`/`A_IsCompiled` folding and dedent fall out of IR + span emission;
they are not a separate front-end anymore.

## Spike findings (2026-06-05, grammar v0.4.0)

Step 0 done: workspace builds, the `tree-sitter-autohotkey` path-dep links and parses
from Rust, `Span` + parse smoke tests green. Probing import/export syntax (fixtures in
`tests/fixtures/probes/`) surfaced grammar coverage gaps to resolve before IR design —
the grammar is the user's, so these are fixable upstream:

| Syntax | Parses? | Notes |
|---|---|---|
| `#Module X` | ✅ | |
| `export Name() { ... }` (block) | ✅ | |
| `export global MyVar := 1` | ✅ | |
| `export default class Foo {}` / `export default Fn() {}` | ✅ | |
| `#Import X` / `#Import Y as Z` | ✅ | |
| `#Import Foo {Bar, Baz as Qux}` | ✅ | |
| `#Import Y {*}` | ✅ | |
| `#Import Y {*, Extra}` | ❌ | documented (`{ *, ExportName ... }`) — likely real gap (pending: confirm doc isn't also stale) |
| `import {Calc as C} from X` (bare) | ❌ | **not a gap** — the bare `import ... from ...` statement form was removed in alpha.21 in favor of the `#Import` directive; correct rejection |
| `import * from Y` (bare) | ❌ | same — removed in alpha.21, correct rejection |
| `export Name() => expr` (fat-arrow) | ❌ | docs say this is *intentionally* parsed as a call to `export` (alpha.22+ back-compat); confirm grammar models it as a call rather than erroring |

> Note: the local `search-ahk-docs` cache predates alpha.21 on this point — its
> Modules.md still shows the removed `import ... from ...` examples. The bundler only
> needs to resolve the **`#Import` directive** forms.

Net: only one likely real grammar gap to chase (`#Import Y {*, Extra}`), plus one
behavior to confirm (fat-arrow `export` as a call). The bundler's import surface is
the `#Import` directive only.

## Open questions / deferred

- Embedded-script imports (`#Import "*RESNAME"`) — out of scope for v1.
- Exact module rename scheme (`__Init`, default exports) — deferred to the v2 rename pass.
- CLI surface: which of the current flags (`--tree-shake`, bitness, comments, dedent,
  dry-run) carry over, and how module/import options are exposed.
