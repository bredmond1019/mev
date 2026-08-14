//! Integration tests for `rank_carryover` — the public ordering API
//! (`MV.ticket.carryover-triage-ranking`, task 3).
//!
//! Tests:
//!   1. Full lane ordering over a mixed fixture: BLOCKING, HOT, AGING, STANDING in order.
//!   2. A fresh (non-stale) P0 outranks a 90-day-old P3 — raw age no longer wins.
//!   3. A BLOCKING P3 gating a P0 sorts above a plain HOT P1.
//!   4. Deterministic order across two calls on shuffled input.
//!   5. STANDING is always last.

use mev::brain::carryover::{CarryoverLane, CarryoverVerdict, TriageLane};
use mev::brain::state::{BlockDep, BlockedBy};
use mev::rank_carryover;
use std::collections::HashMap;

/// Build a `CarryoverVerdict` fixture with only the fields `rank_carryover` reads
/// populated meaningfully; the rest carry inert defaults.
fn verdict(
    repo: &str,
    slug: &str,
    priority: Option<u8>,
    stale: bool,
    age_days: Option<i64>,
    blocks: Vec<BlockedBy>,
    lane: CarryoverLane,
) -> CarryoverVerdict {
    CarryoverVerdict {
        repo: repo.to_string(),
        slug: slug.to_string(),
        kind: "known_issue".to_string(),
        text: format!("some text for {slug}"),
        clears_when: None,
        created: "2026-01-01".to_string(),
        age_days,
        stale,
        lane,
        refs: Vec::new(),
        reason: None,
        priority,
        finding_id: None,
        blocks,
    }
}

fn block_edge(repo: &str, id: &str) -> BlockedBy {
    BlockedBy::Block(BlockDep {
        repo: repo.to_string(),
        id: id.to_string(),
        what: None,
    })
}

fn find<'a>(
    ranked: &'a [mev::brain::carryover::CarryoverRanking],
    repo: &str,
    slug: &str,
) -> &'a mev::brain::carryover::CarryoverRanking {
    ranked
        .iter()
        .find(|r| r.repo == repo && r.slug == slug)
        .unwrap_or_else(|| panic!("expected {repo}:{slug} in ranked output"))
}

fn index_of(ranked: &[mev::brain::carryover::CarryoverRanking], repo: &str, slug: &str) -> usize {
    ranked
        .iter()
        .position(|r| r.repo == repo && r.slug == slug)
        .unwrap_or_else(|| panic!("expected {repo}:{slug} in ranked output"))
}

#[test]
fn rank_carryover_full_lane_ordering_over_mixed_fixture() {
    let entries = vec![
        // BLOCKING: gates an open block.
        verdict(
            "mev",
            "gates-open-block",
            Some(3),
            false,
            Some(1),
            vec![block_edge("mev", "MV.1.A")],
            CarryoverLane::Actionable,
        ),
        // HOT: fresh P0, no blocks.
        verdict(
            "mev",
            "fresh-p0",
            Some(0),
            false,
            Some(1),
            Vec::new(),
            CarryoverLane::Actionable,
        ),
        // AGING: stale P2.
        verdict(
            "mev",
            "stale-p2",
            Some(2),
            true,
            Some(60),
            Vec::new(),
            CarryoverLane::Actionable,
        ),
        // STANDING: no priority, no blocks, not stale.
        verdict(
            "mev",
            "standing-rule",
            None,
            false,
            Some(30),
            Vec::new(),
            CarryoverLane::NotEvaluable,
        ),
    ];

    let mut block_status = HashMap::new();
    block_status.insert("mev:MV.1.A".to_string(), Some("open".to_string()));

    let ranked = rank_carryover(&entries, &HashMap::new(), &block_status);

    assert_eq!(ranked.len(), 4);
    assert_eq!(ranked[0].lane, TriageLane::Blocking);
    assert_eq!(ranked[0].slug, "gates-open-block");
    assert_eq!(ranked[1].lane, TriageLane::Hot);
    assert_eq!(ranked[1].slug, "fresh-p0");
    assert_eq!(ranked[2].lane, TriageLane::Aging);
    assert_eq!(ranked[2].slug, "stale-p2");
    assert_eq!(ranked[3].lane, TriageLane::Standing);
    assert_eq!(ranked[3].slug, "standing-rule");
}

