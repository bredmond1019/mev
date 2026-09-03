//! Field-level authored round-trip regression + the container-count control
//! that misses it (ticket `MV.ticket.emit-state-write-is-corpus-wide-and-unscoped`,
//! task 3).
//!
//! A prior regeneration turned an authored `reference[]` entry's `"related": []`
//! into an absent key. Test A pins that this cannot happen silently again — a
//! field-level comparison, keyed on presence as well as value, over every
//! authored field on a leaf repo's `state.json` `reference[]`/`carryover[]`
//! entries. Test B is the recorded CONTROL: the same regeneration compared the
//! way `engine-rs`'s container-count check compared it (lengths + slug sets
//! only) reports no difference, which is exactly why that check gave a false
//! all-clear on the live regression this block is about.
//!
//! AMENDMENT (2026-09-02, /generate-tasks, D18): the block record's `what`
//! describes the observed change as `related: []` -> `related: null`. That is
//! not the actual mechanism. `git log -S '"related": null' --all` (run from
//! this repo's root) returns zero hits — no commit in this repo's history has
//! ever contained the literal string `"related": null`. `okf-core`'s
//! `Reference::related` is `#[serde(default, skip_serializing_if =
//! "Vec::is_empty")]`, which DROPS the key on re-serialization when the vec is
//! empty; it does not null it. The observed change was `[]` -> KEY ABSENT,
//! which a Python field-level differ (comparing parsed dicts with `.get()`)
//! renders as `None` — indistinguishable from a JSON `null` unless the differ
//! also checks key presence. `okf-core`'s `Carryover::related` carries no
//! `skip_serializing_if`, so the same authored `[]` on a carryover entry is
//! preserved. The two containers disagree, which is the real defect;
//! `related: null` was never literally written to disk.
//!
//! Fixture: a throwaway temp-dir corpus (`mev::testsupport::unique_temp_dir`),
//! never the live brain — a test that writes the real corpus is the defect
//! under repair.

use std::fs;
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    let d = mev::testsupport::unique_temp_dir(&format!("mev-emit-state-authored-roundtrip-{tag}"));
    fs::create_dir_all(&d).unwrap();
    d
}

fn write_file(root: &Path, rel: &str, content: &str) {
    let target = root.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&target, content.as_bytes()).unwrap();
}

fn write_json(root: &Path, rel: &str, value: &serde_json::Value) {
    write_file(root, rel, &serde_json::to_string_pretty(value).unwrap());
}

fn read_json(root: &Path, rel: &str) -> serde_json::Value {
    let s =
        fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("failed to parse {rel} as JSON: {e}"))
}

/// A minimal single-leaf-repo `brain.toml` — no HQ root, no tiers (`discover_state_files`
/// treats the HQ `planning/state.json` as optional, see `src/brain/state.rs`), so this
/// fixture only needs the one leaf repo `[[repos]]` entry whose `related`/`carryover`
/// authored fields are under test.
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
"#;
    fs::write(root.join("brain.toml"), toml.as_bytes()).unwrap();
}

fn write_status_md(root: &Path) {
    let doc = "---\n\
               type: ProjectStatus\n\
               title: alpha status\n\
               description: Status fixture for alpha.\n\
               timestamp: \"2026-07-27T12:00:00Z\"\n\
               ---\n\n\
               # Status\n";
    write_file(root, "repos/alpha/planning/status.md", doc);
}

fn write_cache_doc(root: &Path) {
    let doc = "---\n\
               type: ProjectStatus\n\
               title: alpha cache\n\
               description: Project cache fixture for alpha.\n\
               ---\n\n\
               # alpha\n\n\
               <!-- BEGIN generated:project-cache -->\n\
               <!-- END generated:project-cache -->\n";
    write_file(root, "docs/projects/alpha.md", doc);
}

