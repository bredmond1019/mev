//! Six-state lane-segment availability — `MV.13.C` Task 1.
//!
//! [`FrontierEntry`] (`MV.13.B`) already tells us, per `(roadmap, lane, segment)`,
//! whether a head exists and what it is waiting on intrinsically (`unmet_blocks`,
//! `unmet_gates`). This module folds that — plus environmental holds this block adds
//! in later tasks (repo-busy, slot-capacity) — into exactly one of six
//! [`SegmentAvailability`] states per segment, so downstream consumers (bastion's
//! `/lanes` endpoint, the cockpit board) never re-derive the same judgement call
//! themselves.
//!
//! ## The single source of truth for "a lane is live in repo X" (`MV.13.C` Task 2)
//!
//! **Decided here, at spec time, and not re-litigated in code: the single source of
//! truth for lane liveness is the per-`(repo, roadmap)` orchestration-run record's
//! `lifecycle:` frontmatter** — `planning/orchestration-run/<roadmap-slug>/notes.md`,
//! `lifecycle: active | lane-complete | consolidated` ([D57], the orchestration-run
//! artifact contract). Two other candidates were considered and rejected:
//!
//! - **`lane-log.jsonl`** records **integrated blocks**, not liveness. A lane that
//!   opened and is mid-block has written nothing to it yet, so it reads as idle
//!   exactly when it is busiest. It remains the cross-lane progress channel and is
//!   read by nothing in this module.
//! - **`fleet_concurrency_check.py`'s `.fleet-locks` registry** only ever knows about
//!   **heavy** repos (`heavy_category` returns `None` for a light one), so it
//!   structurally cannot answer the liveness question for the light half of the
//!   fleet. It is the source for `HeldSlot` (Task 3) and for nothing else.
//!
//! The run record is the only candidate that covers every repo, is written when the
//! lane **opens** rather than when a block closes, and is contract-validated
//! (`test_orchestration_run_contract.py`).
//!
//! [D57]: ../../../../base-template/planning/decisions/D57-orchestration-run-artifact-contract.md

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::Diagnostic;
use crate::brain::block_graph::{BlockGraphScope, build_block_graph_export};
use crate::brain::config::{RepoEntry, load_brain_config};
use crate::brain::emit::{EmitAction, EmitPlan};
use crate::brain::frontier::{Frontier, FrontierEntry, compute_frontier, ensure_untruncated};
use crate::brain::lane_segments::{
    DerivedBlockPosition, build_owner_index, derive_lane_positions, discover_lane_files,
    unresolved_owner_diagnostics,
};
use crate::brain::lock::pid_is_alive;
use crate::brain::state::{
    StateEdgeKind, StateFile, StateGraph, StateSource, TierScope, build_state_graph,
    effective_priorities,
};
use crate::shared::extract_frontmatter;

/// The six possible availability states for one lane segment.
///
/// A segment can genuinely match more than one condition at once — e.g. a head with
/// both an unmet block *and* a busy repo. **Exactly one state is reported per
/// segment**, per this fixed, documented precedence (highest first):
///
/// `Done` > `HeldBlock` > `HeldOperator` > `HeldRepoBusy` > `HeldSlot` > `Startable`.
///
/// Intrinsic reasons (the segment's own dependency graph: `Done`, `HeldBlock`,
/// `HeldOperator`) outrank environmental ones (`HeldRepoBusy`, `HeldSlot`) because the
/// intrinsic reason is the one an operator can act on — closing the blocking block or
/// clearing the gate — and it does not change just because some unrelated lane exits
/// and frees a repo or a concurrency slot. Environmental holds are true but transient;
/// intrinsic holds are the actual next action.
///
/// This task (`MV.13.C` Task 1) implements only the intrinsic tier: `Done`,
/// `HeldBlock`, `HeldOperator`, `Startable`. `HeldRepoBusy` (Task 2) and `HeldSlot`
/// (Task 3) are environmental and layered on top without disturbing this precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SegmentAvailability {
    /// The segment has no frontier entry — every block in it is closed.
    Done,
    /// The head's `unmet_blocks` is non-empty.
    HeldBlock,
    /// The head's `unmet_gates` is non-empty (and `unmet_blocks` is empty).
    HeldOperator,
    /// The head's repo has an `active` orchestration-run record for a *different*
    /// roadmap (Task 2). Environmental; ranks below every intrinsic state.
    HeldRepoBusy,
    /// The head's repo is a heavy repo whose concurrency category is at capacity
    /// (Task 3). Environmental; ranks below every intrinsic state and below
    /// `HeldRepoBusy`.
    HeldSlot,
    /// No intrinsic or environmental hold applies — the segment can be worked now.
    Startable,
}

/// One segment's resolved availability, plus the human-readable reason and enough
/// identity (`roadmap`/`lane`/`segment`/`repo`/`head`) for a consumer to act without
/// re-deriving anything.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SegmentStatus {
    pub roadmap: String,
    pub lane: String,
    pub segment: usize,
    pub repo: String,
    /// Canonical `"repo:id"` key of the segment's head block, or `None` for `Done`
    /// (the segment has no frontier entry — there is no head).
    pub head: Option<String>,
    pub availability: SegmentAvailability,
    /// Human-readable why, e.g. `"blocked by bastion:BA.19.C"`. `None` only for
    /// `Startable` and `Done`, which need no explanation.
    pub reason: Option<String>,
}

/// Resolve the intrinsic availability tier for one [`FrontierEntry`] — `Done` is
/// handled by the caller (an entry's mere absence from the frontier), so this only
/// ever returns `HeldBlock`, `HeldOperator`, or `Startable`.
///
/// Precedence within the intrinsic tier: a head matching both `unmet_blocks` and
/// `unmet_gates` reports `HeldBlock`, per [`SegmentAvailability`]'s documented order.
fn intrinsic_status(entry: &FrontierEntry) -> (SegmentAvailability, Option<String>) {
    if !entry.unmet_blocks.is_empty() {
        return (
            SegmentAvailability::HeldBlock,
            Some(format!("blocked by {}", entry.unmet_blocks.join(", "))),
        );
    }
    if !entry.unmet_gates.is_empty() {
        return (
            SegmentAvailability::HeldOperator,
            Some(format!("blocked by {}", entry.unmet_gates.join(", "))),
        );
    }
    (SegmentAvailability::Startable, None)
}

/// One segment's identity — `(roadmap, lane, segment)` plus its owning `repo` — the
/// full segment list [`intrinsic_segment_statuses`], [`segment_statuses`], and
/// [`segment_statuses_with_slots`] use to detect segments with no frontier entry
/// (every block in the segment is closed) and emit `Done` for them.
///
/// Derived from `lane_positions` via [`discover_segments`] — the same
/// [`DerivedBlockPosition`] list [`crate::brain::frontier::compute_frontier`] groups
/// internally — rather than re-parsing lane files a second, independent way, so the
/// frontier's live subset and this module's full segment set agree by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSegment {
    pub roadmap: String,
    pub lane: String,
    pub segment: usize,
    pub repo: String,
}

/// Group `lane_positions` into one [`DiscoveredSegment`] per `(roadmap, lane,
/// segment)`, first-seen order — the same grouping
/// [`crate::brain::frontier::compute_frontier`] performs internally to find each
/// segment's head, except every group survives here (that function drops a group
/// whose every block is closed; this one is what lets a `Done` segment be told apart
/// from a segment that does not exist at all).
pub fn discover_segments(lane_positions: &[DerivedBlockPosition]) -> Vec<DiscoveredSegment> {
    let mut order: Vec<(String, String, usize)> = Vec::new();
    let mut repos: HashMap<(String, String, usize), String> = HashMap::new();
    for p in lane_positions {
        let key = (p.roadmap.clone(), p.lane.clone(), p.segment);
        repos.entry(key.clone()).or_insert_with(|| {
            order.push(key);
            p.repo.clone()
        });
    }
    order
        .into_iter()
        .map(|key| DiscoveredSegment {
            repo: repos.remove(&key).unwrap_or_default(),
            roadmap: key.0,
            lane: key.1,
            segment: key.2,
        })
        .collect()
}

/// Append a `Done` [`SegmentStatus`] for every entry in `segments` that has no
/// matching entry in `frontier` — i.e. every block in that segment is closed.
/// Matched on the `(roadmap, lane, segment)` identity triple only, never on a head
/// block id (a `Done` segment has none).
///
/// Appended after the frontier-derived statuses already in `statuses`, in
/// `segments`' own (first-seen) order — the existing (live) entries keep exactly the
/// order they had before this function ran; nothing is re-sorted to interleave the
/// two, per this task's ordering requirement.
fn append_done_segments(
    statuses: &mut Vec<SegmentStatus>,
    frontier: &Frontier,
    segments: &[DiscoveredSegment],
) {
    let present: HashSet<(String, String, usize)> = frontier
        .entries
        .iter()
        .map(|e| (e.roadmap.clone(), e.lane.clone(), e.segment))
        .collect();
    for seg in segments {
        let key = (seg.roadmap.clone(), seg.lane.clone(), seg.segment);
        if present.contains(&key) {
            continue;
        }
        statuses.push(SegmentStatus {
            roadmap: seg.roadmap.clone(),
            lane: seg.lane.clone(),
            segment: seg.segment,
            repo: seg.repo.clone(),
            head: None,
            availability: SegmentAvailability::Done,
            reason: None,
        });
    }
}

/// Compute intrinsic [`SegmentStatus`]es for every entry in `frontier`, plus a `Done`
/// status for every entry in `segments` the frontier has no record of (every block in
/// that segment is closed — [`crate::brain::frontier::compute_frontier`] skips such
/// segments entirely, so their only trace is their absence). `segments` should be
/// [`discover_segments`] run over the same `lane_positions` `frontier` was computed
/// from, so the two agree by construction; passing a `segments` list that disagrees
/// with `frontier`'s own derivation is a caller bug, not something this function
/// detects.
pub fn intrinsic_segment_statuses(
    frontier: &Frontier,
    segments: &[DiscoveredSegment],
) -> Vec<SegmentStatus> {
    let mut statuses: Vec<SegmentStatus> = frontier
        .entries
        .iter()
        .map(|entry| {
            let (availability, reason) = intrinsic_status(entry);
            SegmentStatus {
                roadmap: entry.roadmap.clone(),
                lane: entry.lane.clone(),
                segment: entry.segment,
                repo: entry.repo.clone(),
                head: Some(entry.key.clone()),
                availability,
                reason,
            }
        })
        .collect();
    append_done_segments(&mut statuses, frontier, segments);
    statuses
}

// ---------------------------------------------------------------------------
// held-repo-busy — MV.13.C Task 2
// ---------------------------------------------------------------------------

