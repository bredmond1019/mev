//! Real-corpus segmentation gate — `MV.13.A` Task 2.
//!
//! The spec is explicit that segmentation must be built and tested against the **real**
//! multi-repo lane files, not only synthetic fixtures written to the documented format —
//! `planning/close-the-loop/lane-substrate.txt` is the worst case on record, spanning
//! seven repos (base-template, mev, okf-core, claude-code-rs, engine-rs, bastion,
//! orchestrator), all contiguous by repo. This pins the exact segment count and
//! boundaries against whatever is actually on disk today.
//!
//! ## Portability
//!
//! Like `tests/fleet_regression.rs`, this walks up from the crate root looking for
//! `brain.toml` (the real `agentic-portfolio` HQ root). If it isn't found — `mev`
//! checked out standalone, outside the fleet — the test prints why and returns rather
//! than failing; the live-corpus guarantee only means something inside the fleet.

use std::path::PathBuf;

use mev::brain::config::{find_brain_root, load_brain_config};
use mev::brain::lane_segments::{
    build_owner_index, discover_lane_files, resolve_owner, segment_lane_blocks,
};
use mev::brain::state::{StateFile, StateSource, discover_state_files, load_state};

#[test]
fn close_the_loop_lane_substrate_segments_into_seven_contiguous_repo_runs() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = match find_brain_root(&manifest_dir) {
        Ok(root) => root,
        Err(e) => {
            eprintln!(
                "lane_segments_fleet: skipping — no brain.toml found walking up from {}: {e}",
                manifest_dir.display()
            );
            return;
        }
    };

    let lane_path = root
        .join("planning")
        .join("close-the-loop")
        .join("lane-substrate.txt");
    let content = match std::fs::read_to_string(&lane_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "lane_segments_fleet: skipping — could not read {}: {e} (this fixture may have \
                 been archived since this test was written; re-point it at whatever lane file \
                 is the current worst-case multi-repo fixture)",
                lane_path.display()
            );
            return;
        }
    };

    let config = load_brain_config(&root.join("brain.toml"))
        .unwrap_or_else(|e| panic!("failed to load {}: {e}", root.join("brain.toml").display()));
    let (sources, _discovery_diags) = discover_state_files(&root, &config);
    assert!(
        sources.len() >= 5,
        "lane_segments_fleet: only found {} state.json sources under {} — this looks like a \
         broken/partial checkout, not the real fleet; refusing to run a vacuous gate",
        sources.len(),
        root.display()
    );

    let mut files: Vec<(StateSource, StateFile)> = Vec::new();
    for src in sources {
        match load_state(&src.abs_path) {
            Ok(file) => files.push((src, file)),
            Err(e) => eprintln!(
                "lane_segments_fleet: skipping unreadable state file {}: {e}",
                src.abs_path.display()
            ),
        }
    }

    let index = build_owner_index(&files);
    let blocks = mev::brain::lane_segments::parse_lane_blocks(&content);
    let segments = segment_lane_blocks(&blocks, |id| {
        resolve_owner(&index, id).map(|r| r.to_string())
    });

    // Hand-checked against the live file's `# C<n> · <repo>` section headers: nine
    // blocks across seven repos, each repo's blocks contiguous, in this order.
    let expected_repos = [
        "base-template",
        "mev",
        "okf-core",
        "claude-code-rs",
        "engine-rs",
        "bastion",
        "orchestrator",
    ];
    let got_repos: Vec<&str> = segments.iter().map(|s| s.repo.as_str()).collect();

    // Every block in the live file must resolve — if the fleet's state.json content
    // has drifted (a ticket renamed or closed out from under this fixture), that's a
    // real signal this test should surface, not silently swallow.
    let resolved_block_count: usize = segments.iter().map(|s| s.blocks.len()).sum();
    assert_eq!(
        resolved_block_count,
        blocks.len(),
        "lane_segments_fleet: {} of {} blocks in {} did not resolve to a repo — segments: \
         {segments:?}",
        blocks.len() - resolved_block_count,
        blocks.len(),
        lane_path.display()
    );

    assert_eq!(
        got_repos, expected_repos,
        "lane_segments_fleet: expected 7 contiguous segments in this repo order, got {got_repos:?}"
    );
    for (i, seg) in segments.iter().enumerate() {
        assert_eq!(
            seg.segment, i,
            "segment index must equal its position in the lane"
        );
    }
}

