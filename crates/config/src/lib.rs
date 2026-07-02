//! Project configuration parsed from `ahkbuild.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

mod dependencies;

pub use ahkbuild_interpret::{AhkVersion, Bitness};
pub use dependencies::{DependencySource, DependencySpec, GitSelector};

#[derive(Debug, Deserialize)]
pub struct BuildConfig {
    pub entry: Option<PathBuf>,
    pub interpreter: InterpreterConfig,
    #[serde(default)]
    pub exe: ExeConfig,
    #[serde(default)]
    pub resources: ResourcesConfig,
    #[serde(default)]
    pub scripts: ScriptsConfig,
    /// User-defined build variables, exposed to build scripts as environment variables and
    /// (forward-compat) reserved to feed the preprocessor/fold pass for conditional compilation.
    #[serde(default)]
    pub defines: BTreeMap<String, DefineValue>,
    /// Module dependencies, keyed by the logical import name written in `#Import Name`. Each value
    /// names exactly one source (git / gist / tarball / path). Resolved and pinned by
    /// `ahkbuild package restore`; see `crates/pkg`.
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
}

impl BuildConfig {
    /// Apply CLI overrides on top of the parsed config values.
    pub fn merge_cli(
        &mut self,
        entry: Option<PathBuf>,
        interpreter_version: Option<AhkVersion>,
        bitness: Option<Bitness>,
    ) {
        if let Some(e) = entry {
            self.entry = Some(e);
        }
        if let Some(v) = interpreter_version {
            self.interpreter.version = v;
        }
        if let Some(b) = bitness {
            self.interpreter.bitness = b;
        }
    }

    /// Validate and stringify `defines` into a name -> value map for build-script environments and
    /// `${NAME}` argv substitution. Names must be valid identifiers (`[A-Za-z_][A-Za-z0-9_]*`) and
    /// must not collide with the reserved `AHKBUILD_` prefix.
    pub fn defines_env(&self) -> Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        for (name, value) in &self.defines {
            if !is_valid_define_name(name) {
                bail!(
                    "invalid define name {name:?}: must match [A-Za-z_][A-Za-z0-9_]* and not start \
                     with the reserved 'AHKBUILD_' prefix"
                );
            }
            out.insert(name.clone(), value.to_env_string());
        }
        Ok(out)
    }
}

