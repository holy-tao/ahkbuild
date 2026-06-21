//! Output emitters that consume the linker's backend-neutral [`BundlePlan`].
//!
//! This crate is the home for every emission backend. The single-`.ahk` emitter
//! ([`emit_ahk`]) lives here today; the planned `.exe` emitter (RCDATA injection, asset
//! embedding, resource naming) will land as a separate, dependency-heavy sibling crate so it
//! can pull in PE/Win32 machinery without weighing down this portable, text-only path or the
//! `link` crate that produces the plan.
//!
//! Emission is span-level, not a re-serialization of the IR: each file starts from its
//! original source text and the emitter splices in [`Edit`](patch::Edit)s for the nodes it
//! needs to change (see [`patch`]). Edits are keyed by [`FileId`] — a group spans several
//! files once `#Include` is in play — and the producers run today are import redirection,
//! module renaming, tree-shaking deletions, and `#Include` splicing. A group is emitted by
//! [`expand`]ing its primary file, recursively pasting each included file's emitted text over
//! its `#Include` directive (the future `.exe` emitter instead keeps the directive and emits
//! each file once as a resource).

pub mod patch;

use std::collections::{HashMap, HashSet};

use ahkbuild_fold::{Branch, ConstValue, FoldResult};
use ahkbuild_ir::node::ImportSource;
use ahkbuild_ir::{FileId, GroupId, NodeId, NodeKind, Program, Span};
use ahkbuild_link::{BundlePlan, IncludeSplice};
use ahkbuild_shake::ShakeResult;

use patch::{apply_edits, Edit};

/// Backend-neutral knobs shared by every emitter (the `.ahk` emitter today, the planned `.exe`
/// emitter tomorrow). Construct with [`EmitOptions::default`] for the standard release bundle.
#[derive(Clone, Copy, Debug)]
pub struct EmitOptions {
    /// Strip comments from the bundle. On by default; the CLI's `--keep-comments` flips it off.
    pub strip_comments: bool,
    /// How aggressively to normalize whitespace in the final output. Defaults to
    /// [`WsLevel::Readable`].
    pub whitespace: WsLevel,
}

/// How much whitespace the final normalization pass collapses. Deletions and comment-stripping
/// leave holes (blank lines, trailing indentation); this controls how they're cleaned up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WsLevel {
    /// Leave the spliced text exactly as the edits produced it.
    Off,
    /// Strip trailing whitespace and collapse runs of 2+ blank lines to one. Targets the holes
    /// left by deletions while preserving the author's single blank-line separators. For `.ahk`.
    Readable,
    /// Strip trailing whitespace, drop all blank lines, strip leading indentation, and collapse
    /// intra-line space runs (outside string literals). For `.exe`, where readability is moot.
    Minify,
}

impl Default for EmitOptions {
    fn default() -> Self {
        Self {
            strip_comments: true,
            whitespace: WsLevel::Readable,
        }
    }
}

/// Normalize whitespace in fully-assembled output text per `level`. Runs after all
/// offset-dependent work, so it's free to rewrite lines without touching any [`Span`].
///
/// [`WsLevel::Readable`] strips trailing whitespace and collapses runs of blank lines to one;
/// [`WsLevel::Minify`] additionally drops every blank line, strips leading indentation, and
/// collapses intra-line space runs outside string literals.
pub fn normalize_whitespace(text: &str, level: WsLevel) -> String {
    if level == WsLevel::Off {
        return text.to_string();
    }
    let minify = level == WsLevel::Minify;

    let mut out = String::with_capacity(text.len());
    let mut pending_blank = false;
    for raw in text.lines() {
        let mut line = raw.trim_end();
        let mut collapsed;
        if minify {
            line = line.trim_start();
            collapsed = String::new();
            collapse_inner_spaces(line, &mut collapsed);
            line = &collapsed;
        }
        if line.is_empty() {
            // Readable: remember we owe at most one blank. Minify: drop entirely.
            pending_blank = !minify;
            continue;
        }
        if pending_blank {
            out.push('\n');
            pending_blank = false;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Collapse runs of spaces/tabs to a single space, but only outside string literals so column
/// alignment inside `"..."` / `'...'` survives. Tracks AHK's doubled-quote escape (`""`, `''`).
/// Note: this is text-only and doesn't recognize comments, so it assumes comment stripping has
/// already removed them (the default).
fn collapse_inner_spaces(line: &str, out: &mut String) {
    let mut quote: Option<char> = None;
    let mut prev_ws = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some(q) => {
                // in a string
                out.push(ch);
                if ch == '`' {
                    if chars.peek() == Some(&q) {
                        out.push(chars.next().unwrap());
                    } else {
                        quote = None;
                    }
                }
                prev_ws = false;
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                    out.push(ch);
                    prev_ws = false;
                } else if ch.is_whitespace() {
                    if !prev_ws {
                        out.push(' ');
                    }
                    prev_ws = true;
                } else {
                    out.push(ch);
                    prev_ws = false;
                }
            }
        }
    }
}