/// Exact, known-real `E_LANE_DIRECTIVE_MALFORMED` cases pre-existing in the live fleet as of
/// `MV.ticket.lane-file-structured-directives` close-out (2026-08-17) — a `# BUDGET:` line
/// that genuinely never states `HEAVY`/`LIGHT` at all (unlike the 27 other pre-existing
/// `# BUDGET:` lines, which state a level and then explain it in prose — those are handled
/// by [`LaneBudget::parse`]'s tolerant grammar, not this list). This is a real content gap in
/// those three lane files, not a parser false positive; the fix belongs in the lane file
/// (state the level explicitly), an editorial call outside this ticket's scope. Listed
/// explicitly rather than just asserting a bare count so a *new*, different diagnostic still
/// fails this test loudly instead of hiding under a stale tolerance number.
const KNOWN_PRE_EXISTING_MALFORMED_BUDGET_FILES: &[&str] = &[
    "planning/close-the-loop/lane-bastion-web.txt",
    "planning/close-the-loop/lane-learn-ai.txt",
    "planning/demand-ready/lane-bastion-web.txt",
];

/// `MV.ticket.lane-file-structured-directives` close-out gate: the structured-directive
/// parser (`parse_lane_directives`, wired through `discover_lane_files`) must produce no
/// `E_LANE_DIRECTIVE_*` diagnostics against the real fleet beyond the known pre-existing
/// content gaps above. Synthetic fixtures alone missed this — every real lane file also
/// carries pre-existing header conventions (`# ORIGIN:`, `# ROADMAP:`, `# LOG:`, free-prose
/// `# BUDGET:` lines, and others) that a too-broad "unrecognised directive key" heuristic or
/// an over-strict `BUDGET` grammar can false-positive on, and a close-out
/// `mev emit-state --write` run against the live corpus is what actually caught it (200
/// errors before the allowlist + tolerant `BUDGET` parse existed). Skips gracefully outside
/// the fleet checkout, like its sibling test above.
#[test]
fn structured_directives_produce_only_known_diagnostics_against_the_live_fleet() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = match find_brain_root(&manifest_dir) {
        Ok(root) => root,
        Err(e) => {
            eprintln!(
                "structured_directives_produce_only_known_diagnostics_against_the_live_fleet: \
                 skipping — no brain.toml found walking up from {}: {e}",
                manifest_dir.display()
            );
            return;
        }
    };

    let (files, diags) = discover_lane_files(&root);
    assert!(
        files.len() >= 10,
        "structured_directives_produce_only_known_diagnostics_against_the_live_fleet: only \
         found {} lane files under {} — this looks like a broken/partial checkout, not the \
         real fleet; refusing to run a vacuous gate",
        files.len(),
        root.display()
    );

    let unexpected: Vec<_> = diags
        .iter()
        .filter(|d| d.locator.starts_with("E_LANE_DIRECTIVE_"))
        .filter(|d| {
            let rel = d.file.strip_prefix(&root).unwrap_or(&d.file);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            !(d.locator == "E_LANE_DIRECTIVE_MALFORMED"
                && KNOWN_PRE_EXISTING_MALFORMED_BUDGET_FILES.contains(&rel_str.as_str()))
        })
        .collect();
    assert!(
        unexpected.is_empty(),
        "structured_directives_produce_only_known_diagnostics_against_the_live_fleet: {} \
         unexpected E_LANE_DIRECTIVE_* diagnostic(s) against the live fleet — a real \
         lane-file convention is colliding with the directive grammar: {unexpected:#?}",
        unexpected.len()
    );
}
