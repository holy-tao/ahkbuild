/************************************************************************
 * @description Builds the IR from a tree-sitter AST.
 * @author
 * @date 2026/02/27
 * @version 0.1.0
 ***********************************************************************/

#Requires AutoHotkey v2.0

#Include <log4ahk\Log>
#Include <tree-sitter\TSNode>
#Include <tree-sitter\TSTreeCursor>

#Include ir.ahk
#Include scope.ahk

/**
 * Builds an IR tree from a tree-sitter AST in two phases:
 *
 *   Phase 1 (Construction): Walk the tree-sitter AST, create IR nodes,
 *           build scope chains, register symbols in the symbol table.
 *
 *   Phase 2 (Resolution): Walk the IR tree, resolve all IR.Identifier nodes
 *           through scope chains, record references in the symbol table.
 */
class IRBuilder {

    /**
     * The IR program node being built.
     * @type {IR.Program}
     */
    program := unset

    /**
     * The global scope.
     * @type {IRScope}
     */
    globalScope := unset

    /**
     * The program-wide symbol table.
     * @type {IRSymbolTable}
     */
    symbolTable := unset

    /**
     * The source buffer (kept for emission).
     * @type {Buffer}
     */
    sourceBuffer := unset

    /**
     * Build an IR tree from a parsed tree-sitter AST.
     *
     * @param {TSTree} tree the parsed tree-sitter tree
     * @param {Buffer} sourceBuffer the original source buffer
     * @returns {IR.Program} the completed IR program
     */
    Build(tree, sourceBuffer) {
        this.sourceBuffer := sourceBuffer

        ; Create the program root
        root := tree.Root
        this.program := IR.Program(unset, root)
        this.symbolTable := IRSymbolTable()
        this.globalScope := IRScope("global", this.program)
        this.program.scope := this.globalScope
        this.program.symbolTable := this.symbolTable
        this.program.sourceBuffer := sourceBuffer
        this.program.tree := tree

        ; Phase 1: Walk tree-sitter AST and build IR nodes
        Log.Debug("IR Phase 1: Building IR nodes from AST")
        this._BuildTopLevel(root)

        ; Phase 2: Resolve all identifier references
        Log.Debug("IR Phase 2: Resolving references")
        this._ResolveReferences(this.program)

        Log.Info(Format("IR built: {1} top-level nodes, {2} symbols",
            this.program.body.Length, this.symbolTable._symbols.Count))

        return this.program
    }

;@region Construction

    /**
     * Walk the top-level children of the source_file node.
     * @param {TSNode} root the source_file TSNode
     */
    _BuildTopLevel(root) {
        i := 0
        while i < root.NamedChildCount {
            child := root.GetNamedChild(i)
            irNode := this._BuildNode(this.program, child, this.globalScope)
            this.program.body.Push(irNode)
            i++
        }
    }

    /**
     * Main dispatcher. Creates an IR node from a tree-sitter node based on its type.
     *
     * @param {IR.Node} parent the parent IR node
     * @param {TSNode} tsNode the tree-sitter node
     * @param {IRScope} scope the current scope
     * @returns {IR.Node} the created IR node
     */
    _BuildNode(parent, tsNode, scope) {
        switch tsNode.Type {
            ; --- Declarations ---
            case "function_declaration":
                return this._BuildFunction(parent, tsNode, scope)
            case "method_declaration":
                return this._BuildFunction(parent, tsNode, scope, true)
            case "class_declaration":
                return this._BuildClass(parent, tsNode, scope)
            case "property_declaration":
                return this._BuildPropertyOrField(parent, tsNode, scope)
            case "variable_declaration":
                return this._BuildVarDecl(parent, tsNode, scope)

            ; --- Expressions: binary ---
            case "additive_operation",
                 "multiplicative_operation",
                 "exponent_operation",
                 "relational_operation",
                 "equality_operation",
                 "inequality_operation",
                 "logical_and_operation",
                 "logical_or_operation",
                 "bitwise_and_operation",
                 "bitwise_or_operation",
                 "bitwise_xor_operation",
                 "bitshift_operation",
                 "explicit_concat_operation",
                 "implicit_concat_operation",
                 "or_maybe_operation":
                return this._BuildBinaryExpr(parent, tsNode, scope)
            case "assignment_operation":
                return this._BuildAssignment(parent, tsNode, scope)

            ; --- Expressions: unary ---
            case "prefix_operation":
                return this._BuildUnaryExpr(parent, tsNode, scope, true)
            case "postfix_operation":
                return this._BuildUnaryExpr(parent, tsNode, scope, false)
            case "verbal_not_operation":
                return this._BuildUnaryExpr(parent, tsNode, scope, true)

            ; --- Expressions: other ---
            case "ternary_expression":
                return this._BuildTernaryExpr(parent, tsNode, scope)
            case "function_call":
                return this._BuildCallExpr(parent, tsNode, scope)
            case "call_statement":
                return this._BuildCallExpr(parent, tsNode, scope, true)
            case "member_access":
                return this._BuildMemberAccess(parent, tsNode, scope)
            case "index_access":
                return this._BuildIndexAccess(parent, tsNode, scope)
            case "identifier":
                return this._BuildIdentifier(parent, tsNode)
            case "integer_literal":
                return this._BuildLiteral(parent, tsNode, "integer")
            case "float_literal":
                return this._BuildLiteral(parent, tsNode, "float")
            case "hex_literal":
                return this._BuildLiteral(parent, tsNode, "integer")
            case "string_literal":
                return this._BuildLiteral(parent, tsNode, "string")
            case "boolean_literal":
                return this._BuildLiteral(parent, tsNode, "boolean")
            case "array_literal":
                return this._BuildArrayLiteral(parent, tsNode, scope)
            case "object_literal":
                return this._BuildObjectLiteral(parent, tsNode, scope)
            case "dereference_operation":
                return this._BuildDerefExpr(parent, tsNode, scope)
            case "varref_operation":
                return this._BuildVarRefExpr(parent, tsNode, scope)
            case "fat_arrow_function":
                return this._BuildFatArrow(parent, tsNode, scope)
            case "expression_sequence":
                return this._BuildExpressionSequence(parent, tsNode, scope)

            ; --- Control flow ---
            case "if_statement":
                return this._BuildIf(parent, tsNode, scope)
            case "while_statement":
                return this._BuildWhile(parent, tsNode, scope)
            case "for_statement":
                return this._BuildFor(parent, tsNode, scope)
            case "loop_statement":
                return this._BuildLoop(parent, tsNode, scope)
            case "switch_statement":
                return this._BuildSwitch(parent, tsNode, scope)
            case "try_statement":
                return this._BuildTry(parent, tsNode, scope)
            case "return_statement":
                return this._BuildReturn(parent, tsNode, scope)
            case "break_statement":
                return this._BuildBreak(parent, tsNode)
            case "continue_statement":
                return this._BuildContinue(parent, tsNode)
            case "throw_statement":
                return this._BuildThrow(parent, tsNode, scope)
            case "goto_statement":
                return this._BuildGoto(parent, tsNode)
            case "label":
                return this._BuildLabel(parent, tsNode, scope)

            ; --- Blocks ---
            case "block":
                return this._BuildBlock(parent, tsNode, scope)

            ; --- AHK-specific ---
            case "hotkey":
                return this._BuildHotkey(parent, tsNode, scope)
            case "hotstring":
                return this._BuildHotstring(parent, tsNode, scope)
            case "hotif_directive":
                return this._BuildHotIfDirective(parent, tsNode, scope)
            case "requires_directive",
                 "warn_directive",
                 "single_instance_directive",
                 "no_tray_icon_directive",
                 "dll_load_directive",
                 "error_stdout_directive",
                 "input_level_directive",
                 "use_hook_directive",
                 "max_threads_directive",
                 "max_threads_per_hotkey_directive",
                 "max_threads_buffer_directive",
                 "clipboard_timeout_directive",
                 "hotif_timeout_directive",
                 "hotstring_directive":
                return this._BuildDirective(parent, tsNode)

            ; --- Fallback ---
            default:
                return this._BuildOpaque(parent, tsNode)
        }
    }

