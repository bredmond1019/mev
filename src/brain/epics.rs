//! Epic-level parking — the authored counterpart to a block's `deferred` status.
//!
//! An epic has its own lifecycle in the HQ `epics[]` registry (`active` ·
//! `focused` · `paused` · `complete`, where `focused` is an active-equivalent
//! refinement meaning "current priority"), and its member blocks have their own authored
//! statuses. Those two can drift: an epic marked `paused` whose blocks are still
//! `open` keeps flooding `focus.next` even though the initiative is parked, and
//! an epic whose work is entirely `deferred` still reads `active` on every board.
//!
//! This module keeps them consistent, in both directions:
//! - [`plan_defer_epic`] — park an initiative: epic → `paused`, its open blocks
//!   → `deferred`.
//! - [`plan_resume_epic`] — the inverse: epic → `active`, its deferred blocks
//!   → `open`.
//! - [`plan_sync_epics`] — reconcile the whole registry without naming a slug:
//!   every fully-deferred epic is paused, and every paused epic's stragglers are
//!   deferred.
//!
//! # Why this is a command and not part of `emit-state`
//!
//! `status` on a block and on an epic are **authored** fields — human intent.
//! `emit-state` is the *derived*-state writer: it regenerates `focus`, `repos[]`,
//! boards and tables from what you authored, and is safe to run on a timer
//! precisely because it never changes what you said. If pausing an epic silently
//! rewrote its blocks during a routine emit, a machine run would be editing your
//! intent, and the authored/derived boundary the validator enforces everywhere
//! else (`E_STATE_AUTHORED_BLOCKED`) would stop meaning anything.
//!
//! So the cascade lives here, behind an explicit command you invoke, with the
//! same dry-run-by-default / `--write` contract as `emit-state`.
//!
//! # In-progress work is never parked silently
//!
//! A member with `status == "in_progress"` is left alone and reported. Parking an
//! initiative you are actively mid-block on is much more likely to be a mistake
//! than an intent; finish it or defer it by hand.

use okf_core::{Epic, StateFile, TrackBlock};

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::brain::emit::{
    EPIC_STATUS_ACTIVE, EPIC_STATUS_FOCUSED, EPIC_STATUS_PAUSED, EmitAction, EmitPlan,
    epic_progress,
};
use crate::brain::state::{StateSource, TierScope, tier_scope_for};

/// Authored block status meaning "parked on the back burner".
const BLOCK_STATUS_DEFERRED: &str = "deferred";
/// Authored block status meaning "not started, and a candidate for next".
const BLOCK_STATUS_OPEN: &str = "open";
/// Authored block status meaning "actively being worked".
const BLOCK_STATUS_IN_PROGRESS: &str = "in_progress";
/// Authored block status meaning "done".
const BLOCK_STATUS_CLOSED: &str = "closed";

/// Which direction a reconcile is being planned in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpicAction {
    /// Park the initiative: epic → `paused`, open members → `deferred`.
    Defer,
    /// Un-park the initiative: epic → `active`, deferred members → `open`.
    Resume,
}

/// Locate the HQ registry file — the single `kind == "brain"` file whose tier
/// scope is `All`. Returns its index into `files`, or `None` when no HQ file is
/// loaded (e.g. a single-repo corpus).
fn hq_index(config: &BrainConfig, files: &[(StateSource, StateFile)]) -> Option<usize> {
    files
        .iter()
        .position(|(_, f)| f.kind == "brain" && matches!(tier_scope_for(f, config), TierScope::All))
}

/// Every `(file_index, block_index_path)` whose block declares membership in `slug`.
///
/// Returns `(file_idx, track_idx, block_idx)` triples so callers can mutate in
/// place without re-resolving. Membership is read off the authored
/// `tracks[].blocks[].epics[]`, which is the source of truth (the registry's own
/// `repos[]` is only a reader hint).
fn member_positions(files: &[(StateSource, StateFile)], slug: &str) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (fi, (_, file)) in files.iter().enumerate() {
        for (ti, track) in file.tracks.iter().enumerate() {
            for (bi, block) in track.blocks.iter().enumerate() {
                if block.epics.iter().any(|e| e == slug) {
                    out.push((fi, ti, bi));
                }
            }
        }
    }
    out
}

