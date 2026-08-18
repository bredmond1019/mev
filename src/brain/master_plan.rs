//! Master-plan generator — renders the initiative index and per-phase block
//! sections into a repo's `master-plan.md`, spliced between sentinel markers
//! (`MV.ticket.master-plan-generator`, task 1).
//!
//! Two data sources feed the render:
//! - `state.json`'s `tracks[]` ([`okf_core::state::TrackBlock`]) — the
//!   authoritative per-block `title`/`description`/`status`/`wave`/`depends_on`,
//!   grouped by phase (a `Track`'s `title` names its phase, matching the wave
//!   table's precedent).
//! - `planning/blocks/*.json` ([`BlockRecord`]) — the authored block record,
//!   read here only for its `initiative` field, to build the initiative index
//!   above the phase sections. `state.json` carries no `initiative` field, so
//!   this is the one piece of the render `tracks[]` alone cannot supply.
//!
//! Follows the wave-table splice's write mechanics exactly
//! ([`crate::brain::emit::splice_generated`]): never rewrite a file wholesale,
//! only the region between `<!-- BEGIN generated:master-plan-body -->` /
//! `<!-- END generated:master-plan-body -->`
//! ([`crate::brain::emit::markers::MASTER_PLAN_BODY`]).

use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

use crate::Diagnostic;
use crate::brain::emit::{EmitAction, EmitPlan, markers, splice_generated};
use crate::brain::state::{BlockedBy, StateFile, StateSource, TrackBlock};

// ---------------------------------------------------------------------------
// BlockRecord — the authored `planning/blocks/*.json` shape, narrowed to what
// this renderer needs
// ---------------------------------------------------------------------------

/// One authored block record file under `planning/blocks/*.json`.
///
/// Only the fields this renderer needs are modeled here — `what`, `why`,
/// `files`, and every other authored key on the block record are irrelevant
/// to the initiative index and intentionally not deserialized. Base-template's
/// `block.schema.json` is the authority on the full record shape; this struct
/// is a narrow read, not a validator (block-record validation is
/// `MV.ticket.block-record-validation`, out of scope here).
#[derive(Debug, Clone, Deserialize)]
pub struct BlockRecord {
    /// Canonical block ID, matching a `state.json` `TrackBlock.id`.
    pub id: String,
    /// Cross-repo initiative slug this block serves, when authored.
    #[serde(default)]
    pub initiative: Option<String>,
}

/// Read every `*.json` file directly under `blocks_dir` as a [`BlockRecord`].
///
/// A file that is not valid JSON or does not match the shape is skipped with
/// a warning [`Diagnostic`] rather than aborting the whole load — one
/// malformed record must not blank the initiative index for every other
/// block in the repo. Returns `(records, diagnostics)`, `records` sorted by
/// `id` for deterministic downstream rendering.
///
/// Returns an empty vec with no diagnostics when `blocks_dir` does not exist
/// — a repo with no `planning/blocks/` directory is not an error, it simply
/// has no initiative index to render (this repo's own history: mev has run
/// without `planning/blocks/` before).
pub fn load_block_records(blocks_dir: &Path) -> (Vec<BlockRecord>, Vec<Diagnostic>) {
    let mut records = Vec::new();
    let mut diagnostics = Vec::new();

    let Ok(entries) = std::fs::read_dir(blocks_dir) else {
        return (records, diagnostics);
    };

    let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();

    for path in paths {
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<BlockRecord>(&text) {
                Ok(record) => records.push(record),
                Err(e) => diagnostics.push(Diagnostic::warning(
                    &path,
                    "W_EMIT_MASTER_PLAN_BAD_RECORD",
                    format!("could not parse block record: {e}"),
                )),
            },
            Err(e) => diagnostics.push(Diagnostic::warning(
                &path,
                "W_EMIT_MASTER_PLAN_BAD_RECORD",
                format!("could not read block record: {e}"),
            )),
        }
    }

    records.sort_by(|a, b| a.id.cmp(&b.id));
    (records, diagnostics)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the initiative index + per-phase block sections for one repo.