    ; -----------------------------------------------------------------
    ; Declarations
    ; -----------------------------------------------------------------

    /**
     * Build an IR.Function from a function_declaration or method_declaration.
     *
     * @param {IR.Node} parent
     * @param {TSNode} tsNode
     * @param {IRScope} scope
     * @param {Boolean} isMethod
     * @returns {IR.Function}
     */
    _BuildFunction(parent, tsNode, scope, isMethod := false) {
        fn := IR.Function(parent, tsNode)

        ; Name
        nameNode := tsNode.GetChildByFieldName("name")
        fn.name := nameNode.IsNull ? "" : nameNode.Text

        fn.isMethod := isMethod

        ; Check for "static" scope qualifier (first child)
        firstChild := tsNode.GetChild(0)
        if !firstChild.IsNull && firstChild.Type == "scope_identifier"
            fn.scope := firstChild.Text

        ; Determine ownerClass from parent chain
        if isMethod && parent is IR.ClassDecl
            fn.ownerClass := parent.fullyQualifiedName

        ; Create function scope
        fn.localScope := IRScope("function", fn, scope)

        ; Build parameters
        headNode := tsNode.GetChildByFieldName("head")
        if !headNode.IsNull
            this._BuildParams(fn, headNode, fn.localScope)

        ; Build body
        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull {
            ; function_body can be a block or => expression
            ; Check children for the actual content
            if bodyNode.Type == "function_body" {
                ; Look for block or fat arrow expression
                j := 0
                while j < bodyNode.NamedChildCount {
                    bodyChild := bodyNode.GetNamedChild(j)
                    if bodyChild.Type == "block" {
                        fn.body := this._BuildBlock(fn, bodyChild, fn.localScope)
                        break
                    } else {
                        ; Arrow body — the expression
                        fn.isArrow := true
                        fn.body := this._BuildNode(fn, bodyChild, fn.localScope)
                        break
                    }
                    j++
                }
            } else {
                fn.body := this._BuildNode(fn, bodyNode, fn.localScope)
            }
        }

        ; Check for bare `global` switching assume-mode
        this._CheckBareGlobal(fn)

        ; Register symbol
        fqn := fn.ownerClass != "" ? fn.ownerClass "." fn.name : fn.name
        if fn.name != "" {
            sym := IRSymbol(fn.name, "function")
            sym.node := fn
            sym.scope := scope
            scope.Define(fn.name, sym)
            this.symbolTable.Register(fqn, sym)
        }

        parent.children.Push(fn)
        return fn
    }

    /**
     * Build parameter nodes from a function_head or param_sequence.
     *
     * @param {IR.Function} fn the function being built
     * @param {TSNode} headNode the function_head node
     * @param {IRScope} fnScope the function's scope
     */
    _BuildParams(fn, headNode, fnScope) {
        ; Walk through all named children looking for param-related nodes
        ; The head may be function_head containing param_sequence, or param_sequence directly
        paramContainer := headNode
        if headNode.Type == "function_head" {
            ; Look for param_sequence inside
            i := 0
            while i < headNode.NamedChildCount {
                child := headNode.GetNamedChild(i)
                if child.Type == "param_sequence" {
                    paramContainer := child
                    break
                }
                i++
            }
        }

        if paramContainer.Type != "param_sequence"
            return

        i := 0
        while i < paramContainer.NamedChildCount {
            child := paramContainer.GetNamedChild(i)
            param := this._BuildParam(fn, child, fnScope)
            if param {
                fn.params.Push(param)
                fn.children.Push(param)

                ; Check if variadic
                if param.isVariadic
                    fn.isVariadic := true

                ; Register param in function scope
                sym := IRSymbol(param.name, "param")
                sym.node := param
                sym.scope := fnScope
                fnScope.Define(param.name, sym)
            }
            i++
        }
    }

