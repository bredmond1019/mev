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

use thiserror::Error;

use crate::brain::state::{BlockedBy, StateFile, StateGraph, StateSource};

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
