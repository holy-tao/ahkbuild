//! [Continuation section] resolver. Collapses all continuation sections to a single
//! line of text.
//!
//! [continuation sections]: https://www.autohotkey.com/docs/v2/Scripts.htm#continuation-section

use anyhow::{bail, Result};

use crate::lex::{ends_with_name_char, scan_line, split_lines};

/// Resolve every continuation section in `src` into a single physical line.
///
/// Sources with no continuation sections are returned byte-for-byte unchanged.
pub(crate) fn resolve_continuations(name: &str, src: &str) -> Result<String> {
    let lines = split_lines(src);
    let mut out = String::with_capacity(src.len());

    // The current pending physical line (the logical line still being built). A
    // continuation section attaches to the line above it, so we hold that line open
    // until the next normal line arrives — which also lets chained sections keep
    // merging onto the same logical line.
    let mut cur = String::new();
    let mut cur_nl = "";
    let mut have_cur = false;

    let mut i = 0;
    while i < lines.len() {
        let (text, nl) = lines[i];

        match opener_rest(text) {
            // A line starting with `(` that classifies as a real section.
            Some(rest) => match classify_opener(rest) {
                OpenerKind::NotSection => {
                    if have_cur {
                        commit(&mut out, &cur, cur_nl);
                    }
                    cur = text.to_string();
                    cur_nl = nl;
                    have_cur = true;
                    i += 1;
                }
                OpenerKind::Section(opts) => {
                    // Gather interior lines up to the closing `)`.
                    let mut interior: Vec<&str> = Vec::new();
                    let mut closer_trailing: Option<&str> = None;
                    let mut closer_nl = "";
                    let mut j = i + 1;
                    while j < lines.len() {
                        let (lt, lnl) = lines[j];
                        if let Some(trailing) = closer_trailing_text(lt) {
                            closer_trailing = Some(trailing);
                            closer_nl = lnl;
                            break;
                        }
                        interior.push(lt);
                        j += 1;
                    }
                    let Some(trailing) = closer_trailing else {
                        bail!("{name}:{}: unterminated continuation section", i + 1);
                    };

                    let prev = if have_cur { cur.as_str() } else { "" };
                    let scan = scan_line(prev);

                    if scan.in_string {
                        // String section: splice the joined value between the open quote
                        // (end of the previous line) and the trailing text after `)`
                        // (which carries the closing quote), re-escaping for one line.
                        let value = build_string_value(&interior, &opts);
                        cur = format!("{prev}{}{trailing}", escape_for_source(&value));
                        cur_nl = closer_nl;
                        have_cur = true;
                    } else {
                        let sep = resolve_join(&opts);
                        if sep.contains('\n') || sep.contains('\r') {
                            bail!(
                                "{name}:{}: code continuation section joins with a newline; \
                                 cannot flatten to a single line",
                                i + 1
                            );
                        }
                        let joined = build_code(&interior, &opts, &sep);
                        if joined.contains('\n') || joined.contains('\r') {
                            bail!(
                                "{name}:{}: code continuation section spans multiple lines; \
                                 cannot flatten to a single line",
                                i + 1
                            );
                        }
                        let prev_code = prev[..scan.code_end].trim_end();
                        if prev_code.is_empty() {
                            // The line above is blank or comment-only: it carries no code
                            // to attach to, so emit it as-is and let the joined code stand
                            // on its own line.
                            if have_cur {
                                commit(&mut out, prev, cur_nl);
                            }
                            cur = format!("{joined}{trailing}");
                        } else {
                            let space = if ends_with_name_char(prev_code) {
                                " "
                            } else {
                                ""
                            };
                            cur = format!("{prev_code}{space}{joined}{trailing}");
                        }
                        cur_nl = closer_nl;
                        have_cur = true;
                    }

                    i = j + 1;
                }
            },
            // An ordinary line.
            None => {
                if have_cur {
                    commit(&mut out, &cur, cur_nl);
                }
                cur = text.to_string();
                cur_nl = nl;
                have_cur = true;
                i += 1;
            }
        }
    }

    if have_cur {
        commit(&mut out, &cur, cur_nl);
    }
    Ok(out)
}

