//! Preprocessing module - handles text-rewriting preprocessing passes
//! before the parser is parsed.
//!
use anyhow::Result;

mod lex;

mod continuation;
use continuation::resolve_continuations;

/// Preprocess the provided source.
///
/// `name` labels the source for error messages. Returns the rewritten source, or an
/// error naming the offending 1-based line for any section that can't be confidently
/// resolved (unterminated, unknown option, or a code section that can't be flattened
/// onto one line).
pub fn run(name: &str, src: &str) -> Result<String> {
    let output = resolve_continuations(name, src)?;
    Ok(output)
}