    /**
     * Build a single IR.Param node.
     *
     * @param {IR.Node} parent
     * @param {TSNode} tsNode
     * @param {IRScope} scope
     * @returns {IR.Param | 0} the param node, or 0 if not a recognized param type
     */
    _BuildParam(parent, tsNode, scope) {
        param := IR.Param(parent, tsNode)

        switch tsNode.Type {
            case "identifier":
                param.name := tsNode.Text
            case "byref_param":
                param.isByRef := true
                ; The identifier is a named child
                nameNode := this._FindNamedChild(tsNode, "identifier")
                param.name := nameNode ? nameNode.Text : tsNode.Text
            case "default_param":
                ; Has an identifier and a default value
                nameNode := this._FindNamedChild(tsNode, "identifier")
                param.name := nameNode ? nameNode.Text : ""
                ; Default value is the expression after :=
                ; Walk named children to find the expression
                j := 0
                while j < tsNode.NamedChildCount {
                    child := tsNode.GetNamedChild(j)
                    if child.Type != "identifier" {
                        param.default := this._BuildNode(param, child, scope)
                        break
                    }
                    j++
                }
            case "variadic_param":
                param.isVariadic := true
                nameNode := this._FindNamedChild(tsNode, "identifier")
                param.name := nameNode ? nameNode.Text : tsNode.Text
            case "optional_param":
                param.isOptional := true
                nameNode := this._FindNamedChild(tsNode, "identifier")
                param.name := nameNode ? nameNode.Text : tsNode.Text
            default:
                return 0
        }

        return param
    }

    /**
     * Check if a function body starts with a bare `global` statement,
     * which switches the function to assume-global mode.
     *
     * @param {IR.Function} fn
     */
    _CheckBareGlobal(fn) {
        if !fn.HasOwnProp("body")
            return
        if !(fn.body is IR.Block)
            return
        if fn.body.body.Length == 0
            return

        first := fn.body.body[1]
        if first is IR.VarDecl && first.declScope == "global" && first.name == ""
            fn.localScope.assumeMode := "global"
    }

    /**
     * Build an IR.ClassDecl from a class_declaration.
     */
    _BuildClass(parent, tsNode, scope) {
        cls := IR.ClassDecl(parent, tsNode)

        ; Name
        nameNode := tsNode.GetChildByFieldName("name")
        cls.name := nameNode.IsNull ? "" : nameNode.Text

        ; Superclass
        superNode := tsNode.GetChildByFieldName("superclass")
        if !superNode.IsNull
            cls.superclass := superNode.Text

        ; Fully qualified name
        if parent is IR.ClassDecl
            cls.fullyQualifiedName := parent.fullyQualifiedName "." cls.name
        else
            cls.fullyQualifiedName := cls.name

        ; Create class scope
        cls.classScope := IRScope("class", cls, scope)

        ; Walk class_body children
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "class_body" {
                this._BuildClassBody(cls, child, cls.classScope)
                break
            }
            i++
        }

        ; Register symbol
        if cls.name != "" {
            sym := IRSymbol(cls.name, "class")
            sym.node := cls
            sym.scope := scope
            scope.Define(cls.name, sym)
            this.symbolTable.Register(cls.fullyQualifiedName, sym)
        }