fn commit(out: &mut String, line: &str, nl: &str) {
    out.push_str(line);
    out.push_str(nl);
}

#[derive(Clone, Copy, PartialEq)]
enum LTrim {
    /// Default "smart" behaviour: strip the first interior line's indentation.
    Smart,
    /// `LTrim`: strip all leading whitespace.
    All,
    /// `LTrim0`: keep leading whitespace.
    None,
}

struct Options {
    /// `None` => default separator (a linefeed). `Some(raw)` => the raw `Join` param
    /// (possibly empty for direct concatenation), pre-escape-translation.
    join: Option<String>,
    ltrim: LTrim,
    /// Default trims trailing whitespace per line; `RTrim0` keeps it.
    rtrim_keep: bool,
    /// `Comments`/`Comment`/`Com`/`C`: strip `;` line comments from interior lines.
    comments: bool,
    /// The accent (`` ` ``) option: treat backticks literally (no escape translation).
    accent: bool,
}

enum OpenerKind {
    /// A line starting with `(` that is really an expression (`((`, `(MyFunc(`, `(x.y)`).
    NotSection,
    Section(Options),
}

/// If `line`'s first non-whitespace character is `(`, return the text after it (the
/// options string); otherwise `None`.
fn opener_rest(line: &str) -> Option<&str> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    trimmed.strip_prefix('(')
}

/// Classify a `(`-opening line by its options string (everything after the `(`).
fn classify_opener(rest: &str) -> OpenerKind {
    let mut opts = Options {
        join: None,
        ltrim: LTrim::Smart,
        rtrim_keep: false,
        comments: false,
        accent: false,
    };

    for tok in rest.split_whitespace() {
        let lower = tok.to_ascii_lowercase();
        if let Some(param) = strip_join(tok, &lower) {
            opts.join = Some(param.to_string());
            continue;
        }
        // A parenthesis anywhere outside a `Join` param means this line is an
        // expression, not a continuation section (per the docs' non-continuation rule).
        if tok.contains('(') || tok.contains(')') {
            return OpenerKind::NotSection;
        }
        match lower.as_str() {
            "ltrim" => opts.ltrim = LTrim::All,
            "ltrim0" => opts.ltrim = LTrim::None,
            "rtrim" => opts.rtrim_keep = false,
            "rtrim0" => opts.rtrim_keep = true,
            "comments" | "comment" | "com" | "c" => opts.comments = true,
            "`" => opts.accent = true,
            _ => return OpenerKind::NotSection,
        }
    }

    OpenerKind::Section(opts)
}

/// If `tok` is a `Join` option token, return its param (the text after `Join`).
fn strip_join<'a>(tok: &'a str, lower: &str) -> Option<&'a str> {
    if lower.starts_with("join") {
        Some(&tok[4..])
    } else {
        None
    }
}

/// If `line`'s first non-whitespace character is `)` (a section closer), return the
/// text after that `)`. An escaped `` `) `` has a backtick as its first non-whitespace
/// character, so it is not a closer.
fn closer_trailing_text(line: &str) -> Option<&str> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    trimmed.strip_prefix(')')
}

/// The first interior line's indentation, used by the default "smart" `LTrim`. Only the
/// first whitespace character type is treated as indentation.
fn smart_indent<'a>(interior: &[&'a str]) -> &'a str {
    let Some(first) = interior.first() else {
        return "";
    };
    let bytes = first.as_bytes();
    let Some(&lead) = bytes.first().filter(|&&c| c == b' ' || c == b'\t') else {
        return "";
    };
    let end = bytes.iter().take_while(|&&c| c == lead).count();
    &first[..end]
}

