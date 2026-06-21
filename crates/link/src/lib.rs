//! The module-graph linker.
//!
//! Given an entry script, resolves its `#Import` graph across files, lowers each origin
//! into its own [`Group`](ahkbuild_ir::Group), and assembles one multi-group
//! [`Program`]. This is the IO-bearing layer above the (pure) `ir` crate; later passes
//! (import rewriting, the `.ahk` / `.exe` emitters) consume the [`Program`] it produces.

mod bundle;
mod include;
mod search;

pub use bundle::{BundlePlan, BundleUnit, IncludeSplice, ResolvedImport, ResolvedInclude};
pub use include::IncludeReport;
pub use search::{Builtins, SearchPath};

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use ahkbuild_syntax::tree_sitter::TreeCursor;

use ahkbuild_ir::node::{ImportSource, Module};
use ahkbuild_ir::{GroupId, Lowering, NodeId, NodeKind, Program};
use anyhow::{anyhow, ensure, Context, Result};

/// Read a source file and resolve its continuation sections before it enters the
/// pipeline. Doing this at the raw-text stage keeps the stored/parsed text and the
/// caller's own copy (used to slice `#Include` offsets) byte-identical.
pub(crate) fn read_source(path: &Path) -> Result<String> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    ahkbuild_preprocess::run(&path.to_string_lossy(), &raw)
}

/// The result of linking: the assembled program, a backend-neutral [`BundlePlan`], and
/// non-fatal diagnostics (unresolved imports, missing sub-modules).
pub struct LinkOutput {
    pub program: Program,
    pub plan: BundlePlan,
    pub warnings: Vec<String>,
}

/// One `#Import`'s resolution intent, recorded during the breadth-first walk and finished into
/// a [`ResolvedImport`] once every group is lowered (so module nodes exist to point at).
enum Resolution {
    /// Resolved to a file: the canonical `path` (a bundled group) and, for a path-qualified
    /// `Path:Module` import, the target `submodule` name (else the group's primary `__Main`).
    File {
        node: NodeId,
        importer: PathBuf,
        path: PathBuf,
        submodule: Option<String>,
    },
    /// An in-group `#Module` reference: `name` is a module defined in the importing `group`.
    InGroup {
        node: NodeId,
        group: GroupId,
        name: String,
    },
}

