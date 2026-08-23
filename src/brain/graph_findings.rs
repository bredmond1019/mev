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

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::Diagnostic;
use crate::brain::config::BrainConfig;
use crate::brain::lane_segments::{LaneFile, discover_lane_files};
use crate::brain::state::{StateFile, StateSource, discover_state_files, load_state};

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
// Detector 1: unregistered-lane-block
// ---------------------------------------------------------------------------

/// Detector 1 — `unregistered-lane-block`, over already-discovered/loaded corpus
/// data. Pure with respect to disk: callers do the I/O (see
/// [`detect_unregistered_lane_blocks`] for the disk-facing wrapper task 4's CLI
/// subcommand uses), which keeps this half unit-testable without a fixture
/// directory per case.
///
/// For every `blocks[]` entry in every discovered lane record, reports a finding
/// when the entry's OWN authored `repo` (never the lane's or the lane file's
/// location — a lane is not single-repo in this corpus, per
/// [`crate::brain::lane_segments::LaneBlockRef::repo`]) has no `state.json` with a
/// matching `tracks[].blocks[].id`. An id registered under a *different* repo than
/// the one the lane entry names still produces a finding — registration is keyed
/// on `(repo, id)`, not `id` alone.
///
/// `lane_diags` (the diagnostics [`discover_lane_files`] returned alongside
/// `lane_files`) are carried through verbatim into the returned diagnostics — a
/// lane record that failed to parse must surface as an error here too, never
/// silently contribute zero findings (standing rule 11: a clean-looking empty
/// result from a detector that could not read its input is the exact failure mode
/// this exists to avoid).
#[must_use]
pub fn unregistered_lane_block_findings(
    lane_files: &[LaneFile],
    lane_diags: &[Diagnostic],
    state_files: &[(StateSource, StateFile)],
) -> (Vec<GraphFinding>, Vec<Diagnostic>) {
    let mut registered: HashSet<(&str, &str)> = HashSet::new();
    for (src, file) in state_files {
        for track in &file.tracks {
            for block in &track.blocks {
                registered.insert((src.repo_slug.as_str(), block.id.as_str()));
            }
        }
    }

    let mut findings = Vec::new();
    for lane_file in lane_files {
        for block_ref in &lane_file.blocks {
            if registered.contains(&(block_ref.repo.as_str(), block_ref.id.as_str())) {
                continue;
            }
            let subject = format!("{}:{}", block_ref.repo, block_ref.id);
            let message = format!(
                "lane '{}' (roadmap '{}', {}) names block '{}' owned by repo '{}', which has \
                 no matching tracks[].blocks[].id in {}'s planning/state.json",
                lane_file.lane,
                lane_file.roadmap,
                lane_file.path.display(),
                block_ref.id,
                block_ref.repo,
                block_ref.repo,
            );
            findings.push(GraphFinding {
                detector: DetectorClass::UnregisteredLaneBlock,
                repo: block_ref.repo.clone(),
                subject: subject.clone(),
                message,
                finding_id: finding_id(DetectorClass::UnregisteredLaneBlock, &subject),
            });
        }
    }

    (findings, lane_diags.to_vec())
}

