//! Span-level source patching: the substrate for node-level emission.
//!
//! Rather than re-serialize the IR, emitters start from a group's original source text and
//! splice in a small set of [`Edit`]s (replace a span's bytes with new text; an empty
//! replacement deletes). Untouched regions — comments, whitespace, formatting — survive
//! verbatim. This is what lets import rewriting, comment-stripping, constant-folding and
//! tree-shaking compose: each is just a producer of [`Edit`]s over the same text.

use ahkbuild_syntax::Span;

/// Replace the bytes of `span` with `text`. An empty `text` deletes the span.
#[derive(Clone, Debug)]
pub struct Edit {
    pub span: Span,
    pub text: String,
}

impl Edit {
    pub fn new(span: Span, text: impl Into<String>) -> Edit {
        Edit {
            span,
            text: text.into(),
        }
    }
}

/// Apply `edits` to one file's text. `base` is the file's offset in the global
/// [`SourceMap`](ahkbuild_syntax::SourceMap) position space — spans are global, so each is
/// shifted back into file-relative range before slicing.
///
/// Edits are applied left-to-right by start position. Overlapping edits are resolved by
/// skipping any edit that begins before the previous one ended (last writer up to that point
/// wins), so a malformed edit set degrades gracefully instead of panicking or corrupting
/// byte offsets.
pub fn apply_edits(file_text: &str, base: u32, edits: &[Edit]) -> String {
    if edits.is_empty() {
        return file_text.to_string();
    }

    let mut ordered: Vec<&Edit> = edits.iter().collect();
    ordered.sort_by_key(|e| e.span.start);

    let mut out = String::with_capacity(file_text.len());
    let mut cursor = 0usize; // file-relative byte position already emitted
    for edit in ordered {
        let start = (edit.span.start - base) as usize;
        let end = (edit.span.end - base) as usize;
        if start < cursor {
            continue; // overlaps an earlier edit; drop it
        }
        out.push_str(&file_text[cursor..start]);
        out.push_str(&edit.text);
        cursor = end;
    }
    out.push_str(&file_text[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    #[test]
    fn no_edits_returns_input() {
        assert_eq!(apply_edits("hello", 0, &[]), "hello");
    }

    #[test]
    fn replaces_a_span() {
        // "abcXYZdef" -> replace XYZ (3..6) with "!"
        let edits = [Edit::new(span(3, 6), "!")];
        assert_eq!(apply_edits("abcXYZdef", 0, &edits), "abc!def");
    }

    #[test]
    fn empty_text_deletes() {
        let edits = [Edit::new(span(3, 6), "")];
        assert_eq!(apply_edits("abcXYZdef", 0, &edits), "abcdef");
    }

    #[test]
    fn applies_multiple_edits_in_order_regardless_of_input_order() {
        // Edits given out of order; both applied.
        let edits = [Edit::new(span(6, 9), "Z"), Edit::new(span(0, 3), "A")];
        assert_eq!(apply_edits("aaabbbccc", 0, &edits), "AbbbZ");
    }

    #[test]
    fn honors_file_base_offset() {
        // The file's text starts at global offset 100; the span is global.
        let edits = [Edit::new(span(103, 106), "!")];
        assert_eq!(apply_edits("abcXYZdef", 100, &edits), "abc!def");
    }

    #[test]
    fn overlapping_edit_is_dropped() {
        // Second edit (2..5) overlaps the first (0..4); it is skipped.
        let edits = [Edit::new(span(0, 4), "WXYZ"), Edit::new(span(2, 5), "!")];
        assert_eq!(apply_edits("abcdef", 0, &edits), "WXYZef");
    }
}
