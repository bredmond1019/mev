//! Emit module — derived-view generator for `mev emit-state`.
//!
//! This module is the **single derivation engine** for every generated view the
//! v2 state schema declares.  The public surface for Task 2:
//!
//! - [`EmitError`] — error type for sentinel-related failures.
//! - [`wave_order`] — all block keys (`"repo:id"`) sorted by `wave` ascending.
//! - [`render_wave_table`] — Markdown table of a repo's blocks in wave order.
//! - [`splice_generated`] — idempotent sentinel-splice into an existing Markdown
//!   document.
//!
//! Tasks 3 and 4 extend this file with the planners (`EmitAction`, `EmitPlan`,
//! `plan_state_json`, `plan_master_plan_tables`, `apply_plan`) and the library
//! entry point (`emit_state`).

use std::collections::HashMap;

use thiserror::Error;

use crate::brain::config::BrainConfig;
use crate::brain::state::{
    Block, BlockedBy, Focus, StateFile, StateGraph, StateSource, derive_brain_focus,
    derive_cross_repo, derive_focus, derive_rollup, tier_scope_for,
};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the emit module.
#[derive(Debug, Error)]
pub enum EmitError {
    /// The `<!-- BEGIN generated:{marker} -->` sentinel is missing from the
    /// document, or the `END` sentinel does not follow the `BEGIN` sentinel.
    #[error("missing or unbalanced sentinels for marker '{marker}' in document")]
    MissingSentinel { marker: String },
}

// ---------------------------------------------------------------------------
// wave_order — full roadmap ordering (all blocks, not only ready/open)
// ---------------------------------------------------------------------------

/// Return all block keys (`"repo:id"`) across `files`, sorted by `wave` ascending
/// (`None` last), with ties broken by track iteration order then block array index.
///
/// This is the full-roadmap sibling of `ready_order` (which filters to ready/open
/// blocks only).  `wave_order` includes every block regardless of status so that
/// [`render_wave_table`] can produce a complete roadmap table.
///
/// The `graph` parameter is accepted for API symmetry with `ready_order` and
/// future forward-compat (e.g. cycle-aware ordering); it is not used today.
pub fn wave_order(_graph: &StateGraph, files: &[(StateSource, StateFile)]) -> Vec<String> {
    // Collect (wave_sort_key, iteration_index, "repo:id") for every block.
    let mut entries: Vec<(i64, usize, String)> = Vec::new();
    let mut iteration_index: usize = 0;

    for (src, file) in files {
        for track in &file.tracks {
            for block in &track.blocks {
                let wave_key = block.wave.unwrap_or(i64::MAX);
                let key = format!("{}:{}", src.repo_slug, block.id);
                entries.push((wave_key, iteration_index, key));
                iteration_index += 1;
            }
        }
    }

    // Primary sort: wave asc (None → i64::MAX → last). Tiebreak: iteration order (stable).
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    entries.into_iter().map(|(_, _, key)| key).collect()
}

// ---------------------------------------------------------------------------
// render_wave_table — Markdown table for one repo's blocks
// ---------------------------------------------------------------------------

