//! The IR node taxonomy.
//!
//! [`NodeKind`] is one big sum type covering every construct the bundler analyses. Note
//! that unlike the previous implementation, analysis fields like `callCount` live in side
//! tables instead of on nodes themselves.

use ahkbuild_syntax::Span;

use crate::arena::NodeId;

/// The kind-specific payload of an IR node. The node's own [`Span`] lives on
/// [`crate::arena::Node`].
#[derive(Clone, Debug)]
pub enum NodeKind {
    // --- Structural ---
    /// A `{ ... }` block of statements.
    Block {
        body: Vec<NodeId>,
    },
    /// An expression used as a statement.
    ExpressionStatement {
        expr: NodeId,
    },
    /// A node the IR doesn't analyse; emitted verbatim from its span.
    Opaque,

    // --- Modules (v2.1) ---
    /// A module: the implicit `__Main` or a `#Module Name` block. May aggregate several
    /// reopened textual blocks of the same name. See [`Module`] notes on `name`/`name_span`.
    Module(Module),
    /// An `#Import` directive.
    ImportDirective(ImportDirective),
    /// A `#Include` / `#IncludeAgain` directive. Unlike `#Import`, an include pastes another
    /// file's text into the *current* module; the directive node stays in the body at its
    /// source position so a backend can decide how to materialize it (the `.ahk` emitter
    /// splices the included file's text over this span; a future `.exe` emitter keeps the
    /// directive and embeds the file as a resource). The spliced content, when resolved, is
    /// lowered into the surrounding module right after this node.
    IncludeDirective(Include),
    /// An `export` / `export default` declaration wrapping an inner declaration.
    ExportDecl {
        default: bool,
        decl: NodeId,
    },

    // --- Declarations ---
    Function(Function),
    /// A `class` declaration; owns members.
    ClassDecl(TypeDecl),
    /// A v2.1 `struct` declaration; like a class, can own methods/properties/fields.
    StructDecl(TypeDecl),
    Property(Property),
    /// A simple `name := value` member of a class/struct body.
    Field(Field),
    /// A v2.1 typed field: `name: Type := value` (currently only in `struct` bodies).
    TypedProperty(TypedProperty),
    /// A v2.1 `: Type` annotation; wraps the type expression. Child of [`TypedProperty`].
    TypeSpecifier {
        type_expr: NodeId,
    },
    /// An explicit `local`/`global`/`static` variable declaration.
    VarDecl(VarDecl),

    // --- Expressions ---
    /// Any binary operation; the specific operator is the `op` span's text.
    BinaryExpr {
        left: NodeId,
        op: Span,
        right: NodeId,
    },
    UnaryExpr {
        op: Span,
        operand: NodeId,
        prefix: bool,
    },
    TernaryExpr {
        condition: NodeId,
        then_branch: NodeId,
        else_branch: NodeId,
    },
    CallExpr(CallExpr),
    Literal {
        kind: LiteralKind,
    },
    /// A plain identifier (name = the node's span text).
    Identifier,
    /// A dynamic identifier such as `%expr%` or `pre%expr%post`.
    DynamicIdentifier {
        parts: Vec<NodeId>,
    },
    MemberAccess {
        object: NodeId,
        member: NodeId,
        is_dynamic: bool,
    },
    IndexAccess {
        object: NodeId,
        args: Vec<NodeId>,
    },
    ArrayLiteral {
        elements: Vec<NodeId>,
    },
    ObjectLiteral {
        members: Vec<ObjectMember>,
    },
    /// A `%expr%` dereference.
    DerefExpr {
        inner: NodeId,
    },
    /// A `&var` variable reference.
    VarRefExpr {
        operand: NodeId,
    },
    /// An anonymous `(params) => expr` arrow function.
    FatArrow {
        params: Vec<Param>,
        body: NodeId,
    },

    // --- Control flow ---
    IfStmt {
        condition: NodeId,
        then_body: NodeId,
        else_body: Option<NodeId>,
    },
    WhileStmt {
        condition: NodeId,
        body: NodeId,
    },
    ForStmt(ForStmt),
    LoopStmt(LoopStmt),
    SwitchStmt {
        discriminant: Option<NodeId>,
        cases: Vec<NodeId>,
    },
    CaseClause {
        values: Vec<NodeId>,
        body: Vec<NodeId>,
        is_default: bool,
    },
    TryStmt(TryStmt),
    CatchClause(CatchClause),
    ReturnStmt {
        value: Option<NodeId>,
    },
    BreakStmt {
        label: Option<Span>,
    },
    ContinueStmt {
        label: Option<Span>,
    },
    ThrowStmt {
        value: Option<NodeId>,
    },
    GotoStmt {
        label: Option<Span>,
    },

