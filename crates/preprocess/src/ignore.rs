//! Processes the line-based ignore preprocessor directives:
//! - ;@ahk2exe-ignore[begin|end]
//! - ;@ahkbuild-ignore[begin|end]
//!
//! This step respects ahk2exe and ahkbuild ignore directives, for compatibility
//! with older scripts.

use anyhow::{ensure, Result};

use crate::lex::split_lines;

const IGNORE_BEGIN: [&str; 2] = [";@ahk2exe-ignorebegin", ";@ahkbuild-ignorebegin"];
const IGNORE_END: [&str; 2] = [";@ahk2exe-ignoreend", ";@ahkbuild-ignoreend"];

pub(crate) fn scan_ignores(name: &str, src: &str) -> Result<String> {
    let lines = split_lines(src);
    let mut out = String::with_capacity(src.len());

    let mut ignoring = false; // whether we're currently in an ignore section
    let mut ignore_start = 0; // line the current ignore section started on, for error reporting

    for (i, (text, nl)) in lines.iter().enumerate() {
        if !ignoring {
            if is_directive(text, IGNORE_BEGIN) {
                // In a new ignore block
                ignoring = true;
                ignore_start = i;
            } else {
                out.push_str(text);
                out.push_str(nl);
            }
        } else {
            if is_directive(text, IGNORE_END) {
                ignoring = false;
            }
        }
    }

    // FIXME: line number may be off if continuation resolution collapsed sections
    ensure!(
        !ignoring,
        format!(
            "Unclosed ignore directive in file '{}' starting at line {}",
            name,
            ignore_start + 1
        )
    );

    Ok(out)
}

fn is_directive<'a>(text: &str, directives: impl IntoIterator<Item = &'a str>) -> bool {
    directives
        .into_iter()
        .any(|d| text.trim().to_ascii_lowercase().starts_with(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ahkbuild_ignore_drops_code() {
        let src = ";@ahkbuild-ignorebegin\nShould be removed\n;@ahkbuild-ignoreend\nshould be kept";

        let procesed = scan_ignores("src", src).expect("scan_ignores should return a string");
        assert_eq!(procesed, "should be kept");
    }

    #[test]
    fn ahk2exe_ignore_drops_code() {
        let src = ";@ahk2exe-ignorebegin\nShould be removed\n;@ahk2exe-ignoreend\nshould be kept";

        let procesed = scan_ignores("src", src).expect("scan_ignores should return a string");
        assert_eq!(procesed, "should be kept");
    }

    #[test]
    fn directive_parsing_is_case_insensitive() {
        let src = ";@Ahk2Exe-IgnoreBegin\nShould be removed\n;@AHK2EXE-ignoreend\nshould be kept";

        let procesed = scan_ignores("src", src).expect("scan_ignores should return a string");
        assert_eq!(procesed, "should be kept");
    }

    #[test]
    fn directive_parsing_allows_trailing_comment() {
        let src = ";@Ahk2Exe-IgnoreBegin this is just a comment\nShould be removed\n;@AHK2EXE-ignoreend\nshould be kept";

        let procesed = scan_ignores("src", src).expect("scan_ignores should return a string");
        assert_eq!(procesed, "should be kept");
    }

    #[test]
    fn unclosed_ignore_directives_error() {
        let src = ";@ahkbuild-ignorebegin\nsomething";
        let _ = scan_ignores("src", src)
            .expect_err("scan_ignore with an unclosed ignore directive should return Err");
    }
}