fn is_valid_define_name(name: &str) -> bool {
    if name.starts_with("AHKBUILD_") {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A build-variable value. Accepts a JSON string, integer, float, or boolean; everything is
/// rendered to a flat string when handed to build scripts.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum DefineValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl DefineValue {
    /// Render the value as the string a build script sees (e.g. `1`, `true`, `An arbitrary string`).
    pub fn to_env_string(&self) -> String {
        match self {
            DefineValue::Bool(b) => b.to_string(),
            DefineValue::Int(n) => n.to_string(),
            DefineValue::Float(f) => f.to_string(),
            DefineValue::Str(s) => s.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct InterpreterConfig {
    #[serde(deserialize_with = "deser_ahk_version")]
    pub version: AhkVersion,
    #[serde(default = "default_bitness", deserialize_with = "deser_bitness")]
    pub bitness: Bitness,
}

fn default_bitness() -> Bitness {
    Bitness::X64
}

fn deser_ahk_version<'de, D: serde::Deserializer<'de>>(d: D) -> Result<AhkVersion, D::Error> {
    let s = String::deserialize(d)?;
    s.parse::<AhkVersion>().map_err(serde::de::Error::custom)
}

fn deser_bitness<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Bitness, D::Error> {
    let n = u32::deserialize(d)?;
    match n {
        32 => Ok(Bitness::X32),
        64 => Ok(Bitness::X64),
        other => Err(serde::de::Error::custom(format!(
            "expected 32 or 64, got {other}"
        ))),
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct ExeConfig {
    pub name: Option<String>,
    /// Four-part version string, e.g. "1.2.3.0". Defaults to "0.0.0.0" if omitted.
    pub version: Option<String>,
    pub description: Option<String>,
    pub copyright: Option<String>,
    /// `CompanyName` string-table entry. Left as the interpreter's value if omitted.
    pub company: Option<String>,
    /// `LegalTrademarks` string-table entry. Left as the interpreter's value if omitted.
    pub trademarks: Option<String>,
    /// `Comments` string-table entry. Left as the interpreter's value if omitted.
    pub comments: Option<String>,
    /// Replaces the interpreter's primary icon (RT_GROUP_ICON group ID 1).
    pub icon: Option<PathBuf>,
    #[serde(default)]
    pub subsystem: Subsystem,
    /// Application-manifest (RT_MANIFEST) overrides applied on top of the interpreter's manifest.
    #[serde(default)]
    pub manifest: ManifestConfig,
}

#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Subsystem {
    #[default]
    Gui,
    Console,
}

/// Overrides applied to the interpreter's embedded application manifest. Each field is `None` by
/// default, meaning "leave the interpreter's manifest value untouched"; only the fields the user
/// sets are surgically edited into the existing manifest (see `docs/EXE_BUNDLING.md`).
#[derive(Debug, Deserialize, Default)]
pub struct ManifestConfig {
    /// UAC requested execution level (`<requestedExecutionLevel level="...">`).
    pub uac: Option<UacLevel>,
    /// Legacy DPI-awareness flag (`<dpiAware>true|false</dpiAware>`).
    #[serde(rename = "dpiAware")]
    pub dpi_aware: Option<bool>,
    /// Modern DPI-awareness mode string, e.g. `"PerMonitorV2"` or `"system"`
    /// (`<dpiAwareness>...</dpiAwareness>`).
    #[serde(rename = "dpiAwareness")]
    pub dpi_awareness: Option<String>,
    /// Opt into long (>MAX_PATH) path support (`<longPathAware>true|false</longPathAware>`).
    #[serde(rename = "longPathAware")]
    pub long_path_aware: Option<bool>,
    /// Opt into GDI bitmap scaling under DPI virtualization (`<gdiScaling>true|false</gdiScaling>`).
    #[serde(rename = "gdiScaling")]
    pub gdi_scaling: Option<bool>,
}

impl ManifestConfig {
    /// True if no manifest override is set, so the interpreter's manifest is left as shipped (and
    /// the emitter skips the RT_MANIFEST update entirely).
    pub fn is_empty(&self) -> bool {
        self.uac.is_none()
            && self.dpi_aware.is_none()
            && self.dpi_awareness.is_none()
            && self.long_path_aware.is_none()
            && self.gdi_scaling.is_none()
    }
}

/// UAC requested execution level. Variants serialize to the exact manifest attribute strings.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum UacLevel {
    #[serde(rename = "asInvoker")]
    AsInvoker,
    #[serde(rename = "highestAvailable")]
    HighestAvailable,
    #[serde(rename = "requireAdministrator")]
    RequireAdministrator,
}

impl UacLevel {
    /// The exact `level="..."` manifest value.
    pub fn as_str(self) -> &'static str {
        match self {
            UacLevel::AsInvoker => "asInvoker",
            UacLevel::HighestAvailable => "highestAvailable",
            UacLevel::RequireAdministrator => "requireAdministrator",
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct ResourcesConfig {
    /// Additional icons embedded as new `RT_GROUP_ICON` groups under explicit resource ids.
    /// A script loads one with `LoadPicture(A_ScriptFullPath, "Icon-" id)` (the negative form
    /// addresses a group by resource id; the positive ordinal form is unstable - see
    /// `docs/EXE_BUNDLING.md`).
    #[serde(default)]
    pub icons: Vec<IconResource>,
    /// Generic extra resources to embed (embedding deferred; schema defined now).
    #[serde(default)]
    pub extra: Vec<ExtraResource>,
}

/// One additional application icon: a `.ico` file and the `RT_GROUP_ICON` resource id it is filed
/// under (the `N` in `LoadPicture(.., "Icon-N")`). The id must not collide with one of the
/// interpreter's built-in icon groups; that is validated at bundle time.
#[derive(Debug, Deserialize)]
pub struct IconResource {
    pub path: PathBuf,
    pub id: u16,
}

#[derive(Debug, Deserialize)]
pub struct ExtraResource {
    pub name: String,
    #[serde(rename = "type")]
    pub resource_type: ResourceType,
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ResourceType {
    Named(String),
    Raw(u16),
}

#[derive(Debug, Deserialize, Default)]
pub struct ScriptsConfig {
    #[serde(default, rename = "pre-bundle")]
    pub pre_bundle: Vec<BuildScript>,
    #[serde(default, rename = "post-bundle")]
    pub post_bundle: Vec<BuildScript>,
}

/// A pre- or post-bundle command run out-of-process. Stored as an argv vector (no shell), so paths
/// with spaces are safe and no quoting is required. The first token may be the builtin `${AHK}`,
/// which resolves to the configured interpreter; any `${NAME}` token is substituted from the build
/// environment (the `AHKBUILD_*` vars and user `defines`) before the command runs.
///
/// Accepts three JSON shapes, all normalized to `command`:
/// - a bare string `"./sign.exe"` - a single executable, no arguments;
/// - an array `["${AHK}", "./postbuild.ahk"]` - an explicit argv;
/// - an object `{ "command": ["upx", "--best", "${AHKBUILD_OUTPUT}"] }` - for forward-compat with
///   future per-script options.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildScript {
    pub command: Vec<String>,
}

impl<'de> Deserialize<'de> for BuildScript {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Shorthand(String),
            Argv(Vec<String>),
            Detailed { command: Vec<String> },
        }
        let command = match Repr::deserialize(d)? {
            Repr::Shorthand(s) => vec![s],
            Repr::Argv(v) => v,
            Repr::Detailed { command } => command,
        };
        if command.is_empty() {
            return Err(serde::de::Error::custom(
                "build script command must have at least one token",
            ));
        }
        Ok(BuildScript { command })
    }
}

/// Walk upward from `start` (file or directory) looking for `ahkbuild.json`.
pub fn find_config(start: &Path) -> Result<Option<PathBuf>> {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut dir = if start.is_file() {
        start.parent().map(Path::to_path_buf).unwrap_or(start)
    } else {
        start
    };
    loop {
        let candidate = dir.join("ahkbuild.json");
        if candidate.is_file() {
            tracing::info!(path = %candidate.display(), "found ahkbuild.json");
            return Ok(Some(candidate));
        }
        match dir.parent() {
            Some(p) => dir = p.to_path_buf(),
            None => return Ok(None),
        }
    }
}

/// Parse `ahkbuild.json` at `path`. Relative paths are resolved against the config
/// file's directory (the project root).
pub fn load(path: &Path) -> Result<BuildConfig> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut config: BuildConfig =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

    let root = path.parent().unwrap_or(Path::new("."));
    resolve_paths(&mut config, root);
    Ok(config)
}

fn resolve_paths(config: &mut BuildConfig, root: &Path) {
    resolve_opt(&mut config.entry, root);
    resolve_opt(&mut config.exe.icon, root);
    for ic in &mut config.resources.icons {
        resolve(&mut ic.path, root);
    }
    for r in &mut config.resources.extra {
        resolve(&mut r.path, root);
    }
    // `path` dependencies resolve against the project root, like every other config path.
    for dep in config.dependencies.values_mut() {
        if let DependencySource::Path { path } = &mut dep.source {
            resolve(path, root);
        }
    }
    // Build-script commands are argv strings, not paths: relative paths in them resolve naturally
    // against the script working directory (the project root), so nothing is rewritten here.
}

fn resolve(p: &mut PathBuf, root: &Path) {
    if p.is_relative() {
        *p = root.join(&*p);
    }
}

fn resolve_opt(opt: &mut Option<PathBuf>, root: &Path) {
    if let Some(p) = opt {
        resolve(p, root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> BuildConfig {
        serde_json::from_str(json).expect("parse failed")
    }

    #[test]
    fn minimal_config() {
        let c = parse(r#"{"interpreter": {"version": "2.1-alpha.27"}}"#);
        assert_eq!(c.interpreter.version.to_string(), "2.1-alpha.27");
        assert_eq!(c.interpreter.bitness, Bitness::X64);
        assert!(c.entry.is_none());
        assert!(c.exe.name.is_none());
    }

    #[test]
    fn explicit_bitness_32() {
        let c = parse(r#"{"interpreter": {"version": "2.0.26", "bitness": 32}}"#);
        assert_eq!(c.interpreter.bitness, Bitness::X32);
    }

    #[test]
    fn subsystem_console() {
        let c = parse(
            r#"{"interpreter": {"version": "2.1-alpha.27"}, "exe": {"subsystem": "console"}}"#,
        );
        assert_eq!(c.exe.subsystem, Subsystem::Console);
    }

    #[test]
    fn manifest_overrides_parse() {
        let c = parse(
            r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "exe": {"manifest": {
                "uac": "requireAdministrator",
                "dpiAwareness": "PerMonitorV2",
                "longPathAware": true,
                "gdiScaling": true
            }}
        }"#,
        );
        assert_eq!(c.exe.manifest.uac, Some(UacLevel::RequireAdministrator));
        assert_eq!(c.exe.manifest.dpi_aware, None);
        assert_eq!(
            c.exe.manifest.dpi_awareness.as_deref(),
            Some("PerMonitorV2")
        );
        assert_eq!(c.exe.manifest.long_path_aware, Some(true));
        assert_eq!(c.exe.manifest.gdi_scaling, Some(true));
        assert!(!c.exe.manifest.is_empty());
    }

    #[test]
    fn manifest_empty_by_default() {
        let c = parse(r#"{"interpreter": {"version": "2.1-alpha.27"}}"#);
        assert!(c.exe.manifest.is_empty());
    }

    #[test]
    fn resource_type_named_and_raw() {
        let c = parse(
            r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "resources": {
                "extra": [
                    {"name": "HELP", "type": "RT_HTML", "path": "help.html"},
                    {"name": "ABOUT", "type": 23, "path": "about.html"}
                ]
            }
        }"#,
        );
        assert_eq!(c.resources.extra.len(), 2);
        assert!(
            matches!(&c.resources.extra[0].resource_type, ResourceType::Named(s) if s == "RT_HTML")
        );
        assert!(matches!(
            &c.resources.extra[1].resource_type,
            ResourceType::Raw(23)
        ));
    }

    #[test]
    fn icons_carry_explicit_ids() {
        let c = parse(
            r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "resources": {
                "icons": [
                    {"path": "assets/a.ico", "id": 300},
                    {"path": "assets/b.ico", "id": 301}
                ]
            }
        }"#,
        );
        assert_eq!(c.resources.icons.len(), 2);
        assert_eq!(c.resources.icons[0].path, PathBuf::from("assets/a.ico"));
        assert_eq!(c.resources.icons[0].id, 300);
        assert_eq!(c.resources.icons[1].id, 301);
    }

    #[test]
    fn scripts_accept_all_three_forms() {
        let c = parse(
            r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "scripts": {
                "pre-bundle": ["./sign.exe", ["${AHK}", "pre.ahk"]],
                "post-bundle": [{"command": ["upx", "--best", "${AHKBUILD_OUTPUT}"]}]
            }
        }"#,
        );
        // bare string -> single-token argv
        assert_eq!(c.scripts.pre_bundle[0].command, vec!["./sign.exe"]);
        // array -> explicit argv with the ${AHK} builtin token
        assert_eq!(c.scripts.pre_bundle[1].command, vec!["${AHK}", "pre.ahk"]);
        // object -> command field
        assert_eq!(
            c.scripts.post_bundle[0].command,
            vec!["upx", "--best", "${AHKBUILD_OUTPUT}"]
        );
    }

    #[test]
    fn empty_script_command_rejected() {
        let err = serde_json::from_str::<BuildConfig>(
            r#"{"interpreter": {"version": "2.1-alpha.27"}, "scripts": {"post-bundle": [[]]}}"#,
        );
        assert!(err.is_err());
    }

    #[test]
    fn defines_stringify_by_type() {
        let c = parse(
            r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "defines": {"DEBUG": 1, "RATIO": 1.5, "FLAG": true, "MODE": "release"}
        }"#,
        );
        let env = c.defines_env().expect("valid defines");
        assert_eq!(env["DEBUG"], "1");
        assert_eq!(env["RATIO"], "1.5");
        assert_eq!(env["FLAG"], "true");
        assert_eq!(env["MODE"], "release");
    }

    #[test]
    fn reserved_define_name_rejected() {
        let c = parse(
            r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "defines": {"AHKBUILD_OUTPUT": "nope"}
        }"#,
        );
        assert!(c.defines_env().is_err());
    }

    #[test]
    fn invalid_define_name_rejected() {
        let c = parse(
            r#"{
            "interpreter": {"version": "2.1-alpha.27"},
            "defines": {"1BAD": "nope"}
        }"#,
        );
        assert!(c.defines_env().is_err());
    }

    #[test]
    fn path_dependency_resolves_against_root() {
        let mut c = parse(
            r#"{"interpreter": {"version": "2.1-alpha.27"},
                "dependencies": {"X": {"path": "../shared/X"}}}"#,
        );
        // A concrete absolute root for the platform under test.
        let root = std::env::current_dir().unwrap().join("proj").join("root");
        resolve_paths(&mut c, &root);
        match &c.dependencies["X"].source {
            DependencySource::Path { path } => {
                assert!(path.is_absolute(), "{} should be absolute", path.display());
                assert!(path.ends_with("shared/X") || path.ends_with("shared\\X"));
            }
            other => panic!("expected path source, got {other:?}"),
        }
    }

    #[test]
    fn merge_cli_overrides() {
        let mut c = parse(r#"{"interpreter": {"version": "2.1-alpha.27"}}"#);
        c.merge_cli(Some(PathBuf::from("other.ahk")), None, Some(Bitness::X32));
        assert_eq!(c.entry, Some(PathBuf::from("other.ahk")));
        assert_eq!(c.interpreter.bitness, Bitness::X32);
    }
}