/// Borrow every member `TrackBlock` of `slug`, tagged with its owning repo — the
/// shape [`epic_progress`] expects.
fn members_of<'a>(
    files: &'a [(StateSource, StateFile)],
    slug: &str,
) -> Vec<(String, &'a TrackBlock)> {
    member_positions(files, slug)
        .into_iter()
        .map(|(fi, ti, bi)| {
            (
                files[fi].0.repo_slug.clone(),
                &files[fi].1.tracks[ti].blocks[bi],
            )
        })
        .collect()
}

/// Serialize a mutated [`StateFile`] into an [`EmitAction`], or `None` when the
/// re-serialized bytes match what is already on disk.
///
/// Uses `to_string_pretty` + a trailing newline, byte-for-byte the same shape
/// `plan_state_json` writes — the corpus is already normalized to it, so an
/// unchanged file produces no action and no churn.
fn action_for(src: &StateSource, file: &StateFile, note: String) -> Option<EmitAction> {
    let mut new_content = serde_json::to_string_pretty(file).ok()?;
    new_content.push('\n');

    let current = std::fs::read_to_string(&src.abs_path).unwrap_or_default();
    if current == new_content {
        return None;
    }

    Some(EmitAction {
        path: src.abs_path.clone(),
        new_content,
        note,
    })
}

/// Set `epic.status`, returning whether it changed.
fn set_epic_status(epic: &mut Epic, want: &str) -> bool {
    if epic.status.as_deref().unwrap_or(EPIC_STATUS_ACTIVE) == want {
        return false;
    }
    epic.status = Some(want.to_string());
    true
}

/// Plan one epic's cascade in `action`'s direction.
///
/// Mutates a working copy of `files`, so the caller's slice is untouched. Returns
/// the plan plus a diagnostic per skipped `in_progress` member.
fn plan_epic_cascade(
    slug: &str,
    action: EpicAction,
    config: &BrainConfig,
    files: &[(StateSource, StateFile)],
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    let Some(hq) = hq_index(config, files) else {
        plan.diagnostics.push(Diagnostic::error(
            std::path::Path::new("."),
            "E_EPIC_NO_REGISTRY",
            "no HQ brain file with an epics[] registry is loaded; cannot resolve epics".to_string(),
        ));
        return plan;
    };

    if !files[hq].1.epics.iter().any(|e| e.slug == slug) {
        plan.diagnostics.push(Diagnostic::error(
            &files[hq].0.abs_path,
            "E_EPIC_UNKNOWN",
            format!("epic '{slug}' is not in the HQ epics[] registry"),
        ));
        return plan;
    }

    // Work on a copy so a dry run cannot mutate the caller's corpus.
    // `touched` maps file index → how many of ITS blocks changed, so each
    // action's note reports that file's own count rather than a global tally.
    let mut work: Vec<(StateSource, StateFile)> = files.to_vec();
    let mut touched: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();

    let (want_epic, from, to) = match action {
        EpicAction::Defer => (EPIC_STATUS_PAUSED, BLOCK_STATUS_OPEN, BLOCK_STATUS_DEFERRED),
        EpicAction::Resume => (EPIC_STATUS_ACTIVE, BLOCK_STATUS_DEFERRED, BLOCK_STATUS_OPEN),
    };

    // 1. The registry entry itself.
    let mut registry_changed = false;
    if let Some(epic) = work[hq].1.epics.iter_mut().find(|e| e.slug == slug)
        && set_epic_status(epic, want_epic)
    {
        registry_changed = true;
        touched.entry(hq).or_insert(0);
    }

    // 2. Its member blocks.
    for (fi, ti, bi) in member_positions(files, slug) {
        let status = work[fi].1.tracks[ti].blocks[bi]
            .status
            .as_deref()
            .unwrap_or(BLOCK_STATUS_OPEN)
            .to_string();

        if status == BLOCK_STATUS_CLOSED {
            continue; // done is done — never reopened or parked
        }
        if status == BLOCK_STATUS_IN_PROGRESS {
            let (src, file) = &work[fi];
            plan.diagnostics.push(Diagnostic::warning(
                &src.abs_path,
                "W_EPIC_SKIPPED_IN_PROGRESS",
                format!(
                    "block '{}:{}' is in_progress and was left untouched — finish it or change \
                     its status by hand before parking epic '{slug}'",
                    src.repo_slug, file.tracks[ti].blocks[bi].id
                ),
            ));
            continue;
        }
        if status == from {
            work[fi].1.tracks[ti].blocks[bi].status = Some(to.to_string());
            *touched.entry(fi).or_insert(0) += 1;
        }
    }

    // 3. One action per touched file.
    let verb = match action {
        EpicAction::Defer => "defer",
        EpicAction::Resume => "resume",
    };
    for (fi, count) in touched {
        let mut parts: Vec<String> = Vec::new();
        if fi == hq && registry_changed {
            parts.push(format!("registry status → {want_epic}"));
        }
        if count > 0 {
            parts.push(format!("{count} block(s) → {to}"));
        }
        let note = format!("{verb} epic '{slug}' ({})", parts.join(", "));
        if let Some(a) = action_for(&work[fi].0, &work[fi].1, note) {
            plan.actions.push(a);
        }
    }

    plan
}