/// Structural edits produced by the (future) inliner: per-file span replacements that paste an
/// inlined callee body over a call site. Defined here because they are emitter [`Edit`]s like
/// any other producer's; the inliner lives in the pipeline driver and hands these to
/// [`materialize`]. Empty today (inlining is not yet implemented), so [`is_empty`] is always
/// true and the driver's outer re-parse loop runs exactly once.
///
/// [`is_empty`]: InlineEdits::is_empty
#[derive(Debug, Default)]
pub struct InlineEdits {
    /// Replacement edits keyed by the file whose text each span falls in.
    pub edits: HashMap<FileId, Vec<Edit>>,
}

impl InlineEdits {
    /// Whether there is no structural work to apply (so no re-parse round is needed).
    pub fn is_empty(&self) -> bool {
        self.edits.values().all(|v| v.is_empty())
    }
}

/// Emit a single self-contained `.ahk` bundle: the entry group's source, then each imported
/// group wrapped in a `#Module Name` block, with every `#Include` spliced inline. Resolved
/// `#Import`s are rewritten to name the in-file module, every module gets a program-unique
/// name, and each included file's text is pasted over its directive (deduped repeats deleted),
/// so the bundle resolves entirely in-process.
///
/// Pass a [`ShakeResult`] to also delete dead declarations and unused imports and omit
/// fully-dead groups; pass `None` for a byte-faithful bundle. Pass a [`FoldResult`] to
/// substitute build-time constants (`A_IsCompiled` -> `0`/`1`) and remove the dead arm of any
/// `if`/ternary whose condition folded. [`EmitOptions`] carries the backend-neutral knobs
/// (e.g. comment stripping, on by default).
///
/// This is the **final** renderer: it applies the cosmetic passes (comment stripping,
/// whitespace normalization) on top of the structural/annotation edits. For an intermediate,
/// re-parseable round inside the fixpoint driver, use [`materialize`] instead.
pub fn emit_ahk(
    program: &Program,
    plan: &BundlePlan,
    shake: Option<&ShakeResult>,
    fold: Option<&FoldResult>,
    options: &EmitOptions,
) -> String {
    assemble(
        program,
        plan,
        shake,
        fold,
        &InlineEdits::default(),
        options.strip_comments,
        options.whitespace,
    )
}

/// Emit an **intermediate**, re-parseable bundle for one round of the fixpoint driver: the same
/// structural and annotation edits as [`emit_ahk`] (import redirects, module renames, `#Include`
/// splices, branch collapses, tree-shaking deletions, constant substitutions) plus the inliner's
/// structural `inline` edits, but **without** the cosmetic passes - comments are kept and
/// whitespace is left exactly as the edits produced it ([`WsLevel::Off`]). Skipping cosmetics is
/// load-bearing: the output is fed back through the parser, so it must stay faithful and
/// re-parseable (e.g. [`WsLevel::Minify`] assumes comments are already gone). Cosmetics run once,
/// at the end, via [`emit_ahk`].
pub fn materialize(
    program: &Program,
    plan: &BundlePlan,
    shake: Option<&ShakeResult>,
    fold: Option<&FoldResult>,
    inline: &InlineEdits,
) -> String {
    assemble(program, plan, shake, fold, inline, false, WsLevel::Off)
}

