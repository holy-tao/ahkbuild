//! Static `FileInstall` detection for the `.exe` backend.
//!
//! AHK enforces that `FileInstall`'s first argument is a quoted literal, so the embedded files
//! can be discovered without running the script. [`scan`] walks the converged program - honoring
//! the same fold/shake decisions the emitter applies - and returns one [`FileInstall`] per *live*
//! call: the resource name the v2.1 interpreter looks the file up under, and the on-disk path to
//! read its bytes from.
//!
//! Resource-name scheme (confirmed against the interpreter, matching Ahk2Exe's `StringUpper` of
//! the raw literal): the first-argument string **as written**, quotes stripped, uppercased, with
//! slashes and a leading `./` preserved - `FileInstall "./assets/x.json"` ->
//! `./ASSETS/X.JSON`. The literal `.` is kept in the resource name but resolved to a real path
//! when reading the file to embed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use ahkbuild_fold::{Branch, FoldResult};
use ahkbuild_ir::node::LiteralKind;
use ahkbuild_ir::{children, NodeId, NodeKind, Program};
use ahkbuild_shake::ShakeResult;

/// A live `FileInstall` to embed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInstall {
    /// `RT_RCDATA` resource name the interpreter looks the file up under (uppercased literal).
    pub name: String,
    /// On-disk path to read the file's bytes from (literal resolved against its source file's dir).
    pub source: PathBuf,
}

/// Find every live `FileInstall` call in `program`. Calls in fold-dead branches or shake-removed
/// declarations/modules are skipped (their `FileInstall` is gone from the emitted source, so
/// embedding the file would be wasteful and could spuriously fail if the file is absent).
///
/// Errors if a `FileInstall`'s first argument is not a simple literal string (dynamic FileInstall
/// is unsupported for exe bundling).
pub fn scan(
    program: &Program,
    shake: Option<&ShakeResult>,
    fold: Option<&FoldResult>,
) -> Result<Vec<FileInstall>> {
    let mut walker = Walker {
        program,
        shake,
        fold,
        out: Vec::new(),
        seen: HashSet::new(),
    };
    let dead_modules: HashSet<NodeId> = shake
        .map(|s| s.dead_modules.iter().copied().collect())
        .unwrap_or_default();

    for group in &program.groups {
        for &module in &group.modules {
            if dead_modules.contains(&module) {
                continue;
            }
            walker.visit(module)?;
        }
    }
    Ok(walker.out)
}

struct Walker<'a> {
    program: &'a Program,
    shake: Option<&'a ShakeResult>,
    fold: Option<&'a FoldResult>,
    out: Vec<FileInstall>,
    seen: HashSet<String>,
}

impl Walker<'_> {
    /// Whether shake removed this node (a dead declaration/statement/module).
    fn is_dead(&self, id: NodeId) -> bool {
        self.shake.is_some_and(|s| s.dead.contains(&id))
    }

    /// Recursively scan `id` and its descendants for live `FileInstall` calls.
    fn visit(&mut self, id: NodeId) -> Result<()> {
        if self.is_dead(id) {
            return Ok(());
        }

        if matches!(self.program.arena[id].kind, NodeKind::CallExpr(_)) {
            self.check_call(id)?;
        }

        // Collect the children to recurse into (releasing the arena borrow before recursing). For
        // branch nodes this is the condition plus only the live arm(s); for everything else it is
        // the full `children` set.
        let targets: Vec<NodeId> = match &self.program.arena[id].kind {
            NodeKind::IfStmt {
                condition,
                then_body,
                else_body,
            } => {
                let mut v = vec![*condition];
                v.extend(self.live_arms(id, *then_body, *else_body));
                v
            }
            NodeKind::TernaryExpr {
                condition,
                then_branch,
                else_branch,
            } => {
                let mut v = vec![*condition];
                v.extend(self.live_arms(id, *then_branch, Some(*else_branch)));
                v
            }
            other => children(other),
        };

        for t in targets {
            self.visit(t)?;
        }
        Ok(())
    }

    /// The arm(s) of branch node `branch` that survive tree-shaking
    fn live_arms(&self, branch: NodeId, then_arm: NodeId, else_arm: Option<NodeId>) -> Vec<NodeId> {
        match self.fold.and_then(|f| f.branches.get(&branch)) {
            Some(Branch::Then) => vec![then_arm],
            Some(Branch::Else) => else_arm.into_iter().collect(),
            Some(Branch::Dead) => Vec::new(),
            None => std::iter::once(then_arm).chain(else_arm).collect(),
        }
    }

    /// If `id` is a statement-position `FileInstall(...)` call, record the embed; otherwise ignore.
    fn check_call(&mut self, id: NodeId) -> Result<()> {
        let NodeKind::CallExpr(call) = &self.program.arena[id].kind else {
            return Ok(());
        };
        if !matches!(self.program.arena[call.callee].kind, NodeKind::Identifier) {
            return Ok(());
        }
        if !self
            .program
            .text(call.callee)
            .trim()
            .eq_ignore_ascii_case("FileInstall")
        {
            return Ok(());
        }

        let where_ = self.location(id);
        let Some(&first) = call.args.first() else {
            bail!("FileInstall with no source-path argument {where_}");
        };
        if !matches!(
            self.program.arena[first].kind,
            NodeKind::Literal {
                kind: LiteralKind::String
            }
        ) {
            bail!(
                "FileInstall source path must be a literal string (dynamic FileInstall is not \
                 supported for exe bundling) {where_}"
            );
        }

        let raw = strip_string_literal(self.program.text(first), &where_)?;

        // Resolve the literal against the directory of the file that contains the call (a group
        // can span several #Include'd files, so use the argument's own source file).
        let file = self
            .program
            .sources
            .file_at(self.program.arena[first].span.start);
        let dir = Path::new(&file.name)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let source = dir.join(&raw);
        let name = raw.to_ascii_uppercase();

        if self.seen.insert(name.clone()) {
            self.out.push(FileInstall { name, source });
        }
        Ok(())
    }

    /// A `"<file>:<line>"` location suffix for diagnostics, derived from the node's source file.
    fn location(&self, id: NodeId) -> String {
        let span = self.program.arena[id].span;
        let file = self.program.sources.file_at(span.start);
        let line = file.text[..(span.start - file.base) as usize]
            .bytes()
            .filter(|&b| b == b'\n')
            .count()
            + 1;
        format!("at {}:{}", file.name, line)
    }
}