/// One repo's known-live lane, read from an orchestration-run record whose
/// `lifecycle:` frontmatter is `active`. See the module doc comment for why this
/// record — and not `lane-log.jsonl` or `.fleet-locks` — is the single source of
/// truth for "a lane is live in repo X".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveRun {
    /// `[[repos]]` slug of the repo the record belongs to.
    pub repo: String,
    /// The roadmap this record's `roadmap:` frontmatter names.
    pub roadmap: String,
}

/// The subset of an orchestration-run record's frontmatter this module reads.
/// Unknown fields are tolerated (no `deny_unknown_fields`) — the full record
/// carries many more fields this module has no use for.
#[derive(Debug, Deserialize)]
struct RunRecordFrontmatter {
    #[serde(default)]
    roadmap: Option<String>,
    #[serde(default)]
    lifecycle: Option<String>,
}

/// Read one `notes.md`'s frontmatter as a [`RunRecordFrontmatter`].
///
/// Returns `Err` (never a silent `None`) when the file cannot be read, carries no
/// frontmatter block, or the block does not parse as YAML — the caller turns every
/// `Err` into a [`Diagnostic`] rather than inventing a hold or dropping the record.
fn read_run_record(path: &Path) -> Result<RunRecordFrontmatter, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let yaml = extract_frontmatter(&contents)
        .ok_or_else(|| format!("no frontmatter block found in {}", path.display()))?;
    serde_yaml::from_str::<RunRecordFrontmatter>(yaml)
        .map_err(|e| format!("could not parse frontmatter in {}: {e}", path.display()))
}

/// Sorted list of `<dir>`'s direct child directories, best-effort (an unreadable or
/// absent `dir` yields an empty list rather than an error — the caller already
/// treats "no `orchestration-run/` directory in this repo" as the normal case for a
/// repo that has never run an orchestrated lane).
fn child_dirs_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(read) => read
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

/// Walk every `[[repos]]` entry's `planning/orchestration-run/*/notes.md` under
/// `root` and return the `active` [`LiveRun`]s, plus a [`Diagnostic`] for every
/// record whose frontmatter could not be read/parsed or is missing the fields this
/// module needs.
///
/// Only `lifecycle: active` counts as live — `lane-complete` and `consolidated`
/// records are read (so a parse failure elsewhere in the same repo is still
/// reported) but never contribute a [`LiveRun`].
pub fn discover_live_runs(root: &Path, repos: &[RepoEntry]) -> (Vec<LiveRun>, Vec<Diagnostic>) {
    let mut live = Vec::new();
    let mut diagnostics = Vec::new();

    for repo in repos {
        let run_root = root
            .join(&repo.repo_path)
            .join("planning")
            .join("orchestration-run");
        for run_dir in child_dirs_sorted(&run_root) {
            let notes = run_dir.join("notes.md");
            if !notes.is_file() {
                continue;
            }
            match read_run_record(&notes) {
                Ok(fm) => {
                    if fm.lifecycle.as_deref() != Some("active") {
                        continue;
                    }
                    match fm.roadmap {
                        Some(roadmap) => live.push(LiveRun {
                            repo: repo.slug.clone(),
                            roadmap,
                        }),
                        None => diagnostics.push(Diagnostic::warning(
                            notes.clone(),
                            "roadmap",
                            format!(
                                "{}: lifecycle is active but `roadmap` is missing — \
                                 not treated as a hold",
                                notes.display()
                            ),
                        )),
                    }
                }
                Err(message) => {
                    diagnostics.push(Diagnostic::warning(notes, "frontmatter", message))
                }
            }
        }
    }

    (live, diagnostics)
}

/// Resolve the environmental `HeldRepoBusy` hold for one [`FrontierEntry`], or
/// `None` if no live run holds its repo against a different roadmap.
///
/// A repo running **this same roadmap's** lane is not busy against itself — that is
/// the segment's own lane, not a competitor for the repo.
fn repo_busy_status(
    entry: &FrontierEntry,
    live_runs: &[LiveRun],
) -> Option<(SegmentAvailability, String)> {
    live_runs
        .iter()
        .find(|run| run.repo == entry.repo && run.roadmap != entry.roadmap)
        .map(|run| {
            (
                SegmentAvailability::HeldRepoBusy,
                format!("repo {} is live on {}", entry.repo, run.roadmap),
            )
        })
}

/// Resolve one [`FrontierEntry`]'s full availability: the intrinsic tier from
/// [`intrinsic_status`] (Task 1), falling through to the `HeldRepoBusy` environmental
/// check (Task 2) only when the intrinsic tier reports `Startable` — per
/// [`SegmentAvailability`]'s fixed precedence, any intrinsic hold outranks any
/// environmental one and is returned as-is without ever consulting `live_runs`.
fn resolve_status(
    entry: &FrontierEntry,
    live_runs: &[LiveRun],
) -> (SegmentAvailability, Option<String>) {
    let (availability, reason) = intrinsic_status(entry);
    if availability != SegmentAvailability::Startable {
        return (availability, reason);
    }
    match repo_busy_status(entry, live_runs) {
        Some((availability, reason)) => (availability, Some(reason)),
        None => (availability, reason),
    }
}

/// Compute full [`SegmentStatus`]es (intrinsic tier + `HeldRepoBusy`) for every entry
/// in `frontier`, plus a `Done` status for every entry in `segments` the frontier has
/// no record of — see [`intrinsic_segment_statuses`] for the `Done`-discovery
/// contract `segments` must satisfy. Supersedes [`intrinsic_segment_statuses`] as the
/// availability entry point once a repo-liveness reading (from
/// [`discover_live_runs`]) is in hand; `intrinsic_segment_statuses` remains for
/// callers (and tests) that only need the intrinsic tier.
pub fn segment_statuses(
    frontier: &Frontier,
    live_runs: &[LiveRun],
    segments: &[DiscoveredSegment],
) -> Vec<SegmentStatus> {
    let mut statuses: Vec<SegmentStatus> = frontier
        .entries
        .iter()
        .map(|entry| {
            let (availability, reason) = resolve_status(entry, live_runs);
            SegmentStatus {
                roadmap: entry.roadmap.clone(),
                lane: entry.lane.clone(),
                segment: entry.segment,
                repo: entry.repo.clone(),
                head: Some(entry.key.clone()),
                availability,
                reason,
            }
        })
        .collect();
    append_done_segments(&mut statuses, frontier, segments);
    statuses
}

// ---------------------------------------------------------------------------
// held-slot — MV.13.C Task 3
// ---------------------------------------------------------------------------

/// Name of the fleet lock directory, relative to the brain root. Mirrors
/// `LOCK_SUBDIR` in `base-template/scripts/fleet_concurrency_check.py`.
///
/// `pub` (not `pub(crate)`) so `src/main.rs` — a separate crate from this library —
/// can resolve `--lock-dir`'s fallback precedence with the identical constant
/// rather than a re-derived literal (`MV.ticket.write-verbs-ignore-the-quiesce-lease`
/// Task 2 requires reuse, not re-derivation).
pub const FLEET_LOCK_SUBDIR: &str = ".fleet-locks";

/// Mirrors `DEFAULT_TTL_SECONDS` in `fleet_concurrency_check.py` — an entry older
/// than this, regardless of pid liveness, is stale.
const DEFAULT_TTL_SECONDS: f64 = 4.0 * 60.0 * 60.0;

/// Mirrors `BROWSER_AUTOMATION_SIGNALS` in `fleet_concurrency_check.py`.
const BROWSER_AUTOMATION_SIGNALS: &[&str] = &[
    "playwright",
    "cypress",
    "puppeteer",
    "next build",
    "vite build",
    "npm run build",
    "yarn build",
    "pnpm build",
];

/// Mirrors `NATIVE_BUILD_SIGNALS` in `fleet_concurrency_check.py`.
const NATIVE_BUILD_SIGNALS: &[&str] = &["cargo build --release"];

/// Mirrors `MAX_LANES_BY_CATEGORY` in `fleet_concurrency_check.py` — capacity is
/// per category, not fleet-wide. `pub` (`BA.19.D` Task 1) so out-of-crate consumers
/// (bastion's `/concurrency` endpoint) can report the same caps this module enforces,
/// rather than re-stating them — mirrors `MAX_LANES_BY_CATEGORY`.
pub fn category_capacity(category: &str) -> usize {
    match category {
        "native-build" => 4,
        // "browser-automation" and any unknown category default to the
        // browser-automation cap, matching the Python script's own default.
        _ => 2,
    }
}

/// One raw entry read from a `.fleet-locks/*.json` file. Deliberately permissive
/// (`pid` as a bare [`serde_json::Value`]) so "pid is absent or not an integer" —
/// one of the documented staleness conditions — is representable rather than a
/// parse failure.
#[derive(Debug, Deserialize)]
struct FleetLockRaw {
    repo: String,
    #[serde(default)]
    pid: Option<serde_json::Value>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    started_at: Option<f64>,
}

// `pub(crate)`: reused by `crate::brain::lease` (`MV.ticket.write-verbs-ignore-the-quiesce-lease`
// Task 1) so the quiesce-lease staleness check shares this exact time source rather than
// inventing a second one.
pub(crate) fn now_unix_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Apply the same staleness rules `fleet_concurrency_check.py`'s `_sweep_stale`
/// applies, without mutating the store — this is a read, never a sweep. An entry
/// is stale when its `pid` is absent/not an integer, the pid is not currently
/// running, or its `started_at` is more than `ttl_seconds` in the past.
fn is_stale(entry: &FleetLockRaw, now: f64, ttl_seconds: f64) -> bool {
    let pid = match entry
        .pid
        .as_ref()
        .and_then(|v| v.as_i64())
        .filter(|p| *p > 0)
    {
        Some(pid) => pid as u32,
        None => return true,
    };
    if !pid_is_alive(pid) {
        return true;
    }
    let started_at = entry.started_at.unwrap_or(0.0);
    (now - started_at) > ttl_seconds
}

/// Read every `*.json` entry directly under `lock_dir`, skipping (not erroring on)
/// any file that is not valid JSON or does not match [`FleetLockRaw`]'s shape —
/// mirroring the Python sweep's "unreadable/corrupt entry: treat as stale" rule,
/// since a skipped entry contributes nothing to any category's live count either
/// way.
///
/// Returns `None` when `lock_dir` itself cannot be listed (missing or
/// unreadable) — the caller turns that into "unknown", never a hold.
fn read_fleet_lock_entries(lock_dir: &Path) -> Option<Vec<FleetLockRaw>> {
    let read_dir = std::fs::read_dir(lock_dir).ok()?;
    let mut entries = Vec::new();
    for dir_entry in read_dir.flatten() {
        let path = dir_entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(raw) = serde_json::from_str::<FleetLockRaw>(&contents) {
            entries.push(raw);
        }
    }
    Some(entries)
}

