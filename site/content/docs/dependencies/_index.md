---
title: Dependencies
weight: 4
bookCollapseSection: false
---

# Module dependencies

ahkbuild resolves the `#Import` graph across project boundaries with a thin, registry-less dependency layer,
similar to Go's package management model. Dependencies are declared in
[`ahkbuild.json`]({{< relref "/docs/reference/config" >}}) and pinned to exact revisions.
Ahkbuild fetches them into a shared content-addressed store and exposes them to your script under
clean logical names.

There is *no central index*: dependencies point directly at sources (a git repo, a gist, a tarball, or a local
path).

Ahkbuild stores all packages in content-addressable directories under `~/.ahkbuild/modules/` and [soft-link]s
to them in your local repository. This system is inspired by `pnpm`'s and means that identical dependencies are
resolved to the same physical files. These soft links live in the *link farm* under `<project>/.ahkbuild/modules/`,
which .gitignores itself. [`ahkbuild run`](#running-from-source) points [`AhkImportPath`] path here automatically.

[soft-link]: https://learn.microsoft.com/en-us/windows/win32/fileio/hard-links-and-junctions#junctions
[`AhkImportPath`]: https://www.autohotkey.com/docs/alpha/Modules.htm#Search_Path

## Dependency Configuration

```jsonc
{
  "entry": "src/main.ahk",
  "interpreter": { "version": "2.1-alpha.27" },
  "dependencies": {
    "cJson": {
      "git": "git@github.com:G33kDude/cJson.ahk.git",
      "rev": "ea5313ce0e5e79aadcb367e7167c1da991717de3",  // Can also be a tag name
      "subdir": "dist"
    },
  }
}
```

```bash
ahkbuild package restore   # resolve, pin, fetch, and build the link-farm
ahkbuild run               # restore + launch the entry script with #Import resolving
```

Your script then imports by manifest key:

```ahk
; For a v2.0 library
#Import "cJSON\JSON.ahk" { JSON }

; For a v2.1 lib with a folder-level __Init:
#Import library { Functon, LibClass }
```

Entries in `dependencies` are keyed by the `#Import` name. The value must describe exactly one source. See the
[`dependencies` reference]({{< relref "/docs/reference/config#dependencies" >}}) for the full field tables.

| Source | Shape | Notes |
| --- | --- | --- |
| `git` | `{ "git": "<.git url>", "tag"\|"branch"\|"rev": "…" }` | A shallow `git` clone of any forge. With no selector, the default branch HEAD is used. In the lockfile, these are always pinned to a commit SHA. |
| `gist` | `{ "gist": "<id>", "rev": "…" }` | Gists are git repos; `rev` is optional (latest HEAD otherwise). |
| `tarball` | `{ "tarball": "<url>", "sha256": "…" }` | A `.zip` or `.tar.gz`. The `sha256` of the archive bytes is required and verified. These can be used to target GitHub releases; point the url at the download link. |
| `path` | `{ "path": "../rel/or/abs" }` | A local directory. Not reproducible, so **excluded from the lockfile**. |

An optional `subdir` on any source points at the module root inside the fetched tree when it is not the
repository/archive root.

## The lockfile

`ahkbuild.lock` sits beside `ahkbuild.json` and pins non-path dependencies:

```json
{
  "version": 1,
  "package": [
    {
      "name": "cJson",
      "source": "git+git@github.com:G33kDude/cJson.ahk.git?rev=ea5313ce0e5e79aadcb367e7167c1da991717de3",
      "resolved": "ea5313ce0e5e79aadcb367e7167c1da991717de3",
      "checksum": "sha256:b48e5186be920adf77d169870637610932cc1f0a702b9ec63834b8a40c10f020"
    }
  ]
}
```

- `source` is the manifest source identity, changing it will cause `ahkbuild` to re-resolve the dependency.
- `resolved` is the immutable revision. For git / gist sources, it is a commit SHA, for tarballs a URL.
- `checksum` is `sha256:<hex>` over the fetched tree.

To ensure reproducibility, use the `--locked` flag, which will error if restoring dependencies would cause a
lockfile change.

```bash
ahkbuild package restore --locked
```

## Running from source

Because [`AhkImportPath`] is an environment variable with no in-script form, the reliable way to run a script with
its dependencies resolved is:

```bash
ahkbuild run [entry] [-- <script args>]
```

`run` restores dependencies, resolves the configured interpreter (auto-installing it if needed, handy when the
project targets a different version than your default), points `AhkImportPath` at the link-farm, and launches the
script. See the [CLI reference]({{< relref "/docs/reference/cli#ahkbuild-run" >}}).
