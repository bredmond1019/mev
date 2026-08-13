//! The Attention-board operator payload type (`MV.ticket.attention-queue-delivery` task 2).
//!
//! Emits each Attention-board row as an `EN.8.A`-compatible operator payload:
//! a stable `item_id`, a `gate_id`, a self-contained `rendered_summary`, a
//! small fixed set of response `options`, and a `digest` computed over the
//! rendered summary + options — the same digest-binding discipline as
//! `engine-core`'s `OperatorPayload::new` (`core/engine-rs/crates/engine-core/src/operator/payload.rs`),
//! so a changed payload re-queues instead of silently executing stale
//! content.
//!
//! This module only *derives and emits* the payload — it never reads or
//! writes `engine-core`'s queue state, never opens a notification channel,
//! and never writes to `state.json`. See the ticket's "Two boundaries this
//! spec does not cross" section.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::brain::carryover::TriageLane;
use crate::brain::emit::AttentionRow;

/// `engine-core`'s WhatsApp body text limit
/// (`core/engine-rs/crates/engine-core/src/operator/limits.rs`'s
/// `WHATSAPP_MAX_BODY_CHARS`, confirmed 2026-08-12) — the ceiling
/// [`render_summary`] must stay under.
const MAX_SUMMARY_CHARS: usize = 1024;

/// Truncation width applied to the free-text portion of [`render_summary`],
/// leaving headroom under [`MAX_SUMMARY_CHARS`] for the repo/lane/kind/
/// slug/age/priority scaffolding that surrounds it. Reuses the same
/// truncation helper the board render already uses
/// (`attention_snippet`, src/brain/emit.rs:1415) rather than adding a
/// second truncation rule.
const SUMMARY_TEXT_SNIPPET_CHARS: usize = 400;

/// Maximum response options a single payload may carry — `engine-core`'s
/// `WHATSAPP_MAX_REPLY_BUTTONS`
/// (`core/engine-rs/crates/engine-core/src/operator/limits.rs`, confirmed
/// against Meta's Cloud API docs 2026-08-12). A payload that cannot fit is
/// rejected rather than falling back to a list message — the gate must
/// declare the `session` channel instead.
pub(crate) const MAX_RESPONSE_OPTIONS: usize = 3;

/// Minimum response options a payload must carry — `engine-core`'s
/// `OPERATOR_MIN_RESPONSE_OPTIONS`
/// (`core/engine-rs/crates/engine-core/src/operator/limits.rs`, confirmed
/// 2026-08-12).
pub(crate) const MIN_RESPONSE_OPTIONS: usize = 2;

/// Maximum characters (not bytes — labels may contain non-ASCII) an
/// operator-visible option label may carry — `engine-core`'s button
/// label-length limit
/// (`core/engine-rs/crates/engine-core/src/operator/limits.rs`, confirmed
/// 2026-08-12).
pub(crate) const MAX_LABEL_CHARS: usize = 20;

/// Which board lane an [`AttentionRow`] belongs to, re-derived from its
/// existing fields (`AttentionRow` carries no lane-category field of its own
/// for the backlog/capture/distilled lanes — see below).
///
/// The four carryover triage lanes are read directly off `row.lane`
/// (populated whenever a row came from `collect_attention_rows`). The other
/// three lanes are `None` there and are told apart by the exact field
/// values `render_attention_section_with_distilled` (src/brain/emit.rs)
/// constructs them with:
/// - **Distilled**: `slug` empty, `kind` is `"knowledge"` or `"memory"` (the
///   distilled-file stem).
/// - **Capture**: `kind` empty (capture rows never set `kind`).
/// - **Backlog**: `kind` is the backlog node's `status`, one of `"idea"` /
///   `"ready"` / `"promoted"` (`VALID_BACKLOG_STATUSES`, src/brain/state.rs)
///   — never empty and never `"knowledge"`/`"memory"`, so this arm is
///   reached only for genuine backlog rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowLane {
    Blocking,
    Hot,
    Aging,
    Standing,
    Backlog,
    Capture,
    Distilled,
}

