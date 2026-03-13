/************************************************************************
 * @description Scope model, symbol tracking, and symbol table for the IR.
 * @author
 * @date 2026/02/27
 * @version 0.1.0
 ***********************************************************************/

#Requires AutoHotkey v2.0

#Include ir.ahk
#Include <log4ahk\Log>

/**
 * Represents a lexical scope in the program. Scopes form a chain from inner
 * to outer, ending at the global scope.
 *
 * AHK v2 scoping rules:
 *   - Top level is assume-global
 *   - Functions are assume-local (variables are local unless declared global/static)
 *   - A bare `global` statement at the top of a function switches it to assume-global
 *   - Fat arrow closures capture variables from the enclosing scope
 *   - Classes create a scope for static members
 */
class IRScope {

    /**
     * Enclosing scope (unset for the global scope).
     * @type {IRScope}
     */
    parent := unset

    /**
     * What kind of scope this is.
     * @type {"global" | "function" | "class" | "arrow"}
     */
    kind := ""

    /**
     * The IR node that owns this scope.
     * @type {IR.Node}
     */
    ownerNode := unset

    /**
     * Variables declared or implicitly created in this scope.
     * Key: lowercase variable name (AHK is case-insensitive)
     * Value: IRSymbol
     * @type {Map}
     */
    symbols := Map()

    /**
     * Explicit scope declarations in this scope (local x, global y, static z).
     * Key: lowercase variable name
     * Value: "local" | "global" | "static"
     * @type {Map}
     */
    declarations := Map()

    /**
     * Whether this scope defaults to local or global for undeclared variables.
     * Functions default to "local", top-level defaults to "global".
     * A function with a bare `global` statement switches to "global".
     * @type {"local" | "global"}
     */
    assumeMode := "local"

    /**
     * @param {String} kind scope kind
     * @param {IR.Node} ownerNode the IR node that owns this scope
     * @param {IRScope} parent enclosing scope, or unset for global
     */
    __New(kind, ownerNode, parent?) {
        this.kind := kind
        this.ownerNode := ownerNode
        if IsSet(parent)
            this.parent := parent

        if kind == "global"
            this.assumeMode := "global"
    }

    /**
     * Look up a variable name in this scope and its parents.
     * Follows AHK v2 resolution rules:
     *
     *   1. If explicitly declared in this scope, route accordingly
     *   2. If already known in this scope's symbols, return it
     *   3. If this is an assume-local function scope, implicitly create a local
     *   4. If this is assume-global, delegate to the global scope
     *   5. Walk parent scopes for closures / nested functions
     *   6. Last resort: create in global scope
     *
     * @param {String} name the variable name to resolve
     * @param {IRScope} globalScope the program's global scope (needed for global routing)
     * @returns {IRSymbol} the resolved symbol
     */
    Resolve(name, globalScope) {
        key := StrLower(name)

        ; 1. Check explicit declarations (local x, global y, static z)
        if this.declarations.Has(key) {
            switch this.declarations[key] {
                case "global":
                    return globalScope._GetOrCreate(key, name)
                case "local", "static":
                    return this._GetOrCreate(key, name)
            }
        }

        ; 2. Already known in this scope
        if this.symbols.Has(key)
            return this.symbols[key]

        ; 3. Assume-local function scope: implicitly create local
        if this.assumeMode == "local" && this.kind == "function"
            return this._GetOrCreate(key, name)

        ; 4. Assume-global: delegate to global scope
        if this.assumeMode == "global" && this.kind != "global"
            return globalScope._GetOrCreate(key, name)

        ; 5. Walk parent scopes (closures / nested)
        if this.HasOwnProp("parent")
            return this.parent.Resolve(name, globalScope)

        ; 6. Global scope itself — create here as last resort
        return this._GetOrCreate(key, name)
    }

    /**
     * Define a symbol in this scope explicitly (used during IR building for
     * function/class/param declarations).
     *
     * @param {String} name the symbol name (original casing)
     * @param {IRSymbol} symbol the symbol to register
     */
    Define(name, symbol) {
        this.symbols[StrLower(name)] := symbol
    }

