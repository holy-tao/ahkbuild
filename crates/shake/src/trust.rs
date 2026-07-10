//! Out-of-band trust: package files a project author has vetted as safe, so a dynamic construct in
//! them does not force a conservative blow-up (see [`crate`] docs and `crates/config/src/trust.rs`).
//! This is an out-of-source equivalent to `;@ahkbuild-safe`, for external packages

use std::path::{Path, PathBuf};

use ahkbuild_ir::{NodeId, Program};

/// Which files of a trusted package are covered.
#[derive(Debug, Clone)]
enum Rule {
    /// Every file under one of the roots.
    All,
    /// Only these exact canonical file paths.
    Files(Vec<PathBuf>),
}

#[derive(Debug, Clone)]
struct Entry {
    roots: Vec<PathBuf>,
    rule: Rule,
}

/// The set of package files whose dynamic constructs are vouched safe. Empty by default, which
/// preserves the conservative behavior (every dynamic construct blows up unless annotated in-source).
#[derive(Debug, Clone, Default)]
pub struct TrustSet {
    entries: Vec<Entry>,
}

impl TrustSet {
    /// Trust a package. `roots` are the canonical directories (or single-file paths) the package's
    /// files may live under; `files`, when `Some`, restricts trust to those exact canonical file
    /// paths, and `None` trusts every file under a root.
    pub fn trust_package(&mut self, roots: Vec<PathBuf>, files: Option<Vec<PathBuf>>) {
        let rule = match files {
            Some(paths) => Rule::Files(paths),
            None => Rule::All,
        };
        self.entries.push(Entry { roots, rule });
    }

    /// Whether the file that node `n` originates from is trusted. False for an empty set.
    pub fn file_is_trusted(&self, program: &Program, n: NodeId) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let span = program.arena[n].span;
        let file = Path::new(program.sources.file_at(span.start).name.as_str());
        self.entries.iter().any(|e| match &e.rule {
            Rule::All => e.roots.iter().any(|r| file.starts_with(r)),
            // A listed file must sit under one of the package's roots *and* be one of the named
            // files (guards against a listed relative path that failed to canonicalize).
            Rule::Files(paths) => {
                e.roots.iter().any(|r| file.starts_with(r)) && paths.iter().any(|p| p == file)
            }
        })
    }
}
