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

/// A handle to a [`SourceFile`] within a [`SourceMap`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileId(pub u32);

/// One physical source: the main script, an imported/embedded file, or (later) a
/// `#Include`d fragment. `base` is where this file's bytes begin in the [`SourceMap`]'s
/// global position space, so a [`Span`] uniquely identifies both a file and a range
/// without carrying a `FileId` itself (keeping `Span` at two `u32`s).
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub id: FileId,
    /// Path or resource label, for diagnostics / `A_LineFile` fidelity.
    pub name: String,
    pub text: String,
    pub base: u32,
}

impl SourceFile {
    /// One past this file's last global byte position.
    pub fn end(&self) -> u32 {
        self.base + self.text.len() as u32
    }

    /// Slice this file's own text for a global span lying within it.
    fn slice(&self, span: Span) -> &str {
        &self.text[(span.start - self.base) as usize..(span.end - self.base) as usize]
    }
}

/// The set of physical sources behind a lowered program, laid out in one global byte
/// position space (rustc-style). This is the single source of truth for span text and the
/// origin axis for diagnostics; it is orthogonal to module *groups* (a group is a
/// module-name namespace, and may span several files via `#Include`).
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> SourceMap {
        SourceMap { files: Vec::new() }
    }

    /// A map holding a single file at base 0 (the current single-file pipeline).
    pub fn single(name: impl Into<String>, text: impl Into<String>) -> (SourceMap, FileId) {
        let mut m = SourceMap::new();
        let id = m.add(name, text);
        (m, id)
    }

    /// Append a file; its bytes occupy `[base, base + len)` where `base` follows the
    /// previously added files. A non-entry file's lowered spans must be offset by its
    /// `base` (see [`SourceMap::base`]) to be valid global positions.
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> FileId {
        let text = text.into();
        let base = self.files.last().map_or(0, SourceFile::end);
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceFile {
            id,
            name: name.into(),
            text,
            base,
        });
        id
    }

    pub fn file(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }

    /// The global base offset of a file — what to add to its file-relative byte offsets.
    pub fn base(&self, id: FileId) -> u32 {
        self.file(id).base
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }

    /// The file whose byte range contains global position `pos`.
    pub fn file_at(&self, pos: u32) -> &SourceFile {
        let idx = self
            .files
            .partition_point(|f| f.base <= pos)
            .saturating_sub(1);
        &self.files[idx]
    }

    /// Slice the source text for a (global) span.
    pub fn text(&self, span: Span) -> &str {
        self.file_at(span.start).slice(span)
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