/// Shared assembly core behind [`emit_ahk`] (final) and [`materialize`] (intermediate). Collects
/// every edit producer, splices `#Include`s, wraps imported groups in `#Module` blocks, and
/// applies `strip_comments` / `whitespace` cosmetics per the caller's mode.
#[allow(clippy::too_many_arguments)]
fn assemble(
    program: &Program,
    plan: &BundlePlan,
    shake: Option<&ShakeResult>,
    fold: Option<&FoldResult>,
    inline: &InlineEdits,
    strip_comments: bool,
    whitespace: WsLevel,
) -> String {
    // Imports the shaker dropped must not also be rewritten - they're being deleted.
    let dropped: HashSet<NodeId> = shake
        .map(|s| s.dropped_imports.iter().copied().collect())
        .unwrap_or_default();
    let dead_nodes: HashSet<NodeId> = shake
        .map(|s| s.dead.iter().copied().collect())
        .unwrap_or_default();
    let dead_groups = shake
        .map(|s| fully_dead_groups(program, s))
        .unwrap_or_default();

    // Rewrite edits (import redirects + module renames) are always applied — they produce the
    // same text for every copy of a file. Tree-shaking deletions are kept separate so they can
    // be suppressed for a file spliced in more than once (see `multiply_spliced`).
    let mut rewrites = import_edits(program, plan, &dropped);
    add_rename_edits(program, plan, &mut rewrites);
    // The inliner's structural edits are rewrites (a call site replaced by the callee body); like
    // other rewrites they are identical in every copy of a multiply-spliced file. Empty today.
    for (file, es) in &inline.edits {
        rewrites
            .entry(*file)
            .or_default()
            .extend(es.iter().cloned());
    }
    let mut deletions: HashMap<FileId, Vec<Edit>> = HashMap::new();
    if let Some(s) = shake {
        add_deletion_edits(program, s, &mut deletions);
    }
    if let Some(f) = fold {
        add_branch_edits(program, f, &mut deletions);
    }

    if strip_comments {
        add_strip_comment_edits(program, &mut deletions);
    }

    // Constant substitution is a rewrite (same value in every copy of a file). Skip any
    // built-in that already lies inside a deleted region (a collapsed branch's condition):
    // the deletion removes it, and emitting both would tie on start position.
    if let Some(f) = fold {
        add_substitution_edits(program, f, &deletions, &mut rewrites);
    }

    let includes = includes_by_file(program, plan, &dead_nodes);
    let multiply = multiply_spliced(plan);

    let mut out = String::new();
    for (i, unit) in plan.units.iter().enumerate() {
        // A group whose every module is dead is omitted entirely (its importer's `#Import`
        // is in `dropped_imports`, so nothing dangles).
        if dead_groups.contains(&unit.group) {
            continue;
        }
        let group = &program.groups[unit.group.0 as usize];
        let mut stack = Vec::new();
        let text = expand(
            program, group.file, &rewrites, &deletions, &includes, &multiply, &mut stack,
        );

        // The entry group's primary module stays the implicit `__Main` (no header). Every
        // imported group's primary needs a synthesized `#Module Name` header before its text;
        // any in-source `#Module` sub-modules are already in `text`, renamed in place.
        let header = if i == 0 {
            None
        } else {
            group
                .modules
                .first()
                .and_then(|m| plan.module_names.get(&(unit.group, *m)))
        };
        match header {
            None => out.push_str(&text),
            Some(name) => {
                // Blank-line separation, then the module header on its own line.
                if !out.is_empty() {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push('\n');
                }
                out.push_str("#Module ");
                out.push_str(name);
                out.push('\n');
                out.push_str(&text);
            }
        }
    }
    normalize_whitespace(&out, whitespace)
}

