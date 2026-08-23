//! Mechanically-detectable carryover findings — `mev graph-findings` (Phase Jynx,
//! `MV.ticket.graph-derived-carryover-findings`).
//!
//! A whole class of `carryover[]` entries is deterministically derivable from the
//! corpus rather than found by an agent reading files: a lane file naming a block
//! no `state.json` registers, or a doc naming a script that exists nowhere. This
//! module owns the report model ([`GraphFinding`], [`DetectorClass`],
//! [`GraphFindingsReport`]) and the stable, content-derived [`finding_id`] shared by
//! every detector, so the *same* finding filed independently by several repos
//! correlates to one id (task 1). The detectors themselves —
//! `unregistered-lane-block` (task 2) and `referenced-path-absent` (task 3) — are
//! layered on top of this module in follow-on tasks of the same file.
//!
//! Modelled on this crate's two established report shapes —
//! [`crate::brain::block_graph::BlockGraphExport`] and
//! [`crate::brain::carryover::CarryoverReport`] — rather than inventing a third
//! convention: a header/summary struct, a flat `Vec` of typed rows, and per-class
//! counts alongside the total.

use serde::Serialize;
use sha2::{Digest as _, Sha256};

// ---------------------------------------------------------------------------
// Detector classes
// ---------------------------------------------------------------------------

/// Which deterministic detector produced a [`GraphFinding`].
///
/// Two classes ship in this ticket. Adding a third means adding a variant here,
/// a `tag()` arm, a counter field on [`GraphFindingsReport`], and — because
/// `tag()` feeds [`finding_id`] — every existing `finding_id` stays stable
/// (the tag strings of the existing variants must never change).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DetectorClass {
    /// A block id named in some `lane-*.json`'s `blocks[]` has no matching
    /// `tracks[].blocks[].id` in its owning repo's `state.json`.
    UnregisteredLaneBlock,
    /// A path named as a script or generator in a command or spec resolves
    /// nowhere in the fleet.
    ReferencedPathAbsent,
}

impl DetectorClass {
    /// Stable, `kebab-case` identifier for this class — used both as the
    /// human-readable label and as the first component hashed into
    /// [`finding_id`]. **Never rename an existing arm's string**: doing so
    /// changes every `finding_id` already written to a live `state.json`.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            DetectorClass::UnregisteredLaneBlock => "unregistered-lane-block",
            DetectorClass::ReferencedPathAbsent => "referenced-path-absent",
        }
    }
}

// ---------------------------------------------------------------------------
// finding_id
// ---------------------------------------------------------------------------

