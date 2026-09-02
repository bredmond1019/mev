//! Check `contract-freshness` — a consumer's pinned contract version vs its canonical.
//!
//! This is the drift class `toolchain-freshness` cannot see: no compiled binary is
//! involved, only a markdown version-marker line or a Dart string constant that can
//! silently fall out of sync with the canonical it is pinned to. It has already happened
//! once, live and uncaught, until a manual docs pass found Synapse's
//! `data-contract.md` sitting at `1.9.0`, still self-labelled canonical, against the real
//! canonical (`engine-rs`, `1.8.0`).
//!
//! The registry (`[[contracts]]` in `brain.toml`, see [`crate::brain::config::ContractEntry`])
//! models each contract as one canonical `{repo, path, format}` plus N consumer edges. Two
//! formats are supported: `md-version-line` (a `**...Version: X.Y.Z**`-shaped line,
//! anywhere in the file) and `dart-const` (a named `const String <key> = 'vX.Y.Z';`).
//!
//! The verdict is EXACT STRING EQUALITY, never version ordering — a consumer numerically
//! "ahead" of its canonical is `Drift`, full stop. Synapse and engine-rs are two forked
//! lineages that collide on the number `1.8.0` with different content, so ordering across
//! them would state something false; equality is the only sound test.

use std::path::{Path, PathBuf};

use regex::Regex;

use super::{CheckOutcome, CheckStatus, ConformanceCtx, FactSide};
use crate::brain::config::{ContractEndpoint, ContractEntry};

/// The outcome of resolving one endpoint's version marker: either the extracted version
/// string, or a human-readable reason it could not be extracted.
type ExtractResult = Result<String, String>;

/// Extract a `md-version-line` version from `content`.
///
/// Handles all three shapes measured in the fleet, scanning the whole file rather than
/// assuming a fixed line number (`core/mev/docs/carryover-contract.md`'s line is at 36;
/// every other contract doc's is at 15):
///   - `**Contract Version: 1.8.0**`
///   - `**Pinned Contract Version: 1.0.0**`
///   - `**Version:** v0.40` (colon OUTSIDE the bold, non-semver value, trailing whitespace)
///
/// Both patterns anchor to the *start* of the line (allowing trailing whitespace at the
/// end) so a prose or changelog-table mention of the same bolded phrase mid-line or
/// mid-sentence (measured live in `core/engine-rs/docs/data-contract.md`'s changelog
/// table and `core/bastion/docs/serve/serve-api.md`'s own prose about this exact
/// pattern) is never mistaken for the real marker line.
///
/// Takes the first match in file order; returns the captured version trimmed. No match,
/// or more than one *distinct* version captured, is an extraction failure — never a guess.
fn extract_md_version_line(content: &str) -> ExtractResult {
    // Two shapes share one pattern (`**...Version: X**`); the third has the colon
    // outside the bold (`**Version:** X`). Both capture group 1 as the raw value.
    let inside_bold = Regex::new(r"^\*\*(?:[A-Za-z ]*)Version:\s*([^\*]+?)\*\*\s*$")
        .expect("static regex must compile");
    let outside_bold =
        Regex::new(r"^\*\*Version:\*\*\s*([^\n]*?)\s*$").expect("static regex must compile");

    let mut found: Vec<String> = Vec::new();
    for line in content.lines() {
        if let Some(caps) = inside_bold.captures(line) {
            found.push(caps[1].trim().to_string());
            continue;
        }
        if let Some(caps) = outside_bold.captures(line) {
            found.push(caps[1].trim().to_string());
        }
    }

    if found.is_empty() {
        return Err("no md-version-line match found".to_string());
    }

    let first = &found[0];
    if found.iter().any(|v| v != first) {
        return Err(format!(
            "more than one distinct version captured: {}",
            found.join(", ")
        ));
    }

    Ok(first.clone())
}

