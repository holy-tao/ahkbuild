//! The module-graph linker.
//!
//! Given an entry script, resolves its `#Import` graph across files, lowers each origin
//! into its own [`Group`](ahkbuild_ir::Group), and assembles one multi-group
//! [`Program`]. This is the IO-bearing layer above the (pure) `ir` crate; later passes
//! (import rewriting, the `.ahk` / `.exe` emitters) consume the [`Program`] it produces.

mod search;

pub use search::{Builtins, SearchPath};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use ahkbuild_ir::{GroupId, Lowering, NodeKind, Program};
use anyhow::{anyhow, Context, Result};

/// The result of linking: the assembled program plus non-fatal diagnostics (unresolved
/// imports, deferred forms, same-name groups).
pub struct LinkOutput {
    pub program: Program,
    pub warnings: Vec<String>,
}

/// Link `entry` and everything it transitively `#Import`s into one multi-group [`Program`].
///
/// Files are loaded breadth-first and deduped by canonical path, so a module imported from
/// several places is lowered once. Imports that name no file — embedded `*RESNAME` and (for
/// now) path-qualified `Path:Module` — are skipped with a diagnostic rather than failing.
pub fn link_entry(entry: &Path, search: &SearchPath) -> Result<LinkOutput> {
    let mut lowering = Lowering::new();
    let mut loaded: HashMap<PathBuf, GroupId> = HashMap::new();
    let mut warnings = Vec::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();

    let entry = canonical(entry).with_context(|| format!("entry script {}", entry.display()))?;
    queue.push_back(entry);

    while let Some(path) = queue.pop_front() {
        if loaded.contains_key(&path) {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let gid = lowering
            .add_file(path.to_string_lossy().into_owned(), text)
            .ok_or_else(|| anyhow!("parser returned no tree for {}", path.display()))?;
        loaded.insert(path.clone(), gid);

        // Names defined in this group (plus the built-in `AHK` module): an `#Import` of one
        // of these refers to an in-group module, not a file, so it is never resolved.
        let mut local: HashSet<String> = lowering
            .group_module_names(gid)
            .iter()
            .map(|n| n.to_ascii_lowercase())
            .collect();
        local.insert("ahk".to_string());

        let importer_dir = path.parent().unwrap_or_else(|| Path::new("."));
        for imp in lowering.group_imports(gid) {
            // Embedded resources (`*RESNAME`) have no file to resolve.
            if imp.spec.starts_with('*') {
                continue;
            }
            // In-group `#Module` reference or the built-in `AHK` module: no file.
            if local.contains(&imp.spec.to_ascii_lowercase()) {
                continue;
            }
            // Path-qualified sub-module imports (`Path:Module`) are not resolved yet.
            if imp.spec.contains(':') {
                warnings.push(format!(
                    "{}: path-qualified import \"{}\" not resolved yet",
                    path.display(),
                    imp.spec
                ));
                continue;
            }
            match search.resolve(&imp.spec, importer_dir) {
                Some(target) => match canonical(&target) {
                    Ok(c) if !loaded.contains_key(&c) => queue.push_back(c),
                    Ok(_) => {}
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

    let program = lowering.finish();
    warnings.extend(same_name_group_warnings(&program));
    Ok(LinkOutput { program, warnings })
}

fn canonical(p: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(p).with_context(|| format!("resolving {}", p.display()))
}

/// Warn about non-`__Main` `#Module` names defined in more than one group — the hazard the
/// single-`.ahk` emitter must rename (the interpreter keeps them isolated; see the runtime
/// probes). `__Main` is excluded: every group has one and they stay distinct per origin.
fn same_name_group_warnings(program: &Program) -> Vec<String> {
    // Keyed by the case-insensitive identity, but keep a representative original-case name
    // for the message.
    let mut counts: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for group in &program.groups {
        let mut here: BTreeMap<String, String> = BTreeMap::new();
        for &m in &group.modules {
            if let NodeKind::Module(module) = &program.arena[m].kind {
                if !module.is_main() {
                    here.entry(module.name.to_ascii_lowercase())
                        .or_insert_with(|| module.name.clone());
                }
            }
        }
        for (key, display) in here {
            let entry = counts.entry(key).or_insert((0, display));
            entry.0 += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, (c, _))| *c > 1)
        .map(|(_, (c, name))| {
            format!(
                "module \"{name}\" is defined in {c} groups; single-.ahk output would merge them"
            )
        })
        .collect()
}