/// A snapshot of which repos currently hold a live (non-stale) fleet-lock slot,
/// per category, plus whether the read degraded (the lock directory could not be
/// listed at all).
#[derive(Debug, Default)]
pub struct FleetSlotView {
    /// `true` when `.fleet-locks` was missing or unreadable — "unknown", which
    /// resolves to *not held*, never to a hold. Mirrors the script's
    /// `{allowed: true, degraded: true}` degrade-to-advisory behavior.
    pub degraded: bool,
    live_by_category: HashMap<String, HashSet<String>>,
}

impl FleetSlotView {
    /// The category names this view knows about — the union of the categories
    /// [`category_capacity`] has an explicit cap for (`native-build`,
    /// `browser-automation`) and any category present in the live-hold data, sorted
    /// for a deterministic wire order. A category with zero live holds still
    /// reports (with an empty `live_repos`), so a caller building a full per-category
    /// table never has to special-case "no one is holding this category right now".
    pub fn known_categories(&self) -> Vec<String> {
        let mut set: HashSet<String> = KNOWN_CATEGORIES.iter().map(|s| s.to_string()).collect();
        set.extend(self.live_by_category.keys().cloned());
        let mut categories: Vec<String> = set.into_iter().collect();
        categories.sort();
        categories
    }

    /// The repos currently holding a live (non-stale) slot in `category`, sorted —
    /// `HashSet` iteration order must never reach the wire. Empty (never `None`) for
    /// a category with no live holds, including one this view has never heard of.
    pub fn live_repos(&self, category: &str) -> Vec<String> {
        let mut repos: Vec<String> = self
            .live_by_category
            .get(category)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        repos.sort();
        repos
    }
}

/// The categories [`category_capacity`] has an explicit cap for — used by
/// [`FleetSlotView::known_categories`] so a zero-hold category still reports.
const KNOWN_CATEGORIES: &[&str] = &["native-build", "browser-automation"];

/// Read `<root>/.fleet-locks` directly (never shells out to
/// `fleet_concurrency_check.py` — `timeout` does not exist on this shell, and a
/// subprocess turns a read into a failure mode) and resolve it into a
/// [`FleetSlotView`].
pub fn compute_fleet_slot_view(root: &Path) -> FleetSlotView {
    let lock_dir = root.join(FLEET_LOCK_SUBDIR);
    let Some(entries) = read_fleet_lock_entries(&lock_dir) else {
        return FleetSlotView {
            degraded: true,
            live_by_category: HashMap::new(),
        };
    };

    let now = now_unix_seconds();
    let mut live_by_category: HashMap<String, HashSet<String>> = HashMap::new();
    for entry in entries {
        if is_stale(&entry, now, DEFAULT_TTL_SECONDS) {
            continue;
        }
        let category = entry
            .category
            .unwrap_or_else(|| "browser-automation".to_string());
        live_by_category
            .entry(category)
            .or_default()
            .insert(entry.repo);
    }

    FleetSlotView {
        degraded: false,
        live_by_category,
    }
}

/// The heavy-lane category for `repo_root`'s `planning/harness.json`, or `None`
/// if the repo is light. Mirrors `heavy_category()` in
/// `fleet_concurrency_check.py` exactly: `uiTest.enabled` or a browser-automation
/// command signal classifies first (checked first because it is the more
/// resource-dangerous category), then a native-build command signal, else light.
/// A missing or unparsable `harness.json` is light, not an error — most repos in
/// the fleet have no harness.json at all.
pub fn heavy_category(repo_root: &Path) -> Option<String> {
    let harness_path = repo_root.join("planning").join("harness.json");
    let contents = std::fs::read_to_string(&harness_path).ok()?;
    let data: serde_json::Value = serde_json::from_str(&contents).ok()?;

    if data
        .get("uiTest")
        .and_then(|v| v.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Some("browser-automation".to_string());
    }

    let commands: Vec<String> = data
        .get("validation")
        .and_then(|v| v.get("checks"))
        .and_then(|v| v.as_array())
        .map(|checks| {
            checks
                .iter()
                .map(|check| {
                    check
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase()
                })
                .collect()
        })
        .unwrap_or_default();

    if commands
        .iter()
        .any(|cmd| BROWSER_AUTOMATION_SIGNALS.iter().any(|s| cmd.contains(s)))
    {
        return Some("browser-automation".to_string());
    }
    if commands
        .iter()
        .any(|cmd| NATIVE_BUILD_SIGNALS.iter().any(|s| cmd.contains(s)))
    {
        return Some("native-build".to_string());
    }
    None
}

/// Resolve the environmental `HeldSlot` hold for one [`FrontierEntry`], or `None`
/// if no hold applies — the repo is light, the view is degraded, the repo already
/// holds its own live entry, or the category is below capacity.
fn slot_status(
    entry: &FrontierEntry,
    repos: &[RepoEntry],
    root: &Path,
    slot_view: &FleetSlotView,
) -> Option<(SegmentAvailability, String)> {
    if slot_view.degraded {
        return None;
    }
    let repo_entry = repos.iter().find(|r| r.slug == entry.repo)?;
    let repo_root = root.join(&repo_entry.repo_path);
    let category = heavy_category(&repo_root)?;

    let live = slot_view.live_by_category.get(&category);
    if live.is_some_and(|repos| repos.contains(&entry.repo)) {
        // The repo already holds its own live entry — not held against itself,
        // mirroring the script's idempotent re-registration.
        return None;
    }

    let count = live.map(HashSet::len).unwrap_or(0);
    let cap = category_capacity(&category);
    if count >= cap {
        Some((
            SegmentAvailability::HeldSlot,
            format!("category {category} at capacity ({count}/{cap} lanes active)"),
        ))
    } else {
        None
    }
}

/// Resolve one [`FrontierEntry`]'s full availability across all three tiers this
/// block adds: intrinsic (Task 1), `HeldRepoBusy` (Task 2), `HeldSlot` (Task 3).
/// Each tier is only consulted when every higher-precedence tier reported
/// `Startable`, per [`SegmentAvailability`]'s fixed order.
fn resolve_status_with_slot(
    entry: &FrontierEntry,
    live_runs: &[LiveRun],
    repos: &[RepoEntry],
    root: &Path,
    slot_view: &FleetSlotView,
) -> (SegmentAvailability, Option<String>) {
    let (availability, reason) = resolve_status(entry, live_runs);
    if availability != SegmentAvailability::Startable {
        return (availability, reason);
    }
    match slot_status(entry, repos, root, slot_view) {
        Some((availability, reason)) => (availability, Some(reason)),
        None => (availability, reason),
    }
}

/// Compute full [`SegmentStatus`]es (intrinsic tier + `HeldRepoBusy` + `HeldSlot`)
/// for every entry in `frontier`, plus a `Done` status for every entry in `segments`
/// the frontier has no record of (see [`intrinsic_segment_statuses`] for the
/// `Done`-discovery contract `segments` must satisfy), plus whether the fleet-lock
/// read degraded (the `.fleet-locks` directory was missing or unreadable —
/// "unknown", never a hold). Supersedes [`segment_statuses`] as the availability
/// entry point once a repo registry and brain root are in hand, for the same reason
/// `segment_statuses` supersedes `intrinsic_segment_statuses`.
pub fn segment_statuses_with_slots(
    frontier: &Frontier,
    live_runs: &[LiveRun],
    repos: &[RepoEntry],
    root: &Path,
    segments: &[DiscoveredSegment],
) -> (Vec<SegmentStatus>, bool) {
    let slot_view = compute_fleet_slot_view(root);
    let degraded = slot_view.degraded;
    let mut statuses: Vec<SegmentStatus> = frontier
        .entries
        .iter()
        .map(|entry| {
            let (availability, reason) =
                resolve_status_with_slot(entry, live_runs, repos, root, &slot_view);
            SegmentStatus {
                roadmap: entry.roadmap.clone(),
                lane: entry.lane.clone(),
                segment: entry.segment,
                repo: entry.repo.clone(),
                head: Some(entry.key.clone()),
                availability,
                reason,
            }
        })
        .collect();
    append_done_segments(&mut statuses, frontier, segments);
    (statuses, degraded)
}

// ---------------------------------------------------------------------------
// Lane-level unblock leverage — MV.13.C Task 4
// ---------------------------------------------------------------------------

/// Identity of one lane segment: `(roadmap, lane, segment)`. Used as the map key
/// for [`lane_leverage`]'s output, and to test self-lane exclusion by comparing
/// `(roadmap, lane)` against a candidate downstream head's own identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize)]
pub struct SegmentKey {
    pub roadmap: String,
    pub lane: String,
    pub segment: usize,
}

/// One segment's lane-level unblock leverage: how many *lanes* (not blocks) are
/// freed by closing this segment.
///
/// Deliberately distinct from [`crate::brain::block_graph::BlockGraphNode::dependent_count`],
/// which counts individual dependent *blocks* corpus-wide. This metric instead
/// counts distinct `(roadmap, lane)` pairs whose *current segment head* falls
/// inside the transitive closure of blocks that depend (directly or indirectly)
/// on any block in this segment — several blocks in the same lane collapse to one
/// lane, and a lane several hops downstream still counts once it is reached.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct LaneLeverage {
    pub lanes_freed: usize,
    /// `"<roadmap>/<lane>"` per freed lane, sorted for deterministic output.
    pub lanes: Vec<String>,
}

/// Group `lane_positions` by `(roadmap, lane, segment)` into the set of
/// `"repo:id"` block keys each segment owns — the closure seed set for
/// [`lane_leverage`]. Mirrors [`crate::brain::frontier::compute_frontier`]'s own
/// `(roadmap, lane, segment)` grouping, but keeps every member (not just the
/// head) since any block in the segment can be the target of a downstream
/// `BlockedBy::Block` edge.
fn group_segment_blocks(
    lane_positions: &[DerivedBlockPosition],
) -> HashMap<SegmentKey, HashSet<String>> {
    let mut map: HashMap<SegmentKey, HashSet<String>> = HashMap::new();
    for p in lane_positions {
        let key = SegmentKey {
            roadmap: p.roadmap.clone(),
            lane: p.lane.clone(),
            segment: p.segment,
        };
        map.entry(key)
            .or_default()
            .insert(format!("{}:{}", p.repo, p.id));
    }
    map
}