/// Render a Markdown table of `repo_slug`'s blocks in wave order.
///
/// Columns: `Wave | Block | Title | Status | Depends on`
///
/// - `Status` shows the **derived** state: an open block with at least one unmet
///   `depends_on` renders as `blocked`; otherwise the block's authored status is
///   used (defaulting to `open` when absent).
/// - `Depends on` lists `depends_on` targets as `repo:id` (for
///   `{type:"block"}`) or `external:<what>` (for `{type:"external"}`).
/// - `Wave` column shows the authored wave number, or `—` when absent.
///
/// The table is rendered without a trailing newline; callers that embed it inside
/// a document are responsible for any required surrounding blank lines.
pub fn render_wave_table(repo_slug: &str, file: &StateFile, graph: &StateGraph) -> String {
    use std::collections::HashMap;

    // Build a status map across all blocks in this file: id → authored status.
    // We need it to compute derived "blocked" status (unmet dep check).
    let mut all_status: HashMap<String, Option<String>> = HashMap::new();
    for track in &file.tracks {
        for block in &track.blocks {
            all_status.insert(block.id.clone(), block.status.clone());
        }
    }

    // We also need a cross-file status map to resolve cross-repo deps.
    // Since this function only receives `file` (one repo), we can only check
    // same-repo deps inline.  Cross-repo deps are always treated as unmet for
    // the purpose of the `blocked` derived status — this is safe/conservative.
    //
    // Build the ordered list of (wave_key, iteration_idx, block_id) for this repo.
    let mut ordered: Vec<(i64, usize, &str)> = Vec::new();
    let mut idx: usize = 0;
    for track in &file.tracks {
        for block in &track.blocks {
            ordered.push((block.wave.unwrap_or(i64::MAX), idx, block.id.as_str()));
            idx += 1;
        }
    }
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    // Build a lookup: block_id → &TrackBlock for cell rendering.
    let mut block_map: HashMap<&str, &crate::brain::state::TrackBlock> = HashMap::new();
    for track in &file.tracks {
        for block in &track.blocks {
            block_map.insert(block.id.as_str(), block);
        }
    }

    // We need the graph to resolve cross-repo dep statuses for the "blocked" derivation.
    // Use wave_order across the whole graph to build a global status map.
    // Since `graph` carries the adjacency / dep data, we extract from `wave_order` context.
    // For simplicity: build a minimal global key→status from what we have in `graph`.
    // StateGraph exposes `blocks` and `deps` but not a status map.  We cannot reconstruct
    // per-repo statuses from `StateGraph` alone here; instead we replicate the same
    // "treat cross-repo dep as unmet unless we can prove it closed" conservative rule.
    //
    // The graph is accepted for forward-compat; we use it only for the _ suppression.
    let _ = graph;

    // Header
    let header = "| Wave | Block | Title | Status | Depends on |";
    let sep = "|------|-------|-------|--------|------------|";

    let mut rows: Vec<String> = Vec::new();
    rows.push(header.to_string());
    rows.push(sep.to_string());

    for (wave_key, _, block_id) in &ordered {
        let Some(block) = block_map.get(block_id) else {
            continue;
        };

        // Wave column value
        let wave_col = if *wave_key == i64::MAX {
            "\u{2014}".to_string() // em-dash
        } else {
            wave_key.to_string()
        };

        // Derived status: check if this block is "blocked" (open + unmet dep).
        let authored_status = block.status.as_deref().unwrap_or("open");
        let derived_status = if authored_status == "open" {
            // Check for unmet deps — conservative: external deps always unmet;
            // block deps only resolved for same-repo.
            let has_unmet = block.depends_on.iter().any(|dep| match dep {
                BlockedBy::External { .. } => true,
                BlockedBy::Block { repo, id, .. } => {
                    if repo == repo_slug {
                        // Same-repo: check authored status.
                        all_status.get(id.as_str()).and_then(|s| s.as_deref()) != Some("closed")
                    } else {
                        // Cross-repo: treat as unmet (conservative).
                        true
                    }
                }
            });
            if has_unmet { "blocked" } else { "open" }
        } else {
            authored_status
        };

        // Depends-on column
        let deps_col = if block.depends_on.is_empty() {
            String::new()
        } else {
            block
                .depends_on
                .iter()
                .map(|dep| match dep {
                    BlockedBy::Block { repo, id, .. } => format!("{repo}:{id}"),
                    BlockedBy::External { what } => format!("external:{what}"),
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        rows.push(format!(
            "| {wave_col} | {block_id} | {} | {derived_status} | {deps_col} |",
            block.title
        ));
    }

    rows.join("\n")
}

// ---------------------------------------------------------------------------
// splice_generated — sentinel-aware idempotent splice
// ---------------------------------------------------------------------------

/// Replace the text between `<!-- BEGIN generated:{marker} -->` and
/// `<!-- END generated:{marker} -->` with `generated`.
///
/// Every line outside the sentinels is preserved verbatim.  The splice is
/// **idempotent**: re-splicing the result yields identical output.
///
/// Returns [`EmitError::MissingSentinel`] when:
/// - the `BEGIN` sentinel is absent from `original`, or
/// - the `BEGIN` sentinel appears but the `END` sentinel does not follow it.
pub fn splice_generated(
    original: &str,
    marker: &str,
    generated: &str,
) -> Result<String, EmitError> {
    let begin_tag = format!("<!-- BEGIN generated:{marker} -->");
    let end_tag = format!("<!-- END generated:{marker} -->");

    // Find the BEGIN sentinel line index.
    let lines: Vec<&str> = original.lines().collect();
    let begin_idx = lines.iter().position(|l| l.trim() == begin_tag.as_str());
    let Some(begin_idx) = begin_idx else {
        return Err(EmitError::MissingSentinel {
            marker: marker.to_string(),
        });
    };

    // Find the END sentinel after BEGIN.
    let end_idx = lines[begin_idx + 1..]
        .iter()
        .position(|l| l.trim() == end_tag.as_str())
        .map(|rel| begin_idx + 1 + rel);
    let Some(end_idx) = end_idx else {
        return Err(EmitError::MissingSentinel {
            marker: marker.to_string(),
        });
    };

    // Reconstruct: everything up to and including BEGIN, then generated, then END onwards.
    let before: Vec<&str> = lines[..=begin_idx].to_vec();
    let after: Vec<&str> = lines[end_idx..].to_vec();

    let mut result_parts: Vec<&str> = before;
    // Push generated lines (may be empty).
    let generated_lines: Vec<&str> = if generated.is_empty() {
        vec![]
    } else {
        generated.lines().collect()
    };
    result_parts.extend(generated_lines);
    result_parts.extend(after);

    // Preserve original trailing newline behaviour: if original ended with a newline, add one.
    let trailing_newline = original.ends_with('\n');
    let mut result = result_parts.join("\n");
    if trailing_newline {
        result.push('\n');
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Task 3 — EmitAction, EmitPlan, planners, apply_plan
// ---------------------------------------------------------------------------

/// A single proposed file write produced by a planner.
///
/// Pure data — no IO is performed until [`apply_plan`] is called.
#[derive(Debug, Clone)]
pub struct EmitAction {
    /// Absolute path of the file to (over)write.
    pub path: std::path::PathBuf,
    /// The complete proposed new contents of the file.
    pub new_content: String,
    /// Human note describing what changed (for the dry-run/write diagnostic message).
    pub note: String,
}

/// The output of a planner: the proposed writes plus any diagnostics raised while planning
/// (e.g. a missing-sentinel warning).
#[derive(Debug, Default)]
pub struct EmitPlan {
    pub actions: Vec<EmitAction>,
    pub diagnostics: Vec<crate::Diagnostic>,
}

impl EmitPlan {
    /// Merge another plan's actions and diagnostics into this one.
    pub fn extend(&mut self, other: EmitPlan) {
        self.actions.extend(other.actions);
        self.diagnostics.extend(other.diagnostics);
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Map every `tracks[].blocks[]` id in one file to its `(title, authored status)`.
fn id_index(file: &StateFile) -> HashMap<String, (String, Option<String>)> {
    let mut map = HashMap::new();
    for track in &file.tracks {
        for block in &track.blocks {
            map.insert(
                block.id.clone(),
                (block.title.clone(), block.status.clone()),
            );
        }
    }
    map
}

/// Call [`derive_focus`] and rehydrate the returned id lists into a [`Focus`] struct,
/// filling titles from this file's `tracks[]`.
fn derived_focus_for(
    src: &StateSource,
    file: &StateFile,
    graph: &StateGraph,
    files: &[(StateSource, StateFile)],
) -> Focus {
    let idx = id_index(file);
    let d = derive_focus(src, file, graph, files);
    let title_of = |id: &str| idx.get(id).map(|(t, _)| t.clone()).unwrap_or_default();

    let now = d
        .now
        .iter()
        .map(|id| Block {
            id: id.clone(),
            title: title_of(id),
            status: Some("in_progress".to_string()),
            note: None,
            repo: None,
            blocked_by: Vec::new(),
        })
        .collect();

    let next = d
        .next
        .iter()
        .map(|id| Block {
            id: id.clone(),
            title: title_of(id),
            status: None,
            note: None,
            repo: None,
            blocked_by: Vec::new(),
        })
        .collect();

    let blocked = d
        .blocked
        .iter()
        .map(|(id, unmet)| Block {
            id: id.clone(),
            title: title_of(id),
            status: None,
            note: None,
            repo: None,
            blocked_by: unmet.clone(),
        })
        .collect();

    Focus { now, next, blocked }
}

// ---------------------------------------------------------------------------
// plan_state_json
// ---------------------------------------------------------------------------

/// Plan the derived-section rewrites for every loaded `state.json`.
///
/// - Leaf (`kind == "project"`): regenerate `focus` from [`derive_focus`].
/// - Brain (`kind == "brain"`): regenerate `repos[]` (tier-scoped, non-destructive —
///   see [`tier_scope_for`] / [`derive_rollup`]), `cross_repo[]`, and `focus`
///   (the repo-tagged union of in-scope children's derived focus — see
///   [`derive_brain_focus`]).
///
/// An [`EmitAction`] is added only when the re-serialised derived file differs from
/// the re-serialised original (fixed-point property — no action when already correct).
pub fn plan_state_json(
    files: &[(StateSource, StateFile)],
    graph: &StateGraph,
    config: &BrainConfig,
) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        let mut derived = file.clone();

        match file.kind.as_str() {
            "project" => {
                derived.focus = derived_focus_for(src, file, graph, files);
            }
            "brain" => {
                let scope = tier_scope_for(file, config);
                derived.repos = derive_rollup(&scope, config, &file.repos, graph, files);
                derived.cross_repo = derive_cross_repo(files);
                derived.focus = derive_brain_focus(&scope, config, graph, files);
            }
            _ => continue, // unknown kind already flagged by check_schema
        }

        // Fixed-point check: compare canonical serialisations (both newline-free).
        let original = match serde_json::to_string_pretty(file) {
            Ok(s) => s,
            Err(e) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &src.abs_path,
                    "W_EMIT_SERIALIZE_FAILED",
                    format!(
                        "could not serialize original state for '{}': {e}",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };
        let new_serialised = match serde_json::to_string_pretty(&derived) {
            Ok(s) => s,
            Err(e) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &src.abs_path,
                    "W_EMIT_SERIALIZE_FAILED",
                    format!(
                        "could not serialize derived state for '{}': {e}",
                        src.repo_slug
                    ),
                ));
                continue;
            }
        };

        if new_serialised != original {
            let note = if file.kind == "project" {
                format!("regenerate focus for '{}'", src.repo_slug)
            } else {
                format!("regenerate repos[]/cross_repo[] for '{}'", src.repo_slug)
            };
            plan.actions.push(EmitAction {
                path: src.abs_path.clone(),
                // Add a trailing newline so the file is a POSIX text file.
                new_content: format!("{new_serialised}\n"),
                note,
            });
        }
    }

    plan
}

// ---------------------------------------------------------------------------
// plan_master_plan_tables
// ---------------------------------------------------------------------------

/// Plan the wave-table splice into each state file's sibling `master-plan.md`.
///
/// For each loaded state file, locates `<state.json parent>/master-plan.md`.  If
/// it exists and carries the `wave-table` sentinels, splices the rendered table
/// and adds an [`EmitAction`].  A missing file or missing sentinels produces a
/// [`W_EMIT_NO_SENTINEL`] warning diagnostic — never invents sentinels into
/// arbitrary prose.
///
/// `portfolio`-kind files are skipped entirely: they are terminal repos
/// (published to GitHub, no further planning state) and never carry a
/// `master-plan.md`, so flagging one would just be noise.
pub fn plan_master_plan_tables(files: &[(StateSource, StateFile)], graph: &StateGraph) -> EmitPlan {
    let mut plan = EmitPlan::default();

    for (src, file) in files {
        if file.kind == "portfolio" {
            continue;
        }
        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };
        let mp_path = planning_dir.join("master-plan.md");

        if !mp_path.exists() {
            plan.diagnostics.push(crate::Diagnostic::warning(
                &mp_path,
                "W_EMIT_NO_SENTINEL",
                format!(
                    "no master-plan.md beside '{}' state.json; skipping table emit",
                    src.repo_slug
                ),
            ));
            continue;
        }

        let original = match std::fs::read_to_string(&mp_path) {
            Ok(s) => s,
            Err(e) => {
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &mp_path,
                    "W_EMIT_NO_SENTINEL",
                    format!("could not read master-plan.md for '{}': {e}", src.repo_slug),
                ));
                continue;
            }
        };

        let table = render_wave_table(&src.repo_slug, file, graph);

        match splice_generated(&original, "wave-table", &table) {
            Ok(new_content) => {
                if new_content != original {
                    plan.actions.push(EmitAction {
                        path: mp_path,
                        new_content,
                        note: format!("splice wave-table for '{}'", src.repo_slug),
                    });
                }
            }
            Err(_) => {
                // Missing or unbalanced sentinels → warning, no write.
                plan.diagnostics.push(crate::Diagnostic::warning(
                    &mp_path,
                    "W_EMIT_NO_SENTINEL",
                    format!(
                        "master-plan.md for '{}' has no <!-- BEGIN generated:wave-table --> \
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
// apply_plan
// ---------------------------------------------------------------------------

/// Execute a plan.
///
/// When `write` is `true`, writes each action's `new_content` to its `path` and
/// emits a `I_EMIT_WROTE` (Warning severity) diagnostic per file.  When `false`
/// (dry-run), writes nothing and emits a `W_EMIT_DRY_RUN` diagnostic per planned
/// action.  Always passes through the plan's own diagnostics.
///
/// `I_EMIT_WROTE` and `W_EMIT_DRY_RUN` use Warning severity (no info level
/// exists in [`crate::Diagnostic`]) so they surface in the reporter without
/// failing the exit code.  Only `E_EMIT_WRITE_FAILED` is Error-severity (a real
/// IO failure that should abort the run).
pub fn apply_plan(plan: &EmitPlan, write: bool) -> Vec<crate::Diagnostic> {
    let mut diags = plan.diagnostics.clone();

    for action in &plan.actions {
        if write {
            match std::fs::write(&action.path, action.new_content.as_bytes()) {
                Ok(()) => diags.push(crate::Diagnostic::warning(
                    &action.path,
                    "I_EMIT_WROTE",
                    format!("wrote: {}", action.note),
                )),
                Err(e) => diags.push(crate::Diagnostic::error(
                    &action.path,
                    "E_EMIT_WRITE_FAILED",
                    format!("failed to write {}: {e}", action.path.display()),
                )),
            }
        } else {
            diags.push(crate::Diagnostic::warning(
                &action.path,
                "W_EMIT_DRY_RUN",
                format!("would write (dry-run): {}", action.note),
            ));
        }
    }

    diags
}
