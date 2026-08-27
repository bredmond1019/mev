//! Integration tests for the dedup suggestion pass — `MV.ticket.carryover-dedup-clusters` task 3.
//!
//! Pins the operator-measured recovery set from `planning/ticket-carryover-dedup-clusters/tasks.md`:
//! five real cross-repo pairs that a crude token-overlap pass over `slug` + `text` recovers, and
//! one documented hard miss it does not (and must not be tuned to catch).
//!
//! The fixture texts below are not verbatim corpus quotes — they are constructed to reproduce the
//! same token-overlap shape the ticket measured against the live corpus. Per the ticket's own
//! instruction, assertions check *pair presence*, never exact float equality: "scores shift with
//! tokenizer detail."

use mev::brain::carryover::{CarryoverLane, CarryoverVerdict, suggest_duplicates};

/// Build a `CarryoverVerdict` fixture with only the fields `suggest_duplicates` reads
/// (`repo`, `slug`, `text`, `finding_id`) populated meaningfully; the rest carry inert defaults.
fn verdict(repo: &str, slug: &str, text: &str) -> CarryoverVerdict {
    CarryoverVerdict {
        repo: repo.to_string(),
        slug: slug.to_string(),
        kind: "known_issue".to_string(),
        text: text.to_string(),
        clears_when: None,
        created: "2026-01-01".to_string(),
        age_days: None,
        stale: false,
        lane: CarryoverLane::NotEvaluable,
        refs: Vec::new(),
        reason: None,
        priority: None,
        finding_id: None,
        blocks: Vec::new(),
        enforce: None,
    }
}

/// Whether `suggestions` contains the unordered pair `(repo_a, slug_a)` / `(repo_b, slug_b)`,
/// regardless of which side `suggest_duplicates` assigned as `a` vs `b`.
fn contains_pair(
    suggestions: &[mev::brain::carryover::DedupSuggestion],
    repo_a: &str,
    slug_a: &str,
    repo_b: &str,
    slug_b: &str,
) -> bool {
    suggestions.iter().any(|s| {
        (s.a_repo == repo_a && s.a_slug == slug_a && s.b_repo == repo_b && s.b_slug == slug_b)
            || (s.a_repo == repo_b
                && s.a_slug == slug_b
                && s.b_repo == repo_a
                && s.b_slug == slug_a)
    })
}