/// Emit one file's text: its own rewrite (and, unless multiply-spliced, deletion) edits, plus
/// an edit per `#Include` directive that pastes the recursively-expanded text of the included
/// file (or deletes a deduped repeat). `stack` guards against include cycles defensively (the
/// linker already rejects them).
fn expand(
    program: &Program,
    file: FileId,
    rewrites: &HashMap<FileId, Vec<Edit>>,
    deletions: &HashMap<FileId, Vec<Edit>>,
    includes: &HashMap<FileId, Vec<(Span, IncludeSplice)>>,
    multiply: &HashSet<FileId>,
    stack: &mut Vec<FileId>,
) -> String {
    let source = program.sources.file(file);
    let mut edits: Vec<Edit> = rewrites.get(&file).cloned().unwrap_or_default();
    if !multiply.contains(&file) {
        if let Some(d) = deletions.get(&file) {
            edits.extend(d.iter().cloned());
        }
    }
    if let Some(list) = includes.get(&file) {
        stack.push(file);
        for (span, splice) in list {
            match splice {
                IncludeSplice::First(inc) => {
                    let content = if stack.contains(inc) {
                        String::new()
                    } else {
                        expand(
                            program, *inc, rewrites, deletions, includes, multiply, stack,
                        )
                    };
                    edits.push(Edit::new(*span, content));
                }
                IncludeSplice::Dedup => edits.push(Edit::new(*span, "")),
                IncludeSplice::Missing => {}
            }
        }
        stack.pop();
    }
    apply_edits(&source.text, source.base, &edits)
}

/// Group `#Include` directives by the file they appear in, as `(directive span, splice)`. A
/// directive whose node is dead (its module was shaken out) is skipped — the deletion edit
/// removes the whole line, so its content must not also be spliced.
fn includes_by_file(
    program: &Program,
    plan: &BundlePlan,
    dead_nodes: &HashSet<NodeId>,
) -> HashMap<FileId, Vec<(Span, IncludeSplice)>> {
    let mut out: HashMap<FileId, Vec<(Span, IncludeSplice)>> = HashMap::new();
    for ri in &plan.resolved_includes {
        if dead_nodes.contains(&ri.node) {
            continue;
        }
        let span = program.arena[ri.node].span;
        let file = program.sources.file_at(span.start).id;
        out.entry(file).or_default().push((span, ri.splice));
    }
    out
}

/// Files spliced in more than once (via `#IncludeAgain`, or included into two modules). Their
/// shared spans can't carry per-copy tree-shaking deletions safely, so they are emitted whole
/// (rewrites only). A conservative over-keep.
fn multiply_spliced(plan: &BundlePlan) -> HashSet<FileId> {
    let mut count: HashMap<FileId, usize> = HashMap::new();
    for ri in &plan.resolved_includes {
        if let IncludeSplice::First(f) = ri.splice {
            *count.entry(f).or_insert(0) += 1;
        }
    }
    count
        .into_iter()
        .filter(|&(_, c)| c > 1)
        .map(|(f, _)| f)
        .collect()
}

/// Rename each written `#Module` directive whose assigned output name differs from its source
/// text, so colliding sub-module names across groups stay distinct in the flat output. Keyed
/// by the file the name span falls in.
fn add_rename_edits(program: &Program, plan: &BundlePlan, edits: &mut HashMap<FileId, Vec<Edit>>) {
    for group in &program.groups {
        for &mid in &group.modules {
            let NodeKind::Module(module) = &program.arena[mid].kind else {
                continue;
            };
            // Only written `#Module Name` directives have a name span to rewrite (the implicit
            // `__Main` primary has none).
            let Some(span) = module.name_span else {
                continue;
            };
            let Some(name) = plan.module_names.get(&(group.id, mid)) else {
                continue;
            };
            if program.span_text(span) == name {
                continue;
            }
            let file = program.sources.file_at(span.start).id;
            edits.entry(file).or_default().push(Edit::new(span, name));
        }
    }
}

/// Groups whose every module the shaker marked dead — omit the whole unit.
fn fully_dead_groups(program: &Program, shake: &ShakeResult) -> HashSet<GroupId> {
    let dead: HashSet<NodeId> = shake.dead_modules.iter().copied().collect();
    program
        .groups
        .iter()
        .filter(|g| !g.modules.is_empty() && g.modules.iter().all(|m| dead.contains(m)))
        .map(|g| g.id)
        .collect()
}

