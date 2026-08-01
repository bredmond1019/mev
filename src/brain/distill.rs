//! D35-distilled entry parsing + the single shared staleness predicate.
//!
//! `knowledge.md` / `memory.md` files hold D35-format entries:
//!
//! ```text
//! - **<claim / fact / convention / lesson>**
//!   source: <path> · date: <ISO> · supersedes: <prior-entry | —> · freshness: <ISO>
//! ```
//!
//! (the `amistad` scope uses a `  - source:` variant instead of `  source:`). The
//! `freshness:` stamp is currently write-only — nothing reads it. This module is the read
//! side: a hand-rolled line scanner (mirroring [`crate::brain::links::extract_links`]'s
//! convention — no `pulldown-cmark`/`comrak`/`regex` dependency) that recovers each entry's
//! claim + dates, plus [`distill_stale_age`], the single predicate shared by the
//! `validate-brain` `W_DISTILL_STALE` warning and the `emit-state` Attention board's "Stale
//! distilled knowledge" lane.

use std::path::Path;

use chrono::NaiveDate;

use crate::Diagnostic;
use crate::brain::config::AttentionThresholds;

/// One D35-distilled entry, hand-parsed from a `knowledge.md` / `memory.md` body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistilledEntry {
    /// The bold claim text, recovered by walking backwards from the `source:` line to the
    /// nearest line starting with `- **`. A claim that wraps across multiple lines is
    /// joined into one string (each line trimmed, joined with a single space).
    pub claim: String,
    /// The entry's authored `date:` field, if present and it parsed as an ISO
    /// (`YYYY-MM-DD`) date.
    pub date: Option<NaiveDate>,
    /// The entry's authored `freshness:` field, if present and it parsed as an ISO date.
    /// `None` for entries whose stamp did not parse — including the `freshness: <as-of>`
    /// template placeholder that appears in D35's own prose.
    pub freshness: Option<NaiveDate>,
    /// 1-indexed line number of the `source:` line this entry was parsed from (for
    /// diagnostics).
    pub line: usize,
}

/// Parse every D35-distilled entry out of `contents` (a `knowledge.md` / `memory.md` body).
///
/// A hand-rolled LINE SCANNER — deliberately not a Markdown parser. A line is accepted as an
/// entry's provenance line when its trimmed form starts with `source:` or `- source:` (the
/// two live shapes: plain `  source:` and the amistad-scope `  - source:`) AND the line
/// contains `· freshness: `. Fields are recovered by splitting the line on `·` and matching
/// each segment's `key:` prefix.
///
/// The claim is recovered by walking BACKWARDS from the provenance line to the nearest line
/// starting with `- **` (claims can wrap across multiple lines before the provenance line).
///
/// Entries whose `freshness:` value does not parse as an ISO date are skipped entirely —
/// this self-excludes the `freshness: <as-of>` template placeholders in D35's own prose with
/// no special-casing.
pub fn parse_distilled(contents: &str) -> Vec<DistilledEntry> {
    let lines: Vec<&str> = contents.lines().collect();
    let mut entries = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let is_provenance_line = trimmed.starts_with("source:") || trimmed.starts_with("- source:");
        if !is_provenance_line || !line.contains("· freshness: ") {
            continue;
        }

        let mut date = None;
        let mut freshness = None;
        for part in line.split('·') {
            let part = part.trim();
            if let Some(rest) = part.strip_prefix("date:") {
                date = NaiveDate::parse_from_str(rest.trim(), "%Y-%m-%d").ok();
            } else if let Some(rest) = part.strip_prefix("freshness:") {
                freshness = NaiveDate::parse_from_str(rest.trim(), "%Y-%m-%d").ok();
            }
        }

        // Unparseable freshness (e.g. the `<as-of>` template placeholder) self-excludes
        // the entry — no special-casing needed.
        let Some(freshness) = freshness else {
            continue;
        };

        // Walk backwards to the nearest `- **` line to recover the (possibly wrapped) claim.
        let mut claim_start = None;
        let mut i = idx;
        while i > 0 {
            i -= 1;
            if lines[i].trim_start().starts_with("- **") {
                claim_start = Some(i);
                break;
            }
        }
        let claim = match claim_start {
            Some(start) => lines[start..idx]
                .iter()
                .map(|l| l.trim())
                .collect::<Vec<_>>()
                .join(" "),
            None => String::new(),
        };

        entries.push(DistilledEntry {
            claim,
            date,
            freshness: Some(freshness),
            line: idx + 1,
        });
    }

    entries
}