        parent.children.Push(cls)
        return cls
    }

    /**
     * Walk the children of a class_body node and populate the class.
     */
    _BuildClassBody(cls, classBodyNode, classScope) {
        i := 0
        while i < classBodyNode.NamedChildCount {
            child := classBodyNode.GetNamedChild(i)
            switch child.Type {
                case "method_declaration":
                    method := this._BuildFunction(cls, child, classScope, true)
                    cls.methods.Push(method)
                case "property_declaration":
                    node := this._BuildPropertyOrField(cls, child, classScope)
                    if node is IR.Property
                        cls.properties.Push(node)
                    else if node is IR.Field {
                        if node.scope == "static"
                            cls.staticFields.Push(node)
                        else
                            cls.instanceFields.Push(node)
                    }
                case "class_declaration":
                    nested := this._BuildClass(cls, child, classScope)
                    cls.nestedClasses.Push(nested)
                default:
                    ; Other things in class body — opaque
                    this._BuildOpaque(cls, child)
            }
            i++
        }
    }

    /**
     * Build an IR.Property or IR.Field from a property_declaration.
     *
     * Decides based on structure:
     *   - If it has a getter/setter block or => arrow → IR.Property
     *   - If it's just `name := value` → IR.Field
     */
    _BuildPropertyOrField(parent, tsNode, scope) {
        ; Determine property name and scope qualifier
        propName := ""
        propScope := ""
        hasArrow := false
        hasBlock := false

        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            switch child.Type {
                case "scope_identifier":
                    propScope := child.Text
                case "identifier":
                    propName := child.Text
                case "property_declaration_block":
                    hasBlock := true
                case "getter", "setter":
                    hasBlock := true
            }
            i++
        }

        ; Check for shorthand arrow getter: look for => in anonymous children
        j := 0
        while j < tsNode.ChildCount {
            child := tsNode.GetChild(j)
            if !child.IsNamed && child.Text == "=>" {
                hasArrow := true
                break
            }
            j++
        }

        ; If it has a getter/setter block or arrow → Property
        if hasArrow || hasBlock
            return this._BuildProperty(parent, tsNode, scope, propName, propScope, hasArrow)

        ; Otherwise → Field
        return this._BuildField(parent, tsNode, scope, propName, propScope)
    }

    /**
     * Build an IR.Property node.
     */
    _BuildProperty(parent, tsNode, scope, propName, propScope, isShorthandArrow) {
        prop := IR.Property(parent, tsNode)
        prop.name := propName
        prop.scope := propScope

        if isShorthandArrow {
            ; Shorthand Prop => expr — the expression is among the named children
            prop.isGetterOnly := true
            prop.isArrowGetter := true
            ; Find the expression (last named child that isn't scope_identifier or identifier)
            i := 0
            while i < tsNode.NamedChildCount {
                child := tsNode.GetNamedChild(i)
                if child.Type != "scope_identifier" && child.Type != "identifier" {
                    ; Build a synthetic function for the getter
                    getter := IR.Function(prop, tsNode)
                    getter.name := propName
                    getter.isArrow := true
                    getter.isMethod := true
                    getter.body := this._BuildNode(getter, child, scope)
                    getter.localScope := IRScope("function", getter, scope)
                    prop.getter := getter
                    prop.children.Push(getter)
                    break
                }
                i++
            }
        } else {
            ; Has a property_declaration_block with getter/setter
            i := 0
            while i < tsNode.NamedChildCount {
                child := tsNode.GetNamedChild(i)
                if child.Type == "property_declaration_block" {
                    this._BuildGetterSetter(prop, child, scope)
                    break
                }
                ; Also handle getter/setter as direct children
                if child.Type == "getter"
                    this._BuildGetterNode(prop, child, scope)
                else if child.Type == "setter"
                    this._BuildSetterNode(prop, child, scope)
                i++
            }
        }

        ; Register as property symbol
        fqn := ""
        if parent is IR.ClassDecl
            fqn := parent.fullyQualifiedName "." propName
        else
            fqn := propName
        if propName != "" {
            sym := IRSymbol(propName, "property")
            sym.node := prop
            sym.scope := scope
            this.symbolTable.Register(fqn, sym)
        }

        parent.children.Push(prop)
        return prop
    }

    /**
     * Build getter and setter from a property_declaration_block.
     */
    _BuildGetterSetter(prop, blockNode, scope) {
        i := 0
        while i < blockNode.NamedChildCount {
            child := blockNode.GetNamedChild(i)
            if child.Type == "getter"
                this._BuildGetterNode(prop, child, scope)
            else if child.Type == "setter"
                this._BuildSetterNode(prop, child, scope)
            i++
        }
    }

    /**
     * Build the getter as an IR.Function and attach it to a property.
     */
    _BuildGetterNode(prop, getterNode, scope) {
        getter := IR.Function(prop, getterNode)
        getter.name := prop.name
        getter.isMethod := true
        getter.localScope := IRScope("function", getter, scope)

        ; getter contains function_body
        i := 0
        while i < getterNode.NamedChildCount {
            child := getterNode.GetNamedChild(i)
            if child.Type == "function_body" {
                this._BuildFunctionBody(getter, child, getter.localScope)
                break
            }
            i++
        }

        prop.getter := getter
        prop.isArrowGetter := getter.isArrow
        prop.children.Push(getter)
    }

    /**
     * Build the setter as an IR.Function and attach it to a property.
     */
    _BuildSetterNode(prop, setterNode, scope) {
        setter := IR.Function(prop, setterNode)
        setter.name := prop.name
        setter.isMethod := true
        setter.localScope := IRScope("function", setter, scope)

        ; setter has an implicit `value` parameter
        ; Build function_body
        i := 0
        while i < setterNode.NamedChildCount {
            child := setterNode.GetNamedChild(i)
            if child.Type == "function_body" {
                this._BuildFunctionBody(setter, child, setter.localScope)
                break
            }
            i++
        }

        prop.setter := setter
        prop.isArrowSetter := setter.isArrow
        prop.children.Push(setter)
    }

    /**
     * Build a function body (shared by getter/setter/function).
     * Handles both block and arrow forms.
     */
    _BuildFunctionBody(fn, bodyNode, fnScope) {
        j := 0
        while j < bodyNode.NamedChildCount {
            bodyChild := bodyNode.GetNamedChild(j)
            if bodyChild.Type == "block" {
                fn.body := this._BuildBlock(fn, bodyChild, fnScope)
            } else {
                fn.isArrow := true
                fn.body := this._BuildNode(fn, bodyChild, fnScope)
            }
            j++
        }
    }

    /**
     * Build an IR.Field (simple property := value).
     */
    _BuildField(parent, tsNode, scope, propName, propScope) {
        field := IR.Field(parent, tsNode)
        field.name := propName
        field.scope := propScope

        ; Find the initializer expression (after :=)
        ; Walk named children, skip scope_identifier and identifier
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type != "scope_identifier" && child.Type != "identifier" {
                field.initializer := this._BuildNode(field, child, scope)
                break
            }
            i++
        }

        parent.children.Push(field)
        return field
    }

    /**
     * Build an IR.VarDecl from a variable_declaration.
     */
    _BuildVarDecl(parent, tsNode, scope) {
        decl := IR.VarDecl(parent, tsNode)

        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            switch child.Type {
                case "scope_identifier":
                    decl.declScope := child.Text
                case "identifier":
                    decl.name := child.Text
            }
            i++
        }

        ; Register the declaration in the scope
        if decl.name != ""
            scope.Declare(decl.name, decl.declScope)
        ; A bare `global` (no name) is handled by _CheckBareGlobal

        parent.children.Push(decl)
        return decl
    }

    ; -----------------------------------------------------------------
    ; Expressions
    ; -----------------------------------------------------------------

    /**
     * Build an IR.BinaryExpr from a binary operation node.
     * All binary ops have `left` and `right` fields, and most have `operator`.
     */
    _BuildBinaryExpr(parent, tsNode, scope) {
        expr := IR.BinaryExpr(parent, tsNode)

        leftNode := tsNode.GetChildByFieldName("left")
        rightNode := tsNode.GetChildByFieldName("right")
        opNode := tsNode.GetChildByFieldName("operator")

        if !leftNode.IsNull
            expr.left := this._BuildNode(expr, leftNode, scope)
        if !rightNode.IsNull
            expr.right := this._BuildNode(expr, rightNode, scope)

        ; Get operator text
        if !opNode.IsNull {
            expr.operator := opNode.Text
        } else {
            ; Some ops (explicit_concat, type_check) may not have an operator field
            ; Use a sensible default based on node type
            switch tsNode.Type {
                case "explicit_concat_operation": expr.operator := "."
                case "implicit_concat_operation": expr.operator := " "
                default: expr.operator := ""
            }
        }

        parent.children.Push(expr)
        return expr
    }

    /**
     * Build an IR.BinaryExpr from an assignment_operation.
     * Assignment has `left` and `right` fields but no `operator` field.
     * The operator is an anonymous child between left and right.
     */
    _BuildAssignment(parent, tsNode, scope) {
        expr := IR.BinaryExpr(parent, tsNode)

        leftNode := tsNode.GetChildByFieldName("left")
        rightNode := tsNode.GetChildByFieldName("right")

        if !leftNode.IsNull
            expr.left := this._BuildNode(expr, leftNode, scope)
        if !rightNode.IsNull
            expr.right := this._BuildNode(expr, rightNode, scope)

        ; Find the operator: anonymous child between left and right
        i := 0
        while i < tsNode.ChildCount {
            child := tsNode.GetChild(i)
            if !child.IsNamed
                && !leftNode.IsNull && child.StartByte > leftNode.EndByte
                && !rightNode.IsNull && child.EndByte <= rightNode.StartByte {
                text := child.Text
                if text != "" {
                    expr.operator := text
                    break
                }
            }
            i++
        }

        ; Fallback
        if expr.operator == ""
            expr.operator := ":="

        parent.children.Push(expr)
        return expr
    }

    /**
     * Build an IR.UnaryExpr.
     */
    _BuildUnaryExpr(parent, tsNode, scope, isPrefix) {
        expr := IR.UnaryExpr(parent, tsNode)
        expr.isPrefix := isPrefix

        opNode := tsNode.GetChildByFieldName("operator")
        operandNode := tsNode.GetChildByFieldName("operand")

        if !opNode.IsNull
            expr.operator := opNode.Text
        if !operandNode.IsNull
            expr.operand := this._BuildNode(expr, operandNode, scope)

        parent.children.Push(expr)
        return expr
    }

    /**
     * Build an IR.TernaryExpr.
     * Tree-sitter ternary_expression has: condition (first named child),
     * true_branch field, false_branch field.
     */
    _BuildTernaryExpr(parent, tsNode, scope) {
        expr := IR.TernaryExpr(parent, tsNode)

        trueBranch := tsNode.GetChildByFieldName("true_branch")
        falseBranch := tsNode.GetChildByFieldName("false_branch")

        ; Condition is the first named child (before the ?)
        if tsNode.NamedChildCount > 0 {
            firstChild := tsNode.GetNamedChild(0)
            expr.condition := this._BuildNode(expr, firstChild, scope)
        }

        if !trueBranch.IsNull
            expr.trueBranch := this._BuildNode(expr, trueBranch, scope)
        if !falseBranch.IsNull
            expr.falseBranch := this._BuildNode(expr, falseBranch, scope)

        parent.children.Push(expr)
        return expr
    }

    /**
     * Build an IR.CallExpr from function_call or call_statement.
     */
    _BuildCallExpr(parent, tsNode, scope, isCommandStyle := false) {
        call := IR.CallExpr(parent, tsNode)
        call.isCommandStyle := isCommandStyle

        ; Callee
        calleeNode := tsNode.GetChildByFieldName("function")
        if !calleeNode.IsNull {
            call.callee := this._BuildNode(call, calleeNode, scope)
            ; Check if dynamic
            if call.callee is IR.DerefExpr
                call.isDynamic := true
        }

        ; Arguments — find arg_sequence among children
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "arg_sequence" {
                this._BuildArgs(call, child, scope)
                break
            }
            i++
        }

        parent.children.Push(call)
        return call
    }

    /**
     * Build argument expressions from an arg_sequence node.
     */
    _BuildArgs(call, argSeqNode, scope) {
        i := 0
        while i < argSeqNode.NamedChildCount {
            child := argSeqNode.GetNamedChild(i)
            if child.Type == "empty_arg" {
                ; Empty argument placeholder — build as opaque
                call.args.Push(this._BuildOpaque(call, child))
            } else {
                arg := this._BuildNode(call, child, scope)
                call.args.Push(arg)
            }
            i++
        }
    }

    /**
     * Build an IR.MemberAccess.
     */
    _BuildMemberAccess(parent, tsNode, scope) {
        ma := IR.MemberAccess(parent, tsNode)

        objNode := tsNode.GetChildByFieldName("object")
        memberNode := tsNode.GetChildByFieldName("member")

        if !objNode.IsNull
            ma.object := this._BuildNode(ma, objNode, scope)
        if !memberNode.IsNull
            ma.member := memberNode.Text

        parent.children.Push(ma)
        return ma
    }

    /**
     * Build an IR.IndexAccess.
     */
    _BuildIndexAccess(parent, tsNode, scope) {
        ia := IR.IndexAccess(parent, tsNode)

        objNode := tsNode.GetChildByFieldName("object")
        if !objNode.IsNull
            ia.object := this._BuildNode(ia, objNode, scope)

        ; Arguments in arg_sequence
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "arg_sequence" {
                j := 0
                while j < child.NamedChildCount {
                    argChild := child.GetNamedChild(j)
                    arg := this._BuildNode(ia, argChild, scope)
                    ia.args.Push(arg)
                    j++
                }
                break
            }
            i++
        }

        parent.children.Push(ia)
        return ia
    }

    /**
     * Build an IR.Identifier.
     */
    _BuildIdentifier(parent, tsNode) {
        id := IR.Identifier(parent, tsNode)
        id.name := tsNode.Text
        parent.children.Push(id)
        return id
    }

    /**
     * Build an IR.Literal.
     */
    _BuildLiteral(parent, tsNode, literalType) {
        lit := IR.Literal(parent, tsNode)
        lit.literalType := literalType
        lit.raw := tsNode.Text

        switch literalType {
            case "integer":
                ; Handle hex (0x...) and decimal
                raw := tsNode.Text
                if SubStr(raw, 1, 2) == "0x" || SubStr(raw, 1, 2) == "0X"
                    lit.value := Integer(raw)
                else
                    lit.value := Integer(raw)
            case "float":
                lit.value := Float(tsNode.Text)
            case "string":
                ; Store with quotes stripped
                raw := tsNode.Text
                lit.value := SubStr(raw, 2, StrLen(raw) - 2)
            case "boolean":
                lit.value := StrLower(tsNode.Text) == "true" ? true : false
        }

        parent.children.Push(lit)
        return lit
    }

    /**
     * Build an IR.ArrayLiteral.
     */
    _BuildArrayLiteral(parent, tsNode, scope) {
        arr := IR.ArrayLiteral(parent, tsNode)

        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            elem := this._BuildNode(arr, child, scope)
            arr.elements.Push(elem)
            i++
        }

        parent.children.Push(arr)
        return arr
    }

    /**
     * Build an IR.ObjectLiteral.
     */
    _BuildObjectLiteral(parent, tsNode, scope) {
        obj := IR.ObjectLiteral(parent, tsNode)

        ; Walk children looking for object_literal_member nodes
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "object_literal_member" {
                keyNode := "", valNode := ""
                j := 0
                while j < child.NamedChildCount {
                    grandchild := child.GetNamedChild(j)
                    if j == 0
                        keyNode := this._BuildNode(obj, grandchild, scope)
                    else
                        valNode := this._BuildNode(obj, grandchild, scope)
                    j++
                }
                if keyNode != ""
                    obj.pairs.Push({key: keyNode, value: valNode})
            }
            ; Handle object_literal_member_sequence
            else if child.Type == "object_literal_member_sequence" {
                k := 0
                while k < child.NamedChildCount {
                    member := child.GetNamedChild(k)
                    if member.Type == "object_literal_member" {
                        keyNode := "", valNode := ""
                        j := 0
                        while j < member.NamedChildCount {
                            grandchild := member.GetNamedChild(j)
                            if j == 0
                                keyNode := this._BuildNode(obj, grandchild, scope)
                            else
                                valNode := this._BuildNode(obj, grandchild, scope)
                            j++
                        }
                        if keyNode != ""
                            obj.pairs.Push({key: keyNode, value: valNode})
                    }
                    k++
                }
            }
            i++
        }

        parent.children.Push(obj)
        return obj
    }

    /**
     * Build an IR.DerefExpr (%expr%).
     */
    _BuildDerefExpr(parent, tsNode, scope) {
        deref := IR.DerefExpr(parent, tsNode)

        ; The inner expression is the named child
        if tsNode.NamedChildCount > 0 {
            child := tsNode.GetNamedChild(0)
            deref.inner := this._BuildNode(deref, child, scope)
        }

        parent.children.Push(deref)
        return deref
    }

    /**
     * Build an IR.VarRefExpr (&var).
     */
    _BuildVarRefExpr(parent, tsNode, scope) {
        vr := IR.VarRefExpr(parent, tsNode)

        if tsNode.NamedChildCount > 0 {
            child := tsNode.GetNamedChild(0)
            vr.operand := this._BuildNode(vr, child, scope)
        }

        parent.children.Push(vr)
        return vr
    }

    /**
     * Build an IR.FatArrow (anonymous arrow function as expression).
     */
    _BuildFatArrow(parent, tsNode, scope) {
        arrow := IR.FatArrow(parent, tsNode)
        arrow.localScope := IRScope("arrow", arrow, scope)

        ; Parameters — look for param_sequence
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "param_sequence" {
                j := 0
                while j < child.NamedChildCount {
                    paramChild := child.GetNamedChild(j)
                    param := this._BuildParam(arrow, paramChild, arrow.localScope)
                    if param {
                        arrow.params.Push(param)
                        arrow.children.Push(param)

                        sym := IRSymbol(param.name, "param")
                        sym.node := param
                        sym.scope := arrow.localScope
                        arrow.localScope.Define(param.name, sym)
                    }
                    j++
                }
                break
            }
            i++
        }

        ; Body — the expression after =>
        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull
            arrow.body := this._BuildNode(arrow, bodyNode, arrow.localScope)

        parent.children.Push(arrow)
        return arrow
    }

    /**
     * Build from an expression_sequence (comma-separated expressions).
     * Returns the first expression if only one, otherwise wraps as opaque.
     */
    _BuildExpressionSequence(parent, tsNode, scope) {
        if tsNode.NamedChildCount == 1
            return this._BuildNode(parent, tsNode.GetNamedChild(0), scope)

        ; Multiple expressions — build each as a child of an opaque wrapper
        ; (expression sequences are rarely interesting for transforms individually)
        opaque := IR.Opaque(parent, tsNode)
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            this._BuildNode(opaque, child, scope)
            i++
        }
        parent.children.Push(opaque)
        return opaque
    }

    ; -----------------------------------------------------------------
    ; Control Flow
    ; -----------------------------------------------------------------

    /**
     * Build an IR.IfStmt from an if_statement.
     */
    _BuildIf(parent, tsNode, scope) {
        ifNode := IR.IfStmt(parent, tsNode)

        ; Walk children to find condition, then-block, and else
        ; Structure: if keyword, condition expr, block, optional else_statement
        foundCondition := false
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "block" {
                ifNode.thenBody := this._BuildBlock(ifNode, child, scope)
            } else if child.Type == "else_statement" {
                ifNode.elseBody := this._BuildElse(ifNode, child, scope)
            } else if !foundCondition {
                ; First non-block, non-else named child is the condition
                ifNode.condition := this._BuildNode(ifNode, child, scope)
                foundCondition := true
            }
            i++
        }

        parent.children.Push(ifNode)
        return ifNode
    }

    /**
     * Build the else branch of an if statement.
     * Can be: else { block } or else if (condition) { ... }
     */
    _BuildElse(parent, tsNode, scope) {
        ; Look inside the else_statement for if_statement or block
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "if_statement"
                return this._BuildIf(parent, child, scope)
            else if child.Type == "block"
                return this._BuildBlock(parent, child, scope)
            i++
        }
        ; Fallback: single statement else
        if tsNode.NamedChildCount > 0
            return this._BuildNode(parent, tsNode.GetNamedChild(0), scope)
        return this._BuildOpaque(parent, tsNode)
    }

    /**
     * Build an IR.WhileStmt.
     */
    _BuildWhile(parent, tsNode, scope) {
        w := IR.WhileStmt(parent, tsNode)

        ; Condition is first named expression child, body is via field
        foundCondition := false
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "block" {
                w.body := this._BuildBlock(w, child, scope)
            } else if !foundCondition {
                w.condition := this._BuildNode(w, child, scope)
                foundCondition := true
            }
            i++
        }

        parent.children.Push(w)
        return w
    }

    /**
     * Build an IR.ForStmt.
     */
    _BuildFor(parent, tsNode, scope) {
        f := IR.ForStmt(parent, tsNode)

        ; For loop: for iterator_vars in iterable { body }
        ; head field contains the iterators and iterable
        ; body field contains the loop body
        headNode := tsNode.GetChildByFieldName("head")
        bodyNode := tsNode.GetChildByFieldName("body")

        ; Parse head: identifiers are iterators, last expression is iterable
        if !headNode.IsNull {
            ; Walk head's named children
            identifiers := []
            lastExpr := unset
            i := 0
            while i < headNode.NamedChildCount {
                child := headNode.GetNamedChild(i)
                if child.Type == "identifier" {
                    id := this._BuildIdentifier(f, child)
                    identifiers.Push(id)
                } else {
                    lastExpr := this._BuildNode(f, child, scope)
                }
                i++
            }
            ; If head itself is the iterable container, the identifiers
            ; came first and the last expression is the iterable
            f.iterators := identifiers
            if IsSet(lastExpr)
                f.iterable := lastExpr
        }

        if !bodyNode.IsNull {
            if bodyNode.Type == "block"
                f.body := this._BuildBlock(f, bodyNode, scope)
            else
                f.body := this._BuildNode(f, bodyNode, scope)
        }

        ; Optional else
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "else_statement" {
                f.elseBody := this._BuildElse(f, child, scope)
                break
            }
            i++
        }

        parent.children.Push(f)
        return f
    }

    /**
     * Build an IR.LoopStmt.
     */
    _BuildLoop(parent, tsNode, scope) {
        l := IR.LoopStmt(parent, tsNode)

        headNode := tsNode.GetChildByFieldName("head")
        bodyNode := tsNode.GetChildByFieldName("body")

        ; Determine loop kind from head
        if headNode.IsNull {
            l.kind := "infinite"
        } else {
            headText := StrLower(headNode.Text)
            if InStr(headText, "parse")
                l.kind := "parse"
            else if InStr(headText, "read")
                l.kind := "read"
            else if InStr(headText, "reg")
                l.kind := "reg"
            else if InStr(headText, "files")
                l.kind := "files"
            else
                l.kind := "count"
            l.head := this._BuildNode(l, headNode, scope)
        }

        if !bodyNode.IsNull {
            if bodyNode.Type == "block"
                l.body := this._BuildBlock(l, bodyNode, scope)
            else
                l.body := this._BuildNode(l, bodyNode, scope)
        }

        ; Check for until clause
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "until_statement" {
                if child.NamedChildCount > 0
                    l.untilCondition := this._BuildNode(l, child.GetNamedChild(0), scope)
                break
            }
            i++
        }

        parent.children.Push(l)
        return l
    }

    /**
     * Build an IR.SwitchStmt.
     */
    _BuildSwitch(parent, tsNode, scope) {
        sw := IR.SwitchStmt(parent, tsNode)

        ; Discriminant from head field
        headNode := tsNode.GetChildByFieldName("head")
        if !headNode.IsNull
            sw.discriminant := this._BuildNode(sw, headNode, scope)

        ; Body contains case_clause and default_clause
        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull {
            i := 0
            while i < bodyNode.NamedChildCount {
                child := bodyNode.GetNamedChild(i)
                if child.Type == "case_clause"
                    sw.cases.Push(this._BuildCase(sw, child, scope))
                else if child.Type == "default_clause" {
                    dc := this._BuildCase(sw, child, scope, true)
                    sw.defaultCase := dc
                    sw.cases.Push(dc)
                }
                i++
            }
        }

        parent.children.Push(sw)
        return sw
    }

    /**
     * Build an IR.CaseClause from a case_clause or default_clause.
     */
    _BuildCase(parent, tsNode, scope, isDefault := false) {
        c := IR.CaseClause(parent, tsNode)
        c.isDefault := isDefault

        ; Walk named children
        ; For case_clause: first children are the value expressions, rest are statements
        ; For default_clause: all children are statements
        passedColon := false
        i := 0
        while i < tsNode.ChildCount {
            child := tsNode.GetChild(i)
            if !child.IsNamed {
                if child.Text == ":"
                    passedColon := true
            } else if passedColon {
                ; Statement after the colon
                c.body.Push(this._BuildNode(c, child, scope))
            } else if !isDefault {
                ; Value expression before the colon
                c.values.Push(this._BuildNode(c, child, scope))
            }
            i++
        }

        parent.children.Push(c)
        return c
    }

    /**
     * Build an IR.TryStmt.
     */
    _BuildTry(parent, tsNode, scope) {
        t := IR.TryStmt(parent, tsNode)

        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            switch child.Type {
                case "block":
                    ; First block is the try body
                    if !t.HasOwnProp("tryBody")
                        t.tryBody := this._BuildBlock(t, child, scope)
                case "catch_clause":
                    t.catchClauses.Push(this._BuildCatch(t, child, scope))
                case "else_statement":
                    if child.NamedChildCount > 0 {
                        elseChild := child.GetNamedChild(0)
                        if elseChild.Type == "block"
                            t.elseBody := this._BuildBlock(t, elseChild, scope)
                        else
                            t.elseBody := this._BuildNode(t, elseChild, scope)
                    }
                case "finally_clause":
                    j := 0
                    while j < child.NamedChildCount {
                        fc := child.GetNamedChild(j)
                        if fc.Type == "block" {
                            t.finallyBody := this._BuildBlock(t, fc, scope)
                            break
                        }
                        j++
                    }
            }
            i++
        }

        parent.children.Push(t)
        return t
    }

    /**
     * Build an IR.CatchClause from a catch_clause.
     */
    _BuildCatch(parent, tsNode, scope) {
        c := IR.CatchClause(parent, tsNode)

        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            switch child.Type {
                case "identifier":
                    ; Could be error type or variable name depending on position
                    ; If there's an "as" keyword before it, it's the variable name
                    c.errorTypes.Push(child.Text)
                case "block":
                    c.body := this._BuildBlock(c, child, scope)
            }
            i++
        }

        ; The last identifier before the block might be the variable name
        ; AHK catch syntax: catch ErrorType as varName { }
        ; Check for "as" in anonymous children
        j := 0
        while j < tsNode.ChildCount {
            child := tsNode.GetChild(j)
            if !child.IsNamed && StrLower(child.Text) == "as" {
                ; Next named sibling is the variable name
                if j + 1 < tsNode.ChildCount {
                    ; Find next named child
                    k := j + 1
                    while k < tsNode.ChildCount {
                        next := tsNode.GetChild(k)
                        if next.IsNamed && next.Type == "identifier" {
                            c.varName := next.Text
                            ; Remove it from errorTypes if it was added
                            if c.errorTypes.Length > 0 && c.errorTypes[c.errorTypes.Length] == c.varName
                                c.errorTypes.Pop()
                            break
                        }
                        k++
                    }
                }
                break
            }
            j++
        }

        parent.children.Push(c)
        return c
    }

    /**
     * Build an IR.ReturnStmt.
     */
    _BuildReturn(parent, tsNode, scope) {
        ret := IR.ReturnStmt(parent, tsNode)

        if tsNode.NamedChildCount > 0
            ret.value := this._BuildNode(ret, tsNode.GetNamedChild(0), scope)

        parent.children.Push(ret)
        return ret
    }

    /**
     * Build an IR.BreakStmt.
     */
    _BuildBreak(parent, tsNode) {
        b := IR.BreakStmt(parent, tsNode)

        if tsNode.NamedChildCount > 0
            b.label := tsNode.GetNamedChild(0).Text

        parent.children.Push(b)
        return b
    }

    /**
     * Build an IR.ContinueStmt.
     */
    _BuildContinue(parent, tsNode) {
        c := IR.ContinueStmt(parent, tsNode)

        if tsNode.NamedChildCount > 0
            c.label := tsNode.GetNamedChild(0).Text

        parent.children.Push(c)
        return c
    }

    /**
     * Build an IR.ThrowStmt.
     */
    _BuildThrow(parent, tsNode, scope) {
        t := IR.ThrowStmt(parent, tsNode)

        if tsNode.NamedChildCount > 0
            t.value := this._BuildNode(t, tsNode.GetNamedChild(0), scope)

        parent.children.Push(t)
        return t
    }

    /**
     * Build an IR.GotoStmt.
     */
    _BuildGoto(parent, tsNode) {
        g := IR.GotoStmt(parent, tsNode)

        if tsNode.NamedChildCount > 0
            g.label := tsNode.GetNamedChild(0).Text

        parent.children.Push(g)
        return g
    }

    /**
     * Build an IR.Label.
     */
    _BuildLabel(parent, tsNode, scope) {
        l := IR.Label(parent, tsNode)

        if tsNode.NamedChildCount > 0
            l.name := tsNode.GetNamedChild(0).Text

        ; Register in symbol table
        if l.name != "" {
            sym := IRSymbol(l.name, "label")
            sym.node := l
            sym.scope := scope
            scope.Define(l.name, sym)
            this.symbolTable.Register(l.name, sym)
        }

        parent.children.Push(l)
        return l
    }

    ; -----------------------------------------------------------------
    ; Blocks and structural
    ; -----------------------------------------------------------------

    /**
     * Build an IR.Block from a block node.
     */
    _BuildBlock(parent, tsNode, scope) {
        blk := IR.Block(parent, tsNode)
        blk.scope := scope

        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            node := this._BuildNode(blk, child, scope)
            blk.body.Push(node)
            i++
        }

        parent.children.Push(blk)
        return blk
    }

    ; -----------------------------------------------------------------
    ; AHK-specific
    ; -----------------------------------------------------------------

    /**
     * Build an IR.Hotkey.
     */
    _BuildHotkey(parent, tsNode, scope) {
        hk := IR.Hotkey(parent, tsNode)

        triggerNode := tsNode.GetChildByFieldName("trigger")
        if !triggerNode.IsNull
            hk.trigger := triggerNode.Text

        ; Body: everything after the trigger that isn't part of the trigger
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type != "hotkey_trigger" {
                if child.Type == "block"
                    hk.body := this._BuildBlock(hk, child, scope)
                else
                    hk.body := this._BuildNode(hk, child, scope)
            }
            i++
        }

        parent.children.Push(hk)
        return hk
    }

    /**
     * Build an IR.Hotstring.
     */
    _BuildHotstring(parent, tsNode, scope) {
        hs := IR.Hotstring(parent, tsNode)

        triggerNode := tsNode.GetChildByFieldName("trigger")
        if !triggerNode.IsNull
            hs.trigger := triggerNode.Text

        modNode := tsNode.GetChildByFieldName("modifiers")
        if !modNode.IsNull
            hs.modifiers := modNode.Text

        ; Replacement: find replacement content
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "hotstring_replacement"
                || child.Type == "block" {
                hs.replacement := this._BuildNode(hs, child, scope)
                break
            }
            i++
        }

        parent.children.Push(hs)
        return hs
    }

    /**
     * Build an IR.Directive for #HotIf (which has an expression body
     * that is a tree-shaking entry point).
     */
    _BuildHotIfDirective(parent, tsNode, scope) {
        dir := IR.Directive(parent, tsNode)
        dir.kind := "hotif"

        ; The expression field
        exprNode := tsNode.GetChildByFieldName("expression")
        if !exprNode.IsNull
            dir.expression := this._BuildNode(dir, exprNode, scope)

        parent.children.Push(dir)
        return dir
    }

    /**
     * Build a generic IR.Directive.
     */
    _BuildDirective(parent, tsNode) {
        dir := IR.Directive(parent, tsNode)

        ; Extract kind from node type (strip _directive suffix)
        nodeType := tsNode.Type
        if SubStr(nodeType, -9) == "_directive"
            dir.kind := SubStr(nodeType, 1, StrLen(nodeType) - 10)
        else
            dir.kind := nodeType

        dir.value := tsNode.Text

        parent.children.Push(dir)
        return dir
    }

    /**
     * Build an IR.Opaque (fallback for unrecognized nodes).
     */
    _BuildOpaque(parent, tsNode) {
        opaque := IR.Opaque(parent, tsNode)
        parent.children.Push(opaque)
        return opaque
    }