#[test]
fn rank_carryover_fresh_p0_outranks_90_day_old_p3() {
    // The whole point of the ticket: raw age must no longer win.
    let entries = vec![
        verdict(
            "mev",
            "old-p3",
            Some(3),
            true,
            Some(90),
            Vec::new(),
            CarryoverLane::Actionable,
        ),
        verdict(
            "mev",
            "fresh-p0",
            Some(0),
            false,
            Some(0),
            Vec::new(),
            CarryoverLane::Actionable,
        ),
    ];

    let ranked = rank_carryover(&entries, &HashMap::new(), &HashMap::new());

    let fresh_idx = index_of(&ranked, "mev", "fresh-p0");
    let old_idx = index_of(&ranked, "mev", "old-p3");
    assert!(
        fresh_idx < old_idx,
        "fresh P0 must outrank a 90-day-old P3, got order {:?}",
        ranked.iter().map(|r| &r.slug).collect::<Vec<_>>()
    );
    assert_eq!(find(&ranked, "mev", "fresh-p0").lane, TriageLane::Hot);
    assert_eq!(find(&ranked, "mev", "old-p3").lane, TriageLane::Aging);
}

#[test]
fn rank_carryover_blocking_p3_gating_p0_sorts_above_plain_hot_p1() {
    let entries = vec![
        // BLOCKING: P3 that gates a hot (P0) block — effective priority 0.
        verdict(
            "mev",
            "blocking-p3",
            Some(3),
            false,
            Some(5),
            vec![block_edge("mev", "MV.1.A")],
            CarryoverLane::Actionable,
        ),
        // HOT: plain P1, no blocks.
        verdict(
            "mev",
            "plain-hot-p1",
            Some(1),
            false,
            Some(5),
            Vec::new(),
            CarryoverLane::Actionable,
        ),
    ];

    let mut block_priorities = HashMap::new();
    block_priorities.insert("mev:MV.1.A".to_string(), 0u8);
    let mut block_status = HashMap::new();
    block_status.insert("mev:MV.1.A".to_string(), Some("open".to_string()));

    let ranked = rank_carryover(&entries, &block_priorities, &block_status);

    let blocking_idx = index_of(&ranked, "mev", "blocking-p3");
    let hot_idx = index_of(&ranked, "mev", "plain-hot-p1");
    assert!(
        blocking_idx < hot_idx,
        "BLOCKING lane must sort above HOT lane regardless of within-lane priority"
    );
    assert_eq!(
        find(&ranked, "mev", "blocking-p3").effective_priority,
        Some(0)
    );
}

#[test]
fn rank_carryover_deterministic_order_across_shuffled_input() {
    let a = verdict(
        "mev",
        "a",
        Some(0),
        false,
        Some(3),
        Vec::new(),
        CarryoverLane::Actionable,
    );
    let b = verdict(
        "engine-rs",
        "b",
        Some(1),
        false,
        Some(10),
        Vec::new(),
        CarryoverLane::Actionable,
    );
    let c = verdict(
        "mev",
        "c",
        Some(2),
        true,
        Some(45),
        Vec::new(),
        CarryoverLane::Actionable,
    );
    let d = verdict(
        "bastion",
        "d",
        None,
        false,
        Some(2),
        Vec::new(),
        CarryoverLane::NotEvaluable,
    );

    let order1 = vec![a.clone(), b.clone(), c.clone(), d.clone()];
    let order2 = vec![d, c, a, b];

    let ranked1 = rank_carryover(&order1, &HashMap::new(), &HashMap::new());
    let ranked2 = rank_carryover(&order2, &HashMap::new(), &HashMap::new());

    let keys1: Vec<(String, String)> = ranked1
        .iter()
        .map(|r| (r.repo.clone(), r.slug.clone()))
        .collect();
    let keys2: Vec<(String, String)> = ranked2
        .iter()
        .map(|r| (r.repo.clone(), r.slug.clone()))
        .collect();
    assert_eq!(
        keys1, keys2,
        "order must be identical regardless of input order"
    );
}

#[test]
fn rank_carryover_standing_is_last() {
    let entries = vec![
        verdict(
            "mev",
            "standing",
            None,
            false,
            Some(500),
            Vec::new(),
            CarryoverLane::NotEvaluable,
        ),
        verdict(
            "mev",
            "aging",
            Some(3),
            true,
            Some(1),
            Vec::new(),
            CarryoverLane::Actionable,
        ),
        verdict(
            "mev",
            "hot",
            Some(1),
            false,
            Some(1),
            Vec::new(),
            CarryoverLane::Actionable,
        ),
        verdict(
            "mev",
            "blocking",
            Some(3),
            false,
            Some(1),
            vec![block_edge("mev", "MV.1.A")],
            CarryoverLane::Actionable,
        ),
    ];

    let mut block_status = HashMap::new();
    block_status.insert("mev:MV.1.A".to_string(), Some("open".to_string()));

    let ranked = rank_carryover(&entries, &HashMap::new(), &block_status);

    assert_eq!(ranked.last().unwrap().slug, "standing");
    assert_eq!(ranked.last().unwrap().lane, TriageLane::Standing);
}
