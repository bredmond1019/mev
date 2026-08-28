//! Quiesce-lease reads for mev's corpus-wide write verbs.
//!
//! `MV.ticket.write-verbs-ignore-the-quiesce-lease` Task 1. This module answers exactly one
//! question — given a lock dir, a repo, and the calling agent's identity, is a write about to
//! land inside a sibling lane's declared quiet window? — by reading `<lock_dir>/leases/*.json`
//! against `.claude/workflows/lease.schema.json`. No call site changes here; wiring the answer
//! into `src/main.rs`'s 8 `lock::acquire_lock` sites is Task 2.
//!
//! The refusal rule mirrors `base-template/scripts/fleet_concurrency_check.py`'s
//! `_find_blocking_exclusive_lease` (the same function `register` already uses) exactly:
//! - Only `kind: exclusive` leases can quiesce; `shared` never does.
//! - A lease whose `agent` matches the caller's own `agent` never quiesces that caller (the
//!   self-exemption trap the block record names — this cost the proposing lane a wasted call on
//!   2026-08-23, and this lane another on 2026-08-27, both from a missing `--agent`). A caller
//!   supplying no agent identity can never match, by design: it is refused by any live exclusive
//!   lease, including one it might have written itself.
//! - `scope: fleet` quiesces every repo's write; `scope: repo` (or `scope` absent, which the
//!   schema defines as meaning `repo`) quiesces only a write naming that same repo.
//! - A stale lease never quiesces.
//! - A missing or unlistable `leases/` dir resolves to `Clear`, never a hold — an unreadable
//!   lease store must never wedge the fleet, the same degrade-to-advisory posture
//!   `availability.rs`'s `FleetSlotView::degraded` and the Python script's own `{allowed: true,
//!   degraded: true}` already take for the sibling `.fleet-locks` read.
//! - A malformed / non-JSON individual lease file is skipped, not an error — the same
//!   permissiveness `availability.rs::read_fleet_lock_entries` already applies to ordinary
//!   fleet-lock entries.
//!
//! ## Which TTL governs lease staleness (there are three numbers in the corpus and they disagree)
//!
//! Three "staleness window" constants exist in this fleet and they are NOT the same rule:
//!
//! - **5400s / 90 min** — `fleet_concurrency_check.py`'s own `DEFAULT_TTL_SECONDS`. This governs
//!   *ordinary* `.fleet-locks/*.json` pid-keyed entries (the `register`/`release`/`status`
//!   subcommands), a different record shape with a different liveness signal (pid + `started_at`).
//! - **14400s / 4h** — `availability.rs`'s own `DEFAULT_TTL_SECONDS`, documented there as
//!   "Mirrors `DEFAULT_TTL_SECONDS` in `fleet_concurrency_check.py`" and used by
//!   [`super::availability::is_stale`] for the same ordinary pid-keyed entries. `/begin-orchestration`
//!   step 3's documented "90 minutes" therefore already disagrees with `availability.rs`'s own 4h
//!   — a pre-existing disagreement this module did not create and does not resolve.
//! - **10800s / 3h** — `check_lane_agents.py`'s `STALE_THRESHOLD_SECONDS`, reused via
//!   `lease_liveness_timestamp`/`staleness_seconds` by `fleet_concurrency_check.py`'s
//!   `_non_stale_exclusive_leases` — i.e. **this is the actual, in-production rule the reference
//!   Python implementation uses to judge *lease* staleness.** Leases are a different record kind
//!   from ordinary `.fleet-locks` entries (no `pid`, an `acquired_at`/`heartbeat` pair instead of
//!   `started_at`), so neither of the two numbers above is the right mirror for this module.
//!
//! [`LEASE_STALE_THRESHOLD_SECONDS`] below is therefore **10800s (3h)**, mirroring
//! `check_lane_agents.py::STALE_THRESHOLD_SECONDS` — not `availability.rs`'s constant. Per this
//! ticket's scope, mev is the mirror here, not the authority: `availability.rs` is left
//! untouched (its constant is correct for the record kind it governs), and this module reuses
//! only its time helper ([`super::availability::now_unix_seconds`]), not its TTL value, because
//! that value governs a different record kind than a lease.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::availability::now_unix_seconds;

/// Staleness threshold for an exclusive lease, in seconds (3 hours) — mirrors
/// `check_lane_agents.py::STALE_THRESHOLD_SECONDS` (`180 * 60`), the value that actually governs
/// lease liveness in the reference Python implementation. See the module doc comment for why
/// this is neither of the other two TTL constants in the corpus.
const LEASE_STALE_THRESHOLD_SECONDS: f64 = 180.0 * 60.0;

/// Directory name, under the fleet lock dir, holding individual `lease-*.json` files. Matches
/// `.claude/workflows/lease.schema.json` and `fleet_concurrency_check.py`'s `discover_lease_files`.
const LEASES_SUBDIR: &str = "leases";

