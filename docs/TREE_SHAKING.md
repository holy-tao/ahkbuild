# Tree-Shaking (Dead Code Elimination)

Tree-shaking removes unreferenced functions and classes from the build output.

Because AutoHotkey is dynamically typed, we're limited in what we can remove with confidence. In practice, the tree-shaking algorithm will keep all code which *could be* referenced.

## Pipeline Position

```text
Source -> Preprocessor -> Link (CST + IR per file) -> [Tree-Shaking] -> Emitter -> Output
```

The tree-shaking pass (`ahkbuild_shake::shake`) runs after the linker has assembled the full multi-group `Program` and before emission. It returns a `ShakeResult` (sets of dead node IDs), which the emitter turns into span deletions. The IR is never mutated.

In the future it should run *after* inlining and constant folding.

## Granularity

- **Functions**: A top-level function is removed if no live code references it.
- **Classes**: A class is removed entirely if no live code references it.
- **Class members**: Within a live class, individual methods, properties, and fields can be pruned if their name never appears in any member access expression in the program. This uses a global name table approach (see **Per-Member Pruning** below).
- **`#Import` directives**: If the bound name of an `#Import` is never referenced from live code, the directive is dropped. The target module still auto-executes if loaded by another path; a module reached *only* through unused imports is never loaded and shakes out whole.
- **Modules**: A `#Module` block (or a whole imported file) is removed if none of its declarations survive and no live code depends on it.
- **Labels**: A label is removed if unreferenced. Labels in the auto-execute section are always live.

### Future plans

- **If / ternary branches**: if an if statement or ternary expression's condition inlines/folds to a constant expression, prune the dead half
- **loops**: if a loop inlines to 0 iterations or we can confidently determine that a for-loop iterates 0 items, delete it
- **nitpicks**: low-effort, low-reward
  - property initializers that initialize values to `unset` can be pruned (TODO is this true in v2.1?)

## Entry Points

Entry points are the roots of the reachability analysis. Everything transitively referenced from an entry point is kept; everything else is dead.