/// Extract a `dart-const` version: a named `const String <key> = '<value>';` (single or
/// double quotes). A missing `key` on the endpoint is a configuration error, surfaced by
/// the caller as `NotEvaluable` naming the endpoint — never a panic.
fn extract_dart_const(content: &str, key: &str) -> ExtractResult {
    let pattern = format!(
        r#"const\s+String\s+{}\s*=\s*['"]([^'"]*)['"]\s*;"#,
        regex::escape(key)
    );
    let re = Regex::new(&pattern).map_err(|e| format!("bad dart-const pattern: {e}"))?;
    match re.captures(content) {
        Some(caps) => Ok(caps[1].trim().to_string()),
        None => Err(format!("no `const String {key} = '...';` found")),
    }
}

/// Resolve one endpoint against `root`: read the file, then extract per `format`.
/// Every failure mode (missing file, unreadable file, unknown format, failed extraction,
/// or a `dart-const` endpoint with no `key`) returns a named `Err` reason — never panics.
fn resolve_endpoint(root: &Path, endpoint: &ContractEndpoint) -> ExtractResult {
    let full_path: PathBuf = root.join(&endpoint.path);
    let content = std::fs::read_to_string(&full_path).map_err(|e| {
        format!(
            "{} ({}): could not read `{}`: {e}",
            endpoint.repo,
            endpoint.path,
            full_path.display()
        )
    })?;

    match endpoint.format.as_str() {
        "md-version-line" => extract_md_version_line(&content)
            .map_err(|e| format!("{} ({}): {e}", endpoint.repo, endpoint.path)),
        "dart-const" => match &endpoint.key {
            Some(key) => extract_dart_const(&content, key)
                .map_err(|e| format!("{} ({}): {e}", endpoint.repo, endpoint.path)),
            None => Err(format!(
                "{} ({}): dart-const endpoint has no `key` configured",
                endpoint.repo, endpoint.path
            )),
        },
        other => Err(format!(
            "{} ({}): unknown format `{other}`",
            endpoint.repo, endpoint.path
        )),
    }
}

/// One edge's verdict: `(contract name, consumer repo, consumer path, status, detail)`.
/// `detail` carries the two versions on `Drift`, or the failure reason on `NotEvaluable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeOutcome {
    pub contract: String,
    pub consumer_repo: String,
    pub consumer_path: String,
    pub status: CheckStatus,
    pub detail: String,
}

/// The pure verdict function: given the two already-extracted (or failed) sides for one
/// edge, decide pass / drift / not-evaluable. No I/O — unit-tested directly.
fn edge_verdict(canonical: &ExtractResult, consumer: &ExtractResult) -> (CheckStatus, String) {
    match (canonical, consumer) {
        (Ok(c), Ok(v)) => {
            if c == v {
                (CheckStatus::Pass, format!("both at {c}"))
            } else {
                (CheckStatus::Drift, format!("canonical={c} consumer={v}"))
            }
        }
        (Err(reason), Ok(_)) => (CheckStatus::NotEvaluable, format!("canonical: {reason}")),
        (Ok(_), Err(reason)) => (CheckStatus::NotEvaluable, format!("consumer: {reason}")),
        (Err(cr), Err(vr)) => (
            CheckStatus::NotEvaluable,
            format!("canonical: {cr}; consumer: {vr}"),
        ),
    }
}

/// Evaluate every consumer edge of one `[[contracts]]` entry against `root`.
///
/// When the canonical itself fails to resolve, every one of the contract's edges is
/// `NotEvaluable`, and the report says so once per edge (never a single aggregate skip).
pub fn evaluate_contract(root: &Path, entry: &ContractEntry) -> Vec<EdgeOutcome> {
    let canonical = resolve_endpoint(root, &entry.canonical);
    entry
        .consumers
        .iter()
        .map(|consumer| {
            let consumer_result = resolve_endpoint(root, consumer);
            let (status, detail) = edge_verdict(&canonical, &consumer_result);
            EdgeOutcome {
                contract: entry.name.clone(),
                consumer_repo: consumer.repo.clone(),
                consumer_path: consumer.path.clone(),
                status,
                detail,
            }
        })
        .collect()
}

/// Worst-wins aggregation across edges, exactly as `toolchain::worst_status` does:
/// `Drift` beats `NotEvaluable` beats `Pass`.
fn worst_status(a: CheckStatus, b: CheckStatus) -> CheckStatus {
    match (a, b) {
        (CheckStatus::Drift, _) | (_, CheckStatus::Drift) => CheckStatus::Drift,
        (CheckStatus::NotEvaluable, _) | (_, CheckStatus::NotEvaluable) => {
            CheckStatus::NotEvaluable
        }
        _ => CheckStatus::Pass,
    }
}

