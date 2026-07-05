//! The top-level lowered program: an arena of nodes, the module *groups* within it, the
//! physical sources (for span slicing), and side tables that hang off [`NodeId`]s.

use std::collections::HashMap;

use ahkbuild_syntax::{FileId, SourceMap, Span};

use crate::arena::{Arena, NodeId};
use crate::node::DirectiveComment;

/// A handle to a [`Group`] in a [`Program`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GroupId(pub u32);

/// A module-name group: the modules that share one origin (the main script, or a single
/// imported / embedded file). Module identity is **`(GroupId, name)`**, not bare `name` —
/// runtime probes (`tests/fixtures/probes/runtime/`) confirm that same-named `#Module`
/// blocks in two different groups stay isolated, whereas same-named blocks *within* one
/// group reopen/merge.
///
/// A group is distinct from a [`SourceFile`](ahkbuild_syntax::SourceFile): one group may be
/// assembled from several `#Include`d files, and one file may (with dedup) feed several
/// groups. `file` records the group's entry/primary file.
#[derive(Clone, Debug)]
pub struct Group {
    pub id: GroupId,
    /// The file this group's primary module was loaded from.
    pub file: FileId,
    /// Distinct modules in this group, in first-appearance order, deduped by name *within
    /// the group*. The entry group's first module is the implicit `__Main`.
    pub modules: Vec<NodeId>,
}

/// A fully lowered program.
pub struct Program {
    /// Module-name groups. `groups[0]` is the entry group (the main script / `__Main`).
    /// Today lowering produces exactly one group; the linker will add one per imported
    /// file/resource.
    pub groups: Vec<Group>,
    /// Backing storage for every IR node.
    pub arena: Arena,
    /// Every physical source behind the program, in one global position space. Slice it
    /// with the `Span`s stored on nodes.
    pub sources: SourceMap,
    /// `;@Name` directive comments, keyed by the statement node they precede.
    pub directives: HashMap<NodeId, Vec<DirectiveComment>>,
}

impl Program {
    /// The entry group (main script).
    pub fn main_group(&self) -> &Group {
        &self.groups[0]
    }

    /// Every module in the program, in group then first-appearance order.
    pub fn modules(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.groups.iter().flat_map(|g| g.modules.iter().copied())
    }

    /// Slice the program's source for a span's text.
    pub fn span_text(&self, span: Span) -> &str {
        self.sources.text(span)
    }

    /// Slice the program's source for a node's span text.
    pub fn text(&self, id: NodeId) -> &str {
        self.span_text(self.arena[id].span)
    }

    /// A human-readable `file:line` for a span's start byte, for diagnostics/tracing. The line
    /// is 1-based within the span's originating file.
    pub fn location(&self, span: Span) -> String {
        let file = self.sources.file_at(span.start);
        let offset = (span.start - file.base) as usize;
        let line = file.text[..offset.min(file.text.len())]
            .bytes()
            .filter(|&b| b == b'\n')
            .count()
            + 1;
        format!("{}:{}", file.name, line)
    }

    /// A `file:line` for a node's span (see [`Program::location`]).
    pub fn node_location(&self, id: NodeId) -> String {
        self.location(self.arena[id].span)
    }

    /// Whether statement/member `node` carries a `;@`-directive named `name` (case-insensitive).
    pub fn has_directive(&self, node: NodeId, name: &str) -> bool {
        self.directives.get(&node).is_some_and(|ds| {
            ds.iter()
                .any(|d| directive_name(self.span_text(d.name)).eq_ignore_ascii_case(name))
        })
    }

    /// The argument text of `node`'s `;@Name` directive (empty string when the directive is
    /// present with no arguments), or `None` when `node` carries no such directive.
    pub fn directive_arg(&self, node: NodeId, name: &str) -> Option<&str> {
        let ds = self.directives.get(&node)?;
        let d = ds
            .iter()
            .find(|d| directive_name(self.span_text(d.name)).eq_ignore_ascii_case(name))?;
        Some(d.arguments.map(|s| self.span_text(s)).unwrap_or(""))
    }
}

/// Strip a leading `@`/`;` decoration from a directive name (`;@Name` lowers its `directive`
/// field to either `Name` or `@Name` depending on the grammar).
fn directive_name(raw: &str) -> &str {
    raw.trim().trim_start_matches(['@', ';']).trim()
}