/// Link `entry` and everything it transitively `#Import`s into one multi-group [`Program`].
///
/// Files are loaded breadth-first and deduped by canonical path, so a module imported from
/// several places is lowered once. Imports that name no file — embedded `*RESNAME` and the
/// built-in `AHK` module — are skipped silently; in-group `#Module` references and
/// path-qualified `Path:Module` sub-module imports resolve to a specific module.
pub fn link_entry(entry: &Path, search: &SearchPath) -> Result<LinkOutput> {
    let mut lowering = Lowering::new();
    let mut loaded: HashMap<PathBuf, GroupId> = HashMap::new();
    let mut warnings = Vec::new();
    // Canonical path each group was loaded from, indexed by group order. Used to assign each
    // group's primary module a name from its file.
    let mut group_paths: Vec<PathBuf> = Vec::new();
    // Each `#Import` directive's resolution intent, finished after lowering completes.
    let mut resolutions: Vec<Resolution> = Vec::new();
    // Every `#Include` outcome, merged across groups (keys are globally-unique `(FileId, off)`).
    let mut include_outcomes: HashMap<(ahkbuild_ir::FileId, u32), IncludeSplice> = HashMap::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();

    let entry = canonical(entry).with_context(|| format!("entry script {}", entry.display()))?;
    // Built-ins for `#Include` resolution (`<Lib>` dirs, `%A_…%` expansion). The interpreter is
    // not running, so `A_ScriptDir` is the entry's directory and `A_AhkPath` is unknown.
    let builtins = Builtins::detect(
        entry
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    );
    queue.push_back(entry);

    let mut errors: Vec<String> = Vec::new();

    while let Some(path) = queue.pop_front() {
        if loaded.contains_key(&path) {
            continue;
        }
        let text = read_source(&path)?;
        // Load + parse the file, resolve its `#Include` graph (loading included files into the
        // shared source map), then lower the group splicing those files in.
        let (entry_file, entry_tree) = lowering
            .load(path.to_string_lossy().into_owned(), text.clone())
            .ok_or_else(|| anyhow!("parser returned no tree for {}", path.display()))?;

        // Collect sytntax errors if we have any for later display
        if entry_tree.root_node().has_error() {
            let mut cursor = entry_tree.root_node().walk();
            collect_errors(&mut cursor, &mut errors, &path.display().to_string());
            break;
        }

        let report = include::resolve_includes(
            &mut lowering,
            &builtins,
            entry_file,
            &path,
            &text,
            &entry_tree,
        )?;
        let splices = report.splices();
        include_outcomes.extend(report.outcomes);
        warnings.extend(report.warnings);
        let gid = lowering
            .lower_group(entry_file, &splices)
            .ok_or_else(|| anyhow!("parser returned no tree for {}", path.display()))?;
        loaded.insert(path.clone(), gid);
        group_paths.push(path.clone());

        // Names defined in this group: an `#Import` of one of these refers to an in-group
        // module, not a file, and in-file modules take precedence over the filesystem.
        let module_names_lc: HashSet<String> = lowering
            .group_module_names(gid)
            .iter()
            .map(|n| n.to_ascii_lowercase())
            .collect();

        let importer_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        for imp in lowering.group_imports(gid) {
            // Embedded resources (`*RESNAME`) have no file to resolve.
            if imp.spec.starts_with('*') {
                continue;
            }
            let spec_lc = imp.spec.to_ascii_lowercase();
            // In-file `#Module` reference: resolves to that module, not a file.
            if module_names_lc.contains(&spec_lc) {
                resolutions.push(Resolution::InGroup {
                    node: imp.node,
                    group: gid,
                    name: imp.spec.clone(),
                });
                continue;
            }
            // The built-in `AHK` module: no file.
            if spec_lc == "ahk" {
                continue;
            }
            // A quoted spec may be path-qualified (`Path:Module`); an unquoted spec is a bare
            // module name resolved as a file.
            let (path_spec, submodule) = if imp.quoted {
                let (p, m) = split_module_qualifier(&imp.spec);
                (p, m.map(str::to_string))
            } else {
                (imp.spec.as_str(), None)
            };
            match search.resolve(path_spec, &importer_dir) {
                Some(target) => match canonical(&target) {
                    Ok(c) => {
                        if !loaded.contains_key(&c) {
                            queue.push_back(c.clone());
                        }
                        resolutions.push(Resolution::File {
                            node: imp.node,
                            importer: path.clone(),
                            path: c,
                            submodule,
                        });
                    }
                    Err(e) => warnings.push(format!("{}: {e:#}", target.display())),
                },
                None => warnings.push(format!(
                    "{}: unresolved import \"{}\"",
                    path.display(),
                    imp.spec
                )),
            }
        }
    }

    ensure!(
        errors.is_empty(),
        anyhow!(
            "Encountered {} error(s) parsing file(s): {:#?}",
            errors.len(),
            errors
        )
    );

    let program = lowering.finish();

    // `(group, lowercased module name) -> module node`, for resolving path-qualified and
    // in-group import targets. First definition wins (a within-group reopen merges).
    let mut registry: HashMap<(GroupId, String), NodeId> = HashMap::new();
    for group in &program.groups {
        for &mid in &group.modules {
            if let NodeKind::Module(m) = &program.arena[mid].kind {
                registry
                    .entry((group.id, m.name.to_ascii_lowercase()))
                    .or_insert(mid);
            }
        }
    }

    let mut resolved_imports = Vec::new();
    for res in resolutions {
        match res {
            Resolution::File {
                node,
                importer,
                path,
                submodule,
            } => {
                let Some(&group) = loaded.get(&path) else {
                    continue;
                };
                let module = match submodule {
                    None => {
                        // Plain file import -> the group's primary `__Main` module.
                        match program.groups[group.0 as usize].modules.first() {
                            Some(&m) => m,
                            None => continue,
                        }
                    }
                    Some(name) => match registry.get(&(group, name.to_ascii_lowercase())) {
                        Some(&m) => m,
                        None => {
                            warnings.push(format!(
                                "{}: sub-module \"{}\" not found in \"{}\"",
                                importer.display(),
                                name,
                                path.display()
                            ));
                            continue;
                        }
                    },
                };
                resolved_imports.push(ResolvedImport {
                    node,
                    group,
                    module,
                });
            }
            Resolution::InGroup { node, group, name } => {
                if let Some(&module) = registry.get(&(group, name.to_ascii_lowercase())) {
                    resolved_imports.push(ResolvedImport {
                        node,
                        group,
                        module,
                    });
                }
            }
        }
    }

    // Join each lowered `IncludeDirective` node to its resolution outcome. The directive's span
    // start, minus its file's base, is the file-relative offset the resolver keyed on.
    let mut resolved_includes = Vec::new();
    for (node_id, node) in program.arena.iter() {
        if !matches!(node.kind, NodeKind::IncludeDirective(_)) {
            continue;
        }
        let file = program.sources.file_at(node.span.start);
        let off = node.span.start - file.base;
        if let Some(&splice) = include_outcomes.get(&(file.id, off)) {
            resolved_includes.push(ResolvedInclude {
                node: node_id,
                splice,
            });
        }
    }

    let module_names = assign_module_names(&program, &group_paths);
    let units = program
        .groups
        .iter()
        .map(|g| BundleUnit { group: g.id })
        .collect();

    Ok(LinkOutput {
        program,
        plan: BundlePlan {
            units,
            resolved_imports,
            module_names,
            resolved_includes,
        },
        warnings,
    })
}

