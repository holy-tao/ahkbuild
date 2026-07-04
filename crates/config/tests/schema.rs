//! Drift guard for the published JSON schema (`site/static/schema/ahkbuild.schema.json`).
//!
//! The schema is hand-maintained (see the crate's custom `Deserialize` impls, which a schema
//! generator can't see through), so it can silently diverge from what this crate actually accepts.
//! These tests pin it against a corpus of configs:
//!
//! - every config we consider *valid* must pass the schema (catches a schema that is too strict -
//!   the main drift risk when a new field is added to the crate but not the schema);
//! - a curated set of *invalid* configs must fail the schema (catches a schema that is too loose on
//!   the constraints it is meant to encode: exactly-one dependency source, source-specific fields,
//!   non-empty script argv, define-name rules).

use jsonschema::Validator;
use serde_json::{json, Value};

const SCHEMA_SRC: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../site/static/schema/ahkbuild.schema.json"
));

fn validator() -> Validator {
    let schema: Value = serde_json::from_str(SCHEMA_SRC).expect("schema is valid JSON");
    jsonschema::validator_for(&schema).expect("schema compiles")
}

/// Every config here must be accepted by both the schema and the crate's deserializer.
fn valid_configs() -> Vec<Value> {
    vec![
        // Minimal: just the required interpreter block.
        json!({ "interpreter": { "version": "2.1-alpha.27" } }),
        // A stable release version with explicit bitness.
        json!({ "interpreter": { "version": "2.0.26", "bitness": 32 } }),
        // The full example from the reference docs, exercising exe/resources/scripts/defines.
        json!({
            "entry": "src/main.ahk",
            "interpreter": { "version": "2.1-alpha.27", "bitness": 64 },
            "exe": {
                "name": "MyApp",
                "version": "1.2.3.0",
                "description": "My application",
                "copyright": "Copyright 2026 Example",
                "company": "Example, LLC",
                "trademarks": "MyApp is a trademark of Example, LLC",
                "comments": "Built with ahkbuild",
                "icon": "assets/icon.ico",
                "subsystem": "gui",
                "manifest": {
                    "uac": "requireAdministrator",
                    "dpiAwareness": "PerMonitorV2",
                    "longPathAware": true,
                    "gdiScaling": true
                }
            },
            "resources": {
                "icons": [ { "path": "assets/extra.ico", "id": 300 } ],
                "extra": [
                    { "name": "HELP", "type": "RT_HTML", "path": "assets/help.html" },
                    { "name": "ABOUT", "type": 23, "path": "assets/about.html" }
                ]
            },
            "scripts": {
                "pre-bundle": [ "./generate.exe", ["${AHK}", "scripts/codegen.ahk"] ],
                "post-bundle": [ { "command": ["upx", "--best", "${AHKBUILD_OUTPUT}"] } ]
            },
            "defines": { "DEBUG": 1, "RATIO": 1.5, "FLAG": true, "MODE": "release" }
        }),
        // One dependency of every source kind (mirrors the dependencies.rs parse tests).
        json!({
            "interpreter": { "version": "2.1-alpha.27" },
            "dependencies": {
                "GuiEnhancer": { "git": "https://github.com/x/y.git", "tag": "v1.0.3" },
                "OnGit":       { "git": "https://gitlab.com/x/y.git" },
                "cJson":       { "gist": "abc123", "rev": "deadbeef" },
                "Rapid":       { "tarball": "https://e.com/r.zip", "sha256": "ff", "subdir": "src" },
                "YAML64.ahk":  { "release": "holy-tao/YAML", "tag": "v0.5.0", "asset": "YAML64.ahk", "sha256": "ff", "alias": "YAML" },
                "MyLocal":     { "path": "../shared/MyLocal" }
            }
        }),
    ]
}

/// Every config here must be rejected by the schema. Each targets a constraint the schema is
/// responsible for encoding (and which the crate also rejects - see the cited tests).
fn invalid_configs() -> Vec<(&'static str, Value)> {
    let dep = |spec: Value| {
        json!({
            "interpreter": { "version": "2.1-alpha.27" },
            "dependencies": { "X": spec }
        })
    };
    vec![
        ("dependency with no source", dep(json!({ "tag": "v1" }))),
        (
            "dependency with conflicting sources",
            dep(json!({ "git": "u", "path": "p" })),
        ),
        (
            "sha256 on a git source",
            dep(json!({ "git": "u", "sha256": "ff" })),
        ),
        (
            "asset on a git source",
            dep(json!({ "git": "u", "asset": "a.ahk" })),
        ),
        ("tarball missing sha256", dep(json!({ "tarball": "u" }))),
        (
            "release missing asset",
            dep(json!({ "release": "o/r", "tag": "v1", "sha256": "ff" })),
        ),
        (
            "release asset with a path separator",
            dep(json!({ "release": "o/r", "tag": "v1", "asset": "sub/a.ahk", "sha256": "ff" })),
        ),
        (
            "alias that is not an identifier",
            dep(json!({ "git": "u", "alias": "not.valid" })),
        ),
        // Empty script argv (crate: empty_script_command_rejected).
        (
            "empty script argv",
            json!({ "interpreter": { "version": "2.1-alpha.27" }, "scripts": { "post-bundle": [[]] } }),
        ),
        // Define-name rules (crate: reserved/invalid define name rejected in defines_env()).
        (
            "reserved define name",
            json!({ "interpreter": { "version": "2.1-alpha.27" }, "defines": { "AHKBUILD_OUTPUT": "nope" } }),
        ),
        (
            "invalid define name",
            json!({ "interpreter": { "version": "2.1-alpha.27" }, "defines": { "1BAD": "nope" } }),
        ),
        // Missing the one required top-level block.
        ("missing interpreter", json!({ "entry": "main.ahk" })),
    ]
}

#[test]
fn schema_accepts_every_valid_config() {
    let validator = validator();
    for cfg in valid_configs() {
        if let Err(err) = validator.validate(&cfg) {
            panic!("schema rejected a valid config: {err}\n{cfg:#}");
        }
    }
}

#[test]
fn schema_rejects_every_invalid_config() {
    let validator = validator();
    for (why, cfg) in invalid_configs() {
        assert!(
            !validator.is_valid(&cfg),
            "schema accepted an invalid config ({why}):\n{cfg:#}"
        );
    }
}
