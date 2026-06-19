//! Shared line/lexing primitives for the text-preprocessing passes.

/// The result of scanning one physical line: its end-of-line string state and where its
/// code ends (before any trailing line comment).
pub(crate) struct LineScan {
    /// Whether the line ends inside an open double-quoted string.
    pub(crate) in_string: bool,
    /// Byte index where the code ends (before any trailing line comment).
    pub(crate) code_end: usize,
}

/// Single-pass scan of one physical line, tracking string state and locating a trailing
/// line comment. Used to decide whether a continuation section starts inside a string
/// and, for code sections, where the previous line's code ends.
///
/// A `;` begins a comment only at the start of the line or after whitespace, and never
/// inside a string. A backtick escapes the following character in or out of a string.
pub(crate) fn scan_line(s: &str) -> LineScan {
    let b = s.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    let mut code_end = s.len();
    while i < b.len() {
        match b[i] {
            b'`' => {
                // Backtick escapes the next character, in or out of a string.
                i += 2;
                continue;
            }
            b'"' => in_string = !in_string,
            b';' if !in_string => {
                let prev_ws = i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t';
                if prev_ws {
                    code_end = i;
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    LineScan {
        in_string,
        code_end,
    }
}

/// Whether `s` ends with an AutoHotkey name character (alphanumeric or `_`).
pub(crate) fn ends_with_name_char(s: &str) -> bool {
    s.chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Split `src` into `(content, newline)` pairs where `content` excludes the line
/// terminator and `newline` is `"\r\n"`, `"\n"`, or `""` (final line without a trailing
/// newline). Concatenating every `content + newline` reproduces `src` exactly.
pub(crate) fn split_lines(src: &str) -> Vec<(&str, &str)> {
    let mut lines = Vec::new();
    let bytes = src.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            if i > start && bytes[i - 1] == b'\r' {
                lines.push((&src[start..i - 1], &src[i - 1..=i]));
            } else {
                lines.push((&src[start..i], &src[i..=i]));
            }
            start = i + 1;
        }
        i += 1;
    }
    if start < bytes.len() {
        lines.push((&src[start..], ""));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_lines_roundtrips() {
        for src in ["", "a", "a\n", "a\r\nb\r\n", "a\n\nb", "\n", "x\ny\nz"] {
            let rejoined: String = split_lines(src)
                .iter()
                .map(|(c, nl)| format!("{c}{nl}"))
                .collect();
            assert_eq!(rejoined, src, "src = {src:?}");
        }
    }

    #[test]
    fn split_lines_preserves_terminators() {
        assert_eq!(split_lines("a\r\nb\n"), vec![("a", "\r\n"), ("b", "\n")]);
        assert_eq!(split_lines("a"), vec![("a", "")]);
    }

    #[test]
    fn scan_line_tracks_open_string() {
        assert!(scan_line("str := \"").in_string);
        assert!(!scan_line("str := \"closed\"").in_string);
        // A quote escaped by a backtick does not toggle the string state.
        assert!(!scan_line("x := \"a`\"b\"").in_string);
    }

    #[test]
    fn scan_line_finds_comment_boundary() {
        let s = "foo() ; note";
        assert_eq!(scan_line(s).code_end, 6);
        // A semicolon not preceded by whitespace is not a comment.
        assert_eq!(scan_line("a;b").code_end, "a;b".len());
        // A semicolon inside a string is not a comment.
        assert_eq!(scan_line("x := \"a ; b\"").code_end, "x := \"a ; b\"".len());
    }

    #[test]
    fn name_char_detection() {
        assert!(ends_with_name_char("Do"));
        assert!(ends_with_name_char("a_1"));
        assert!(!ends_with_name_char("x :="));
        assert!(!ends_with_name_char("\""));
        assert!(!ends_with_name_char(""));
    }
}