#[test]
fn suggest_duplicates_recovers_all_five_operator_measured_pairs() {
    let entries = vec![
        // Pair 1 — mev:wave0-tickets-ship-without-tasks-json = okf-core:ticket-specs-ship-without-tasks-json
        verdict(
            "mev",
            "wave0-tickets-ship-without-tasks-json",
            "Wave 0 tickets ship without a tasks.json file, leaving the block spec incomplete.",
        ),
        verdict(
            "okf-core",
            "ticket-specs-ship-without-tasks-json",
            "Ticket specs ship without a tasks.json companion file in okf-core.",
        ),
        // Pair 2 — mev:nextest-rule-is-mev-scoped-not-fleet-wide = okf-core:lane-file-nextest-claim-overgeneralized
        verdict(
            "mev",
            "nextest-rule-is-mev-scoped-not-fleet-wide",
            "The nextest rule that hooks fire is mev-scoped, not a fleet-wide guarantee; \
             okf-core has no hook so the same claim overgeneralizes there.",
        ),
        verdict(
            "okf-core",
            "lane-file-nextest-claim-overgeneralized",
            "A lane file states the nextest claim as universal, but it is overgeneralized: \
             the rule is mev-scoped, and okf-core has no hook so it does not fire there.",
        ),
        // Pair 3 — mev:harness-json-schema-path-resolves-only-lexically = okf-core:harness-json-schema-ref-breaks-under-realpath
        verdict(
            "mev",
            "harness-json-schema-path-resolves-only-lexically",
            "The harness.json schema path resolves only lexically, so a symlinked planning \
             directory breaks path resolution silently.",
        ),
        verdict(
            "okf-core",
            "harness-json-schema-ref-breaks-under-realpath",
            "The harness.json schema ref breaks under realpath because symlink resolution \
             changes the ref path unexpectedly.",
        ),
        // Pair 4 — learn-ai:synapse-slug-lags-its-name = orchestrator:synapse-rename-mechanical-flip-pending
        verdict(
            "learn-ai",
            "synapse-slug-lags-its-name",
            "The synapse project slug still lags its real name; the rename to synapse is a \
             mechanical flip that has not landed yet across references.",
        ),
        verdict(
            "orchestrator",
            "synapse-rename-mechanical-flip-pending",
            "The orchestrator rename to synapse is a mechanical flip that is still pending; \
             the slug has not caught up with the new name yet.",
        ),
        // Pair 5 — the PROOF CASE: bastion:grep-inventory-is-a-hypothesis = mev:sdlc-spec-acceptance-vs-purpose-gap.
        // These two slugs share ZERO vocabulary (`grep-inventory-is-a-hypothesis` vs
        // `sdlc-spec-acceptance-vs-purpose-gap`) — this pair is recoverable ONLY because
        // `dedup_tokens` tokenizes `text` as well as `slug`. If a future edit narrows the
        // tokenizer to slug-only, this assertion must fail loudly.
        verdict(
            "bastion",
            "grep-inventory-is-a-hypothesis",
            "A repo inventory produced by grep is only a hypothesis about the fleet, not \
             verified ground truth, until every result is checked by hand.",
        ),
        verdict(
            "mev",
            "sdlc-spec-acceptance-vs-purpose-gap",
            "An sdlc spec's acceptance criteria is only a hypothesis about the purpose gap \
             it claims to close, not verified proof, until checked by hand.",
        ),
    ];

    let suggestions = suggest_duplicates(&entries);

    assert!(
        contains_pair(
            &suggestions,
            "mev",
            "wave0-tickets-ship-without-tasks-json",
            "okf-core",
            "ticket-specs-ship-without-tasks-json",
        ),
        "pair 1 (wave0-tickets-ship-without-tasks-json) missing: {suggestions:#?}"
    );
    assert!(
        contains_pair(
            &suggestions,
            "mev",
            "nextest-rule-is-mev-scoped-not-fleet-wide",
            "okf-core",
            "lane-file-nextest-claim-overgeneralized",
        ),
        "pair 2 (nextest-rule-is-mev-scoped) missing: {suggestions:#?}"
    );
    assert!(
        contains_pair(
            &suggestions,
            "mev",
            "harness-json-schema-path-resolves-only-lexically",
            "okf-core",
            "harness-json-schema-ref-breaks-under-realpath",
        ),
        "pair 3 (harness-json-schema) missing: {suggestions:#?}"
    );
    assert!(
        contains_pair(
            &suggestions,
            "learn-ai",
            "synapse-slug-lags-its-name",
            "orchestrator",
            "synapse-rename-mechanical-flip-pending",
        ),
        "pair 4 (synapse-slug-lags-its-name) missing: {suggestions:#?}"
    );
    assert!(
        contains_pair(
            &suggestions,
            "bastion",
            "grep-inventory-is-a-hypothesis",
            "mev",
            "sdlc-spec-acceptance-vs-purpose-gap",
        ),
        "pair 5 — the zero-shared-slug-vocabulary PROOF CASE — missing: {suggestions:#?}"
    );
}

/// PINNED MISS — `mev:okf-related-must-be-a-real-doc-id` vs
/// `okf-core:okf-core-doc-ids-are-inconsistent-with-filenames` is a real many-to-one case naive
/// token overlap cannot reach (measured ~0.29, under both thresholds). This is a DOCUMENTED
/// LIMITATION, not a bug: it is exactly why `finding_id` is authored rather than inferred. Do NOT
/// lower `DEDUP_JACCARD_MIN` / `DEDUP_OVERLAP_MIN` to make this pass — that trades a known,
/// documented miss for an unknown number of false positives across the fleet.
#[test]
fn suggest_duplicates_pinned_miss_okf_related_doc_id_is_not_suggested() {
    let entries = vec![
        verdict(
            "mev",
            "okf-related-must-be-a-real-doc-id",
            "The related[] field only accepts a doc_id that resolves to a real document \
             elsewhere in the corpus, but nothing validates that the string actually exists \
             as a target.",
        ),
        verdict(
            "okf-core",
            "okf-core-doc-ids-are-inconsistent-with-filenames",
            "Doc ids inside okf-core are inconsistent with their actual filenames on disk, \
             so a related[] lookup by id silently fails to find the file.",
        ),
    ];

    let suggestions = suggest_duplicates(&entries);

    assert!(
        !contains_pair(
            &suggestions,
            "mev",
            "okf-related-must-be-a-real-doc-id",
            "okf-core",
            "okf-core-doc-ids-are-inconsistent-with-filenames",
        ),
        "this pair is a documented limitation of naive token overlap and must stay a miss: {suggestions:#?}"
    );
}

