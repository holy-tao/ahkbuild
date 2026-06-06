//! Intermediate representation and CST -> IR lowering. (Step 1 — not yet implemented.)
//!
//! Design constraints from `docs/PORTING_PLAN.md`:
//!   - Nodes live in an **arena** (`Vec<Node>` indexed by a `NodeId(u32)`), not via
//!     parent pointers or `Rc<RefCell<_>>`.
//!   - Each node is owned and carries a [`ahkbuild_syntax::Span`] for source mapping
//!     and patch-based emission — never a borrowed `tree_sitter::Node`.
//!   - `Node` is a single `enum`; reachability/reference walks `match` it with no
//!     catch-all arm, so adding a variant forces every walker to handle it.

// Re-exported so downstream crates have one canonical Span type.
pub use ahkbuild_syntax::Span;
