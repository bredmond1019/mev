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
    ApprovalDep, BlockDep, BlockedBy, Carryover, ClearsWhen, ClearsWhenPredicate, ExternalDep,
    OperatorDep, StateFile, StateSource,
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
    /// (same two-root strategy as [`Self::Path`]) and its contents contain
    /// `pattern` as a literal substring. Every failure mode — missing file,
    /// unreadable file, non-UTF8 contents, oversized file — is `satisfied:
    /// false`, never a panic.
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
}

/// The full fleet-wide sweep result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CarryoverReport {
    pub total: usize,
    pub cleared: usize,
    pub actionable: usize,
    pub not_evaluable: usize,
    pub entries: Vec<CarryoverVerdict>,
    /// Every `carryover[]` entry sharing an authored `finding_id`, grouped one
    /// cluster per distinct id. See [`cluster_by_finding_id`].
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
/// Attention-section formatters). Currently identical to
/// [`clears_when_prose`] — `Predicate` entries have no display string yet —
/// kept as a separate name so those call sites read as "what do I show a
/// human" rather than "what do I evaluate", and so a future predicate
/// summary (`MV.ticket.clears-when-evaluation`) has one place to land.
pub fn clears_when_display(cw: &ClearsWhen) -> Option<&str> {
    clears_when_prose(cw)
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

/// Whether a path token resolves to an existing file, relative to the brain
/// root or the owning repo's `repo_path` (either is sufficient).
fn path_ref_satisfied(
    path: &str,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    owning_repo: &str,
) -> bool {
    resolve_existing_path(path, brain_root, repo_paths, owning_repo).is_some()
}

/// Resolve a path reference against the same two-root strategy
/// [`path_ref_satisfied`] uses (brain root first, then the owning repo's
/// `repo_path`), returning the first candidate that actually exists.
fn resolve_existing_path(
    path: &str,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    owning_repo: &str,
) -> Option<PathBuf> {
    let brain_candidate = brain_root.join(path);
    if brain_candidate.exists() {
        return Some(brain_candidate);
    }
    repo_paths.get(owning_repo).and_then(|repo_path| {
        let candidate = repo_path.join(path);
        candidate.exists().then_some(candidate)
    })
}

/// Bound on how much of a `file_contains` target we will read into memory —
/// a stray binary or huge path named in a data file must not blow up memory
/// during a fleet sweep. 5 MiB comfortably covers any real doc/source file
/// this predicate is meant to check.
const FILE_CONTAINS_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Whether a `file_contains` predicate is satisfied: the path resolves (same
/// two-root strategy as [`path_ref_satisfied`]), its size is within
/// [`FILE_CONTAINS_MAX_BYTES`], its contents decode as UTF-8, and `pattern`
/// appears as a literal substring (never a regex — see the module header).
/// Every failure mode — missing file, oversized file, unreadable file,
/// non-UTF8 contents, IO error — returns `false`, never panics.
fn file_contains_satisfied(
    path: &str,
    pattern: &str,
    brain_root: &Path,
    repo_paths: &HashMap<String, PathBuf>,
    owning_repo: &str,
) -> bool {
    let Some(resolved) = resolve_existing_path(path, brain_root, repo_paths, owning_repo) else {
        return false;
    };
    let Ok(metadata) = std::fs::metadata(&resolved) else {
        return false;
    };
    if metadata.len() > FILE_CONTAINS_MAX_BYTES {
        return false;
    }
    let Ok(bytes) = std::fs::read(&resolved) else {
        return false;
    };
    let Ok(contents) = String::from_utf8(bytes) else {
        return false;
    };
    contents.contains(pattern)
}

/// Wall-clock bound for a `command_exits_zero` child process. `timeout(1)`
/// does not exist on this macOS shell, so the bound is enforced in-process
/// by polling `try_wait` and killing the child on expiry — never by
/// shelling out to `timeout`.
const COMMAND_EXEC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Poll interval for the in-process watchdog.
const COMMAND_EXEC_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Whether a `command_exits_zero` predicate is satisfied: spawns `sh -c
/// <command>` in `cwd`, and returns `true` only on a clean exit status of 0
/// observed within [`COMMAND_EXEC_TIMEOUT`]. Spawn failure, non-zero exit,
/// signal death, and timeout (the child is killed and reaped) all return
/// `false` — never `true`, and never a panic that aborts the sweep. Only
/// called when the caller has already confirmed `allow_exec` is set.
fn command_exit_zero_satisfied(command: &str, cwd: &Path) -> bool {
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
        Err(_) => return false,
    };

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if start.elapsed() >= COMMAND_EXEC_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(COMMAND_EXEC_POLL_INTERVAL);
            }
            Err(_) => return false,
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
/// prose extraction.
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
) -> CarryoverReport {
    let known_keys: HashSet<String> = status_map.keys().cloned().collect();
    let today_date = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d").ok();

    let mut entries: Vec<CarryoverVerdict> = Vec::new();

    for (src, file) in files {
        if let Some(filter) = repo_filter
            && src.repo_slug != filter
        {
            continue;
        }

        for item in &file.carryover {
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
                    // Reuse `path_ref_satisfied` verbatim — no second
                    // resolution strategy for the typed form.
                    let satisfied = path_ref_satisfied(path, brain_root, repo_paths, own_repo);
                    refs.push(CarryoverRef::Path {
                        path: path.clone(),
                        satisfied,
                    });
                }
                Some(ClearsWhen::Predicate(ClearsWhenPredicate::FileContains {
                    path,
                    pattern,
                    ..
                })) => {
                    // Same two-root resolution strategy as `FileExists`;
                    // every failure mode (missing/oversized/unreadable/
                    // non-UTF8) folds into `satisfied: false`.
                    let satisfied =
                        file_contains_satisfied(path, pattern, brain_root, repo_paths, own_repo);
                    refs.push(CarryoverRef::FileContains {
                        path: path.clone(),
                        pattern: pattern.clone(),
                        satisfied,
                    });
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
                        let satisfied = command_exit_zero_satisfied(command, cwd);
                        refs.push(CarryoverRef::CommandExitsZero {
                            command: command.clone(),
                            satisfied,
                        });
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
                clears_when: item
                    .clears_when
                    .as_ref()
                    .and_then(clears_when_display)
                    .map(String::from),
                created: item.created.clone(),
                age_days,
                stale,
                lane,
                refs,
                reason,
                priority: item.priority,
                finding_id: item.finding_id.clone(),
                blocks: item.blocks.clone(),
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

    CarryoverReport {
        total,
        cleared,
        actionable,
        not_evaluable,
        entries,
        clusters,
        suggestions,
        single_repo_finding_ids,
    }
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
/// 3. Renders the evidence string via [`describe_clearing_evidence`].
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
            evidence: describe_clearing_evidence(verdict),
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
pub fn describe_clearing_evidence(verdict: &CarryoverVerdict) -> String {
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
                format!("command `{command}` exited 0")
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
///   (`path_ref_satisfied`/`file_contains_satisfied`) resolves it the same
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
/// Composed entirely from the same `files` slice [`evaluate_carryover`] was given and
/// the [`CarryoverReport`] it already produced — no new filesystem read, no new corpus
/// walk. `reference[]` entries are never evaluated by `evaluate_carryover` (D72 — they
/// are permanently-true material with no clock and no lane), so their counts are
/// gathered here directly from `files` instead.
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
        if let Some(filter) = repo_filter
            && src.repo_slug != filter
        {
            continue;
        }
        for item in &file.carryover {
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
    }
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

/// Unmet `blocks[]` keys for one carryover entry.
///
/// Mirrors `has_unmet_dep`'s predicate shape verbatim
/// ([`crate::brain::emit`], private, near line 603) so ranking and the
/// unified board's `depends_on` predicate can never drift apart: a
/// [`BlockedBy::External`] edge is always unmet (there is no node target to
/// resolve), and a [`BlockedBy::Block`] edge is unmet unless its target's
/// authored status in `block_status` is exactly `"closed"` — an
/// unresolvable target (absent from `block_status` entirely) counts as
/// unmet too. An empty `repo` on a `Block` edge falls back to the entry's
/// own `repo`, mirroring [`block_refs_from_related`]'s fallback.
///
/// `External` edges are keyed `"external:{what}"` (matching the display
/// convention already used for `depends_on` at
/// `crate::brain::emit::render_wave_table`) so every returned string is a
/// stable, human-readable identifier — never an empty string.
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
            BlockedBy::Block(BlockDep { repo, id, .. }) => {
                let target_repo = if repo.is_empty() {
                    entry.repo.as_str()
                } else {
                    repo.as_str()
                };
                let key = format!("{target_repo}:{id}");
                let closed = block_status.get(&key).and_then(|s| s.as_deref()) == Some("closed");
                if closed { None } else { Some(key) }
            }
        })
        .collect()
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
        );
        assert_eq!(report.total, 1);
        assert_eq!(report.entries[0].repo, "mev");
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
    fn file_contains_predicate_absent_file_is_actionable_never_panics() {
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
        );
        assert_eq!(report.actionable, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::FileContains {
                path: "definitely/does/not/exist.md".to_string(),
                pattern: "anything".to_string(),
                satisfied: false,
            }]
        );
    }

    #[test]
    fn file_contains_predicate_oversized_file_is_actionable_never_panics() {
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
                        pattern: "x".to_string(),
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
        );
        assert_eq!(report.actionable, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::FileContains {
                path: "huge.md".to_string(),
                pattern: "x".to_string(),
                satisfied: false,
            }],
            "an oversized file must never be read into memory to satisfy the predicate"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_contains_predicate_non_utf8_file_is_actionable_never_panics() {
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
        );
        assert_eq!(report.actionable, 1);
        assert!(!matches!(report.entries[0].lane, CarryoverLane::Cleared));

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
        );
        assert_eq!(report.actionable, 1);
        assert!(!matches!(report.entries[0].lane, CarryoverLane::Cleared));
    }

    #[test]
    fn command_exits_zero_with_opt_in_and_slow_command_times_out_and_is_actionable() {
        // Exceeds COMMAND_EXEC_TIMEOUT; the in-process watchdog must kill it
        // and report not-satisfied within roughly the bound rather than
        // hanging the sweep. `timeout(1)` is never invoked to enforce this.
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
        );
        let elapsed = start.elapsed();

        assert_eq!(report.actionable, 1);
        assert_eq!(
            report.entries[0].refs,
            vec![CarryoverRef::CommandExitsZero {
                command: "sleep 30".to_string(),
                satisfied: false,
            }]
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
        }
    }

    fn block_edge(repo: &str, id: &str) -> BlockedBy {
        BlockedBy::Block(BlockDep {
            repo: repo.to_string(),
            id: id.to_string(),
            what: None,
        })
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
        );
        assert_eq!(report.cleared, 1);
        assert_eq!(report.actionable, 1);

        let plan = compute_disposal_plan(&report, &files, &[]);
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
        );

        let load_errors = vec![(
            "repo-broken".to_string(),
            "invalid type: string, expected a boolean at line 4 column 12".to_string(),
        )];
        let plan = compute_disposal_plan(&report, &files, &load_errors);

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
        );
        assert_eq!(report.cleared, 0);
        assert_eq!(
            report.entries[0].reason,
            Some(NotEvaluableReason::ExecutionNotAllowed)
        );

        let plan = compute_disposal_plan(&report, &files, &[]);
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
        };
        assert_eq!(
            describe_clearing_evidence(&verdict),
            "block repo-a:MV.1.A closed; path docs/x.md exists"
        );
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
        let result = write_graph_findings_for_repo(&source, &file, &[f.clone()], "2026-08-23")
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
}
