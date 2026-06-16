//! The linker's backend-neutral bundle plan.
//!
//! Linking produces a [`BundlePlan`]: an ordered list of group units (emission order), a
//! program-wide assignment of an output module name to **every** module, and the set of
//! resolved imports — which `#Import` directive resolved to which module. All three are
//! backend-agnostic facts: the `.ahk` emitter renames each module's `#Module` header to its
//! assigned name and rewrites each resolved import's spec to the target module's name; a
//! future `.exe` emitter turns groups into RCDATA resources and redirects imports to
//! `#Import "*RES"`.

use std::collections::HashMap;

use ahkbuild_ir::{GroupId, NodeId};

/// An ordered, backend-neutral description of how groups become output modules.
#[derive(Clone, Debug)]
pub struct BundlePlan {
    /// Units in emission order. `units[0]` is the entry group.
    pub units: Vec<BundleUnit>,
    /// Every `#Import` directive that resolved to a bundled module, so backends can redirect
    /// it away from the filesystem. Covers file imports, path-qualified sub-module imports
    /// (`Path:Module`), and in-group `#Module` references (which need redirecting only if the
    /// target module is renamed). Embedded (`*RES`), built-in `AHK`, and unresolved imports
    /// are *not* listed.
    pub resolved_imports: Vec<ResolvedImport>,
    /// The output module name assigned to every module in the program, keyed by
    /// `(group, module node)`. Names are program-unique (valid as an AHK `#Module` name /
    /// resource name) so the flat single-`.ahk` output never merges two distinct modules. The
    /// entry group's primary module keeps `"__Main"` and is emitted without a header.
    pub module_names: HashMap<(GroupId, NodeId), String>,
}

/// One group's placement in the bundle.
#[derive(Clone, Debug)]
pub struct BundleUnit {
    pub group: GroupId,
}

/// A resolved `#Import`: the directive node and the specific module it points at. The `.ahk`
/// emitter rewrites the directive's source spec to that module's assigned
/// [`BundlePlan::module_names`] entry.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedImport {
    /// The `ImportDirective` node (its source spec is what gets rewritten).
    pub node: NodeId,
    /// The bundled group the target module lives in.
    pub group: GroupId,
    /// The target module node within `group` (a specific sub-module for a path-qualified or
    /// in-group import; the group's primary `__Main` for a plain file import).
    pub module: NodeId,
}
