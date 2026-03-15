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
#Include <Collections\Typed\TypedMap>
#Include <Collections\Typed\TypedArray>

/**
 * Tracks all member names referenced anywhere in the program.
 * Used during tree-shaking to determine which class members can be pruned.
 *
 * When `isBlownUp` is true, a fully-dynamic member access was found
 * and member-level pruning must be skipped entirely.
 */
class MemberNameTable {

    /** @type {Map} case-insensitive: lowercase name -> IR.Node[] */
    exactNames := TypedMap(String, TypedArray)

    /** @type {Map} lowercase prefix strings -> IR.Node[]*/
    prefixPatterns := TypedMap(String, TypedArray)

    /** @type {Map} lowercase suffix strings -> IR.Node[]*/
    suffixPatterns := TypedMap(String, TypedArray)

    /** @type {Boolean} true if analysis is defeated by fully-dynamic access */
    isBlownUp := false

    /**
     * Add a known exact member name.
     * @param {String} name
     * @param {IR.Node} node the node that's referencing the name
     */
    AddExact(name, node) {
        name := this._Normalize(name)

        if !this.exactNames.Has(name) {
            Log.Trace(Format("Adding exact name to member name table: '{1}' (referenced at '{2}')", name, node.GetText()))
            this.exactNames[name] := TypedArray(IR.Node)
        }
        this.exactNames[name].Push(node)
    }

    /**
     * Add a prefix pattern — any member starting with this is considered referenced.
     * @param {String} prefix
     */
    AddPrefix(prefix, node) {
        prefix := this._Normalize(prefix)
        if prefix != "" && !this.prefixPatterns.Has(prefix) {
            Log.Trace(Format("Adding prefix to member name table: '{1}' (referenced at '{2}')", prefix, node.GetText()))
            this.prefixPatterns[prefix] := TypedArray(IR.Node)
        }

        this.prefixPatterns[prefix].Push(prefix)
    }

    /**
     * Add a suffix pattern — any member ending with this is considered referenced.
     * @param {String} suffix
     */
    AddSuffix(suffix, node) {
        suffix := this._Normalize(suffix)
        if suffix != "" && !this.suffixPatterns.Has(suffix) {
            Log.Trace(Format("Adding suffix to member name table: '{1}' (referenced at '{2}')", suffix, node.GetText()))
            this.suffixPatterns[suffix] := TypedArray(IR.Node)
        }

        this.suffixPatterns[suffix].Push(suffix)
    }

    /**
     * Mark the analysis as defeated. All members will be kept.
     */
    BlowUp() {
        this.isBlownUp := true
    }

    /**
     * Check if `name` could be referenced based on collected data.
     *
     * @param {String} name the member name to check
     * @returns {IR.Node[] | 0} the nodes that reference the name (if a prefix / suffix, this is empty), or 0
     *              if no match
     */
    Matches(name) {
        if this.isBlownUp
            return []

        key := this._Normalize(name)
        if this.exactNames.Has(key)
            return this.exactNames[key]

        for prefix, nodes in this.prefixPatterns
            if SubStr(key, 1, StrLen(prefix)) == prefix
                return nodes

        for suffix, nodes in this.suffixPatterns
            if SubStr(key, -StrLen(suffix)) == suffix
                return nodes

        return 0
    }

    /**
     * Removes all descendants of `parent` from the table, deleting keys if they become
     * empty
     * 
     * @param {IR.Node} parent the node whose descendants you want to remove 
     */
    RemoveDescendantReferencers(parent) {
        this._CleanMap(parent, this.exactNames)
        this._CleanMap(parent, this.prefixPatterns)
        this._CleanMap(parent, this.suffixPatterns)

        for child in parent.children {
            this.RemoveDescendantReferencers(child)
        }
    }

    /**
     * Remove referencers that are descendants of `parent` from a name map,
     * deleting keys whose arrays become empty.
     *
     * @param {IR.Node} parent
     * @param {Map<String, IR.Node[]>} map  name -> referencers
     */
    _CleanMap(parent, map) {
        for name, nodes in map {
            i := nodes.Length
            while i >= 1 {
                if nodes[i].IsDescendentOf(parent)
                    nodes.RemoveAt(i)
                i--
            }

            if nodes.Length == 0 {
                Log.Trace(Format("Removing '{1}' from name map; all referencers pruned", name))
                map.Delete(name)
            }
        }
    }

