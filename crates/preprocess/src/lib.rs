//! Preprocessing module - handles text-rewriting preprocessing passes
//! before the parser is parsed.
//!
use anyhow::Result;

mod lex;

mod continuation;
use continuation::resolve_continuations;

mod ignore;
use ignore::scan_ignores;

/// Preprocess the provided source.
///
/// `name` labels the source for error messages. Returns the rewritten source, or an
/// error naming the offending 1-based line for any section that can't be confidently
/// resolved (unterminated, unknown option, or a code section that can't be flattened
/// onto one line).
pub fn run(name: &str, src: &str) -> Result<String> {
    let mut output = resolve_continuations(name, src)?;
    output = scan_ignores(name, &output)?;
    Ok(output)
}
