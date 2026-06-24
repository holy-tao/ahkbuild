//! The fixpoint build driver.
//!
//! Every optimization pass is a pure function of the IR plus side tables, and the passes
//! compose: tree-shaking can expose more dead code, and inlining (planned) will expose new
//! folding and shaking opportunities. This crate runs them to a **fixpoint** via two nested
//! loops, keyed on the additive-vs-subtractive split:
//!
//! - **Inner loop** ([`run_inner_fixpoint`]) - the *subtractive / substitutive* passes
//!   ([`fold`](ahkbuild_fold::fold), [`shake`](ahkbuild_shake::shake)) iterate until their
//!   tables stop growing.
//! - **Outer loop** ([`bundle_ahk`]) - *additive / structural* change (inlining) can't be
//!   expressed as a span edit on the original text, so it is applied by [materializing] the
//!   current edits to text, re-parsing, re-lowering, and re-running everything.
//!
//! [materializing]: ahkbuild_emit::materialize

use anyhow::{anyhow, Result};

use ahkbuild_emit::{emit_ahk, materialize, EmitOptions, InlineEdits};
use ahkbuild_fold::{fold, Constants, FoldResult};
use ahkbuild_ir::Program;
use ahkbuild_link::{link_bundle, BundlePlan, LinkOutput};
use ahkbuild_shake::{shake, ShakeResult};

/// Hard cap on outer (structural) rounds, a safety net against an inliner that fails to
/// converge (e.g. mutually recursive inlining that the budget doesn't catch). Generous: real
/// builds settle in 1–2 rounds.
const MAX_STRUCTURAL_ROUNDS: u32 = 16;

/// Default inlining budget carried in [`Facts`]; unused until the inliner lands.
const DEFAULT_INLINE_BUDGET: u32 = 8;

/// Cross-round facts: IR-independent knowledge that **survives a re-parse**. Unlike a [`Round`],
/// nothing here is keyed by `NodeId`, so it is carried whole into each new structural round.
#[derive(Clone, Debug)]
pub struct Facts {
    /// Build-time constants (`A_IsCompiled`, `A_PtrSize`) for [`fold`](ahkbuild_fold::fold).
    pub consts: Constants,
    /// Remaining inlining budget. Present as the seam for the future inliner; unused today.
    #[allow(dead_code)]
    inline_budget: u32,
    // Future: variables proven single-assignment (named constants) discovered by `shake`, fed
    // back into `fold` - the shake -> fold back-edge that gives the inner loop real iterations.
}

impl Facts {
    /// Seed the cross-round facts from the known build-time constants.
    pub fn new(consts: Constants) -> Self {
        Self {
            consts,
            inline_budget: DEFAULT_INLINE_BUDGET,
        }
    }
}

/// Round-local analysis: the `NodeId`-keyed side tables produced by the passes. Valid only for
/// the current IR generation - a re-parse invalidates every `NodeId`, so a fresh `Round` is
/// built each structural round.
#[derive(Debug, Default)]
pub struct Round {
    /// Constant-folding result, or `None` when not optimizing (faithful bundle).
    pub fold: Option<FoldResult>,
    /// Tree-shaking result, or `None` when not optimizing.
    pub shake: Option<ShakeResult>,
}

impl Round {
    /// Total recorded entries. Because every pass is **monotone** (it only ever adds to the
    /// tables), this is non-decreasing across inner-loop sweeps, so a stable value means the
    /// fixpoint is reached.
    fn len(&self) -> usize {
        let f = self
            .fold
            .as_ref()
            .map_or(0, |f| f.literals.len() + f.branches.len());
        let s = self.shake.as_ref().map_or(0, |s| {
            s.dead.len() + s.dropped_imports.len() + s.dead_modules.len()
        });
        f + s
    }
}

/// A program after the optimization passes have run to a fixpoint: the (possibly re-linked) IR
/// and plan, plus the converged side tables ([`Round`]). This is the shared hand-off to every
/// emit backend - the `.ahk` emitter ([`bundle_ahk`]) and the `.exe` emitter both consume it, so
/// optimization stays identical across targets.
pub struct Converged {
    pub program: Program,
    pub plan: BundlePlan,
    pub round: Round,
}

/// Run the optimization passes over a linked program to a fixpoint and return the converged
/// program, plan, and side tables, without emitting. Both emit backends build on this.
///
/// `consts` seeds constant folding; `optimize` runs fold + shake (set `false` for a byte-faithful
/// bundle, as `--no-tree-shake` does).
pub fn converge(link_out: LinkOutput, consts: Constants, optimize: bool) -> Result<Converged> {
    let mut facts = Facts::new(consts);
    let mut program = link_out.program;
    let mut plan = link_out.plan;

    for round_no in 0..=MAX_STRUCTURAL_ROUNDS {
        // Inner loop: settle the side-table passes over the current IR.
        let round = run_inner_fixpoint(&program, &plan, &facts, optimize);

        // Structural pass (inlining): stubbed to no edits today, so we always take this branch
        // on the first round and return the converged state.
        let inline = plan_inline(&program, &plan, &round, &mut facts);
        if inline.is_empty() {
            return Ok(Converged {
                program,
                plan,
                round,
            });
        }

        if round_no == MAX_STRUCTURAL_ROUNDS {
            return Err(anyhow!(
                "bundle did not converge after {MAX_STRUCTURAL_ROUNDS} structural rounds"
            ));
        }

        // Apply all current edits (annotation + structural) to faithful, re-parseable text, then
        // re-parse and re-link for the next round. `facts` persists; `round`'s NodeIds do not.
        let text = materialize(
            &program,
            &plan,
            round.shake.as_ref(),
            round.fold.as_ref(),
            &inline,
        );
        let relinked = relower_and_relink(&text)?;
        program = relinked.program;
        plan = relinked.plan;
    }

    unreachable!("loop returns on convergence or errors at the round cap")
}