/// Strip the surrounding quotes from a string-literal span's text. Rejects escape sequences
/// (a backtick) so the computed resource name can never diverge from the interpreter's: paths
/// with escapes are not supported for exe bundling.
fn strip_string_literal(text: &str, where_: &str) -> Result<String> {
    let text = text.trim();
    let inner = match text.chars().next() {
        Some(q @ ('"' | '\'')) if text.len() >= 2 && text.ends_with(q) => &text[1..text.len() - 1],
        _ => text,
    };
    if inner.contains('`') {
        bail!("FileInstall path contains an escape sequence (`); not supported for exe bundling {where_}");
    }
    // Within a quoted AHK string a doubled delimiter is one literal delimiter.
    let inner = inner.replace("\"\"", "\"").replace("''", "'");
    Ok(inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahkbuild_ir::lower::lower;
    use ahkbuild_ir::Program;

    fn program(src: &str) -> Program {
        let tree = ahkbuild_syntax::parse(src).expect("tree");
        assert!(!tree.root_node().has_error(), "parse error in fixture");
        lower(&tree, src)
    }

    #[test]
    fn computes_uppercased_forward_slash_name() {
        let p = program("FileInstall \"./assets/config.json\", \"out.json\"\n");
        let found = scan(&p, None, None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "./ASSETS/CONFIG.JSON");
    }

    #[test]
    fn dedups_identical_literals() {
        let p = program("FileInstall \"data/x.txt\", \"a\"\nFileInstall \"data/x.txt\", \"b\"\n");
        let found = scan(&p, None, None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "DATA/X.TXT");
    }

    #[test]
    fn rejects_dynamic_first_arg() {
        let p = program("src := \"x\"\nFileInstall src, \"out\"\n");
        let err = scan(&p, None, None).unwrap_err().to_string();
        assert!(err.contains("literal string"), "{err}");
    }

    #[test]
    fn finds_calls_inside_functions() {
        let p = program("Extract() {\n  FileInstall \"a/b.txt\", \"b\"\n}\n");
        let found = scan(&p, None, None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "A/B.TXT");
    }

    #[test]
    fn skips_fold_dead_branch() {
        // With A_IsCompiled folded to true, only the `then` arm survives, so the `else` arm's
        // FileInstall must not be embedded.
        let p = program(
            "if A_IsCompiled\n  FileInstall \"kept.txt\", \"k\"\nelse\n  FileInstall \"dropped.txt\", \"d\"\n",
        );
        let fold = ahkbuild_fold::fold(
            &p,
            &ahkbuild_fold::Constants {
                is_compiled: Some(true),
                ptr_size: None,
            },
        );
        let found = scan(&p, None, Some(&fold)).unwrap();
        assert_eq!(
            found.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            vec!["KEPT.TXT"]
        );
    }
}
