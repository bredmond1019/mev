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