    _Normalize(name) => Trim(StrLower(name), " `r`n`t")
}

/**
 * Tree-shaking pass that identifies entry points, performs reachability
 * analysis, and deletes dead (unreachable) symbols from the IR.
 *
 * Supports per-member pruning within live classes via a global name table.
 * If fully-dynamic member access is detected, member-level pruning is
 * disabled and whole-class granularity is used instead.
 */
class TreeShaker {

    /**
     * Meta-functions and special methods that are never pruned from live classes.
     * These can be invoked implicitly by the AHK runtime.
     */
    static ProtectedMembers := this._BuildProtectedMap()

    static _BuildProtectedMap() {
        m := Map()
        m.CaseSense := "Off"
        for name in ["__New", "__Delete", "__Call", "__Get", "__Set", "__Item", "__Enum", "Call"]
            m[name] := true
        return m
    }

    /**
     * Built-in functions that take a member/method name as a string argument.
     * Key: lowercase function name. Value: {argIndex: 0-based index of the name arg}.
     */
    static ReflectionFunctions := this._BuildReflectionMap()

    static _BuildReflectionMap() {
        m := Map()
        m.CaseSense := "Off"
        m["ObjBindMethod"]      := {argIndex: 2}
        m["GetOwnPropDesc"]     := {argIndex: 1}
        m["GetMethod"]          := {argIndex: 2}
        return m
    }

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

        ; Build member name table for per-member pruning
        nameTable := this._CollectMemberNames(program)

        ; Prune DefineProp calls that create never-referenced properties.
        ; Must run before MarkLive so references inside pruned descriptors
        ; are not followed during reachability analysis.
        if !nameTable.isBlownUp {
            pruned := this._PruneDefinePropCalls(program, nameTable)
            if pruned > 0
                Log.Info(Format("DefineProp pruning: removed {1} call(s)", pruned))
        }

        if nameTable.isBlownUp {
            Log.Info("Member pruning disabled: fully-dynamic member access detected")
            st.MarkLive(entryPoints)
        } 
        else {
            Log.Info(Format("Member pruning: {1} exact names, {2} prefix patterns, {3} suffix patterns",
                nameTable.exactNames.Count, nameTable.prefixPatterns.Count, nameTable.suffixPatterns.Count))
            st.MarkLive(entryPoints, nameTable)
        }

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

