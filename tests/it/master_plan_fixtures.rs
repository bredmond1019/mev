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
//!   - `ordering/` (`MV.ticket.master-plan-generator`, task 3) — three
//!     phases with interleaved wave numbers; asserts the render orders
//!     blocks by phase (authored `tracks[]` order) then by wave within a
//!     phase (authored block order, which this fixture pins to ascending
//!     wave), and that successive runs over the same input produce
//!     byte-identical output.

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

// ---------------------------------------------------------------------------
// ordering — deterministic ordering by phase then wave, stable across runs
// ---------------------------------------------------------------------------

#[test]
fn ordering_fixture_renders_blocks_by_phase_then_ascending_wave() {
    let (src, file) = load_fixture("ordering");

    let plan = plan_master_plan_body(&[(src, file)]);
    assert_eq!(
        plan.actions.len(),
        1,
        "expected exactly one splice action; got {:?}",
        plan.actions
    );
    let rendered = &plan.actions[0].new_content;

    // Every block, across all three phases, must appear in this exact
    // sequence: authored `tracks[]` (phase) order, and within each phase,
    // authored block order — which this fixture pins to ascending wave.
    let expected_order = ["ORD.1.A", "ORD.1.B", "ORD.2.A", "ORD.2.B", "ORD.3.A"];
    let mut positions = Vec::with_capacity(expected_order.len());
    for id in expected_order {
        let pos = rendered
            .find(id)
            .unwrap_or_else(|| panic!("rendered output must contain block '{id}'"));
        positions.push((id, pos));
    }
    for window in positions.windows(2) {
        let (prev_id, prev_pos) = window[0];
        let (next_id, next_pos) = window[1];
        assert!(
            prev_pos < next_pos,
            "expected '{prev_id}' to render before '{next_id}' (phase-then-wave order); \
             got positions {prev_pos} >= {next_pos} in:\n{rendered}"
        );
    }

    // The phase headings themselves must also appear in authored order.
    let phase1 = rendered.find("### Phase 1: Foundations").unwrap();
    let phase2 = rendered.find("### Phase 2: Build").unwrap();
    let phase3 = rendered.find("### Phase 3: Ship").unwrap();
    assert!(phase1 < phase2 && phase2 < phase3);
}

#[test]
fn ordering_fixture_is_deterministic_across_repeated_runs() {
    let (src1, file1) = load_fixture("ordering");
    let (src2, file2) = load_fixture("ordering");

    let plan1 = plan_master_plan_body(&[(src1, file1)]);
    let plan2 = plan_master_plan_body(&[(src2, file2)]);

    assert_eq!(plan1.actions.len(), 1);
    assert_eq!(plan2.actions.len(), 1);
    assert_eq!(
        plan1.actions[0].new_content, plan2.actions[0].new_content,
        "rendering the same fixture twice must produce byte-identical output"
    );

    // A third, independent render (re-invoking the render function directly
    // rather than the full plan/splice path) must also match, pinning
    // determinism at the render layer, not just the splice layer.
    let (_, file3) = load_fixture("ordering");
    let out_a = mev::brain::master_plan::render_master_plan_body(&file3, &[]);
    let (_, file4) = load_fixture("ordering");
    let out_b = mev::brain::master_plan::render_master_plan_body(&file4, &[]);
    assert_eq!(out_a, out_b);
}

// ---------------------------------------------------------------------------
// emit_state wiring — the generator is actually reached by the real pipeline
// ---------------------------------------------------------------------------

/// Every other test in this file calls `plan_master_plan_body` directly, so all
/// of them would still pass if the `emit_state` call site in `src/lib.rs` were
/// deleted. This one closes that gap: it builds a minimal brain root on disk,
/// runs the real `mev::emit_state(root, write = true, scope = None)` entry
/// point, and asserts the `master-plan-body` region of the leaf repo's
/// `master-plan.md` was actually filled in — i.e. the generator is wired into
/// the pipeline, not merely reachable in a unit test.
#[test]
fn emit_state_splices_the_master_plan_body_region_end_to_end() {
    use std::fs;

    let root = mev::testsupport::unique_temp_dir("mev-master-plan-body-e2e");
    fs::create_dir_all(&root).unwrap();

    let write = |rel: &str, content: &str| {
        let target = root.join(rel);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, content.as_bytes()).unwrap();
    };

    write(
        "brain.toml",
        r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "brain"
tier = "_root"
repo_path = "."
status_file = "planning/status.md"
cache_doc = "README.md"
heading = "Company Brain"

[[repos]]
slug = "fx"
tier = "_root"
repo_path = "fx"
status_file = "fx/planning/status.md"
cache_doc = "docs/projects/fx.md"
heading = "fx"
"#,
    );

    write(
        "planning/state.json",
        r#"{
  "repo": "brain",
  "kind": "brain",
  "updated": "2026-08-18",
  "focus": { "now": [], "next": [], "blocked": [] },
  "repos": [],
  "cross_repo": [],
  "tiers": []
}
"#,
    );

    // The leaf repo: a state.json with one block, and a master-plan.md carrying
    // the sentinel pair the generator splices into.
    write(
        "fx/planning/state.json",
        r#"{
  "repo": "fx",
  "kind": "project",
  "updated": "2026-08-18",
  "focus": { "now": [], "next": [], "blocked": [] },
  "tracks": [
    {
      "title": "Phase 1: Foundations",
      "blocks": [
        {
          "id": "FX.1.A",
          "title": "First fixture block",
          "status": "open",
          "wave": 1,
          "description": "A fixture block used to exercise the master-plan renderer."
        }
      ]
    }
  ]
}
"#,
    );

    write(
        "fx/planning/master-plan.md",
        "# fx — Master Plan\n\
         \n\
         ## Preface\n\
         \n\
         Authored prose that must survive.\n\
         \n\
         <!-- BEGIN generated:master-plan-body -->\n\
         (stale placeholder content the renderer must replace)\n\
         <!-- END generated:master-plan-body -->\n",
    );

    mev::emit_state(&root, true, None).expect("emit_state should not error on this fixture brain");

    let rendered = fs::read_to_string(root.join("fx/planning/master-plan.md"))
        .expect("master-plan.md must still exist after emit_state");

    assert!(
        !rendered.contains("stale placeholder"),
        "emit_state did not replace the generated region — the master_plan generator is not \
         wired into the pipeline. Rendered:\n{rendered}"
    );
    assert!(
        rendered.contains("FX.1.A"),
        "expected the block id in the spliced region; got:\n{rendered}"
    );
    assert!(
        rendered.contains("## Preface") && rendered.contains("Authored prose that must survive."),
        "authored prose outside the sentinels must survive emit_state; got:\n{rendered}"
    );
}