impl RowLane {
    /// Categorize a row per the field-correspondence documented on
    /// [`RowLane`] itself.
    #[must_use]
    pub(crate) fn for_row(row: &AttentionRow) -> Self {
        if let Some(lane) = row.lane {
            return match lane {
                TriageLane::Blocking => RowLane::Blocking,
                TriageLane::Hot => RowLane::Hot,
                TriageLane::Aging => RowLane::Aging,
                TriageLane::Standing => RowLane::Standing,
            };
        }
        if row.slug.is_empty() && (row.kind == "knowledge" || row.kind == "memory") {
            RowLane::Distilled
        } else if row.kind.is_empty() {
            RowLane::Capture
        } else {
            RowLane::Backlog
        }
    }

    /// Stable, lowercase machine tag for this lane — part of [`item_id_for`]'s
    /// identity input. Never changes once assigned; a rename here would
    /// silently mint new `item_id`s for every existing row in that lane.
    #[must_use]
    fn tag(self) -> &'static str {
        match self {
            RowLane::Blocking => "blocking",
            RowLane::Hot => "hot",
            RowLane::Aging => "aging",
            RowLane::Standing => "standing",
            RowLane::Backlog => "backlog",
            RowLane::Capture => "capture",
            RowLane::Distilled => "distilled",
        }
    }

    /// Operator-visible, uppercase label for this lane, used in
    /// [`render_summary`] — matches the casing of the board's own `##
    /// BLOCKING` / `## HOT` / `## AGING` / `## STANDING` headings
    /// (`render_triage_lane`, src/brain/emit.rs) for the four carryover
    /// lanes, extended consistently for the other three.
    #[must_use]
    fn label(self) -> &'static str {
        match self {
            RowLane::Blocking => "BLOCKING",
            RowLane::Hot => "HOT",
            RowLane::Aging => "AGING",
            RowLane::Standing => "STANDING",
            RowLane::Backlog => "BACKLOG",
            RowLane::Capture => "CAPTURE",
            RowLane::Distilled => "DISTILLED",
        }
    }
}

