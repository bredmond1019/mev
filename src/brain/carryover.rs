//! Fleet-wide `carryover[]` sweep — model + predicate extraction.
//!
//! `mev carryover` (see `planning/ticket-carryover-sweep-command/tasks.md`) reads every
//! discovered `planning/state.json`'s `carryover[]` array and, where `clears_when` is
//! machine-checkable, evaluates it, sorting the fleet into three lanes: `cleared`
//! (safe to delete), `actionable` (predicate false, with the unmet reference named),
//! and `not-evaluable` (the predicate is prose, or there is no predicate at all).
//!
//! This module owns the read-only report model ([`CarryoverReport`],
//! [`CarryoverVerdict`], [`CarryoverRef`], [`CarryoverLane`], [`NotEvaluableReason`])
//! and the pure, independently-testable predicate extractors:
//! [`block_refs_from_related`], [`block_refs_from_prose`], and
//! [`path_refs_from_prose`]. The evaluator that assigns a lane to each entry
//! (`evaluate_carryover`) and the `carryover_sweep` driver are separate follow-on
//! work; this module intentionally stops at "extract the references".
//!
//! # The two evaluable predicate classes — deliberately narrow
//!
//! **Class A — block references.** Two sources, both resolved against the corpus:
//! 1. `related[]` entries with `type == "block"` — structured, zero parsing risk.
//!    Always used ([`block_refs_from_related`]).
//! 2. Block IDs matched in the `clears_when` prose by a strict grammar
//!    ([`block_refs_from_prose`]): `[A-Z]{2,3}\.(?:\d+\.[A-Z0-9]+|ticket\.[a-z0-9][a-z0-9-]*|chore\.[a-z0-9][a-z0-9-]*)`.
//!    A match is kept only when it resolves to exactly one node in the loaded corpus
//!    (preferring the carryover's own scope repo when the bare ID is ambiguous; if
//!    still ambiguous across repos, the match is dropped and the ambiguity is
//!    reported instead). An unresolvable token is not a block reference — discarded
//!    silently.
//!
//! **Class B — path existence.** Only when `clears_when` contains the literal word
//! `exists` ([`path_refs_from_prose`]).
//!
//! No `regex` dependency is used or added — the grammar is small and fixed, so it is
//! matched by hand (char scanning) in [`extract_block_id_tokens`].

use std::collections::HashSet;

use serde::Serialize;

use okf_core::{BlockedBy, Carryover};

// ---------------------------------------------------------------------------
// Report model
// ---------------------------------------------------------------------------

/// Which of the three lanes a `carryover[]` entry landed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CarryoverLane {
    /// At least one reference was extracted and every one of them is satisfied —
    /// safe to delete (a recommendation for a human, never automatic).
    Cleared,
    /// At least one reference was extracted, but at least one is unsatisfied.
    Actionable,
    /// No reference could be extracted (or there is no `clears_when` at all).
    NotEvaluable,
}

/// Why an entry could not be evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotEvaluableReason {
    /// `clears_when` is present but no reference (block or path) could be
    /// extracted from it — it is pure prose.
    Prose,
    /// `clears_when` is `None`.
    NoPredicate,
    /// A bare block ID in the prose matched nodes in more than one repo and did
    /// not resolve to the carryover's own scope repo either — dropped rather
    /// than guessed at.
    AmbiguousReference,
}

/// One extracted, resolved reference and whether it is currently satisfied.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CarryoverRef {
    /// A block reference, keyed `"{repo}:{id}"`. Satisfied when the referenced
    /// block's authored status is `"closed"`.
    Block { key: String, satisfied: bool },
    /// A path-existence reference. Satisfied when the path resolves to an
    /// existing file (relative to the brain root or the owning repo's path).
    Path { path: String, satisfied: bool },
}

/// The evaluated verdict for a single `carryover[]` entry.
#[derive(Debug, Clone, Serialize)]
pub struct CarryoverVerdict {
    pub repo: String,
    pub slug: String,
    pub kind: String,
    pub text: String,
    pub clears_when: Option<String>,
    pub created: String,
    /// `None` when the entry is currently snoozed (staleness clock paused) or
    /// has no parseable anchor date; `Some(age_days)` otherwise.
    pub age_days: Option<i64>,
    /// Derived from [`crate::brain::state::carryover_stale_age`] against the
    /// kind's threshold — `true` when that call returned `Some`.
    pub stale: bool,
    pub lane: CarryoverLane,
    pub refs: Vec<CarryoverRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<NotEvaluableReason>,
}