/// Raw on-disk shape of one lease file, per `.claude/workflows/lease.schema.json`. Deliberately
/// permissive: every field serde can fail to find or type-match simply fails deserialization for
/// that one file, which the caller already treats as "skip, don't wedge" rather than an error.
#[derive(Debug, Deserialize)]
struct LeaseRaw {
    repo: String,
    lane: String,
    agent: String,
    acquired_at: String,
    #[serde(default)]
    heartbeat: Option<String>,
    kind: String,
    #[serde(default)]
    scope: Option<String>,
}

/// Identity of one lease that quiesces a write — enough for a refusal to name the holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldLease {
    pub lane: String,
    pub agent: String,
    pub repo: String,
    /// Effective scope — `"repo"` or `"fleet"`. A lease with no `scope` on disk resolves here to
    /// `"repo"`, per the schema's documented default; the effective value is what a refusal
    /// message should show, not the raw (possibly absent) field.
    pub scope: String,
    /// Absolute path of the lease file that quiesced the write, for the refusal message.
    pub path: PathBuf,
}

/// The answer to "is a write about to land inside a sibling lane's declared quiet window?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quiesce {
    /// No lease quiesces this write — proceed.
    Clear,
    /// A sibling lane's exclusive lease quiesces this write — refuse, naming the holder.
    Held(HeldLease),
}

/// Parse an RFC 3339 timestamp (the format both `acquired_at` and `heartbeat` use per the
/// schema) into Unix seconds. `None` for anything that does not parse — treated by the caller as
/// unparsable-therefore-stale, never as a wedge.
fn parse_timestamp_seconds(value: &str) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_nanos()) / 1e9)
}

/// The timestamp staleness is judged on: `heartbeat` when present, else `acquired_at` — exactly
/// the rule `lease.schema.json`'s `heartbeat` field documents and `check_lane_agents.py`'s
/// `lease_liveness_timestamp` implements. `acquired_at` is immutable by schema contract, so a
/// long-held lease MUST carry a fresh `heartbeat` to stay live.
fn liveness_timestamp(raw: &LeaseRaw) -> &str {
    raw.heartbeat.as_deref().unwrap_or(&raw.acquired_at)
}

/// Whether `raw`'s liveness timestamp is more than [`LEASE_STALE_THRESHOLD_SECONDS`] old, or
/// unparsable — an unparsable timestamp cannot be judged live, so it is treated as stale rather
/// than as a wedge, the same "unreadable/corrupt: treat as stale" posture the sibling
/// `.fleet-locks` sweep already takes.
fn is_stale(raw: &LeaseRaw, now: f64) -> bool {
    match parse_timestamp_seconds(liveness_timestamp(raw)) {
        Some(ts) => (now - ts) > LEASE_STALE_THRESHOLD_SECONDS,
        None => true,
    }
}

/// Read every `*.json` file directly under `<lock_dir>/leases`, skipping (never erroring on) a
/// file that is not valid JSON or does not match [`LeaseRaw`]'s shape — mirrors
/// `availability.rs::read_fleet_lock_entries`'s permissive read.
///
/// Returns `None` when `<lock_dir>/leases` itself cannot be listed (missing or unreadable) — the
/// caller resolves that to [`Quiesce::Clear`], never a hold. This is the normal case for every
/// repo before any lane has ever taken an exclusive lease.
fn read_lease_files(lock_dir: &Path) -> Option<Vec<(PathBuf, LeaseRaw)>> {
    let leases_dir = lock_dir.join(LEASES_SUBDIR);
    let read_dir = std::fs::read_dir(&leases_dir).ok()?;
    let mut out = Vec::new();
    for dir_entry in read_dir.flatten() {
        let path = dir_entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(raw) = serde_json::from_str::<LeaseRaw>(&contents) {
            out.push((path, raw));
        }
    }
    Some(out)
}

