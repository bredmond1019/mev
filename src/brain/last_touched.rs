//! Per-block SDLC-artifact recency derivation (Phase 10, Block MV.10.D).
//!
//! [`derive_last_touched`] answers "when was this block's code last touched by an SDLC
//! run?" by scanning the block's own on-disk spec-folder artifacts for the newest
//! `updated_at` timestamp across all four state-file kinds the harness writes
//! (`sdlc-flow-state.json`, `sdlc-task-state.json`, `sdlc-run-state.json`,
//! `sdlc-state.json`).
//!
//! **Honesty constraint (binding on every code path in this module): absence means
//! "never worked", never "worked long ago".** A block with no resolvable SDLC run has
//! **no entry** in the returned map — never a sentinel date, never an epoch, never
//! `state.json.updated`, and never another block's timestamp. Callers (bastion's
//! `BA.11.S` included) must read a missing key as "unknown", not "old".

use std::collections::HashMap;
use std::path::Path;

use crate::brain::config::BrainConfig;
use crate::brain::state::{StateFile, StateSource};

/// The four on-disk state-file kinds an SDLC spec folder's `sdlc/` directory may hold.
/// All four are checked — mev's own `MV.10.C` folder, for example, has only
/// `sdlc-task-state.json` on disk.
const STATE_FILE_NAMES: [&str; 4] = [
    "sdlc-flow-state.json",
    "sdlc-task-state.json",
    "sdlc-run-state.json",
    "sdlc-state.json",
];

/// Derive a `"{repo_slug}:{block_id}" -> updated_at` map from every loaded repo's
/// on-disk SDLC spec folders.
///
/// For each `(src, file)` pair, the owning planning directory is `src.abs_path`'s
/// parent (state.json lives at `planning/state.json`). Candidate spec folders are the
/// direct children of that planning directory, plus the direct children of its
/// `archive/` subdirectory when one exists (one archive level only — closed blocks'
/// folders are moved there per the repo's `/archive` convention).
///
/// A block's `id` is matched against candidate folder names in precedence order:
/// first the verbatim `id` (e.g. `EN.7.C-materialize-harvest-gate`, or the bare
/// `BW.9.A`), and only when that yields nothing, the `id` with its repo's configured
/// `prefix` (looked up in `config.repos` by `src.repo_slug`) plus `"."` stripped from
/// the front (e.g. `MV.10.C` -> `10.C`, matching mev's `10.C-emit-block-graph-cli`). A
/// folder matches a candidate when its name equals the candidate exactly, or starts
/// with the candidate immediately followed by `-` (a name-boundary requirement: `BW.9.A`
/// must not match a folder named `BW.9.A2-something`). Matching is case-sensitive.
///
/// Every matched folder's `sdlc/` directory is checked for all four
/// [`STATE_FILE_NAMES`]; each readable, well-formed file with a non-empty string
/// `updated_at` contributes that value. The **maximum by plain string comparison**
/// across every matched folder and every state-file kind wins — the on-disk corpus is
/// uniformly formatted `YYYY-MM-DDTHH:MM:SSZ` (verified against the live corpus, see
/// `planning/10.D-derive-last-touched/tasks.md`), so lexicographic max is exactly
/// chronological max; no date-parsing dependency is needed or added here.
///
/// Every failure mode — an unreadable directory, an unreadable file, invalid JSON, a
/// non-object top level, or an absent/empty/non-string `updated_at` — is silently
/// skipped and contributes no value. There is deliberately no fallback to `started_at`:
/// a run that somehow lacks `updated_at` yields no value from that file, never a
/// substituted one.
pub fn derive_last_touched(
    _root: &Path,
    config: &BrainConfig,
    loaded: &[(StateSource, StateFile)],
) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();

    for (src, file) in loaded {
        let Some(planning_dir) = src.abs_path.parent() else {
            continue;
        };

        let prefix = config
            .repos
            .iter()
            .find(|r| r.slug == src.repo_slug)
            .and_then(|r| r.prefix.as_deref());

        let candidate_folders = collect_candidate_folders(planning_dir);

        for track in &file.tracks {
            for block in &track.blocks {
                let key = format!("{}:{}", src.repo_slug, block.id);
                if out.contains_key(&key) {
                    // Shouldn't happen (block IDs are namespaced per repo and unique
                    // within a StateFile), but stay defensive rather than overwrite.
                    continue;
                }

                if let Some(newest) = newest_for_block(&candidate_folders, &block.id, prefix) {
                    out.insert(key, newest);
                }
            }
        }
    }

    out
}

