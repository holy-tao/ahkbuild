//! `#Include` / `#IncludeAgain` resolution (the IO half, sibling to [`search`](crate::search)).
//!
//! Unlike `#Import` — which loads each file as its own [`Group`](ahkbuild_ir::Group) — an
//! include pastes another file's text into the *current* module. [`resolve_includes`] walks a
//! group's entry tree, recursively resolves every include against the filesystem, loads each
//! referenced file into the shared source map, and returns an [`IncludeReport`] saying how each
//! directive should be materialized. Lowering then splices the `First` files into the IR; the
//! emitters consume the report's [`IncludeSplice`]s.
//!
//! Semantics implemented (v2.1): relative paths resolve against the directory of the file
//! containing the directive (overridable per-file by `#Include Dir`); dedup is per-module
//! (`#IncludeAgain` bypasses it); `<LibName>` searches the Lib folders with an underscore-prefix
//! fallback; `%A_…%` (incl. `%A_LineFile%`) is expanded; `*i` makes a missing file non-fatal;
//! and an include cycle is a hard error.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ahkbuild_ir::node::Module;
use ahkbuild_ir::{FileId, Lowering};
use ahkbuild_syntax::tree_sitter::{Node, Tree};
use anyhow::{anyhow, Context, Result};

use crate::bundle::IncludeSplice;
use crate::search::Builtins;

/// How a group's `#Include` graph resolved.
pub struct IncludeReport {
    /// Per directive — keyed `(including file, directive start offset within that file)` — how
    /// a backend should materialize it. A `#Include Dir` (directory) directive is absent.
    pub outcomes: HashMap<(FileId, u32), IncludeSplice>,
    pub warnings: Vec<String>,
}

impl IncludeReport {
    /// The subset lowering needs: directives that splice content, mapped to the included file.
    pub fn splices(&self) -> ahkbuild_ir::IncludeSplices {
        self.outcomes
            .iter()
            .filter_map(|(&k, v)| match v {
                IncludeSplice::First(f) => Some((k, *f)),
                _ => None,
            })
            .collect()
    }
}

/// Resolve every `#Include` reachable from a group's entry file. `entry_file`/`entry_tree`/
/// `entry_text` are the already-loaded entry (via [`Lowering::load`]); `entry_path` is its path
/// on disk (relative includes resolve against its directory).
pub fn resolve_includes(
    lowering: &mut Lowering,
    builtins: &Builtins,
    entry_file: FileId,
    entry_path: &Path,
    entry_text: &str,
    entry_tree: &Tree,
) -> Result<IncludeReport> {
    let entry_canon = canonical(entry_path);
    let mut r = Resolver {
        lowering,
        builtins,
        outcomes: HashMap::new(),
        warnings: Vec::new(),
        included: HashSet::new(),
        stack: vec![entry_canon.clone()],
        first_count: HashMap::new(),
        module_files: HashSet::new(),
    };
    let mut module = Module::MAIN.to_ascii_lowercase();
    r.process(entry_file, &entry_canon, entry_text, entry_tree, &mut module)?;

    // A `#Module` in a file pasted in more than once (via `#IncludeAgain` or into two modules)
    // is prohibited — it would reopen and duplicate the module. Warn rather than fail.
    for path in &r.module_files {
        if r.first_count.get(path).copied().unwrap_or(0) > 1 {
            r.warnings.push(format!(
                "{}: file uses #Module but is #Include'd multiple times (prohibited in v2.1)",
                path.display()
            ));
        }
    }

    Ok(IncludeReport {
        outcomes: r.outcomes,
        warnings: r.warnings,
    })
}

struct Resolver<'a> {
    lowering: &'a mut Lowering,
    builtins: &'a Builtins,
    outcomes: HashMap<(FileId, u32), IncludeSplice>,
    warnings: Vec<String>,
    /// Per-module dedup set: `(lowercased module name, canonical file path)`.
    included: HashSet<(String, PathBuf)>,
    /// Canonical paths currently being expanded, for cycle detection.
    stack: Vec<PathBuf>,
    /// How many times each canonical file is spliced (`First`), for the multi-include `#Module`
    /// warning.
    first_count: HashMap<PathBuf, usize>,
    /// Canonical paths of files that contain a `#Module` directive.
    module_files: HashSet<PathBuf>,
}