/// Re-link an already-bundled, single self-contained [`Program`] - the output of a prior emit
/// round, re-parsed and re-lowered - into a fresh [`BundlePlan`], with **no file IO**.
///
/// A bundle is post-link: its `#Include`s are already spliced away, its module names are already
/// final and unique, and every surviving `#Import` is an **in-group** reference to a `#Module`
/// defined in the same program. So this only rebuilds the in-group import resolution (the
/// `registry` + [`Resolution::InGroup`] half of [`link_entry`]); `module_names` is the identity
/// of each module's own name and `resolved_includes` is empty. The fixpoint driver's outer
/// (re-parse) loop calls this for every round after the first.
pub fn link_bundle(program: Program) -> LinkOutput {
    let mut warnings = Vec::new();

    // `(group, lowercased module name) -> module node`, for resolving in-group import targets.
    // First definition wins (a within-group reopen merges).
    let mut registry: HashMap<(GroupId, String), NodeId> = HashMap::new();
    // Module names are already final in a bundle: the plan's name map is the identity of each
    // module's own name (entry primary stays `__Main`, emitted headerless).
    let mut module_names: HashMap<(GroupId, NodeId), String> = HashMap::new();
    for group in &program.groups {
        for &mid in &group.modules {
            if let NodeKind::Module(m) = &program.arena[mid].kind {
                registry
                    .entry((group.id, m.name.to_ascii_lowercase()))
                    .or_insert(mid);
                module_names.insert((group.id, mid), m.name.clone());
            }
        }
    }

    // Resolve every `#Import` to the in-group `#Module` it names, walking module bodies so each
    // directive's owning group is known directly.
    let mut resolved_imports = Vec::new();
    for group in &program.groups {
        for &mid in &group.modules {
            let NodeKind::Module(m) = &program.arena[mid].kind else {
                continue;
            };
            for &stmt in &m.body {
                let NodeKind::ImportDirective(directive) = &program.arena[stmt].kind else {
                    continue;
                };
                let spec_span = match &directive.source {
                    ImportSource::Name(s) | ImportSource::Path(s) => *s,
                };
                let spec = program.span_text(spec_span);
                // Embedded resources (`*RESNAME`) and the built-in `AHK` module name no in-group
                // module; they stay as written, exactly as the file linker skips them.
                if spec.starts_with('*') || spec.eq_ignore_ascii_case("ahk") {
                    continue;
                }
                match registry.get(&(group.id, spec.to_ascii_lowercase())) {
                    Some(&module) => resolved_imports.push(ResolvedImport {
                        node: stmt,
                        group: group.id,
                        module,
                    }),
                    None => warnings.push(format!("bundle: unresolved in-group import \"{spec}\"")),
                }
            }
        }
    }

    let units = program
        .groups
        .iter()
        .map(|g| BundleUnit { group: g.id })
        .collect();

    LinkOutput {
        program,
        plan: BundlePlan {
            units,
            resolved_imports,
            module_names,
            resolved_includes: Vec::new(),
        },
        warnings,
    }
}