/// Resolve whether `repo`'s write is quiesced by any exclusive lease other than `agent`'s own,
/// reading `<lock_dir>/leases/*.json`. See the module doc comment for the full refusal rule.
///
/// `agent: None` means the caller supplied no identity — such a caller can never be
/// self-exempted and is refused by any live exclusive lease, including one it might have
/// written itself, so the guard cannot be defeated by omitting `--agent`.
pub fn check_quiesce(lock_dir: &Path, repo: &str, agent: Option<&str>) -> Quiesce {
    let Some(entries) = read_lease_files(lock_dir) else {
        return Quiesce::Clear;
    };

    let now = now_unix_seconds();
    for (path, raw) in entries {
        if raw.kind != "exclusive" {
            continue;
        }
        if is_stale(&raw, now) {
            continue;
        }
        if let Some(agent) = agent
            && raw.agent == agent
        {
            continue;
        }
        let scope = raw.scope.clone().unwrap_or_else(|| "repo".to_string());
        if scope != "fleet" && raw.repo != repo {
            continue;
        }
        return Quiesce::Held(HeldLease {
            lane: raw.lane,
            agent: raw.agent,
            repo: raw.repo,
            scope,
            path,
        });
    }

    Quiesce::Clear
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_rfc3339() -> String {
        chrono::Local::now().to_rfc3339()
    }

    /// An RFC 3339 timestamp `secs_ago` seconds in the past, for building stale fixtures.
    fn rfc3339_secs_ago(secs_ago: f64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        let target = now - secs_ago;
        let dt = chrono::DateTime::from_timestamp(target as i64, 0).unwrap();
        dt.to_rfc3339()
    }

    fn write_lease(dir: &Path, name: &str, contents: &serde_json::Value) -> PathBuf {
        let leases_dir = dir.join(LEASES_SUBDIR);
        std::fs::create_dir_all(&leases_dir).unwrap();
        let path = leases_dir.join(name);
        std::fs::write(&path, serde_json::to_string_pretty(contents).unwrap()).unwrap();
        path
    }

    fn lease_json(
        repo: &str,
        lane: &str,
        agent: &str,
        kind: &str,
        scope: Option<&str>,
        acquired_at: &str,
    ) -> serde_json::Value {
        let mut v = serde_json::json!({
            "repo": repo,
            "lane": lane,
            "agent": agent,
            "acquired_at": acquired_at,
            "kind": kind,
        });
        if let Some(scope) = scope {
            v["scope"] = serde_json::Value::String(scope.to_string());
        }
        v
    }

    #[test]
    fn missing_lock_dir_resolves_to_clear() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-missing-dir");
        // Deliberately never created.
        let result = check_quiesce(&dir, "mev", Some("agent-a"));
        assert_eq!(result, Quiesce::Clear);
    }

    #[test]
    fn empty_leases_dir_resolves_to_clear() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-empty-dir");
        std::fs::create_dir_all(dir.join(LEASES_SUBDIR)).unwrap();
        let result = check_quiesce(&dir, "mev", Some("agent-a"));
        assert_eq!(result, Quiesce::Clear);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Positive control (required by the block record's testing strategy): the fixture store
    /// CAN refuse before any test asserts that it does not.
    #[test]
    fn fleet_scope_exclusive_lease_held_by_another_agent_refuses() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-fleet-refuse");
        write_lease(
            &dir,
            "lease-other.json",
            &lease_json(
                "engine-rs",
                "engine-rs-e3",
                "agent-other",
                "exclusive",
                Some("fleet"),
                &now_rfc3339(),
            ),
        );

        let result = check_quiesce(&dir, "mev", Some("agent-a"));
        match result {
            Quiesce::Held(held) => {
                assert_eq!(held.lane, "engine-rs-e3");
                assert_eq!(held.agent, "agent-other");
                assert_eq!(held.repo, "engine-rs");
                assert_eq!(held.scope, "fleet");
            }
            Quiesce::Clear => panic!("expected a fleet-scope exclusive lease to refuse"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_agent_lease_never_refuses_that_agent() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-self-exempt");
        write_lease(
            &dir,
            "lease-self.json",
            &lease_json(
                "mev",
                "mev-lane",
                "agent-a",
                "exclusive",
                Some("fleet"),
                &now_rfc3339(),
            ),
        );

        let result = check_quiesce(&dir, "mev", Some("agent-a"));
        assert_eq!(
            result,
            Quiesce::Clear,
            "the lease holder's own agent must never be refused by its own lease"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unidentified_caller_is_refused_by_any_live_exclusive_lease() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-no-agent");
        write_lease(
            &dir,
            "lease-self.json",
            &lease_json(
                "mev",
                "mev-lane",
                "agent-a",
                "exclusive",
                Some("fleet"),
                &now_rfc3339(),
            ),
        );

        // No --agent supplied: even a lease that "agent-a" itself holds refuses an unidentified
        // caller, since an unidentified caller can never be recognized as the holder.
        let result = check_quiesce(&dir, "mev", None);
        assert!(
            matches!(result, Quiesce::Held(_)),
            "an unidentified caller must be refused by a live exclusive lease"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_lease_does_not_refuse_but_its_fresh_twin_does() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-stale");

        // Fresh twin first: prove the store CAN refuse with an identical shape.
        write_lease(
            &dir,
            "lease-fresh.json",
            &lease_json(
                "mev",
                "mev-lane",
                "agent-other",
                "exclusive",
                Some("fleet"),
                &now_rfc3339(),
            ),
        );
        let fresh_result = check_quiesce(&dir, "mev", Some("agent-a"));
        assert!(
            matches!(fresh_result, Quiesce::Held(_)),
            "positive control: a fresh exclusive lease must refuse"
        );
        std::fs::remove_file(dir.join(LEASES_SUBDIR).join("lease-fresh.json")).unwrap();

        // Same shape, but the liveness timestamp is well past the staleness threshold.
        let stale_acquired_at = rfc3339_secs_ago(LEASE_STALE_THRESHOLD_SECONDS + 3600.0);
        write_lease(
            &dir,
            "lease-stale.json",
            &lease_json(
                "mev",
                "mev-lane",
                "agent-other",
                "exclusive",
                Some("fleet"),
                &stale_acquired_at,
            ),
        );
        let stale_result = check_quiesce(&dir, "mev", Some("agent-a"));
        assert_eq!(
            stale_result,
            Quiesce::Clear,
            "a stale exclusive lease must not refuse"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn heartbeat_overrides_stale_acquired_at() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-heartbeat");
        let stale_acquired_at = rfc3339_secs_ago(LEASE_STALE_THRESHOLD_SECONDS + 3600.0);
        let mut lease = lease_json(
            "mev",
            "mev-lane",
            "agent-other",
            "exclusive",
            Some("fleet"),
            &stale_acquired_at,
        );
        // acquired_at is stale, but a fresh heartbeat keeps the lease live.
        lease["heartbeat"] = serde_json::Value::String(now_rfc3339());
        write_lease(&dir, "lease-heartbeat.json", &lease);

        let result = check_quiesce(&dir, "mev", Some("agent-a"));
        assert!(
            matches!(result, Quiesce::Held(_)),
            "a fresh heartbeat must keep an otherwise-stale-by-acquired_at lease live"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn repo_scope_on_unrelated_repo_does_not_refuse_but_same_repo_does() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-repo-scope");
        write_lease(
            &dir,
            "lease-repo.json",
            &lease_json(
                "engine-rs",
                "engine-rs-lane",
                "agent-other",
                "exclusive",
                Some("repo"),
                &now_rfc3339(),
            ),
        );

        let unrelated = check_quiesce(&dir, "mev", Some("agent-a"));
        assert_eq!(
            unrelated,
            Quiesce::Clear,
            "a repo-scoped lease on an unrelated repo must not refuse"
        );

        let same_repo = check_quiesce(&dir, "engine-rs", Some("agent-a"));
        assert!(
            matches!(same_repo, Quiesce::Held(_)),
            "a repo-scoped lease naming this repo must refuse"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn absent_scope_defaults_to_repo() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-absent-scope");
        write_lease(
            &dir,
            "lease-absent-scope.json",
            &lease_json(
                "engine-rs",
                "engine-rs-lane",
                "agent-other",
                "exclusive",
                None,
                &now_rfc3339(),
            ),
        );

        let unrelated = check_quiesce(&dir, "mev", Some("agent-a"));
        assert_eq!(
            unrelated,
            Quiesce::Clear,
            "scope-absent must default to repo, not fleet"
        );

        let same_repo = check_quiesce(&dir, "engine-rs", Some("agent-a"));
        match same_repo {
            Quiesce::Held(held) => assert_eq!(held.scope, "repo"),
            Quiesce::Clear => panic!("expected the same-repo write to be refused"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_lease_never_refuses_at_either_scope() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-shared");
        write_lease(
            &dir,
            "lease-shared-fleet.json",
            &lease_json(
                "mev",
                "mev-lane",
                "agent-other",
                "shared",
                Some("fleet"),
                &now_rfc3339(),
            ),
        );
        write_lease(
            &dir,
            "lease-shared-repo.json",
            &lease_json(
                "mev",
                "mev-lane",
                "agent-other",
                "shared",
                Some("repo"),
                &now_rfc3339(),
            ),
        );

        let result = check_quiesce(&dir, "mev", Some("agent-a"));
        assert_eq!(result, Quiesce::Clear, "a shared lease must never refuse");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_lease_file_is_skipped_not_wedged() {
        let dir = crate::testsupport::unique_temp_dir("mev-lease-malformed");
        let leases_dir = dir.join(LEASES_SUBDIR);
        std::fs::create_dir_all(&leases_dir).unwrap();
        std::fs::write(leases_dir.join("lease-broken.json"), "{ not valid json").unwrap();
        std::fs::write(
            leases_dir.join("lease-wrong-shape.json"),
            serde_json::to_string(&serde_json::json!({"unexpected": "shape"})).unwrap(),
        )
        .unwrap();
        // A non-JSON file in the directory too, for good measure.
        std::fs::write(leases_dir.join("readme.txt"), "not a lease").unwrap();

        let result = check_quiesce(&dir, "mev", Some("agent-a"));
        assert_eq!(
            result,
            Quiesce::Clear,
            "malformed lease files must be skipped, never wedge the write"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
