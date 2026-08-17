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

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Diagnostic;
use crate::brain::config::RepoEntry;
use crate::brain::frontier::{Frontier, FrontierEntry};
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

/// Compute intrinsic [`SegmentStatus`]es for every entry in `frontier`.
///
/// `Done` segments (every block closed, so no frontier entry exists) are not produced
/// here — the frontier itself carries no record of a closed-out segment, since
/// [`crate::brain::frontier::compute_frontier`] skips segments whose head-search finds
/// nothing. A future task threading in `lane_segments`' full segment list (not just
/// the frontier's live subset) is what would let `Done` segments be reported too; this
/// task's tests exercise `Done` directly against a synthetic entry set instead of
/// wiring that discovery in yet, since no task in this spec names it as in scope.
pub fn intrinsic_segment_statuses(frontier: &Frontier) -> Vec<SegmentStatus> {
    frontier
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
        .collect()
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
/// in `frontier`. Supersedes [`intrinsic_segment_statuses`] as the availability
/// entry point once a repo-liveness reading (from [`discover_live_runs`]) is in
/// hand; `intrinsic_segment_statuses` remains for callers (and tests) that only need
/// the intrinsic tier.
pub fn segment_statuses(frontier: &Frontier, live_runs: &[LiveRun]) -> Vec<SegmentStatus> {
    frontier
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
        .collect()
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

    #[test]
    fn held_block_outranks_held_operator_when_both_present() {
        // A head matching both unmet_blocks and unmet_gates reports HeldBlock per the
        // documented precedence, not HeldOperator.
        let e = entry(vec!["mev:MV.13.B"], vec!["operator:review-session"]);
        let (avail, reason) = intrinsic_status(&e);
        assert_eq!(avail, SegmentAvailability::HeldBlock);
        assert_eq!(reason.as_deref(), Some("blocked by mev:MV.13.B"));
    }

    #[test]
    fn all_closed_segment_reports_done_with_no_head() {
        // A segment with every block closed contributes no FrontierEntry at all
        // (compute_frontier skips it) — Done is represented by absence, and a
        // SegmentStatus built for a Done segment carries head: None.
        let frontier = Frontier::default();
        let statuses = intrinsic_segment_statuses(&frontier);
        assert!(
            statuses.is_empty(),
            "an all-closed / entryless frontier should yield no live segment statuses"
        );

        // Directly construct the Done representation a caller would build once it
        // knows (from lane_segments, out of scope for this task) that a segment
        // exists but has no live head.
        let done = SegmentStatus {
            roadmap: "engine-orchestration".to_string(),
            lane: "derive".to_string(),
            segment: 0,
            repo: "mev".to_string(),
            head: None,
            availability: SegmentAvailability::Done,
            reason: None,
        };
        assert_eq!(done.availability, SegmentAvailability::Done);
        assert_eq!(done.head, None);
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
        let statuses = segment_statuses(&frontier, &live_runs);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].availability, SegmentAvailability::HeldRepoBusy);
    }
}
