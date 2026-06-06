//! Parsing front-end: drives the `tree-sitter-autohotkey` grammar and exposes the
//! source-mapping primitives the rest of the pipeline lowers from.
//!
//! The IR is owned and carries [`Span`]s rather than borrowed `tree_sitter::Node`s
//! (which borrow the [`Tree`]). This crate is the only place that touches tree-sitter
//! directly.

pub use tree_sitter;
pub use tree_sitter_autohotkey::LANGUAGE;

use tree_sitter::{Node, Parser, Tree};

/// A half-open byte range `[start, end)` into the original source.
///
/// IR nodes store spans instead of `tree_sitter::Node`s so the IR can outlive the
/// parse tree and be freely moved/owned. Slice the source with [`Span::text`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// The span covering a tree-sitter node's byte range.
    pub fn of(node: Node<'_>) -> Span {
        Span {
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
        }
    }

    /// Slice the originating source for this span's text.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start as usize..self.end as usize]
    }

    pub fn len(&self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// A parser preloaded with the AutoHotkey grammar.
///
/// Panics only if the linked grammar is ABI-incompatible with this `tree-sitter`
/// version, which is a build-time/dependency error, not a runtime input error.
pub fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&LANGUAGE.into())
        .expect("ahkbuild-syntax: grammar/tree-sitter ABI mismatch");
    parser
}

/// Parse AutoHotkey source into a concrete syntax tree.
///
/// Returns `None` only if tree-sitter itself yields no tree (e.g. a cancellation or
/// timeout was set, which we don't). A successfully returned tree may still contain
/// `ERROR`/`MISSING` nodes — check [`tree_sitter::Node::has_error`] on the root.
pub fn parse(source: &str) -> Option<Tree> {
    parser().parse(source, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_module_directive() {
        let src = "#Module Foo\nexport global Answer := 42\n";
        let tree = parse(src).expect("tree");
        assert!(
            !tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
    }

    #[test]
    fn span_slices_source() {
        let src = "x := 1";
        let tree = parse(src).expect("tree");
        let root = tree.root_node();
        let span = Span::of(root);
        assert_eq!(span.text(src), src);
    }
}
