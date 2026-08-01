//! `state.json` validation/derivation engine for `mev validate-brain --state`.
//!
//! Phase 3, Block P: schema validation of each repo's `planning/state.json` and the
//! cross-repo block-dependency graph integrity check.
//!
//! The serde schema, [`load_state`] loader, and [`StateGraph`]/[`build_state_graph`]
//! model are re-exported from bastion's `okf-core` crate (BA.15.12 / D15/D16 format
//! convergence) — this module no longer defines its own copies. What stays here is
//! the mev-specific logic that consumes those shared types: discovery
//! ([`discover_state_files`]), schema-ring checks ([`check_schema`]), graph
//! integrity checks ([`check_state_graph`] and friends), and the focus/rollup
//! derivation engine (`derive_*`).
//!
//! Diagnostic locator codes emitted by later tasks that build on this foundation:
//! - `E_STATE_MALFORMED_JSON` — file is not parseable JSON.
//! - `E_STATE_ROOT_LOAD_FAILED` — the HQ root `state.json` exists but failed to
//!   load/parse, so tier sub-brain classification falls back to `brain.toml`
//!   (instead of the root's `tiers[]`) and may be incomplete. Emitted once,
//!   alongside the root's own (detailed) `E_STATE_MALFORMED_JSON`, instead of
//!   letting every tier sub-brain cascade into a spurious `E_STATE_SCHEMA_BAD_KIND`.
//! - `E_STATE_SCHEMA_MISSING_FIELD` — a required key is absent.
//! - `E_STATE_SCHEMA_BAD_KIND` — `kind` ∉ `{project, brain, portfolio}`.
//! - `E_STATE_SCHEMA_BAD_STATUS` — a `status` value ∉ the enum.
//! - `E_STATE_SCHEMA_BAD_BLOCKED_BY` — a `blocked_by[]` entry has an unknown `type`.
//! - `E_STATE_DUPLICATE_BLOCK_ID` — two `tracks[]` blocks in one repo share an `id`.
//! - `E_STATE_DANGLING_FOCUS` — a focus entry's `block` is absent from `tracks[]`.
//! - `E_STATE_DANGLING_BLOCKED_BY` — a cross-repo dependency block doesn't exist.
//! - `E_STATE_UNKNOWN_REPO` — a `blocked_by` / `cross_repo` entry names unknown repo.
//! - `E_STATE_DANGLING_CROSS_REPO` — a brain `cross_repo[]` endpoint doesn't resolve.
//! - `W_STATE_ROLLUP_DRIFT` — brain `repos[]` headline drifted from child `focus`.
//! - `W_STATE_FOCUS_DRIFT` — a leaf file's `focus` snapshot has drifted from the tracks[] derivation.
//! - `W_STATE_FILE_MISSING` — a registered repo has no `planning/state.json`.
//! - `E_STATE_PRIORITY_RANGE` — a `priority` value is not in 0..=3.
//! - `E_STATE_DUE_FORMAT` — a `due` value is not a valid YYYY-MM-DD date.
//! - `E_STATE_SDLC_WORKFLOW_ENUM` — an `sdlc_workflow` value ∉ {none,patch,task,run,flow}.
//! - `E_STATE_MODEL_ENUM` — a `model` value ∉ {sonnet,gemini-pro,gemini-flash,either}.
//! - `E_STATE_DATE_FORMAT` — a carryover/backlog `created`/`reviewed`/`snoozed_until` value
//!   is not a valid `YYYY-MM-DD` (or RFC3339) date.
//! - `W_STATE_CARRYOVER_STALE` — a `carryover[]` entry has aged past its per-kind threshold.
//! - `W_STATE_BACKLOG_STALE` — an HQ `backlog[]` `idea`/`ready` node has aged past threshold.
//! - `E_STATE_DUPLICATE_EPIC_SLUG` — two HQ `epics[]` entries share a `slug`.
//! - `E_STATE_EPIC_BAD_STATUS` — an epic `status` ∉ `{active, paused, complete}`.
//! - `E_STATE_EPIC_BAD_WEIGHT` — an epic `weight` is outside `0..=100`.
//! - `E_STATE_UNKNOWN_EPIC` — a block's `epics[]` entry is not in the HQ registry.
//! - `W_STATE_EPIC_REGISTRY_IGNORED` — a non-HQ file declares its own `epics[]`.
//! - `W_STATE_EPIC_EMPTY` — a registered epic has no member blocks.
//! - `W_STATE_EPIC_UNREACHABLE_DEP` — an unclosed epic block depends on an unclosed
//!   block that belongs to no epic (a gate invisible on the epic's board).

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use crate::Diagnostic;
use crate::brain::config::BrainConfig;

// ---------------------------------------------------------------------------
// Schema model, loader, and state graph model — delegated to okf-core
// (BA.15.12 convergence, D15/D16)
// ---------------------------------------------------------------------------

/// The `state.json` serde schema, loader, and block-dependency graph model.
///
/// Single source of truth: `okf_core::state` (BA.15.12/D15/D16). This module's
/// own validation/derivation logic (`check_*`/`derive_*`, `discover_state_files`,
/// `build_graph`/`check_graph` helpers below) depends on this crate's
/// `BrainConfig`/`Diagnostic` types and stays here — it consumes these shared
/// types instead of duplicating them.
pub use okf_core::{
    Backlog, BacklogOrigin, Block, BlockedBy, Carryover, CarryoverScope, CrossRepoEdge, Endpoint,
    Epic, Focus, Origin, RepoRollup, StateEdge, StateEdgeKind, StateFile, StateGraph,
    StateLoadError, StateNode, StateSource, TierEntry, Track, TrackBlock, build_state_graph,
    load_state,
};

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Discover all `planning/state.json` files reachable from `root`.
///
/// Returns `(sources, diagnostics)`:
/// - `sources` — every file that exists, ready to be loaded.
/// - `diagnostics` — one [`Diagnostic`] with locator `W_STATE_FILE_MISSING`
///   per registered path that does not exist on disk (warning severity); plus,
///   if the HQ root exists but fails to load, one `E_STATE_ROOT_LOAD_FAILED`
///   (see point 2 below).
///
/// Discovery strategy (per scoping decision 1 — cross-repo read mode):
/// 1. HQ brain: `root/planning/state.json` (always expected; `kind:"brain"`).
///    If found, the file is loaded internally to enumerate `tiers[]` so that
///    tier sub-brain paths (`tiers[].rollup`) can be discovered.
/// 2. Tier sub-brains: each `tiers[].rollup` path (relative to `root`) that is
///    non-null is expected as a brain-kind file. If the HQ root body failed to
///    load (a parse error, distinct from the file simply not existing), the
///    `tiers[]` list is unavailable — instead of silently letting every tier
///    sub-brain fall through to the leaf loop below (step 3) and be
///    mis-registered `kind:"project"` (a cascade of spurious
///    `E_STATE_SCHEMA_BAD_KIND`), tier sub-brain paths are recovered from
///    `brain.toml`'s tier-container self-entries (a `[[repos]]` entry whose
///    `slug` equals its own `repo_path`) and registered `expected_kind:"brain"`
///    directly, alongside one `E_STATE_ROOT_LOAD_FAILED` diagnostic noting the
///    classification is degraded.
/// 3. Leaf repos: each `[[repos]]` entry in `config` whose `repo_path` is not
///    `"."` (the HQ root itself) → `root/{repo_path}/planning/state.json`
///    (`kind:"project"`, or `kind:"portfolio"` when `tier == "portfolio"` —
///    terminal repos published to GitHub with no further planning state).
pub fn discover_state_files(
    root: &Path,
    config: &BrainConfig,
) -> (Vec<StateSource>, Vec<Diagnostic>) {
    let mut sources: Vec<StateSource> = Vec::new();
    let mut diags: Vec<Diagnostic> = Vec::new();

    // --- 1. HQ brain state ---
    let hq_path = root.join("planning").join("state.json");
    if hq_path.exists() {
        // Derive the HQ slug from the [[repos]] entry with repo_path="." if
        // present; fall back to "hq".
        let hq_slug = config
            .repos
            .iter()
            .find(|r| r.repo_path == "." || r.repo_path.is_empty())
            .map(|r| r.slug.clone())
            .unwrap_or_else(|| "hq".to_string());

        sources.push(StateSource {
            repo_slug: hq_slug,
            abs_path: hq_path.clone(),
            expected_kind: "brain",
        });

        // --- 2. Tier sub-brains (from HQ tiers[].rollup) ---
        if let Ok(hq_state) = load_state(&hq_path) {
            for tier_entry in &hq_state.tiers {
                let rollup = match &tier_entry.rollup {
                    Some(r) if !r.trim().is_empty() => r.clone(),
                    _ => continue, // null or empty rollup → no state file expected
                };
                let tier_path = root.join(&rollup);
                if tier_path.exists() {
                    sources.push(StateSource {
                        repo_slug: tier_entry.tier.clone(),
                        abs_path: tier_path,
                        expected_kind: "brain",
                    });
                } else {
                    diags.push(Diagnostic::warning(
                        tier_path,
                        "W_STATE_FILE_MISSING",
                        format!(
                            "tier '{}': state.json not found at rollup path '{rollup}'",
                            tier_entry.tier
                        ),
                    ));
                }
            }
        } else {
            // Facet 2(a) (state-load-error-surfacing): the HQ root exists but
            // failed to parse/deserialize, so `tiers[]` is unavailable and the
            // loop above never ran. Without this arm, every tier sub-brain
            // would fall through to the leaf `[[repos]]` loop below and be
            // mis-registered `expected_kind: "project"`, firing a spurious
            // `E_STATE_SCHEMA_BAD_KIND` per tier — masking the real, single
            // cause (the root's own detailed `E_STATE_MALFORMED_JSON`, see
            // `src/lib.rs`).
            //
            // Recover tier sub-brain paths from `brain.toml` instead of the
            // (unavailable) `tiers[]`: a tier-container self-entry is a
            // `[[repos]]` entry whose `slug` equals its own `repo_path`
            // (mirrors the "tier container" definition in `tier_scope_for`,
            // e.g. `slug = "core"`, `repo_path = "core"`). Register each such
            // path directly as `expected_kind: "brain"` here so the leaf loop's
            // existing "already discovered" skip-guard keeps it out of the
            // `project` fallback.
            diags.push(Diagnostic::warning(
                hq_path.clone(),
                "E_STATE_ROOT_LOAD_FAILED",
                "HQ root planning/state.json could not be loaded; tier sub-brain classification is degraded (derived from brain.toml instead of tiers[]) and may be incomplete".to_string(),
            ));

            for repo in &config.repos {
                if repo.repo_path == "." || repo.repo_path.is_empty() {
                    continue;
                }
                if repo.slug != repo.repo_path {
                    continue; // not a tier-container self-entry
                }
                let tier_path = root
                    .join(&repo.repo_path)
                    .join("planning")
                    .join("state.json");
                if tier_path.exists() {
                    sources.push(StateSource {
                        repo_slug: repo.slug.clone(),
                        abs_path: tier_path,
                        expected_kind: "brain",
                    });
                } else {
                    diags.push(Diagnostic::warning(
                        tier_path,
                        "W_STATE_FILE_MISSING",
                        format!(
                            "tier '{}': state.json not found (derived from brain.toml; root failed to load)",
                            repo.slug
                        ),
                    ));
                }
            }
        }
    } else {
        diags.push(Diagnostic::warning(
            hq_path,
            "W_STATE_FILE_MISSING",
            "HQ brain: planning/state.json not found at root".to_string(),
        ));
    }

    // --- 3. Leaf repos from [[repos]] config ---
    for repo in &config.repos {
        // Skip the HQ root entry itself.
        if repo.repo_path == "." || repo.repo_path.is_empty() {
            continue;
        }
        let state_path = root
            .join(&repo.repo_path)
            .join("planning")
            .join("state.json");
        // Skip entries already discovered as a tier sub-brain rollup (e.g. a
        // `[[repos]]` entry for the tier's own repo, like `core`, alongside
        // HQ's `tiers[].rollup` pointing at the same file) — otherwise the
        // same file is registered twice with conflicting `expected_kind`.
        if sources.iter().any(|s| s.abs_path == state_path) {
            continue;
        }
        if state_path.exists() {
            let expected_kind = if repo.tier == "portfolio" {
                "portfolio"
            } else {
                "project"
            };
            sources.push(StateSource {
                repo_slug: repo.slug.clone(),
                abs_path: state_path,
                expected_kind,
            });
        } else {
            diags.push(Diagnostic::warning(
                state_path,
                "W_STATE_FILE_MISSING",
                format!(
                    "repo '{}': planning/state.json not found (path: '{}')",
                    repo.slug, repo.repo_path
                ),
            ));
        }
    }

    (sources, diags)
}

// ---------------------------------------------------------------------------
// Schema-ring checks
// ---------------------------------------------------------------------------

/// Valid `status` values for `focus.now`, `focus.blocked`, and `focus.deferred`
/// block entries (derived view).
///
/// `"deferred"` appears here as well as in [`VALID_TRACK_BLOCK_STATUSES`] because
/// the emitter stamps `status: "deferred"` onto the `focus.deferred[]` entries it
/// derives — omit it and `emit-state --write` would produce files that
/// `validate-brain` immediately rejects with `E_STATE_SCHEMA_BAD_STATUS`.
const VALID_STATUSES: &[&str] = &["open", "in_progress", "blocked", "deferred", "closed"];

/// Valid *authored* `status` values for `tracks[].blocks[]` entries.
///
/// `"blocked"` is intentionally excluded — it is a derived property, never an authored one.
/// Any block authored with `"blocked"` triggers `E_STATE_AUTHORED_BLOCKED`.
///
/// `"deferred"`, by contrast, *is* authored: it is a deliberate human decision to park
/// a block on the back burner, which nothing can derive. It is manual and sticky (no
/// expiry date — edit back to `"open"` to resume).
const VALID_TRACK_BLOCK_STATUSES: &[&str] = &["open", "in_progress", "deferred", "closed"];

/// The focus lanes, in the order they are reported in diagnostics and boards.
///
/// Used to drive [`check_focus_drift`]'s per-lane comparison from a single list,
/// so a new lane cannot be added to some comparisons but forgotten in others.
const FOCUS_LANES: [&str; 4] = ["now", "next", "blocked", "deferred"];

/// Valid `status` values for `backlog[]` entries (HQ brain only).
const VALID_BACKLOG_STATUSES: &[&str] = &["idea", "ready", "promoted"];

/// Valid `kind` values for `carryover[]` entries.
const VALID_CARRYOVER_KINDS: &[&str] = &["constraint", "known_issue", "env", "deferred"];

/// Parse an authored state-graph date that may be either bare `YYYY-MM-DD` or a
/// full RFC3339 timestamp (some `carryover[].created` values were stamped with a
/// time+offset). Returns the calendar date, or `None` if neither form parses.
///
/// This is the shared anchor parser for the Attention staleness clock and the
/// `E_STATE_DATE_FORMAT` check — a value that returns `None` here is what the
/// schema pass flags as malformed.
pub fn parse_state_date(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(s.trim())
                .ok()
                .map(|dt| dt.date_naive())
        })
}

/// The effective staleness anchor for an item: the latest of its `created` and
/// (optional) `reviewed` dates that parses. `None` when no date parses (the
/// item cannot age — the malformed date is surfaced separately as an error).
fn staleness_anchor(created: Option<&str>, reviewed: Option<&str>) -> Option<chrono::NaiveDate> {
    let c = created.and_then(parse_state_date);
    let r = reviewed.and_then(parse_state_date);
    match (c, r) {
        (Some(c), Some(r)) => Some(c.max(r)),
        (Some(c), None) => Some(c),
        (None, Some(r)) => Some(r),
        (None, None) => None,
    }
}

/// Whether an item is currently snoozed: `snoozed_until` parses to a date that
/// is still in the future (`today < snoozed_until`). An absent or unparseable
/// value is not snoozed.
fn is_snoozed(snoozed_until: Option<&str>, today: chrono::NaiveDate) -> bool {
    snoozed_until
        .and_then(parse_state_date)
        .is_some_and(|d| today < d)
}

/// The staleness verdict for a `carryover[]` entry: `Some(age_days)` when the
/// entry is past its per-`kind` threshold and not currently snoozed, else
/// `None`. This is the **single** predicate shared by the `validate-brain`
/// warnings and the `emit-state` Attention board, so the board shows exactly
/// what the warnings fire on.
pub fn carryover_stale_age(
    item: &Carryover,
    today: chrono::NaiveDate,
    thresholds: &crate::brain::config::AttentionThresholds,
) -> Option<i64> {
    if is_snoozed(item.snoozed_until.as_deref(), today) {
        return None;
    }
    let anchor = staleness_anchor(Some(&item.created), item.reviewed.as_deref())?;
    let age = (today - anchor).num_days();
    (age > thresholds.carryover_threshold(&item.kind)).then_some(age)
}

/// The staleness verdict for a `backlog[]` node: `Some(age_days)` when the node
/// is an `idea`/`ready` past the backlog threshold and not snoozed, else `None`.
/// (Whether the node lives in an HQ/brain file is the caller's concern.)
pub fn backlog_stale_age(
    item: &Backlog,
    today: chrono::NaiveDate,
    thresholds: &crate::brain::config::AttentionThresholds,
) -> Option<i64> {
    if item.status != "idea" && item.status != "ready" {
        return None;
    }
    if is_snoozed(item.snoozed_until.as_deref(), today) {
        return None;
    }
    let anchor = staleness_anchor(item.created.as_deref(), item.reviewed.as_deref())?;
    let age = (today - anchor).num_days();
    (age > thresholds.backlog_days).then_some(age)
}

/// Staleness warnings for `carryover[]` — one `W_STATE_CARRYOVER_STALE` per
/// entry whose age (from `max(created, reviewed)`) exceeds its per-`kind`
/// threshold and which is not currently snoozed. WARNING severity only — never
/// flips the exit code. Runs on every repo's `state.json` (each file's own
/// carryover is checked where it lives).
pub fn check_carryover_staleness(
    src: &StateSource,
    file: &StateFile,
    today: chrono::NaiveDate,
    thresholds: &crate::brain::config::AttentionThresholds,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let path = &src.abs_path;

    for item in &file.carryover {
        if let Some(age) = carryover_stale_age(item, today, thresholds) {
            let threshold = thresholds.carryover_threshold(&item.kind);
            let clears = item
                .clears_when
                .as_deref()
                .map(|c| format!(" (clears when: {c})"))
                .unwrap_or_default();
            diags.push(Diagnostic::warning(
                path,
                "W_STATE_CARRYOVER_STALE",
                format!(
                    "carryover '{}' (kind '{}') is {age}d old (threshold {threshold}d){clears} — \
                     promote it into a block/backlog node, resolve its clears_when, re-affirm it \
                     (bump 'reviewed'), or /snooze it",
                    item.slug, item.kind
                ),
            ));
        }
    }
    diags
}

/// Staleness warnings for the HQ `backlog[]` — one `W_STATE_BACKLOG_STALE` per
/// `idea`/`ready` node older than the backlog threshold that is not snoozed.
/// Nodes with no parseable `created` cannot age (never stale until dated).
/// WARNING severity only.
pub fn check_backlog_staleness(
    src: &StateSource,
    file: &StateFile,
    today: chrono::NaiveDate,
    thresholds: &crate::brain::config::AttentionThresholds,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    if file.kind != "brain" {
        return diags; // backlog[] is HQ/brain-only
    }
    let path = &src.abs_path;
    let threshold = thresholds.backlog_days;

    for item in &file.backlog {
        if let Some(age) = backlog_stale_age(item, today, thresholds) {
            let is_capture = item.origin.as_ref().is_some_and(|o| o.kind == "capture");
            let lead = if is_capture {
                "captured note never triaged"
            } else {
                "backlog idea"
            };
            diags.push(Diagnostic::warning(
                path,
                "W_STATE_BACKLOG_STALE",
                format!(
                    "{lead}: backlog '{}' (status '{}') is {age}d old (threshold {threshold}d) — \
                     promote it (/plan · /ticket · /chore) or /snooze it",
                    item.slug, item.status
                ),
            ));
        }
    }
    diags
}

