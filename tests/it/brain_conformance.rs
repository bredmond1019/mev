//! Integration tests for `mev::conformance` — the `mev conformance` driver
//! (`MV.ticket.conformance-check-registry` — Task 6).
//!
//! Exercises the public driver end to end: `find_brain_config` → `discover_state_files` →
//! `load_state` → `ConformanceCtx` → `run_checks`, over a minimal temp-dir fixture. The
//! four seed checks each have their own focused unit tests in
//! `src/brain/conformance/*.rs`; these tests cover only the driver's own responsibilities:
//! wiring the registry together, the `--check` narrowing + error path, and the drift-count
//! → exit-code contract the CLI layer relies on.

use std::fs;
use std::path::{Path, PathBuf};

/// Make a fresh uniquely-named temp dir for a test and return its path.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-conformance-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a minimal `brain.toml` with no `[[repos]]` entries — enough for `find_brain_config`
/// to resolve, but light enough that the disk-backed checks (`backlog-parity`,
/// `epics-index-parity`, `project-cache-watermark`) all land on `not-evaluable` (their
/// inputs are absent), which is exactly the "missing input" contract the driver must not
/// mistake for drift.
fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// Serialize `value` as pretty JSON and write it to `root/rel`.
fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

/// Create a file at `root/rel` (creating parent dirs as needed) with `content`.
fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, content.as_bytes()).unwrap();
}

/// Write the HQ brain `planning/state.json` with one backlog title (`"Ticket One"`) and
/// one epic (`alpha`, status `active`, `plan` pointing inside `core/planning/epics/`) — the
/// full corpus shape `backlog-parity` and `epics-index-parity` both read from.
fn write_hq_state_full(root: &Path) {
    let state = serde_json::json!({
        "repo": "hq",
        "kind": "brain",
        "updated": "2026-08-03",
        "focus": { "now": [], "next": [], "blocked": [] },
        "backlog": [
            {
                "slug": "ticket-one",
                "title": "Ticket One",
                "repo": "cross-repo",
                "type": "improvement",
                "status": "idea"
            }
        ],
        "epics": [
            {
                "slug": "alpha",
                "title": "Alpha Epic",
                "status": "active",
                "plan": "core/planning/epics/alpha.md"
            }
        ]
    });
    write_json(root, "planning/state.json", &state);
}

/// Write `planning/backlog.md` whose `## Active` title matches the JSON side
/// (`"Ticket One"`) — `## Superseded`/`## Shipped` are present but empty to exercise the
/// section filter without affecting parity.
fn write_backlog_md_matching(root: &Path) {
    write_file(
        root,
        "planning/backlog.md",
        "## Active\n\n### [2026-08-01] Ticket One\nbody\n\n## Promoted\n\n## Superseded\n\n## Shipped\n",
    );
}

/// Write `core/planning/epics/index.md` with one row matching the `alpha` epic.
fn write_epics_index_matching(root: &Path) {
    write_file(
        root,
        "core/planning/epics/index.md",
        "| Doc | Epic | Status | Repos |\n|---|---|---|---|\n\
         | [alpha.md](alpha.md) | **Alpha Epic** | `active` | `mev` |\n",
    );
}

/// Write the per-epic doc the `alpha` epic's `plan` points at.
fn write_epic_doc(root: &Path) {
    write_file(root, "core/planning/epics/alpha.md", "# Alpha Epic\n");
}

/// Build the full corpus fixture: `brain.toml` + HQ `state.json` (`backlog[]` + `epics[]`)
/// plus `planning/backlog.md`, `core/planning/epics/index.md`, and the per-epic doc —
/// every side both disk-backed checks read is present and matching.
fn write_full_clean_fixture(root: &Path) {
    write_brain_toml(root);
    write_hq_state_full(root);
    write_backlog_md_matching(root);
    write_epics_index_matching(root);
    write_epic_doc(root);
}