/// Collect every direct child directory of `planning_dir`, plus every direct child
/// directory of `planning_dir/archive` when that directory exists. Skips `sdlc` itself
/// (it is a leaf artifact directory, never a spec folder) and any non-directory entry.
fn collect_candidate_folders(planning_dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    collect_dir_children(planning_dir, &mut out);
    collect_dir_children(&planning_dir.join("archive"), &mut out);
    out
}

fn collect_dir_children(dir: &Path, out: &mut Vec<(String, std::path::PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name == "sdlc" {
            continue;
        }
        out.push((name.to_string(), path));
    }
}

/// Find the newest `updated_at` across every folder in `candidate_folders` that
/// matches `block_id` (full-ID precedence, then prefix-stripped), reading all four
/// state-file kinds per matched folder.
fn newest_for_block(
    candidate_folders: &[(String, std::path::PathBuf)],
    block_id: &str,
    prefix: Option<&str>,
) -> Option<String> {
    // Full-ID match takes precedence: try it across all folders first, and only fall
    // back to the prefix-stripped candidate when the full ID matched nothing.
    if let Some(newest) = newest_for_candidate(candidate_folders, block_id) {
        return Some(newest);
    }

    let stripped = prefix.and_then(|p| {
        let with_dot = format!("{p}.");
        block_id.strip_prefix(&with_dot)
    })?;

    newest_for_candidate(candidate_folders, stripped)
}

fn newest_for_candidate(
    candidate_folders: &[(String, std::path::PathBuf)],
    candidate: &str,
) -> Option<String> {
    let mut newest: Option<String> = None;

    for (name, path) in candidate_folders {
        if !folder_name_matches(name, candidate) {
            continue;
        }

        for updated_at in read_state_updated_ats(path) {
            newest = Some(match newest {
                Some(current) if current >= updated_at => current,
                _ => updated_at,
            });
        }
    }

    newest
}

/// A folder name matches a candidate block ID when it equals the candidate exactly,
/// or starts with the candidate immediately followed by `-`. This is the name-boundary
/// rule: `BW.9.A` must match `BW.9.A` or `BW.9.A-something`, but not `BW.9.A2-x`.
fn folder_name_matches(folder_name: &str, candidate: &str) -> bool {
    folder_name == candidate
        || folder_name
            .strip_prefix(candidate)
            .is_some_and(|rest| rest.starts_with('-'))
}

