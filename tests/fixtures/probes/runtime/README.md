# Runtime probes: module identity

Unlike the parse probes in the parent dir (which check the *grammar*), these run the
real v2.1 interpreter to settle **module-identity semantics** that drive the bundler's
backend design. Reproduce with `pwsh -File run.ps1`.

## Question

When the bundler combines modules from several origins, are two same-named modules
(`#Module Helper` in two different files/resources) kept **distinct**, or do they
**merge** like a textual reopen? This decides whether the single-`.ahk` backend needs a
rename pass and whether the `.exe` (`*RESNAME`) backend is collision-free.

## Findings (AutoHotkey v2.1-alpha.30, 64-bit)

| Probe | Setup | Output | Conclusion |
|---|---|---|---|
| P-A | two `#Module Helper` in **one file** | `P-A GetX()=123` | same-file blocks **reopen/merge** into one namespace |
| P-B | two **imported files**, each `#Module Helper` | `P-B A=A B=B` | each file is its own **group**; same-named sub-modules are **isolated** |
| P-C | same, from embedded **RCDATA** via `#Import "*RES"` | `P-C A=A B=B` | `*RESNAME` units are groups too — identical to file imports |

**Module identity is `(origin-group, name)`, not bare `name`.** Merge happens only
*within* a group (one file). Therefore:

- **Single-`.ahk` backend** must not naively concatenate two groups' `#Module Helper`
  blocks — they'd merge. Needs the group-aware rename pass (PORTING_PLAN v2 fix).
- **`.exe` backend** can keep each origin as its own RCDATA resource; the interpreter
  preserves isolation. No merge, no rename pass. P-C built the `.exe` by injecting
  RCDATA into a *copy of the interpreter* — **Ahk2Exe is not involved**.

## Gotchas confirmed along the way

- `export Name() => expr` parses as a **call to `export`** (back-compat trap); use a
  block body `export Name() { ... }` to actually export. (See GroupA/GroupB.)
- A **quoted** import (`#Import "*RES"`) does not bind a name by itself — needs `as Alias`.
- `#Requires AutoHotkey v2.1` is rejected by alpha builds; probes omit it and invoke the
  alpha interpreter directly.
- Embedded scripts: main script = RCDATA **integer id 1** (`*#1`), named modules =
  RCDATA string names (`*GROUPA`). Resources written UTF-8 **with BOM**, language 1033.
