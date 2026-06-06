//! CST -> IR lowering.
//!
//! Lowers the tree-sitter CST into an owned IR. Analysis (scope chains, symbol tables,
//! reference resolution) is not done here. This just builds the data structure.
//!
//! Note on exhaustiveness: the `match node.kind()` dispatch below has a `_ => Opaque`
//! catch-all, but that arm is on the *tree-sitter* kind string, not on [`NodeKind`]. The
//! enum-exhaustiveness guarantee that motivates the rewrite lives on the IR *walkers*
//! (`print.rs` and later passes), which `match &NodeKind` with no catch-all.

use std::collections::HashMap;

use ahkbuild_syntax::tree_sitter::{Node, Tree};
use ahkbuild_syntax::Span;

use crate::arena::{Arena, NodeId};
use crate::node::*;
use crate::program::Program;

/// Lower a parsed tree into an owned [`Program`].
pub fn lower(tree: &Tree, source: &str) -> Program {
    let mut l = Lowerer {
        arena: Arena::new(),
        source,
        directives: HashMap::new(),
    };
    let (modules, main) = l.build_top_level(tree.root_node());
    let _ = main;
    Program {
        modules,
        arena: l.arena,
        source: source.to_string(),
        directives: l.directives,
    }
}

struct Lowerer<'a> {
    arena: Arena,
    source: &'a str,
    directives: HashMap<NodeId, Vec<DirectiveComment>>,
}

impl<'a> Lowerer<'a> {
    // -----------------------------------------------------------------
    // Top level + module grouping
    // -----------------------------------------------------------------