    /**
     * Register an explicit scope declaration (local/global/static).
     *
     * @param {String} name the variable name
     * @param {String} declKind "local", "global", or "static"
     */
    Declare(name, declKind) {
        this.declarations[StrLower(name)] := declKind
    }

    /**
     * Check if a name is defined in this scope (not parents).
     *
     * @param {String} name the variable name
     * @returns {Boolean}
     */
    Has(name) => this.symbols.Has(StrLower(name))

    /**
     * Get a symbol from this scope only (not parents). Returns unset if not found.
     *
     * @param {String} name the variable name
     * @returns {IRSymbol}
     */
    Get(name) {
        key := StrLower(name)
        if this.symbols.Has(key)
            return this.symbols[key]

        return ""
    }

    /**
     * Get or create a symbol in this scope. Used internally during resolution
     * when a variable is implicitly created (assume-local / assume-global).
     *
     * @param {String} key lowercase name
     * @param {String} originalName original-cased name
     * @returns {IRSymbol}
     */
    _GetOrCreate(key, originalName) {
        if this.symbols.Has(key)
            return this.symbols[key]

        sym := IRSymbol(originalName, "variable")
        sym.scope := this
        this.symbols[key] := sym
        return sym
    }
}

/**
 * A single symbol in the program: a function, class, variable, property, label, or parameter.
 * Tracks definitions, references, constant values, and liveness for tree-shaking.
 */
class IRSymbol {

    /**
     * The symbol name as originally written (preserves casing for emission).
     * @type {String}
     */
    name := ""

    /**
     * What kind of symbol this is.
     * @type {"function" | "class" | "variable" | "property" | "label" | "param"}
     */
    kind := ""

    /**
     * The IR node where this symbol is declared.
     * @type {IR.Node}
     */
    node := unset

    /**
     * The scope this symbol belongs to.
     * @type {IRScope}
     */
    scope := unset

    ; --- Reference tracking ---

    /**
     * All assignment/definition sites.
     * @type {Array<IR.Node>}
     */
    definitions := []

    /**
     * All read sites (Identifier nodes that resolve to this symbol).
     * @type {Array<IR.Node>}
     */
    references := []

    /**
     * Number of definitions.
     * @type {Integer}
     */
    defCount => this.definitions.Length

    /**
     * Number of references.
     * @type {Integer}
     */
    refCount => this.references.Length

    ; --- Constant propagation ---

    /**
     * If this symbol has a single constant value, stores it here.
     * @type {String | Integer | Float}
     */
    constValue := unset

    /**
     * Type of constant value.
     * @type {"integer" | "float" | "string" | "boolean" | ""}
     */
    constType := ""

    /**
     * True if value never changes after initial assignment.
     * False if assigned more than once, is a param, loop var, captured, or by-ref.
     * @type {Boolean}
     */
    isConst := false

    ; --- Tree-shaking ---

    /**
     * Set to true during reachability analysis if this symbol is reachable
     * from entry points.
     * @type {Boolean}
     */
    isLive := false

    ; --- Inlining ---

    /**
     * For functions: number of call sites.
     * @type {Integer}
     */
    callCount := 0

    /**
     * Set by inlining analysis.
     * @type {Boolean}
     */
    isInlineable := false

    /**
     * @param {String} name the symbol name
     * @param {String} kind the symbol kind
     */
    __New(name, kind) {
        this.name := name
        this.kind := kind
    }
}

/**
 * Program-wide symbol table. Provides registration, lookup, reference tracking,
 * and reachability analysis (tree-shaking) for all declarations.
 */
class IRSymbolTable {

    /**
     * All symbols indexed by fully-qualified lowercase name.
     * E.g. "myclass.mymethod", "myfunc", "myclass.myprop"
     * @type {Map}
     */
    _symbols := (m := Map(), m.CaseSense := false, m)

    /**
     * Top-level functions.
     * @type {Map}
     */
    functions := (m := Map(), m.CaseSense := false, m)

    /**
     * Top-level classes.
     * @type {Map}
     */
    classes := (m := Map(), m.CaseSense := false, m)