/// The full fleet-wide sweep result.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CarryoverReport {
    pub total: usize,
    pub cleared: usize,
    pub actionable: usize,
    pub not_evaluable: usize,
    pub entries: Vec<CarryoverVerdict>,
}

// ---------------------------------------------------------------------------
// Class A — block references
// ---------------------------------------------------------------------------

/// Block references from a carryover's structured `related[]` edges.
///
/// Every `related[]` entry with `type == "block"` (i.e. `BlockedBy::Block`) becomes
/// a `"{repo}:{id}"` key. `BlockedBy::External` edges are not block references and
/// are skipped. An edge whose `repo` is empty falls back to the carryover's own
/// scope repo (`item.scope.repo`); if that is also absent the edge is skipped
/// rather than keyed with an empty repo.
pub fn block_refs_from_related(item: &Carryover) -> Vec<String> {
    item.related
        .iter()
        .filter_map(|edge| match edge {
            BlockedBy::Block { repo, id, .. } => {
                let repo = if repo.is_empty() {
                    item.scope.repo.clone()?
                } else {
                    repo.clone()
                };
                Some(format!("{repo}:{id}"))
            }
            BlockedBy::External { .. } => None,
        })
        .collect()
}

/// Block ID tokens matched in free text by the grammar
/// `[A-Z]{2,3}\.(?:\d+\.[A-Z0-9]+|ticket\.[a-z0-9][a-z0-9-]*|chore\.[a-z0-9][a-z0-9-]*)`.
///
/// Hand-scanned (no `regex` dependency). Matching is greedy and non-overlapping:
/// once a match is found starting at a position, the scan resumes immediately
/// after it. A run of 4+ uppercase letters is rejected as a prefix (it is not a
/// bounded 2-3 letter tag), which also prevents `is_ascii_uppercase` prefixes
/// from swallowing unrelated all-caps text.
fn extract_block_id_tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if let Some(end) = match_block_id_at(&chars, i) {
            out.push(chars[i..end].iter().collect());
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// Attempt a single grammar match starting exactly at `start`; returns the
/// exclusive end index on success.
fn match_block_id_at(chars: &[char], start: usize) -> Option<usize> {
    let n = chars.len();

    // Prefix: 2-3 uppercase ASCII letters, not followed by a 4th (that would
    // make it an unbounded run, not a tag).
    let mut p = start;
    let mut letters = 0;
    while p < n && chars[p].is_ascii_uppercase() && letters < 3 {
        p += 1;
        letters += 1;
    }
    if letters < 2 {
        return None;
    }
    if p < n && chars[p].is_ascii_uppercase() {
        return None;
    }
    if p >= n || chars[p] != '.' {
        return None;
    }
    p += 1; // consume '.'

    // Branch 1: \d+\.[A-Z0-9]+
    let digit_start = p;
    let mut dp = p;
    while dp < n && chars[dp].is_ascii_digit() {
        dp += 1;
    }
    if dp > digit_start && dp < n && chars[dp] == '.' {
        let alnum_start = dp + 1;
        let mut ap = alnum_start;
        while ap < n && (chars[ap].is_ascii_uppercase() || chars[ap].is_ascii_digit()) {
            ap += 1;
        }
        if ap > alnum_start {
            return Some(ap);
        }
    }

    // Branch 2/3: ("ticket." | "chore.") [a-z0-9][a-z0-9-]*
    for kw in ["ticket.", "chore."] {
        let kw_chars: Vec<char> = kw.chars().collect();
        let kw_end = p + kw_chars.len();
        if kw_end <= n && chars[p..kw_end] == kw_chars[..] {
            let lower_start = kw_end;
            let mut lp = lower_start;
            if lp < n && (chars[lp].is_ascii_lowercase() || chars[lp].is_ascii_digit()) {
                lp += 1;
                while lp < n
                    && (chars[lp].is_ascii_lowercase()
                        || chars[lp].is_ascii_digit()
                        || chars[lp] == '-')
                {
                    lp += 1;
                }
                return Some(lp);
            }
        }
    }

    None
}

/// Block references matched in `clears_when` prose, resolved against the loaded
/// corpus's known `"{repo}:{id}"` keys.
///
/// A grammar match is kept only when it resolves to exactly one key in
/// `known_keys`. When it matches keys in more than one repo, the carryover's own
/// scope repo (`own_repo`) is preferred if present among the matches; otherwise
/// the match is dropped and the returned `bool` (ambiguous) is set to `true`. A
/// grammar match with zero resolving keys is not a block reference at all — it
/// is discarded silently (no ambiguity signal).
///
/// Returns `(resolved_keys, ambiguous)`.
pub fn block_refs_from_prose(
    clears_when: &str,
    own_repo: Option<&str>,
    known_keys: &HashSet<String>,
) -> (Vec<String>, bool) {
    let mut refs = Vec::new();
    let mut ambiguous = false;

    for token in extract_block_id_tokens(clears_when) {
        let suffix = format!(":{token}");
        let matches: Vec<&String> = known_keys.iter().filter(|k| k.ends_with(&suffix)).collect();
        match matches.len() {
            0 => {} // unresolvable — not a block reference, discard silently
            1 => refs.push(matches[0].clone()),
            _ => {
                if let Some(repo) = own_repo {
                    let preferred = format!("{repo}{suffix}");
                    if matches.iter().any(|k| **k == preferred) {
                        refs.push(preferred);
                        continue;
                    }
                }
                ambiguous = true;
            }
        }
    }

    (refs, ambiguous)
}

// ---------------------------------------------------------------------------
// Class B — path existence
// ---------------------------------------------------------------------------

/// File extensions a path-existence token may end in.
const PATH_EXTENSIONS: &[&str] = &["md", "rs", "py", "sh", "ts", "tsx", "json", "toml"];

/// Path tokens matched in `clears_when` prose — only when the text contains the
/// literal word `exists` (case-insensitive, word-bounded). Every
/// whitespace-delimited token containing `/` and ending in one of
/// [`PATH_EXTENSIONS`] is returned, trimmed of surrounding punctuation/quotes.
/// Returns `[]` when the word `exists` is absent.
pub fn path_refs_from_prose(clears_when: &str) -> Vec<String> {
    let has_exists = clears_when
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.eq_ignore_ascii_case("exists"));
    if !has_exists {
        return Vec::new();
    }

    clears_when
        .split_whitespace()
        .filter_map(|tok| {
            let trimmed = tok.trim_matches(|c: char| !c.is_alphanumeric());
            if !trimmed.contains('/') {
                return None;
            }
            let ext = trimmed.rsplit('.').next()?;
            if PATH_EXTENSIONS.contains(&ext) {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use okf_core::CarryoverScope;

    fn scope(repo: Option<&str>) -> CarryoverScope {
        CarryoverScope {
            repo: repo.map(str::to_string),
            tier: None,
            cross_repo: None,
        }
    }

    fn carryover(related: Vec<BlockedBy>, own_repo: Option<&str>) -> Carryover {
        Carryover {
            slug: "test-slug".to_string(),
            scope: scope(own_repo),
            kind: "deferred".to_string(),
            text: "some text".to_string(),
            related,
            clears_when: None,
            created: "2026-01-01".to_string(),
            reviewed: None,
            snoozed_until: None,
        }
    }

    // -- block_refs_from_related ------------------------------------------

    #[test]
    fn block_refs_from_related_keys_block_edges_repo_and_id() {
        let item = carryover(
            vec![BlockedBy::Block {
                repo: "engine-rs".to_string(),
                id: "EN.5.B1".to_string(),
                what: None,
            }],
            None,
        );
        assert_eq!(block_refs_from_related(&item), vec!["engine-rs:EN.5.B1"]);
    }

    #[test]
    fn block_refs_from_related_skips_external_edges() {
        let item = carryover(
            vec![BlockedBy::External {
                what: "waiting on vendor API".to_string(),
            }],
            None,
        );
        assert!(block_refs_from_related(&item).is_empty());
    }

    #[test]
    fn block_refs_from_related_falls_back_to_own_scope_repo() {
        let item = carryover(
            vec![BlockedBy::Block {
                repo: String::new(),
                id: "MV.3.A".to_string(),
                what: None,
            }],
            Some("mev"),
        );
        assert_eq!(block_refs_from_related(&item), vec!["mev:MV.3.A"]);
    }

    #[test]
    fn block_refs_from_related_skips_edge_with_no_repo_anywhere() {
        let item = carryover(
            vec![BlockedBy::Block {
                repo: String::new(),
                id: "MV.3.A".to_string(),
                what: None,
            }],
            None,
        );
        assert!(block_refs_from_related(&item).is_empty());
    }

    // -- block_refs_from_prose ----------------------------------------------

    #[test]
    fn block_refs_from_prose_resolves_ticket_style_id() {
        let mut known = HashSet::new();
        known.insert("base-template:BT.ticket.gate-skip-count-regression".to_string());
        let (refs, ambiguous) = block_refs_from_prose(
            "BT.ticket.gate-skip-count-regression ships in base-template",
            None,
            &known,
        );
        assert_eq!(
            refs,
            vec!["base-template:BT.ticket.gate-skip-count-regression"]
        );
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_resolves_multiple_slash_separated_ids() {
        let mut known = HashSet::new();
        known.insert("engine-rs:EN.5.B1".to_string());
        known.insert("engine-rs:EN.5.B2".to_string());
        let (mut refs, ambiguous) = block_refs_from_prose("EN.5.B1/EN.5.B2 land", None, &known);
        refs.sort();
        assert_eq!(refs, vec!["engine-rs:EN.5.B1", "engine-rs:EN.5.B2"]);
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_resolves_single_letter_block_id() {
        let mut known = HashSet::new();
        known.insert("bastion-web:BW.8.N".to_string());
        let (refs, ambiguous) = block_refs_from_prose("BW.8.N lands", None, &known);
        assert_eq!(refs, vec!["bastion-web:BW.8.N"]);
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_drops_unresolvable_token_silently() {
        let known = HashSet::new();
        let (refs, ambiguous) = block_refs_from_prose("EN.5.B1 lands", None, &known);
        assert!(refs.is_empty());
        assert!(!ambiguous, "an unresolvable token is not an ambiguity");
    }

    #[test]
    fn block_refs_from_prose_no_resolvable_ids_for_bare_chore_prefix() {
        // Real example from the ticket: "MV.chore" (no trailing ".xxx") does not
        // match the grammar's chore branch at all, so this yields zero tokens —
        // correctly not-evaluable.
        let mut known = HashSet::new();
        known.insert("mev:MV.chore.unique-temp-dirs-in-tests".to_string());
        known.insert("bastion:BE.chore.unique-temp-dirs-in-tests".to_string());
        let (refs, ambiguous) = block_refs_from_prose(
            "MV.chore and BE.chore unique-temp-dirs-in-tests both land",
            None,
            &known,
        );
        assert!(refs.is_empty(), "expected no resolvable IDs, got {refs:?}");
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_flags_ambiguity_across_repos() {
        let mut known = HashSet::new();
        known.insert("repo-a:MV.3.A".to_string());
        known.insert("repo-b:MV.3.A".to_string());
        let (refs, ambiguous) = block_refs_from_prose("MV.3.A lands", None, &known);
        assert!(refs.is_empty());
        assert!(ambiguous);
    }

    #[test]
    fn block_refs_from_prose_prefers_own_repo_when_ambiguous() {
        let mut known = HashSet::new();
        known.insert("repo-a:MV.3.A".to_string());
        known.insert("repo-b:MV.3.A".to_string());
        let (refs, ambiguous) = block_refs_from_prose("MV.3.A lands", Some("repo-b"), &known);
        assert_eq!(refs, vec!["repo-b:MV.3.A"]);
        assert!(!ambiguous);
    }

    #[test]
    fn block_refs_from_prose_rejects_four_letter_prefix() {
        let mut known = HashSet::new();
        known.insert("x:ABCD.1.A".to_string());
        let (refs, ambiguous) = block_refs_from_prose("ABCD.1.A lands", None, &known);
        assert!(refs.is_empty());
        assert!(!ambiguous);
    }

    // -- path_refs_from_prose -------------------------------------------------

    #[test]
    fn path_refs_from_prose_empty_when_no_exists_word() {
        assert!(path_refs_from_prose("docs/decisions/D58-foo.md is linked").is_empty());
    }

    #[test]
    fn path_refs_from_prose_extracts_paths_when_exists_present() {
        let text = "docs/decisions/D58-us-market-entry-and-two-domain-split.md exists and is \
                     linked from docs/decisions/index.md";
        let refs = path_refs_from_prose(text);
        assert_eq!(
            refs,
            vec![
                "docs/decisions/D58-us-market-entry-and-two-domain-split.md",
                "docs/decisions/index.md",
            ]
        );
    }

    #[test]
    fn path_refs_from_prose_ignores_tokens_without_slash_or_known_extension() {
        let text = "exists check: README exists but foo.exe/bar does not count";
        let refs = path_refs_from_prose(text);
        assert!(refs.is_empty(), "expected no matches, got {refs:?}");
    }

    #[test]
    fn path_refs_from_prose_trims_surrounding_punctuation_and_quotes() {
        let text = "exists: \"docs/index.md\", (planning/status.md).";
        let refs = path_refs_from_prose(text);
        assert_eq!(refs, vec!["docs/index.md", "planning/status.md"]);
    }
}
