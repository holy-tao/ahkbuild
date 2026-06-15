//! The linker's backend-neutral bundle plan.
//!
//! Linking produces a [`BundlePlan`]: an ordered list of module units (one per group) and the
//! module name each gets in the output. It is deliberately backend-agnostic — the `.ahk`
//! emitter (in the `emit` crate) turns each non-entry unit into a `#Module Name` block, and a
//! future `.exe` emitter would turn the same units into RCDATA resources named `Name`.

use ahkbuild_ir::GroupId;

/// An ordered, backend-neutral description of how groups become output modules.
#[derive(Clone, Debug)]
pub struct BundlePlan {
    /// Units in emission order. `units[0]` is the entry group.
    pub units: Vec<BundleUnit>,
}

/// One group's placement in the bundle.
#[derive(Clone, Debug)]
pub struct BundleUnit {
    pub group: GroupId,
    /// The module name this group takes in the output: `None` for the entry group (it stays
    /// the implicit `__Main`), `Some(name)` for an imported group — emitted as `#Module
    /// name` (`.ahk`) or a resource named `name` (`.exe`).
    pub module_name: Option<String>,
}
