//! The linker's backend-neutral bundle plan.
//!
//! Linking produces a [`BundlePlan`]: an ordered list of module units (one per group, each
//! carrying the in-file module name it was assigned) plus the set of resolved imports — which
//! `#Import` directive resolved to which bundled group. Both are backend-agnostic facts: the
//! `.ahk` emitter turns a unit into a `#Module Name` block and rewrites each resolved import's
//! spec to that name; a future `.exe` emitter turns units into RCDATA resources and redirects
//! imports to `#Import "*RES"`.

use ahkbuild_ir::{GroupId, NodeId};

/// An ordered, backend-neutral description of how groups become output modules.
#[derive(Clone, Debug)]
pub struct BundlePlan {
    /// Units in emission order. `units[0]` is the entry group.
    pub units: Vec<BundleUnit>,
    /// Every `#Import` directive that resolved to a bundled group, so backends can redirect
    /// it away from the filesystem. In-group, embedded (`*RES`), path-qualified and
    /// unresolved imports are *not* listed.
    pub resolved_imports: Vec<ResolvedImport>,
}

/// One group's placement in the bundle.
#[derive(Clone, Debug)]
pub struct BundleUnit {
    pub group: GroupId,
    /// The module name this group takes in the output: `None` for the entry group (it stays
    /// the implicit `__Main`), `Some(name)` for an imported group. The name is a sanitized,
    /// program-unique identifier (valid as an AHK `#Module` name / resource name).
    pub module_name: Option<String>,
}

/// A resolved `#Import`: the directive node and the bundled group it points at. The `.ahk`
/// emitter rewrites the directive's source spec to the target group's [`BundleUnit::module_name`].
#[derive(Clone, Copy, Debug)]
pub struct ResolvedImport {
    /// The `ImportDirective` node (its source spec is what gets rewritten).
    pub node: NodeId,
    /// The bundled group this import resolves to.
    pub group: GroupId,
}
