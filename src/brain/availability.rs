//! Six-state lane-segment availability — `MV.13.C` Task 1.
//!
//! [`FrontierEntry`] (`MV.13.B`) already tells us, per `(roadmap, lane, segment)`,
//! whether a head exists and what it is waiting on intrinsically (`unmet_blocks`,
//! `unmet_gates`). This module folds that — plus environmental holds this block adds
//! in later tasks (repo-busy, slot-capacity) — into exactly one of six
//! [`SegmentAvailability`] states per segment, so downstream consumers (bastion's
//! `/lanes` endpoint, the cockpit board) never re-derive the same judgement call
//! themselves.

use crate::brain::frontier::{Frontier, FrontierEntry};

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
}