/// Corpus-wide fan-in index: `to_ref -> {from, from, ...}`, over
/// `StateEdgeKind::BlockedBy` edges only (`CrossRepo` edges are not dependency
/// closure edges). The same edge-kind filter
/// [`crate::brain::block_graph::build_block_graph_export`]'s `dependent_count`
/// derivation uses, but keeping the actual dependent keys rather than only a
/// count, since [`lane_leverage`] needs to walk the closure, not just size it.
fn dependents_index(graph: &StateGraph) -> HashMap<&str, HashSet<&str>> {
    let mut map: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in &graph.edges {
        if edge.kind == StateEdgeKind::BlockedBy {
            map.entry(edge.to_ref.as_str())
                .or_default()
                .insert(edge.from.as_str());
        }
    }
    map
}

/// Transitive closure of every block reachable from `seed` by following
/// dependent edges outward (`seed`'s own members are never included in the
/// result) — a BFS/DFS over [`dependents_index`]'s adjacency, not a single hop,
/// so a three-deep dependency chain across three lanes reaches all three, not
/// just the first.
pub(crate) fn transitive_closure(
    seed: &HashSet<String>,
    dependents_of: &HashMap<&str, HashSet<&str>>,
) -> HashSet<String> {
    let mut visited: HashSet<String> = seed.clone();
    let mut queue: Vec<String> = seed.iter().cloned().collect();
    let mut closure: HashSet<String> = HashSet::new();

    while let Some(key) = queue.pop() {
        if let Some(deps) = dependents_of.get(key.as_str()) {
            for &dep in deps {
                let dep_owned = dep.to_string();
                if visited.insert(dep_owned.clone()) {
                    closure.insert(dep_owned.clone());
                    queue.push(dep_owned);
                }
            }
        }
    }

    closure
}

/// Compute lane-level unblock leverage for every segment discovered in
/// `lane_positions`: for each segment `S`, the transitive closure (via
/// `BlockedBy::Block` edges in `graph`) of every block that depends, directly or
/// indirectly, on any block in `S`, then the distinct `(roadmap, lane)` pairs
/// whose *current* segment head (from `frontier.entries`) falls inside that
/// closure.
///
/// A segment's own lane is always excluded from its own `lanes_freed` — a later
/// segment in the same lane depending on an earlier one is not a *different*
/// lane being freed.
///
/// `graph` MUST be built over the untruncated in-process corpus
/// (`build_state_graph`, never a `max_nodes`-truncated `BlockGraphExport`) — see
/// [`lane_leverage_over_untruncated_graph`], which layers
/// [`crate::brain::frontier::ensure_untruncated`]'s refusal on top of this pure
/// closure computation.
pub fn lane_leverage(
    graph: &StateGraph,
    lane_positions: &[DerivedBlockPosition],
    frontier: &Frontier,
) -> HashMap<SegmentKey, LaneLeverage> {
    let segment_blocks = group_segment_blocks(lane_positions);
    let dependents_of = dependents_index(graph);

    let mut result = HashMap::with_capacity(segment_blocks.len());
    for (seg_key, blocks) in &segment_blocks {
        let closure = transitive_closure(blocks, &dependents_of);

        let mut lanes: HashSet<(String, String)> = HashSet::new();
        for entry in &frontier.entries {
            if entry.roadmap == seg_key.roadmap && entry.lane == seg_key.lane {
                continue; // self-lane exclusion — never counts toward its own leverage
            }
            if closure.contains(&entry.key) {
                lanes.insert((entry.roadmap.clone(), entry.lane.clone()));
            }
        }

        let mut lane_names: Vec<String> = lanes
            .into_iter()
            .map(|(roadmap, lane)| format!("{roadmap}/{lane}"))
            .collect();
        lane_names.sort();

        result.insert(
            seg_key.clone(),
            LaneLeverage {
                lanes_freed: lane_names.len(),
                lanes: lane_names,
            },
        );
    }

    result
}

/// Plan-time wrapper around [`lane_leverage`] that builds the untruncated
/// (`max_nodes: usize::MAX`) block-graph export purely to run it through
/// [`crate::brain::frontier::ensure_untruncated`] — the same refusal `MV.13.B`
/// established for [`crate::brain::frontier::plan_frontier`], reused here rather
/// than re-implemented: this closure MUST run over the full in-process graph,
/// never a partial one, for the same reason a truncated frontier silently drops
/// gates.
pub fn lane_leverage_over_untruncated_graph(
    root: &Path,
    loaded: &[(StateSource, StateFile)],
    graph: &StateGraph,
    lane_positions: &[DerivedBlockPosition],
    frontier: &Frontier,
) -> Result<HashMap<SegmentKey, LaneLeverage>, Diagnostic> {
    use crate::brain::block_graph::{BlockGraphScope, build_block_graph_export};
    use crate::brain::config::load_brain_config;
    use crate::brain::frontier::ensure_untruncated;
    use crate::brain::state::TierScope;

    let config = load_brain_config(&root.join("brain.toml")).unwrap_or_default();
    let scope = BlockGraphScope {
        tier: TierScope::All,
        epic: None,
        repo: None,
        include_closed: true,
        include_boundary: false,
        max_nodes: usize::MAX,
    };
    let export = build_block_graph_export(root, &config, graph, loaded, &scope);
    ensure_untruncated(&export)?;

    Ok(lane_leverage(graph, lane_positions, frontier))
}

// ---------------------------------------------------------------------------
// Artifact + emit-state wiring + `mev lanes` CLI — MV.13.C Task 5
// ---------------------------------------------------------------------------

/// Relative path (from the brain root) of the derived lane-availability artifact
/// [`plan_availability`] writes. Like [`crate::brain::frontier::LANE_FRONTIER_ARTIFACT`]
/// and [`crate::brain::lane_segments::LANE_SEGMENTS_ARTIFACT`], this is a cross-repo,
/// corpus-wide derivation, not one repo's scoped surface — it is written
/// unconditionally, never narrowed by `emit_state`'s `--scope <repo>`.
pub const LANE_AVAILABILITY_ARTIFACT: &str = "planning/lane-availability.json";

/// One segment's [`SegmentStatus`] plus its [`LaneLeverage`], flattened together —
/// the per-segment row of [`LaneAvailabilityArtifact`]. `#[serde(flatten)]` on
/// `status` means the JSON shape carries every `SegmentStatus` field alongside
/// `leverage` at the same level, not nested under a `status` key.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LaneAvailabilityEntry {
    #[serde(flatten)]
    pub status: SegmentStatus,
    pub leverage: LaneLeverage,
}

/// The full availability derivation, serialized as-is — `mev emit-state`'s JSON
/// artifact at [`LANE_AVAILABILITY_ARTIFACT`], plus `derived_at`. `pub` so `mev lanes
/// --json` can emit this exact shape to stdout without a second definition.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LaneAvailabilityArtifact {
    /// RFC 3339 timestamp of the derivation run (`chrono::Local::now()`), same
    /// rationale as [`crate::brain::frontier::FrontierArtifact::derived_at`].
    pub derived_at: String,
    /// `true` when the fleet-lock read that feeds `HeldSlot` degraded (`.fleet-locks`
    /// missing or unreadable) — "unknown", never a hold, mirrors
    /// [`FleetSlotView::degraded`]. A consumer can use this to tell a corpus with
    /// zero live `HeldSlot` holds apart from one that could not check.
    pub degraded: bool,
    pub segments: Vec<LaneAvailabilityEntry>,
}

/// Plan the [`LANE_AVAILABILITY_ARTIFACT`] write: derive lane positions and the
/// untruncated in-process block graph the same way [`crate::brain::frontier::plan_frontier`]
/// does, refuse via [`ensure_untruncated`] if the export somehow reports
/// `truncated: true`, then compute the full [`segment_statuses_with_slots`] tier
/// (intrinsic + `HeldRepoBusy` + `HeldSlot`) and [`lane_leverage`] over the result,
/// joining the two by `(roadmap, lane, segment)` into one [`LaneAvailabilityEntry`]
/// per segment — modelled directly on `plan_frontier`, one [`EmitPlan`] for
/// `emit_state` to apply alongside its other planners.
///
/// No `EmitAction` is planned when zero lane files are discovered (an empty corpus,
/// or `root` has no `planning/` at all), nor when [`ensure_untruncated`] refuses the
/// export — a diagnostic is carried on the plan in the latter case, but never a
/// partial write. A segment absent from the resulting `leverage` map (should not
/// happen — every frontier entry has a corresponding lane-position group) falls back
/// to [`LaneLeverage::default`] (`lanes_freed: 0`) rather than panicking.
pub fn plan_availability(root: &Path, loaded: &[(StateSource, StateFile)]) -> EmitPlan {
    let mut plan = EmitPlan::default();

    let (lane_files, discover_diags) = discover_lane_files(root);
    plan.diagnostics.extend(discover_diags);

    if lane_files.is_empty() {
        return plan;
    }

    let owner_index = build_owner_index(loaded);
    for lf in &lane_files {
        plan.diagnostics
            .extend(unresolved_owner_diagnostics(lf, &owner_index));
    }

    let (lane_positions, derive_diags) = derive_lane_positions(&lane_files, loaded);
    plan.diagnostics.extend(derive_diags);

    // `config` feeds both the block-graph export's TIER stage (a no-op here,
    // `TierScope::All`, no `--repo`/`--epic` filter) and the `repos[]` list this
    // module reads for `HeldRepoBusy`/`HeldSlot` — a missing/unreadable
    // `brain.toml` falls back to `BrainConfig::default()` (empty `repos[]`, so no
    // environmental holds can be resolved) rather than aborting the plan;
    // `emit_state`'s own top-level `find_brain_config` call already gates the whole
    // run on a real config existing.
    let config = load_brain_config(&root.join("brain.toml")).unwrap_or_default();

    let graph = build_state_graph(loaded);
    let scope = BlockGraphScope {
        tier: TierScope::All,
        epic: None,
        repo: None,
        include_closed: true,
        include_boundary: false,
        max_nodes: usize::MAX,
    };
    let export = build_block_graph_export(root, &config, &graph, loaded, &scope);
    if let Err(diag) = ensure_untruncated(&export) {
        plan.diagnostics.push(diag);
        return plan;
    }

    let effective = effective_priorities(&graph, loaded);
    let frontier = compute_frontier(&lane_positions, &graph, loaded, &effective, None);

    let (live_runs, live_run_diags) = discover_live_runs(root, &config.repos);
    plan.diagnostics.extend(live_run_diags);

    let all_segments = discover_segments(&lane_positions);
    let (statuses, degraded) =
        segment_statuses_with_slots(&frontier, &live_runs, &config.repos, root, &all_segments);
    let leverage_by_segment = lane_leverage(&graph, &lane_positions, &frontier);

    let segments: Vec<LaneAvailabilityEntry> = statuses
        .into_iter()
        .map(|status| {
            let key = SegmentKey {
                roadmap: status.roadmap.clone(),
                lane: status.lane.clone(),
                segment: status.segment,
            };
            let leverage = leverage_by_segment.get(&key).cloned().unwrap_or_default();
            LaneAvailabilityEntry { status, leverage }
        })
        .collect();

    let segment_count = segments.len();
    let artifact = LaneAvailabilityArtifact {
        derived_at: chrono::Local::now().to_rfc3339(),
        degraded,
        segments,
    };

    let new_content = match serde_json::to_string_pretty(&artifact) {
        Ok(mut s) => {
            s.push('\n');
            s
        }
        Err(e) => {
            plan.diagnostics.push(Diagnostic::error(
                root,
                "E_EMIT_AVAILABILITY_SERIALIZE",
                format!("failed to serialize lane-availability artifact: {e}"),
            ));
            return plan;
        }
    };

    plan.actions.push(EmitAction {
        path: root.join(LANE_AVAILABILITY_ARTIFACT),
        new_content,
        note: format!("derived lane availability: {segment_count} segment(s), degraded={degraded}"),
    });

    plan
}

