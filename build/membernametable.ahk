#Include ir.ahk
#Include <Collections\Typed\TypedArray>
#Include <Collections\Typed\TypedMap>

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

        this.prefixPatterns[prefix].Push(node)
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

        this.suffixPatterns[suffix].Push(node)
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