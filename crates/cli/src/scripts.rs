//! Pre- and post-bundle build-script execution.
//!
//! Scripts are run out-of-process as argv vectors (no shell). The first token may be the builtin
//! `${AHK}`, which resolves to the configured interpreter; any `${NAME}` token is substituted from
//! the build environment - the standard `AHKBUILD_*` vars plus user `defines`. Each script inherits
//! that environment and runs with its working directory set to the project root (the config dir).
//! A non-zero exit aborts the build.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

/// Stdin handed to a build script: inherit a usable handle, else `NUL`.
///
/// We can't blindly inherit on Windows. A GUI-subsystem child like AutoHotkey, which has no console
/// of its own, throws `(6) The handle is invalid` the instant it touches a standard stream if it
/// inherited a broken handle - which is what it gets when ahkbuild itself was launched without a
/// console (a GUI launcher, an IDE task runner). For stdin, substituting `NUL` when our own handle
/// is invalid gives the child a clean EOF instead of a poisoned handle. Stdout/stderr take the
/// sturdier route of piping + relaying (see `run_one`).
#[cfg(windows)]
pub(crate) fn child_stdin() -> Stdio {
    use std::os::windows::io::AsRawHandle;
    let h = std::io::stdin().as_raw_handle();
    if h.is_null() || h as isize == -1 {
        Stdio::null()
    } else {
        Stdio::inherit()
    }
}

#[cfg(not(windows))]
pub(crate) fn child_stdin() -> Stdio {
    Stdio::inherit()
}

use ahkbuild_config::{BuildScript, Subsystem};
use ahkbuild_interpret::Bitness;

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    Pre,
    Post,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Pre => "pre",
            Stage::Post => "post",
        }
    }
}

/// Everything a build script needs to know about the bundle in progress.
pub(crate) struct ScriptContext<'a> {
    pub target: &'a str,
    pub output: &'a Path,
    pub entry: &'a Path,
    pub interpreter: &'a Path,
    pub bitness: Bitness,
    pub subsystem: Subsystem,
    pub version: Option<&'a str>,
    pub config_dir: &'a Path,
    /// Validated, stringified `defines` (see `BuildConfig::defines_env`).
    pub defines: &'a BTreeMap<String, String>,
}

impl ScriptContext<'_> {
    /// The full `${NAME}` substitution table: the standard vars, the `AHK` builtin, and user
    /// defines. The same map (minus `AHK`) is exported into each script's environment.
    fn vars(&self, stage: Stage) -> BTreeMap<String, String> {
        let mut vars = BTreeMap::new();
        vars.insert("AHKBUILD_STAGE".into(), stage.as_str().into());
        vars.insert("AHKBUILD_TARGET".into(), self.target.into());
        vars.insert(
            "AHKBUILD_OUTPUT".into(),
            self.output.to_string_lossy().into_owned(),
        );
        vars.insert(
            "AHKBUILD_ENTRY".into(),
            self.entry.to_string_lossy().into_owned(),
        );
        vars.insert(
            "AHKBUILD_INTERPRETER".into(),
            self.interpreter.to_string_lossy().into_owned(),
        );
        vars.insert(
            "AHKBUILD_BITNESS".into(),
            match self.bitness {
                Bitness::X32 => "32".into(),
                Bitness::X64 => "64".into(),
            },
        );
        vars.insert(
            "AHKBUILD_SUBSYSTEM".into(),
            match self.subsystem {
                Subsystem::Gui => "gui".into(),
                Subsystem::Console => "console".into(),
            },
        );
        vars.insert(
            "AHKBUILD_CONFIG_DIR".into(),
            self.config_dir.to_string_lossy().into_owned(),
        );
        if let Ok(exe) = std::env::current_exe() {
            vars.insert("AHKBUILD_EXE".into(), exe.to_string_lossy().into_owned());
        }
        if let Some(v) = self.version {
            vars.insert("AHKBUILD_VERSION".into(), v.into());
        }
        for (k, v) in self.defines {
            vars.insert(k.clone(), v.clone());
        }
        vars
    }
}

/// Run every script for `stage` in order, aborting on the first non-zero exit.
pub(crate) fn run_scripts(
    stage: Stage,
    scripts: &[BuildScript],
    ctx: &ScriptContext,
) -> Result<()> {
    if scripts.is_empty() {
        return Ok(());
    }
    // The environment exported to scripts: the standard vars plus user defines. The substitution
    // table additionally exposes the `AHK` builtin, which resolves to the configured interpreter.
    let env = ctx.vars(stage);
    let mut subst = env.clone();
    subst.insert("AHK".into(), ctx.interpreter.to_string_lossy().into_owned());
    for script in scripts {
        run_one(stage, script, ctx, &env, &subst)?;
    }
    Ok(())
}

