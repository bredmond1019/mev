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