/// The leaf `state.json` under test: `focus.now` is empty even though
/// `tracks[]` has a live `in_progress` block, forcing `plan_state_json` to
/// produce a real rewrite (mirrors `tests/it/emit_state_scope.rs`'s
/// `write_stale_leaf_state`) — a run over a file that is already fixed-point
/// would prove nothing about the round trip.
///
/// Carries one authored `reference[]` entry with `"related": []` (the field
/// that silently disappears) and one authored `carryover[]` entry with
/// authored `related`, `needs`, `priority` and `scope` (the sibling
/// container that does NOT lose its `related: []`, and whose other fields
/// must also survive byte-for-byte).
fn write_leaf_state(root: &Path) -> serde_json::Value {
    let state = serde_json::json!({
        "repo": "alpha",
        "kind": "project",
        "updated": "2026-07-27",
        "focus": { "now": [], "next": [], "blocked": [] },
        "tracks": [
            {
                "title": "Phase 1",
                "blocks": [
                    { "id": "ALPHA.1.A", "title": "alpha block A", "status": "in_progress" }
                ]
            }
        ],
        "reference": [
            {
                "slug": "alpha-ref-1",
                "scope": { "repo": "alpha", "tier": null, "cross_repo": null },
                "class": "invariant",
                "text": "Authored reference entry whose empty related[] must round-trip.",
                "created": "2026-08-01",
                "related": []
            }
        ],
        "carryover": [
            {
                "slug": "alpha-carry-1",
                "scope": { "repo": "alpha", "tier": null, "cross_repo": null },
                "kind": "deferred",
                "needs": "code",
                "text": "Authored carryover entry whose related[]/needs/priority/scope must round-trip.",
                "related": [],
                "priority": 2,
                "created": "2026-08-01"
            }
        ]
    });
    write_json(root, "repos/alpha/planning/state.json", &state);
    state
}

fn write_fixture(root: &Path) -> serde_json::Value {
    write_brain_toml(root);
    write_status_md(root);
    write_cache_doc(root);
    write_leaf_state(root)
}

/// Field-level diff over a parsed JSON `Value`, keyed on PRESENCE as well as
/// value — an absent key and a present-but-empty-array value at the same
/// path are NOT equal. Returns a human-readable list of `path: before -> after`
/// mismatches (empty means "identical").
fn field_diff(before: &serde_json::Value, after: &serde_json::Value, path: &str) -> Vec<String> {
    use serde_json::Value;
    let mut out = Vec::new();
    match (before, after) {
        (Value::Object(b), Value::Object(a)) => {
            let mut keys: std::collections::BTreeSet<&String> = b.keys().collect();
            keys.extend(a.keys());
            for k in keys {
                let child_path = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match (b.get(k), a.get(k)) {
                    (Some(bv), Some(av)) => out.extend(field_diff(bv, av, &child_path)),
                    (Some(bv), None) => out.push(format!(
                        "{child_path}: PRESENT ({bv}) -> ABSENT (key dropped)"
                    )),
                    (None, Some(av)) => out.push(format!(
                        "{child_path}: ABSENT -> PRESENT ({av}) (key added)"
                    )),
                    (None, None) => unreachable!("key came from one of the two maps"),
                }
            }
        }
        (Value::Array(b), Value::Array(a)) => {
            if b.len() != a.len() {
                out.push(format!("{path}: array length {} -> {}", b.len(), a.len()));
            }
            for (i, (bv, av)) in b.iter().zip(a.iter()).enumerate() {
                out.extend(field_diff(bv, av, &format!("{path}[{i}]")));
            }
        }
        (bv, av) if bv != av => {
            out.push(format!("{path}: {bv} -> {av}"));
        }
        _ => {}
    }
    out
}

// ---------------------------------------------------------------------------
// Test A: authored fields must round-trip byte-identically (field-level).
// ---------------------------------------------------------------------------

