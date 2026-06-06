//! The top-level lowered program: an arena of nodes, the modules within it, the original
//! source (for span slicing), and side tables that hang off [`NodeId`]s.

use std::collections::HashMap;

use crate::arena::{Arena, NodeId};
use crate::node::DirectiveComment;

/// A fully lowered program.
///
/// `modules` lists the *distinct* modules (deduped by name): the implicit `__Main` plus one
/// entry per `#Module` name, each a [`crate::node::NodeKind::Module`] node in `arena`.
pub struct Program {
    /// Distinct modules, in first-appearance order. The implicit `__Main` is first.
    pub modules: Vec<NodeId>,
    /// Backing storage for every IR node.
    pub arena: Arena,
    /// The original source, owned so the IR can outlive the parse tree. Slice it with the
    /// `Span`s stored on nodes.
    pub source: String,
    /// `;@Name` directive comments, keyed by the statement node they precede.
    pub directives: HashMap<NodeId, Vec<DirectiveComment>>,
}

impl Program {
    /// Slice this program's source for a node's span text.
    pub fn text(&self, id: NodeId) -> &str {
        self.arena[id].span.text(&self.source)
    }
}
