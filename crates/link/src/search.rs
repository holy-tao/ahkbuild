//! The v2.1 module search path.
//!
//! Mirrors how the interpreter resolves `#Import Name` to a file: the directory of the
//! importing file is searched first, then the directories from the `AhkImportPath`
//! environment variable (semicolon-separated, with `%A_…%` built-ins expanded and items
//! relative to `A_ScriptDir`), or — if `AhkImportPath` is unset — the default list
//! `%A_ScriptDir%;%A_MyDocuments%\AutoHotkey;%A_AhkPath%\..`. Within each directory the
//! lookup order is `Name`, `Name\__Init.ahk`, `Name.ahk`.
//!
//! See https://www.autohotkey.com/docs/alpha/Modules.htm#Search_Path

use std::path::{Path, PathBuf};

/// The default `AhkImportPath` list, used when the environment variable is unset.
const DEFAULT_LIST: &str = r"%A_ScriptDir%;%A_MyDocuments%\AutoHotkey;%A_AhkPath%\..";

/// AutoHotkey built-in variables that parameterize the search path. The bundler is not the
/// interpreter, so these are supplied/detected rather than read from a running script.
#[derive(Clone, Debug)]
pub struct Builtins {
    /// `A_ScriptDir` — the **main** script's directory (a global; it does not change per
    /// module). Used to expand `%A_ScriptDir%` and to resolve relative `AhkImportPath`
    /// items. Note this is *not* how a module file's own relative imports resolve: the
    /// directory of the file containing a given `#Import` is always searched first, via the
    /// per-import `importer_dir` argument to [`SearchPath::resolve`].
    pub script_dir: PathBuf,
    /// `A_MyDocuments`.
    pub my_documents: PathBuf,
    /// `A_AhkPath` — the interpreter executable. `None` if unknown, in which case any
    /// search item referencing it (e.g. the default list's `%A_AhkPath%\..`) is dropped.
    pub ahk_path: Option<PathBuf>,
}

impl Builtins {
    /// Detect built-ins from the environment, given the entry script's directory.
    /// `A_AhkPath` is left unknown (the bundler does not run under the interpreter).
    pub fn detect(script_dir: impl Into<PathBuf>) -> Builtins {
        let my_documents = std::env::var_os("USERPROFILE")
            .map(|p| PathBuf::from(p).join("Documents"))
            .unwrap_or_else(|| PathBuf::from("Documents"));
        Builtins {
            script_dir: script_dir.into(),
            my_documents,
            ahk_path: None,
        }
    }