/// The staleness verdict for a D35-distilled entry: `Some(age_days)` when the entry's
/// anchor date is more than `stem`'s threshold days in the past, else `None`. `stem` is the
/// source file's stem (`"knowledge"` or `"memory"`), used to look up the right threshold via
/// [`AttentionThresholds::distill_threshold`].
///
/// This is the **single** predicate shared by the `validate-brain` `W_DISTILL_STALE`
/// warning and the `emit-state` Attention board's "Stale distilled knowledge" lane, so the
/// board shows exactly what the warning fires on.
///
/// The anchor is `max(date, freshness)` (defensive — `freshness >= date` holds for every live
/// entry today). The comparison is strictly `>`, not `>=` (matches
/// [`crate::brain::state::carryover_stale_age`]'s convention): an entry exactly at the
/// threshold is not yet stale.
///
/// An entry whose `freshness` did not parse can never age — there is no fallback anchor once
/// it is absent, mirroring [`crate::brain::state::backlog_stale_age`]'s "no parseable date ⇒
/// never stale" rule. There is deliberately no snooze branch: entries have no stable id
/// (their identity is mutable prose, unlike `carryover[]`/`backlog[]` nodes), bumping
/// `freshness:` already *is* the re-affirmation a snooze would otherwise provide, and the
/// emit layer only ever writes generated content between sentinels — never into hand-authored
/// body prose — so a snooze marker would have nowhere durable to live.
pub fn distill_stale_age(
    entry: &DistilledEntry,
    today: NaiveDate,
    thresholds: &AttentionThresholds,
    stem: &str,
) -> Option<i64> {
    let freshness = entry.freshness?;
    let anchor = match entry.date {
        Some(date) => date.max(freshness),
        None => freshness,
    };
    let age = (today - anchor).num_days();
    (age > thresholds.distill_threshold(stem)).then_some(age)
}