    /// Walk the `source_file` children, grouping statements into modules. Everything before
    /// the first `#Module` goes into an implicit `__Main` module; each `#Module Name`
    /// reopens-or-creates its module and subsequent statements append to it.
    fn build_top_level(&mut self, root: Node) -> (Vec<NodeId>, NodeId) {
        // Implicit __Main, created up front and keyed "__Main".
        let main_span = Span::of(root);
        let main = self.arena.alloc(
            main_span,
            NodeKind::Module(Module {
                name: Module::MAIN.to_string(),
                name_span: None,
                body: Vec::new(),
            }),
        );
        let mut modules = vec![main];
        let mut by_name: HashMap<String, NodeId> = HashMap::new();
        by_name.insert(Module::MAIN.to_ascii_lowercase(), main);

        let mut current = main;
        let mut pending: Vec<DirectiveComment> = Vec::new();

        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                "directive_comment" => {
                    pending.push(self.parse_directive_comment(child));
                }
                "module_directive" => {
                    let ident = child.named_child(0);
                    let name = ident
                        .map(|n| n.span_text(self.source).trim().to_string())
                        .unwrap_or_default();
                    let key = name.to_ascii_lowercase();
                    current = if let Some(&existing) = by_name.get(&key) {
                        existing
                    } else {
                        let id = self.arena.alloc(
                            Span::of(child),
                            NodeKind::Module(Module {
                                name,
                                name_span: ident.map(|n| self.name_of(n)),
                                body: Vec::new(),
                            }),
                        );
                        modules.push(id);
                        by_name.insert(key, id);
                        id
                    };
                }
                _ => {
                    let id = self.build_node(child);
                    self.attach_directives(id, &mut pending);
                    self.push_to_module(current, id);
                }
            }
        }
        self.warn_trailing(&pending);
        (modules, main)
    }

    fn push_to_module(&mut self, module: NodeId, child: NodeId) {
        if let NodeKind::Module(m) = &mut self.arena.get_mut(module).kind {
            m.body.push(child);
        }
    }

    // -----------------------------------------------------------------
    // Directive comments
    // -----------------------------------------------------------------

    fn parse_directive_comment(&self, node: Node) -> DirectiveComment {
        DirectiveComment {
            name: node
                .child_by_field_name("directive")
                .map(Span::of)
                .unwrap_or_else(|| Span::of(node)),
            arguments: node.child_by_field_name("arguments").map(Span::of),
        }
    }

    fn attach_directives(&mut self, id: NodeId, pending: &mut Vec<DirectiveComment>) {
        if !pending.is_empty() {
            self.directives.entry(id).or_default().append(pending);
        }
    }

    fn warn_trailing(&self, pending: &[DirectiveComment]) {
        for d in pending {
            eprintln!(
                "warning: directive `;@{}` has no following statement",
                d.name.text(self.source)
            );
        }
    }

    // -----------------------------------------------------------------
    // Dispatch
    // -----------------------------------------------------------------

    /// Build one IR node from a tree-sitter node. Mirrors `_BuildNode`.
    fn build_node(&mut self, node: Node) -> NodeId {
        match node.kind() {
            // Declarations
            "function_declaration" => self.build_function(node, false, false),
            "function_expression" => self.build_function(node, false, true),
            "method_declaration" => self.build_function(node, true, false),
            "class_declaration" => self.build_type_decl(node, false),
            "struct_declaration" => self.build_type_decl(node, true),
            "property_declaration" => self.build_property_or_field(node),
            "typed_property_declaration" => self.build_typed_property(node),
            "type_specifier" => self.build_type_specifier(node),
            "variable_declaration" => self.build_var_decl(node),
            "export_declaration" => self.build_export(node),
            "import_directive" => self.build_import(node),

            // Binary expressions
            "additive_operation"
            | "multiplicative_operation"
            | "exponent_operation"
            | "relational_operation"
            | "equality_operation"
            | "inequality_operation"
            | "logical_and_operation"
            | "logical_or_operation"
            | "bitwise_and_operation"
            | "bitwise_or_operation"
            | "bitwise_xor_operation"
            | "bitshift_operation"
            | "explicit_concat_operation"
            | "implicit_concat_operation"
            | "or_maybe_operation"
            | "regex_match_operation"
            | "type_check_operation" => self.build_binary(node, false),
            "assignment_operation" => self.build_binary(node, true),

            // Unary expressions
            "prefix_operation" | "verbal_not_operation" => self.build_unary(node, true),
            "postfix_operation" => self.build_unary(node, false),

            // Other expressions
            "ternary_expression" => self.build_ternary(node),
            "function_call" => self.build_call(node, false),
            "call_statement" => self.build_call(node, true),
            "member_access" => self.build_member_access(node),
            "index_access" => self.build_index_access(node),
            "identifier" => self.alloc(node, NodeKind::Identifier),
            "integer_literal" | "hex_literal" => self.alloc(
                node,
                NodeKind::Literal {
                    kind: LiteralKind::Integer,
                },
            ),
            "float_literal" => self.alloc(
                node,
                NodeKind::Literal {
                    kind: LiteralKind::Float,
                },
            ),
            "string_literal" | "multiline_string_literal" => self.alloc(
                node,
                NodeKind::Literal {
                    kind: LiteralKind::String,
                },
            ),
            "boolean_literal" => self.alloc(
                node,
                NodeKind::Literal {
                    kind: LiteralKind::Boolean,
                },
            ),
            "array_literal" => self.build_array_literal(node),
            "object_literal" => self.build_object_literal(node),
            "dereference_operation" => self.build_deref(node),
            "dynamic_identifier" => self.build_dynamic_identifier(node),
            "varref_operation" => self.build_varref(node),
            "fat_arrow_function" => self.build_fat_arrow(node),
            "expression_sequence" => self.build_expression_sequence(node),

            // Control flow
            "if_statement" => self.build_if(node),
            "while_statement" => self.build_while(node),
            "for_statement" => self.build_for(node),
            "loop_statement" => self.build_loop(node),
            "switch_statement" => self.build_switch(node),
            "try_statement" => self.build_try(node),
            "return_statement" => self.build_return(node),
            "break_statement" => self.build_break(node),
            "continue_statement" => self.build_continue(node),
            "throw_statement" => self.build_throw(node),
            "goto_statement" => self.build_goto(node),
            "label" => self.build_label(node),

            // Blocks
            "block" => self.build_block(node),

            // AHK-specific
            "hotkey" => self.build_hotkey(node),
            "hotstring" => self.build_hotstring(node),
            "hotif_directive" => self.build_hotif(node),
            k if k.ends_with("_directive") => self.build_directive(node),

            // Fallback
            _ => self.alloc(node, NodeKind::Opaque),
        }
    }

    // -----------------------------------------------------------------
    // Declarations
    // -----------------------------------------------------------------

    fn build_function(&mut self, node: Node, is_method: bool, is_expression: bool) -> NodeId {
        self.build_function_owned(node, is_method, is_expression, None)
    }

    fn build_function_owned(
        &mut self,
        node: Node,
        is_method: bool,
        is_expression: bool,
        owner: Option<NodeId>,
    ) -> NodeId {
        let name = self.field_name(node, "name");

        let is_static = node
            .child(0)
            .map(|c| {
                c.kind() == "scope_identifier"
                    && c.span_text(self.source).eq_ignore_ascii_case("static")
            })
            .unwrap_or(false);

        let mut params = Vec::new();
        if let Some(head) = node.child_by_field_name("head") {
            self.build_params(head, &mut params);
        }
        let is_variadic = params.last().map(|p| p.variadic).unwrap_or(false);

        let (body, is_arrow) = match node.child_by_field_name("body") {
            Some(b) if b.kind() == "function_body" => self.build_function_body(b),
            Some(b) => (Some(self.build_node(b)), false),
            None => (None, false),
        };

        self.alloc(
            node,
            NodeKind::Function(Function {
                name,
                params,
                body,
                is_static,
                is_method,
                is_variadic,
                is_arrow,
                is_expression,
                owner,
            }),
        )
    }

    /// Build a `function_body`, returning `(body_id, is_arrow)`. A `block` child is a
    /// statement body; anything else is an arrow (`=>`) expression body.
    fn build_function_body(&mut self, body: Node) -> (Option<NodeId>, bool) {
        match body.named_child(0) {
            Some(child) if child.kind() == "block" => (Some(self.build_block(child)), false),
            Some(child) => (Some(self.build_node(child)), true),
            None => (None, false),
        }
    }

    fn build_params(&mut self, head: Node, out: &mut Vec<Param>) {
        // `head` may be a function_head containing a param_sequence, or a param_sequence.
        let container = if head.kind() == "function_head" {
            self.named_child_of_kind(head, "param_sequence")
                .unwrap_or(head)
        } else {
            head
        };
        if container.kind() != "param_sequence" {
            return;
        }
        let mut cursor = container.walk();
        for child in container.named_children(&mut cursor) {
            if let Some(param) = self.build_param(child) {
                out.push(param);
            }
        }
    }

    fn build_param(&mut self, node: Node) -> Option<Param> {
        let mut param = Param {
            name: None,
            default: None,
            by_ref: false,
            variadic: false,
            optional: false,
        };
        match node.kind() {
            "identifier" => param.name = Some(self.name_of(node)),
            "byref_param" => {
                param.by_ref = true;
                if let Some(inner) = node.child_by_field_name("param") {
                    self.fill_param_inner(&mut param, inner);
                } else {
                    param.name = Some(self.name_of(node));
                }
            }
            "default_param" => {
                param.name = self.field_name(node, "name");
                param.default = node
                    .child_by_field_name("value")
                    .map(|v| self.build_node(v));
            }
            "variadic_param" => {
                param.variadic = true;
                param.name = self.field_name(node, "name");
            }
            "optional_param" => {
                param.optional = true;
                param.name = self
                    .named_child_of_kind(node, "identifier")
                    .map(|n| self.name_of(n));
            }
            _ => return None,
        }
        Some(param)
    }

    fn fill_param_inner(&mut self, param: &mut Param, inner: Node) {
        match inner.kind() {
            "identifier" => param.name = Some(self.name_of(inner)),
            "default_param" => {
                param.name = self.field_name(inner, "name");
                param.default = inner
                    .child_by_field_name("value")
                    .map(|v| self.build_node(v));
            }
            _ => {
                param.name = self
                    .named_child_of_kind(inner, "identifier")
                    .map(|n| self.name_of(n))
                    .or(Some(self.name_of(inner)));
            }
        }
    }

    fn build_type_decl(&mut self, node: Node, is_struct: bool) -> NodeId {
        let name = self.field_name(node, "name");
        let superclass = self.field_name(node, "superclass");

        // Allocate the type node first so members can reference it as `owner`.
        let placeholder = TypeDecl {
            name,
            superclass,
            ..TypeDecl::default()
        };
        let id = self.alloc(
            node,
            if is_struct {
                NodeKind::StructDecl(placeholder.clone())
            } else {
                NodeKind::ClassDecl(placeholder.clone())
            },
        );

        let mut decl = placeholder;
        if let Some(body) = node.child_by_field_name("body") {
            self.build_type_body(body, id, &mut decl);
        }

        self.arena.get_mut(id).kind = if is_struct {
            NodeKind::StructDecl(decl)
        } else {
            NodeKind::ClassDecl(decl)
        };
        id
    }

    fn build_type_body(&mut self, body: Node, owner: NodeId, decl: &mut TypeDecl) {
        let mut pending: Vec<DirectiveComment> = Vec::new();
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            let member = match child.kind() {
                "directive_comment" => {
                    pending.push(self.parse_directive_comment(child));
                    continue;
                }
                "method_declaration" => {
                    let m = self.build_function_owned(child, true, false, Some(owner));
                    decl.methods.push(m);
                    m
                }
                "property_declaration" => {
                    let p = self.build_property_or_field_owned(child, Some(owner));
                    match &self.arena[p].kind {
                        NodeKind::Property(_) => decl.properties.push(p),
                        NodeKind::Field(f) => {
                            if f.is_static {
                                decl.static_fields.push(p)
                            } else {
                                decl.instance_fields.push(p)
                            }
                        }
                        _ => {}
                    }
                    p
                }
                "typed_property_declaration" => {
                    let t = self.build_typed_property(child);
                    decl.typed_fields.push(t);
                    t
                }
                "class_declaration" => {
                    let c = self.build_type_decl(child, false);
                    decl.nested.push(c);
                    c
                }
                "struct_declaration" => {
                    let s = self.build_type_decl(child, true);
                    decl.nested.push(s);
                    s
                }
                _ => self.alloc(child, NodeKind::Opaque),
            };
            self.attach_directives(member, &mut pending);
        }
        self.warn_trailing(&pending);
    }

    fn build_property_or_field(&mut self, node: Node) -> NodeId {
        self.build_property_or_field_owned(node, None)
    }

    fn build_property_or_field_owned(&mut self, node: Node, owner: Option<NodeId>) -> NodeId {
        let name = self.field_name(node, "name");
        let is_static = self
            .named_child_of_kind(node, "scope_identifier")
            .map(|n| n.span_text(self.source).eq_ignore_ascii_case("static"))
            .unwrap_or(false);

        let mut has_block = false;
        let mut has_arrow = false;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "property_declaration_block" => has_block = true,
                "getter" | "setter" => has_arrow = true,
                _ => {}
            }
        }

        if has_arrow || has_block {
            self.build_property(node, name, is_static, has_arrow, owner)
        } else {
            self.build_field(node, name, is_static)
        }
    }

    fn build_property(
        &mut self,
        node: Node,
        name: Option<Span>,
        is_static: bool,
        shorthand_arrow: bool,
        owner: Option<NodeId>,
    ) -> NodeId {
        let mut prop = Property {
            name,
            is_static,
            getter: None,
            setter: None,
            is_getter_only: false,
            is_arrow_getter: false,
            is_arrow_setter: false,
        };

        if shorthand_arrow {
            // Shorthand `Prop => expr` — exposed as a `getter` named child wrapping the
            // arrow body, built like any other accessor.
            prop.is_getter_only = true;
            if let Some(getter_child) = self.named_child_of_kind(node, "getter") {
                let (g, is_arrow) = self.build_accessor(getter_child, name, is_static, owner);
                prop.getter = Some(g);
                prop.is_arrow_getter = is_arrow;
            }
        } else if let Some(block) = self.named_child_of_kind(node, "property_declaration_block") {
            self.build_accessors(block, name, is_static, owner, &mut prop);
        }

        self.alloc(node, NodeKind::Property(prop))
    }

    fn build_accessors(
        &mut self,
        block: Node,
        name: Option<Span>,
        is_static: bool,
        owner: Option<NodeId>,
        prop: &mut Property,
    ) {
        let mut cursor = block.walk();
        for child in block.named_children(&mut cursor) {
            match child.kind() {
                "getter" => {
                    let (id, is_arrow) = self.build_accessor(child, name, is_static, owner);
                    prop.getter = Some(id);
                    prop.is_arrow_getter = is_arrow;
                }
                "setter" => {
                    let (id, is_arrow) = self.build_accessor(child, name, is_static, owner);
                    prop.setter = Some(id);
                    prop.is_arrow_setter = is_arrow;
                }
                _ => {}
            }
        }
    }

    /// Build a getter/setter accessor as an `IR.Function`, returning `(id, is_arrow)`.
    fn build_accessor(
        &mut self,
        node: Node,
        name: Option<Span>,
        is_static: bool,
        owner: Option<NodeId>,
    ) -> (NodeId, bool) {
        let (body, is_arrow) = match self.named_child_of_kind(node, "function_body") {
            Some(b) => self.build_function_body(b),
            // Shorthand `Prop => expr` holds the expression directly under the `getter`
            // node, with no `function_body` wrapper — build it as the arrow body.
            None => match node.named_child(0) {
                Some(expr) => (Some(self.build_node(expr)), true),
                None => (None, false),
            },
        };
        let id = self.alloc(
            node,
            NodeKind::Function(Function {
                name,
                params: Vec::new(),
                body,
                is_static,
                is_method: true,
                is_variadic: false,
                is_arrow,
                is_expression: false,
                owner,
            }),
        );
        (id, is_arrow)
    }

    fn build_field(&mut self, node: Node, name: Option<Span>, is_static: bool) -> NodeId {
        let initializer = node
            .child_by_field_name("value")
            .map(|v| self.build_node(v));
        self.alloc(
            node,
            NodeKind::Field(Field {
                name,
                is_static,
                initializer,
            }),
        )
    }

    /// Build a v2.1 typed field: `name: Type := value`.
    fn build_typed_property(&mut self, node: Node) -> NodeId {
        let name = self.field_name(node, "name");
        let type_spec = match node.child_by_field_name("type") {
            Some(t) => self.build_type_specifier(t),
            None => self.alloc(node, NodeKind::Opaque),
        };
        let initializer = node
            .child_by_field_name("value")
            .map(|v| self.build_node(v));
        self.alloc(
            node,
            NodeKind::TypedProperty(TypedProperty {
                name,
                type_spec,
                initializer,
            }),
        )
    }

    /// Build a v2.1 `: Type` annotation, wrapping its single type expression child.
    fn build_type_specifier(&mut self, node: Node) -> NodeId {
        let type_expr = match node.named_child(0) {
            Some(e) => self.build_node(e),
            None => self.alloc(node, NodeKind::Opaque),
        };
        self.alloc(node, NodeKind::TypeSpecifier { type_expr })
    }

    fn build_var_decl(&mut self, node: Node) -> NodeId {
        let scope = match node
            .child_by_field_name("scope")
            .map(|n| n.span_text(self.source).to_ascii_lowercase())
            .as_deref()
        {
            Some("global") => VarScope::Global,
            Some("static") => VarScope::Static,
            _ => VarScope::Local,
        };
        let name = self.field_name(node, "name");
        let initializer = node
            .child_by_field_name("value")
            .map(|v| self.build_node(v));
        self.alloc(
            node,
            NodeKind::VarDecl(VarDecl {
                name,
                scope,
                initializer,
            }),
        )
    }

    // -----------------------------------------------------------------
    // Modules: import / export
    // -----------------------------------------------------------------

    fn build_import(&mut self, node: Node) -> NodeId {
        let source = match node.child_by_field_name("module") {
            Some(m) if m.kind() == "string_literal" => ImportSource::Path(self.name_of(m)),
            Some(m) => ImportSource::Name(self.name_of(m)),
            None => ImportSource::Name(self.name_of(node)),
        };

        let binding = if let Some(alias) = node.child_by_field_name("alias") {
            ImportBinding::Alias(self.name_of(alias))
        } else if self.has_named_child(node, "export_name")
            || self.has_named_child(node, "wildcard")
        {
            let mut wildcard = false;
            let mut names = Vec::new();
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    "wildcard" => wildcard = true,
                    "export_name" => names.push(ImportName {
                        name: self
                            .field_name(child, "export")
                            .unwrap_or_else(|| self.name_of(child)),
                        alias: self.field_name(child, "alias"),
                    }),
                    _ => {}
                }
            }
            ImportBinding::Selective { wildcard, names }
        } else {
            ImportBinding::Whole
        };

        self.alloc(
            node,
            NodeKind::ImportDirective(ImportDirective { source, binding }),
        )
    }

    fn build_export(&mut self, node: Node) -> NodeId {
        let default = self.has_named_child(node, "default");
        // The exported entity is either a `declaration` field (function/class/struct) or a
        // `variable_declaration` child (`export global X := 1`).
        let inner = node
            .child_by_field_name("declaration")
            .or_else(|| self.named_child_of_kind(node, "variable_declaration"));
        let decl = match inner {
            Some(d) => self.build_node(d),
            None => self.alloc(node, NodeKind::Opaque),
        };
        // An exported variable (`export global X := 1`) is always module-global. The
        // `global` keyword is mandatory disambiguation (it distinguishes an exported
        // variable from `export var`, a command-style call to `export`), not a scope
        // choice — there is only one possible scope — and the grammar drops it from the
        // tree, so the inner `variable_declaration` has no scope field. Set it here.
        if matches!(inner.map(|d| d.kind()), Some("variable_declaration")) {
            if let NodeKind::VarDecl(v) = &mut self.arena.get_mut(decl).kind {
                v.scope = VarScope::Global;
            }
        }
        self.alloc(node, NodeKind::ExportDecl { default, decl })
    }

    // -----------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------

    fn build_binary(&mut self, node: Node, is_assignment: bool) -> NodeId {
        let left = node.child_by_field_name("left").map(|n| self.build_node(n));
        let right = node
            .child_by_field_name("right")
            .map(|n| self.build_node(n));

        let op = if is_assignment {
            self.named_child_of_kind(node, "assignment_operator")
                .map(Span::of)
        } else {
            node.child_by_field_name("operator").map(Span::of)
        };

        // Fall back to the gap between operands (covers implicit-concat whitespace).
        let (left, right) = (
            left.unwrap_or_else(|| self.alloc(node, NodeKind::Opaque)),
            right.unwrap_or_else(|| self.alloc(node, NodeKind::Opaque)),
        );
        let op = op.unwrap_or(Span {
            start: self.arena[left].span.end,
            end: self.arena[right].span.start,
        });

        self.alloc(node, NodeKind::BinaryExpr { left, op, right })
    }

    fn build_unary(&mut self, node: Node, prefix: bool) -> NodeId {
        let op = node
            .child_by_field_name("operator")
            .map(Span::of)
            .unwrap_or_else(|| Span::of(node));
        let operand = node
            .child_by_field_name("operand")
            .map(|n| self.build_node(n))
            .unwrap_or_else(|| self.alloc(node, NodeKind::Opaque));
        self.alloc(
            node,
            NodeKind::UnaryExpr {
                op,
                operand,
                prefix,
            },
        )
    }

    fn build_ternary(&mut self, node: Node) -> NodeId {
        let condition = self.build_field_or_opaque(node, "condition");
        let then_branch = self.build_field_or_opaque(node, "true_branch");
        let else_branch = self.build_field_or_opaque(node, "false_branch");
        self.alloc(
            node,
            NodeKind::TernaryExpr {
                condition,
                then_branch,
                else_branch,
            },
        )
    }

    fn build_call(&mut self, node: Node, is_command_style: bool) -> NodeId {
        let callee = node
            .child_by_field_name("function")
            .map(|n| self.build_node(n))
            .unwrap_or_else(|| self.alloc(node, NodeKind::Opaque));
        let is_dynamic = matches!(self.arena[callee].kind, NodeKind::DerefExpr { .. });

        let mut args = Vec::new();
        if let Some(arg_seq) = node.child_by_field_name("arguments") {
            let mut cursor = arg_seq.walk();
            for child in arg_seq.named_children(&mut cursor) {
                args.push(self.build_node(child));
            }
        }

        self.alloc(
            node,
            NodeKind::CallExpr(CallExpr {
                callee,
                args,
                is_command_style,
                is_dynamic,
            }),
        )
    }

    fn build_member_access(&mut self, node: Node) -> NodeId {
        let object = self.build_field_or_opaque(node, "object");
        let member = self.build_field_or_opaque(node, "member");
        let is_dynamic = matches!(
            self.arena[member].kind,
            NodeKind::DerefExpr { .. } | NodeKind::DynamicIdentifier { .. }
        );
        self.alloc(
            node,
            NodeKind::MemberAccess {
                object,
                member,
                is_dynamic,
            },
        )
    }

    fn build_index_access(&mut self, node: Node) -> NodeId {
        let object = self.build_field_or_opaque(node, "object");
        let mut args = Vec::new();
        if let Some(arg_seq) = node.child_by_field_name("arguments") {
            let mut cursor = arg_seq.walk();
            for child in arg_seq.named_children(&mut cursor) {
                args.push(self.build_node(child));
            }
        }
        self.alloc(node, NodeKind::IndexAccess { object, args })
    }

    fn build_array_literal(&mut self, node: Node) -> NodeId {
        let mut elements = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            elements.push(self.build_node(child));
        }
        self.alloc(node, NodeKind::ArrayLiteral { elements })
    }

    fn build_object_literal(&mut self, node: Node) -> NodeId {
        let mut members = Vec::new();
        self.collect_object_members(node, &mut members);
        self.alloc(node, NodeKind::ObjectLiteral { members })
    }

    fn collect_object_members(&mut self, node: Node, out: &mut Vec<ObjectMember>) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "object_literal_member" => {
                    if let Some(key) = child.child_by_field_name("key") {
                        let key = self.build_node(key);
                        let value = child
                            .child_by_field_name("value")
                            .map(|v| self.build_node(v));
                        out.push(ObjectMember { key, value });
                    }
                }
                "object_literal_member_sequence" => self.collect_object_members(child, out),
                _ => {}
            }
        }
    }

    fn build_deref(&mut self, node: Node) -> NodeId {
        let inner = self.build_field_or_opaque(node, "operand");
        self.alloc(node, NodeKind::DerefExpr { inner })
    }

    fn build_dynamic_identifier(&mut self, node: Node) -> NodeId {
        let mut parts = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            parts.push(self.build_node(child));
        }
        self.alloc(node, NodeKind::DynamicIdentifier { parts })
    }

    fn build_varref(&mut self, node: Node) -> NodeId {
        let operand = self.build_field_or_opaque(node, "operand");
        self.alloc(node, NodeKind::VarRefExpr { operand })
    }

    fn build_fat_arrow(&mut self, node: Node) -> NodeId {
        let mut params = Vec::new();
        if let Some(head) = node.child_by_field_name("head") {
            self.build_params(head, &mut params);
        }
        let body = self.build_field_or_opaque(node, "body");
        self.alloc(node, NodeKind::FatArrow { params, body })
    }

    fn build_expression_sequence(&mut self, node: Node) -> NodeId {
        // A single-expression sequence is just that expression. Multi-expression sequences
        // (comma operator) are rarely interesting to transforms; emit verbatim as Opaque.
        // TODO: model multi-expression sequences if a pass needs their sub-references.
        if node.named_child_count() == 1 {
            if let Some(child) = node.named_child(0) {
                return self.build_node(child);
            }
        }
        self.alloc(node, NodeKind::Opaque)
    }

    // -----------------------------------------------------------------
    // Control flow
    // -----------------------------------------------------------------

    fn build_if(&mut self, node: Node) -> NodeId {
        let condition = self.build_field_or_opaque(node, "condition");
        let then_body = self.build_field_or_opaque(node, "body");
        let else_body = node
            .child_by_field_name("else_block")
            .map(|e| self.build_else(e));
        self.alloc(
            node,
            NodeKind::IfStmt {
                condition,
                then_body,
                else_body,
            },
        )
    }

    fn build_else(&mut self, node: Node) -> NodeId {
        match node.child_by_field_name("body") {
            Some(b) => self.build_node(b),
            None => self.alloc(node, NodeKind::Opaque),
        }
    }

    fn build_while(&mut self, node: Node) -> NodeId {
        let condition = self.build_field_or_opaque(node, "condition");
        let body = self.build_field_or_opaque(node, "body");
        self.alloc(node, NodeKind::WhileStmt { condition, body })
    }

    fn build_for(&mut self, node: Node) -> NodeId {
        let mut iterators = Vec::new();
        let mut iterable = None;
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.is_named() {
                    match cursor.field_name() {
                        Some("iterator") => iterators.push(self.build_node(child)),
                        Some("iterable") => iterable = Some(self.build_node(child)),
                        _ => {}
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        let body = node.child_by_field_name("body").map(|b| self.build_node(b));
        let else_body = node
            .child_by_field_name("else_block")
            .map(|e| self.build_else(e));
        self.alloc(
            node,
            NodeKind::ForStmt(ForStmt {
                iterators,
                iterable,
                body,
                else_body,
            }),
        )
    }

    fn build_loop(&mut self, node: Node) -> NodeId {
        let head = node.child_by_field_name("head");
        let kind = match head {
            None => LoopKind::Infinite,
            Some(h) => {
                let t = h.span_text(self.source).to_ascii_lowercase();
                if t.contains("parse") {
                    LoopKind::Parse
                } else if t.contains("read") {
                    LoopKind::Read
                } else if t.contains("reg") {
                    LoopKind::Reg
                } else if t.contains("files") {
                    LoopKind::Files
                } else {
                    LoopKind::Count
                }
            }
        };
        let head = head.map(|h| self.build_node(h));
        let body = node.child_by_field_name("body").map(|b| self.build_node(b));
        let until = node
            .child_by_field_name("until_block")
            .and_then(|u| u.child_by_field_name("condition"))
            .map(|c| self.build_node(c));
        self.alloc(
            node,
            NodeKind::LoopStmt(LoopStmt {
                kind,
                head,
                body,
                until,
            }),
        )
    }

    fn build_switch(&mut self, node: Node) -> NodeId {
        let discriminant = node.child_by_field_name("head").map(|h| self.build_node(h));
        let mut cases = Vec::new();
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.named_children(&mut cursor) {
                match child.kind() {
                    "case_clause" => cases.push(self.build_case(child, false)),
                    "default_clause" => cases.push(self.build_case(child, true)),
                    _ => {}
                }
            }
        }
        self.alloc(
            node,
            NodeKind::SwitchStmt {
                discriminant,
                cases,
            },
        )
    }

    fn build_case(&mut self, node: Node, is_default: bool) -> NodeId {
        let mut values = Vec::new();
        let mut body = Vec::new();
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.is_named() {
                    match cursor.field_name() {
                        Some("value") => values.push(self.build_node(child)),
                        Some("body") => body.push(self.build_node(child)),
                        _ => {}
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        self.alloc(
            node,
            NodeKind::CaseClause {
                values,
                body,
                is_default,
            },
        )
    }

    fn build_try(&mut self, node: Node) -> NodeId {
        let mut try_body = None;
        let mut catches = Vec::new();
        let mut else_body = None;
        let mut finally_body = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "block" if try_body.is_none() => try_body = Some(self.build_block(child)),
                "catch_clause" => catches.push(self.build_catch(child)),
                "else_statement" => {
                    else_body = child
                        .child_by_field_name("body")
                        .map(|b| self.build_node(b));
                }
                "finally_clause" => {
                    finally_body = child
                        .child_by_field_name("body")
                        .map(|b| self.build_node(b));
                }
                _ => {}
            }
        }
        let try_body = try_body.unwrap_or_else(|| self.alloc(node, NodeKind::Opaque));
        self.alloc(
            node,
            NodeKind::TryStmt(TryStmt {
                try_body,
                catches,
                else_body,
                finally_body,
            }),
        )
    }

    fn build_catch(&mut self, node: Node) -> NodeId {
        let mut error_types = Vec::new();
        if let Some(t) = node.child_by_field_name("type") {
            error_types.push(self.name_of(t));
        }
        let var_name = self.field_name(node, "variable");
        let body = node
            .child_by_field_name("body")
            .map(|b| self.build_block(b))
            .unwrap_or_else(|| self.alloc(node, NodeKind::Opaque));
        self.alloc(
            node,
            NodeKind::CatchClause(CatchClause {
                error_types,
                var_name,
                body,
            }),
        )
    }

    fn build_return(&mut self, node: Node) -> NodeId {
        let value = node
            .child_by_field_name("value")
            .map(|v| self.build_node(v));
        self.alloc(node, NodeKind::ReturnStmt { value })
    }

    fn build_break(&mut self, node: Node) -> NodeId {
        let label = self.field_name(node, "looplabel");
        self.alloc(node, NodeKind::BreakStmt { label })
    }

    fn build_continue(&mut self, node: Node) -> NodeId {
        let label = self.field_name(node, "looplabel");
        self.alloc(node, NodeKind::ContinueStmt { label })
    }

    fn build_throw(&mut self, node: Node) -> NodeId {
        let value = node
            .child_by_field_name("thrown")
            .map(|v| self.build_node(v));
        self.alloc(node, NodeKind::ThrowStmt { value })
    }

    fn build_goto(&mut self, node: Node) -> NodeId {
        let label = self.field_name(node, "label");
        self.alloc(node, NodeKind::GotoStmt { label })
    }

    fn build_label(&mut self, node: Node) -> NodeId {
        let name = self.field_name(node, "name");
        self.alloc(node, NodeKind::Label { name })
    }

    // -----------------------------------------------------------------
    // Blocks + AHK-specific
    // -----------------------------------------------------------------

    fn build_block(&mut self, node: Node) -> NodeId {
        let mut body = Vec::new();
        let mut pending: Vec<DirectiveComment> = Vec::new();
        let mut cursor = node.walk();

        for child in node.named_children(&mut cursor) {
            if child.kind() == "directive_comment" {
                pending.push(self.parse_directive_comment(child));
                continue;
            }
            let id = self.build_node(child);
            self.attach_directives(id, &mut pending);
            body.push(id);
        }
        self.warn_trailing(&pending);
        self.alloc(node, NodeKind::Block { body })
    }

    fn build_hotkey(&mut self, node: Node) -> NodeId {
        let trigger = node.child_by_field_name("trigger").map(Span::of);
        let body = node.child_by_field_name("body").map(|b| self.build_node(b));
        self.alloc(node, NodeKind::Hotkey { trigger, body })
    }

    fn build_hotstring(&mut self, node: Node) -> NodeId {
        let trigger = node.child_by_field_name("trigger").map(Span::of);
        let modifiers = node.child_by_field_name("modifiers").map(Span::of);
        let replacement = node.child_by_field_name("body").map(|b| self.build_node(b));
        self.alloc(
            node,
            NodeKind::Hotstring {
                modifiers,
                trigger,
                replacement,
            },
        )
    }

    fn build_hotif(&mut self, node: Node) -> NodeId {
        let expression = node
            .child_by_field_name("expression")
            .map(|e| self.build_node(e));
        self.alloc(
            node,
            NodeKind::Directive {
                kind: "hotif".to_string(),
                expression,
            },
        )
    }

    fn build_directive(&mut self, node: Node) -> NodeId {
        let kind = node
            .kind()
            .strip_suffix("_directive")
            .unwrap_or(node.kind())
            .to_string();
        self.alloc(
            node,
            NodeKind::Directive {
                kind,
                expression: None,
            },
        )
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    fn alloc(&mut self, node: Node, kind: NodeKind) -> NodeId {
        self.arena.alloc(Span::of(node), kind)
    }

    /// This grammar bakes leading whitespace into many tokens (e.g. an `identifier` may be
    /// `"    Origin"`, an `assignment_operator` `" :="`). A node's own span keeps that
    /// whitespace for faithful emission, but extracted *names* are identity, so trim them.
    fn trim_span(&self, span: Span) -> Span {
        let text = span.text(self.source);
        let lead = (text.len() - text.trim_start().len()) as u32;
        let trail = (text.len() - text.trim_end().len()) as u32;
        Span {
            start: span.start + lead,
            end: span.end - trail,
        }
    }

    /// The trimmed span of `node`, for use as a name.
    fn name_of(&self, node: Node) -> Span {
        self.trim_span(Span::of(node))
    }

    /// The trimmed span of a named field child, for use as a name.
    fn field_name(&self, node: Node, field: &str) -> Option<Span> {
        node.child_by_field_name(field).map(|n| self.name_of(n))
    }

    fn build_field_or_opaque(&mut self, node: Node, field: &str) -> NodeId {
        match node.child_by_field_name(field) {
            Some(c) => self.build_node(c),
            None => self.alloc(node, NodeKind::Opaque),
        }
    }

    fn named_child_of_kind<'t>(&self, node: Node<'t>, kind: &str) -> Option<Node<'t>> {
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let c = cursor.node();
                if c.is_named() && c.kind() == kind {
                    return Some(c);
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    }

    fn has_named_child(&self, node: Node, kind: &str) -> bool {
        self.named_child_of_kind(node, kind).is_some()
    }
}

/// Convenience for slicing a node's own text.
trait NodeTextExt {
    fn span_text<'s>(&self, source: &'s str) -> &'s str;
}

impl NodeTextExt for Node<'_> {
    fn span_text<'s>(&self, source: &'s str) -> &'s str {
        &source[self.start_byte()..self.end_byte()]
    }
}