fn run_one(
    stage: Stage,
    script: &BuildScript,
    ctx: &ScriptContext,
    env: &BTreeMap<String, String>,
    subst: &BTreeMap<String, String>,
) -> Result<()> {
    let mut argv = Vec::with_capacity(script.command.len());
    for token in &script.command {
        argv.push(substitute(token, subst).with_context(|| {
            format!("in {}-bundle script {:?}", stage.as_str(), script.command)
        })?);
    }

    let (program, args) = argv.split_first().expect("command validated non-empty");

    // A span tags every ahkbuild log emitted while this script runs, so its surrounding
    // diagnostics are attributable to a specific build-script stage.
    let _span = tracing::info_span!("build_script", stage = stage.as_str()).entered();
    tracing::info!(cmd = %argv.join(" "), "running");

    // Pipe the child's stdout/stderr and relay them onto ours, rather than letting the child
    // inherit ahkbuild's standard handles. The child then always writes to a clean pipe Rust
    // created, so a GUI-subsystem script (AutoHotkey) has a valid stdout no matter how ahkbuild was
    // launched - the source of the `(6) The handle is invalid` failure when inheriting directly. If
    // our own stream is unwritable, the copied bytes are dropped, which is harmless.
    let mut child = Command::new(program)
        .args(args)
        .current_dir(ctx.config_dir)
        .envs(env)
        .stdin(child_stdin())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to launch {}-bundle script {program:?}",
                stage.as_str()
            )
        })?;

    let mut child_out = child.stdout.take().expect("stdout piped");
    let mut child_err = child.stderr.take().expect("stderr piped");
    let relay_out = std::thread::spawn(move || {
        let _ = std::io::copy(&mut child_out, &mut std::io::stdout());
    });
    let relay_err = std::thread::spawn(move || {
        let _ = std::io::copy(&mut child_err, &mut std::io::stderr());
    });

    let status = child
        .wait()
        .with_context(|| format!("waiting on {}-bundle script {program:?}", stage.as_str()))?;
    let _ = relay_out.join();
    let _ = relay_err.join();

    if !status.success() {
        match status.code() {
            Some(code) => bail!(
                "{}-bundle script {program:?} exited with status {code}",
                stage.as_str()
            ),
            None => bail!(
                "{}-bundle script {program:?} terminated by signal",
                stage.as_str()
            ),
        }
    }
    Ok(())
}

/// Expand `${NAME}` references in a single argv token from `vars`. An unknown name is an error
/// (it almost always means a typo); a literal `$` is otherwise passed through unchanged.
fn substitute(token: &str, vars: &BTreeMap<String, String>) -> Result<String> {
    let mut out = String::with_capacity(token.len());
    let mut rest = token;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .with_context(|| format!("unterminated '${{' in token {token:?}"))?;
        let name = &after[..end];
        let value = vars
            .get(name)
            .with_context(|| format!("unknown variable '${{{name}}}' in token {token:?}"))?;
        out.push_str(value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("AHKBUILD_OUTPUT".into(), "C:\\out\\app.exe".into());
        m.insert("DEBUG".into(), "1".into());
        m
    }

    #[test]
    fn substitutes_known_vars() {
        assert_eq!(
            substitute("${AHKBUILD_OUTPUT}", &vars()).unwrap(),
            "C:\\out\\app.exe"
        );
        assert_eq!(substitute("debug=${DEBUG}", &vars()).unwrap(), "debug=1");
        assert_eq!(substitute("--no-vars", &vars()).unwrap(), "--no-vars");
    }

    #[test]
    fn unknown_var_errors() {
        assert!(substitute("${NOPE}", &vars()).is_err());
    }

    #[test]
    fn unterminated_brace_errors() {
        assert!(substitute("${OOPS", &vars()).is_err());
    }

    #[cfg(windows)]
    mod run {
        use super::super::*;
        use std::collections::BTreeMap;
        use std::path::Path;

        fn ctx<'a>(defines: &'a BTreeMap<String, String>) -> ScriptContext<'a> {
            ScriptContext {
                target: "exe",
                output: Path::new("C:\\out\\app.exe"),
                entry: Path::new("C:\\proj\\main.ahk"),
                interpreter: Path::new("C:\\interp\\AutoHotkey64.exe"),
                bitness: Bitness::X64,
                subsystem: Subsystem::Gui,
                version: Some("1.2.3.0"),
                config_dir: Path::new("."),
                defines,
            }
        }

        fn cmd(line: &str) -> BuildScript {
            BuildScript {
                command: vec!["cmd".into(), "/c".into(), line.into()],
            }
        }

        #[test]
        fn nonzero_exit_aborts() {
            let defines = BTreeMap::new();
            let err = run_scripts(Stage::Post, &[cmd("exit 3")], &ctx(&defines));
            assert!(err.is_err());
        }

        #[test]
        fn zero_exit_succeeds() {
            let defines = BTreeMap::new();
            assert!(run_scripts(Stage::Post, &[cmd("exit 0")], &ctx(&defines)).is_ok());
        }

        #[test]
        fn standard_vars_reach_the_environment() {
            // cmd expands %AHKBUILD_TARGET% from the child environment.
            let defines = BTreeMap::new();
            let script = cmd("if \"%AHKBUILD_TARGET%\"==\"exe\" (exit 0) else (exit 1)");
            assert!(run_scripts(Stage::Post, &[script], &ctx(&defines)).is_ok());
        }

        #[test]
        fn defines_reach_the_environment() {
            let mut defines = BTreeMap::new();
            defines.insert("MODE".into(), "release".into());
            let script = cmd("if \"%MODE%\"==\"release\" (exit 0) else (exit 1)");
            assert!(run_scripts(Stage::Post, &[script], &ctx(&defines)).is_ok());
        }

        #[test]
        fn ahk_token_substitutes_before_launch() {
            // ${AHK} is replaced by ahkbuild (not the shell), so cmd sees the literal path.
            let defines = BTreeMap::new();
            let script = BuildScript {
                command: vec![
                    "cmd".into(),
                    "/c".into(),
                    "if \"${AHK}\"==\"C:\\interp\\AutoHotkey64.exe\" (exit 0) else (exit 1)".into(),
                ],
            };
            assert!(run_scripts(Stage::Pre, &[script], &ctx(&defines)).is_ok());
        }

        #[test]
        fn unknown_token_aborts_before_launch() {
            let defines = BTreeMap::new();
            let script = BuildScript {
                command: vec!["cmd".into(), "/c".into(), "${TYPO}".into()],
            };
            assert!(run_scripts(Stage::Post, &[script], &ctx(&defines)).is_err());
        }
    }
}
