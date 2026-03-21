/************************************************************************
 * @description Intermediate Representation for AHK build transformations
 * @author
 * @date 2026/02/27
 * @version 0.1.0
 ***********************************************************************/

#Requires AutoHotkey v2.0

#Include <tree-sitter\TSNode>
#Include <Collections\Typed\TypedArray>
#Include <Collections\Typed\TypedMap>

/**
 * The intermediate representation used to perform transformations on source trees.
 * All IR node types are nested classes within this container.
 */
class IR {

    __Item[val] {
        get => MsgBox(val)
    }

    ; =========================================================================
    ; 1.0 DirectiveComment (metadata, not an IR node)
    ; =========================================================================

    /**
     * A parsed directive comment (`;@Name arguments`). Attached as metadata to
     * the following statement-level IR node. Not part of the IR tree itself.
     */
    class DirectiveComment {
        /** @type {String} directive name, case-preserved */
        name := ""
        /** @type {String} raw argument string after the directive name */
        arguments := ""
        /** @type {TSNode} original directive_comment node */
        tsNode := unset

        __New(name, arguments, tsNode) {
            this.name := name
            this.arguments := arguments
            this.tsNode := tsNode
        }
    }

    ; =========================================================================
    ; 1.1 Base & Structural Nodes
    ; =========================================================================

    /**
     * Base class for all IR nodes. Every node stores a reference to its
     * originating tree-sitter node for source mapping and patch-based emission.
     */
    class Node {

        ; --- Source mapping ---

        /**
         * The tree-sitter node this IR node was generated from.
         * @type {TSNode}
         */
        tsNode := unset

        /**
         * Start byte offset in source.
         * @type {Integer}
         */
        start => this.tsNode.StartByte

        /**
         * End byte offset in source.
         * @type {Integer}
         */
        end => this.tsNode.EndByte

        ; --- Tree structure ---

        /**
         * Parent node in the IR tree.
         * @type {IR.Node}
         */
        parent := unset

        /**
         * Child nodes.
         * @type {Array<IR.Node>}
         */
        children := TypedArray(IR.Node)

        ; --- Emission ---

        /**
         * When true, the emitter skips this node entirely (produces no output).
         * @type {Boolean}
         */
        deleted := false

        __New(parent?, node?) {
            if IsSet(parent)
                this.parent := parent

            if IsSet(node)
                this.tsNode := node
        }

        /**
         * Gets the text for emission. If an override has been set, returns that;
         * otherwise returns the original source text from the tree-sitter node.
         * @returns {String}
         */
        GetText() => this.HasOwnProp("_overrideText") ? this._overrideText : this.tsNode.Text

        /**
         * Sets override text, marking this node as transformed.
         * @param {String} text the replacement text
         */
        SetOverride(text) => this._overrideText := text

        /**
         * Clears override text, reverting to original source.
         */
        ClearOverride() => this.DeleteProp("_overrideText")

        /**
         * Determines whether this node is a descendant of `other` by walking
         * parents up to the root
         * 
         * @param {IR.Node} other the node to check against
         * @returns 1 if `this` is a child of `other`, 0 otherwise 
         */
        IsDescendentOf(other) {
            node := this.HasOwnProp("parent") && this.parent
            while !(node is IR.Program) {
                if node == other {
                    return true
                }

                node := node.HasOwnProp("parent") && node.parent
            }

            return false
        }

        /**
         * Whether this node has been transformed from its original source.
         * @type {Boolean}
         */
        IsTransformed => this.HasOwnProp("_overrideText") || this.deleted

        ; --- Directive comments ---

        /**
         * Attaches a parsed directive comment to this node.
         * @param {IR.DirectiveComment} directive
         */
        AddDirective(directive) {
            if !this.HasOwnProp("_directives")
                this._directives := TypedArray(IR.DirectiveComment)
            this._directives.Push(directive)
        }