#[test]
fn conformance_runs_every_registered_check() {
    let dir = temp_dir("all-checks");
    write_brain_toml(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");

    assert_eq!(
        report.results.len(),
        mev::all_checks().len(),
        "driver should run every registered check when --check is not given"
    );
    let names: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"backlog-parity"));
    assert!(names.contains(&"epics-index-parity"));
    assert!(names.contains(&"project-cache-watermark"));
    assert!(names.contains(&"toolchain-freshness"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_check_flag_narrows_to_one_check() {
    let dir = temp_dir("narrow");
    write_brain_toml(&dir);

    let report =
        mev::conformance(&dir, Some("backlog-parity")).expect("conformance should not error");

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].name, "backlog-parity");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_unknown_check_name_errors_naming_valid_checks() {
    let dir = temp_dir("unknown-check");
    write_brain_toml(&dir);

    let err = mev::conformance(&dir, Some("does-not-exist")).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("does-not-exist"));
    assert!(msg.contains("backlog-parity"));
    assert!(msg.contains("epics-index-parity"));
    assert!(msg.contains("project-cache-watermark"));
    assert!(msg.contains("toolchain-freshness"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_missing_inputs_report_not_evaluable_never_drift() {
    let dir = temp_dir("missing-inputs");
    write_brain_toml(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");

    // backlog.md and the epics index are both absent from this minimal fixture — a
    // missing input must land on not-evaluable, never be misreported as drift.
    let backlog = report
        .results
        .iter()
        .find(|r| r.name == "backlog-parity")
        .expect("backlog-parity should be present");
    assert_eq!(backlog.outcome.status, mev::CheckStatus::NotEvaluable);

    let epics = report
        .results
        .iter()
        .find(|r| r.name == "epics-index-parity")
        .expect("epics-index-parity should be present");
    assert_eq!(epics.outcome.status, mev::CheckStatus::NotEvaluable);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_tallies_sum_to_results_len() {
    let dir = temp_dir("tallies");
    write_brain_toml(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");

    assert_eq!(
        report.pass_count + report.drift_count + report.not_evaluable_count,
        report.results.len()
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn conformance_errors_when_brain_toml_is_missing() {
    let dir = temp_dir("no-brain-toml");
    // No brain.toml written — this must surface as a hard configuration error, matching
    // block_graph_brain / carryover_sweep's contract.
    let err = mev::conformance(&dir, None).unwrap_err();
    assert!(err.to_string().contains("brain.toml"));

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Full corpus fixture — clean fixture, seeded drift, single-check filter, tallies.
// ---------------------------------------------------------------------------

/// Names of every registered check whose `reads_live_checkout` property is set — i.e. it
/// compares against the *running checkout's own source tree* (a `build.rs`-stamped path)
/// rather than purely against the fixture passed in, and so can legitimately report
/// `Drift` for reasons that have nothing to do with the fixture under test (an
/// uncommitted worktree, a checkout path the CI runner stamped differently, etc). Derived
/// from the registry's own `reads_live_checkout` field — never a hard-coded name list —
/// so the next check with this property is excluded automatically instead of failing
/// these fixture assertions the day it lands.
fn live_checkout_check_names() -> std::collections::HashSet<&'static str> {
    mev::all_checks()
        .into_iter()
        .filter(|c| c.reads_live_checkout)
        .map(|c| c.name)
        .collect()
}

/// An all-clean full corpus fixture (matching `backlog[]`/`backlog.md` and
/// `epics[]`/index.md sides) reports zero drift. `project-cache-watermark` has no
/// matching inputs in this fixture and is expected to be `not-evaluable`, never `drift`.
///
/// Checks with `reads_live_checkout: true` (currently `toolchain-freshness` and
/// `sibling-rule-coverage`) are excluded from this assertion: they don't read the fixture
/// at all — they compare the *running test binary's* compiled-in build stamp / source
/// text against the live state of the real `core/mev` source tree (see
/// `src/brain/conformance/toolchain.rs` and `sibling.rs`), so they legitimately report
/// `Drift` whenever `cargo test` runs from an uncommitted mev worktree or a checkout the
/// runner stamped at a different path. That's a property of the checkout running the
/// test, not of this fixture — see `ConformanceCheck::reads_live_checkout`'s doc comment.
#[test]
fn full_fixture_reports_zero_drift() {
    let dir = temp_dir("full-clean");
    write_full_clean_fixture(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");
    let live_checkout_checks = live_checkout_check_names();

    let non_live_checkout_drift = report
        .results
        .iter()
        .filter(|r| {
            !live_checkout_checks.contains(r.name.as_str())
                && r.outcome.status == mev::CheckStatus::Drift
        })
        .count();

    assert_eq!(
        non_live_checkout_drift, 0,
        "clean full corpus fixture should report zero drift outside live-checkout checks, got: {:#?}",
        report.results
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Seeding one backlog title into `backlog.md` that has no `state.json backlog[]`
/// counterpart is detected as exactly one drifting check (`backlog-parity`), and the
/// offending title is named in its findings.
#[test]
fn seeded_backlog_title_drift_is_detected_and_named() {
    let dir = temp_dir("seeded-backlog-drift");
    write_full_clean_fixture(&dir);

    // Overwrite backlog.md with an extra markdown-only title that has no state.json
    // backlog[] counterpart.
    write_file(
        &dir,
        "planning/backlog.md",
        "## Active\n\n### [2026-08-01] Ticket One\nbody\n\n### [2026-08-02] Markdown Only Ticket\nbody\n\n## Promoted\n\n## Superseded\n\n## Shipped\n",
    );

    let report = mev::conformance(&dir, None).expect("conformance should not error");
    let live_checkout_checks = live_checkout_check_names();

    // Checks with `reads_live_checkout: true` are excluded here for the same reason as in
    // `full_fixture_reports_zero_drift`: they reflect the real mev checkout's build
    // provenance / source tree, not this fixture, and can independently report `Drift`
    // when `cargo test` runs from an uncommitted mev worktree or a differently-stamped
    // checkout.
    let drifting: Vec<_> = report
        .results
        .iter()
        .filter(|r| {
            !live_checkout_checks.contains(r.name.as_str())
                && r.outcome.status == mev::CheckStatus::Drift
        })
        .collect();

    assert_eq!(
        drifting.len(),
        1,
        "expected exactly one drifting check, got: {:#?}",
        report.results
    );
    assert_eq!(drifting[0].name, "backlog-parity");
    assert!(
        drifting[0]
            .outcome
            .findings
            .iter()
            .any(|f| f.contains("Markdown Only Ticket")),
        "backlog-parity findings should name the offending title, got: {:#?}",
        drifting[0].outcome.findings
    );

    let _ = fs::remove_dir_all(&dir);
}

/// `only = Some("epics-index-parity")` over the full corpus fixture runs exactly that one
/// check and it passes (both sides match).
#[test]
fn only_filter_over_full_fixture_runs_exactly_one_check() {
    let dir = temp_dir("full-only-filter");
    write_full_clean_fixture(&dir);

    let report = mev::conformance(&dir, Some("epics-index-parity"))
        .expect("conformance with a valid --check name should not error");

    assert_eq!(
        report.results.len(),
        1,
        "expected exactly one result, got: {:#?}",
        report.results
    );
    assert_eq!(report.results[0].name, "epics-index-parity");
    assert_eq!(report.results[0].outcome.status, mev::CheckStatus::Pass);

    let _ = fs::remove_dir_all(&dir);
}

/// The full corpus fixture's report tallies still sum to the number of results.
#[test]
fn full_fixture_tallies_sum_to_results_len() {
    let dir = temp_dir("full-tallies");
    write_full_clean_fixture(&dir);

    let report = mev::conformance(&dir, None).expect("conformance should not error");

    assert_eq!(
        report.pass_count + report.drift_count + report.not_evaluable_count,
        report.results.len(),
        "tallies must sum to the number of results, got report: {:#?}",
        report
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------
// `toolchain::differ_build_inputs` — the impure git-diff helper `toolchain-freshness`
// consults before reporting Drift, exercised against a throwaway repo (never the real
// repo or corpus — `conformance-fixture-tests-depend-on-live-repo-state` is a recorded
// trap here). `MV.ticket.toolchain-freshness-keys-on-build-inputs-not-head` — Task 2.
// ---------------------------------------------------------------------------------------

/// Run `git` with `-C dir`, asserting success, mirroring `brain_emit.rs`'s `run_git` — the
/// established pattern for throwaway-repo fixtures in this integration binary.
fn differ_run_git(dir: &Path, args: &[&str]) -> String {
    let output = mev::testsupport::git_command()
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("failed to spawn git");
    assert!(
        output.status.success(),
        "git {:?} failed in {}: {}",
        args,
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn differ_build_inputs_covers_same_differ_and_unknown() {
    let dir = temp_dir("differ-build-inputs");

    differ_run_git(&dir, &["init", "-q"]);
    differ_run_git(&dir, &["config", "user.email", "test@example.com"]);
    differ_run_git(&dir, &["config", "user.name", "Test"]);

    // Commit 1: a docs file only.
    write_file(&dir, "docs/x.md", "# hello\n");
    differ_run_git(&dir, &["add", "."]);
    differ_run_git(&dir, &["commit", "-q", "-m", "add docs/x.md"]);
    let sha1 = differ_run_git(&dir, &["rev-parse", "HEAD"]);

    // Commit 2: another docs-only change. Nothing under a build input path differs
    // between sha1 and sha2 -> Same.
    write_file(&dir, "docs/x.md", "# hello again\n");
    differ_run_git(&dir, &["add", "."]);
    differ_run_git(&dir, &["commit", "-q", "-m", "edit docs/x.md"]);
    let sha2 = differ_run_git(&dir, &["rev-parse", "HEAD"]);

    let same = mev::brain::conformance::toolchain::differ_build_inputs(
        dir.to_str().unwrap(),
        &sha1,
        &sha2,
    );
    assert_eq!(
        same,
        mev::brain::conformance::toolchain::BuildInputComparison::Same,
        "a docs-only difference between two commits must not read as a build-input change"
    );

    // Commit 3: touches src/x.rs, a build input path -> Differ against sha2.
    write_file(&dir, "src/x.rs", "fn main() {}\n");
    differ_run_git(&dir, &["add", "."]);
    differ_run_git(&dir, &["commit", "-q", "-m", "add src/x.rs"]);
    let sha3 = differ_run_git(&dir, &["rev-parse", "HEAD"]);

    let differ = mev::brain::conformance::toolchain::differ_build_inputs(
        dir.to_str().unwrap(),
        &sha2,
        &sha3,
    );
    assert_eq!(
        differ,
        mev::brain::conformance::toolchain::BuildInputComparison::Differ,
        "a src/ change between two commits must read as a build-input change"
    );

    // A fabricated, unresolvable SHA -> Unknown. Absence of a diff answer must never be
    // read as "no difference".
    let unknown = mev::brain::conformance::toolchain::differ_build_inputs(
        dir.to_str().unwrap(),
        "0000000000000000000000000000000000000000",
        &sha3,
    );
    assert_eq!(
        unknown,
        mev::brain::conformance::toolchain::BuildInputComparison::Unknown,
        "an unresolvable SHA must report Unknown, never Same"
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------
// `surface-leak` (MV.19.A task 1) — five shown-failing fixtures, one per defect class this
// block closes. Each drives the PUBLIC surface (`mev::brain::conformance::surface::{run,
// evaluate_repo}`) rather than any helper task 2-5 has yet to write, so this task compiles
// and fails rather than failing to build (D58 compilable boundaries). D68: every one of
// these was run and OBSERVED RED before any fix landed — see task notes for the pasted
// failure output.
// ---------------------------------------------------------------------------------------

/// Run `git` with the given args in `dir`, asserting success — mirrors `differ_run_git`
/// above and `surface.rs`'s own `#[cfg(test)]` fixtures, which deliberately shell out to a
/// real `git init` rather than mocking, because the whole subject of this check IS git's
/// tracked set.
fn surface_run_git(dir: &Path, args: &[&str]) {
    let output = mev::testsupport::git_command()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be on PATH for these tests");
    assert!(
        output.status.success(),
        "git {args:?} failed in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn surface_init_repo(dir: &Path) {
    surface_run_git(dir, &["init", "-q"]);
    surface_run_git(dir, &["config", "user.email", "test@example.com"]);
    surface_run_git(dir, &["config", "user.name", "Test"]);
}

fn surface_commit_all(dir: &Path) {
    surface_run_git(dir, &["add", "-A"]);
    surface_run_git(dir, &["commit", "-q", "-m", "fixture"]);
}

/// Build one `[[repos]]` entry for surface-leak fixtures — mirrors the private
/// `repo_entry` helper inside `surface.rs`'s own `#[cfg(test)]` module, duplicated here
/// because that helper is not (and should not become) part of the public surface.
fn surface_repo_entry(slug: &str, repo_path: &str, public: bool) -> mev::brain::config::RepoEntry {
    mev::brain::config::RepoEntry {
        slug: slug.to_string(),
        tier: "core".to_string(),
        repo_path: repo_path.to_string(),
        status_file: String::new(),
        cache_doc: String::new(),
        heading: String::new(),
        prefix: None,
        public,
    }
}

/// Class 1 — VERSION STRING: a tracked file containing `1.27.2.3` (13 live instances) and
/// `15.8.1.060` (3 live instances, zero-padded final octet) must produce ZERO rule-2
/// findings once `fn is_version_string` (task 2) lands. Today rule 2 has no such filter, so
/// this fires and the test is RED.
#[test]
fn version_shaped_dotted_quads_do_not_fire_rule2() {
    let dir = temp_dir("surface-version-string");
    surface_init_repo(&dir);
    write_file(
        &dir,
        "pyproject.toml",
        "requires = \"1.27.2.3\"\nimage = \"registry/foo:15.8.1.060\"\n",
    );
    surface_commit_all(&dir);

    let repo = surface_repo_entry("fixture", "", true);
    let findings = mev::brain::conformance::surface::evaluate_repo(&dir, &repo, &[]).unwrap();
    let rule2: Vec<_> = findings.iter().filter(|f| f.rule == "rule2").collect();
    assert!(
        rule2.is_empty(),
        "version-shaped literals must not fire rule 2, got: {rule2:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Class 2 — SELF-FIXTURE: a tracked file listed in `[surface_allowlist] self_fixtures`
/// (task 3) must produce zero rule-2 findings for its own literal, while an IDENTICAL
/// literal in a file NOT listed still fires — the control half. Without the control, a
/// stub that suppresses rule 2 everywhere would pass this test trivially.
///
/// Drives `surface::run` through a real `brain.toml` (loaded with
/// `mev::brain::config::load_brain_config`) rather than naming a not-yet-written helper
/// like `evaluate_repo_with_self_fixtures` — that would be a build error, not a red test
/// (D58). Today `SurfaceAllowlist` has no `self_fixtures` field, so `[surface_allowlist]
/// self_fixtures = [...]` in the TOML is silently ignored by serde as an unknown key (no
/// `deny_unknown_fields`), the exemption never applies, and the self-fixture file's literal
/// still fires rule 2 — RED. The control half (the unlisted file) already fires today,
/// which is expected; the class is proven by the exempted half flipping to green once task
/// 3 adds the field and wires the exemption.
#[test]
fn self_fixture_file_is_exempt_from_rule2_but_control_file_still_fires() {
    let dir = temp_dir("surface-self-fixture");
    surface_init_repo(&dir);
    write_file(
        &dir,
        "src/brain/conformance/surface.rs",
        "// positive-control fixture literal: 100.64.1.2\n",
    );
    write_file(
        &dir,
        "docs/unrelated.md",
        "not a fixture, real leak: 100.64.1.2\n",
    );
    surface_commit_all(&dir);

    // `self_fixtures` entries are shaped `<repo-slug>:<repo-relative-path>` (task 3).
    let brain_toml_path = dir.join("test-brain.toml");
    fs::write(
        &brain_toml_path,
        format!(
            "[[repos]]\nslug = \"fixture\"\nrepo_path = \"{}\"\npublic = true\n\n\
             [surface_allowlist]\nself_fixtures = [\"fixture:src/brain/conformance/surface.rs\"]\n",
            dir.display()
        ),
    )
    .unwrap();
    let config = mev::brain::config::load_brain_config(&brain_toml_path).expect(
        "test brain.toml must parse — an unknown [surface_allowlist] key must not be a hard error",
    );
    let ctx = mev::ConformanceCtx {
        root: dir.clone(),
        config,
        files: Vec::new(),
    };

    let outcome = mev::brain::conformance::surface::run(&ctx);
    let rule2: Vec<&String> = outcome
        .findings
        .iter()
        .filter(|f| f.contains(" rule2:"))
        .collect();

    assert!(
        !rule2
            .iter()
            .any(|f| f.contains("src/brain/conformance/surface.rs")),
        "self-fixture file must not fire rule 2, got: {rule2:?}"
    );
    assert!(
        rule2.iter().any(|f| f.contains("docs/unrelated.md")),
        "control: an unlisted file with the identical literal must still fire, got: {rule2:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Class 3 — DIRECTORY LINK TARGET: a markdown link whose target is a tracked DIRECTORY
/// PREFIX (some tracked file has it as a path prefix) must resolve and produce no rule-1
/// finding; a link to a genuinely absent path in the same fixture must still fire (the
/// control half, task 1's existing `climb_out_of_repo_root_link_fires`-style pattern).
/// Today the tracked set is file-only, so `crates/engine-contract` (a directory, not a
/// file) is absent from it and this fires — RED.
#[test]
fn directory_prefix_link_target_resolves_but_absent_path_still_fires() {
    let dir = temp_dir("surface-dir-link");
    surface_init_repo(&dir);
    write_file(
        &dir,
        "README.md",
        "see [engine-contract](crates/engine-contract) and [nope](crates/does-not-exist)\n",
    );
    write_file(&dir, "crates/engine-contract/Cargo.toml", "[package]\n");
    surface_commit_all(&dir);

    let repo = surface_repo_entry("fixture", "", true);
    let findings = mev::brain::conformance::surface::evaluate_repo(&dir, &repo, &[]).unwrap();
    let rule1: Vec<_> = findings.iter().filter(|f| f.rule == "rule1").collect();

    assert!(
        !rule1
            .iter()
            .any(|f| f.detail.contains("crates/engine-contract")),
        "a tracked directory prefix must resolve, got: {rule1:?}"
    );
    assert!(
        rule1
            .iter()
            .any(|f| f.detail.contains("crates/does-not-exist")),
        "control: a genuinely absent path must still fire, got: {rule1:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Class 4 — EMPTY TRACKED SET: a `git init`-ed repo with NO commit must make the run
/// report an error naming that repo, not `Pass`. Today `tracked_set` has no emptiness
/// check, so `git ls-files` on an uncommitted repo succeeds with empty output, `Ok(empty
/// set)` is returned, every rule passes having scanned zero bytes, and the overall status
/// is `Pass` — RED (this constructs the fail-open state rather than asserting a message).
#[test]
fn uninitialized_uncommitted_repo_errors_naming_the_repo_not_pass() {
    let dir = temp_dir("surface-empty-tracked-set");
    surface_run_git(&dir, &["init", "-q"]);
    // Deliberately no `git add` / `git commit` — an init-ed but uncommitted repo.

    let repo = surface_repo_entry("uncommitted-repo", dir.to_str().unwrap(), true);
    let config = mev::brain::config::BrainConfig {
        repos: vec![repo],
        ..Default::default()
    };
    let ctx = mev::ConformanceCtx {
        root: PathBuf::from("."),
        config,
        files: Vec::new(),
    };

    let outcome = mev::brain::conformance::surface::run(&ctx);
    assert_ne!(
        outcome.status,
        mev::CheckStatus::Pass,
        "an uncommitted repo scanning zero bytes must never report Pass, got: {outcome:?}"
    );
    let reason = outcome.reason.as_deref().unwrap_or_default();
    assert!(
        reason.contains("uncommitted-repo"),
        "the reason must name the offending repo, got: {reason:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// Class 5 — ZERO PUBLIC REPOS: a `brain.toml` whose `[[repos]]` entries all omit `public`
/// must return `NotEvaluable` with a reason naming that condition, not `Pass`. Today
/// `run()` returns `Pass` with `reason: None` when `public_repos.is_empty()` — RED (this
/// constructs the config rather than asserting on prose).
#[test]
fn zero_public_repos_is_not_evaluable_with_a_reason_not_a_silent_pass() {
    let mut config = mev::brain::config::BrainConfig::default();
    // Every entry omits `public` (defaults false, fail-closed per config.rs) — none are
    // public, so nothing is walked.
    config.repos = vec![
        mev::brain::config::RepoEntry {
            slug: "private-one".to_string(),
            ..Default::default()
        },
        mev::brain::config::RepoEntry {
            slug: "private-two".to_string(),
            ..Default::default()
        },
    ];
    let ctx = mev::ConformanceCtx {
        root: PathBuf::from("."),
        config,
        files: Vec::new(),
    };

    let outcome = mev::brain::conformance::surface::run(&ctx);
    assert_eq!(
        outcome.status,
        mev::CheckStatus::NotEvaluable,
        "a run with no public repos must be NotEvaluable, not Pass, got: {outcome:?}"
    );
    assert!(
        outcome.reason.is_some(),
        "NotEvaluable must carry a reason naming the zero-public-repos condition"
    );
}