/// Validate the schema-ring constraints for a successfully-deserialized
/// [`StateFile`].
///
/// Checks performed (all against the deserialized model — JSON structural
/// errors are already surfaced as [`StateLoadError::Parse`] before this
/// function is called):
///
/// 1. **`kind` membership** (`E_STATE_SCHEMA_BAD_KIND`) — `kind` must be
///    `"project"`, `"brain"`, or `"portfolio"`.  Also flags if `kind`
///    disagrees with the source's `expected_kind`.
/// 2. **`updated` non-empty** (`E_STATE_SCHEMA_MISSING_FIELD`) — the
///    `updated` string must not be blank (format checked by `MV.3.M`).
/// 3. **`status` enum** (`E_STATE_SCHEMA_BAD_STATUS`) — every `focus.now`
///    and `focus.blocked` entry whose `status` field is present must hold a
///    value in `{open, in_progress, blocked, closed}`.
/// 4. **`blocked_by` well-formedness** (`E_STATE_SCHEMA_BAD_BLOCKED_BY`) —
///    a `{type:"block"}` entry must have non-empty `repo` and `id`.
/// 5. **Kind-appropriate sections** (`E_STATE_SCHEMA_MISSING_FIELD`, warning)
///    — a `project` file is expected to carry `tracks[]`; a `brain` file is
///    expected to carry `repos[]`; a `portfolio` file is expected to carry a
///    non-empty `note` (terminal-state summary — it will never have `tracks[]`
///    or a sibling `master-plan.md`).
/// 6. **Authored `tracks[].blocks[].status` not `"blocked"`**
///    (`E_STATE_AUTHORED_BLOCKED`) — `"blocked"` is a derived property; an
///    authored track-block must use `{open, in_progress, closed}` only.
/// 7. **`tracks[].blocks[].depends_on` well-formedness**
///    (`E_STATE_SCHEMA_BAD_BLOCKED_BY`) — a `{type:"block"}` `depends_on`
///    entry must have non-empty `repo` and `id`.
/// 8. **`backlog[].status` enum** (`E_STATE_SCHEMA_BAD_STATUS`) — every
///    backlog node's `status` must be one of `{idea, ready, promoted}`.
pub fn check_schema(src: &StateSource, file: &StateFile) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();
    let path = &src.abs_path;

    // --- 1. kind membership ---
    match file.kind.as_str() {
        "project" | "brain" | "portfolio" => {
            // Valid — also check it matches the source's expected kind.
            if file.kind != src.expected_kind {
                diags.push(Diagnostic::error(
                    path,
                    "E_STATE_SCHEMA_BAD_KIND",
                    format!(
                        "kind '{}' does not match expected '{}' for this source",
                        file.kind, src.expected_kind
                    ),
                ));
            }
        }
        other => {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_BAD_KIND",
                format!("kind '{other}' is not valid; expected 'project', 'brain', or 'portfolio'"),
            ));
        }
    }

    // --- 2. updated non-empty ---
    if file.updated.trim().is_empty() {
        diags.push(Diagnostic::error(
            path,
            "E_STATE_SCHEMA_MISSING_FIELD",
            "required field 'updated' is present but empty".to_string(),
        ));
    }

    // --- 3. status enum + 4. blocked_by well-formedness ---
    // Check all focus collections that carry status or blocked_by.
    for block in file
        .focus
        .now
        .iter()
        .chain(file.focus.blocked.iter())
        .chain(file.focus.deferred.iter())
    {
        // status enum
        if let Some(status) = &block.status
            && !VALID_STATUSES.contains(&status.as_str())
        {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_BAD_STATUS",
                format!(
                    "block '{}' has invalid status '{}'; expected one of: {}",
                    block.id,
                    status,
                    VALID_STATUSES.join(", ")
                ),
            ));
        }

        // blocked_by well-formedness
        for bb in &block.blocked_by {
            if let BlockedBy::Block { repo, id, .. } = bb {
                let repo_empty = repo.trim().is_empty();
                let id_empty = id.trim().is_empty();
                if repo_empty || id_empty {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_SCHEMA_BAD_BLOCKED_BY",
                        format!(
                            "blocked_by entry in block '{}' is missing required \
                             field(s): {}",
                            block.id,
                            [repo_empty.then_some("'repo'"), id_empty.then_some("'id'")]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                }
            }
        }
    }

    // --- 5. kind-appropriate sections (warnings) ---
    if file.kind == "project" && file.tracks.is_empty() {
        diags.push(Diagnostic::warning(
            path,
            "E_STATE_SCHEMA_MISSING_FIELD",
            "project state.json is missing 'tracks[]'; expected a roadmap catalog".to_string(),
        ));
    }
    if file.kind == "brain" && file.repos.is_empty() {
        diags.push(Diagnostic::warning(
            path,
            "E_STATE_SCHEMA_MISSING_FIELD",
            "brain state.json is missing 'repos[]'; expected a child-repo rollup".to_string(),
        ));
    }
    if file.kind == "portfolio" && file.note.as_deref().unwrap_or("").trim().is_empty() {
        diags.push(Diagnostic::warning(
            path,
            "E_STATE_SCHEMA_MISSING_FIELD",
            "portfolio state.json is missing 'note'; expected a terminal-state summary \
             (e.g. \"Completed — live on GitHub\")"
                .to_string(),
        ));
    }

    // --- 6. Authored track-block status must not be "blocked" (v2: blocked is derived) ---
    // --- 7. depends_on entry well-formedness ---
    for track in &file.tracks {
        for block in &track.blocks {
            // check 6: authored status must not be "blocked"
            if let Some(status) = &block.status {
                if status == "blocked" {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_AUTHORED_BLOCKED",
                        format!(
                            "track block '{}' has authored status 'blocked'; \
                             'blocked' is a derived property — use open/in_progress/closed",
                            block.id
                        ),
                    ));
                } else if !VALID_TRACK_BLOCK_STATUSES.contains(&status.as_str()) {
                    // Also catch other invalid statuses on track blocks.
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_SCHEMA_BAD_STATUS",
                        format!(
                            "track block '{}' has invalid status '{}'; expected one of: {}",
                            block.id,
                            status,
                            VALID_TRACK_BLOCK_STATUSES.join(", ")
                        ),
                    ));
                }
            }

            // check 7: depends_on {type:block} entries must have non-empty repo and id
            for dep in &block.depends_on {
                if let BlockedBy::Block { repo, id, .. } = dep {
                    let repo_empty = repo.trim().is_empty();
                    let id_empty = id.trim().is_empty();
                    if repo_empty || id_empty {
                        diags.push(Diagnostic::error(
                            path,
                            "E_STATE_SCHEMA_BAD_BLOCKED_BY",
                            format!(
                                "depends_on entry in track block '{}' is missing required \
                                 field(s): {}",
                                block.id,
                                [repo_empty.then_some("'repo'"), id_empty.then_some("'id'")]
                                    .into_iter()
                                    .flatten()
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        ));
                    }
                }
            }
        }
    }

    // --- 8. backlog[].status enum + date formats (HQ brain only) ---
    for item in &file.backlog {
        if !VALID_BACKLOG_STATUSES.contains(&item.status.as_str()) {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_BAD_STATUS",
                format!(
                    "backlog item '{}' has invalid status '{}'; expected one of: {}",
                    item.slug,
                    item.status,
                    VALID_BACKLOG_STATUSES.join(", ")
                ),
            ));
        }
        for (field, value) in [
            ("created", item.created.as_deref()),
            ("reviewed", item.reviewed.as_deref()),
            ("snoozed_until", item.snoozed_until.as_deref()),
        ] {
            if let Some(v) = value
                && parse_state_date(v).is_none()
            {
                diags.push(Diagnostic::error(
                    path,
                    "E_STATE_DATE_FORMAT",
                    format!(
                        "backlog item '{}' has malformed {field} date '{}'; must be YYYY-MM-DD",
                        item.slug, v
                    ),
                ));
            }
        }
    }

    // --- 9. carryover[] validation ---
    for item in &file.carryover {
        if !VALID_CARRYOVER_KINDS.contains(&item.kind.as_str()) {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_BAD_KIND",
                format!(
                    "carryover item '{}' has invalid kind '{}'; expected one of: {}",
                    item.slug,
                    item.kind,
                    VALID_CARRYOVER_KINDS.join(", ")
                ),
            ));
        }

        let scope_fields_set = item.scope.repo.is_some() as u8
            + item.scope.tier.is_some() as u8
            + item.scope.cross_repo.is_some() as u8;

        if scope_fields_set != 1 {
            diags.push(Diagnostic::error(
                path,
                "E_STATE_SCHEMA_MALFORMED_SCOPE",
                format!(
                    "carryover item '{}' has malformed scope; exactly one of 'repo', 'tier', or 'cross_repo' must be set",
                    item.slug
                ),
            ));
        }

        for dep in &item.related {
            if let BlockedBy::Block { repo, id, .. } = dep {
                let repo_empty = repo.trim().is_empty();
                let id_empty = id.trim().is_empty();
                if repo_empty || id_empty {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_SCHEMA_BAD_BLOCKED_BY",
                        format!(
                            "related entry in carryover item '{}' is missing required \
                             field(s): {}",
                            item.slug,
                            [repo_empty.then_some("'repo'"), id_empty.then_some("'id'")]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ));
                }
            }
        }

        for (field, value) in [
            ("created", Some(item.created.as_str())),
            ("reviewed", item.reviewed.as_deref()),
            ("snoozed_until", item.snoozed_until.as_deref()),
        ] {
            if let Some(v) = value
                && parse_state_date(v).is_none()
            {
                diags.push(Diagnostic::error(
                    path,
                    "E_STATE_DATE_FORMAT",
                    format!(
                        "carryover item '{}' has malformed {field} date '{}'; must be YYYY-MM-DD \
                         or RFC3339",
                        item.slug, v
                    ),
                ));
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------

/// Run policy checks on the four newly-introduced optional block fields.
pub fn check_field_policy(src: &StateSource, file: &StateFile) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let path = &src.abs_path;

    for track in &file.tracks {
        for block in &track.blocks {
            if let Some(priority) = block.priority
                && priority > 3
            {
                diags.push(Diagnostic::error(
                    path,
                    "E_STATE_PRIORITY_RANGE",
                    format!(
                        "block '{}' has out-of-range priority {}; must be 0..=3",
                        block.id, priority
                    ),
                ));
            }

            if let Some(ref due) = block.due
                && chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d").is_err()
            {
                diags.push(Diagnostic::error(
                    path,
                    "E_STATE_DUE_FORMAT",
                    format!(
                        "block '{}' has malformed due date '{}'; must be YYYY-MM-DD",
                        block.id, due
                    ),
                ));
            }

            if let Some(ref wf) = block.sdlc_workflow {
                match wf.as_str() {
                    "none" | "patch" | "task" | "run" | "flow" => {}
                    _ => {
                        diags.push(Diagnostic::error(
                            path,
                            "E_STATE_SDLC_WORKFLOW_ENUM",
                            format!("block '{}' has invalid sdlc_workflow '{}'; must be one of {{none, patch, task, run, flow}}", block.id, wf),
                        ));
                    }
                }
            }

            if let Some(ref model) = block.model {
                match model.as_str() {
                    "sonnet" | "gemini-pro" | "gemini-flash" | "either" => {}
                    _ => {
                        diags.push(Diagnostic::error(
                            path,
                            "E_STATE_MODEL_ENUM",
                            format!("block '{}' has invalid model '{}'; must be one of {{sonnet, gemini-pro, gemini-flash, either}}", block.id, model),
                        ));
                    }
                }
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Epic registry + membership integrity
// ---------------------------------------------------------------------------

/// The valid values of an [`Epic`]'s `status` field.
const EPIC_STATUSES: [&str; 3] = ["active", "paused", "complete"];

/// The inclusive upper bound of an [`Epic`]'s authored `weight`.
///
/// `Epic::weight` is a `u8`, so `0..=255` parses; this is the policy bound mev
/// enforces via `E_STATE_EPIC_BAD_WEIGHT`.
const EPIC_WEIGHT_MAX: u8 = 100;

/// Locate the HQ brain file's `epics[]` registry.
///
/// The registry is HQ-only (same precedent as `backlog[]`, D2), so it lives on
/// the single `kind:"brain"` file whose [`tier_scope_for`] resolves to
/// [`TierScope::All`]. Returns an empty slice when there is no such file, which
/// makes every tagged block an `E_STATE_UNKNOWN_EPIC` — the correct outcome:
/// membership without a registry has nothing to validate against.
pub(crate) fn epic_registry<'a>(
    config: &BrainConfig,
    files: &'a [(StateSource, StateFile)],
) -> &'a [Epic] {
    files
        .iter()
        .find(|(_, f)| f.kind == "brain" && matches!(tier_scope_for(f, config), TierScope::All))
        .map(|(_, f)| f.epics.as_slice())
        .unwrap_or(&[])
}

/// Validate the `epics[]` registry and every block's epic membership.
///
/// A corpus-level check (not per-file, like [`check_field_policy`]) because
/// membership on any repo's block is validated against a registry that lives in
/// exactly one other file — the same shape as [`check_backlog_integrity`].
///
/// Checks performed:
///
/// 1. **`E_STATE_DUPLICATE_EPIC_SLUG`** — two registry entries share a `slug`.
/// 2. **`E_STATE_EPIC_BAD_STATUS`** — a registry `status` ∉ [`EPIC_STATUSES`].
/// 3. **`E_STATE_EPIC_BAD_WEIGHT`** — a registry `weight` outside `0..=100`.
///    `Epic::weight` is a `u8` (0..=255) precisely so this range check is real
///    validation rather than a type tautology; okf-core holds the field, mev
///    holds the policy.
/// 4. **`W_STATE_EPIC_REGISTRY_IGNORED`** — a non-HQ file carries its own
///    `epics[]`. The registry is HQ-only, so such entries are silently unused;
///    without this warning a shadow registry would look authoritative and every
///    block referencing it would fail with a confusing `E_STATE_UNKNOWN_EPIC`.
/// 5. **`E_STATE_UNKNOWN_EPIC`** — a block's `epics[]` entry resolves to no
///    registry slug. This is what turns a typo into an error instead of a
///    silently-empty board.
/// 6. **`W_STATE_EPIC_EMPTY`** — a registered epic has no member blocks.
/// 7. **`W_STATE_EPIC_UNREACHABLE_DEP`** — an unclosed block in some epic
///    `depends_on` an unclosed block belonging to no epic. That dependency gates
///    the epic but would never appear on its board — the silent-gate case.
pub fn check_epics(config: &BrainConfig, files: &[(StateSource, StateFile)]) -> Vec<Diagnostic> {
    use std::collections::{HashMap, HashSet};

    let mut diags = Vec::new();
    let registry = epic_registry(config, files);

    // --- 1/2/3. Registry well-formedness ---
    let mut known: HashSet<&str> = HashSet::new();
    for epic in registry {
        // Report the HQ file itself; find it the same way `epic_registry` did.
        let path = files
            .iter()
            .find(|(_, f)| f.kind == "brain" && matches!(tier_scope_for(f, config), TierScope::All))
            .map(|(s, _)| s.abs_path.clone())
            .unwrap_or_default();

        if !known.insert(epic.slug.as_str()) {
            diags.push(Diagnostic::error(
                &path,
                "E_STATE_DUPLICATE_EPIC_SLUG",
                format!(
                    "epics[] declares slug '{}' more than once; slugs are the membership key \
                     and must be unique",
                    epic.slug
                ),
            ));
        }

        if let Some(ref status) = epic.status
            && !EPIC_STATUSES.contains(&status.as_str())
        {
            diags.push(Diagnostic::error(
                &path,
                "E_STATE_EPIC_BAD_STATUS",
                format!(
                    "epic '{}' has invalid status '{}'; must be one of {{{}}}",
                    epic.slug,
                    status,
                    EPIC_STATUSES.join(", ")
                ),
            ));
        }

        // No lower-bound check: `weight` is a `u8`, so it cannot be negative and
        // `0` is a legitimate authored value. Do not add a redundant `w < 0`.
        if let Some(weight) = epic.weight
            && weight > EPIC_WEIGHT_MAX
        {
            diags.push(Diagnostic::error(
                &path,
                "E_STATE_EPIC_BAD_WEIGHT",
                format!(
                    "epic '{}' has invalid weight {}; must be in 0..={}",
                    epic.slug, weight, EPIC_WEIGHT_MAX
                ),
            ));
        }
    }

    // --- 4. Shadow registries on non-HQ files ---
    for (src, file) in files {
        let is_hq = file.kind == "brain" && matches!(tier_scope_for(file, config), TierScope::All);
        if !is_hq && !file.epics.is_empty() {
            diags.push(Diagnostic::warning(
                &src.abs_path,
                "W_STATE_EPIC_REGISTRY_IGNORED",
                format!(
                    "'{}' declares {} epics[] entr{}, but the registry is HQ-only and these are \
                     ignored; move them to the HQ brain's state.json",
                    src.repo_slug,
                    file.epics.len(),
                    if file.epics.len() == 1 { "y" } else { "ies" }
                ),
            ));
        }
    }

    // --- 5. Membership resolves; collect members for check 6 ---
    let mut members: HashMap<&str, usize> = known.iter().map(|s| (*s, 0usize)).collect();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                for slug in &block.epics {
                    match members.get_mut(slug.as_str()) {
                        Some(count) => *count += 1,
                        None => diags.push(Diagnostic::error(
                            &src.abs_path,
                            "E_STATE_UNKNOWN_EPIC",
                            format!(
                                "block '{}' claims epic '{}', which is not declared in the HQ \
                                 epics[] registry",
                                block.id, slug
                            ),
                        )),
                    }
                }
            }
        }
    }

    // --- 6. Registered but unused ---
    for epic in registry {
        if members.get(epic.slug.as_str()) == Some(&0) {
            let path = files
                .iter()
                .find(|(_, f)| {
                    f.kind == "brain" && matches!(tier_scope_for(f, config), TierScope::All)
                })
                .map(|(s, _)| s.abs_path.clone())
                .unwrap_or_default();
            diags.push(Diagnostic::warning(
                &path,
                "W_STATE_EPIC_EMPTY",
                format!(
                    "epic '{}' has no member blocks; its board will render empty until blocks \
                     declare it in their epics[]",
                    epic.slug
                ),
            ));
        }
    }

    // --- 7. Silent gates: an unclosed dependency outside every epic ---
    let mut by_key: HashMap<String, &TrackBlock> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                by_key.insert(format!("{}:{}", src.repo_slug, block.id), block);
            }
        }
    }
    let is_closed = |b: &TrackBlock| b.status.as_deref() == Some("closed");

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                if block.epics.is_empty() || is_closed(block) {
                    continue;
                }
                for dep in &block.depends_on {
                    let BlockedBy::Block { repo, id, .. } = dep else {
                        continue; // external deps have no target node
                    };
                    // A dangling target is reported by check_state_graph; skip it
                    // here rather than double-reporting it as an untagged gate.
                    let Some(target) = by_key.get(&format!("{repo}:{id}")) else {
                        continue;
                    };
                    if is_closed(target) || !target.epics.is_empty() {
                        continue;
                    }
                    diags.push(Diagnostic::warning(
                        &src.abs_path,
                        "W_STATE_EPIC_UNREACHABLE_DEP",
                        format!(
                            "block '{}' (epic{} {}) depends on open block '{repo}:{id}', which \
                             belongs to no epic — it gates the epic but will not appear on its board",
                            block.id,
                            if block.epics.len() == 1 { "" } else { "s" },
                            block.epics.join(", ")
                        ),
                    ));
                }
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Graph integrity checks
// ---------------------------------------------------------------------------

/// Run integrity checks over a built [`StateGraph`].
///
/// Checks performed:
///
/// 1. **`E_STATE_DUPLICATE_BLOCK_ID`** — two `tracks[]` blocks in the same
///    repo share an `id`.  Emits one diagnostic per duplicate key (not one
///    per occurrence).
/// 2. **`E_STATE_DANGLING_FOCUS`** — a leaf (`kind:"project"`) file's
///    `focus.now/next/blocked` entry's `block` is absent from that repo's
///    `tracks[]`.  Brain focus entries are cross-repo and intentionally
///    excluded from this check.
/// 3. **`E_STATE_UNKNOWN_REPO`** — a `blocked_by` or `cross_repo` edge
///    references a `repo` that has no discoverable `state.json` (i.e. is not
///    in `files`).
/// 4. **`E_STATE_DANGLING_BLOCKED_BY`** — a `{type:"block",repo,id}` edge's
///    target `id` does not exist in the named repo's `tracks[]` (repo is
///    known, block is not).
/// 5. **`E_STATE_DANGLING_CROSS_REPO`** — a brain `cross_repo[]` edge's
///    `from` or `to` endpoint's block does not exist in the named repo's
///    `tracks[]` (repo is known, block is not).
pub fn check_state_graph(
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Vec<Diagnostic> {
    use std::collections::{HashMap, HashSet};

    let mut diags: Vec<Diagnostic> = Vec::new();

    // --- Build lookup structures ---

    // All repo slugs that have a loaded state.json.
    let known_repos: HashSet<&str> = files.iter().map(|(s, _)| s.repo_slug.as_str()).collect();

    // Count occurrences of each "repo:id" key so we can detect duplicates.
    let mut node_counts: HashMap<&str, usize> = HashMap::new();
    for node in &graph.nodes {
        *node_counts.entry(node.key.as_str()).or_insert(0) += 1;
    }

    // Set of all registered "repo:id" keys (for dangling checks).
    let node_set: HashSet<&str> = node_counts.keys().copied().collect();

    // --- 1. Duplicate block IDs ---
    let mut duplicate_reported: HashSet<&str> = HashSet::new();
    for node in &graph.nodes {
        let key = node.key.as_str();
        if node_counts[key] > 1 && !duplicate_reported.contains(key) {
            duplicate_reported.insert(key);
            diags.push(Diagnostic::error(
                &node.source_path,
                "E_STATE_DUPLICATE_BLOCK_ID",
                format!(
                    "duplicate block id '{}' in repo '{}' tracks[]",
                    node.id, node.repo
                ),
            ));
        }
    }

    // --- 2. Dangling focus (leaf files only) ---
    for (src, file) in files {
        if file.kind != "project" {
            continue;
        }
        let path = &src.abs_path;
        let all_focus = file
            .focus
            .now
            .iter()
            .chain(file.focus.next.iter())
            .chain(file.focus.blocked.iter())
            .chain(file.focus.deferred.iter());

        for block in all_focus {
            let key = format!("{}:{}", src.repo_slug, block.id);
            if !node_set.contains(key.as_str()) {
                diags.push(Diagnostic::error(
                    path,
                    "E_STATE_DANGLING_FOCUS",
                    format!(
                        "focus block '{}' is not registered in this repo's tracks[]",
                        block.id
                    ),
                ));
            }
        }
    }

    // --- 3–5. Edge integrity checks ---
    for edge in &graph.edges {
        let path = &edge.source_path;

        match edge.kind {
            StateEdgeKind::BlockedBy => {
                // Parse to_ref as "repo:id".
                let Some((to_repo, to_id)) = edge.to_ref.split_once(':') else {
                    continue;
                };
                if !known_repos.contains(to_repo) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_UNKNOWN_REPO",
                        format!("blocked_by references unknown repo '{to_repo}'"),
                    ));
                } else if !node_set.contains(edge.to_ref.as_str()) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_DANGLING_BLOCKED_BY",
                        format!(
                            "blocked_by block '{to_id}' does not exist in repo '{to_repo}' tracks[]"
                        ),
                    ));
                }
            }

            StateEdgeKind::CrossRepo => {
                // Check from endpoint.
                let Some((from_repo, from_id)) = edge.from.split_once(':') else {
                    continue;
                };
                if !known_repos.contains(from_repo) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_UNKNOWN_REPO",
                        format!("cross_repo 'from' references unknown repo '{from_repo}'"),
                    ));
                } else if !node_set.contains(edge.from.as_str()) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_DANGLING_CROSS_REPO",
                        format!(
                            "cross_repo 'from' block '{from_id}' does not exist in repo \
                             '{from_repo}' tracks[]"
                        ),
                    ));
                }

                // Check to endpoint.
                let Some((to_repo, to_id)) = edge.to_ref.split_once(':') else {
                    continue;
                };
                if !known_repos.contains(to_repo) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_UNKNOWN_REPO",
                        format!("cross_repo 'to' references unknown repo '{to_repo}'"),
                    ));
                } else if !node_set.contains(edge.to_ref.as_str()) {
                    diags.push(Diagnostic::error(
                        path,
                        "E_STATE_DANGLING_CROSS_REPO",
                        format!(
                            "cross_repo 'to' block '{to_id}' does not exist in repo \
                             '{to_repo}' tracks[]"
                        ),
                    ));
                }
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Status-consistency check (Task 4)
// ---------------------------------------------------------------------------

