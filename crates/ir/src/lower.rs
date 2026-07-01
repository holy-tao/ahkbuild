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
use ahkbuild_syntax::{FileId, SourceMap, Span};

use crate::arena::{Arena, NodeId};
use crate::node::*;
use crate::program::{Group, GroupId, Program};

/// Which `#Include` directives should splice an included file's content into the IR, keyed by
/// `(including file, directive start offset within that file)` -> the included [`FileId`].
///
/// The linker resolves the include graph (IO) and fills this in before lowering; lowering then
/// pastes each named file's top-level statements into the surrounding module. A directive with
/// no entry here (an unresolved `*i` include, a deduped repeat, or a `#Include Dir`) lowers to
/// a bare [`IncludeDirective`](NodeKind::IncludeDirective) with no spliced content.
pub type IncludeSplices = HashMap<(FileId, u32), FileId>;

/// Lower a single already-parsed file into an owned [`Program`] (one entry group).
///
/// Convenience over [`Lowering`] for the single-file path (CLI `--ir`, tests).
pub fn lower(tree: &Tree, source: &str) -> Program {
    let mut lw = Lowering::new();
    lw.add_parsed("<main>", source.to_string(), tree);
    lw.finish()
}

/// Incremental, multi-file lowering into one shared [`Program`].
///
/// Each added file becomes its own [`Group`] (own module-name namespace) but shares the
/// arena and [`SourceMap`]. Files are laid out consecutively in the source map's global
/// position space, and each file's spans are offset by its base so a bare [`Span`] still
/// identifies both a file and a range. This is the lowering substrate the linker drives.
#[derive(Default)]
pub struct Lowering {
    arena: Arena,
    sources: SourceMap,
    directives: HashMap<NodeId, Vec<DirectiveComment>>,
    groups: Vec<Group>,
}

impl Lowering {
    pub fn new() -> Lowering {
        Lowering::default()
    }

    /// Add a file's `text` to the [`SourceMap`] and parse it *without* lowering. Returns the
    /// new [`FileId`] and its parse tree, or `None` if the parser yields no tree. The linker
    /// uses this to load a file (and walk it for `#Include`s) before lowering its group.
    pub fn load(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
    ) -> Option<(FileId, Tree)> {
        let file = self.sources.add(name, text);
        let tree = ahkbuild_syntax::parse(&self.sources.file(file).text)?;
        Some((file, tree))
    }

    /// Lower an already-loaded `file` (see [`load`](Self::load)) as a new group, splicing in
    /// each `#Include`d file named by `splices`. The included files must already be in the
    /// source map. Returns `None` if `file` fails to parse.
    pub fn lower_group(&mut self, file: FileId, splices: &IncludeSplices) -> Option<GroupId> {
        let base = self.sources.base(file);
        let tree = ahkbuild_syntax::parse(&self.sources.file(file).text)?;
        let modules = {
            let src: &str = &self.sources.file(file).text;
            lower_group_into(
                &mut self.arena,
                &mut self.directives,
                &self.sources,
                splices,
                file,
                base,
                src,
                tree.root_node(),
            )
        };
        let id = GroupId(self.groups.len() as u32);
        self.groups.push(Group { id, file, modules });
        Some(id)
    }

    /// Parse `text` and lower it as a new group with no `#Include` resolution. Returns `None`
    /// if the parser yields no tree. The text is moved into the [`SourceMap`].
    pub fn add_file(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
    ) -> Option<GroupId> {
        let file = self.sources.add(name, text);
        self.lower_group(file, &IncludeSplices::new())
    }

    /// Lower a file whose `tree` is already parsed (from the same bytes as `text`), as a new
    /// group with no `#Include` resolution. The text is moved into the [`SourceMap`].
    pub fn add_parsed(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
        tree: &Tree,
    ) -> GroupId {
        let file = self.sources.add(name, text);
        let base = self.sources.base(file);
        let empty = IncludeSplices::new();
        let modules = {
            let src: &str = &self.sources.file(file).text;
            lower_group_into(
                &mut self.arena,
                &mut self.directives,
                &self.sources,
                &empty,
                file,
                base,
                src,
                tree.root_node(),
            )
        };
        let id = GroupId(self.groups.len() as u32);
        self.groups.push(Group { id, file, modules });
        id
    }

    pub fn finish(self) -> Program {
        Program {
            groups: self.groups,
            arena: self.arena,
            sources: self.sources,
            directives: self.directives,
        }
    }