/// Add deletion edits for every dead node and dropped import, keyed by the file whose text the
/// span falls in. (Whitespace left behind is a known cosmetic follow-up; orphaned `;@`
/// directive comments on a deleted node are harmless and left for now.)
fn add_deletion_edits(
    program: &Program,
    shake: &ShakeResult,
    edits: &mut HashMap<FileId, Vec<Edit>>,
) {
    let mut delete = |span: Span| {
        if span.is_empty() {
            return;
        }
        let file = program.sources.file_at(span.start).id;
        edits.entry(file).or_default().push(Edit::new(span, ""));
    };
    for &node in shake.dead.iter().chain(&shake.dropped_imports) {
        delete(program.arena[node].span);
    }
}

/// Add rewrite edits replacing each folded build-time constant identifier with its literal
/// value (e.g. `A_IsCompiled` -> `0`). Safe as a rewrite: the value is identical in every copy
/// of a multiply-spliced file. A substitution falling inside a removed branch's condition is
/// swallowed by that branch's outer deletion via `apply_edits`'s overlap resolution.
fn add_substitution_edits(
    program: &Program,
    fold: &FoldResult,
    deletions: &HashMap<FileId, Vec<Edit>>,
    edits: &mut HashMap<FileId, Vec<Edit>>,
) {
    for (&node, value) in &fold.literals {
        let span = trim_span(program.arena[node].span, program.text(node));
        if span.is_empty() {
            continue;
        }
        let file = program.sources.file_at(span.start).id;
        // Drop substitutions covered by a deletion (e.g. a collapsed branch's condition).
        let covered = deletions.get(&file).is_some_and(|ds| {
            ds.iter()
                .any(|d| d.span.start <= span.start && span.end <= d.span.end)
        });
        if covered {
            continue;
        }
        edits
            .entry(file)
            .or_default()
            .push(Edit::new(span, render_const(value)));
    }
}

/// Add deletion edits that collapse each folded `if`/ternary down to its surviving arm by
/// deleting the scaffolding around it — the condition and dead arm — while leaving the live
/// arm's body in place so its own inner edits (substitutions, tree-shaking) still apply. When
/// the surviving arm is a braced block its braces are stripped too, leaving just the inner
/// statements; a dead `if` with no `else` (or whose arm is an empty block) is deleted whole.
fn add_branch_edits(program: &Program, fold: &FoldResult, edits: &mut HashMap<FileId, Vec<Edit>>) {
    let mut delete = |start: u32, end: u32| {
        if start >= end {
            return;
        }
        let span = Span { start, end };
        let file = program.sources.file_at(start).id;
        edits.entry(file).or_default().push(Edit::new(span, ""));
    };
    for (&node, &branch) in &fold.branches {
        let stmt = program.arena[node].span;
        // The node of the surviving arm, if any. `None` => delete the whole statement.
        let keep = match &program.arena[node].kind {
            NodeKind::IfStmt {
                then_body,
                else_body,
                ..
            } => match branch {
                Branch::Then => Some(*then_body),
                Branch::Else => *else_body,
                Branch::Dead => None,
            },
            NodeKind::TernaryExpr {
                then_branch,
                else_branch,
                ..
            } => match branch {
                Branch::Then => Some(*then_branch),
                Branch::Else | Branch::Dead => Some(*else_branch),
            },
            _ => continue,
        };
        // The span to preserve in place: a braced block's *interior* (so the redundant braces
        // go too), otherwise the arm node itself. An empty block leaves nothing.
        let kept = keep.and_then(|arm| match &program.arena[arm].kind {
            NodeKind::Block { body } => {
                let first = body.first()?;
                let last = body.last()?;
                Some((
                    program.arena[*first].span.start,
                    program.arena[*last].span.end,
                ))
            }
            _ => {
                let s = program.arena[arm].span;
                Some((s.start, s.end))
            }
        });
        match kept {
            Some((start, end)) => {
                delete(stmt.start, start); // leading `if cond {` / `cond ?`
                delete(end, stmt.end); // trailing ` else { … }` / closing `}` / `: …`
            }
            None => delete(stmt.start, stmt.end),
        }
    }
}

/// Shrink `span` to its non-whitespace content, given its source `text`.
fn trim_span(span: Span, text: &str) -> Span {
    let lead = (text.len() - text.trim_start().len()) as u32;
    let trail = (text.len() - text.trim_end().len()) as u32;
    Span {
        start: span.start + lead,
        end: span.end - trail,
    }
}