/// Strip a trailing `;` comment (and the whitespace to its left) when `comments` is set.
fn strip_comment(line: &str, comments: bool) -> &str {
    if !comments {
        return line;
    }
    let scan = scan_line(line);
    if scan.code_end == line.len() {
        line
    } else {
        line[..scan.code_end].trim_end_matches([' ', '\t'])
    }
}

fn apply_ltrim<'a>(line: &'a str, ltrim: LTrim, indent: &str) -> &'a str {
    match ltrim {
        LTrim::None => line,
        LTrim::All => line.trim_start_matches([' ', '\t']),
        LTrim::Smart => line.strip_prefix(indent).unwrap_or(line),
    }
}

fn apply_rtrim(line: &str, rtrim_keep: bool) -> &str {
    if rtrim_keep {
        line
    } else {
        line.trim_end_matches([' ', '\t'])
    }
}

/// The resolved `Join` separator (escape-translated unless accent mode); default is a
/// linefeed.
fn resolve_join(opts: &Options) -> String {
    match &opts.join {
        None => "\n".to_string(),
        Some(raw) => translate_escapes(raw, opts.accent),
    }
}

/// Build the *value* of a string section: interior lines trimmed/comment-stripped,
/// escape-translated, and joined by the resolved separator.
fn build_string_value(interior: &[&str], opts: &Options) -> String {
    let indent = smart_indent(interior);
    let sep = resolve_join(opts);
    let parts: Vec<String> = interior
        .iter()
        .map(|raw| {
            let line = strip_comment(raw, opts.comments);
            let line = apply_ltrim(line, opts.ltrim, indent);
            let line = apply_rtrim(line, opts.rtrim_keep);
            translate_escapes(line, opts.accent)
        })
        .collect();
    parts.join(&sep)
}

/// Build the flattened *code* of a non-string section: interior lines trimmed/comment-
/// stripped (but not escape-translated — they are source code) and joined.
fn build_code(interior: &[&str], opts: &Options, sep: &str) -> String {
    let indent = smart_indent(interior);
    let parts: Vec<&str> = interior
        .iter()
        .map(|raw| {
            let line = strip_comment(raw, opts.comments);
            let line = apply_ltrim(line, opts.ltrim, indent);
            apply_rtrim(line, opts.rtrim_keep)
        })
        .collect();
    parts.join(sep)
}

/// Translate AutoHotkey escape sequences in `s` to their literal characters. In accent
/// mode, backticks are literal and `s` is returned unchanged.
fn translate_escapes(s: &str, accent: bool) -> String {
    if accent || !s.contains('`') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '`' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some('a') => out.push('\u{7}'),
            // Any other escaped char (including `` ` ``, `"`, `;`) is taken literally.
            Some(other) => out.push(other),
            None => out.push('`'),
        }
    }
    out
}