    /// The module names defined in a group (as written). The linker uses these to tell an
    /// in-group sub-module reference (`#Import Helper` where `#Module Helper` lives in the
    /// same file) from a filesystem import — only the latter names a file.
    pub fn group_module_names(&self, gid: GroupId) -> Vec<String> {
        let Some(group) = self.groups.get(gid.0 as usize) else {
            return Vec::new();
        };
        group
            .modules
            .iter()
            .filter_map(|&m| match &self.arena[m].kind {
                NodeKind::Module(module) => Some(module.name.clone()),
                _ => None,
            })
            .collect()
    }

    /// The `#Import` targets declared by a group's modules, in source order. The linker
    /// uses this to discover and resolve the next files to load.
    pub fn group_imports(&self, gid: GroupId) -> Vec<ImportSpec> {
        let mut out = Vec::new();
        let Some(group) = self.groups.get(gid.0 as usize) else {
            return out;
        };
        for &m in &group.modules {
            let NodeKind::Module(module) = &self.arena[m].kind else {
                continue;
            };
            for &stmt in &module.body {
                if let NodeKind::ImportDirective(d) = &self.arena[stmt].kind {
                    let (quoted, span) = match &d.source {
                        ImportSource::Name(s) => (false, *s),
                        ImportSource::Path(s) => (true, *s),
                    };
                    let raw = self.sources.text(span);
                    let spec = if quoted {
                        unquote(raw)
                    } else {
                        raw.trim().to_string()
                    };
                    out.push(ImportSpec {
                        quoted,
                        spec,
                        node: stmt,
                    });
                }
            }
        }
        out
    }

    /// The `IncludeDirective` nodes in a group's module bodies, in source order (a group's
    /// own directives plus those of any files spliced into it). The linker joins these against
    /// its include resolution to build the backend-neutral `resolved_includes`.
    pub fn group_includes(&self, gid: GroupId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let Some(group) = self.groups.get(gid.0 as usize) else {
            return out;
        };
        for &m in &group.modules {
            let NodeKind::Module(module) = &self.arena[m].kind else {
                continue;
            };
            for &stmt in &module.body {
                if matches!(self.arena[stmt].kind, NodeKind::IncludeDirective(_)) {
                    out.push(stmt);
                }
            }
        }
        out
    }
}

/// A single `#Import` target found in a group, for the linker to resolve.
#[derive(Clone, Debug)]
pub struct ImportSpec {
    /// `true` if written as a quoted string (`#Import "X"`). Quoted imports don't bind a
    /// name and also cover the embedded (`*RESNAME`) and path-qualified (`Path:Module`)
    /// forms.
    pub quoted: bool,
    /// The target text: a bare module name, a path, `*RESNAME`, or `Path:Module`.
    pub spec: String,
    /// The `ImportDirective` node, for later import rewriting.
    pub node: NodeId,
}

/// Strip one layer of matching surrounding quotes from a quoted import's text.
fn unquote(s: &str) -> String {
    let t = s.trim();
    let b = t.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

/// Lower one group's entry tree, appending nodes to a shared arena and splicing in any
/// `#Include`d files named by `splices`; returns the group's modules. A free function (not a
/// `&mut self` method) so it can borrow the [`Lowering`] fields as disjoint pieces while
/// `source`/`sources` still borrow the source map.
#[allow(clippy::too_many_arguments)]
fn lower_group_into<'a>(
    arena: &'a mut Arena,
    directives: &'a mut HashMap<NodeId, Vec<DirectiveComment>>,
    sources: &'a SourceMap,
    splices: &'a IncludeSplices,
    file: FileId,
    base: u32,
    source: &'a str,
    root: Node,
) -> Vec<NodeId> {
    let mut l = Lowerer {
        arena,
        directives,
        sources,
        splices,
        file,
        base,
        source,
    };
    let (modules, _main) = l.build_top_level(root);
    modules
}

struct Lowerer<'a> {
    arena: &'a mut Arena,
    directives: &'a mut HashMap<NodeId, Vec<DirectiveComment>>,
    /// Every source behind the program, for slicing files spliced in via `#Include`.
    sources: &'a SourceMap,
    /// Which `#Include` directives to splice (see [`IncludeSplices`]).
    splices: &'a IncludeSplices,
    /// The file currently being walked — swapped while splicing an included file so spliced
    /// nodes carry spans into *their* file.
    file: FileId,
    /// `file`'s start offset in the [`SourceMap`] global position space.
    base: u32,
    /// `file`'s own text (file-relative). Combined with `base` to slice global spans.
    source: &'a str,
}