/// Read every `updated_at` value present across the four state-file kinds under
/// `spec_folder/sdlc/`. Unreadable files, invalid JSON, a non-object top level, and a
/// missing/empty/non-string `updated_at` are silently skipped.
fn read_state_updated_ats(spec_folder: &Path) -> Vec<String> {
    let sdlc_dir = spec_folder.join("sdlc");
    let mut out = Vec::new();

    for name in STATE_FILE_NAMES {
        let path = sdlc_dir.join(name);
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
            continue;
        };
        let Some(updated_at) = value.get("updated_at").and_then(|v| v.as_str()) else {
            continue;
        };
        if updated_at.is_empty() {
            continue;
        }
        out.push(updated_at.to_string());
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::config::RepoEntry;
    use std::path::PathBuf;

    fn config_with_prefix(slug: &str, prefix: Option<&str>) -> BrainConfig {
        BrainConfig {
            repos: vec![RepoEntry {
                slug: slug.to_string(),
                prefix: prefix.map(|p| p.to_string()),
                tier: String::new(),
                repo_path: String::new(),
                status_file: String::new(),
                cache_doc: String::new(),
                heading: String::new(),
            }],
            ..BrainConfig::default()
        }
    }

    fn src(repo: &str, planning_dir: &Path) -> StateSource {
        StateSource {
            repo_slug: repo.to_string(),
            abs_path: planning_dir.join("state.json"),
            expected_kind: "project",
        }
    }

    fn state_file(repo: &str, block_id: &str) -> StateFile {
        let json = format!(
            r#"{{
                "repo": "{repo}",
                "kind": "project",
                "updated": "2026-01-01",
                "tracks": [
                    {{
                        "title": "Phase 1",
                        "blocks": [
                            {{ "id": "{block_id}", "title": "test block" }}
                        ]
                    }}
                ]
            }}"#
        );
        serde_json::from_str(&json).expect("fixture StateFile must parse")
    }

    fn write_state_file(spec_folder: &Path, name: &str, updated_at: &str) {
        let sdlc_dir = spec_folder.join("sdlc");
        std::fs::create_dir_all(&sdlc_dir).unwrap();
        std::fs::write(
            sdlc_dir.join(name),
            format!(r#"{{"updated_at": "{updated_at}"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn one_folder_one_state_file_reports_its_updated_at() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        std::fs::create_dir_all(&planning).unwrap();
        let spec = planning.join("MV.10.C-slug");
        std::fs::create_dir_all(&spec).unwrap();
        write_state_file(&spec, "sdlc-task-state.json", "2026-07-01T10:00:00Z");

        let config = config_with_prefix("mev", Some("MV"));
        let loaded = vec![(src("mev", &planning), state_file("mev", "MV.10.C-slug"))];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert_eq!(
            out.get("mev:MV.10.C-slug").map(String::as_str),
            Some("2026-07-01T10:00:00Z")
        );
    }

    #[test]
    fn newest_wins_across_state_file_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        std::fs::create_dir_all(&planning).unwrap();
        let spec = planning.join("MV.10.C");
        std::fs::create_dir_all(&spec).unwrap();
        write_state_file(&spec, "sdlc-flow-state.json", "2026-07-01T10:00:00Z");
        write_state_file(&spec, "sdlc-task-state.json", "2026-07-05T10:00:00Z");

        let config = config_with_prefix("mev", None);
        let loaded = vec![(src("mev", &planning), state_file("mev", "MV.10.C"))];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert_eq!(
            out.get("mev:MV.10.C").map(String::as_str),
            Some("2026-07-05T10:00:00Z")
        );
    }

    #[test]
    fn newest_wins_across_two_matching_folders() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        std::fs::create_dir_all(&planning).unwrap();
        let spec_a = planning.join("MV.10.C-first-attempt");
        let spec_b = planning.join("MV.10.C-final");
        std::fs::create_dir_all(&spec_a).unwrap();
        std::fs::create_dir_all(&spec_b).unwrap();
        write_state_file(&spec_a, "sdlc-task-state.json", "2026-06-01T10:00:00Z");
        write_state_file(&spec_b, "sdlc-task-state.json", "2026-07-10T10:00:00Z");

        let config = config_with_prefix("mev", None);
        let loaded = vec![(src("mev", &planning), state_file("mev", "MV.10.C"))];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert_eq!(
            out.get("mev:MV.10.C").map(String::as_str),
            Some("2026-07-10T10:00:00Z")
        );
    }

    #[test]
    fn no_folder_yields_no_entry() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        std::fs::create_dir_all(&planning).unwrap();

        let config = config_with_prefix("mev", None);
        let loaded = vec![(src("mev", &planning), state_file("mev", "MV.99.Z-ghost"))];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert!(
            !out.contains_key("mev:MV.99.Z-ghost"),
            "a block with no resolvable SDLC run must have no map entry, got {out:?}"
        );
    }

    #[test]
    fn archive_folder_is_matched() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        let archive_spec = planning.join("archive").join("MV.10.C-closed");
        std::fs::create_dir_all(&archive_spec).unwrap();
        write_state_file(
            &archive_spec,
            "sdlc-task-state.json",
            "2026-05-01T10:00:00Z",
        );

        let config = config_with_prefix("mev", None);
        let loaded = vec![(src("mev", &planning), state_file("mev", "MV.10.C"))];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert_eq!(
            out.get("mev:MV.10.C").map(String::as_str),
            Some("2026-05-01T10:00:00Z")
        );
    }

    #[test]
    fn prefix_stripped_resolution_when_full_id_folder_absent() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        let spec = planning.join("10.C-emit-block-graph-cli");
        std::fs::create_dir_all(&spec).unwrap();
        write_state_file(&spec, "sdlc-task-state.json", "2026-07-01T10:00:00Z");

        let config = config_with_prefix("mev", Some("MV"));
        let loaded = vec![(src("mev", &planning), state_file("mev", "MV.10.C"))];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert_eq!(
            out.get("mev:MV.10.C").map(String::as_str),
            Some("2026-07-01T10:00:00Z")
        );
    }

    #[test]
    fn full_id_takes_precedence_over_prefix_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        let full = planning.join("EN.7.C-materialize-harvest-gate");
        let stripped = planning.join("7.C-materialize-harvest-gate");
        std::fs::create_dir_all(&full).unwrap();
        std::fs::create_dir_all(&stripped).unwrap();
        write_state_file(&full, "sdlc-task-state.json", "2026-07-20T10:00:00Z");
        write_state_file(&stripped, "sdlc-task-state.json", "2026-01-01T10:00:00Z");

        let config = config_with_prefix("engine-rs", Some("EN"));
        let loaded = vec![(
            src("engine-rs", &planning),
            state_file("engine-rs", "EN.7.C-materialize-harvest-gate"),
        )];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert_eq!(
            out.get("engine-rs:EN.7.C-materialize-harvest-gate")
                .map(String::as_str),
            Some("2026-07-20T10:00:00Z"),
            "full-ID match must win over the prefix-stripped folder"
        );
    }

    #[test]
    fn boundary_rejection_prevents_prefix_collision() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        let decoy = planning.join("BW.9.A2-something");
        std::fs::create_dir_all(&decoy).unwrap();
        write_state_file(&decoy, "sdlc-task-state.json", "2026-07-01T10:00:00Z");

        let config = config_with_prefix("bastion-web", None);
        let loaded = vec![(
            src("bastion-web", &planning),
            state_file("bastion-web", "BW.9.A"),
        )];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert!(
            !out.contains_key("bastion-web:BW.9.A"),
            "BW.9.A must not match BW.9.A2-something (missing name boundary), got {out:?}"
        );
    }

    #[test]
    fn malformed_json_is_skipped_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        let spec = planning.join("MV.10.C");
        let sdlc_dir = spec.join("sdlc");
        std::fs::create_dir_all(&sdlc_dir).unwrap();
        std::fs::write(sdlc_dir.join("sdlc-task-state.json"), "{ not valid json").unwrap();

        let config = config_with_prefix("mev", None);
        let loaded = vec![(src("mev", &planning), state_file("mev", "MV.10.C"))];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert!(
            !out.contains_key("mev:MV.10.C"),
            "malformed JSON must be skipped, not fabricate a value, got {out:?}"
        );
    }

    #[test]
    fn missing_updated_at_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        let spec = planning.join("MV.10.C");
        let sdlc_dir = spec.join("sdlc");
        std::fs::create_dir_all(&sdlc_dir).unwrap();
        std::fs::write(
            sdlc_dir.join("sdlc-task-state.json"),
            r#"{"started_at": "2026-07-01T10:00:00Z"}"#,
        )
        .unwrap();

        let config = config_with_prefix("mev", None);
        let loaded = vec![(src("mev", &planning), state_file("mev", "MV.10.C"))];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert!(
            !out.contains_key("mev:MV.10.C"),
            "a file missing updated_at must not fall back to started_at, got {out:?}"
        );
    }

    #[test]
    fn missing_sdlc_directory_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let planning = dir.path().join("planning");
        let spec = planning.join("MV.10.C");
        std::fs::create_dir_all(&spec).unwrap(); // no sdlc/ subdir at all

        let config = config_with_prefix("mev", None);
        let loaded = vec![(src("mev", &planning), state_file("mev", "MV.10.C"))];

        let out = derive_last_touched(&PathBuf::from("."), &config, &loaded);
        assert!(
            !out.contains_key("mev:MV.10.C"),
            "a spec folder with no sdlc/ dir must yield no value, got {out:?}"
        );
    }
}