/// Check that no `closed` block depends on a non-`closed` block.
///
/// A block that declares `status: "closed"` with a `{type:"block"}` entry in its
/// `depends_on[]` that points to a block whose authored status is **not** `"closed"`
/// is inconsistent — the dependency was not complete before the dependent was closed.
///
/// Emits **`E_STATE_STATUS_INCONSISTENT`** for each such pair.  Dependencies whose
/// target does not exist in any loaded file are silently skipped here — they are
/// already reported as `E_STATE_DANGLING_BLOCKED_BY` by [`check_state_graph`].
pub fn check_status_consistency(files: &[(StateSource, StateFile)]) -> Vec<Diagnostic> {
    use std::collections::HashMap;

    // Build a status lookup: "repo:id" → authored status (None = absent = treated as open).
    let mut status_map: HashMap<String, Option<String>> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                status_map.insert(key, block.status.clone());
            }
        }
    }

    let mut diags: Vec<Diagnostic> = Vec::new();

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                // Only closed blocks participate in this check.
                if block.status.as_deref() != Some("closed") {
                    continue;
                }

                let from_key = format!("{}:{}", src.repo_slug, block.id);

                for dep in &block.depends_on {
                    if let BlockedBy::Block { repo, id, .. } = dep {
                        let dep_key = format!("{repo}:{id}");
                        // If the dep target is not in any loaded file, skip — it will
                        // be reported as E_STATE_DANGLING_BLOCKED_BY by check_state_graph.
                        if let Some(dep_status) = status_map.get(&dep_key) {
                            let dep_is_closed = dep_status.as_deref() == Some("closed");
                            if !dep_is_closed {
                                diags.push(Diagnostic::error(
                                    &src.abs_path,
                                    "E_STATE_STATUS_INCONSISTENT",
                                    format!(
                                        "closed block '{from_key}' has a non-closed depends_on \
                                         target '{dep_key}' (status: {})",
                                        dep_status.as_deref().unwrap_or("open")
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Backlog-node integrity check (Task 4)
// ---------------------------------------------------------------------------

/// Validate referential integrity for `backlog[]` nodes (HQ brain files only).
///
/// Two checks are performed:
///
/// 1. **`E_STATE_DANGLING_BLOCKED_BY`** — a backlog node's `depends_on[]` entry of
///    `{type:"block"}` references a block that is not registered in any loaded
///    repo's `tracks[]`.
///
/// 2. **`E_STATE_DANGLING_PROMOTION`** — a backlog node whose `status` is `"promoted"`
///    carries a `block` pointer (or is missing one) that does not resolve to an
///    existing node in `{backlog.repo}:tracks[]`.
///
/// Backlog nodes with `status` other than `"promoted"` are not checked for
/// promotion integrity (they have no `block` pointer by contract).
pub fn check_backlog_integrity(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
) -> Vec<Diagnostic> {
    use std::collections::HashSet;

    // Set of all registered "repo:id" keys.
    let node_set: HashSet<&str> = graph.nodes.iter().map(|n| n.key.as_str()).collect();

    let mut diags: Vec<Diagnostic> = Vec::new();

    for (src, file) in files {
        if file.backlog.is_empty() {
            continue;
        }

        let path = &src.abs_path;

        for backlog_node in &file.backlog {
            // --- 1. Dangling depends_on ---
            for dep in &backlog_node.depends_on {
                if let BlockedBy::Block { repo, id, .. } = dep {
                    let dep_key = format!("{repo}:{id}");
                    if !node_set.contains(dep_key.as_str()) {
                        diags.push(Diagnostic::error(
                            path,
                            "E_STATE_DANGLING_BLOCKED_BY",
                            format!(
                                "backlog node '{}' depends_on references unknown block '{dep_key}'",
                                backlog_node.slug
                            ),
                        ));
                    }
                }
            }

            // --- 2. Orphan promoted node ---
            if backlog_node.status == "promoted" {
                match &backlog_node.block {
                    None => {
                        // Promoted with no block pointer — the promotion target is unknown.
                        diags.push(Diagnostic::error(
                            path,
                            "E_STATE_DANGLING_PROMOTION",
                            format!(
                                "backlog node '{}' has status 'promoted' but no 'block' pointer",
                                backlog_node.slug
                            ),
                        ));
                    }
                    Some(block_id) => {
                        // Promoted and pointing at a block — verify the block exists anywhere in the graph.
                        let block_exists = graph.nodes.iter().any(|n| n.id == *block_id);
                        if !block_exists {
                            diags.push(Diagnostic::error(
                                path,
                                "E_STATE_DANGLING_PROMOTION",
                                format!(
                                    "backlog node '{}' promoted to block '{block_id}' which does \
                                     not exist in any repo's tracks[]",
                                    backlog_node.slug
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Rollup-drift check (brain files)
// ---------------------------------------------------------------------------

/// Compare a brain `repos[]` headline cache against the children's actual `focus`.
///
/// For each entry in `brain.repos[]`, looks up the child's actual state in
/// `children` (keyed by repo slug).  If the cached `now`/`next`/`blocked`
/// block-id sets don't match the child's live `focus`, emits one
/// `W_STATE_ROLLUP_DRIFT` warning per drifted child.
///
/// Children absent from `children` (no loaded state.json) are silently
/// skipped — `W_STATE_FILE_MISSING` was already emitted during discovery.
///
/// Per scoping decision 4, rollup drift is a **warning** (exit 0); only
/// referential errors are `Error`-severity (exit 1).
///
/// Comparison is over block-id sets only; cosmetic field differences
/// (`title`, `note`) are intentionally ignored.
pub fn check_rollup(
    brain_path: &Path,
    brain: &StateFile,
    children: &std::collections::HashMap<String, StateFile>,
) -> Vec<Diagnostic> {
    use std::collections::HashSet;

    let mut diags: Vec<Diagnostic> = Vec::new();

    for rollup in &brain.repos {
        let child = match children.get(&rollup.repo) {
            Some(c) => c,
            // No state.json loaded for this child — W_STATE_FILE_MISSING was
            // already emitted during discovery; skip silently here.
            None => continue,
        };

        // Compare block-id sets; ignore title/note cosmetic differences.
        let cached_now: HashSet<&str> = rollup.now.iter().map(|b| b.id.as_str()).collect();
        let actual_now: HashSet<&str> = child.focus.now.iter().map(|b| b.id.as_str()).collect();

        let cached_next: HashSet<&str> = rollup.next.iter().map(|b| b.id.as_str()).collect();
        let actual_next: HashSet<&str> = child.focus.next.iter().map(|b| b.id.as_str()).collect();

        let cached_blocked: HashSet<&str> = rollup.blocked.iter().map(|b| b.id.as_str()).collect();
        let actual_blocked: HashSet<&str> =
            child.focus.blocked.iter().map(|b| b.id.as_str()).collect();

        let cached_deferred: HashSet<&str> =
            rollup.deferred.iter().map(|b| b.id.as_str()).collect();
        let actual_deferred: HashSet<&str> =
            child.focus.deferred.iter().map(|b| b.id.as_str()).collect();

        if cached_now != actual_now
            || cached_next != actual_next
            || cached_blocked != actual_blocked
            || cached_deferred != actual_deferred
        {
            // Build a compact diff summary for the warning message.
            let mut diffs: Vec<String> = Vec::new();
            if cached_now != actual_now {
                diffs.push(format!(
                    "now: cached={:?} actual={:?}",
                    sorted_set(&cached_now),
                    sorted_set(&actual_now)
                ));
            }
            if cached_next != actual_next {
                diffs.push(format!(
                    "next: cached={:?} actual={:?}",
                    sorted_set(&cached_next),
                    sorted_set(&actual_next)
                ));
            }
            if cached_blocked != actual_blocked {
                diffs.push(format!(
                    "blocked: cached={:?} actual={:?}",
                    sorted_set(&cached_blocked),
                    sorted_set(&actual_blocked)
                ));
            }
            if cached_deferred != actual_deferred {
                diffs.push(format!(
                    "deferred: cached={:?} actual={:?}",
                    sorted_set(&cached_deferred),
                    sorted_set(&actual_deferred)
                ));
            }

            diags.push(Diagnostic::warning(
                brain_path,
                "W_STATE_ROLLUP_DRIFT",
                format!(
                    "repos[] entry for '{}' has drifted from child's actual focus — {}",
                    rollup.repo,
                    diffs.join("; ")
                ),
            ));
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Cycle detection
// ---------------------------------------------------------------------------

/// A single cycle found in the `depends_on` subgraph.
///
/// `keys` holds the cycle's nodes in DFS traversal order, **without** the repeated
/// closing node (i.e. the node that both starts and ends the cycle appears exactly
/// once). `source_path` is the back-edge's `source_path` — where the diagnostic that
/// reports this cycle should be anchored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CyclePath {
    pub keys: Vec<String>,
    pub source_path: std::path::PathBuf,
}

/// Find every cycle in the `depends_on` edge subgraph of `graph`.
///
/// Performs a DFS over [`StateEdgeKind::BlockedBy`] edges only; [`StateEdgeKind::CrossRepo`]
/// edges are intentionally excluded (they annotate inter-repo intent, not block ordering
/// and are not part of the authoritative DAG).
///
/// Results are deduplicated by **canonical rotation**: each cycle's `keys` are rotated so
/// the lexicographically smallest key is first, and only the first cycle to produce a given
/// rotated form is kept. Returned in discovery order.
///
/// Returns an empty `Vec` when the `depends_on` subgraph is acyclic.
pub fn cycle_paths(graph: &StateGraph) -> Vec<CyclePath> {
    use std::collections::{HashMap, HashSet};

    // Build adjacency: from_key → Vec<(to_ref, source_path)>.
    // Initialise every node so isolated nodes are visited and early-exited cleanly.
    let mut adj: HashMap<&str, Vec<(&str, &std::path::Path)>> = HashMap::new();
    for node in &graph.nodes {
        adj.entry(node.key.as_str()).or_default();
    }
    for edge in &graph.edges {
        if edge.kind == StateEdgeKind::BlockedBy {
            adj.entry(edge.from.as_str())
                .or_default()
                .push((edge.to_ref.as_str(), edge.source_path.as_path()));
        }
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut paths: Vec<CyclePath> = Vec::new();
    let mut seen_rotations: HashSet<Vec<String>> = HashSet::new();

    // Iterate in a deterministic order (node-insertion order from the graph).
    let starts: Vec<String> = graph.nodes.iter().map(|n| n.key.clone()).collect();
    for start in &starts {
        if !visited.contains(start.as_str()) {
            let mut rec_stack: Vec<String> = Vec::new();
            cycle_paths_dfs(
                start.as_str(),
                &adj,
                &mut visited,
                &mut rec_stack,
                &mut paths,
                &mut seen_rotations,
            );
        }
    }

    paths
}

/// Rotate `keys` so the lexicographically smallest entry is first, for canonical dedup.
fn canonical_rotation(keys: &[String]) -> Vec<String> {
    if keys.is_empty() {
        return Vec::new();
    }
    let min_idx = keys
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.cmp(b))
        .map(|(i, _)| i)
        .unwrap_or(0);
    keys[min_idx..]
        .iter()
        .chain(keys[..min_idx].iter())
        .cloned()
        .collect()
}

/// DFS worker for [`cycle_paths`].
///
/// `rec_stack` tracks the current DFS path (nodes in the "gray" / visiting state).
/// `visited` is the union of gray + black nodes (prevents re-visiting fully-explored nodes).
/// `seen_rotations` deduplicates cycles by canonical rotation so each cycle is emitted once.
fn cycle_paths_dfs<'a>(
    node: &'a str,
    adj: &std::collections::HashMap<&'a str, Vec<(&'a str, &'a std::path::Path)>>,
    visited: &mut std::collections::HashSet<String>,
    rec_stack: &mut Vec<String>,
    paths: &mut Vec<CyclePath>,
    seen_rotations: &mut std::collections::HashSet<Vec<String>>,
) {
    visited.insert(node.to_string());
    rec_stack.push(node.to_string());

    if let Some(neighbors) = adj.get(node) {
        for (neighbor, source_path) in neighbors {
            if !visited.contains(*neighbor) {
                cycle_paths_dfs(neighbor, adj, visited, rec_stack, paths, seen_rotations);
            } else if let Some(pos) = rec_stack.iter().position(|n| n == neighbor) {
                // Back-edge — `neighbor` is still on the recursion stack.
                let cycle: Vec<String> = rec_stack[pos..].to_vec();
                let rotated = canonical_rotation(&cycle);
                if seen_rotations.insert(rotated) {
                    paths.push(CyclePath {
                        keys: cycle,
                        source_path: source_path.to_path_buf(),
                    });
                }
            }
        }
    }

    rec_stack.pop();
}

/// Detect cycles in the `depends_on` edge subgraph of `graph`.
///
/// Performs a DFS over [`StateEdgeKind::BlockedBy`] edges only; [`StateEdgeKind::CrossRepo`]
/// edges are intentionally excluded (they annotate inter-repo intent, not block ordering
/// and are not part of the authoritative DAG).
///
/// On detection of a back-edge, emits **`E_STATE_CYCLE`** naming the cycle path in the
/// form `A → B → C → A`.  Each distinct cycle path is reported once.
///
/// Returns an empty `Vec` when the `depends_on` subgraph is acyclic.
pub fn detect_cycles(graph: &StateGraph) -> Vec<Diagnostic> {
    use std::collections::HashSet;

    let mut diags: Vec<Diagnostic> = Vec::new();
    let mut reported: HashSet<String> = HashSet::new();

    for cycle in cycle_paths(graph) {
        let path_str = format!(
            "{} \u{2192} {}",
            cycle.keys.join(" \u{2192} "),
            cycle.keys[0]
        );
        if reported.insert(path_str.clone()) {
            diags.push(Diagnostic::error(
                &cycle.source_path,
                "E_STATE_CYCLE",
                format!("cycle detected in depends_on DAG: {path_str}"),
            ));
        }
    }

    diags
}

// ---------------------------------------------------------------------------
// Effective priority (reverse-topo min-propagation — MV.7.A)
// ---------------------------------------------------------------------------

/// Compute each block's **effective priority** by reverse-topological `min`-
/// propagation over the `depends_on` DAG.
///
/// `effective(n) = min(own(n), min{ effective(m) : m depends_on n })` — a
/// block's effective priority is the hottest (numerically lowest) priority
/// among itself and every block that (transitively) depends on it. This lets
/// an engineering block that gates a hot business block inherit that
/// hotness, so it floats to the top of the unified board's `NEXT` ordering
/// even though its own authored priority is cold or absent.
///
/// Keyed by canonical `"repo:id"` (matching [`StateNode::key`]). Own priority
/// is read from `TrackBlock.priority` in `files` (mirroring the
/// [`derive_focus`] status-map pattern); an absent own priority is treated as
/// the lowest hotness (`u8::MAX`) so it never wins a `min` against a real
/// priority.
///
/// Propagation walks the **reverse** `BlockedBy` adjacency (from a
/// dependency node to its dependents), mirroring the forward-adjacency build
/// in [`detect_cycles`]. The walk is memoized DFS with a recursion-stack
/// guard: a node already on the stack short-circuits to its own priority
/// instead of recursing again, so a `depends_on` cycle terminates
/// deterministically without hanging or panicking — this pass does not
/// assume `MV.3.P2` has already rejected cycles.
///
/// Only nodes whose effective value lands in the real priority range
/// (`0..=3`) get a map entry; a node whose effective value stays `u8::MAX`
/// (no own priority and no hotter dependent, transitively) is omitted, so
/// callers that `.get(key).copied()` naturally treat it as absent and sort
/// it last — matching how raw `Block.priority: None` sorted before this
/// pass existed.
pub fn effective_priorities(
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> std::collections::HashMap<String, u8> {
    use std::collections::{HashMap, HashSet};

    // Own priority per node key ("repo:id"); absent → u8::MAX (never wins a min).
    let mut own: HashMap<String, u8> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                own.insert(key, block.priority.unwrap_or(u8::MAX));
            }
        }
    }
    // Defensive: every graph node gets an own-priority entry even if it was
    // somehow absent from `files` (keeps the DFS total over `graph.nodes`).
    for node in &graph.nodes {
        own.entry(node.key.clone()).or_insert(u8::MAX);
    }

    // Reverse adjacency: to_ref (dependency) → [from, ...] (its dependents).
    // `from depends_on to_ref`, so propagating min *into* to_ref from its
    // dependents' effective values is exactly "a block inherits the hotness
    // of what it gates".
    let mut reverse_adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for node in &graph.nodes {
        reverse_adj.entry(node.key.as_str()).or_default();
    }
    for edge in &graph.edges {
        if edge.kind == StateEdgeKind::BlockedBy {
            reverse_adj
                .entry(edge.to_ref.as_str())
                .or_default()
                .push(edge.from.as_str());
        }
    }

    let mut memo: HashMap<String, u8> = HashMap::new();
    let mut on_stack: HashSet<String> = HashSet::new();

    fn compute(
        key: &str,
        own: &HashMap<String, u8>,
        reverse_adj: &HashMap<&str, Vec<&str>>,
        memo: &mut HashMap<String, u8>,
        on_stack: &mut HashSet<String>,
    ) -> u8 {
        if let Some(&v) = memo.get(key) {
            return v;
        }
        let own_priority = own.get(key).copied().unwrap_or(u8::MAX);
        // Cycle guard: `key` is already being computed further up this DFS
        // path — short-circuit to its own priority instead of recursing
        // again, so a `depends_on` cycle can't recurse forever.
        if on_stack.contains(key) {
            return own_priority;
        }
        on_stack.insert(key.to_string());

        let mut best = own_priority;
        if let Some(dependents) = reverse_adj.get(key) {
            for dep in dependents {
                let v = compute(dep, own, reverse_adj, memo, on_stack);
                if v < best {
                    best = v;
                }
            }
        }

        on_stack.remove(key);
        memo.insert(key.to_string(), best);
        best
    }

    let keys: Vec<String> = own.keys().cloned().collect();
    for key in &keys {
        compute(key, &own, &reverse_adj, &mut memo, &mut on_stack);
    }

    memo.into_iter().filter(|(_, v)| *v <= 3).collect()
}

// ---------------------------------------------------------------------------
// Ready-order (reusable — MV.3B.T topo-emitter input)
// ---------------------------------------------------------------------------

/// Compute the wave-ordered list of **ready** `open` blocks across all files.
///
/// A block is *ready* iff:
/// - Its authored status is `"open"` (or absent — treated as open).
/// - It has **zero** `{type:"external"}` `depends_on` entries (external dependencies
///   mean the block is gated on something outside the graph).
/// - Every `{type:"block"}` `depends_on` target has authored status `"closed"`.
///
/// The returned `Vec<String>` lists canonical `"repo:id"` keys ordered by:
/// 1. `wave` ascending (`None` treated as `i64::MAX` — lowest priority, goes last).
/// 2. Track iteration order across `files` (stable tiebreak: position of the containing
///    track, then block array index within that track).
///
/// `graph` is accepted for forward-compatibility — `MV.3B.T` will extend this function
/// to query graph structure; the current implementation derives all information from `files`.
/// Callers should always pass the graph built from the same `files` slice.
///
/// This function is **standalone and public** — do not inline it into any check function.
pub fn ready_order(_graph: &StateGraph, files: &[(StateSource, StateFile)]) -> Vec<String> {
    use std::collections::HashMap;

    // Status lookup: "repo:id" → authored status (None = absent = open).
    let mut status_map: HashMap<String, Option<String>> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                status_map.insert(key, block.status.clone());
            }
        }
    }

    // Collect (wave, iteration_order, "repo:id") for every ready open block.
    let mut ready: Vec<(i64, usize, String)> = Vec::new();
    let mut order: usize = 0;

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let current_order = order;
                order += 1;

                // Only open (or status-absent) blocks are candidates.
                let status = block.status.as_deref().unwrap_or("open");
                if status != "open" {
                    continue;
                }

                // Any external dep disqualifies the block (not yet runnable).
                let has_external = block
                    .depends_on
                    .iter()
                    .any(|d| matches!(d, BlockedBy::External { .. }));
                if has_external {
                    continue;
                }

                // All block deps must be closed.
                let all_block_deps_closed = block.depends_on.iter().all(|d| {
                    if let BlockedBy::Block { repo, id, .. } = d {
                        let dep_key = format!("{repo}:{id}");
                        status_map.get(&dep_key).and_then(|s| s.as_deref()) == Some("closed")
                    } else {
                        true // External entries handled above; this branch is unreachable here.
                    }
                });

                if all_block_deps_closed {
                    let wave = block.wave.unwrap_or(i64::MAX);
                    let key = format!("{}:{}", src.repo_slug, block.id);
                    ready.push((wave, current_order, key));
                }
            }
        }
    }

    // Primary sort: wave asc. Tiebreak: iteration order (stable).
    ready.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    ready.into_iter().map(|(_, _, key)| key).collect()
}

// ---------------------------------------------------------------------------
// Focus derivation (MV.3B.T — single-source derivation engine)
// ---------------------------------------------------------------------------

/// The derived view of a file's focus — computed from `tracks[]` by [`derive_focus`].
///
/// All four lists contain canonical block IDs (without the `"repo:"` prefix).
/// `blocked` additionally carries the **unmet subset** of `depends_on` for each blocked
/// block — the unmet items are used by the emitter to populate `focus.blocked[].blocked_by[]`.
#[derive(Debug, Clone, Default)]
pub struct DerivedFocus {
    /// Block IDs with authored `status == "in_progress"`.
    pub now: Vec<String>,
    /// Block IDs that are ready (open, no external deps, all block deps closed), in wave order.
    pub next: Vec<String>,
    /// `(block_id, unmet_deps)` — open blocks with at least one unmet dependency.
    pub blocked: Vec<(String, Vec<BlockedBy>)>,
    /// Block IDs with authored `status == "deferred"` — parked on the back burner.
    ///
    /// Plain IDs, like `now`: a deferred block carries no `blocked_by`, because
    /// deferral is a terminal lane assignment that never consults dependencies.
    pub deferred: Vec<String>,
}

/// Derive the expected `focus` from a file's `tracks[]`.
///
/// This is the **leaf derivation**, used directly for `kind == "project"` files by both
/// [`check_focus_drift`] (the validator) and `mev emit-state` (the writer) — because both
/// call this function for project-kind files, the validator and the emitter cannot disagree
/// for that kind.
///
/// For `kind == "brain"` files the writer and validator instead use [`derive_brain_focus`],
/// which computes the children-union (over each in-scope child's own `derive_focus`) folded
/// with the brain file's **own** `tracks[]`-derived focus (via a `derive_focus` call on the
/// brain file itself, for "dual-role" brains that also author their own tracks). So the
/// single-derivation invariant holds per-kind: project files compare directly against
/// `derive_focus`; brain files compare against `derive_brain_focus`, which itself calls
/// `derive_focus` once per contributing source (self + each in-scope child) and unions the
/// results.
///
/// Returns an empty [`DerivedFocus`] for files with an empty `tracks[]` (the derivation
/// is undefined when there is no roadmap catalog — typically trackless tier brain files).
///
/// **Derivation rules:**
/// - `now` — every `tracks[]` block with authored `status == "in_progress"`.
/// - `blocked` — every `tracks[]` block that is `open` and has at least one unmet
///   dependency: any `External` dep, or any `Block` dep whose target is not `closed`.
///   The returned `blocked` entry carries only the **unmet** subset, not the full
///   `depends_on` list.
/// - `next` — every `tracks[]` block returned by [`ready_order`] for this file
///   (open blocks with no external deps and all block deps `closed`), in wave order.
/// - `deferred` — every `tracks[]` block with authored `status == "deferred"`.
///
/// **Why `deferred` is terminal, not a flavour of `blocked`.** A deferred block with
/// unmet dependencies lands in `deferred` only, never in `blocked` — the unmet-subset
/// computation runs exclusively under the `"open"` arm, exactly as `in_progress` blocks
/// are never reported as blocked. Deferral is a statement about *attention*, not about
/// *readiness*, so it wins over whatever the DAG says. Conversely, deferral does not
/// propagate: an `open` block that depends on a deferred block is still `blocked`,
/// because the dep is not `closed`.
pub fn derive_focus(
    src: &StateSource,
    file: &StateFile,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> DerivedFocus {
    use std::collections::HashMap;

    if file.tracks.is_empty() {
        return DerivedFocus::default();
    }

    // Build a status map: "repo:id" → authored status (None = absent = open).
    let mut status_map: HashMap<String, Option<String>> = HashMap::new();
    for (s, f) in files {
        for track in &f.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", s.repo_slug, block.id);
                status_map.insert(key, block.status.clone());
            }
        }
    }

    let mut now: Vec<String> = Vec::new();
    let mut blocked: Vec<(String, Vec<BlockedBy>)> = Vec::new();
    let mut deferred: Vec<String> = Vec::new();

    for track in &file.tracks {
        for block in &track.blocks {
            let authored_status = block.status.as_deref().unwrap_or("open");

            match authored_status {
                "in_progress" => {
                    now.push(block.id.clone());
                }
                "deferred" => {
                    // Terminal — no dependency inspection. See the doc comment above.
                    deferred.push(block.id.clone());
                }
                "open" => {
                    // Collect the unmet subset of depends_on.
                    let unmet: Vec<BlockedBy> = block
                        .depends_on
                        .iter()
                        .filter(|d| match d {
                            BlockedBy::External { .. } => true,
                            BlockedBy::Block { repo, id, .. } => {
                                let dep_key = format!("{repo}:{id}");
                                status_map.get(&dep_key).and_then(|s| s.as_deref())
                                    != Some("closed")
                            }
                        })
                        .cloned()
                        .collect();
                    if !unmet.is_empty() {
                        blocked.push((block.id.clone(), unmet));
                    }
                }
                // `closed` and `blocked` (invalid authored, caught by
                // `E_STATE_AUTHORED_BLOCKED`) are skipped — they have no derived
                // lane. Any NEW authored status added to
                // `VALID_TRACK_BLOCK_STATUSES` must also get an arm above, or it
                // silently vanishes from every derived view with no diagnostic.
                _ => {}
            }
        }
    }

    // next = ready_order filtered to this file's blocks (returns canonical "repo:id" keys).
    let ready = ready_order(graph, files);
    let this_prefix = format!("{}:", src.repo_slug);
    let next: Vec<String> = ready
        .into_iter()
        .filter(|key| key.starts_with(&this_prefix))
        .map(|key| key[this_prefix.len()..].to_string())
        .collect();

    DerivedFocus {
        now,
        next,
        blocked,
        deferred,
    }
}

// ---------------------------------------------------------------------------
// Focus-drift check (task 5) — rewritten to delegate to derive_focus
// ---------------------------------------------------------------------------

/// Recompute the expected `focus` from `tracks[]` and warn when it disagrees
/// with the stored `focus` snapshot.
///
/// Delegates to [`derive_focus`] so the validator and the emitter share one
/// derivation and can never disagree.
///
/// Comparison is **block-id sets only** (mirrors [`check_rollup`]'s strategy —
/// cosmetic title/note differences are ignored).
///
/// **Severity:** warning only — focus is a derived view maintained by hand
/// (the warn→error flip is deferred until the `/log-work` writer exists).
/// Drift never causes exit 1.
///
/// Skips files with an empty `tracks[]` (the derivation is undefined when there
/// is no roadmap catalog — typically brain files whose focus comes from
/// aggregated child repos).
pub fn check_focus_drift(
    src: &StateSource,
    file: &StateFile,
    config: &BrainConfig,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Vec<Diagnostic> {
    use std::collections::HashSet;

    // Undefined for files without a roadmap catalog.
    if file.tracks.is_empty() {
        return vec![];
    }

    // Kind-aware expected derivation (Facet B): brain-kind files must be
    // checked against the same derivation the writer used to emit their
    // stored focus — `derive_brain_focus` (children-union + own tracks[]
    // folding, Facet A) — not the bare per-file `derive_focus`, which only
    // ever looks at the file's own tracks[] and can never agree with the
    // writer's children-union for a brain with in-scope children. Project-
    // kind files are unaffected and keep using `derive_focus` exactly as
    // before.
    // One entry per focus lane, in report order. Adding a fifth lane means
    // adding a row to FOCUS_LANES and one element to each array below — not
    // editing four parallel blocks that can drift apart.
    let derived_owned: [Vec<String>; FOCUS_LANES.len()] = if file.kind == "brain" {
        let scope = tier_scope_for(file, config);
        let derived = derive_brain_focus(src, file, &scope, config, graph, files);
        let ids = |blocks: &[Block]| blocks.iter().map(|b| b.id.clone()).collect();
        [
            ids(&derived.now),
            ids(&derived.next),
            ids(&derived.blocked),
            ids(&derived.deferred),
        ]
    } else {
        let derived = derive_focus(src, file, graph, files);
        [
            derived.now.clone(),
            derived.next.clone(),
            derived.blocked.iter().map(|(id, _)| id.clone()).collect(),
            derived.deferred.clone(),
        ]
    };

    let stored_lanes: [&[Block]; FOCUS_LANES.len()] = [
        &file.focus.now,
        &file.focus.next,
        &file.focus.blocked,
        &file.focus.deferred,
    ];

    // Compare stored focus to derived (block-id sets only), building a compact
    // per-lane diff for the warning message.
    let mut diffs: Vec<String> = Vec::new();
    for ((lane, stored_blocks), derived_ids) in FOCUS_LANES
        .iter()
        .zip(stored_lanes.iter())
        .zip(derived_owned.iter())
    {
        let stored: HashSet<&str> = stored_blocks.iter().map(|b| b.id.as_str()).collect();
        let derived: HashSet<&str> = derived_ids.iter().map(|s| s.as_str()).collect();
        if stored != derived {
            diffs.push(format!(
                "{lane}: stored={:?} derived={:?}",
                sorted_set(&stored),
                sorted_set(&derived),
            ));
        }
    }

    if diffs.is_empty() {
        return vec![];
    }

    vec![Diagnostic::warning(
        &src.abs_path,
        "W_STATE_FOCUS_DRIFT",
        format!(
            "focus snapshot has drifted from tracks[] derivation — {}",
            diffs.join("; ")
        ),
    )]
}

// ---------------------------------------------------------------------------
// Cross-repo edge derivation (MV.3B.T)
// ---------------------------------------------------------------------------

/// Derive cross-repo block dependency edges from all loaded state files.
///
/// For every `tracks[].blocks[].depends_on` entry of `{type:"block"}` where the
/// dependency's `repo` differs from the owning repo, produce a [`CrossRepoEdge`].
/// Same-repo deps and `{type:"external"}` entries are skipped — they are not
/// cross-repo structural edges.
pub fn derive_cross_repo(files: &[(StateSource, StateFile)]) -> Vec<CrossRepoEdge> {
    let mut edges: Vec<CrossRepoEdge> = Vec::new();

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                for dep in &block.depends_on {
                    if let BlockedBy::Block { repo, id, what } = dep
                        && repo != &src.repo_slug
                    {
                        edges.push(CrossRepoEdge {
                            from: Endpoint {
                                repo: src.repo_slug.clone(),
                                id: block.id.clone(),
                            },
                            to: Endpoint {
                                repo: repo.clone(),
                                id: id.clone(),
                            },
                            note: what.clone(),
                        });
                    }
                }
            }
        }
    }

    edges
}

// ---------------------------------------------------------------------------
// Epic-scoped derivations
// ---------------------------------------------------------------------------

/// Filter an already-derived [`Focus`] down to one epic's member blocks.
///
/// Takes the *output* of [`derive_brain_focus`] rather than re-deriving, so an
/// epic board can never disagree with the unified board about what is now, next,
/// blocked, or deferred — it is the same union, just narrower. Order (and
/// therefore the unified board's effective-priority sort) is preserved.
///
/// Membership is read off each [`Block`]'s `epics`, which
/// [`derive_brain_focus`] carries through from the authoring `tracks[]` entry.
pub fn derive_epic_focus(focus: &Focus, slug: &str) -> Focus {
    let member = |b: &Block| b.epics.iter().any(|s| s == slug);
    Focus {
        now: focus.now.iter().filter(|b| member(b)).cloned().collect(),
        next: focus.next.iter().filter(|b| member(b)).cloned().collect(),
        blocked: focus
            .blocked
            .iter()
            .filter(|b| member(b))
            .cloned()
            .collect(),
        deferred: focus
            .deferred
            .iter()
            .filter(|b| member(b))
            .cloned()
            .collect(),
    }
}

/// One `depends_on` edge crossing an epic's boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct EpicEdge {
    /// `"repo:id"` of the dependent block (the one waiting).
    pub from: String,
    /// `"repo:id"` of the dependency block (the one being waited on).
    pub to: String,
    /// The epic slugs of whichever endpoint lies *outside* the epic being
    /// derived. Empty when that block belongs to no epic at all.
    pub other_epics: Vec<String>,
    /// Whether this edge still gates anything — true while the dependency is
    /// not `closed` and the dependent is not `closed`.
    pub blocking: bool,
}

/// Every cross-epic `depends_on` edge for one epic, split by direction.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EpicEdges {
    /// Edges where a member block depends on something outside the epic —
    /// what this initiative is waiting on.
    pub outbound: Vec<EpicEdge>,
    /// Edges where something outside the epic depends on a member block —
    /// what this initiative is holding up.
    pub inbound: Vec<EpicEdge>,
}

/// Derive one epic's relationships to the rest of the corpus from the block
/// `depends_on` graph.
///
/// **Nothing here is authored.** Epic-to-epic relationships are computed from
/// the same edges that already drive blocked-ness, so they cannot drift from the
/// work graph the way a hand-maintained epic-level `depends_on` would.
///
/// A block belonging to *both* endpoints' epics is "inside" for that epic, so a
/// shared block never produces a self-edge. Edges whose target does not resolve
/// are skipped (`check_state_graph` reports those); `external` deps have no
/// target node and are skipped too.
///
/// Both lists are returned in full — including already-satisfied edges — so
/// callers can render either the live gates or the complete relationship map.
/// Use [`EpicEdge::blocking`] to tell them apart.
pub fn derive_epic_edges(files: &[(StateSource, StateFile)], slug: &str) -> EpicEdges {
    use std::collections::HashMap;

    let mut by_key: HashMap<String, &TrackBlock> = HashMap::new();
    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                by_key.insert(format!("{}:{}", src.repo_slug, block.id), block);
            }
        }
    }

    let in_epic = |b: &TrackBlock| b.epics.iter().any(|s| s == slug);
    let is_closed = |b: &TrackBlock| b.status.as_deref() == Some("closed");

    let mut edges = EpicEdges::default();

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let from_key = format!("{}:{}", src.repo_slug, block.id);
                for dep in &block.depends_on {
                    let BlockedBy::Block { repo, id, .. } = dep else {
                        continue;
                    };
                    let to_key = format!("{repo}:{id}");
                    let Some(target) = by_key.get(&to_key) else {
                        continue;
                    };

                    let (from_in, to_in) = (in_epic(block), in_epic(target));
                    if from_in == to_in {
                        continue; // wholly inside or wholly outside — not a boundary
                    }

                    let edge = EpicEdge {
                        from: from_key.clone(),
                        to: to_key,
                        other_epics: if from_in {
                            target.epics.clone()
                        } else {
                            block.epics.clone()
                        },
                        blocking: !is_closed(target) && !is_closed(block),
                    };

                    if from_in {
                        edges.outbound.push(edge);
                    } else {
                        edges.inbound.push(edge);
                    }
                }
            }
        }
    }

    edges
}

// ---------------------------------------------------------------------------
// Tier scoping (MV.3B.U)
// ---------------------------------------------------------------------------

/// The set of `[[repos]]` a brain file's `repos[]` rollup should be scoped to.
///
/// Determined by [`tier_scope_for`]: a brain file whose `repo` slug matches a
/// `tier` value declared in `brain.toml` scopes to just that tier; a brain file
/// whose `repo` slug matches no tier (the HQ root) scopes to every repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierScope {
    /// Scope to only the `[[repos]]` entries whose `tier` equals this value.
    Tier(String),
    /// Scope to every `[[repos]]` entry (the HQ root).
    All,
}

/// Determine the [`TierScope`] a brain file's `repos[]` rollup should use.
///
/// `brain_file.repo` is matched against the set of `tier` values declared across
/// `config.repos[]`. A match yields [`TierScope::Tier`] (that single tier); no
/// match (e.g. the HQ root, whose `repo` is not itself a tier name) yields
/// [`TierScope::All`].
pub fn tier_scope_for(brain_file: &StateFile, config: &BrainConfig) -> TierScope {
    // A node scopes to a single tier if either:
    //  (a) some repo declares it as a `tier` value — the common case, where the
    //      tier has child repos carrying `tier = "<name>"`; or
    //  (b) it is registered as a tier-container self-entry (`slug == repo_path ==
    //      "<name>"`) — a childless brain tier, e.g. a document-only sub-brain
    //      whose own `[[repos]]` entry is the only thing naming it. Without this,
    //      such a tier matches no `tier` value and is wrongly scoped as the HQ
    //      root (`All`), producing a spurious hq-board emit.
    // The HQ root itself (`repo_path == "."`) never matches (b) and correctly
    // resolves to `All`.
    let is_tier_name = config.repos.iter().any(|r| r.tier == brain_file.repo);
    let is_tier_container = config
        .repos
        .iter()
        .any(|r| r.slug == brain_file.repo && r.repo_path == brain_file.repo);
    if is_tier_name || is_tier_container {
        TierScope::Tier(brain_file.repo.clone())
    } else {
        TierScope::All
    }
}

// ---------------------------------------------------------------------------
// Rollup derivation (MV.3B.T, tier-scoped in MV.3B.U)
// ---------------------------------------------------------------------------