/// Derive a stable identifier for `row` from its IDENTITY fields only —
/// repo, lane (task 4's "lane kind"), and slug — never from mutable content
/// (age, text, priority). Re-running on an unchanged corpus reproduces this
/// exactly; an item whose text/age/priority changed keeps the same
/// `item_id` and gets a new [`AttentionQueuePayload::digest`] instead — a
/// re-queue rather than a new item, per `EN.8.A`.
///
/// Hashed (rather than a plain delimited string) so that a slug or repo
/// name containing the delimiter can never collide with a different
/// repo/lane/slug triple.
#[must_use]
pub(crate) fn item_id_for(row: &AttentionRow) -> String {
    let mut hasher = Sha256::new();
    hasher.update(row.repo.as_bytes());
    hasher.update(b"\0");
    hasher.update(RowLane::for_row(row).tag().as_bytes());
    hasher.update(b"\0");
    hasher.update(row.slug.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Render a self-contained decision text for `row`: repo, lane, kind, slug,
/// age, the item's text, and the effective priority where present — an
/// operator reading only this must be able to decide without opening the
/// repo (task 4's Acceptance Criteria).
///
/// Reuses [`attention_snippet`](crate::brain::emit::attention_snippet) for
/// the free-text truncation rather than adding a second truncation rule,
/// and stays within `engine-core`'s [`MAX_SUMMARY_CHARS`] body limit.
/// Absent data stays absent: a `None` age renders the board's existing `—`
/// convention (`render_triage_lane`, src/brain/emit.rs), never a fabricated
/// `0d` — this repo's standing honesty rule (see the same discipline in
/// `last_touched.rs`).
#[must_use]
pub(crate) fn render_summary(row: &AttentionRow) -> String {
    let lane = RowLane::for_row(row);
    let age = row
        .age
        .map(|a| format!("{a}d"))
        .unwrap_or_else(|| "—".to_string());
    let text =
        crate::brain::emit::attention_snippet(&row.title_or_text, SUMMARY_TEXT_SNIPPET_CHARS);

    let mut summary = format!(
        "[{}] {} kind={} slug={} age={age} — {text}",
        row.repo,
        lane.label(),
        row.kind,
        row.slug,
    );

    match (row.priority, row.effective_priority) {
        (Some(p), Some(ep)) if p == ep => summary.push_str(&format!(" [P{p}]")),
        (Some(p), Some(ep)) => summary.push_str(&format!(" [P{p} -> effective P{ep}]")),
        (Some(p), None) => summary.push_str(&format!(" [P{p}]")),
        (None, Some(ep)) => summary.push_str(&format!(" [effective P{ep}]")),
        (None, None) => {}
    }

    if summary.chars().count() > MAX_SUMMARY_CHARS {
        let truncated: String = summary.chars().take(MAX_SUMMARY_CHARS - 1).collect();
        summary = format!("{}…", truncated.trim_end());
    }

    summary
}

/// The `session` option every lane's set includes for actions that did not
/// fit in the ≤3-option budget — routes the operator to the full triage
/// surface rather than dropping capability silently.
fn session_option() -> AttentionResponseOption {
    AttentionResponseOption::new("session", "Open session")
}

/// Assign the response option set for one Attention-board row, enforced
/// against `engine-core`'s 3-button / 20-character cap (task 3 of
/// `MV.ticket.attention-queue-delivery`).
///
/// Per-lane behavior, all required by the ticket's Acceptance Criteria:
/// - **Distilled**: re-affirm (bumps `freshness:`) + session. Never `snooze`
///   — HQ `CLAUDE.md`'s Attention rule: the distilled lane is "re-affirmed by
///   bumping `freshness:`, never snoozed".
/// - **Standing** (carryover): keep + session only. Never `promote` or
///   `resolve` — its entries are permanently-true constraints
///   (`TriageLane::Standing`'s own doc comment).
/// - **Blocking / Hot / Aging** (carryover): promote, snooze, session.
/// - **Backlog / Capture**: promote, snooze, session.
#[must_use]
pub(crate) fn options_for(row: &AttentionRow) -> Vec<AttentionResponseOption> {
    let options = match RowLane::for_row(row) {
        RowLane::Distilled => vec![
            AttentionResponseOption::new("reaffirm", "Re-affirm"),
            session_option(),
        ],
        RowLane::Standing => vec![
            AttentionResponseOption::new("keep", "Keep"),
            session_option(),
        ],
        RowLane::Blocking | RowLane::Hot | RowLane::Aging | RowLane::Backlog | RowLane::Capture => {
            vec![
                AttentionResponseOption::new("promote", "Promote"),
                AttentionResponseOption::new("snooze", "Snooze"),
                session_option(),
            ]
        }
    };
    assert_valid_options(&options);
    options
}

/// Enforce the 2..=3 count and ≤20-character label caps in code, rather than
/// only by convention — a set outside the bounds is a bug the constructor
/// rejects rather than emits.
fn assert_valid_options(options: &[AttentionResponseOption]) {
    assert!(
        (MIN_RESPONSE_OPTIONS..=MAX_RESPONSE_OPTIONS).contains(&options.len()),
        "option set must have {MIN_RESPONSE_OPTIONS}..={MAX_RESPONSE_OPTIONS} options, got {}",
        options.len()
    );
    for opt in options {
        assert!(
            opt.label.chars().count() <= MAX_LABEL_CHARS,
            "option label {:?} exceeds {MAX_LABEL_CHARS} characters",
            opt.label
        );
    }
}

/// One named response option offered to the operator — e.g. `("promote",
/// "Promote")`. `key` is the stable machine identifier a response resolves
/// against; `label` is the operator-visible text and is what the digest and
/// the channel's 20-character label-length limit apply to.
///
/// Field names and `rename_all = "snake_case"` are chosen so this type's
/// serialized form deserializes unchanged into `engine-core`'s
/// `OperatorResponseOption`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AttentionResponseOption {
    /// Stable machine identifier for this option, e.g. `"promote"`.
    pub(crate) key: String,
    /// Operator-visible label rendered in the channel, e.g. `"Promote"`.
    pub(crate) label: String,
}

impl AttentionResponseOption {
    /// Construct a named response option.
    pub(crate) fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
        }
    }
}