/// Shared state threaded through [`Lowerer::walk_top_level`] so an `#Include`d file's
/// statements (and any `#Module` it opens) join the includer's modules, as a paste would.
struct TopLevel {
    modules: Vec<NodeId>,
    by_name: HashMap<String, NodeId>,
    current: NodeId,
    pending: Vec<DirectiveComment>,
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
        let main_span = self.span(root);
        let main = self.arena.alloc(
            main_span,
            NodeKind::Module(Module {
                name: Module::MAIN.to_string(),
                name_span: None,
                body: Vec::new(),
            }),
        );
        let mut st = TopLevel {
            modules: vec![main],
            by_name: HashMap::from([(Module::MAIN.to_ascii_lowercase(), main)]),
            current: main,
            pending: Vec::new(),
        };
        self.walk_top_level(root, &mut st);
        self.warn_trailing(&st.pending);
        (st.modules, main)
    }

    /// Walk one file's top-level statements into the shared module-grouping `st`. Called
    /// re-entrantly by [`Self::maybe_splice`] for `#Include`d files.
    fn walk_top_level(&mut self, root: Node, st: &mut TopLevel) {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                "directive_comment" => {
                    st.pending.push(self.parse_directive_comment(child));
                }
                "module_directive" => self.open_module(child, st),
                _ => {
                    let id = self.build_node(child);
                    self.attach_directives(id, &mut st.pending);
                    self.push_to_module(st.current, id);
                    if matches!(
                        child.kind(),
                        "include_directive" | "include_again_directive"
                    ) {
                        self.maybe_splice(child, st);
                    }
                }
            }
        }
    }

    /// Open (or reopen) the module named by a `#Module` directive, updating `st.current`.
    fn open_module(&mut self, child: Node, st: &mut TopLevel) {
        let ident = child.named_child(0);
        let name = ident
            .map(|n| n.span_text(self.source).trim().to_string())
            .unwrap_or_default();
        let key = name.to_ascii_lowercase();
        st.current = if let Some(&existing) = st.by_name.get(&key) {
            existing
        } else {
            let child_span = self.span(child);
            let id = self.arena.alloc(
                child_span,
                NodeKind::Module(Module {
                    name,
                    name_span: ident.map(|n| self.name_of(n)),
                    body: Vec::new(),
                }),
            );
            st.modules.push(id);
            st.by_name.insert(key, id);
            id
        };
    }

    /// If `directive` (an `#Include`/`#IncludeAgain` node in the current file) is marked to
    /// splice, paste the included file's top-level statements into `st`, switching the file
    /// context for the duration so spliced nodes carry spans into the included file.
    fn maybe_splice(&mut self, directive: Node, st: &mut TopLevel) {
        let off = directive.start_byte() as u32;
        let Some(&inc) = self.splices.get(&(self.file, off)) else {
            return;
        };
        let sources = self.sources;
        let text = sources.file(inc).text.as_str();
        let base = sources.base(inc);
        let Some(tree) = ahkbuild_syntax::parse(text) else {
            return;
        };
        let (prev_file, prev_base, prev_source) = (self.file, self.base, self.source);
        self.file = inc;
        self.base = base;
        self.source = text;
        self.walk_top_level(tree.root_node(), st);
        self.file = prev_file;
        self.base = prev_base;
        self.source = prev_source;
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
                .map(|n| self.span(n))
                .unwrap_or_else(|| self.span(node)),
            arguments: node.child_by_field_name("arguments").map(|n| self.span(n)),
        }
    }

    fn attach_directives(&mut self, id: NodeId, pending: &mut Vec<DirectiveComment>) {
        if !pending.is_empty() {
            self.directives.entry(id).or_default().append(pending);
        }
    }

    fn warn_trailing(&self, pending: &[DirectiveComment]) {
        for d in pending {
            tracing::warn!(
                directive = self.slice(d.name),
                "directive `;@<name>` has no following statement",
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
            "include_directive" | "include_again_directive" => self.build_include(node),

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

            // Comments
            "block_comment" | "line_comment" => self.build_comment(node),

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
            match self.build_param(child) {
                Some(param) => out.push(param),
                None => {
                    self.lower_stray_comment(child);
                }
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
                "block_comment" | "line_comment" => {
                    let cm = self.build_comment(child);
                    decl.nested.push(cm);
                    cm
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
                _ => {
                    self.lower_stray_comment(child);
                }
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
        let initializer = self.build_field_expr(node, "value");
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
                    _ => {
                        self.lower_stray_comment(child);
                    }
                }
            }
            ImportBinding::Selective { wildcard, names }
        } else {
            ImportBinding::Whole
        };

        // `#Import export X` — the grammar exposes the re-export modifier as an `export` child.
        let reexport = self.has_named_child(node, "export");

        self.alloc(
            node,
            NodeKind::ImportDirective(ImportDirective {
                source,
                binding,
                reexport,
            }),
        )
    }

    fn build_include(&mut self, node: Node) -> NodeId {
        let again = node.kind() == "include_again_directive";
        let ignore_missing = self.has_named_child(node, "include_ignore_failure");
        let (path, is_lib) = if let Some(p) = self.named_child_of_kind(node, "lib_name") {
            (self.name_of(p), true)
        } else if let Some(p) = self.named_child_of_kind(node, "file_or_dir_name") {
            (self.name_of(p), false)
        } else {
            (self.name_of(node), false)
        };
        self.alloc(
            node,
            NodeKind::IncludeDirective(Include {
                path,
                again,
                ignore_missing,
                is_lib,
            }),
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
        let left = self.build_field_expr(node, "left");
        let right = self.build_field_expr(node, "right");

        let op = if is_assignment {
            self.named_child_of_kind(node, "assignment_operator")
                .map(|n| self.span(n))
        } else {
            node.child_by_field_name("operator").map(|n| self.span(n))
        };

        let (left, right) = (
            left.unwrap_or_else(|| self.alloc(node, NodeKind::Opaque)),
            right.unwrap_or_else(|| self.alloc(node, NodeKind::Opaque)),
        );
        // Concat operations (`explicit_concat_operation` / `implicit_concat_operation`) carry no
        // `operator` field, so fall back to the gap between operands. A parenthesized operand
        // surfaces its *inner* node, leaving the wrapping `(`/`)` inside that gap (e.g.
        // `"x" . (y)` -> gap `" . ("`); trim whitespace and parens so the span lands on the bare
        // operator (`.`) - or an empty span for implicit concat, which folds on an empty op.
        let op = op.unwrap_or_else(|| {
            self.trim_operator_gap(self.arena[left].span.end, self.arena[right].span.start)
        });

        self.alloc(node, NodeKind::BinaryExpr { left, op, right })
    }

    fn build_unary(&mut self, node: Node, prefix: bool) -> NodeId {
        let op = node
            .child_by_field_name("operator")
            .map(|n| self.span(n))
            .unwrap_or_else(|| self.span(node));
        let operand = self
            .build_field_expr(node, "operand")
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
        let callee = self
            .build_field_expr(node, "function")
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
                        let value = self.build_field_expr(child, "value");
                        out.push(ObjectMember { key, value });
                    }
                }
                "object_literal_member_sequence" => self.collect_object_members(child, out),
                _ => {
                    self.lower_stray_comment(child);
                }
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
        // A single-expression sequence is just that expression. A multi-expression sequence
        // (comma operator) keeps each sub-expression as a child so references inside them -
        // e.g. the imported names in an array literal `[A, B]` - stay visible to analyses.
        if node.named_child_count() == 1 {
            if let Some(child) = node.named_child(0) {
                return self.build_node(child);
            }
        }
        let mut exprs = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            exprs.push(self.build_node(child));
        }
        self.alloc(node, NodeKind::ExpressionSequence { exprs })
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
                        _ => {
                            self.lower_stray_comment(child);
                        }
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
            .and_then(|u| self.field_node(u, "condition"))
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
        let discriminant = self.build_field_expr(node, "head");
        let mut cases = Vec::new();
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.named_children(&mut cursor) {
                match child.kind() {
                    "case_clause" => cases.push(self.build_case(child, false)),
                    "default_clause" => cases.push(self.build_case(child, true)),
                    _ => {
                        self.lower_stray_comment(child);
                    }
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
                        // A comment in a case clause has no field name; give it a home in the
                        // body so it is lowered (and stripped) like any other case statement.
                        _ if matches!(child.kind(), "line_comment" | "block_comment") => {
                            body.push(self.build_comment(child));
                        }
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
        let try_body = match node.child_by_field_name("body") {
            Some(body_node) => self.build_node(body_node),
            None => self.alloc(node, NodeKind::Opaque),
        };
        let mut catches = Vec::new();
        let mut else_body = None;
        let mut finally_body = None;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
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
                _ => {
                    self.lower_stray_comment(child);
                }
            }
        }

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
        let value = self.build_field_expr(node, "value");
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
        let value = self.build_field_expr(node, "thrown");
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

    fn build_comment(&mut self, node: Node) -> NodeId {
        self.alloc(node, NodeKind::Comment)
    }

    /// Lower a comment that lands in a structural slot with no place for it in the IR.
    ///
    /// Comments are grammar *extras*, so they can appear between the specific children that
    /// these builders recognise (param/import-name lists, object literals, `switch`/`try`
    /// bodies, property blocks, `for` headers). We still allocate a [`NodeKind::Comment`] so the
    /// arena-wide comment-strip pass can delete its span, but there is no parent to hang it on,
    /// so it stays unparented: invisible to the root-reachable walks (`print`, `children`,
    /// tree-shaking) and seen only by the strip pass, which iterates the whole arena. Non-comment
    /// children are left alone. Returns whether `node` was a comment.
    fn lower_stray_comment(&mut self, node: Node) -> bool {
        let is_comment = matches!(node.kind(), "line_comment" | "block_comment");
        if is_comment {
            self.build_comment(node);
        }
        is_comment
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
        let trigger = node.child_by_field_name("trigger").map(|n| self.span(n));
        let body = node.child_by_field_name("body").map(|b| self.build_node(b));
        self.alloc(node, NodeKind::Hotkey { trigger, body })
    }

    fn build_hotstring(&mut self, node: Node) -> NodeId {
        let trigger = node.child_by_field_name("trigger").map(|n| self.span(n));
        let modifiers = node.child_by_field_name("modifiers").map(|n| self.span(n));
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
        let expression = self.build_field_expr(node, "expression");
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
        let span = self.span(node);
        self.arena.alloc(span, kind)
    }

    /// This file's [`Span`] for a tree-sitter node, in the source map's global position
    /// space (tree-sitter byte offsets are file-relative, so add this file's `base`).
    fn span(&self, node: Node) -> Span {
        Span {
            start: node.start_byte() as u32 + self.base,
            end: node.end_byte() as u32 + self.base,
        }
    }

    /// Trim the (absolute) span `[start, end)` to the operator it brackets: drop leading and
    /// trailing whitespace and parentheses. Used for the fieldless concat operators, where a
    /// parenthesized operand otherwise leaks its `(`/`)` into the gap span. An all-whitespace
    /// gap (implicit concat) collapses to an empty span at the trimmed start.
    fn trim_operator_gap(&self, start: u32, end: u32) -> Span {
        let (lo, hi) = ((start - self.base) as usize, (end - self.base) as usize);
        let gap = &self.source[lo..hi];
        let is_pad = |c: char| c.is_whitespace() || c == '(' || c == ')';
        let lead = gap.len() - gap.trim_start_matches(is_pad).len();
        let trimmed = gap.trim_matches(is_pad);
        let op_start = start + lead as u32;
        Span {
            start: op_start,
            end: op_start + trimmed.len() as u32,
        }
    }

    /// Slice this file's own text for one of its (global) spans.
    fn slice(&self, span: Span) -> &str {
        &self.source[(span.start - self.base) as usize..(span.end - self.base) as usize]
    }

    /// This grammar bakes leading whitespace into many tokens (e.g. an `identifier` may be
    /// `"    Origin"`, an `assignment_operator` `" :="`). A node's own span keeps that
    /// whitespace for faithful emission, but extracted *names* are identity, so trim them.
    fn trim_span(&self, span: Span) -> Span {
        let text = self.slice(span);
        let lead = (text.len() - text.trim_start().len()) as u32;
        let trail = (text.len() - text.trim_end().len()) as u32;
        Span {
            start: span.start + lead,
            end: span.end - trail,
        }
    }

    /// The trimmed span of `node`, for use as a name.
    fn name_of(&self, node: Node) -> Span {
        self.trim_span(self.span(node))
    }

    /// The trimmed span of a named field child, for use as a name.
    fn field_name(&self, node: Node, field: &str) -> Option<Span> {
        node.child_by_field_name(field).map(|n| self.name_of(n))
    }

    /// The *named* node for a field. The grammar's `_parenthesized_expression` is hidden
    /// (`seq("(", expression_sequence, ")")`), so for a parenthesized field value
    /// `child_by_field_name` can return the anonymous `(` token — which would lower to
    /// `Opaque` and hide every reference inside the parens. Picking the first named child
    /// carrying the field surfaces the real `expression_sequence` instead.
    fn field_node<'t>(&self, node: Node<'t>, field: &str) -> Option<Node<'t>> {
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let c = cursor.node();
                if cursor.field_name() == Some(field) && c.is_named() {
                    return Some(c);
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        node.child_by_field_name(field)
    }

    /// Build the expression at `field`, unwrapping a parenthesized value (see [`field_node`]).
    fn build_field_expr(&mut self, node: Node, field: &str) -> Option<NodeId> {
        self.field_node(node, field).map(|c| self.build_node(c))
    }

    fn build_field_or_opaque(&mut self, node: Node, field: &str) -> NodeId {
        match self.field_node(node, field) {
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