/// Render a folded constant back to AHK source text.
fn render_const(v: &ConstValue) -> String {
    match v {
        ConstValue::Int(i) => i.to_string(),
        ConstValue::Float(f) => f.to_string(),
        ConstValue::Str(s) => format!("\"{}\"", s.replace('"', "`\"")),
    }
}

/// Add deletion edits for all comment nodes to edits
pub fn add_strip_comment_edits(program: &Program, edits: &mut HashMap<FileId, Vec<Edit>>) {
    // No need for a tree walk, we can just check the arena
    program
        .arena
        .iter()
        .filter(|node| matches!(program.arena[node.0].kind, NodeKind::Comment))
        .filter_map(|(id, _)| {
            let span = program.arena[id].span;
            (!span.is_empty()).then_some(span)
        })
        .for_each(|span| {
            let file = program.sources.file_at(span.start).id;
            edits.entry(file).or_default().push(Edit::new(span, ""));
        });
}

/// Build the per-file source edits that redirect each resolved `#Import` to its target group's
/// in-file module name. Keyed by the file whose text the edit lands in. Imports in `dropped`
/// are skipped — they're being deleted, not redirected.
fn import_edits(
    program: &Program,
    plan: &BundlePlan,
    dropped: &HashSet<NodeId>,
) -> HashMap<FileId, Vec<Edit>> {
    let mut edits: HashMap<FileId, Vec<Edit>> = HashMap::new();
    for ri in &plan.resolved_imports {
        if dropped.contains(&ri.node) {
            continue;
        }
        // The output name of the specific target module this import resolves to.
        let Some(target) = plan.module_names.get(&(ri.group, ri.module)) else {
            continue;
        };
        let target = target.as_str();
        // The span of the import's source spec — a bare name or a quoted path/string.
        let NodeKind::ImportDirective(directive) = &program.arena[ri.node].kind else {
            continue;
        };
        let spec_span = match &directive.source {
            ImportSource::Name(s) | ImportSource::Path(s) => *s,
        };
        // Already spelled exactly as the target module name: nothing to rewrite.
        if program.span_text(spec_span) == target {
            continue;
        }
        let file = program.sources.file_at(spec_span.start).id;
        edits
            .entry(file)
            .or_default()
            .push(Edit::new(spec_span, target));
    }
    edits
}

#[cfg(test)]
mod ws_tests {
    use super::{normalize_whitespace, WsLevel};

    #[test]
    fn off_is_identity() {
        let s = "a\n\n\n  b  \n";
        assert_eq!(normalize_whitespace(s, WsLevel::Off), s);
    }

    #[test]
    fn readable_strips_trailing_and_collapses_blank_runs() {
        let input = "foo\n   \n\n\nbar  \t\n";
        // The 3-blank hole collapses to one; trailing whitespace on `bar` goes.
        assert_eq!(
            normalize_whitespace(input, WsLevel::Readable),
            "foo\n\nbar\n"
        );
    }

    #[test]
    fn readable_keeps_single_author_blank() {
        let input = "a\n\nb\n";
        assert_eq!(normalize_whitespace(input, WsLevel::Readable), "a\n\nb\n");
    }

    #[test]
    fn minify_drops_blanks_and_leading_indent() {
        let input = "foo\n\n    bar\n";
        assert_eq!(normalize_whitespace(input, WsLevel::Minify), "foo\nbar\n");
    }

    #[test]
    fn minify_collapses_inner_spaces_outside_strings() {
        let input = "x    :=    1\n";
        assert_eq!(normalize_whitespace(input, WsLevel::Minify), "x := 1\n");
    }

    #[test]
    fn minify_preserves_spaces_inside_strings() {
        let input = "msg   :=   \"a    b\"\n";
        assert_eq!(
            normalize_whitespace(input, WsLevel::Minify),
            "msg := \"a    b\"\n"
        );
    }

    #[test]
    fn minify_handles_backtick_quote_escape() {
        let input = "s := \"a    `\"    b\"\n";
        assert_eq!(
            normalize_whitespace(input, WsLevel::Minify),
            "s := \"a    `\"    b\"\n"
        );
    }
}