/// `W_DISTILL_STALE` warnings for the `knowledge.md` / `memory.md` siblings of
/// `planning_dir`, one per D35-distilled entry whose [`distill_stale_age`] exceeds its
/// file's threshold.
///
/// A missing `knowledge.md` or `memory.md` is a **silent skip** — not a warning — because
/// `base-template/scaffold/planning/` legitimately has neither file. WARNING severity
/// only; this never flips the exit code (mirrors the `carryover`/`backlog` staleness
/// contract at [`crate::brain::state::carryover_stale_age`]). The locator is
/// `W_DISTILL_STALE` — a single shared locator, not a knowledge/memory pair, because the
/// diagnostic's `file` path already names which of the two files is stale.
///
/// This is the `validate-brain` counterpart to the `emit-state` Attention board's "Stale
/// distilled knowledge" lane — both read the exact same [`distill_stale_age`] predicate,
/// so the board never shows an entry the warning didn't also fire on.
pub fn check_distill_staleness(
    planning_dir: &Path,
    today: NaiveDate,
    thresholds: &AttentionThresholds,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    for stem in ["knowledge", "memory"] {
        let path = planning_dir.join(format!("{stem}.md"));
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue; // missing file -> silent skip, not a warning
        };

        for entry in parse_distilled(&contents) {
            let Some(age) = distill_stale_age(&entry, today, thresholds, stem) else {
                continue;
            };
            let threshold = thresholds.distill_threshold(stem);
            let claim = if entry.claim.is_empty() {
                "(no claim text found)".to_string()
            } else {
                entry.claim.clone()
            };
            diags.push(Diagnostic::warning(
                &path,
                "W_DISTILL_STALE",
                format!(
                    "distilled entry at line {} is {age}d old (threshold {threshold}d): {claim} \
                     — re-affirm it (bump 'freshness'), supersede it, or archive it",
                    entry.line
                ),
            ));
        }
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    // -- parse_distilled -----------------------------------------------------------------

    #[test]
    fn parses_plain_source_prefix() {
        let contents = "\
- **The sky is blue.** Some elaboration.
  source: log.md · date: 2026-06-26 · supersedes: — · freshness: 2026-06-27
";
        let entries = parse_distilled(contents);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].claim, "- **The sky is blue.** Some elaboration.");
        assert_eq!(entries[0].date, Some(date("2026-06-26")));
        assert_eq!(entries[0].freshness, Some(date("2026-06-27")));
        assert_eq!(entries[0].line, 2);
    }

    #[test]
    fn parses_amistad_dash_source_prefix() {
        let contents = "\
- **Tech stack: Expo/React Native + Supabase.** Details here.
  - source: planning/decisions/D2.md · date: 2026-06-22 · supersedes: — · freshness: 2026-06-27
";
        let entries = parse_distilled(contents);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].freshness, Some(date("2026-06-27")));
        assert_eq!(entries[0].date, Some(date("2026-06-22")));
    }

    #[test]
    fn both_prefix_shapes_parse_in_same_file() {
        let contents = "\
- **Plain-prefix claim.** Body text.
  source: a.md · date: 2026-06-01 · supersedes: — · freshness: 2026-06-02

- **Dash-prefix claim.** Body text.
  - source: b.md · date: 2026-06-03 · supersedes: — · freshness: 2026-06-04
";
        let entries = parse_distilled(contents);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].freshness, Some(date("2026-06-02")));
        assert_eq!(entries[1].freshness, Some(date("2026-06-04")));
    }

    #[test]
    fn wrapped_claim_resolves_to_full_bold_text() {
        let contents = "\
- **Geolocation social app for traveler-local friendship.** The app connects visitors in a city with
  locals or new-to-city residents who share activity interests. Core loop: auth to profile
  (visitor/local/new-to-city mode) to map discovery to interest match to 1:1 chat to report/block.
  - source: planning/context.md · date: 2026-06-22 · supersedes: — · freshness: 2026-06-27
";
        let entries = parse_distilled(contents);
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0]
                .claim
                .contains("Geolocation social app for traveler-local friendship.")
        );
        assert!(entries[0].claim.contains("locals or new-to-city residents"));
        assert!(entries[0].claim.contains("report/block."));
    }

    #[test]
    fn template_placeholder_freshness_yields_no_entry() {
        let contents = "\
>   source: <path> · date: <ISO> · supersedes: <prior | —> · freshness: <as-of>
";
        let entries = parse_distilled(contents);
        assert!(
            entries.is_empty(),
            "expected no entries for the <as-of> template placeholder, got {entries:?}"
        );
    }

    #[test]
    fn line_missing_freshness_field_is_skipped() {
        let contents = "\
- **Some claim.** Body.
  source: a.md · date: 2026-06-01 · supersedes: —
";
        let entries = parse_distilled(contents);
        assert!(entries.is_empty());
    }

    #[test]
    fn no_preceding_bold_claim_line_yields_empty_claim() {
        let contents = "\
  source: a.md · date: 2026-06-01 · supersedes: — · freshness: 2026-06-02
";
        let entries = parse_distilled(contents);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].claim, "");
    }

    // -- distill_stale_age ----------------------------------------------------------------

    fn entry(date_s: &str, freshness_s: Option<&str>) -> DistilledEntry {
        DistilledEntry {
            claim: "claim".to_string(),
            date: Some(date(date_s)),
            freshness: freshness_s.map(date),
            line: 1,
        }
    }

    #[test]
    fn exactly_at_threshold_is_not_stale() {
        let thresholds = AttentionThresholds::default(); // knowledge_days = 45
        let e = entry("2026-06-01", Some("2026-06-01"));
        let today = date("2026-06-01") + chrono::Duration::days(45);
        assert_eq!(distill_stale_age(&e, today, &thresholds, "knowledge"), None);
    }

    #[test]
    fn one_day_past_threshold_is_stale() {
        let thresholds = AttentionThresholds::default();
        let e = entry("2026-06-01", Some("2026-06-01"));
        let today = date("2026-06-01") + chrono::Duration::days(46);
        assert_eq!(
            distill_stale_age(&e, today, &thresholds, "knowledge"),
            Some(46)
        );
    }

    #[test]
    fn memory_uses_its_own_shorter_threshold() {
        let thresholds = AttentionThresholds::default(); // memory_days = 30
        let e = entry("2026-06-01", Some("2026-06-01"));
        let today = date("2026-06-01") + chrono::Duration::days(31);
        assert_eq!(
            distill_stale_age(&e, today, &thresholds, "memory"),
            Some(31)
        );
    }

    #[test]
    fn absent_freshness_can_never_age() {
        let thresholds = AttentionThresholds::default();
        let e = DistilledEntry {
            claim: "claim".to_string(),
            date: Some(date("2020-01-01")),
            freshness: None,
            line: 1,
        };
        let today = date("2026-08-01");
        assert_eq!(distill_stale_age(&e, today, &thresholds, "knowledge"), None);
    }

    #[test]
    fn anchor_uses_max_of_date_and_freshness() {
        // freshness later than date -> anchor is freshness; not yet stale relative to it.
        let thresholds = AttentionThresholds::default();
        let e = entry("2020-01-01", Some("2026-07-01"));
        let today = date("2026-07-10"); // 9 days past freshness, well under 45
        assert_eq!(distill_stale_age(&e, today, &thresholds, "knowledge"), None);
    }

    #[test]
    fn missing_date_falls_back_to_freshness_only() {
        let thresholds = AttentionThresholds::default();
        let e = DistilledEntry {
            claim: "claim".to_string(),
            date: None,
            freshness: Some(date("2026-06-01")),
            line: 1,
        };
        let today = date("2026-06-01") + chrono::Duration::days(46);
        assert_eq!(
            distill_stale_age(&e, today, &thresholds, "knowledge"),
            Some(46)
        );
    }

    #[test]
    fn unknown_stem_falls_back_to_longer_threshold() {
        let thresholds = AttentionThresholds::default(); // knowledge 45, memory 30 -> max 45
        let e = entry("2026-06-01", Some("2026-06-01"));
        let today = date("2026-06-01") + chrono::Duration::days(31); // past memory's 30, under knowledge's 45
        assert_eq!(distill_stale_age(&e, today, &thresholds, "weird"), None);
    }
}