/// One Attention-board item, emitted as a validated operator payload.
///
/// Field names and `rename_all = "snake_case"` are chosen so the
/// `gate_id`/`rendered_summary`/`options`/`digest` subset of this type's
/// serialized form deserializes unchanged into `engine-core`'s
/// `OperatorPayload` — the contract this block emits into. `item_id`,
/// `effective_priority`, `lane`, `repo`, and `source` are mev-owned fields
/// the queue (`EN.8.B`) uses for ordering and provenance; they ride
/// alongside the `OperatorPayload` subset rather than inside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AttentionQueuePayload {
    /// Stable identity for this item — derived from repo/lane-kind/slug only
    /// (task 4), never from mutable content, so re-running on an unchanged
    /// corpus reproduces it and a changed item keeps its `item_id` but gets a
    /// new `digest` (a re-queue, per `EN.8.A`).
    pub(crate) item_id: String,
    /// Identity of the gate that produced this payload — `EN.8.A`'s
    /// `OperatorPayload::gate_id`. Deliberately excluded from the digest, per
    /// that type's doc comment.
    pub(crate) gate_id: String,
    /// The inline rendered summary the operator sees — self-contained: repo,
    /// lane, slug, age, and the item's text (task 4).
    pub(crate) rendered_summary: String,
    /// The small fixed set of named response options (2..=3, task 3).
    pub(crate) options: Vec<AttentionResponseOption>,
    /// Digest over `rendered_summary` + `options`, computed at construction
    /// by [`AttentionQueuePayload::new`] so the two can never be built out of
    /// sync — reproduces `engine-core`'s `OperatorPayload::digest_of` exactly.
    pub(crate) digest: String,
    /// The effective priority (post `blocks[]` min-propagation) this item was
    /// ranked at on the board, supplied to the queue rather than recomputed
    /// by it — `EN.8.B`'s `OperatorQueueItem::effective_priority` is
    /// enqueuer-supplied, never queue-computed.
    pub(crate) effective_priority: Option<u8>,
    /// Which Attention-board lane this item came from.
    pub(crate) lane: Option<TriageLane>,
    /// The repo this item belongs to.
    pub(crate) repo: String,
    /// Provenance of this payload, for the second consumer identified by the
    /// roadmap (reviewing what `emit-state --write` changed) — not wired up
    /// in this block, but present so the shape does not need to change later.
    pub(crate) source: String,
}