        if node is IR.MemberAccess {
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

    /**
     * Walk the entire IR tree and build a MemberNameTable of all member
     * names that could be referenced at runtime. This includes:
     *   - Static member access (obj.Foo)
     *   - Dynamic member access with extractable constant parts (obj.prefix%expr%)
     *   - String arguments to configured reflection functions (ObjBindMethod etc.)
     *
     * @param {IR.Program} program
     * @returns {MemberNameTable}
     */
    _CollectMemberNames(program) {
        table := MemberNameTable()
        this._WalkForMemberNames(program, table)
        return table
    }

    /**
     * Recursive walker for member name collection.
     *
     * @param {IR.Node} node
     * @param {MemberNameTable} table
     */
    _WalkForMemberNames(node, table) {
        if table.isBlownUp
            return

        if node is IR.MemberAccess {
            if !node.isDynamic {
                ; Static member access: exact name
                table.AddExact(node.member.GetText(), node)
            } else {
                ; Dynamic member access: extract constant parts from TS node
                this._ExtractDynamicMemberParts(node, table)
            }
        }

        if node is IR.CallExpr {
            this._CheckReflectionCall(node, table)
        }

        for child in node.children
            this._WalkForMemberNames(child, table)
    }

    /**
     * Analyze a dynamic member access's tree-sitter node to extract
     * any constant parts (outer prefix/suffix identifiers, inner string
     * literals in dereference expressions).
     *
     * @param {IR.MemberAccess} maNode the IR member access node
     * @param {MemberNameTable} table
     */
    _ExtractDynamicMemberParts(maNode, table) {
        memberTSNode := maNode.tsNode.GetChildByFieldName("member")
        if memberTSNode.IsNull {
            throw Error("Member access with no object?", , maNode.GetText())
        }

        ; Walk children of the dynamic_identifier to find:
        ;   - identifier nodes (outer constant text parts)
        ;   - dereference_operation nodes (dynamic parts)
        hasConstant := false
        outerPrefix := ""
        outerSuffix := ""
        derefNodes := []

        if memberTSNode.Type == "dereference_operation" {
            derefNodes.Push(memberTSNode)
            goto scan_derefs
        }

        i := 0
        count := memberTSNode.NamedChildCount
        while i < count {
            child := memberTSNode.GetNamedChild(i)
            childType := child.Type

            if childType == "identifier" {
                if derefNodes.Length == 0 {
                    ; Before any deref - outer prefix
                    outerPrefix .= child.Text
                } 
                else {
                    ; After a deref - outer suffix (could also be middle, but
                    ; we conservatively treat the last identifier as suffix)
                    outerSuffix := child.Text
                }
                hasConstant := true
            } else if childType == "dereference_operation" {
                derefNodes.Push(child)
                outerSuffix := "" ; reset suffix, we only want the trailing one
            }

            i++
        }

        scan_derefs:

        ; Check inner expressions of dereference operations for string literals
        for derefNode in derefNodes {
            operandNode := derefNode.GetChildByFieldName("operand")
            if operandNode.IsNull
                continue

            if operandNode.Type == "string_literal" {
                ; Pure string literal inside deref: %"literal"% → exact name
                ; Strip quotes from the literal text
                litText := operandNode.Text
                litText := SubStr(litText, 2, StrLen(litText) - 2)
                table.AddExact(outerPrefix . litText . outerSuffix, maNode)
                hasConstant := true
            } 
            else if operandNode.Type == "explicit_concat_operation" || operandNode.Type == "implicit_concat_operation" {
                ; Concatenation: check if either side is a string literal
                leftNode := operandNode.GetChildByFieldName("left")
                rightNode := operandNode.GetChildByFieldName("right")

                if !leftNode.IsNull && leftNode.Type == "string_literal" {
                    litText := leftNode.Text
                    litText := SubStr(litText, 2, StrLen(litText) - 2)
                    table.AddPrefix(outerPrefix . litText, maNode)
                    hasConstant := true
                }
                if !rightNode.IsNull && rightNode.Type == "string_literal" {
                    litText := rightNode.Text
                    litText := SubStr(litText, 2, StrLen(litText) - 2)
                    table.AddSuffix(litText . outerSuffix, maNode)
                    hasConstant := true
                }
            }
        }

        ; If we found outer prefix/suffix, add those as patterns
        if outerPrefix != "" {
            table.AddPrefix(outerPrefix, maNode)
            hasConstant := true
        }
        if outerSuffix != "" {
            table.AddSuffix(outerSuffix, maNode)
            hasConstant := true
        }

        ; No constant parts at all → analysis is defeated
        if !hasConstant {
            Log.Warn(Format("Member access with dereference expression '{1}' with no constant parts defeats member pruning", 
                Trim(maNode.GetText())))
            Log.Warn("Possible resolutions: `r`nUse DefineProp() or GetOwnPropDesc() `r`nAdd a constant prefix or suffix like %`"pre`" fix% to narrow possibilities")
            table.BlowUp()
        }
    }

    /**
     * Check if a CallExpr is a call to a configured reflection function
     * (e.g. ObjBindMethod) and extract the member name string argument.
     *
     * @param {IR.CallExpr} callNode
     * @param {MemberNameTable} table
     */
    _CheckReflectionCall(callNode, table) {
        ; Only handle calls where the callee is a simple identifier
        if !callNode.HasOwnProp("callee") || !(callNode.callee is IR.Identifier)
            return

        calleeName := callNode.callee.name
        if !TreeShaker.ReflectionFunctions.Has(calleeName)
            return

        config := TreeShaker.ReflectionFunctions[calleeName]
        argIdx := config.argIndex

        if callNode.args.Length < argIdx
            return

        arg := callNode.args[argIdx]
        this._ExtractStringExprParts(arg, table, callNode)
    }

    /**
     * Analyze an IR expression node for string constant parts and add them
     * to the name table. Handles string literals, concatenation with literal
     * sides, and falls back to BlowUp() if no constant parts are found.
     *
     * Used for both reflection function arguments and deref inner expressions.
     *
     * @param {IR.Node} expr the expression to analyze
     * @param {MemberNameTable} table
     * @param {IR.Node} contextNode the enclosing node (for error messages)
     * @param {String} prefix constant text to prepend (from outer context)
     * @param {String} suffix constant text to append (from outer context)
     */
    _ExtractStringExprParts(expr, table, contextNode, prefix := "", suffix := "") {
        if expr is IR.Literal && expr.literalType == "string" {
            ; Exact string literal
            table.AddExact(prefix . expr.value . suffix, contextNode.GetText())
            return
        }

        if expr is IR.BinaryExpr && (expr.operator == "." || expr.operator == " ") {
            ; Concatenation — check if either side is a string literal
            hasConstant := false

            if expr.HasOwnProp("left") && expr.left is IR.Literal && expr.left.literalType == "string" {
                table.AddPrefix(prefix . expr.left.value, expr)
                hasConstant := true
            }
            if expr.HasOwnProp("right") && expr.right is IR.Literal && expr.right.literalType == "string" {
                table.AddSuffix(expr.right.value . suffix, expr)
                hasConstant := true
            }

            if hasConstant
                return
        }

        ; No constant parts extractable — analysis defeated
        Log.Warn(Format("Non-constant member name expression defeats member pruning: {1}",
            contextNode.GetText()))
        table.BlowUp()
    }

    ; ==========================================================================
    ; DefineProp pruning
    ; ==========================================================================

    /**
     * Walk the IR tree and prune DefineProp calls that create properties whose
     * names never appear in the member name table. Must run BEFORE MarkLive so
     * that references inside pruned descriptors are not followed.
     *
     * @param {IR.Program} program
     * @param {MemberNameTable} nameTable
     * @returns {Integer} number of pruned calls
     */
    _PruneDefinePropCalls(program, nameTable) {
        if nameTable.isBlownUp
            return 0
        pruned := 0
        this._WalkForDefineProp(program, nameTable, &pruned)
        return pruned
    }

    /**
     * Recursive walker that checks each CallExpr for a prunable DefineProp call.
     *
     * @param {IR.Node} node
     * @param {MemberNameTable} nameTable
     * @param {VarRef<Integer>} &pruned counter
     */
    _WalkForDefineProp(node, nameTable, &pruned) {
        if node is IR.CallExpr {
            if this._TryPruneDefineProp(node, nameTable) {
                pruned++
                return ; Don't recurse into pruned call's children
            }
        }

        for child in node.children
            this._WalkForDefineProp(child, nameTable, &pruned)
    }

    /**
     * Check if a CallExpr is a prunable DefineProp call and mark it deleted.
     *
     * Conditions for pruning:
     *   1. Callee is a static MemberAccess with member "DefineProp"
     *   2. First argument is a string literal (property name)
     *   3. Property name is not a protected meta-function
     *   4. Property name is not in the member name table (never referenced)
     *   5. Call is a standalone statement (parent is Block or Program)
     *   6. Callee object is not a CallExpr (guards against chained calls)
     *
     * @param {IR.CallExpr} callNode
     * @param {MemberNameTable} nameTable
     * @returns {Boolean} true if pruned
     */
    _TryPruneDefineProp(callNode, nameTable) {
        ; Must have a callee that is a static MemberAccess named "DefineProp"
        if !callNode.HasOwnProp("callee") || !(callNode.callee is IR.MemberAccess)
            return false
        if callNode.callee.isDynamic
            return false
        if StrLower(callNode.callee.member.GetText()) != "defineprop"
            return false

        ; First argument must be a string literal
        if callNode.args.Length < 1
            return false
        nameArg := callNode.args[1]
        if !(nameArg is IR.Literal) || nameArg.literalType != "string"
            return false

        propName := nameArg.value

        ; Never prune protected meta-function names
        if TreeShaker.ProtectedMembers.Has(propName)
            return false

        ; Keep if the name is referenced anywhere outside of the DefineProp call itself
        if referencers := nameTable.Matches(propName) {
            if (referencers.Length > 1) || !referencers[1].IsDescendentOf(callNode)
                return false
        }

        ; Only prune standalone statement calls (parent is Block or Program)
        if !callNode.HasOwnProp("parent")
            return false
        if !(callNode.parent is IR.Block || callNode.parent is IR.Program)
            return false

        ; Guard against chained calls: obj.DefineProp("A", d).DefineProp("B", d)
        ; Pruning the outer would incorrectly delete the inner
        if callNode.callee.HasOwnProp("object") && callNode.callee.object is IR.CallExpr
            return false

        Log.Trace(Format("Pruning DefineProp call: property '{1}' is never referenced", propName))
        callNode.deleted := true
        ; Remove name table entries contributed by nodes inside this pruned call
        nameTable.RemoveDescendantReferencers(callNode)
        return true
    }
}