/// Bundle a linked program to a single self-contained `.ahk`, running the optimization passes
/// to a fixpoint.
///
/// `consts` seeds constant folding; `optimize` runs fold + shake (set `false` for a
/// byte-faithful bundle, as `--no-tree-shake` does); `emit` carries the final cosmetic knobs.
pub fn bundle_ahk(
    link_out: LinkOutput,
    consts: Constants,
    optimize: bool,
    emit: &EmitOptions,
) -> Result<String> {
    let c = converge(link_out, consts, optimize)?;
    Ok(emit_ahk(
        &c.program,
        &c.plan,
        c.round.shake.as_ref(),
        c.round.fold.as_ref(),
        emit,
    ))
}

/// Run the subtractive passes ([`fold`], then [`shake`]) over one IR until their side tables
/// stop growing. Returns an empty [`Round`] when `optimize` is false (faithful bundle).
///
/// Convergence is detected by total table size being unchanged between sweeps - sound because
/// the passes are monotone. With no shake -> fold back-edge yet, this settles after one full
/// sweep (the second sweep recomputes identical tables and breaks).
fn run_inner_fixpoint(
    program: &Program,
    plan: &BundlePlan,
    facts: &Facts,
    optimize: bool,
) -> Round {
    let mut round = Round::default();
    if !optimize {
        return round;
    }
    loop {
        let before = round.len();
        round.fold = Some(fold(program, &facts.consts));
        round.shake = Some(shake(program, plan, round.fold.as_ref()));
        if round.len() == before {
            break;
        }
    }
    round
}

/// Structural inlining pass - **stub**. Returns no edits, so [`bundle_ahk`]'s outer loop runs
/// exactly once and emits directly. The real inliner will consult `facts` (inline budget /
/// visited set) and the converged `round`, and return per-call-site replacement edits.
fn plan_inline(
    _program: &Program,
    _plan: &BundlePlan,
    _round: &Round,
    _facts: &mut Facts,
) -> InlineEdits {
    InlineEdits::default()
}

/// Re-parse and re-link a [materialized](materialize) bundle for the next structural round. The
/// text is already a single self-contained program (imports in-group, includes spliced, names
/// final), so this re-lowers and re-links **in memory** via [`link_bundle`] - no file IO.
fn relower_and_relink(text: &str) -> Result<LinkOutput> {
    let tree = ahkbuild_syntax::parse(text).ok_or_else(|| anyhow!("re-parse produced no tree"))?;
    if tree.root_node().has_error() {
        return Err(anyhow!(
            "re-parse of materialized bundle hit syntax errors:\n{}",
            tree.root_node().to_sexp()
        ));
    }
    let program = ahkbuild_ir::lower(&tree, text);
    Ok(link_bundle(program))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lower a single source into a trivial single-group bundle plan (no file IO).
    fn bundle_program(src: &str) -> LinkOutput {
        let tree = ahkbuild_syntax::parse(src).expect("tree");
        assert!(
            !tree.root_node().has_error(),
            "{}",
            tree.root_node().to_sexp()
        );
        link_bundle(ahkbuild_ir::lower(&tree, src))
    }

    /// The inner loop converges to exactly what a single `fold` + `shake` produces (no
    /// back-edge today), and re-running it does not grow the tables.
    #[test]
    fn inner_fixpoint_matches_single_pass() {
        // `A_IsCompiled` is false, so the `else` arm survives: `Bar` is live, `Foo` (then-arm
        // only) and `Dead` (unreferenced) shake out.
        let src = "if A_IsCompiled {\n  Foo()\n} else {\n  Bar()\n}\n\
                   Bar() {\n  return 1\n}\nFoo() {\n  return 2\n}\nDead() {\n  return 3\n}\n";
        let lo = bundle_program(src);
        let facts = Facts::new(Constants {
            is_compiled: Some(false),
            ptr_size: None,
        });

        let round = run_inner_fixpoint(&lo.program, &lo.plan, &facts, true);

        let f = fold(&lo.program, &facts.consts);
        let s = shake(&lo.program, &lo.plan, Some(&f));
        let rf = round.fold.as_ref().expect("fold ran");
        let rs = round.shake.as_ref().expect("shake ran");
        assert_eq!(rf.branches.len(), f.branches.len());
        assert_eq!(rf.literals.len(), f.literals.len());
        assert_eq!(rs.dead.len(), s.dead.len());
        // Something actually folded and shook, else the test proves nothing.
        assert_eq!(rf.branches.len(), 1, "the one `if` should resolve");
        assert!(rs.dead.len() >= 2, "Foo and Dead should shake out");
    }

    /// Without optimization the inner loop produces an empty round (faithful bundle).
    #[test]
    fn no_optimize_yields_empty_round() {
        let lo = bundle_program("x := 1\nDead() {\n  return 1\n}\n");
        let facts = Facts::new(Constants::default());
        let round = run_inner_fixpoint(&lo.program, &lo.plan, &facts, false);
        assert_eq!(round.len(), 0);
        assert!(round.fold.is_none() && round.shake.is_none());
    }
}
