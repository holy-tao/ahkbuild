/************************************************************************
 * @description Tree-shaking pass: dead code elimination via reachability.
 * @author
 * @date 2026/03/01
 * @version 0.1.0
 ***********************************************************************/

#Requires AutoHotkey v2.0

#Include ir.ahk
#Include scope.ahk
#Include <log4ahk\Log>

/**
 * Tree-shaking pass that identifies entry points, performs reachability
 * analysis, and deletes dead (unreachable) symbols from the IR.
 *
 * Granularity: entire functions and entire classes. Within a live class,
 * all members are kept (per-method pruning is future work).
 */
class TreeShaker {

    /**
     * Run tree-shaking on a program.
     *
     * @param {IR.Program} program the IR program
     * @returns {Array<IRSymbol>} the dead symbols that were deleted
     */
    Run(program) {
        st := program.symbolTable
        entryPoints := this._CollectEntryPoints(program, st)

        Log.Info(Format("Tree-shaking: {1} entry point symbols", entryPoints.Length))
        if Log.CurrentLevel <= Log.Level.TRACE {
            for ep in entryPoints
                Log.Trace(Format("  Entry: {1} '{2}'", ep.kind, ep.name))
        }

        st.MarkLive(entryPoints)

        dead := st.GetDeadSymbols()
        Log.Info(Format("Tree-shaking: {1} dead symbols", dead.Length))
        if Log.CurrentLevel <= Log.Level.TRACE {
            for sym in dead
                Log.Trace(Format("  Dead: {1} '{2}'", sym.kind, sym.name))
        }

        st.DeleteDeadSymbols()
        return dead
    }

    /**
     * Walk the program's top-level body and collect symbols that serve as
     * entry points for reachability analysis.
     *
     * Entry point categories:
     *   1. Symbols referenced by top-level non-declaration statements (auto-execute)
     *   2. Symbols referenced inside hotkey bodies
     *   3. Symbols referenced inside hotstring replacements
     *   4. Symbols referenced in #HotIf directive expressions
     *   5. Classes with static __New() methods (load-time side effects)
     *
     * @param {IR.Program} program
     * @param {IRSymbolTable} st
     * @returns {Array<IRSymbol>}
     */
    _CollectEntryPoints(program, st) {
        entryPoints := []
        seen := Map()

        for node in program.body {
            if node is IR.Function {
                ; Top-level function declaration — only live if referenced
                continue
            }

            if node is IR.ClassDecl {
                ; Only an entry point if it has static __New (load-time side effects)
                if this._HasStaticNew(node) {
                    sym := st.Lookup(node.fullyQualifiedName)
                    if sym != ""
                        this._AddEntry(entryPoints, seen, sym)
                }
                continue
            }

            if node is IR.Hotkey {
                ; Hotkey body is always an entry point
                if node.HasOwnProp("body")
                    this._WalkForSymbols(node.body, st, entryPoints, seen)
                continue
            }

            if node is IR.Hotstring {
                ; Hotstring replacement is always an entry point
                if node.HasOwnProp("replacement")
                    this._WalkForSymbols(node.replacement, st, entryPoints, seen)
                continue
            }

            if node is IR.Directive {
                ; #HotIf expression is an entry point
                if node.kind == "hotif" && node.HasOwnProp("expression")
                    this._WalkForSymbols(node.expression, st, entryPoints, seen)
                ; Other directives (#Requires, #Warn, etc.) are not entry points
                continue
            }

            ; Everything else is part of the auto-execute section:
            ; expression statements, VarDecl, labels, etc.
            this._WalkForSymbols(node, st, entryPoints, seen)
        }

        return entryPoints
    }

    /**
     * Walk an IR node subtree and collect all referenced symbols as entry points.
     * Used for non-symbol entry points (hotkey bodies, auto-execute statements).
     *
     * @param {IR.Node} node
     * @param {IRSymbolTable} st
     * @param {Array<IRSymbol>} entryPoints
     * @param {Map} seen
     */
    _WalkForSymbols(node, st, entryPoints, seen) {
        if node is IR.Identifier {
            if node.HasOwnProp("resolvedSymbol")
                this._AddEntry(entryPoints, seen, node.resolvedSymbol)
        }

        if node is IR.CallExpr {
            if node.HasOwnProp("resolvedTarget") {
                targetSym := st.Lookup(node.resolvedTarget.name)
                if targetSym != ""
                    this._AddEntry(entryPoints, seen, targetSym)
            }
            else if !node.isDynamic {
                ; Unresolved non-dynamic call — warn unless it's a built-in
                try calleeName := node.callee.GetText()
                if IsSet(calleeName) {
                    try builtin := %calleeName%
                    if !IsSet(builtin) || !((builtin is Func) && builtin.IsBuiltIn)
                        Log.Warn(Format("Call target '{1}' is not resolved and not a built-in", calleeName))
                }
            }

            if node.isDynamic
                Log.Warn(Format("Dynamic call detected — cannot resolve for tree-shaking: {1}",
                    node.HasOwnProp("tsNode") ? node.tsNode.Text : "(unknown)"))
        }

        for child in node.children
            this._WalkForSymbols(child, st, entryPoints, seen)
    }

    /**
     * Check if a class (or any of its nested classes) has a static __New method.
     *
     * @param {IR.ClassDecl} cls
     * @returns {Boolean}
     */
    _HasStaticNew(cls) {
        for method in cls.methods {
            if StrLower(method.name) == "__new" && StrLower(method.scope) == "static"
                return true
        }
        for nested in cls.nestedClasses {
            if this._HasStaticNew(nested)
                return true
        }
        return false
    }

    /**
     * Add a symbol to the entry points list, deduplicating by object identity.
     *
     * @param {Array<IRSymbol>} entryPoints
     * @param {Map} seen
     * @param {IRSymbol} sym
     */
    _AddEntry(entryPoints, seen, sym) {
        key := ObjPtr(sym)
        if !seen.Has(key) {
            seen[key] := true
            entryPoints.Push(sym)
        }
    }
}