    // --- AHK-specific ---
    Hotkey {
        trigger: Option<Span>,
        body: Option<NodeId>,
    },
    Hotstring {
        modifiers: Option<Span>,
        trigger: Option<Span>,
        replacement: Option<NodeId>,
    },
    /// A surviving runtime directive (`#Requires`, `#Warn`, `#HotIf`, …). `expression` is
    /// set for `#HotIf`, whose condition is a tree-shaking entry point.
    Directive {
        kind: String,
        expression: Option<NodeId>,
    },
    Label {
        name: Option<Span>,
    },
    /// A line or block comment. Tracked in the IR so that we can drop them easily
    Comment,
}

/// A module. `name` is the case-insensitive identity key *within its [`Group`]* (implicit
/// module = `"__Main"`), so an explicit `#Module __Main` merges into the implicit one, but
/// same-named modules in *different* groups stay distinct (see [`Group`]). `name_span` is
/// `None` for the implicit module and `Some` for an explicit `#Module` directive (for
/// emission). The node's own span anchors the first `#Module` directive, not a covering
/// range.
///
/// [`Group`]: crate::program::Group
#[derive(Clone, Debug)]
pub struct Module {
    pub name: String,
    pub name_span: Option<Span>,
    pub body: Vec<NodeId>,
}

impl Module {
    /// The implicit-module identity name.
    pub const MAIN: &'static str = "__Main";

    pub fn is_main(&self) -> bool {
        self.name.eq_ignore_ascii_case(Module::MAIN)
    }
}

/// `#Import` directive payload.
#[derive(Clone, Debug)]
pub struct ImportDirective {
    pub source: ImportSource,
    pub binding: ImportBinding,
    /// `#Import export X` — a re-export (barrel): the imported names also become exports of
    /// the importing module, so they are part of its public surface and must not be trimmed.
    pub reexport: bool,
}

/// `#Include` / `#IncludeAgain` directive payload. The raw `path` span (which may keep its
/// surrounding quotes and, for `is_lib`, its `<...>` brackets) is what a backend rewrites;
/// the linker resolves it against the filesystem.
#[derive(Clone, Debug)]
pub struct Include {
    /// The `file_or_dir_name` / `lib_name` text span (verbatim, including any quotes/brackets).
    pub path: Span,
    /// `#IncludeAgain` — paste even if the file was already included in this module.
    pub again: bool,
    /// The `*i` flag — a failure to resolve the file is non-fatal.
    pub ignore_missing: bool,
    /// `#Include <LibName>` library-include form (vs. a plain file/dir path).
    pub is_lib: bool,
}

/// What an `#Import` resolves against: a bare module name or a quoted path/spec.
#[derive(Clone, Debug)]
pub enum ImportSource {
    Name(Span),
    Path(Span),
}

/// How imported names are bound into the importing module.
#[derive(Clone, Debug)]
pub enum ImportBinding {
    /// `#Import X` — whole module under its own name.
    Whole,
    /// `#Import X as Z`.
    Alias(Span),
    /// `#Import X {a, b as c}`, `{*}`, or `{*, Extra}`.
    Selective {
        wildcard: bool,
        names: Vec<ImportName>,
    },
}

/// One name in a selective import: `Name` or `Name as Alias`.
#[derive(Clone, Debug)]
pub struct ImportName {
    pub name: Span,
    pub alias: Option<Span>,
}

/// Shared body of a `class` or `struct` declaration.
#[derive(Clone, Debug, Default)]
pub struct TypeDecl {
    pub name: Option<Span>,
    /// Superclass / base name (`Base.Inner` keeps its dotted span).
    pub superclass: Option<Span>,
    pub methods: Vec<NodeId>,
    pub properties: Vec<NodeId>,
    pub static_fields: Vec<NodeId>,
    pub instance_fields: Vec<NodeId>,
    /// v2.1 typed fields (`name: Type := value`) — `TypedProperty` nodes, mostly in structs.
    pub typed_fields: Vec<NodeId>,
    /// Nested `ClassDecl` / `StructDecl` nodes.
    pub nested: Vec<NodeId>,
}