///
/// `file.tracks[]` is the phase grouping — each [`okf_core::state::Track`]'s
/// `title` names a phase, and its `blocks[]` are that phase's [`TrackBlock`]s,
/// rendered in authored file order (matching
/// [`crate::brain::emit::render_wave_table`]'s precedent of trusting authored
/// track order rather than re-sorting; ordering *within* a phase across
/// several phases in one document is [`MV.ticket.master-plan-generator`]
/// task 3's concern). `records` supplies only the initiative index above the
/// phases — every rendered per-block field (`title`, `description`,
/// `status`, `wave`, `depends_on`) comes from `state.json`'s `TrackBlock`,
/// the authoritative DAG.
///
/// Returns an empty string when every track has no blocks — a repo with no
/// block records renders nothing, so the caller can skip the splice entirely
/// and leave the file untouched (an acceptance criterion of this block).
pub fn render_master_plan_body(file: &StateFile, records: &[BlockRecord]) -> String {
    if file.tracks.iter().all(|t| t.blocks.is_empty()) {
        return String::new();
    }

    let mut out = String::new();

    let initiatives: BTreeSet<&str> = records
        .iter()
        .filter_map(|r| r.initiative.as_deref())
        .collect();
    if !initiatives.is_empty() {
        out.push_str("### Initiatives\n\n");
        for initiative in &initiatives {
            out.push_str(&format!("- {initiative}\n"));
        }
        out.push('\n');
    }

    for track in &file.tracks {
        if track.blocks.is_empty() {
            continue;
        }
        out.push_str(&format!("### {}\n\n", track.title));
        for block in &track.blocks {
            out.push_str(&render_block_line(block));
            out.push('\n');
        }
        out.push('\n');
    }

    out.trim_end_matches('\n').to_string()
}

/// Render one `TrackBlock` as a single Markdown bullet carrying its title,
/// status, wave, dependency edges, and (when authored) description.
fn render_block_line(block: &TrackBlock) -> String {
    let status = block.status.as_deref().unwrap_or("open");
    let wave = block
        .wave
        .map(|w| w.to_string())
        .unwrap_or_else(|| "\u{2014}".to_string());
    let description = block.description.as_deref().unwrap_or("");

    let deps = if block.depends_on.is_empty() {
        String::new()
    } else {
        let joined = block
            .depends_on
            .iter()
            .map(dep_label)
            .collect::<Vec<_>>()
            .join(", ");
        format!(" — depends on: {joined}")
    };

    if description.is_empty() {
        format!(
            "- **{}** — {} (status: {status}, wave: {wave}){deps}",
            block.id, block.title
        )
    } else {
        format!(
            "- **{}** — {} (status: {status}, wave: {wave}){deps}: {description}",
            block.id, block.title
        )
    }
}

/// Render one dependency edge to its `"{repo}:{id}"` / `"external:{what}"` /
/// `"operator:{slug}"` / `"approval:{slug}"` label, matching
/// [`crate::brain::emit::render_wave_table`]'s existing convention.
fn dep_label(dep: &BlockedBy) -> String {
    match dep {
        BlockedBy::Block(b) => format!("{}:{}", b.repo, b.id),
        BlockedBy::External(e) => format!("external:{}", e.what),
        BlockedBy::Operator(o) => format!("operator:{}", o.slug),
        BlockedBy::Approval(a) => format!("approval:{}", a.slug),
    }
}

// ---------------------------------------------------------------------------
// Planner — sentinel splice into master-plan.md
// ---------------------------------------------------------------------------

