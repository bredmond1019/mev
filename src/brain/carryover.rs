//! Fleet-wide `carryover[]` sweep — model + predicate extraction.
//!
//! `mev carryover` (see `planning/ticket-carryover-sweep-command/tasks.md`) reads every
//! discovered `planning/state.json`'s `carryover[]` array and, where `clears_when` is
//! machine-checkable, evaluates it, sorting the fleet into three lanes: `cleared`
//! (safe to delete), `actionable` (predicate false, with the unmet reference named),
//! and `not-evaluable` (the predicate is prose, or there is no predicate at all).
//!
//! This module owns the read-only report model ([`CarryoverReport`],
//! [`CarryoverVerdict`], [`CarryoverRef`], [`CarryoverLane`], [`NotEvaluableReason`]),
//! the pure, independently-testable predicate extractors
//! ([`block_refs_from_related`], [`block_refs_from_prose`], [`path_refs_from_prose`]),
//! and the evaluator that assigns a lane to each entry ([`evaluate_carryover`]). The
//! `carryover_sweep` driver (repo discovery + status-map construction) is separate
//! follow-on work in `src/lib.rs`; this module intentionally stops at "given the
//! loaded files and a status map, produce the report".
//!
//! # The two evaluable predicate classes — deliberately narrow
//!
//! **Class A — block references, from `clears_when` only.** Block IDs matched in the
//! `clears_when` prose by a strict grammar ([`block_refs_from_prose`]):
//! `[A-Z]{2,3}\.(?:\d+\.[A-Z0-9]+|ticket\.[a-z0-9][a-z0-9-]*|chore\.[a-z0-9][a-z0-9-]*)`.
//! A match is kept only when (a) the predicate contains a [`CLOSURE_VERBS`] entry, and
//! (b) the token resolves to exactly one node in the loaded corpus (preferring the
//! carryover's own scope repo when the bare ID is ambiguous; if still ambiguous across
//! repos, the match is dropped and the ambiguity is reported instead). An unresolvable
//! token is not a block reference — discarded silently.
//!
//! **`related[]` is NOT a clearing condition** and does not affect the lane.
//! [`block_refs_from_related`] remains available as an accessor, but the schema
//! documents `related[]` as "optional related edges" — a *see also*. A carryover
//! merely related to block X does not clear when X closes, and wiring it into the
//! verdict produced false `cleared` results against the live corpus.
//!
//! Both gates exist because of the same 2026-08-03 finding: `core:ba-0-a-id-collision`
//! reads *"one of the two BA.0.A blocks is renamed and Phase 0 is backfilled"*, and
//! `BA.0.A` **is** closed — so without the closure-verb gate the sweep recommended
//! deleting a live, unresolved known_issue. A false `cleared` is the only verdict here
//! that destroys durable knowledge; every ambiguity resolves away from it.
//!
//! **Class B — path assertions.** A path token (contains `/`, ends in a known
//! extension) is extracted only when `clears_when` also contains a word-bounded
//! entry from [`PATH_PRESENCE_VERBS`] (`exists`, `created`, `added`, `written`,
//! `present`, `corrected`, `fixed`) or [`PATH_ABSENCE_VERBS`] (`removed`,
//! `deleted`, `gone`) — the path analogue of [`CLOSURE_VERBS`]: a path named
//! with no assertion verb is a *subject*, not a *condition*
//! ([`path_refs_from_prose`]). Presence verbs are satisfied when the path
//! exists ([`CarryoverRef::Path`]); absence verbs are satisfied when it does
//! **not** ([`CarryoverRef::PathAbsent`]) — a distinct ref variant because the
//! satisfaction polarity is flipped and conflating the two would let "X is
//! deleted" read as cleared merely because X is *named*, not because it is
//! actually gone.
//!
//! No `regex` dependency is used or added — the grammar is small and fixed, so it is
//! matched by hand (char scanning) in [`extract_block_id_tokens`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use okf_core::{
    ApprovalDep, BlockDep, BlockedBy, Carryover, CarryoverNeeds, CarryoverScope, ClearsWhen,
    ClearsWhenPredicate, ExternalDep, KnownCarryoverNeeds, OperatorDep, StateFile, StateSource,
};

use crate::brain::config::AttentionThresholds;
use crate::brain::state::{carryover_kind_str, carryover_stale_age, is_snoozed, staleness_anchor};

// ---------------------------------------------------------------------------
// Report model
// ---------------------------------------------------------------------------

/// Which of the three lanes a `carryover[]` entry landed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CarryoverLane {
    /// At least one reference was extracted and every one of them is satisfied —
    /// safe to delete (a recommendation for a human, never automatic).
    Cleared,
    /// At least one reference was extracted, but at least one is unsatisfied.
    Actionable,
    /// No reference could be extracted (or there is no `clears_when` at all).
    NotEvaluable,
}

/// Why an entry could not be evaluated.
///
/// Confirmed reachable from `src/brain/state.rs`'s state pass (`MV.ticket.broken-predicate-diagnostic`
/// task 1): [`CarryoverVerdict::reason`] already carries this value out of [`evaluate_carryover`],
/// and [`CarryoverVerdict::repo`] + [`CarryoverVerdict::slug`] are the exact lookup key
/// `check_carryover_already_satisfied` (`state.rs`) already uses to find a `carryover[]` item's
/// verdict inside a loaded [`CarryoverReport`] — see that function's `report.entries.iter().find(...)`
/// pattern, which `check_carryover_broken_predicate` (state.rs) reuses verbatim. In particular
/// [`Self::FileUnreadable`] and [`Self::PatternNotLiteral`] need no new plumbing here: the state
/// pass reads `verdict.reason` directly. Nothing in this module changed for that outflow — the
/// classification below is unmodified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotEvaluableReason {
    /// `clears_when` is present but no reference (block or path) could be
    /// extracted from it — it is pure prose.
    Prose,
    /// `clears_when` is `None`.
    NoPredicate,
    /// A bare block ID in the prose matched nodes in more than one repo and did
    /// not resolve to the carryover's own scope repo either — dropped rather
    /// than guessed at.
    ///
    /// Also produced for a `file_exists`/`file_contains` path that resolves
    /// to two DIFFERENT files under the brain root and the owning repo's
    /// root (or where either candidate's canonicalization fails, the safe
    /// direction) — silently preferring the brain-root candidate would guess
    /// at which file the author meant, so it is dropped rather than guessed
    /// at instead. See [`PathResolution::Ambiguous`].
    AmbiguousReference,
    /// `clears_when` names a block ID but never says the block must *close* —
    /// e.g. *"one of the two BA.0.A blocks is renamed"* or *"BL.2.A's
    /// `blocked_by` is narrowed"*. The ID is a subject, not a closure
    /// condition, so the predicate is not machine-checkable. See
    /// [`CLOSURE_VERBS`].
    NoClosureVerb,
    /// A `CommandExitsZero` predicate was present but execution is not
    /// opted in (see `allow_exec` on [`evaluate_carryover`]). Never
    /// `Cleared` regardless of what the command would have exited — an
    /// unrun command is unknown, and unknown must never read as cleared.
    ExecutionNotAllowed,
    /// A `CommandExitsZero` predicate's child process was still running when
    /// the configured bound elapsed and was killed by the in-process
    /// watchdog. Distinct from a genuine non-zero exit: a timeout tells us
    /// nothing about what the command would have exited, so it is unknown,
    /// not failed, and unknown must never read as `Cleared` — the same
    /// safe-direction rule as [`Self::ExecutionNotAllowed`]. See C141
    /// (`clears-when-network-predicates-can-never-clear`): a network-touching
    /// command can outrun any reasonable in-process bound, and folding that
    /// into a plain `false` reported it as a permanent, indistinguishable
    /// false red.
    CommandTimedOut,
    /// A `CommandExitsZero` predicate's child process could not be spawned at
    /// all. Evidence about the environment the sweep ran in (e.g. `sh` not on
    /// `PATH`), never evidence about the predicate's subject — never
    /// `Cleared`.
    CommandSpawnFailed,
    /// A `FileContains` predicate's target could not be read to completion —
    /// missing, resolved ambiguously under the two-root strategy, larger
    /// than [`FILE_CONTAINS_MAX_BYTES`], or not valid UTF-8. Evidence about
    /// the file, never evidence that the pattern is genuinely absent — only
    /// a file that was read successfully is checkable, so this is never
    /// `Cleared` or `Actionable` alongside a genuine negative.
    FileUnreadable,
    /// A `FileContains` predicate's `pattern` carries a shape (`.*`, `\d`, a
    /// `[...]` class, alternation, anchors, …) that cannot plausibly be the
    /// author's literal intent. The evaluator does literal substring
    /// matching only (see the module header) and a regex-shaped pattern can
    /// therefore never match — evaluating it literally would report a
    /// permanent, indistinguishable false red instead of naming the actual
    /// problem: the pattern was authored as a regex.
    PatternNotLiteral,
    /// `clears_when` mentions a validator/gate/CI concept (see
    /// [`GATE_MENTION_WORDS`]) but no path or block reference could be
    /// extracted from it — e.g. *"the validator is green"* alone. Not
    /// checkable from a data file; surfaced distinctly from plain
    /// [`Self::Prose`] so an operator sees it as a candidate for a typed
    /// `command_exits_zero` predicate rather than as generic unstructured
    /// text. Deriving and running a command from this prose automatically
    /// is explicitly out of scope — this reason only names the possibility
    /// for a human.
    GateMentionNotCheckable,
}

/// One extracted, resolved reference and whether it is currently satisfied.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CarryoverRef {
    /// A block reference, keyed `"{repo}:{id}"`. Satisfied when the referenced
    /// block's authored status is `"closed"`.
    Block { key: String, satisfied: bool },
    /// A path-existence reference. Satisfied when the path resolves to an
    /// existing file (relative to the brain root or the owning repo's path).
    Path { path: String, satisfied: bool },
    /// A path-*absence* reference, produced when prose asserts a path was
    /// removed/deleted/gone rather than that it exists (see
    /// [`PATH_ABSENCE_VERBS`]). Satisfied when the path does **not** resolve
    /// to an existing file — the inverse polarity of [`Self::Path`]. Kept as
    /// a distinct variant rather than a flag on `Path` so a reader of the
    /// enum (and `print_carryover_report`) cannot mistake "the path exists"
    /// for "the path is gone", which is exactly the false-`cleared` shape
    /// this reference exists to avoid.
    PathAbsent { path: String, satisfied: bool },
    /// A typed `block_closed` predicate whose `"{repo}:{id}"` key has no
    /// entry at all in the loaded status map — an unresolvable target
    /// (wrong repo slug, wrong ID, or a corpus that simply never loaded that
    /// repo). Always unsatisfied, and kept distinct from [`Self::Block`] so
    /// the report can tell "the block exists and is open" (an ordinary,
    /// actionable unmet ref) apart from "the block was never found" (a data
    /// problem worth flagging differently) rather than rendering both as an
    /// identical "unmet: key" line.
    UnresolvedBlock { key: String },
    /// A typed `file_contains` predicate. Satisfied when the path resolves
    /// uniquely (same two-root strategy as [`Self::Path`]) and its contents
    /// contain `pattern` as a literal substring; unsatisfied when the file
    /// was read successfully and the pattern is genuinely absent. Produced
    /// only for [`FileContainsOutcome::Found`]/[`FileContainsOutcome::NotFound`]
    /// — a read failure (missing, ambiguous, oversized, unreadable, non-UTF8)
    /// or a regex-shaped pattern produces NO ref at all and forces
    /// `NotEvaluable` instead (see [`NotEvaluableReason::FileUnreadable`] /
    /// [`NotEvaluableReason::PatternNotLiteral`]), so this variant is never a
    /// stand-in for "could not tell".
    FileContains {
        path: String,
        pattern: String,
        satisfied: bool,
    },
    /// A typed `command_exits_zero` predicate, evaluated only when the sweep
    /// opted in (see `allow_exec` on [`evaluate_carryover`]). Satisfied iff
    /// the command's exit status is exactly 0; spawn failure, non-zero exit,
    /// signal death, and an in-process watchdog timeout are all `satisfied:
    /// false`. When execution is not opted in, no `CommandExitsZero` ref is
    /// produced at all — the entry surfaces as `NotEvaluable` with
    /// [`NotEvaluableReason::ExecutionNotAllowed`] instead.
    CommandExitsZero { command: String, satisfied: bool },
}

/// The evaluated verdict for a single `carryover[]` entry.
#[derive(Debug, Clone, Serialize)]
pub struct CarryoverVerdict {
    pub repo: String,
    pub slug: String,
    pub kind: String,
    pub text: String,
    pub clears_when: Option<String>,
    pub created: String,
    /// `None` when the entry is currently snoozed (staleness clock paused) or
    /// has no parseable anchor date; `Some(age_days)` otherwise.
    pub age_days: Option<i64>,
    /// Derived from [`crate::brain::state::carryover_stale_age`] against the
    /// kind's threshold — `true` when that call returned `Some`.
    pub stale: bool,
    pub lane: CarryoverLane,
    pub refs: Vec<CarryoverRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<NotEvaluableReason>,
    /// Authored value-if-resolved priority, passed through verbatim from the
    /// source [`Carryover`] item. Per-repo, never reconciled — see
    /// [`cluster_by_finding_id`]'s doc comment for why divergence across
    /// repos on the same `finding_id` is correct, not a conflict.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    /// Free-form shared identity string, passed through verbatim from the
    /// source [`Carryover`] item. `None`/empty means the entry has not been
    /// linked to a cross-repo finding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finding_id: Option<String>,
    /// Edges to the work this carryover blocks, passed through verbatim from
    /// the source [`Carryover`] item's `blocks[]`. Never authored as a
    /// `blocking: bool` — blocking-ness is always derived from this list, by
    /// [`assign_triage_lane`] (unmet-block membership) and
    /// [`carryover_effective_priorities`] (priority propagation).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub blocks: Vec<BlockedBy>,
    /// Per-entry enforcement opt-out, passed through verbatim from the
    /// source [`Carryover`] item's `enforce` field (`MV.16.C` task 2).
    /// `None` and `Some(true)` both enforce; only `Some(false)` suppresses
    /// every edge this entry's `blocks[]` would otherwise contribute to
    /// [`build_carryover_gating_sets`] — mirroring okf-core's own
    /// `enforce == Some(false)` suppression in its `StateGraph` edge
    /// derivation (`okf-core/src/state.rs:1226`) rather than re-deriving it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforce: Option<bool>,
    /// What kind of work closes this entry (`code`/`docs`/`state`/`operator`/
    /// `dedupe`), passed through verbatim from the source [`Carryover`]
    /// item's `needs` field. `None` when the entry carries no `needs` value
    /// at all — the overwhelming live default (D18,
    /// `MV.ticket.carryover-needs-validation`). Kept as the typed
    /// [`okf_core::CarryoverNeeds`] (not a plain string) so
    /// [`compute_needs_distribution`] can distinguish a known value from an
    /// [`okf_core::CarryoverNeeds::Unknown`] one without re-parsing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs: Option<CarryoverNeeds>,
}

/// Per-`needs`-value counts, computed by [`compute_needs_distribution`].
///
/// The five known values, `unknown` (an authored value outside the fixed
/// vocabulary — [`okf_core::CarryoverNeeds::Unknown`]), and `absent` (no
/// `needs` field at all) are counted SEPARATELY and never folded into a
/// single total: `absent` is currently 361 of 361 live entries, so a report
/// that silently merged it into `unknown` or dropped it would tell the
/// reader nothing about the field's actual coverage — which is the whole
/// point of reporting the distribution from day one rather than assuming it
/// (D18, `MV.ticket.carryover-needs-validation`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct NeedsCounts {
    pub code: usize,
    pub docs: usize,
    pub state: usize,
    pub operator: usize,
    pub dedupe: usize,
    pub unknown: usize,
    pub absent: usize,
}

impl NeedsCounts {
    /// Total entries counted across every bucket, `absent` included.
    pub fn total(&self) -> usize {
        self.code
            + self.docs
            + self.state
            + self.operator
            + self.dedupe
            + self.unknown
            + self.absent
    }

    fn record(&mut self, needs: Option<&CarryoverNeeds>) {
        match needs {
            None => self.absent += 1,
            Some(CarryoverNeeds::Known(KnownCarryoverNeeds::Code)) => self.code += 1,
            Some(CarryoverNeeds::Known(KnownCarryoverNeeds::Docs)) => self.docs += 1,
            Some(CarryoverNeeds::Known(KnownCarryoverNeeds::State)) => self.state += 1,
            Some(CarryoverNeeds::Known(KnownCarryoverNeeds::Operator)) => self.operator += 1,
            Some(CarryoverNeeds::Known(KnownCarryoverNeeds::Dedupe)) => self.dedupe += 1,
            Some(CarryoverNeeds::Unknown(_)) => self.unknown += 1,
        }
    }
}

/// Computes the `needs` distribution over `entries`, per repo AND
/// fleet-wide, in one pass.
///
/// Called from every site that builds a [`CarryoverReport`]'s final entry
/// list — the unfiltered evaluator and the `--grep`-narrowed one alike — so
/// the distribution always describes the same set the report's other counts
/// (`total`/`cleared`/`actionable`/`not_evaluable`) describe, never the whole
/// (unfiltered) corpus. Mirrors that recompute-after-filter discipline
/// exactly (see [`evaluate_carryover_with_grep`]'s doc comment for why).
///
/// Returns `(per_repo, fleet_wide)`. `per_repo` is a [`BTreeMap`] for
/// deterministic iteration order in both the printed summary and `--json`.
pub fn compute_needs_distribution(
    entries: &[CarryoverVerdict],
) -> (BTreeMap<String, NeedsCounts>, NeedsCounts) {
    let mut per_repo: BTreeMap<String, NeedsCounts> = BTreeMap::new();
    let mut fleet = NeedsCounts::default();
    for entry in entries {
        per_repo
            .entry(entry.repo.clone())
            .or_default()
            .record(entry.needs.as_ref());
        fleet.record(entry.needs.as_ref());
    }
    (per_repo, fleet)
}

/// Renders [`compute_needs_distribution`]'s output as the `needs`
/// distribution block of `mev carryover`'s human summary — per-repo rows
/// followed by the fleet-wide total, each bucket named explicitly (including
/// `absent`) so coverage is legible rather than assumed. Returns an empty
/// string (nothing to print) when `entries` is empty, matching how the rest
/// of `mev carryover`'s summary skips empty sections.
pub fn render_needs_distribution_summary(entries: &[CarryoverVerdict]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let (per_repo, fleet) = compute_needs_distribution(entries);
    let mut out = String::new();
    out.push_str(&format!(
        "\nneeds distribution (fleet-wide, {} entries): code={} docs={} state={} operator={} dedupe={} unknown={} absent={}\n",
        fleet.total(),
        fleet.code,
        fleet.docs,
        fleet.state,
        fleet.operator,
        fleet.dedupe,
        fleet.unknown,
        fleet.absent
    ));
    for (repo, counts) in &per_repo {
        out.push_str(&format!(
            "  {repo}: code={} docs={} state={} operator={} dedupe={} unknown={} absent={}\n",
            counts.code,
            counts.docs,
            counts.state,
            counts.operator,
            counts.dedupe,
            counts.unknown,
            counts.absent
        ));
    }
    out
}

/// The full fleet-wide sweep result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CarryoverReport {
    pub total: usize,
    pub cleared: usize,
    pub actionable: usize,
    pub not_evaluable: usize,
    pub entries: Vec<CarryoverVerdict>,
    /// `needs` distribution over `entries`, per repo. See
    /// [`compute_needs_distribution`].
    #[serde(default)]
    pub needs_by_repo: BTreeMap<String, NeedsCounts>,
    /// `needs` distribution over `entries`, fleet-wide. See
    /// [`compute_needs_distribution`].
    #[serde(default)]
    pub needs_fleet: NeedsCounts,
    /// Every `carryover[]` entry sharing an authored `finding_id`, grouped one
    /// cluster per distinct id. See [`cluster_by_finding_id`].
    ///
    /// This is also the corpus-wide `finding_id -> {repos, slugs}` view the
    /// state pass reasons about for `W_STATE_FINDING_ID_ORPHAN`
    /// (`MV.16.D`): each [`FindingCluster`] already carries every repo and
    /// slug an id was used from, keyed by the exact authored string — no
    /// second pass or new plumbing route is needed, the same way
    /// `check_carryover_broken_predicate` (`state.rs`) consumes this report
    /// directly rather than re-walking `files`. `cluster_by_finding_id`'s own
    /// semantics (exact-string grouping, deterministic ordering,
    /// finding_id-less entries excluded) are unchanged by this use.
    pub clusters: Vec<FindingCluster>,
    /// Heuristic candidate-duplicate pairs over entries that carry no
    /// `finding_id` yet. Always unconfirmed — see [`suggest_duplicates`].
    pub suggestions: Vec<DedupSuggestion>,
    /// Sorted list of `finding_id` values whose cluster spans exactly one
    /// repo — the typo guard.
    ///
    /// A `finding_id` is meant to link the *same* finding across repos. One
    /// used in only a single repo did not link anything: the entry *looks*
    /// deduplicated (it carries a `finding_id`) while actually being alone,
    /// which is usually a mistyped id that silently failed to group rather
    /// than a genuinely solitary cross-repo finding. This is the same "field
    /// nothing validates" defect class `MV.ticket.carryover-dedup-clusters`
    /// exists to remove from `finding_id` itself.
    pub single_repo_finding_ids: Vec<String>,
    /// Count of `carryover[]` entries scoped `cross_repo: true` or to a
    /// `tier` (no single owning repo) that an active `--repo` filter
    /// excluded from this run. Always `0` when no `--repo` filter is active.
    /// `MV.ticket.repo-filter-hides-cross-repo-entries` — powers the CLI's
    /// filter-aware summary and empty-result lines, naming
    /// `--include-cross-repo` as the way to widen the view.
    #[serde(default)]
    pub repo_filter_excluded_cross_repo: usize,
}

// ---------------------------------------------------------------------------
// Class A — block references
// ---------------------------------------------------------------------------

/// Block references from a carryover's structured `related[]` edges.
///
/// Every `related[]` entry with `type == "block"` (i.e. `BlockedBy::Block`) becomes
/// a `"{repo}:{id}"` key. `BlockedBy::External` edges are not block references and
/// are skipped. An edge whose `repo` is empty falls back to the carryover's own
/// scope repo (`item.scope.repo`); if that is also absent the edge is skipped
/// rather than keyed with an empty repo.
pub fn block_refs_from_related(item: &Carryover) -> Vec<String> {
    item.related
        .iter()
        .filter_map(|edge| match edge {
            BlockedBy::Block(BlockDep { repo, id, .. }) => {
                let repo = if repo.is_empty() {
                    item.scope.repo.clone()?
                } else {
                    repo.clone()
                };
                Some(format!("{repo}:{id}"))
            }
            BlockedBy::External(_) | BlockedBy::Operator(_) | BlockedBy::Approval(_) => None,
        })
        .collect()
}

/// Block ID tokens matched in free text by the grammar
/// `[A-Z]{2,3}\.(?:\d+\.[A-Z0-9]+|ticket\.[a-z0-9][a-z0-9-]*|chore\.[a-z0-9][a-z0-9-]*)`.
///
/// Hand-scanned (no `regex` dependency). Matching is greedy and non-overlapping:
/// once a match is found starting at a position, the scan resumes immediately
/// after it. A run of 4+ uppercase letters is rejected as a prefix (it is not a
/// bounded 2-3 letter tag), which also prevents `is_ascii_uppercase` prefixes
/// from swallowing unrelated all-caps text.
fn extract_block_id_tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if let Some(end) = match_block_id_at(&chars, i) {
            out.push(chars[i..end].iter().collect());
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// Attempt a single grammar match starting exactly at `start`; returns the
/// exclusive end index on success.
fn match_block_id_at(chars: &[char], start: usize) -> Option<usize> {
    let n = chars.len();

    // Prefix: 2-3 uppercase ASCII letters, not followed by a 4th (that would
    // make it an unbounded run, not a tag).
    let mut p = start;
    let mut letters = 0;
    while p < n && chars[p].is_ascii_uppercase() && letters < 3 {
        p += 1;
        letters += 1;
    }
    if letters < 2 {
        return None;
    }
    if p < n && chars[p].is_ascii_uppercase() {
        return None;
    }
    if p >= n || chars[p] != '.' {
        return None;
    }
    p += 1; // consume '.'

    // Branch 1: \d+\.[A-Z0-9]+
    let digit_start = p;
    let mut dp = p;
    while dp < n && chars[dp].is_ascii_digit() {
        dp += 1;
    }
    if dp > digit_start && dp < n && chars[dp] == '.' {
        let alnum_start = dp + 1;
        let mut ap = alnum_start;
        while ap < n && (chars[ap].is_ascii_uppercase() || chars[ap].is_ascii_digit()) {
            ap += 1;
        }
        if ap > alnum_start {
            return Some(ap);
        }
    }

    // Branch 2/3: ("ticket." | "chore.") [a-z0-9][a-z0-9-]*
    for kw in ["ticket.", "chore."] {
        let kw_chars: Vec<char> = kw.chars().collect();
        let kw_end = p + kw_chars.len();
        if kw_end <= n && chars[p..kw_end] == kw_chars[..] {
            let lower_start = kw_end;
            let mut lp = lower_start;
            if lp < n && (chars[lp].is_ascii_lowercase() || chars[lp].is_ascii_digit()) {
                lp += 1;
                while lp < n
                    && (chars[lp].is_ascii_lowercase()
                        || chars[lp].is_ascii_digit()
                        || chars[lp] == '-')
                {
                    lp += 1;
                }
                return Some(lp);
            }
        }
    }

    None
}

/// The prose string of a `clears_when`, if it is the legacy free-form form.
///
/// Returns `Some(s)` for [`ClearsWhen::Prose`], `None` for
/// [`ClearsWhen::Predicate`]. Every site that previously did
/// `item.clears_when.as_deref()` against the old `Option<String>` shape
/// should now do `item.clears_when.as_ref().and_then(clears_when_prose)` —
/// behaviour-identical for prose entries, and a `Predicate` value produces
/// exactly what a `None` did before (no refs extracted, no display string).
/// This is deliberately temporary: predicate evaluation and a richer display
/// form are `MV.ticket.clears-when-evaluation`'s job, not this one's.
pub fn clears_when_prose(cw: &ClearsWhen) -> Option<&str> {
    match cw {
        ClearsWhen::Prose(s) => Some(s.as_str()),
        ClearsWhen::Predicate(_) => None,
    }
}

/// Human-facing display string for a `clears_when`, for the report/summary
/// sites (`CarryoverVerdict.clears_when`, the staleness-warning and
/// Attention-section formatters).
///
/// Unlike [`clears_when_prose`] — which stays `None` for every
/// `ClearsWhen::Predicate` because the EVALUATION sites depend on that
/// `None` — this renders a compact, unambiguous string for every typed
/// predicate variant too, so an operator hand-verifying a `--dispose`
/// candidate can see what the entry claims to be waiting on straight from
/// the report. Returns an owned `String` (rather than `&str`, like
/// [`clears_when_prose`]) because the predicate branch has nothing to borrow
/// from — it is always freshly formatted.
pub fn clears_when_display(cw: &ClearsWhen) -> Option<String> {
    match cw {
        ClearsWhen::Prose(s) => Some(s.clone()),
        ClearsWhen::Predicate(p) => Some(predicate_display(p)),
    }
}

/// Render one typed [`ClearsWhenPredicate`] compactly and unambiguously for
/// [`clears_when_display`]. The predicate's `note`, when present, is
/// appended so an author's gloss is never silently dropped from the report.
fn predicate_display(p: &ClearsWhenPredicate) -> String {
    let (body, note) = match p {
        ClearsWhenPredicate::BlockClosed { repo, id, note } => {
            (format!("block_closed {repo}:{id}"), note)
        }
        ClearsWhenPredicate::FileExists { path, note } => (format!("file_exists {path}"), note),
        ClearsWhenPredicate::FileContains {
            path,
            pattern,
            note,
        } => (format!("file_contains {path} ~ \"{pattern}\""), note),
        ClearsWhenPredicate::CommandExitsZero { command, note } => {
            (format!("command_exits_zero \"{command}\""), note)
        }
    };
    match note {
        Some(n) => format!("{body} — {n}"),
        None => body,
    }
}

/// Verbs that turn a block ID mentioned in `clears_when` into a *closure*
/// condition rather than a passing reference.
///
/// Without this gate the grammar happily reads `"one of the two BA.0.A blocks
/// is renamed and Phase 0 is backfilled"` as "clears when `BA.0.A` closes" —
/// and since `BA.0.A` *is* closed, the entry was reported `cleared` while the
/// collision it documents is still live. That is a **false cleared**: the one
/// verdict that loses durable knowledge, found against the live corpus on
/// 2026-08-03 before this gate existed.
///
/// Matched word-bounded and case-insensitively against the whole predicate.
pub const CLOSURE_VERBS: &[&str] = &[
    "land", "lands", "landed", "landing", "ship", "ships", "shipped", "shipping", "merge",
    "merges", "merged", "closes", "closed",
];

/// Whether `clears_when` contains a word-bounded [`CLOSURE_VERBS`] entry.
///
/// Word-bounding matters: `"overland"` must not match `"land"`, and
/// `"relationship"` must not match `"ship"`.
pub fn has_closure_verb(clears_when: &str) -> bool {
    let lower = clears_when.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().any(|w| CLOSURE_VERBS.contains(w))
}

/// Block references matched in `clears_when` prose, resolved against the loaded
/// corpus's known `"{repo}:{id}"` keys.
///
/// Returns `([], false)` when the predicate contains no [`CLOSURE_VERBS`] entry:
/// a block ID with no closure verb is a subject, not a condition.
///
/// A grammar match is kept only when it resolves to exactly one key in
/// `known_keys`. When it matches keys in more than one repo, the carryover's own
/// scope repo (`own_repo`) is preferred if present among the matches; otherwise
/// the match is dropped and the returned `bool` (ambiguous) is set to `true`. A
/// grammar match with zero resolving keys is not a block reference at all — it
/// is discarded silently (no ambiguity signal).
///
/// Returns `(resolved_keys, ambiguous)`.
pub fn block_refs_from_prose(
    clears_when: &str,
    own_repo: Option<&str>,
    known_keys: &HashSet<String>,
) -> (Vec<String>, bool) {
    let mut refs = Vec::new();
    let mut ambiguous = false;

    if !has_closure_verb(clears_when) {
        return (refs, ambiguous);
    }

    for token in extract_block_id_tokens(clears_when) {
        let suffix = format!(":{token}");
        let matches: Vec<&String> = known_keys.iter().filter(|k| k.ends_with(&suffix)).collect();
        match matches.len() {
            0 => {} // unresolvable — not a block reference, discard silently
            1 => refs.push(matches[0].clone()),
            _ => {
                if let Some(repo) = own_repo {
                    let preferred = format!("{repo}{suffix}");
                    if matches.iter().any(|k| **k == preferred) {
                        refs.push(preferred);
                        continue;
                    }
                }
                ambiguous = true;
            }
        }
    }

    (refs, ambiguous)
}

// ---------------------------------------------------------------------------
// Class B — path existence
// ---------------------------------------------------------------------------

/// File extensions a path-existence token may end in.
const PATH_EXTENSIONS: &[&str] = &["md", "rs", "py", "sh", "ts", "tsx", "json", "toml"];

/// Whether a path assertion in `clears_when` claims the path is *present* or
/// *absent* — see [`PATH_PRESENCE_VERBS`] / [`PATH_ABSENCE_VERBS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathAssertion {
    /// The predicate claims the path exists / was created / written /
    /// corrected — satisfied when it resolves to an existing file.
    Present,
    /// The predicate claims the path was removed / deleted / is gone —
    /// satisfied when it does **not** resolve to an existing file.
    Absent,
}

/// Verbs asserting that a named path's *presence* is the clearing condition —
/// the path analogue of [`CLOSURE_VERBS`] for the path axis. Includes the
/// `corrected`/`fixed` family: a predicate like *"the count in X.md is
/// corrected"* is only checkable via the named file's existence, so those
/// verbs widen the presence vocabulary rather than adding new grammar (see
/// module header).
pub const PATH_PRESENCE_VERBS: &[&str] = &[
    "exists",
    "created",
    "added",
    "written",
    "present",
    "corrected",
    "fixed",
];

/// Verbs asserting that a named path's *absence* is the clearing condition —
/// satisfied when the path does **not** exist. See [`CarryoverRef::PathAbsent`].
pub const PATH_ABSENCE_VERBS: &[&str] = &["removed", "deleted", "gone"];

/// Word-bounded scan of `clears_when` for a [`PATH_ABSENCE_VERBS`] or
/// [`PATH_PRESENCE_VERBS`] entry. Absence verbs are checked first: they are
/// the smaller, more specific vocabulary, and a predicate is expected to
/// assert one polarity, not both. Returns `None` when neither vocabulary is
/// present — a path named with no assertion verb at all is a *subject*, not
/// a *condition* (the identical trap [`CLOSURE_VERBS`] closes for block IDs).
fn path_assertion(clears_when: &str) -> Option<PathAssertion> {
    let lower = clears_when.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if words.iter().any(|w| PATH_ABSENCE_VERBS.contains(w)) {
        Some(PathAssertion::Absent)
    } else if words.iter().any(|w| PATH_PRESENCE_VERBS.contains(w)) {
        Some(PathAssertion::Present)
    } else {
        None
    }
}

/// Path tokens matched in `clears_when` prose, each paired with the
/// [`PathAssertion`] polarity the predicate asserts for it.
///
/// Returns `[]` when [`path_assertion`] finds neither a presence nor an
/// absence verb — a bounded, documented gate (not the previous single literal
/// `exists` check) that still keeps a bare path mention from being read as a
/// clearing condition. Every whitespace-delimited token containing `/` and
/// ending in one of [`PATH_EXTENSIONS`] is returned, trimmed of surrounding
/// punctuation/quotes. All paths in one predicate share the same polarity —
/// a predicate asserting both presence of one path and absence of another in
/// the same sentence is outside this module's grammar (same "keep it small
/// and fixed" bias as [`extract_block_id_tokens`]).
pub fn path_refs_from_prose(clears_when: &str) -> Vec<(String, PathAssertion)> {
    let Some(assertion) = path_assertion(clears_when) else {
        return Vec::new();
    };

    clears_when
        .split_whitespace()
        .filter_map(|tok| {
            let trimmed = tok.trim_matches(|c: char| !c.is_alphanumeric());
            if !trimmed.contains('/') {
                return None;
            }
            let ext = trimmed.rsplit('.').next()?;
            if PATH_EXTENSIONS.contains(&ext) {
                Some((trimmed.to_string(), assertion))
            } else {
                None
            }
        })
        .collect()
}

/// Words naming a validator/gate/CI concept in `clears_when` — see
/// [`NotEvaluableReason::GateMentionNotCheckable`]. Matched word-bounded and
/// case-insensitively, the same way [`has_closure_verb`] matches
/// [`CLOSURE_VERBS`]. Deliberately does NOT include the bare word `check` —
/// too common in ordinary English to be a reliable gate signal on its own.
pub const GATE_MENTION_WORDS: &[&str] = &[
    "validator",
    "validators",
    "gate",
    "gates",
    "lint",
    "linter",
    "harness",
    "pipeline",
    "suite",
    "ci",
];

/// Whether `clears_when` contains a word-bounded [`GATE_MENTION_WORDS`] entry.
pub fn mentions_gate(clears_when: &str) -> bool {
    let lower = clears_when.to_ascii_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().any(|w| GATE_MENTION_WORDS.contains(w))
}

// ---------------------------------------------------------------------------
// Evaluator — assign a lane per entry
// ---------------------------------------------------------------------------

/// Outcome of resolving a path reference against the two-root strategy (brain
/// root and the owning repo's `repo_path`). Kept distinct from a bare
/// `Option<PathBuf>` so a caller can tell "resolved to exactly one file"
/// apart from "resolved to two DIFFERENT files under the two roots" —
/// collapsing the latter into "the brain-root candidate wins" silently
/// guesses at which file the author meant, which is exactly the false-clear
/// shape `MV.16.G` exists to remove.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathResolution {
    /// Neither root has a FILE at this path. Note: a directory of the same
    /// name does not count — see [`Self::Unique`].
    None,
    /// Exactly one root resolves the path to a file, or both roots resolve
    /// it to the SAME underlying file (canonicalized paths equal) — e.g. a
    /// repo directory reachable through the brain root. Not ambiguous.
    Unique(PathBuf),
    /// Both roots resolve the path to a file, and they are not the same
    /// underlying file (or canonicalization failed for either candidate,
    /// the safe direction) — dropped rather than guessed at. See
    /// [`NotEvaluableReason::AmbiguousReference`].
    Ambiguous { brain: PathBuf, repo: PathBuf },
}

/// Whether a path token resolves, unambiguously, to an existing FILE
/// (never a directory), relative to the brain root or the owning repo's
/// `repo_path`. An [`PathResolution::Ambiguous`] resolution is NOT
/// satisfied — the caller cannot tell which file was meant, so it must not
/// read as "the path exists" any more than as "the path is absent".
fn path_ref_satisfied(
    path: &str,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    owning_repo: &str,
) -> bool {
    matches!(
        resolve_existing_path(path, brain_root, repo_paths, owning_repo),
        PathResolution::Unique(_)
    )
}

/// Resolve a path reference against both roots — the brain root and the
/// owning repo's `repo_path` — and report whether the result is absent,
/// unique, or ambiguous. Requires `is_file()`, not `.exists()`, so a
/// directory of the same name never satisfies a `file_exists`/`file_contains`
/// predicate (symlinks are still followed, which `is_file()` already does).
/// Two candidates that resolve to the SAME file (canonicalized paths equal)
/// are [`PathResolution::Unique`], not ambiguous — a repo directory reachable
/// through the brain root must not produce a spurious ambiguity. If
/// canonicalization fails for either candidate, the result is
/// [`PathResolution::Ambiguous`] — the safe direction.
fn resolve_existing_path(
    path: &str,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    owning_repo: &str,
) -> PathResolution {
    let brain_candidate = brain_root.join(path);
    let brain_hit = brain_candidate.is_file().then_some(brain_candidate);

    let repo_hit = repo_paths.get(owning_repo).and_then(|repo_path| {
        let candidate = repo_path.join(path);
        candidate.is_file().then_some(candidate)
    });

    match (brain_hit, repo_hit) {
        (Some(brain), Some(repo)) => {
            let same_file = match (brain.canonicalize(), repo.canonicalize()) {
                (Ok(b), Ok(r)) => b == r,
                // Canonicalization failure on either side: don't guess.
                _ => false,
            };
            if same_file {
                PathResolution::Unique(brain)
            } else {
                PathResolution::Ambiguous { brain, repo }
            }
        }
        (Some(brain), None) => PathResolution::Unique(brain),
        (None, Some(repo)) => PathResolution::Unique(repo),
        (None, None) => PathResolution::None,
    }
}

/// Bound on how much of a `file_contains` target we will read into memory —
/// a stray binary or huge path named in a data file must not blow up memory
/// during a fleet sweep. 5 MiB comfortably covers any real doc/source file
/// this predicate is meant to check.
const FILE_CONTAINS_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// The observed outcome of evaluating a `file_contains` predicate. Kept
/// distinct from a bare `bool` so a caller can tell "the file was read and
/// the pattern is genuinely absent" (`NotFound`) apart from "we never got a
/// real answer" (`Unreadable`/`PatternNotLiteral`) — collapsing every
/// failure mode (missing, oversized, unreadable, non-UTF-8, resolves
/// ambiguously) into one `false` is exactly the unsoundness `MV.16.G` exists
/// to remove: an unknown outcome must never read as satisfied, and a broken
/// predicate must never be indistinguishable from a real signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileContainsOutcome {
    /// The path resolved to a unique, readable, UTF-8 file within the size
    /// bound, and `pattern` appears in it as a literal substring.
    Found,
    /// The path resolved to a unique, readable, UTF-8 file within the size
    /// bound, and `pattern` does NOT appear in it — a genuine negative.
    NotFound,
    /// The path did not resolve to a unique file (missing, ambiguous under
    /// the two-root strategy, oversized, unreadable, or not valid UTF-8).
    /// Evidence about the file, not about the pattern — see
    /// [`NotEvaluableReason::FileUnreadable`].
    Unreadable,
    /// `pattern` carries a shape (`.*`, `\d`, a `[...]` class, alternation,
    /// anchors, …) that cannot plausibly be the author's literal intent —
    /// the evaluator does literal substring matching only and never adds a
    /// `regex` dependency (see the module header), so a regex-shaped
    /// pattern can never match and would otherwise read as a permanent
    /// false red. See [`NotEvaluableReason::PatternNotLiteral`].
    PatternNotLiteral,
}

/// Detect a pattern shape that cannot plausibly be an author's literal
/// intent — composite regex metacharacter sequences only, never a single
/// bare metacharacter, so a legitimate literal like `docs/cli.md` or
/// `exit $?` is never refused. Deliberately conservative: false negatives
/// (a real regex slipping through as "literal") are cheaper here than false
/// positives (a working literal predicate turned permanently
/// not-evaluable) — the caller only ever falls back to literal substring
/// matching, never to actual regex evaluation, so a slip-through just keeps
/// today's (already sound) literal-match behavior.
fn pattern_is_regex_shaped(pattern: &str) -> bool {
    const COMPOSITE_MARKERS: &[&str] = &[".*", ".+", "\\d", "\\w", "\\s", "\\S", "\\D", "\\W"];
    if COMPOSITE_MARKERS.iter().any(|m| pattern.contains(m)) {
        return true;
    }
    // A bracket class: an unescaped `[` followed somewhere later by `]`.
    if pattern.contains('[') && pattern.contains(']') {
        return true;
    }
    // An alternation group: `(...|...)`.
    if pattern.contains('(') && pattern.contains('|') && pattern.contains(')') {
        return true;
    }
    // Anchors are only refused when they open/close the whole pattern —
    // that is the shape that means "match the whole line/string", not a
    // bare `$` or `^` embedded in prose (e.g. `exit $?`).
    if pattern.starts_with('^') || pattern.ends_with('$') {
        return true;
    }
    false
}

/// Evaluate a `file_contains` predicate: the path resolves uniquely (same
/// two-root strategy as [`path_ref_satisfied`]), its size is within
/// [`FILE_CONTAINS_MAX_BYTES`], its contents decode as UTF-8, `pattern` is
/// not regex-shaped (see [`pattern_is_regex_shaped`]), and `pattern`
/// appears as a literal substring (never a regex — see the module header).
/// Never panics — every failure mode is a [`FileContainsOutcome`] variant.
fn file_contains_outcome(
    path: &str,
    pattern: &str,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    owning_repo: &str,
) -> FileContainsOutcome {
    if pattern_is_regex_shaped(pattern) {
        return FileContainsOutcome::PatternNotLiteral;
    }
    let resolved = match resolve_existing_path(path, brain_root, repo_paths, owning_repo) {
        PathResolution::Unique(resolved) => resolved,
        PathResolution::None | PathResolution::Ambiguous { .. } => {
            return FileContainsOutcome::Unreadable;
        }
    };
    let Ok(metadata) = std::fs::metadata(&resolved) else {
        return FileContainsOutcome::Unreadable;
    };
    if metadata.len() > FILE_CONTAINS_MAX_BYTES {
        return FileContainsOutcome::Unreadable;
    }
    let Ok(bytes) = std::fs::read(&resolved) else {
        return FileContainsOutcome::Unreadable;
    };
    let Ok(contents) = String::from_utf8(bytes) else {
        return FileContainsOutcome::Unreadable;
    };
    if contents.contains(pattern) {
        FileContainsOutcome::Found
    } else {
        FileContainsOutcome::NotFound
    }
}

/// Wall-clock bound for a `command_exits_zero` child process. `timeout(1)`
/// does not exist on this macOS shell, so the bound is enforced in-process
/// by polling `try_wait` and killing the child on expiry — never by
/// shelling out to `timeout`.
pub const COMMAND_EXEC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Poll interval for the in-process watchdog.
const COMMAND_EXEC_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// The observed outcome of running a `command_exits_zero` predicate's child
/// process. Kept distinct from a bare `bool` so a caller can tell "the
/// command ran and told us something" (`ExitZero`/`ExitNonZero`) apart from
/// "we never got a real answer" (`SpawnFailed`/`TimedOut`) — collapsing the
/// four into one `false` is exactly the unsoundness `MV.16.G` exists to
/// remove: an unknown outcome must never read as satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandOutcome {
    /// The child exited with status 0 within the bound.
    ExitZero,
    /// The child exited with a non-zero status (or was killed by a signal)
    /// within the bound — a genuine, observed failure.
    ExitNonZero,
    /// The child could not be spawned at all (e.g. `sh` not on `PATH`). Not
    /// evidence about the predicate's subject — evidence about the
    /// environment the sweep ran in.
    SpawnFailed,
    /// The child was still running when the configured bound elapsed and was
    /// killed and reaped by the in-process watchdog. Unknown, not failed —
    /// see [`NotEvaluableReason::CommandTimedOut`].
    TimedOut,
}

/// Run a `command_exits_zero` predicate's command: spawns `sh -c <command>`
/// in `cwd` and observes its outcome within `timeout`, via an in-process
/// watchdog (`timeout(1)` does not exist on this macOS shell, so the bound
/// is enforced by polling `try_wait` and killing the child on expiry — never
/// by shelling out to `timeout`). Never panics — every failure mode is a
/// [`CommandOutcome`] variant, never an abort of the sweep. Only called when
/// the caller has already confirmed `allow_exec` is set.
fn command_exit_zero_outcome(
    command: &str,
    cwd: &Path,
    timeout: std::time::Duration,
) -> CommandOutcome {
    use std::process::Stdio;

    let mut child = match std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return CommandOutcome::SpawnFailed,
    };

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    CommandOutcome::ExitZero
                } else {
                    CommandOutcome::ExitNonZero
                };
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return CommandOutcome::TimedOut;
                }
                std::thread::sleep(COMMAND_EXEC_POLL_INTERVAL);
            }
            Err(_) => return CommandOutcome::SpawnFailed,
        }
    }
}

/// Rank used to sort lanes `Cleared` < `Actionable` < `NotEvaluable`.
fn lane_rank(lane: CarryoverLane) -> u8 {
    match lane {
        CarryoverLane::Cleared => 0,
        CarryoverLane::Actionable => 1,
        CarryoverLane::NotEvaluable => 2,
    }
}

/// Evaluate every `carryover[]` entry across `files` and sort the fleet into
/// the three lanes.
///
/// `status_map` is a pre-built `"{repo}:{id}"` → authored block status lookup
/// (see `derive_focus`'s local map at `src/brain/state.rs:1298` for the same
/// shape) — it also doubles as the known-key corpus that
/// [`block_refs_from_prose`] resolves prose IDs against. `brain_root` and
/// `repo_paths` (repo slug → absolute repo directory) are used to satisfy
/// Class B path references. `today` is a `YYYY-MM-DD` date string; an
/// unparseable value degrades every entry's `age_days` to `None` and `stale`
/// to `false` rather than panicking. `repo_filter`, when set, restricts the
/// sweep to one repo's entries (matched against the owning file's
/// `StateSource::repo_slug`). `allow_exec` is the opt-in gate for the
/// `CommandExitsZero` typed predicate — see that arm's doc comment for the
/// safe-direction reasoning; it has no effect on any other predicate or on
/// prose extraction. `exec_timeout` is the wall-clock bound the in-process
/// watchdog enforces on a `CommandExitsZero` child process when `allow_exec`
/// is set — pass [`COMMAND_EXEC_TIMEOUT`] for the default 2s bound; it has no
/// effect when `allow_exec` is `false`.
///
/// **References are combined conjunctively (AND), even when the source prose
/// reads as a disjunction ("or").** This is a deliberate safe-direction bias:
/// it can misreport a genuinely-cleared `or`-predicate as `actionable`, but it
/// can never misreport an unmet `and`-predicate as `cleared`. A false
/// `cleared` verdict destroys durable knowledge; a false `actionable` verdict
/// merely wastes a glance. Disjunction parsing is explicitly out of scope
/// (see `planning/ticket-carryover-sweep-command/tasks.md`).
#[allow(clippy::too_many_arguments)]
pub fn evaluate_carryover(
    files: &[(StateSource, StateFile)],
    status_map: &HashMap<String, Option<String>>,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    today: &str,
    thresholds: &AttentionThresholds,
    repo_filter: Option<&str>,
    allow_exec: bool,
    exec_timeout: std::time::Duration,
) -> CarryoverReport {
    evaluate_carryover_with_dedup(
        files,
        status_map,
        brain_root,
        repo_paths,
        today,
        thresholds,
        repo_filter,
        allow_exec,
        true,
        exec_timeout,
    )
}

/// Same as [`evaluate_carryover`], with the O(n²) `clusters`/`suggestions` dedup
/// pass made explicit via `include_dedup`.
///
/// That pass (`cluster_by_finding_id` + `suggest_duplicates`) re-tokenizes every
/// pair of `finding_id`-less entries — cheap at CLI scale, but ~2.2s at the
/// HQ-scoped ~150-entry corpus `bastion serve`'s `/api/attention` sees on every
/// request, none of which is read: `AttentionDto` has no `clusters`/`suggestions`
/// field (`bastion-web-attention-perf`, 2026-08-10). Callers that only need
/// `entries` (`build_attention` in `bastion`) should pass `false`; callers that
/// print or serialize the report (`mev carryover`) should pass `true`.
/// The repo that "owns" a `carryover[]`/`reference[]` entry for the purposes
/// of `--repo <slug>` filtering.
///
/// This is deliberately distinct from the `own_repo` fallback used elsewhere
/// in this module for `clears_when` path/command resolution (which always
/// falls back to the file's repo, even for `tier`/`cross_repo`-scoped
/// entries — that fallback is about "where do we run this check", not "who
/// owns this finding"). For `--repo` filtering specifically: an entry scoped
/// to a `tier` or marked `cross_repo` has no single owning repo and must
/// match no `--repo` filter at all, rather than silently being attributed to
/// whichever file it happens to live in.
///
/// Returns:
/// - `Some(repo)` when `scope.repo` is set — the entry's declared owner.
/// - `None` when `scope.repo` is absent but `scope.tier` or `scope.cross_repo`
///   is set — no single owning repo.
/// - `Some(file_repo)` when the scope is entirely empty — falls back to the
///   file's own repo (same fallback `own_repo` computes today).
fn carryover_filter_owner<'a>(scope: &'a CarryoverScope, file_repo: &'a str) -> Option<&'a str> {
    if let Some(repo) = scope.repo.as_deref() {
        return Some(repo);
    }
    if scope.tier.is_some() || scope.cross_repo.is_some() {
        return None;
    }
    Some(file_repo)
}

#[allow(clippy::too_many_arguments)]
pub fn evaluate_carryover_with_dedup(
    files: &[(StateSource, StateFile)],
    status_map: &HashMap<String, Option<String>>,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    today: &str,
    thresholds: &AttentionThresholds,
    repo_filter: Option<&str>,
    allow_exec: bool,
    include_dedup: bool,
    exec_timeout: std::time::Duration,
) -> CarryoverReport {
    evaluate_carryover_with_dedup_and_widening(
        files,
        status_map,
        brain_root,
        repo_paths,
        today,
        thresholds,
        repo_filter,
        false,
        allow_exec,
        include_dedup,
        exec_timeout,
    )
}

/// Same as [`evaluate_carryover_with_dedup`], with `include_cross_repo` exposed —
/// kept as a separate function (rather than a new parameter on
/// `evaluate_carryover_with_dedup`) so that function's signature stays stable for
/// its existing direct callers outside this crate (`bastion`'s `/api/attention`
/// handler calls it by name). `--include-cross-repo` is CLI-only
/// (`MV.ticket.repo-filter-hides-cross-repo-entries`) and reaches this function
/// only via [`evaluate_carryover_with_grep`]'s unfiltered pass.
///
/// `include_cross_repo`, when `true` together with `repo_filter`, additionally
/// matches entries scoped `cross_repo: true` — widening to the unattributable,
/// never to a *different* named repo. `tier`-scoped entries stay excluded either
/// way (pinned decision: a separate `--include-tier` is out of scope for now).
/// Has no effect when `repo_filter` is `None`.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_carryover_with_dedup_and_widening(
    files: &[(StateSource, StateFile)],
    status_map: &HashMap<String, Option<String>>,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    today: &str,
    thresholds: &AttentionThresholds,
    repo_filter: Option<&str>,
    include_cross_repo: bool,
    allow_exec: bool,
    include_dedup: bool,
    exec_timeout: std::time::Duration,
) -> CarryoverReport {
    let known_keys: HashSet<String> = status_map.keys().cloned().collect();
    let today_date = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d").ok();

    let mut entries: Vec<CarryoverVerdict> = Vec::new();
    // Count of entries scoped `cross_repo: true` or to a `tier` (no single
    // owning repo) that `repo_filter` excluded from this run. `0` whenever
    // `repo_filter` is `None`. Powers `mev carryover`'s filter-aware summary
    // line — see `CarryoverReport::repo_filter_excluded_cross_repo`.
    let mut repo_filter_excluded_cross_repo: usize = 0;

    for (src, file) in files {
        // Cheap pre-pass, kept for perf on a 25-file corpus: a file whose repo
        // doesn't match the filter can still contribute an entry when that
        // entry carries its own `scope.repo` override, or is scoped
        // `cross_repo`/`tier` (no single owning repo — such an entry can live
        // in ANY file, and skipping the file would hide it from both the
        // `--include-cross-repo` widening and the excluded-count above). Only
        // skip the whole file when NONE of those apply.
        if let Some(filter) = repo_filter
            && src.repo_slug != filter
            && !file.carryover.iter().any(|item| {
                item.scope.repo.is_some()
                    || item.scope.tier.is_some()
                    || item.scope.cross_repo.is_some()
            })
        {
            continue;
        }

        for item in &file.carryover {
            let filter_owner = carryover_filter_owner(&item.scope, src.repo_slug.as_str());
            if let Some(filter) = repo_filter {
                let matches = match filter_owner {
                    Some(owner) => owner == filter,
                    None => include_cross_repo && item.scope.cross_repo == Some(true),
                };
                if !matches {
                    if filter_owner.is_none() {
                        repo_filter_excluded_cross_repo += 1;
                    }
                    continue;
                }
            }

            let own_repo = item.scope.repo.as_deref().unwrap_or(src.repo_slug.as_str());

            let mut refs: Vec<CarryoverRef> = Vec::new();
            let mut ambiguous = false;
            let mut forced_reason: Option<NotEvaluableReason> = None;

            // `related[]` is deliberately NOT consulted here. It is documented
            // as "optional related edges" — a *see also*, not a clearing
            // condition. A carryover related to block X does not clear when X
            // closes, and treating it as one produced false `cleared` verdicts
            // against the live corpus. Only `clears_when` decides the lane.
            match item.clears_when.as_ref() {
                Some(ClearsWhen::Prose(clears_when)) => {
                    // Class A: prose block IDs, resolved against the corpus, and
                    // only when the predicate actually asserts closure.
                    let (prose_keys, prose_ambiguous) =
                        block_refs_from_prose(clears_when, Some(own_repo), &known_keys);
                    ambiguous = prose_ambiguous;
                    for key in prose_keys {
                        let satisfied = status_map
                            .get(&key)
                            .map(|s| s.as_deref() == Some("closed"))
                            .unwrap_or(false);
                        refs.push(CarryoverRef::Block { key, satisfied });
                    }

                    // Class B: path assertions, gated by a bounded presence/
                    // absence verb vocabulary (see `path_assertion`).
                    for (path, assertion) in path_refs_from_prose(clears_when) {
                        let exists = path_ref_satisfied(
                            &path,
                            brain_root,
                            repo_paths,
                            src.repo_slug.as_str(),
                        );
                        match assertion {
                            PathAssertion::Present => {
                                refs.push(CarryoverRef::Path {
                                    path,
                                    satisfied: exists,
                                });
                            }
                            PathAssertion::Absent => {
                                refs.push(CarryoverRef::PathAbsent {
                                    path,
                                    satisfied: !exists,
                                });
                            }
                        }
                    }
                }
                Some(ClearsWhen::Predicate(ClearsWhenPredicate::BlockClosed {
                    repo, id, ..
                })) => {
                    let key = format!("{repo}:{id}");
                    match status_map.get(&key) {
                        // Present in the corpus: satisfied iff its authored
                        // status is exactly "closed" — same predicate the
                        // prose path uses above.
                        Some(status) => {
                            let satisfied = status.as_deref() == Some("closed");
                            refs.push(CarryoverRef::Block { key, satisfied });
                        }
                        // Absent from the corpus entirely: an unresolvable
                        // target, never satisfied, and reported distinctly
                        // from a plain unmet `Block` ref (see
                        // `CarryoverRef::UnresolvedBlock`).
                        None => {
                            refs.push(CarryoverRef::UnresolvedBlock { key });
                        }
                    }
                }
                Some(ClearsWhen::Predicate(ClearsWhenPredicate::FileExists { path, .. })) => {
                    // Reuse `resolve_existing_path` verbatim — no second
                    // resolution strategy for the typed form. An ambiguous
                    // resolution pushes no ref and forces `NotEvaluable`
                    // rather than guessing which candidate the author meant.
                    match resolve_existing_path(path, brain_root, repo_paths, own_repo) {
                        PathResolution::Unique(_) => {
                            refs.push(CarryoverRef::Path {
                                path: path.clone(),
                                satisfied: true,
                            });
                        }
                        PathResolution::None => {
                            refs.push(CarryoverRef::Path {
                                path: path.clone(),
                                satisfied: false,
                            });
                        }
                        PathResolution::Ambiguous { .. } => {
                            forced_reason = Some(NotEvaluableReason::AmbiguousReference);
                        }
                    }
                }
                Some(ClearsWhen::Predicate(ClearsWhenPredicate::FileContains {
                    path,
                    pattern,
                    ..
                })) => {
                    // Same two-root resolution strategy as `FileExists`.
                    // `Found`/`NotFound` push a ref as today; `Unreadable`
                    // and `PatternNotLiteral` push none and force
                    // `NotEvaluable` — the same shape `FileExists`'s
                    // `Ambiguous` arm already uses, so a read failure or a
                    // regex-shaped pattern is never indistinguishable from
                    // a genuine negative.
                    match file_contains_outcome(path, pattern, brain_root, repo_paths, own_repo) {
                        FileContainsOutcome::Found => {
                            refs.push(CarryoverRef::FileContains {
                                path: path.clone(),
                                pattern: pattern.clone(),
                                satisfied: true,
                            });
                        }
                        FileContainsOutcome::NotFound => {
                            refs.push(CarryoverRef::FileContains {
                                path: path.clone(),
                                pattern: pattern.clone(),
                                satisfied: false,
                            });
                        }
                        FileContainsOutcome::Unreadable => {
                            forced_reason = Some(NotEvaluableReason::FileUnreadable);
                        }
                        FileContainsOutcome::PatternNotLiteral => {
                            forced_reason = Some(NotEvaluableReason::PatternNotLiteral);
                        }
                    }
                }
                Some(ClearsWhen::Predicate(ClearsWhenPredicate::CommandExitsZero {
                    command,
                    ..
                })) => {
                    if allow_exec {
                        let cwd = repo_paths.get(own_repo).map(PathBuf::as_path).unwrap_or(
                            // Falls back to the brain root when the owning
                            // repo has no known path — still a real cwd,
                            // never a no-op.
                            brain_root,
                        );
                        match command_exit_zero_outcome(command, cwd, exec_timeout) {
                            CommandOutcome::ExitZero => {
                                refs.push(CarryoverRef::CommandExitsZero {
                                    command: command.clone(),
                                    satisfied: true,
                                });
                            }
                            CommandOutcome::ExitNonZero => {
                                refs.push(CarryoverRef::CommandExitsZero {
                                    command: command.clone(),
                                    satisfied: false,
                                });
                            }
                            // Unknown outcomes: no ref, so the entry lands in
                            // `NotEvaluable` rather than `Actionable` next to
                            // a genuinely-failing command — the same shape
                            // `ExecutionNotAllowed` already uses below.
                            CommandOutcome::TimedOut => {
                                forced_reason = Some(NotEvaluableReason::CommandTimedOut);
                            }
                            CommandOutcome::SpawnFailed => {
                                forced_reason = Some(NotEvaluableReason::CommandSpawnFailed);
                            }
                        }
                    } else {
                        // Opt-in is off: this predicate is NOT evaluated at
                        // all — no ref is produced, and the entry is
                        // reported via a dedicated reason rather than
                        // falling through to the generic `NoPredicate`
                        // case. An unrun command is unknown, and unknown
                        // must never read as `Cleared`.
                        forced_reason = Some(NotEvaluableReason::ExecutionNotAllowed);
                    }
                }
                None => {}
            }

            let (lane, reason) = if !refs.is_empty() {
                let all_satisfied = refs.iter().all(|r| match r {
                    CarryoverRef::Block { satisfied, .. } => *satisfied,
                    CarryoverRef::Path { satisfied, .. } => *satisfied,
                    CarryoverRef::PathAbsent { satisfied, .. } => *satisfied,
                    CarryoverRef::UnresolvedBlock { .. } => false,
                    CarryoverRef::FileContains { satisfied, .. } => *satisfied,
                    CarryoverRef::CommandExitsZero { satisfied, .. } => *satisfied,
                });
                let lane = if all_satisfied {
                    CarryoverLane::Cleared
                } else {
                    CarryoverLane::Actionable
                };
                (lane, None)
            } else if let Some(reason) = forced_reason {
                (CarryoverLane::NotEvaluable, Some(reason))
            } else if let Some(ClearsWhen::Prose(clears_when)) = item.clears_when.as_ref() {
                let reason = if ambiguous {
                    NotEvaluableReason::AmbiguousReference
                } else if !has_closure_verb(clears_when)
                    && !extract_block_id_tokens(clears_when).is_empty()
                {
                    // Names a block but never says it must close.
                    NotEvaluableReason::NoClosureVerb
                } else if mentions_gate(clears_when) {
                    // Names a validator/gate/CI concept but nothing checkable
                    // (no path, no block) could be extracted from it — a
                    // candidate for a typed `command_exits_zero` predicate,
                    // never something this sweep derives and runs itself.
                    NotEvaluableReason::GateMentionNotCheckable
                } else {
                    NotEvaluableReason::Prose
                };
                (CarryoverLane::NotEvaluable, Some(reason))
            } else {
                (
                    CarryoverLane::NotEvaluable,
                    Some(NotEvaluableReason::NoPredicate),
                )
            };

            let (age_days, stale) = match today_date {
                Some(today_d) => {
                    let snoozed = is_snoozed(item.snoozed_until.as_deref(), today_d);
                    let age = if snoozed {
                        None
                    } else {
                        staleness_anchor(Some(item.created.as_str()), item.reviewed.as_deref())
                            .map(|anchor| (today_d - anchor).num_days())
                    };
                    let stale = carryover_stale_age(item, today_d, thresholds).is_some();
                    (age, stale)
                }
                None => (None, false),
            };

            entries.push(CarryoverVerdict {
                repo: src.repo_slug.clone(),
                slug: item.slug.clone(),
                kind: carryover_kind_str(&item.kind).into_owned(),
                text: item.text.clone(),
                clears_when: item.clears_when.as_ref().and_then(clears_when_display),
                created: item.created.clone(),
                age_days,
                stale,
                lane,
                refs,
                reason,
                priority: item.priority,
                finding_id: item.finding_id.clone(),
                blocks: item.blocks.clone(),
                enforce: item.enforce,
                needs: item.needs.clone(),
            });
        }
    }

    entries.sort_by(|a, b| {
        lane_rank(a.lane)
            .cmp(&lane_rank(b.lane))
            .then_with(|| b.stale.cmp(&a.stale))
            .then_with(|| a.repo.cmp(&b.repo))
            .then_with(|| a.slug.cmp(&b.slug))
    });

    let cleared = entries
        .iter()
        .filter(|e| e.lane == CarryoverLane::Cleared)
        .count();
    let actionable = entries
        .iter()
        .filter(|e| e.lane == CarryoverLane::Actionable)
        .count();
    let not_evaluable = entries
        .iter()
        .filter(|e| e.lane == CarryoverLane::NotEvaluable)
        .count();
    let total = entries.len();

    // Dedup: cluster on the authored `finding_id`, suggest candidates for the
    // rest, and flag single-repo clusters as likely typos. All three operate
    // purely on the already-built `entries` vector — no re-walk of `files`, no
    // filesystem access, no new discovery pass (`MV.ticket.carryover-dedup-
    // clusters` task 4's no-new-I/O constraint).
    //
    // Gated behind `include_dedup`: `suggest_duplicates` is O(n²) over
    // finding_id-less entries and costs ~2.2s at HQ scope, but its output is
    // discarded by `bastion serve`'s `/api/attention` — see
    // `evaluate_carryover_with_dedup`'s doc comment.
    let (clusters, suggestions, single_repo_finding_ids) = if include_dedup {
        let clusters = cluster_by_finding_id(&entries);
        let suggestions = suggest_duplicates(&entries);
        let mut single_repo_finding_ids: Vec<String> = clusters
            .iter()
            .filter(|c| c.single_repo)
            .map(|c| c.finding_id.clone())
            .collect();
        single_repo_finding_ids.sort();
        (clusters, suggestions, single_repo_finding_ids)
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };

    let (needs_by_repo, needs_fleet) = compute_needs_distribution(&entries);

    CarryoverReport {
        total,
        cleared,
        actionable,
        not_evaluable,
        entries,
        needs_by_repo,
        needs_fleet,
        clusters,
        suggestions,
        single_repo_finding_ids,
        repo_filter_excluded_cross_repo,
    }
}

/// Same as [`evaluate_carryover`], with an optional `--grep <PATTERN>` filter
/// applied to the swept entries before the report's lane counts are
/// (re)computed (`MV.ticket.carryover-grep`, task 2).
///
/// `grep_pattern: None` is a pure pass-through to [`evaluate_carryover`] —
/// identical report, dedup sections included. `grep_pattern: Some(pattern)`
/// runs the full evaluation once (with dedup skipped — see below), then
/// narrows `entries` to the subset [`filter_carryover_entries_by_grep`]
/// selects, and recomputes `total`/`cleared`/`actionable`/`not_evaluable`
/// from THAT filtered subset — never from the full corpus. This is what
/// keeps the header counts and the printed/serialized rows from ever
/// disagreeing: a filter applied after the original (unfiltered) counts were
/// already computed would look correct in isolation while misreporting the
/// fleet.
///
/// The three cross-repo dedup sections (`clusters`, `suggestions`,
/// `single_repo_finding_ids`) are always empty when a filter is active —
/// they are statements about the whole corpus (a `finding_id` used in only
/// one repo, or a heuristic duplicate pair) and computing them over a
/// filtered slice would assert something false about entries the filter
/// excluded. Skipping the (O(n²)) dedup pass here is also why the
/// unfiltered-report evaluation the filtered path is built on passes
/// `include_dedup: false` — that work would only be thrown away.
///
/// Returns `Err` when `pattern` fails to compile as a regex. The error is
/// never swallowed into an empty-report "no matches" result — that would be
/// indistinguishable from a pattern that legitimately matched nothing.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_carryover_with_grep(
    files: &[(StateSource, StateFile)],
    status_map: &HashMap<String, Option<String>>,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    today: &str,
    thresholds: &AttentionThresholds,
    repo_filter: Option<&str>,
    include_cross_repo: bool,
    allow_exec: bool,
    exec_timeout: std::time::Duration,
    grep_pattern: Option<&str>,
) -> Result<CarryoverReport, regex::Error> {
    let Some(pattern) = grep_pattern else {
        return Ok(evaluate_carryover_with_dedup_and_widening(
            files,
            status_map,
            brain_root,
            repo_paths,
            today,
            thresholds,
            repo_filter,
            include_cross_repo,
            allow_exec,
            true,
            exec_timeout,
        ));
    };

    let unfiltered = evaluate_carryover_with_dedup_and_widening(
        files,
        status_map,
        brain_root,
        repo_paths,
        today,
        thresholds,
        repo_filter,
        include_cross_repo,
        allow_exec,
        false,
        exec_timeout,
    );

    let entries = filter_carryover_entries_by_grep(&unfiltered.entries, pattern)?;
    let cleared = entries
        .iter()
        .filter(|e| e.lane == CarryoverLane::Cleared)
        .count();
    let actionable = entries
        .iter()
        .filter(|e| e.lane == CarryoverLane::Actionable)
        .count();
    let not_evaluable = entries
        .iter()
        .filter(|e| e.lane == CarryoverLane::NotEvaluable)
        .count();
    let total = entries.len();

    let (needs_by_repo, needs_fleet) = compute_needs_distribution(&entries);

    Ok(CarryoverReport {
        total,
        cleared,
        actionable,
        not_evaluable,
        entries,
        needs_by_repo,
        needs_fleet,
        clusters: Vec::new(),
        suggestions: Vec::new(),
        single_repo_finding_ids: Vec::new(),
        // Carried from the unfiltered pass, not recomputed over the
        // grep-narrowed subset: this count describes what `--repo` excluded
        // from the corpus, which `--grep` narrowing afterward neither adds
        // to nor removes from.
        repo_filter_excluded_cross_repo: unfiltered.repo_filter_excluded_cross_repo,
    })
}

// ---------------------------------------------------------------------------
// Dispose plan — `mev carryover --dispose` (`MV.ticket.carryover-dispose`
// task 1)
// ---------------------------------------------------------------------------

/// One `carryover[]` entry selected for disposal by `mev carryover --dispose`.
///
/// Pairs a `Cleared`-lane [`CarryoverVerdict`] with the raw [`Carryover`] item
/// it was computed from (kept verbatim so a later write path can
/// `serde(flatten)` it straight into a `CarryoverArchiveRow` — task 2) and a
/// human-readable description of the predicate(s) that cleared it (that
/// row's `evidence` field — task 2).
#[derive(Debug, Clone)]
pub struct DisposalCandidate {
    /// Owning repo slug (`StateSource::repo_slug` / `CarryoverVerdict::repo`).
    pub repo: String,
    /// Stable node key, unique within the owning repo's `carryover[]`.
    pub slug: String,
    /// The raw entry, verbatim, as loaded from the owning repo's `state.json`.
    pub entry: Carryover,
    /// Human-readable description of the predicate(s) that cleared this
    /// entry — see [`describe_clearing_evidence`].
    pub evidence: String,
}

/// One repo the disposal plan could not evaluate at all: its `state.json`
/// failed to load or parse.
///
/// Kept distinct from "loaded, contributed zero disposal candidates" per
/// guard (3) on `MV.ticket.carryover-dispose` — a repo that never evaluated
/// must never be silently treated as having nothing to dispose.
#[derive(Debug, Clone)]
pub struct SkippedRepo {
    /// The repo slug this failure is reported against.
    pub repo: String,
    /// The load/parse error, verbatim, for the disposal run's output.
    pub error: String,
}

/// The read-only plan `mev carryover --dispose` computes before writing
/// anything: which entries it would move (`candidates`) and which repos it
/// could not evaluate at all (`skipped`).
#[derive(Debug, Clone, Default)]
pub struct DisposalPlan {
    pub candidates: Vec<DisposalCandidate>,
    pub skipped: Vec<SkippedRepo>,
}

/// Compute the disposal plan for `mev carryover --dispose` over an
/// already-evaluated sweep.
///
/// `report` is the [`CarryoverReport`] produced by [`evaluate_carryover`] (or
/// [`evaluate_carryover_with_dedup`]) against `files` — the same
/// successfully-loaded corpus. `load_errors` names every repo whose
/// `state.json` failed to load/parse and therefore contributed nothing to
/// either `report` or `files`; each becomes one [`SkippedRepo`] with zero
/// disposal candidates, per guard (3).
///
/// Pure and read-only — touches no filesystem, and decides nothing new about
/// what counts as `Cleared`. It only:
/// 1. Selects the entries `report` already assigned [`CarryoverLane::Cleared`].
/// 2. Looks up each one's raw [`Carryover`] record in `files` by
///    `(repo, slug)` — unique within one state file's `carryover[]` — so the
///    write path (task 2) has the full record to archive verbatim. A verdict
///    with no matching raw entry is skipped defensively (unreachable in
///    practice: every verdict in `report.entries` was produced from exactly
///    one entry in `files` by [`evaluate_carryover_with_dedup`]) rather than
///    panicking or fabricating a record.
/// 3. Renders the evidence string via [`describe_clearing_evidence`],
///    passing `exec_timeout` through so a `command_exits_zero` disposal
///    records the bound that was actually in force for this sweep.
///
/// **Guard (4) — `--dispose` never implies `--allow-exec` — needs no special
/// case here.** `evaluate_carryover_with_dedup` already refuses to mark a
/// `command_exits_zero` predicate `Cleared` unless it was actually run (with
/// `allow_exec: true`) and exited 0; without that opt-in the entry lands in
/// `NotEvaluable` with [`NotEvaluableReason::ExecutionNotAllowed`] instead
/// (see that function's `CommandExitsZero` match arm). So an entry whose
/// command was never run can never appear in `report.entries` with
/// `lane == Cleared` in the first place — selecting on that lane alone is
/// sufficient, and passing `--dispose` without `--allow-exec` naturally
/// yields a report with no `Cleared` `CommandExitsZero` candidates at all.
pub fn compute_disposal_plan(
    report: &CarryoverReport,
    files: &[(StateSource, StateFile)],
    load_errors: &[(String, String)],
    exec_timeout: std::time::Duration,
) -> DisposalPlan {
    let mut candidates = Vec::new();

    for verdict in &report.entries {
        if verdict.lane != CarryoverLane::Cleared {
            continue;
        }

        let Some(entry) = files
            .iter()
            .find(|(src, _)| src.repo_slug == verdict.repo)
            .and_then(|(_, file)| file.carryover.iter().find(|c| c.slug == verdict.slug))
        else {
            continue;
        };

        candidates.push(DisposalCandidate {
            repo: verdict.repo.clone(),
            slug: verdict.slug.clone(),
            entry: entry.clone(),
            evidence: describe_clearing_evidence(verdict, exec_timeout),
        });
    }

    let skipped = load_errors
        .iter()
        .map(|(repo, error)| SkippedRepo {
            repo: repo.clone(),
            error: error.clone(),
        })
        .collect();

    DisposalPlan {
        candidates,
        skipped,
    }
}

/// Render the predicate(s) that cleared one `Cleared`-lane verdict as a
/// single human-readable line — the disposal candidate's `evidence` and,
/// downstream, the archive row's `evidence` field (task 2).
///
/// Every satisfied [`CarryoverRef`] in `verdict.refs` contributes one clause,
/// joined with `"; "`. A `Cleared` verdict always has at least one ref — an
/// empty `refs` list can never reach `Cleared` (see
/// `evaluate_carryover_with_dedup`'s lane assignment: the `Cleared`/
/// `Actionable` split is only reached when `!refs.is_empty()`) — so this
/// never returns an empty string for a genuine disposal candidate.
///
/// `exec_timeout` is the wall-clock bound that was actually in force for
/// this sweep (see [`COMMAND_EXEC_TIMEOUT`] for the default). A
/// `CommandExitsZero` clause records it, so an archived disposal names the
/// watchdog that applied rather than the unfalsifiable `command X exited 0`
/// this used to emit — without it, nothing on disk after the fact says a
/// bound was even in force.
pub fn describe_clearing_evidence(
    verdict: &CarryoverVerdict,
    exec_timeout: std::time::Duration,
) -> String {
    verdict
        .refs
        .iter()
        .map(|r| match r {
            CarryoverRef::Block { key, .. } => format!("block {key} closed"),
            CarryoverRef::Path { path, .. } => format!("path {path} exists"),
            CarryoverRef::PathAbsent { path, .. } => format!("path {path} absent"),
            CarryoverRef::UnresolvedBlock { key } => format!("block {key} unresolved"),
            CarryoverRef::FileContains { path, pattern, .. } => {
                format!("{path} contains \"{pattern}\"")
            }
            CarryoverRef::CommandExitsZero { command, .. } => {
                format!(
                    "command `{command}` exited 0 (bound {}s)",
                    exec_timeout.as_secs()
                )
            }
        })
        .collect::<Vec<String>>()
        .join("; ")
}

// ---------------------------------------------------------------------------
// Dispose write path — `mev carryover --dispose` (`MV.ticket.carryover-dispose`
// task 2)
// ---------------------------------------------------------------------------

/// Build the archive record (okf-core `OK.4.A`) for one disposal candidate.
///
/// The disposed entry is embedded verbatim via `CarryoverArchiveRow`'s own
/// `#[serde(flatten)]` on `entry` — nothing here re-derives or re-shapes it.
/// `reason` is always [`okf_core::DisposalReason::Cleared`]: this write path
/// only ever disposes entries the sweep already landed in
/// [`CarryoverLane::Cleared`] (see [`compute_disposal_plan`], task 1) —
/// `superseded`/`promoted`/`withdrawn` disposals are a different call site
/// this block does not add. `reconstructed` is always `false` — this is a
/// disposal written at the moment it happens, never a historical backfill
/// (that is `MV.16.B`, explicitly out of scope).
pub fn build_archive_row(
    candidate: &DisposalCandidate,
    disposed_at: &str,
) -> okf_core::CarryoverArchiveRow {
    okf_core::CarryoverArchiveRow {
        entry: candidate.entry.clone(),
        disposed_at: disposed_at.to_string(),
        reason: okf_core::DisposalReason::Cleared,
        reconstructed: false,
        evidence: Some(candidate.evidence.clone()),
        amends: None,
    }
}

/// One repo's disposal, written to disk (or, under `--dry-run`, computed but
/// not written) — what task 3's reporting layer prints per repo.
#[derive(Debug, Clone)]
pub struct RepoDisposalWrite {
    /// Owning repo slug.
    pub repo: String,
    /// Absolute path of the repo's `planning/state.json`.
    pub state_path: PathBuf,
    /// Absolute path of the repo's `planning/carryover-archive.jsonl`.
    pub archive_path: PathBuf,
    /// The disposed entries, full record, in the order they were archived —
    /// constraint (5) needs the whole text, not just the slug.
    pub disposed: Vec<DisposalCandidate>,
    /// Whether disk was actually touched (`false` under `--dry-run`, or when
    /// `disposed` is empty and there was nothing to write).
    pub written: bool,
}

/// One repo's disposal failed partway through — which repo, and why, so
/// constraint (3)'s reporting can name it without aborting the whole run.
#[derive(Debug, Clone)]
pub struct RepoDisposalError {
    pub repo: String,
    pub message: String,
}

impl std::fmt::Display for RepoDisposalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.repo, self.message)
    }
}

impl std::error::Error for RepoDisposalError {}

/// Serialize a mutated [`StateFile`] byte-faithfully: `to_string_pretty` plus
/// a trailing newline — the exact shape [`crate::brain::epics::action_for`]
/// and `plan_state_json` already write, so an edit that touches nothing but
/// the `carryover[]` array produces a diff of exactly the removed elements
/// (constraint (2)): no re-indentation, no key reordering, and `serde_json`
/// never escapes non-ASCII (em dashes and friends) by default, so this needs
/// no `ensure_ascii` equivalent — it already behaves as `ensure_ascii=False`.
fn serialize_state_file_pretty(file: &StateFile) -> anyhow::Result<String> {
    let mut content = serde_json::to_string_pretty(file)?;
    content.push('\n');
    Ok(content)
}

/// Apply one repo's disposal: remove every `candidates` entry from
/// `state_file`'s `carryover[]`, append one [`okf_core::CarryoverArchiveRow`]
/// per candidate to `archive_path`, and write both to disk — or, when
/// `dry_run` is `true`, compute the identical result without touching either
/// file (task 3's `--dispose --dry-run` reuses this same function; there is
/// no second code path).
///
/// Empty `candidates` is a no-op that still returns `Ok` with `written:
/// false` — constraint (6) needs a per-repo summary even when nothing moved,
/// and a repo with zero candidates must never attempt (or fail) a write.
///
/// # Ordering — why archive writes before state.json (constraint (1))
///
/// The two writes are staged as complete in-memory strings before either
/// touches disk, then committed **archive first, state.json second**. An
/// archived-but-not-yet-removed entry is merely redundant — it is still
/// correctly present in `carryover[]`. A removed-but-unarchived entry is the
/// data loss this block exists to prevent. Writing the archive first means
/// the only way to reach the dangerous state is for the *second* write (a
/// same-filesystem rename over a file `write_atomic` just staged) to fail
/// after the first already succeeded; on that path this function best-effort
/// reverts the archive file to its original bytes and returns
/// [`RepoDisposalError`] — the repo is reported failed and both files must be
/// checked against their pre-run contents by the caller's tests, never
/// assumed intact from the `Err` alone.
pub fn dispose_repo(
    state_source: &StateSource,
    state_file: &StateFile,
    candidates: &[DisposalCandidate],
    archive_path: &Path,
    disposed_at: &str,
    dry_run: bool,
) -> Result<RepoDisposalWrite, RepoDisposalError> {
    let repo = state_source.repo_slug.clone();

    if candidates.is_empty() {
        return Ok(RepoDisposalWrite {
            repo,
            state_path: state_source.abs_path.clone(),
            archive_path: archive_path.to_path_buf(),
            disposed: Vec::new(),
            written: false,
        });
    }

    let slugs: HashSet<&str> = candidates.iter().map(|c| c.slug.as_str()).collect();

    let mut new_state = state_file.clone();
    new_state
        .carryover
        .retain(|c| !slugs.contains(c.slug.as_str()));
    let new_state_content =
        serialize_state_file_pretty(&new_state).map_err(|e| RepoDisposalError {
            repo: repo.clone(),
            message: format!(
                "failed to serialize {}: {e}",
                state_source.abs_path.display()
            ),
        })?;

    let archive_existed = archive_path.exists();
    let original_archive_content = if archive_existed {
        std::fs::read_to_string(archive_path).map_err(|e| RepoDisposalError {
            repo: repo.clone(),
            message: format!("failed to read {}: {e}", archive_path.display()),
        })?
    } else {
        String::new()
    };

    let mut new_archive_content = original_archive_content.clone();
    for candidate in candidates {
        let row = build_archive_row(candidate, disposed_at);
        let line = serde_json::to_string(&row).map_err(|e| RepoDisposalError {
            repo: repo.clone(),
            message: format!(
                "failed to serialize archive row for '{}': {e}",
                candidate.slug
            ),
        })?;
        new_archive_content.push_str(&line);
        new_archive_content.push('\n');
    }

    if dry_run {
        return Ok(RepoDisposalWrite {
            repo,
            state_path: state_source.abs_path.clone(),
            archive_path: archive_path.to_path_buf(),
            disposed: candidates.to_vec(),
            written: false,
        });
    }

    // Archive first — see the ordering rationale above.
    crate::brain::emit::write_atomic(archive_path, new_archive_content.as_bytes()).map_err(
        |e| RepoDisposalError {
            repo: repo.clone(),
            message: format!("failed to write {}: {e}", archive_path.display()),
        },
    )?;

    if let Err(e) =
        crate::brain::emit::write_atomic(&state_source.abs_path, new_state_content.as_bytes())
    {
        // Best-effort revert of the archive write so a failed run never
        // leaves a disposed entry duplicated between both files.
        let revert_result = if archive_existed {
            crate::brain::emit::write_atomic(archive_path, original_archive_content.as_bytes())
        } else {
            std::fs::remove_file(archive_path)
        };
        let revert_note = if revert_result.is_err() {
            " (archive revert ALSO FAILED — manual check required)"
        } else {
            " (archive write reverted)"
        };
        return Err(RepoDisposalError {
            repo: repo.clone(),
            message: format!(
                "failed to write {}: {e}{revert_note}",
                state_source.abs_path.display()
            ),
        });
    }

    Ok(RepoDisposalWrite {
        repo,
        state_path: state_source.abs_path.clone(),
        archive_path: archive_path.to_path_buf(),
        disposed: candidates.to_vec(),
        written: true,
    })
}

// ---------------------------------------------------------------------------
// Disposal reporting, commit pathspec, and `--dry-run` orchestration —
// `mev carryover --dispose` (`MV.ticket.carryover-dispose` task 3)
// ---------------------------------------------------------------------------

/// Derive a repo's `planning/carryover-archive.jsonl` path from its
/// `planning/state.json` path — the two files always live side by side.
pub fn archive_path_for(state_path: &Path) -> PathBuf {
    state_path.with_file_name("carryover-archive.jsonl")
}

/// The full result of one `mev carryover --dispose` (or `--dispose
/// --dry-run`) run: one [`RepoDisposalWrite`] per successfully-loaded repo
/// [`run_dispose`] reached (even when nothing was disposed there — constraint
/// (6)), any repo whose write failed partway through, and every repo the
/// sweep itself could never reach (`skipped`, carried over from
/// [`DisposalPlan::skipped`] unchanged).
#[derive(Debug, Clone, Default)]
pub struct DisposeRunReport {
    pub writes: Vec<RepoDisposalWrite>,
    pub failures: Vec<RepoDisposalError>,
    pub skipped: Vec<SkippedRepo>,
    pub dry_run: bool,
}

impl DisposeRunReport {
    /// Whether this run is clean enough for `mev carryover --dispose` to
    /// exit 0: a repo the sweep never reached (`skipped`) is reported, not
    /// fatal, but a repo whose write itself failed partway through is.
    pub fn succeeded(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Run (or, under `dry_run`, simulate) the write half of `mev carryover
/// --dispose` over every repo [`compute_disposal_plan`] evaluated.
///
/// This is the single code path for both a real run and `--dispose
/// --dry-run` — see [`dispose_repo`]'s own `dry_run` parameter, which this
/// simply threads through unchanged for every repo in `files`. A repo is
/// visited once per entry in `files` (the already-successfully-loaded
/// corpus [`compute_disposal_plan`] was given), so a repo that contributed
/// zero `Cleared` entries still gets a [`RepoDisposalWrite`] with
/// `disposed: []` — constraint (6) needs a summary line for that repo too,
/// distinguishing "reached, nothing to dispose" from "never reached"
/// (`skipped`). A repo named in `plan.skipped` is never visited here at all:
/// it has no entry in `files`, so both of its files are left byte-identical
/// by construction, not by a special case.
pub fn run_dispose(
    plan: &DisposalPlan,
    files: &[(StateSource, StateFile)],
    disposed_at: &str,
    dry_run: bool,
) -> DisposeRunReport {
    let mut writes = Vec::new();
    let mut failures = Vec::new();

    for (source, file) in files {
        let candidates: Vec<DisposalCandidate> = plan
            .candidates
            .iter()
            .filter(|c| c.repo == source.repo_slug)
            .cloned()
            .collect();
        let archive_path = archive_path_for(&source.abs_path);

        match dispose_repo(
            source,
            file,
            &candidates,
            &archive_path,
            disposed_at,
            dry_run,
        ) {
            Ok(write) => writes.push(write),
            Err(err) => failures.push(err),
        }
    }

    DisposeRunReport {
        writes,
        failures,
        skipped: plan.skipped.clone(),
        dry_run,
    }
}

/// Render one disposal candidate's FULL text — constraint (5): the whole
/// entry, not its slug or a truncated note, so a run whose output scrolled
/// past the terminal's scrollback is still fully readable from this block
/// alone. Pretty-printed JSON of the raw [`Carryover`] record is the
/// unambiguous choice: every field the entry carries (including whatever
/// landed in `extra`) is present verbatim, with nothing summarized or
/// elided.
pub fn render_disposal_candidate_full_text(candidate: &DisposalCandidate) -> String {
    let json = serde_json::to_string_pretty(&candidate.entry)
        .unwrap_or_else(|e| format!("<failed to render entry: {e}>"));
    format!(
        "[{}] disposing '{}' — evidence: {}\n{}",
        candidate.repo, candidate.slug, candidate.evidence, json
    )
}

/// Render every disposal candidate's full text, in plan order, as the
/// preamble a caller prints BEFORE calling [`run_dispose`] — so the full
/// text is on the terminal ahead of the write that moves the entry
/// (constraint (5) is about print ordering relative to the move, not just
/// content). Empty when the plan has no candidates.
pub fn render_dispose_preamble(plan: &DisposalPlan) -> String {
    plan.candidates
        .iter()
        .map(render_disposal_candidate_full_text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Render the exact `git commit -o <pathspec>` line covering every file this
/// run wrote (or, under `--dry-run`, would have written) — constraint (1)'s
/// "both writes land in one commit", made mechanical rather than a
/// remembered follow-up step. A repo with nothing disposed contributes no
/// paths (there is nothing to commit for it); `None` when no repo in the run
/// disposed anything, so a no-op run never prints an empty `git commit -o`
/// with no arguments.
pub fn render_commit_pathspec(report: &DisposeRunReport) -> Option<String> {
    let mut paths: Vec<String> = Vec::new();
    for write in &report.writes {
        if write.disposed.is_empty() {
            continue;
        }
        paths.push(write.state_path.display().to_string());
        paths.push(write.archive_path.display().to_string());
    }

    if paths.is_empty() {
        None
    } else {
        Some(format!("git commit -o {}", paths.join(" ")))
    }
}

/// Render the per-repo summary + commit pathspec block a caller prints AFTER
/// [`run_dispose`] returns — constraint (6): one line for every repo
/// `run_dispose` reached (`0 disposed` included) plus one for every repo
/// `--dispose` could never reach (`SKIPPED`) and every repo whose write
/// itself failed (`FAILED`), so a no-op run is distinguishable from a run
/// that never got there. Identical under `--dry-run` apart from the
/// "(dry-run, not written)" suffix on a repo that had something to dispose —
/// same code path, same content, nothing written.
pub fn render_dispose_summary(report: &DisposeRunReport) -> String {
    let mut lines = Vec::new();

    for write in &report.writes {
        let suffix = if report.dry_run && !write.disposed.is_empty() {
            " (dry-run, not written)"
        } else {
            ""
        };
        lines.push(format!(
            "{}: {} disposed{}",
            write.repo,
            write.disposed.len(),
            suffix
        ));
    }

    for skipped in &report.skipped {
        lines.push(format!("{}: SKIPPED — {}", skipped.repo, skipped.error));
    }

    for failure in &report.failures {
        lines.push(format!("{}: FAILED — {}", failure.repo, failure.message));
    }

    if let Some(pathspec) = render_commit_pathspec(report) {
        lines.push(String::new());
        lines.push(pathspec);
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Historical removal walk — `mev carryover --backfill` (`MV.16.B` task 1)
// ---------------------------------------------------------------------------
//
// A one-time, read-only reconstruction pass over git history: for every
// `planning/state.json` this brain corpus discovers, walk the commits that
// touched it and recover every `carryover[]` entry a commit deleted, taken
// verbatim from the removing commit's PARENT. Nothing here writes anything —
// task 2 (reason derivation + archive row construction) and task 3 (the
// refusal-guarded writer) build on this plan.

/// One historical `carryover[]` removal recovered from git history.
///
/// `entry` is the removed entry exactly as it appeared in the removing
/// commit's PARENT — never re-synthesized from the fields this pass happens
/// to care about, so an unmodeled key (captured in [`Carryover::extra`])
/// survives unchanged into task 2's archive row.
#[derive(Debug, Clone)]
pub struct HistoricalRemoval {
    /// Owning repo slug (the [`StateSource::repo_slug`] of the state file
    /// the removal was found in).
    pub repo: String,
    /// Absolute path of the repo's `planning/carryover-archive.jsonl` — the
    /// file task 3 appends this removal's archive row to.
    pub archive_path: PathBuf,
    /// The removed entry, verbatim, as loaded from the removing commit's
    /// parent revision.
    pub entry: Carryover,
    /// Short (`git log --format=%h`) sha of the commit that removed the
    /// entry.
    pub commit_sha: String,
    /// Subject line of the removing commit — task 2 derives the archive
    /// row's `DisposalReason` from this text.
    pub commit_subject: String,
    /// Committer date of the removing commit, `YYYY-MM-DD`.
    pub commit_date: String,
}

/// One diagnostic surfaced by the history walk without aborting it — either
/// a whole state file git could not be walked at all, or a single historical
/// revision of one that failed to parse. Never a panic: constraint from task
/// 1's own spec ("a state file that fails to parse at some historical
/// revision is a diagnostic, not a panic").
#[derive(Debug, Clone)]
pub struct HistoryWalkDiagnostic {
    /// Owning repo slug the diagnostic is reported against.
    pub repo: String,
    /// Human-readable message — the git error or JSON parse error, verbatim.
    pub message: String,
}

/// The read-only result of `mev carryover --backfill`'s history walk: every
/// recovered [`HistoricalRemoval`], plus every [`HistoryWalkDiagnostic`]
/// raised along the way. `removals.len()` IS the re-derived removal count —
/// `sequence.md` SQ-05's carried-over figure of 311 is never asserted
/// against; this walk's own count is the number.
#[derive(Debug, Clone, Default)]
pub struct HistoryWalkPlan {
    pub removals: Vec<HistoricalRemoval>,
    pub diagnostics: Vec<HistoryWalkDiagnostic>,
}

/// Read one revision of one file as a `carryover[]` list, distinguishing
/// "this revision doesn't have the file" (not an error — a root commit's
/// `<sha>~1`, or the commit that introduced the file) from "the file is
/// there but doesn't parse" (a genuine diagnostic).
///
/// `spec` is a `git show`-style revision spec, `<rev>:<path>`.
fn read_carryover_at_revision(
    git_root: &Path,
    rev: &str,
    rel_path: &str,
) -> Result<Option<Vec<Carryover>>, String> {
    let spec = format!("{rev}:{rel_path}");
    let output = crate::shared::git_command()
        .arg("-C")
        .arg(git_root)
        .arg("show")
        .arg(&spec)
        .output()
        .map_err(|e| format!("failed to run `git show {spec}`: {e}"))?;

    if !output.status.success() {
        // Not present at this point in history — a root commit's `~1`, or
        // the commit that introduced the path. Nothing to diff against;
        // this is not a diagnostic.
        return Ok(None);
    }

    let content = String::from_utf8_lossy(&output.stdout);
    let file: StateFile = serde_json::from_str(&content)
        .map_err(|e| format!("`git show {spec}` did not parse as state.json: {e}"))?;
    Ok(Some(file.carryover))
}

/// Walk one state file's full history and recover every `carryover[]`
/// removal it shows, per [`enumerate_historical_removals`]'s contract.
///
/// `git_root` must already be canonicalized (the caller resolves it once for
/// every source it walks). `source.abs_path` is resolved here because it may
/// reach the file through a `planning/` symlink (D46) — `git log`/`git show`
/// need the real, repo-relative path, not the symlinked one, or they see no
/// history at all (measured: `git log -- <repo>/planning/state.json` returns
/// nothing while `git log -- <repo>/../_planning/<slug>/state.json` returns
/// the real log).
fn walk_removals_for_state_file(
    git_root: &Path,
    source: &StateSource,
) -> (Vec<HistoricalRemoval>, Vec<HistoryWalkDiagnostic>) {
    let mut removals = Vec::new();
    let mut diagnostics = Vec::new();

    let real_path = match source.abs_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            diagnostics.push(HistoryWalkDiagnostic {
                repo: source.repo_slug.clone(),
                message: format!("failed to canonicalize {}: {e}", source.abs_path.display()),
            });
            return (removals, diagnostics);
        }
    };
    let rel_path = match real_path.strip_prefix(git_root) {
        Ok(p) => p,
        Err(_) => {
            diagnostics.push(HistoryWalkDiagnostic {
                repo: source.repo_slug.clone(),
                message: format!(
                    "{} does not resolve under git root {}",
                    real_path.display(),
                    git_root.display()
                ),
            });
            return (removals, diagnostics);
        }
    };
    let rel_path_str = rel_path.to_string_lossy().to_string();
    let archive_path = archive_path_for(&source.abs_path);

    let log_output = crate::shared::git_command()
        .arg("-C")
        .arg(git_root)
        .arg("log")
        .arg("--format=%H%x00%h%x00%cd%x00%s")
        .arg("--date=format:%Y-%m-%d")
        .arg("--follow")
        .arg("--")
        .arg(&rel_path_str)
        .output();

    let output = match log_output {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            diagnostics.push(HistoryWalkDiagnostic {
                repo: source.repo_slug.clone(),
                message: format!(
                    "git log failed for {rel_path_str}: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
            });
            return (removals, diagnostics);
        }
        Err(e) => {
            diagnostics.push(HistoryWalkDiagnostic {
                repo: source.repo_slug.clone(),
                message: format!("failed to run `git log` for {rel_path_str}: {e}"),
            });
            return (removals, diagnostics);
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.splitn(4, '\0');
        let (full_sha, short_sha, date, subject) =
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
                _ => {
                    diagnostics.push(HistoryWalkDiagnostic {
                        repo: source.repo_slug.clone(),
                        message: format!("unparseable `git log` line: {line}"),
                    });
                    continue;
                }
            };

        let child = match read_carryover_at_revision(git_root, full_sha, &rel_path_str) {
            Ok(Some(entries)) => entries,
            Ok(None) => continue,
            Err(message) => {
                diagnostics.push(HistoryWalkDiagnostic {
                    repo: source.repo_slug.clone(),
                    message: format!("{full_sha} ({subject}): {message}"),
                });
                continue;
            }
        };

        let parent_rev = format!("{full_sha}~1");
        let parent = match read_carryover_at_revision(git_root, &parent_rev, &rel_path_str) {
            Ok(Some(entries)) => entries,
            // Root commit, or the commit that introduced the file: no
            // parent version exists to diff against, so nothing was
            // removed here by definition.
            Ok(None) => continue,
            Err(message) => {
                diagnostics.push(HistoryWalkDiagnostic {
                    repo: source.repo_slug.clone(),
                    message: format!("{parent_rev} ({subject}): {message}"),
                });
                continue;
            }
        };

        let child_slugs: HashSet<&str> = child.iter().map(|c| c.slug.as_str()).collect();
        for entry in &parent {
            if !child_slugs.contains(entry.slug.as_str()) {
                removals.push(HistoricalRemoval {
                    repo: source.repo_slug.clone(),
                    archive_path: archive_path.clone(),
                    entry: entry.clone(),
                    commit_sha: short_sha.to_string(),
                    commit_subject: subject.to_string(),
                    commit_date: date.to_string(),
                });
            }
        }
    }

    (removals, diagnostics)
}

/// Compute the full history-walk plan for `mev carryover --backfill` (task
/// 1): discover every `planning/state.json` this brain corpus registers
/// (reusing [`crate::brain::config::find_brain_config`] +
/// [`crate::brain::state::discover_state_files`], the same pair
/// `load_and_evaluate_carryover_corpus_for_dispose` already uses), then walk
/// each one's git history for removed `carryover[]` entries.
///
/// `repo_filter`, when set, restricts the walk to exactly that repo slug and
/// errors out (naming the valid slugs) if the slug is unknown — the same
/// contract `--dispose --repo` already has.
///
/// Read-only: this touches no filesystem beyond reading, spawns no `git`
/// command that mutates the working tree, and returns before any archive
/// write is ever considered (task 3).
pub fn enumerate_historical_removals(
    git_root: &Path,
    repo_filter: Option<&str>,
) -> anyhow::Result<HistoryWalkPlan> {
    let config = crate::brain::config::find_brain_config(git_root)
        .map_err(|e| anyhow::anyhow!("brain.toml not found or unreadable: {e}"))?;
    let (sources, _discovery_diags) = crate::brain::state::discover_state_files(git_root, &config);

    if let Some(slug) = repo_filter
        && !sources.iter().any(|s| s.repo_slug == slug)
    {
        let mut valid_slugs: Vec<&str> = sources.iter().map(|s| s.repo_slug.as_str()).collect();
        valid_slugs.sort_unstable();
        valid_slugs.dedup();
        return Err(anyhow::anyhow!(
            "unknown --repo slug '{slug}'; valid slugs: {}",
            valid_slugs.join(", ")
        ));
    }

    let real_root = git_root
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("failed to canonicalize {}: {e}", git_root.display()))?;

    let mut plan = HistoryWalkPlan::default();
    for source in &sources {
        if let Some(slug) = repo_filter
            && source.repo_slug != slug
        {
            continue;
        }
        let (removals, diagnostics) = walk_removals_for_state_file(&real_root, source);
        plan.removals.extend(removals);
        plan.diagnostics.extend(diagnostics);
    }

    Ok(plan)
}

// ---------------------------------------------------------------------------
// Reason derivation + archive row construction — `mev carryover --backfill`
// (`MV.16.B` task 2)
// ---------------------------------------------------------------------------
//
// `okf_core::DisposalReason` is a closed four-value enum with no `unknown`
// fallback (see its doc comment in okf-core `src/state.rs`) — that type
// lives in another repo and this block does not add a fifth member to it.
// A removing commit whose subject names nothing definite maps to
// `Withdrawn` ("retired without being resolved" — the only member that
// asserts nothing unevidenced), and the uncertainty is recorded in the
// archive row's `evidence` string instead.

/// Derive a [`okf_core::DisposalReason`] from a removing commit's subject,
/// plus whether the mapping was attributable to explicit wording (`true`)
/// or defaulted for lack of any (`false`).
///
/// Matching is case-insensitive substring matching against a small,
/// deliberately narrow keyword set per variant — a heuristic, and kept
/// small and documented in this one place so it reads like one:
///
/// - `Cleared`: wording about the underlying condition resolving —
///   "clear", "resolved", "resolve".
/// - `Superseded`: wording about replacement — "supersede", "superseded",
///   "replace", "replaced".
/// - `Promoted`: wording about graduating into a tracked container —
///   "promote", "promoted".
/// - Anything else: `Withdrawn`, with `attributable: false`.
///
/// The order above is also the match priority when a subject happens to
/// contain more than one keyword family (rare, but a commit message is free
/// text) — `Cleared` is checked first, then `Superseded`, then `Promoted`.
#[must_use]
pub fn derive_disposal_reason(commit_subject: &str) -> (okf_core::DisposalReason, bool) {
    let lower = commit_subject.to_lowercase();

    const CLEARED_WORDS: &[&str] = &["clear", "resolved", "resolve"];
    const SUPERSEDED_WORDS: &[&str] = &["supersede", "superseded", "replace", "replaced"];
    const PROMOTED_WORDS: &[&str] = &["promote", "promoted"];

    if CLEARED_WORDS.iter().any(|w| lower.contains(w)) {
        (okf_core::DisposalReason::Cleared, true)
    } else if SUPERSEDED_WORDS.iter().any(|w| lower.contains(w)) {
        (okf_core::DisposalReason::Superseded, true)
    } else if PROMOTED_WORDS.iter().any(|w| lower.contains(w)) {
        (okf_core::DisposalReason::Promoted, true)
    } else {
        (okf_core::DisposalReason::Withdrawn, false)
    }
}

/// Build the archive record for one [`HistoricalRemoval`] recovered by
/// task 1's history walk.
///
/// The embedded entry is `removal.entry` unchanged — it was already loaded
/// verbatim from the removing commit's PARENT blob by
/// [`walk_removals_for_state_file`], including any key [`Carryover`] does
/// not model (preserved via its own `#[serde(flatten)] extra`). This
/// function never re-synthesizes the entry from selected fields.
///
/// `reconstructed` is always `true` — every row this pass emits is a
/// backfill from history, never a disposal written at the moment it
/// happened (that is [`build_archive_row`], the live `--dispose` path).
///
/// `evidence` always names the removing commit as `<short-sha> <subject>`;
/// when [`derive_disposal_reason`] could not attribute the reason to
/// explicit wording, a trailing note records that the reason was defaulted
/// rather than observed, so a reader of the archive line can tell a
/// confident mapping from a guessed one without re-deriving it.
#[must_use]
pub fn build_historical_archive_row(removal: &HistoricalRemoval) -> okf_core::CarryoverArchiveRow {
    let (reason, attributable) = derive_disposal_reason(&removal.commit_subject);

    let mut evidence = format!("{} {}", removal.commit_sha, removal.commit_subject);
    if !attributable {
        evidence.push_str(" (reason not attributable from commit subject; defaulted to withdrawn)");
    }

    okf_core::CarryoverArchiveRow {
        entry: removal.entry.clone(),
        disposed_at: removal.commit_date.clone(),
        reason,
        reconstructed: true,
        evidence: Some(evidence),
        amends: None,
    }
}

// ---------------------------------------------------------------------------
// The refusal guard + atomic per-repo archive writer — `mev carryover
// --backfill` (`MV.16.B` task 3)
// ---------------------------------------------------------------------------
//
// Idempotency here is by REFUSAL, never by merge: a second run over a
// populated archive must abort the whole run rather than appending
// duplicates or attempting a diff-and-merge. Detection is on the
// `(slug, disposed_at)` pair, the identity `okf_core::AmendsRef` already
// establishes as an archive row's unique key. This pass writes ONLY
// `carryover-archive.jsonl` — it never touches `state.json`, because the
// entries it archives are already gone from it (that is the premise of the
// whole block).

/// One `(slug, disposed_at)` pair a planned backfill row would have
/// duplicated against an already-populated archive — the refusal guard's
/// error. Naming the colliding pair (not just "duplicate found") is the
/// point: the operator needs to know which line to go look at.
#[derive(Debug, Clone)]
pub struct BackfillCollision {
    pub repo: String,
    pub archive_path: PathBuf,
    pub slug: String,
    pub disposed_at: String,
}

impl std::fmt::Display for BackfillCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: archive {} already has a row for (slug='{}', disposed_at='{}') — refusing to backfill over a populated archive",
            self.repo,
            self.archive_path.display(),
            self.slug,
            self.disposed_at
        )
    }
}

impl std::error::Error for BackfillCollision {}

/// One repo's backfill write (or, under `--dry-run`, computed-but-not-written
/// plan) — what task 4's CLI driver prints per repo.
#[derive(Debug, Clone)]
pub struct RepoBackfillWrite {
    /// Owning repo slug.
    pub repo: String,
    /// Absolute path of the repo's `planning/carryover-archive.jsonl`.
    pub archive_path: PathBuf,
    /// The archive rows this repo contributed, in the order they were
    /// appended.
    pub rows: Vec<okf_core::CarryoverArchiveRow>,
    /// Whether disk was actually touched (`false` under `--dry-run`, or when
    /// `rows` is empty and there was nothing to write).
    pub written: bool,
}

/// One repo's backfill write failed partway through — mirrors
/// [`RepoDisposalError`] so task 4's reporting can name the repo without
/// aborting the whole run.
#[derive(Debug, Clone)]
pub struct RepoBackfillError {
    pub repo: String,
    pub message: String,
}

impl std::fmt::Display for RepoBackfillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.repo, self.message)
    }
}

impl std::error::Error for RepoBackfillError {}

/// The full result of one `mev carryover --backfill` (or `--backfill
/// --dry-run`) run: one [`RepoBackfillWrite`] per repo that had at least one
/// removal to backfill, any repo whose write failed partway through, and
/// every [`HistoryWalkDiagnostic`] the walk itself raised.
#[derive(Debug, Clone, Default)]
pub struct BackfillRunReport {
    pub writes: Vec<RepoBackfillWrite>,
    pub failures: Vec<RepoBackfillError>,
    pub diagnostics: Vec<HistoryWalkDiagnostic>,
    pub dry_run: bool,
}

impl BackfillRunReport {
    /// Whether this run is clean enough for `mev carryover --backfill` to
    /// exit 0 — a repo whose write itself failed is fatal.
    pub fn succeeded(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Parse an existing `carryover-archive.jsonl` file's lines into
/// [`okf_core::CarryoverArchiveRow`]s, skipping blank lines. A malformed
/// line is a diagnostic-worthy condition in principle, but this reader is
/// used only to build the collision index before writing — a line that
/// fails to parse cannot be a `(slug, disposed_at)` collision target, so it
/// is skipped rather than aborting the whole backfill (the file predates
/// this pass and may not even exist yet).
fn read_archive_rows(archive_path: &Path) -> Vec<okf_core::CarryoverArchiveRow> {
    let Ok(content) = std::fs::read_to_string(archive_path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<okf_core::CarryoverArchiveRow>(l).ok())
        .collect()
}

/// Run (or, under `dry_run`, simulate) `mev carryover --backfill`'s write
/// half over every [`HistoricalRemoval`] in `plan`.
///
/// # The refusal guard runs FIRST, over the whole plan
///
/// Before any file is touched, every repo's existing archive is read and
/// indexed by `(slug, disposed_at)`. Every planned row (across every repo)
/// is checked against that index; the first collision found aborts the
/// ENTIRE run with `Err(BackfillCollision)` before a single byte is
/// written anywhere — a partial backfill is harder to reason about than
/// none. This is why the guard is a pre-pass rather than folded into the
/// per-repo write loop below: a collision in one repo must not leave a
/// sibling repo's rows written while this repo's are refused.
///
/// # Per-repo atomic write, revert on failure
///
/// Once the guard has cleared, each repo with at least one planned row is
/// written independently: the archive's original bytes are read first, and
/// on any write error the file is reverted to exactly those bytes before
/// the error is collected into `failures` — mirroring [`dispose_repo`]'s
/// own discipline, minus the `state.json` half (this pass never touches
/// `state.json`; the entries it archives are already gone from it).
pub fn run_backfill(
    plan: &HistoryWalkPlan,
    dry_run: bool,
) -> Result<BackfillRunReport, BackfillCollision> {
    // Group removals by repo, preserving plan order.
    let mut repos: Vec<String> = Vec::new();
    let mut by_repo: std::collections::HashMap<String, (PathBuf, Vec<&HistoricalRemoval>)> =
        std::collections::HashMap::new();
    for removal in &plan.removals {
        let entry = by_repo
            .entry(removal.repo.clone())
            .or_insert_with(|| (removal.archive_path.clone(), Vec::new()));
        entry.1.push(removal);
        if !repos.contains(&removal.repo) {
            repos.push(removal.repo.clone());
        }
    }

    // Refusal guard: check every planned row against its repo's existing
    // archive (and against rows already accepted earlier in this same
    // pre-pass, in case the walk itself produced a duplicate pair) before
    // writing anything.
    let mut seen_this_run: HashSet<(String, String, String)> = HashSet::new();
    for repo in &repos {
        let (archive_path, removals) = &by_repo[repo];
        let existing = read_archive_rows(archive_path);
        let existing_keys: HashSet<(String, String)> = existing
            .iter()
            .map(|r| (r.entry.slug.clone(), r.disposed_at.clone()))
            .collect();

        for removal in removals {
            let row = build_historical_archive_row(removal);
            let key = (row.entry.slug.clone(), row.disposed_at.clone());
            if existing_keys.contains(&key)
                || !seen_this_run.insert((repo.clone(), key.0.clone(), key.1.clone()))
            {
                return Err(BackfillCollision {
                    repo: repo.clone(),
                    archive_path: archive_path.clone(),
                    slug: key.0,
                    disposed_at: key.1,
                });
            }
        }
    }

    // Guard cleared — build and (unless dry_run) write each repo's rows.
    let mut writes = Vec::new();
    let mut failures = Vec::new();

    for repo in &repos {
        let (archive_path, removals) = &by_repo[repo];
        let rows: Vec<okf_core::CarryoverArchiveRow> = removals
            .iter()
            .map(|r| build_historical_archive_row(r))
            .collect();

        if dry_run {
            writes.push(RepoBackfillWrite {
                repo: repo.clone(),
                archive_path: archive_path.clone(),
                rows,
                written: false,
            });
            continue;
        }

        let archive_existed = archive_path.exists();
        let original_content = if archive_existed {
            match std::fs::read_to_string(archive_path) {
                Ok(c) => c,
                Err(e) => {
                    failures.push(RepoBackfillError {
                        repo: repo.clone(),
                        message: format!("failed to read {}: {e}", archive_path.display()),
                    });
                    continue;
                }
            }
        } else {
            String::new()
        };

        let mut new_content = original_content.clone();
        let mut serialize_failed = false;
        for row in &rows {
            match serde_json::to_string(row) {
                Ok(line) => {
                    new_content.push_str(&line);
                    new_content.push('\n');
                }
                Err(e) => {
                    failures.push(RepoBackfillError {
                        repo: repo.clone(),
                        message: format!(
                            "failed to serialize archive row for '{}': {e}",
                            row.entry.slug
                        ),
                    });
                    serialize_failed = true;
                    break;
                }
            }
        }
        if serialize_failed {
            continue;
        }

        if let Err(e) = crate::brain::emit::write_atomic(archive_path, new_content.as_bytes()) {
            // Revert to the original bytes so a failed write never leaves
            // the archive partially updated.
            let revert_result = if archive_existed {
                crate::brain::emit::write_atomic(archive_path, original_content.as_bytes())
            } else {
                std::fs::remove_file(archive_path)
            };
            let revert_note = if revert_result.is_err() {
                " (archive revert ALSO FAILED — manual check required)"
            } else {
                " (archive write reverted)"
            };
            failures.push(RepoBackfillError {
                repo: repo.clone(),
                message: format!(
                    "failed to write {}: {e}{revert_note}",
                    archive_path.display()
                ),
            });
            continue;
        }

        writes.push(RepoBackfillWrite {
            repo: repo.clone(),
            archive_path: archive_path.clone(),
            rows,
            written: true,
        });
    }

    Ok(BackfillRunReport {
        writes,
        failures,
        diagnostics: plan.diagnostics.clone(),
        dry_run,
    })
}

/// Render the explicit `git commit -o <pathspec>` line covering every
/// archive file this run wrote (or, under `--dry-run`, would have written)
/// — every `planning/` is a symlink into the one HQ git repo where Standing
/// Rule 10 bans `git add -A`. `None` when no repo in the run wrote
/// anything, so a no-op run never prints an empty `git commit -o` with no
/// arguments.
pub fn render_backfill_commit_pathspec(report: &BackfillRunReport) -> Option<String> {
    let mut paths: Vec<String> = Vec::new();
    for write in &report.writes {
        if write.rows.is_empty() {
            continue;
        }
        paths.push(write.archive_path.display().to_string());
    }

    if paths.is_empty() {
        None
    } else {
        Some(format!("git commit -o {}", paths.join(" ")))
    }
}

/// Render the per-repo backfill summary a caller prints after
/// [`run_backfill`] returns: per repo, the archive path, the row count, and
/// the per-[`okf_core::DisposalReason`] breakdown; then every
/// [`HistoryWalkDiagnostic`] the walk raised; then the commit pathspec.
/// Identical under `--dry-run` apart from the "(dry-run, not written)"
/// suffix on a repo that had rows to write — same code path, same content,
/// nothing written.
pub fn render_backfill_summary(report: &BackfillRunReport) -> String {
    let mut lines = Vec::new();

    for write in &report.writes {
        let suffix = if report.dry_run && !write.rows.is_empty() {
            " (dry-run, not written)"
        } else {
            ""
        };

        let mut cleared = 0usize;
        let mut superseded = 0usize;
        let mut promoted = 0usize;
        let mut withdrawn = 0usize;
        for row in &write.rows {
            match row.reason {
                okf_core::DisposalReason::Cleared => cleared += 1,
                okf_core::DisposalReason::Superseded => superseded += 1,
                okf_core::DisposalReason::Promoted => promoted += 1,
                okf_core::DisposalReason::Withdrawn => withdrawn += 1,
            }
        }

        lines.push(format!(
            "{}: {} — {} row(s){} (cleared={cleared} superseded={superseded} promoted={promoted} withdrawn={withdrawn})",
            write.repo,
            write.archive_path.display(),
            write.rows.len(),
            suffix
        ));
    }

    for failure in &report.failures {
        lines.push(format!("{}: FAILED — {}", failure.repo, failure.message));
    }

    for diag in &report.diagnostics {
        lines.push(format!("{}: DIAGNOSTIC — {}", diag.repo, diag.message));
    }

    if let Some(pathspec) = render_backfill_commit_pathspec(report) {
        lines.push(String::new());
        lines.push(pathspec);
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// `--write` for `mev graph-findings` (`MV.ticket.graph-derived-carryover-findings`
// task 5) — append mechanically-detected findings as typed `carryover[]` entries
// ---------------------------------------------------------------------------
//
// Routed through this module (per the block record) so an emitted entry inherits
// the existing entry shape, the `scope` exactly-one-of rule, and the dispose
// sweep — a `graph-findings`-authored entry is indistinguishable from a hand-filed
// one except for carrying `finding_id`.

/// Slugify a [`GraphFinding`] into a stable, human-readable `carryover[].slug`.
///
/// Not the `finding_id` itself — slugs are meant to read like the fleet's existing
/// hand-authored ones (`epic-weight-not-surfaced-by-bastion`), and `finding_id` is a
/// 64-hex-character digest that would make every diff and board listing unreadable.
/// Built from the detector tag plus the finding's own `subject`, lowercased with
/// every run of non `[a-z0-9]` collapsed to a single `-`, trimmed of leading/trailing
/// `-`, and capped at 80 characters (a subject like a long path stays legible without
/// growing the slug unboundedly). Deterministic and content-derived exactly like
/// `finding_id`, so it does not double as an extra source of drift between two runs
/// over the same corpus.
#[must_use]
pub fn slug_for_finding(finding: &crate::brain::graph_findings::GraphFinding) -> String {
    let raw = format!(
        "graph-finding-{}-{}",
        finding.detector.tag(),
        finding.subject
    );
    let mut slug = String::with_capacity(raw.len());
    let mut last_was_dash = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.len() > 80 {
        trimmed[..80].trim_end_matches('-').to_string()
    } else {
        trimmed.to_string()
    }
}

/// Build the `carryover[]` entry `--write` appends for one [`GraphFinding`].
///
/// - `scope` has exactly one non-null key (`repo`, per the finding's own `repo` —
///   never `tier`/`cross_repo`), so `bastion validate-brain --state` never reports
///   `E_STATE_SCHEMA_MALFORMED_SCOPE` for an emitted entry.
/// - `kind` is [`okf_core::KnownCarryoverKind::Drift`] for both detector classes —
///   an `unregistered-lane-block` finding is a lane record and a `state.json`
///   disagreeing about a block's existence, and a `referenced-path-absent` finding
///   is a command/spec and the filesystem disagreeing about a script's existence;
///   both are "a fact held in two places that no longer agree," which is `drift`'s
///   definition, not `defect`/`deferred`/`env`.
/// - `clears_when` is set from the finding's own typed predicate
///   (`MV.ticket.graph-findings-path-resolution` task 3 — supersedes this
///   function's earlier "deliberately `None`" posture). That earlier
///   reasoning treated "re-run `graph-findings` and this finding is gone" as
///   the only honest predicate, and no `ClearsWhenPredicate` variant
///   expresses "re-run a detector" directly — but the block record's
///   verdict is that emitting `None` at fleet scale converts one-time
///   detection into permanent, unclearable manual debt, which is worse.
///   [`crate::brain::graph_findings::GraphFinding::clears_when`] resolves
///   this instead: both detector classes emit a typed, filesystem-checkable
///   predicate that is a genuine STAND-IN for "re-run the detector" — a
///   `file_exists`/`file_contains` check over the exact fact the detector
///   itself checked — spelled so `mev carryover`'s own evaluator
///   (`path_ref_satisfied`/`file_contains_outcome`) resolves it the same
///   way. `--write`'s idempotence (dedup on `finding_id`) still matters
///   independently: it is what keeps a repeated `--write` from duplicating
///   an entry, not what retires one — that is now the predicate's job.
#[must_use]
pub fn carryover_entry_for_finding(
    finding: &crate::brain::graph_findings::GraphFinding,
    created: &str,
) -> Carryover {
    Carryover {
        slug: slug_for_finding(finding),
        scope: okf_core::CarryoverScope {
            repo: Some(finding.repo.clone()),
            tier: None,
            cross_repo: None,
        },
        kind: okf_core::CarryoverKind::Known(okf_core::KnownCarryoverKind::Drift),
        text: format!(
            "MECHANICALLY DETECTED by `mev graph-findings` ({}). {}",
            finding.detector.tag(),
            finding.message
        ),
        finding_id: Some(finding.finding_id.clone()),
        clears_when: finding.clears_when.clone().map(ClearsWhen::Predicate),
        created: created.to_string(),
        ..Carryover::default()
    }
}

/// Result of attempting `--write` for one repo: how many findings were newly
/// appended (already-present `finding_id`s are silently skipped — the dedup that
/// makes a repeated `--write` idempotent) and whether the file was actually
/// touched.
#[derive(Debug, Clone)]
pub struct GraphFindingsWrite {
    /// Owning repo slug.
    pub repo: String,
    /// Absolute path of the repo's `planning/state.json`.
    pub state_path: PathBuf,
    /// `finding_id`s newly appended this run (excludes ones already present,
    /// which are the idempotence case).
    pub appended: Vec<String>,
    /// Whether disk was actually touched (`false` when every finding for this
    /// repo already had a matching `finding_id` in `carryover[]`).
    pub written: bool,
}

/// Append `findings` scoped to `state_source`'s repo onto `state_file`'s
/// `carryover[]` and write the result back to disk — the disk-facing half of
/// `--write` for one repo. Findings for OTHER repos in `findings` are ignored (the
/// caller is expected to have already partitioned by repo, but filtering here too
/// means a caller mistake produces a no-op for the wrong repo rather than a
/// cross-repo write).
///
/// **Idempotence**: a finding whose `finding_id` already appears on some existing
/// `carryover[]` entry in `state_file` — or was already appended earlier in this
/// same call, covering two findings in one run that happen to normalize to the
/// same subject — is skipped, never duplicated. Running `--write` twice against an
/// unchanged corpus therefore appends nothing the second time.
///
/// **Byte-faithful for the untouched portion**: serialized via
/// [`serialize_state_file_pretty`] — the same `to_string_pretty` + trailing newline
/// [`dispose_repo`] uses — so a write that only appends entries produces a diff of
/// exactly the appended lines, never a re-indent or key-reorder of the rest of the
/// file. Writes nothing to disk (`written: false`) when there is nothing new to
/// append, matching [`dispose_repo`]'s "empty candidates is a no-op" convention.
pub fn write_graph_findings_for_repo(
    state_source: &StateSource,
    state_file: &StateFile,
    findings: &[crate::brain::graph_findings::GraphFinding],
    created: &str,
) -> anyhow::Result<GraphFindingsWrite> {
    let mut seen_ids: HashSet<String> = state_file
        .carryover
        .iter()
        .filter_map(|c| c.finding_id.clone())
        .collect();

    let mut new_state = state_file.clone();
    let mut appended = Vec::new();
    for finding in findings.iter().filter(|f| f.repo == state_source.repo_slug) {
        if seen_ids.contains(&finding.finding_id) {
            continue;
        }
        seen_ids.insert(finding.finding_id.clone());
        new_state
            .carryover
            .push(carryover_entry_for_finding(finding, created));
        appended.push(finding.finding_id.clone());
    }

    if appended.is_empty() {
        return Ok(GraphFindingsWrite {
            repo: state_source.repo_slug.clone(),
            state_path: state_source.abs_path.clone(),
            appended,
            written: false,
        });
    }

    let content = serialize_state_file_pretty(&new_state)?;
    crate::brain::emit::write_atomic(&state_source.abs_path, content.as_bytes())?;

    Ok(GraphFindingsWrite {
        repo: state_source.repo_slug.clone(),
        state_path: state_source.abs_path.clone(),
        appended,
        written: true,
    })
}

// ---------------------------------------------------------------------------
// Audit — `mev carryover --audit` (`MV.ticket.reference-container-validation`
// task 4)
// ---------------------------------------------------------------------------

/// Census of the two triage containers (`carryover[]` and `reference[]`) across the
/// already-loaded corpus, for `mev carryover --audit`.
///
/// Composed mostly from the same `files` slice [`evaluate_carryover`] was given and the
/// [`CarryoverReport`] it already produced — no new corpus walk. `reference[]` entries
/// are never evaluated by `evaluate_carryover` (D72 — they are permanently-true material
/// with no clock and no lane), so their counts are gathered here directly from `files`
/// instead. The one exception is `archive_outflow`: composing it performs one
/// `planning/carryover-archive.jsonl` read per selected repo via
/// [`read_archive_outflow`] — the only filesystem read this function performs, and it
/// happens only here, on the `--audit` path.
///
/// The audit only recommends; it never deletes or rewrites anything.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CarryoverAudit {
    /// `carryover[]` entries plus `reference[]` entries, fleet-wide.
    pub total: usize,
    /// `carryover[]` entry count.
    pub carryover_count: usize,
    /// `reference[]` entry count.
    pub reference_count: usize,
    /// `carryover[]` entries grouped by `kind` (post-D72 narrowing, so this includes
    /// the legacy `constraint`/`known_issue` values wherever they still appear).
    pub per_kind: BTreeMap<String, usize>,
    /// `reference[]` entries grouped by `class` (`trap`/`invariant`/`lesson`/
    /// `deliberate`, plus any not-yet-valid value present in the corpus).
    pub per_class: BTreeMap<String, usize>,
    /// `carryover[]` entries whose `clears_when` is a typed predicate
    /// (`block_closed`/`file_exists`/`file_contains`/`command_exits_zero`), as
    /// opposed to free prose or no predicate at all.
    pub typed_predicate_count: usize,
    /// `carryover[]` entries eligible for a clear-rate denominator — i.e. every
    /// `carryover[]` entry. `reference[]` entries are structurally never clearable
    /// (no `clears_when`) and are excluded here by construction, not by a filter: a
    /// raw per-repo rate that counted them would punish reference-heavy repos for
    /// behaving correctly (measured on the live corpus: `bastiel` 11%,
    /// `okf-core` 0/14 — composition, not discipline).
    pub clearable_total: usize,
    /// `carryover[]` entries [`CarryoverReport`] assigned [`CarryoverLane::Cleared`].
    pub cleared_total: usize,
    /// `cleared_total / clearable_total`, or `0.0` when `clearable_total` is zero.
    pub clear_rate: f64,
    /// Window, in days, `inflow`/`outflow` are measured over.
    pub window_days: i64,
    /// `carryover[]` + `reference[]` entries whose `created` date falls within
    /// `window_days` of `today` — new material entering the corpus.
    pub inflow: usize,
    /// `carryover[]` entries landing in [`CarryoverLane::Cleared`] whose staleness
    /// anchor (`max(created, reviewed)`) falls within `window_days` of `today` —
    /// material that recently became safe to delete. A proxy, not an exact
    /// clear-timestamp: no container here records when an entry was actually
    /// deleted, only when it was last authored or re-affirmed.
    pub outflow: usize,
    /// The measured disposition record over `planning/carryover-archive.jsonl` —
    /// per-`reason` counts of what actually left `carryover[]`, split observed vs.
    /// reconstructed. Distinct from `outflow` above, which is a proxy over entries
    /// still present in `carryover[]` and cannot see a disposal at all.
    pub archive_outflow: ArchiveOutflow,
}

/// Compose a [`CarryoverAudit`] from the same loaded corpus and [`CarryoverReport`]
/// [`evaluate_carryover`] already produced. See [`CarryoverAudit`] for what each field
/// means and why `reference[]` is excluded from every clear-rate denominator.
/// `repo_filter` mirrors [`evaluate_carryover`]'s own `repo_filter` — pass the same
/// value so the audit and the report it was composed alongside agree on scope.
pub fn audit_carryover(
    files: &[(StateSource, StateFile)],
    report: &CarryoverReport,
    today: &str,
    window_days: i64,
    repo_filter: Option<&str>,
) -> CarryoverAudit {
    let today_date = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d").ok();

    let mut carryover_count = 0usize;
    let mut reference_count = 0usize;
    let mut per_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut typed_predicate_count = 0usize;
    let mut inflow = 0usize;

    let within_window = |created: &str| -> bool {
        let (Some(today_d), Some(anchor)) =
            (today_date, crate::brain::state::parse_state_date(created))
        else {
            return false;
        };
        let age = (today_d - anchor).num_days();
        (0..=window_days).contains(&age)
    };

    for (src, file) in files {
        // Same cheap pre-pass as `evaluate_carryover_with_dedup`: only skip a
        // file outright when neither its own repo matches the filter nor any
        // carryover/reference entry in it carries a `scope.repo` override.
        if let Some(filter) = repo_filter
            && src.repo_slug != filter
            && !file.carryover.iter().any(|item| item.scope.repo.is_some())
            && !file.reference.iter().any(|item| item.scope.repo.is_some())
        {
            continue;
        }
        for item in &file.carryover {
            let filter_owner = carryover_filter_owner(&item.scope, src.repo_slug.as_str());
            if let Some(filter) = repo_filter
                && filter_owner != Some(filter)
            {
                continue;
            }
            carryover_count += 1;
            let kind = carryover_kind_str(&item.kind).into_owned();
            *per_kind.entry(kind).or_insert(0) += 1;
            if matches!(item.clears_when, Some(ClearsWhen::Predicate(_))) {
                typed_predicate_count += 1;
            }
            if within_window(&item.created) {
                inflow += 1;
            }
        }
        for item in &file.reference {
            let filter_owner = carryover_filter_owner(&item.scope, src.repo_slug.as_str());
            if let Some(filter) = repo_filter
                && filter_owner != Some(filter)
            {
                continue;
            }
            reference_count += 1;
            *per_class.entry(item.class.clone()).or_insert(0) += 1;
            if within_window(&item.created) {
                inflow += 1;
            }
        }
    }

    let cleared_total = report
        .entries
        .iter()
        .filter(|e| e.lane == CarryoverLane::Cleared)
        .count();
    let clearable_total = carryover_count;
    let clear_rate = if clearable_total == 0 {
        0.0
    } else {
        cleared_total as f64 / clearable_total as f64
    };

    let outflow = report
        .entries
        .iter()
        .filter(|e| {
            e.lane == CarryoverLane::Cleared
                && e.age_days
                    .map(|d| (0..=window_days).contains(&d))
                    .unwrap_or(false)
        })
        .count();

    let archive_outflow = read_archive_outflow(files, today, window_days, repo_filter);

    CarryoverAudit {
        total: carryover_count + reference_count,
        carryover_count,
        reference_count,
        per_kind,
        per_class,
        typed_predicate_count,
        clearable_total,
        cleared_total,
        clear_rate,
        window_days,
        inflow,
        outflow,
        archive_outflow,
    }
}

/// Per-[`okf_core::DisposalReason`] disposition count, split by whether the row was
/// observed at disposal time (`observed`) or reconstructed after the fact from git
/// history by MV.16.B's one-time backfill (`reconstructed`). The two are kept apart
/// deliberately: a reconstructed row carries `reason: unknown`-grade evidence and may
/// include a relocation rather than a true disposal (at least one backfilled removal's
/// commit subject is *"move bastiel-registration carryover to business"*), so blending
/// it into the observed count would inflate a figure a downstream post quotes verbatim.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReasonSplit {
    /// Rows written at live disposal time (`--dispose`).
    pub observed: usize,
    /// Rows backfilled from git history (`--backfill`, MV.16.B).
    pub reconstructed: usize,
}

impl ReasonSplit {
    /// `observed + reconstructed`.
    pub fn total(&self) -> usize {
        self.observed + self.reconstructed
    }
}

/// The measured disposition record over `planning/carryover-archive.jsonl` — a direct
/// count of what actually left `carryover[]` and why, as opposed to [`CarryoverAudit`]'s
/// `outflow` field (a proxy over entries still present in `carryover[]`). See
/// [`read_archive_outflow`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArchiveOutflow {
    /// Total archive rows read across every selected repo's archive.
    pub rows_total: usize,
    /// Of `rows_total`, the rows whose `disposed_at` falls within `window_days` of
    /// `today`. A row whose `disposed_at` fails to parse counts toward `rows_total`
    /// but never toward `rows_in_window`.
    pub rows_in_window: usize,
    /// Disposition counts keyed on the row's `reason`, rendered in the same
    /// lowercase form the enum serializes to (`cleared`/`superseded`/`promoted`/
    /// `withdrawn`), each split observed vs. reconstructed.
    pub per_reason: BTreeMap<String, ReasonSplit>,
    /// Number of `carryover-archive.jsonl` files found and read for the selected
    /// repos.
    pub archives_read: usize,
    /// Number of selected repos with no `carryover-archive.jsonl` on disk yet — the
    /// normal case until `--backfill` or `--dispose` has run once.
    pub archives_missing: usize,
    /// `"<path>:<1-based-line-no>"` for every archive line that failed to parse as
    /// an [`okf_core::CarryoverArchiveRow`]. Never fatal — a malformed line is
    /// named and skipped, not dropped silently and not aborted on.
    pub malformed_lines: Vec<String>,
}

/// Bookkeeping [`collect_archive_rows`] returns alongside the parsed rows: how many
/// archives were actually read vs. missing, and which lines failed to parse. Shared by
/// every caller of [`collect_archive_rows`] so `archives_read`/`archives_missing`/
/// `malformed_lines` are computed exactly once, in exactly one place.
#[derive(Debug, Clone, Default)]
pub struct ArchiveReadStats {
    /// Number of selected repos with a `carryover-archive.jsonl` found and read.
    pub archives_read: usize,
    /// Number of selected repos with no `carryover-archive.jsonl` on disk yet — the
    /// normal case until `--backfill` or `--dispose` has run once.
    pub archives_missing: usize,
    /// `"<path>:<1-based-line-no>"` for every archive line that failed to parse as
    /// an [`okf_core::CarryoverArchiveRow`]. Never fatal — a malformed line is
    /// named and skipped, not dropped silently and not aborted on.
    pub malformed_lines: Vec<String>,
}

/// Read every selected repo's `planning/carryover-archive.jsonl` (derived from its
/// `planning/state.json` path via [`archive_path_for`]) and return the parsed rows plus
/// read/skip bookkeeping.
///
/// `repo_filter` mirrors [`audit_carryover`]'s own filter: a `Some` value restricts the
/// read to files whose `StateSource::repo_slug` matches, so a caller scoped by `--repo X`
/// never reads a different repo's archive. A path already visited (a repo appearing more
/// than once in `files`) is read only once.
///
/// This is the ONE archive reader in this module — [`read_archive_outflow`] (the
/// `--audit` path) and [`build_trajectory`] (the `--trajectory` path, MV.16.F) both
/// delegate to it rather than opening a second parser, so the two commands can never
/// disagree about what rows exist, only about how they're summarized.
pub fn collect_archive_rows(
    files: &[(
        crate::brain::state::StateSource,
        crate::brain::state::StateFile,
    )],
    repo_filter: Option<&str>,
) -> (Vec<okf_core::CarryoverArchiveRow>, ArchiveReadStats) {
    let mut rows = Vec::new();
    let mut stats = ArchiveReadStats::default();
    let mut visited: HashSet<PathBuf> = HashSet::new();

    for (src, _file) in files {
        if let Some(filter) = repo_filter
            && src.repo_slug != filter
        {
            continue;
        }

        let archive_path = archive_path_for(&src.abs_path);
        if !visited.insert(archive_path.clone()) {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&archive_path) else {
            stats.archives_missing += 1;
            continue;
        };
        stats.archives_read += 1;

        for (idx, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<okf_core::CarryoverArchiveRow>(line) {
                Ok(row) => rows.push(row),
                Err(_) => {
                    stats
                        .malformed_lines
                        .push(format!("{}:{}", archive_path.display(), idx + 1));
                }
            }
        }
    }

    (rows, stats)
}

/// Read every selected repo's `planning/carryover-archive.jsonl` (via
/// [`collect_archive_rows`]) and tally disposition counts.
///
/// `repo_filter` mirrors [`audit_carryover`]'s own filter: a `Some` value restricts the
/// read to files whose `StateSource::repo_slug` matches, so `--audit --repo X` never
/// reports outflow for a repo whose inflow it excluded.
///
/// This is the ONE filesystem read this pass introduces, and it is intended to run only
/// on the `--audit` path — see [`audit_carryover`], which is the sole caller.
pub fn read_archive_outflow(
    files: &[(
        crate::brain::state::StateSource,
        crate::brain::state::StateFile,
    )],
    today: &str,
    window_days: i64,
    repo_filter: Option<&str>,
) -> ArchiveOutflow {
    let today_date = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d").ok();

    let (rows, stats) = collect_archive_rows(files, repo_filter);
    let mut outflow = ArchiveOutflow {
        archives_read: stats.archives_read,
        archives_missing: stats.archives_missing,
        malformed_lines: stats.malformed_lines,
        ..Default::default()
    };

    for row in &rows {
        outflow.rows_total += 1;

        if let (Some(today_d), Some(anchor)) = (
            today_date,
            crate::brain::state::parse_state_date(&row.disposed_at),
        ) {
            let age = (today_d - anchor).num_days();
            if (0..=window_days).contains(&age) {
                outflow.rows_in_window += 1;
            }
        }

        let reason_key = match row.reason {
            okf_core::DisposalReason::Cleared => "cleared",
            okf_core::DisposalReason::Superseded => "superseded",
            okf_core::DisposalReason::Promoted => "promoted",
            okf_core::DisposalReason::Withdrawn => "withdrawn",
        };
        let split = outflow
            .per_reason
            .entry(reason_key.to_string())
            .or_default();
        if row.reconstructed {
            split.reconstructed += 1;
        } else {
            split.observed += 1;
        }
    }

    outflow
}

/// One week's row in [`TrajectoryReport`]'s table — the ISO week of `disposed_at`
/// (`YYYY-Www`, zero-padded), the observed/reconstructed split for that week, and the
/// running cumulative total through the end of that week (inclusive of
/// [`TrajectoryReport::before_window`] and every earlier emitted week).
#[derive(Debug, Clone, Default, Serialize)]
pub struct WeekRow {
    /// ISO-8601 week label, e.g. `"2026-W35"`.
    pub iso_week: String,
    /// Rows disposed at live disposal time (`--dispose`) that fall in this week.
    pub observed: usize,
    /// Rows backfilled from git history (`--backfill`, MV.16.B) that fall in this week.
    pub reconstructed: usize,
    /// Running total: `before_window` + every earlier emitted week's `observed +
    /// reconstructed` + this week's own `observed + reconstructed`.
    pub cumulative: usize,
}

impl WeekRow {
    /// `observed + reconstructed` for this week alone (not cumulative).
    pub fn total(&self) -> usize {
        self.observed + self.reconstructed
    }
}

/// The weekly outflow trajectory over `planning/carryover-archive.jsonl` — `mev
/// carryover --trajectory` (MV.16.F). Built by [`build_trajectory`] from the exact same
/// rows [`read_archive_outflow`] reads (via [`collect_archive_rows`]), so its last row's
/// `cumulative` (plus `undated`) equals [`ArchiveOutflow::rows_total`] for the same
/// `repo_filter` whenever the requested window covers the whole archive — the coherence
/// guarantee this command exists to keep true.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TrajectoryReport {
    /// Exactly the requested number of week rows, most recent (the week containing
    /// `today`) last. Includes weeks with zero disposals.
    pub weeks: Vec<WeekRow>,
    /// Archive rows whose `disposed_at` falls strictly before the first emitted week —
    /// folded into the first row's `cumulative` but never shown as its own week.
    pub before_window: usize,
    /// Total archive rows read across every selected repo's archive (mirrors
    /// [`ArchiveOutflow::rows_total`] for the same scope).
    pub rows_total: usize,
    /// Number of selected repos with a `carryover-archive.jsonl` found and read.
    pub archives_read: usize,
    /// Number of selected repos with no `carryover-archive.jsonl` on disk yet.
    pub archives_missing: usize,
    /// `"<path>:<1-based-line-no>"` for every archive line that failed to parse.
    pub malformed_lines: Vec<String>,
    /// Rows whose `disposed_at` does not parse — excluded from every week bucket and
    /// from `before_window`, but still counted in `rows_total`. A row here cannot be
    /// placed on the trajectory at all: silently bucketing an unparseable date would
    /// put a lie in a published table, so it is named instead and left out.
    pub undated: usize,
}

/// Build the weekly outflow trajectory over the same archive rows [`read_archive_outflow`]
/// reads — via [`collect_archive_rows`], never a second parser and never git. Git history
/// was MV.16.B's one-time reconstruction pass that populated the archive in the first
/// place; a trajectory command that re-walked git would recreate the problem the archive
/// exists to solve, and would disagree with `--audit` the moment a disposal happened
/// outside the walked range.
///
/// Emits exactly `weeks` rows ending with the ISO week containing `today`, walking
/// backwards 7 days at a time (7 days always advances exactly one ISO week, so no
/// day-of-week alignment is needed). Weeks with zero disposals are included — a
/// collapsed gap would misrepresent the trajectory. `today` is caller-supplied rather
/// than read from the clock, so callers (and tests) can pin it.
pub fn build_trajectory(
    files: &[(
        crate::brain::state::StateSource,
        crate::brain::state::StateFile,
    )],
    today: &str,
    weeks: usize,
    repo_filter: Option<&str>,
) -> TrajectoryReport {
    use chrono::Datelike;

    let (rows, stats) = collect_archive_rows(files, repo_filter);

    let mut report = TrajectoryReport {
        archives_read: stats.archives_read,
        archives_missing: stats.archives_missing,
        malformed_lines: stats.malformed_lines,
        ..Default::default()
    };

    let Some(today_date) = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d").ok() else {
        // No valid `today` to anchor the window on — every row is undated relative to
        // the trajectory (still counted in `rows_total`), and no week rows are emitted.
        report.rows_total = rows.len();
        report.undated = rows.len();
        return report;
    };

    // Anchor dates for each emitted week, oldest first, ending on `today_date`. Walking
    // back 7 days at a time always lands one ISO week earlier.
    let mut week_dates: Vec<chrono::NaiveDate> = Vec::with_capacity(weeks);
    for i in (0..weeks).rev() {
        let offset = chrono::Duration::days(7 * i as i64);
        if let Some(d) = today_date.checked_sub_signed(offset) {
            week_dates.push(d);
        }
    }

    let iso_label = |d: chrono::NaiveDate| -> String {
        let iw = d.iso_week();
        format!("{}-W{:02}", iw.year(), iw.week())
    };

    let week_labels: Vec<String> = week_dates.iter().map(|d| iso_label(*d)).collect();
    report.weeks = week_labels
        .iter()
        .map(|label| WeekRow {
            iso_week: label.clone(),
            ..Default::default()
        })
        .collect();

    let first_label = week_labels.first().cloned();
    let last_index = report.weeks.len().saturating_sub(1);

    for row in &rows {
        report.rows_total += 1;

        let Some(anchor) = crate::brain::state::parse_state_date(&row.disposed_at) else {
            report.undated += 1;
            continue;
        };

        let label = iso_label(anchor);
        let target_index = week_labels.iter().position(|l| *l == label);

        match target_index {
            Some(idx) => {
                let wr = &mut report.weeks[idx];
                if row.reconstructed {
                    wr.reconstructed += 1;
                } else {
                    wr.observed += 1;
                }
            }
            None if first_label.as_deref().is_some_and(|f| label.as_str() < f) => {
                report.before_window += 1;
            }
            None if !report.weeks.is_empty() => {
                // Later than the last emitted week (a future-dated row) — fold into the
                // last week rather than dropping it, so `cumulative` on the last row
                // always equals `rows_total - undated` regardless of clock skew.
                let wr = &mut report.weeks[last_index];
                if row.reconstructed {
                    wr.reconstructed += 1;
                } else {
                    wr.observed += 1;
                }
            }
            None => {
                // No weeks requested (`weeks == 0`) — nothing to place it in but
                // `before_window`, so it goes there rather than being silently lost.
                report.before_window += 1;
            }
        }
    }

    let mut running = report.before_window;
    for wr in &mut report.weeks {
        running += wr.total();
        wr.cumulative = running;
    }

    report
}

// --- Dedup: tokenization + similarity ---
//
// `MV.ticket.carryover-dedup-clusters` task 1. Pure, dependency-free primitives that
// support two later passes: exact clustering on the authored `finding_id` field
// (trusted — a human wrote it) and a heuristic token-overlap suggestion pass over
// entries that have no `finding_id` yet (untrusted — suggestions only, never
// auto-merged). See `planning/ticket-carryover-dedup-clusters/tasks.md` for the full
// governing design decisions.
//
// No regex crate here by convention — this module is hand-scanned, matching the style
// of `extract_block_id_tokens` above.

/// English function words plus corpus-noise terms that carry no identity signal for
/// the dedup token-overlap pass. Removed from both `slug` and `text` tokens before
/// similarity is computed.
pub const DEDUP_STOPWORDS: &[&str] = &[
    "a", "an", "and", "the", "is", "are", "was", "were", "be", "been", "it", "its", "this", "that",
    "to", "of", "in", "on", "for", "from", "with", "without", "but", "not", "no", "as", "at", "by",
    "or", "so", "than", "then", "when", "which", "what", "all", "any", "only", "own", "has",
    "have", "had", "does", "do", "did", "can", "must", "should", "will", "would", "there", "their",
    "them", "they", "we", "you", "i",
];

/// Tokenizes a carryover entry's `slug` and `text` into one deduplicated,
/// deterministically-ordered set for similarity scoring.
///
/// Lowercases both inputs, splits on any non-ASCII-alphanumeric character (so
/// `finding_id`-style hyphens, slashes, dots, and backticks all act as separators),
/// drops tokens shorter than 3 characters, and drops [`DEDUP_STOPWORDS`] members.
///
/// CRITICAL: both `slug` and `text` are tokenized into the SAME set. The proof case
/// (`bastion:grep-inventory-is-a-hypothesis` = `mev:sdlc-spec-acceptance-vs-purpose-gap`,
/// overlap ~0.38) shares ZERO slug vocabulary and is recoverable only from `text` — a
/// slug-only tokenizer would silently fail that case, and the block's whole point with
/// it. A [`BTreeSet`] is used (not a hash set) so iteration order — and therefore any
/// test or report that walks the tokens — is deterministic across runs and does not
/// depend on hash-seed randomization.
pub fn dedup_tokens(slug: &str, text: &str) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    for source in [slug, text] {
        let mut current = String::new();
        for ch in source.chars() {
            if ch.is_ascii_alphanumeric() {
                current.push(ch.to_ascii_lowercase());
            } else if !current.is_empty() {
                push_dedup_token(&mut tokens, std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            push_dedup_token(&mut tokens, std::mem::take(&mut current));
        }
    }
    tokens
}

fn push_dedup_token(tokens: &mut BTreeSet<String>, token: String) {
    if token.len() < 3 {
        return;
    }
    if DEDUP_STOPWORDS.contains(&token.as_str()) {
        return;
    }
    tokens.insert(token);
}

/// Jaccard similarity: `|a ∩ b| / |a ∪ b|`. Returns `0.0` (never `NaN`) when both sets
/// are empty.
pub fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let union_len = a.union(b).count();
    if union_len == 0 {
        return 0.0;
    }
    let intersection_len = a.intersection(b).count();
    intersection_len as f64 / union_len as f64
}

/// Overlap coefficient: `|a ∩ b| / min(|a|, |b|)`. Returns `0.0` (never divides by
/// zero) when either set is empty.
pub fn overlap_coefficient(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let min_len = a.len().min(b.len());
    if min_len == 0 {
        return 0.0;
    }
    let intersection_len = a.intersection(b).count();
    intersection_len as f64 / min_len as f64
}

/// Acceptance threshold on Jaccard similarity for the dedup suggestion pass.
///
/// OPERATOR-MEASURED against the live 142-entry `carryover[]` corpus (see
/// `planning/ticket-carryover-dedup-clusters/tasks.md`). Lowering this to catch the
/// documented miss (`mev:okf-related-must-be-a-real-doc-id` vs
/// `okf-core:okf-core-doc-ids-are-inconsistent-with-filenames`, which scores ~0.29)
/// trades a known, documented miss for an unknown number of false positives across
/// the fleet. Do not lower it.
pub const DEDUP_JACCARD_MIN: f64 = 0.18;

/// Acceptance threshold on overlap coefficient for the dedup suggestion pass.
///
/// OPERATOR-MEASURED against the live 142-entry `carryover[]` corpus (see
/// `planning/ticket-carryover-dedup-clusters/tasks.md`). Lowering this to catch the
/// documented miss (`mev:okf-related-must-be-a-real-doc-id` vs
/// `okf-core:okf-core-doc-ids-are-inconsistent-with-filenames`, which scores ~0.29)
/// trades a known, documented miss for an unknown number of false positives across
/// the fleet. Do not lower it.
pub const DEDUP_OVERLAP_MIN: f64 = 0.34;

// --- Dedup: authored clustering by finding_id ---
//
// `MV.ticket.carryover-dedup-clusters` task 2. This is the TRUSTED half of dedup:
// `finding_id` is hand-written by a human onto a `carryover[]` entry, so grouping on
// it is exact — no normalization, no fuzzy join. Contrast with `suggest_duplicates`
// (task 3), which is the untrusted heuristic half over entries that carry no
// `finding_id` at all.

/// One entry inside a [`FindingCluster`] — a flattened, report-friendly view of the
/// source [`CarryoverVerdict`] fields a cluster reader needs.
#[derive(Debug, Clone, Serialize)]
pub struct ClusterMember {
    pub repo: String,
    pub slug: String,
    pub priority: Option<u8>,
    pub kind: String,
    pub text: String,
}

/// Every `carryover[]` entry sharing one authored `finding_id`.
///
/// # Per-repo priority divergence is correct, not a conflict
///
/// Each [`ClusterMember`] keeps its own `priority` verbatim. There is
/// deliberately no reconciled/effective/max/min priority field on this type and
/// no diagnostic emitted when members disagree — the measured case is the
/// `nextest` claim, which is genuinely P0 in `okf-core` (the hook does not fire
/// there — a real bail) and genuinely P2 in `mev` (it works exactly as
/// documented). Dedup merges the *claim*; it never merges the *priority*. See
/// `planning/ticket-carryover-dedup-clusters/tasks.md`, governing decision 1.
#[derive(Debug, Clone, Serialize)]
pub struct FindingCluster {
    pub finding_id: String,
    pub members: Vec<ClusterMember>,
    /// Sorted, deduplicated set of repos represented among `members`.
    pub repos: Vec<String>,
    /// `true` iff every member shares one repo. A cluster with `single_repo:
    /// true` is the typo-guard signal upstream (`MV.ticket.carryover-dedup-
    /// clusters` task 4): a `finding_id` used in only one repo did not link
    /// anything across repos, which is usually a mistyped id silently failing
    /// to group rather than a genuinely solitary finding.
    pub single_repo: bool,
}

/// Group every entry carrying a non-empty, authored `finding_id` into one
/// [`FindingCluster`] per distinct id.
///
/// Grouping is on the **exact** string — no case-folding, no trimming-based
/// joining, no fuzzy matching. An authored id is authored; the human who wrote
/// it is the identity authority, not this function.
///
/// Many-to-one is normal and expected: two or more entries in the *same* repo
/// may legitimately share one `finding_id` (e.g. a lesson filed once per
/// affected module within a repo). Such entries are never collapsed to one
/// member — both/all appear as distinct [`ClusterMember`]s.
///
/// Entries with `finding_id: None` or an empty string are excluded from every
/// cluster; they are candidates for [`suggest_duplicates`] instead.
///
/// Ordering is fully deterministic: members sort by `(repo, slug)`, clusters
/// sort by `finding_id`. The report is diffed by humans across runs, so a
/// stable order (not insertion order, not a hash-map iteration order) is load-
/// bearing here, not cosmetic.
pub fn cluster_by_finding_id(entries: &[CarryoverVerdict]) -> Vec<FindingCluster> {
    let mut by_id: std::collections::BTreeMap<String, Vec<ClusterMember>> =
        std::collections::BTreeMap::new();

    for entry in entries {
        let Some(finding_id) = entry.finding_id.as_ref() else {
            continue;
        };
        if finding_id.is_empty() {
            continue;
        }
        by_id
            .entry(finding_id.clone())
            .or_default()
            .push(ClusterMember {
                repo: entry.repo.clone(),
                slug: entry.slug.clone(),
                priority: entry.priority,
                kind: entry.kind.clone(),
                text: entry.text.clone(),
            });
    }

    by_id
        .into_iter()
        .map(|(finding_id, mut members)| {
            members.sort_by(|a, b| a.repo.cmp(&b.repo).then_with(|| a.slug.cmp(&b.slug)));
            let mut repos: Vec<String> = members.iter().map(|m| m.repo.clone()).collect();
            repos.sort();
            repos.dedup();
            let single_repo = repos.len() == 1;
            FindingCluster {
                finding_id,
                members,
                repos,
                single_repo,
            }
        })
        .collect()
}

// --- Dedup: heuristic suggestion pass over ungrouped entries ---
//
// `MV.ticket.carryover-dedup-clusters` task 3. This is the UNTRUSTED half of dedup:
// a crude token-overlap pass over entries that carry no `finding_id` at all, offered
// as candidate duplicates for a human to confirm. Contrast with `cluster_by_finding_id`
// (task 2), which is exact and trusted because a human authored the `finding_id`.

/// One candidate duplicate pair surfaced by [`suggest_duplicates`].
///
/// `a_repo`/`a_slug` and `b_repo`/`b_slug` are ordered by `(repo, slug)` so the same
/// unordered pair always renders identically across runs — there is no "first" or
/// "second" entry with any semantic meaning beyond that canonical ordering.
#[derive(Debug, Clone, Serialize)]
pub struct DedupSuggestion {
    pub a_repo: String,
    pub a_slug: String,
    pub b_repo: String,
    pub b_slug: String,
    pub jaccard: f64,
    pub overlap: f64,
}

/// Heuristic candidate-duplicate pass over every `carryover[]` entry that carries no
/// `finding_id` yet.
///
/// # This is a suggestion, never a merge
///
/// Every pair returned here is **unconfirmed**. This function must never be trusted as
/// ground truth: it does not, and must never, mutate `finding_id` on anything, take
/// `&mut` to any entry, or write to any file — it is a pure read over borrowed
/// [`CarryoverVerdict`] slices. A human confirms a suggestion by hand-authoring a
/// shared `finding_id` into both entries' `state.json`; nothing in this module does
/// that automatically. A false merge destroys durable knowledge the same way a false
/// `cleared` verdict does — this module already biases in exactly that direction
/// elsewhere (the conjunctive reference combination and the [`CLOSURE_VERBS`] gate,
/// both added after a live false-`cleared` incident) and this pass follows the same
/// discipline.
///
/// # Scope and rules
///
/// - Only entries whose `finding_id` is `None` or empty are considered. An authored
///   `finding_id` is the human's answer for that entry; this pass never second-guesses
///   it, so entries that already have one are excluded entirely (they were handled by
///   [`cluster_by_finding_id`] instead).
/// - Every unordered pair is considered at most once (same-repo pairs included — a
///   duplicate filed twice in one repo is still a duplicate).
/// - A pair is accepted when `jaccard >= DEDUP_JACCARD_MIN || overlap >= DEDUP_OVERLAP_MIN`.
///   The `OR` is deliberate: the operator-measured recovery set needs both metrics —
///   some real pairs clear only on Jaccard, others only on overlap coefficient.
/// - Output is sorted by `overlap` descending, then `jaccard` descending, then by the
///   canonical `(a_repo, a_slug, b_repo, b_slug)` tuple, so two runs over the same
///   input always produce the same order. Float comparisons use `partial_cmp(..)
///   .unwrap_or(Ordering::Equal)` — never a bare `unwrap()` — because a comparison
///   would panic on `NaN`, which `jaccard`/`overlap_coefficient` never produce but a
///   defensive comparator should not assume.
pub fn suggest_duplicates(entries: &[CarryoverVerdict]) -> Vec<DedupSuggestion> {
    let candidates: Vec<&CarryoverVerdict> = entries
        .iter()
        .filter(|entry| entry.finding_id.as_deref().unwrap_or("").is_empty())
        .collect();

    let mut suggestions = Vec::new();
    for i in 0..candidates.len() {
        for j in (i + 1)..candidates.len() {
            let left = candidates[i];
            let right = candidates[j];
            let left_tokens = dedup_tokens(&left.slug, &left.text);
            let right_tokens = dedup_tokens(&right.slug, &right.text);
            let score_jaccard = jaccard(&left_tokens, &right_tokens);
            let score_overlap = overlap_coefficient(&left_tokens, &right_tokens);
            if score_jaccard < DEDUP_JACCARD_MIN && score_overlap < DEDUP_OVERLAP_MIN {
                continue;
            }
            let (first, second) = if (left.repo.as_str(), left.slug.as_str())
                <= (right.repo.as_str(), right.slug.as_str())
            {
                (left, right)
            } else {
                (right, left)
            };
            suggestions.push(DedupSuggestion {
                a_repo: first.repo.clone(),
                a_slug: first.slug.clone(),
                b_repo: second.repo.clone(),
                b_slug: second.slug.clone(),
                jaccard: score_jaccard,
                overlap: score_overlap,
            });
        }
    }

    suggestions.sort_by(|a, b| {
        b.overlap
            .partial_cmp(&a.overlap)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.jaccard
                    .partial_cmp(&a.jaccard)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                (
                    a.a_repo.as_str(),
                    a.a_slug.as_str(),
                    a.b_repo.as_str(),
                    a.b_slug.as_str(),
                )
                    .cmp(&(
                        b.a_repo.as_str(),
                        b.a_slug.as_str(),
                        b.b_repo.as_str(),
                        b.b_slug.as_str(),
                    ))
            })
    });

    suggestions
}

// ---------------------------------------------------------------------------
// Triage ranking (MV.ticket.carryover-triage-ranking)
// ---------------------------------------------------------------------------

/// The four-lane re-cut of the carryover board (`MV.ticket.carryover-triage-ranking`).
///
/// Replaces raw-age ordering: every `carryover[]` entry lands in exactly one of
/// these, assigned in this priority order by [`assign_triage_lane`] — BLOCKING
/// first, then HOT, then AGING, then STANDING. See that function's doc comment
/// for the full membership rules, and in particular why board membership must
/// **not** gate on staleness alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriageLane {
    /// At least one unmet `blocks[]` edge — this entry is gating other work.
    /// Ordered by the effective priority of what it blocks (0 hottest first),
    /// then age descending.
    Blocking,
    /// Authored `priority` 0 or 1, and not already in [`Self::Blocking`].
    /// Ordered by priority ascending, then age descending.
    Hot,
    /// Stale (per [`carryover_stale_age`]) with `priority` 2, 3, or absent.
    /// Ordered by age descending.
    Aging,
    /// No authored priority and no `blocks[]` edges — a constraint that is
    /// simply true forever (e.g. "planning/ is a symlink, pass `-L`").
    /// Ordered by age descending.
    ///
    /// This lane exists so permanent rules stop competing with actionable
    /// work: re-affirmed at low frequency rather than shown next to a fresh
    /// P0. It is a re-affirm lane, not a backlog.
    Standing,
}

/// One `carryover[]` entry, ranked and carrying every field a renderer or a
/// consumer (bastion, via the public `rank_carryover` API — `MV.ticket
/// .carryover-triage-ranking`) needs in order to re-rank or explain the
/// ranking, without re-deriving anything from a pre-flattened string.
#[derive(Debug, Clone, Serialize)]
pub struct CarryoverRanking {
    pub repo: String,
    pub slug: String,
    pub kind: String,
    pub lane: TriageLane,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_priority: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_days: Option<i64>,
    pub stale: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unmet_blocks: Vec<String>,
    pub clears_when_satisfied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finding_id: Option<String>,
}

/// Assign the [`TriageLane`] for one carryover entry.
///
/// Membership is evaluated in this exact order, so every entry lands in
/// exactly one lane (the assignment is total):
///
/// 1. **BLOCKING** — `unmet_blocks` is non-empty.
/// 2. **HOT** — authored `priority` is `Some(0)` or `Some(1)`.
/// 3. **AGING** — `verdict.stale` is `true` (i.e. `priority` 2, 3, or absent,
///    and old enough per [`carryover_stale_age`]).
/// 4. **STANDING** — everything else: no priority and no blocks.
///
/// **Board membership must not gate on staleness alone.** `stale` is
/// consulted only for AGING membership — BLOCKING, HOT and STANDING never
/// look at it. Before this ranking existed, membership on the Attention
/// board gated on staleness alone, and only 6 of 142 entries were stale —
/// hiding 136, including every P0 filed the same day (by construction not
/// yet stale). A fresh, non-stale P0 must still land somewhere actionable
/// (HOT), which is exactly what this ordering guarantees.
///
/// Blocking-ness is never authored (there is deliberately no `blocking: bool`
/// field anywhere in this crate) — it is always derived from `unmet_blocks`,
/// which the caller computes from the entry's `blocks[]` edges.
pub fn assign_triage_lane(v: &CarryoverVerdict, unmet_blocks: &[String]) -> TriageLane {
    if !unmet_blocks.is_empty() {
        return TriageLane::Blocking;
    }
    if matches!(v.priority, Some(0) | Some(1)) {
        return TriageLane::Hot;
    }
    if v.stale {
        return TriageLane::Aging;
    }
    TriageLane::Standing
}

// ---------------------------------------------------------------------------
// Effective priority across carryover blocks[] edges (MV.ticket.carryover-
// triage-ranking, task 2) — mirrors state::effective_priorities (MV.7.A)
// ---------------------------------------------------------------------------

/// Compute each carryover entry's **effective priority** by reverse-topological
/// `min`-propagation over its `blocks[]` edges, generalizing the same pass used
/// for blocks ([`crate::brain::state::effective_priorities`], mirrored at
/// [`crate::brain::emit::effective_priority_for`] /
/// [`crate::brain::block_graph`]'s recursion guard) rather than writing a
/// second one.
///
/// `effective(c) = min(own(c), min{ target_effective(t) : t in c.blocks })` —
/// a carryover gating a hotter dependent (block *or* carryover) inherits that
/// hotness. `block_priorities` supplies the already-computed effective
/// priority for every **block** node, keyed `"{repo}:{id}"` (the same map
/// produced by [`crate::brain::state::effective_priorities`]); this pass
/// never recomputes a block's priority and therefore never changes it — a
/// block target is always treated as terminal.
///
/// A `blocks[]` edge is resolved in this order:
/// 1. `BlockedBy::Block(BlockDep { repo, id, .. })` — empty `repo` falls back to the
///    carryover's own `repo` field (mirroring [`block_refs_from_related`]'s
///    fallback). The resolved `"{repo}:{id}"` key is looked up first in
///    `block_priorities` (a block target — terminal); if absent there but
///    present among `entries` (another carryover's `"{repo}:{slug}"` key),
///    it is treated as a carryover target and its effective priority is
///    computed recursively. An unresolvable key in neither map contributes
///    nothing.
/// 2. `BlockedBy::External(_)` — has no node target and contributes no
///    priority. It still counts as an unmet `blocks[]` edge for
///    [`assign_triage_lane`]'s BLOCKING membership, but that is a distinct
///    question this function does not answer.
///
/// **Cycle-safe**: the walk is memoized DFS with an on-stack recursion
/// guard, identical in shape to
/// [`crate::brain::state::effective_priorities`]'s — a key already being
/// computed further up the DFS path short-circuits to its own priority
/// instead of recursing again, so a two-carryover cycle (or a self-edge)
/// terminates deterministically without hanging or panicking.
///
/// Only keys whose effective value lands in the real priority range
/// (`0..=3`) get a map entry; an entry with no own priority and no hotter
/// target, transitively, is **absent** from the result — matching
/// [`crate::brain::state::effective_priorities`]'s absent-not-`u8::MAX`
/// convention, so callers `.get(key).copied()` naturally read it as `None`.
pub fn carryover_effective_priorities(
    entries: &[CarryoverVerdict],
    block_priorities: &HashMap<String, u8>,
) -> HashMap<String, u8> {
    // Own priority per "{repo}:{slug}" key; absent -> u8::MAX (never wins a min).
    let mut own: HashMap<String, u8> = HashMap::new();
    let mut by_key: HashMap<String, &CarryoverVerdict> = HashMap::new();
    for entry in entries {
        let key = format!("{}:{}", entry.repo, entry.slug);
        own.insert(key.clone(), entry.priority.unwrap_or(u8::MAX));
        by_key.insert(key, entry);
    }

    let mut memo: HashMap<String, u8> = HashMap::new();
    let mut on_stack: HashSet<String> = HashSet::new();

    #[allow(clippy::too_many_arguments)]
    fn compute(
        key: &str,
        own: &HashMap<String, u8>,
        by_key: &HashMap<String, &CarryoverVerdict>,
        block_priorities: &HashMap<String, u8>,
        memo: &mut HashMap<String, u8>,
        on_stack: &mut HashSet<String>,
    ) -> u8 {
        if let Some(&v) = memo.get(key) {
            return v;
        }
        let own_priority = own.get(key).copied().unwrap_or(u8::MAX);
        // Cycle guard: `key` is already being computed further up this DFS
        // path (a two-carryover cycle, or a self-edge) — short-circuit to
        // its own priority instead of recursing again.
        if on_stack.contains(key) {
            return own_priority;
        }
        on_stack.insert(key.to_string());

        let mut best = own_priority;
        if let Some(entry) = by_key.get(key) {
            for edge in &entry.blocks {
                match edge {
                    BlockedBy::Block(BlockDep { repo, id, .. }) => {
                        let target_repo = if repo.is_empty() {
                            entry.repo.as_str()
                        } else {
                            repo.as_str()
                        };
                        let target_key = format!("{target_repo}:{id}");
                        if let Some(&bp) = block_priorities.get(&target_key) {
                            // A block target is terminal — never recomputed,
                            // so no block's effective priority can change.
                            if bp < best {
                                best = bp;
                            }
                        } else if by_key.contains_key(&target_key) {
                            let v =
                                compute(&target_key, own, by_key, block_priorities, memo, on_stack);
                            if v < best {
                                best = v;
                            }
                        }
                        // Unresolvable in both maps: contributes nothing.
                    }
                    // No node target, so no priority to propagate — see
                    // this function's doc comment.
                    BlockedBy::External(_) | BlockedBy::Operator(_) | BlockedBy::Approval(_) => {}
                }
            }
        }

        on_stack.remove(key);
        memo.insert(key.to_string(), best);
        best
    }

    let keys: Vec<String> = own.keys().cloned().collect();
    for key in &keys {
        compute(
            key,
            &own,
            &by_key,
            block_priorities,
            &mut memo,
            &mut on_stack,
        );
    }

    memo.into_iter().filter(|(_, v)| *v <= 3).collect()
}

// ---------------------------------------------------------------------------
// rank_carryover — the public ordering API (MV.ticket.carryover-triage-
// ranking, task 3) — THE contract surface bastion calls
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// classify_blocked_by_edge — the shared per-edge resolution core
// (MV.16.A, task 1). THE predicate `unmet_carryover_block_keys` (ranking)
// and `--would-block`'s report (MV.16.A, tasks 2-4) both build on — they
// must never re-derive resolution rules independently, because MV.16.C's
// enforcement is built on this exact predicate and a dry-run that disagrees
// with the gate it previews is worse than no dry-run.
// ---------------------------------------------------------------------------

/// Which kind of `BlockedBy` edge a row reports — independent of the
/// payload, for display and grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockedByEdgeType {
    Block,
    External,
    Operator,
    Approval,
}

/// The verdict for one `BlockedBy` edge, resolved against a live authored
/// block-status map.
///
/// Deliberately NOT collapsed into a `!= "closed"` boolean: `Closed` and
/// `Wontfix` are both terminal "gates nothing" outcomes but are distinct
/// statuses live in the corpus today (`wontfix` on `JF.2.A`, measured
/// 2026-08-22 — not anticipated by `sequence.md` SQ-A34), and
/// `Unresolvable` is a data defect (a typo) that must never inflate a
/// blocking count the way a false `Blocking` would.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeBlockVerdict {
    /// A `block` edge whose target resolved with a status that is neither
    /// `"closed"` nor `"wontfix"` — this edge would actually stop work.
    Blocking,
    /// A `block` edge whose target resolved with status `"closed"` — gates
    /// nothing.
    Closed,
    /// A `block` edge whose target resolved with status `"wontfix"` — gates
    /// nothing.
    Wontfix,
    /// A `block` edge whose target is absent from the status map entirely —
    /// a data defect, not counted as blocking.
    Unresolvable,
    /// An `external` / `operator` / `approval` edge — there is no node
    /// target to resolve, so no blocking verdict applies.
    NoNodeTarget,
}

/// The result of classifying one `BlockedBy` edge against a block-status
/// map — [`classify_blocked_by_edge`]'s return type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockedByEdgeClassification {
    pub edge_type: BlockedByEdgeType,
    /// The resolved target key (`"{repo}:{id}"`), or `None` for
    /// `External`/`Operator`/`Approval` edges, which have no node target.
    pub target_key: Option<String>,
    /// The target's live authored status, if the edge resolved to a node
    /// present in `block_status`. `None` for a non-`Block` edge, and also
    /// `None` when the target resolved but carries no status value.
    pub target_status: Option<String>,
    pub verdict: EdgeBlockVerdict,
}

impl BlockedByEdgeClassification {
    /// `true` only for [`EdgeBlockVerdict::Blocking`] — the one verdict
    /// that would actually stop work.
    pub fn is_blocking(&self) -> bool {
        matches!(self.verdict, EdgeBlockVerdict::Blocking)
    }
}

/// Classifies ONE `BlockedBy` edge against a live authored-status map.
///
/// Resolution rules, all explicit:
/// - [`BlockedBy::Block`]: the target key is `"{repo}:{id}"`, where an empty
///   `repo` on the edge falls back to `entry_repo` (the owning entry's own
///   repo), mirroring [`block_refs_from_related`]'s fallback. The verdict is
///   [`EdgeBlockVerdict::Blocking`] when the target resolves in
///   `block_status` with a status that is neither `"closed"` nor
///   `"wontfix"`; [`EdgeBlockVerdict::Closed`] / [`EdgeBlockVerdict::Wontfix`]
///   when it resolves to exactly that status; and
///   [`EdgeBlockVerdict::Unresolvable`] when the target key is absent from
///   `block_status` entirely.
/// - [`BlockedBy::External`] / [`BlockedBy::Operator`] / [`BlockedBy::Approval`]:
///   no node target exists to resolve, so `target_key` and `target_status`
///   are both `None` and the verdict is [`EdgeBlockVerdict::NoNodeTarget`].
pub fn classify_blocked_by_edge(
    entry_repo: &str,
    edge: &BlockedBy,
    block_status: &HashMap<String, Option<String>>,
) -> BlockedByEdgeClassification {
    match edge {
        BlockedBy::External(_) => BlockedByEdgeClassification {
            edge_type: BlockedByEdgeType::External,
            target_key: None,
            target_status: None,
            verdict: EdgeBlockVerdict::NoNodeTarget,
        },
        BlockedBy::Operator(_) => BlockedByEdgeClassification {
            edge_type: BlockedByEdgeType::Operator,
            target_key: None,
            target_status: None,
            verdict: EdgeBlockVerdict::NoNodeTarget,
        },
        BlockedBy::Approval(_) => BlockedByEdgeClassification {
            edge_type: BlockedByEdgeType::Approval,
            target_key: None,
            target_status: None,
            verdict: EdgeBlockVerdict::NoNodeTarget,
        },
        BlockedBy::Block(BlockDep { repo, id, .. }) => {
            let target_repo = if repo.is_empty() {
                entry_repo
            } else {
                repo.as_str()
            };
            let key = format!("{target_repo}:{id}");
            match block_status.get(&key) {
                None => BlockedByEdgeClassification {
                    edge_type: BlockedByEdgeType::Block,
                    target_key: Some(key),
                    target_status: None,
                    verdict: EdgeBlockVerdict::Unresolvable,
                },
                Some(status_opt) => {
                    let status = status_opt.clone();
                    let verdict = match status.as_deref() {
                        Some("closed") => EdgeBlockVerdict::Closed,
                        Some("wontfix") => EdgeBlockVerdict::Wontfix,
                        _ => EdgeBlockVerdict::Blocking,
                    };
                    BlockedByEdgeClassification {
                        edge_type: BlockedByEdgeType::Block,
                        target_key: Some(key),
                        target_status: status,
                        verdict,
                    }
                }
            }
        }
    }
}

/// Unmet `blocks[]` keys for one carryover entry.
///
/// Mirrors `has_unmet_dep`'s predicate shape verbatim
/// ([`crate::brain::emit`], private, near line 603) so ranking and the
/// unified board's `depends_on` predicate can never drift apart: a
/// [`BlockedBy::External`] edge is always unmet (there is no node target to
/// resolve), and a [`BlockedBy::Block`] edge is unmet unless its target's
/// authored status in `block_status` is exactly `"closed"` — an
/// unresolvable target (absent from `block_status` entirely) counts as
/// unmet too, and so (unchanged from before `--would-block`'s wontfix
/// carve-out existed) does a `"wontfix"` target: this function's contract
/// predates that distinction and must keep treating both as unmet for its
/// existing callers (`rank_carryover`, the triage lanes).
///
/// `External` edges are keyed `"external:{what}"` (matching the display
/// convention already used for `depends_on` at
/// `crate::brain::emit::render_wave_table`) so every returned string is a
/// stable, human-readable identifier — never an empty string.
///
/// The `Block`-edge resolution itself (empty-`repo` fallback, target-status
/// lookup) is delegated to [`classify_blocked_by_edge`] so this function and
/// `--would-block`'s report can never resolve the same edge differently;
/// only the closed-vs-not-closed collapse into "unmet" below is specific to
/// this legacy, narrower contract.
fn unmet_carryover_block_keys(
    entry: &CarryoverVerdict,
    block_status: &HashMap<String, Option<String>>,
) -> Vec<String> {
    entry
        .blocks
        .iter()
        .filter_map(|edge| match edge {
            BlockedBy::External(ExternalDep { what }) => Some(format!("external:{what}")),
            BlockedBy::Operator(OperatorDep { slug, .. }) => Some(okf_core::op_id(slug)),
            BlockedBy::Approval(ApprovalDep { slug, .. }) => Some(okf_core::op_id(slug)),
            BlockedBy::Block(_) => {
                let classification = classify_blocked_by_edge(&entry.repo, edge, block_status);
                let key = classification
                    .target_key
                    .expect("a Block edge always resolves a target key");
                match classification.verdict {
                    EdgeBlockVerdict::Closed => None,
                    _ => Some(key),
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Lane-residency lookup over `discover_lane_files` (MV.16.A, task 2) — a
// second axis, independent of `classify_blocked_by_edge`'s status verdict.
// An `open` target in no lane and an `open` target someone is actively
// driving both classify as `Blocking`; only this index tells them apart.
//
// `lane_segments::discover_lane_files`'s existing public surface
// (`Vec<LaneFile>` with each `LaneBlockRef` already carrying its own `repo`)
// is already sufficient to build this index directly from here, so
// `lane_segments.rs` itself is left untouched — no new surface was needed
// there.
// ---------------------------------------------------------------------------

/// A `{repo}:{id}` target key's lane residency: which lane(s), if any, a
/// discovered `lane-<name>.json` record lists that exact block under.
///
/// Built once per run from every [`LaneFile`] `discover_lane_files` finds,
/// keyed by `"{repo}:{id}"` — matching on the lane entry's own authored
/// `repo` (never the owning entry's `repo`, and never assumed single-repo:
/// "a lane is not single-repo in this corpus").
#[derive(Debug, Default, Clone)]
pub struct LaneResidencyIndex {
    /// target key -> the lane identifiers (`"{roadmap}/lane-{lane}.json"`)
    /// that list it, in first-discovered order. Absence from this map means
    /// "resident in no lane", not "unknown" — every discovered lane record
    /// (parse failures aside; see [`build_lane_residency_index`]'s returned
    /// diagnostics) has already been folded in.
    by_target: HashMap<String, Vec<String>>,
}

impl LaneResidencyIndex {
    /// The lane identifiers (`"{roadmap}/lane-{lane}.json"`) that list
    /// `target_key` (`"{repo}:{id}"`) among their `blocks[]`. Empty when the
    /// target is resident in no discovered lane.
    pub fn lanes_for(&self, target_key: &str) -> &[String] {
        self.by_target
            .get(target_key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// `true` iff `target_key` appears in at least one discovered lane.
    pub fn is_resident(&self, target_key: &str) -> bool {
        !self.lanes_for(target_key).is_empty()
    }
}

/// Builds a [`LaneResidencyIndex`] once per run by walking every lane record
/// `discover_lane_files` finds under `root`. Callers must build this ONCE
/// and reuse it across every edge in a report — `discover_lane_files` walks
/// the whole roadmaps tree and must not be called per edge.
///
/// Returns the index plus every diagnostic `discover_lane_files` produced
/// (e.g. a malformed `lane-<name>.json` record, or a roadmap slug claimed by
/// both the current and legacy layout). Diagnostics are returned, never
/// swallowed here — a record that fails to parse must not silently make its
/// blocks look non-resident; the caller is responsible for surfacing these
/// alongside the report rather than dropping them.
pub fn build_lane_residency_index(
    root: &std::path::Path,
) -> (LaneResidencyIndex, Vec<crate::Diagnostic>) {
    let (lane_files, diags) = crate::brain::lane_segments::discover_lane_files(root);
    let mut by_target: HashMap<String, Vec<String>> = HashMap::new();
    for lane_file in &lane_files {
        let lane_id = format!("{}/lane-{}.json", lane_file.roadmap, lane_file.lane);
        for block in &lane_file.blocks {
            let key = format!("{}:{}", block.repo, block.id);
            by_target.entry(key).or_default().push(lane_id.clone());
        }
    }
    (LaneResidencyIndex { by_target }, diags)
}

// ---------------------------------------------------------------------------
// `--would-block` report (MV.16.A, task 3) — the honest blast radius, with
// enforcement off. One row per `carryover[].blocks[]` edge in the swept
// corpus, built purely from [`classify_blocked_by_edge`] (task 1) and
// [`LaneResidencyIndex`] (task 2) — no resolution logic is re-derived here.
// This report writes nothing; it only reads already-evaluated state.
// ---------------------------------------------------------------------------

/// One row of the `--would-block` report: one `carryover[].blocks[]` edge,
/// fully classified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WouldBlockRow {
    /// The owning entry, `"{repo}:{slug}"`.
    pub owner: String,
    pub edge_type: BlockedByEdgeType,
    /// The resolved target key (`"{repo}:{id}"`), or `None` for a
    /// non-`Block` edge, which has no node target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_key: Option<String>,
    /// The target's live authored status, or `None` when the edge has no
    /// node target, or the target resolved but carries no status value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_status: Option<String>,
    /// `true` iff `target_key` appears in at least one lane discovered by
    /// [`build_lane_residency_index`]. Always `false` for a non-`Block`
    /// edge (no target to look up). Independent of `verdict` — an `open`
    /// target in no lane and an `open` target someone is actively driving
    /// are both `Blocking`; this field is what tells them apart.
    pub lane_resident: bool,
    /// The lane identifiers (`"{roadmap}/lane-{lane}.json"`) that list the
    /// target, if any. Empty when `lane_resident` is `false`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub lanes: Vec<String>,
    pub verdict: EdgeBlockVerdict,
}

/// Summary counts over a [`WouldBlockReport`]'s rows: the headline blocking
/// count plus a breakdown of every non-blocking reason, so a reader can see
/// at a glance how many edges were excluded and why — never just a bare
/// total that hides the classification.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WouldBlockSummary {
    pub total_edges: usize,
    pub blocking: usize,
    pub closed: usize,
    pub wontfix: usize,
    pub unresolvable: usize,
    pub no_node_target: usize,
}

/// The full `--would-block` report: every row plus its summary. Produced by
/// [`compute_would_block_report`]; rendered by [`render_would_block_table`]
/// (human) and [`render_would_block_json`] (machine) — both pure functions
/// over this type, so the two renderers can never independently derive a
/// different verdict for the same row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WouldBlockReport {
    pub rows: Vec<WouldBlockRow>,
    pub summary: WouldBlockSummary,
}

/// Walks every `carryover[].blocks[]` edge across `entries` and classifies
/// each one via [`classify_blocked_by_edge`] and `lane_index`, producing one
/// [`WouldBlockRow`] per edge plus the [`WouldBlockSummary`] breakdown.
///
/// Pure and read-only: this function opens no file handle and writes
/// nothing — it only reads `entries` (already evaluated by
/// [`evaluate_carryover`]), `block_status`, and `lane_index` (built once by
/// [`build_lane_residency_index`] and reused, per that function's own
/// contract, across the whole run rather than rebuilt per edge).
///
/// Row order matches `entries`' order, and within an entry, `entry.blocks`'
/// order — deterministic given deterministic inputs.
pub fn compute_would_block_report(
    entries: &[CarryoverVerdict],
    block_status: &HashMap<String, Option<String>>,
    lane_index: &LaneResidencyIndex,
) -> WouldBlockReport {
    let mut rows = Vec::new();
    let mut summary = WouldBlockSummary::default();

    for entry in entries {
        let owner = format!("{}:{}", entry.repo, entry.slug);
        for edge in &entry.blocks {
            let classification = classify_blocked_by_edge(&entry.repo, edge, block_status);

            summary.total_edges += 1;
            match classification.verdict {
                EdgeBlockVerdict::Blocking => summary.blocking += 1,
                EdgeBlockVerdict::Closed => summary.closed += 1,
                EdgeBlockVerdict::Wontfix => summary.wontfix += 1,
                EdgeBlockVerdict::Unresolvable => summary.unresolvable += 1,
                EdgeBlockVerdict::NoNodeTarget => summary.no_node_target += 1,
            }

            let lanes = classification
                .target_key
                .as_deref()
                .map(|key| lane_index.lanes_for(key).to_vec())
                .unwrap_or_default();
            let lane_resident = !lanes.is_empty();

            rows.push(WouldBlockRow {
                owner: owner.clone(),
                edge_type: classification.edge_type,
                target_key: classification.target_key,
                target_status: classification.target_status,
                lane_resident,
                lanes,
                verdict: classification.verdict,
            });
        }
    }

    WouldBlockReport { rows, summary }
}

// ---------------------------------------------------------------------------
// Block-level enforcement gating set (`MV.16.C`, task 2) — turns
// `carryover[].blocks[]` edges into real holds, behind `enforce_blocks` and
// `max_gates_per_repo`. Built on `classify_blocked_by_edge` (the same
// predicate `--would-block` uses) so the gate and its own dry-run can never
// classify the same edge differently.
// ---------------------------------------------------------------------------

/// One applied gate: `target_key` (`"{repo}:{id}"`) held by `owner`
/// (`"{repo}:{slug}"` of the carryover entry whose `blocks[]` edge gates
/// it) — the reason the block-level derivation (`MV.16.C` task 3) names on
/// `focus.blocked[]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarryoverGate {
    pub target_key: String,
    pub owner: String,
}

/// The gating verdict for one repo (the repo the *target* block belongs
/// to — `derive_focus`/`ready_order` run per repo, so the cap is scoped to
/// match): the gates actually applied, plus enough of the cap decision for
/// a caller to report `cap exceeded — N of M gates applied` rather than
/// silently truncating.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoGatingReport {
    /// Applied gates, keyed by target block key. Never more than `cap`
    /// entries.
    pub gates: BTreeMap<String, CarryoverGate>,
    /// How many distinct target blocks this repo had `Blocking` edges onto,
    /// before the cap was applied.
    pub candidate_count: usize,
    /// `min(candidate_count, cap)` — how many gates were actually applied.
    pub applied_count: usize,
    /// The configured `max_gates_per_repo` this report was computed under.
    pub cap: usize,
    /// `true` iff `candidate_count > cap` — some candidates were reported
    /// but not applied. Never silently dropped: the caller has
    /// `candidate_count - applied_count` to say so.
    pub cap_exceeded: bool,
}

/// Builds the per-repo gating set from every `entries[]` carryover's
/// `blocks[]` edges, classified through [`classify_blocked_by_edge`] (never
/// a second predicate) and capped per target repo by `max_gates_per_repo`.
///
/// - When `enforce_blocks` is `false`, returns an empty map — one place
///   decides enforcement is off, so no consumer can accidentally apply a
///   gate.
/// - An entry carrying `enforce: Some(false)` (`MV.16.C` task requirement;
///   mirrors okf-core's own `StateGraph` edge suppression at
///   `okf-core/src/state.rs:1226`) contributes no gate from any of its
///   edges, even with `enforce_blocks` on. `None` and `Some(true)` both
///   enforce.
/// - Only edges classified [`EdgeBlockVerdict::Blocking`] contribute a
///   candidate gate — `Closed`, `Wontfix`, `Unresolvable`, and
///   `NoNodeTarget` (external/operator/approval) all contribute none,
///   exactly matching `--would-block`'s verdicts.
/// - A target gated by more than one edge (from the same or different
///   entries) is counted once, attributed to whichever entry's edge is
///   discovered first (`entries` order, then each entry's own `blocks[]`
///   order) — deterministic given deterministic inputs.
/// - Candidates are grouped by the *target* block's repo (parsed from the
///   `"{repo}:{id}"` key), since that is the repo whose
///   `derive_focus`/`ready_order` run the cap must bound. Within a repo,
///   candidates beyond `max_gates_per_repo` are excluded from `gates`
///   entirely — `RepoGatingReport::cap_exceeded` and the candidate/applied
///   counts are how a caller reports the excess; nothing here truncates
///   silently.
pub fn build_carryover_gating_sets(
    entries: &[CarryoverVerdict],
    block_status: &HashMap<String, Option<String>>,
    enforce_blocks: bool,
    max_gates_per_repo: usize,
) -> BTreeMap<String, RepoGatingReport> {
    let mut result: BTreeMap<String, RepoGatingReport> = BTreeMap::new();
    if !enforce_blocks {
        return result;
    }

    // First pass: dedupe candidate gates by target key, in deterministic
    // discovery order, recording which entry's edge first named each one.
    let mut owner_by_target: BTreeMap<String, String> = BTreeMap::new();
    let mut discovery_order: Vec<String> = Vec::new();

    for entry in entries {
        if entry.enforce == Some(false) {
            continue;
        }
        let owner = format!("{}:{}", entry.repo, entry.slug);
        for edge in &entry.blocks {
            let classification = classify_blocked_by_edge(&entry.repo, edge, block_status);
            if classification.verdict != EdgeBlockVerdict::Blocking {
                continue;
            }
            let target_key = classification
                .target_key
                .expect("a Blocking verdict always carries a resolved target key");
            owner_by_target
                .entry(target_key.clone())
                .or_insert_with(|| {
                    discovery_order.push(target_key.clone());
                    owner.clone()
                });
        }
    }

    // Second pass: group discovery-ordered candidates by target repo, then
    // cap per repo.
    let mut by_repo: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for target_key in &discovery_order {
        let repo = target_key
            .split_once(':')
            .map(|(repo, _)| repo)
            .unwrap_or(target_key.as_str())
            .to_string();
        by_repo.entry(repo).or_default().push(target_key.clone());
    }

    for (repo, targets) in by_repo {
        let candidate_count = targets.len();
        let applied_count = candidate_count.min(max_gates_per_repo);
        let cap_exceeded = candidate_count > max_gates_per_repo;
        let mut gates = BTreeMap::new();
        for target_key in targets.into_iter().take(applied_count) {
            let owner = owner_by_target
                .get(&target_key)
                .cloned()
                .unwrap_or_default();
            gates.insert(target_key.clone(), CarryoverGate { target_key, owner });
        }
        result.insert(
            repo,
            RepoGatingReport {
                gates,
                candidate_count,
                applied_count,
                cap: max_gates_per_repo,
                cap_exceeded,
            },
        );
    }

    result
}

/// `EdgeBlockVerdict` as the short label used by both renderers.
fn would_block_verdict_label(verdict: EdgeBlockVerdict) -> &'static str {
    match verdict {
        EdgeBlockVerdict::Blocking => "blocking",
        EdgeBlockVerdict::Closed => "closed",
        EdgeBlockVerdict::Wontfix => "wontfix",
        EdgeBlockVerdict::Unresolvable => "unresolvable",
        EdgeBlockVerdict::NoNodeTarget => "no-node-target",
    }
}

/// `BlockedByEdgeType` as the short label used by both renderers.
fn would_block_edge_type_label(edge_type: BlockedByEdgeType) -> &'static str {
    match edge_type {
        BlockedByEdgeType::Block => "block",
        BlockedByEdgeType::External => "external",
        BlockedByEdgeType::Operator => "operator",
        BlockedByEdgeType::Approval => "approval",
    }
}

/// Renders a [`WouldBlockReport`] as a human-readable table plus a summary
/// footer — one line per row, in the report's row order, followed by the
/// blocking headline and the non-blocking breakdown. Pure: reads only
/// `report`, prints nothing itself.
pub fn render_would_block_table(report: &WouldBlockReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "{:<28} {:<10} {:<28} {:<12} {:<7} {:<14} lanes",
        "owner", "edge-type", "target", "status", "lane?", "verdict"
    ));

    for row in &report.rows {
        lines.push(format!(
            "{:<28} {:<10} {:<28} {:<12} {:<7} {:<14} {}",
            row.owner,
            would_block_edge_type_label(row.edge_type),
            row.target_key.as_deref().unwrap_or("-"),
            row.target_status.as_deref().unwrap_or("-"),
            row.lane_resident,
            would_block_verdict_label(row.verdict),
            if row.lanes.is_empty() {
                "-".to_string()
            } else {
                row.lanes.join(",")
            }
        ));
    }

    lines.push(String::new());
    lines.push(format!(
        "total: {}  blocking: {}  closed: {}  wontfix: {}  unresolvable: {}  no-node-target: {}",
        report.summary.total_edges,
        report.summary.blocking,
        report.summary.closed,
        report.summary.wontfix,
        report.summary.unresolvable,
        report.summary.no_node_target,
    ));

    lines.join("\n")
}

/// Renders a [`WouldBlockReport`] as pretty-printed JSON — the same rows and
/// the same verdicts as [`render_would_block_table`], serialized directly
/// from `report` rather than re-derived, so the two renderers can never
/// disagree.
pub fn render_would_block_json(report: &WouldBlockReport) -> serde_json::Result<String> {
    serde_json::to_string_pretty(report)
}

// ---------------------------------------------------------------------------
// Enforcement-state reporting on `--would-block` (`MV.16.C`, task 5) — makes
// the dry-run honest once enforcement exists: without this, the same
// `--would-block` output means two different things ("this would hold
// nothing" vs. "this actually holds these blocks") depending on
// `[carryover]` config nobody can see from the report itself.
// ---------------------------------------------------------------------------

/// One repo's cap-exceeded line: `applied` of `candidates` gates were kept,
/// the rest reported rather than silently dropped (mirrors
/// [`RepoGatingReport::cap_exceeded`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapExceededRepo {
    pub repo: String,
    pub applied: usize,
    pub candidates: usize,
}

/// Renders the `--would-block` enforcement header: `enforcement: ON (cap
/// N/repo)` when `enforce_blocks` is on, `enforcement: OFF` otherwise —
/// plus one `cap exceeded — {repo}: N of M gates applied` line per repo
/// whose [`RepoGatingReport::cap_exceeded`] is `true`. `gating` is expected
/// to already reflect `enforce_blocks`/`max_gates_per_repo` (i.e. built via
/// [`build_carryover_gating_sets`] with the same config) — when
/// `enforce_blocks` is `false`, `gating` is the empty map that builder
/// itself returns, so no cap lines are ever printed for a disabled flag.
///
/// Pure: takes only its arguments, prints nothing itself. Callers own
/// whether this precedes or follows the row table.
pub fn render_would_block_enforcement_summary(
    enforce_blocks: bool,
    max_gates_per_repo: usize,
    gating: &BTreeMap<String, RepoGatingReport>,
) -> String {
    let mut lines = Vec::new();
    if enforce_blocks {
        lines.push(format!("enforcement: ON (cap {max_gates_per_repo}/repo)"));
    } else {
        lines.push("enforcement: OFF".to_string());
    }
    for (repo, report) in gating {
        if report.cap_exceeded {
            lines.push(format!(
                "cap exceeded — {repo}: {} of {} gates applied",
                report.applied_count, report.candidate_count
            ));
        }
    }
    lines.join("\n")
}

/// The same enforcement state as [`render_would_block_enforcement_summary`],
/// shaped as a `serde_json::Value` for embedding under an `"enforcement"`
/// key alongside the `--would-block --json` report — so the JSON output
/// carries the identical posture as structured fields rather than only the
/// human-readable string.
pub fn would_block_enforcement_json(
    enforce_blocks: bool,
    max_gates_per_repo: usize,
    gating: &BTreeMap<String, RepoGatingReport>,
) -> serde_json::Value {
    let cap_exceeded: Vec<CapExceededRepo> = gating
        .iter()
        .filter(|(_, report)| report.cap_exceeded)
        .map(|(repo, report)| CapExceededRepo {
            repo: repo.clone(),
            applied: report.applied_count,
            candidates: report.candidate_count,
        })
        .collect();
    serde_json::json!({
        "enforce_blocks": enforce_blocks,
        "max_gates_per_repo": max_gates_per_repo,
        "cap_exceeded": cap_exceeded,
    })
}

/// `TriageLane` sort rank: BLOCKING first, then HOT, AGING, STANDING.
fn triage_lane_rank(lane: TriageLane) -> u8 {
    match lane {
        TriageLane::Blocking => 0,
        TriageLane::Hot => 1,
        TriageLane::Aging => 2,
        TriageLane::Standing => 3,
    }
}

/// Sort key for "age descending, absent last" — the opposite convention
/// from [`effective_priority_for`]'s "absent sorts last" for priorities:
/// here a missing age must not out-rank a real (however small) age, so it
/// maps to `i64::MIN`, the smallest possible key under a *descending* sort.
fn age_desc_key(age: Option<i64>) -> i64 {
    age.unwrap_or(i64::MIN)
}

/// Total order for [`rank_carryover`]'s output — lane first (BLOCKING, HOT,
/// AGING, STANDING), then the per-lane key from the spec table, then age
/// descending as the shared secondary key (BLOCKING/HOT's explicit
/// secondary; AGING/STANDING's only key), then `(repo, slug)` as the final
/// deterministic tiebreak so output order never depends on input order or
/// hash-map iteration.
///
/// Absent priority/effective-priority sorts **last** within a lane,
/// matching [`crate::brain::emit::effective_priority_for`]'s
/// absent-sorts-`u8::MAX` convention — this function deliberately does not
/// invent a different absent convention.
fn rank_carryover_cmp(a: &CarryoverRanking, b: &CarryoverRanking) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    triage_lane_rank(a.lane)
        .cmp(&triage_lane_rank(b.lane))
        .then_with(|| match a.lane {
            TriageLane::Blocking => {
                let pa = a.effective_priority.unwrap_or(u8::MAX);
                let pb = b.effective_priority.unwrap_or(u8::MAX);
                pa.cmp(&pb)
            }
            TriageLane::Hot => {
                let pa = a.priority.unwrap_or(u8::MAX);
                let pb = b.priority.unwrap_or(u8::MAX);
                pa.cmp(&pb)
            }
            TriageLane::Aging | TriageLane::Standing => Ordering::Equal,
        })
        .then_with(|| age_desc_key(b.age_days).cmp(&age_desc_key(a.age_days)))
        .then_with(|| a.repo.cmp(&b.repo))
        .then_with(|| a.slug.cmp(&b.slug))
}

/// The public ranking entry point — **the contract surface**
/// (`MV.ticket.carryover-triage-ranking`). bastion calls this and projects
/// the result; it must never re-derive ranking (the same discipline already
/// in place at `core/bastion/src/serve/handlers/attention.rs:140-141` for
/// `carryover_stale_age`).
///
/// Composes [`assign_triage_lane`] (task 1) and
/// [`carryover_effective_priorities`] (task 2) over `entries`:
/// - Computes each entry's unmet `blocks[]` keys via
///   [`unmet_carryover_block_keys`], reusing `has_unmet_dep`'s predicate
///   shape (`external` always unmet; unresolved/non-`closed` target unmet).
/// - Assigns a [`TriageLane`] from those unmet keys.
/// - Looks up each entry's effective priority from
///   `carryover_effective_priorities(entries, block_priorities)`.
/// - Reads `clears_when_satisfied` from the entry's own already-evaluated
///   [`CarryoverLane::Cleared`] verdict — `MV.ticket.clears-when-evaluation`
///   already did that evaluation; this function never re-runs a predicate.
///
/// Returns the entries sorted per [`rank_carryover_cmp`] — see that
/// function's doc comment for the full ordering, including the
/// absent-sorts-last convention and the deterministic `(repo, slug)`
/// tiebreak that makes output order identical across calls regardless of
/// input order.
pub fn rank_carryover(
    entries: &[CarryoverVerdict],
    block_priorities: &HashMap<String, u8>,
    block_status: &HashMap<String, Option<String>>,
) -> Vec<CarryoverRanking> {
    let effective = carryover_effective_priorities(entries, block_priorities);

    let mut ranked: Vec<CarryoverRanking> = entries
        .iter()
        .map(|entry| {
            let unmet_blocks = unmet_carryover_block_keys(entry, block_status);
            let lane = assign_triage_lane(entry, &unmet_blocks);
            let key = format!("{}:{}", entry.repo, entry.slug);

            CarryoverRanking {
                repo: entry.repo.clone(),
                slug: entry.slug.clone(),
                kind: entry.kind.clone(),
                lane,
                priority: entry.priority,
                effective_priority: effective.get(&key).copied(),
                age_days: entry.age_days,
                stale: entry.stale,
                unmet_blocks,
                clears_when_satisfied: entry.lane == CarryoverLane::Cleared,
                finding_id: entry.finding_id.clone(),
            }
        })
        .collect();

    ranked.sort_by(rank_carryover_cmp);
    ranked
}

// ---------------------------------------------------------------------------
// Notification policy filter (MV.ticket.attention-notify-policy, task 2)
// ---------------------------------------------------------------------------

/// Apply the notification policy read from `brain.toml`'s `[attention]`
/// table ([`AttentionThresholds`]'s `notify_*`/`digest_everything_else`
/// fields) as a filter over an already-ranked, already-ordered slice of
/// [`CarryoverRanking`] — the interrupt subset `bastion:BA.21.D`'s digest
/// consumes. Pure: no corpus load, no config discovery, no I/O, and it never
/// re-ranks, re-sorts, or re-derives anything — the caller's ordering
/// (`rank_carryover`'s deterministic sort) survives untouched, since
/// [`Iterator::filter`] preserves relative order.
///
/// Rules, evaluated per entry in this order:
/// 1. [`TriageLane::Blocking`] is included whenever
///    `thresholds.notify_blocking_any_priority` is `true`, **regardless of
///    `priority`** (including `None`) — blocking-ness alone is the signal,
///    and `notify_lanes` is not consulted for this lane (see the doc comment
///    on [`AttentionThresholds::notify_priority_floor`]).
/// 2. [`TriageLane::Hot`] is included only if `"hot"` is present in
///    `thresholds.notify_lanes` **and** the entry's `priority` is
///    `<= thresholds.notify_priority_floor`. `Hot` membership is always
///    authored priority `0` or `1` ([`assign_triage_lane`]), so with the
///    documented default floor of `0` a hot `P1` is excluded even though
///    it's in the `Hot` lane — this is deliberate, not a bug (D43
///    over-assignment of P1).
/// 3. Any other lane ([`TriageLane::Aging`], [`TriageLane::Standing`]) is
///    excluded — it still shows on `/attention` and still warns, it simply
///    waits for the once-daily digest ([`AttentionThresholds::
///    digest_everything_else`]).
///
/// A `blocking` item at P3 is included (rule 1 never looks at `priority`); a
/// `hot` item at P1 is excluded (rule 2's floor check) — the two cases that
/// distinguish this from a naive "priority <= floor" filter applied across
/// every lane.
pub fn notify_subset(
    entries: &[CarryoverRanking],
    thresholds: &AttentionThresholds,
) -> Vec<CarryoverRanking> {
    entries
        .iter()
        .filter(|entry| match entry.lane {
            TriageLane::Blocking => thresholds.notify_blocking_any_priority,
            TriageLane::Hot => {
                thresholds
                    .notify_lanes
                    .iter()
                    .any(|lane| lane.eq_ignore_ascii_case("hot"))
                    && entry
                        .priority
                        .is_some_and(|p| p <= thresholds.notify_priority_floor)
            }
            TriageLane::Aging | TriageLane::Standing => false,
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// `--grep` filter (MV.ticket.carryover-grep, task 1)
// ---------------------------------------------------------------------------

/// Filter already-swept entries down to the subset whose `slug` OR `text`
/// matches `pattern`, case-insensitively.
///
/// Pure: no I/O, no globals, no `std::process`. The pattern is compiled
/// exactly once here and reused across every entry (never recompiled per
/// entry). Matching covers `slug` and `text` only — not `clears_when`,
/// `refs`, or `finding_id` (see the ticket's `out_of_scope`); those are not
/// what a human searches by, and widening the match surface makes a common
/// word like `block` hit nearly everything.
///
/// A malformed `pattern` is returned as `Err` rather than silently
/// downgraded to a substring match or to matching nothing — a silent
/// zero-match result is indistinguishable from "no such entry", which is
/// exactly the failure this ticket exists to remove. The caller (the CLI
/// mode handler, task 2/3) is responsible for reporting the error and
/// exiting non-zero; this function never swallows it.
pub fn filter_carryover_entries_by_grep(
    entries: &[CarryoverVerdict],
    pattern: &str,
) -> Result<Vec<CarryoverVerdict>, regex::Error> {
    let re = regex::RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()?;
    Ok(entries
        .iter()
        .filter(|entry| re.is_match(&entry.slug) || re.is_match(&entry.text))
        .cloned()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::state::carryover_kind_from_str;
    use okf_core::CarryoverScope;

    fn scope(repo: Option<&str>) -> CarryoverScope {
        CarryoverScope {
            repo: repo.map(str::to_string),
            tier: None,
            cross_repo: None,
        }
    }

    fn carryover(related: Vec<BlockedBy>, own_repo: Option<&str>) -> Carryover {
        Carryover {
            slug: "test-slug".to_string(),
            scope: scope(own_repo),
            kind: okf_core::CarryoverKind::Known(okf_core::KnownCarryoverKind::Deferred),
            text: "some text".to_string(),
            related,
            clears_when: None,
            created: "2026-01-01".to_string(),
            reviewed: None,
            snoozed_until: None,
            ..Default::default()
        }
    }

    // -- block_refs_from_related ------------------------------------------

    #[test]
    fn block_refs_from_related_keys_block_edges_repo_and_id() {
        let item = carryover(
            vec![BlockedBy::Block(BlockDep {
                repo: "engine-rs".to_string(),
                id: "EN.5.B1".to_string(),
                what: None,
            })],
            None,
        );
        assert_eq!(block_refs_from_related(&item), vec!["engine-rs:EN.5.B1"]);
    }

    #[test]
    fn block_refs_from_related_skips_external_edges() {
        let item = carryover(
            vec![BlockedBy::External(ExternalDep {
                what: "waiting on vendor API".to_string(),
            })],
            None,
        );
        assert!(block_refs_from_related(&item).is_empty());
    }

    #[test]
    fn block_refs_from_related_falls_back_to_own_scope_repo() {
        let item = carryover(
            vec![BlockedBy::Block(BlockDep {
                repo: String::new(),
                id: "MV.3.A".to_string(),
                what: None,
            })],
            Some("mev"),
        );
        assert_eq!(block_refs_from_related(&item), vec!["mev:MV.3.A"]);
    }

    #[test]
    fn block_refs_from_related_skips_edge_with_no_repo_anywhere() {
        let item = carryover(
            vec![BlockedBy::Block(BlockDep {
                repo: String::new(),
                id: "MV.3.A".to_string(),
                what: None,
            })],
            None,
        );
        assert!(block_refs_from_related(&item).is_empty());
    }

    // -- block_refs_from_prose ----------------------------------------------

    #[test]
    fn block_refs_from_prose_resolves_ticket_style_id() {
        let mut known = HashSet::new();
        known.insert("base-template:BT.ticket.gate-skip-count-regression".to_string());
        let (refs, ambiguous) = block_refs_from_prose(
            "BT.ticket.gate-skip-count-regression ships in base-template",
            None,
            &known,
        );
        assert_eq!(
            refs,
            vec!["base-template:BT.ticket.gate-skip-count-regression"]
        );
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_resolves_multiple_slash_separated_ids() {
        let mut known = HashSet::new();
        known.insert("engine-rs:EN.5.B1".to_string());
        known.insert("engine-rs:EN.5.B2".to_string());
        let (mut refs, ambiguous) = block_refs_from_prose("EN.5.B1/EN.5.B2 land", None, &known);
        refs.sort();
        assert_eq!(refs, vec!["engine-rs:EN.5.B1", "engine-rs:EN.5.B2"]);
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_resolves_single_letter_block_id() {
        let mut known = HashSet::new();
        known.insert("bastion-web:BW.8.N".to_string());
        let (refs, ambiguous) = block_refs_from_prose("BW.8.N lands", None, &known);
        assert_eq!(refs, vec!["bastion-web:BW.8.N"]);
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_drops_unresolvable_token_silently() {
        let known = HashSet::new();
        let (refs, ambiguous) = block_refs_from_prose("EN.5.B1 lands", None, &known);
        assert!(refs.is_empty());
        assert!(!ambiguous, "an unresolvable token is not an ambiguity");
    }

    #[test]
    fn block_refs_from_prose_no_resolvable_ids_for_bare_chore_prefix() {
        // Real example from the ticket: "MV.chore" (no trailing ".xxx") does not
        // match the grammar's chore branch at all, so this yields zero tokens —
        // correctly not-evaluable.
        let mut known = HashSet::new();
        known.insert("mev:MV.chore.unique-temp-dirs-in-tests".to_string());
        known.insert("bastion:BE.chore.unique-temp-dirs-in-tests".to_string());
        let (refs, ambiguous) = block_refs_from_prose(
            "MV.chore and BE.chore unique-temp-dirs-in-tests both land",
            None,
            &known,
        );
        assert!(refs.is_empty(), "expected no resolvable IDs, got {refs:?}");
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_flags_ambiguity_across_repos() {
        let mut known = HashSet::new();
        known.insert("repo-a:MV.3.A".to_string());
        known.insert("repo-b:MV.3.A".to_string());
        let (refs, ambiguous) = block_refs_from_prose("MV.3.A lands", None, &known);
        assert!(refs.is_empty());
        assert!(ambiguous);
    }

    #[test]
    fn block_refs_from_prose_prefers_own_repo_when_ambiguous() {
        let mut known = HashSet::new();
        known.insert("repo-a:MV.3.A".to_string());
        known.insert("repo-b:MV.3.A".to_string());
        let (refs, ambiguous) = block_refs_from_prose("MV.3.A lands", Some("repo-b"), &known);
        assert_eq!(refs, vec!["repo-b:MV.3.A"]);
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_rejects_four_letter_prefix() {
        let mut known = HashSet::new();
        known.insert("x:ABCD.1.A".to_string());
        let (refs, ambiguous) = block_refs_from_prose("ABCD.1.A lands", None, &known);
        assert!(refs.is_empty());
        assert!(!ambiguous);
    }

    // -- path_refs_from_prose -------------------------------------------------

    #[test]
    fn path_refs_from_prose_empty_when_no_exists_word() {
        assert!(path_refs_from_prose("docs/decisions/D58-foo.md is linked").is_empty());
    }

    #[test]
    fn path_refs_from_prose_extracts_paths_when_exists_present() {
        let text = "docs/decisions/D58-us-market-entry-and-two-domain-split.md exists and is \
                     linked from docs/decisions/index.md";
        let refs = path_refs_from_prose(text);
        assert_eq!(
            refs,
            vec![
                (
                    "docs/decisions/D58-us-market-entry-and-two-domain-split.md".to_string(),
                    PathAssertion::Present
                ),
                (
                    "docs/decisions/index.md".to_string(),
                    PathAssertion::Present
                ),
            ]
        );
    }

    #[test]
    fn path_refs_from_prose_ignores_tokens_without_slash_or_known_extension() {
        let text = "exists check: README exists but foo.exe/bar does not count";
        let refs = path_refs_from_prose(text);
        assert!(refs.is_empty(), "expected no matches, got {refs:?}");
    }

    #[test]
    fn path_refs_from_prose_trims_surrounding_punctuation_and_quotes() {
        let text = "exists: \"docs/index.md\", (planning/status.md).";
        let refs = path_refs_from_prose(text);
        assert_eq!(
            refs,
            vec![
                ("docs/index.md".to_string(), PathAssertion::Present),
                ("planning/status.md".to_string(), PathAssertion::Present),
            ]
        );
    }

    // -- Task 3: broadened path assertion vocabulary -------------------------

    /// RED-FIRST GUARD (a): a path named in prose with no assertion verb at
    /// all — presence or absence — must stay unextracted. Before this task's
    /// widening, this test was equivalent to
    /// `path_refs_from_prose_empty_when_no_exists_word` and passed trivially;
    /// it is kept as its own guard because the widening below adds many new
    /// verbs and this is the case a careless widening (e.g. dropping the gate
    /// instead of bounding it) would break first.
    #[test]
    fn path_refs_from_prose_stays_empty_for_a_bare_path_mention_with_no_assertion_verb() {
        let text = "see docs/decisions/D58-foo.md for the rationale";
        assert!(path_refs_from_prose(text).is_empty());
    }

    #[test]
    fn path_refs_from_prose_extracts_via_created_added_written_present_verbs() {
        assert_eq!(
            path_refs_from_prose("docs/new.md is created"),
            vec![("docs/new.md".to_string(), PathAssertion::Present)]
        );
        assert_eq!(
            path_refs_from_prose("docs/new.md is added to the index"),
            vec![("docs/new.md".to_string(), PathAssertion::Present)]
        );
        assert_eq!(
            path_refs_from_prose("docs/new.md is written"),
            vec![("docs/new.md".to_string(), PathAssertion::Present)]
        );
        assert_eq!(
            path_refs_from_prose("docs/new.md is present"),
            vec![("docs/new.md".to_string(), PathAssertion::Present)]
        );
    }

    /// (b) corrected/fixed pair with a named file — extraction goes through
    /// the widened presence vocabulary, not new grammar.
    #[test]
    fn path_refs_from_prose_extracts_via_corrected_and_fixed_verbs() {
        assert_eq!(
            path_refs_from_prose("the count in docs/report.md is corrected"),
            vec![("docs/report.md".to_string(), PathAssertion::Present)]
        );
        assert_eq!(
            path_refs_from_prose("docs/report.md is fixed"),
            vec![("docs/report.md".to_string(), PathAssertion::Present)]
        );
    }

    /// "X is corrected" alone, with nothing checkable named, stays
    /// NotEvaluable — verified at the `evaluate_carryover` level below
    /// (`corrected_predicate_naming_nothing_checkable_stays_not_evaluable`).
    #[test]
    fn path_refs_from_prose_extracts_via_removed_deleted_gone_verbs_as_absent() {
        assert_eq!(
            path_refs_from_prose("docs/old.md is removed"),
            vec![("docs/old.md".to_string(), PathAssertion::Absent)]
        );
        assert_eq!(
            path_refs_from_prose("docs/old.md is deleted"),
            vec![("docs/old.md".to_string(), PathAssertion::Absent)]
        );
        assert_eq!(
            path_refs_from_prose("docs/old.md is gone"),
            vec![("docs/old.md".to_string(), PathAssertion::Absent)]
        );
    }

    // -- mentions_gate ---------------------------------------------------------

    #[test]
    fn mentions_gate_matches_word_bounded_validator_and_gate_vocabulary() {
        assert!(mentions_gate("the validator is green"));
        assert!(mentions_gate("CI passes"));
        assert!(mentions_gate("the harness gate is satisfied"));
        assert!(!mentions_gate("the count is corrected"));
        // Word-bounding: "gated" must not match "gate" as a substring of an
        // unrelated word (mirrors `has_closure_verb_is_word_bounded`).
        assert!(!mentions_gate("the feature is gated behind a flag"));
    }

    // -- evaluate_carryover ----------------------------------------------------

    fn src(repo: &str) -> StateSource {
        StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("/fake/{repo}/planning/state.json")),
            expected_kind: "project",
        }
    }

    fn state_file(repo: &str, blocks: Vec<(&str, &str)>, carryover: Vec<Carryover>) -> StateFile {
        StateFile {
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-08-01".to_string(),
            focus: Default::default(),
            tracks: vec![okf_core::Track {
                title: "wave 1".to_string(),
                blocks: blocks
                    .into_iter()
                    .map(|(id, status)| okf_core::TrackBlock {
                        id: id.to_string(),
                        title: "a block".to_string(),
                        status: Some(status.to_string()),
                        depends_on: Vec::new(),
                        wave: None,
                        origin: None,
                        note: None,
                        description: None,
                        priority: None,
                        due: None,
                        sdlc_workflow: None,
                        model: None,
                        epics: Vec::new(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            repos: Vec::new(),
            cross_repo: Vec::new(),
            tiers: Vec::new(),
            epics: Vec::new(),
            note: None,
            backlog: Vec::new(),
            carryover,
            ..Default::default()
        }
    }

    fn item(
        slug: &str,
        kind: &str,
        clears_when: Option<&str>,
        related: Vec<BlockedBy>,
        created: &str,
        reviewed: Option<&str>,
        snoozed_until: Option<&str>,
    ) -> Carryover {
        Carryover {
            slug: slug.to_string(),
            scope: CarryoverScope {
                repo: None,
                tier: None,
                cross_repo: None,
            },
            kind: carryover_kind_from_str(kind),
            text: "some carryover text".to_string(),
            related,
            clears_when: clears_when.map(|s| ClearsWhen::Prose(s.to_string())),
            created: created.to_string(),
            reviewed: reviewed.map(str::to_string),
            snoozed_until: snoozed_until.map(str::to_string),
            ..Default::default()
        }
    }

    /// Like [`item`], but with an explicit `scope` override — used to build
    /// the cross-file-attribution fixtures for `--repo` filtering (Task 2 of
    /// MV.ticket.carryover-repo-filter-keys-on-file).
    fn item_scoped(slug: &str, scope: CarryoverScope) -> Carryover {
        let mut c = item(slug, "env", None, vec![], "2020-01-01", None, None);
        c.scope = scope;
        c
    }

    fn status_map(files: &[(StateSource, StateFile)]) -> HashMap<String, Option<String>> {
        let mut map = HashMap::new();
        for (s, f) in files {
            for track in &f.tracks {
                for block in &track.blocks {
                    map.insert(
                        format!("{}:{}", s.repo_slug, block.id),
                        block.status.clone(),
                    );
                }
            }
        }
        map
    }

    fn thresholds() -> AttentionThresholds {
        AttentionThresholds::default()
    }

    #[test]
    fn evaluate_satisfied_block_ref_lands_cleared() {
        let files = vec![
            (
                src("engine-rs"),
                state_file("engine-rs", vec![("EN.5.B1", "closed")], vec![]),
            ),
            (
                src("mev"),
                state_file(
                    "mev",
                    vec![],
                    vec![item(
                        "waits-on-en",
                        "deferred",
                        Some("EN.5.B1 lands"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
        ];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.total, 1);
        assert_eq!(report.cleared, 1);
        assert_eq!(report.entries[0].lane, CarryoverLane::Cleared);
    }

    #[test]
    fn evaluate_unsatisfied_block_ref_lands_actionable() {
        let files = vec![
            (
                src("engine-rs"),
                state_file("engine-rs", vec![("EN.5.B1", "open")], vec![]),
            ),
            (
                src("mev"),
                state_file(
                    "mev",
                    vec![],
                    vec![item(
                        "waits-on-en",
                        "deferred",
                        Some("EN.5.B1 lands"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
        ];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.total, 1);
        assert_eq!(report.actionable, 1);
        let entry = &report.entries[0];
        assert_eq!(entry.lane, CarryoverLane::Actionable);
        assert_eq!(
            entry.refs,
            vec![CarryoverRef::Block {
                key: "engine-rs:EN.5.B1".to_string(),
                satisfied: false,
            }]
        );
    }

    #[test]
    fn evaluate_unresolvable_prose_token_lands_not_evaluable_prose() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![item(
                    "prose-only",
                    "known_issue",
                    Some("MV.chore and BE.chore unique-temp-dirs-in-tests both land"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.not_evaluable, 1);
        let entry = &report.entries[0];
        assert_eq!(entry.lane, CarryoverLane::NotEvaluable);
        assert_eq!(entry.reason, Some(NotEvaluableReason::Prose));
    }

    #[test]
    fn evaluate_ambiguous_bare_id_lands_not_evaluable_ambiguous() {
        // A bare "MV.3.A" resolves to nodes in both repo-a and repo-b. The
        // carryover's own scope repo is repo-c (neither of the two matches),
        // so the ambiguity cannot be preferentially resolved and the match is
        // dropped rather than guessed at.
        let mut ambiguous_item = item(
            "ambiguous",
            "deferred",
            Some("MV.3.A lands"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        ambiguous_item.scope.repo = Some("repo-c".to_string());

        let files = vec![
            (
                src("repo-a"),
                state_file("repo-a", vec![("MV.3.A", "closed")], vec![]),
            ),
            (
                src("repo-b"),
                state_file("repo-b", vec![("MV.3.A", "open")], vec![]),
            ),
            (
                src("repo-c"),
                state_file("repo-c", vec![], vec![ambiguous_item]),
            ),
        ];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.not_evaluable, 1);
        let entry = &report.entries[0];
        assert_eq!(entry.lane, CarryoverLane::NotEvaluable);
        assert_eq!(entry.reason, Some(NotEvaluableReason::AmbiguousReference));
    }

    #[test]
    fn evaluate_related_edge_alone_never_clears_an_entry() {
        // `related[]` is a "see also" edge, not a clearing condition. A closed
        // related block must NOT clear the carryover — the predicate here names
        // no block, so the entry is prose and stays not-evaluable.
        let files = vec![
            (
                src("bastion"),
                state_file("bastion", vec![("BE.2.A", "closed")], vec![]),
            ),
            (
                src("mev"),
                state_file(
                    "mev",
                    vec![],
                    vec![item(
                        "structured-only",
                        "deferred",
                        Some("the upstream fix ships"),
                        vec![BlockedBy::Block(BlockDep {
                            repo: "bastion".to_string(),
                            id: "BE.2.A".to_string(),
                            what: None,
                        })],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
        ];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 0, "a related[] edge must never clear");
        assert_eq!(report.not_evaluable, 1);
        assert_eq!(report.entries[0].lane, CarryoverLane::NotEvaluable);
        assert_eq!(report.entries[0].reason, Some(NotEvaluableReason::Prose));
        assert!(
            report.entries[0].refs.is_empty(),
            "related[] must contribute no verdict-bearing refs"
        );
    }

    #[test]
    fn has_closure_verb_is_word_bounded() {
        assert!(has_closure_verb("BT.ticket.foo lands in base-template"));
        assert!(has_closure_verb("EN.5.B1/EN.5.B2 land"));
        assert!(has_closure_verb("BW.8.N SHIPPED"));
        // Substring hits must not count.
        assert!(!has_closure_verb("the overland route is documented"));
        assert!(!has_closure_verb("the relationship is clarified"));
        assert!(!has_closure_verb("one of the two BA.0.A blocks is renamed"));
    }

    #[test]
    fn evaluate_block_id_without_a_closure_verb_is_not_evaluable() {
        // The live false-cleared found 2026-08-03: `core:ba-0-a-id-collision`
        // reads "one of the two BA.0.A blocks is renamed and Phase 0 is
        // backfilled". BA.0.A IS closed, so without the closure-verb gate this
        // reported `cleared` while the collision it documents was still live.
        let files = vec![
            (
                src("bastion"),
                state_file("bastion", vec![("BA.0.A", "closed")], vec![]),
            ),
            (
                src("core"),
                state_file(
                    "core",
                    vec![],
                    vec![item(
                        "ba-0-a-id-collision",
                        "known_issue",
                        Some("one of the two BA.0.A blocks is renamed and Phase 0 is backfilled"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
        ];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(
            report.cleared, 0,
            "a renamed-not-closed predicate must not clear"
        );
        assert_eq!(report.not_evaluable, 1);
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::NoClosureVerb)
        );
    }

    #[test]
    fn evaluate_block_id_with_a_closure_verb_still_clears() {
        // The gate must not break the legitimate case.
        let files = vec![
            (
                src("base-template"),
                state_file(
                    "base-template",
                    vec![("BT.ticket.worktree-env-file-copy", "closed")],
                    vec![],
                ),
            ),
            (
                src("orchestrator"),
                state_file(
                    "orchestrator",
                    vec![],
                    vec![item(
                        "init-worktree-missing-app-env-copy",
                        "known_issue",
                        Some("BT.ticket.worktree-env-file-copy ships in base-template"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
        ];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 1);
        assert_eq!(report.entries[0].lane, CarryoverLane::Cleared);
    }

    #[test]
    fn evaluate_no_clears_when_lands_not_evaluable_no_predicate() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![item(
                    "no-predicate",
                    "env",
                    None,
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.not_evaluable, 1);
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::NoPredicate)
        );
    }

    #[test]
    fn evaluate_exists_path_predicate_satisfied_and_unsatisfied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let present = dir.path().join("docs/present.md");
        std::fs::create_dir_all(present.parent().unwrap()).unwrap();
        std::fs::write(&present, "x").unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![item(
                    "path-check",
                    "known_issue",
                    Some("docs/present.md exists and docs/missing.md exists"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            dir.path(),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.actionable, 1, "one path missing -> actionable");
        let entry = &report.entries[0];
        let mut refs = entry.refs.clone();
        refs.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        assert_eq!(
            refs,
            vec![
                CarryoverRef::Path {
                    path: "docs/missing.md".to_string(),
                    satisfied: false,
                },
                CarryoverRef::Path {
                    path: "docs/present.md".to_string(),
                    satisfied: true,
                },
            ]
        );
    }

    #[test]
    fn evaluate_stale_flag_honours_reviewed_and_snoozed_until() {
        let old_item = item(
            "old-and-fresh-review",
            "known_issue",
            None,
            vec![],
            "2020-01-01",
            Some("2026-08-01"),
            None,
        );
        let snoozed_item = item(
            "old-but-snoozed",
            "known_issue",
            None,
            vec![],
            "2020-01-01",
            None,
            Some("2099-01-01"),
        );
        let files = vec![(
            src("mev"),
            state_file("mev", vec![], vec![old_item, snoozed_item]),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        let by_slug = |slug: &str| report.entries.iter().find(|e| e.slug == slug).unwrap();
        assert!(
            !by_slug("old-and-fresh-review").stale,
            "reviewed 2 days ago must reset the staleness clock"
        );
        assert!(
            !by_slug("old-but-snoozed").stale,
            "snoozed entries must not be stale"
        );
        assert_eq!(by_slug("old-but-snoozed").age_days, None);
    }

    #[test]
    fn evaluate_repo_filter_restricts_to_one_repo() {
        let files = vec![
            (
                src("mev"),
                state_file(
                    "mev",
                    vec![],
                    vec![item(
                        "mev-item",
                        "env",
                        None,
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
            (
                src("bastion"),
                state_file(
                    "bastion",
                    vec![],
                    vec![item(
                        "bastion-item",
                        "env",
                        None,
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
        ];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            Some("mev"),
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.total, 1);
        assert_eq!(report.entries[0].repo, "mev");
    }

    /// Fixture corpus for the `--repo` ownership tests below. Two files:
    /// `base-template` holds only same-file (unscoped) entries — the
    /// positive control. `brain` holds a mix: one entry scoped to
    /// `base-template` (the cross-file-attribution case the ticket exists
    /// for), one unscoped (falls back to `brain`), one scoped to a `tier`,
    /// and one marked `cross_repo` (neither of the last two has a single
    /// owning repo and must match no `--repo` filter).
    fn repo_filter_fixture() -> Vec<(StateSource, StateFile)> {
        vec![
            (
                src("base-template"),
                state_file(
                    "base-template",
                    vec![],
                    vec![
                        item_scoped(
                            "four-repos-still-narrow-clippy",
                            CarryoverScope {
                                repo: None,
                                tier: None,
                                cross_repo: None,
                            },
                        ),
                        item_scoped(
                            "installed-mev-and-bastion-are-stale",
                            CarryoverScope {
                                repo: None,
                                tier: None,
                                cross_repo: None,
                            },
                        ),
                    ],
                ),
            ),
            (
                src("brain"),
                state_file(
                    "brain",
                    vec![],
                    vec![
                        item_scoped(
                            "sdlc-flow-is-structurally-unrunnable-in-hq",
                            CarryoverScope {
                                repo: Some("base-template".to_string()),
                                tier: None,
                                cross_repo: None,
                            },
                        ),
                        item_scoped(
                            "brain-native-entry",
                            CarryoverScope {
                                repo: None,
                                tier: None,
                                cross_repo: None,
                            },
                        ),
                        item_scoped(
                            "tier-scoped-entry",
                            CarryoverScope {
                                repo: None,
                                tier: Some("core".to_string()),
                                cross_repo: None,
                            },
                        ),
                        item_scoped(
                            "cross-repo-scoped-entry",
                            CarryoverScope {
                                repo: None,
                                tier: None,
                                cross_repo: Some(true),
                            },
                        ),
                    ],
                ),
            ),
        ]
    }

    #[test]
    fn evaluate_repo_filter_finds_entry_owned_by_filter_but_filed_in_another_repo_file() {
        // The measured live repro (2026-08-23): an entry physically living in
        // `brain`'s state.json, scoped to `base-template`, must be returned
        // by `--repo base-template`. Pre-fix, the file-level skip discarded
        // the whole `brain` file before this entry's own `scope.repo` was
        // ever read, so this assertion fails against the pre-fix behaviour.
        let files = repo_filter_fixture();
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            Some("base-template"),
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        let slugs: Vec<&str> = report.entries.iter().map(|e| e.slug.as_str()).collect();
        assert!(
            slugs.contains(&"sdlc-flow-is-structurally-unrunnable-in-hq"),
            "entry filed in brain's file but scope.repo=base-template must be visible to \
             --repo base-template; got {slugs:?}"
        );
    }

    #[test]
    fn evaluate_repo_filter_positive_control_same_file_entries_still_listed() {
        // Required so the above is a true positive and not a broken filter
        // that now matches everything: entries living AND owned in
        // base-template's own file must still appear under --repo
        // base-template.
        let files = repo_filter_fixture();
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            Some("base-template"),
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        let slugs: Vec<&str> = report.entries.iter().map(|e| e.slug.as_str()).collect();
        assert!(slugs.contains(&"four-repos-still-narrow-clippy"));
        assert!(slugs.contains(&"installed-mev-and-bastion-are-stale"));
        // Exactly the three base-template-owned entries — no more, no less.
        assert_eq!(report.total, 3, "unexpected entry set: {slugs:?}");
    }

    #[test]
    fn evaluate_repo_filter_entry_with_no_scope_repo_falls_back_to_file_repo() {
        let files = repo_filter_fixture();
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            Some("brain"),
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        let slugs: Vec<&str> = report.entries.iter().map(|e| e.slug.as_str()).collect();
        assert!(
            slugs.contains(&"brain-native-entry"),
            "unscoped entry must fall back to its file's repo; got {slugs:?}"
        );
        assert!(
            !slugs.contains(&"sdlc-flow-is-structurally-unrunnable-in-hq"),
            "an entry scoped away to base-template must NOT also match --repo brain; got {slugs:?}"
        );
    }

    #[test]
    fn evaluate_repo_filter_tier_and_cross_repo_scoped_entries_match_no_repo_filter() {
        let files = repo_filter_fixture();
        let status = status_map(&files);
        for filter in ["brain", "base-template", "core"] {
            let report = evaluate_carryover(
                &files,
                &status,
                Path::new("/fake/brain"),
                &HashMap::new(),
                "2026-08-03",
                &thresholds(),
                Some(filter),
                false,
                COMMAND_EXEC_TIMEOUT,
            );
            let slugs: Vec<&str> = report.entries.iter().map(|e| e.slug.as_str()).collect();
            assert!(
                !slugs.contains(&"tier-scoped-entry"),
                "tier-scoped entry must match no --repo filter (tried {filter}); got {slugs:?}"
            );
            assert!(
                !slugs.contains(&"cross-repo-scoped-entry"),
                "cross_repo-scoped entry must match no --repo filter (tried {filter}); got {slugs:?}"
            );
        }
    }

    #[test]
    fn evaluate_repo_filter_absent_is_unchanged_regression() {
        // The unfiltered path must be byte-identical in behaviour to before
        // this ticket: every entry in the fixture corpus, regardless of
        // scope, appears when no --repo filter is passed.
        let files = repo_filter_fixture();
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.total, 6);
        let slugs: std::collections::HashSet<&str> =
            report.entries.iter().map(|e| e.slug.as_str()).collect();
        for expected in [
            "four-repos-still-narrow-clippy",
            "installed-mev-and-bastion-are-stale",
            "sdlc-flow-is-structurally-unrunnable-in-hq",
            "brain-native-entry",
            "tier-scoped-entry",
            "cross-repo-scoped-entry",
        ] {
            assert!(
                slugs.contains(expected),
                "missing {expected} in unfiltered set"
            );
        }
    }

    #[test]
    fn audit_carryover_repo_filter_agrees_with_evaluate_carryover() {
        // `--audit --repo B` must select the identical entry set as
        // `--repo B` on the same corpus, for every case above: the
        // cross-file-owned entry, the positive control, the fallback, and
        // the no-owner tier/cross_repo entries.
        let files = repo_filter_fixture();
        let status = status_map(&files);

        for filter in [Some("base-template"), Some("brain"), Some("core"), None] {
            let report = evaluate_carryover(
                &files,
                &status,
                Path::new("/fake/brain"),
                &HashMap::new(),
                "2026-08-03",
                &thresholds(),
                filter,
                false,
                COMMAND_EXEC_TIMEOUT,
            );
            let audit = audit_carryover(&files, &report, "2026-08-03", 90, filter);
            assert_eq!(
                audit.carryover_count, report.total,
                "audit_carryover disagreed with evaluate_carryover for --repo {filter:?}"
            );
        }
    }

    #[test]
    fn evaluate_carryover_with_dedup_false_skips_clusters_and_suggestions() {
        // Two entries with identical `text` (from `item()`'s fixed fixture text)
        // and no `finding_id` — enough token overlap for `suggest_duplicates` to
        // flag them when the dedup pass runs at all.
        let files = vec![
            (
                src("mev"),
                state_file(
                    "mev",
                    vec![],
                    vec![item(
                        "similar-entry-a",
                        "env",
                        None,
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
            (
                src("okf-core"),
                state_file(
                    "okf-core",
                    vec![],
                    vec![item(
                        "similar-entry-b",
                        "env",
                        None,
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
        ];
        let status = status_map(&files);

        let with_dedup = evaluate_carryover_with_dedup(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            true,
            COMMAND_EXEC_TIMEOUT,
        );
        assert!(
            !with_dedup.suggestions.is_empty(),
            "sanity check: this fixture pair must actually trigger a suggestion \
             when the dedup pass runs, or the negative case below proves nothing"
        );

        let without_dedup = evaluate_carryover_with_dedup(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert!(
            without_dedup.clusters.is_empty(),
            "include_dedup: false must skip cluster_by_finding_id entirely"
        );
        assert!(
            without_dedup.suggestions.is_empty(),
            "include_dedup: false must skip suggest_duplicates entirely"
        );
        assert!(without_dedup.single_repo_finding_ids.is_empty());
        // Everything else in the report is unaffected by the flag.
        assert_eq!(without_dedup.total, with_dedup.total);
        assert_eq!(
            without_dedup
                .entries
                .iter()
                .map(|e| e.slug.as_str())
                .collect::<Vec<_>>(),
            with_dedup
                .entries
                .iter()
                .map(|e| e.slug.as_str())
                .collect::<Vec<_>>()
        );
    }

    // -- evaluate_carryover_with_grep (MV.ticket.carryover-grep, task 2) ----

    #[test]
    fn grep_filter_counts_equal_filtered_entries_not_full_corpus() {
        // Three entries, only one of which mentions "synapse" — a filter
        // applied after the (unfiltered) counts were computed would still
        // report total=3; this asserts the header always matches the body.
        let mut synapse_entry = item(
            "synapse-rename",
            "env",
            None,
            vec![],
            "2020-01-01",
            None,
            None,
        );
        synapse_entry.text = "the synapse rename is pending".to_string();
        let mut other_a = item("other-a", "env", None, vec![], "2020-01-01", None, None);
        other_a.text = "unrelated finding a".to_string();
        let mut other_b = item("other-b", "env", None, vec![], "2020-01-01", None, None);
        other_b.text = "unrelated finding b".to_string();

        let files = vec![(
            src("mev"),
            state_file("mev", vec![], vec![synapse_entry, other_a, other_b]),
        )];
        let status = status_map(&files);

        let full = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(full.total, 3, "sanity: all three entries load unfiltered");

        let filtered = evaluate_carryover_with_grep(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            false,
            COMMAND_EXEC_TIMEOUT,
            Some("synapse"),
        )
        .expect("valid pattern must compile");

        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.entries.len(), 1);
        assert_eq!(filtered.entries[0].slug, "synapse-rename");
        // The header counts must equal the body: not-evaluable is the only
        // populated lane here (no clears_when), so it alone should equal 1.
        assert_eq!(
            filtered.cleared + filtered.actionable + filtered.not_evaluable,
            1
        );
        assert_eq!(
            filtered.cleared + filtered.actionable + filtered.not_evaluable,
            filtered.entries.len(),
            "header counts must always sum to the number of printed rows"
        );
    }

    #[test]
    fn grep_filter_matching_nothing_reports_empty_report_not_error() {
        let mut entry = item("alpha", "env", None, vec![], "2020-01-01", None, None);
        entry.text = "some text".to_string();
        let files = vec![(src("mev"), state_file("mev", vec![], vec![entry]))];
        let status = status_map(&files);

        let filtered = evaluate_carryover_with_grep(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            false,
            COMMAND_EXEC_TIMEOUT,
            Some("no-such-pattern-anywhere"),
        )
        .expect("a pattern matching nothing is still a valid pattern");

        assert_eq!(filtered.total, 0);
        assert!(filtered.entries.is_empty());
        assert_eq!(filtered.cleared, 0);
        assert_eq!(filtered.actionable, 0);
        assert_eq!(filtered.not_evaluable, 0);
    }

    #[test]
    fn grep_filter_report_level_invalid_regex_returns_err() {
        let mut entry = item("alpha", "env", None, vec![], "2020-01-01", None, None);
        entry.text = "some text".to_string();
        let files = vec![(src("mev"), state_file("mev", vec![], vec![entry]))];
        let status = status_map(&files);

        let err = evaluate_carryover_with_grep(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            false,
            COMMAND_EXEC_TIMEOUT,
            Some("(unclosed["),
        )
        .expect_err("malformed regex must error, never silently match nothing");
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn grep_filter_suppresses_dedup_sections_that_are_populated_without_it() {
        // Reuses the exact fixture `evaluate_carryover_with_dedup_false_skips_clusters_and_suggestions`
        // proves triggers a suggestion when the dedup pass runs unfiltered.
        let files = vec![
            (
                src("mev"),
                state_file(
                    "mev",
                    vec![],
                    vec![item(
                        "similar-entry-a",
                        "env",
                        None,
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
            (
                src("okf-core"),
                state_file(
                    "okf-core",
                    vec![],
                    vec![item(
                        "similar-entry-b",
                        "env",
                        None,
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
        ];
        let status = status_map(&files);

        // Unfiltered: dedup sections are populated (this is the positive
        // control — proves the fixture actually triggers dedup at all).
        let unfiltered = evaluate_carryover_with_grep(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            false,
            COMMAND_EXEC_TIMEOUT,
            None,
        )
        .expect("no pattern given, cannot fail");
        assert!(
            !unfiltered.suggestions.is_empty(),
            "sanity check: unfiltered dedup pass must populate suggestions"
        );

        // Filtered down to just one of the two similar entries: dedup
        // sections must be empty, not recomputed over the filtered slice.
        let filtered = evaluate_carryover_with_grep(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            false,
            COMMAND_EXEC_TIMEOUT,
            Some("similar-entry-a"),
        )
        .expect("valid pattern must compile");
        assert_eq!(filtered.total, 1);
        assert!(filtered.clusters.is_empty());
        assert!(filtered.suggestions.is_empty());
        assert!(filtered.single_repo_finding_ids.is_empty());
    }

    #[test]
    fn grep_filter_composes_with_repo_filter_requiring_both() {
        let mut mev_entry = item("shared-slug", "env", None, vec![], "2020-01-01", None, None);
        mev_entry.text = "mentions synapse rename".to_string();
        let mut okf_entry = item("shared-slug", "env", None, vec![], "2020-01-01", None, None);
        okf_entry.text = "mentions synapse rename".to_string();

        let files = vec![
            (src("mev"), state_file("mev", vec![], vec![mev_entry])),
            (
                src("okf-core"),
                state_file("okf-core", vec![], vec![okf_entry]),
            ),
        ];
        let status = status_map(&files);

        // --grep alone matches both repos' entries.
        let grep_only = evaluate_carryover_with_grep(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            false,
            COMMAND_EXEC_TIMEOUT,
            Some("synapse"),
        )
        .expect("valid pattern must compile");
        assert_eq!(grep_only.total, 2);

        // --grep AND --repo together: only the entry satisfying BOTH survives.
        let both = evaluate_carryover_with_grep(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            Some("mev"),
            false,
            false,
            COMMAND_EXEC_TIMEOUT,
            Some("synapse"),
        )
        .expect("valid pattern must compile");
        assert_eq!(both.total, 1);
        assert_eq!(both.entries[0].repo, "mev");
    }

    #[test]
    fn evaluate_output_ordering_is_deterministic() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![("MV.1.A", "closed"), ("MV.1.B", "open")],
                vec![
                    item(
                        "zz-cleared",
                        "env",
                        Some("MV.1.A lands"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    ),
                    item(
                        "aa-actionable",
                        "known_issue",
                        Some("MV.1.B lands"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    ),
                    item(
                        "mm-not-evaluable",
                        "deferred",
                        Some("prose with no ids"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    ),
                ],
            ),
        )];
        let status = status_map(&files);
        let report1 = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        let report2 = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        let lanes1: Vec<CarryoverLane> = report1.entries.iter().map(|e| e.lane).collect();
        let lanes2: Vec<CarryoverLane> = report2.entries.iter().map(|e| e.lane).collect();
        assert_eq!(lanes1, lanes2);
        assert_eq!(
            lanes1,
            vec![
                CarryoverLane::Cleared,
                CarryoverLane::Actionable,
                CarryoverLane::NotEvaluable,
            ]
        );
    }

    // -- typed predicates: BlockClosed / FileExists -------------------------

    /// Builds a `Carryover` with a typed `clears_when` predicate instead of
    /// prose, sharing every other field with [`item`]'s defaults.
    fn predicate_item(slug: &str, kind: &str, predicate: ClearsWhenPredicate) -> Carryover {
        Carryover {
            clears_when: Some(ClearsWhen::Predicate(predicate)),
            ..item(slug, kind, None, vec![], "2020-01-01", None, None)
        }
    }

    #[test]
    fn block_closed_predicate_naming_a_closed_block_clears() {
        let files = vec![(
            src("engine-rs"),
            state_file(
                "engine-rs",
                vec![("EN.5.B1", "closed")],
                vec![predicate_item(
                    "typed-closed",
                    "deferred",
                    ClearsWhenPredicate::BlockClosed {
                        repo: "engine-rs".to_string(),
                        id: "EN.5.B1".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 1);
        assert_eq!(report.entries[0].lane, CarryoverLane::Cleared);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::Block {
                key: "engine-rs:EN.5.B1".to_string(),
                satisfied: true,
            }]
        );
    }

    #[test]
    fn block_closed_predicate_naming_an_open_block_is_actionable_with_unmet_ref() {
        let files = vec![(
            src("engine-rs"),
            state_file(
                "engine-rs",
                vec![("EN.5.B1", "in-progress")],
                vec![predicate_item(
                    "typed-open",
                    "deferred",
                    ClearsWhenPredicate::BlockClosed {
                        repo: "engine-rs".to_string(),
                        id: "EN.5.B1".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.actionable, 1);
        assert_eq!(report.entries[0].lane, CarryoverLane::Actionable);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::Block {
                key: "engine-rs:EN.5.B1".to_string(),
                satisfied: false,
            }]
        );
    }

    #[test]
    fn block_closed_predicate_whose_target_is_absent_from_status_map_is_never_cleared() {
        // A false `cleared` here would mean a typo'd repo/id silently
        // vanishing the entry instead of flagging the data problem.
        let files = vec![(
            src("engine-rs"),
            state_file(
                "engine-rs",
                vec![],
                vec![predicate_item(
                    "typed-unresolvable",
                    "deferred",
                    ClearsWhenPredicate::BlockClosed {
                        repo: "engine-rs".to_string(),
                        id: "EN.99.Z".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 0);
        assert_eq!(report.actionable, 1);
        assert_eq!(report.entries[0].lane, CarryoverLane::Actionable);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::UnresolvedBlock {
                key: "engine-rs:EN.99.Z".to_string(),
            }],
            "an absent target must be surfaced distinctly from a plain unmet Block ref"
        );
    }

    #[test]
    fn file_exists_predicate_resolves_against_brain_root() {
        let dir = std::env::temp_dir().join(format!(
            "mev-carryover-file-exists-brain-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("marker.md");
        std::fs::write(&target, "present").unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-exists-brain-root",
                    "deferred",
                    ClearsWhenPredicate::FileExists {
                        path: "marker.md".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::Path {
                path: "marker.md".to_string(),
                satisfied: true,
            }]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_exists_predicate_resolves_against_owning_repo_path() {
        let dir = std::env::temp_dir().join(format!(
            "mev-carryover-file-exists-repo-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("marker.md");
        std::fs::write(&target, "present").unwrap();

        let mut repo_paths = HashMap::new();
        repo_paths.insert("mev".to_string(), dir.clone());

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-exists-repo-path",
                    "deferred",
                    ClearsWhenPredicate::FileExists {
                        path: "marker.md".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/definitely/not/the/brain/root"),
            &repo_paths,
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_exists_predicate_naming_a_missing_path_is_actionable() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-missing",
                    "deferred",
                    ClearsWhenPredicate::FileExists {
                        path: "definitely/does/not/exist.md".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.actionable, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::Path {
                path: "definitely/does/not/exist.md".to_string(),
                satisfied: false,
            }]
        );
    }

    #[test]
    fn file_exists_predicate_naming_a_directory_is_never_cleared() {
        let dir = std::env::temp_dir().join(format!(
            "mev-carryover-file-exists-dir-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // The predicate names a DIRECTORY, not a file — `.exists()` would
        // say yes, but `is_file()` (what the evaluator must use) says no.
        let target_dir = dir.join("marker-dir");
        std::fs::create_dir_all(&target_dir).unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-exists-directory",
                    "deferred",
                    ClearsWhenPredicate::FileExists {
                        path: "marker-dir".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(
            report.cleared, 0,
            "a directory must never satisfy file_exists"
        );
        assert_eq!(report.actionable, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::Path {
                path: "marker-dir".to_string(),
                satisfied: false,
            }]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_exists_predicate_ambiguous_across_both_roots_is_not_evaluable() {
        let brain_dir = std::env::temp_dir().join(format!(
            "mev-carryover-file-exists-ambig-brain-{}",
            std::process::id()
        ));
        let repo_dir = std::env::temp_dir().join(format!(
            "mev-carryover-file-exists-ambig-repo-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&brain_dir).unwrap();
        std::fs::create_dir_all(&repo_dir).unwrap();
        // Two DIFFERENT files, same relative path, one under each root.
        std::fs::write(brain_dir.join("marker.md"), "brain copy").unwrap();
        std::fs::write(repo_dir.join("marker.md"), "repo copy").unwrap();

        let mut repo_paths = HashMap::new();
        repo_paths.insert("mev".to_string(), repo_dir.clone());

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-exists-ambiguous",
                    "deferred",
                    ClearsWhenPredicate::FileExists {
                        path: "marker.md".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &brain_dir,
            &repo_paths,
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(
            report.cleared, 0,
            "an ambiguous two-root resolution must never read as Cleared"
        );
        assert_eq!(report.not_evaluable, 1);
        assert_eq!(report.entries[0].lane, CarryoverLane::NotEvaluable);
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::AmbiguousReference)
        );
        assert!(
            report.entries[0].refs.is_empty(),
            "an ambiguous resolution pushes no ref — dropped rather than guessed at"
        );

        std::fs::remove_dir_all(&brain_dir).ok();
        std::fs::remove_dir_all(&repo_dir).ok();
    }

    #[test]
    fn file_exists_predicate_present_under_only_repo_root_still_clears() {
        // Positive control for the two tests above: proves the new rejection
        // is specific to the directory/ambiguity shapes, not a blanket
        // regression of the existing repo-root resolution path.
        let brain_dir = std::env::temp_dir().join(format!(
            "mev-carryover-file-exists-repo-only-brain-{}",
            std::process::id()
        ));
        let repo_dir = std::env::temp_dir().join(format!(
            "mev-carryover-file-exists-repo-only-repo-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&brain_dir).unwrap();
        std::fs::create_dir_all(&repo_dir).unwrap();
        std::fs::write(repo_dir.join("marker.md"), "present").unwrap();

        let mut repo_paths = HashMap::new();
        repo_paths.insert("mev".to_string(), repo_dir.clone());

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-exists-repo-only",
                    "deferred",
                    ClearsWhenPredicate::FileExists {
                        path: "marker.md".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &brain_dir,
            &repo_paths,
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 1);
        assert_eq!(report.not_evaluable, 0);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::Path {
                path: "marker.md".to_string(),
                satisfied: true,
            }]
        );

        std::fs::remove_dir_all(&brain_dir).ok();
        std::fs::remove_dir_all(&repo_dir).ok();
    }

    #[test]
    fn resolve_existing_path_same_file_under_both_roots_is_unique_not_ambiguous() {
        // A repo directory reachable through the brain root (e.g. brain root
        // == repo root, or a symlink) resolves to the SAME underlying file
        // under both candidates and must not be flagged ambiguous.
        let dir = std::env::temp_dir().join(format!(
            "mev-carryover-resolve-same-file-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("marker.md"), "present").unwrap();

        let mut repo_paths = HashMap::new();
        repo_paths.insert("mev".to_string(), dir.clone());

        let resolution = resolve_existing_path("marker.md", &dir, &repo_paths, "mev");
        assert_eq!(resolution, PathResolution::Unique(dir.join("marker.md")));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Scratch dir helper for `file_contains` fixtures — mirrors the
    /// `file_exists_predicate_*` tests' `std::env::temp_dir()` pattern.
    fn scratch_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mev-carryover-file-contains-{suffix}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn file_contains_predicate_matching_pattern_clears() {
        let dir = scratch_dir("match");
        std::fs::write(dir.join("target.md"), "the quick brown fox").unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-contains-match",
                    "deferred",
                    ClearsWhenPredicate::FileContains {
                        path: "target.md".to_string(),
                        pattern: "brown fox".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::FileContains {
                path: "target.md".to_string(),
                pattern: "brown fox".to_string(),
                satisfied: true,
            }]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_contains_predicate_non_matching_pattern_is_actionable() {
        let dir = scratch_dir("no-match");
        std::fs::write(dir.join("target.md"), "the quick brown fox").unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-contains-no-match",
                    "deferred",
                    ClearsWhenPredicate::FileContains {
                        path: "target.md".to_string(),
                        pattern: "lazy dog".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.actionable, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::FileContains {
                path: "target.md".to_string(),
                pattern: "lazy dog".to_string(),
                satisfied: false,
            }]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_contains_predicate_absent_file_is_not_evaluable_never_panics() {
        // Was `Actionable` with `satisfied: false` before MV.16.G task 3 —
        // updated because a missing file is unreadable, not a genuine
        // negative (a false red, never a false clear).
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-contains-absent",
                    "deferred",
                    ClearsWhenPredicate::FileContains {
                        path: "definitely/does/not/exist.md".to_string(),
                        pattern: "anything".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.not_evaluable, 1);
        assert!(report.entries[0].refs.is_empty());
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::FileUnreadable)
        );
    }

    #[test]
    fn file_contains_predicate_oversized_file_is_not_evaluable_never_panics() {
        // Was `Actionable` with `satisfied: false` before MV.16.G task 3.
        let dir = scratch_dir("oversized");
        // One byte past FILE_CONTAINS_MAX_BYTES — must be rejected, not read.
        let oversized = vec![b'x'; (FILE_CONTAINS_MAX_BYTES + 1) as usize];
        std::fs::write(dir.join("huge.md"), &oversized).unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-contains-oversized",
                    "deferred",
                    ClearsWhenPredicate::FileContains {
                        path: "huge.md".to_string(),
                        pattern: "z".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.not_evaluable, 1);
        assert!(
            report.entries[0].refs.is_empty(),
            "an oversized file must never be read into memory to satisfy the predicate"
        );
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::FileUnreadable)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_contains_predicate_non_utf8_file_is_not_evaluable_never_panics() {
        // Was `Actionable` before MV.16.G task 3.
        let dir = scratch_dir("non-utf8");
        // 0xFF is never valid as a UTF-8 lead byte.
        std::fs::write(dir.join("binary.md"), [0xFFu8, 0xFE, 0x00, 0x01]).unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-contains-non-utf8",
                    "deferred",
                    ClearsWhenPredicate::FileContains {
                        path: "binary.md".to_string(),
                        pattern: "anything".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert!(!matches!(report.entries[0].lane, CarryoverLane::Cleared));
        assert_eq!(report.not_evaluable, 1);
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::FileUnreadable)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_contains_predicate_regex_shaped_pattern_is_not_evaluable() {
        // Live corpus shape: `bastion:session-qa-chat-about-never-tapped-live`
        // authors `"pattern": "ChatAbout .*live"`, which the literal-match
        // evaluator can never satisfy — a permanent false red that must be
        // named, not evaluated as a literal.
        let dir = scratch_dir("regex-shaped");
        std::fs::write(dir.join("target.md"), "ChatAbout something live").unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-contains-regex-shaped",
                    "deferred",
                    ClearsWhenPredicate::FileContains {
                        path: "target.md".to_string(),
                        pattern: "ChatAbout .*live".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.not_evaluable, 1);
        assert!(report.entries[0].refs.is_empty());
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::PatternNotLiteral)
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_contains_predicate_literal_single_dot_still_matches() {
        // A bare `.` is NOT enough to be treated as regex-shaped — refusing
        // a legitimate literal like `docs/cli.md` would turn a working
        // predicate into a permanent not-evaluable. Positive control for
        // `file_contains_predicate_regex_shaped_pattern_is_not_evaluable`.
        let dir = scratch_dir("literal-dot");
        std::fs::write(dir.join("target.md"), "see docs/cli.md for details").unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-file-contains-literal-dot",
                    "deferred",
                    ClearsWhenPredicate::FileContains {
                        path: "target.md".to_string(),
                        pattern: "docs/cli.md".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::FileContains {
                path: "target.md".to_string(),
                pattern: "docs/cli.md".to_string(),
                satisfied: true,
            }]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn command_exits_zero_with_opt_in_and_exit_zero_clears() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-command-exit-zero",
                    "deferred",
                    ClearsWhenPredicate::CommandExitsZero {
                        command: "true".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            std::env::temp_dir().as_path(),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            true, // allow_exec
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::CommandExitsZero {
                command: "true".to_string(),
                satisfied: true,
            }]
        );
    }

    #[test]
    fn command_exits_zero_with_opt_in_and_nonzero_exit_is_actionable() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-command-exit-one",
                    "deferred",
                    ClearsWhenPredicate::CommandExitsZero {
                        command: "false".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            std::env::temp_dir().as_path(),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            true,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.actionable, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::CommandExitsZero {
                command: "false".to_string(),
                satisfied: false,
            }]
        );
    }

    #[test]
    fn command_exits_zero_with_opt_in_and_nonexistent_binary_is_actionable_never_panics() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-command-nonexistent",
                    "deferred",
                    ClearsWhenPredicate::CommandExitsZero {
                        command: "definitely-not-a-real-binary-xyz".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            std::env::temp_dir().as_path(),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            true,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.actionable, 1);
        assert!(!matches!(report.entries[0].lane, CarryoverLane::Cleared));
    }

    #[test]
    fn command_exits_zero_with_opt_in_and_slow_command_times_out_and_is_actionable() {
        // MV.16.G: exceeds COMMAND_EXEC_TIMEOUT; the in-process watchdog must
        // kill it within roughly the bound rather than hanging the sweep.
        // `timeout(1)` is never invoked to enforce this. UPDATED by MV.16.G:
        // a timeout is now UNKNOWN, not a genuine failure — before this
        // block it collapsed into the same `Actionable`/`satisfied: false`
        // shape as a real non-zero exit (asserted below as the pre-fix
        // baseline), which is indistinguishable from C141's exact failure
        // mode. It now lands in `NotEvaluable` with a dedicated reason and
        // produces no ref at all.
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-command-timeout",
                    "deferred",
                    ClearsWhenPredicate::CommandExitsZero {
                        command: "sleep 30".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);

        let start = std::time::Instant::now();
        let report = evaluate_carryover(
            &files,
            &status,
            std::env::temp_dir().as_path(),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            true,
            COMMAND_EXEC_TIMEOUT,
        );
        let elapsed = start.elapsed();

        // Pre-fix baseline (what this test asserted before MV.16.G): `report.actionable == 1`
        // with `refs == [CarryoverRef::CommandExitsZero { satisfied: false, .. }]` — a timeout
        // was indistinguishable from a genuine non-zero exit.
        assert_eq!(
            report.not_evaluable, 1,
            "a timed-out command must be NotEvaluable, not Actionable"
        );
        assert!(
            report.entries[0].refs.is_empty(),
            "a timeout produces no ref"
        );
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::CommandTimedOut)
        );
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "watchdog should kill the child at roughly COMMAND_EXEC_TIMEOUT, took {elapsed:?}"
        );
    }

    #[test]
    fn command_exits_zero_opt_in_off_is_not_evaluable_never_cleared_even_if_would_exit_zero() {
        // The safe-direction bias on a new axis: an unrun command is
        // unknown, and unknown must never read as Cleared — even though
        // `true` would have exited 0 had it run.
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-command-opt-in-off",
                    "deferred",
                    ClearsWhenPredicate::CommandExitsZero {
                        command: "true".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false, // allow_exec off
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 0);
        assert_eq!(report.not_evaluable, 1);
        assert_eq!(report.entries[0].lane, CarryoverLane::NotEvaluable);
        assert!(report.entries[0].refs.is_empty());
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::ExecutionNotAllowed)
        );
    }

    #[test]
    fn command_exits_zero_predicate_raised_exec_timeout_lets_a_slow_command_reach_exit_zero() {
        // Proves `exec_timeout` is the bound the watchdog actually enforces,
        // not a value threaded through and ignored: a command that sleeps
        // past the module's own 2s default (`COMMAND_EXEC_TIMEOUT`) but
        // inside a raised bound must complete and be observed as a clean
        // exit, not killed.
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "typed-command-raised-bound",
                    "deferred",
                    ClearsWhenPredicate::CommandExitsZero {
                        command: "sleep 3 && true".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            std::env::temp_dir().as_path(),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            true,
            std::time::Duration::from_secs(10),
        );
        assert_eq!(
            report.cleared, 1,
            "a raised --exec-timeout must let a >2s command finish and clear"
        );
        assert_eq!(report.entries[0].reason, None);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::CommandExitsZero {
                command: "sleep 3 && true".to_string(),
                satisfied: true,
            }]
        );
    }

    #[test]
    fn c141_clears_when_network_predicates_can_never_clear_retro_fixture() {
        // MV.16.G task 5 retro-fixture for finding C141, slug
        // `clears-when-network-predicates-can-never-clear` (engine-rs). C141's
        // real evidence is `git -C core/engine-rs push --dry-run origin main`
        // exiting 0 in 19.5s against a 2s bound — a network call this suite
        // must never make. This reproduces the SHAPE without the network: a
        // command that reliably outruns the configured bound.
        //
        // Pre-fix baseline (what this exact shape produced before MV.16.G):
        // `command_exit_zero_satisfied` returned a bare `bool`, so a timeout
        // and a genuine non-zero exit both collapsed to `satisfied: false`
        // and both landed in `report.actionable` with an identical
        // `CarryoverRef::CommandExitsZero { satisfied: false, .. }` — exactly
        // C141's complaint: a network-touching predicate that can never
        // clear reads identically to a real, actionable failure.
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "c141-network-predicate-shape",
                    "deferred",
                    ClearsWhenPredicate::CommandExitsZero {
                        command: "sleep 30".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let start = std::time::Instant::now();
        let report = evaluate_carryover(
            &files,
            &status,
            std::env::temp_dir().as_path(),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            true,
            COMMAND_EXEC_TIMEOUT,
        );
        let elapsed = start.elapsed();

        assert_eq!(
            report.not_evaluable, 1,
            "post-fix: a timed-out command must be NotEvaluable, never Actionable"
        );
        assert_eq!(report.actionable, 0);
        assert!(report.entries[0].refs.is_empty());
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::CommandTimedOut),
            "the timeout must carry a dedicated reason, distinct from a genuine non-zero exit"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "watchdog should kill the child at roughly COMMAND_EXEC_TIMEOUT, took {elapsed:?}"
        );
    }

    #[test]
    fn c180_command_exits_zero_predicates_are_unsound_across_a_live_fleet_retro_fixture() {
        // MV.16.G task 5 retro-fixture for finding C180, slug
        // `command-exits-zero-predicates-are-unsound-across-a-live-fleet`
        // (okf-core). C180's real evidence is `--manifest-path core/bastion`,
        // which compiles bastion, mev AND okf-core together — a multi-crate
        // build this suite must never invoke. This reproduces the SHAPE
        // without the build: a command whose non-zero exit is caused by
        // something other than the entry's own subject (an "upstream"
        // failure, standing in for an unrelated crate breaking the shared
        // build).
        //
        // C180's own conclusion, restated here because it is the reason this
        // fixture exists: a false (non-zero) result from a command that
        // exercises more than the entry's subject is evidence that
        // *something upstream* is red, and is never evidence about the entry
        // it is attached to — so it must be reported as Actionable (a human
        // has to look), never silently as Cleared, and its outcome must be
        // distinguishable from a timeout so an operator does not mistake one
        // failure mode for the other.
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![predicate_item(
                    "c180-upstream-failure-shape",
                    "deferred",
                    ClearsWhenPredicate::CommandExitsZero {
                        // Stands in for "cargo build --manifest-path
                        // core/bastion" failing because an unrelated sibling
                        // crate (okf-core) is red, not because of this
                        // entry's own subject.
                        command: "sh -c 'exit 1'".to_string(),
                        note: None,
                    },
                )],
            ),
        )];
        let status = status_map(&files);
        let start = std::time::Instant::now();
        let report = evaluate_carryover(
            &files,
            &status,
            std::env::temp_dir().as_path(),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            true,
            COMMAND_EXEC_TIMEOUT,
        );
        let elapsed = start.elapsed();

        assert_eq!(
            report.actionable, 1,
            "an upstream-caused non-zero exit is Actionable, a human must look"
        );
        assert_eq!(
            report.cleared, 0,
            "never Cleared on an unsound upstream signal"
        );
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::CommandExitsZero {
                command: "sh -c 'exit 1'".to_string(),
                satisfied: false,
            }],
            "an upstream ExitNonZero still carries a ref, distinguishing it from a TimedOut/SpawnFailed which carries none"
        );
        assert_eq!(
            report.entries[0].reason, None,
            "an ExitNonZero is not forced into NotEvaluable the way TimedOut/SpawnFailed are"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "a genuine non-zero exit must resolve fast, unlike a bound-outrunning timeout: took {elapsed:?}"
        );
    }

    #[test]
    fn mixed_typed_satisfied_and_prose_unsatisfied_entries_each_preserve_conjunctive_and() {
        // The schema allows exactly one `clears_when` per carryover entry, so
        // "mixed typed-and-prose reference sets" is exercised across a fleet
        // sweep containing one entry of each kind, each independently
        // proving the same `refs`-vec AND logic governs both source types.
        let files = vec![(
            src("engine-rs"),
            state_file(
                "engine-rs",
                vec![("EN.5.B1", "closed"), ("EN.5.B2", "in-progress")],
                vec![
                    predicate_item(
                        "typed-satisfied",
                        "deferred",
                        ClearsWhenPredicate::BlockClosed {
                            repo: "engine-rs".to_string(),
                            id: "EN.5.B1".to_string(),
                            note: None,
                        },
                    ),
                    item(
                        "prose-unsatisfied",
                        "deferred",
                        Some("EN.5.B2 lands"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    ),
                ],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        let by_slug = |slug: &str| {
            report
                .entries
                .iter()
                .find(|e| e.slug == slug)
                .unwrap_or_else(|| panic!("missing entry {slug}"))
        };
        assert_eq!(by_slug("typed-satisfied").lane, CarryoverLane::Cleared);
        assert_eq!(by_slug("prose-unsatisfied").lane, CarryoverLane::Actionable);
    }

    #[test]
    fn closure_verbs_and_pinning_test_guard_are_untouched_by_typed_predicate_support() {
        // Re-affirms the CLOSURE_VERBS gate still applies on the prose path
        // even though the typed BlockClosed path (correctly) bypasses it —
        // see the module-level doc on `evaluate_carryover`'s Predicate arm.
        assert!(has_closure_verb("BA.0.A closes"));
        assert!(!has_closure_verb("one of the two BA.0.A blocks is renamed"));
    }

    // -- Task 3: prose widening, wired through evaluate_carryover ------------

    /// RED-FIRST GUARD (a): a bare path mention with no assertion verb, run
    /// through the full evaluator (not just the extractor), stays
    /// NotEvaluable — never Actionable, and certainly never Cleared, merely
    /// because a path-shaped token happens to appear in the prose.
    #[test]
    fn bare_path_mention_with_no_assertion_verb_stays_not_evaluable() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![item(
                    "bare-path-mention",
                    "deferred",
                    Some("see docs/decisions/D58-foo.md for the rationale"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.entries[0].lane, CarryoverLane::NotEvaluable);
        assert_eq!(report.entries[0].reason, Some(NotEvaluableReason::Prose));
    }

    /// RED-FIRST GUARD (a continued) — the polarity guard: an
    /// absence-assertion ("X is removed") over a path that in fact still
    /// EXISTS must land Actionable, never Cleared. Before `PathAbsent`
    /// existed, the only representable ref was `Path { satisfied: exists }`,
    /// which would have reported this entry `cleared` purely because the
    /// path is named and resolves — exactly the false-`cleared` shape this
    /// guard exists to catch.
    #[test]
    fn absence_assertion_over_a_still_existing_path_is_actionable_never_cleared() {
        let dir = std::env::temp_dir().join(format!(
            "mev-carryover-path-absent-still-exists-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/stale.md"), "still here").unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![item(
                    "absence-over-existing",
                    "deferred",
                    Some("docs/stale.md is removed"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.entries[0].lane, CarryoverLane::Actionable);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::PathAbsent {
                path: "docs/stale.md".to_string(),
                satisfied: false,
            }]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The positive case, for completeness: an absence-assertion over a path
    /// that has genuinely been removed clears.
    #[test]
    fn absence_assertion_over_a_missing_path_clears() {
        let dir = std::env::temp_dir().join(format!(
            "mev-carryover-path-absent-gone-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![item(
                    "absence-over-missing",
                    "deferred",
                    Some("docs/stale.md is deleted"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            &dir,
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.entries[0].lane, CarryoverLane::Cleared);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// (b) "X is corrected" alone, with nothing checkable named, stays
    /// NotEvaluable — the widening only reaches predicates that resolve to
    /// an already-checkable file/block reference.
    #[test]
    fn corrected_predicate_naming_nothing_checkable_stays_not_evaluable() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![item(
                    "corrected-nothing-checkable",
                    "deferred",
                    Some("the count is corrected"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.entries[0].lane, CarryoverLane::NotEvaluable);
        assert_eq!(report.entries[0].reason, Some(NotEvaluableReason::Prose));
    }

    /// RED-FIRST GUARD (c): a gate/validator mention with nothing checkable
    /// stays NotEvaluable with the dedicated reason, never Actionable or
    /// Cleared — no ref is fabricated from "the validator is green" and no
    /// command is derived from it and run.
    #[test]
    fn gate_mention_with_nothing_checkable_stays_not_evaluable_never_cleared() {
        let files = vec![(
            src("mev"),
            state_file(
                "mev",
                vec![],
                vec![item(
                    "gate-mention-only",
                    "deferred",
                    Some("the validator is green"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.entries[0].lane, CarryoverLane::NotEvaluable);
        assert!(report.entries[0].refs.is_empty());
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::GateMentionNotCheckable)
        );
    }

    /// A gate mention that ALSO names a closing block still evaluates via
    /// the existing block path — `mentions_gate` only supplies a more
    /// specific reason label when nothing else was extracted; it never
    /// suppresses real extraction.
    #[test]
    fn gate_mention_paired_with_a_closing_block_still_evaluates_via_block_ref() {
        let files = vec![(
            src("engine-rs"),
            state_file(
                "engine-rs",
                vec![("EN.5.B1", "closed")],
                vec![item(
                    "gate-mention-with-block",
                    "deferred",
                    Some("the CI gate clears when EN.5.B1 lands"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.entries[0].lane, CarryoverLane::Cleared);
    }

    /// Live-data twin of the CLOSURE_VERBS pinning test, re-run through the
    /// widened Task 3 code path: the 2026-08-03 false-cleared shape must
    /// still stay NotEvaluable even with the broadened path/gate vocabulary
    /// in play.
    #[test]
    fn ba_0_a_id_collision_shape_stays_not_evaluable_after_task3_widening() {
        let files = vec![(
            src("core"),
            state_file(
                "core",
                vec![("BA.0.A", "closed")],
                vec![item(
                    "core:ba-0-a-id-collision",
                    "known_issue",
                    Some("one of the two BA.0.A blocks is renamed and Phase 0 is backfilled"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-09",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_ne!(report.entries[0].lane, CarryoverLane::Cleared);
    }

    // -- dedup_tokens / jaccard / overlap_coefficient -----------------------

    #[test]
    fn dedup_tokens_removes_stopwords() {
        let tokens = dedup_tokens("the-and-of", "this is not a real finding");
        // "real" and "finding" survive; every stopword and short token is dropped.
        assert!(tokens.contains("real"));
        assert!(tokens.contains("finding"));
        assert!(!tokens.contains("the"));
        assert!(!tokens.contains("and"));
        assert!(!tokens.contains("of"));
        assert!(!tokens.contains("is"));
        assert!(!tokens.contains("not"));
        assert!(!tokens.contains("a"));
    }

    #[test]
    fn dedup_tokens_splits_on_hyphens_slashes_dots_and_backticks() {
        let tokens = dedup_tokens(
            "wave0-tickets-ship-without-tasks-json",
            "see `src/brain/carryover.rs` version 1.2 for details",
        );
        assert!(tokens.contains("wave0"));
        assert!(tokens.contains("tickets"));
        assert!(tokens.contains("ship"));
        assert!(tokens.contains("tasks"));
        assert!(tokens.contains("json"));
        assert!(tokens.contains("src"));
        assert!(tokens.contains("brain"));
        assert!(tokens.contains("carryover"));
        // "rs" is 2 chars and is dropped by the short-token rule; splitting is
        // still proven by "src"/"brain"/"carryover" landing as separate tokens.
        assert!(!tokens.contains("rs"));
        assert!(tokens.contains("version"));
        assert!(tokens.contains("details"));
    }

    #[test]
    fn dedup_tokens_drops_short_tokens() {
        let tokens = dedup_tokens("ab-cd-ok", "xy go up");
        // All of ab, cd, ok, xy, go, up are < 3 chars.
        assert!(tokens.is_empty());
    }

    #[test]
    fn dedup_tokens_merges_slug_and_text_into_one_set() {
        // Proof case shape: slug-only tokens are disjoint from text-only tokens, but
        // both must land in the same returned set.
        let tokens = dedup_tokens("grep-inventory-hypothesis", "acceptance purpose gap");
        assert!(tokens.contains("grep"));
        assert!(tokens.contains("inventory"));
        assert!(tokens.contains("hypothesis"));
        assert!(tokens.contains("acceptance"));
        assert!(tokens.contains("purpose"));
        assert!(tokens.contains("gap"));
    }

    #[test]
    fn jaccard_empty_sets_is_zero_not_nan() {
        let empty: BTreeSet<String> = BTreeSet::new();
        let score = jaccard(&empty, &empty);
        assert_eq!(score, 0.0);
        assert!(!score.is_nan());
    }

    #[test]
    fn overlap_coefficient_empty_sets_is_zero_not_nan_or_panic() {
        let empty: BTreeSet<String> = BTreeSet::new();
        let mut one = BTreeSet::new();
        one.insert("token".to_string());

        let score_both_empty = overlap_coefficient(&empty, &empty);
        assert_eq!(score_both_empty, 0.0);
        assert!(!score_both_empty.is_nan());

        let score_one_empty = overlap_coefficient(&empty, &one);
        assert_eq!(score_one_empty, 0.0);
        assert!(!score_one_empty.is_nan());
    }

    #[test]
    fn jaccard_and_overlap_of_identical_sets_is_one() {
        let mut set = BTreeSet::new();
        set.insert("finding".to_string());
        set.insert("token".to_string());
        assert_eq!(jaccard(&set, &set), 1.0);
        assert_eq!(overlap_coefficient(&set, &set), 1.0);
    }

    #[test]
    fn jaccard_and_overlap_of_disjoint_sets_is_zero() {
        let mut a = BTreeSet::new();
        a.insert("alpha".to_string());
        let mut b = BTreeSet::new();
        b.insert("beta".to_string());
        assert_eq!(jaccard(&a, &b), 0.0);
        assert_eq!(overlap_coefficient(&a, &b), 0.0);
    }

    #[test]
    fn overlap_coefficient_exceeds_jaccard_for_asymmetric_pair() {
        // a is a small set fully contained in the much larger set b: overlap
        // coefficient (relative to the smaller set) is 1.0, while jaccard (relative
        // to the union) is much smaller.
        let mut a = BTreeSet::new();
        a.insert("shared".to_string());

        let mut b = BTreeSet::new();
        b.insert("shared".to_string());
        b.insert("other1".to_string());
        b.insert("other2".to_string());
        b.insert("other3".to_string());
        b.insert("other4".to_string());

        let ov = overlap_coefficient(&a, &b);
        let jac = jaccard(&a, &b);
        assert!(ov > jac, "overlap {ov} should exceed jaccard {jac}");
    }

    // -- cluster_by_finding_id -----------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn verdict(
        repo: &str,
        slug: &str,
        priority: Option<u8>,
        finding_id: Option<&str>,
    ) -> CarryoverVerdict {
        CarryoverVerdict {
            repo: repo.to_string(),
            slug: slug.to_string(),
            kind: "known_issue".to_string(),
            text: format!("some text for {slug}"),
            clears_when: None,
            created: "2026-01-01".to_string(),
            age_days: None,
            stale: false,
            lane: CarryoverLane::NotEvaluable,
            refs: Vec::new(),
            reason: None,
            priority,
            finding_id: finding_id.map(str::to_string),
            blocks: Vec::new(),
            enforce: None,
            needs: None,
        }
    }

    #[test]
    fn cluster_by_finding_id_groups_across_repos_into_one_cluster() {
        let entries = vec![
            verdict(
                "okf-core",
                "nextest-bail",
                Some(0),
                Some("nextest-hook-gap"),
            ),
            verdict("mev", "nextest-scoped", Some(2), Some("nextest-hook-gap")),
        ];
        let clusters = cluster_by_finding_id(&entries);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].finding_id, "nextest-hook-gap");
        assert_eq!(clusters[0].members.len(), 2);
        assert!(!clusters[0].single_repo);
        assert_eq!(clusters[0].repos, vec!["mev", "okf-core"]);
    }

    #[test]
    fn cluster_by_finding_id_many_to_one_same_repo_yields_two_distinct_members() {
        let entries = vec![
            verdict("mev", "issue-a", None, Some("shared-lesson")),
            verdict("mev", "issue-b", None, Some("shared-lesson")),
        ];
        let clusters = cluster_by_finding_id(&entries);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].members.len(), 2);
        assert!(clusters[0].single_repo, "both members are in 'mev'");
        assert_eq!(clusters[0].repos, vec!["mev"]);
        let slugs: Vec<&str> = clusters[0]
            .members
            .iter()
            .map(|m| m.slug.as_str())
            .collect();
        assert_eq!(slugs, vec!["issue-a", "issue-b"]);
    }

    #[test]
    fn cluster_by_finding_id_preserves_divergent_priorities_verbatim() {
        let entries = vec![
            verdict(
                "okf-core",
                "nextest-bail",
                Some(0),
                Some("nextest-hook-gap"),
            ),
            verdict("mev", "nextest-scoped", Some(2), Some("nextest-hook-gap")),
        ];
        let clusters = cluster_by_finding_id(&entries);
        let member_priority = |repo: &str| {
            clusters[0]
                .members
                .iter()
                .find(|m| m.repo == repo)
                .and_then(|m| m.priority)
        };
        assert_eq!(
            member_priority("okf-core"),
            Some(0),
            "okf-core keeps its own P0"
        );
        assert_eq!(member_priority("mev"), Some(2), "mev keeps its own P2");
    }

    #[test]
    fn cluster_by_finding_id_excludes_none_and_empty_finding_id() {
        let entries = vec![
            verdict("mev", "no-id", None, None),
            verdict("mev", "empty-id", None, Some("")),
            verdict("mev", "has-id", None, Some("real-id")),
        ];
        let clusters = cluster_by_finding_id(&entries);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].finding_id, "real-id");
        assert_eq!(clusters[0].members.len(), 1);
        assert_eq!(clusters[0].members[0].slug, "has-id");
    }

    #[test]
    fn cluster_by_finding_id_ordering_is_stable_regardless_of_input_order() {
        let forward = vec![
            verdict("bastion", "z-slug", None, Some("beta-id")),
            verdict("mev", "a-slug", None, Some("alpha-id")),
            verdict("mev", "b-slug", None, Some("alpha-id")),
        ];
        let mut shuffled = forward.clone();
        shuffled.reverse();

        let clusters_a = cluster_by_finding_id(&forward);
        let clusters_b = cluster_by_finding_id(&shuffled);

        let ids_a: Vec<&str> = clusters_a.iter().map(|c| c.finding_id.as_str()).collect();
        let ids_b: Vec<&str> = clusters_b.iter().map(|c| c.finding_id.as_str()).collect();
        assert_eq!(ids_a, ids_b);
        assert_eq!(ids_a, vec!["alpha-id", "beta-id"]);

        let alpha_slugs_a: Vec<&str> = clusters_a[0]
            .members
            .iter()
            .map(|m| m.slug.as_str())
            .collect();
        let alpha_slugs_b: Vec<&str> = clusters_b[0]
            .members
            .iter()
            .map(|m| m.slug.as_str())
            .collect();
        assert_eq!(alpha_slugs_a, alpha_slugs_b);
        assert_eq!(alpha_slugs_a, vec!["a-slug", "b-slug"]);
    }

    #[test]
    fn cluster_by_finding_id_no_diagnostic_field_exists_for_divergence() {
        // Compile-time-adjacent guard: FindingCluster carries no reconciled
        // priority and no diagnostic/warning field. This test documents that
        // invariant by construction — if a future edit adds such a field, this
        // struct literal (and every other FindingCluster construction site)
        // will fail to compile unless updated, forcing a conscious decision.
        let cluster = FindingCluster {
            finding_id: "id".to_string(),
            members: vec![],
            repos: vec![],
            single_repo: true,
        };
        assert_eq!(cluster.finding_id, "id");
    }

    #[test]
    fn clusters_exposes_id_to_repos_and_slugs_view_for_state_pass() {
        // MV.16.D task 1: confirms `CarryoverReport::clusters` (built from
        // `cluster_by_finding_id`) already carries, per finding_id, every repo
        // and slug it was used from — the exact "id -> {repos, slugs}" shape
        // the state pass's forthcoming `W_STATE_FINDING_ID_ORPHAN` check
        // (task 2) needs, reachable the same way
        // `check_carryover_broken_predicate` already consumes `entries` off
        // this same report. No new plumbing route, no change to
        // `cluster_by_finding_id`'s own grouping/ordering semantics.
        let entries = vec![
            verdict("bastion", "b-slug", Some(1), Some("shared-typo-guard")),
            verdict("mev", "m-slug", Some(2), Some("shared-typo-guard")),
            verdict("mev", "solo-slug", None, Some("solo-finding")),
        ];
        let clusters = cluster_by_finding_id(&entries);

        let cross_repo = clusters
            .iter()
            .find(|c| c.finding_id == "shared-typo-guard")
            .expect("cross-repo cluster present");
        assert_eq!(cross_repo.repos, vec!["bastion", "mev"]);
        let slugs: Vec<&str> = cross_repo.members.iter().map(|m| m.slug.as_str()).collect();
        assert_eq!(slugs, vec!["b-slug", "m-slug"]);
        assert!(!cross_repo.single_repo);

        let solo = clusters
            .iter()
            .find(|c| c.finding_id == "solo-finding")
            .expect("single-repo cluster present");
        assert_eq!(solo.repos, vec!["mev"]);
        assert_eq!(solo.members[0].slug, "solo-slug");
        assert!(solo.single_repo);
    }

    // -- suggest_duplicates ---------------------------------------------------

    fn verdict_text(
        repo: &str,
        slug: &str,
        text: &str,
        finding_id: Option<&str>,
    ) -> CarryoverVerdict {
        CarryoverVerdict {
            repo: repo.to_string(),
            slug: slug.to_string(),
            kind: "known_issue".to_string(),
            text: text.to_string(),
            clears_when: None,
            created: "2026-01-01".to_string(),
            age_days: None,
            stale: false,
            lane: CarryoverLane::NotEvaluable,
            refs: Vec::new(),
            reason: None,
            priority: None,
            finding_id: finding_id.map(str::to_string),
            blocks: Vec::new(),
            enforce: None,
            needs: None,
        }
    }

    #[test]
    fn suggest_duplicates_skips_entries_that_already_carry_a_finding_id() {
        let entries = vec![
            verdict_text(
                "mev",
                "already-linked-a",
                "identical text tokens here for matching purposes",
                Some("linked"),
            ),
            verdict_text(
                "okf-core",
                "already-linked-b",
                "identical text tokens here for matching purposes",
                Some("linked"),
            ),
        ];
        let suggestions = suggest_duplicates(&entries);
        assert!(
            suggestions.is_empty(),
            "entries with an authored finding_id must never be suggested, even if similar"
        );
    }

    #[test]
    fn suggest_duplicates_accepts_on_jaccard_or_overlap_and_orders_pair_canonically() {
        let entries = vec![
            verdict_text(
                "zeta-repo",
                "zzz-slug",
                "shared overlapping duplicate token corpus words here",
                None,
            ),
            verdict_text(
                "alpha-repo",
                "aaa-slug",
                "shared overlapping duplicate token corpus words here",
                None,
            ),
        ];
        let suggestions = suggest_duplicates(&entries);
        assert_eq!(suggestions.len(), 1);
        let s = &suggestions[0];
        // Canonical ordering by (repo, slug): "alpha-repo" sorts before "zeta-repo".
        assert_eq!(s.a_repo, "alpha-repo");
        assert_eq!(s.b_repo, "zeta-repo");
        assert!(s.jaccard >= DEDUP_JACCARD_MIN || s.overlap >= DEDUP_OVERLAP_MIN);
    }

    #[test]
    fn suggest_duplicates_emits_each_pair_at_most_once() {
        let entries = vec![
            verdict_text("a", "one", "shared overlapping duplicate token words", None),
            verdict_text("b", "two", "shared overlapping duplicate token words", None),
        ];
        let suggestions = suggest_duplicates(&entries);
        assert_eq!(suggestions.len(), 1, "one unordered pair, one suggestion");
    }

    #[test]
    fn suggest_duplicates_allows_same_repo_pairs() {
        let entries = vec![
            verdict_text(
                "mev",
                "dup-one",
                "shared overlapping duplicate token words",
                None,
            ),
            verdict_text(
                "mev",
                "dup-two",
                "shared overlapping duplicate token words",
                None,
            ),
        ];
        let suggestions = suggest_duplicates(&entries);
        assert_eq!(
            suggestions.len(),
            1,
            "a duplicate filed twice in one repo is still a duplicate"
        );
        assert_eq!(suggestions[0].a_repo, "mev");
        assert_eq!(suggestions[0].b_repo, "mev");
    }

    #[test]
    fn suggest_duplicates_rejects_pairs_below_both_thresholds() {
        let entries = vec![
            verdict_text(
                "mev",
                "unrelated-one",
                "completely different topic entirely",
                None,
            ),
            verdict_text(
                "okf-core",
                "unrelated-two",
                "another distinct subject matter altogether",
                None,
            ),
        ];
        let suggestions = suggest_duplicates(&entries);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn suggest_duplicates_output_is_deterministically_ordered() {
        let entries = vec![
            verdict_text(
                "mev",
                "pair-one-a",
                "alpha bravo charlie delta echo foxtrot",
                None,
            ),
            verdict_text(
                "okf-core",
                "pair-one-b",
                "alpha bravo charlie delta echo foxtrot",
                None,
            ),
            verdict_text(
                "mev",
                "pair-two-a",
                "golf hotel india juliet kilo lima",
                None,
            ),
            verdict_text(
                "okf-core",
                "pair-two-b",
                "golf hotel india juliet kilo lima",
                None,
            ),
        ];
        let run_a = suggest_duplicates(&entries);
        let mut shuffled = entries.clone();
        shuffled.reverse();
        let run_b = suggest_duplicates(&shuffled);

        let keys_a: Vec<(String, String, String, String)> = run_a
            .iter()
            .map(|s| {
                (
                    s.a_repo.clone(),
                    s.a_slug.clone(),
                    s.b_repo.clone(),
                    s.b_slug.clone(),
                )
            })
            .collect();
        let keys_b: Vec<(String, String, String, String)> = run_b
            .iter()
            .map(|s| {
                (
                    s.a_repo.clone(),
                    s.a_slug.clone(),
                    s.b_repo.clone(),
                    s.b_slug.clone(),
                )
            })
            .collect();
        assert_eq!(keys_a, keys_b, "identical output regardless of input order");
    }

    // -- assign_triage_lane (MV.ticket.carryover-triage-ranking) -------------

    fn triage_verdict(priority: Option<u8>, stale: bool) -> CarryoverVerdict {
        CarryoverVerdict {
            repo: "mev".to_string(),
            slug: "triage-slug".to_string(),
            kind: "known_issue".to_string(),
            text: "some triage text".to_string(),
            clears_when: None,
            created: "2026-01-01".to_string(),
            age_days: Some(1),
            stale,
            lane: CarryoverLane::NotEvaluable,
            refs: Vec::new(),
            reason: None,
            priority,
            finding_id: None,
            blocks: Vec::new(),
            enforce: None,
            needs: None,
        }
    }

    #[test]
    fn assign_triage_lane_fresh_p0_lands_hot_not_gated_on_staleness() {
        // The whole point: membership must not gate on staleness alone. A
        // fresh (non-stale) P0 must still be HOT, not invisible.
        let v = triage_verdict(Some(0), false);
        assert_eq!(assign_triage_lane(&v, &[]), TriageLane::Hot);
    }

    #[test]
    fn assign_triage_lane_p1_is_hot() {
        let v = triage_verdict(Some(1), false);
        assert_eq!(assign_triage_lane(&v, &[]), TriageLane::Hot);
    }

    #[test]
    fn assign_triage_lane_unmet_block_wins_even_when_also_p0() {
        // BLOCKING is evaluated before HOT, so a P0 with an unmet block edge
        // lands in BLOCKING, not HOT.
        let v = triage_verdict(Some(0), false);
        let unmet = vec!["mev:MV.3.A".to_string()];
        assert_eq!(assign_triage_lane(&v, &unmet), TriageLane::Blocking);
    }

    #[test]
    fn assign_triage_lane_stale_p2_is_aging() {
        let v = triage_verdict(Some(2), true);
        assert_eq!(assign_triage_lane(&v, &[]), TriageLane::Aging);
    }

    #[test]
    fn assign_triage_lane_stale_no_priority_is_aging() {
        let v = triage_verdict(None, true);
        assert_eq!(assign_triage_lane(&v, &[]), TriageLane::Aging);
    }

    #[test]
    fn assign_triage_lane_no_priority_no_blocks_non_stale_is_standing() {
        let v = triage_verdict(None, false);
        assert_eq!(assign_triage_lane(&v, &[]), TriageLane::Standing);
    }

    #[test]
    fn assign_triage_lane_non_stale_p2_is_standing_not_aging() {
        // p2/p3 without staleness has no lane of its own — it falls through
        // to STANDING since AGING requires `stale`.
        let v = triage_verdict(Some(2), false);
        assert_eq!(assign_triage_lane(&v, &[]), TriageLane::Standing);
    }

    #[test]
    fn assign_triage_lane_is_total_over_every_combination() {
        // Every (priority, stale, has_unmet_blocks) combination must land in
        // exactly one lane — no panic, no ambiguity.
        let priorities: [Option<u8>; 5] = [None, Some(0), Some(1), Some(2), Some(3)];
        for priority in priorities {
            for stale in [true, false] {
                for unmet in [Vec::new(), vec!["mev:MV.1.A".to_string()]] {
                    let v = triage_verdict(priority, stale);
                    // Must not panic; result is one of the four variants by
                    // construction (the return type is TriageLane itself).
                    let _lane = assign_triage_lane(&v, &unmet);
                }
            }
        }
    }

    // -- carryover_effective_priorities (MV.ticket.carryover-triage-ranking, task 2) ----

    fn ranking_verdict(
        repo: &str,
        slug: &str,
        priority: Option<u8>,
        blocks: Vec<BlockedBy>,
    ) -> CarryoverVerdict {
        CarryoverVerdict {
            repo: repo.to_string(),
            slug: slug.to_string(),
            kind: "known_issue".to_string(),
            text: format!("some text for {slug}"),
            clears_when: None,
            created: "2026-01-01".to_string(),
            age_days: Some(1),
            stale: false,
            lane: CarryoverLane::NotEvaluable,
            refs: Vec::new(),
            reason: None,
            priority,
            finding_id: None,
            blocks,
            enforce: None,
            needs: None,
        }
    }

    fn block_edge(repo: &str, id: &str) -> BlockedBy {
        BlockedBy::Block(BlockDep {
            repo: repo.to_string(),
            id: id.to_string(),
            what: None,
        })
    }

    // -- classify_blocked_by_edge (MV.16.A, task 1) --------------------------

    #[test]
    fn classify_blocked_by_edge_open_target_is_blocking() {
        let block_status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let edge = block_edge("mev", "MV.1.A");
        let c = classify_blocked_by_edge("mev", &edge, &block_status);
        assert_eq!(c.edge_type, BlockedByEdgeType::Block);
        assert_eq!(c.target_key.as_deref(), Some("mev:MV.1.A"));
        assert_eq!(c.target_status.as_deref(), Some("open"));
        assert_eq!(c.verdict, EdgeBlockVerdict::Blocking);
        assert!(c.is_blocking());
    }

    #[test]
    fn classify_blocked_by_edge_closed_target_is_not_blocking() {
        let block_status = HashMap::from([("mev:MV.1.A".to_string(), Some("closed".to_string()))]);
        let edge = block_edge("mev", "MV.1.A");
        let c = classify_blocked_by_edge("mev", &edge, &block_status);
        assert_eq!(c.verdict, EdgeBlockVerdict::Closed);
        assert!(!c.is_blocking());
    }

    #[test]
    fn classify_blocked_by_edge_wontfix_target_is_not_blocking() {
        let block_status = HashMap::from([("mev:JF.2.A".to_string(), Some("wontfix".to_string()))]);
        let edge = block_edge("mev", "JF.2.A");
        let c = classify_blocked_by_edge("mev", &edge, &block_status);
        assert_eq!(c.verdict, EdgeBlockVerdict::Wontfix);
        assert!(!c.is_blocking());
    }

    #[test]
    fn classify_blocked_by_edge_unresolvable_target_is_not_blocking() {
        let edge = block_edge("mev", "MV.99.Z");
        let c = classify_blocked_by_edge("mev", &edge, &HashMap::new());
        assert_eq!(c.target_key.as_deref(), Some("mev:MV.99.Z"));
        assert_eq!(c.target_status, None);
        assert_eq!(c.verdict, EdgeBlockVerdict::Unresolvable);
        assert!(!c.is_blocking());
    }

    #[test]
    fn classify_blocked_by_edge_external_has_no_node_target() {
        let edge = BlockedBy::External(ExternalDep {
            what: "waiting on vendor API".to_string(),
        });
        let c = classify_blocked_by_edge("mev", &edge, &HashMap::new());
        assert_eq!(c.edge_type, BlockedByEdgeType::External);
        assert_eq!(c.target_key, None);
        assert_eq!(c.target_status, None);
        assert_eq!(c.verdict, EdgeBlockVerdict::NoNodeTarget);
        assert!(!c.is_blocking());
    }

    #[test]
    fn classify_blocked_by_edge_operator_has_no_node_target() {
        let edge = BlockedBy::Operator(OperatorDep {
            slug: "some-session".to_string(),
            exit: "some/artifact.md".to_string(),
            start: "/begin-session some-session".to_string(),
            what: None,
        });
        let c = classify_blocked_by_edge("mev", &edge, &HashMap::new());
        assert_eq!(c.edge_type, BlockedByEdgeType::Operator);
        assert_eq!(c.target_key, None);
        assert_eq!(c.verdict, EdgeBlockVerdict::NoNodeTarget);
        assert!(!c.is_blocking());
    }

    #[test]
    fn classify_blocked_by_edge_approval_has_no_node_target() {
        let edge = BlockedBy::Approval(ApprovalDep {
            slug: "some-approval".to_string(),
            what: "ship the thing".to_string(),
            digest: "deadbeef".to_string(),
        });
        let c = classify_blocked_by_edge("mev", &edge, &HashMap::new());
        assert_eq!(c.edge_type, BlockedByEdgeType::Approval);
        assert_eq!(c.target_key, None);
        assert_eq!(c.verdict, EdgeBlockVerdict::NoNodeTarget);
        assert!(!c.is_blocking());
    }

    #[test]
    fn classify_blocked_by_edge_empty_repo_falls_back_to_entry_repo() {
        let block_status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let edge = block_edge("", "MV.1.A");
        let c = classify_blocked_by_edge("mev", &edge, &block_status);
        assert_eq!(c.target_key.as_deref(), Some("mev:MV.1.A"));
        assert_eq!(c.verdict, EdgeBlockVerdict::Blocking);
    }

    // -- unmet_carryover_block_keys (delegates to classify_blocked_by_edge) --

    #[test]
    fn unmet_carryover_block_keys_treats_wontfix_as_unmet() {
        let entry = ranking_verdict("mev", "gated", None, vec![block_edge("mev", "JF.2.A")]);
        let block_status = HashMap::from([("mev:JF.2.A".to_string(), Some("wontfix".to_string()))]);
        assert_eq!(
            unmet_carryover_block_keys(&entry, &block_status),
            vec!["mev:JF.2.A".to_string()]
        );
    }

    #[test]
    fn unmet_carryover_block_keys_treats_closed_as_met() {
        let entry = ranking_verdict("mev", "gated", None, vec![block_edge("mev", "MV.1.A")]);
        let block_status = HashMap::from([("mev:MV.1.A".to_string(), Some("closed".to_string()))]);
        assert!(unmet_carryover_block_keys(&entry, &block_status).is_empty());
    }

    #[test]
    fn unmet_carryover_block_keys_treats_unresolvable_as_unmet() {
        let entry = ranking_verdict("mev", "gated", None, vec![block_edge("mev", "MV.99.Z")]);
        assert_eq!(
            unmet_carryover_block_keys(&entry, &HashMap::new()),
            vec!["mev:MV.99.Z".to_string()]
        );
    }

    // -- build_lane_residency_index / LaneResidencyIndex (MV.16.A, task 2) --

    fn write_fixture(dir: &std::path::Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn lane_json(lane: &str, roadmap: &str, blocks: &[(&str, &str)]) -> String {
        // blocks: (repo, id)
        let blocks_json: Vec<String> = blocks
            .iter()
            .map(|(repo, id)| {
                format!(r#"{{"id":"{id}","origin_roadmap":"{roadmap}","repo":"{repo}"}}"#)
            })
            .collect();
        format!(
            r#"{{"lane":"{lane}","roadmap":"{roadmap}","blocks":[{}]}}"#,
            blocks_json.join(",")
        )
    }

    #[test]
    fn lane_residency_target_present_in_one_lane() {
        let dir = crate::testsupport::unique_temp_dir("mev-carryover-lane-residency-one");
        write_fixture(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &lane_json("substrate", "alpha", &[("mev", "MV.1.A")]),
        );

        let (index, diags) = build_lane_residency_index(&dir);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert!(index.is_resident("mev:MV.1.A"));
        assert_eq!(
            index.lanes_for("mev:MV.1.A"),
            &["alpha/lane-substrate.json".to_string()]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lane_residency_target_present_in_two_lanes() {
        let dir = crate::testsupport::unique_temp_dir("mev-carryover-lane-residency-two");
        write_fixture(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &lane_json("substrate", "alpha", &[("mev", "MV.1.A")]),
        );
        write_fixture(
            &dir,
            "planning/roadmaps/beta/lane-web.json",
            &lane_json("web", "beta", &[("mev", "MV.1.A")]),
        );

        let (index, diags) = build_lane_residency_index(&dir);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        let lanes = index.lanes_for("mev:MV.1.A");
        assert_eq!(lanes.len(), 2, "expected 2 lanes, got {lanes:?}");
        assert!(lanes.contains(&"alpha/lane-substrate.json".to_string()));
        assert!(lanes.contains(&"beta/lane-web.json".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lane_residency_target_in_no_lane() {
        let dir = crate::testsupport::unique_temp_dir("mev-carryover-lane-residency-none");
        write_fixture(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &lane_json("substrate", "alpha", &[("mev", "MV.1.A")]),
        );

        let (index, diags) = build_lane_residency_index(&dir);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert!(!index.is_resident("mev:MV.2.B"));
        assert!(index.lanes_for("mev:MV.2.B").is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lane_residency_id_match_but_repo_mismatch_is_not_resident() {
        let dir = crate::testsupport::unique_temp_dir("mev-carryover-lane-residency-repo-mismatch");
        write_fixture(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            &lane_json("substrate", "alpha", &[("mev", "MV.1.A")]),
        );

        let (index, diags) = build_lane_residency_index(&dir);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        // Same id, different repo — must NOT be resident.
        assert!(!index.is_resident("base-template:MV.1.A"));
        assert!(index.is_resident("mev:MV.1.A"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -- compute_would_block_report / renderers (MV.16.A, task 3) -----------

    fn would_block_fixture_entries() -> Vec<CarryoverVerdict> {
        vec![
            ranking_verdict(
                "mev",
                "blocking-entry",
                None,
                vec![block_edge("mev", "MV.1.A")],
            ),
            ranking_verdict(
                "mev",
                "closed-entry",
                None,
                vec![block_edge("mev", "MV.2.B")],
            ),
            ranking_verdict(
                "mev",
                "wontfix-entry",
                None,
                vec![block_edge("mev", "JF.2.A")],
            ),
            ranking_verdict(
                "mev",
                "unresolvable-entry",
                None,
                vec![block_edge("mev", "MV.99.Z")],
            ),
            ranking_verdict(
                "mev",
                "external-entry",
                None,
                vec![BlockedBy::External(ExternalDep {
                    what: "waiting on vendor API".to_string(),
                })],
            ),
        ]
    }

    fn would_block_fixture_status() -> HashMap<String, Option<String>> {
        HashMap::from([
            ("mev:MV.1.A".to_string(), Some("open".to_string())),
            ("mev:MV.2.B".to_string(), Some("closed".to_string())),
            ("mev:JF.2.A".to_string(), Some("wontfix".to_string())),
        ])
    }

    #[test]
    fn compute_would_block_report_summary_counts_one_of_each_verdict() {
        let entries = would_block_fixture_entries();
        let status = would_block_fixture_status();
        let lane_index = LaneResidencyIndex::default();

        let report = compute_would_block_report(&entries, &status, &lane_index);

        assert_eq!(report.summary.total_edges, 5);
        assert_eq!(report.summary.blocking, 1);
        assert_eq!(report.summary.closed, 1);
        assert_eq!(report.summary.wontfix, 1);
        assert_eq!(report.summary.unresolvable, 1);
        assert_eq!(report.summary.no_node_target, 1);
        assert_eq!(report.rows.len(), 5);
    }

    #[test]
    fn compute_would_block_report_row_carries_owner_and_lane_residency() {
        let entries = vec![ranking_verdict(
            "mev",
            "blocking-entry",
            None,
            vec![block_edge("mev", "MV.1.A")],
        )];
        let status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let mut by_target: HashMap<String, Vec<String>> = HashMap::new();
        by_target.insert(
            "mev:MV.1.A".to_string(),
            vec!["alpha/lane-substrate.json".to_string()],
        );
        let lane_index = LaneResidencyIndex { by_target };

        let report = compute_would_block_report(&entries, &status, &lane_index);

        assert_eq!(report.rows.len(), 1);
        let row = &report.rows[0];
        assert_eq!(row.owner, "mev:blocking-entry");
        assert_eq!(row.edge_type, BlockedByEdgeType::Block);
        assert_eq!(row.target_key.as_deref(), Some("mev:MV.1.A"));
        assert_eq!(row.target_status.as_deref(), Some("open"));
        assert!(row.lane_resident);
        assert_eq!(row.lanes, vec!["alpha/lane-substrate.json".to_string()]);
        assert_eq!(row.verdict, EdgeBlockVerdict::Blocking);
    }

    #[test]
    fn compute_would_block_report_open_target_in_no_lane_is_blocking_but_not_resident() {
        let entries = vec![ranking_verdict(
            "mev",
            "blocking-entry",
            None,
            vec![block_edge("mev", "MV.1.A")],
        )];
        let status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let lane_index = LaneResidencyIndex::default();

        let report = compute_would_block_report(&entries, &status, &lane_index);

        let row = &report.rows[0];
        assert_eq!(row.verdict, EdgeBlockVerdict::Blocking);
        assert!(!row.lane_resident);
        assert!(row.lanes.is_empty());
    }

    #[test]
    fn compute_would_block_report_non_block_edges_have_no_target_and_are_not_resident() {
        let entries = vec![ranking_verdict(
            "mev",
            "external-entry",
            None,
            vec![BlockedBy::External(ExternalDep {
                what: "waiting on vendor API".to_string(),
            })],
        )];
        let lane_index = LaneResidencyIndex::default();

        let report = compute_would_block_report(&entries, &HashMap::new(), &lane_index);

        let row = &report.rows[0];
        assert_eq!(row.edge_type, BlockedByEdgeType::External);
        assert_eq!(row.target_key, None);
        assert_eq!(row.target_status, None);
        assert!(!row.lane_resident);
        assert_eq!(row.verdict, EdgeBlockVerdict::NoNodeTarget);
    }

    #[test]
    fn compute_would_block_report_writes_nothing() {
        let dir = crate::testsupport::unique_temp_dir("mev-carryover-would-block-no-write");
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("untouched.txt");
        std::fs::write(&marker, "before").unwrap();
        let before = std::fs::read_to_string(&marker).unwrap();

        let entries = would_block_fixture_entries();
        let status = would_block_fixture_status();
        let (lane_index, _diags) = build_lane_residency_index(&dir);
        let report = compute_would_block_report(&entries, &status, &lane_index);
        let _ = render_would_block_table(&report);
        let _ = render_would_block_json(&report).unwrap();

        let after = std::fs::read_to_string(&marker).unwrap();
        assert_eq!(before, after, "compute/render must never write to disk");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn would_block_json_and_table_renderers_agree_row_for_row() {
        let entries = would_block_fixture_entries();
        let status = would_block_fixture_status();
        let lane_index = LaneResidencyIndex::default();
        let report = compute_would_block_report(&entries, &status, &lane_index);

        let json = render_would_block_json(&report).unwrap();
        let table = render_would_block_table(&report);

        let parsed: WouldBlockReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, report);

        for row in &report.rows {
            let target = row.target_key.as_deref().unwrap_or("-");
            assert!(
                table.contains(&row.owner) && table.contains(target),
                "table missing row for {} -> {}",
                row.owner,
                target
            );
            assert!(
                table.contains(would_block_verdict_label(row.verdict)),
                "table missing verdict label {:?}",
                row.verdict
            );
        }

        assert!(table.contains(&format!("total: {}", report.summary.total_edges)));
    }

    // -- differential test: --would-block vs unmet_carryover_block_keys (MV.16.A,
    // task 5) -----------------------------------------------------------------
    //
    // The load-bearing test named in this block's `notes`: `unmet_carryover_block_keys`
    // and `compute_would_block_report`'s per-edge verdict must never resolve a `Block`
    // edge's target differently, since both delegate to `classify_blocked_by_edge`
    // (task 1's doc comment on `unmet_carryover_block_keys` makes this contract
    // explicit). The two functions DO deliberately diverge on what counts as
    // "blocking": the legacy predicate treats `Wontfix` and `Unresolvable` targets as
    // unmet (it predates the wontfix/unresolvable distinction and has other callers —
    // `rank_carryover`, the triage lanes — that must keep seeing them as unmet), while
    // `--would-block` explicitly does not count either toward its blocking headline.
    // This test asserts BOTH: agreement everywhere else, and the two carve-outs named
    // by identity rather than tolerated as an unexplained difference.
    #[test]
    fn would_block_blocking_verdict_agrees_with_unmet_carryover_block_keys_except_wontfix_and_unresolvable()
     {
        let entries = vec![
            ranking_verdict("mev", "open-entry", None, vec![block_edge("mev", "MV.1.A")]),
            ranking_verdict(
                "mev",
                "closed-entry",
                None,
                vec![block_edge("mev", "MV.2.B")],
            ),
            ranking_verdict(
                "mev",
                "wontfix-entry",
                None,
                vec![block_edge("mev", "JF.2.A")],
            ),
            ranking_verdict(
                "mev",
                "unresolvable-entry",
                None,
                vec![block_edge("mev", "MV.99.Z")],
            ),
        ];
        let status = HashMap::from([
            ("mev:MV.1.A".to_string(), Some("open".to_string())),
            ("mev:MV.2.B".to_string(), Some("closed".to_string())),
            ("mev:JF.2.A".to_string(), Some("wontfix".to_string())),
        ]);
        let lane_index = LaneResidencyIndex::default();
        let report = compute_would_block_report(&entries, &status, &lane_index);

        assert_eq!(entries.len(), report.rows.len());
        let mut carved_out = Vec::new();
        for (entry, row) in entries.iter().zip(report.rows.iter()) {
            let key = row
                .target_key
                .clone()
                .expect("every entry here carries exactly one Block edge");
            let legacy_unmet = unmet_carryover_block_keys(entry, &status).contains(&key);

            match row.verdict {
                EdgeBlockVerdict::Wontfix | EdgeBlockVerdict::Unresolvable => {
                    // Deliberate carve-out: the legacy predicate still treats this
                    // target as unmet (it must, for `rank_carryover`'s existing
                    // contract), but `--would-block` does not count it as blocking.
                    assert!(
                        legacy_unmet,
                        "legacy predicate should still treat {key} as unmet (verdict {:?})",
                        row.verdict
                    );
                    assert!(
                        row.verdict != EdgeBlockVerdict::Blocking,
                        "--would-block must not count a {:?} target as blocking",
                        row.verdict
                    );
                    carved_out.push(row.verdict);
                }
                _ => {
                    assert_eq!(
                        legacy_unmet,
                        row.verdict == EdgeBlockVerdict::Blocking,
                        "divergence on {key}: legacy unmet={legacy_unmet}, \
                         would-block is_blocking={}, verdict={:?}",
                        row.verdict == EdgeBlockVerdict::Blocking,
                        row.verdict
                    );
                }
            }
        }

        assert!(
            carved_out.contains(&EdgeBlockVerdict::Wontfix),
            "expected the wontfix row to hit the carve-out branch"
        );
        assert!(
            carved_out.contains(&EdgeBlockVerdict::Unresolvable),
            "expected the unresolvable row to hit the carve-out branch"
        );
    }

    // -- build_carryover_gating_sets (MV.16.C, task 2) ------------------------

    fn verdict_with_enforce(
        repo: &str,
        slug: &str,
        blocks: Vec<BlockedBy>,
        enforce: Option<bool>,
    ) -> CarryoverVerdict {
        CarryoverVerdict {
            enforce,
            ..ranking_verdict(repo, slug, None, blocks)
        }
    }

    #[test]
    fn gating_set_empty_when_enforce_blocks_is_false() {
        let entries = vec![verdict_with_enforce(
            "mev",
            "e1",
            vec![block_edge("mev", "MV.1.A")],
            None,
        )];
        let status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let sets = build_carryover_gating_sets(&entries, &status, false, 10);
        assert!(
            sets.is_empty(),
            "enforce_blocks=false must yield an empty gating set regardless of edges"
        );
    }

    #[test]
    fn gating_set_holds_open_target_and_names_owner() {
        let entries = vec![verdict_with_enforce(
            "mev",
            "finding-1",
            vec![block_edge("mev", "MV.1.A")],
            None,
        )];
        let status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let sets = build_carryover_gating_sets(&entries, &status, true, 10);
        let repo_report = sets.get("mev").expect("mev repo should have a report");
        assert_eq!(repo_report.candidate_count, 1);
        assert_eq!(repo_report.applied_count, 1);
        assert!(!repo_report.cap_exceeded);
        let gate = repo_report
            .gates
            .get("mev:MV.1.A")
            .expect("mev:MV.1.A should be gated");
        assert_eq!(gate.owner, "mev:finding-1");
    }

    #[test]
    fn gating_set_skips_closed_and_wontfix_and_unresolvable_targets() {
        let entries = vec![
            verdict_with_enforce("mev", "closed-e", vec![block_edge("mev", "MV.2.B")], None),
            verdict_with_enforce("mev", "wontfix-e", vec![block_edge("mev", "JF.2.A")], None),
            verdict_with_enforce(
                "mev",
                "unresolvable-e",
                vec![block_edge("mev", "MV.99.Z")],
                None,
            ),
        ];
        let status = HashMap::from([
            ("mev:MV.2.B".to_string(), Some("closed".to_string())),
            ("mev:JF.2.A".to_string(), Some("wontfix".to_string())),
        ]);
        let sets = build_carryover_gating_sets(&entries, &status, true, 10);
        assert!(
            sets.is_empty(),
            "closed/wontfix/unresolvable targets must never contribute a gate"
        );
    }

    #[test]
    fn gating_set_honours_per_entry_enforce_false_opt_out() {
        let entries = vec![verdict_with_enforce(
            "mev",
            "opted-out",
            vec![block_edge("mev", "MV.1.A")],
            Some(false),
        )];
        let status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let sets = build_carryover_gating_sets(&entries, &status, true, 10);
        assert!(
            sets.is_empty(),
            "an entry with enforce: Some(false) must contribute no gate even with enforce_blocks on"
        );
    }

    #[test]
    fn gating_set_enforce_some_true_behaves_like_none() {
        let status = HashMap::from([
            ("mev:MV.1.A".to_string(), Some("open".to_string())),
            ("mev:MV.1.B".to_string(), Some("open".to_string())),
        ]);
        let none_entries = vec![verdict_with_enforce(
            "mev",
            "e1",
            vec![block_edge("mev", "MV.1.A")],
            None,
        )];
        let true_entries = vec![verdict_with_enforce(
            "mev",
            "e2",
            vec![block_edge("mev", "MV.1.B")],
            Some(true),
        )];
        let none_sets = build_carryover_gating_sets(&none_entries, &status, true, 10);
        let true_sets = build_carryover_gating_sets(&true_entries, &status, true, 10);
        assert_eq!(none_sets["mev"].applied_count, 1);
        assert_eq!(true_sets["mev"].applied_count, 1);
    }

    #[test]
    fn gating_set_cap_zero_applies_nothing_and_reports_cap_exceeded() {
        let entries = vec![verdict_with_enforce(
            "mev",
            "e1",
            vec![block_edge("mev", "MV.1.A")],
            None,
        )];
        let status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let sets = build_carryover_gating_sets(&entries, &status, true, 0);
        let repo_report = &sets["mev"];
        assert_eq!(repo_report.candidate_count, 1);
        assert_eq!(repo_report.applied_count, 0);
        assert!(repo_report.gates.is_empty());
        assert!(repo_report.cap_exceeded);
    }

    #[test]
    fn gating_set_cap_between_zero_and_count_applies_exactly_cap_and_reports_remainder() {
        let entries = vec![verdict_with_enforce(
            "mev",
            "e1",
            vec![
                block_edge("mev", "MV.1.A"),
                block_edge("mev", "MV.1.B"),
                block_edge("mev", "MV.1.C"),
            ],
            None,
        )];
        let status = HashMap::from([
            ("mev:MV.1.A".to_string(), Some("open".to_string())),
            ("mev:MV.1.B".to_string(), Some("open".to_string())),
            ("mev:MV.1.C".to_string(), Some("open".to_string())),
        ]);
        let sets = build_carryover_gating_sets(&entries, &status, true, 2);
        let repo_report = &sets["mev"];
        assert_eq!(repo_report.candidate_count, 3);
        assert_eq!(repo_report.applied_count, 2);
        assert_eq!(repo_report.gates.len(), 2);
        assert!(repo_report.cap_exceeded);
    }

    #[test]
    fn gating_set_duplicate_target_from_two_entries_counts_once() {
        let entries = vec![
            verdict_with_enforce("mev", "first", vec![block_edge("mev", "MV.1.A")], None),
            verdict_with_enforce("mev", "second", vec![block_edge("mev", "MV.1.A")], None),
        ];
        let status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let sets = build_carryover_gating_sets(&entries, &status, true, 10);
        let repo_report = &sets["mev"];
        assert_eq!(
            repo_report.candidate_count, 1,
            "the same target gated by two entries should count once"
        );
        let gate = &repo_report.gates["mev:MV.1.A"];
        assert_eq!(
            gate.owner, "mev:first",
            "the first-discovered entry should be recorded as owner"
        );
    }

    // -- render_would_block_enforcement_summary / would_block_enforcement_json
    // (MV.16.C, task 5) ------------------------------------------------------

    #[test]
    fn enforcement_summary_reports_off_with_no_gating() {
        let summary = render_would_block_enforcement_summary(false, 10, &BTreeMap::new());
        assert_eq!(summary, "enforcement: OFF");
    }

    #[test]
    fn enforcement_summary_reports_on_with_cap() {
        let summary = render_would_block_enforcement_summary(true, 10, &BTreeMap::new());
        assert_eq!(summary, "enforcement: ON (cap 10/repo)");
    }

    #[test]
    fn enforcement_summary_reports_cap_exceeded_line_per_repo() {
        let entries = vec![verdict_with_enforce(
            "mev",
            "e1",
            vec![
                block_edge("mev", "MV.1.A"),
                block_edge("mev", "MV.1.B"),
                block_edge("mev", "MV.1.C"),
            ],
            None,
        )];
        let status = HashMap::from([
            ("mev:MV.1.A".to_string(), Some("open".to_string())),
            ("mev:MV.1.B".to_string(), Some("open".to_string())),
            ("mev:MV.1.C".to_string(), Some("open".to_string())),
        ]);
        let sets = build_carryover_gating_sets(&entries, &status, true, 2);
        let summary = render_would_block_enforcement_summary(true, 2, &sets);
        assert!(summary.contains("enforcement: ON (cap 2/repo)"));
        assert!(
            summary.contains("cap exceeded — mev: 2 of 3 gates applied"),
            "unexpected summary: {summary}"
        );
    }

    #[test]
    fn enforcement_summary_omits_cap_exceeded_line_when_under_cap() {
        let entries = vec![verdict_with_enforce(
            "mev",
            "e1",
            vec![block_edge("mev", "MV.1.A")],
            None,
        )];
        let status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        let sets = build_carryover_gating_sets(&entries, &status, true, 10);
        let summary = render_would_block_enforcement_summary(true, 10, &sets);
        assert_eq!(summary, "enforcement: ON (cap 10/repo)");
    }

    #[test]
    fn enforcement_summary_empty_gating_when_flag_off_never_prints_cap_line() {
        let entries = vec![verdict_with_enforce(
            "mev",
            "e1",
            vec![block_edge("mev", "MV.1.A")],
            None,
        )];
        let status = HashMap::from([("mev:MV.1.A".to_string(), Some("open".to_string()))]);
        // enforce_blocks=false -> build_carryover_gating_sets returns an empty map,
        // so even though this fixture would otherwise exceed a cap of 0, no cap
        // line is possible: the flag alone must fully explain the output.
        let sets = build_carryover_gating_sets(&entries, &status, false, 0);
        let summary = render_would_block_enforcement_summary(false, 0, &sets);
        assert_eq!(summary, "enforcement: OFF");
    }

    #[test]
    fn enforcement_json_carries_flag_cap_and_cap_exceeded_repos() {
        let entries = vec![verdict_with_enforce(
            "mev",
            "e1",
            vec![block_edge("mev", "MV.1.A"), block_edge("mev", "MV.1.B")],
            None,
        )];
        let status = HashMap::from([
            ("mev:MV.1.A".to_string(), Some("open".to_string())),
            ("mev:MV.1.B".to_string(), Some("open".to_string())),
        ]);
        let sets = build_carryover_gating_sets(&entries, &status, true, 1);
        let value = would_block_enforcement_json(true, 1, &sets);
        assert_eq!(value["enforce_blocks"], serde_json::json!(true));
        assert_eq!(value["max_gates_per_repo"], serde_json::json!(1));
        let cap_exceeded = value["cap_exceeded"]
            .as_array()
            .expect("cap_exceeded should be an array");
        assert_eq!(cap_exceeded.len(), 1);
        assert_eq!(cap_exceeded[0]["repo"], serde_json::json!("mev"));
        assert_eq!(cap_exceeded[0]["applied"], serde_json::json!(1));
        assert_eq!(cap_exceeded[0]["candidates"], serde_json::json!(2));
    }

    #[test]
    fn enforcement_json_cap_exceeded_empty_when_off() {
        let value = would_block_enforcement_json(false, 10, &BTreeMap::new());
        assert_eq!(value["enforce_blocks"], serde_json::json!(false));
        assert_eq!(
            value["cap_exceeded"].as_array().map(Vec::len),
            Some(0),
            "no repo should be reported over cap when enforcement is off"
        );
    }

    #[test]
    fn gating_set_agrees_with_would_block_on_shared_fixture() {
        let entries = vec![
            verdict_with_enforce("mev", "open-entry", vec![block_edge("mev", "MV.1.A")], None),
            verdict_with_enforce(
                "mev",
                "closed-entry",
                vec![block_edge("mev", "MV.2.B")],
                None,
            ),
            verdict_with_enforce(
                "mev",
                "wontfix-entry",
                vec![block_edge("mev", "JF.2.A")],
                None,
            ),
        ];
        let status = HashMap::from([
            ("mev:MV.1.A".to_string(), Some("open".to_string())),
            ("mev:MV.2.B".to_string(), Some("closed".to_string())),
            ("mev:JF.2.A".to_string(), Some("wontfix".to_string())),
        ]);
        let lane_index = LaneResidencyIndex::default();
        let would_block = compute_would_block_report(&entries, &status, &lane_index);
        let gating = build_carryover_gating_sets(&entries, &status, true, 10);

        let blocking_targets: BTreeSet<String> = would_block
            .rows
            .iter()
            .filter(|r| r.verdict == EdgeBlockVerdict::Blocking)
            .filter_map(|r| r.target_key.clone())
            .collect();
        let gated_targets: BTreeSet<String> = gating
            .values()
            .flat_map(|report| report.gates.keys().cloned())
            .collect();
        assert_eq!(
            blocking_targets, gated_targets,
            "the gating set must agree edge-for-edge with --would-block's Blocking verdicts"
        );
    }

    #[test]
    fn carryover_effective_priorities_p3_blocking_p0_block_resolves_to_zero() {
        let entries = vec![ranking_verdict(
            "mev",
            "gates-a-p0",
            Some(3),
            vec![block_edge("mev", "MV.1.A")],
        )];
        let block_priorities = HashMap::from([("mev:MV.1.A".to_string(), 0u8)]);

        let effective = carryover_effective_priorities(&entries, &block_priorities);

        assert_eq!(effective.get("mev:gates-a-p0"), Some(&0));
    }

    #[test]
    fn carryover_effective_priorities_no_blocks_keeps_own_priority() {
        let entries = vec![ranking_verdict("mev", "solo", Some(2), Vec::new())];
        let effective = carryover_effective_priorities(&entries, &HashMap::new());
        assert_eq!(effective.get("mev:solo"), Some(&2));
    }

    #[test]
    fn carryover_effective_priorities_no_priority_no_blocks_is_absent() {
        let entries = vec![ranking_verdict("mev", "bare", None, Vec::new())];
        let effective = carryover_effective_priorities(&entries, &HashMap::new());
        assert_eq!(effective.get("mev:bare"), None);
        assert!(!effective.contains_key("mev:bare"));
    }

    #[test]
    fn carryover_effective_priorities_unresolvable_target_contributes_nothing() {
        let entries = vec![ranking_verdict(
            "mev",
            "dangling",
            Some(2),
            vec![block_edge("mev", "MV.99.Z")],
        )];
        let effective = carryover_effective_priorities(&entries, &HashMap::new());
        assert_eq!(effective.get("mev:dangling"), Some(&2));
    }

    #[test]
    fn carryover_effective_priorities_external_edge_contributes_no_priority() {
        let entries = vec![ranking_verdict(
            "mev",
            "external-only",
            Some(3),
            vec![BlockedBy::External(ExternalDep {
                what: "nightly cron".to_string(),
            })],
        )];
        let effective = carryover_effective_priorities(&entries, &HashMap::new());
        assert_eq!(effective.get("mev:external-only"), Some(&3));
    }

    #[test]
    fn carryover_effective_priorities_empty_repo_falls_back_to_own_repo() {
        let entries = vec![ranking_verdict(
            "mev",
            "own-repo-target",
            Some(3),
            vec![BlockedBy::Block(BlockDep {
                repo: String::new(),
                id: "MV.1.A".to_string(),
                what: None,
            })],
        )];
        let block_priorities = HashMap::from([("mev:MV.1.A".to_string(), 1u8)]);
        let effective = carryover_effective_priorities(&entries, &block_priorities);
        assert_eq!(effective.get("mev:own-repo-target"), Some(&1));
    }

    #[test]
    fn carryover_effective_priorities_two_node_cycle_terminates_without_hang_or_panic() {
        let entries = vec![
            ranking_verdict("mev", "a", Some(3), vec![block_edge("mev", "b")]),
            ranking_verdict("mev", "b", Some(2), vec![block_edge("mev", "a")]),
        ];
        let effective = carryover_effective_priorities(&entries, &HashMap::new());
        // Must terminate and produce a deterministic, non-panicking result.
        // `a` sees `b`'s own priority (2) via one hop; `b` sees `a`'s own
        // priority (3) via one hop but keeps its own (2) since 2 < 3.
        assert_eq!(effective.get("mev:a"), Some(&2));
        assert_eq!(effective.get("mev:b"), Some(&2));
    }

    #[test]
    fn carryover_effective_priorities_self_edge_terminates_without_hang_or_panic() {
        let entries = vec![ranking_verdict(
            "mev",
            "self-blocker",
            Some(1),
            vec![block_edge("mev", "self-blocker")],
        )];
        let effective = carryover_effective_priorities(&entries, &HashMap::new());
        assert_eq!(effective.get("mev:self-blocker"), Some(&1));
    }

    #[test]
    fn carryover_effective_priorities_reuses_block_pass_without_changing_block_priorities() {
        // A carryover gating a block must never mutate the block's own
        // effective priority map — block targets are terminal.
        let entries = vec![ranking_verdict(
            "mev",
            "gates-block",
            Some(3),
            vec![block_edge("mev", "MV.2.B")],
        )];
        let block_priorities = HashMap::from([("mev:MV.2.B".to_string(), 1u8)]);
        let before = block_priorities.clone();

        let _effective = carryover_effective_priorities(&entries, &block_priorities);

        assert_eq!(
            block_priorities, before,
            "block_priorities must be untouched"
        );
    }

    // -- compute_disposal_plan --------------------------------------------------

    #[test]
    fn compute_disposal_plan_selects_only_cleared_entries_with_raw_record_and_evidence() {
        let files = vec![
            (
                src("repo-a"),
                state_file(
                    "repo-a",
                    vec![("MV.3.A", "closed")],
                    vec![item(
                        "cleared-one",
                        "deferred",
                        Some("MV.3.A lands"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
            (
                src("repo-b"),
                state_file(
                    "repo-b",
                    vec![("MV.9.A", "open")],
                    vec![item(
                        "still-actionable",
                        "deferred",
                        Some("MV.9.A lands"),
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    )],
                ),
            ),
        ];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 1);
        assert_eq!(report.actionable, 1);

        let plan = compute_disposal_plan(&report, &files, &[], COMMAND_EXEC_TIMEOUT);
        assert_eq!(plan.candidates.len(), 1);
        assert!(plan.skipped.is_empty());

        let candidate = &plan.candidates[0];
        assert_eq!(candidate.repo, "repo-a");
        assert_eq!(candidate.slug, "cleared-one");
        // The raw entry is carried verbatim (not just re-derived fields).
        assert_eq!(candidate.entry.slug, "cleared-one");
        assert_eq!(
            candidate.entry.clears_when,
            Some(ClearsWhen::Prose("MV.3.A lands".to_string()))
        );
        assert_eq!(candidate.evidence, "block repo-a:MV.3.A closed");
    }

    #[test]
    fn compute_disposal_plan_reports_load_errors_as_skipped_with_zero_candidates() {
        // A repo with no entries in `files`/`report` at all (as if its
        // state.json failed to load) must still surface as SKIPPED, distinct
        // from a repo that loaded and legitimately cleared nothing.
        let files = vec![(
            src("repo-a"),
            state_file(
                "repo-a",
                vec![("MV.3.A", "closed")],
                vec![item(
                    "cleared-one",
                    "deferred",
                    Some("MV.3.A lands"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                )],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false,
            COMMAND_EXEC_TIMEOUT,
        );

        let load_errors = vec![(
            "repo-broken".to_string(),
            "invalid type: string, expected a boolean at line 4 column 12".to_string(),
        )];
        let plan = compute_disposal_plan(&report, &files, &load_errors, COMMAND_EXEC_TIMEOUT);

        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(plan.skipped.len(), 1);
        assert_eq!(plan.skipped[0].repo, "repo-broken");
        assert!(plan.skipped[0].error.contains("expected a boolean"));
    }

    #[test]
    fn compute_disposal_plan_excludes_command_exits_zero_without_allow_exec() {
        // Guard (4): `--dispose` without `--allow-exec` must never make a
        // `command_exits_zero` entry disposal-eligible. Since `allow_exec:
        // false` keeps the entry out of the `Cleared` lane entirely (it lands
        // `NotEvaluable` with `ExecutionNotAllowed`), selecting on `Cleared`
        // alone is sufficient — assert exactly that here.
        let files = vec![(
            src("repo-a"),
            state_file(
                "repo-a",
                vec![],
                vec![Carryover {
                    slug: "needs-exec".to_string(),
                    scope: CarryoverScope {
                        repo: None,
                        tier: None,
                        cross_repo: None,
                    },
                    kind: carryover_kind_from_str("deferred"),
                    text: "some carryover text".to_string(),
                    related: vec![],
                    clears_when: Some(ClearsWhen::Predicate(
                        ClearsWhenPredicate::CommandExitsZero {
                            command: "true".to_string(),
                            note: None,
                        },
                    )),
                    created: "2020-01-01".to_string(),
                    ..Default::default()
                }],
            ),
        )];
        let status = status_map(&files);
        let report = evaluate_carryover(
            &files,
            &status,
            Path::new("/fake/brain"),
            &HashMap::new(),
            "2026-08-03",
            &thresholds(),
            None,
            false, // allow_exec: false, as --dispose alone must leave it
            COMMAND_EXEC_TIMEOUT,
        );
        assert_eq!(report.cleared, 0);
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::ExecutionNotAllowed)
        );

        let plan = compute_disposal_plan(&report, &files, &[], COMMAND_EXEC_TIMEOUT);
        assert!(plan.candidates.is_empty());
    }

    #[test]
    fn describe_clearing_evidence_joins_every_satisfied_ref_as_one_line() {
        let verdict = CarryoverVerdict {
            repo: "repo-a".to_string(),
            slug: "multi-ref".to_string(),
            kind: "deferred".to_string(),
            text: "text".to_string(),
            clears_when: Some("MV.1.A lands and docs/x.md exists".to_string()),
            created: "2020-01-01".to_string(),
            age_days: None,
            stale: false,
            lane: CarryoverLane::Cleared,
            refs: vec![
                CarryoverRef::Block {
                    key: "repo-a:MV.1.A".to_string(),
                    satisfied: true,
                },
                CarryoverRef::Path {
                    path: "docs/x.md".to_string(),
                    satisfied: true,
                },
            ],
            reason: None,
            priority: None,
            finding_id: None,
            blocks: vec![],
            enforce: None,
            needs: None,
        };
        assert_eq!(
            describe_clearing_evidence(&verdict, COMMAND_EXEC_TIMEOUT),
            "block repo-a:MV.1.A closed; path docs/x.md exists"
        );
    }

    #[test]
    fn describe_clearing_evidence_records_the_exec_timeout_in_force() {
        let verdict = CarryoverVerdict {
            repo: "repo-a".to_string(),
            slug: "cmd-cleared".to_string(),
            kind: "deferred".to_string(),
            text: "text".to_string(),
            clears_when: Some("true exits 0".to_string()),
            created: "2020-01-01".to_string(),
            age_days: None,
            stale: false,
            lane: CarryoverLane::Cleared,
            refs: vec![CarryoverRef::CommandExitsZero {
                command: "true".to_string(),
                satisfied: true,
            }],
            reason: None,
            priority: None,
            finding_id: None,
            blocks: vec![],
            enforce: None,
            needs: None,
        };
        assert_eq!(
            describe_clearing_evidence(&verdict, std::time::Duration::from_secs(5)),
            "command `true` exited 0 (bound 5s)"
        );
    }

    // -- clears_when_display (task 4) --------------------------------------------

    #[test]
    fn clears_when_display_prose_is_byte_identical_to_pre_task4_behaviour() {
        // Regression floor: `clears_when_display` for `Prose` must render
        // exactly the string it always has, unchanged by this task's
        // `Predicate` widening.
        let cw = ClearsWhen::Prose("MV.3.A lands and docs/x.md exists".to_string());
        assert_eq!(
            clears_when_display(&cw).as_deref(),
            Some("MV.3.A lands and docs/x.md exists")
        );
    }

    #[test]
    fn clears_when_display_renders_block_closed_predicate() {
        let cw = ClearsWhen::Predicate(ClearsWhenPredicate::BlockClosed {
            repo: "mev".to_string(),
            id: "MV.16.G".to_string(),
            note: None,
        });
        assert_eq!(
            clears_when_display(&cw).as_deref(),
            Some("block_closed mev:MV.16.G")
        );
    }

    #[test]
    fn clears_when_display_renders_file_exists_predicate() {
        let cw = ClearsWhen::Predicate(ClearsWhenPredicate::FileExists {
            path: "docs/x.md".to_string(),
            note: None,
        });
        assert_eq!(
            clears_when_display(&cw).as_deref(),
            Some("file_exists docs/x.md")
        );
    }

    #[test]
    fn clears_when_display_renders_file_contains_predicate() {
        let cw = ClearsWhen::Predicate(ClearsWhenPredicate::FileContains {
            path: "docs/x.md".to_string(),
            pattern: "done".to_string(),
            note: None,
        });
        assert_eq!(
            clears_when_display(&cw).as_deref(),
            Some("file_contains docs/x.md ~ \"done\"")
        );
    }

    #[test]
    fn clears_when_display_renders_command_exits_zero_predicate_with_note() {
        let cw = ClearsWhen::Predicate(ClearsWhenPredicate::CommandExitsZero {
            command: "true".to_string(),
            note: Some("sanity check".to_string()),
        });
        assert_eq!(
            clears_when_display(&cw).as_deref(),
            Some("command_exits_zero \"true\" — sanity check")
        );
    }

    #[test]
    fn clears_when_display_is_none_only_when_clears_when_itself_is_none() {
        // Every typed predicate variant now produces Some(..) — the None-
        // for-Predicate behaviour stays on `clears_when_prose` only.
        for cw in [
            ClearsWhen::Predicate(ClearsWhenPredicate::BlockClosed {
                repo: "mev".to_string(),
                id: "MV.1.A".to_string(),
                note: None,
            }),
            ClearsWhen::Predicate(ClearsWhenPredicate::FileExists {
                path: "x".to_string(),
                note: None,
            }),
            ClearsWhen::Predicate(ClearsWhenPredicate::FileContains {
                path: "x".to_string(),
                pattern: "y".to_string(),
                note: None,
            }),
            ClearsWhen::Predicate(ClearsWhenPredicate::CommandExitsZero {
                command: "true".to_string(),
                note: None,
            }),
        ] {
            assert!(clears_when_display(&cw).is_some());
            assert!(clears_when_prose(&cw).is_none());
        }
    }

    // -- dispose write path (task 2) --------------------------------------------

    fn candidate(slug: &str, evidence: &str, entry: Carryover) -> DisposalCandidate {
        DisposalCandidate {
            repo: "repo-a".to_string(),
            slug: slug.to_string(),
            entry,
            evidence: evidence.to_string(),
        }
    }

    /// A scratch repo dir with a real `planning/state.json` on disk, mirroring
    /// what `dispose_repo` is actually pointed at in production.
    fn scratch_repo(tag: &str, file: &StateFile) -> (PathBuf, StateSource, PathBuf) {
        let dir = crate::testsupport::unique_temp_dir(&format!("mev-carryover-dispose-{tag}"));
        let planning = dir.join("planning");
        std::fs::create_dir_all(&planning).unwrap();
        let state_path = planning.join("state.json");
        let mut content = serde_json::to_string_pretty(file).unwrap();
        content.push('\n');
        std::fs::write(&state_path, &content).unwrap();
        let archive_path = planning.join("carryover-archive.jsonl");
        let source = StateSource {
            repo_slug: file.repo.clone(),
            abs_path: state_path,
            expected_kind: "project",
        };
        (dir, source, archive_path)
    }

    #[test]
    fn build_archive_row_flattens_entry_verbatim_with_cleared_reason() {
        let entry = item(
            "cleared-one",
            "deferred",
            Some("MV.3.A lands"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let candidate = candidate("cleared-one", "block repo-a:MV.3.A closed", entry.clone());
        let row = build_archive_row(&candidate, "2026-08-21");

        assert_eq!(row.entry.slug, entry.slug);
        assert_eq!(row.entry.clears_when, entry.clears_when);
        assert_eq!(row.disposed_at, "2026-08-21");
        assert_eq!(row.reason, okf_core::DisposalReason::Cleared);
        assert!(!row.reconstructed);
        assert_eq!(row.evidence.as_deref(), Some("block repo-a:MV.3.A closed"));
        assert!(row.amends.is_none());
    }

    // -- build_trajectory (MV.16.F, task 1) --------------------------------------

    /// Write one archive line for `slug`, disposed on `disposed_at`, `reconstructed`
    /// or not, to `archive_path` (appending — callers write several lines).
    fn append_archive_row(archive_path: &Path, slug: &str, disposed_at: &str, reconstructed: bool) {
        let entry = item(slug, "deferred", None, vec![], "2020-01-01", None, None);
        let mut row = build_archive_row(&candidate(slug, "evidence", entry), disposed_at);
        row.reconstructed = reconstructed;
        let mut content = std::fs::read_to_string(archive_path).unwrap_or_default();
        content.push_str(&serde_json::to_string(&row).unwrap());
        content.push('\n');
        std::fs::write(archive_path, content).unwrap();
    }

    #[test]
    fn build_trajectory_buckets_rows_into_their_iso_week() {
        let (_dir, source, archive_path) = scratch_repo(
            "trajectory-bucketing",
            &StateFile {
                repo: "repo-a".into(),
                ..Default::default()
            },
        );
        // 2026-08-24 is a Monday in ISO week 2026-W35; 2026-08-17 is a Monday in
        // 2026-W34.
        append_archive_row(&archive_path, "a", "2026-08-24", false);
        append_archive_row(&archive_path, "b", "2026-08-17", false);
        let files = vec![(source, StateFile::default())];

        let report = build_trajectory(&files, "2026-08-24", 4, None);

        assert_eq!(report.weeks.len(), 4);
        assert_eq!(report.weeks.last().unwrap().iso_week, "2026-W35");
        assert_eq!(report.weeks.last().unwrap().observed, 1);
        let w34 = report
            .weeks
            .iter()
            .find(|w| w.iso_week == "2026-W34")
            .unwrap();
        assert_eq!(w34.observed, 1);
    }

    #[test]
    fn build_trajectory_includes_zero_disposal_weeks_and_last_cumulative_matches_rows_total() {
        let (_dir, source, archive_path) = scratch_repo(
            "trajectory-zero-weeks",
            &StateFile {
                repo: "repo-a".into(),
                ..Default::default()
            },
        );
        append_archive_row(&archive_path, "only-one", "2026-08-24", false);
        let files = vec![(source, StateFile::default())];

        let report = build_trajectory(&files, "2026-08-24", 4, None);

        assert_eq!(report.weeks.len(), 4);
        // Three weeks with zero disposals must still be present, not omitted.
        assert_eq!(report.weeks.iter().filter(|w| w.total() == 0).count(), 3);
        assert_eq!(
            report.weeks.last().unwrap().cumulative,
            report.rows_total - report.undated
        );
    }

    #[test]
    fn build_trajectory_reconstructed_rows_land_in_their_own_column() {
        let (_dir, source, archive_path) = scratch_repo(
            "trajectory-reconstructed",
            &StateFile {
                repo: "repo-a".into(),
                ..Default::default()
            },
        );
        append_archive_row(&archive_path, "observed-row", "2026-08-24", false);
        append_archive_row(&archive_path, "reconstructed-row", "2026-08-24", true);
        let files = vec![(source, StateFile::default())];

        let report = build_trajectory(&files, "2026-08-24", 1, None);

        let week = &report.weeks[0];
        assert_eq!(week.observed, 1);
        assert_eq!(week.reconstructed, 1);
        // Reconstructed rows must never be folded into the observed column.
        assert_ne!(week.observed, week.total());
    }

    #[test]
    fn build_trajectory_shares_the_same_rows_and_stats_read_archive_outflow_reads() {
        let (_dir, source, archive_path) = scratch_repo(
            "trajectory-coherence",
            &StateFile {
                repo: "repo-a".into(),
                ..Default::default()
            },
        );
        append_archive_row(&archive_path, "row-one", "2026-08-24", false);
        append_archive_row(&archive_path, "row-two", "2026-08-17", false);
        let files = vec![(source, StateFile::default())];

        let trajectory = build_trajectory(&files, "2026-08-24", 8, None);
        let outflow = read_archive_outflow(&files, "2026-08-24", 3650, None);

        assert_eq!(trajectory.rows_total, outflow.rows_total);
        assert_eq!(trajectory.archives_read, outflow.archives_read);
        assert_eq!(
            trajectory.weeks.last().unwrap().cumulative,
            outflow.rows_total - trajectory.undated
        );
    }

    #[test]
    fn build_trajectory_undated_row_is_excluded_from_buckets_but_counted_in_rows_total() {
        let (_dir, source, archive_path) = scratch_repo(
            "trajectory-undated",
            &StateFile {
                repo: "repo-a".into(),
                ..Default::default()
            },
        );
        append_archive_row(&archive_path, "good-row", "2026-08-24", false);
        append_archive_row(&archive_path, "bad-row", "not-a-date", false);
        let files = vec![(source, StateFile::default())];

        let report = build_trajectory(&files, "2026-08-24", 1, None);

        assert_eq!(report.rows_total, 2);
        assert_eq!(report.undated, 1);
        assert_eq!(report.weeks[0].total(), 1);
        assert_eq!(report.weeks.last().unwrap().cumulative, 1);
    }

    fn historical_removal(entry: Carryover, commit_subject: &str) -> HistoricalRemoval {
        HistoricalRemoval {
            repo: "repo-a".to_string(),
            archive_path: PathBuf::from("/tmp/repo-a/planning/carryover-archive.jsonl"),
            entry,
            commit_sha: "abc1234".to_string(),
            commit_subject: commit_subject.to_string(),
            commit_date: "2026-08-01".to_string(),
        }
    }

    #[test]
    fn derive_disposal_reason_maps_clearing_wording_to_cleared() {
        let (reason, attributable) = derive_disposal_reason("clear stale carryover entry");
        assert_eq!(reason, okf_core::DisposalReason::Cleared);
        assert!(attributable);

        let (reason, attributable) = derive_disposal_reason("resolve the OK.4.A blocker note");
        assert_eq!(reason, okf_core::DisposalReason::Cleared);
        assert!(attributable);
    }

    #[test]
    fn derive_disposal_reason_maps_replacement_wording_to_superseded() {
        let (reason, attributable) =
            derive_disposal_reason("supersede old constraint carryover with new one");
        assert_eq!(reason, okf_core::DisposalReason::Superseded);
        assert!(attributable);
    }

    #[test]
    fn derive_disposal_reason_maps_promotion_wording_to_promoted() {
        let (reason, attributable) = derive_disposal_reason("promote carryover to MV.9.C block");
        assert_eq!(reason, okf_core::DisposalReason::Promoted);
        assert!(attributable);
    }

    #[test]
    fn derive_disposal_reason_defaults_to_withdrawn_when_unattributable() {
        let (reason, attributable) =
            derive_disposal_reason("move bastiel-registration carryover to business");
        assert_eq!(reason, okf_core::DisposalReason::Withdrawn);
        assert!(!attributable);
    }

    #[test]
    fn build_historical_archive_row_is_verbatim_reconstructed_and_names_the_commit() {
        let entry = item(
            "withdrawn-one",
            "deferred",
            Some("some prose"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let removal = historical_removal(
            entry.clone(),
            "move bastiel-registration carryover to business",
        );
        let row = build_historical_archive_row(&removal);

        assert_eq!(row.entry.slug, entry.slug);
        assert_eq!(row.entry.clears_when, entry.clears_when);
        assert_eq!(row.disposed_at, "2026-08-01");
        assert_eq!(row.reason, okf_core::DisposalReason::Withdrawn);
        assert!(row.reconstructed);
        let evidence = row.evidence.expect("evidence must be set");
        assert!(evidence.starts_with("abc1234 move bastiel-registration carryover to business"));
        assert!(evidence.contains("not attributable"));
        assert!(row.amends.is_none());
    }

    #[test]
    fn build_historical_archive_row_records_attributable_reason_without_the_defaulted_note() {
        let entry = item(
            "cleared-two",
            "defect",
            None,
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let removal = historical_removal(entry, "clear the stale defect entry");
        let row = build_historical_archive_row(&removal);

        assert_eq!(row.reason, okf_core::DisposalReason::Cleared);
        let evidence = row.evidence.expect("evidence must be set");
        assert_eq!(evidence, "abc1234 clear the stale defect entry");
        assert!(!evidence.contains("not attributable"));
    }

    #[test]
    fn dispose_repo_removes_cleared_entry_and_leaves_rest_of_state_byte_identical() {
        let survivor = item(
            "still-actionable",
            "deferred",
            Some("MV.9.A lands"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let cleared_entry = item(
            "cleared-one",
            "deferred",
            Some("MV.3.A lands"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let file = state_file(
            "repo-a",
            vec![],
            vec![survivor.clone(), cleared_entry.clone()],
        );
        let (dir, source, archive_path) = scratch_repo("removes-cleared", &file);
        let original_content = std::fs::read_to_string(&source.abs_path).unwrap();

        let candidates = vec![candidate(
            "cleared-one",
            "block repo-a:MV.3.A closed",
            cleared_entry,
        )];
        let result = dispose_repo(
            &source,
            &file,
            &candidates,
            &archive_path,
            "2026-08-21",
            false,
        )
        .expect("dispose_repo should succeed");
        assert!(result.written);
        assert_eq!(result.disposed.len(), 1);

        let new_content = std::fs::read_to_string(&source.abs_path).unwrap();
        let new_file: StateFile = serde_json::from_str(&new_content).unwrap();
        assert_eq!(new_file.carryover.len(), 1);
        assert_eq!(new_file.carryover[0].slug, "still-actionable");

        // Byte-faithful apart from the removed element: re-serializing what
        // should remain must match on-disk bytes exactly.
        let mut expected_survivor_only = file.clone();
        expected_survivor_only.carryover = vec![survivor];
        let mut expected_content = serde_json::to_string_pretty(&expected_survivor_only).unwrap();
        expected_content.push('\n');
        assert_eq!(new_content, expected_content);
        assert_ne!(new_content, original_content);

        let archive_content = std::fs::read_to_string(&archive_path).unwrap();
        let lines: Vec<&str> = archive_content.lines().collect();
        assert_eq!(lines.len(), 1);
        let row: okf_core::CarryoverArchiveRow = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(row.entry.slug, "cleared-one");
        assert_eq!(row.reason, okf_core::DisposalReason::Cleared);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dispose_repo_appends_to_existing_archive_without_disturbing_prior_lines() {
        let cleared_entry = item(
            "cleared-two",
            "deferred",
            Some("MV.4.A lands"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let file = state_file("repo-a", vec![], vec![cleared_entry.clone()]);
        let (dir, source, archive_path) = scratch_repo("appends-archive", &file);
        let existing_line = r#"{"slug":"already-disposed","kind":"deferred","text":"old","related":[],"created":"2020-01-01","disposed_at":"2026-08-01","reason":"cleared","reconstructed":false}"#;
        std::fs::write(&archive_path, format!("{existing_line}\n")).unwrap();

        let candidates = vec![candidate(
            "cleared-two",
            "block repo-a:MV.4.A closed",
            cleared_entry,
        )];
        dispose_repo(
            &source,
            &file,
            &candidates,
            &archive_path,
            "2026-08-21",
            false,
        )
        .expect("dispose_repo should succeed");

        let archive_content = std::fs::read_to_string(&archive_path).unwrap();
        let lines: Vec<&str> = archive_content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], existing_line);
        let row: okf_core::CarryoverArchiveRow = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(row.entry.slug, "cleared-two");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dispose_repo_dry_run_writes_nothing_but_returns_the_same_disposal_list() {
        let cleared_entry = item(
            "cleared-three",
            "deferred",
            Some("MV.5.A lands"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let file = state_file("repo-a", vec![], vec![cleared_entry.clone()]);
        let (dir, source, archive_path) = scratch_repo("dry-run", &file);
        let original_content = std::fs::read_to_string(&source.abs_path).unwrap();

        let candidates = vec![candidate(
            "cleared-three",
            "block repo-a:MV.5.A closed",
            cleared_entry,
        )];
        let result = dispose_repo(
            &source,
            &file,
            &candidates,
            &archive_path,
            "2026-08-21",
            true,
        )
        .expect("dry-run should succeed without writing");
        assert!(!result.written);
        assert_eq!(result.disposed.len(), 1);

        let after_content = std::fs::read_to_string(&source.abs_path).unwrap();
        assert_eq!(
            after_content, original_content,
            "dry-run must not touch state.json"
        );
        assert!(
            !archive_path.exists(),
            "dry-run must not create the archive file"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dispose_repo_with_no_candidates_is_a_no_op_and_reports_unwritten() {
        let file = state_file("repo-a", vec![], vec![]);
        let (dir, source, archive_path) = scratch_repo("no-candidates", &file);

        let result = dispose_repo(&source, &file, &[], &archive_path, "2026-08-21", false)
            .expect("empty candidates must succeed as a no-op");
        assert!(!result.written);
        assert!(result.disposed.is_empty());
        assert!(
            !archive_path.exists(),
            "a no-op dispose must never create the archive file"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dispose_repo_fails_atomically_when_archive_write_is_impossible() {
        // Constraint (1), shown failing: force the archive write to fail (its
        // parent directory does not exist and cannot be created because a
        // file occupies that path) and assert state.json is left byte-
        // identical — the entry must never end up removed-but-unarchived.
        let cleared_entry = item(
            "cleared-four",
            "deferred",
            Some("MV.6.A lands"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let file = state_file("repo-a", vec![], vec![cleared_entry.clone()]);
        let (dir, source, _unused_archive) = scratch_repo("archive-fails", &file);
        let original_content = std::fs::read_to_string(&source.abs_path).unwrap();

        // Make the archive's parent path unusable: a plain file where a
        // directory would need to exist for the archive path to be writable.
        let blocker_dir = dir.join("blocked");
        std::fs::write(&blocker_dir, b"not a directory").unwrap();
        let impossible_archive_path = blocker_dir.join("carryover-archive.jsonl");

        let candidates = vec![candidate(
            "cleared-four",
            "block repo-a:MV.6.A closed",
            cleared_entry,
        )];
        let result = dispose_repo(
            &source,
            &file,
            &candidates,
            &impossible_archive_path,
            "2026-08-21",
            false,
        );
        assert!(result.is_err(), "archive write must fail here");

        let after_content = std::fs::read_to_string(&source.abs_path).unwrap();
        assert_eq!(
            after_content, original_content,
            "state.json must be untouched when the archive write fails"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    // -- graph-findings `--write` (task 5) ---------------------------------------

    /// Test fixture builder. `clears_when` defaults to `None` here for every
    /// call site EXCEPT the one test
    /// (`carryover_entry_for_finding_sets_typed_clears_when_from_the_finding`)
    /// that exists specifically to pin the task-3 reconciliation this
    /// module's `carryover_entry_for_finding` now performs — real detector
    /// output never has `None` (see
    /// `crate::brain::graph_findings::GraphFinding::clears_when`'s doc
    /// comment), but every OTHER test in this section is about the
    /// write/dedup/slug machinery, not the predicate, so `None` keeps them
    /// focused.
    fn finding(
        detector: crate::brain::graph_findings::DetectorClass,
        repo: &str,
        subject: &str,
        message: &str,
    ) -> crate::brain::graph_findings::GraphFinding {
        finding_with_clears_when(detector, repo, subject, message, None)
    }

    fn finding_with_clears_when(
        detector: crate::brain::graph_findings::DetectorClass,
        repo: &str,
        subject: &str,
        message: &str,
        clears_when: Option<ClearsWhenPredicate>,
    ) -> crate::brain::graph_findings::GraphFinding {
        crate::brain::graph_findings::GraphFinding {
            detector,
            repo: repo.to_string(),
            subject: subject.to_string(),
            message: message.to_string(),
            finding_id: crate::brain::graph_findings::finding_id(detector, subject),
            clears_when,
        }
    }

    #[test]
    fn slug_for_finding_is_readable_kebab_case_and_deterministic() {
        let f = finding(
            crate::brain::graph_findings::DetectorClass::UnregisteredLaneBlock,
            "acme",
            "acme:ACME.9.Z",
            "msg",
        );
        let slug = slug_for_finding(&f);
        assert_eq!(slug, "graph-finding-unregistered-lane-block-acme-acme-9-z");
        // Deterministic: recomputing from the same finding yields the same slug.
        assert_eq!(slug, slug_for_finding(&f));
    }

    #[test]
    fn slug_for_finding_caps_at_eighty_chars_with_no_trailing_dash() {
        let long_subject = "scripts/".to_string() + &"x".repeat(200) + ".py";
        let f = finding(
            crate::brain::graph_findings::DetectorClass::ReferencedPathAbsent,
            "acme",
            &long_subject,
            "msg",
        );
        let slug = slug_for_finding(&f);
        assert!(slug.len() <= 80, "slug too long: {} chars", slug.len());
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn carryover_entry_for_finding_has_single_key_scope_and_drift_kind() {
        let f = finding(
            crate::brain::graph_findings::DetectorClass::ReferencedPathAbsent,
            "acme",
            "scripts/render_spec.py",
            "example.md references 'scripts/render_spec.py', which does not exist",
        );
        let entry = carryover_entry_for_finding(&f, "2026-08-23");

        assert_eq!(entry.scope.repo.as_deref(), Some("acme"));
        assert!(entry.scope.tier.is_none());
        assert!(entry.scope.cross_repo.is_none());
        assert_eq!(
            entry.kind,
            okf_core::CarryoverKind::Known(okf_core::KnownCarryoverKind::Drift)
        );
        assert_eq!(entry.finding_id.as_deref(), Some(f.finding_id.as_str()));
        assert_eq!(entry.created, "2026-08-23");
        assert!(
            entry.clears_when.is_none(),
            "this fixture uses the plain `finding()` builder, which defaults to no \
             predicate -- see carryover_entry_for_finding_sets_typed_clears_when_from_the_finding \
             for the task-3 predicate-propagation assertion"
        );
        assert!(entry.text.contains("render_spec.py"));
    }

    #[test]
    fn carryover_entry_for_finding_sets_typed_clears_when_from_the_finding() {
        // MV.ticket.graph-findings-path-resolution task 3: `clears_when` must
        // come from the finding's own typed predicate, not be left `None`.
        let predicate = ClearsWhenPredicate::FileExists {
            path: "scripts/render_spec.py".to_string(),
            note: Some("clears when the script resolves".to_string()),
        };
        let f = finding_with_clears_when(
            crate::brain::graph_findings::DetectorClass::ReferencedPathAbsent,
            "acme",
            "scripts/render_spec.py",
            "example.md references 'scripts/render_spec.py', which does not exist",
            Some(predicate.clone()),
        );
        let entry = carryover_entry_for_finding(&f, "2026-08-23");

        assert_eq!(
            entry.clears_when,
            Some(ClearsWhen::Predicate(predicate)),
            "carryover_entry_for_finding must propagate the finding's own \
             clears_when instead of leaving it None"
        );
    }

    #[test]
    fn write_graph_findings_for_repo_appends_and_leaves_untouched_portion_byte_identical() {
        let survivor = item(
            "unrelated-entry",
            "deferred",
            Some("MV.9.A lands"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let file = state_file("acme", vec![], vec![survivor.clone()]);
        let (dir, source, _archive) = scratch_repo("write-appends", &file);

        let f = finding(
            crate::brain::graph_findings::DetectorClass::UnregisteredLaneBlock,
            "acme",
            "acme:ACME.9.Z",
            "lane names an unregistered block",
        );
        let result =
            write_graph_findings_for_repo(&source, &file, std::slice::from_ref(&f), "2026-08-23")
                .expect("write must succeed");
        assert!(result.written);
        assert_eq!(result.appended, vec![f.finding_id.clone()]);

        let new_content = std::fs::read_to_string(&source.abs_path).unwrap();
        let new_file: StateFile = serde_json::from_str(&new_content).unwrap();
        assert_eq!(new_file.carryover.len(), 2);
        assert_eq!(new_file.carryover[0].slug, "unrelated-entry");
        assert_eq!(
            new_file.carryover[1].finding_id.as_deref(),
            Some(f.finding_id.as_str())
        );

        // Byte-faithful for everything but the appended entry: reconstructing
        // "original plus exactly this one new entry" and re-serializing must match
        // on-disk bytes exactly (no re-indent, no key reorder).
        let mut expected = file.clone();
        expected
            .carryover
            .push(carryover_entry_for_finding(&f, "2026-08-23"));
        let mut expected_content = serde_json::to_string_pretty(&expected).unwrap();
        expected_content.push('\n');
        assert_eq!(new_content, expected_content);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_graph_findings_for_repo_ignores_findings_for_other_repos() {
        let file = state_file("acme", vec![], vec![]);
        let (dir, source, _archive) = scratch_repo("write-filters-repo", &file);

        let other_repo_finding = finding(
            crate::brain::graph_findings::DetectorClass::UnregisteredLaneBlock,
            "other-repo",
            "other-repo:X.1.A",
            "msg",
        );
        let result =
            write_graph_findings_for_repo(&source, &file, &[other_repo_finding], "2026-08-23")
                .expect("write must succeed");
        assert!(!result.written);
        assert!(result.appended.is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_graph_findings_for_repo_is_idempotent_on_finding_id() {
        let f = finding(
            crate::brain::graph_findings::DetectorClass::ReferencedPathAbsent,
            "acme",
            "scripts/render_spec.py",
            "msg",
        );
        // Simulate a `finding_id` already present from a prior --write.
        let already_written = carryover_entry_for_finding(&f, "2026-08-20");
        let file = state_file("acme", vec![], vec![already_written]);
        let (dir, source, _archive) = scratch_repo("write-idempotent", &file);

        let result = write_graph_findings_for_repo(&source, &file, &[f], "2026-08-23")
            .expect("write must succeed");
        assert!(
            !result.written,
            "a finding already present by finding_id must not be re-appended"
        );
        assert!(result.appended.is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_graph_findings_for_repo_dedups_two_findings_sharing_one_finding_id_in_one_call() {
        // The same missing path can be extracted twice from the same repo (e.g.
        // referenced from two different files) -- both findings share one
        // finding_id and only one carryover[] entry must result.
        let file = state_file("acme", vec![], vec![]);
        let (dir, source, _archive) = scratch_repo("write-dedups-batch", &file);

        let a = finding(
            crate::brain::graph_findings::DetectorClass::ReferencedPathAbsent,
            "acme",
            "scripts/render_spec.py",
            "referenced from command a.md",
        );
        let b = finding(
            crate::brain::graph_findings::DetectorClass::ReferencedPathAbsent,
            "acme",
            "scripts/render_spec.py",
            "referenced from command b.md",
        );
        assert_eq!(a.finding_id, b.finding_id);

        let result = write_graph_findings_for_repo(&source, &file, &[a, b], "2026-08-23")
            .expect("write must succeed");
        assert!(result.written);
        assert_eq!(result.appended.len(), 1);

        let new_content = std::fs::read_to_string(&source.abs_path).unwrap();
        let new_file: StateFile = serde_json::from_str(&new_content).unwrap();
        assert_eq!(new_file.carryover.len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_graph_findings_for_repo_with_no_findings_is_a_no_op() {
        let file = state_file("acme", vec![], vec![]);
        let (dir, source, _archive) = scratch_repo("write-no-findings", &file);
        let original = std::fs::read_to_string(&source.abs_path).unwrap();

        let result = write_graph_findings_for_repo(&source, &file, &[], "2026-08-23")
            .expect("write must succeed");
        assert!(!result.written);
        assert!(result.appended.is_empty());

        let after = std::fs::read_to_string(&source.abs_path).unwrap();
        assert_eq!(after, original);

        let _ = std::fs::remove_dir_all(dir);
    }

    // -- disposal reporting + orchestration (task 3) ----------------------------

    #[test]
    fn archive_path_for_derives_sibling_of_state_json() {
        let state_path = PathBuf::from("/fake/repo-a/planning/state.json");
        assert_eq!(
            archive_path_for(&state_path),
            PathBuf::from("/fake/repo-a/planning/carryover-archive.jsonl")
        );
    }

    #[test]
    fn render_disposal_candidate_full_text_contains_the_whole_entry_not_just_the_slug() {
        let entry = item(
            "cleared-one",
            "deferred",
            Some("MV.3.A lands"),
            vec![],
            "2020-01-01",
            None,
            None,
        );
        let c = candidate("cleared-one", "block repo-a:MV.3.A closed", entry);
        let rendered = render_disposal_candidate_full_text(&c);

        assert!(rendered.contains("repo-a"));
        assert!(rendered.contains("cleared-one"));
        assert!(rendered.contains("block repo-a:MV.3.A closed"));
        // The full entry text, not merely the slug/evidence line above.
        assert!(rendered.contains("some carryover text"));
        assert!(rendered.contains("2020-01-01"));
        assert!(rendered.contains("MV.3.A lands"));
    }

    #[test]
    fn render_dispose_preamble_joins_every_candidate_in_plan_order() {
        let plan = DisposalPlan {
            candidates: vec![
                candidate(
                    "first",
                    "block a closed",
                    item("first", "deferred", None, vec![], "2020-01-01", None, None),
                ),
                candidate(
                    "second",
                    "block b closed",
                    item("second", "deferred", None, vec![], "2020-01-02", None, None),
                ),
            ],
            skipped: vec![],
        };
        let rendered = render_dispose_preamble(&plan);
        let first_idx = rendered.find("'first'").expect("first candidate present");
        let second_idx = rendered.find("'second'").expect("second candidate present");
        assert!(
            first_idx < second_idx,
            "candidates must render in plan order"
        );
    }

    #[test]
    fn render_dispose_preamble_is_empty_for_an_empty_plan() {
        let plan = DisposalPlan::default();
        assert_eq!(render_dispose_preamble(&plan), "");
    }

    #[test]
    fn run_dispose_covers_every_loaded_repo_even_with_zero_candidates() {
        // Constraint (6): a repo the sweep reached with nothing to dispose
        // must still produce a RepoDisposalWrite (0 disposed), distinct from
        // a repo `run_dispose` never visited at all.
        let file_a = state_file(
            "repo-a",
            vec![],
            vec![item(
                "cleared-one",
                "deferred",
                Some("MV.3.A lands"),
                vec![],
                "2020-01-01",
                None,
                None,
            )],
        );
        let file_b = state_file("repo-b", vec![], vec![]);
        let (dir_a, source_a, _archive_a) = scratch_repo("run-dispose-a", &file_a);
        let (dir_b, source_b, _archive_b) = scratch_repo("run-dispose-b", &file_b);
        let files = vec![
            (source_a.clone(), file_a),
            (source_b.clone(), file_b.clone()),
        ];

        let plan = DisposalPlan {
            candidates: vec![candidate(
                "cleared-one",
                "block repo-a:MV.3.A closed",
                item(
                    "cleared-one",
                    "deferred",
                    Some("MV.3.A lands"),
                    vec![],
                    "2020-01-01",
                    None,
                    None,
                ),
            )],
            skipped: vec![SkippedRepo {
                repo: "repo-broken".to_string(),
                error: "invalid JSON".to_string(),
            }],
        };

        let report = run_dispose(&plan, &files, "2026-08-21", false);

        assert_eq!(report.writes.len(), 2);
        let write_a = report
            .writes
            .iter()
            .find(|w| w.repo == "repo-a")
            .expect("repo-a written");
        assert_eq!(write_a.disposed.len(), 1);
        assert!(write_a.written);
        let write_b = report
            .writes
            .iter()
            .find(|w| w.repo == "repo-b")
            .expect("repo-b written");
        assert!(write_b.disposed.is_empty());
        assert!(!write_b.written, "zero-candidate repo writes nothing");

        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].repo, "repo-broken");
        assert!(report.failures.is_empty());
        assert!(report.succeeded());

        let _ = std::fs::remove_dir_all(dir_a);
        let _ = std::fs::remove_dir_all(dir_b);
    }

    #[test]
    fn render_dispose_summary_reports_zero_disposed_skipped_and_failed_repos() {
        let report = DisposeRunReport {
            writes: vec![
                RepoDisposalWrite {
                    repo: "repo-a".to_string(),
                    state_path: PathBuf::from("/fake/repo-a/planning/state.json"),
                    archive_path: PathBuf::from("/fake/repo-a/planning/carryover-archive.jsonl"),
                    disposed: vec![candidate(
                        "cleared-one",
                        "block a closed",
                        item(
                            "cleared-one",
                            "deferred",
                            None,
                            vec![],
                            "2020-01-01",
                            None,
                            None,
                        ),
                    )],
                    written: true,
                },
                RepoDisposalWrite {
                    repo: "repo-b".to_string(),
                    state_path: PathBuf::from("/fake/repo-b/planning/state.json"),
                    archive_path: PathBuf::from("/fake/repo-b/planning/carryover-archive.jsonl"),
                    disposed: vec![],
                    written: false,
                },
            ],
            failures: vec![RepoDisposalError {
                repo: "repo-c".to_string(),
                message: "disk full".to_string(),
            }],
            skipped: vec![SkippedRepo {
                repo: "repo-broken".to_string(),
                error: "invalid JSON".to_string(),
            }],
            dry_run: false,
        };

        let summary = render_dispose_summary(&report);
        assert!(summary.contains("repo-a: 1 disposed"));
        assert!(summary.contains("repo-b: 0 disposed"));
        assert!(summary.contains("repo-broken: SKIPPED"));
        assert!(summary.contains("invalid JSON"));
        assert!(summary.contains("repo-c: FAILED"));
        assert!(summary.contains("disk full"));
        assert!(summary.contains("git commit -o"));
        assert!(summary.contains("/fake/repo-a/planning/state.json"));
        assert!(summary.contains("/fake/repo-a/planning/carryover-archive.jsonl"));
        // repo-b disposed nothing — its paths must not appear in the pathspec.
        assert!(!summary.contains("/fake/repo-b/planning/state.json"));
    }

    #[test]
    fn render_commit_pathspec_is_none_when_nothing_was_disposed_anywhere() {
        let report = DisposeRunReport {
            writes: vec![RepoDisposalWrite {
                repo: "repo-a".to_string(),
                state_path: PathBuf::from("/fake/repo-a/planning/state.json"),
                archive_path: PathBuf::from("/fake/repo-a/planning/carryover-archive.jsonl"),
                disposed: vec![],
                written: false,
            }],
            failures: vec![],
            skipped: vec![],
            dry_run: false,
        };
        assert_eq!(render_commit_pathspec(&report), None);
    }

    #[test]
    fn dry_run_summary_marks_disposed_repos_as_not_written_but_keeps_the_pathspec() {
        // `--dispose --dry-run` reuses the same rendering: the pathspec
        // preview is identical, only the suffix on a disposing repo differs.
        let report = DisposeRunReport {
            writes: vec![RepoDisposalWrite {
                repo: "repo-a".to_string(),
                state_path: PathBuf::from("/fake/repo-a/planning/state.json"),
                archive_path: PathBuf::from("/fake/repo-a/planning/carryover-archive.jsonl"),
                disposed: vec![candidate(
                    "cleared-one",
                    "block a closed",
                    item(
                        "cleared-one",
                        "deferred",
                        None,
                        vec![],
                        "2020-01-01",
                        None,
                        None,
                    ),
                )],
                written: false,
            }],
            failures: vec![],
            skipped: vec![],
            dry_run: true,
        };

        let summary = render_dispose_summary(&report);
        assert!(summary.contains("repo-a: 1 disposed (dry-run, not written)"));
        assert!(
            render_commit_pathspec(&report).is_some(),
            "dry-run must still preview the commit pathspec"
        );
    }

    #[test]
    fn dispose_run_report_fails_only_on_write_failures_not_on_skipped_repos() {
        let clean_skip_only = DisposeRunReport {
            writes: vec![],
            failures: vec![],
            skipped: vec![SkippedRepo {
                repo: "repo-broken".to_string(),
                error: "invalid JSON".to_string(),
            }],
            dry_run: false,
        };
        assert!(
            clean_skip_only.succeeded(),
            "a skipped repo is reported, not fatal"
        );

        let with_failure = DisposeRunReport {
            writes: vec![],
            failures: vec![RepoDisposalError {
                repo: "repo-c".to_string(),
                message: "disk full".to_string(),
            }],
            skipped: vec![],
            dry_run: false,
        };
        assert!(!with_failure.succeeded());
    }

    // -- notify_subset (`MV.ticket.attention-notify-policy` task 2) ---------

    fn ranking(lane: TriageLane, priority: Option<u8>, slug: &str) -> CarryoverRanking {
        CarryoverRanking {
            repo: "mev".to_string(),
            slug: slug.to_string(),
            kind: "deferred".to_string(),
            lane,
            priority,
            effective_priority: priority,
            age_days: Some(1),
            stale: false,
            unmet_blocks: vec![],
            clears_when_satisfied: false,
            finding_id: None,
        }
    }

    #[test]
    fn notify_subset_includes_blocking_at_p3() {
        let entries = vec![ranking(TriageLane::Blocking, Some(3), "a")];
        let out = notify_subset(&entries, &thresholds());
        assert_eq!(out.len(), 1, "blocking is included regardless of priority");
    }

    #[test]
    fn notify_subset_includes_blocking_with_no_priority() {
        let entries = vec![ranking(TriageLane::Blocking, None, "a")];
        let out = notify_subset(&entries, &thresholds());
        assert_eq!(out.len(), 1, "blocking with no priority is still included");
    }

    #[test]
    fn notify_subset_excludes_blocking_when_flag_off() {
        let mut t = thresholds();
        t.notify_blocking_any_priority = false;
        let entries = vec![ranking(TriageLane::Blocking, Some(0), "a")];
        assert!(notify_subset(&entries, &t).is_empty());
    }

    #[test]
    fn notify_subset_includes_hot_at_p0() {
        let entries = vec![ranking(TriageLane::Hot, Some(0), "a")];
        let out = notify_subset(&entries, &thresholds());
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn notify_subset_excludes_hot_at_p1_by_default() {
        let entries = vec![ranking(TriageLane::Hot, Some(1), "a")];
        assert!(
            notify_subset(&entries, &thresholds()).is_empty(),
            "default floor is 0 (P0 only), so hot P1 must be excluded"
        );
    }

    #[test]
    fn notify_subset_excludes_hot_when_lane_not_in_notify_lanes() {
        let mut t = thresholds();
        t.notify_lanes = vec!["blocking".to_string()];
        let entries = vec![ranking(TriageLane::Hot, Some(0), "a")];
        assert!(notify_subset(&entries, &t).is_empty());
    }

    #[test]
    fn notify_subset_excludes_aging_and_standing() {
        let entries = vec![
            ranking(TriageLane::Aging, Some(2), "a"),
            ranking(TriageLane::Standing, None, "b"),
        ];
        assert!(notify_subset(&entries, &thresholds()).is_empty());
    }

    #[test]
    fn notify_subset_preserves_input_order() {
        let entries = vec![
            ranking(TriageLane::Blocking, Some(3), "z-blocking"),
            ranking(TriageLane::Aging, Some(2), "skip"),
            ranking(TriageLane::Hot, Some(0), "a-hot"),
        ];
        let out = notify_subset(&entries, &thresholds());
        let slugs: Vec<&str> = out.iter().map(|r| r.slug.as_str()).collect();
        assert_eq!(slugs, vec!["z-blocking", "a-hot"]);
    }

    // -- notify_subset (`MV.ticket.attention-notify-policy` task 4) ---------
    //
    // Filter-level fixture tests: the fail-closed default, the two cases
    // that distinguish this from a naive priority floor, and each policy
    // key demonstrably changing the returned set. `digest_everything_else`
    // is intentionally NOT exercised here as a filter-changing key — it is
    // read (see `config.rs::notify_policy_each_key_is_actually_read`) but
    // consumed only by the digest side (bastion:BA.21.D), never by
    // `notify_subset` itself; see the rule doc comment above.

    #[test]
    fn fail_closed_when_attention_table_absent() {
        // `AttentionThresholds::default()` is what an absent `[attention]`
        // table deserializes to (config.rs pins the deserialization side;
        // this pins what the filter DOES with those defaults). Getting this
        // backwards reproduces the 395-item notification burst the ticket
        // exists to prevent.
        let t = AttentionThresholds::default();
        let entries = vec![
            ranking(TriageLane::Blocking, Some(3), "blocking-p3"),
            ranking(TriageLane::Hot, Some(0), "hot-p0"),
            ranking(TriageLane::Hot, Some(1), "hot-p1"),
            ranking(TriageLane::Aging, Some(0), "aging"),
            ranking(TriageLane::Standing, None, "standing"),
        ];
        let out = notify_subset(&entries, &t);
        let slugs: Vec<&str> = out.iter().map(|r| r.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec!["blocking-p3", "hot-p0"],
            "absent [attention] table must yield blocking-any-priority + hot-P0 only, never notify-everything"
        );
    }

    #[test]
    fn fail_closed_when_table_present_but_no_policy_keys() {
        // A present `[attention]` table that only overrides an unrelated
        // staleness field (never a policy key) must still yield the
        // documented default rule.
        let t = AttentionThresholds {
            deferred_days: 2,
            ..Default::default()
        };
        let entries = vec![
            ranking(TriageLane::Blocking, Some(3), "blocking-p3"),
            ranking(TriageLane::Hot, Some(0), "hot-p0"),
            ranking(TriageLane::Hot, Some(1), "hot-p1"),
        ];
        let out = notify_subset(&entries, &t);
        let slugs: Vec<&str> = out.iter().map(|r| r.slug.as_str()).collect();
        assert_eq!(slugs, vec!["blocking-p3", "hot-p0"]);
    }

    #[test]
    fn blocking_at_p3_is_included() {
        let entries = vec![ranking(TriageLane::Blocking, Some(3), "a")];
        let out = notify_subset(&entries, &thresholds());
        assert_eq!(
            out.len(),
            1,
            "blocking never consults priority — P3 is included"
        );
    }

    #[test]
    fn hot_at_p1_is_excluded() {
        let entries = vec![ranking(TriageLane::Hot, Some(1), "a")];
        assert!(
            notify_subset(&entries, &thresholds()).is_empty(),
            "hot P1 is excluded under the default floor of 0 — the case a naive \
             priority<=floor filter over every lane would get wrong the other way"
        );
    }

    #[test]
    fn key_notify_lanes_changes_output() {
        let entries = vec![ranking(TriageLane::Hot, Some(0), "hot-p0")];
        assert_eq!(
            notify_subset(&entries, &thresholds()).len(),
            1,
            "hot is in notify_lanes by default"
        );
        let mut t = thresholds();
        t.notify_lanes = vec!["blocking".to_string()];
        assert!(
            notify_subset(&entries, &t).is_empty(),
            "removing hot from notify_lanes must exclude a hot P0 that was included before"
        );
    }

    #[test]
    fn key_notify_priority_floor_changes_output() {
        let entries = vec![ranking(TriageLane::Hot, Some(1), "hot-p1")];
        assert!(
            notify_subset(&entries, &thresholds()).is_empty(),
            "default floor of 0 excludes hot P1"
        );
        let mut t = thresholds();
        t.notify_priority_floor = 1;
        assert_eq!(
            notify_subset(&entries, &t).len(),
            1,
            "raising the floor to 1 must include a hot P1 that was excluded before"
        );
    }

    #[test]
    fn key_notify_blocking_any_priority_changes_output() {
        let entries = vec![ranking(TriageLane::Blocking, Some(0), "blocking-p0")];
        assert_eq!(
            notify_subset(&entries, &thresholds()).len(),
            1,
            "notify_blocking_any_priority defaults true"
        );
        let mut t = thresholds();
        t.notify_blocking_any_priority = false;
        assert!(
            notify_subset(&entries, &t).is_empty(),
            "flipping notify_blocking_any_priority to false must exclude a blocking \
             item that was included before"
        );
    }

    #[test]
    fn lane_absent_from_notify_lanes_is_excluded() {
        let mut t = thresholds();
        t.notify_lanes = vec![]; // neither lane named
        let entries = vec![
            ranking(TriageLane::Blocking, Some(0), "blocking"),
            ranking(TriageLane::Hot, Some(0), "hot"),
        ];
        let out = notify_subset(&entries, &t);
        // `notify_blocking_any_priority` still governs Blocking independent
        // of notify_lanes (rule 1's doc comment), so only Hot drops out
        // here; this pins that Hot specifically requires lane membership.
        let slugs: Vec<&str> = out.iter().map(|r| r.slug.as_str()).collect();
        assert_eq!(slugs, vec!["blocking"]);
    }

    // -- filter_carryover_entries_by_grep (MV.ticket.carryover-grep, task 1) -

    fn grep_verdict(slug: &str, text: &str) -> CarryoverVerdict {
        CarryoverVerdict {
            repo: "mev".to_string(),
            slug: slug.to_string(),
            kind: "known_issue".to_string(),
            text: text.to_string(),
            clears_when: None,
            created: "2026-01-01".to_string(),
            age_days: Some(1),
            stale: false,
            lane: CarryoverLane::NotEvaluable,
            refs: Vec::new(),
            reason: None,
            priority: None,
            finding_id: None,
            blocks: Vec::new(),
            enforce: None,
            needs: None,
        }
    }

    #[test]
    fn grep_filter_matches_on_slug() {
        let entries = vec![
            grep_verdict("synapse-rename-mechanical-flip-pending", "some text"),
            grep_verdict("unrelated-slug", "other text"),
        ];
        let out = filter_carryover_entries_by_grep(&entries, "synapse-rename")
            .expect("valid pattern must compile");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slug, "synapse-rename-mechanical-flip-pending");
    }

    #[test]
    fn grep_filter_matches_on_text() {
        let entries = vec![
            grep_verdict("alpha", "mentions the synapse rename in prose"),
            grep_verdict("beta", "nothing relevant here"),
        ];
        let out = filter_carryover_entries_by_grep(&entries, "synapse rename")
            .expect("valid pattern must compile");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].slug, "alpha");
    }

    #[test]
    fn grep_filter_is_case_insensitive() {
        let entries = vec![grep_verdict("Synapse-Rename", "MIXED Case Text")];
        let out = filter_carryover_entries_by_grep(&entries, "synapse-rename")
            .expect("valid pattern must compile");
        assert_eq!(out.len(), 1);

        let out = filter_carryover_entries_by_grep(&entries, "mixed case")
            .expect("valid pattern must compile");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn grep_filter_matches_several_entries() {
        let entries = vec![
            grep_verdict("build-fix-a", "flaky build on macOS"),
            grep_verdict("build-fix-b", "flaky build on linux"),
            grep_verdict("unrelated", "nothing to do with it"),
        ];
        let out = filter_carryover_entries_by_grep(&entries, "flaky build")
            .expect("valid pattern must compile");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn grep_filter_matching_nothing_returns_empty_not_error() {
        let entries = vec![grep_verdict("alpha", "some text")];
        let out = filter_carryover_entries_by_grep(&entries, "no-such-pattern-anywhere")
            .expect("a pattern matching nothing is still a valid pattern");
        assert!(out.is_empty());
    }

    #[test]
    fn grep_filter_invalid_regex_returns_err() {
        let entries = vec![grep_verdict("alpha", "some text")];
        let err = filter_carryover_entries_by_grep(&entries, "(unclosed[")
            .expect_err("malformed regex must error, never silently match nothing");
        assert!(!err.to_string().is_empty());
    }

    // -- enumerate_historical_removals (`MV.16.B` task 1) -------------------

    mod backfill_history_walk {
        use super::*;
        use std::fs;
        use std::path::{Path, PathBuf};

        fn temp_dir(tag: &str) -> PathBuf {
            let dir =
                crate::testsupport::unique_temp_dir(&format!("mev-carryover-backfill-unit-{tag}"));
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn run_git(dir: &Path, args: &[&str]) {
            let output = crate::shared::git_command()
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .expect("failed to spawn git");
            assert!(
                output.status.success(),
                "git {:?} failed in {}: {}",
                args,
                dir.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn write_brain_toml(root: &Path, repos: &[&str]) {
            let mut toml = String::from(
                r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

"#,
            );
            for slug in repos {
                toml.push_str(&format!(
                    r#"[[repos]]
slug = "{slug}"
tier = "primary"
repo_path = "repos/{slug}"
status_file = "repos/{slug}/planning/status.md"
cache_doc = "docs/projects/{slug}.md"
heading = "{slug}"

"#
                ));
            }
            fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
        }

        fn write_state(root: &Path, slug: &str, value: &serde_json::Value) {
            let path = root
                .join("repos")
                .join(slug)
                .join("planning")
                .join("state.json");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap();
        }

        fn entry(slug: &str, extra_key: Option<(&str, &str)>) -> serde_json::Value {
            let mut v = serde_json::json!({
                "slug": slug,
                "scope": { "repo": "alpha" },
                "kind": "known_issue",
                "text": format!("entry {slug}"),
                "created": "2026-01-01"
            });
            if let Some((k, val)) = extra_key {
                v.as_object_mut()
                    .unwrap()
                    .insert(k.to_string(), serde_json::json!(val));
            }
            v
        }

        fn state_with_carryover(carryover: Vec<serde_json::Value>) -> serde_json::Value {
            serde_json::json!({
                "repo": "alpha",
                "kind": "project",
                "updated": "2026-01-01",
                "focus": { "now": [], "next": [], "blocked": [] },
                "carryover": carryover
            })
        }

        fn init_repo(root: &Path) {
            run_git(root, &["init", "-q"]);
            run_git(root, &["config", "user.email", "test@example.com"]);
            run_git(root, &["config", "user.name", "Test"]);
        }

        fn commit_all(root: &Path, msg: &str) {
            run_git(root, &["add", "."]);
            run_git(root, &["commit", "-q", "-m", msg]);
        }

        #[test]
        fn removal_commit_yields_one_row_with_verbatim_entry_and_unmodeled_key() {
            let root = temp_dir("removal");
            write_brain_toml(&root, &["alpha"]);
            write_state(
                &root,
                "alpha",
                &state_with_carryover(vec![entry(
                    "alpha-a",
                    Some(("totally_unmodeled_field", "F-1")),
                )]),
            );
            init_repo(&root);
            commit_all(&root, "add alpha-a");

            write_state(&root, "alpha", &state_with_carryover(vec![]));
            commit_all(&root, "clear alpha-a: resolved upstream");

            let plan = enumerate_historical_removals(&root, None).expect("walk should succeed");
            assert!(
                plan.diagnostics.is_empty(),
                "unexpected diagnostics: {:?}",
                plan.diagnostics
            );
            assert_eq!(plan.removals.len(), 1);
            let removal = &plan.removals[0];
            assert_eq!(removal.repo, "alpha");
            assert_eq!(removal.entry.slug, "alpha-a");
            assert_eq!(
                removal
                    .entry
                    .extra
                    .get("totally_unmodeled_field")
                    .and_then(|v| v.as_str()),
                Some("F-1"),
                "an unmodeled key on the entry must survive verbatim into the removal"
            );
            assert_eq!(removal.commit_subject, "clear alpha-a: resolved upstream");
            assert_eq!(
                removal.archive_path,
                archive_path_for(&root.join("repos/alpha/planning/state.json"))
            );
        }

        #[test]
        fn commit_removing_three_entries_yields_three_rows() {
            let root = temp_dir("removal-three");
            write_brain_toml(&root, &["alpha"]);
            write_state(
                &root,
                "alpha",
                &state_with_carryover(vec![
                    entry("alpha-a", None),
                    entry("alpha-b", None),
                    entry("alpha-c", None),
                ]),
            );
            init_repo(&root);
            commit_all(&root, "add three entries");

            write_state(&root, "alpha", &state_with_carryover(vec![]));
            commit_all(&root, "clear all three");

            let plan = enumerate_historical_removals(&root, None).expect("walk should succeed");
            assert_eq!(plan.removals.len(), 3);
            let mut slugs: Vec<&str> = plan
                .removals
                .iter()
                .map(|r| r.entry.slug.as_str())
                .collect();
            slugs.sort_unstable();
            assert_eq!(slugs, vec!["alpha-a", "alpha-b", "alpha-c"]);
        }

        #[test]
        fn commit_that_only_adds_or_only_edits_yields_no_removal() {
            let root = temp_dir("add-edit");
            write_brain_toml(&root, &["alpha"]);
            write_state(
                &root,
                "alpha",
                &state_with_carryover(vec![entry("alpha-a", None)]),
            );
            init_repo(&root);
            commit_all(&root, "add alpha-a");

            // Only ADD alpha-b.
            write_state(
                &root,
                "alpha",
                &state_with_carryover(vec![entry("alpha-a", None), entry("alpha-b", None)]),
            );
            commit_all(&root, "add alpha-b");

            // Only EDIT alpha-a's text.
            let mut edited = entry("alpha-a", None);
            edited["text"] = serde_json::json!("edited text");
            write_state(
                &root,
                "alpha",
                &state_with_carryover(vec![edited, entry("alpha-b", None)]),
            );
            commit_all(&root, "edit alpha-a");

            let plan = enumerate_historical_removals(&root, None).expect("walk should succeed");
            assert!(
                plan.removals.is_empty(),
                "add-only and edit-only commits must yield no removal, got: {:?}",
                plan.removals
            );
        }

        #[test]
        fn root_commit_yields_no_removal() {
            let root = temp_dir("root-commit");
            write_brain_toml(&root, &["alpha"]);
            write_state(
                &root,
                "alpha",
                &state_with_carryover(vec![entry("alpha-a", None)]),
            );
            init_repo(&root);
            commit_all(&root, "initial commit with alpha-a already present");

            let plan = enumerate_historical_removals(&root, None).expect("walk should succeed");
            assert!(
                plan.removals.is_empty(),
                "a root commit has no parent to diff against, so it must yield nothing"
            );
        }

        #[test]
        fn repo_filter_restricts_the_walk_to_one_repo() {
            let root = temp_dir("repo-filter");
            write_brain_toml(&root, &["alpha", "beta"]);
            write_state(
                &root,
                "alpha",
                &state_with_carryover(vec![entry("alpha-a", None)]),
            );
            write_state(
                &root,
                "beta",
                &state_with_carryover(vec![entry("beta-a", None)]),
            );
            init_repo(&root);
            commit_all(&root, "add both");

            write_state(&root, "alpha", &state_with_carryover(vec![]));
            write_state(&root, "beta", &state_with_carryover(vec![]));
            commit_all(&root, "clear both");

            let plan =
                enumerate_historical_removals(&root, Some("alpha")).expect("walk should succeed");
            assert_eq!(plan.removals.len(), 1);
            assert_eq!(plan.removals[0].repo, "alpha");
        }

        #[test]
        fn unknown_repo_filter_errors_naming_valid_slugs() {
            let root = temp_dir("unknown-repo");
            write_brain_toml(&root, &["alpha"]);
            write_state(&root, "alpha", &state_with_carryover(vec![]));
            init_repo(&root);
            commit_all(&root, "initial");

            let err = enumerate_historical_removals(&root, Some("nonexistent"))
                .expect_err("unknown --repo slug must error");
            assert!(
                err.to_string().contains("alpha"),
                "error should name valid slugs: {err}"
            );
        }
    }

    // -------------------------------------------------------------------
    // needs distribution (MV.ticket.carryover-needs-validation, task 2)
    // -------------------------------------------------------------------

    fn needs_verdict(repo: &str, slug: &str, needs: Option<CarryoverNeeds>) -> CarryoverVerdict {
        CarryoverVerdict {
            repo: repo.to_string(),
            slug: slug.to_string(),
            kind: "deferred".to_string(),
            text: "text".to_string(),
            clears_when: None,
            created: "2026-01-01".to_string(),
            age_days: None,
            stale: false,
            lane: CarryoverLane::NotEvaluable,
            refs: Vec::new(),
            reason: None,
            priority: None,
            finding_id: None,
            blocks: Vec::new(),
            enforce: None,
            needs,
        }
    }

    #[test]
    fn needs_distribution_counts_known_unknown_and_absent_separately() {
        let entries = vec![
            needs_verdict(
                "mev",
                "a",
                Some(CarryoverNeeds::Known(KnownCarryoverNeeds::Code)),
            ),
            needs_verdict(
                "mev",
                "b",
                Some(CarryoverNeeds::Known(KnownCarryoverNeeds::Operator)),
            ),
            needs_verdict("mev", "c", Some(CarryoverNeeds::Unknown("bogus".into()))),
            needs_verdict("mev", "d", None),
            needs_verdict(
                "bastiel",
                "e",
                Some(CarryoverNeeds::Known(KnownCarryoverNeeds::Docs)),
            ),
        ];

        let (per_repo, fleet) = compute_needs_distribution(&entries);

        assert_eq!(fleet.code, 1);
        assert_eq!(fleet.operator, 1);
        assert_eq!(fleet.unknown, 1);
        assert_eq!(fleet.absent, 1);
        assert_eq!(fleet.docs, 1);
        assert_eq!(fleet.total(), 5);

        let mev = per_repo.get("mev").expect("mev row present");
        assert_eq!(mev.code, 1);
        assert_eq!(mev.operator, 1);
        assert_eq!(mev.unknown, 1);
        assert_eq!(mev.absent, 1);
        assert_eq!(mev.total(), 4);

        let bastiel = per_repo.get("bastiel").expect("bastiel row present");
        assert_eq!(bastiel.docs, 1);
        assert_eq!(bastiel.total(), 1);
    }

    #[test]
    fn needs_distribution_empty_corpus_yields_zero_counts_everywhere() {
        let (per_repo, fleet) = compute_needs_distribution(&[]);
        assert!(per_repo.is_empty());
        assert_eq!(fleet.total(), 0);
    }

    #[test]
    fn render_needs_distribution_summary_names_every_bucket_including_absent() {
        let entries = vec![
            needs_verdict(
                "mev",
                "a",
                Some(CarryoverNeeds::Known(KnownCarryoverNeeds::State)),
            ),
            needs_verdict("mev", "b", None),
        ];
        let out = render_needs_distribution_summary(&entries);
        assert!(
            out.contains("needs distribution"),
            "summary should be labeled: {out}"
        );
        assert!(out.contains("state=1"), "summary should count state: {out}");
        assert!(
            out.contains("absent=1"),
            "summary should count absent distinctly: {out}"
        );
        assert!(
            out.contains("mev:"),
            "summary should have a per-repo row: {out}"
        );
    }

    #[test]
    fn render_needs_distribution_summary_empty_entries_yields_empty_string() {
        assert_eq!(render_needs_distribution_summary(&[]), "");
    }
}
