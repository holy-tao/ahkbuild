//! Libraries for managing interpreters. Ahkbuild maintains a cache of interpreters
//! at ~/.ahkbuild/interpreters/<version>/ - each version file contains one or more
//! of AutoHotkey32.exe and AutoHotkey64.exe.
//!
//! Users manage these with the `ahkbuild interpret` cli command.

mod version;

pub use version::AhkVersion;