;@endregion

;@region Reference Resolution

    /**
     * Walk the IR tree and resolve all IR.Identifier nodes through scope chains.
     * Records references in the symbol table.
     *
     * @param {IR.Node} node the node to walk
     */
    _ResolveReferences(node) {
        if node is IR.Identifier {
            this._ResolveIdentifier(node)
        }

        ; Recurse into all children
        for child in node.children
            this._ResolveReferences(child)
    }

    /**
     * Resolve a single identifier through scope chains.
     */
    _ResolveIdentifier(id) {
        ; Find the enclosing scope by walking parent chain
        scope := this._FindScope(id)
        if !scope
            return

        sym := scope.Resolve(id.name, this.globalScope)
        if sym {
            id.resolvedSymbol := sym
            id.resolvedScope := sym.scope
            this.symbolTable.AddReference(sym, id)
        }
    }

    /**
     * Find the nearest enclosing scope for an IR node by walking up the parent chain.
     *
     * @param {IR.Node} node
     * @returns {IRScope | 0}
     */
    _FindScope(node) {
        current := node
        while current.HasOwnProp("parent") {
            current := current.parent

            if current is IR.Program
                return current.scope
            if current is IR.Function && current.HasOwnProp("localScope")
                return current.localScope
            if current is IR.ClassDecl && current.HasOwnProp("classScope")
                return current.classScope
            if current is IR.FatArrow && current.HasOwnProp("localScope")
                return current.localScope
            if current is IR.Block && current.HasOwnProp("scope")
                return current.scope
        }

        ; Fallback to global
        return this.globalScope
    }

;@endregion

;@region Utilities

    /**
     * Find the first named child of a given type within a tree-sitter node.
     *
     * @param {TSNode} tsNode
     * @param {String} type the node type to look for
     * @returns {TSNode | 0} the found node, or 0
     */
    _FindNamedChild(tsNode, type) {
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == type
                return child
            i++
        }
        return 0
    }

;@endregion
}
