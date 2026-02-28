/************************************************************************
 * @description Patch-based emitter for reconstructing source from the IR.
 * @author
 * @date 2026/02/27
 * @version 0.1.0
 ***********************************************************************/

#Requires AutoHotkey v2.0

#Include ir.ahk

/**
 * Represents a single patch to the source buffer.
 * Replaces the byte range [start, end) with the replacement text.
 */
class Patch {

    /**
     * Start byte offset (inclusive).
     * @type {Integer}
     */
    start := 0

    /**
     * End byte offset (exclusive).
     * @type {Integer}
     */
    end := 0

    /**
     * The replacement text. Empty string means deletion.
     * @type {String}
     */
    replacement := ""

    /**
     * @param {Integer} start start byte offset
     * @param {Integer} end end byte offset
     * @param {String} replacement the replacement text
     */
    __New(start, end, replacement) {
        this.start := start
        this.end := end
        this.replacement := replacement
    }
}

/**
 * Walks the IR tree and collects patches, then applies them to the original
 * source buffer to produce the output.
 *
 * Emission strategy (hybrid):
 *   - Unmodified nodes: original source is preserved verbatim (formatting, comments, etc.)
 *   - Nodes with _overrideText: replaced with the override text
 *   - Deleted nodes: removed entirely (replaced with "")
 *   - An override on a parent supersedes any overrides on its children
 *     (the emitter does NOT recurse into overridden/deleted nodes)
 *
 * Patches are collected, sorted by byte offset, validated for non-overlap,
 * and applied in reverse order so earlier byte offsets remain valid.
 */
class Emitter {

    /**
     * Collected patches, sorted by start byte after collection.
     * @type {Array<Patch>}
     */
    patches := []

    /**
     * The original source buffer.
     * @type {Buffer}
     */
    sourceBuffer := unset

    /**
     * The encoding used to read strings from the source buffer.
     * Matches what TSParser.Parse stored on the tree.
     * @type {String}
     */
    encoding := "UTF-8"

    /**
     * Collect all patches from the IR tree and emit the transformed source.
     *
     * @param {IR.Program} program the IR program node
     * @returns {String} the emitted source text
     */
    Emit(program) {
        this.sourceBuffer := program.sourceBuffer
        this.patches := []

        ; Walk the IR tree to collect patches
        this._Walk(program)

        ; Sort patches by start byte (ascending)
        this._SortPatches()

        ; Validate no overlapping patches
        this._ValidatePatches()

        ; Apply patches to produce the output
        return this._ApplyPatches()
    }

    /**
     * Recursively walk the IR tree collecting patches from transformed nodes.
     *
     * Key principle: an override on a parent node supersedes any overrides on
     * its children. When we see a node with _overrideText or deleted=true, we
     * create a patch and do NOT recurse into children.
     *
     * @param {IR.Node} node the node to walk
     */
    _Walk(node) {
        ; Deleted nodes produce a deletion patch (empty replacement)
        if node.deleted {
            this.patches.Push(Patch(node.start, node.end, ""))
            return
        }

        ; Overridden nodes produce a replacement patch
        if node.HasOwnProp("_overrideText") {
            this.patches.Push(Patch(node.start, node.end, node._overrideText))
            return
        }

        ; Otherwise recurse into children
        for child in node.children
            this._Walk(child)
    }

    /**
     * Sort patches by start byte offset (ascending).
     * Uses insertion sort since patch count is typically small relative to
     * the source size, and the list is often nearly sorted already.
     */
    _SortPatches() {
        patches := this.patches
        i := 2
        while i <= patches.Length {
            key := patches[i]
            j := i - 1
            while j >= 1 && patches[j].start > key.start {
                patches[j + 1] := patches[j]
                j--
            }
            patches[j + 1] := key
            i++
        }
    }

    /**
     * Validate that no patches overlap. Overlapping patches indicate a bug
     * in the transformation passes (e.g. both a parent and child were marked
     * as transformed, which _Walk should prevent).
     *
     * @throws {Error} if overlapping patches are detected
     */
    _ValidatePatches() {
        patches := this.patches
        i := 1
        while i < patches.Length {
            curr := patches[i]
            next := patches[i + 1]
            if curr.end > next.start {
                throw Error(
                    Format("Overlapping patches detected: [{1}, {2}) and [{3}, {4})",
                        curr.start, curr.end, next.start, next.end),
                    -1
                )
            }
            i++
        }
    }

    /**
     * Apply all collected patches to the source buffer and return the result.
     *
     * Works by building the output from left to right: for each gap between
     * patches, copy the original source bytes; for each patch, insert the
     * replacement text.
     *
     * @returns {String} the fully patched source text
     */
    _ApplyPatches() {
        buf := this.sourceBuffer
        patches := this.patches
        result := ""
        pos := 0

        for patch in patches {
            ; Copy original source from current position to patch start
            if patch.start > pos
                result .= StrGet(buf.Ptr + pos, patch.start - pos, this.encoding)

            ; Insert replacement text
            result .= patch.replacement

            ; Advance past the patched region
            pos := patch.end
        }

        ; Copy any remaining source after the last patch
        if pos < buf.Size
            result .= StrGet(buf.Ptr + pos, buf.Size - pos, this.encoding)

        return result
    }
}