impl Resolver<'_> {
    /// Walk one file's top-level statements, resolving its includes. `module` is the current
    /// module name (lowercased); it is threaded by `&mut` and *persists* across includes, since
    /// a `#Module` in an included file carries into the includer's later statements (a paste).
    fn process(
        &mut self,
        file: FileId,
        canon: &Path,
        text: &str,
        tree: &Tree,
        module: &mut String,
    ) -> Result<()> {
        // `#Include Dir` changes the base directory for *subsequent* includes in this file only.
        let mut base_dir = canon.parent().unwrap_or(Path::new(".")).to_path_buf();
        let root = tree.root_node();
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                "module_directive" => {
                    self.module_files.insert(canon.to_path_buf());
                    if let Some(name) = child.named_child(0) {
                        *module = node_text(name, text).trim().to_ascii_lowercase();
                    }
                }
                "include_directive" | "include_again_directive" => {
                    self.handle_include(child, file, canon, text, &mut base_dir, module)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_include(
        &mut self,
        node: Node,
        includer_file: FileId,
        includer_canon: &Path,
        text: &str,
        base_dir: &mut PathBuf,
        module: &mut String,
    ) -> Result<()> {
        let off = node.start_byte() as u32;
        let again = node.kind() == "include_again_directive";
        let ignore = child_of_kind(node, "include_ignore_failure").is_some();

        if let Some(lib) = child_of_kind(node, "lib_name") {
            let name = strip_angle(node_text(lib, text));
            match self.resolve_lib(name) {
                Some(p) => self.splice(off, includer_file, &canonical(&p), again, module)?,
                None => self.unresolved(off, includer_file, ignore, includer_canon, &format!("<{name}>"))?,
            }
            return Ok(());
        }

        let Some(path_node) = child_of_kind(node, "file_or_dir_name") else {
            return Ok(());
        };
        let raw = unquote(node_text(path_node, text));
        let expanded = self.builtins.expand_include(raw, includer_canon);
        let candidate = resolve_path(&expanded, base_dir);

        if candidate.is_dir() {
            // `#Include Dir` — repoint subsequent includes in this file; nothing to emit.
            *base_dir = candidate;
            return Ok(());
        }
        if !candidate.is_file() {
            return self.unresolved(
                off,
                includer_file,
                ignore,
                includer_canon,
                &candidate.display().to_string(),
            );
        }
        self.splice(off, includer_file, &canonical(&candidate), again, module)
    }

    /// Record a splice of `canon` at this directive, deduping (unless `again`) per-module and
    /// recursing into the included file. Loads the file into the source map.
    fn splice(
        &mut self,
        off: u32,
        includer_file: FileId,
        canon: &Path,
        again: bool,
        module: &mut String,
    ) -> Result<()> {
        let key = (module.clone(), canon.to_path_buf());
        if !again && self.included.contains(&key) {
            self.outcomes.insert((includer_file, off), IncludeSplice::Dedup);
            return Ok(());
        }
        if self.stack.iter().any(|p| p == canon) {
            return Err(anyhow!(
                "#Include cycle detected:\n  {}\n  -> {} (cycle)",
                self.stack
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n  -> "),
                canon.display()
            ));
        }
        if !again {
            self.included.insert(key);
        }

        let text = std::fs::read_to_string(canon)
            .with_context(|| format!("reading #Include {}", canon.display()))?;
        let (inc_file, inc_tree) = self
            .lowering
            .load(canon.to_string_lossy().into_owned(), text.clone())
            .ok_or_else(|| anyhow!("parser returned no tree for {}", canon.display()))?;

        self.outcomes
            .insert((includer_file, off), IncludeSplice::First(inc_file));
        *self.first_count.entry(canon.to_path_buf()).or_insert(0) += 1;

        self.stack.push(canon.to_path_buf());
        self.process(inc_file, canon, &text, &inc_tree, module)?;
        self.stack.pop();
        Ok(())
    }

    fn unresolved(
        &mut self,
        off: u32,
        file: FileId,
        ignore: bool,
        includer: &Path,
        what: &str,
    ) -> Result<()> {
        if !ignore {
            return Err(anyhow!(
                "{}: #Include not found: {}",
                includer.display(),
                what
            ));
        }
        self.outcomes.insert((file, off), IncludeSplice::Missing);
        self.warnings.push(format!(
            "{}: ignored missing #Include {} (*i)",
            includer.display(),
            what
        ));
        Ok(())
    }

    /// Resolve `<LibName>` against the Lib folders, trying the full name then — for a
    /// `Prefix_Func` name — the prefix (`Prefix.ahk`), per AHK's library lookup.
    fn resolve_lib(&self, name: &str) -> Option<PathBuf> {
        let dirs = self.builtins.lib_dirs();
        let try_one = |n: &str| -> Option<PathBuf> {
            dirs.iter().find_map(|d| {
                let p = d.join(format!("{n}.ahk"));
                p.is_file().then_some(p)
            })
        };
        try_one(name).or_else(|| name.split_once('_').and_then(|(prefix, _)| try_one(prefix)))
    }
}

/// Slice a tree-sitter node's text from its file's source.
fn node_text<'t>(node: Node, text: &'t str) -> &'t str {
    &text[node.start_byte()..node.end_byte()]
}

fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    let found = node.named_children(&mut cursor).find(|c| c.kind() == kind);
    found
}

/// Strip one optional layer of surrounding quotes from an include path.
fn unquote(s: &str) -> &str {
    let t = s.trim();
    let b = t.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        &t[1..t.len() - 1]
    } else {
        t
    }
}

/// Strip the surrounding `<...>` (and optional quotes) from a `lib_name` token.
fn strip_angle(s: &str) -> &str {
    let t = unquote(s);
    t.strip_prefix('<')
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(t)
        .trim()
}

/// Resolve an (already `%var%`-expanded) include spec against `base_dir` if relative.
fn resolve_path(spec: &str, base_dir: &Path) -> PathBuf {
    let p = Path::new(spec.trim());
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base_dir.join(p)
    }
}

/// Canonicalize a path for dedup/cycle keys, falling back to the path itself if it can't be
/// canonicalized (e.g. it no longer exists — callers check `is_file` first for real includes).
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}