/// Derive the stable, content-derived `finding_id` for one finding.
///
/// Hashed over `(detector.tag(), subject)` and **nothing else** — never the
/// owning repo, the file path the finding was found in, a timestamp, or an
/// index. That is the entire point (per the block record): the same finding
/// filed independently from three different repos must produce the same id,
/// so `mev carryover`'s existing clustering can correlate them. `subject`
/// must already be normalized by the caller (see
/// [`normalize_referenced_path`] for the `ReferencedPathAbsent` case) —
/// this function does no normalization of its own, so two differently
/// spelled subjects for the same real-world thing will *not* collide unless
/// the caller normalizes first.
///
/// `sha2` is already a direct dependency (used by
/// [`crate::brain::attention_payload::item_id_for`]); reused here rather
/// than adding a second hasher.
#[must_use]
pub fn finding_id(detector: DetectorClass, subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(detector.tag().as_bytes());
    hasher.update(b"\0");
    hasher.update(subject.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Path normalization (contract for the ReferencedPathAbsent subject)
// ---------------------------------------------------------------------------

/// Normalize a referenced path into the stable subject fed to [`finding_id`]
/// for the `referenced-path-absent` class.
///
/// Collapses a bare relative reference, a `./`-prefixed one, and one
/// prefixed with a single leading repo/tier directory to the same subject —
/// exactly the `render-spec.py` case the block record requires to
/// correlate across repos: `scripts/render_spec.py`,
/// `./scripts/render_spec.py`, and `base-template/scripts/render_spec.py`
/// all normalize to `scripts/render_spec.py`.
///
/// Rule: strip a leading `./`, split on `/`, and keep at most the last two
/// non-empty components (parent directory + file name). A path of one or
/// two components is returned unchanged (aside from the `./` strip); a
/// deeper path has any leading repo/tier/nesting prefix dropped. This is
/// deliberately coarse — `parent/name.ext` is specific enough to avoid
/// collapsing genuinely distinct scripts that merely share a filename in
/// different parent directories (e.g. `scripts/build.sh` vs
/// `hooks/build.sh` stay distinct), while dropping exactly the kind of
/// single leading repo-name prefix that made the same script look like
/// three different subjects across three repos.
#[must_use]
pub fn normalize_referenced_path(path: &str) -> String {
    let trimmed = path.strip_prefix("./").unwrap_or(path);
    let components: Vec<&str> = trimmed.split('/').filter(|c| !c.is_empty()).collect();
    if components.len() <= 2 {
        components.join("/")
    } else {
        components[components.len() - 2..].join("/")
    }
}

// ---------------------------------------------------------------------------
// Report model
// ---------------------------------------------------------------------------

/// One deterministically-detected finding.
#[derive(Debug, Clone, Serialize)]
pub struct GraphFinding {
    /// Which detector produced this row.
    pub detector: DetectorClass,
    /// The repo the finding is scoped to (the owning repo whose `state.json`
    /// would receive the `carryover[]` entry on `--write`).
    pub repo: String,
    /// The normalized subject hashed into `finding_id` — see
    /// [`normalize_referenced_path`] for the `ReferencedPathAbsent` case, and
    /// the `{repo}:{id}` key for `UnregisteredLaneBlock` (task 2).
    pub subject: String,
    /// Human-readable explanation, self-contained enough to act on without
    /// opening the source file.
    pub message: String,
    /// Stable, content-derived id from [`finding_id`] — identical across
    /// every repo independently reporting the same `(detector, subject)`.
    pub finding_id: String,
}

/// The full `mev graph-findings` report.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GraphFindingsReport {
    /// Total findings across every detector class.
    pub total: usize,
    /// Count of [`DetectorClass::UnregisteredLaneBlock`] findings.
    pub unregistered_lane_block: usize,
    /// Count of [`DetectorClass::ReferencedPathAbsent`] findings.
    pub referenced_path_absent: usize,
    /// Every finding, in detection order.
    pub findings: Vec<GraphFinding>,
}

impl GraphFindingsReport {
    /// Build a report from a flat list of findings, deriving `total` and the
    /// per-class counts from the rows themselves so the counts can never
    /// drift out of sync with `findings`.
    #[must_use]
    pub fn from_findings(findings: Vec<GraphFinding>) -> Self {
        let mut report = GraphFindingsReport {
            total: findings.len(),
            ..GraphFindingsReport::default()
        };
        for finding in &findings {
            match finding.detector {
                DetectorClass::UnregisteredLaneBlock => report.unregistered_lane_block += 1,
                DetectorClass::ReferencedPathAbsent => report.referenced_path_absent += 1,
            }
        }
        report.findings = findings;
        report
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_class_same_subject_yields_same_id() {
        let a = finding_id(DetectorClass::UnregisteredLaneBlock, "mev:MV.1.A");
        let b = finding_id(DetectorClass::UnregisteredLaneBlock, "mev:MV.1.A");
        assert_eq!(a, b);
    }

    #[test]
    fn different_class_same_subject_yields_different_id() {
        let a = finding_id(
            DetectorClass::UnregisteredLaneBlock,
            "scripts/render_spec.py",
        );
        let b = finding_id(
            DetectorClass::ReferencedPathAbsent,
            "scripts/render_spec.py",
        );
        assert_ne!(a, b);
    }

    #[test]
    fn different_subject_same_class_yields_different_id() {
        let a = finding_id(
            DetectorClass::ReferencedPathAbsent,
            "scripts/render_spec.py",
        );
        let b = finding_id(DetectorClass::ReferencedPathAbsent, "scripts/other.py");
        assert_ne!(a, b);
    }

    #[test]
    fn finding_id_excludes_repo_and_is_stable_across_calls() {
        // The block record requires finding_id to be derived ONLY from
        // (detector class, normalized subject) -- never the owning repo, a
        // path it was found in, a timestamp, or an index. Simulate three
        // "repos" independently computing the id for the identical
        // normalized subject and assert they all agree, and that repeated
        // calls (standing in for repeated runs) are identical too.
        let subject = "scripts/render_spec.py";
        let from_mev = finding_id(DetectorClass::ReferencedPathAbsent, subject);
        let from_base_template = finding_id(DetectorClass::ReferencedPathAbsent, subject);
        let from_engine_rs = finding_id(DetectorClass::ReferencedPathAbsent, subject);
        assert_eq!(from_mev, from_base_template);
        assert_eq!(from_base_template, from_engine_rs);

        // Re-running "later" (nothing time-dependent in the function) still
        // agrees.
        let again = finding_id(DetectorClass::ReferencedPathAbsent, subject);
        assert_eq!(from_mev, again);
    }

    #[test]
    fn three_path_spellings_normalize_to_one_subject() {
        let bare = normalize_referenced_path("scripts/render_spec.py");
        let dot_relative = normalize_referenced_path("./scripts/render_spec.py");
        let repo_prefixed = normalize_referenced_path("base-template/scripts/render_spec.py");

        assert_eq!(bare, "scripts/render_spec.py");
        assert_eq!(dot_relative, bare);
        assert_eq!(repo_prefixed, bare);

        // And therefore they hash to one finding_id.
        let id_bare = finding_id(DetectorClass::ReferencedPathAbsent, &bare);
        let id_dot = finding_id(DetectorClass::ReferencedPathAbsent, &dot_relative);
        let id_prefixed = finding_id(DetectorClass::ReferencedPathAbsent, &repo_prefixed);
        assert_eq!(id_bare, id_dot);
        assert_eq!(id_dot, id_prefixed);
    }

    #[test]
    fn normalize_preserves_distinct_parent_directories() {
        // Two scripts that merely share a filename in different parent
        // directories must NOT collapse to the same subject.
        let a = normalize_referenced_path("scripts/build.sh");
        let b = normalize_referenced_path("hooks/build.sh");
        assert_ne!(a, b);
    }

    #[test]
    fn normalize_single_component_path_is_unchanged() {
        assert_eq!(normalize_referenced_path("README.md"), "README.md");
    }

    #[test]
    fn detector_class_tag_is_kebab_case_and_stable() {
        assert_eq!(
            DetectorClass::UnregisteredLaneBlock.tag(),
            "unregistered-lane-block"
        );
        assert_eq!(
            DetectorClass::ReferencedPathAbsent.tag(),
            "referenced-path-absent"
        );
    }

    #[test]
    fn report_from_findings_derives_counts_from_rows() {
        let findings = vec![
            GraphFinding {
                detector: DetectorClass::UnregisteredLaneBlock,
                repo: "mev".to_string(),
                subject: "mev:MV.1.A".to_string(),
                message: "unregistered".to_string(),
                finding_id: finding_id(DetectorClass::UnregisteredLaneBlock, "mev:MV.1.A"),
            },
            GraphFinding {
                detector: DetectorClass::ReferencedPathAbsent,
                repo: "mev".to_string(),
                subject: "scripts/render_spec.py".to_string(),
                message: "missing".to_string(),
                finding_id: finding_id(
                    DetectorClass::ReferencedPathAbsent,
                    "scripts/render_spec.py",
                ),
            },
            GraphFinding {
                detector: DetectorClass::ReferencedPathAbsent,
                repo: "base-template".to_string(),
                subject: "scripts/render_spec.py".to_string(),
                message: "missing".to_string(),
                finding_id: finding_id(
                    DetectorClass::ReferencedPathAbsent,
                    "scripts/render_spec.py",
                ),
            },
        ];

        let report = GraphFindingsReport::from_findings(findings);
        assert_eq!(report.total, 3);
        assert_eq!(report.unregistered_lane_block, 1);
        assert_eq!(report.referenced_path_absent, 2);
        assert_eq!(report.findings.len(), 3);
    }

    #[test]
    fn report_default_is_empty() {
        let report = GraphFindingsReport::default();
        assert_eq!(report.total, 0);
        assert_eq!(report.unregistered_lane_block, 0);
        assert_eq!(report.referenced_path_absent, 0);
        assert!(report.findings.is_empty());
    }
}