/// Split a quoted import spec into `(path, Some(submodule))` for a path-qualified
/// `Path:Module` import (v2.1-alpha.21+), or `(spec, None)` otherwise. The sub-module suffix
/// is the text after the **last** `:`, and only counts if it is a bare identifier — no path
/// separators — so a drive-letter colon (`C:\dir\file.ahk`) is not mistaken for a qualifier.
fn split_module_qualifier(spec: &str) -> (&str, Option<&str>) {
    if let Some(idx) = spec.rfind(':') {
        let (path, rest) = (&spec[..idx], &spec[idx + 1..]);
        let mut chars = rest.chars();
        let valid = match chars.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            }
            _ => false,
        };
        if valid && !path.is_empty() {
            return (path, Some(rest));
        }
    }
    (spec, None)
}

/// Assign every module a program-unique output name, keyed by `(group, module node)`. The
/// entry group's primary keeps `__Main` (emitted headerless); every imported group's primary
/// is named from its file, and every written `#Module` sub-module keeps its name. Groups are
/// processed in order (entry first), so the main script's modules keep their names and later
/// collisions get a numeric suffix.
fn assign_module_names(
    program: &Program,
    group_paths: &[PathBuf],
) -> HashMap<(GroupId, NodeId), String> {
    // Reserve the implicit module and the built-in `AHK` module so we never collide with them.
    let mut used: HashSet<String> = HashSet::new();
    used.insert("__main".to_string());
    used.insert("ahk".to_string());

    let mut names = HashMap::new();
    for (gi, group) in program.groups.iter().enumerate() {
        for (mi, &mid) in group.modules.iter().enumerate() {
            let NodeKind::Module(module) = &program.arena[mid].kind else {
                continue;
            };
            if gi == 0 && mi == 0 {
                // The entry group's primary stays the implicit `__Main` (no header emitted).
                names.insert((group.id, mid), Module::MAIN.to_string());
                continue;
            }
            let base = if mi == 0 {
                // An imported group's primary: name it from its file.
                module_base_name(&group_paths[gi])
            } else {
                // A written `#Module` sub-module: keep its name.
                module.name.clone()
            };
            let mut name = base.clone();
            let mut n = 2;
            while used.contains(&name.to_ascii_lowercase()) {
                name = format!("{base}_{n}");
                n += 1;
            }
            used.insert(name.to_ascii_lowercase());
            names.insert((group.id, mid), name);
        }
    }
    names
}

/// The base module name for a file: its stem, or the parent directory name for a package
/// `__Init.ahk`, sanitized to a valid AHK identifier.
fn module_base_name(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let base = if stem.eq_ignore_ascii_case("__init") {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or(stem)
    } else {
        stem
    };
    sanitize_ident(base)
}

/// Coerce arbitrary text into a valid AHK module name: ASCII alphanumerics kept (others
/// become `_`), and a leading letter guaranteed (module names may not start with a digit or
/// underscore), so a file like `3d-utils.ahk` becomes `M3d_utils`.
fn sanitize_ident(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if !s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        s.insert(0, 'M');
    }
    s
}

fn canonical(p: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(p).with_context(|| format!("resolving {}", p.display()))
}

/// Collect all of the errors for the subtree of `cursor` into a vec of strings
/// for later display
fn collect_errors(cursor: &mut TreeCursor, errors: &mut Vec<String>, path: &str) {
    let node = cursor.node();

    // Check if this specific node is an ERROR or MISSING node
    if node.is_error() {
        let start = node.start_position();
        errors.push(format!(
            "{} - ERROR at line {}, col {}",
            path, start.row, start.column
        ));
    } else if node.is_missing() {
        let start = node.start_position();
        errors.push(format!(
            "{} - MISSING node at line {}, col {}",
            path, start.row, start.column
        ));
    }

    // Only recurse into branches that actually contain errors to optimize speed
    if node.has_error() && cursor.goto_first_child() {
        loop {
            collect_errors(cursor, errors, path);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}