/// Plan the [`markers::MASTER_PLAN_BODY`] splice for every non-portfolio repo
/// in `files`, following [`crate::brain::emit::plan_master_plan_tables`]'s
/// exact shape: locate `master-plan.md` beside the repo's `state.json`, read
/// its block records from `planning/blocks/`, render, and splice.
///
/// - A repo with no `planning/blocks/` records (or none with any blocks in
///   `tracks[]`) is left untouched — no action, no diagnostic.
/// - A `master-plan.md` that exists but has no `MASTER_PLAN_BODY` sentinel
///   pair is skipped with a `W_EMIT_NO_SENTINEL` diagnostic, never rewritten
///   wholesale.
/// - A missing `master-plan.md` for a repo that *does* have block records is
///   also a `W_EMIT_NO_SENTINEL` diagnostic, not a hard error — the operator
///   gate on this block (`operator-master-plan-prose-disposition`) governs
///   whether any given repo's file even carries the sentinel yet.
pub fn plan_master_plan_body(files: &[(StateSource, StateFile)]) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        if file.kind == "portfolio" {
            continue;
        }
        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };

        let blocks_dir = planning_dir.join("blocks");
        let (records, record_diags) = load_block_records(&blocks_dir);
        plan.diagnostics.extend(record_diags);

        let generated = render_master_plan_body(file, &records);
        if generated.is_empty() {
            // No block records (or none with tracks[] blocks) for this repo —
            // leave master-plan.md untouched, no diagnostic.
            continue;
        }

        let mp_path = planning_dir.join("master-plan.md");
        let original = match std::fs::read_to_string(&mp_path) {
            Ok(s) => s,
            Err(_) => {
                plan.diagnostics.push(Diagnostic::warning(
                    &mp_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "no master-plan.md beside '{}' state.json; skipping master-plan-body emit",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };

        match splice_generated(&original, markers::MASTER_PLAN_BODY, &generated) {
            Ok(new_content) => {
                if new_content != original {
                    plan.actions.push(EmitAction {
                        path: mp_path,
                        new_content,
                        note: format!("splice master-plan-body for '{}'", src.repo_slug),
                    });
                }
            }
            Err(_) => {
                plan.diagnostics.push(Diagnostic::warning(
                    &mp_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "master-plan.md for '{}' has no <!-- BEGIN generated:master-plan-body --> \
                         sentinels; skipping",
                        src.repo_slug
                    ),
                ));
            }
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::state::{BlockDep, Track};

    fn track_block(id: &str, title: &str, status: Option<&str>, wave: Option<i64>) -> TrackBlock {
        TrackBlock {
            id: id.to_string(),
            title: title.to_string(),
            status: status.map(|s| s.to_string()),
            wave,
            description: Some("a description".to_string()),
            ..Default::default()
        }
    }

    fn state_file(tracks: Vec<Track>) -> StateFile {
        StateFile {
            repo: "demo".to_string(),
            kind: "project".to_string(),
            updated: "2026-08-17".to_string(),
            tracks,
            ..Default::default()
        }
    }

    #[test]
    fn render_empty_tracks_is_empty_string() {
        let file = state_file(vec![]);
        assert_eq!(render_master_plan_body(&file, &[]), "");

        let file = state_file(vec![Track {
            title: "Phase 1".to_string(),
            blocks: vec![],
            extra: Default::default(),
        }]);
        assert_eq!(render_master_plan_body(&file, &[]), "");
    }

    #[test]
    fn render_includes_title_description_status_wave() {
        let file = state_file(vec![Track {
            title: "Phase 1: Foundations".to_string(),
            blocks: vec![track_block("X.1.A", "First block", Some("open"), Some(1))],
            extra: Default::default(),
        }]);

        let out = render_master_plan_body(&file, &[]);
        assert!(out.contains("### Phase 1: Foundations"));
        assert!(out.contains("X.1.A"));
        assert!(out.contains("First block"));
        assert!(out.contains("status: open"));
        assert!(out.contains("wave: 1"));
        assert!(out.contains("a description"));
    }

    #[test]
    fn render_defaults_status_open_and_wave_dash_when_absent() {
        let file = state_file(vec![Track {
            title: "Phase 1".to_string(),
            blocks: vec![track_block("X.1.A", "First block", None, None)],
            extra: Default::default(),
        }]);

        let out = render_master_plan_body(&file, &[]);
        assert!(out.contains("status: open"));
        assert!(out.contains("wave: \u{2014}"));
    }

    #[test]
    fn render_includes_dependency_edges() {
        let mut block = track_block("X.1.B", "Second block", Some("open"), Some(2));
        block.depends_on.push(BlockedBy::Block(BlockDep {
            repo: "demo".to_string(),
            id: "X.1.A".to_string(),
            what: None,
        }));
        let file = state_file(vec![Track {
            title: "Phase 1".to_string(),
            blocks: vec![block],
            extra: Default::default(),
        }]);

        let out = render_master_plan_body(&file, &[]);
        assert!(out.contains("depends on: demo:X.1.A"));
    }

    #[test]
    fn render_groups_multiple_phases_into_separate_sections() {
        let file = state_file(vec![
            Track {
                title: "Phase 1".to_string(),
                blocks: vec![track_block("X.1.A", "First", Some("closed"), Some(1))],
                extra: Default::default(),
            },
            Track {
                title: "Phase 2".to_string(),
                blocks: vec![track_block("X.2.A", "Second", Some("open"), Some(2))],
                extra: Default::default(),
            },
        ]);

        let out = render_master_plan_body(&file, &[]);
        assert!(out.contains("### Phase 1"));
        assert!(out.contains("### Phase 2"));
        let phase1_idx = out.find("### Phase 1").unwrap();
        let phase2_idx = out.find("### Phase 2").unwrap();
        assert!(phase1_idx < phase2_idx);
    }

    #[test]
    fn render_builds_initiative_index_from_records_deduped_and_sorted() {
        let file = state_file(vec![Track {
            title: "Phase 1".to_string(),
            blocks: vec![track_block("X.1.A", "First", Some("open"), Some(1))],
            extra: Default::default(),
        }]);
        let records = vec![
            BlockRecord {
                id: "X.1.A".to_string(),
                initiative: Some("zeta-initiative".to_string()),
            },
            BlockRecord {
                id: "X.1.B".to_string(),
                initiative: Some("alpha-initiative".to_string()),
            },
            BlockRecord {
                id: "X.1.C".to_string(),
                initiative: Some("alpha-initiative".to_string()),
            },
        ];

        let out = render_master_plan_body(&file, &records);
        assert!(out.contains("### Initiatives"));
        let alpha_idx = out.find("alpha-initiative").unwrap();
        let zeta_idx = out.find("zeta-initiative").unwrap();
        assert!(
            alpha_idx < zeta_idx,
            "initiatives should sort alphabetically"
        );
        // Deduped: only one "alpha-initiative" bullet, not two.
        assert_eq!(out.matches("alpha-initiative").count(), 1);
    }

    #[test]
    fn render_omits_initiatives_heading_when_no_records_carry_one() {
        let file = state_file(vec![Track {
            title: "Phase 1".to_string(),
            blocks: vec![track_block("X.1.A", "First", Some("open"), Some(1))],
            extra: Default::default(),
        }]);
        let records = vec![BlockRecord {
            id: "X.1.A".to_string(),
            initiative: None,
        }];

        let out = render_master_plan_body(&file, &records);
        assert!(!out.contains("### Initiatives"));
    }

    #[test]
    fn load_block_records_returns_empty_for_missing_dir() {
        let (records, diags) = load_block_records(Path::new("/nonexistent/path/does/not/exist"));
        assert!(records.is_empty());
        assert!(diags.is_empty());
    }

    #[test]
    fn load_block_records_reads_and_sorts_by_id() {
        let dir = std::env::temp_dir().join(format!("mev-master-plan-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Z.1.json"),
            r#"{"id": "Z.1", "initiative": "zeta"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("A.1.json"),
            r#"{"id": "A.1", "initiative": "alpha"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("not-json.txt"), "ignore me").unwrap();

        let (records, diags) = load_block_records(&dir);
        std::fs::remove_dir_all(&dir).ok();

        assert!(diags.is_empty());
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].id, "A.1");
        assert_eq!(records[1].id, "Z.1");
    }

    #[test]
    fn load_block_records_warns_on_malformed_json_without_aborting() {
        let dir =
            std::env::temp_dir().join(format!("mev-master-plan-test-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("good.json"), r#"{"id": "A.1"}"#).unwrap();
        std::fs::write(dir.join("bad.json"), "{ not valid json").unwrap();

        let (records, diags) = load_block_records(&dir);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(records.len(), 1);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("could not parse"));
    }

    #[test]
    fn plan_master_plan_body_skips_repo_with_no_block_records() {
        // No `blocks/` dir beside a nonexistent state.json path — the planner
        // must not attempt to read/write master-plan.md at all.
        let src = StateSource {
            repo_slug: "demo".to_string(),
            abs_path: std::path::PathBuf::from("/nonexistent/demo/planning/state.json"),
            expected_kind: "project",
        };
        let file = state_file(vec![Track {
            title: "Phase 1".to_string(),
            blocks: vec![],
            extra: Default::default(),
        }]);

        let plan = plan_master_plan_body(&[(src, file)]);
        assert!(plan.actions.is_empty());
        assert!(plan.diagnostics.is_empty());
    }
}