#[test]
fn authored_fields_round_trip_byte_identically() {
    let dir = temp_dir("field-level");
    let before = write_fixture(&dir);

    let report = mev::emit_state(&dir, true, None).expect("unscoped emit should not error");
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "fixture emit should have no errors; got: {errors:#?}"
    );

    let after = read_json(&dir, "repos/alpha/planning/state.json");

    let before_reference = &before["reference"];
    let after_reference = &after["reference"];
    let before_carryover = &before["carryover"];
    let after_carryover = &after["carryover"];

    let reference_diff = field_diff(before_reference, after_reference, "reference");

    // KNOWN AND DELIBERATELY UNFIXED: a carryover entry authored with NO
    // `clears_when` gains an explicit `"clears_when": null` on regeneration.
    // That is the same class of authored-field mutation this test exists to
    // catch, in the opposite direction, and it is real — it is excluded here,
    // not absent.
    //
    // Operator decision 2026-09-02 on
    // MV.ticket.emit-state-write-is-corpus-wide-and-unscoped: the serializer
    // fix (`skip_serializing_if = "Option::is_none"` on
    // `okf_core::state::Carryover::clears_when`) would strip that key from 21
    // live carryover entries across 5 repos on the next
    // `mev emit-state --write` — a fleet-wide canonicalization diff every
    // concurrent lane would have to absorb, on top of the 45 reference entries
    // the `related` fix in this same block already rewrites. The `related`
    // asymmetry was in this block's scope; `clears_when` was not. Removing this
    // exclusion is the whole of the follow-up work, and it must be sequenced
    // when no other lane is live.
    let carryover_diff: Vec<String> = field_diff(before_carryover, after_carryover, "carryover")
        .into_iter()
        .filter(|d| !d.contains(".clears_when:"))
        .collect();

    assert!(
        reference_diff.is_empty(),
        "authored reference[] entry did not round-trip byte-identically through \
         `mev set-block-status --write` (unscoped emit_state):\n{}\n\n\
         before: {}\nafter:  {}",
        reference_diff.join("\n"),
        serde_json::to_string_pretty(before_reference).unwrap(),
        serde_json::to_string_pretty(after_reference).unwrap(),
    );
    assert!(
        carryover_diff.is_empty(),
        "authored carryover[] entry did not round-trip byte-identically:\n{}\n\n\
         before: {}\nafter:  {}",
        carryover_diff.join("\n"),
        serde_json::to_string_pretty(before_carryover).unwrap(),
        serde_json::to_string_pretty(after_carryover).unwrap(),
    );

    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Test B: the CONTROL. A container-count/slug-set comparison — the shape of
// check engine-rs actually ran — reports NO difference across the same write
// test A catches. This test must PASS both before and after the fix to task
// 3/4: it records why the count check gave a false all-clear, it is not a
// second regression test.
// ---------------------------------------------------------------------------

#[test]
fn container_count_check_is_the_control_that_misses_it() {
    let dir = temp_dir("container-count-control");
    let before = write_fixture(&dir);

    let report = mev::emit_state(&dir, true, None).expect("unscoped emit should not error");
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == mev::Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "fixture emit should have no errors; got: {errors:#?}"
    );

    let after = read_json(&dir, "repos/alpha/planning/state.json");

    fn container_shape(v: &serde_json::Value, container: &str) -> (usize, Vec<String>) {
        let arr = v[container].as_array().cloned().unwrap_or_default();
        let len = arr.len();
        let slugs: Vec<String> = arr
            .iter()
            .filter_map(|e| e["slug"].as_str().map(|s| s.to_string()))
            .collect();
        (len, slugs)
    }

    let (before_ref_len, before_ref_slugs) = container_shape(&before, "reference");
    let (after_ref_len, after_ref_slugs) = container_shape(&after, "reference");
    let (before_carry_len, before_carry_slugs) = container_shape(&before, "carryover");
    let (after_carry_len, after_carry_slugs) = container_shape(&after, "carryover");

    // This is the assertion engine-rs's container-count check makes: same
    // length, same slug set => "no data loss". It passes here even though
    // test A (above) demonstrates the SAME write dropped `reference[0].related`
    // entirely — the count/slug-set comparison is structurally blind to a
    // mutation *inside* an unchanged-length container.
    assert_eq!(
        before_ref_len, after_ref_len,
        "control expectation violated: reference[] length changed ({before_ref_len} -> {after_ref_len}) \
         — if this fails, engine-rs's container-count check would ALSO have caught this regression \
         and the 'false all-clear' claim in the block record is wrong"
    );
    assert_eq!(
        before_ref_slugs, after_ref_slugs,
        "control expectation violated: reference[] slug set changed"
    );
    assert_eq!(
        before_carry_len, after_carry_len,
        "control expectation violated: carryover[] length changed ({before_carry_len} -> {after_carry_len})"
    );
    assert_eq!(
        before_carry_slugs, after_carry_slugs,
        "control expectation violated: carryover[] slug set changed"
    );

    let _ = fs::remove_dir_all(&dir);
}