    /**
     * Labels.
     * @type {Map}
     */
    labels := (m := Map(), m.CaseSense := false, m)

    /**
     * Register a symbol in the table.
     *
     * @param {String} fullyQualifiedName e.g. "MyClass.MyMethod" or "MyFunc"
     * @param {IRSymbol} symbol the symbol to register
     */
    Register(fullyQualifiedName, symbol) {
        key := Trim(StrLower(fullyQualifiedName))
        this._symbols[key] := symbol

        switch symbol.kind {
            case "function": this.functions[key] := symbol
            case "class":    this.classes[key] := symbol
            case "label":    this.labels[key] := symbol
        }
    }

    /**
     * Look up a symbol by name. Returns the symbol or an empty string if not found.
     *
     * @param {String} name the name to look up
     * @returns {IRSymbol | String} the symbol, or "" if not found
     */
    Lookup(name) {
        key := Trim(StrLower(name))
        return this._symbols.Has(key) ? this._symbols[key] : ""
    }

    /**
     * Check if a symbol is registered.
     *
     * @param {String} name the name to check
     * @returns {Boolean}
     */
    Has(name) => this._symbols.Has(Trim(StrLower(name)))

    /**
     * Record that an IR node references a symbol (a read site).
     *
     * @param {IRSymbol} symbol the symbol being referenced
     * @param {IR.Node} refNode the node that references it
     */
    AddReference(symbol, refNode) {
        symbol.references.Push(refNode)
    }

    /**
     * Record a definition site for a symbol (an assignment or declaration).
     *
     * @param {IRSymbol} symbol the symbol being defined
     * @param {IR.Node} defNode the node that defines it
     */
    AddDefinition(symbol, defNode) {
        symbol.definitions.Push(defNode)
    }

    /**
     * Run reachability analysis for tree-shaking. Starts from the given entry
     * points and marks everything reachable as live using a worklist algorithm.
     *
     * Entry points include:
     *   - Top-level non-declaration statements (auto-execute section)
     *   - Hotkeys and hotstrings
     *   - #HotIf directive expressions
     *   - static __New() methods on any class
     *   - Labels referenced by Goto / string-based builtins
     *
     * @param {Array<IRSymbol>} entryPoints symbols to start from
     * @param {MemberNameTable} nameTable optional — if provided, enables per-member
     *        pruning within live classes. Members are only kept if their name appears
     *        in the table or is a protected meta-function. If omitted, all members of
     *        live classes are kept (whole-class granularity).
     */
    MarkLive(entryPoints, nameTable := "") {
        worklist := []
        for ep in entryPoints
            worklist.Push(ep)

        while worklist.Length > 0 {
            sym := worklist.Pop()
            if sym.isLive
                continue
            sym.isLive := true
            Log.Trace(Format.Bind("Marking {1} '{2}' live", sym.kind, sym.name))

            ; If this is a class, propagate liveness to member symbols.
            ; With a nameTable, only members whose names are referenced are kept.
            ; Without one, all members are kept (whole-class granularity).
            if sym.kind == "class" && sym.HasOwnProp("node") && sym.node is IR.ClassDecl
                this._MarkClassMembersLive(sym, worklist, nameTable)

            ; Walk the symbol's declaring node to find all references to other symbols.
            ; Add any un-visited symbols to the worklist.
            if sym.HasOwnProp("node")
                this._CollectReferencesInto(sym.node, worklist)
        }
    }