| Entry Point | Rationale |
| --- | --- |
| Top-level non-declaration statements | The auto-execute section runs unconditionally at startup |
| Hotkey bodies | Hotkeys are user-triggered; their bodies must be preserved |
| Hotstring replacements | Same as hotkeys |
| [`#HotIf`](https://www.autohotkey.com/docs/v2/lib/_HotIf.htm) directive expressions | Evaluated repeatedly at runtime to determine hotkey context |
| Classes with `static __New()` | Called automatically when the class is declared |

### What is NOT an entry point

- **Top-level function declarations** — only live if referenced from live code.
- **Top-level class declarations** (without `static __New`) — only live if referenced.
- **Other directives** (`#Requires`, `#Warn`, etc.) — no executable code to trace.

## Reachability Analysis

We do worklist-based reachability analysis. The four passes in `ahkbuild_shake::shake` are:

1. **Resolve** (`shake::resolve::resolve`): Build per-module declaration tables and import-binding tables from the linked `Program` + `BundlePlan`. This is the foundation for name lookups during the mark phase.

2. **Build member name table** (`shake::members::collect`): Walk the entire IR and collect all member names that could be referenced at runtime. Sources include static member access expressions (`obj.Foo`), [dynamic](https://www.autohotkey.com/docs/v2/Language.htm#dynamic-variables) member access with extractable constant parts (`obj.prefix%expr%`), and string arguments to reflection functions (`ObjBindMethod(obj, "Method")`).

   > [!WARNING]
   > If a fully-dynamic member access with no constant parts is found (e.g. `obj.%prop%`, `ObjBindMethod(myObj, variable)`), the table is marked as "blown up" and member-level pruning is disabled.

   See [Per-member pruning](#per-member-pruning) below for details.

3. **Prune dead DefineProp calls** (`shake::defineprop::prune`): Walk the IR for `*.DefineProp("literal", ...)` calls. Mark the call dead if:
   1. The property name is not in the name table ***or*** it's in the name table exactly once, and the only referencer is inside the `DefineProp` call itself, ***and***
   2. The property is not a [protected meta-function](#protected-meta-functions) like `__New`

   This must run before marking so references inside pruned descriptors are not followed.

4. **Mark live** (`shake::reach::mark`): Load the entry group, then drain a worklist of `(node, owning module)` pairs. For each live node:
   - Walk its subtree to find referenced names and add their declarations to the worklist.
   - If it's a class, propagate liveness to member nodes. With a name table, only members whose names appear in the table (or are [protected meta-functions](#protected-meta-functions)) are included. Without one (blown up), all members are kept.
   - Track which `#Import` directives are "taken" (their bound name is used); taken imports load their target group, seeding its auto-execute roots.

5. **Collect result**: The `ShakeResult` accumulates dead node IDs (`dead`), droppable `#Import` directives (`dropped_imports`), and dead module nodes (`dead_modules`). The emitter turns each into a span deletion.

## Reference Tracking

The reachability walk (`shake::reach`) finds symbol references by walking IR node subtrees:

| Reference Type | How It's Found |
| --- | --- |
| Variable/function name | `NodeKind::Identifier` — text looked up in the owning module's declaration table |
| Superclass name | `NodeKind::ClassDecl.superclass` — text looked up by name |
| Goto label target | `NodeKind::GotoStmt` target text — looked up by name |
| Catch error types | `NodeKind::CatchClause` error type names — each looked up by name |
| Cross-module import binding | `NodeKind::ImportDirective` — bound name resolved via the `BundlePlan` |

Superclass, goto, and catch references are raw-text fields on their IR nodes (not child `Identifier` nodes), so they require explicit handling in the reachability walker.

## Per-Member Pruning

Per-member pruning removes unused methods, properties, and fields from live classes using a **global name table** approach. Instead of type inference, it tracks all member names referenced anywhere in the program and prunes members whose names can never be referenced.

### How it works

A `MemberNameTable` is built by walking the entire IR:

| Source | What's collected |
| --- | --- |
| Static member access (`obj.Foo`) | Exact name "Foo" |
| Dynamic member with outer prefix (`obj.Get%name%`) | Prefix pattern "Get" |
| Dynamic member with outer suffix (`obj.%expr%Handler`) | Suffix pattern "Handler" |
| Dynamic member with inner string literal (`obj.%"literal"%`) | Exact name "literal" |
| Dynamic member with inner concat (`obj.%"prefix" . var%`) | Prefix pattern "prefix" |
| `ObjBindMethod(obj, "Method")` | Exact name "Method" |
| `ObjGetOwnPropDesc(obj, "Prop")` | Exact name "Prop" |
| `GetMethod(obj, "Name")` | Exact name "Name" |
| Fully-dynamic member access (`obj.%someVar%`) | **Blown up** - member pruning disabled |
| Non-literal reflection arg (`ObjBindMethod(obj, var)`) | **Blown up** - member pruning disabled |

Prefix and suffix patterns prevent all members whose names match them from being pruned. For example, a call to `obj.%variable%Handler` will prevent all class members whose names end with *Handler* from being pruned. The name table also collects the locations where the names are mentioned; if further analysis proves that those locations can be deleted, the table is modified accordingly (`DefineProp` pruning does this, for example).

### Protected meta-functions

These members are **never pruned** from live classes, regardless of the name table:

- [`__New`](https://www.autohotkey.com/docs/v2/Objects.htm#Custom_NewDelete)
- [`__Delete`](https://www.autohotkey.com/docs/v2/Objects.htm#Custom_NewDelete)
- [`__Call`](https://www.autohotkey.com/docs/v2/Objects.htm#Meta_Functions)
- [`__Get`](https://www.autohotkey.com/docs/v2/Objects.htm#Meta_Functions)
- [`__Set`](https://www.autohotkey.com/docs/v2/Objects.htm#Meta_Functions)
- [`__Item`](https://www.autohotkey.com/docs/v2/Objects.htm#__Item)
- [`__Enum`](https://www.autohotkey.com/docs/v2/Objects.htm#__Enum)
- [`Call`](https://www.autohotkey.com/docs/v2/Functions.htm#DynCall)
- [`__value`](https://www.autohotkey.com/docs/alpha/Structs.htm#abstract)
- [`__Ref`](https://www.autohotkey.com/docs/alpha/lib/Struct.htm#__Ref)

These are invoked implicitly by the AHK runtime, so it's not possible to statically determine whether they're called or not. `Call` is included because calling a non-function object implicitly invokes `.Call()`, and without type inference we cannot determine when this happens.

### Reflection functions

The algorithm tracks the following reflection functions, because callers can use these to access the underlying properties:

- [`ObjBindMethod`](https://www.autohotkey.com/docs/v2/lib/ObjBindMethod.htm)
- [`GetOwnPropDesc`](https://www.autohotkey.com/docs/v2/lib/Object.htm#GetOwnPropDesc)
- [`GetMethod`](https://www.autohotkey.com/docs/v2/lib/GetMethod.htm)

Using any of these functions creates a reference to the relevant name or prefix / suffix pattern, if the name isn't constant. Using a variable with no constant parts will defeat member pruning, as with dereference expressions.

## Known Limitations

### Dynamic calls (`%var%()`)

When the callee of a call expression is a `DerefExpr` (e.g., `%funcName%()`), the target function cannot be determined statically. A warning is logged. The call's arguments are still traced for references, but the callee itself is opaque.

#### Future improvements

- if constant propagation resolves the variable to a known string, the dynamic call can be resolved.
- Similarly, if the name is an identifier or member access expression and we can resolve it to a limited set of possible strings, the dynamic call can be resolved to one or more names. For example:

   ```autohotkey
   for propertyName in ["A", "B", "C"]
      obj.%propertyName% := "example"    ; Check every variable with name "propertyName"
   ```

- Currently only prefixes and suffixes are tracked in the name table, but we can build much more sophisticated patterns in some cases. For example, a construct like `My%deref%cool%deref%thing` should only match names following a pattern like `My*cool*thing`, but currently the part between the derefs is ignored

#### Dynamic member access disables member pruning

A fully-dynamic member access with no extractable constant parts (e.g. `obj.%someVar%` where `someVar` is not a string literal) defeats member-level pruning entirely. The pass falls back to whole-class granularity. Similarly, calls to reflection functions with non-literal name arguments (e.g. `ObjBindMethod(obj, varName)`) trigger the same fallback.

### Name-based, not type-based

Per-member pruning is conservative: if `.Foo` appears anywhere in the program, ALL classes keep their `Foo` member, even if the access is on a completely unrelated type. This is an overapproximation that avoids the need for type inference.

### [`DefineProp`](https://www.autohotkey.com/docs/v2/lib/Object.htm#DefineProp) calls are prunable

`DefineProp` calls with a string literal first argument are checked against the member name table. If the defined property name is never referenced anywhere in the program outside of the `DefineProp` call itself and is not a protected meta-function, the entire call statement is deleted. This also enables transitive pruning: functions referenced only from a pruned `DefineProp` descriptor become dead code.

Conservatively kept (not pruned):

- Non-literal property name (`this.DefineProp(varName, ...)`)
- Protected meta-function names (`__Get`, `__Set`, etc.)
- Calls embedded in larger expressions (not standalone statements)
- Chained calls (`obj.DefineProp("A", d1).DefineProp("B", d2)`)