#[test]
fn suggest_duplicates_never_suggests_an_entry_that_already_has_a_finding_id() {
    let mut a = verdict(
        "mev",
        "already-linked-a",
        "Wave 0 tickets ship without a tasks.json file, leaving the block spec incomplete.",
    );
    a.finding_id = Some("already-confirmed".to_string());
    let mut b = verdict(
        "okf-core",
        "already-linked-b",
        "Ticket specs ship without a tasks.json companion file in okf-core.",
    );
    b.finding_id = Some("already-confirmed".to_string());

    let suggestions = suggest_duplicates(&[a, b]);

    assert!(
        suggestions.is_empty(),
        "entries already carrying an authored finding_id must never be re-suggested"
    );
}

// ---------------------------------------------------------------------------
// End-to-end through `carryover_sweep` — `MV.ticket.carryover-dedup-clusters` task 4.
//
// Follows the temp-dir fixture style of `tests/brain_carryover.rs` (`temp_dir`,
// `write_brain_toml`, two leaf repos), but this file keeps its own copies of those
// helpers since they are private to that file.
// ---------------------------------------------------------------------------

use std::fs;
use std::path::Path;

/// Make a fresh uniquely-named temp dir for a test and return its path.
fn temp_dir(suffix: &str) -> std::path::PathBuf {
    let dir = mev::testsupport::unique_temp_dir(&format!("mev-brain-carryover-dedup-it-{suffix}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `brain.toml` with two leaf repos (alpha, beta) and a standard `[vocab]` block.
fn write_brain_toml(root: &Path) {
    let toml = r#"[vocab]
layer = ["brain", "engine", "factory", "console", "surface", "infra", "business", "content", "meta"]
status = ["active", "draft", "deprecated", "superseded", "archived"]

[crawl]
skip_dirs = ["target", "node_modules", ".git"]

[[repos]]
slug = "alpha"
tier = "primary"
repo_path = "repos/alpha"
status_file = "repos/alpha/planning/status.md"
cache_doc = "docs/projects/alpha.md"
heading = "Alpha"

[[repos]]
slug = "beta"
tier = "primary"
repo_path = "repos/beta"
status_file = "repos/beta/planning/status.md"
cache_doc = "docs/projects/beta.md"
heading = "Beta"
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

/// Serialize `value` as pretty JSON and write it to `root/rel`.
fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(
        &full,
        serde_json::to_string_pretty(value).unwrap().as_bytes(),
    )
    .unwrap();
}

/// Write alpha's leaf state: one entry sharing `finding_id: "shared-nextest-claim"` with
/// beta (the cross-repo cluster case) and one entry with a `finding_id` alpha alone uses
/// (the single-repo typo-guard case).
fn write_alpha_dedup_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [],
        "carryover": [
            {
                "slug": "alpha-nextest-scoped",
                "scope": { "repo": "alpha" },
                "kind": "known_issue",
                "text": "The nextest policy override rule is scoped to this repo only.",
                "created": "2026-06-01",
                "priority": 2,
                "finding_id": "shared-nextest-claim"
            },
            {
                "slug": "alpha-only-finding",
                "scope": { "repo": "alpha" },
                "kind": "constraint",
                "text": "A finding_id authored only in this repo, never linked elsewhere.",
                "created": "2026-06-01",
                "priority": 1,
                "finding_id": "alpha-only-typo-suspect"
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
}

/// Write beta's leaf state: one entry sharing `finding_id: "shared-nextest-claim"` with
/// alpha, at a divergent priority (P0 here vs P2 in alpha) — per the governing design
/// decision, both priorities must render side by side, never reconciled.
fn write_beta_dedup_state(root: &Path) {
    let state = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [],
        "carryover": [
            {
                "slug": "beta-nextest-hook-bail",
                "scope": { "repo": "beta" },
                "kind": "known_issue",
                "text": "The nextest policy override hook does not fire here — a real bail.",
                "created": "2026-06-01",
                "priority": 0,
                "finding_id": "shared-nextest-claim"
            }
        ]
    });
    write_json(root, "repos/beta/planning/state.json", &state);
}

/// Build the complete fixture: brain.toml + alpha leaf + beta leaf.
fn write_dedup_fixture(root: &Path) {
    write_brain_toml(root);
    write_alpha_dedup_state(root);
    write_beta_dedup_state(root);
}

#[test]
fn carryover_sweep_clusters_shared_finding_id_across_repos_and_flags_single_repo_ids() {
    let dir = temp_dir("e2e");
    write_dedup_fixture(&dir);

    let report = mev::carryover_sweep(&dir, None, false).expect("carryover_sweep should not error");

    // The shared finding_id spans both repos and forms exactly one cluster with both
    // members, priorities preserved verbatim (never reconciled).
    let shared_cluster = report
        .clusters
        .iter()
        .find(|c| c.finding_id == "shared-nextest-claim")
        .expect("shared-nextest-claim cluster should be present");
    assert_eq!(shared_cluster.members.len(), 2);
    assert!(!shared_cluster.single_repo);
    assert_eq!(
        shared_cluster.repos,
        vec!["alpha".to_string(), "beta".to_string()]
    );
    let alpha_member = shared_cluster
        .members
        .iter()
        .find(|m| m.repo == "alpha")
        .expect("alpha member should be present");
    let beta_member = shared_cluster
        .members
        .iter()
        .find(|m| m.repo == "beta")
        .expect("beta member should be present");
    assert_eq!(alpha_member.priority, Some(2));
    assert_eq!(beta_member.priority, Some(0));

    // The single-repo finding_id shows up in the typo-guard list, and the shared one does
    // NOT.
    assert!(
        report
            .single_repo_finding_ids
            .contains(&"alpha-only-typo-suspect".to_string()),
        "single-repo finding_id should be flagged, got: {:#?}",
        report.single_repo_finding_ids
    );
    assert!(
        !report
            .single_repo_finding_ids
            .contains(&"shared-nextest-claim".to_string()),
        "cross-repo finding_id must not be flagged as single-repo, got: {:#?}",
        report.single_repo_finding_ids
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// CLI-level: `mev carryover`'s human summary — `MV.ticket.carryover-dedup-clusters`
// task 5. Drives the built binary directly (`CARGO_BIN_EXE_mev`), following the pattern
// `tests/doc_cli.rs` uses, so the UNCONFIRMED heading (not just the note) is genuinely
// exercised through the real print path rather than the library types alone.
// ---------------------------------------------------------------------------

/// Write two leaf repos whose entries carry no `finding_id` but overlap enough on
/// `slug` + `text` tokens to clear `suggest_duplicates`'s accept rule.
fn write_suggestion_fixture(root: &Path) {
    write_brain_toml(root);
    let alpha = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [],
        "carryover": [
            {
                "slug": "wave0-tickets-ship-without-tasks-json",
                "scope": { "repo": "alpha" },
                "kind": "known_issue",
                "text": "Wave 0 tickets ship without a tasks.json file, leaving the block spec incomplete.",
                "created": "2026-06-01"
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &alpha);
    let beta = serde_json::json!({
        "repo": "beta",
        "kind": "project",
        "updated": "2026-08-01",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [],
        "carryover": [
            {
                "slug": "ticket-specs-ship-without-tasks-json",
                "scope": { "repo": "beta" },
                "kind": "known_issue",
                "text": "Ticket specs ship without a tasks.json file, leaving the block underspecified.",
                "created": "2026-06-01"
            }
        ]
    });
    write_json(root, "repos/beta/planning/state.json", &beta);
}

#[test]
fn cli_human_summary_labels_suggestions_unconfirmed() {
    let dir = temp_dir("cli-suggest");
    write_suggestion_fixture(&dir);

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mev"))
        .arg("carryover")
        .arg(&dir)
        .output()
        .expect("mev carryover should run");
    assert!(
        output.status.success(),
        "mev carryover should exit 0 regardless of section contents, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("SUGGESTED DUPLICATES"),
        "expected a SUGGESTED DUPLICATES section, got:\n{stdout}"
    );
    assert!(
        stdout.contains("UNCONFIRMED"),
        "the UNCONFIRMED label must appear whenever suggestions are printed, got:\n{stdout}"
    );
    // The label must be on the heading line itself, not only in the trailing note.
    let heading_line = stdout
        .lines()
        .find(|l| l.contains("SUGGESTED DUPLICATES"))
        .expect("SUGGESTED DUPLICATES heading line should exist");
    assert!(
        heading_line.contains("UNCONFIRMED"),
        "UNCONFIRMED must appear on the heading line itself, got: {heading_line}"
    );

    let _ = fs::remove_dir_all(&dir);
}
