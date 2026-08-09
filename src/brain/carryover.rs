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
//! **Class B — path existence.** Only when `clears_when` contains the literal word
//! `exists` ([`path_refs_from_prose`]).
//!
//! No `regex` dependency is used or added — the grammar is small and fixed, so it is
//! matched by hand (char scanning) in [`extract_block_id_tokens`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use okf_core::{BlockedBy, Carryover, ClearsWhen, ClearsWhenPredicate, StateFile, StateSource};

use crate::brain::config::AttentionThresholds;
use crate::brain::state::{carryover_stale_age, is_snoozed, staleness_anchor};

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
}

/// The full fleet-wide sweep result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CarryoverReport {
    pub total: usize,
    pub cleared: usize,
    pub actionable: usize,
    pub not_evaluable: usize,
    pub entries: Vec<CarryoverVerdict>,
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
            BlockedBy::Block { repo, id, .. } => {
                let repo = if repo.is_empty() {
                    item.scope.repo.clone()?
                } else {
                    repo.clone()
                };
                Some(format!("{repo}:{id}"))
            }
            BlockedBy::External { .. } => None,
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

/// Path tokens matched in `clears_when` prose — only when the text contains the
/// literal word `exists` (case-insensitive, word-bounded). Every
/// whitespace-delimited token containing `/` and ending in one of
/// [`PATH_EXTENSIONS`] is returned, trimmed of surrounding punctuation/quotes.
/// Returns `[]` when the word `exists` is absent.
pub fn path_refs_from_prose(clears_when: &str) -> Vec<String> {
    let has_exists = clears_when
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.eq_ignore_ascii_case("exists"));
    if !has_exists {
        return Vec::new();
    }

    clears_when
        .split_whitespace()
        .filter_map(|tok| {
            let trimmed = tok.trim_matches(|c: char| !c.is_alphanumeric());
            if !trimmed.contains('/') {
                return None;
            }
            let ext = trimmed.rsplit('.').next()?;
            if PATH_EXTENSIONS.contains(&ext) {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect()
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

                    // Class B: path existence, only when "exists" appears.
                    for path in path_refs_from_prose(clears_when) {
                        let satisfied = path_ref_satisfied(
                            &path,
                            brain_root,
                            repo_paths,
                            src.repo_slug.as_str(),
                        );
                        refs.push(CarryoverRef::Path { path, satisfied });
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
                kind: item.kind.clone(),
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

    CarryoverReport {
        total,
        cleared,
        actionable,
        not_evaluable,
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            kind: "deferred".to_string(),
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
            vec![BlockedBy::Block {
                repo: "engine-rs".to_string(),
                id: "EN.5.B1".to_string(),
                what: None,
            }],
            None,
        );
        assert_eq!(block_refs_from_related(&item), vec!["engine-rs:EN.5.B1"]);
    }

    #[test]
    fn block_refs_from_related_skips_external_edges() {
        let item = carryover(
            vec![BlockedBy::External {
                what: "waiting on vendor API".to_string(),
            }],
            None,
        );
        assert!(block_refs_from_related(&item).is_empty());
    }

    #[test]
    fn block_refs_from_related_falls_back_to_own_scope_repo() {
        let item = carryover(
            vec![BlockedBy::Block {
                repo: String::new(),
                id: "MV.3.A".to_string(),
                what: None,
            }],
            Some("mev"),
        );
        assert_eq!(block_refs_from_related(&item), vec!["mev:MV.3.A"]);
    }

    #[test]
    fn block_refs_from_related_skips_edge_with_no_repo_anywhere() {
        let item = carryover(
            vec![BlockedBy::Block {
                repo: String::new(),
                id: "MV.3.A".to_string(),
                what: None,
            }],
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
                "docs/decisions/D58-us-market-entry-and-two-domain-split.md",
                "docs/decisions/index.md",
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
        assert_eq!(refs, vec!["docs/index.md", "planning/status.md"]);
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
            kind: kind.to_string(),
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
                        vec![BlockedBy::Block {
                            repo: "bastion".to_string(),
                            id: "BE.2.A".to_string(),
                            what: None,
                        }],
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
}
