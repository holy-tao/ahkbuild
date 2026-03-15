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

    ; -----------------------------------------------------------------
    ; Directive comment helpers
    ; -----------------------------------------------------------------

    /**
     * Parses a directive_comment TSNode into an IR.DirectiveComment.
     * @param {TSNode} tsNode a directive_comment node
     * @returns {IR.DirectiveComment}
     */
    _ParseDirectiveComment(tsNode) {
        nameNode := tsNode.GetChildByFieldName("directive")
        argsNode := tsNode.GetChildByFieldName("arguments")

        name := nameNode.IsNull ? "" : nameNode.Text 
        arguments := argsNode.IsNull ? "" : Trim(argsNode.Text)
        return IR.DirectiveComment(name, arguments, tsNode)
    }

    /**
     * Attaches pending directives to an IR node and clears the pending list.
     * @param {IR.Node} irNode the node to attach directives to
     * @param {Array<IR.DirectiveComment>} pending accumulated directives
     */
    _AttachDirectives(irNode, pending) {
        for d in pending
            irNode.AddDirective(d)
        pending.Length := 0
    }

    /**
     * Warns about trailing directive comments that have no following statement.
     * @param {Array<IR.DirectiveComment>} pending leftover directives
     */
    _WarnTrailingDirectives(pending) {
        for d in pending
            Log.Warn("Directive ``;@" d.name "`` has no following statement")
    }

    ; -----------------------------------------------------------------
    ; Top-level and block building
    ; -----------------------------------------------------------------

    /**
     * Walk the top-level children of the source_file node.
     * @param {TSNode} root the source_file TSNode
     */
    _BuildTopLevel(root) {
        pendingDirectives := []
        i := 0
        while i < root.NamedChildCount {
            child := root.GetNamedChild(i)
            if child.Type == "directive_comment" {
                pendingDirectives.Push(this._ParseDirectiveComment(child))
                i++
                continue
            }
            irNode := this._BuildNode(this.program, child, this.globalScope)
            this._AttachDirectives(irNode, pendingDirectives)
            this.program.body.Push(irNode)
            i++
        }
        this._WarnTrailingDirectives(pendingDirectives)
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
            case "dynamic_identifier":
                return this._BuildDynamicIdentifier(parent, tsNode, scope)
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
                ; Grammar: "&" field("param", _param)
                param.isByRef := true
                innerNode := tsNode.GetChildByFieldName("param")
                if !innerNode.IsNull {
                    ; Inner param can be identifier, optional_param, or default_param
                    if innerNode.Type == "identifier"
                        param.name := innerNode.Text
                    else if innerNode.Type == "default_param" {
                        nameNode := innerNode.GetChildByFieldName("name")
                        param.name := nameNode.IsNull ? "" : nameNode.Text
                        valNode := innerNode.GetChildByFieldName("value")
                        if !valNode.IsNull
                            param.default := this._BuildNode(param, valNode, scope)
                    } else {
                        nameNode := this._FindNamedChild(innerNode, "identifier")
                        param.name := nameNode ? nameNode.Text : innerNode.Text
                    }
                } else
                    param.name := tsNode.Text
            case "default_param":
                ; Grammar: field("name", identifier) ":=" field("value", expr)
                nameNode := tsNode.GetChildByFieldName("name")
                param.name := nameNode.IsNull ? "" : nameNode.Text
                valNode := tsNode.GetChildByFieldName("value")
                if !valNode.IsNull
                    param.default := this._BuildNode(param, valNode, scope)
            case "variadic_param":
                ; Grammar: field("name", identifier) "*"
                param.isVariadic := true
                nameNode := tsNode.GetChildByFieldName("name")
                param.name := nameNode.IsNull ? tsNode.Text : nameNode.Text
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

        ; Class body from "body" field
        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull
            this._BuildClassBody(cls, bodyNode, cls.classScope)

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
        pendingDirectives := []
        i := 0
        while i < classBodyNode.NamedChildCount {
            child := classBodyNode.GetNamedChild(i)
            if child.Type == "directive_comment" {
                pendingDirectives.Push(this._ParseDirectiveComment(child))
                i++
                continue
            }
            irNode := unset
            switch child.Type {
                case "method_declaration":
                    irNode := this._BuildFunction(cls, child, classScope, true)
                    cls.methods.Push(irNode)
                case "property_declaration":
                    irNode := this._BuildPropertyOrField(cls, child, classScope)
                    if irNode is IR.Property
                        cls.properties.Push(irNode)
                    else if irNode is IR.Field {
                        if irNode.scope == "static"
                            cls.staticFields.Push(irNode)
                        else
                            cls.instanceFields.Push(irNode)
                    }
                case "class_declaration":
                    irNode := this._BuildClass(cls, child, classScope)
                    cls.nestedClasses.Push(irNode)
                default:
                    ; Other things in class body — opaque
                    irNode := this._BuildOpaque(cls, child)
            }
            if IsSet(irNode)
                this._AttachDirectives(irNode, pendingDirectives)
            i++
        }
        this._WarnTrailingDirectives(pendingDirectives)
    }

    /**
     * Build an IR.Property or IR.Field from a property_declaration.
     *
     * Grammar: optional(scope_identifier) field("name", identifier)
     *          choice(_initializer, seq("=>", getter), property_declaration_block)
     *
     * Decides based on structure:
     *   - If it has a getter/setter block or => arrow → IR.Property
     *   - If it's just `name := value` → IR.Field
     */
    _BuildPropertyOrField(parent, tsNode, scope) {
        ; Name from field
        nameNode := tsNode.GetChildByFieldName("name")
        propName := nameNode.IsNull ? "" : nameNode.Text

        ; Scope qualifier (optional scope_identifier before name)
        propScope := ""
        scopeNode := this._FindNamedChild(tsNode, "scope_identifier")
        if scopeNode
            propScope := scopeNode.Text

        ; Determine form by checking for getter/property_declaration_block children
        hasArrow := false
        hasBlock := false
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            switch child.Type {
                case "property_declaration_block":
                    hasBlock := true
                case "getter", "setter":
                    hasArrow := true
            }
            i++
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
            ; Shorthand Prop => expr — aliased as a "getter" named child
            prop.isGetterOnly := true
            prop.isArrowGetter := true
            getterChild := this._FindNamedChild(tsNode, "getter")
            if getterChild {
                getter := IR.Function(prop, tsNode)
                getter.name := propName
                getter.isArrow := true
                getter.isMethod := true
                getter.body := this._BuildNode(getter, getterChild, scope)
                getter.localScope := IRScope("function", getter, scope)
                prop.getter := getter
                prop.children.Push(getter)
            }
        } else {
            ; Has a property_declaration_block with getter/setter
            blockChild := this._FindNamedChild(tsNode, "property_declaration_block")
            if blockChild
                this._BuildGetterSetter(prop, blockChild, scope)
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
     *
     * Grammar: _initializer is inline, so field("value", expr) propagates here.
     */
    _BuildField(parent, tsNode, scope, propName, propScope) {
        field := IR.Field(parent, tsNode)
        field.name := propName
        field.scope := propScope

        ; The "value" field comes from the inline _initializer rule
        valNode := tsNode.GetChildByFieldName("value")
        if !valNode.IsNull
            field.initializer := this._BuildNode(field, valNode, scope)

        parent.children.Push(field)
        return field
    }

    /**
     * Build an IR.VarDecl from a variable_declaration.
     *
     * Grammar: field("scope", scope_identifier) field("name", identifier)
     */
    _BuildVarDecl(parent, tsNode, scope) {
        decl := IR.VarDecl(parent, tsNode)

        scopeNode := tsNode.GetChildByFieldName("scope")
        if !scopeNode.IsNull
            decl.declScope := scopeNode.Text

        nameNode := tsNode.GetChildByFieldName("name")
        if !nameNode.IsNull
            decl.name := nameNode.Text

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
     * Assignment has `left` and `right` fields. The operator is the
     * `assignment_operator` named child.
     */
    _BuildAssignment(parent, tsNode, scope) {
        expr := IR.BinaryExpr(parent, tsNode)

        leftNode := tsNode.GetChildByFieldName("left")
        rightNode := tsNode.GetChildByFieldName("right")

        if !leftNode.IsNull
            expr.left := this._BuildNode(expr, leftNode, scope)
        if !rightNode.IsNull
            expr.right := this._BuildNode(expr, rightNode, scope)

        ; The operator is the assignment_operator named child
        opNode := this._FindNamedChild(tsNode, "assignment_operator")
        expr.operator := opNode ? opNode.Text : ":="

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
     *
     * Grammar: field("condition", expr) "?" field("true_branch", expr) ":" field("false_branch", expr)
     */
    _BuildTernaryExpr(parent, tsNode, scope) {
        expr := IR.TernaryExpr(parent, tsNode)

        condNode := tsNode.GetChildByFieldName("condition")
        trueBranch := tsNode.GetChildByFieldName("true_branch")
        falseBranch := tsNode.GetChildByFieldName("false_branch")

        if !condNode.IsNull
            expr.condition := this._BuildNode(expr, condNode, scope)
        if !trueBranch.IsNull
            expr.trueBranch := this._BuildNode(expr, trueBranch, scope)
        if !falseBranch.IsNull
            expr.falseBranch := this._BuildNode(expr, falseBranch, scope)

        parent.children.Push(expr)
        return expr
    }

    /**
     * Build an IR.CallExpr from function_call or call_statement.
     *
     * Grammar: field("function", expr) "(" field("arguments", optional(arg_sequence)) ")"
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

        ; Arguments from "arguments" field
        argsNode := tsNode.GetChildByFieldName("arguments")
        if !argsNode.IsNull
            this._BuildArgs(call, argsNode, scope)

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

        if !objNode.IsNull {
            ma.object := this._BuildNode(ma, objNode, scope)
        }
        if !memberNode.IsNull {
            ma.member := this._BuildNode(ma, memberNode, scope)
            ma.isDynamic := ma.member is IR.DerefExpr || ma.member is IR.DynamicIdentifier
        }

        parent.children.Push(ma)
        return ma
    }

    /**
     * Build an IR.IndexAccess.
     *
     * Grammar: field("object", expr) "[" field("arguments", optional(arg_sequence)) "]"
     */
    _BuildIndexAccess(parent, tsNode, scope) {
        ia := IR.IndexAccess(parent, tsNode)

        objNode := tsNode.GetChildByFieldName("object")
        if !objNode.IsNull
            ia.object := this._BuildNode(ia, objNode, scope)

        ; Arguments from "arguments" field
        argsNode := tsNode.GetChildByFieldName("arguments")
        if !argsNode.IsNull {
            j := 0
            while j < argsNode.NamedChildCount {
                argChild := argsNode.GetNamedChild(j)
                arg := this._BuildNode(ia, argChild, scope)
                ia.args.Push(arg)
                j++
            }
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
     *
     * Grammar: object_literal_member has field("key", ...) and field("value", ...)
     */
    _BuildObjectLiteral(parent, tsNode, scope) {
        obj := IR.ObjectLiteral(parent, tsNode)

        ; Walk children looking for object_literal_member nodes
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "object_literal_member" {
                this._BuildObjectMember(obj, child, scope)
            }
            ; Handle object_literal_member_sequence (inline _object_literal_member_sequence)
            else if child.Type == "object_literal_member_sequence" {
                k := 0
                while k < child.NamedChildCount {
                    member := child.GetNamedChild(k)
                    if member.Type == "object_literal_member"
                        this._BuildObjectMember(obj, member, scope)
                    k++
                }
            }
            i++
        }

        parent.children.Push(obj)
        return obj
    }

    /**
     * Build a single object literal member using field-based access.
     */
    _BuildObjectMember(obj, memberNode, scope) {
        keyTsNode := memberNode.GetChildByFieldName("key")
        valTsNode := memberNode.GetChildByFieldName("value")

        if !keyTsNode.IsNull {
            keyNode := this._BuildNode(obj, keyTsNode, scope)
            valNode := !valTsNode.IsNull ? this._BuildNode(obj, valTsNode, scope) : ""
            obj.pairs.Push({key: keyNode, value: valNode})
        }
    }

    /**
     * Build an IR.DerefExpr (%expr%).
     *
     * Grammar: "%" field("operand", expr) "%"
     */
    _BuildDerefExpr(parent, tsNode, scope) {
        deref := IR.DerefExpr(parent, tsNode)

        operandNode := tsNode.GetChildByFieldName("operand")
        if !operandNode.IsNull
            deref.inner := this._BuildNode(deref, operandNode, scope)

        parent.children.Push(deref)
        return deref
    }

    _BuildDynamicIdentifier(parent, tsNode, scope) {
        ident := IR.DynamicIdentifier(parent, tsNode)

        i := 0
        while i < tsNode.NamedChildCount {
            this._BuildNode(ident, tsNode.GetNamedChild(i), scope)
            i++
        }

        parent.children.Push(ident)
        return ident
    }

    /**
     * Build an IR.VarRefExpr (&var).
     *
     * Grammar: "&" field("operand", expr)
     */
    _BuildVarRefExpr(parent, tsNode, scope) {
        vr := IR.VarRefExpr(parent, tsNode)

        operandNode := tsNode.GetChildByFieldName("operand")
        if !operandNode.IsNull
            vr.operand := this._BuildNode(vr, operandNode, scope)

        parent.children.Push(vr)
        return vr
    }

    /**
     * Build an IR.FatArrow (anonymous arrow function as expression).
     *
     * Grammar: function_head "=>" field("body", expr)
     */
    _BuildFatArrow(parent, tsNode, scope) {
        arrow := IR.FatArrow(parent, tsNode)
        arrow.localScope := IRScope("arrow", arrow, scope)

        ; Parameters — find function_head, then delegate to _BuildParams
        headNode := tsNode.GetChildByFieldName("head")
        if !headNode.IsNull
            this._BuildFatArrowParams(arrow, headNode, arrow.localScope)

        ; Body — the expression after =>
        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull
            arrow.body := this._BuildNode(arrow, bodyNode, arrow.localScope)

        parent.children.Push(arrow)
        return arrow
    }

    /**
     * Build parameters for a fat arrow function, similar to _BuildParams
     * but stores on the FatArrow node.
     */
    _BuildFatArrowParams(arrow, headNode, fnScope) {
        ; Find param_sequence inside function_head
        paramContainer := headNode
        if headNode.Type == "function_head" {
            ps := this._FindNamedChild(headNode, "param_sequence")
            if ps
                paramContainer := ps
        }
        if paramContainer.Type != "param_sequence"
            return

        i := 0
        while i < paramContainer.NamedChildCount {
            child := paramContainer.GetNamedChild(i)
            param := this._BuildParam(arrow, child, fnScope)
            if param {
                arrow.params.Push(param)
                arrow.children.Push(param)

                sym := IRSymbol(param.name, "param")
                sym.node := param
                sym.scope := fnScope
                fnScope.Define(param.name, sym)
            }
            i++
        }
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
     *
     * Grammar: if field("condition", expr) field("body", block|stmt) field("else_block", repeat(else_statement))
     */
    _BuildIf(parent, tsNode, scope) {
        ifNode := IR.IfStmt(parent, tsNode)

        ; Condition
        condNode := tsNode.GetChildByFieldName("condition")
        if !condNode.IsNull
            ifNode.condition := this._BuildNode(ifNode, condNode, scope)

        ; Body (then-block)
        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull {
            if bodyNode.Type == "block"
                ifNode.thenBody := this._BuildBlock(ifNode, bodyNode, scope)
            else
                ifNode.thenBody := this._BuildNode(ifNode, bodyNode, scope)
        }

        ; Else block(s)
        elseNode := tsNode.GetChildByFieldName("else_block")
        if !elseNode.IsNull
            ifNode.elseBody := this._BuildElse(ifNode, elseNode, scope)

        parent.children.Push(ifNode)
        return ifNode
    }

    /**
     * Build the else branch of an if statement.
     * Can be: else { block } or else if (condition) { ... }
     *
     * Grammar: else field("body", choice(if_statement, block, stmt))
     */
    _BuildElse(parent, tsNode, scope) {
        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull {
            if bodyNode.Type == "if_statement"
                return this._BuildIf(parent, bodyNode, scope)
            else if bodyNode.Type == "block"
                return this._BuildBlock(parent, bodyNode, scope)
            else
                return this._BuildNode(parent, bodyNode, scope)
        }
        return this._BuildOpaque(parent, tsNode)
    }

    /**
     * Build an IR.WhileStmt.
     *
     * Grammar: while field("condition", expr) field("body", stmt)
     */
    _BuildWhile(parent, tsNode, scope) {
        w := IR.WhileStmt(parent, tsNode)

        condNode := tsNode.GetChildByFieldName("condition")
        if !condNode.IsNull
            w.condition := this._BuildNode(w, condNode, scope)

        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull {
            if bodyNode.Type == "block"
                w.body := this._BuildBlock(w, bodyNode, scope)
            else
                w.body := this._BuildNode(w, bodyNode, scope)
        }

        parent.children.Push(w)
        return w
    }

    /**
     * Build an IR.ForStmt.
     *
     * Grammar: for field("head", _for_params) field("body", stmt) optional(field("else_block", else_statement))
     * _for_params (inline): field("iterator", id) [, field("iterator", id)] in field("iterable", expr)
     */
    _BuildFor(parent, tsNode, scope) {
        f := IR.ForStmt(parent, tsNode)

        bodyNode := tsNode.GetChildByFieldName("body")

        ; Iterators and iterable — from inline _for_params fields on for_statement
        ; Collect all children with "iterator" field name, and the "iterable" field
        i := 0
        while i < tsNode.ChildCount {
            fieldName := tsNode.GetFieldNameForChild(i)
            child := tsNode.GetChild(i)
            if fieldName == "iterator" && child.IsNamed {
                id := this._BuildIdentifier(f, child)
                f.iterators.Push(id)
            } else if fieldName == "iterable" && child.IsNamed {
                f.iterable := this._BuildNode(f, child, scope)
            }
            i++
        }

        if !bodyNode.IsNull {
            if bodyNode.Type == "block"
                f.body := this._BuildBlock(f, bodyNode, scope)
            else
                f.body := this._BuildNode(f, bodyNode, scope)
        }

        ; Optional else from "else_block" field
        elseNode := tsNode.GetChildByFieldName("else_block")
        if !elseNode.IsNull
            f.elseBody := this._BuildElse(f, elseNode, scope)

        parent.children.Push(f)
        return f
    }

    /**
     * Build an IR.LoopStmt.
     *
     * Grammar: loop field("head", optional(...)) field("body", block|stmt) optional(field("until_block", until_statement))
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

        ; Until clause from "until_block" field
        untilNode := tsNode.GetChildByFieldName("until_block")
        if !untilNode.IsNull {
            ; until_statement has field("condition", expr)
            condNode := untilNode.GetChildByFieldName("condition")
            if !condNode.IsNull
                l.untilCondition := this._BuildNode(l, condNode, scope)
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
     *
     * Grammar: case_clause has field("value", ...) and field("body", ...)
     *          default_clause has field("body", ...)
     */
    _BuildCase(parent, tsNode, scope, isDefault := false) {
        c := IR.CaseClause(parent, tsNode)
        c.isDefault := isDefault

        ; Use field names to distinguish values from body statements
        i := 0
        while i < tsNode.ChildCount {
            fieldName := tsNode.GetFieldNameForChild(i)
            child := tsNode.GetChild(i)
            if child.IsNamed {
                if fieldName == "value"
                    c.values.Push(this._BuildNode(c, child, scope))
                else if fieldName == "body"
                    c.body.Push(this._BuildNode(c, child, scope))
            }
            i++
        }

        parent.children.Push(c)
        return c
    }

    /**
     * Build an IR.TryStmt.
     *
     * Grammar: try field("body", choice(seq(block, catch*, else?, finally?), expr))
     * The "body" field encompasses the entire structure; iterate by type for sub-parts.
     * catch_clause and finally_clause have their own "body" fields.
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
                    ; else_statement has field("body", ...)
                    elseBody := child.GetChildByFieldName("body")
                    if !elseBody.IsNull {
                        if elseBody.Type == "block"
                            t.elseBody := this._BuildBlock(t, elseBody, scope)
                        else
                            t.elseBody := this._BuildNode(t, elseBody, scope)
                    }
                case "finally_clause":
                    ; finally_clause has field("body", block)
                    finallyBody := child.GetChildByFieldName("body")
                    if !finallyBody.IsNull
                        t.finallyBody := this._BuildBlock(t, finallyBody, scope)
            }
            i++
        }

        parent.children.Push(t)
        return t
    }

    /**
     * Build an IR.CatchClause from a catch_clause.
     *
     * Grammar: catch field("head", _catch_params?) field("body", block)
     * _catch_params: field("type", identifier) optional(as field("variable", identifier))
     */
    _BuildCatch(parent, tsNode, scope) {
        c := IR.CatchClause(parent, tsNode)

        ; Error type from the "type" field (propagated from inline _catch_params)
        typeNode := tsNode.GetChildByFieldName("type")
        if !typeNode.IsNull
            c.errorTypes.Push(typeNode.Text)

        ; Variable name from the "variable" field
        varNode := tsNode.GetChildByFieldName("variable")
        if !varNode.IsNull
            c.varName := varNode.Text

        ; Body block
        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull
            c.body := this._BuildBlock(c, bodyNode, scope)

        parent.children.Push(c)
        return c
    }

    /**
     * Build an IR.ReturnStmt.
     *
     * Grammar: return field("value", optional(expr))
     */
    _BuildReturn(parent, tsNode, scope) {
        ret := IR.ReturnStmt(parent, tsNode)

        valNode := tsNode.GetChildByFieldName("value")
        if !valNode.IsNull
            ret.value := this._BuildNode(ret, valNode, scope)

        parent.children.Push(ret)
        return ret
    }

    /**
     * Build an IR.BreakStmt.
     *
     * Grammar: break field("looplabel", optional(identifier|string_literal))
     */
    _BuildBreak(parent, tsNode) {
        b := IR.BreakStmt(parent, tsNode)

        labelNode := tsNode.GetChildByFieldName("looplabel")
        if !labelNode.IsNull
            b.label := labelNode.Text

        parent.children.Push(b)
        return b
    }

    /**
     * Build an IR.ContinueStmt.
     *
     * Grammar: continue field("looplabel", optional(identifier|string_literal))
     */
    _BuildContinue(parent, tsNode) {
        c := IR.ContinueStmt(parent, tsNode)

        labelNode := tsNode.GetChildByFieldName("looplabel")
        if !labelNode.IsNull
            c.label := labelNode.Text

        parent.children.Push(c)
        return c
    }

    /**
     * Build an IR.ThrowStmt.
     *
     * Grammar: throw field("thrown", expr)
     */
    _BuildThrow(parent, tsNode, scope) {
        t := IR.ThrowStmt(parent, tsNode)

        thrownNode := tsNode.GetChildByFieldName("thrown")
        if !thrownNode.IsNull
            t.value := this._BuildNode(t, thrownNode, scope)

        parent.children.Push(t)
        return t
    }

    /**
     * Build an IR.GotoStmt.
     *
     * Grammar: goto field("label", expr)
     */
    _BuildGoto(parent, tsNode) {
        g := IR.GotoStmt(parent, tsNode)

        labelNode := tsNode.GetChildByFieldName("label")
        if !labelNode.IsNull
            g.label := labelNode.Text

        parent.children.Push(g)
        return g
    }

    /**
     * Build an IR.Label.
     *
     * Grammar: field("name", identifier) ":"
     */
    _BuildLabel(parent, tsNode, scope) {
        l := IR.Label(parent, tsNode)

        nameNode := tsNode.GetChildByFieldName("name")
        if !nameNode.IsNull
            l.name := nameNode.Text

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

        pendingDirectives := []
        i := 0
        while i < tsNode.NamedChildCount {
            child := tsNode.GetNamedChild(i)
            if child.Type == "directive_comment" {
                pendingDirectives.Push(this._ParseDirectiveComment(child))
                i++
                continue
            }
            node := this._BuildNode(blk, child, scope)
            this._AttachDirectives(node, pendingDirectives)
            blk.body.Push(node)
            i++
        }
        this._WarnTrailingDirectives(pendingDirectives)

        parent.children.Push(blk)
        return blk
    }

    ; -----------------------------------------------------------------
    ; AHK-specific
    ; -----------------------------------------------------------------

    /**
     * Build an IR.Hotkey.
     *
     * Grammar: field("trigger", hotkey_trigger) "::" field("body", optional(...))
     */
    _BuildHotkey(parent, tsNode, scope) {
        hk := IR.Hotkey(parent, tsNode)

        triggerNode := tsNode.GetChildByFieldName("trigger")
        if !triggerNode.IsNull
            hk.trigger := triggerNode.Text

        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull {
            if bodyNode.Type == "block"
                hk.body := this._BuildBlock(hk, bodyNode, scope)
            else
                hk.body := this._BuildNode(hk, bodyNode, scope)
        }

        parent.children.Push(hk)
        return hk
    }

    /**
     * Build an IR.Hotstring.
     *
     * Grammar: field("modifiers", ...) field("trigger", ...) "::" field("body", optional(...))
     */
    _BuildHotstring(parent, tsNode, scope) {
        hs := IR.Hotstring(parent, tsNode)

        triggerNode := tsNode.GetChildByFieldName("trigger")
        if !triggerNode.IsNull
            hs.trigger := triggerNode.Text

        modNode := tsNode.GetChildByFieldName("modifiers")
        if !modNode.IsNull
            hs.modifiers := modNode.Text

        bodyNode := tsNode.GetChildByFieldName("body")
        if !bodyNode.IsNull
            hs.replacement := this._BuildNode(hs, bodyNode, scope)

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

        ; Member access: resolve object subtree, skip member (it's a name,
        ; not a variable reference), then resolve the full qualified chain.
        if node is IR.MemberAccess {
            if node.HasOwnProp("object")
                this._ResolveReferences(node.object)
            if node.isDynamic
                this._ResolveReferences(node.member)
            this._ResolveMemberAccessChain(node)
            return
        }

        ; Recurse into all children
        for child in node.children
            this._ResolveReferences(child)

        ; After children are resolved, resolve call targets via function namespace
        if node is IR.CallExpr
            this._ResolveCallTarget(node)
    }

    /**
     * Resolve a CallExpr's target by looking up the callee in the function
     * namespace (symbol table). In AHK v2, `Foo()` resolves to function `Foo`
     * regardless of variable scope — functions have their own namespace.
     *
     * @param {IR.CallExpr} callNode
     */
    _ResolveCallTarget(callNode) {
        if callNode.isDynamic || !callNode.HasOwnProp("callee")
            return

        ; TODO if callee is a class, search it for a static Call method, add a reference to that, and increment its callCount
        if callNode.callee is IR.Identifier {
            ; In AHK v2, function calls resolve through the function namespace,
            ; not variable scope. Look up in the symbol table directly.
            sym := this.symbolTable.Lookup(callNode.callee.name)

            if sym && sym.HasOwnProp("node") {
                callNode.resolvedTarget := sym.node

                ; TODO if not a function increment "Call" method call count
                if(sym.kind == "function")
                    sym.callCount++
            }
        }
        else if callNode.callee is IR.MemberAccess
            && callNode.callee.HasOwnProp("resolvedSymbol") {
            ; Member access chain was resolved by _ResolveMemberAccessChain.
            sym := callNode.callee.resolvedSymbol
            if sym.HasOwnProp("node") {
                callNode.resolvedTarget := sym.node
                if sym.kind == "function"
                    sym.callCount++
            }
        }
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
     * Resolve a member access chain (e.g. Outer.Inner) by building the
     * fully qualified name and looking it up in the symbol table.
     *
     * Uses identifier text (not resolved symbols) because scope resolution
     * may create spurious locals in assume-local functions.
     *
     * @param {IR.MemberAccess} node
     */
    _ResolveMemberAccessChain(node) {
        if node.isDynamic
            return

        qName := this._BuildQualifiedName(node)
        if qName == ""
            return

        sym := this.symbolTable.Lookup(qName)
        if sym {
            node.resolvedSymbol := sym
            this.symbolTable.AddReference(sym, node)
        }

        ; Also resolve the object via the symbol table. Scope resolution
        ; may have created a spurious local (assume-local functions), but
        ; if the full chain matched a symbol, the object must be a class.
        if node.object is IR.MemberAccess {
            this._ResolveMemberAccessChain(node.object)
        } else if node.object is IR.Identifier {
            objSym := this.symbolTable.Lookup(node.object.name)
            if objSym {
                node.object.resolvedSymbol := objSym
                this.symbolTable.AddReference(objSym, node.object)
            }
        }
    }

    /**
     * Recursively build a dotted qualified name from a member access chain.
     * Returns "" if any part is dynamic or not an identifier/member-access.
     *
     * @param {IR.Node} node
     * @returns {String}
     */
    _BuildQualifiedName(node) {
        if node is IR.Identifier
            return node.name

        if node is IR.MemberAccess && !node.isDynamic {
            objPart := this._BuildQualifiedName(node.object)
            if objPart == ""
                return ""
            return StrLower(objPart "." node.member.GetText())
        }
        return ""
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
