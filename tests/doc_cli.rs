//! Integration tests for `mev doc ...` CLI wiring — the `materialize` and
//! `opportunity ingest|set-stage|add-action|merge-contacts` verbs
//! (`MV.9.A` task 4).
//!
//! Drives the built binary directly (`CARGO_BIN_EXE_mev`), the same pattern
//! `tests/brain_emit.rs`'s linked-worktree tests use, so the CLI's flag
//! parsing, dry-run/write dispatch, `--json` envelope, and exit codes are
//! all genuinely exercised — not just the `mev::doc_*` library functions.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Build a fresh tempdir with a `brain.toml` marker (all `find_brain_root`
/// requires) and an empty `business/docs/opportunities/` directory.
fn setup_corpus() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("brain.toml"), "").unwrap();
    fs::create_dir_all(tmp.path().join("business/docs/opportunities")).unwrap();
    tmp
}

fn fixture_brief_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/company_brief.json")
}

fn opportunity_path(root: &Path) -> PathBuf {
    root.join("business/docs/opportunities/anthropic.md")
}

fn run_mev(args: &[&str], cwd: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mev"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("failed to spawn mev binary")
}

// ── dry-run vs write ────────────────────────────────────────────────────────

#[test]
fn ingest_dry_run_touches_nothing_but_reports_the_planned_action() {
    let tmp = setup_corpus();
    let brief = fixture_brief_path();
    let target = opportunity_path(tmp.path());

    let output = run_mev(
        &[
            "doc",
            "opportunity",
            "ingest",
            "--input",
            brief.to_str().unwrap(),
            ".",
        ],
        tmp.path(),
    );

    assert!(
        output.status.success(),
        "dry-run ingest must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!target.exists(), "dry-run must not create the target file");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("W_EMIT_DRY_RUN"),
        "dry-run must report the planned action; stdout: {stdout}"
    );
    assert!(
        stdout.contains("dry-run"),
        "summary line must say dry-run: {stdout}"
    );
}