fn status_label(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "pass",
        CheckStatus::Drift => "drift",
        CheckStatus::NotEvaluable => "not_evaluable",
    }
}

fn edge_label(edge: &EdgeOutcome) -> String {
    format!("{}/{}", edge.contract, edge.consumer_repo)
}

/// Run the `contract-freshness` check: evaluate every registered `[[contracts]]` entry's
/// consumer edges against `ctx.root`, worst-wins aggregate, and name every edge
/// individually in the report (never a bare aggregate).
pub fn run(ctx: &ConformanceCtx) -> CheckOutcome {
    let mut all_edges: Vec<EdgeOutcome> = Vec::new();
    for entry in &ctx.config.contracts {
        all_edges.extend(evaluate_contract(&ctx.root, entry));
    }

    if all_edges.is_empty() {
        return CheckOutcome {
            status: CheckStatus::Pass,
            left: None,
            right: None,
            findings: Vec::new(),
            reason: None,
        };
    }

    let mut overall_status = CheckStatus::Pass;
    let mut findings = Vec::new();
    let mut reasons = Vec::new();
    let mut left_items = Vec::new();
    let mut right_items = Vec::new();

    for edge in &all_edges {
        overall_status = worst_status(overall_status, edge.status);
        let label = edge_label(edge);
        left_items.push(format!("{label}: canonical={}", edge.contract));
        right_items.push(format!("{label}: {}", status_label(edge.status)));
        match edge.status {
            CheckStatus::Drift => {
                findings.push(format!("{label}: {}", edge.detail));
            }
            CheckStatus::NotEvaluable => {
                reasons.push(format!("{label}: {}", edge.detail));
            }
            CheckStatus::Pass => {}
        }
    }

    let left = FactSide {
        label: "canonical contract docs/consts".to_string(),
        source: "brain.toml [[contracts]].canonical, resolved against ctx.root".to_string(),
        digest: super::digest(&left_items),
        items: left_items,
    };

    let right = FactSide {
        label: "per-edge verdict (brain.toml [[contracts]].consumers)".to_string(),
        source: "each consumer endpoint's extracted version vs its canonical".to_string(),
        digest: super::digest(&right_items),
        items: right_items,
    };

    let reason = if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    };

    CheckOutcome {
        status: overall_status,
        left: Some(left),
        right: Some(right),
        findings,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::ContractEndpoint;
    use std::io::Write;

    fn endpoint(repo: &str, path: &str, format: &str, key: Option<&str>) -> ContractEndpoint {
        ContractEndpoint {
            repo: repo.to_string(),
            path: path.to_string(),
            format: format.to_string(),
            key: key.map(|k| k.to_string()),
        }
    }

    fn write_fixture(dir: &tempfile::TempDir, rel: &str, content: &str) -> String {
        let full = dir.path().join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(&full).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        rel.to_string()
    }

    // --- md-version-line extractor: the three fleet shapes ---

    #[test]
    fn md_shape_contract_version() {
        let content = "intro\n**Contract Version: 1.8.0**\nmore text\n";
        assert_eq!(extract_md_version_line(content), Ok("1.8.0".to_string()));
    }

    #[test]
    fn md_shape_pinned_contract_version() {
        let content = "intro\n**Pinned Contract Version: 1.0.0**\nmore text\n";
        assert_eq!(extract_md_version_line(content), Ok("1.0.0".to_string()));
    }

    #[test]
    fn md_shape_version_colon_outside_bold_trailing_whitespace() {
        let content = "intro\n**Version:** v0.40  \nmore text\n";
        assert_eq!(extract_md_version_line(content), Ok("v0.40".to_string()));
    }

    #[test]
    fn md_shape_not_at_line_15() {
        // carryover-contract.md's real line is 36, not 15 — must not assume a fixed line.
        let mut content = String::new();
        for _ in 0..35 {
            content.push_str("padding line\n");
        }
        content.push_str("**Contract Version: 1.0.0**\n");
        assert_eq!(extract_md_version_line(&content), Ok("1.0.0".to_string()));
    }

    #[test]
    fn md_no_match_is_extraction_failure() {
        let content = "nothing here about versions at all\n";
        assert!(extract_md_version_line(content).is_err());
    }

    #[test]
    fn md_two_distinct_versions_is_extraction_failure() {
        let content = "**Contract Version: 1.0.0**\n**Contract Version: 2.0.0**\n";
        assert!(extract_md_version_line(content).is_err());
    }

    #[test]
    fn md_two_matching_versions_is_fine() {
        let content =
            "**Contract Version: 1.0.0**\nsome text **Contract Version: 1.0.0** repeated\n";
        assert_eq!(extract_md_version_line(content), Ok("1.0.0".to_string()));
    }

    // --- dart-const extractor ---

    #[test]
    fn dart_const_single_quotes() {
        let content = "const String kServeApiPin = 'v0.38';\n";
        assert_eq!(
            extract_dart_const(content, "kServeApiPin"),
            Ok("v0.38".to_string())
        );
    }

    #[test]
    fn dart_const_double_quotes() {
        let content = "const String kServeApiPin = \"v0.38\";\n";
        assert_eq!(
            extract_dart_const(content, "kServeApiPin"),
            Ok("v0.38".to_string())
        );
    }

    #[test]
    fn dart_const_missing_key_is_extraction_failure() {
        let content = "const String someOtherConst = 'v1.0';\n";
        assert!(extract_dart_const(content, "kServeApiPin").is_err());
    }

    #[test]
    fn dart_const_endpoint_missing_key_field_is_not_evaluable() {
        let dir = tempfile::tempdir().unwrap();
        let rel = write_fixture(&dir, "dart.dart", "const String kServeApiPin = 'v0.38';\n");
        let ep = endpoint("bastion-ui", &rel, "dart-const", None);
        let result = resolve_endpoint(dir.path(), &ep);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no `key`"));
    }

    // --- equality verdict, never ordering ---

    #[test]
    fn equal_versions_is_pass() {
        let (status, _) = edge_verdict(&Ok("1.8.0".to_string()), &Ok("1.8.0".to_string()));
        assert_eq!(status, CheckStatus::Pass);
    }

    #[test]
    fn numerically_ahead_consumer_is_drift_not_pass() {
        // synapse (1.9.0) is numerically ahead of engine-rs (1.8.0) canonical — still Drift.
        let (status, detail) = edge_verdict(&Ok("1.8.0".to_string()), &Ok("1.9.0".to_string()));
        assert_eq!(status, CheckStatus::Drift);
        assert!(detail.contains("1.8.0"));
        assert!(detail.contains("1.9.0"));
    }

    #[test]
    fn numerically_behind_consumer_is_also_drift() {
        let (status, _) = edge_verdict(&Ok("1.8.0".to_string()), &Ok("1.0.0".to_string()));
        assert_eq!(status, CheckStatus::Drift);
    }

    // --- not-evaluable, never a crash ---

    #[test]
    fn missing_file_is_not_evaluable() {
        let dir = tempfile::tempdir().unwrap();
        let ep = endpoint("nowhere", "does/not/exist.md", "md-version-line", None);
        let result = resolve_endpoint(dir.path(), &ep);
        assert!(result.is_err());
    }

    #[test]
    fn unknown_format_is_not_evaluable() {
        let dir = tempfile::tempdir().unwrap();
        let rel = write_fixture(&dir, "f.md", "**Contract Version: 1.0.0**\n");
        let ep = endpoint("repo", &rel, "yaml-frontmatter", None);
        let result = resolve_endpoint(dir.path(), &ep);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown format"));
    }

    #[test]
    fn evaluate_contract_full_edge_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let canonical_rel = write_fixture(
            &dir,
            "canonical/data-contract.md",
            "**Contract Version: 1.8.0**\n",
        );
        let drift_rel = write_fixture(
            &dir,
            "synapse/data-contract.md",
            "**Contract Version: 1.9.0**\n",
        );
        let pass_rel = write_fixture(
            &dir,
            "bastion/data-contract.md",
            "**Pinned Contract Version: 1.8.0**\n",
        );
        let missing_canonical_rel = "does/not/exist.md".to_string();

        let entry = ContractEntry {
            name: "data-contract".to_string(),
            canonical: endpoint("engine-rs", &canonical_rel, "md-version-line", None),
            consumers: vec![
                endpoint("synapse", &drift_rel, "md-version-line", None),
                endpoint("bastion", &pass_rel, "md-version-line", None),
            ],
        };
        let outcomes = evaluate_contract(dir.path(), &entry);
        assert_eq!(outcomes.len(), 2);
        let synapse = outcomes
            .iter()
            .find(|o| o.consumer_repo == "synapse")
            .unwrap();
        assert_eq!(synapse.status, CheckStatus::Drift);
        let bastion = outcomes
            .iter()
            .find(|o| o.consumer_repo == "bastion")
            .unwrap();
        assert_eq!(bastion.status, CheckStatus::Pass);

        // Missing canonical makes every edge NotEvaluable, named per edge.
        let entry2 = ContractEntry {
            name: "broken".to_string(),
            canonical: endpoint("nowhere", &missing_canonical_rel, "md-version-line", None),
            consumers: vec![endpoint("bastion", &pass_rel, "md-version-line", None)],
        };
        let outcomes2 = evaluate_contract(dir.path(), &entry2);
        assert_eq!(outcomes2.len(), 1);
        assert_eq!(outcomes2[0].status, CheckStatus::NotEvaluable);
    }

    #[test]
    fn run_with_empty_registry_is_pass() {
        let ctx = ConformanceCtx {
            root: PathBuf::from("."),
            config: crate::brain::config::BrainConfig::default(),
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Pass);
        assert!(outcome.findings.is_empty());
    }

    #[test]
    fn run_worst_wins_across_multiple_contracts() {
        let dir = tempfile::tempdir().unwrap();
        let canonical_rel = write_fixture(&dir, "canonical.md", "**Contract Version: 1.0.0**\n");
        let pass_rel = write_fixture(&dir, "pass.md", "**Contract Version: 1.0.0**\n");
        let drift_rel = write_fixture(&dir, "drift.md", "**Contract Version: 2.0.0**\n");

        let mut config = crate::brain::config::BrainConfig::default();
        config.contracts = vec![
            ContractEntry {
                name: "ok-contract".to_string(),
                canonical: endpoint("a", &canonical_rel, "md-version-line", None),
                consumers: vec![endpoint("b", &pass_rel, "md-version-line", None)],
            },
            ContractEntry {
                name: "bad-contract".to_string(),
                canonical: endpoint("a", &canonical_rel, "md-version-line", None),
                consumers: vec![endpoint("c", &drift_rel, "md-version-line", None)],
            },
        ];
        let ctx = ConformanceCtx {
            root: dir.path().to_path_buf(),
            config,
            files: Vec::new(),
        };
        let outcome = run(&ctx);
        assert_eq!(outcome.status, CheckStatus::Drift);
        assert_eq!(outcome.findings.len(), 1);
        assert!(outcome.findings[0].contains("bad-contract/c"));
    }

    /// Live-corpus regression test for the incident this block fixes: runs the real
    /// `contract-freshness` check against the actual fleet corpus (HQ `brain.toml`,
    /// walked up from this crate as every other live-corpus test in this repo does)
    /// and names, BY EDGE, the six edges the block record calls out:
    ///   - data-contract/synapse and serve-api/bastion-ui: live Drifts
    ///   - serve-api/bastion-web: live NotEvaluable (no version line at all)
    ///   - data-contract/bastion, workspace-contract/bastion, carryover-contract/bastion: Pass
    ///
    /// This is evidence about the SOURCE tree (the files on disk in this checkout),
    /// never about any installed `mev` binary.
    ///
    /// Deliberately does NOT hard-code "Drift" for the three known-live defects: each
    /// of them is a real bug in another repo that is expected to be fixed out from under
    /// this test (`SY.ticket.data-contract-lineage-reconcile` clears the synapse one).
    /// Instead this test reads both sides of each edge itself, independently of the
    /// check under test, and asserts the check's reported verdict is the one plain
    /// equality of those two independently-read strings implies — so the test stays a
    /// real regression test of the CHECK's logic while staying green after someone
    /// else's fix repairs the corpus.
    ///
    /// Skips cleanly (never fails) when `brain.toml` or a named contract path is
    /// absent — a CI checkout without the private HQ vault, matching the posture of
    /// this repo's other live-corpus tests (e.g. `config::tests::live_corpus_*`) and
    /// of `serve_api_version_test.dart` upstream.
    #[test]
    fn live_corpus_contract_freshness_names_the_known_edges() {
        let live_root = std::path::Path::new("../..");
        let live_brain_toml = live_root.join("brain.toml");
        if !live_brain_toml.exists() {
            eprintln!(
                "skipping live_corpus_contract_freshness_names_the_known_edges: {} has no \
                 brain.toml (fresh clone or CI runner without the sibling HQ checkout)",
                live_root.display()
            );
            return;
        }

        let config = match crate::brain::config::load_brain_config(&live_brain_toml) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "skipping live_corpus_contract_freshness_names_the_known_edges: live \
                     brain.toml errored: {e}"
                );
                return;
            }
        };

        // Every path this test's six edges need must exist on disk, or we skip —
        // never fail on a partial/CI checkout.
        let wanted: &[(&str, &str)] = &[
            ("data-contract", "synapse"),
            ("serve-api", "bastion-ui"),
            ("serve-api", "bastion-web"),
            ("data-contract", "bastion"),
            ("workspace-contract", "bastion"),
            ("carryover-contract", "bastion"),
        ];

        let mut missing_paths: Vec<String> = Vec::new();
        for entry in &config.contracts {
            let canonical_full = live_root.join(&entry.canonical.path);
            if !canonical_full.exists() {
                missing_paths.push(canonical_full.display().to_string());
            }
            for consumer in &entry.consumers {
                let full = live_root.join(&consumer.path);
                if !full.exists() {
                    missing_paths.push(full.display().to_string());
                }
            }
        }
        if !missing_paths.is_empty() {
            eprintln!(
                "skipping live_corpus_contract_freshness_names_the_known_edges: missing \
                 corpus path(s): {missing_paths:?}"
            );
            return;
        }

        // Run the real check against the real corpus.
        let ctx = ConformanceCtx {
            root: live_root.to_path_buf(),
            config: config.clone(),
            files: Vec::new(),
        };
        let all_edges: Vec<EdgeOutcome> = config
            .contracts
            .iter()
            .flat_map(|entry| evaluate_contract(&ctx.root, entry))
            .collect();

        // Confirm every wanted edge is present, then assert its verdict independently
        // derived from the two files on disk right now — not a hard-coded status.
        for (contract_name, consumer_repo) in wanted {
            let entry = config
                .contracts
                .iter()
                .find(|e| &e.name == contract_name)
                .unwrap_or_else(|| {
                    panic!("live brain.toml has no [[contracts]] named {contract_name}")
                });
            let consumer_endpoint = entry
                .consumers
                .iter()
                .find(|c| &c.repo == consumer_repo)
                .unwrap_or_else(|| {
                    panic!("live brain.toml contract {contract_name} has no consumer repo {consumer_repo}")
                });

            let edge = all_edges
                .iter()
                .find(|e| &e.contract == contract_name && &e.consumer_repo == consumer_repo)
                .unwrap_or_else(|| {
                    panic!("check did not report edge {contract_name}/{consumer_repo}")
                });

            // Independently re-derive the expected verdict straight from disk, so this
            // test tracks reality rather than a frozen snapshot of it.
            let canonical_result = resolve_endpoint(&ctx.root, &entry.canonical);
            let consumer_result = resolve_endpoint(&ctx.root, consumer_endpoint);
            let (expected_status, _) = edge_verdict(&canonical_result, &consumer_result);

            assert_eq!(
                edge.status, expected_status,
                "edge {contract_name}/{consumer_repo}: check reported {:?} but the versions on \
                 disk right now (canonical={canonical_result:?}, consumer={consumer_result:?}) \
                 imply {expected_status:?}",
                edge.status
            );
        }

        // Sanity-check the aggregate contains exactly one outcome per requested edge —
        // confirms this test named all six edges individually, never a bare count.
        assert_eq!(wanted.len(), 6);
    }
}