    /// Expand `%A_ScriptDir%` / `%A_MyDocuments%` / `%A_AhkPath%` (case-insensitive) in a
    /// search-path item. Percent signs that are not a recognized built-in reference are
    /// kept literally, matching the documented behavior.
    fn expand(&self, value: &str) -> String {
        let mut out = String::new();
        let mut rest = value;
        while let Some(open) = rest.find('%') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            match after.find('%') {
                Some(close) => {
                    let name = &after[..close];
                    match self.lookup(name) {
                        Some(sub) => out.push_str(&sub),
                        None => {
                            // Not a known built-in: keep the `%name%` literally.
                            out.push('%');
                            out.push_str(name);
                            out.push('%');
                        }
                    }
                    rest = &after[close + 1..];
                }
                None => {
                    // Unterminated `%`: literal.
                    out.push('%');
                    out.push_str(after);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    fn lookup(&self, name: &str) -> Option<String> {
        let path = if name.eq_ignore_ascii_case("A_ScriptDir") {
            self.script_dir.clone()
        } else if name.eq_ignore_ascii_case("A_MyDocuments") {
            self.my_documents.clone()
        } else if name.eq_ignore_ascii_case("A_AhkPath") {
            self.ahk_path.clone()?
        } else {
            return env_builtin(name);
        };
        Some(path.to_string_lossy().into_owned())
    }

    /// Expand built-in variables in a `#Include` path. Like [`expand`](Self::expand) but also
    /// resolves `%A_LineFile%` to `line_file` (the full path of the file containing the
    /// directive), which is per-directive and so not part of the fixed search-path expansion.
    pub fn expand_include(&self, value: &str, line_file: &Path) -> String {
        let line_file = line_file.to_string_lossy();
        let mut out = String::with_capacity(value.len());
        let mut rest = value;
        while let Some(open) = rest.find('%') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            match after.find('%') {
                Some(close) => {
                    let name = &after[..close];
                    if name.eq_ignore_ascii_case("A_LineFile") {
                        out.push_str(&line_file);
                    } else {
                        // Defer everything else to `expand`'s known-builtins handling.
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                    rest = &after[close + 1..];
                }
                None => {
                    out.push('%');
                    out.push_str(after);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        self.expand(&out)
    }

    /// The `<LibName>` library search directories, in order: local (`<ScriptDir>\Lib`), user
    /// (`<MyDocuments>\AutoHotkey\Lib`), and — if `A_AhkPath` is known — standard
    /// (`<AhkDir>\Lib`).
    pub fn lib_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![
            self.script_dir.join("Lib"),
            self.my_documents.join("AutoHotkey").join("Lib"),
        ];
        if let Some(parent) = self.ahk_path.as_ref().and_then(|p| p.parent()) {
            dirs.push(parent.join("Lib"));
        }
        dirs
    }
}

/// Resolve the remaining environment-backed `A_*` built-ins usable in a `#Include` path.
/// Returns `None` when the backing environment variable is unset, so the `%A_…%` is kept
/// literal (and the include then fails to resolve, with a warning).
fn env_builtin(name: &str) -> Option<String> {
    let var = if name.eq_ignore_ascii_case("A_AppData") {
        "APPDATA"
    } else if name.eq_ignore_ascii_case("A_AppDataCommon") {
        "ALLUSERSPROFILE"
    } else if name.eq_ignore_ascii_case("A_Temp") {
        "TEMP"
    } else if name.eq_ignore_ascii_case("A_WinDir") {
        "WINDIR"
    } else if name.eq_ignore_ascii_case("A_ComSpec") {
        "ComSpec"
    } else if name.eq_ignore_ascii_case("A_ProgramFiles") {
        "ProgramFiles"
    } else {
        return None;
    };
    std::env::var(var).ok()
}

/// The resolved, fixed search directories (those after the per-import importer dir).
#[derive(Clone, Debug)]
pub struct SearchPath {
    dirs: Vec<PathBuf>,
}

impl SearchPath {
    /// Build from the process environment's `AhkImportPath` (or the default list if unset).
    pub fn from_env(builtins: &Builtins) -> SearchPath {
        let value = std::env::var("AhkImportPath").ok();
        SearchPath::build(builtins, value.as_deref())
    }

    /// Build from an explicit `AhkImportPath` value (`None` = use the default list). Pure,
    /// so it is unit-testable without touching the environment.
    pub fn build(builtins: &Builtins, ahk_import_path: Option<&str>) -> SearchPath {
        let list = ahk_import_path.unwrap_or(DEFAULT_LIST);
        let mut dirs = Vec::new();
        for item in list.split(';') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let expanded = builtins.expand(item);
            // An item still containing `%` referenced an unknown built-in (e.g. A_AhkPath
            // when undetected) — it can't name a real directory, so drop it.
            if expanded.contains('%') {
                continue;
            }
            let p = PathBuf::from(&expanded);
            dirs.push(if p.is_absolute() {
                p
            } else {
                builtins.script_dir.join(p)
            });
        }
        SearchPath { dirs }
    }

    /// Build directly from a list of directories (for non-interpreter use and tests).
    pub fn from_dirs(dirs: impl IntoIterator<Item = PathBuf>) -> SearchPath {
        SearchPath {
            dirs: dirs.into_iter().collect(),
        }
    }

    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// Resolve a module name to a file. `importer_dir` — the directory of the file
    /// containing the `#Import` — is searched first (so a module file's relative imports
    /// resolve against its own location), then the fixed dirs. Within a directory the order
    /// is `Name`, then `Name\__Init.ahk`, then `Name.ahk`.
    pub fn resolve(&self, name: &str, importer_dir: &Path) -> Option<PathBuf> {
        std::iter::once(importer_dir)
            .chain(self.dirs.iter().map(PathBuf::as_path))
            .find_map(|dir| Self::lookup_in(dir, name))
    }

    fn lookup_in(dir: &Path, name: &str) -> Option<PathBuf> {
        let exact = dir.join(name);
        if exact.is_file() {
            return Some(exact);
        }
        let init = exact.join("__Init.ahk");
        if init.is_file() {
            return Some(init);
        }
        let with_ext = dir.join(format!("{name}.ahk"));
        if with_ext.is_file() {
            return Some(with_ext);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtins() -> Builtins {
        Builtins {
            script_dir: PathBuf::from("/proj"),
            my_documents: PathBuf::from("/home/docs"),
            ahk_path: Some(PathBuf::from("/opt/ahk/AutoHotkey.exe")),
        }
    }

    #[test]
    fn default_list_expands_builtins() {
        let dirs = SearchPath::build(&builtins(), None);
        let got: Vec<_> = dirs
            .dirs()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(
            got,
            vec![
                "/proj",
                "/home/docs/AutoHotkey",
                "/opt/ahk/AutoHotkey.exe/.."
            ]
        );
    }

    #[test]
    fn ahk_import_path_splits_expands_and_resolves_relative() {
        let sp = SearchPath::build(&builtins(), Some("%A_ScriptDir%;libs;/abs/dir"));
        let got: Vec<_> = sp
            .dirs()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        // bare `libs` is relative -> joined to A_ScriptDir; /abs/dir kept absolute.
        assert_eq!(got, vec!["/proj", "/proj/libs", "/abs/dir"]);
    }

    #[test]
    fn unknown_builtin_item_is_dropped() {
        let mut b = builtins();
        b.ahk_path = None; // A_AhkPath undetected
        let sp = SearchPath::build(&b, None);
        let got: Vec<_> = sp
            .dirs()
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        // The %A_AhkPath%\.. entry is dropped; the other two remain.
        assert_eq!(got, vec!["/proj", "/home/docs/AutoHotkey"]);
    }

    #[test]
    fn stray_percent_is_literal() {
        let b = builtins();
        assert_eq!(b.expand("100%done"), "100%done");
        assert_eq!(b.expand("%A_ScriptDir%/x"), "/proj/x");
    }
}