#[test]
fn ingest_write_creates_the_file_and_second_write_is_a_zero_action_noop() {
    let tmp = setup_corpus();
    let brief = fixture_brief_path();
    let target = opportunity_path(tmp.path());

    let output = run_mev(
        &[
            "doc",
            "opportunity",
            "ingest",
            "--input",
            brief.to_str().unwrap(),
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(
        output.status.success(),
        "--write ingest must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(target.exists(), "--write must create the target file");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("I_EMIT_WROTE"),
        "must report the write: {stdout}"
    );

    let bytes_after_first = fs::read(&target).unwrap();

    // Second identical --write must be a zero-action no-op: file unchanged,
    // and the summary reports zero errors (W_DOC_UNCHANGED is a warning).
    let output2 = run_mev(
        &[
            "doc",
            "opportunity",
            "ingest",
            "--input",
            brief.to_str().unwrap(),
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(
        output2.status.success(),
        "second --write must still exit 0; stderr: {}",
        String::from_utf8_lossy(&output2.stderr)
    );
    let bytes_after_second = fs::read(&target).unwrap();
    assert_eq!(
        bytes_after_first, bytes_after_second,
        "second --write must leave the file byte-identical"
    );
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("W_DOC_UNCHANGED"),
        "second write must report W_DOC_UNCHANGED: {stdout2}"
    );
    assert!(
        !stdout2.contains("I_EMIT_WROTE"),
        "second write must not report another write: {stdout2}"
    );
}

// ── --json envelope ─────────────────────────────────────────────────────────

#[test]
fn json_flag_emits_a_parseable_envelope() {
    let tmp = setup_corpus();
    let brief = fixture_brief_path();

    let output = run_mev(
        &[
            "--json",
            "doc",
            "opportunity",
            "ingest",
            "--input",
            brief.to_str().unwrap(),
            ".",
        ],
        tmp.path(),
    );
    assert!(
        output.status.success(),
        "dry-run --json ingest must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("--json output must be valid JSON");
    assert_eq!(parsed["validator"], "doc-opportunity-ingest");
    assert!(parsed["diagnostics"].is_array());
    assert_eq!(parsed["errors"], 0);
}

// ── materialize (generic model dispatch) ────────────────────────────────────

#[test]
fn materialize_learning_artifact_dry_run_reports_planned_action() {
    let tmp = setup_corpus();
    let input = tmp.path().join("artifact.json");
    fs::write(
        &input,
        serde_json::json!({
            "artifact_id": "artifact-1",
            "channel_type": "podcast",
            "source_ref": "https://example.com/ep1",
            "summary": "A podcast episode.",
            "digest_markdown": "# Digest\n",
            "entities": ["thing"],
            "language": "en",
        })
        .to_string(),
    )
    .unwrap();

    let output = run_mev(
        &[
            "doc",
            "materialize",
            "--model",
            "learning-artifact",
            "--input",
            input.to_str().unwrap(),
            ".",
        ],
        tmp.path(),
    );
    assert!(
        output.status.success(),
        "materialize dry-run must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("W_EMIT_DRY_RUN"), "stdout: {stdout}");
}

// ── failure exit codes ───────────────────────────────────────────────────────

#[test]
fn unresolvable_root_exits_1() {
    // A tempdir with no brain.toml anywhere in its ancestry (use the tempdir
    // itself, which is off in a scratch location with no brain.toml above
    // it in the tree we control — assert failure specifically on our
    // synthesized empty dir rather than assuming about the real filesystem
    // root, since a brain.toml could theoretically exist far above /tmp).
    let tmp = tempfile::tempdir().unwrap();
    let brief = fixture_brief_path();

    let output = run_mev(
        &[
            "doc",
            "opportunity",
            "ingest",
            "--input",
            brief.to_str().unwrap(),
            tmp.path().to_str().unwrap(),
        ],
        tmp.path(),
    );

    // Either this exits 1 because brain.toml truly cannot be found, or (in
    // an environment where an ancestor happens to carry one) it still
    // exercises the same code path; the load-bearing assertion is the
    // exit-1 contract when resolution fails, which we verify against a
    // path argument that is guaranteed not to contain brain.toml.
    if !output.status.success() {
        assert!(!output.status.success());
    } else {
        panic!(
            "expected exit 1 resolving brain.toml under a bare tempdir with no brain.toml; stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn bad_model_exits_1() {
    let tmp = setup_corpus();
    let input = tmp.path().join("payload.json");
    fs::write(&input, "{}").unwrap();

    let output = run_mev(
        &[
            "doc",
            "materialize",
            "--model",
            "not-a-real-model",
            "--input",
            input.to_str().unwrap(),
            ".",
        ],
        tmp.path(),
    );

    assert!(
        !output.status.success(),
        "a bad --model must exit non-zero; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("E_DOC_UNKNOWN_MODEL"),
        "must report the unknown-model diagnostic; stdout: {stdout}"
    );
}

// ── set-stage / add-action / merge-contacts idempotency ─────────────────────

#[test]
fn set_stage_add_action_merge_contacts_round_trip_and_are_idempotent() {
    let tmp = setup_corpus();
    let brief = fixture_brief_path();
    let target = opportunity_path(tmp.path());

    // Seed the opportunity file first.
    let out = run_mev(
        &[
            "doc",
            "opportunity",
            "ingest",
            "--input",
            brief.to_str().unwrap(),
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(out.status.success());
    assert!(target.exists());

    // set-stage
    let out = run_mev(
        &[
            "doc",
            "opportunity",
            "set-stage",
            "anthropic",
            "contacted",
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(out.status.success(), "set-stage must succeed");
    let after_stage = fs::read_to_string(&target).unwrap();
    assert!(after_stage.contains("stage: contacted"));

    let out2 = run_mev(
        &[
            "doc",
            "opportunity",
            "set-stage",
            "anthropic",
            "contacted",
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(out2.status.success());
    assert_eq!(fs::read_to_string(&target).unwrap(), after_stage);
    assert!(String::from_utf8_lossy(&out2.stdout).contains("W_DOC_UNCHANGED"));

    // add-action
    let out = run_mev(
        &[
            "doc",
            "opportunity",
            "add-action",
            "anthropic",
            "--kind",
            "email",
            "--note",
            "Sent initial outreach",
            "--at",
            "2026-07-27",
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(out.status.success(), "add-action must succeed");
    let after_action = fs::read_to_string(&target).unwrap();
    assert!(after_action.contains("Sent initial outreach"));

    let out2 = run_mev(
        &[
            "doc",
            "opportunity",
            "add-action",
            "anthropic",
            "--kind",
            "email",
            "--note",
            "Sent initial outreach",
            "--at",
            "2026-07-27",
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(out2.status.success());
    assert_eq!(fs::read_to_string(&target).unwrap(), after_action);
    assert!(String::from_utf8_lossy(&out2.stdout).contains("W_DOC_UNCHANGED"));

    // merge-contacts
    let contact_input = tmp.path().join("contact.json");
    fs::write(
        &contact_input,
        serde_json::json!({"name": "Alice", "emails": ["alice@example.com"]}).to_string(),
    )
    .unwrap();
    let out = run_mev(
        &[
            "doc",
            "opportunity",
            "merge-contacts",
            "anthropic",
            "--input",
            contact_input.to_str().unwrap(),
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(out.status.success(), "merge-contacts must succeed");
    let after_contacts = fs::read_to_string(&target).unwrap();
    assert!(after_contacts.contains("alice@example.com"));

    let out2 = run_mev(
        &[
            "doc",
            "opportunity",
            "merge-contacts",
            "anthropic",
            "--input",
            contact_input.to_str().unwrap(),
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(out2.status.success());
    assert_eq!(fs::read_to_string(&target).unwrap(), after_contacts);
    assert!(String::from_utf8_lossy(&out2.stdout).contains("W_DOC_UNCHANGED"));
}

#[test]
fn set_stage_bad_stage_exits_1() {
    let tmp = setup_corpus();
    let brief = fixture_brief_path();
    let out = run_mev(
        &[
            "doc",
            "opportunity",
            "ingest",
            "--input",
            brief.to_str().unwrap(),
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(out.status.success());

    let out = run_mev(
        &[
            "doc",
            "opportunity",
            "set-stage",
            "anthropic",
            "not-a-stage",
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(!out.status.success(), "an invalid stage must exit non-zero");
    assert!(String::from_utf8_lossy(&out.stdout).contains("E_DOC_BAD_STAGE"));
}

#[test]
fn mutator_on_missing_opportunity_exits_1() {
    let tmp = setup_corpus();
    let out = run_mev(
        &[
            "doc",
            "opportunity",
            "set-stage",
            "does-not-exist",
            "contacted",
            ".",
            "--write",
        ],
        tmp.path(),
    );
    assert!(
        !out.status.success(),
        "set-stage on a missing opportunity must exit non-zero"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("E_DOC_NOT_FOUND"));
}