/// A function, method, getter/setter body, or anonymous function expression.
#[derive(Clone, Debug)]
pub struct Function {
    pub name: Option<Span>,
    pub params: Vec<Param>,
    /// Block (statement body) or an expression (arrow body). `None` for declarations with
    /// no body, e.g. abstract-looking forms.
    pub body: Option<NodeId>,
    pub is_static: bool,
    pub is_method: bool,
    pub is_variadic: bool,
    /// `=>` single-expression body.
    pub is_arrow: bool,
    /// Anonymous `function_expression` (vs. a named declaration).
    pub is_expression: bool,
    /// Owning `ClassDecl` or `StructDecl`, if this is a method/accessor.
    pub owner: Option<NodeId>,
}

/// A single function/method parameter. Stored inline (never referenced polymorphically);
/// its `default` expression still points into the arena.
#[derive(Clone, Debug)]
pub struct Param {
    pub name: Option<Span>,
    pub default: Option<NodeId>,
    pub by_ref: bool,
    pub variadic: bool,
    pub optional: bool,
}

/// A property with a getter and/or setter (or shorthand `Prop => expr`).
#[derive(Clone, Debug)]
pub struct Property {
    pub name: Option<Span>,
    pub is_static: bool,
    pub getter: Option<NodeId>,
    pub setter: Option<NodeId>,
    pub is_getter_only: bool,
    pub is_arrow_getter: bool,
    pub is_arrow_setter: bool,
}

/// A `name := value` field in a class/struct body.
#[derive(Clone, Debug)]
pub struct Field {
    pub name: Option<Span>,
    pub is_static: bool,
    pub initializer: Option<NodeId>,
}

/// A v2.1 typed field: `name: Type := value`. The `: Type` is held as a `TypeSpecifier`
/// node (`type_spec`); `initializer` is the optional `:= value`.
#[derive(Clone, Debug)]
pub struct TypedProperty {
    pub name: Option<Span>,
    pub type_spec: NodeId,
    pub initializer: Option<NodeId>,
}

/// An explicit variable declaration.
#[derive(Clone, Debug)]
pub struct VarDecl {
    pub name: Option<Span>,
    pub scope: VarScope,
    pub initializer: Option<NodeId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarScope {
    Local,
    Global,
    Static,
}

/// A function/method call.
#[derive(Clone, Debug)]
pub struct CallExpr {
    pub callee: NodeId,
    pub args: Vec<NodeId>,
    /// Command-style call (no parentheses).
    pub is_command_style: bool,
    /// Callee is a deref/dynamic, so the target is un-analysable.
    pub is_dynamic: bool,
}

/// One `key: value` (or shorthand) member of an object literal.
#[derive(Clone, Debug)]
pub struct ObjectMember {
    pub key: NodeId,
    pub value: Option<NodeId>,
}

#[derive(Clone, Debug)]
pub struct ForStmt {
    pub iterators: Vec<NodeId>,
    pub iterable: Option<NodeId>,
    pub body: Option<NodeId>,
    pub else_body: Option<NodeId>,
}

#[derive(Clone, Debug)]
pub struct LoopStmt {
    pub kind: LoopKind,
    pub head: Option<NodeId>,
    pub body: Option<NodeId>,
    pub until: Option<NodeId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopKind {
    Infinite,
    Count,
    Parse,
    Read,
    Reg,
    Files,
}

#[derive(Clone, Debug)]
pub struct TryStmt {
    pub try_body: NodeId,
    pub catches: Vec<NodeId>,
    pub else_body: Option<NodeId>,
    pub finally_body: Option<NodeId>,
}

#[derive(Clone, Debug)]
pub struct CatchClause {
    /// Error class names to catch (raw spans — looked up by name later).
    pub error_types: Vec<Span>,
    pub var_name: Option<Span>,
    pub body: NodeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiteralKind {
    Integer,
    Float,
    String,
    Boolean,
}

/// A parsed `;@Name args` directive comment, attached to the following statement via a
/// side table on [`crate::Program`] rather than living in the IR tree.
#[derive(Clone, Debug)]
pub struct DirectiveComment {
    pub name: Span,
    pub arguments: Option<Span>,
}