/// Derive the brain `repos[]` rollup, tier-scoped and non-destructive.
///
/// Iterates the **in-scope** `config.repos[]` entries (filtered by `scope`, in
/// config order) and, for each, produces one [`RepoRollup`]:
/// - If a loadable `kind == "project"` child exists in `files` for that slug,
///   derive its headline via [`derive_focus`] (as before) and set
///   `tier: Some(<config tier>)`.
/// - Else if `existing` (the brain file's current `repos[]`) already has an
///   entry for that slug, **preserve it verbatim** (backfilling `tier` from
///   config only when it was previously `None`). This is what prevents a
///   malformed or not-yet-authored child `state.json` from silently dropping
///   the repo out of the rollup.
/// - Else, emit a tier-tagged empty stub.
///
/// `graph` and `files` are forwarded to [`derive_focus`] / [`ready_order`] so
/// cross-repo dependency statuses are resolved correctly.
pub fn derive_rollup(
    scope: &TierScope,
    config: &BrainConfig,
    existing: &[RepoRollup],
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Vec<RepoRollup> {
    let in_scope = config.repos.iter().filter(|r| match scope {
        TierScope::Tier(t) => &r.tier == t,
        TierScope::All => true,
    });

    in_scope
        .map(|entry| {
            let child = files
                .iter()
                .find(|(src, f)| src.repo_slug == entry.slug && f.kind == "project");

            if let Some((src, file)) = child {
                let derived = derive_focus(src, file, graph, files);

                // Build a title lookup from this child's tracks[].
                let mut title_map: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                for track in &file.tracks {
                    for block in &track.blocks {
                        title_map.insert(block.id.clone(), block.title.clone());
                    }
                }
                let title_of = |id: &str| title_map.get(id).cloned().unwrap_or_default();

                // `epics` stays empty here on purpose: `repos[]` is the per-repo
                // headline cache, and the epic board filters `derive_brain_focus`'s
                // union, not this. Leaving it empty means the field is skipped on
                // serialization, so tagging blocks causes zero churn in `repos[]`.
                let now = derived
                    .now
                    .iter()
                    .map(|id| Block {
                        due: None,
                        priority: None,
                        id: id.clone(),
                        title: title_of(id),
                        status: Some("in_progress".to_string()),
                        note: None,
                        repo: None,
                        blocked_by: Vec::new(),
                        epics: Vec::new(),
                    })
                    .collect();

                let next = derived
                    .next
                    .iter()
                    .map(|id| Block {
                        due: None,
                        priority: None,
                        id: id.clone(),
                        title: title_of(id),
                        status: None,
                        note: None,
                        repo: None,
                        blocked_by: Vec::new(),
                        epics: Vec::new(),
                    })
                    .collect();

                let blocked = derived
                    .blocked
                    .iter()
                    .map(|(id, unmet)| Block {
                        due: None,
                        priority: None,
                        id: id.clone(),
                        title: title_of(id),
                        status: None,
                        note: None,
                        repo: None,
                        blocked_by: unmet.clone(),
                        epics: Vec::new(),
                    })
                    .collect();

                let deferred = derived
                    .deferred
                    .iter()
                    .map(|id| Block {
                        due: None,
                        priority: None,
                        id: id.clone(),
                        title: title_of(id),
                        status: Some("deferred".to_string()),
                        note: None,
                        repo: None,
                        blocked_by: Vec::new(),
                        epics: Vec::new(),
                    })
                    .collect();

                RepoRollup {
                    repo: entry.slug.clone(),
                    tier: Some(entry.tier.clone()),
                    now,
                    next,
                    blocked,
                    deferred,
                }
            } else if let Some(preserved) = existing.iter().find(|r| r.repo == entry.slug) {
                let mut preserved = preserved.clone();
                if preserved.tier.is_none() {
                    preserved.tier = Some(entry.tier.clone());
                }
                preserved
            } else {
                RepoRollup {
                    repo: entry.slug.clone(),
                    tier: Some(entry.tier.clone()),
                    now: Vec::new(),
                    next: Vec::new(),
                    blocked: Vec::new(),
                    deferred: Vec::new(),
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Shared block-lookup helpers (derive_brain_focus + the epic derivations)
// ---------------------------------------------------------------------------

/// Index a state file's `tracks[].blocks[]` by block id.
///
/// `focus` is a set of bare ids; every derivation that materializes it back into
/// [`Block`] entries needs the authored title / `priority` / `due` / `epics` from
/// `tracks[]`. Later blocks with a duplicate id overwrite earlier ones — a
/// duplicate is already an error (`E_STATE_DUPLICATE_BLOCK_ID`), so which one
/// wins is not load-bearing.
fn track_block_index(file: &StateFile) -> std::collections::HashMap<&str, &TrackBlock> {
    let mut index = std::collections::HashMap::new();
    for track in &file.tracks {
        for block in &track.blocks {
            index.insert(block.id.as_str(), block);
        }
    }
    index
}

/// Materialize one derived-focus id into a [`Block`], pulling the authored
/// title / `priority` / `due` / `epics` out of `index`.
///
/// An id absent from `index` (only possible for a dangling focus entry, already
/// reported as `E_STATE_DANGLING_FOCUS`) yields an empty title and no metadata
/// rather than panicking.
fn focus_block(
    id: &str,
    index: &std::collections::HashMap<&str, &TrackBlock>,
    repo: Option<String>,
    status: Option<String>,
    blocked_by: Vec<BlockedBy>,
) -> Block {
    let authored = index.get(id);
    Block {
        id: id.to_string(),
        title: authored.map(|b| b.title.clone()).unwrap_or_default(),
        status,
        note: None,
        repo,
        blocked_by,
        priority: authored.and_then(|b| b.priority),
        due: authored.and_then(|b| b.due.clone()),
        epics: authored.map(|b| b.epics.clone()).unwrap_or_default(),
    }
}

/// Derive a brain file's `focus.now/next/blocked` as the repo-tagged union of
/// its in-scope children's derived focus (MV.3B.U task 2).
///
/// Iterates the **in-scope** `config.repos[]` entries (filtered by `scope`, in
/// config order); for each repo with a loadable `kind == "project"` child in
/// `files`, calls [`derive_focus`] and appends its `now`/`next`/`blocked`
/// blocks — each tagged with `repo: Some(<slug>)` — to the corresponding
/// brain-level list, in the child's own within-focus order.
///
/// Repos with no loadable child contribute nothing (mirrors [`derive_rollup`]'s
/// preserve/stub branches, which operate on the cached `repos[]` headline, not
/// `focus`, since there is no live tracks[] to derive from).
///
/// Deduplicated by `(repo, id)` within each of `now`/`next`/`blocked`/`deferred`
/// independently — the first occurrence wins.
pub fn derive_brain_focus(
    self_src: &StateSource,
    self_file: &StateFile,
    scope: &TierScope,
    config: &BrainConfig,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Focus {
    use std::collections::HashSet;

    let in_scope = config.repos.iter().filter(|r| match scope {
        TierScope::Tier(t) => &r.tier == t,
        TierScope::All => true,
    });

    let mut now: Vec<Block> = Vec::new();
    let mut next: Vec<Block> = Vec::new();
    let mut blocked: Vec<Block> = Vec::new();
    let mut deferred: Vec<Block> = Vec::new();
    let mut seen_now: HashSet<(String, String)> = HashSet::new();
    let mut seen_next: HashSet<(String, String)> = HashSet::new();
    let mut seen_blocked: HashSet<(String, String)> = HashSet::new();
    let mut seen_deferred: HashSet<(String, String)> = HashSet::new();

    for entry in in_scope {
        // A registered repo is either a leaf ("project") or a tier sub-brain
        // root ("brain", `tier = "_root"` in brain.toml) that carries its own
        // authored `tracks[]` (e.g. `business`'s "Business Ops" BZ.* track,
        // D43). Both fold into the union the same way; a "brain" entry with
        // empty `tracks[]` is a no-op via `derive_focus`'s short-circuit, so
        // this is byte-identical for the pure container tiers (core, side,
        // client, portfolio).
        let child = files.iter().find(|(src, f)| {
            src.repo_slug == entry.slug && (f.kind == "project" || f.kind == "brain")
        });

        let Some((src, file)) = child else {
            continue;
        };

        let derived = derive_focus(src, file, graph, files);

        // Index this child's tracks[] for the title/priority/due/epics lookups.
        let index = track_block_index(file);

        for id in &derived.now {
            let key = (entry.slug.clone(), id.clone());
            if seen_now.insert(key) {
                now.push(focus_block(
                    id,
                    &index,
                    Some(entry.slug.clone()),
                    Some("in_progress".to_string()),
                    Vec::new(),
                ));
            }
        }

        for id in &derived.next {
            let key = (entry.slug.clone(), id.clone());
            if seen_next.insert(key) {
                next.push(focus_block(
                    id,
                    &index,
                    Some(entry.slug.clone()),
                    None,
                    Vec::new(),
                ));
            }
        }

        for (id, unmet) in &derived.blocked {
            let key = (entry.slug.clone(), id.clone());
            if seen_blocked.insert(key) {
                blocked.push(focus_block(
                    id,
                    &index,
                    Some(entry.slug.clone()),
                    None,
                    unmet.clone(),
                ));
            }
        }

        for id in &derived.deferred {
            let key = (entry.slug.clone(), id.clone());
            if seen_deferred.insert(key) {
                deferred.push(focus_block(
                    id,
                    &index,
                    Some(entry.slug.clone()),
                    Some("deferred".to_string()),
                    Vec::new(),
                ));
            }
        }
    }

    // Facet A — dual-role folding: fold the brain file's OWN tracks[]-derived
    // focus in as well (tagged with the self repo slug), deduped alongside the
    // children via the same seen_* sets. A brain with empty own tracks[] folds
    // nothing here (derive_focus short-circuits to DerivedFocus::default()),
    // so this is a byte-identical no-op for the pure tier sub-brains.
    let self_derived = derive_focus(self_src, self_file, graph, files);
    let self_slug = &self_src.repo_slug;

    // Index the self file's own tracks[] for the same lookups.
    let self_index = track_block_index(self_file);

    for id in &self_derived.now {
        let key = (self_slug.clone(), id.clone());
        if seen_now.insert(key) {
            now.push(focus_block(
                id,
                &self_index,
                Some(self_slug.clone()),
                Some("in_progress".to_string()),
                Vec::new(),
            ));
        }
    }

    for id in &self_derived.next {
        let key = (self_slug.clone(), id.clone());
        if seen_next.insert(key) {
            next.push(focus_block(
                id,
                &self_index,
                Some(self_slug.clone()),
                None,
                Vec::new(),
            ));
        }
    }

    for (id, unmet) in &self_derived.blocked {
        let key = (self_slug.clone(), id.clone());
        if seen_blocked.insert(key) {
            blocked.push(focus_block(
                id,
                &self_index,
                Some(self_slug.clone()),
                None,
                unmet.clone(),
            ));
        }
    }

    for id in &self_derived.deferred {
        let key = (self_slug.clone(), id.clone());
        if seen_deferred.insert(key) {
            deferred.push(focus_block(
                id,
                &self_index,
                Some(self_slug.clone()),
                Some("deferred".to_string()),
                Vec::new(),
            ));
        }
    }

    Focus {
        now,
        next,
        blocked,
        deferred,
    }
}

/// Return a deterministically sorted `Vec<&str>` from a `HashSet<&str>` for
/// use in diagnostic messages (avoids non-deterministic output from Set debug).
fn sorted_set<'a>(set: &std::collections::HashSet<&'a str>) -> Vec<&'a str> {
    let mut v: Vec<&str> = set.iter().copied().collect();
    v.sort_unstable();
    v
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Minimal fixture strings (representative of the five live state.json files)
    // -----------------------------------------------------------------------

    /// Minimal leaf state.json (mev / bastion / orchestrator shape).
    pub(crate) fn leaf_json(repo: &str) -> String {
        format!(
            r#"{{
  "repo": "{repo}",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {{
    "now": [
      {{ "id": "MV.3.J", "title": "Graph integrity", "status": "closed" }}
    ],
    "next": [
      {{ "id": "MV.3.K", "title": "Link integrity" }}
    ],
    "blocked": []
  }},
  "tracks": [
    {{
      "title": "Phase 3",
      "blocks": [
        {{ "id": "MV.3.J", "title": "Graph integrity", "status": "closed" }},
        {{ "id": "MV.3.K", "title": "Link integrity", "status": "open" }}
      ]
    }}
  ]
}}"#
        )
    }

    /// Minimal core-brain state.json shape.
    fn core_brain_json() -> &'static str {
        r#"{
  "repo": "core",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": {
    "now": [
      { "id": "BA.11.C0", "repo": "bastion", "title": "Manifest engine", "status": "in_progress" }
    ],
    "next": [
      { "id": "BA.11.C", "repo": "bastion", "title": "WebSocket hub" }
    ],
    "blocked": []
  },
  "repos": [
    {
      "repo": "bastion",
      "now": [{ "id": "BA.11.C0", "title": "Manifest engine", "status": "in_progress" }],
      "next": [{ "id": "BA.11.C", "title": "WebSocket hub" }],
      "blocked": []
    }
  ],
  "cross_repo": [
    {
      "from": { "repo": "bastion-ui", "id": "BU.1.A" },
      "to": { "repo": "bastion", "id": "BA.11.C" },
      "note": "Session-control screens need the WS hub."
    }
  ]
}"#
    }

    /// Minimal HQ brain state.json shape (adds `tiers[]` and `note`).
    fn hq_brain_json() -> &'static str {
        r#"{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-06-29",
  "note": "Top company brain.",
  "focus": {
    "now": [
      { "id": "BA.11.C0", "repo": "bastion", "title": "Manifest engine", "status": "in_progress" }
    ],
    "next": [],
    "blocked": [
      {
        "id": "OR.B",
        "repo": "orchestrator",
        "title": "Semantic Brain Q&A",
        "blocked_by": [
          { "type": "external", "what": "At-home Ollama session" }
        ]
      }
    ]
  },
  "repos": [
    {
      "repo": "bastion",
      "tier": "core",
      "now": [{ "id": "BA.11.C0", "title": "Manifest engine", "status": "in_progress" }],
      "next": [],
      "blocked": []
    }
  ],
  "cross_repo": [],
  "tiers": [
    { "tier": "core", "rollup": "core", "summary": "Primary program — Bastion + tooling." }
  ]
}"#
    }

    // -----------------------------------------------------------------------
    // Deserialize clean: three leaf shapes
    // -----------------------------------------------------------------------

    #[test]
    fn leaf_mev_shape_deserializes_clean() {
        let json = leaf_json("mev");
        let file: StateFile = serde_json::from_str(&json).expect("mev leaf should deserialize");
        assert_eq!(file.repo, "mev");
        assert_eq!(file.kind, "project");
        assert_eq!(file.focus.now.len(), 1);
        assert_eq!(file.focus.next.len(), 1);
        assert_eq!(file.tracks.len(), 1);
        assert_eq!(file.tracks[0].blocks.len(), 2);
    }

    #[test]
    fn leaf_bastion_shape_deserializes_clean() {
        let json = leaf_json("bastion");
        let file: StateFile = serde_json::from_str(&json).expect("bastion leaf should deserialize");
        assert_eq!(file.repo, "bastion");
        assert_eq!(file.kind, "project");
    }

    #[test]
    fn leaf_orchestrator_shape_deserializes_clean() {
        let json = leaf_json("orchestrator");
        let file: StateFile =
            serde_json::from_str(&json).expect("orchestrator leaf should deserialize");
        assert_eq!(file.repo, "orchestrator");
        assert_eq!(file.kind, "project");
    }

    // -----------------------------------------------------------------------
    // Deserialize clean: two brain shapes
    // -----------------------------------------------------------------------

    #[test]
    fn core_brain_shape_deserializes_clean() {
        let file: StateFile =
            serde_json::from_str(core_brain_json()).expect("core brain should deserialize");
        assert_eq!(file.repo, "core");
        assert_eq!(file.kind, "brain");
        assert_eq!(file.repos.len(), 1);
        assert_eq!(file.cross_repo.len(), 1);
        assert!(file.tiers.is_empty());
    }

    #[test]
    fn hq_brain_shape_deserializes_clean() {
        let file: StateFile =
            serde_json::from_str(hq_brain_json()).expect("HQ brain should deserialize");
        assert_eq!(file.repo, "hq");
        assert_eq!(file.kind, "brain");
        assert_eq!(file.tiers.len(), 1);
        assert_eq!(file.tiers[0].tier, "core");
        assert_eq!(file.note.as_deref(), Some("Top company brain."));
    }

    // -----------------------------------------------------------------------
    // BlockedBy variants
    // -----------------------------------------------------------------------

    #[test]
    fn blocked_by_block_type_deserializes() {
        let json =
            r#"{ "type": "block", "repo": "bastion", "id": "BA.11.C", "what": "needs WS hub" }"#;
        let bb: BlockedBy = serde_json::from_str(json).expect("block type should deserialize");
        match bb {
            BlockedBy::Block { repo, id, what } => {
                assert_eq!(repo, "bastion");
                assert_eq!(id, "BA.11.C");
                assert_eq!(what.as_deref(), Some("needs WS hub"));
            }
            _ => panic!("expected BlockedBy::Block"),
        }
    }

    #[test]
    fn blocked_by_external_type_deserializes() {
        let json = r#"{ "type": "external", "what": "At-home Mac Mini session" }"#;
        let bb: BlockedBy = serde_json::from_str(json).expect("external type should deserialize");
        match bb {
            BlockedBy::External { what } => {
                assert_eq!(what, "At-home Mac Mini session");
            }
            _ => panic!("expected BlockedBy::External"),
        }
    }

    #[test]
    fn blocked_by_unknown_type_is_rejected() {
        let json = r#"{ "type": "unknown_type", "what": "some value" }"#;
        let result = serde_json::from_str::<BlockedBy>(json);
        assert!(
            result.is_err(),
            "unknown blocked_by type should be rejected by serde, got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // load_state function tests (temp files)
    // -----------------------------------------------------------------------

    #[test]
    fn load_state_returns_parsed_file_for_valid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        std::fs::write(&path, leaf_json("mev")).unwrap();

        let file = load_state(&path).expect("load_state should succeed");
        assert_eq!(file.repo, "mev");
    }

    #[test]
    fn load_state_surfaces_parse_error_for_malformed_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        std::fs::write(&path, b"{ not valid json }").unwrap();

        let result = load_state(&path);
        assert!(
            matches!(result, Err(StateLoadError::Parse { .. })),
            "malformed JSON should produce StateLoadError::Parse, got: {result:?}"
        );
    }

    #[test]
    fn load_state_surfaces_io_error_for_missing_file() {
        let path = std::path::PathBuf::from("/nonexistent/path/state.json");
        let result = load_state(&path);
        assert!(
            matches!(result, Err(StateLoadError::Io { .. })),
            "missing file should produce StateLoadError::Io, got: {result:?}"
        );
    }

    #[test]
    fn load_state_rejects_unknown_blocked_by_type_in_full_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        // A state.json with an unknown blocked_by type in a focus.blocked entry.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [
      {
        "block": "TEST.1",
        "title": "Some block",
        "blocked_by": [
          { "type": "mystery_type", "what": "unknown" }
        ]
      }
    ]
  }
}"#;
        std::fs::write(&path, json).unwrap();
        let result = load_state(&path);
        assert!(
            matches!(result, Err(StateLoadError::Parse { .. })),
            "unknown blocked_by type in full file should produce StateLoadError::Parse, got: {result:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Focus defaults
    // -----------------------------------------------------------------------

    #[test]
    fn focus_defaults_to_empty_collections() {
        // A minimal file with no focus sub-fields.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {}
}"#;
        let file: StateFile = serde_json::from_str(json).expect("should deserialize");
        assert!(file.focus.now.is_empty());
        assert!(file.focus.next.is_empty());
        assert!(file.focus.blocked.is_empty());
    }

    // -----------------------------------------------------------------------
    // Optional collections default to empty
    // -----------------------------------------------------------------------

    #[test]
    fn optional_top_level_collections_default_to_empty() {
        // Minimal file with none of the optional arrays.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29"
}"#;
        let file: StateFile = serde_json::from_str(json).expect("should deserialize");
        assert!(file.tracks.is_empty());
        assert!(file.repos.is_empty());
        assert!(file.cross_repo.is_empty());
        assert!(file.tiers.is_empty());
        assert!(file.note.is_none());
    }

    // -----------------------------------------------------------------------
    // Task 2 — discover_state_files tests
    // -----------------------------------------------------------------------

    /// Build a minimal HQ temp dir with two leaf repos and a brain state.json.
    fn build_hq_fixture(dir: &std::path::Path) {
        // HQ brain planning/state.json
        let hq_planning = std::path::Path::new(dir).join("planning");
        std::fs::create_dir_all(&hq_planning).unwrap();
        let hq_state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [
                { "repo": "alpha", "now": [], "next": [], "blocked": [] }
            ],
            "tiers": [
                { "tier": "core", "rollup": "core/planning/state.json" }
            ]
        });
        std::fs::write(
            hq_planning.join("state.json"),
            serde_json::to_string_pretty(&hq_state).unwrap(),
        )
        .unwrap();

        // Tier sub-brain: core/planning/state.json
        let core_planning = dir.join("core").join("planning");
        std::fs::create_dir_all(&core_planning).unwrap();
        let core_state = serde_json::json!({
            "repo": "core",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [{ "repo": "alpha", "now": [], "next": [], "blocked": [] }]
        });
        std::fs::write(
            core_planning.join("state.json"),
            serde_json::to_string_pretty(&core_state).unwrap(),
        )
        .unwrap();

        // Leaf repo alpha: core/alpha/planning/state.json
        let alpha_planning = dir.join("core").join("alpha").join("planning");
        std::fs::create_dir_all(&alpha_planning).unwrap();
        let alpha_state = serde_json::json!({
            "repo": "alpha",
            "kind": "project",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "tracks": [{ "title": "Phase 1", "blocks": [{ "id": "AL.1.A", "title": "Start" }] }]
        });
        std::fs::write(
            alpha_planning.join("state.json"),
            serde_json::to_string_pretty(&alpha_state).unwrap(),
        )
        .unwrap();
    }

    fn make_config_with_alpha(alpha_repo_path: &str) -> BrainConfig {
        use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
        BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: alpha_repo_path.to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
            ],
        }
    }

    #[test]
    fn discover_finds_hq_tier_and_leaf_sources() {
        let dir = tempfile::tempdir().expect("tempdir");
        build_hq_fixture(dir.path());
        let config = make_config_with_alpha("core/alpha");

        let (sources, diags) = discover_state_files(dir.path(), &config);

        // 0 warnings expected (all files exist)
        assert!(
            diags.is_empty(),
            "expected no diagnostics for complete fixture, got: {diags:?}"
        );

        // Expect 3 sources: HQ brain, core tier brain, alpha leaf
        assert_eq!(
            sources.len(),
            3,
            "expected 3 sources (hq, core, alpha), got: {sources:?}"
        );

        let slugs: Vec<&str> = sources.iter().map(|s| s.repo_slug.as_str()).collect();
        assert!(
            slugs.contains(&"brain"),
            "expected hq source with slug 'brain'"
        );
        assert!(
            slugs.contains(&"core"),
            "expected tier source with slug 'core'"
        );
        assert!(
            slugs.contains(&"alpha"),
            "expected leaf source with slug 'alpha'"
        );

        // Verify expected_kinds
        for src in &sources {
            match src.repo_slug.as_str() {
                "brain" | "core" => assert_eq!(src.expected_kind, "brain"),
                "alpha" => assert_eq!(src.expected_kind, "project"),
                _ => panic!("unexpected slug: {}", src.repo_slug),
            }
        }
    }

    /// Mirrors the real HQ `brain.toml`, which carries a `[[repos]]` entry for
    /// `core` itself (`tier = "_root"`, `repo_path = "core"`) *in addition to*
    /// HQ's `tiers[].rollup` pointing at the same `core/planning/state.json`.
    /// Without dedup, `core` was discovered twice with conflicting
    /// `expected_kind` ("brain" via the tier rollup, "project" via the
    /// `[[repos]]` loop), producing a false `E_STATE_SCHEMA_BAD_KIND`.
    #[test]
    fn discover_dedupes_repo_entry_that_shadows_a_tier_rollup() {
        let dir = tempfile::tempdir().expect("tempdir");
        build_hq_fixture(dir.path());

        use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
        let config = BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "core".to_string(),
                    tier: "_root".to_string(),
                    repo_path: "core".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/alpha".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
            ],
        };

        let (sources, diags) = discover_state_files(dir.path(), &config);
        assert!(
            diags.is_empty(),
            "expected no diagnostics for complete fixture, got: {diags:?}"
        );

        // Still exactly 3 sources — the `core` [[repos]] entry must not add a
        // second, conflicting registration of the same file.
        assert_eq!(
            sources.len(),
            3,
            "expected core/planning/state.json to be registered once, got: {sources:?}"
        );
        let core_sources: Vec<_> = sources.iter().filter(|s| s.repo_slug == "core").collect();
        assert_eq!(
            core_sources.len(),
            1,
            "expected exactly one 'core' source, got: {core_sources:?}"
        );
        assert_eq!(core_sources[0].expected_kind, "brain");
    }

    #[test]
    fn discover_assigns_portfolio_expected_kind_for_portfolio_tier() {
        use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};

        let dir = tempfile::tempdir().expect("tempdir");
        build_hq_fixture(dir.path());

        // A leaf repo tagged tier:"portfolio" — terminal, no tracks[] expected.
        let re_planning = dir.path().join("portfolio").join("re-rs").join("planning");
        std::fs::create_dir_all(&re_planning).unwrap();
        let re_state = serde_json::json!({
            "repo": "re-rs",
            "kind": "portfolio",
            "updated": "2026-07-02",
            "note": "Completed — live on GitHub",
            "focus": { "now": [], "next": [], "blocked": [] }
        });
        std::fs::write(
            re_planning.join("state.json"),
            serde_json::to_string_pretty(&re_state).unwrap(),
        )
        .unwrap();

        let config = BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "re-rs".to_string(),
                    tier: "portfolio".to_string(),
                    repo_path: "portfolio/re-rs".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
            ],
        };

        let (sources, diags) = discover_state_files(dir.path(), &config);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");

        let re_src = sources
            .iter()
            .find(|s| s.repo_slug == "re-rs")
            .expect("re-rs source should be discovered");
        assert_eq!(re_src.expected_kind, "portfolio");
    }

    #[test]
    fn discover_emits_missing_warning_for_absent_leaf() {
        let dir = tempfile::tempdir().expect("tempdir");
        build_hq_fixture(dir.path());
        // Add a second repo entry that has no state.json on disk
        use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
        let config = BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/alpha".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "missing-repo".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/missing-repo".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
            ],
        };

        let (sources, diags) = discover_state_files(dir.path(), &config);

        // One W_STATE_FILE_MISSING for the missing leaf
        let missing_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FILE_MISSING")
            .collect();
        assert_eq!(
            missing_diags.len(),
            1,
            "expected exactly one W_STATE_FILE_MISSING, got: {diags:?}"
        );

        // missing-repo is NOT in sources
        assert!(
            !sources.iter().any(|s| s.repo_slug == "missing-repo"),
            "missing-repo should not appear in sources"
        );
    }

    #[test]
    fn discover_emits_missing_warning_for_absent_tier_sub_brain() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Build HQ with a tier that has no actual state.json file on disk
        let hq_planning = dir.path().join("planning");
        std::fs::create_dir_all(&hq_planning).unwrap();
        let hq_state = serde_json::json!({
            "repo": "hq",
            "kind": "brain",
            "updated": "2026-06-29",
            "focus": { "now": [], "next": [], "blocked": [] },
            "repos": [],
            "tiers": [
                { "tier": "ghost-tier", "rollup": "ghost/planning/state.json" }
            ]
        });
        std::fs::write(
            hq_planning.join("state.json"),
            serde_json::to_string_pretty(&hq_state).unwrap(),
        )
        .unwrap();

        let config = BrainConfig::default();
        let (sources, diags) = discover_state_files(dir.path(), &config);

        let missing: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FILE_MISSING")
            .collect();
        assert_eq!(
            missing.len(),
            1,
            "expected one W_STATE_FILE_MISSING for ghost-tier, got: {diags:?}"
        );

        // HQ itself is in sources
        assert_eq!(sources.len(), 1, "only HQ brain should be in sources");
    }

    /// Facet 2 (state-load-error-surfacing): a malformed HQ root must not
    /// cascade into tier sub-brains being mis-registered `expected_kind:
    /// "project"`. Instead, tier paths are recovered from `brain.toml`'s
    /// tier-container self-entries and registered `expected_kind: "brain"`,
    /// alongside a single `E_STATE_ROOT_LOAD_FAILED` diagnostic.
    #[test]
    fn discover_recovers_tier_brains_from_config_when_root_fails_to_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        build_hq_fixture(dir.path());

        // Corrupt only the HQ root so `load_state` fails (the file exists —
        // this is a parse failure, not a missing file).
        let hq_path = dir.path().join("planning").join("state.json");
        std::fs::write(&hq_path, "{ not valid json").unwrap();

        // `core` is a tier-container self-entry (slug == repo_path == "core"),
        // matching the real `brain.toml` shape covered by
        // `discover_dedupes_repo_entry_that_shadows_a_tier_rollup`.
        use crate::brain::config::{BrainConfig, CrawlConfig, RepoEntry, VocabConfig};
        let config = BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "core".to_string(),
                    tier: "_root".to_string(),
                    repo_path: "core".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/alpha".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
            ],
        };

        let (sources, diags) = discover_state_files(dir.path(), &config);

        // Exactly one degraded-classification diagnostic; no W_STATE_FILE_MISSING
        // (every file on disk exists).
        let root_load_failed: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_ROOT_LOAD_FAILED")
            .collect();
        assert_eq!(
            root_load_failed.len(),
            1,
            "expected exactly one E_STATE_ROOT_LOAD_FAILED, got: {diags:?}"
        );

        // `core` must still be registered brain-kind, not cascaded to "project".
        let core_src = sources
            .iter()
            .find(|s| s.repo_slug == "core")
            .expect("core tier sub-brain should still be discovered");
        assert_eq!(
            core_src.expected_kind, "brain",
            "core tier sub-brain must not cascade to expected_kind \"project\""
        );

        // `alpha` (a plain leaf repo) is unaffected.
        let alpha_src = sources
            .iter()
            .find(|s| s.repo_slug == "alpha")
            .expect("alpha leaf source should be discovered");
        assert_eq!(alpha_src.expected_kind, "project");
    }

    // -----------------------------------------------------------------------
    // Task 2 — check_schema tests
    // -----------------------------------------------------------------------

    fn make_source(path: &std::path::Path, kind: &'static str) -> StateSource {
        StateSource {
            repo_slug: "test".to_string(),
            abs_path: path.to_path_buf(),
            expected_kind: kind,
        }
    }

    fn parse_file(json: &str) -> StateFile {
        serde_json::from_str(json).expect("fixture must parse")
    }

    // ---- Attention staleness (parse_state_date / carryover / backlog) ----

    fn day(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn parse_state_date_accepts_bare_and_rfc3339() {
        assert_eq!(parse_state_date("2026-07-05"), Some(day("2026-07-05")));
        // RFC3339 with offset → calendar date in that offset.
        assert_eq!(
            parse_state_date("2026-07-05T07:10:00-03:00"),
            Some(day("2026-07-05"))
        );
        assert_eq!(parse_state_date("not-a-date"), None);
        assert_eq!(parse_state_date("2026-13-99"), None);
    }

    #[test]
    fn carryover_staleness_fires_past_threshold_and_respects_snooze_and_reviewed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");
        let today = day("2026-07-15");
        let cfg = crate::brain::config::AttentionThresholds::default(); // deferred = 5

        // deferred created 2026-07-01 → 14d old > 5d → stale.
        let stale = parse_file(
            r#"{"repo":"mev","kind":"project","updated":"2026-07-15",
                "carryover":[{"slug":"old-thing","scope":{"repo":"mev"},"kind":"deferred",
                              "text":"x","created":"2026-07-01"}]}"#,
        );
        let d = check_carryover_staleness(&src, &stale, today, &cfg);
        assert_eq!(d.len(), 1, "14d-old deferred should be stale: {d:?}");
        assert_eq!(d[0].locator, "W_STATE_CARRYOVER_STALE");
        assert_eq!(d[0].severity, crate::Severity::Warning);

        // Same item but reviewed 2026-07-13 → age 2d < 5d → not stale.
        let reviewed = parse_file(
            r#"{"repo":"mev","kind":"project","updated":"2026-07-15",
                "carryover":[{"slug":"old-thing","scope":{"repo":"mev"},"kind":"deferred",
                              "text":"x","created":"2026-07-01","reviewed":"2026-07-13"}]}"#,
        );
        assert!(
            check_carryover_staleness(&src, &reviewed, today, &cfg).is_empty(),
            "reviewed within threshold resets the clock"
        );

        // Same item but snoozed_until in the future → suppressed regardless of age.
        let snoozed = parse_file(
            r#"{"repo":"mev","kind":"project","updated":"2026-07-15",
                "carryover":[{"slug":"old-thing","scope":{"repo":"mev"},"kind":"deferred",
                              "text":"x","created":"2026-07-01","snoozed_until":"2026-07-20"}]}"#,
        );
        assert!(
            check_carryover_staleness(&src, &snoozed, today, &cfg).is_empty(),
            "future snoozed_until suppresses the warning"
        );
    }

    #[test]
    fn carryover_staleness_permanent_constraint_still_nags() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");
        let today = day("2026-07-15");
        let cfg = crate::brain::config::AttentionThresholds::default(); // constraint = 10

        // A permanent constraint (no clears_when), created 30d ago → still nags.
        let file = parse_file(
            r#"{"repo":"mev","kind":"project","updated":"2026-07-15",
                "carryover":[{"slug":"perm","scope":{"repo":"mev"},"kind":"constraint",
                              "text":"always true","created":"2026-06-15"}]}"#,
        );
        let d = check_carryover_staleness(&src, &file, today, &cfg);
        assert_eq!(d.len(), 1, "permanent constraint past threshold nags");
    }

    #[test]
    fn backlog_staleness_hq_only_and_status_scoped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let today = day("2026-07-15");
        let cfg = crate::brain::config::AttentionThresholds::default(); // backlog = 7

        // Brain file: an idea 10d old (stale) + a promoted node (skipped).
        let brain = parse_file(
            r#"{"repo":"hq","kind":"brain","updated":"2026-07-15",
                "backlog":[
                  {"slug":"aged","title":"t","repo":"core","type":"research","status":"idea","created":"2026-07-05"},
                  {"slug":"done","title":"t","repo":"core","type":"feature","status":"promoted","block":"X.1.A","created":"2026-01-01"}
                ]}"#,
        );
        let brain_src = make_source(&path, "brain");
        let d = check_backlog_staleness(&brain_src, &brain, today, &cfg);
        assert_eq!(d.len(), 1, "only the aged idea is stale: {d:?}");
        assert_eq!(d[0].locator, "W_STATE_BACKLOG_STALE");

        // Same content in a project file → no backlog staleness (HQ-only).
        let leaf_src = make_source(&path, "project");
        let mut leaf = brain.clone();
        leaf.kind = "project".to_string();
        assert!(
            check_backlog_staleness(&leaf_src, &leaf, today, &cfg).is_empty(),
            "backlog staleness is brain-only"
        );
    }

    #[test]
    fn malformed_dates_raise_date_format_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");
        let file = parse_file(
            r#"{"repo":"mev","kind":"project","updated":"2026-07-15",
                "carryover":[{"slug":"bad","scope":{"repo":"mev"},"kind":"deferred",
                              "text":"x","created":"July 5th"}]}"#,
        );
        let d = check_schema(&src, &file);
        assert!(
            d.iter().any(|x| x.locator == "E_STATE_DATE_FORMAT"),
            "malformed created must raise E_STATE_DATE_FORMAT: {d:?}"
        );
    }

    #[test]
    fn check_schema_clean_leaf_emits_no_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");
        let file = parse_file(&leaf_json("mev"));

        let diags = check_schema(&src, &file);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "clean leaf should produce no errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_bad_kind_emits_e_state_schema_bad_kind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        let json = r#"{
  "repo": "test",
  "kind": "invalid_kind",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "T.1", "title": "X" }] }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let bad_kind: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_KIND")
            .collect();
        assert_eq!(
            bad_kind.len(),
            1,
            "expected exactly one E_STATE_SCHEMA_BAD_KIND, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_portfolio_kind_with_note_emits_no_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "portfolio");

        let json = r#"{
  "repo": "rag-engine-rs",
  "kind": "portfolio",
  "updated": "2026-07-02",
  "note": "Completed — live on GitHub",
  "focus": { "now": [], "next": [], "blocked": [] }
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        assert!(
            diags.is_empty(),
            "clean portfolio file should produce no diagnostics, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_portfolio_kind_missing_note_warns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "portfolio");

        let json = r#"{
  "repo": "rag-engine-rs",
  "kind": "portfolio",
  "updated": "2026-07-02",
  "focus": { "now": [], "next": [], "blocked": [] }
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let missing_note: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_MISSING_FIELD" && d.message.contains("note"))
            .collect();
        assert_eq!(
            missing_note.len(),
            1,
            "expected exactly one missing-note warning, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_portfolio_kind_mismatched_expected_kind_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        // Source expects "project" (e.g. tier misconfigured in brain.toml) but the
        // file declares "portfolio" — must be flagged, not silently accepted.
        let src = make_source(&path, "project");

        let json = r#"{
  "repo": "rag-engine-rs",
  "kind": "portfolio",
  "updated": "2026-07-02",
  "note": "Completed — live on GitHub",
  "focus": { "now": [], "next": [], "blocked": [] }
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let bad_kind: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_KIND")
            .collect();
        assert_eq!(
            bad_kind.len(),
            1,
            "expected exactly one E_STATE_SCHEMA_BAD_KIND for mismatched expected kind, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_bad_status_emits_e_state_schema_bad_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [
      { "id": "T.1", "title": "Work", "status": "flying" }
    ],
    "next": [],
    "blocked": []
  },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "T.1", "title": "Work" }] }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let bad_status: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_STATUS")
            .collect();
        assert_eq!(
            bad_status.len(),
            1,
            "expected exactly one E_STATE_SCHEMA_BAD_STATUS, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_blocked_by_empty_id_emits_e_state_schema_bad_blocked_by() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        // blocked_by entry with type "block" but empty id — parses fine, fails schema check
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [
      {
        "id": "T.1",
        "title": "Blocked block",
        "blocked_by": [
          { "type": "block", "repo": "other", "id": "" }
        ]
      }
    ]
  },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "T.1", "title": "Blocked block" }] }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let bad_bb: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_BLOCKED_BY")
            .collect();
        assert_eq!(
            bad_bb.len(),
            1,
            "expected exactly one E_STATE_SCHEMA_BAD_BLOCKED_BY for empty id, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_brain_clean_emits_no_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "brain");
        let file = parse_file(core_brain_json());

        let diags = check_schema(&src, &file);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "clean brain should produce no errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_project_missing_tracks_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] }
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        // Should have a warning about missing tracks
        let track_warnings: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.severity == crate::Severity::Warning
                    && d.locator == "E_STATE_SCHEMA_MISSING_FIELD"
            })
            .collect();
        assert!(
            !track_warnings.is_empty(),
            "should warn about missing tracks[], got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 — build_state_graph / check_state_graph tests
    // -----------------------------------------------------------------------

    /// Build a minimal (source, file) pair for use in graph tests.
    fn make_pair(
        dir: &std::path::Path,
        filename: &str,
        kind: &'static str,
        json: &str,
    ) -> (StateSource, StateFile) {
        let path = dir.join(filename);
        std::fs::write(&path, json).unwrap();
        let src = StateSource {
            repo_slug: serde_json::from_str::<serde_json::Value>(json).unwrap()["repo"]
                .as_str()
                .unwrap()
                .to_string(),
            abs_path: path,
            expected_kind: kind,
        };
        let file: StateFile = serde_json::from_str(json).expect("fixture must parse");
        (src, file)
    }

    /// Minimal brain state.json with empty `tracks[]` — the brain's own
    /// "self" derived focus is a no-op (`derive_focus` short-circuits to
    /// `DerivedFocus::default()`), so callers can pass this pair as
    /// `(self_src, self_file)` without perturbing the children-only assertions.
    fn empty_brain_pair(dir: &std::path::Path, repo: &str) -> (StateSource, StateFile) {
        let json = format!(
            r#"{{
  "repo": "{repo}",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": []
}}"#
        );
        make_pair(dir, &format!("{repo}-state.json"), "brain", &json)
    }

    /// Dual-role brain state.json: `kind: "brain"` that ALSO carries its own
    /// `tracks[]` (one `in_progress` block and one `open` block with an unmet
    /// external dep) — the fixture shape used to exercise Facet A's
    /// self-folding and Facet B's kind-aware drift check.
    fn dual_role_brain_pair(dir: &std::path::Path, repo: &str) -> (StateSource, StateFile) {
        let json = format!(
            r#"{{
  "repo": "{repo}",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [{{
    "title": "Own Track",
    "blocks": [
      {{ "id": "{repo_upper}.1.A", "title": "Own now work", "status": "in_progress" }},
      {{
        "id": "{repo_upper}.1.B",
        "title": "Own blocked work",
        "status": "open",
        "depends_on": [{{ "type": "external", "what": "upstream dep" }}]
      }}
    ]
  }}]
}}"#,
            repo_upper = repo.to_uppercase()
        );
        make_pair(dir, &format!("{repo}-state.json"), "brain", &json)
    }

    /// Minimal leaf state.json with one tracks block and a clean focus.
    fn leaf_pair(dir: &std::path::Path, repo: &str, block_id: &str) -> (StateSource, StateFile) {
        let json = format!(
            r#"{{
  "repo": "{repo}",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {{
    "now": [{{ "id": "{block_id}", "title": "Work", "status": "in_progress" }}],
    "next": [],
    "blocked": []
  }},
  "tracks": [{{
    "title": "Phase 1",
    "blocks": [{{ "id": "{block_id}", "title": "Work", "status": "in_progress" }}]
  }}]
}}"#
        );
        make_pair(dir, &format!("{repo}-state.json"), "project", &json)
    }

    #[test]
    fn build_and_check_clean_two_repo_fixture_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_a = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let pair_b = leaf_pair(dir.path(), "beta", "BE.1.A");
        let files = vec![pair_a, pair_b];

        let graph = build_state_graph(&files);

        // Two nodes, one per repo.
        assert_eq!(
            graph.nodes.len(),
            2,
            "expected 2 nodes, got: {:?}",
            graph.nodes
        );
        assert_eq!(graph.edges.len(), 0, "expected 0 edges for clean fixture");

        let diags = check_state_graph(&graph, &files);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "clean two-repo fixture should produce no errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_detects_dangling_blocked_by_to_nonexistent_id() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has block AL.1.A in tracks
        let pair_a = leaf_pair(dir.path(), "alpha", "AL.1.A");

        // beta's tracks[].blocks[].depends_on references alpha's block "AL.1.GHOST" which does NOT exist
        let beta_json = r#"{
  "repo": "beta",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [
      { "id": "BE.1.A", "title": "Blocked work" }
    ]
  },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{
      "id": "BE.1.A",
      "title": "Blocked work",
      "depends_on": [
        { "type": "block", "repo": "alpha", "id": "AL.1.GHOST" }
      ]
    }]
  }]
}"#;
        let pair_b = make_pair(dir.path(), "beta-state.json", "project", beta_json);

        let files = vec![pair_a, pair_b];
        let graph = build_state_graph(&files);
        let diags = check_state_graph(&graph, &files);

        let dangling: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_BLOCKED_BY")
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected exactly one E_STATE_DANGLING_BLOCKED_BY, got: {diags:?}"
        );
    }

    #[test]
    fn check_detects_unknown_repo_in_blocked_by() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Only one repo in the corpus; the block's depends_on references an unknown repo.
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [{ "id": "AL.1.A", "title": "Blocked" }]
  },
  "tracks": [{
    "title": "P1",
    "blocks": [{
      "id": "AL.1.A",
      "title": "Blocked",
      "depends_on": [
        { "type": "block", "repo": "ghost-repo", "id": "GH.1.X" }
      ]
    }]
  }]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);
        let diags = check_state_graph(&graph, &files);

        let unknown: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_UNKNOWN_REPO")
            .collect();
        assert_eq!(
            unknown.len(),
            1,
            "expected exactly one E_STATE_UNKNOWN_REPO, got: {diags:?}"
        );
    }

    #[test]
    fn check_detects_duplicate_block_id_in_tracks() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has block AL.1.A registered TWICE in tracks (two entries with same id)
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [
    {
      "title": "Phase 1",
      "blocks": [
        { "id": "AL.1.A", "title": "First" },
        { "id": "AL.1.A", "title": "Duplicate!" }
      ]
    }
  ]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);
        // Two nodes with the same key
        assert_eq!(graph.nodes.len(), 2, "builder emits both duplicate nodes");

        let diags = check_state_graph(&graph, &files);

        let dup: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DUPLICATE_BLOCK_ID")
            .collect();
        assert_eq!(
            dup.len(),
            1,
            "expected exactly one E_STATE_DUPLICATE_BLOCK_ID, got: {diags:?}"
        );
    }

    #[test]
    fn check_detects_dangling_focus_in_leaf_file() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha's focus.now references "AL.1.GHOST" which is not in tracks
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [{ "id": "AL.1.GHOST", "title": "Missing", "status": "in_progress" }],
    "next": [],
    "blocked": []
  },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "AL.1.A", "title": "Real block" }] }]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);
        let diags = check_state_graph(&graph, &files);

        let dangling_focus: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_FOCUS")
            .collect();
        assert_eq!(
            dangling_focus.len(),
            1,
            "expected exactly one E_STATE_DANGLING_FOCUS, got: {diags:?}"
        );
    }

    #[test]
    fn check_cross_repo_edge_dangling_target_emits_dangling_cross_repo() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has block AL.1.A
        let pair_a = leaf_pair(dir.path(), "alpha", "AL.1.A");

        // brain file with cross_repo edge pointing at "alpha:AL.1.GHOST" (nonexistent)
        let brain_json = r#"{
  "repo": "core",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "repos": [{ "repo": "alpha", "now": [], "next": [], "blocked": [] }],
  "cross_repo": [
    {
      "from": { "repo": "alpha", "id": "AL.1.A" },
      "to": { "repo": "alpha", "id": "AL.1.GHOST" }
    }
  ]
}"#;
        let pair_brain = make_pair(dir.path(), "brain-state.json", "brain", brain_json);
        let files = vec![pair_a, pair_brain];

        let graph = build_state_graph(&files);
        let diags = check_state_graph(&graph, &files);

        let dangling_cross: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_CROSS_REPO")
            .collect();
        assert_eq!(
            dangling_cross.len(),
            1,
            "expected exactly one E_STATE_DANGLING_CROSS_REPO for ghost 'to' endpoint, got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 2 — depends_on DAG edges + check_schema v2 checks
    // -----------------------------------------------------------------------

    /// A v2 leaf fixture with `depends_on` wired through `tracks[].blocks[]`.
    fn leaf_with_depends_on(
        dir: &std::path::Path,
        repo: &str,
        block_id: &str,
        dep_repo: &str,
        dep_id: &str,
    ) -> (StateSource, StateFile) {
        let json = format!(
            r#"{{
  "repo": "{repo}",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {{
    "now": [{{ "id": "{block_id}", "title": "Waiting", "status": "open" }}],
    "next": [],
    "blocked": []
  }},
  "tracks": [{{
    "title": "Phase 1",
    "blocks": [{{
      "id": "{block_id}",
      "title": "Waiting",
      "status": "open",
      "depends_on": [
        {{ "type": "block", "repo": "{dep_repo}", "id": "{dep_id}" }}
      ]
    }}]
  }}]
}}"#
        );
        make_pair(dir, &format!("{repo}-state.json"), "project", &json)
    }

    #[test]
    fn build_depends_on_edges_from_tracks() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has block AL.1.A that depends_on beta:BE.1.A
        let pair_alpha = leaf_with_depends_on(dir.path(), "alpha", "AL.1.A", "beta", "BE.1.A");
        let pair_beta = leaf_pair(dir.path(), "beta", "BE.1.A");
        let files = vec![pair_alpha, pair_beta];

        let graph = build_state_graph(&files);

        // Two nodes (one per repo)
        assert_eq!(
            graph.nodes.len(),
            2,
            "expected 2 nodes, got: {:?}",
            graph.nodes
        );

        // One BlockedBy edge from depends_on
        let bb_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == StateEdgeKind::BlockedBy)
            .collect();
        assert_eq!(
            bb_edges.len(),
            1,
            "expected exactly one BlockedBy edge from depends_on, got: {:?}",
            graph.edges
        );
        assert_eq!(bb_edges[0].from, "alpha:AL.1.A");
        assert_eq!(bb_edges[0].to_ref, "beta:BE.1.A");
    }

    #[test]
    fn build_external_depends_on_entries_are_excluded_from_edges() {
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has a block that depends_on an external constraint only — no graph edges.
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{
      "id": "AL.1.A",
      "title": "Needs hardware",
      "status": "open",
      "depends_on": [
        { "type": "external", "what": "New Mac Mini delivery" }
      ]
    }]
  }]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);

        assert_eq!(graph.nodes.len(), 1, "expected 1 node");
        assert_eq!(
            graph.edges.len(),
            0,
            "external depends_on must not produce a graph edge, got: {:?}",
            graph.edges
        );
    }

    #[test]
    fn build_no_edges_from_focus_blocked_by() {
        // Confirm that focus.blocked_by is NOT an edge source in v2.
        let dir = tempfile::tempdir().expect("tempdir");

        // alpha has a focus.blocked entry with blocked_by — should NOT produce an edge.
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [],
    "next": [],
    "blocked": [{
      "id": "AL.1.A",
      "title": "Waiting",
      "blocked_by": [
        { "type": "block", "repo": "beta", "id": "BE.1.A" }
      ]
    }]
  },
  "tracks": [{ "title": "P1", "blocks": [{ "id": "AL.1.A", "title": "Waiting" }] }]
}"#;
        let pair = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair];

        let graph = build_state_graph(&files);

        assert_eq!(
            graph.edges.len(),
            0,
            "focus.blocked_by must not produce edges in v2; got: {:?}",
            graph.edges
        );
    }

    #[test]
    fn check_schema_authored_deferred_is_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        // `deferred` is a legal AUTHORED status (unlike `blocked`), and the
        // emitter also stamps it onto the derived focus.deferred[] entries —
        // both must validate clean, or `emit-state --write` would produce files
        // that `validate-brain` immediately rejects.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-07-26",
  "focus": {
    "now": [], "next": [], "blocked": [],
    "deferred": [{ "id": "T.1", "title": "Back burner", "status": "deferred" }]
  },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [
      { "id": "T.1", "title": "Back burner", "status": "deferred" }
    ]
  }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        assert!(
            diags.is_empty(),
            "authored + derived `deferred` must validate clean, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_authored_blocked_emits_e_state_authored_blocked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        // A track block with authored status "blocked" — this is illegal in v2.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [
      { "id": "T.1", "title": "Should not be authored blocked", "status": "blocked" }
    ]
  }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let authored_blocked: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_AUTHORED_BLOCKED")
            .collect();
        assert_eq!(
            authored_blocked.len(),
            1,
            "expected exactly one E_STATE_AUTHORED_BLOCKED, got: {diags:?}"
        );
        assert!(
            authored_blocked[0].message.contains("T.1"),
            "E_STATE_AUTHORED_BLOCKED message should name the block id, got: {:?}",
            authored_blocked[0].message
        );
    }

    #[test]
    fn check_schema_bad_backlog_status_emits_e_state_schema_bad_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "brain");

        // A backlog item with an invalid status value.
        let json = r#"{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "repos": [{ "repo": "some-child", "now": [], "next": [], "blocked": [] }],
  "backlog": [
    {
      "slug": "my-idea",
      "title": "Some idea",
      "repo": "mev",
      "type": "feature",
      "status": "flying"
    }
  ]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);

        let bad_status: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_STATUS")
            .collect();
        assert_eq!(
            bad_status.len(),
            1,
            "expected exactly one E_STATE_SCHEMA_BAD_STATUS for bad backlog status, got: {diags:?}"
        );
        assert!(
            bad_status[0].message.contains("my-idea"),
            "E_STATE_SCHEMA_BAD_STATUS message should name the backlog slug, got: {:?}",
            bad_status[0].message
        );
    }

    #[test]
    fn check_schema_valid_backlog_statuses_pass() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "brain");

        for status in &["idea", "ready", "promoted"] {
            let json = format!(
                r#"{{
  "repo": "hq",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "repos": [{{ "repo": "c", "now": [], "next": [], "blocked": [] }}],
  "backlog": [
    {{
      "slug": "item-{status}",
      "title": "Test",
      "repo": "mev",
      "type": "feature",
      "status": "{status}"
    }}
  ]
}}"#
            );
            let file = parse_file(&json);
            let diags = check_schema(&src, &file);
            let bad: Vec<_> = diags
                .iter()
                .filter(|d| {
                    d.locator == "E_STATE_SCHEMA_BAD_STATUS"
                        || d.locator == "E_STATE_AUTHORED_BLOCKED"
                })
                .collect();
            assert!(
                bad.is_empty(),
                "backlog status '{status}' should be valid, got: {diags:?}"
            );
        }
    }

    #[test]
    fn check_schema_clean_v2_file_with_depends_on_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        // A clean v2 leaf with depends_on in tracks and backlog-free.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [{ "id": "T.2", "title": "Active", "status": "in_progress" }],
    "next": [{ "id": "T.1", "title": "Ready" }],
    "blocked": []
  },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [
      { "id": "T.1", "title": "Ready", "status": "open" },
      {
        "id": "T.2",
        "title": "Active",
        "status": "in_progress",
        "depends_on": [
          { "type": "block", "repo": "other", "id": "OT.1.A" }
        ],
        "wave": 1
      }
    ]
  }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "clean v2 file with depends_on should produce no schema errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_schema_depends_on_empty_repo_emits_bad_blocked_by() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state.json");
        let src = make_source(&path, "project");

        // A track block whose depends_on entry has an empty repo field.
        let json = r#"{
  "repo": "test",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{
      "id": "T.1",
      "title": "Work",
      "depends_on": [
        { "type": "block", "repo": "", "id": "OT.1.A" }
      ]
    }]
  }]
}"#;
        let file = parse_file(json);
        let diags = check_schema(&src, &file);
        let bad_bb: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_SCHEMA_BAD_BLOCKED_BY")
            .collect();
        assert_eq!(
            bad_bb.len(),
            1,
            "expected one E_STATE_SCHEMA_BAD_BLOCKED_BY for empty depends_on repo, got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 — detect_cycles tests
    // -----------------------------------------------------------------------

    /// Build a `StateGraph` directly from node keys and BlockedBy edge tuples.
    /// `nodes`: (repo, id); `edges`: (from_key, to_ref).
    /// Uses a stable fake path derived from `from_key` for each edge's `source_path`.
    fn make_cycle_graph(
        dir: &std::path::Path,
        nodes: &[(&str, &str)],
        edges: &[(&str, &str)],
    ) -> StateGraph {
        let node_vec: Vec<StateNode> = nodes
            .iter()
            .map(|(repo, id)| StateNode {
                epics: Vec::new(),
                key: format!("{repo}:{id}"),
                repo: repo.to_string(),
                id: id.to_string(),
                title: format!("{repo}:{id}"),
                source_path: dir.join(format!("{repo}.json")),
            })
            .collect();
        let edge_vec: Vec<StateEdge> = edges
            .iter()
            .map(|(from, to_ref)| {
                let repo = from.split(':').next().unwrap_or("unknown");
                StateEdge {
                    from: from.to_string(),
                    to_ref: to_ref.to_string(),
                    kind: StateEdgeKind::BlockedBy,
                    source_path: dir.join(format!("{repo}.json")),
                }
            })
            .collect();
        StateGraph {
            nodes: node_vec,
            edges: edge_vec,
        }
    }

    #[test]
    fn detect_cycles_simple_two_node_cycle_is_flagged() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X depends_on b:Y, b:Y depends_on a:X → cycle
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y")],
            &[("a:X", "b:Y"), ("b:Y", "a:X")],
        );

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert_eq!(
            cycles.len(),
            1,
            "expected exactly one E_STATE_CYCLE for two-node cycle, got: {diags:?}"
        );
        // The message should contain both node keys.
        assert!(
            cycles[0].message.contains("a:X") && cycles[0].message.contains("b:Y"),
            "E_STATE_CYCLE message should name the cycle nodes, got: {:?}",
            cycles[0].message
        );
    }

    #[test]
    fn detect_cycles_three_node_cycle_path_is_flagged() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X → b:Y → c:Z → a:X
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y"), ("c", "Z")],
            &[("a:X", "b:Y"), ("b:Y", "c:Z"), ("c:Z", "a:X")],
        );

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert_eq!(
            cycles.len(),
            1,
            "expected exactly one E_STATE_CYCLE for three-node cycle, got: {diags:?}"
        );
        // All three keys must appear in the message.
        assert!(
            cycles[0].message.contains("a:X")
                && cycles[0].message.contains("b:Y")
                && cycles[0].message.contains("c:Z"),
            "E_STATE_CYCLE message should name all cycle nodes, got: {:?}",
            cycles[0].message
        );
    }

    #[test]
    fn detect_cycles_acyclic_dag_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X → b:Y → c:Z (no cycle)
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y"), ("c", "Z")],
            &[("a:X", "b:Y"), ("b:Y", "c:Z")],
        );

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert!(
            cycles.is_empty(),
            "acyclic DAG should produce no E_STATE_CYCLE, got: {diags:?}"
        );
    }

    #[test]
    fn detect_cycles_self_loop_is_flagged() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X depends_on itself
        let graph = make_cycle_graph(dir.path(), &[("a", "X")], &[("a:X", "a:X")]);

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert_eq!(
            cycles.len(),
            1,
            "self-loop should produce exactly one E_STATE_CYCLE, got: {diags:?}"
        );
        assert!(
            cycles[0].message.contains("a:X"),
            "E_STATE_CYCLE message should name the self-loop node, got: {:?}",
            cycles[0].message
        );
    }

    #[test]
    fn detect_cycles_cross_repo_edges_are_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Two nodes with a CrossRepo edge — CrossRepo edges must NOT trigger cycle detection.
        let node_vec = vec![
            StateNode {
                epics: Vec::new(),
                key: "a:X".to_string(),
                repo: "a".to_string(),
                id: "X".to_string(),
                title: "X".to_string(),
                source_path: dir.path().join("a.json"),
            },
            StateNode {
                epics: Vec::new(),
                key: "b:Y".to_string(),
                repo: "b".to_string(),
                id: "Y".to_string(),
                title: "Y".to_string(),
                source_path: dir.path().join("b.json"),
            },
        ];
        // CrossRepo edges forming a "cycle" — should be ignored by detect_cycles.
        let edge_vec = vec![
            StateEdge {
                from: "a:X".to_string(),
                to_ref: "b:Y".to_string(),
                kind: StateEdgeKind::CrossRepo,
                source_path: dir.path().join("brain.json"),
            },
            StateEdge {
                from: "b:Y".to_string(),
                to_ref: "a:X".to_string(),
                kind: StateEdgeKind::CrossRepo,
                source_path: dir.path().join("brain.json"),
            },
        ];
        let graph = StateGraph {
            nodes: node_vec,
            edges: edge_vec,
        };

        let diags = detect_cycles(&graph);
        let cycles: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .collect();
        assert!(
            cycles.is_empty(),
            "CrossRepo edges must not trigger E_STATE_CYCLE, got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 2 (MV.10.A) — cycle_paths tests
    // -----------------------------------------------------------------------

    #[test]
    fn cycle_paths_three_node_cycle_has_no_repeated_closing_node() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X → b:Y → c:Z → a:X
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y"), ("c", "Z")],
            &[("a:X", "b:Y"), ("b:Y", "c:Z"), ("c:Z", "a:X")],
        );

        let paths = cycle_paths(&graph);
        assert_eq!(paths.len(), 1, "expected exactly one cycle, got: {paths:?}");
        let cycle = &paths[0];
        assert_eq!(
            cycle.keys.len(),
            3,
            "cycle keys should have length 3 (no repeated closing node), got: {:?}",
            cycle.keys
        );
        let mut sorted = cycle.keys.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec!["a:X".to_string(), "b:Y".to_string(), "c:Z".to_string()]
        );
        assert_eq!(cycle.source_path, dir.path().join("c.json"));
    }

    #[test]
    fn cycle_paths_self_loop_has_single_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let graph = make_cycle_graph(dir.path(), &[("a", "X")], &[("a:X", "a:X")]);

        let paths = cycle_paths(&graph);
        assert_eq!(paths.len(), 1, "expected exactly one cycle, got: {paths:?}");
        assert_eq!(
            paths[0].keys.len(),
            1,
            "self-loop cycle keys should have length 1, got: {:?}",
            paths[0].keys
        );
        assert_eq!(paths[0].keys, vec!["a:X".to_string()]);
    }

    #[test]
    fn cycle_paths_dedups_by_canonical_rotation() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Two entry points (a:X and b:Y) into the same 3-node cycle: a→b→c→a.
        // Starting the DFS at a:X yields [a:X, b:Y, c:Z]; nothing else enters the
        // cycle first here, but this test exercises that a single DFS produces one
        // canonical-rotation entry for the cycle regardless of which node the
        // back-edge closes on.
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y"), ("c", "Z")],
            &[("a:X", "b:Y"), ("b:Y", "c:Z"), ("c:Z", "a:X")],
        );

        let paths = cycle_paths(&graph);
        assert_eq!(
            paths.len(),
            1,
            "canonical-rotation dedup should collapse to one cycle, got: {paths:?}"
        );

        // Directly verify the rotation-dedup helper collapses rotated variants of the
        // same cycle to the same canonical form.
        let a = canonical_rotation(&["a:X".to_string(), "b:Y".to_string(), "c:Z".to_string()]);
        let b = canonical_rotation(&["b:Y".to_string(), "c:Z".to_string(), "a:X".to_string()]);
        let c = canonical_rotation(&["c:Z".to_string(), "a:X".to_string(), "b:Y".to_string()]);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn cycle_paths_cross_repo_edges_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let node_vec = vec![
            StateNode {
                epics: Vec::new(),
                key: "a:X".to_string(),
                repo: "a".to_string(),
                id: "X".to_string(),
                title: "X".to_string(),
                source_path: dir.path().join("a.json"),
            },
            StateNode {
                epics: Vec::new(),
                key: "b:Y".to_string(),
                repo: "b".to_string(),
                id: "Y".to_string(),
                title: "Y".to_string(),
                source_path: dir.path().join("b.json"),
            },
        ];
        let edge_vec = vec![
            StateEdge {
                from: "a:X".to_string(),
                to_ref: "b:Y".to_string(),
                kind: StateEdgeKind::CrossRepo,
                source_path: dir.path().join("brain.json"),
            },
            StateEdge {
                from: "b:Y".to_string(),
                to_ref: "a:X".to_string(),
                kind: StateEdgeKind::CrossRepo,
                source_path: dir.path().join("brain.json"),
            },
        ];
        let graph = StateGraph {
            nodes: node_vec,
            edges: edge_vec,
        };

        let paths = cycle_paths(&graph);
        assert!(
            paths.is_empty(),
            "CrossRepo edges must not produce a CyclePath, got: {paths:?}"
        );
    }

    #[test]
    fn cycle_paths_and_detect_cycles_are_in_parity() {
        // Every message detect_cycles emits must be exactly derivable from cycle_paths'
        // output via the same formatting rule, proving the formatter composition is a
        // behaviour-preserving refactor.
        let dir = tempfile::tempdir().expect("tempdir");
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y"), ("c", "Z"), ("d", "W")],
            &[
                ("a:X", "b:Y"),
                ("b:Y", "a:X"),
                ("c:Z", "d:W"),
                ("d:W", "c:Z"),
            ],
        );

        let diags = detect_cycles(&graph);
        let messages: std::collections::HashSet<String> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_CYCLE")
            .map(|d| d.message.clone())
            .collect();

        let expected: std::collections::HashSet<String> = cycle_paths(&graph)
            .iter()
            .map(|cycle| {
                let path_str = format!(
                    "{} \u{2192} {}",
                    cycle.keys.join(" \u{2192} "),
                    cycle.keys[0]
                );
                format!("cycle detected in depends_on DAG: {path_str}")
            })
            .collect();

        assert_eq!(
            messages, expected,
            "detect_cycles messages must be exactly derivable from cycle_paths"
        );
    }

    // -----------------------------------------------------------------------
    // Task 3 — ready_order tests
    // -----------------------------------------------------------------------

    /// One `(id, status, wave, depends_on)` block spec for [`make_ready_pair`] fixtures.
    type ReadyBlockSpec<'a> = (&'a str, Option<&'a str>, Option<i64>, Vec<BlockedBy>);

    /// Build a minimal (StateSource, StateFile) pair for ready_order testing.
    fn make_ready_pair(
        dir: &std::path::Path,
        repo: &str,
        blocks: &[ReadyBlockSpec],
    ) -> (StateSource, StateFile) {
        // blocks: (id, status, wave, depends_on)
        let track_blocks: Vec<TrackBlock> = blocks
            .iter()
            .map(|(id, status, wave, deps)| TrackBlock {
                epics: Vec::new(),
                due: None,
                priority: None,
                sdlc_workflow: None,
                model: None,
                id: id.to_string(),
                title: id.to_string(),
                status: status.map(|s| s.to_string()),
                depends_on: deps.clone(),
                wave: *wave,
                origin: None,
            })
            .collect();

        let path = dir.join(format!("{repo}-state.json"));
        let file = StateFile {
            epics: Vec::new(),
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks: track_blocks,
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: path,
            expected_kind: "project",
        };
        (src, file)
    }

    #[test]
    fn ready_order_no_deps_open_block_is_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), None, vec![])],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A"],
            "open block with no deps should appear in ready_order, got: {order:?}"
        );
    }

    #[test]
    fn derive_focus_deferred_block_lands_in_deferred_lane_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("deferred"), None, vec![])],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];

        let d = derive_focus(src, file, &graph, &files);

        assert_eq!(d.deferred, vec!["AL.1.A"]);
        assert!(d.now.is_empty(), "deferred must not be now: {:?}", d.now);
        assert!(d.next.is_empty(), "deferred must not be next: {:?}", d.next);
        assert!(
            d.blocked.is_empty(),
            "deferred must not be blocked: {:?}",
            d.blocked
        );
    }

    #[test]
    fn derive_focus_deferred_with_unmet_deps_stays_deferred_not_blocked() {
        // Deferral is a statement about ATTENTION, not readiness — it wins over
        // whatever the DAG says. A deferred block with a wide-open external dep
        // is still just deferred, never blocked.
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[
                (
                    "AL.1.A",
                    Some("deferred"),
                    None,
                    vec![
                        BlockedBy::External {
                            what: "waiting on vendor".to_string(),
                        },
                        BlockedBy::Block {
                            repo: "alpha".to_string(),
                            id: "AL.1.B".to_string(),
                            what: None,
                        },
                    ],
                ),
                ("AL.1.B", Some("open"), None, vec![]),
            ],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];

        let d = derive_focus(src, file, &graph, &files);

        assert_eq!(d.deferred, vec!["AL.1.A"]);
        assert!(
            d.blocked.is_empty(),
            "a deferred block must never appear in blocked, got: {:?}",
            d.blocked
        );
    }

    #[test]
    fn derive_focus_open_block_depending_on_deferred_is_blocked() {
        // Deferral does NOT propagate down the DAG. The dependent stays blocked,
        // because its dep is not `closed` — which is exactly why deferring a
        // block that others depend on quietly stalls them.
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("deferred"), None, vec![]),
                (
                    "AL.1.B",
                    Some("open"),
                    None,
                    vec![BlockedBy::Block {
                        repo: "alpha".to_string(),
                        id: "AL.1.A".to_string(),
                        what: None,
                    }],
                ),
            ],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];

        let d = derive_focus(src, file, &graph, &files);

        assert_eq!(d.deferred, vec!["AL.1.A"]);
        assert_eq!(d.blocked.len(), 1, "dependent must be blocked");
        assert_eq!(d.blocked[0].0, "AL.1.B");
        assert_eq!(
            d.blocked[0].1.len(),
            1,
            "the deferred dep must be reported as the unmet edge"
        );
        assert!(
            d.next.is_empty(),
            "dependent must not be ready while its dep is deferred: {:?}",
            d.next
        );
    }

    #[test]
    fn ready_order_deferred_block_excluded() {
        // Regression pin. `ready_order`'s gate is `status != "open"`, so a
        // deferred block is excluded for free — but that is load-bearing, not
        // incidental. If anyone ever relaxes the gate to `!= "closed"`,
        // deferred work silently floods back into `focus.next` and the whole
        // back-burner feature stops working. This test is the tripwire.
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("deferred"), None, vec![]),
                ("AL.1.B", Some("open"), None, vec![]),
            ],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.B"],
            "deferred block must never be ready, even with zero deps; got: {order:?}"
        );
    }

    #[test]
    fn ready_order_absent_status_treated_as_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No status field — treated as open.
        let pair = make_ready_pair(dir.path(), "alpha", &[("AL.1.A", None, None, vec![])]);
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A"],
            "absent-status block should be treated as open and appear in ready_order"
        );
    }

    #[test]
    fn ready_order_closed_block_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("closed"), None, vec![])],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert!(
            order.is_empty(),
            "closed block must not appear in ready_order, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_in_progress_block_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("in_progress"), None, vec![])],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert!(
            order.is_empty(),
            "in_progress block must not appear in ready_order, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_block_with_external_dep_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ext_dep = BlockedBy::External {
            what: "Mac Mini delivery".to_string(),
        };
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), None, vec![ext_dep])],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert!(
            order.is_empty(),
            "open block with external dep must not appear in ready_order, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_block_with_unclosed_block_dep_excluded() {
        let dir = tempfile::tempdir().expect("tempdir");
        // alpha:AL.1.A depends_on beta:BE.1.A which is open (not closed).
        let block_dep = BlockedBy::Block {
            repo: "beta".to_string(),
            id: "BE.1.A".to_string(),
            what: None,
        };
        let pair_a = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), None, vec![block_dep])],
        );
        // beta:BE.1.A is open (not closed).
        let pair_b = make_ready_pair(
            dir.path(),
            "beta",
            &[("BE.1.A", Some("open"), None, vec![])],
        );
        let files = vec![pair_a, pair_b];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        // beta:BE.1.A is open → alpha:AL.1.A is not ready
        // beta:BE.1.A has no deps → it IS ready
        assert!(
            !order.contains(&"alpha:AL.1.A".to_string()),
            "alpha:AL.1.A should not be ready when its dep is open; order={order:?}"
        );
        assert!(
            order.contains(&"beta:BE.1.A".to_string()),
            "beta:BE.1.A (no deps, open) should be ready; order={order:?}"
        );
    }

    #[test]
    fn ready_order_block_with_closed_dep_is_ready() {
        let dir = tempfile::tempdir().expect("tempdir");
        // alpha:AL.1.A depends_on beta:BE.1.A which is closed → AL.1.A is ready.
        let block_dep = BlockedBy::Block {
            repo: "beta".to_string(),
            id: "BE.1.A".to_string(),
            what: None,
        };
        let pair_a = make_ready_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), None, vec![block_dep])],
        );
        let pair_b = make_ready_pair(
            dir.path(),
            "beta",
            &[("BE.1.A", Some("closed"), None, vec![])],
        );
        let files = vec![pair_a, pair_b];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert!(
            order.contains(&"alpha:AL.1.A".to_string()),
            "alpha:AL.1.A should be ready when its only dep is closed; order={order:?}"
        );
    }

    #[test]
    fn ready_order_wave_ordering_applied() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Three open blocks: wave 3, wave 1, wave 2 → should come out 1, 2, 3.
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.C", Some("open"), Some(3), vec![]),
                ("AL.1.A", Some("open"), Some(1), vec![]),
                ("AL.1.B", Some("open"), Some(2), vec![]),
            ],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A", "alpha:AL.1.B", "alpha:AL.1.C"],
            "ready_order should sort by wave ascending, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_none_wave_goes_last() {
        let dir = tempfile::tempdir().expect("tempdir");
        // One block with wave=1, one with no wave → wave=1 should come first.
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.B", Some("open"), None, vec![]), // no wave → last
                ("AL.1.A", Some("open"), Some(1), vec![]),
            ],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A", "alpha:AL.1.B"],
            "block with wave=None should sort after wave=1, got: {order:?}"
        );
    }

    #[test]
    fn ready_order_equal_wave_preserves_iteration_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Three open blocks all with wave=1 → should preserve array order.
        let pair = make_ready_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("open"), Some(1), vec![]),
                ("AL.1.B", Some("open"), Some(1), vec![]),
                ("AL.1.C", Some("open"), Some(1), vec![]),
            ],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);

        let order = ready_order(&graph, &files);
        assert_eq!(
            order,
            vec!["alpha:AL.1.A", "alpha:AL.1.B", "alpha:AL.1.C"],
            "equal-wave blocks should preserve array order, got: {order:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 4 — status consistency + backlog integrity tests
    // -----------------------------------------------------------------------

    // Helper: build a (StateSource, StateFile) pair with blocks that have known statuses
    // and depends_on edges. Each block tuple is (id, status, Vec<depends_on>).
    fn make_consistency_pair(
        dir: &std::path::Path,
        repo: &str,
        blocks: &[(&str, Option<&str>, Vec<BlockedBy>)],
    ) -> (StateSource, StateFile) {
        let track_blocks: Vec<TrackBlock> = blocks
            .iter()
            .map(|(id, status, deps)| TrackBlock {
                epics: Vec::new(),
                due: None,
                priority: None,
                sdlc_workflow: None,
                model: None,
                id: id.to_string(),
                title: id.to_string(),
                status: status.map(|s| s.to_string()),
                depends_on: deps.clone(),
                wave: None,
                origin: None,
            })
            .collect();

        let path = dir.join(format!("{repo}-state.json"));
        let file = StateFile {
            epics: Vec::new(),
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks: track_blocks,
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: path,
            expected_kind: "project",
        };
        (src, file)
    }

    #[test]
    fn check_status_consistency_closed_depends_on_open_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // alpha has two blocks: AL.1.A (open) and AL.1.B (closed, depends on AL.1.A).
        let dep = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_consistency_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("open"), vec![]),
                ("AL.1.B", Some("closed"), vec![dep]),
            ],
        );
        let files = vec![pair];
        let diags = check_status_consistency(&files);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_STATUS_INCONSISTENT")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "closed block depending on open block should emit exactly one \
             E_STATE_STATUS_INCONSISTENT, got: {diags:?}"
        );
    }

    #[test]
    fn check_status_consistency_closed_depends_on_closed_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Both blocks are closed — no inconsistency.
        let dep = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_consistency_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("closed"), vec![]),
                ("AL.1.B", Some("closed"), vec![dep]),
            ],
        );
        let files = vec![pair];
        let diags = check_status_consistency(&files);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_STATUS_INCONSISTENT")
            .collect();
        assert!(
            errs.is_empty(),
            "closed block depending on closed block should not emit errors, got: {diags:?}"
        );
    }

    #[test]
    fn check_status_consistency_closed_depends_on_in_progress_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dep = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_consistency_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("in_progress"), vec![]),
                ("AL.1.B", Some("closed"), vec![dep]),
            ],
        );
        let files = vec![pair];
        let diags = check_status_consistency(&files);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_STATUS_INCONSISTENT")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "closed block depending on in_progress block should emit E_STATE_STATUS_INCONSISTENT, \
             got: {diags:?}"
        );
    }

    #[test]
    fn check_status_consistency_dangling_dep_is_skipped_silently() {
        let dir = tempfile::tempdir().expect("tempdir");
        // AL.1.B is closed but depends on "alpha:AL.1.GHOST" which is not in any file.
        // Should NOT emit E_STATE_STATUS_INCONSISTENT (that's E_STATE_DANGLING_BLOCKED_BY's job).
        let dep = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.GHOST".to_string(),
            what: None,
        };
        let pair = make_consistency_pair(
            dir.path(),
            "alpha",
            &[("AL.1.B", Some("closed"), vec![dep])],
        );
        let files = vec![pair];
        let diags = check_status_consistency(&files);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_STATUS_INCONSISTENT")
            .collect();
        assert!(
            errs.is_empty(),
            "dangling dep (not in any loaded file) should not produce \
             E_STATE_STATUS_INCONSISTENT (that's check_state_graph's job), got: {diags:?}"
        );
    }

    // Helper: build a (StateSource, StateFile) pair that has a backlog[] array.
    fn make_brain_with_backlog(
        dir: &std::path::Path,
        repo: &str,
        track_blocks: Vec<TrackBlock>,
        backlog_nodes: Vec<Backlog>,
    ) -> (StateSource, StateFile) {
        let path = dir.join(format!("{repo}-state.json"));
        let file = StateFile {
            epics: Vec::new(),
            repo: repo.to_string(),
            kind: "brain".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: if track_blocks.is_empty() {
                vec![]
            } else {
                vec![Track {
                    title: "Phase 1".to_string(),
                    blocks: track_blocks,
                }]
            },
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: backlog_nodes,
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: path,
            expected_kind: "brain",
        };
        (src, file)
    }

    #[test]
    fn check_backlog_integrity_dangling_depends_on_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Backlog node depends on "mev:MV.3.GHOST" which doesn't exist in any tracks[].
        let backlog_node = Backlog {
            slug: "add-foo".to_string(),
            title: "Add foo".to_string(),
            repo: "mev".to_string(),
            kind: "feature".to_string(),
            status: "idea".to_string(),
            depends_on: vec![BlockedBy::Block {
                repo: "mev".to_string(),
                id: "MV.3.GHOST".to_string(),
                what: None,
            }],
            block: None,
            notes: None,
            ..Default::default()
        };
        let pair = make_brain_with_backlog(dir.path(), "hq", vec![], vec![backlog_node]);
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let diags = check_backlog_integrity(&files, &graph);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_BLOCKED_BY")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "backlog node with dangling depends_on should emit E_STATE_DANGLING_BLOCKED_BY, \
             got: {diags:?}"
        );
    }

    #[test]
    fn check_backlog_integrity_promoted_no_block_pointer_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Backlog node with status "promoted" but no block pointer.
        let backlog_node = Backlog {
            slug: "add-bar".to_string(),
            title: "Add bar".to_string(),
            repo: "mev".to_string(),
            kind: "feature".to_string(),
            status: "promoted".to_string(),
            depends_on: vec![],
            block: None, // missing pointer
            notes: None,
            ..Default::default()
        };
        let pair = make_brain_with_backlog(dir.path(), "hq", vec![], vec![backlog_node]);
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let diags = check_backlog_integrity(&files, &graph);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_PROMOTION")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "promoted backlog node with no block pointer should emit E_STATE_DANGLING_PROMOTION, \
             got: {diags:?}"
        );
    }

    #[test]
    fn check_backlog_integrity_promoted_block_pointer_resolves_to_nothing_emits_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Backlog node with status "promoted" pointing at a block that doesn't exist.
        let backlog_node = Backlog {
            slug: "add-baz".to_string(),
            title: "Add baz".to_string(),
            repo: "mev".to_string(),
            kind: "feature".to_string(),
            status: "promoted".to_string(),
            depends_on: vec![],
            block: Some("MV.3.GHOST".to_string()), // block doesn't exist in mev tracks[]
            notes: None,
            ..Default::default()
        };
        let pair = make_brain_with_backlog(dir.path(), "hq", vec![], vec![backlog_node]);
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let diags = check_backlog_integrity(&files, &graph);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "E_STATE_DANGLING_PROMOTION")
            .collect();
        assert_eq!(
            errs.len(),
            1,
            "promoted backlog node with orphan block pointer should emit \
             E_STATE_DANGLING_PROMOTION, got: {diags:?}"
        );
    }

    #[test]
    fn check_backlog_integrity_clean_promotion_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A promoted backlog node pointing at a real block that carries an origin.
        // The origin back-pointer is structural metadata — the integrity check validates
        // that the block exists in tracks[], not that it carries an origin field.
        let real_block = TrackBlock {
            epics: Vec::new(),
            due: None,
            priority: None,
            sdlc_workflow: None,
            model: None,
            id: "MV.3.P2".to_string(),
            title: "P2 block".to_string(),
            status: Some("in_progress".to_string()),
            depends_on: vec![],
            wave: Some(1),
            origin: Some(Origin {
                kind: "backlog".to_string(),
                slug: "add-p2".to_string(),
            }),
        };
        let backlog_node = Backlog {
            slug: "add-p2".to_string(),
            title: "Add P2".to_string(),
            repo: "mev".to_string(),
            kind: "feature".to_string(),
            status: "promoted".to_string(),
            depends_on: vec![],
            block: Some("MV.3.P2".to_string()), // resolves to the real block
            notes: None,
            ..Default::default()
        };

        // Two files: a mev leaf (owns the track block) and hq brain (owns the backlog node).
        let mev_path = dir.path().join("mev-state.json");
        let mev_file = StateFile {
            epics: Vec::new(),
            repo: "mev".to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 3".to_string(),
                blocks: vec![real_block],
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let mev_src = StateSource {
            repo_slug: "mev".to_string(),
            abs_path: mev_path,
            expected_kind: "project",
        };

        let hq_pair = make_brain_with_backlog(dir.path(), "hq", vec![], vec![backlog_node]);
        let files = vec![(mev_src, mev_file), hq_pair];
        let graph = build_state_graph(&files);
        let diags = check_backlog_integrity(&files, &graph);

        let errs: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errs.is_empty(),
            "clean promotion (block exists in tracks[]) should produce no errors, \
             got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 4 — check_rollup tests
    // -----------------------------------------------------------------------

    /// Build a brain StateFile with a repos[] entry for one child.
    fn brain_with_rollup(
        child_slug: &str,
        now_ids: &[&str],
        next_ids: &[&str],
        blocked_ids: &[&str],
    ) -> StateFile {
        let make_blocks = |ids: &[&str]| -> Vec<Block> {
            ids.iter()
                .map(|id| Block {
                    epics: Vec::new(),
                    due: None,
                    priority: None,
                    id: id.to_string(),
                    title: "placeholder".to_string(),
                    status: None,
                    note: None,
                    repo: None,
                    blocked_by: vec![],
                })
                .collect()
        };

        StateFile {
            epics: Vec::new(),
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-06-29".to_string(),
            focus: Focus::default(),
            tracks: vec![],
            repos: vec![RepoRollup {
                repo: child_slug.to_string(),
                tier: None,
                now: make_blocks(now_ids),
                next: make_blocks(next_ids),
                blocked: make_blocks(blocked_ids),
                deferred: Vec::new(),
            }],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        }
    }

    /// Build a child StateFile with the given focus.
    fn child_with_focus(
        slug: &str,
        now_ids: &[&str],
        next_ids: &[&str],
        blocked_ids: &[&str],
    ) -> StateFile {
        let make_blocks = |ids: &[&str], with_status: bool| -> Vec<Block> {
            ids.iter()
                .map(|id| Block {
                    epics: Vec::new(),
                    due: None,
                    priority: None,
                    id: id.to_string(),
                    title: "placeholder".to_string(),
                    status: if with_status {
                        Some("in_progress".to_string())
                    } else {
                        None
                    },
                    note: None,
                    repo: None,
                    blocked_by: vec![],
                })
                .collect()
        };

        StateFile {
            epics: Vec::new(),
            repo: slug.to_string(),
            kind: "project".to_string(),
            updated: "2026-06-29".to_string(),
            focus: Focus {
                now: make_blocks(now_ids, true),
                next: make_blocks(next_ids, false),
                blocked: make_blocks(blocked_ids, false),
                deferred: Vec::new(),
            },
            tracks: vec![],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        }
    }

    #[test]
    fn check_rollup_in_sync_returns_no_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let brain_path = dir.path().join("brain-state.json");

        // Brain repos[] cache and child focus are identical (both have ["AL.1.A"]).
        let brain = brain_with_rollup("alpha", &["AL.1.A"], &["AL.1.B"], &[]);
        let child = child_with_focus("alpha", &["AL.1.A"], &["AL.1.B"], &[]);

        let mut children = std::collections::HashMap::new();
        children.insert("alpha".to_string(), child);

        let diags = check_rollup(&brain_path, &brain, &children);
        assert!(
            diags.is_empty(),
            "in-sync rollup should produce no diagnostics, got: {diags:?}"
        );
    }

    #[test]
    fn check_rollup_drifted_child_emits_w_state_rollup_drift() {
        let dir = tempfile::tempdir().expect("tempdir");
        let brain_path = dir.path().join("brain-state.json");

        // Brain cache says child.now = ["AL.1.A"] but child has advanced to ["AL.1.B"].
        let brain = brain_with_rollup("alpha", &["AL.1.A"], &["AL.1.B"], &[]);
        let child = child_with_focus("alpha", &["AL.1.B"], &["AL.1.C"], &[]);

        let mut children = std::collections::HashMap::new();
        children.insert("alpha".to_string(), child);

        let diags = check_rollup(&brain_path, &brain, &children);

        let drift: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_ROLLUP_DRIFT")
            .collect();
        assert_eq!(
            drift.len(),
            1,
            "drifted child should produce exactly one W_STATE_ROLLUP_DRIFT, got: {diags:?}"
        );
        // It must be a Warning, not an Error (decision 4).
        assert_eq!(
            drift[0].severity,
            crate::Severity::Warning,
            "W_STATE_ROLLUP_DRIFT must be Warning severity"
        );
    }

    #[test]
    fn check_rollup_missing_child_is_skipped_silently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let brain_path = dir.path().join("brain-state.json");

        // Brain has a repos[] entry for "alpha" but children map is empty.
        let brain = brain_with_rollup("alpha", &["AL.1.A"], &[], &[]);
        let children: std::collections::HashMap<String, StateFile> =
            std::collections::HashMap::new();

        let diags = check_rollup(&brain_path, &brain, &children);
        assert!(
            diags.is_empty(),
            "missing child (no loaded state.json) should be skipped silently; got: {diags:?}"
        );
    }

    #[test]
    fn check_rollup_only_blocked_drift_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        let brain_path = dir.path().join("brain-state.json");

        // now/next match but blocked list differs.
        let brain = brain_with_rollup("alpha", &["AL.1.A"], &["AL.1.B"], &[]);
        let child = child_with_focus("alpha", &["AL.1.A"], &["AL.1.B"], &["AL.1.C"]);

        let mut children = std::collections::HashMap::new();
        children.insert("alpha".to_string(), child);

        let diags = check_rollup(&brain_path, &brain, &children);
        let drift: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_ROLLUP_DRIFT")
            .collect();
        assert_eq!(
            drift.len(),
            1,
            "blocked-only drift should also emit W_STATE_ROLLUP_DRIFT, got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Task 5 — check_focus_drift tests
    // -----------------------------------------------------------------------

    /// Build a (StateSource, StateFile) pair for focus-drift testing.
    ///
    /// `blocks` = `(id, authored_status, depends_on)`.
    /// `stored_now/next/blocked` are the block IDs stored in the `focus` object.
    fn make_drift_pair(
        dir: &std::path::Path,
        repo: &str,
        blocks: &[(&str, Option<&str>, Vec<BlockedBy>)],
        stored_now: &[&str],
        stored_next: &[&str],
        stored_blocked: &[&str],
    ) -> (StateSource, StateFile) {
        let track_blocks: Vec<TrackBlock> = blocks
            .iter()
            .map(|(id, status, deps)| TrackBlock {
                epics: Vec::new(),
                due: None,
                priority: None,
                sdlc_workflow: None,
                model: None,
                id: id.to_string(),
                title: id.to_string(),
                status: status.map(|s| s.to_string()),
                depends_on: deps.clone(),
                wave: None,
                origin: None,
            })
            .collect();

        let make_focus_blocks = |ids: &[&str]| -> Vec<Block> {
            ids.iter()
                .map(|id| Block {
                    epics: Vec::new(),
                    due: None,
                    priority: None,
                    id: id.to_string(),
                    title: "placeholder".to_string(),
                    status: None,
                    note: None,
                    repo: None,
                    blocked_by: vec![],
                })
                .collect()
        };

        let path = dir.join(format!("{repo}-drift-state.json"));
        let file = StateFile {
            epics: Vec::new(),
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus {
                now: make_focus_blocks(stored_now),
                next: make_focus_blocks(stored_next),
                blocked: make_focus_blocks(stored_blocked),
                deferred: Vec::new(),
            },
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks: track_blocks,
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: path,
            expected_kind: "project",
        };
        (src, file)
    }

    #[test]
    fn check_focus_drift_in_sync_produces_no_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        // One in_progress block, one open block (no deps) → it's in next.
        // Stored focus matches exactly.
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("in_progress"), vec![]),
                ("AL.1.B", Some("open"), vec![]),
            ],
            &["AL.1.A"], // stored now
            &["AL.1.B"], // stored next
            &[],         // stored blocked
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        assert!(
            diags.is_empty(),
            "in-sync focus should produce no diagnostics, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_deferred_mismatch_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Block is deferred in tracks[] but stored focus has no deferred lane.
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("deferred"), vec![])],
            &[], // stored now
            &[], // stored next
            &[], // stored blocked — and no stored deferred either
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        assert_eq!(diags.len(), 1, "expected one drift warning, got: {diags:?}");
        assert_eq!(diags[0].locator, "W_STATE_FOCUS_DRIFT");
        assert!(
            diags[0].message.contains("deferred:"),
            "drift message must name the deferred lane, got: {}",
            diags[0].message
        );
    }

    #[test]
    fn check_focus_drift_deferred_in_sync_produces_no_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("deferred"), vec![])],
            &[],
            &[],
            &[],
        );
        // Stored deferred lane matches the derivation.
        pair.1.focus.deferred = vec![Block {
            epics: Vec::new(),
            due: None,
            priority: None,
            id: "AL.1.A".to_string(),
            title: "AL.1.A".to_string(),
            status: Some("deferred".to_string()),
            note: None,
            repo: None,
            blocked_by: Vec::new(),
        }];
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        assert!(
            diags.is_empty(),
            "matching deferred lane should not drift, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_no_deferred_blocks_and_no_deferred_key_is_clean() {
        // The day-one case for all ~23 existing repos: nothing is deferred and no
        // state.json carries a `deferred` key. Adding the lane must not make a
        // single one of them start warning.
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("in_progress"), vec![]),
                ("AL.1.B", Some("open"), vec![]),
            ],
            &["AL.1.A"],
            &["AL.1.B"],
            &[],
        );
        let files = vec![pair];
        assert!(files[0].1.focus.deferred.is_empty());
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        assert!(
            diags.is_empty(),
            "pre-existing corpus must stay clean, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_now_mismatch_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Block is in_progress in tracks[] but stored focus.now is empty.
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("in_progress"), vec![])],
            &[], // stored now — stale (should be ["AL.1.A"])
            &[], // stored next
            &[], // stored blocked
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        let drifts: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert_eq!(
            drifts.len(),
            1,
            "stale now should emit exactly one W_STATE_FOCUS_DRIFT, got: {diags:?}"
        );
        assert_eq!(
            drifts[0].severity,
            crate::Severity::Warning,
            "W_STATE_FOCUS_DRIFT must be Warning severity"
        );
    }

    #[test]
    fn check_focus_drift_next_mismatch_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        // One open block with no deps → should be in next.
        // Stored focus.next is empty → drift.
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("open"), vec![])],
            &[], // stored now
            &[], // stored next — stale (should be ["AL.1.A"])
            &[], // stored blocked
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        let drifts: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert_eq!(
            drifts.len(),
            1,
            "stale next should emit exactly one W_STATE_FOCUS_DRIFT, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_blocked_mismatch_emits_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Open block with external dep → should be in blocked.
        // Stored focus.blocked is empty → drift.
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[(
                "AL.1.A",
                Some("open"),
                vec![BlockedBy::External {
                    what: "upstream dep".to_string(),
                }],
            )],
            &[], // stored now
            &[], // stored next
            &[], // stored blocked — stale (should be ["AL.1.A"])
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        let drifts: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert_eq!(
            drifts.len(),
            1,
            "stale blocked should emit exactly one W_STATE_FOCUS_DRIFT, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_drift_is_warning_not_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[("AL.1.A", Some("in_progress"), vec![])],
            &[], // stored now — deliberately wrong
            &[],
            &[],
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        // No errors should be emitted.
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "drift must never produce errors (exit-0 behaviour), got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_empty_tracks_produces_no_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("brain-state.json");
        // Brain file: non-empty focus but no tracks[] — skip.
        let file = StateFile {
            epics: Vec::new(),
            repo: "hq".to_string(),
            kind: "brain".to_string(),
            updated: "2026-06-30".to_string(),
            focus: Focus {
                now: vec![Block {
                    epics: Vec::new(),
                    due: None,
                    priority: None,
                    id: "BA.1.A".to_string(),
                    title: "something".to_string(),
                    status: Some("in_progress".to_string()),
                    note: None,
                    repo: Some("bastion".to_string()),
                    blocked_by: vec![],
                }],
                next: vec![],
                blocked: vec![],
                deferred: Vec::new(),
            },
            tracks: vec![],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let src = StateSource {
            repo_slug: "hq".to_string(),
            abs_path: path,
            expected_kind: "brain",
        };
        let files = vec![(src.clone(), file.clone())];
        let graph = build_state_graph(&files);
        let diags = check_focus_drift(&src, &file, &BrainConfig::default(), &graph, &files);

        assert!(
            diags.is_empty(),
            "empty tracks[] should skip focus drift check, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_open_with_unclosed_block_dep_goes_to_blocked() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Block B is open but dep A is in_progress (not closed) → B goes in blocked.
        // Stored focus.blocked is empty → drift.
        let dep_a = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("in_progress"), vec![]),
                ("AL.1.B", Some("open"), vec![dep_a]),
            ],
            &["AL.1.A"], // stored now (correct)
            &[],         // stored next (correct — B has unmet dep)
            &[],         // stored blocked — stale (should be ["AL.1.B"])
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        let drifts: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert_eq!(
            drifts.len(),
            1,
            "B blocked by in_progress A should trigger drift, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_open_with_closed_dep_goes_to_next() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Block B is open; dep A is closed → B is ready (goes in next).
        // Stored focus matches exactly → no drift.
        let dep_a = BlockedBy::Block {
            repo: "alpha".to_string(),
            id: "AL.1.A".to_string(),
            what: None,
        };
        let pair = make_drift_pair(
            dir.path(),
            "alpha",
            &[
                ("AL.1.A", Some("closed"), vec![]),
                ("AL.1.B", Some("open"), vec![dep_a]),
            ],
            &[],         // stored now (A is closed, not in_progress)
            &["AL.1.B"], // stored next (B is ready)
            &[],         // stored blocked
        );
        let files = vec![pair];
        let graph = build_state_graph(&files);
        let (src, file) = &files[0];
        let diags = check_focus_drift(src, file, &BrainConfig::default(), &graph, &files);

        assert!(
            diags.is_empty(),
            "in-sync focus (B ready after A closed) should produce no diagnostics, \
             got: {diags:?}"
        );
    }

    // -----------------------------------------------------------------------
    // check_focus_drift — Facet B: kind-aware validation for dual-role brains
    // (brain-focus-dual-role-drift task 3/4)
    // -----------------------------------------------------------------------

    /// Build a dual-role `kind: "brain"` file (its own `tracks[]`: one
    /// `in_progress` block "CO.1.A" and one `open`+external-dep block
    /// "CO.1.B") plus one in-scope `kind: "project"` child ("alpha", one open
    /// ready block "AL.1.A"), and a `BrainConfig` that scopes the self repo
    /// ("core") to just that child's tier via [`tier_scope_for`].
    fn dual_role_drift_fixture(
        dir: &std::path::Path,
        stored_now: &[&str],
        stored_next: &[&str],
        stored_blocked: &[&str],
    ) -> (
        BrainConfig,
        (StateSource, StateFile),
        Vec<(StateSource, StateFile)>,
    ) {
        let config = make_mixed_tier_config();

        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [{ "id": "AL.1.A", "title": "Work" }], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{ "id": "AL.1.A", "title": "Work", "status": "open" }]
  }]
}"#;
        let pair_alpha = make_pair(dir, "alpha-state.json", "project", alpha_json);

        let make_block = |id: &str| Block {
            epics: Vec::new(),
            due: None,
            priority: None,
            id: id.to_string(),
            title: "placeholder".to_string(),
            status: None,
            note: None,
            repo: None,
            blocked_by: vec![],
        };
        let self_file = StateFile {
            epics: Vec::new(),
            repo: "core".to_string(),
            kind: "brain".to_string(),
            updated: "2026-06-29".to_string(),
            focus: Focus {
                now: stored_now.iter().map(|id| make_block(id)).collect(),
                next: stored_next.iter().map(|id| make_block(id)).collect(),
                blocked: stored_blocked.iter().map(|id| make_block(id)).collect(),
                deferred: Vec::new(),
            },
            tracks: vec![Track {
                title: "Own Track".to_string(),
                blocks: vec![
                    TrackBlock {
                        epics: Vec::new(),
                        due: None,
                        priority: None,
                        sdlc_workflow: None,
                        model: None,
                        id: "CO.1.A".to_string(),
                        title: "Own now work".to_string(),
                        status: Some("in_progress".to_string()),
                        depends_on: vec![],
                        wave: None,
                        origin: None,
                    },
                    TrackBlock {
                        epics: Vec::new(),
                        due: None,
                        priority: None,
                        sdlc_workflow: None,
                        model: None,
                        id: "CO.1.B".to_string(),
                        title: "Own blocked work".to_string(),
                        status: Some("open".to_string()),
                        depends_on: vec![BlockedBy::External {
                            what: "upstream dep".to_string(),
                        }],
                        wave: None,
                        origin: None,
                    },
                ],
            }],
            repos: vec![],
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        };
        let self_src = StateSource {
            repo_slug: "core".to_string(),
            abs_path: dir.join("core-state.json"),
            expected_kind: "brain",
        };

        (config, (self_src, self_file), vec![pair_alpha])
    }

    #[test]
    fn check_focus_drift_dual_role_brain_in_sync_produces_no_diagnostics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (config, (self_src, self_file), mut files) =
            dual_role_drift_fixture(dir.path(), &["CO.1.A"], &["AL.1.A"], &["CO.1.B"]);
        files.push((self_src.clone(), self_file.clone()));
        let graph = build_state_graph(&files);

        let diags = check_focus_drift(&self_src, &self_file, &config, &graph, &files);

        assert!(
            diags.is_empty(),
            "dual-role brain stored focus matching derive_brain_focus (children ∪ own \
             tracks[]) should produce no diagnostics, got: {diags:?}"
        );
    }

    #[test]
    fn check_focus_drift_dual_role_brain_stale_still_warns() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Stored focus is missing the self ready-now block "CO.1.A" — stale.
        let (config, (self_src, self_file), mut files) =
            dual_role_drift_fixture(dir.path(), &[], &["AL.1.A"], &["CO.1.B"]);
        files.push((self_src.clone(), self_file.clone()));
        let graph = build_state_graph(&files);

        let diags = check_focus_drift(&self_src, &self_file, &config, &graph, &files);

        let drifts: Vec<_> = diags
            .iter()
            .filter(|d| d.locator == "W_STATE_FOCUS_DRIFT")
            .collect();
        assert_eq!(
            drifts.len(),
            1,
            "a dual-role brain whose stored focus is missing a now-ready self \
             block must still warn, got: {diags:?}"
        );
    }

    #[test]
    fn state_graph_is_serializable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_a = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let files = vec![pair_a];

        let graph = build_state_graph(&files);
        let json = serde_json::to_string(&graph).expect("StateGraph must serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Must have "nodes" and "edges" arrays
        assert!(parsed["nodes"].is_array());
        assert!(parsed["edges"].is_array());

        // The node must carry key, repo, id, title but NOT source_path (skipped)
        let node = &parsed["nodes"][0];
        assert_eq!(node["key"], "alpha:AL.1.A");
        assert_eq!(node["repo"], "alpha");
        assert_eq!(node["id"], "AL.1.A");
        assert!(
            node["source_path"].is_null(),
            "source_path should be skipped in JSON"
        );
    }

    // -----------------------------------------------------------------------
    // TierScope / tier_scope_for / derive_rollup tier-scoping (MV.3B.U task 1)
    // -----------------------------------------------------------------------

    /// Config with a mix of core-tier and other-tier repos, mirroring the live
    /// `brain.toml` shape (an HQ-root entry plus several tier repos).
    fn make_mixed_tier_config() -> BrainConfig {
        use crate::brain::config::{CrawlConfig, RepoEntry, VocabConfig};
        BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/alpha".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "beta".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/beta".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "gamma".to_string(),
                    tier: "portfolio".to_string(),
                    repo_path: "portfolio/gamma".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
            ],
        }
    }

    fn brain_state_file(repo: &str, repos: Vec<RepoRollup>) -> StateFile {
        StateFile {
            epics: Vec::new(),
            repo: repo.to_string(),
            kind: "brain".to_string(),
            updated: "2026-07-01".to_string(),
            focus: Focus::default(),
            tracks: vec![],
            repos,
            cross_repo: vec![],
            tiers: vec![],
            note: None,
            backlog: vec![],
            carryover: vec![],
        }
    }

    #[test]
    fn tier_scope_for_returns_tier_when_repo_slug_matches_a_tier_name() {
        let config = make_mixed_tier_config();
        let core_brain = brain_state_file("core", vec![]);
        let scope = tier_scope_for(&core_brain, &config);
        assert_eq!(scope, TierScope::Tier("core".to_string()));
    }

    #[test]
    fn tier_scope_for_returns_all_when_repo_slug_matches_no_tier() {
        let config = make_mixed_tier_config();
        // "hq" (or "brain") is not itself a tier value in the config — it's the
        // HQ root's own repo slug — so it must scope to every repo.
        let hq_brain = brain_state_file("hq", vec![]);
        let scope = tier_scope_for(&hq_brain, &config);
        assert_eq!(scope, TierScope::All);
    }

    #[test]
    fn tier_scope_for_returns_tier_for_childless_tier_container_self_entry() {
        use crate::brain::config::{CrawlConfig, RepoEntry, VocabConfig};
        // A document-only brain tier with NO child repos carrying `tier =
        // "business"` — its only declaration is its own `_root` container
        // self-entry (`slug == repo_path == "business"`). It must still scope to
        // its own tier (not `All`, which would spuriously target it as HQ).
        let config = BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "business".to_string(),
                    tier: "_root".to_string(),
                    repo_path: "business".to_string(),
                    status_file: "business/planning/status.md".to_string(),
                    cache_doc: "business/index.md".to_string(),
                    heading: "business Sub-Brain".to_string(),
                    prefix: None,
                },
            ],
        };
        let business_brain = brain_state_file("business", vec![]);
        assert_eq!(
            tier_scope_for(&business_brain, &config),
            TierScope::Tier("business".to_string())
        );
        // The HQ root (repo_path ".") must remain `All`, not be caught by the
        // container-self-entry rule.
        let hq_brain = brain_state_file("brain", vec![]);
        assert_eq!(tier_scope_for(&hq_brain, &config), TierScope::All);
    }

    #[test]
    fn derive_rollup_core_scope_includes_only_core_tier_repos() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let files: Vec<(StateSource, StateFile)> = vec![];

        let rollups = derive_rollup(&scope, &config, &[], &StateGraph::default(), &files);

        let repos: Vec<&str> = rollups.iter().map(|r| r.repo.as_str()).collect();
        assert_eq!(
            repos,
            vec!["alpha", "beta"],
            "core scope must include only the two core-tier repos, in config order"
        );
    }

    #[test]
    fn derive_rollup_hq_scope_includes_every_tier() {
        let config = make_mixed_tier_config();
        let files: Vec<(StateSource, StateFile)> = vec![];

        let rollups = derive_rollup(
            &TierScope::All,
            &config,
            &[],
            &StateGraph::default(),
            &files,
        );

        let repos: Vec<&str> = rollups.iter().map(|r| r.repo.as_str()).collect();
        assert_eq!(
            repos,
            vec!["brain", "alpha", "beta", "gamma"],
            "All scope must include every configured repo"
        );
    }

    #[test]
    fn derive_rollup_derive_branch_uses_loaded_child_state() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_alpha = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let files = vec![pair_alpha];

        let rollups = derive_rollup(&scope, &config, &[], &StateGraph::default(), &files);

        // alpha has a loaded child → derived headline; beta has none → stub.
        let alpha = rollups.iter().find(|r| r.repo == "alpha").expect("alpha");
        assert_eq!(alpha.now.len(), 1);
        assert_eq!(alpha.now[0].id, "AL.1.A");
        assert_eq!(alpha.tier.as_deref(), Some("core"));

        let beta = rollups.iter().find(|r| r.repo == "beta").expect("beta");
        assert!(beta.now.is_empty() && beta.next.is_empty() && beta.blocked.is_empty());
        assert_eq!(beta.tier.as_deref(), Some("core"));
    }

    #[test]
    fn derive_rollup_preserve_branch_keeps_existing_entry_when_no_child_state() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let files: Vec<(StateSource, StateFile)> = vec![];

        // beta has no loadable child state.json, but the brain file already has
        // a hand-authored entry for it — this must be preserved verbatim
        // (this is the fix for the live bastion-drop incident).
        let existing = vec![RepoRollup {
            repo: "beta".to_string(),
            tier: None, // authored before tier was ever populated
            now: vec![Block {
                epics: Vec::new(),
                due: None,
                priority: None,
                id: "BE.1.A".to_string(),
                title: "Hand-authored headline".to_string(),
                status: Some("in_progress".to_string()),
                note: None,
                repo: None,
                blocked_by: vec![],
            }],
            next: vec![],
            blocked: vec![],
            deferred: Vec::new(),
        }];

        let rollups = derive_rollup(&scope, &config, &existing, &StateGraph::default(), &files);

        let beta = rollups.iter().find(|r| r.repo == "beta").expect("beta");
        assert_eq!(beta.now.len(), 1);
        assert_eq!(beta.now[0].id, "BE.1.A");
        assert_eq!(beta.now[0].title, "Hand-authored headline");
        // tier is backfilled from config even though the preserved entry had None.
        assert_eq!(beta.tier.as_deref(), Some("core"));
    }

    #[test]
    fn derive_rollup_stub_branch_emits_empty_tier_tagged_entry_when_neither_exists() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let files: Vec<(StateSource, StateFile)> = vec![];

        // No loaded child, no existing entry → a stub.
        let rollups = derive_rollup(&scope, &config, &[], &StateGraph::default(), &files);

        let alpha = rollups.iter().find(|r| r.repo == "alpha").expect("alpha");
        assert!(alpha.now.is_empty());
        assert!(alpha.next.is_empty());
        assert!(alpha.blocked.is_empty());
        assert_eq!(alpha.tier.as_deref(), Some("core"));
    }

    #[test]
    fn derive_rollup_tier_populated_in_every_branch() {
        let config = make_mixed_tier_config();
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_alpha = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let files = vec![pair_alpha];

        // beta: preserve branch.
        let existing = vec![RepoRollup {
            repo: "beta".to_string(),
            tier: None,
            now: vec![],
            next: vec![],
            blocked: vec![],
            deferred: Vec::new(),
        }];

        let rollups = derive_rollup(
            &TierScope::Tier("core".to_string()),
            &config,
            &existing,
            &StateGraph::default(),
            &files,
        );

        // alpha: derive branch. beta: preserve branch (no gamma — out of scope).
        assert_eq!(rollups.len(), 2);
        for rollup in &rollups {
            assert!(
                rollup.tier.is_some(),
                "tier must be populated for repo '{}' in every branch",
                rollup.repo
            );
        }
    }

    // -----------------------------------------------------------------------
    // derive_brain_focus — repo-tagged union of children's derived focus
    // (MV.3B.U task 2)
    // -----------------------------------------------------------------------

    #[test]
    fn derive_brain_focus_unions_two_children() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_alpha = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let pair_beta = leaf_pair(dir.path(), "beta", "BE.1.A");
        let files = vec![pair_alpha, pair_beta];
        let (self_src, self_file) = empty_brain_pair(dir.path(), "core");

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &scope,
            &config,
            &StateGraph::default(),
            &files,
        );

        let now_ids: Vec<&str> = focus.now.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            now_ids,
            vec!["AL.1.A", "BE.1.A"],
            "focus.now must union both children's now blocks, in config order"
        );
    }

    #[test]
    fn derive_brain_focus_tags_each_block_with_its_source_repo() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_alpha = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let files = vec![pair_alpha];
        let (self_src, self_file) = empty_brain_pair(dir.path(), "core");

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &scope,
            &config,
            &StateGraph::default(),
            &files,
        );

        assert_eq!(focus.now.len(), 1);
        assert_eq!(focus.now[0].repo.as_deref(), Some("alpha"));
    }

    #[test]
    fn derive_brain_focus_unions_child_deferred_blocks_repo_tagged() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let dir = tempfile::tempdir().expect("tempdir");
        let json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-07-26",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{ "id": "AL.9.A", "title": "Back burner", "status": "deferred" }]
  }]
}"#;
        let files = vec![make_pair(dir.path(), "alpha-state.json", "project", json)];
        let (self_src, self_file) = empty_brain_pair(dir.path(), "core");

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &scope,
            &config,
            &StateGraph::default(),
            &files,
        );

        assert_eq!(
            focus.deferred.len(),
            1,
            "child deferred must fold into union"
        );
        assert_eq!(focus.deferred[0].id, "AL.9.A");
        assert_eq!(focus.deferred[0].repo.as_deref(), Some("alpha"));
        assert_eq!(focus.deferred[0].status.as_deref(), Some("deferred"));
        assert!(focus.next.is_empty(), "must not leak into next");
        assert!(focus.blocked.is_empty(), "must not leak into blocked");
    }

    #[test]
    fn derive_rollup_carries_child_deferred_lane() {
        let config = make_mixed_tier_config();
        let dir = tempfile::tempdir().expect("tempdir");
        let json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-07-26",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{ "id": "AL.9.A", "title": "Back burner", "status": "deferred" }]
  }]
}"#;
        let files = vec![make_pair(dir.path(), "alpha-state.json", "project", json)];
        let graph = build_state_graph(&files);

        let rollups = derive_rollup(
            &TierScope::Tier("core".to_string()),
            &config,
            &[],
            &graph,
            &files,
        );

        let alpha = rollups
            .iter()
            .find(|r| r.repo == "alpha")
            .expect("alpha rollup");
        assert_eq!(alpha.deferred.len(), 1);
        assert_eq!(alpha.deferred[0].id, "AL.9.A");
        assert_eq!(alpha.deferred[0].status.as_deref(), Some("deferred"));
        assert!(alpha.next.is_empty());
    }

    #[test]
    fn derive_brain_focus_carries_priority_and_due_from_source_block() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let dir = tempfile::tempdir().expect("tempdir");
        let json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": {
    "now": [{ "id": "AL.1.A", "title": "Work", "status": "in_progress" }],
    "next": [{ "id": "AL.1.B", "title": "Next work" }],
    "blocked": []
  },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [
      { "id": "AL.1.A", "title": "Work", "status": "in_progress", "priority": 1, "due": "2026-07-10" },
      { "id": "AL.1.B", "title": "Next work", "priority": 2 }
    ]
  }]
}"#;
        let pair_alpha = make_pair(dir.path(), "alpha-state.json", "project", json);
        let files = vec![pair_alpha];
        let (self_src, self_file) = empty_brain_pair(dir.path(), "core");

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &scope,
            &config,
            &StateGraph::default(),
            &files,
        );

        assert_eq!(focus.now.len(), 1);
        assert_eq!(focus.now[0].priority, Some(1));
        assert_eq!(focus.now[0].due.as_deref(), Some("2026-07-10"));

        assert_eq!(focus.next.len(), 1);
        assert_eq!(focus.next[0].priority, Some(2));
        assert_eq!(
            focus.next[0].due, None,
            "block with no due date must carry None, not a fabricated value"
        );
    }

    #[test]
    fn derive_brain_focus_respects_tier_scope_exclusion() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let dir = tempfile::tempdir().expect("tempdir");
        // gamma is portfolio-tier — out of scope for the core brain.
        let pair_gamma = leaf_pair(dir.path(), "gamma", "GA.1.A");
        let files = vec![pair_gamma];
        let (self_src, self_file) = empty_brain_pair(dir.path(), "core");

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &scope,
            &config,
            &StateGraph::default(),
            &files,
        );

        assert!(
            focus.now.is_empty(),
            "gamma (portfolio tier) must not appear in a core-scoped brain focus"
        );
    }

    #[test]
    fn derive_brain_focus_dedups_by_repo_and_id_and_preserves_ordering() {
        use crate::brain::config::{CrawlConfig, RepoEntry, VocabConfig};

        // A config where "alpha" is (accidentally) listed twice — the second
        // listing must not produce a duplicate (repo, id) block in focus.now.
        let config = BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/alpha".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "beta".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/beta".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/alpha".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
            ],
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_alpha = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let pair_beta = leaf_pair(dir.path(), "beta", "BE.1.A");
        let files = vec![pair_alpha, pair_beta];
        let (self_src, self_file) = empty_brain_pair(dir.path(), "core");

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &TierScope::Tier("core".to_string()),
            &config,
            &StateGraph::default(),
            &files,
        );

        let now_pairs: Vec<(String, String)> = focus
            .now
            .iter()
            .map(|b| (b.repo.clone().unwrap_or_default(), b.id.clone()))
            .collect();
        assert_eq!(
            now_pairs,
            vec![
                ("alpha".to_string(), "AL.1.A".to_string()),
                ("beta".to_string(), "BE.1.A".to_string()),
            ],
            "duplicate (repo, id) entries must be deduped, config order preserved"
        );
    }

    // -----------------------------------------------------------------------
    // derive_brain_focus — Facet A: dual-role self-tracks folding
    // (brain-focus-dual-role-drift task 1/4)
    // -----------------------------------------------------------------------

    #[test]
    fn derive_brain_focus_dual_role_folds_self_tracks_with_children() {
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let dir = tempfile::tempdir().expect("tempdir");
        // Child: project-kind file with a single OPEN, dep-free block — ready,
        // so it lands in `next` (not `now`).
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [{ "id": "AL.1.A", "title": "Work" }], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{ "id": "AL.1.A", "title": "Work", "status": "open" }]
  }]
}"#;
        let pair_alpha = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair_alpha];
        let (self_src, self_file) = dual_role_brain_pair(dir.path(), "core");

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &scope,
            &config,
            &StateGraph::default(),
            &files,
        );

        // Self's own in_progress block must surface in `now`, tagged with the
        // self repo slug.
        let now_pairs: Vec<(Option<&str>, &str)> = focus
            .now
            .iter()
            .map(|b| (b.repo.as_deref(), b.id.as_str()))
            .collect();
        assert!(
            now_pairs.contains(&(Some("core"), "CORE.1.A")),
            "self ready-now block must be folded in, tagged with self slug, got: {now_pairs:?}"
        );

        // Child's own ready block must still surface in `next`, tagged with the
        // child repo slug — self-folding must not crowd out children.
        let next_pairs: Vec<(Option<&str>, &str)> = focus
            .next
            .iter()
            .map(|b| (b.repo.as_deref(), b.id.as_str()))
            .collect();
        assert!(
            next_pairs.contains(&(Some("alpha"), "AL.1.A")),
            "child ready block must still appear in next, got: {next_pairs:?}"
        );

        // Self's own blocked (unmet external dep) block must surface in
        // `blocked`, tagged with the self repo slug.
        let blocked_pairs: Vec<(Option<&str>, &str)> = focus
            .blocked
            .iter()
            .map(|b| (b.repo.as_deref(), b.id.as_str()))
            .collect();
        assert!(
            blocked_pairs.contains(&(Some("core"), "CORE.1.B")),
            "self blocked block must be folded in, tagged with self slug, got: {blocked_pairs:?}"
        );
    }

    #[test]
    fn derive_brain_focus_regression_pure_brain_empty_self_tracks_is_noop() {
        // A brain with NO own tracks[] (the ordinary tier sub-brain shape) must
        // derive an identical children-only union to the pre-Facet-A behaviour
        // — self-folding is strictly additive and a no-op when self tracks[]
        // is empty.
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_alpha = leaf_pair(dir.path(), "alpha", "AL.1.A");
        let pair_beta = leaf_pair(dir.path(), "beta", "BE.1.A");
        let files = vec![pair_alpha, pair_beta];
        let (self_src, self_file) = empty_brain_pair(dir.path(), "core");

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &scope,
            &config,
            &StateGraph::default(),
            &files,
        );

        let now_ids: Vec<&str> = focus.now.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(
            now_ids,
            vec!["AL.1.A", "BE.1.A"],
            "empty self tracks[] must fold nothing — byte-identical children-only union"
        );
        assert!(
            focus.now.iter().all(|b| b.repo.as_deref() != Some("core")),
            "no block should be tagged with the self slug when self tracks[] is empty"
        );
    }

    #[test]
    fn derive_brain_focus_unions_a_non_self_tier_brains_own_tracks() {
        // Regression for the backlog defect (2026-07-17): a NON-self tier
        // sub-brain root (kind: "brain", e.g. `business`, registered
        // `tier = "_root"` like the HQ root itself) that carries its own
        // `tracks[]` must fold into a `TierScope::All` union — not just the
        // `self_file` passed to `derive_brain_focus` (Facet A only folds the
        // literal self; a *sibling* tier root's own tracks were previously
        // dropped entirely because the union loop required `kind == "project"`).
        use crate::brain::config::{CrawlConfig, RepoEntry, VocabConfig};
        let config = BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                RepoEntry {
                    slug: "brain".to_string(),
                    tier: "_root".to_string(),
                    repo_path: ".".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "business".to_string(),
                    tier: "_root".to_string(),
                    repo_path: "business".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
                RepoEntry {
                    slug: "alpha".to_string(),
                    tier: "core".to_string(),
                    repo_path: "core/alpha".to_string(),
                    status_file: String::new(),
                    cache_doc: String::new(),
                    heading: String::new(),
                    prefix: None,
                },
            ],
        };
        let scope = TierScope::All;
        let dir = tempfile::tempdir().expect("tempdir");
        let pair_alpha = leaf_pair(dir.path(), "alpha", "AL.1.A");
        // `business` is a tier-brain root (kind: "brain") whose own tracks[]
        // authors a P0 revenue block — mirrors `business/planning/state.json`.
        let business_json = r#"{
  "repo": "business",
  "kind": "brain",
  "updated": "2026-07-17",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Business Ops",
    "blocks": [{ "id": "BZ.1.B", "title": "First client", "status": "open", "priority": 0 }]
  }]
}"#;
        let pair_business = make_pair(dir.path(), "business-state.json", "brain", business_json);
        let files = vec![pair_alpha, pair_business];
        let (self_src, self_file) = empty_brain_pair(dir.path(), "brain");

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &scope,
            &config,
            &StateGraph::default(),
            &files,
        );

        let next_pairs: Vec<(Option<&str>, &str)> = focus
            .next
            .iter()
            .map(|b| (b.repo.as_deref(), b.id.as_str()))
            .collect();
        assert!(
            next_pairs.contains(&(Some("business"), "BZ.1.B")),
            "a non-self tier sub-brain's own tracks[] must fold into the TierScope::All union, \
             tagged with its own slug, got: {next_pairs:?}"
        );
        let now_pairs: Vec<(Option<&str>, &str)> = focus
            .now
            .iter()
            .map(|b| (b.repo.as_deref(), b.id.as_str()))
            .collect();
        assert!(
            now_pairs.contains(&(Some("alpha"), "AL.1.A")),
            "leaf project children must still union alongside the tier-brain fold, got: {now_pairs:?}"
        );
        let business_block = focus
            .next
            .iter()
            .find(|b| b.repo.as_deref() == Some("business") && b.id == "BZ.1.B")
            .expect("business block present");
        assert_eq!(
            business_block.priority,
            Some(0),
            "priority must be carried through from the tier-brain's own tracks[]"
        );
    }

    #[test]
    fn derive_brain_focus_dedups_self_and_child_sharing_same_repo_and_id() {
        // Contrived: the self file's repo slug collides with an in-scope
        // child's slug (the same (repo, id) pair authored by both the self
        // tracks[] and a "project"-kind child sharing that slug) — the pair
        // must appear exactly once, not twice.
        let config = make_mixed_tier_config();
        let scope = TierScope::Tier("core".to_string());
        let dir = tempfile::tempdir().expect("tempdir");
        // Child: project-kind file for "alpha" with a ready (open) block
        // "AL.1.A" — must land in `next`, same bucket the self track below
        // targets, so the collision is exercised in the same bucket.
        let alpha_json = r#"{
  "repo": "alpha",
  "kind": "project",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [{ "id": "AL.1.A", "title": "Work" }], "blocked": [] },
  "tracks": [{
    "title": "Phase 1",
    "blocks": [{ "id": "AL.1.A", "title": "Work", "status": "open" }]
  }]
}"#;
        let pair_alpha = make_pair(dir.path(), "alpha-state.json", "project", alpha_json);
        let files = vec![pair_alpha];
        // Self: brain-kind file whose OWN repo slug is also "alpha" and whose
        // own tracks[] authors the exact same block id "AL.1.A".
        let self_json = r#"{
  "repo": "alpha",
  "kind": "brain",
  "updated": "2026-06-29",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [{
    "title": "Own Track",
    "blocks": [{ "id": "AL.1.A", "title": "Work", "status": "open" }]
  }]
}"#;
        let (self_src, self_file) =
            make_pair(dir.path(), "alpha-brain-state.json", "brain", self_json);

        let focus = derive_brain_focus(
            &self_src,
            &self_file,
            &scope,
            &config,
            &StateGraph::default(),
            &files,
        );

        let next_matches = focus
            .next
            .iter()
            .filter(|b| b.repo.as_deref() == Some("alpha") && b.id == "AL.1.A")
            .count();
        assert_eq!(
            next_matches, 1,
            "the (repo, id) pair authored by both self and a same-slug child must appear once, got: {:?}",
            focus.next
        );
    }

    // -----------------------------------------------------------------------
    // effective_priorities (MV.7.A) — reverse-topo min-propagation
    // -----------------------------------------------------------------------

    /// One test-fixture block: `(id, priority, depends_on)`, where each dep is
    /// a `(repo, id)` pair. Named to defuse clippy's `type_complexity` lint on
    /// the nested slice-of-tuples signature used by [`priority_pair`].
    type PriorityBlockSpec<'a> = (&'a str, Option<u8>, &'a [(&'a str, &'a str)]);

    /// Build a leaf `(StateSource, StateFile)` with one track of blocks, each
    /// `(id, priority)`, and `depends_on` edges declared inline as
    /// `{type:"block", repo, id}` entries so [`build_state_graph`] can derive
    /// the same graph the emit pipeline would see.
    fn priority_pair(
        dir: &std::path::Path,
        repo: &str,
        blocks: &[PriorityBlockSpec],
    ) -> (StateSource, StateFile) {
        let block_json: Vec<String> = blocks
            .iter()
            .map(|(id, priority, deps)| {
                let priority_field = match priority {
                    Some(p) => format!(r#""priority": {p},"#),
                    None => String::new(),
                };
                let deps_json: Vec<String> = deps
                    .iter()
                    .map(|(dep_repo, dep_id)| {
                        format!(r#"{{"type": "block", "repo": "{dep_repo}", "id": "{dep_id}"}}"#)
                    })
                    .collect();
                format!(
                    r#"{{ "id": "{id}", "title": "{id}", {priority_field} "depends_on": [{}] }}"#,
                    deps_json.join(", ")
                )
            })
            .collect();
        let json = format!(
            r#"{{
  "repo": "{repo}",
  "kind": "project",
  "updated": "2026-07-14",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [{{ "title": "Phase 1", "blocks": [{}] }}]
}}"#,
            block_json.join(", ")
        );
        make_pair(dir, &format!("{repo}-state.json"), "project", &json)
    }

    #[test]
    fn effective_priorities_eng_block_gating_p0_business_block_inherits_p0() {
        let dir = tempfile::tempdir().expect("tempdir");
        // eng:E.1 gates biz:B.1 (P0): biz:B.1 depends_on eng:E.1 (B.1 cannot
        // proceed until E.1 is done, so E.1 is what "gates" it).
        let eng = priority_pair(dir.path(), "eng", &[("E.1", None, &[])]);
        let biz = priority_pair(dir.path(), "biz", &[("B.1", Some(0), &[("eng", "E.1")])]);
        let files = vec![eng, biz];
        let graph = build_state_graph(&files);

        let effective = effective_priorities(&graph, &files);

        assert_eq!(
            effective.get("eng:E.1").copied(),
            Some(0),
            "an eng block with no own priority that gates a P0 business block \
             must inherit effective priority P0; got {effective:?}"
        );
        assert_eq!(effective.get("biz:B.1").copied(), Some(0));
    }

    #[test]
    fn effective_priorities_propagates_min_across_two_hops() {
        let dir = tempfile::tempdir().expect("tempdir");
        // biz:B1 (P0) depends_on a:A1 depends_on a:A2 — two hops of gating:
        // A1 directly gates B1, A2 gates A1 which gates B1.
        let a = priority_pair(
            dir.path(),
            "a",
            &[("A1", None, &[("a", "A2")]), ("A2", Some(3), &[])],
        );
        let biz = priority_pair(dir.path(), "biz", &[("B1", Some(0), &[("a", "A1")])]);
        let files = vec![a, biz];
        let graph = build_state_graph(&files);

        let effective = effective_priorities(&graph, &files);

        assert_eq!(
            effective.get("a:A1").copied(),
            Some(0),
            "the direct gate must inherit P0 from its dependent"
        );
        assert_eq!(
            effective.get("a:A2").copied(),
            Some(0),
            "the min must propagate two hops back to the root dependent"
        );
    }

    #[test]
    fn effective_priorities_block_with_no_hot_dependents_keeps_own_priority() {
        let dir = tempfile::tempdir().expect("tempdir");
        // solo:S has its own priority (2) and nothing depends on it.
        let solo = priority_pair(dir.path(), "solo", &[("S", Some(2), &[])]);
        let files = vec![solo];
        let graph = build_state_graph(&files);

        let effective = effective_priorities(&graph, &files);

        assert_eq!(effective.get("solo:S").copied(), Some(2));
    }

    #[test]
    fn effective_priorities_absent_own_priority_and_no_hot_dependents_is_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let solo = priority_pair(dir.path(), "solo", &[("S", None, &[])]);
        let files = vec![solo];
        let graph = build_state_graph(&files);

        let effective = effective_priorities(&graph, &files);

        assert!(
            !effective.contains_key("solo:S"),
            "a block with no own priority and no hot dependents must stay absent \
             (sorts last), got {effective:?}"
        );
    }

    #[test]
    fn effective_priorities_cycle_terminates_without_hang_or_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        // a:X depends_on b:Y, b:Y depends_on a:X — a two-node cycle.
        let graph = make_cycle_graph(
            dir.path(),
            &[("a", "X"), ("b", "Y")],
            &[("a:X", "b:Y"), ("b:Y", "a:X")],
        );
        let a = priority_pair(dir.path(), "a", &[("X", Some(1), &[])]);
        let b = priority_pair(dir.path(), "b", &[("Y", Some(2), &[])]);
        let files = vec![a, b];

        // Must return promptly (no hang) and not panic; exact values are not
        // load-bearing here, only that the pass terminates deterministically.
        let effective = effective_priorities(&graph, &files);
        let effective_again = effective_priorities(&graph, &files);
        assert_eq!(
            effective, effective_again,
            "effective_priorities over a cyclic graph must be deterministic"
        );
    }
}

