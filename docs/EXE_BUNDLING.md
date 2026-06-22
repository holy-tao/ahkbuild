# Exe Bundling

`ahkbuild bundle exe` produces a standalone Windows executable by embedding AHK source and
resources into a copy of the AHK interpreter binary. It is a superset of `bundle ahk` in intent
but uses a **different emit path** - the output is not a `.ahk` file fed into a PE wrapper; it is
a PE file assembled directly.

## How compiled AHK works

A "compiled" AHK script is an AHK interpreter binary with script files embedded as `RT_RCDATA`
resources.

- **v2.0** - the entire script is bundled into a single `.ahk` file and embedded as the RCDATA
  resource named `*1`. This is exactly the output of `bundle ahk` embedded into the interpreter.
- **v2.1** - each module is embedded as a separate RCDATA resource. The entry-point module is
  `*1`; imported modules are embedded under their module name (resource name TBD - verify against
  interpreter source). `#Import` directives are rewritten to reference the embedded resource names
  (`#Import *<name>`) rather than file paths. This means the v2.1 exe target bypasses the
  single-file concatenation that `bundle ahk` performs and uses a **parallel emit path** that walks
  the module graph and writes one resource per module.

Because the v2.1 path embeds modules under their original names, none of the module-name mangling
that `bundle ahk` does to avoid collisions in a flat file is needed.

`FileInstall` files are also embedded as RCDATA resources. In v2.0, the resource name is the
uppercased basename of the source path (e.g. `FileInstall "data\config.ini", ...` -> resource
`CONFIG.INI`). Verify whether v2.1 preserves this convention.

## Pipeline position

```text
Source -> Preprocess -> Link -> [Fold] -> [Shake] -> Emit (exe)
                                                        |
                                   PE base (interpreter) + resources -> .exe
```

The fold and shake passes are identical to `bundle ahk`. The emitter receives the `Program`,
`BundlePlan`, and optional `FoldResult`/`ShakeResult` and, instead of concatenating source spans
into a single text file, writes each module's (post-fold, post-shake) source into a PE resource
slot. A separate PE assembly step then takes the interpreter binary, injects all resources, and
writes the final `.exe`.

Build-time constants default to `A_IsCompiled = true` and `A_PtrSize = <target bitness>` for the
exe target (see [CONSTANT_FOLDING.md](CONSTANT_FOLDING.md)).

## Config file

Declarative build config lives in a `ahkbuild.json` (or `ahkbuild.yaml`) file at the project
root. JSON is preferred for portability in the AHK ecosystem; YAML is acceptable for readability.
Config covers interpreter pinning, exe metadata, resources, and build scripts. The schema is
intentionally minimal to start.

```json
{
  "entry": "src/main.ahk",
  "interpreter": {
    "version": "2.1-alpha.27",
    "bitness": 64
  },
  "exe": {
    "name": "MyApp",
    "version": "1.2.3.0",
    "description": "My application",
    "copyright": "Copyright 2026 Example",
    "icon": "assets/icon.ico",
    "subsystem": "gui"
  },
  "resources": {
    "icons": [
      "assets/icon1.ico"
    ],
  },
  "scripts": {
    "pre-bundle": [
      "scripts/pre-1.ahk",
      "scripts/pre-2.ahk"
    ],
    "post-bundle": ["scripts/post.ahk"]
  }
}
```