/// Escape a string *value* back into AutoHotkey source for a single-line double-quoted
/// literal. Backtick must be escaped first.
fn escape_for_source(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '`' => out.push_str("``"),
            '"' => out.push_str("`\""),
            '\n' => out.push_str("`n"),
            '\r' => out.push_str("`r"),
            '\t' => out.push_str("`t"),
            '\u{8}' => out.push_str("`b"),
            '\u{c}' => out.push_str("`f"),
            '\u{b}' => out.push_str("`v"),
            '\u{7}' => out.push_str("`a"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(src: &str) -> String {
        resolve_continuations("test.ahk", src).expect("resolve")
    }

    #[test]
    fn no_sections_is_byte_identical() {
        for src in [
            "",
            "x := 1",
            "x := 1\n",
            "a\r\nb\r\n",
            "; comment\nMsgBox \"hi\"\n",
            "x := (a\n    + b)\n", // `(` not at line start: continuation by enclosure
        ] {
            assert_eq!(resolve(src), src, "src = {src:?}");
        }
    }

    #[test]
    fn code_section_joins_into_a_call() {
        // The doc example: JoinB stitches "Msg" + "B" + "ox ..." into a MsgBox call.
        let src = "; This calls MsgBox:\n( JoinB\nMsg\nox \"Hello, World!\"\n)\n";
        let out = resolve(src);
        assert_eq!(out, "; This calls MsgBox:\nMsgBox \"Hello, World!\"\n");
    }

    #[test]
    fn string_section_collapses_to_single_line() {
        // The doc example: `c` strips the comment, Join`n`t joins with LF+TAB.
        let src = "str := \"\n( c Join`n`t\n    This is a multiline string ; that allows comments\n    where the lines are joined by a newline and a tab\n)\"\n";
        let out = resolve(src);
        assert_eq!(
            out,
            "str := \"This is a multiline string`n`twhere the lines are joined by a newline and a tab\"\n"
        );
    }

    #[test]
    fn default_string_section_uses_linefeed_and_escapes_quotes() {
        let src = "x := \"\n(\nLine \"one\"\nLine two\n)\"\n";
        let out = resolve(src);
        assert_eq!(out, "x := \"Line `\"one`\"`nLine two\"\n");
    }

    #[test]
    fn join_with_empty_param_concatenates_directly() {
        let src = "x := \"\n( Join\nab\ncd\n)\"\n";
        assert_eq!(resolve(src), "x := \"abcd\"\n");
    }

    #[test]
    fn ltrim_strips_all_leading_whitespace() {
        let src = "x := \"\n( LTrim\n    a\n        b\n)\"\n";
        assert_eq!(resolve(src), "x := \"a`nb\"\n");
    }

    #[test]
    fn smart_ltrim_strips_first_line_indent() {
        let src = "x := \"\n(\n    a\n        b\n)\"\n";
        // First line indent is four spaces; "        b" keeps the remaining four.
        assert_eq!(resolve(src), "x := \"a`n    b\"\n");
    }

    #[test]
    fn ltrim0_keeps_indentation() {
        let src = "x := \"\n( LTrim0\n    a\n)\"\n";
        assert_eq!(resolve(src), "x := \"    a\"\n");
    }

    #[test]
    fn rtrim0_keeps_trailing_whitespace() {
        let src = "x := \"\n( LTrim0 RTrim0 Join\na  \nb\n)\"\n";
        assert_eq!(resolve(src), "x := \"a  b\"\n");
    }

    #[test]
    fn accent_treats_backticks_literally() {
        let src = "x := \"\n( `\na`nb\n)\"\n";
        // With accent, `n is two literal characters: backtick + n, re-escaped as ``n.
        assert_eq!(resolve(src), "x := \"a``nb\"\n");
    }

    #[test]
    fn trailing_code_after_closer_is_preserved() {
        let src = "FileAppend \"\n(\nLine 1\nLine 2\n)\", A_Desktop\n";
        assert_eq!(resolve(src), "FileAppend \"Line 1`nLine 2\", A_Desktop\n");
    }

    #[test]
    fn expression_lines_starting_with_paren_are_left_alone() {
        for src in [
            "x := y\n((a + b)\n)\n",   // `((` -> expression
            "(x.y)[z]()\n",            // parens to the right -> expression
            "(MyFunc(\n    arg\n))\n", // `(MyFunc(` -> expression
        ] {
            assert_eq!(resolve(src), src, "src = {src:?}");
        }
    }

    #[test]
    fn name_char_above_inserts_a_space() {
        let src = "Result := Do\n( Join\nStuff()\n)\n";
        assert_eq!(resolve(src), "Result := Do Stuff()\n");
    }

    #[test]
    fn unterminated_section_is_an_error() {
        let err = resolve_continuations("f.ahk", "x := \"\n(\nnever closed\n").unwrap_err();
        assert!(err.to_string().contains("unterminated"), "{err}");
    }

    #[test]
    fn code_section_with_newline_join_is_an_error() {
        let err = resolve_continuations("f.ahk", "y := Foo\n( Join`n\na\nb\n)\n").unwrap_err();
        assert!(err.to_string().contains("single line"), "{err}");
    }
}