    /**
     * Walk an IR node subtree and collect all symbols referenced by Identifier,
     * CallExpr, MemberAccess, etc. Adds unreached symbols to the worklist.
     *
     * @param {IR.Node} node the node to walk
     * @param {Array<IRSymbol>} worklist the worklist to add to
     */
    _CollectReferencesInto(node, worklist) {
        ; Skip nodes already marked as deleted (e.g., pruned DefineProp calls)
        if node.deleted
            return

        ; If this node is an Identifier with a resolved symbol, add it
        if node is IR.Identifier {
            if node.HasOwnProp("resolvedSymbol") {
                if !node.resolvedSymbol.isLive
                    worklist.Push(node.resolvedSymbol)
            }
        }

        ; If this is a CallExpr with a resolved target, add that function's symbol
        if node is IR.CallExpr {
            if node.HasOwnProp("resolvedTarget") {
                targetSym := this.Lookup(node.resolvedTarget.name)
                if targetSym != "" && !targetSym.isLive
                    worklist.Push(targetSym)
            }
        }

        ; ClassDecl superclass is stored as a raw string, not an IR.Identifier child.
        ; Must explicitly look it up.
        if node is IR.ClassDecl {
            if node.superclass != "" {
                superSym := this.Lookup(node.superclass)
                if superSym != "" && !superSym.isLive
                    worklist.Push(superSym)
            }
        }

        ; GotoStmt label is a raw string, not an IR.Identifier child.
        if node is IR.GotoStmt {
            if node.label != "" {
                labelSym := this.Lookup(node.label)
                if labelSym != "" && !labelSym.isLive
                    worklist.Push(labelSym)
            }
        }

        ; CatchClause error types are raw strings (class names).
        if node is IR.CatchClause {
            for errorType in node.errorTypes {
                errSym := this.Lookup(errorType)
                if errSym != "" && !errSym.isLive
                    worklist.Push(errSym)
            }
        }

        if node is IR.MemberAccess {
            this._CollectReferencesInto(node.object, worklist)
        }

        if node is IR.Block {
            for bodyNode in node.body {
                this._CollectReferencesInto(bodyNode, worklist)
            }
        }

        ; Recurse into children
        for child in node.children
            this._CollectReferencesInto(child, worklist)
    }

    /**
     * When a class is marked live, push its member symbols onto the worklist.
     *
     * If `nameTable` is provided, only members whose names are referenced in
     * the program (or are protected meta-functions) are pushed. Otherwise all
     * members are pushed (whole-class granularity).
     *
     * @param {IRSymbol} classSym the class symbol
     * @param {Array<IRSymbol>} worklist the worklist to add to
     * @param {MemberNameTable} nameTable optional member name filter
     */
    _MarkClassMembersLive(classSym, worklist, nameTable := "") {
        prefix := StrLower(classSym.node.fullyQualifiedName) "."
        prefixLen := StrLen(prefix)
        for fqn, memberSym in this._symbols {
            if SubStr(fqn, 1, prefixLen) != prefix || memberSym.isLive
                continue

            ; No name table → keep all members (current behavior)
            if nameTable == "" {
                worklist.Push(memberSym)
                continue
            }

            ; Extract the member's own name (after the class FQN prefix)
            memberName := SubStr(fqn, prefixLen + 1)

            ; Always keep protected meta-functions
            if TreeShaker.ProtectedMembers.Has(memberName) {
                worklist.Push(memberSym)
                continue
            }

            ; Keep if name appears in the member name table
            if nameTable.Matches(memberName) {
                worklist.Push(memberSym)
            } else {
                Log.Trace(Format("  Pruning member '{1}' — name not referenced", fqn))
            }
        }
    }

    /**
     * Get all symbols that are dead (not reachable from entry points).
     * Call this after MarkLive().
     *
     * @returns {Array<IRSymbol>}
     */
    GetDeadSymbols() {
        dead := []
        for _, sym in this._symbols {
            if !sym.isLive
                dead.Push(sym)
        }
        return dead
    }

    /**
     * Mark all dead symbols' nodes as deleted.
     * Call this after MarkLive() to prepare for emission.
     */
    DeleteDeadSymbols() {
        for _, sym in this._symbols {
            if !sym.isLive && sym.HasOwnProp("node")
                sym.node.deleted := true
        }
    }

    /**
     * Dumps the contents of the symbol table into the log at level TRACE, for
     * debugging
     * @returns {String} the symbol table
     */
    TraceDump() {
        str := ""
        for fqn, sym in this._symbols {
            str .= Format("{5} - {1} - {2} '{3}' (live: {4})`r`n", 
                fqn, sym.kind, sym.name, sym.isLive, A_Index)
        }

        return str
    }
}
