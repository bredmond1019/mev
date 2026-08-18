//! Fixture-backed safety tests for `brain::master_plan::plan_master_plan_body`
//! (`MV.ticket.master-plan-generator`, task 2).
//!
//! Unlike `src/brain/master_plan.rs`'s inline unit tests (which build
//! `StateFile`/`Track`/`TrackBlock` in Rust and exercise the render function
//! directly), these tests run the full planner — `load_state` +
//! `plan_master_plan_body` — against real `master-plan.md`/`state.json` pairs
//! committed under `tests/fixtures/master-plan/`, matching the fixture-tree
//! precedent set by `tests/blog_validate.rs` (`tests/fixtures/blog/`). Each
//! fixture directory stands in for one repo's `planning/` directory: the
//! `state.json` inside it is what `abs_path` points at, and `master-plan.md`
//! (plus an optional `blocks/`) sit beside it, exactly as the planner expects.
//!
//! Fixtures:
//!   - `prose-preserved/` — authored prose above *and* below the sentinel
//!     pair; asserts the region outside the sentinels is byte-identical
//!     before and after the splice.
//!   - `no-sentinel/` — a `master-plan.md` with no sentinel pair at all;
//!     asserts the file is skipped with a `W_EMIT_NO_SENTINEL` diagnostic and
//!     never rewritten.
//!   - `no-block-records/` — a repo whose `state.json` has no blocks in any
//!     track (and no `planning/blocks/` dir); asserts the file is left
//!     completely untouched, no action and no diagnostic.

use mev::brain::master_plan::plan_master_plan_body;
use mev::brain::state::{StateSource, load_state};
use std::path::{Path, PathBuf};

/// Root of the fixture tree checked into `tests/fixtures/master-plan/`.
fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/master-plan")
}

/// Load `<fixture_root>/<name>/state.json` into a `(StateSource, StateFile)`
/// pair, exactly as the real `emit-state` pipeline discovers one repo.
fn load_fixture(name: &str) -> (StateSource, mev::brain::state::StateFile) {
    let dir = fixture_root().join(name);
    let state_path = dir.join("state.json");
    let file = load_state(&state_path)
        .unwrap_or_else(|e| panic!("fixture '{name}' state.json failed to load: {e:?}"));
    let src = StateSource {
        repo_slug: file.repo.clone(),
        abs_path: state_path,
        expected_kind: "project",
    };
    (src, file)
}

// ---------------------------------------------------------------------------
// prose-preserved — byte-equality outside the sentinels
// ---------------------------------------------------------------------------

#[test]
fn prose_preserved_fixture_keeps_outside_sentinel_bytes_identical() {
    const BEGIN: &str = "<!-- BEGIN generated:master-plan-body -->";
    const END: &str = "<!-- END generated:master-plan-body -->";

    let (src, file) = load_fixture("prose-preserved");
    let mp_path = fixture_root()
        .join("prose-preserved")
        .join("master-plan.md");
    let original =
        std::fs::read_to_string(&mp_path).expect("fixture master-plan.md must be readable");

    let plan = plan_master_plan_body(&[(src, file)]);

    assert_eq!(
        plan.actions.len(),
        1,
        "expected exactly one splice action; got {:?}",
        plan.actions
    );
    let action = &plan.actions[0];
    assert_eq!(action.path, mp_path);

    // Region before (and including) the BEGIN sentinel must be byte-identical.
    let orig_begin = original
        .find(BEGIN)
        .expect("fixture must carry BEGIN sentinel");
    let new_begin = action
        .new_content
        .find(BEGIN)
        .expect("rendered output must carry BEGIN sentinel");
    assert_eq!(
        &original[..orig_begin + BEGIN.len()],
        &action.new_content[..new_begin + BEGIN.len()],
        "content up to and including the BEGIN sentinel must be byte-identical"
    );

    // Region from the END sentinel onward must be byte-identical.
    let orig_end = original.find(END).expect("fixture must carry END sentinel");
    let new_end = action
        .new_content
        .find(END)
        .expect("rendered output must carry END sentinel");
    assert_eq!(
        &original[orig_end..],
        &action.new_content[new_end..],
        "content from the END sentinel onward must be byte-identical"
    );

    // Sanity: the authored prose markers are actually present (i.e. this
    // assertion isn't vacuously true because the fixture has no prose).
    assert!(original.contains("## Preface"));
    assert!(original.contains("## Appendix"));
    assert!(action.new_content.contains("## Preface"));
    assert!(action.new_content.contains("## Appendix"));

    // The stale placeholder inside the old generated region must be gone,
    // replaced by the real render.
    assert!(!action.new_content.contains("stale placeholder"));
    assert!(action.new_content.contains("FX.1.A"));
    assert!(action.new_content.contains("fixture-initiative"));

    // The fixture file on disk itself is untouched by merely planning.
    let reread = std::fs::read_to_string(&mp_path).unwrap();
    assert_eq!(reread, original, "planning must never write to disk itself");
}

// ---------------------------------------------------------------------------
// no-sentinel — skipped with a diagnostic, never rewritten
// ---------------------------------------------------------------------------

#[test]
fn no_sentinel_fixture_is_skipped_with_diagnostic_and_never_rewritten() {
    let (src, file) = load_fixture("no-sentinel");
    let mp_path = fixture_root().join("no-sentinel").join("master-plan.md");
    let original =
        std::fs::read_to_string(&mp_path).expect("fixture master-plan.md must be readable");

    let plan = plan_master_plan_body(&[(src, file)]);

    assert!(
        plan.actions.is_empty(),
        "a master-plan.md with no sentinel pair must never be rewritten; got actions: {:?}",
        plan.actions
    );
    assert!(
        plan.diagnostics
            .iter()
            .any(|d| d.locator == "W_EMIT_NO_SENTINEL"),
        "expected a W_EMIT_NO_SENTINEL diagnostic; got: {:?}",
        plan.diagnostics
    );

    let reread = std::fs::read_to_string(&mp_path).unwrap();
    assert_eq!(
        reread, original,
        "fixture file on disk must be byte-identical after planning"
    );
}

// ---------------------------------------------------------------------------
// no-block-records — untouched, no action, no diagnostic
// ---------------------------------------------------------------------------

#[test]
fn no_block_records_fixture_is_left_completely_untouched() {
    let (src, file) = load_fixture("no-block-records");
    let mp_path = fixture_root()
        .join("no-block-records")
        .join("master-plan.md");
    let original =
        std::fs::read_to_string(&mp_path).expect("fixture master-plan.md must be readable");

    let plan = plan_master_plan_body(&[(src, file)]);

    assert!(
        plan.actions.is_empty(),
        "a repo with no blocks in any track must never be written; got actions: {:?}",
        plan.actions
    );
    assert!(
        plan.diagnostics.is_empty(),
        "a repo with no blocks in any track must produce no diagnostic either; got: {:?}",
        plan.diagnostics
    );

    let reread = std::fs::read_to_string(&mp_path).unwrap();
    assert_eq!(
        reread, original,
        "fixture file on disk must be byte-identical after planning"
    );
}