/// Disk-facing wrapper for [`unregistered_lane_block_findings`]: discovers every
/// lane record and every `planning/state.json` under `root` and reduces them
/// through the pure detector above. This is what task 4's `mev graph-findings`
/// subcommand calls; a `state.json` that fails to load is skipped (matching
/// [`crate::block_graph_brain`]'s posture — an individual malformed file is not
/// fatal to the run), while a lane record that fails to parse is surfaced as an
/// error diagnostic via `lane_diags`, never silently dropped.
#[must_use]
pub fn detect_unregistered_lane_blocks(
    root: &Path,
    config: &BrainConfig,
) -> (Vec<GraphFinding>, Vec<Diagnostic>) {
    let (lane_files, lane_diags) = discover_lane_files(root);
    let (sources, _state_discovery_diags) = discover_state_files(root, config);

    let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
    for src in &sources {
        if let Ok(file) = load_state(&src.abs_path) {
            loaded.push((src.clone(), file));
        }
    }

    unregistered_lane_block_findings(&lane_files, &lane_diags, &loaded)
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

    // -----------------------------------------------------------------
    // Detector 1: unregistered-lane-block
    // -----------------------------------------------------------------

    use crate::brain::lane_segments::LaneBlockRef;
    use crate::brain::state::{Focus, Track, TrackBlock};
    use std::path::PathBuf;

    fn lane_block_ref(id: &str, repo: &str) -> LaneBlockRef {
        LaneBlockRef {
            id: id.to_string(),
            line: 1,
            origin_roadmap: Some("alpha".to_string()),
            repo: repo.to_string(),
        }
    }

    fn lane_file(lane: &str, roadmap: &str, blocks: Vec<LaneBlockRef>) -> LaneFile {
        LaneFile {
            roadmap: roadmap.to_string(),
            lane: lane.to_string(),
            path: PathBuf::from(format!("planning/roadmaps/{roadmap}/lane-{lane}.json")),
            blocks,
            directives: None,
        }
    }

    fn state_source(repo: &str) -> StateSource {
        StateSource {
            repo_slug: repo.to_string(),
            abs_path: PathBuf::from(format!("/{repo}/planning/state.json")),
            expected_kind: "project",
        }
    }

    fn track_block(id: &str) -> TrackBlock {
        TrackBlock {
            id: id.to_string(),
            title: format!("Block {id}"),
            status: None,
            depends_on: Vec::new(),
            ..Default::default()
        }
    }

    fn project_file(repo: &str, blocks: Vec<TrackBlock>) -> StateFile {
        StateFile {
            repo: repo.to_string(),
            kind: "project".to_string(),
            updated: "2026-01-01".to_string(),
            focus: Focus::default(),
            tracks: vec![Track {
                title: "Phase 1".to_string(),
                blocks,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn registered_lane_block_produces_no_finding() {
        let lane = lane_file("substrate", "alpha", vec![lane_block_ref("MV.1.A", "mev")]);
        let state_files = vec![(
            state_source("mev"),
            project_file("mev", vec![track_block("MV.1.A")]),
        )];

        let (findings, diags) = unregistered_lane_block_findings(&[lane], &[], &state_files);
        assert!(
            findings.is_empty(),
            "expected no findings, got {findings:?}"
        );
        assert!(diags.is_empty());
    }

    #[test]
    fn unregistered_lane_block_produces_exactly_one_finding() {
        let lane = lane_file("substrate", "alpha", vec![lane_block_ref("MV.1.A", "mev")]);
        // The registered id is a DIFFERENT id in the same repo -- MV.1.A itself is
        // never registered.
        let state_files = vec![(
            state_source("mev"),
            project_file("mev", vec![track_block("MV.9.Z")]),
        )];

        let (findings, diags) = unregistered_lane_block_findings(&[lane], &[], &state_files);
        assert_eq!(
            findings.len(),
            1,
            "expected exactly one finding, got {findings:?}"
        );
        assert!(diags.is_empty());

        let f = &findings[0];
        assert_eq!(f.detector, DetectorClass::UnregisteredLaneBlock);
        assert_eq!(f.repo, "mev");
        assert_eq!(f.subject, "mev:MV.1.A");
        assert_eq!(
            f.finding_id,
            finding_id(DetectorClass::UnregisteredLaneBlock, "mev:MV.1.A")
        );
    }

    #[test]
    fn id_registered_in_a_different_repo_still_produces_a_finding() {
        // The lane entry names repo "mev", but the ONLY state.json registering
        // "MV.1.A" belongs to "base-template". Ownership resolves against the
        // lane entry's own authored `repo`, never against wherever the id
        // happens to be registered -- so this must still fire.
        let lane = lane_file("substrate", "alpha", vec![lane_block_ref("MV.1.A", "mev")]);
        let state_files = vec![(
            state_source("base-template"),
            project_file("base-template", vec![track_block("MV.1.A")]),
        )];

        let (findings, _diags) = unregistered_lane_block_findings(&[lane], &[], &state_files);
        assert_eq!(
            findings.len(),
            1,
            "expected exactly one finding, got {findings:?}"
        );
        assert_eq!(findings[0].repo, "mev");
        assert_eq!(findings[0].subject, "mev:MV.1.A");
    }

    #[test]
    fn unparseable_lane_record_surfaces_an_error_not_a_clean_empty_result() {
        // Reuse the existing malformed-record fixture (lane_segments.rs's own
        // "unknown top-level key" regression) rather than duplicating one: run
        // real discovery over it, then feed the diagnostics it returns through
        // the detector and assert they survive -- this is standing rule 11's
        // positive control applied to this detector specifically. A detector
        // that silently swallowed discover_lane_files' diagnostics would report
        // zero findings here for exactly the wrong reason (it could not read
        // its input, not because the corpus is clean).
        let dir = crate::testsupport::unique_temp_dir("mev-graph-findings-malformed-lane");
        let fixture = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/lane_json/unknown_key_lane.json"),
        )
        .expect("fixture must exist");
        let target = dir.join("planning/roadmaps/alpha/lane-docs-sync.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, fixture).unwrap();

        let (lane_files, lane_diags) = discover_lane_files(&dir);
        assert!(
            lane_files.is_empty(),
            "the malformed record must not be parsed into a usable LaneFile"
        );
        assert!(
            !lane_diags.is_empty(),
            "discover_lane_files must surface a diagnostic for the malformed record"
        );

        let (findings, diags) = unregistered_lane_block_findings(&lane_files, &lane_diags, &[]);
        assert!(
            findings.is_empty(),
            "no lane blocks were discoverable, so there is nothing to report as unregistered"
        );
        assert!(
            diags.iter().any(|d| d.severity == crate::Severity::Error),
            "the parse failure must surface as an error diagnostic, not silently \
             produce a clean-looking empty result: {diags:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