        /**
         * Whether this node has an attached directive with the given name.
         * Comparison is case-insensitive.
         * @param {String} name
         * @returns {Boolean}
         */
        HasDirective(name) {
            node := this

            ; Loop upwards until the end of the statement
            while !(node is IR.Program || node is IR.Block || node is IR.ClassDecl) {
                if node.GetDirective(name)
                    return true
                node := node.parent
            }
            
            return false
        }

        /**
         * Returns the first attached directive with the given name, or an empty string.
         * @param {String} name
         * @returns {IR.DirectiveComment}
         */
        GetDirective(name) {
            for d in this.Directives
                if d.name = name
                    return d
            return ""
        }

        /**
         * All attached directive comments, or an empty array.
         * @type {Array<IR.DirectiveComment>}
         */
        Directives => this.HasOwnProp("_directives") ? this._directives : []

        /**
         * Returns a string that can be used to pretty-print the IR tree
         */
        ToDetailedString(indent := "", isLast := true) {
            nodeTextLine := Trim(StrReplace(StrReplace(this.GetText(), "`n"), "`t"))
            if(InStr(this.GetText(), "`n"))
                nodeTextLine .= "..."

            prefix := indent = "" ? "" : indent . (isLast ? "└─ " : "├─ ")
            str := Format("{1}{2} ({3}), [{4}, {5}]: `"{6}`"",
                prefix, Type(this), this.tsNode.Type, this.start, this.end, nodeTextLine)

            childIndent := indent . (isLast ? "   " : "│  ")
            for (i, child in this.children) {
                isChildLast := (i = this.children.Length)
                str .= "`r`n" . child.ToDetailedString(childIndent, isChildLast)
            }

            return str
        }
    }

    /**
     * The root node representing the entire program (merged source file).
     * Owns the global scope and the symbol table.
     */
    class Program extends IR.Node {
        /**
         * All top-level statements and declarations.
         * @type {Array<IR.Node>}
         */
        body := TypedArray(IR.Node)

        /**
         * The global scope.
         * @type {IR.Scope}
         */
        scope := unset

        /**
         * Program-wide symbol registry.
         * @type {IR.SymbolTable}
         */
        symbolTable := unset

        /**
         * The original source buffer (for emission).
         * @type {Buffer}
         */
        sourceBuffer := unset

        /**
         * The tree-sitter tree.
         * @type {TSTree}
         */
        tree := unset
    }

    /**
     * A block of statements enclosed in braces.
     */
    class Block extends IR.Node {
        /**
         * Statements within the block.
         * @type {Array<IR.Node>}
         */
        body := TypedArray(IR.Node)

        /**
         * Scope associated with this block, if any.
         * @type {IR.Scope}
         */
        scope := unset

        /**
         * True if the block contains no statements
         * @type {Boolean}
         */
        isEmpty => this.body.Length = 0
    }

    /**
     * A node the IR doesn't need to analyze. Emitted verbatim via tsNode.Text.
     */
    class Opaque extends IR.Node {
    }

    /**
     * A standalone expression used as a statement.
     */
    class ExpressionStatement extends IR.Node {
        /**
         * The expression.
         * @type {IR.Node}
         */
        expression := unset
    }

    ; =========================================================================
    ; 1.2 Declaration Nodes
    ; =========================================================================

    /**
     * A function or method declaration.
     * Maps from: function_declaration, method_declaration
     */
    class Function extends IR.Node {
        /**
         * Function name (empty for anonymous fat arrows).
         * @type {String}
         */
        name := ""

        /**
         * Parameter list.
         * @type {Array<IR.Param>}
         */
        params := TypedArray(IR.Param)

        /**
         * Function body — either an IR.Block or an expression node (for arrow functions).
         * @type {IR.Block | IR.Node}
         */
        body := unset

        /**
         * AHK scope qualifier.
         * @type {"" | "static"}
         */
        scope := ""

        /**
         * True if this is a method inside a class.
         * @type {Boolean}
         */
        isMethod := false

        /**
         * True if last param is variadic (p*).
         * @type {Boolean}
         */
        isVariadic := false

        /**
         * True if defined with => (single expression body).
         * @type {Boolean}
         */
        isArrow := false

        /**
         * Fully-qualified class name if this is a method.
         * @type {String}
         */
        ownerClass := ""

        ; --- Analysis metadata ---

        /**
         * Number of call sites referencing this function.
         * @type {Integer}
         */
        callCount := 0

        /**
         * Set by inlining analysis pass.
         * @type {Boolean}
         */
        canInline := false

        /**
         * True if function calls itself directly or indirectly.
         * @type {Boolean}
         */
        isRecursive := false

        /**
         * True if the function body is empty
         * @type {Boolean}
         */
        isEmpty => this.HasOwnProp("body") && this.body is IR.Block && this.body.isEmpty

        /**
         * True if function has side effects (set by analysis).
         * @type {Boolean}
         */
        sideEffects := unset

        /**
         * Inferred return type for constant propagation.
         * @type {String}
         */
        returnType := "unknown"

        ; --- Scope ---

        /**
         * The function's own local scope.
         * @type {IR.Scope}
         */
        localScope := unset
    }

    /**
     * A single function/method parameter.
     * Maps from: identifier, byref_param, default_param, variadic_param, optional_param
     */
    class Param extends IR.Node {
        /**
         * @type {String}
         */
        name := ""

        /**
         * Default value expression, if any.
         * @type {IR.Node}
         */
        default := unset

        /**
         * &param
         * @type {Boolean}
         */
        isByRef := false

        /**
         * param*
         * @type {Boolean}
         */
        isVariadic := false

        /**
         * param?
         * @type {Boolean}
         */
        isOptional := false
    }

    /**
     * A class declaration.
     * Maps from: class_declaration
     */
    class ClassDecl extends IR.Node {
        /**
         * @type {String}
         */
        name := ""

        /**
         * Superclass name ("" if none, may be dotted for nested: "Base.Inner").
         * @type {String}
         */
        superclass := ""

        /**
         * Methods (including __New, __Delete, static __New, etc.).
         * @type {Array<IR.Function>}
         */
        methods := TypedArray(IR.Function)

        /**
         * Property declarations (with getter/setter or arrow).
         * @type {Array<IR.Property>}
         */
        properties := TypedArray(IR.Property)

        /**
         * Static fields (static prop := value).
         * @type {Array<IR.Field>}
         */
        staticFields := TypedArray(IR.Field)

        /**
         * Instance fields (prop := value).
         * @type {Array<IR.Field>}
         */
        instanceFields := TypedArray(IR.Field)

        /**
         * Nested class declarations.
         * @type {Array<IR.ClassDecl>}
         */
        nestedClasses := TypedArray(IR.ClassDecl)

        ; --- Analysis metadata ---

        /**
         * Number of references to this class.
         * @type {Integer}
         */
        refCount := 0

        /**
         * True if `ClassName()` or `.Call()` found.
         * @type {Boolean}
         */
        isInstantiated := false

        /**
         * E.g. "Outer.Inner" for nested classes.
         * @type {String}
         */
        fullyQualifiedName := ""

        ; --- Scope ---

        /**
         * Scope for the class body (static context).
         * @type {IR.Scope}
         */
        classScope := unset
    }

    /**
     * A property declaration within a class.
     * Maps from: property_declaration (when it has a getter/setter block or => arrow)
     *
     * Fat arrows appear in three property forms:
     *   - Shorthand getter-only: `Prop => value`
     *   - Arrow getter in block: `Prop { get => expr }`
     *   - Arrow setter in block: `Prop { set => expr }`
     * Static arrow properties are commonly used as constants/enum values
     * (e.g. Encoding.Utf8) and are prime inlining targets.
     */
    class Property extends IR.Node {
        /**
         * @type {String}
         */
        name := ""

        /**
         * @type {"" | "static"}
         */
        scope := ""

        /**
         * Getter function body (get { ... }, get => expr, or shorthand Prop => expr).
         * @type {IR.Function}
         */
        getter := unset

        /**
         * Setter function body (set { ... } or set => expr).
         * @type {IR.Function}
         */
        setter := unset

        /**
         * True for shorthand `Prop => expr` (no setter possible).
         * @type {Boolean}
         */
        isGetterOnly := false

        /**
         * True if getter uses => syntax (prime inlining candidate).
         * @type {Boolean}
         */
        isArrowGetter := false

        /**
         * True if setter uses => syntax.
         * @type {Boolean}
         */
        isArrowSetter := false

        ; --- Analysis metadata ---

        /**
         * How many times the getter is accessed.
         * @type {Integer}
         */
        getterCallCount := 0

        /**
         * How many times the setter is written.
         * @type {Integer}
         */
        setterCallCount := 0

        /**
         * @type {Boolean}
         */
        canInlineGetter := false
    }

    /**
     * A field declaration (simple property := value in a class body).
     * Maps from: property_declaration with just an initializer
     */
    class Field extends IR.Node {
        /**
         * @type {String}
         */
        name := ""

        /**
         * @type {"" | "static"}
         */
        scope := ""

        /**
         * The initial value expression.
         * @type {IR.Node}
         */
        initializer := unset
    }

    /**
     * An explicit variable declaration (local x, global y, static z).
     * Maps from: variable_declaration (scope_identifier + identifier)
     */
    class VarDecl extends IR.Node {
        /**
         * @type {String}
         */
        name := ""

        /**
         * @type {"local" | "global" | "static"}
         */
        declScope := ""

        /**
         * Optional := initializer.
         * @type {IR.Node}
         */
        initializer := unset
    }

    ; =========================================================================
    ; 1.3 Expression Nodes
    ; =========================================================================

    /**
     * A binary operation.
     * Maps from: additive_operation, multiplicative_operation, exponent_operation,
     *   relational_operation, equality_operation, inequality_operation,
     *   logical_and_operation, logical_or_operation, bitwise_and/or/xor_operation,
     *   bitshift_operation, explicit_concat_operation, implicit_concat_operation,
     *   or_maybe_operation, assignment_operation
     */
    class BinaryExpr extends IR.Node {
        /**
         * Left operand.
         * @type {IR.Node}
         */
        left := unset

        /**
         * The operator text: "+", "-", "*", "&&", ":=", ".", " ", "??", etc.
         * @type {String}
         */
        operator := ""

        /**
         * Right operand.
         * @type {IR.Node}
         */
        right := unset

        ; --- Constant folding ---

        /**
         * If folded, the computed value.
         * @type {String | Integer | Float}
         */
        foldedValue := unset

        /**
         * Type of folded value.
         * @type {"integer" | "float" | "string" | ""}
         */
        foldedType := ""
    }

    /**
     * A unary operation (prefix or postfix).
     * Maps from: prefix_operation, postfix_operation, verbal_not_operation
     */
    class UnaryExpr extends IR.Node {
        /**
         * The operator: "!", "~", "++", "--", "not", "+", "-"
         * @type {String}
         */
        operator := ""

        /**
         * The operand expression.
         * @type {IR.Node}
         */
        operand := unset

        /**
         * True for prefix, false for postfix.
         * @type {Boolean}
         */
        isPrefix := true

        ; --- Constant folding ---

        /** @type {String | Integer | Float} */
        foldedValue := unset

        /** @type {"integer" | "float" | "string" | ""} */
        foldedType := ""
    }

    /**
     * A ternary expression: condition ? trueBranch : falseBranch
     * Maps from: ternary_expression
     */
    class TernaryExpr extends IR.Node {
        /**
         * @type {IR.Node}
         */
        condition := unset

        /**
         * Value if condition is truthy.
         * @type {IR.Node}
         */
        trueBranch := unset

        /**
         * Value if condition is falsy.
         * @type {IR.Node}
         */
        falseBranch := unset
    }

    /**
     * A function or method call expression.
     * Maps from: function_call, call_statement (command-style)
     *
     * Note: Calling a non-function object (e.g. obj() where obj is a class instance)
     * implicitly invokes obj.Call(). The reference resolution pass must account for this.
     */
    class CallExpr extends IR.Node {
        /**
         * What's being called (Identifier, MemberAccess, etc.).
         * @type {IR.Node}
         */
        callee := unset

        /**
         * Argument expressions.
         * @type {Array<IR.Node>}
         */
        args := TypedArray(IR.Node)

        /**
         * True for call_statement (command-style, no parens).
         * @type {Boolean}
         */
        isCommandStyle := false

        /**
         * True if callee is %expr%() or a deref.
         * @type {Boolean}
         */
        isDynamic := false

        /**
         * If callee resolves to a known function.
         * @type {IR.Function}
         */
        resolvedTarget := unset
    }

    /**
     * A literal value.
     * Maps from: integer_literal, float_literal, hex_literal, string_literal, boolean_literal
     */
    class Literal extends IR.Node {
        /**
         * The parsed literal value.
         * @type {String | Integer | Float}
         */
        value := unset

        /**
         * @type {"integer" | "float" | "string" | "boolean"}
         */
        literalType := ""

        /**
         * The raw source text (preserves hex notation, quotes, etc.).
         * @type {String}
         */
        raw := ""
    }

    /**
     * An identifier (variable reference, function name reference).
     * Maps from: identifier (in expression contexts)
     */
    class Identifier extends IR.Node {
        /**
         * @type {String}
         */
        name := ""

        ; --- Scope resolution (populated during reference resolution phase) ---

        /**
         * The symbol this identifier refers to.
         * @type {IR.Symbol}
         */
        resolvedSymbol := unset

        /**
         * Which scope owns the resolved symbol.
         * @type {IR.Scope}
         */
        resolvedScope := unset
    }

    /**
     * A dynamic identifier like `%expr%` or `ext%expr%`
     * Maps from: dynamic_identifier
     */
    class DynamicIdentifier extends IR.Node {

    }

    /**
     * A member access expression: obj.member
     * Maps from: member_access
     */
    class MemberAccess extends IR.Node {
        /**
         * The object expression.
         * @type {IR.Node}
         */
        object := unset

        /**
         * The member name.
         * @type {String}
         */
        member := ""

        /**
         * True when the member is a dynamic_identifier (contains %expr%).
         * @type {Boolean}
         */
        isDynamic := false
    }

    /**
     * An index access expression: obj[index]
     * Maps from: index_access
     *
     * Note: Implicitly invokes __Item on the object.
     */
    class IndexAccess extends IR.Node {
        /**
         * The object expression.
         * @type {IR.Node}
         */
        object := unset

        /**
         * Index argument(s).
         * @type {Array<IR.Node>}
         */
        args := TypedArray(IR.Node)
    }

    /**
     * An array literal: [a, b, c]
     * Maps from: array_literal
     */
    class ArrayLiteral extends IR.Node {
        /**
         * Element expressions.
         * @type {Array<IR.Node>}
         */
        elements := TypedArray(IR.Node)
    }

    /**
     * An object literal: {key: val, key2: val2}
     * Maps from: object_literal
     */
    class ObjectLiteral extends IR.Node {
        /**
         * Key-value pairs. Each element is {key: IR.Node, value: IR.Node}.
         * @type {Array<Object>}
         */
        pairs := []
    }

    /**
     * A dereference expression: %expr%
     * Maps from: dereference_operation
     *
     * Critical for marking calls/references as dynamic (un-analyzable).
     */
    class DerefExpr extends IR.Node {
        /**
         * The expression inside %...%
         * @type {IR.Node}
         */
        inner := unset
    }

    /**
     * A VarRef expression: &var
     * Maps from: varref_operation
     */
    class VarRefExpr extends IR.Node {
        /**
         * The variable being referenced.
         * @type {IR.Node}
         */
        operand := unset
    }

    /**
     * An anonymous fat arrow function expression: (params) => expr
     * Maps from: fat_arrow_function (when used as an expression, not a named declaration)
     */
    class FatArrow extends IR.Node {
        /**
         * @type {Array<IR.Param>}
         */
        params := TypedArray(IR.Param)

        /**
         * The body expression.
         * @type {IR.Node}
         */
        body := unset

        /**
         * Variables captured from enclosing scope.
         * @type {Array<String>}
         */
        captures := TypedArray(String)

        /**
         * The arrow's own scope.
         * @type {IR.Scope}
         */
        localScope := unset
    }

    ; =========================================================================
    ; 1.4 Control Flow Nodes
    ; =========================================================================

    /**
     * An if statement, potentially with else-if and else branches.
     * Maps from: if_statement + else_statement chain
     */
    class IfStmt extends IR.Node {
        /**
         * The condition expression.
         * @type {IR.Node}
         */
        condition := unset

        /**
         * The "then" block.
         * @type {IR.Block}
         */
        thenBody := unset

        /**
         * Either IR.Block (else), IR.IfStmt (else-if chain), or unset.
         * @type {IR.Block | IR.IfStmt}
         */
        elseBody := unset

        ; --- Dead branch elimination ---

        /**
         * If the condition folds to a known value.
         * @type {Boolean}
         */
        conditionValue := unset
    }

    /**
     * A while loop.
     * Maps from: while_statement
     */
    class WhileStmt extends IR.Node {
        /**
         * @type {IR.Node}
         */
        condition := unset

        /**
         * @type {IR.Block}
         */
        body := unset
    }

    /**
     * A for-in loop: for key, val in iterable
     * Maps from: for_statement
     *
     * Note: Implicitly invokes __Enum on iterable, plus the enumerator's Call method.
     */
    class ForStmt extends IR.Node {
        /**
         * Loop variables (key, value, etc.).
         * @type {Array<IR.Identifier>}
         */
        iterators := TypedArray(IR.Identifier)

        /**
         * What is being iterated.
         * @type {IR.Node}
         */
        iterable := unset

        /**
         * @type {IR.Block}
         */
        body := unset

        /**
         * Optional else clause.
         * @type {IR.Block}
         */
        elseBody := unset
    }

    /**
     * A loop statement (Loop, Loop N, Loop Parse, Loop Read, Loop Reg, Loop Files).
     * Maps from: loop_statement
     */
    class LoopStmt extends IR.Node {
        /**
         * @type {"count" | "parse" | "read" | "reg" | "files" | "infinite"}
         */
        kind := ""

        /**
         * Loop expression/arguments (may be unset for infinite loops).
         * @type {IR.Node}
         */
        head := unset

        /**
         * @type {IR.Block}
         */
        body := unset

        /**
         * Optional Until clause.
         * @type {IR.Node}
         */
        untilCondition := unset
    }

    /**
     * A switch statement.
     * Maps from: switch_statement
     */
    class SwitchStmt extends IR.Node {
        /**
         * The value being switched on.
         * @type {IR.Node}
         */
        discriminant := unset

        /**
         * Case clauses.
         * @type {Array<IR.CaseClause>}
         */
        cases := TypedArray(IR.CaseClause)

        /**
         * The default clause, if any.
         * @type {IR.CaseClause}
         */
        defaultCase := unset
    }

    /**
     * A single case clause within a switch.
     * Maps from: case_clause, default_clause
     */
    class CaseClause extends IR.Node {
        /**
         * The case value expressions (empty for default).
         * @type {Array<IR.Node>}
         */
        values := TypedArray(IR.Node)

        /**
         * Statements in this case.
         * @type {Array<IR.Node>}
         */
        body := TypedArray(IR.Node)

        /**
         * True for the default clause.
         * @type {Boolean}
         */
        isDefault := false
    }

    /**
     * A try statement.
     * Maps from: try_statement
     */
    class TryStmt extends IR.Node {
        /**
         * @type {IR.Block}
         */
        tryBody := unset

        /**
         * @type {Array<IR.CatchClause>}
         */
        catchClauses := TypedArray(IR.CatchClause)

        /**
         * Optional else block.
         * @type {IR.Block}
         */
        elseBody := unset

        /**
         * Optional finally block.
         * @type {IR.Block}
         */
        finallyBody := unset
    }

    /**
     * A catch clause.
     * Maps from: catch_clause
     */
    class CatchClause extends IR.Node {
        /**
         * Error class names to catch.
         * @type {Array<String>}
         */
        errorTypes := TypedArray(String)

        /**
         * The variable the caught error is bound to.
         * @type {String}
         */
        varName := ""

        /**
         * @type {IR.Block}
         */
        body := unset
    }

    /**
     * A return statement.
     * Maps from: return_statement
     */
    class ReturnStmt extends IR.Node {
        /**
         * The return expression, or unset.
         * @type {IR.Node}
         */
        value := unset
    }

    /**
     * A break statement.
     * Maps from: break_statement
     */
    class BreakStmt extends IR.Node {
        /**
         * Optional target label.
         * @type {String}
         */
        label := ""
    }

    /**
     * A continue statement.
     * Maps from: continue_statement
     */
    class ContinueStmt extends IR.Node {
        /**
         * Optional target label.
         * @type {String}
         */
        label := ""
    }

    /**
     * A throw statement.
     * Maps from: throw_statement
     */
    class ThrowStmt extends IR.Node {
        /**
         * The expression being thrown.
         * @type {IR.Node}
         */
        value := unset
    }

    /**
     * A goto statement.
     * Maps from: goto_statement
     */
    class GotoStmt extends IR.Node {
        /**
         * Target label name.
         * @type {String}
         */
        label := ""
    }

    ; =========================================================================
    ; 1.5 AHK-Specific Nodes
    ; =========================================================================

    /**
     * A hotkey definition.
     * Maps from: hotkey
     *
     * Must be in the IR so tree-shaking can see what functions/methods
     * the hotkey body references.
     */
    class Hotkey extends IR.Node {
        /**
         * The key combination text, e.g. "^a", "#f"
         * @type {String}
         */
        trigger := ""

        /**
         * @type {String}
         */
        modifiers := ""

        /**
         * The hotkey's action body.
         * @type {IR.Block | IR.Node}
         */
        body := unset
    }

    /**
     * A hotstring definition.
     * Maps from: hotstring
     */
    class Hotstring extends IR.Node {
        /**
         * The trigger text.
         * @type {String}
         */
        trigger := ""

        /**
         * Hotstring options.
         * @type {String}
         */
        modifiers := ""

        /**
         * The replacement (text or function body).
         * @type {IR.Node}
         */
        replacement := unset
    }

    /**
     * A preprocessor/runtime directive that survived preprocessing.
     * Maps from: requires_directive, warn_directive, hotif_directive, etc.
     *
     * #HotIf bodies are entry points — any functions/methods referenced
     * in the condition expression must be kept alive by tree-shaking.
     */
    class Directive extends IR.Node {
        /**
         * Directive type: "requires", "warn", "hotif", etc.
         * @type {String}
         */
        kind := ""

        /**
         * The directive's argument text.
         * @type {String}
         */
        value := ""

        /**
         * For #HotIf — the condition expression (entry point for tree-shaking).
         * @type {IR.Node}
         */
        expression := unset
    }

    /**
     * A label declaration (labelname:).
     * Maps from: label
     *
     * Kept in IR because labels can be targets of Goto and of
     * SetTimer/Hotkey command string references.
     */
    class Label extends IR.Node {
        /**
         * The label name.
         * @type {String}
         */
        name := ""

        /**
         * How many Goto/SetTimer/Hotkey references.
         * @type {Integer}
         */
        refCount := 0
    }
}