Open questions on config schema:
- Top-level `[profiles]` / per-profile overrides (e.g. `dev` skips tree-shaking)?
- How to declare additional embedded resources beyond FileInstall (see [Resource embedding](#resource-embedding))?

## Interpreter management

Interpreter binaries are cached in `~/.ahkbuild/interpreters/<version>-<bitness>/` (e.g.
`~/.ahkbuild/interpreters/2.1-alpha.27-x64/`). The cache is shared across projects.

Acquisition order (for `ahkbuild interpreter install <version>`):
1. Check local cache - skip download if already present.
2. Try `https://www.autohotkey.com/download` (works on user machines; v2.0 also available from
   GitHub releases as a fallback).
3. Compile from source (slow; acceptable in CI where the download fails, e.g. Cloudflare blocks).

Related CLI commands:
- `ahkbuild interpreter install <version> [--bitness 32|64]`
- `ahkbuild interpreter list` - show cached versions
- `ahkbuild interpreter prune [<version>]` - remove cached binaries

The interpreter version and bitness declared in config are used automatically when running
`bundle exe`; `install` can be called in a CI setup step. If the declared version is not cached,
`bundle exe` fails with a clear error pointing to `interpreter install`.

## PE manipulation

The tool needs to embed RCDATA resources and set version information, application manifest, and
icon in the interpreter binary. The preferred approach is a **Rust PE library** (e.g. `pelite`)
rather than shelling out to external tools (`rcedit`, Windows SDK `mt.exe`, etc.).

Benefits of a Rust library:
- No external tool dependency for users
- Cross-platform builds work (produce a Windows `.exe` from Linux CI)
- Deterministic output enables reproducible builds (zero out timestamps, sort resources)

Things that need PE-level management:
- **RCDATA resources** - script modules, FileInstall files, additional declared resources
- **Icon** (`RT_ICON` / `RT_GROUP_ICON`) - replace the interpreter's default icon
- **Version info** (`RT_VERSION`) - `FileVersion`, `ProductVersion`, `ProductName`,
  `FileDescription`, `LegalCopyright`, `InternalName`, `OriginalFilename`
- **Application manifest** (`RT_MANIFEST`) - UAC elevation level, DPI awareness, supported OS
  versions. The interpreter ships a default manifest; config should allow replacing it or patching
  specific fields.

Code signing is **not** managed by ahkbuild. It is a post-bundle concern and is naturally handled
by a post-bundle build script (e.g. invoking `signtool` or `osslsigncode`).

## Resource embedding

### FileInstall

`FileInstall "literal-path", dest` calls in the IR are detected statically (AHK enforces that the
first argument is a quoted literal string, so detection is reliable). At bundle time, the source
file is embedded as RCDATA under a resource name matching the interpreter's lookup convention
(uppercased basename in v2.0 - verify for v2.1). The `FileInstall` call is left in the emitted
source unchanged; the runtime handles extraction transparently when `A_IsCompiled` is true.

If a FileInstall call is in a branch that fold/shake removes, its file is **not** embedded (the
call will never execute).

Dynamic FileInstall (first argument is a variable) is a build error.

### Icons and images for `LoadPicture`

`LoadPicture` can load images from the current executable using the `IconN` syntax, which loads the
nth icon of the target exe. Embedding images to be loaded this way requires declaring them in config.
The config `resources` array handles this:

```json
"resources": {
    // Must be an ordered array so Icon1, Icon2 etc works as expected
    "icons": [
      "assets/icon1.ico"
    ],
    // Maybe these go in a separate object?
    "help": {
      "type": "RT_HTML",
      "path": "assets/help.html"
    },
    "about": {
      "type": 23, // allow bare resource number
      "path": "assets/about.html"
    },
    "logo": {
      "type": "RT_BITMAP",
      "path": "assets/images/logo.bmp"
    }
  }
```

Whether ahkbuild should provide AHK helper functions for extracting other embedded resource types
(text files, etc.) at runtime is an open question.

Also up for debate is the exact schema - should icons be separate from the resources map?

## Build scripts

Pre- and post-bundle scripts are AHK scripts run out-of-process using the same interpreter
declared in config. Running with the same interpreter means `A_AhkVersion`, `A_PtrSize`,
`A_ScriptDir` (project root), etc. are naturally correct.

Additional context is passed via environment variables:

| Variable | Value |
| --- | --- |
| `AHKBUILD_STAGE` | `pre` or `post` |
| `AHKBUILD_TARGET` | `ahk` or `exe` |
| `AHKBUILD_OUTPUT` | Absolute path to the output file |
| `AHKBUILD_VERSION` | Version string from config |
| `AHKBUILD_ENTRY` | Absolute path to the entry script |

A non-zero exit code from a build script aborts the build. Stdout/stderr are forwarded to the
ahkbuild console output.

Post-bundle scripts receive `AHKBUILD_OUTPUT` pointing to the completed `.exe` and are the
natural place for steps like code signing or MPRESS / UPX compression.