/// Park an initiative: epic → `paused`, its `open` members → `deferred`.
pub fn plan_defer_epic(
    slug: &str,
    config: &BrainConfig,
    files: &[(StateSource, StateFile)],
) -> EmitPlan {
    plan_epic_cascade(slug, EpicAction::Defer, config, files)
}

/// Un-park an initiative: epic → `active`, its `deferred` members → `open`.
pub fn plan_resume_epic(
    slug: &str,
    config: &BrainConfig,
    files: &[(StateSource, StateFile)],
) -> EmitPlan {
    plan_epic_cascade(slug, EpicAction::Resume, config, files)
}

/// Reconcile every epic in the registry, in both directions.
///
/// - An epic whose remaining work is entirely deferred
///   ([`crate::brain::emit::EpicProgress::is_fully_deferred`]) but which is still
///   live — `active` **or** `focused` — → set `paused`.
/// - An epic already `paused` that still has `open` members → defer them.
///
/// Deliberately **not** symmetric about resuming: an `active` epic with some
/// deferred blocks is a perfectly normal state (you parked two of nine blocks),
/// so nothing is ever un-deferred automatically. Un-parking is always explicit,
/// via [`plan_resume_epic`].
pub fn plan_sync_epics(config: &BrainConfig, files: &[(StateSource, StateFile)]) -> EmitPlan {
    let mut plan = EmitPlan::default();

    let Some(hq) = hq_index(config, files) else {
        return plan;
    };

    let slugs: Vec<(String, Option<String>)> = files[hq]
        .1
        .epics
        .iter()
        .map(|e| (e.slug.clone(), e.status.clone()))
        .collect();

    for (slug, status) in slugs {
        let status = status.unwrap_or_else(|| EPIC_STATUS_ACTIVE.to_string());
        let progress = epic_progress(&members_of(files, &slug));

        // `focused` is active-equivalent here on purpose: a focused epic whose
        // remaining work is entirely deferred must pause exactly as an active one
        // does, or `focused` becomes a hole in the reconciler.
        let is_live = status == EPIC_STATUS_ACTIVE || status == EPIC_STATUS_FOCUSED;
        let needs_pause = is_live && progress.is_fully_deferred();
        let needs_straggler_defer = status == EPIC_STATUS_PAUSED && progress.open > 0;

        if needs_pause || needs_straggler_defer {
            let sub = plan_epic_cascade(&slug, EpicAction::Defer, config, files);
            plan.actions.extend(sub.actions);
            plan.diagnostics.extend(sub.diagnostics);
        }
    }

    plan
}