// -----------------------------------------------------------------------
// Carryover tests
// -----------------------------------------------------------------------

#[test]
fn carryover_array_deserializes() {
    let json = r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-06-30",
  "carryover": [
    {
      "slug": "some-caveat",
      "scope": { "repo": "bastion" },
      "kind": "constraint",
      "text": "A durable caveat.",
      "created": "2026-06-30"
    }
  ]
}"#;
    let file: StateFile = serde_json::from_str(json).expect("should deserialize");
    assert_eq!(file.carryover.len(), 1);
    assert_eq!(file.carryover[0].slug, "some-caveat");
    assert_eq!(file.carryover[0].scope.repo.as_deref(), Some("bastion"));
    assert!(file.carryover[0].scope.tier.is_none());
    assert_eq!(file.carryover[0].kind, "constraint");
}

#[test]
fn carryover_schema_checks() {
    let json = r#"{
  "repo": "bastion",
  "kind": "project",
  "updated": "2026-06-30",
  "carryover": [
    {
      "slug": "bad-kind",
      "scope": { "repo": "bastion" },
      "kind": "unknown_kind",
      "text": "Bad kind.",
      "created": "2026-06-30"
    },
    {
      "slug": "bad-scope",
      "scope": { "repo": "bastion", "tier": "core" },
      "kind": "known_issue",
      "text": "Malformed scope.",
      "created": "2026-06-30"
    }
  ]
}"#;
    let file: StateFile = serde_json::from_str(json).unwrap();
    let src = StateSource {
        repo_slug: "bastion".to_string(),
        abs_path: PathBuf::from("planning/state.json"),
        expected_kind: "project",
    };
    let diags = check_schema(&src, &file);

    let bad_kind = diags
        .iter()
        .any(|d| d.locator == "E_STATE_SCHEMA_BAD_KIND" && d.message.contains("bad-kind"));
    let bad_scope = diags
        .iter()
        .any(|d| d.locator == "E_STATE_SCHEMA_MALFORMED_SCOPE" && d.message.contains("bad-scope"));

    assert!(bad_kind, "Should flag bad kind");
    assert!(bad_scope, "Should flag malformed scope");
}

