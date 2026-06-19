//! Arena storage for IR nodes.
//!
//! IR nodes live in a flat `Vec` and are referenced by integer [`NodeId`] rather than
//! by pointers (`Box`/`Rc`/`RefCell`) or parent links. This is a deliberate departure
//! from the AHK reference IR (`build/ir.ahk`), whose `parent`/`children`/`resolvedSymbol`
//! reference graph does not translate to Rust ownership.

use std::ops::Index;

use ahkbuild_syntax::Span;

use crate::node::NodeKind;

/// A handle to a node stored in an [`Arena`].
///
/// `NodeId`s are stable for the lifetime of the arena and cheap to copy/store. They are
/// the *only* way IR nodes reference one another.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub u32);

impl NodeId {
    /// The underlying index, for use as a side-table key.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One IR node: its syntactic [`Span`] plus its [`NodeKind`].
///
/// Analysis results never live here — they go in side tables keyed by [`NodeId`], owned
/// by the passes that produce them (per the porting-plan "metadata in side tables" rule).
#[derive(Clone, Debug)]
pub struct Node {
    pub span: Span,
    pub kind: NodeKind,
}

/// Flat storage for every [`Node`] in a program.
#[derive(Clone, Debug, Default)]
pub struct Arena {
    nodes: Vec<Node>,
}

impl Arena {
    pub fn new() -> Arena {
        Arena { nodes: Vec::new() }
    }

    /// Append a node and return its [`NodeId`].
    pub fn alloc(&mut self, span: Span, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node { span, kind });
        id
    }

    /// Number of nodes allocated.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Borrow a node by id.
    pub fn get(&self, id: NodeId) -> &Node {
        &self.nodes[id.index()]
    }

    /// Mutably borrow a node by id (used during lowering to append to a module/block body
    /// that was allocated before its children were built).
    pub fn get_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.index()]
    }

    /// Iterate over `(NodeId, &Node)` in allocation order.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (NodeId(i as u32), n))
    }
}

impl Index<NodeId> for Arena {
    type Output = Node;

    fn index(&self, id: NodeId) -> &Node {
        self.get(id)
    }
}