impl AttentionQueuePayload {
    /// Construct a payload, computing `digest` from `rendered_summary` and
    /// `options` so the two can never be built out of sync — same discipline
    /// as `engine-core`'s `OperatorPayload::new`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        item_id: impl Into<String>,
        gate_id: impl Into<String>,
        rendered_summary: impl Into<String>,
        options: Vec<AttentionResponseOption>,
        effective_priority: Option<u8>,
        lane: Option<TriageLane>,
        repo: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        let rendered_summary = rendered_summary.into();
        let digest = Self::digest_of(&rendered_summary, &options);
        Self {
            item_id: item_id.into(),
            gate_id: gate_id.into(),
            rendered_summary,
            options,
            digest,
            effective_priority,
            lane,
            repo: repo.into(),
            source: source.into(),
        }
    }

    /// Compute the digest a rendered summary + option set would carry,
    /// independent of any particular payload instance.
    ///
    /// REPRODUCES `engine-core`'s `OperatorPayload::digest_of` exactly
    /// (`core/engine-rs/crates/engine-core/src/operator/payload.rs`, the
    /// authority for this algorithm): SHA-256 over, in order, the summary
    /// length as `u64::to_le_bytes`, the summary bytes, the option count as
    /// `u64::to_le_bytes`, then per option the key length, key bytes, label
    /// length, label bytes — all lengths little-endian `u64`. Formatted as
    /// lowercase hex. This exact reproduction is the cross-repo pin: if
    /// either side's algorithm drifts, the hard-coded digest test below
    /// fails loudly rather than the queue silently re-queueing every payload
    /// forever.
    #[must_use]
    pub(crate) fn digest_of(rendered_summary: &str, options: &[AttentionResponseOption]) -> String {
        let mut hasher = Sha256::new();
        hasher.update((rendered_summary.len() as u64).to_le_bytes());
        hasher.update(rendered_summary.as_bytes());
        hasher.update((options.len() as u64).to_le_bytes());
        for opt in options {
            hasher.update((opt.key.len() as u64).to_le_bytes());
            hasher.update(opt.key.as_bytes());
            hasher.update((opt.label.len() as u64).to_le_bytes());
            hasher.update(opt.label.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `AttentionRow` matching the exact field values
    /// `render_attention_section_with_distilled` constructs for each lane —
    /// see [`RowLane`]'s doc comment.
    fn row_for(lane: RowLane) -> AttentionRow {
        let (row_lane, kind, slug) = match lane {
            RowLane::Blocking => (
                Some(TriageLane::Blocking),
                "carryover".to_string(),
                "s".to_string(),
            ),
            RowLane::Hot => (
                Some(TriageLane::Hot),
                "carryover".to_string(),
                "s".to_string(),
            ),
            RowLane::Aging => (
                Some(TriageLane::Aging),
                "carryover".to_string(),
                "s".to_string(),
            ),
            RowLane::Standing => (
                Some(TriageLane::Standing),
                "carryover".to_string(),
                "s".to_string(),
            ),
            RowLane::Backlog => (None, "idea".to_string(), "s".to_string()),
            RowLane::Capture => (None, String::new(), "s".to_string()),
            RowLane::Distilled => (None, "knowledge".to_string(), String::new()),
        };
        AttentionRow {
            repo: "mev".to_string(),
            age: Some(10),
            kind,
            slug,
            title_or_text: "text".to_string(),
            priority: None,
            effective_priority: None,
            lane: row_lane,
            clears_when: None,
        }
    }

    const ALL_LANES: [RowLane; 7] = [
        RowLane::Blocking,
        RowLane::Hot,
        RowLane::Aging,
        RowLane::Standing,
        RowLane::Backlog,
        RowLane::Capture,
        RowLane::Distilled,
    ];

    #[test]
    fn row_lane_for_row_matches_construction() {
        for lane in ALL_LANES {
            assert_eq!(RowLane::for_row(&row_for(lane)), lane);
        }
    }

    #[test]
    fn every_lane_option_set_is_within_bounds_exhaustively() {
        for lane in ALL_LANES {
            let options = options_for(&row_for(lane));
            assert!(
                (MIN_RESPONSE_OPTIONS..=MAX_RESPONSE_OPTIONS).contains(&options.len()),
                "lane {lane:?} produced {} options",
                options.len()
            );
            for opt in &options {
                assert!(
                    opt.label.chars().count() <= MAX_LABEL_CHARS,
                    "lane {lane:?} option {:?} exceeds {MAX_LABEL_CHARS} chars",
                    opt.label
                );
            }
        }
    }

    #[test]
    fn distilled_lane_never_offers_snooze() {
        let options = options_for(&row_for(RowLane::Distilled));
        assert!(
            !options.iter().any(|o| o.key == "snooze"),
            "distilled lane must never offer snooze (HQ CLAUDE.md)"
        );
    }

    #[test]
    fn distilled_lane_offers_reaffirm() {
        let options = options_for(&row_for(RowLane::Distilled));
        assert!(options.iter().any(|o| o.key == "reaffirm"));
    }

    #[test]
    fn standing_lane_never_offers_promote_or_resolve() {
        let options = options_for(&row_for(RowLane::Standing));
        assert!(
            !options
                .iter()
                .any(|o| o.key == "promote" || o.key == "resolve"),
            "Standing carryover lane entries are permanent constraints; promote/resolve are meaningless"
        );
    }

    #[test]
    fn backlog_and_capture_lanes_may_offer_promote() {
        for lane in [RowLane::Backlog, RowLane::Capture] {
            let options = options_for(&row_for(lane));
            assert!(
                options.iter().any(|o| o.key == "promote"),
                "lane {lane:?} should offer promote"
            );
        }
    }

    #[test]
    #[should_panic(expected = "exceeds")]
    fn over_long_label_is_rejected() {
        assert_valid_options(&[
            AttentionResponseOption::new("a", "This label is definitely way too long"),
            session_option(),
        ]);
    }

    fn approve_reject() -> Vec<AttentionResponseOption> {
        vec![
            AttentionResponseOption::new("promote", "Promote"),
            AttentionResponseOption::new("session", "Open session"),
        ]
    }

    /// Cross-repo pin: this hex digest must match what `engine-core`'s
    /// `OperatorPayload::digest_of("same summary", approve_reject())` would
    /// compute for the identical summary + options. If this test starts
    /// failing after touching either side's hashing code, the two
    /// implementations have drifted — fix the drift, do not update the
    /// hard-coded hex to make the test pass.
    #[test]
    fn digest_matches_engine_core_hard_coded_hex() {
        let digest = AttentionQueuePayload::digest_of("same summary", &approve_reject());
        assert_eq!(
            digest,
            "af870cf3e86f6a309b68b489f8c4fed6e5df5fd882ee6f6c3473d53964f1d5c4"
        );
    }

    #[test]
    fn changed_summary_changes_digest() {
        let a = AttentionQueuePayload::digest_of("summary A", &approve_reject());
        let b = AttentionQueuePayload::digest_of("summary B", &approve_reject());
        assert_ne!(a, b);
    }

    #[test]
    fn changed_option_label_changes_digest() {
        let a = AttentionQueuePayload::digest_of("same summary", &approve_reject());
        let mut changed = approve_reject();
        changed[0].label = "Promoted".to_string();
        let b = AttentionQueuePayload::digest_of("same summary", &changed);
        assert_ne!(a, b);
    }

    #[test]
    fn reordered_options_changes_digest() {
        let mut reordered = approve_reject();
        reordered.reverse();
        let a = AttentionQueuePayload::digest_of("same summary", &approve_reject());
        let b = AttentionQueuePayload::digest_of("same summary", &reordered);
        assert_ne!(a, b);
    }

    #[test]
    fn digest_independent_of_gate_id() {
        let a = AttentionQueuePayload::new(
            "item-1",
            "gate-1",
            "same summary",
            approve_reject(),
            Some(1),
            Some(TriageLane::Hot),
            "mev",
            "attention-board",
        );
        let b = AttentionQueuePayload::new(
            "item-1",
            "gate-2",
            "same summary",
            approve_reject(),
            Some(1),
            Some(TriageLane::Hot),
            "mev",
            "attention-board",
        );
        assert_eq!(
            a.digest, b.digest,
            "digest must depend only on rendered summary + options, not gate_id"
        );
    }

    #[test]
    fn item_id_is_stable_across_two_calls() {
        let row = row_for(RowLane::Blocking);
        assert_eq!(item_id_for(&row), item_id_for(&row));
    }

    #[test]
    fn item_id_unchanged_when_only_text_or_age_differ() {
        let mut row = row_for(RowLane::Blocking);
        let base_id = item_id_for(&row);

        row.title_or_text = "a completely different text".to_string();
        row.age = Some(999);
        row.priority = Some(3);
        row.effective_priority = Some(1);

        assert_eq!(
            item_id_for(&row),
            base_id,
            "item_id must depend only on repo/lane/slug, never mutable content"
        );
    }

    #[test]
    fn item_id_differs_when_only_slug_differs() {
        let mut row_a = row_for(RowLane::Backlog);
        row_a.slug = "slug-a".to_string();
        let mut row_b = row_for(RowLane::Backlog);
        row_b.slug = "slug-b".to_string();

        assert_ne!(item_id_for(&row_a), item_id_for(&row_b));
    }

    #[test]
    fn item_id_differs_across_lanes_for_same_repo_and_slug() {
        let row_hot = row_for(RowLane::Hot);
        let row_aging = row_for(RowLane::Aging);
        assert_ne!(item_id_for(&row_hot), item_id_for(&row_aging));
    }

    #[test]
    fn render_summary_contains_repo_slug_and_age() {
        let mut row = row_for(RowLane::Hot);
        row.slug = "my-slug".to_string();
        row.age = Some(42);

        let summary = render_summary(&row);
        assert!(summary.contains("mev"), "summary missing repo: {summary}");
        assert!(
            summary.contains("my-slug"),
            "summary missing slug: {summary}"
        );
        assert!(summary.contains("42d"), "summary missing age: {summary}");
    }

    #[test]
    fn render_summary_none_age_renders_em_dash() {
        let mut row = row_for(RowLane::Hot);
        row.age = None;
        let summary = render_summary(&row);
        assert!(
            summary.contains('—'),
            "None age must render the `—` convention, never a fabricated 0d: {summary}"
        );
        assert!(!summary.contains("0d"), "must not fabricate 0d: {summary}");
    }

    #[test]
    fn render_summary_very_long_text_stays_within_body_limit() {
        let mut row = row_for(RowLane::Hot);
        row.title_or_text = "x".repeat(5000);
        let summary = render_summary(&row);
        assert!(
            summary.chars().count() <= MAX_SUMMARY_CHARS,
            "summary exceeds {MAX_SUMMARY_CHARS} chars: {}",
            summary.chars().count()
        );
    }

    #[test]
    fn constructor_never_builds_summary_and_digest_out_of_sync() {
        let payload = AttentionQueuePayload::new(
            "item-1",
            "gate-1",
            "original",
            approve_reject(),
            None,
            None,
            "mev",
            "attention-board",
        );
        assert_eq!(
            payload.digest,
            AttentionQueuePayload::digest_of(&payload.rendered_summary, &payload.options)
        );
    }
}