/// Render one line per segment: `{roadmap}/{lane}#{segment} {repo}:{head} —
/// {availability} ({reason}) frees N lane(s)`. `head`'s leading `"{repo}:"` prefix
/// (see [`SegmentStatus::head`]'s `"repo:id"` shape) is stripped before display, so
/// the repo is not printed twice; `head` renders as `-` for `Done` segments (no live
/// head). The `(reason)` clause is omitted entirely for `Startable`/`Done`, which
/// carry `reason: None`. `frees N lane(s)` is always present, including `frees 0
/// lanes` — a reader scanning for the highest-leverage segment needs the zero case
/// visible, not silently absent.
pub fn render_availability_text(artifact: &LaneAvailabilityArtifact) -> String {
    artifact
        .segments
        .iter()
        .map(|entry| {
            let s = &entry.status;
            let head = match &s.head {
                Some(h) => h
                    .strip_prefix(&format!("{}:", s.repo))
                    .unwrap_or(h.as_str()),
                None => "-",
            };
            let avail = serde_json::to_string(&s.availability)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            let reason = match &s.reason {
                Some(r) => format!(" ({r})"),
                None => String::new(),
            };
            let lanes = entry.leverage.lanes_freed;
            let noun = if lanes == 1 { "lane" } else { "lanes" };
            format!(
                "{}/{}#{} {}:{} — {avail}{reason} frees {lanes} {noun}",
                s.roadmap, s.lane, s.segment, s.repo, head,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(unmet_blocks: Vec<&str>, unmet_gates: Vec<&str>) -> FrontierEntry {
        let startable = unmet_blocks.is_empty() && unmet_gates.is_empty();
        FrontierEntry {
            roadmap: "engine-orchestration".to_string(),
            lane: "derive".to_string(),
            segment: 0,
            repo: "mev".to_string(),
            key: "mev:MV.13.C".to_string(),
            id: "MV.13.C".to_string(),
            title: "Segment availability".to_string(),
            status: "open".to_string(),
            unmet_blocks: unmet_blocks.into_iter().map(String::from).collect(),
            unmet_gates: unmet_gates.into_iter().map(String::from).collect(),
            startable,
        }
    }

    #[test]
    fn held_block_when_unmet_blocks_nonempty() {
        let e = entry(vec!["mev:MV.13.B"], vec![]);
        let (avail, reason) = intrinsic_status(&e);
        assert_eq!(avail, SegmentAvailability::HeldBlock);
        assert_eq!(reason.as_deref(), Some("blocked by mev:MV.13.B"));
    }

    #[test]
    fn held_operator_when_only_unmet_gates_nonempty() {
        let e = entry(vec![], vec!["operator:review-session"]);
        let (avail, reason) = intrinsic_status(&e);
        assert_eq!(avail, SegmentAvailability::HeldOperator);
        assert_eq!(
            reason.as_deref(),
            Some("blocked by operator:review-session")
        );
    }

    #[test]
    fn startable_when_no_unmet_anything() {
        let e = entry(vec![], vec![]);
        let (avail, reason) = intrinsic_status(&e);
        assert_eq!(avail, SegmentAvailability::Startable);
        assert_eq!(reason, None);
    }

    /// `MV.16.C` task 4: availability recomputes nothing about carryover gating
    /// itself — it only ever reads `FrontierEntry::unmet_gates`/`startable`, so a
    /// carryover-held head (`"carryover:{owner}"`, the shape
    /// [`crate::brain::frontier::compute_frontier`] now emits) must flow through
    /// as `HeldOperator` with the owner named in the reason, with no availability
    /// code change required — proving this module consumes the derivation rather
    /// than recomputing held-ness from `depends_on`.
    #[test]
    fn carryover_gate_reason_flows_through_as_held_operator() {
        let e = entry(vec![], vec!["carryover:mev:finding-1"]);
        let (avail, reason) = intrinsic_status(&e);
        assert_eq!(avail, SegmentAvailability::HeldOperator);
        assert_eq!(
            reason.as_deref(),
            Some("blocked by carryover:mev:finding-1")
        );
    }

    #[test]
    fn held_block_outranks_held_operator_when_both_present() {
        // A head matching both unmet_blocks and unmet_gates reports HeldBlock per the
        // documented precedence, not HeldOperator.
        let e = entry(vec!["mev:MV.13.B"], vec!["operator:review-session"]);
        let (avail, reason) = intrinsic_status(&e);
        assert_eq!(avail, SegmentAvailability::HeldBlock);
        assert_eq!(reason.as_deref(), Some("blocked by mev:MV.13.B"));
    }

    fn discovered(roadmap: &str, lane: &str, segment: usize, repo: &str) -> DiscoveredSegment {
        DiscoveredSegment {
            roadmap: roadmap.to_string(),
            lane: lane.to_string(),
            segment,
            repo: repo.to_string(),
        }
    }

    #[test]
    fn all_closed_segment_reports_done_with_no_head() {
        // A segment with every block closed contributes no FrontierEntry at all
        // (compute_frontier skips it) — Done is represented by absence from the
        // frontier, and intrinsic_segment_statuses is what turns that absence back
        // into a real SegmentStatus, driven by real (discovered) segment data rather
        // than a hand-built one.
        let frontier = Frontier::default();
        let segments = vec![discovered("engine-orchestration", "derive", 0, "mev")];

        let statuses = intrinsic_segment_statuses(&frontier, &segments);

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].roadmap, "engine-orchestration");
        assert_eq!(statuses[0].lane, "derive");
        assert_eq!(statuses[0].segment, 0);
        assert_eq!(statuses[0].repo, "mev");
        assert_eq!(statuses[0].availability, SegmentAvailability::Done);
        assert_eq!(statuses[0].head, None);
        assert_eq!(statuses[0].reason, None);
    }

    #[test]
    fn done_segment_appears_once_alongside_live_sibling_unaffected() {
        // One live segment (segment 0, has a frontier entry) and one all-closed
        // sibling (segment 1, no frontier entry). The closed one must appear exactly
        // once as Done with head: None; the live one must keep exactly the status it
        // had before Done discovery existed.
        let live = entry(vec![], vec![]);
        let frontier = Frontier {
            entries: vec![live],
            gate_ranks: Vec::new(),
        };
        let segments = vec![
            discovered("engine-orchestration", "derive", 0, "mev"),
            discovered("engine-orchestration", "derive", 1, "mev"),
        ];

        let statuses = intrinsic_segment_statuses(&frontier, &segments);

        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].segment, 0);
        assert_eq!(statuses[0].availability, SegmentAvailability::Startable);
        assert_eq!(statuses[0].head, Some("mev:MV.13.C".to_string()));
        assert_eq!(statuses[1].segment, 1);
        assert_eq!(statuses[1].availability, SegmentAvailability::Done);
        assert_eq!(statuses[1].head, None);
        assert_eq!(statuses[1].reason, None);
    }

    #[test]
    fn no_closed_segments_produces_output_identical_to_frontier_only() {
        // Every discovered segment has a live frontier entry — output must match
        // what intrinsic_segment_statuses produced before Done discovery existed,
        // byte for byte (no spurious Done entries appended).
        let live = entry(vec![], vec![]);
        let frontier = Frontier {
            entries: vec![live],
            gate_ranks: Vec::new(),
        };
        let segments = vec![discovered("engine-orchestration", "derive", 0, "mev")];

        let statuses = intrinsic_segment_statuses(&frontier, &segments);

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].availability, SegmentAvailability::Startable);
        assert_eq!(statuses[0].head, Some("mev:MV.13.C".to_string()));
    }

    #[test]
    fn segment_availability_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&SegmentAvailability::HeldRepoBusy).unwrap(),
            "\"held-repo-busy\""
        );
        assert_eq!(
            serde_json::to_string(&SegmentAvailability::HeldBlock).unwrap(),
            "\"held-block\""
        );
        assert_eq!(
            serde_json::to_string(&SegmentAvailability::HeldOperator).unwrap(),
            "\"held-operator\""
        );
        assert_eq!(
            serde_json::to_string(&SegmentAvailability::HeldSlot).unwrap(),
            "\"held-slot\""
        );
        assert_eq!(
            serde_json::to_string(&SegmentAvailability::Startable).unwrap(),
            "\"startable\""
        );
        assert_eq!(
            serde_json::to_string(&SegmentAvailability::Done).unwrap(),
            "\"done\""
        );
    }

    // -----------------------------------------------------------------
    // held-repo-busy — MV.13.C Task 2
    // -----------------------------------------------------------------

    fn repo_entry(slug: &str) -> RepoEntry {
        RepoEntry {
            slug: slug.to_string(),
            tier: "core".to_string(),
            repo_path: slug.to_string(),
            status_file: String::new(),
            cache_doc: String::new(),
            heading: String::new(),
            prefix: None,
        }
    }

    fn write_run_record(root: &Path, repo: &str, roadmap: &str, lifecycle: &str) {
        let dir = root
            .join(repo)
            .join("planning")
            .join("orchestration-run")
            .join(roadmap);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("notes.md"),
            format!(
                "---\ntype: Reference\ntitle: run\ndescription: run\nroadmap: {roadmap}\nlane: derive\nlifecycle: {lifecycle}\n---\n\n# run\n"
            ),
        )
        .unwrap();
    }

    fn write_malformed_run_record(root: &Path, repo: &str, roadmap: &str) {
        let dir = root
            .join(repo)
            .join("planning")
            .join("orchestration-run")
            .join(roadmap);
        std::fs::create_dir_all(&dir).unwrap();
        // No frontmatter fence at all — extract_frontmatter returns None.
        std::fs::write(dir.join("notes.md"), "# not a frontmatter file\n").unwrap();
    }

    #[test]
    fn discover_live_runs_finds_active_record() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-live-runs-active");
        write_run_record(&root, "bastion", "close-the-loop", "active");

        let (live, diags) = discover_live_runs(&root, &[repo_entry("bastion")]);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert_eq!(
            live,
            vec![LiveRun {
                repo: "bastion".to_string(),
                roadmap: "close-the-loop".to_string(),
            }]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discover_live_runs_skips_lane_complete_and_consolidated() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-live-runs-inactive");
        write_run_record(&root, "bastion", "roadmap-a", "lane-complete");
        write_run_record(&root, "bastion", "roadmap-b", "consolidated");

        let (live, diags) = discover_live_runs(&root, &[repo_entry("bastion")]);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
        assert!(
            live.is_empty(),
            "lane-complete/consolidated records must never count as live, got {live:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discover_live_runs_malformed_frontmatter_yields_diagnostic_and_no_hold() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-live-runs-malformed");
        write_malformed_run_record(&root, "bastion", "roadmap-a");

        let (live, diags) = discover_live_runs(&root, &[repo_entry("bastion")]);
        assert!(
            live.is_empty(),
            "a parse failure must never invent a hold, got {live:?}"
        );
        assert_eq!(
            diags.len(),
            1,
            "expected exactly one diagnostic, got {diags:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discover_live_runs_missing_orchestration_run_dir_is_not_an_error() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-live-runs-missing-dir");
        std::fs::create_dir_all(&root).unwrap();

        let (live, diags) = discover_live_runs(&root, &[repo_entry("bastion")]);
        assert!(live.is_empty());
        assert!(diags.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn held_repo_busy_when_active_record_is_a_different_roadmap() {
        let e = entry(vec![], vec![]);
        let live_runs = vec![LiveRun {
            repo: "mev".to_string(),
            roadmap: "close-the-loop".to_string(),
        }];
        let (avail, reason) = resolve_status(&e, &live_runs);
        assert_eq!(avail, SegmentAvailability::HeldRepoBusy);
        assert_eq!(
            reason.as_deref(),
            Some("repo mev is live on close-the-loop")
        );
    }

    #[test]
    fn own_roadmap_live_record_does_not_hold_the_repo_against_itself() {
        // entry()'s FrontierEntry.roadmap is "engine-orchestration".
        let e = entry(vec![], vec![]);
        let live_runs = vec![LiveRun {
            repo: "mev".to_string(),
            roadmap: "engine-orchestration".to_string(),
        }];
        let (avail, reason) = resolve_status(&e, &live_runs);
        assert_eq!(avail, SegmentAvailability::Startable);
        assert_eq!(reason, None);
    }

    #[test]
    fn intrinsic_held_block_outranks_a_live_repo() {
        let e = entry(vec!["mev:MV.13.B"], vec![]);
        let live_runs = vec![LiveRun {
            repo: "mev".to_string(),
            roadmap: "close-the-loop".to_string(),
        }];
        let (avail, reason) = resolve_status(&e, &live_runs);
        assert_eq!(avail, SegmentAvailability::HeldBlock);
        assert_eq!(reason.as_deref(), Some("blocked by mev:MV.13.B"));
    }

    #[test]
    fn segment_statuses_layers_repo_busy_over_frontier_entries() {
        let e = entry(vec![], vec![]);
        let frontier = Frontier {
            entries: vec![e],
            gate_ranks: Vec::new(),
        };
        let live_runs = vec![LiveRun {
            repo: "mev".to_string(),
            roadmap: "close-the-loop".to_string(),
        }];
        let statuses = segment_statuses(&frontier, &live_runs, &[]);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].availability, SegmentAvailability::HeldRepoBusy);
    }

    // -----------------------------------------------------------------
    // held-slot — MV.13.C Task 3
    // -----------------------------------------------------------------

    fn write_harness(root: &Path, repo: &str, category: Option<&str>) {
        let dir = root.join(repo).join("planning");
        std::fs::create_dir_all(&dir).unwrap();
        let json = match category {
            Some("native-build") => serde_json::json!({
                "validation": {"checks": [{"command": "cargo build --release"}]}
            }),
            Some("browser-automation") => serde_json::json!({
                "uiTest": {"enabled": true}
            }),
            _ => serde_json::json!({}),
        };
        std::fs::write(
            dir.join("harness.json"),
            serde_json::to_string(&json).unwrap(),
        )
        .unwrap();
    }

    fn write_lock_entry(
        root: &Path,
        repo: &str,
        category: &str,
        pid: i64,
        started_at: f64,
        label: &str,
    ) {
        let dir = root.join(FLEET_LOCK_SUBDIR);
        std::fs::create_dir_all(&dir).unwrap();
        let json = serde_json::json!({
            "repo": repo,
            "pid": pid,
            "category": category,
            "started_at": started_at,
        });
        std::fs::write(
            dir.join(format!("{repo}__{label}.json")),
            serde_json::to_string(&json).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn held_slot_when_category_at_capacity() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-slot-at-capacity");
        write_harness(&root, "mev", Some("native-build"));
        let now = now_unix_seconds();
        for i in 0..4 {
            write_lock_entry(
                &root,
                &format!("other-{i}"),
                "native-build",
                std::process::id() as i64,
                now,
                "p",
            );
        }

        let e = entry(vec![], vec![]);
        let repos = vec![repo_entry("mev")];
        let slot_view = compute_fleet_slot_view(&root);
        assert!(!slot_view.degraded);
        let status = slot_status(&e, &repos, &root, &slot_view);
        assert!(
            matches!(status, Some((SegmentAvailability::HeldSlot, _))),
            "expected HeldSlot at capacity, got {status:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn not_held_when_category_below_capacity() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-slot-below-capacity");
        write_harness(&root, "mev", Some("native-build"));
        let now = now_unix_seconds();
        for i in 0..3 {
            write_lock_entry(
                &root,
                &format!("other-{i}"),
                "native-build",
                std::process::id() as i64,
                now,
                "p",
            );
        }

        let e = entry(vec![], vec![]);
        let repos = vec![repo_entry("mev")];
        let slot_view = compute_fleet_slot_view(&root);
        let status = slot_status(&e, &repos, &root, &slot_view);
        assert!(
            status.is_none(),
            "expected no hold below capacity, got {status:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_dead_pid_entry_does_not_count_toward_capacity() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-slot-dead-pid");
        write_harness(&root, "mev", Some("native-build"));
        let now = now_unix_seconds();
        // Three live entries plus one with a dead pid: capacity is 4, so a dead-pid
        // entry counting would wrongly hold; it must not.
        for i in 0..3 {
            write_lock_entry(
                &root,
                &format!("other-{i}"),
                "native-build",
                std::process::id() as i64,
                now,
                "p",
            );
        }
        write_lock_entry(&root, "other-dead", "native-build", 999_999_999, now, "p");

        let e = entry(vec![], vec![]);
        let repos = vec![repo_entry("mev")];
        let slot_view = compute_fleet_slot_view(&root);
        let status = slot_status(&e, &repos, &root, &slot_view);
        assert!(
            status.is_none(),
            "a stale dead-pid entry must not count toward capacity, got {status:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_over_ttl_entry_does_not_count_toward_capacity() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-slot-over-ttl");
        write_harness(&root, "mev", Some("native-build"));
        let now = now_unix_seconds();
        for i in 0..3 {
            write_lock_entry(
                &root,
                &format!("other-{i}"),
                "native-build",
                std::process::id() as i64,
                now,
                "p",
            );
        }
        // Alive pid, but started_at is well past DEFAULT_TTL_SECONDS (4h) ago.
        write_lock_entry(
            &root,
            "other-expired",
            "native-build",
            std::process::id() as i64,
            now - (5.0 * 60.0 * 60.0),
            "p",
        );

        let e = entry(vec![], vec![]);
        let repos = vec![repo_entry("mev")];
        let slot_view = compute_fleet_slot_view(&root);
        let status = slot_status(&e, &repos, &root, &slot_view);
        assert!(
            status.is_none(),
            "an over-TTL entry must not count toward capacity, got {status:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn light_repo_is_never_held_slot() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-slot-light-repo");
        // No harness.json at all -> light.
        let now = now_unix_seconds();
        for i in 0..4 {
            write_lock_entry(
                &root,
                &format!("other-{i}"),
                "native-build",
                std::process::id() as i64,
                now,
                "p",
            );
        }

        let e = entry(vec![], vec![]);
        let repos = vec![repo_entry("mev")];
        let slot_view = compute_fleet_slot_view(&root);
        let status = slot_status(&e, &repos, &root, &slot_view);
        assert!(
            status.is_none(),
            "a light repo must never be HeldSlot, got {status:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_lock_dir_degrades_and_holds_nothing() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-slot-missing-dir");
        std::fs::create_dir_all(&root).unwrap();
        write_harness(&root, "mev", Some("native-build"));
        // Deliberately no .fleet-locks directory created.

        let e = entry(vec![], vec![]);
        let repos = vec![repo_entry("mev")];
        let slot_view = compute_fleet_slot_view(&root);
        assert!(
            slot_view.degraded,
            "missing lock dir must set degraded: true"
        );
        let status = slot_status(&e, &repos, &root, &slot_view);
        assert!(
            status.is_none(),
            "a degraded read must never report a hold, got {status:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn repo_already_holding_own_entry_is_not_held_against_itself() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-slot-self-hold");
        write_harness(&root, "mev", Some("native-build"));
        let now = now_unix_seconds();
        // Capacity (4) reached, and "mev" itself is one of the four holders.
        write_lock_entry(
            &root,
            "mev",
            "native-build",
            std::process::id() as i64,
            now,
            "self",
        );
        for i in 0..3 {
            write_lock_entry(
                &root,
                &format!("other-{i}"),
                "native-build",
                std::process::id() as i64,
                now,
                "p",
            );
        }

        let e = entry(vec![], vec![]);
        let repos = vec![repo_entry("mev")];
        let slot_view = compute_fleet_slot_view(&root);
        let status = slot_status(&e, &repos, &root, &slot_view);
        assert!(
            status.is_none(),
            "a repo already holding its own slot must not be held against itself, got {status:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_status_with_slot_reaches_held_slot_only_when_startable() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-slot-resolve");
        write_harness(&root, "mev", Some("native-build"));
        let now = now_unix_seconds();
        for i in 0..4 {
            write_lock_entry(
                &root,
                &format!("other-{i}"),
                "native-build",
                std::process::id() as i64,
                now,
                "p",
            );
        }
        let repos = vec![repo_entry("mev")];
        let slot_view = compute_fleet_slot_view(&root);

        // A held-block segment must report HeldBlock, never HeldSlot, even at
        // fleet capacity.
        let held = entry(vec!["mev:MV.13.B"], vec![]);
        let (avail, _) = resolve_status_with_slot(&held, &[], &repos, &root, &slot_view);
        assert_eq!(avail, SegmentAvailability::HeldBlock);

        // A startable segment falls through to HeldSlot when at capacity.
        let startable = entry(vec![], vec![]);
        let (avail, reason) = resolve_status_with_slot(&startable, &[], &repos, &root, &slot_view);
        assert_eq!(avail, SegmentAvailability::HeldSlot);
        assert!(reason.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------
    // lane_leverage — MV.13.C Task 4
    // -----------------------------------------------------------------

    fn lane_pos(
        roadmap: &str,
        lane: &str,
        segment: usize,
        repo: &str,
        id: &str,
    ) -> DerivedBlockPosition {
        DerivedBlockPosition {
            roadmap: roadmap.to_string(),
            lane: lane.to_string(),
            repo: repo.to_string(),
            id: id.to_string(),
            line: 1,
            segment,
            position: 0,
            origin_roadmap: None,
            directives: None,
        }
    }

    fn state_edge(from: &str, to_ref: &str) -> crate::brain::state::StateEdge {
        crate::brain::state::StateEdge {
            from: from.to_string(),
            to_ref: to_ref.to_string(),
            kind: StateEdgeKind::BlockedBy,
            source_path: PathBuf::new(),
        }
    }

    fn frontier_entry(roadmap: &str, lane: &str, segment: usize, key: &str) -> FrontierEntry {
        FrontierEntry {
            roadmap: roadmap.to_string(),
            lane: lane.to_string(),
            segment,
            repo: key.split(':').next().unwrap().to_string(),
            key: key.to_string(),
            id: key.split(':').nth(1).unwrap().to_string(),
            title: String::new(),
            status: "open".to_string(),
            unmet_blocks: Vec::new(),
            unmet_gates: Vec::new(),
            startable: true,
        }
    }

    #[test]
    fn transitive_chain_across_three_lanes_scores_two_not_one() {
        // S = lane-a/0 = {repo:A1}. lane-b/0 head B1 depends directly on A1.
        // lane-c/0 head C1 depends on B1 (two hops from S, not a direct dependent).
        // A one-hop implementation would score 1 (only B1); the real answer is 2.
        let lane_positions = vec![
            lane_pos("r", "lane-a", 0, "repo", "A1"),
            lane_pos("r", "lane-b", 0, "repo", "B1"),
            lane_pos("r", "lane-c", 0, "repo", "C1"),
        ];
        let graph = StateGraph {
            nodes: Vec::new(),
            edges: vec![
                state_edge("repo:B1", "repo:A1"),
                state_edge("repo:C1", "repo:B1"),
            ],
        };
        let frontier = Frontier {
            entries: vec![
                frontier_entry("r", "lane-b", 0, "repo:B1"),
                frontier_entry("r", "lane-c", 0, "repo:C1"),
            ],
            gate_ranks: Vec::new(),
        };

        let leverage = lane_leverage(&graph, &lane_positions, &frontier);
        let s = leverage
            .get(&SegmentKey {
                roadmap: "r".to_string(),
                lane: "lane-a".to_string(),
                segment: 0,
            })
            .expect("segment must be present");
        assert_eq!(
            s.lanes_freed, 2,
            "expected both lane-b and lane-c freed, got {s:?}"
        );
        assert_eq!(
            s.lanes,
            vec!["r/lane-b".to_string(), "r/lane-c".to_string()]
        );
    }

    #[test]
    fn two_segments_in_the_same_lane_collapse_to_one_lane() {
        // S = lane-a/0 = {repo:A1}. lane-b has two segments, both reachable from A1:
        // lane-b/0 head B1 depends on A1; lane-b/1 head B2 depends on B1. Both are
        // lane-b -> lanes_freed must be 1, not 2.
        let lane_positions = vec![
            lane_pos("r", "lane-a", 0, "repo", "A1"),
            lane_pos("r", "lane-b", 0, "repo", "B1"),
            lane_pos("r", "lane-b", 1, "repo", "B2"),
        ];
        let graph = StateGraph {
            nodes: Vec::new(),
            edges: vec![
                state_edge("repo:B1", "repo:A1"),
                state_edge("repo:B2", "repo:B1"),
            ],
        };
        let frontier = Frontier {
            entries: vec![
                frontier_entry("r", "lane-b", 0, "repo:B1"),
                frontier_entry("r", "lane-b", 1, "repo:B2"),
            ],
            gate_ranks: Vec::new(),
        };

        let leverage = lane_leverage(&graph, &lane_positions, &frontier);
        let s = leverage
            .get(&SegmentKey {
                roadmap: "r".to_string(),
                lane: "lane-a".to_string(),
                segment: 0,
            })
            .unwrap();
        assert_eq!(
            s.lanes_freed, 1,
            "two segments in lane-b must collapse to one lane, got {s:?}"
        );
        assert_eq!(s.lanes, vec!["r/lane-b".to_string()]);
    }

    #[test]
    fn segment_gating_nothing_scores_zero() {
        let lane_positions = vec![lane_pos("r", "lane-a", 0, "repo", "A1")];
        let graph = StateGraph {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        let frontier = Frontier {
            entries: vec![frontier_entry("r", "lane-a", 0, "repo:A1")],
            gate_ranks: Vec::new(),
        };

        let leverage = lane_leverage(&graph, &lane_positions, &frontier);
        let s = leverage
            .get(&SegmentKey {
                roadmap: "r".to_string(),
                lane: "lane-a".to_string(),
                segment: 0,
            })
            .unwrap();
        assert_eq!(s.lanes_freed, 0);
        assert!(s.lanes.is_empty());
    }

    #[test]
    fn self_lane_exclusion_holds() {
        // S = lane-a/0 = {repo:A1}. lane-a/1 head A2 depends on A1 — same lane,
        // later segment. Must never count toward its own lanes_freed.
        let lane_positions = vec![
            lane_pos("r", "lane-a", 0, "repo", "A1"),
            lane_pos("r", "lane-a", 1, "repo", "A2"),
        ];
        let graph = StateGraph {
            nodes: Vec::new(),
            edges: vec![state_edge("repo:A2", "repo:A1")],
        };
        let frontier = Frontier {
            entries: vec![frontier_entry("r", "lane-a", 1, "repo:A2")],
            gate_ranks: Vec::new(),
        };

        let leverage = lane_leverage(&graph, &lane_positions, &frontier);
        let s = leverage
            .get(&SegmentKey {
                roadmap: "r".to_string(),
                lane: "lane-a".to_string(),
                segment: 0,
            })
            .unwrap();
        assert_eq!(
            s.lanes_freed, 0,
            "own lane must never count toward its own leverage, got {s:?}"
        );
        assert!(s.lanes.is_empty());
    }

    #[test]
    fn lane_leverage_over_untruncated_graph_returns_the_same_map() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-lane-leverage-plan");
        std::fs::create_dir_all(&root).unwrap();

        let lane_positions = vec![
            lane_pos("r", "lane-a", 0, "repo", "A1"),
            lane_pos("r", "lane-b", 0, "repo", "B1"),
        ];
        let graph = StateGraph {
            nodes: Vec::new(),
            edges: vec![state_edge("repo:B1", "repo:A1")],
        };
        let frontier = Frontier {
            entries: vec![frontier_entry("r", "lane-b", 0, "repo:B1")],
            gate_ranks: Vec::new(),
        };

        let result =
            lane_leverage_over_untruncated_graph(&root, &[], &graph, &lane_positions, &frontier)
                .expect("untruncated in-process graph must never be refused");
        let s = result
            .get(&SegmentKey {
                roadmap: "r".to_string(),
                lane: "lane-a".to_string(),
                segment: 0,
            })
            .unwrap();
        assert_eq!(s.lanes_freed, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn segment_statuses_with_slots_reports_degraded_flag() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-slots-artifact");
        std::fs::create_dir_all(&root).unwrap();
        write_harness(&root, "mev", Some("native-build"));
        // No .fleet-locks directory -> degraded read, no hold.

        let e = entry(vec![], vec![]);
        let frontier = Frontier {
            entries: vec![e],
            gate_ranks: Vec::new(),
        };
        let repos = vec![repo_entry("mev")];
        let (statuses, degraded) = segment_statuses_with_slots(&frontier, &[], &repos, &root, &[]);
        assert!(degraded);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].availability, SegmentAvailability::Startable);
        let _ = std::fs::remove_dir_all(&root);
    }

    // -----------------------------------------------------------------------
    // plan_availability + render_availability_text — MV.13.C Task 5
    // -----------------------------------------------------------------------

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn availability_state_file_fixture(repo: &str, id: &str) -> (StateSource, StateFile) {
        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("{repo}/planning/state.json")),
            expected_kind: "project",
        };
        let file: StateFile = serde_json::from_str(&format!(
            r#"{{"repo":"{repo}","kind":"project","updated":"2026-08-17","tracks":[
                {{"title":"t","blocks":[{{"id":"{id}","title":"x","status":"open"}}]}}
            ]}}"#
        ))
        .unwrap();
        (src, file)
    }

    #[test]
    fn plan_availability_writes_artifact_with_leverage_and_derived_at() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-availability-basic");
        write_file(
            &dir,
            "planning/roadmaps/alpha/lane-substrate.json",
            r#"{"lane":"substrate","roadmap":"alpha","blocks":[{"id":"MV.ticket.a","origin_roadmap":"alpha","repo":"mev"}]}"#,
        );
        let loaded = vec![availability_state_file_fixture("mev", "MV.ticket.a")];

        let plan = plan_availability(&dir, &loaded);
        assert!(
            plan.diagnostics.is_empty(),
            "expected no diagnostics, got {:?}",
            plan.diagnostics
        );
        assert_eq!(plan.actions.len(), 1, "expected exactly one write action");

        let action = &plan.actions[0];
        assert_eq!(action.path, dir.join(LANE_AVAILABILITY_ARTIFACT));

        let artifact: serde_json::Value =
            serde_json::from_str(&action.new_content).expect("artifact must be valid JSON");
        let derived_at = artifact["derived_at"].as_str().expect("derived_at string");
        chrono::DateTime::parse_from_rfc3339(derived_at)
            .expect("derived_at must be a parseable RFC 3339 timestamp");
        assert_eq!(
            artifact["degraded"], true,
            "no .fleet-locks dir -> degraded"
        );

        let segments = artifact["segments"].as_array().expect("segments array");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0]["roadmap"], "alpha");
        assert_eq!(segments[0]["lane"], "substrate");
        assert_eq!(segments[0]["repo"], "mev");
        assert_eq!(segments[0]["availability"], "startable");
        assert_eq!(segments[0]["leverage"]["lanes_freed"], 0);
        assert!(
            segments[0]["leverage"]["lanes"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_availability_no_lane_files_plans_nothing() {
        let dir = crate::testsupport::unique_temp_dir("mev-plan-availability-empty");
        std::fs::create_dir_all(dir.join("planning")).unwrap();

        let plan = plan_availability(&dir, &[]);
        assert!(plan.actions.is_empty());
        assert!(plan.diagnostics.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn render_availability_text_shape_for_startable_and_held_block_entries() {
        let artifact = LaneAvailabilityArtifact {
            derived_at: "2026-08-17T00:00:00+00:00".to_string(),
            degraded: false,
            segments: vec![
                LaneAvailabilityEntry {
                    status: SegmentStatus {
                        roadmap: "alpha".to_string(),
                        lane: "derive".to_string(),
                        segment: 0,
                        repo: "mev".to_string(),
                        head: Some("mev:MV.13.C".to_string()),
                        availability: SegmentAvailability::Startable,
                        reason: None,
                    },
                    leverage: LaneLeverage {
                        lanes_freed: 2,
                        lanes: vec!["alpha/derive-2".to_string(), "beta/derive".to_string()],
                    },
                },
                LaneAvailabilityEntry {
                    status: SegmentStatus {
                        roadmap: "alpha".to_string(),
                        lane: "consolidate".to_string(),
                        segment: 1,
                        repo: "bastion".to_string(),
                        head: Some("bastion:BA.19.C".to_string()),
                        availability: SegmentAvailability::HeldBlock,
                        reason: Some("blocked by mev:MV.13.C".to_string()),
                    },
                    leverage: LaneLeverage::default(),
                },
            ],
        };

        let text = render_availability_text(&artifact);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "alpha/derive#0 mev:MV.13.C — startable frees 2 lanes"
        );
        assert_eq!(
            lines[1],
            "alpha/consolidate#1 bastion:BA.19.C — held-block (blocked by mev:MV.13.C) frees 0 lanes"
        );
    }

    #[test]
    fn render_availability_text_singular_lane_noun() {
        let artifact = LaneAvailabilityArtifact {
            derived_at: "2026-08-17T00:00:00+00:00".to_string(),
            degraded: false,
            segments: vec![LaneAvailabilityEntry {
                status: SegmentStatus {
                    roadmap: "alpha".to_string(),
                    lane: "derive".to_string(),
                    segment: 0,
                    repo: "mev".to_string(),
                    head: None,
                    availability: SegmentAvailability::Done,
                    reason: None,
                },
                leverage: LaneLeverage {
                    lanes_freed: 1,
                    lanes: vec!["alpha/consolidate".to_string()],
                },
            }],
        };

        let text = render_availability_text(&artifact);
        assert_eq!(text, "alpha/derive#0 mev:- — done frees 1 lane");
    }

    // -----------------------------------------------------------------
    // discover_segments — MV.ticket.done-segment-discovery
    // -----------------------------------------------------------------

    /// The hotfix's core: `done` is only reachable because this returns segments the
    /// frontier has no entry for. Covered end-to-end by `tests/lanes_driver.rs`, but its
    /// own contract — one entry per `(roadmap, lane, segment)` triple, first-appearance
    /// order, repo carried through — was never asserted directly.
    #[test]
    fn discover_segments_yields_one_entry_per_triple_in_first_appearance_order() {
        let positions = vec![
            lane_pos("rm", "derive", 0, "mev", "MV.1.A"),
            lane_pos("rm", "derive", 0, "mev", "MV.1.B"),
            lane_pos("rm", "derive", 1, "bastion", "BA.1.A"),
            lane_pos("rm", "wire", 0, "bastion", "BA.2.A"),
            lane_pos("rm", "derive", 0, "mev", "MV.1.C"),
        ];

        let segs = discover_segments(&positions);

        let keys: Vec<(String, String, usize, String)> = segs
            .iter()
            .map(|s| (s.roadmap.clone(), s.lane.clone(), s.segment, s.repo.clone()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("rm".into(), "derive".into(), 0, "mev".into()),
                ("rm".into(), "derive".into(), 1, "bastion".into()),
                ("rm".into(), "wire".into(), 0, "bastion".into()),
            ],
            "three distinct triples, deduped, in first-appearance order — the trailing \
             MV.1.C must not re-add derive#0"
        );
    }

    #[test]
    fn discover_segments_is_empty_for_no_positions() {
        assert!(discover_segments(&[]).is_empty());
    }

    // -----------------------------------------------------------------
    // heavy_category — MV.13.C Task 3
    // -----------------------------------------------------------------

    /// Distinct from the 3-arg `write_harness` above (which builds a canned category
    /// under a named sub-repo); this writes a literal body at the repo root under test.
    fn write_harness_body(root: &std::path::Path, body: &str) {
        let dir = root.join("planning");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("harness.json"), body).unwrap();
    }

    #[test]
    fn heavy_category_classifies_browser_before_native_and_light_as_none() {
        let dir = tempfile::tempdir().unwrap();

        // uiTest.enabled wins outright, even with a native-build command present.
        write_harness_body(
            dir.path(),
            r#"{"uiTest":{"enabled":true},"validation":{"checks":[{"command":"cargo build --release"}]}}"#,
        );
        assert_eq!(
            heavy_category(dir.path()).as_deref(),
            Some("browser-automation"),
            "browser-automation is the more resource-dangerous class and is checked first"
        );

        write_harness_body(
            dir.path(),
            r#"{"validation":{"checks":[{"command":"cargo build --release"}]}}"#,
        );
        assert_eq!(heavy_category(dir.path()).as_deref(), Some("native-build"));

        write_harness_body(
            dir.path(),
            r#"{"validation":{"checks":[{"command":"cargo fmt --check"}]}}"#,
        );
        assert_eq!(
            heavy_category(dir.path()),
            None,
            "a repo with only cheap gates is light"
        );
    }

    /// Pins a KNOWN HAZARD rather than endorsing it: a repo path with no
    /// `planning/harness.json` returns `None`, which is indistinguishable from "this repo
    /// is light". A mistyped or wrongly-relative path therefore reads as light in the one
    /// derivation that exists to stop the fleet being overloaded. This bit for real on
    /// 2026-08-17 via the Python twin (`fleet_concurrency_check.py:305-307`), where
    /// `is-heavy --repo-path core/mev` answered `heavy: false` for a path that did not
    /// resolve, and mev was in fact `native-build`. Tracked as carryover
    /// `is-heavy-answers-light-for-a-nonexistent-repo-path` (owner base-template). If that
    /// carryover is resolved by making absence an error, this test is the one that must
    /// change — deliberately, not by accident.
    #[test]
    fn heavy_category_returns_none_for_a_missing_harness_which_reads_as_light() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(heavy_category(&dir.path().join("no-such-repo")), None);
        assert_eq!(
            heavy_category(dir.path()),
            None,
            "existing dir, absent harness.json — same answer as a genuinely light repo"
        );
    }

    // -----------------------------------------------------------------
    // FleetSlotView accessors + pub category_capacity — BA.19.D Task 1
    // -----------------------------------------------------------------

    #[test]
    fn category_capacity_returns_documented_caps() {
        assert_eq!(category_capacity("native-build"), 4);
        assert_eq!(category_capacity("browser-automation"), 2);
        assert_eq!(category_capacity("some-unknown-category"), 2);
    }

    #[test]
    fn known_categories_includes_zero_hold_categories_and_sorts() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-known-categories");
        let now = now_unix_seconds();
        // Only native-build has a live hold; browser-automation has none.
        write_lock_entry(
            &root,
            "some-repo",
            "native-build",
            std::process::id() as i64,
            now,
            "p",
        );

        let slot_view = compute_fleet_slot_view(&root);
        assert!(!slot_view.degraded);
        assert_eq!(
            slot_view.known_categories(),
            vec!["browser-automation".to_string(), "native-build".to_string()],
            "browser-automation still reports with zero live holds, sorted order"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn known_categories_includes_a_category_seen_only_in_live_data() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-known-categories-extra");
        let now = now_unix_seconds();
        write_lock_entry(
            &root,
            "some-repo",
            "some-other-category",
            std::process::id() as i64,
            now,
            "p",
        );

        let slot_view = compute_fleet_slot_view(&root);
        assert_eq!(
            slot_view.known_categories(),
            vec![
                "browser-automation".to_string(),
                "native-build".to_string(),
                "some-other-category".to_string(),
            ],
            "a category present only in live-hold data still appears, alongside the two \
             known-capacity categories, sorted"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn live_repos_covers_live_stale_ttl_dead_pid_and_no_category_entries() {
        let root = crate::testsupport::unique_temp_dir("mev-availability-live-repos-mix");
        let now = now_unix_seconds();

        // (a) a live entry.
        write_lock_entry(
            &root,
            "repo-live",
            "browser-automation",
            std::process::id() as i64,
            now,
            "p",
        );
        // (b) a stale-by-ttl entry (alive pid, started_at well past the 4h TTL).
        write_lock_entry(
            &root,
            "repo-stale-ttl",
            "browser-automation",
            std::process::id() as i64,
            now - (5.0 * 60.0 * 60.0),
            "p",
        );
        // (c) a dead-pid entry.
        write_lock_entry(
            &root,
            "repo-dead-pid",
            "browser-automation",
            999_999_999,
            now,
            "p",
        );
        // (d) an entry with no `category` field — defaults to browser-automation.
        let dir = root.join(FLEET_LOCK_SUBDIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("repo-no-category__p.json"),
            serde_json::to_string(&serde_json::json!({
                "repo": "repo-no-category",
                "pid": std::process::id(),
                "started_at": now,
            }))
            .unwrap(),
        )
        .unwrap();

        let slot_view = compute_fleet_slot_view(&root);
        assert!(!slot_view.degraded);
        assert_eq!(
            slot_view.live_repos("browser-automation"),
            vec!["repo-live".to_string(), "repo-no-category".to_string()],
            "only the genuinely live entries count, sorted; the no-category entry \
             defaults into browser-automation; stale-ttl and dead-pid entries are excluded"
        );
        assert_eq!(
            slot_view.live_repos("native-build"),
            Vec::<String>::new(),
            "a category with no live holds returns an empty vec, never absent/None"
        );
        assert_eq!(
            slot_view.live_repos("unknown-category"),
            Vec::<String>::new(),
            "an entirely unknown category also returns an empty vec"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn category_capacity_matches_known_categories_documented_values() {
        // Cross-check: every category known_categories() ever surfaces resolves
        // through category_capacity() to one of the two documented caps.
        let root = crate::testsupport::unique_temp_dir("mev-availability-cap-cross-check");
        let slot_view = compute_fleet_slot_view(&root);
        for category in slot_view.known_categories() {
            let cap = category_capacity(&category);
            assert!(
                cap == 4 || cap == 2,
                "category {category} resolved to unexpected cap {cap}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