// --- check_field_policy tests ---

#[cfg(test)]
mod check_field_policy_tests {
    use super::*;
    use std::path::PathBuf;

    fn run_field_policy(block: okf_core::TrackBlock) -> Vec<Diagnostic> {
        let mut file: StateFile =
            serde_json::from_str(tests::leaf_json("test_repo").as_str()).unwrap();
        file.tracks[0].blocks[0] = block;
        let src = StateSource {
            repo_slug: "test_repo".to_string(),
            abs_path: PathBuf::from("test.json"),
            expected_kind: "project",
        };
        check_field_policy(&src, &file)
    }

    fn base_block() -> okf_core::TrackBlock {
        okf_core::TrackBlock {
            epics: Vec::new(),
            id: "B.1".to_string(),
            title: "Test".to_string(),
            status: Some("open".to_string()),
            depends_on: vec![],
            wave: None,
            origin: None,
            priority: None,
            due: None,
            sdlc_workflow: None,
            model: None,
        }
    }

    #[test]
    fn test_valid_all_none() {
        assert!(run_field_policy(base_block()).is_empty());
    }

    #[test]
    fn test_priority_range() {
        let mut b = base_block();
        b.priority = Some(0);
        assert!(run_field_policy(b.clone()).is_empty());
        b.priority = Some(3);
        assert!(run_field_policy(b.clone()).is_empty());
        b.priority = Some(4);
        let diags = run_field_policy(b);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].locator, "E_STATE_PRIORITY_RANGE");
    }

    #[test]
    fn test_due_format() {
        let mut b = base_block();
        b.due = Some("2026-06-18".to_string());
        assert!(run_field_policy(b.clone()).is_empty());
        b.due = Some("Q3".to_string());
        let diags = run_field_policy(b.clone());
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].locator, "E_STATE_DUE_FORMAT");
        b.due = Some("2026-13-99".to_string());
        let diags = run_field_policy(b);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].locator, "E_STATE_DUE_FORMAT");
    }

    #[test]
    fn test_sdlc_workflow() {
        let mut b = base_block();
        for val in ["none", "patch", "task", "run", "flow"] {
            b.sdlc_workflow = Some(val.to_string());
            assert!(run_field_policy(b.clone()).is_empty());
        }
        b.sdlc_workflow = Some("pipeline".to_string());
        let diags = run_field_policy(b);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].locator, "E_STATE_SDLC_WORKFLOW_ENUM");
    }

    #[test]
    fn test_model() {
        let mut b = base_block();
        for val in ["sonnet", "gemini-pro", "gemini-flash", "either"] {
            b.model = Some(val.to_string());
            assert!(run_field_policy(b.clone()).is_empty());
        }
        b.model = Some("gpt".to_string());
        let diags = run_field_policy(b);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].locator, "E_STATE_MODEL_ENUM");
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod check_epics_tests {
    use super::*;

    /// Build a `(source, file)` pair from a JSON fixture, deriving the repo slug
    /// and path from the fixture itself.
    fn pair(dir: &std::path::Path, kind: &'static str, json: &str) -> (StateSource, StateFile) {
        let file: StateFile = serde_json::from_str(json).expect("fixture must parse");
        let path = dir.join(format!("{}-state.json", file.repo));
        std::fs::write(&path, json).unwrap();
        let src = StateSource {
            repo_slug: file.repo.clone(),
            abs_path: path,
            expected_kind: kind,
        };
        (src, file)
    }

    /// A config whose HQ file (`repo: "hq"`) matches no declared tier, so
    /// `tier_scope_for` resolves it to `TierScope::All` — the registry holder.
    fn epic_config() -> BrainConfig {
        use crate::brain::config::{CrawlConfig, RepoEntry, VocabConfig};
        let entry = |slug: &str, tier: &str, path: &str| RepoEntry {
            slug: slug.to_string(),
            tier: tier.to_string(),
            repo_path: path.to_string(),
            status_file: String::new(),
            cache_doc: String::new(),
            heading: String::new(),
            prefix: None,
        };
        BrainConfig {
            attention: Default::default(),
            vocab: VocabConfig::default(),
            crawl: CrawlConfig::default(),
            repos: vec![
                entry("hq", "_root", "."),
                entry("bastion", "core", "core/bastion"),
                entry("mev", "core", "core/mev"),
            ],
        }
    }

    /// Collect just the locators, for order-insensitive assertions.
    fn locators(diags: &[Diagnostic]) -> Vec<&str> {
        diags.iter().map(|d| d.locator.as_str()).collect()
    }

    #[test]
    fn check_epics_accepts_valid_membership() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hq = pair(
            dir.path(),
            "brain",
            r#"{
  "repo": "hq", "kind": "brain", "updated": "2026-07-24",
  "epics": [
    { "slug": "bastion-os", "title": "Bastion OS", "status": "active" },
    { "slug": "bastion-web", "title": "Bastion Web + UI", "status": "active" }
  ]
}"#,
        );
        let bastion = pair(
            dir.path(),
            "project",
            r#"{
  "repo": "bastion", "kind": "project", "updated": "2026-07-24",
  "tracks": [{ "title": "P", "blocks": [
    { "id": "BA.11.K", "title": "board endpoint", "status": "closed",
      "epics": ["bastion-os", "bastion-web"] }
  ]}]
}"#,
        );

        let diags = check_epics(&epic_config(), &[hq, bastion]);
        assert!(
            diags.is_empty(),
            "valid multi-epic membership must be clean, got: {:?}",
            locators(&diags)
        );
    }

    #[test]
    fn check_epics_flags_an_unknown_slug() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hq = pair(
            dir.path(),
            "brain",
            r#"{
  "repo": "hq", "kind": "brain", "updated": "2026-07-24",
  "epics": [{ "slug": "bastion-os", "title": "Bastion OS", "status": "active" }]
}"#,
        );
        // `bastion-osx` is the typo this check exists to catch — without it the
        // block would silently belong to no board at all.
        let bastion = pair(
            dir.path(),
            "project",
            r#"{
  "repo": "bastion", "kind": "project", "updated": "2026-07-24",
  "tracks": [{ "title": "P", "blocks": [
    { "id": "BA.1.A", "title": "typo'd", "status": "closed", "epics": ["bastion-osx"] }
  ]}]
}"#,
        );

        let diags = check_epics(&epic_config(), &[hq, bastion]);
        assert!(
            locators(&diags).contains(&"E_STATE_UNKNOWN_EPIC"),
            "got: {:?}",
            locators(&diags)
        );
        assert!(
            diags.iter().any(|d| d.message.contains("bastion-osx")),
            "the diagnostic must name the offending slug: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn check_epics_flags_duplicate_slugs_and_bad_status() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hq = pair(
            dir.path(),
            "brain",
            r#"{
  "repo": "hq", "kind": "brain", "updated": "2026-07-24",
  "epics": [
    { "slug": "dup", "title": "First", "status": "active" },
    { "slug": "dup", "title": "Second", "status": "in-flight" }
  ],
  "tracks": [{ "title": "P", "blocks": [
    { "id": "HQ.1.A", "title": "member", "status": "closed", "epics": ["dup"] }
  ]}]
}"#,
        );

        let found = check_epics(&epic_config(), &[hq]);
        let found = locators(&found);
        assert!(
            found.contains(&"E_STATE_DUPLICATE_EPIC_SLUG"),
            "got: {found:?}"
        );
        assert!(found.contains(&"E_STATE_EPIC_BAD_STATUS"), "got: {found:?}");
    }

    #[test]
    fn check_epics_warns_on_an_epic_with_no_members() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hq = pair(
            dir.path(),
            "brain",
            r#"{
  "repo": "hq", "kind": "brain", "updated": "2026-07-24",
  "epics": [{ "slug": "ghost", "title": "Nobody's work", "status": "active" }]
}"#,
        );

        let diags = check_epics(&epic_config(), &[hq]);
        assert_eq!(locators(&diags), vec!["W_STATE_EPIC_EMPTY"]);
        assert!(
            diags[0].severity != crate::Severity::Error,
            "an empty epic must never fail the exit code"
        );
    }

    /// Build a one-epic HQ registry plus a member block, so the only diagnostics
    /// that can fire are registry well-formedness ones (never `W_STATE_EPIC_EMPTY`).
    fn hq_with_epic(dir: &Path, epic_json: &str) -> (StateSource, StateFile) {
        pair(
            dir,
            "brain",
            &format!(
                r#"{{
  "repo": "hq", "kind": "brain", "updated": "2026-08-01",
  "epics": [{epic_json}],
  "tracks": [{{ "title": "P", "blocks": [
    {{ "id": "HQ.1.A", "title": "member", "status": "open", "epics": ["w"] }}
  ]}}]
}}"#
            ),
        )
    }

    #[test]
    fn check_epics_flags_a_weight_above_100() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (src, file) = hq_with_epic(
            dir.path(),
            r#"{ "slug": "w", "title": "W", "weight": 101 }"#,
        );
        let hq_path = src.abs_path.clone();

        let diags = check_epics(&epic_config(), &[(src, file)]);
        assert_eq!(
            locators(&diags),
            vec!["E_STATE_EPIC_BAD_WEIGHT"],
            "101 is one past the inclusive bound and must raise exactly one error"
        );
        assert_eq!(diags[0].severity, crate::Severity::Error);
        assert_eq!(
            diags[0].file, hq_path,
            "the weight error is reported against the HQ state.json, like E_STATE_EPIC_BAD_STATUS"
        );
        assert!(
            diags[0].message.contains('w') && diags[0].message.contains("101"),
            "the diagnostic must name the epic and the offending value: {}",
            diags[0].message
        );
    }

    #[test]
    fn check_epics_accepts_weights_at_and_inside_the_bounds() {
        // 100 is the inclusive max, 0 is a legitimate authored value (not "absent"),
        // and an absent weight is the overwhelmingly common case today.
        for body in [
            r#"{ "slug": "w", "title": "W", "weight": 100 }"#,
            r#"{ "slug": "w", "title": "W", "weight": 0 }"#,
            r#"{ "slug": "w", "title": "W" }"#,
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let diags = check_epics(&epic_config(), &[hq_with_epic(dir.path(), body)]);
            assert!(
                diags.is_empty(),
                "{body} must be clean, got: {:?}",
                locators(&diags)
            );
        }
    }

    #[test]
    fn check_epics_flags_the_extreme_weight_a_u8_still_accepts() {
        // `weight` is a u8, so 255 parses fine — the range check is real
        // validation, which is exactly why it lives here and not in serde.
        let dir = tempfile::tempdir().expect("tempdir");
        let diags = check_epics(
            &epic_config(),
            &[hq_with_epic(
                dir.path(),
                r#"{ "slug": "w", "title": "W", "weight": 255 }"#,
            )],
        );
        assert_eq!(locators(&diags), vec!["E_STATE_EPIC_BAD_WEIGHT"]);
    }

    #[test]
    fn check_epics_warns_when_a_non_hq_file_declares_a_registry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hq = pair(
            dir.path(),
            "brain",
            r#"{ "repo": "hq", "kind": "brain", "updated": "2026-07-24" }"#,
        );
        // A shadow registry on a leaf: silently ignored, so it must be flagged.
        let mev = pair(
            dir.path(),
            "project",
            r#"{
  "repo": "mev", "kind": "project", "updated": "2026-07-24",
  "epics": [{ "slug": "local-only", "title": "Shadow", "status": "active" }]
}"#,
        );

        let diags = check_epics(&epic_config(), &[hq, mev]);
        assert_eq!(locators(&diags), vec!["W_STATE_EPIC_REGISTRY_IGNORED"]);
    }

    #[test]
    fn check_epics_warns_on_an_untagged_open_gate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hq = pair(
            dir.path(),
            "brain",
            r#"{
  "repo": "hq", "kind": "brain", "updated": "2026-07-24",
  "epics": [{ "slug": "bastion-web", "title": "Bastion Web", "status": "active" }]
}"#,
        );
        let bastion = pair(
            dir.path(),
            "project",
            r#"{
  "repo": "bastion", "kind": "project", "updated": "2026-07-24",
  "tracks": [{ "title": "P", "blocks": [
    { "id": "BA.9.A", "title": "untagged open gate", "status": "open" },
    { "id": "BA.9.B", "title": "untagged but closed", "status": "closed" }
  ]}]
}"#,
        );
        let web = pair(
            dir.path(),
            "project",
            r#"{
  "repo": "bastion-web", "kind": "project", "updated": "2026-07-24",
  "tracks": [{ "title": "P", "blocks": [
    { "id": "BW.1.C", "title": "epic member", "status": "open",
      "epics": ["bastion-web"],
      "depends_on": [
        { "type": "block", "repo": "bastion", "id": "BA.9.A" },
        { "type": "block", "repo": "bastion", "id": "BA.9.B" },
        { "type": "external", "what": "a hardware session" }
      ]
    }
  ]}]
}"#,
        );

        let diags = check_epics(&epic_config(), &[hq, bastion, web]);
        let found = locators(&diags);
        assert_eq!(
            found,
            vec!["W_STATE_EPIC_UNREACHABLE_DEP"],
            "only the OPEN untagged dep is a silent gate: a closed one is already \
             satisfied and an external one has no target node; got {found:?}"
        );
        assert!(
            diags[0].message.contains("BA.9.A"),
            "the warning must name the gating block: {}",
            diags[0].message
        );
    }

    // -----------------------------------------------------------------------
    // derive_epic_focus / derive_epic_edges
    // -----------------------------------------------------------------------

    fn focus_entry(repo: &str, id: &str, epics: &[&str]) -> Block {
        Block {
            id: id.to_string(),
            title: format!("{id} title"),
            status: None,
            note: None,
            repo: Some(repo.to_string()),
            blocked_by: Vec::new(),
            priority: None,
            due: None,
            epics: epics.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn derive_epic_focus_narrows_the_union_and_keeps_order() {
        let focus = Focus {
            now: vec![focus_entry("bastion", "BA.1", &["bastion-os"])],
            next: vec![
                focus_entry("mev", "MV.1", &["bastion-os"]),
                focus_entry("amistad", "AM.1", &[]),
                // A shared block must appear on BOTH epics' boards.
                focus_entry("bastion", "BA.2", &["bastion-os", "bastion-web"]),
            ],
            blocked: vec![focus_entry("bastion-web", "BW.1", &["bastion-web"])],
            deferred: Vec::new(),
        };

        let os = derive_epic_focus(&focus, "bastion-os");
        assert_eq!(
            os.now.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            vec!["BA.1"]
        );
        assert_eq!(
            os.next.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            vec!["MV.1", "BA.2"],
            "unrelated blocks drop out; surviving order (the effective-priority \
             sort) is preserved"
        );
        assert!(os.blocked.is_empty());

        let web = derive_epic_focus(&focus, "bastion-web");
        assert_eq!(
            web.next.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            vec!["BA.2"],
            "a block in two epics is a member of both boards"
        );
        assert_eq!(
            web.blocked
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>(),
            vec!["BW.1"]
        );

        let none = derive_epic_focus(&focus, "no-such-epic");
        assert!(none.now.is_empty() && none.next.is_empty() && none.blocked.is_empty());
    }

    /// Fixture: `bastion-web`'s BW.1.C depends on bastion's BA.7.D (a
    /// `bastion-os` block) and on an untagged BA.9.A; engine-rs's EN.2.B
    /// depends back on BW.1.C. Mirrors the real cross-initiative shape.
    fn epic_edge_files(dir: &std::path::Path) -> Vec<(StateSource, StateFile)> {
        vec![
            pair(
                dir,
                "project",
                r#"{
  "repo": "bastion", "kind": "project", "updated": "2026-07-24",
  "tracks": [{ "title": "P", "blocks": [
    { "id": "BA.7.D", "title": "os block", "status": "open", "epics": ["bastion-os"] },
    { "id": "BA.9.A", "title": "untagged", "status": "closed" }
  ]}]
}"#,
            ),
            pair(
                dir,
                "project",
                r#"{
  "repo": "bastion-web", "kind": "project", "updated": "2026-07-24",
  "tracks": [{ "title": "P", "blocks": [
    { "id": "BW.1.C", "title": "web block", "status": "open", "epics": ["bastion-web"],
      "depends_on": [
        { "type": "block", "repo": "bastion", "id": "BA.7.D" },
        { "type": "block", "repo": "bastion", "id": "BA.9.A" },
        { "type": "block", "repo": "bastion", "id": "BA.NOPE" },
        { "type": "external", "what": "a hardware session" }
      ] },
    { "id": "BW.2.A", "title": "sibling", "status": "open", "epics": ["bastion-web"],
      "depends_on": [{ "type": "block", "repo": "bastion-web", "id": "BW.1.C" }] }
  ]}]
}"#,
            ),
            pair(
                dir,
                "project",
                r#"{
  "repo": "engine-rs", "kind": "project", "updated": "2026-07-24",
  "tracks": [{ "title": "P", "blocks": [
    { "id": "EN.2.B", "title": "engine block", "status": "open", "epics": ["engine-split"],
      "depends_on": [{ "type": "block", "repo": "bastion-web", "id": "BW.1.C" }] }
  ]}]
}"#,
            ),
        ]
    }

    #[test]
    fn derive_epic_edges_splits_boundary_edges_by_direction() {
        let dir = tempfile::tempdir().expect("tempdir");
        let files = epic_edge_files(dir.path());

        let edges = derive_epic_edges(&files, "bastion-web");

        // Outbound: what bastion-web waits on outside itself.
        let out: Vec<(&str, &str)> = edges
            .outbound
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        assert_eq!(
            out,
            vec![
                ("bastion-web:BW.1.C", "bastion:BA.7.D"),
                ("bastion-web:BW.1.C", "bastion:BA.9.A"),
            ],
            "the internal BW.2.C→BW.1.C edge, the external dep, and the dangling \
             BA.NOPE target must all be excluded; got {out:?}"
        );

        // The bastion-os counterpart is attributed to its epic; the untagged one
        // reports no epics at all.
        assert_eq!(edges.outbound[0].other_epics, vec!["bastion-os"]);
        assert!(edges.outbound[1].other_epics.is_empty());

        // BA.7.D is open → still gating. BA.9.A is closed → satisfied.
        assert!(edges.outbound[0].blocking);
        assert!(!edges.outbound[1].blocking);

        // Inbound: what bastion-web is holding up.
        let inb: Vec<(&str, &str)> = edges
            .inbound
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        assert_eq!(inb, vec![("engine-rs:EN.2.B", "bastion-web:BW.1.C")]);
        assert_eq!(edges.inbound[0].other_epics, vec!["engine-split"]);
        assert!(edges.inbound[0].blocking);
    }

    #[test]
    fn derive_epic_edges_ignores_a_block_shared_by_both_endpoints() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hq = pair(
            dir.path(),
            "project",
            r#"{
  "repo": "bastion", "kind": "project", "updated": "2026-07-24",
  "tracks": [{ "title": "P", "blocks": [
    { "id": "BA.1", "title": "shared gate", "status": "open",
      "epics": ["bastion-os", "bastion-web"] },
    { "id": "BA.2", "title": "web block", "status": "open", "epics": ["bastion-web"],
      "depends_on": [{ "type": "block", "repo": "bastion", "id": "BA.1" }] }
  ]}]
}"#,
        );

        let edges = derive_epic_edges(&[hq], "bastion-web");
        assert_eq!(
            edges,
            EpicEdges::default(),
            "both endpoints are inside bastion-web, so this is not a boundary edge"
        );
    }

    #[test]
    fn check_epics_is_silent_on_an_untagged_corpus() {
        // Today's state: no registry, no membership anywhere. Adding this check
        // must not produce a single diagnostic until epics are actually adopted.
        let dir = tempfile::tempdir().expect("tempdir");
        let hq = pair(
            dir.path(),
            "brain",
            r#"{ "repo": "hq", "kind": "brain", "updated": "2026-07-24" }"#,
        );
        let leaf = pair(
            dir.path(),
            "project",
            r#"{
  "repo": "mev", "kind": "project", "updated": "2026-07-24",
  "tracks": [{ "title": "P", "blocks": [
    { "id": "MV.1.A", "title": "untagged", "status": "open",
      "depends_on": [{ "type": "block", "repo": "mev", "id": "MV.0.A" }] }
  ]}]
}"#,
        );

        let diags = check_epics(&epic_config(), &[hq, leaf]);
        assert!(diags.is_empty(), "got: {:?}", locators(&diags));
    }
}
