//! Authored block/ticket/chore **creation** — the sibling of
//! [`crate::brain::blocks`] one step earlier in a block's life.
//!
//! [`crate::brain::blocks`] mutates exactly one existing block's authored
//! `status` and nothing else. This module is the other half: it defines the
//! payload a caller (the `mev create-block --from <file>` CLI, or later
//! engine-rs's SQ-41 shelling out programmatically) supplies to **file a new
//! block or ticket record**, plus the pure validation that payload must pass
//! before anything is ever written to disk.
//!
//! # Task 1 scope
//!
//! This task is types and validation only — no file writes, no `state.json`
//! mutation, no CLI wiring. [`CreateBlockPayload`] is the `--from <file>`
//! deserialization target; [`validate_payload`] is the pure function later
//! tasks call before planning any [`crate::brain::emit::EmitPlan`] actions.
//!
//! # CLI surface: `--from <file>`, not per-field flags
//!
//! `block.schema.json` has 15 required fields and four of them are long prose
//! or arrays (`what`, `why`, `files`, `acceptance_criteria`); shell-quoting
//! those is unusable, and the intended caller is engine-rs's SQ-41 shelling
//! out programmatically rather than a human typing flags. The record itself
//! flagged this OPEN; the lane settled it as a JSON payload file.
//!
//! # `epics` is not a block-record field
//!
//! `block.schema.json` has `additionalProperties: false` and no `epics`
//! property — a created record can never carry it. `epics` is instead the
//! cross-repo epic membership carried on the **`state.json`** registration
//! (`state::TrackBlock::epics` / `state::Block::epics`), and the generated
//! epic-sequence table renders from that *authored* membership, not from
//! derived lane membership (the carryover
//! `epic-sequence-table-uses-authored-epics-not-derived-lane-membership`).
//! A block created with no epic renders on no epic-sequence table, so
//! [`CreateBlockPayload::epics`] is required to be non-empty and
//! [`validate_payload`] refuses a payload that omits it — it is never
//! silently written with an empty list.
//!
//! # `spec_dir` is always derived
//!
//! `spec_dir` is never read from the payload. It is always exactly
//! `planning/<BlockID>/` (see [`derive_spec_dir`]) — a payload-supplied value
//! would let a typo diverge the schema's own pattern constraint from the
//! record's true spec directory.
//!
//! # Vocabularies enforced here
//!
//! Verified against base-template's `block.schema.json` on 2026-09-02.
//! Do **not** import the SDLC engines' `{haiku, sonnet, opus}` stage-model
//! vocabulary into [`VALID_MODELS`] — it is a different closed set for a
//! different purpose, and `"opus"` is deliberately *not* a legal block-record
//! `model` value.
//!
//! - [`VALID_KINDS`]: `block` | `ticket` | `chore`
//! - [`VALID_SDLC_WORKFLOWS`]: `none` | `patch` | `task` | `run` | `flow`
//! - [`VALID_MODELS`]: `sonnet` | `gemini-pro` | `gemini-flash` | `either`

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Diagnostic;
use crate::brain::emit::{EmitAction, EmitPlan};
use crate::brain::state::{BlockedBy, StateFile, StateSource, Track, TrackBlock};

/// Legal `kind` values — which producer authored the record and how it is
/// scheduled.
pub const VALID_KINDS: &[&str] = &["block", "ticket", "chore"];

/// Legal `sdlc_workflow` values — which engine consumes the block.
pub const VALID_SDLC_WORKFLOWS: &[&str] = &["none", "patch", "task", "run", "flow"];

/// Legal `model` values for a block record. Deliberately **not** the engines'
/// `{haiku, sonnet, opus}` stage-model vocabulary — `"opus"` is not legal
/// here, and the two sets are mutually invalid (see module docs).
pub const VALID_MODELS: &[&str] = &["sonnet", "gemini-pro", "gemini-flash", "either"];

/// Diagnostic code: `kind` is outside [`VALID_KINDS`].
pub const E_BLOCK_CREATE_KIND_ENUM: &str = "E_BLOCK_CREATE_KIND_ENUM";
/// Diagnostic code: `sdlc_workflow` is outside [`VALID_SDLC_WORKFLOWS`].
pub const E_BLOCK_CREATE_SDLC_WORKFLOW_ENUM: &str = "E_BLOCK_CREATE_SDLC_WORKFLOW_ENUM";
/// Diagnostic code: `model` is outside [`VALID_MODELS`].
pub const E_BLOCK_CREATE_MODEL_ENUM: &str = "E_BLOCK_CREATE_MODEL_ENUM";
/// Diagnostic code: `epics` is empty — refused, never written with an empty list.
pub const E_BLOCK_CREATE_MISSING_EPICS: &str = "E_BLOCK_CREATE_MISSING_EPICS";
/// Diagnostic code: a required prose/string field is empty.
pub const E_BLOCK_CREATE_EMPTY_FIELD: &str = "E_BLOCK_CREATE_EMPTY_FIELD";
/// Diagnostic code: `kind: "block"` with no `phase` — schema's conditional
/// `allOf` requires `phase` for that kind.
pub const E_BLOCK_CREATE_BLOCK_NEEDS_PHASE: &str = "E_BLOCK_CREATE_BLOCK_NEEDS_PHASE";
/// Diagnostic code: `kind: "ticket"` with no `testing_strategy` — schema's
/// conditional `allOf` requires `testing_strategy` for that kind.
pub const E_BLOCK_CREATE_TICKET_NEEDS_TESTING_STRATEGY: &str =
    "E_BLOCK_CREATE_TICKET_NEEDS_TESTING_STRATEGY";
/// Diagnostic code: `out_of_scope` is empty — schema requires `minItems: 1`.
pub const E_BLOCK_CREATE_EMPTY_OUT_OF_SCOPE: &str = "E_BLOCK_CREATE_EMPTY_OUT_OF_SCOPE";
/// Diagnostic code: `acceptance_criteria` is empty — schema requires `minItems: 1`.
pub const E_BLOCK_CREATE_EMPTY_ACCEPTANCE_CRITERIA: &str =
    "E_BLOCK_CREATE_EMPTY_ACCEPTANCE_CRITERIA";

// ---------------------------------------------------------------------------
// Payload sub-shapes — mirror block.schema.json's nested objects exactly.
// ---------------------------------------------------------------------------

/// One entry in `files.new[]`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct NewFileEntry {
    pub path: String,
    /// What the new file holds.
    pub purpose: String,
}

/// One entry in `files.modified[]`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ModifiedFileEntry {
    pub path: String,
    /// What changes in the existing file.
    pub change: String,
}

/// `files` — new/modified file manifest. Mirrors `block.schema.json`'s
/// `files` object (`additionalProperties: false`, both members optional).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct BlockFiles {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new: Vec<NewFileEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modified: Vec<ModifiedFileEntry>,
}

/// One entry in `acceptance_criteria[]` — either a bare string, or the
/// `{criterion, gateable, evidence}` object form for an un-gateable
/// criterion (D64).
///
/// `#[serde(untagged)]` with `Simple(String)` declared first mirrors the
/// schema's `oneOf` — a bare JSON string always deserializes to `Simple`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AcceptanceCriterion {
    Simple(String),
    Detailed {
        criterion: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gateable: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        evidence: Option<String>,
    },
}

impl AcceptanceCriterion {
    /// The criterion text, regardless of which form it was authored in.
    pub fn criterion_text(&self) -> &str {
        match self {
            AcceptanceCriterion::Simple(s) => s,
            AcceptanceCriterion::Detailed { criterion, .. } => criterion,
        }
    }
}

// ---------------------------------------------------------------------------
// CreateBlockPayload — the `--from <file>` deserialization target
// ---------------------------------------------------------------------------

/// The authored fields a caller supplies via `mev create-block --from <file>`.
///
/// Deliberately **not** identical to `block.schema.json`'s field set: it adds
/// `epics` (state.json-only; see module docs) and omits `spec_dir`/`created`/
/// `updated`, which [`derive_spec_dir`] and the driver derive rather than
/// accept from the caller.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CreateBlockPayload {
    pub id: String,
    pub repo: String,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub what: String,
    pub why: String,
    pub sdlc_workflow: String,
    pub model: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiative: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_rationale: Option<String>,

    #[serde(default)]
    pub files: BlockFiles,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<String>,

    pub out_of_scope: Vec<String>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub testing_strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_commands: Vec<String>,

    /// Dependency edges — reuses `okf-core`'s `BlockedBy`, the exact shape
    /// `state.json`'s own `depends_on` uses, so a later task can mirror this
    /// list into both the block record and the state.json registration
    /// without a second parallel type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<BlockedBy>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carryover_context: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub forward_looking: bool,

    /// Cross-repo epic membership for the **state.json** registration.
    /// Absent from `block.schema.json` itself (see module docs). Required to
    /// be non-empty by [`validate_payload`] — a payload with no epic is
    /// refused, never written with an empty list.
    #[serde(default)]
    pub epics: Vec<String>,
}

/// Derive `spec_dir` from a block id — always exactly `planning/<id>/`,
/// matching `block.schema.json`'s `spec_dir` pattern constraint. Never read
/// from the payload; see module docs.
pub fn derive_spec_dir(id: &str) -> String {
    format!("planning/{id}/")
}

/// Where diagnostics raised before any file exists point to — a virtual path
/// naming the record that *would* be written, so a caller sees exactly which
/// creation this refusal concerns.
fn virtual_record_path(payload: &CreateBlockPayload) -> PathBuf {
    Path::new("planning/blocks").join(format!("{}.json", payload.id))
}

/// Validate a [`CreateBlockPayload`] against every vocabulary and shape rule
/// this module enforces, ahead of any planning or write.
///
/// Pure — takes no filesystem or corpus state, and returns every violation
/// found rather than stopping at the first (so a caller sees the whole
/// problem in one round trip). An empty return means the payload is legal to
/// carry forward into record construction.
///
/// Does **not** check cross-corpus concerns (an existing id, a dangling
/// `depends_on` target, wave allocation) — those need the loaded corpus and
/// belong to a later task's planner, not this pure payload check.
pub fn validate_payload(payload: &CreateBlockPayload) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let path = virtual_record_path(payload);

    // --- Required prose/string fields must not be empty. ---
    let required_strings: &[(&str, &str)] = &[
        ("id", &payload.id),
        ("repo", &payload.repo),
        ("title", &payload.title),
        ("description", &payload.description),
        ("what", &payload.what),
        ("why", &payload.why),
    ];
    for (field, value) in required_strings {
        if value.trim().is_empty() {
            diags.push(Diagnostic::error(
                &path,
                E_BLOCK_CREATE_EMPTY_FIELD,
                format!("payload field '{field}' is empty; block.schema.json requires it"),
            ));
        }
    }

    // --- kind enum. ---
    if !VALID_KINDS.contains(&payload.kind.as_str()) {
        diags.push(Diagnostic::error(
            &path,
            E_BLOCK_CREATE_KIND_ENUM,
            format!(
                "'{}' is not a legal kind; must be one of {}",
                payload.kind,
                VALID_KINDS.join(", ")
            ),
        ));
    }

    // --- sdlc_workflow enum. ---
    if !VALID_SDLC_WORKFLOWS.contains(&payload.sdlc_workflow.as_str()) {
        diags.push(Diagnostic::error(
            &path,
            E_BLOCK_CREATE_SDLC_WORKFLOW_ENUM,
            format!(
                "'{}' is not a legal sdlc_workflow; must be one of {}",
                payload.sdlc_workflow,
                VALID_SDLC_WORKFLOWS.join(", ")
            ),
        ));
    }

    // --- model enum. Deliberately rejects "opus" — see module docs. ---
    if !VALID_MODELS.contains(&payload.model.as_str()) {
        diags.push(Diagnostic::error(
            &path,
            E_BLOCK_CREATE_MODEL_ENUM,
            format!(
                "'{}' is not a legal model; must be one of {} (the SDLC engines' \
                 {{haiku, sonnet, opus}} stage-model vocabulary does not apply here)",
                payload.model,
                VALID_MODELS.join(", ")
            ),
        ));
    }

    // --- epics: required, never silently defaulted to empty. ---
    if payload.epics.is_empty() {
        diags.push(Diagnostic::error(
            &path,
            E_BLOCK_CREATE_MISSING_EPICS,
            "payload carries no 'epics'; block.schema.json has no epics field (it is state.json-only), \
             but a block created with no authored epic renders on no epic-sequence table, so an empty \
             or absent epics list is refused rather than written"
                .to_string(),
        ));
    }

    // --- out_of_scope: schema requires minItems: 1. ---
    if payload.out_of_scope.is_empty() {
        diags.push(Diagnostic::error(
            &path,
            E_BLOCK_CREATE_EMPTY_OUT_OF_SCOPE,
            "payload 'out_of_scope' is empty; block.schema.json requires at least one entry \
             — a block with no stated boundary is under-specified"
                .to_string(),
        ));
    }

    // --- acceptance_criteria: schema requires minItems: 1. ---
    if payload.acceptance_criteria.is_empty() {
        diags.push(Diagnostic::error(
            &path,
            E_BLOCK_CREATE_EMPTY_ACCEPTANCE_CRITERIA,
            "payload 'acceptance_criteria' is empty; block.schema.json requires at least one entry"
                .to_string(),
        ));
    }

    // --- conditional requirements from the schema's allOf. ---
    if payload.kind == "block" && payload.phase.is_none() {
        diags.push(Diagnostic::error(
            &path,
            E_BLOCK_CREATE_BLOCK_NEEDS_PHASE,
            "kind is 'block' but 'phase' is absent; block.schema.json requires phase for kind=block"
                .to_string(),
        ));
    }
    if payload.kind == "ticket" && payload.testing_strategy.is_none() {
        diags.push(Diagnostic::error(
            &path,
            E_BLOCK_CREATE_TICKET_NEEDS_TESTING_STRATEGY,
            "kind is 'ticket' but 'testing_strategy' is absent; block.schema.json requires \
             testing_strategy for kind=ticket"
                .to_string(),
        ));
    }

    diags
}

// ---------------------------------------------------------------------------
// Planning — wave allocation, depends_on resolution, and the two-record plan
// ---------------------------------------------------------------------------

/// Diagnostic code: `repo` does not match any loaded corpus repo.
pub const E_BLOCK_CREATE_UNKNOWN_REPO: &str = "E_BLOCK_CREATE_UNKNOWN_REPO";
/// Diagnostic code: `id` already exists in the target repo's `state.json` —
/// a no-op refusal, never an overwrite.
pub const E_BLOCK_CREATE_EXISTS: &str = "E_BLOCK_CREATE_EXISTS";
/// Diagnostic code: a `depends_on` block-type edge names a `(repo, id)` that
/// does not resolve in the loaded corpus.
pub const E_BLOCK_CREATE_DANGLING_DEPENDENCY: &str = "E_BLOCK_CREATE_DANGLING_DEPENDENCY";

/// Repo-relative path of the block record this payload would write.
fn record_repo_path(payload: &CreateBlockPayload) -> String {
    format!("planning/blocks/{}.json", payload.id)
}

/// Whether `(repo, id)` resolves to a block somewhere in the loaded corpus —
/// any repo, since a `depends_on` edge may be cross-repo.
fn dependency_resolves(repo: &str, id: &str, files: &[(StateSource, StateFile)]) -> bool {
    files.iter().any(|(src, file)| {
        src.repo_slug == repo
            && file
                .tracks
                .iter()
                .any(|t| t.blocks.iter().any(|b| b.id == id))
    })
}

/// Next multiple of ten strictly past `max_wave` — the ticket/chore wave
/// rule. Deliberately **not** `max_wave + 1`: that lands inside the lattice
/// and silently interleaves one-offs with roadmap phases (see module docs
/// and the block record's own TRAP notes).
fn next_wave_past(max_wave: i64) -> i64 {
    ((max_wave.max(0) / 10) + 1) * 10
}

/// Allocate `wave` for a new block, per the two rules this block's
/// acceptance criteria pin: `10 * phase` for `kind: "block"`; the next
/// multiple of ten past the target repo's highest existing wave for
/// `kind: "ticket"` or `"chore"` — never `max + 1`.
///
/// `phase` is required to be `Some` for `kind: "block"` by
/// [`validate_payload`] (`E_BLOCK_CREATE_BLOCK_NEEDS_PHASE`); this function
/// trusts that precondition and does not re-check it.
fn allocate_wave(payload: &CreateBlockPayload, file: &StateFile) -> i64 {
    if payload.kind == "block" {
        return 10 * payload.phase.unwrap_or(0);
    }
    let max_wave = file
        .tracks
        .iter()
        .flat_map(|t| t.blocks.iter())
        .filter_map(|b| b.wave)
        .max()
        .unwrap_or(0);
    next_wave_past(max_wave)
}

/// The track title a new block of this `kind`/`phase` belongs under —
/// `"Phase {phase}"` for a roadmap block, `"Tickets"` / `"Chores"` for a
/// ticket or chore, matching `block-registration.md` step 5's convention.
fn target_track_title(payload: &CreateBlockPayload) -> String {
    match payload.kind.as_str() {
        "ticket" => "Tickets".to_string(),
        "chore" => "Chores".to_string(),
        _ => format!("Phase {}", payload.phase.unwrap_or(0)),
    }
}

/// Find the index of the track a new block belongs under — matching by
/// title *prefix* for a phase track (so "Phase 14" matches an existing
/// "Phase 14 — Orchestration extensions…" heading) and by exact title for
/// the "Tickets"/"Chores" catch-alls.
fn find_track_index(file: &StateFile, wanted_title: &str, is_phase: bool) -> Option<usize> {
    file.tracks.iter().position(|t| {
        if is_phase {
            t.title.starts_with(wanted_title)
        } else {
            t.title == wanted_title
        }
    })
}

/// Convert one planned edge's JSON representation from the `state.json`
/// shape (`BlockedBy`'s native serde, which glosses a block-type edge with
/// `"what"`) to the block-record shape (`block.schema.json` glosses the same
/// edge with `"why"`, and its `additionalProperties: false` rejects a
/// literal `null` for an absent optional field).
///
/// The two files' `depends_on` therefore cannot be byte-identical at the
/// JSON-key level for a glossed block edge — the schemas name the gloss
/// field differently — but every edge this planner writes carries the same
/// `(type, repo, id)` triple and the same gloss text into both records, so
/// the dependency graph the two files describe never disagrees, which is
/// what AC 7 ("byte-identical `depends_on` edges") protects against: one
/// record naming a dependency the other omits or contradicts.
fn depends_on_for_record(edges: &[BlockedBy]) -> Vec<serde_json::Value> {
    edges
        .iter()
        .map(|edge| {
            let mut value = serde_json::to_value(edge).expect("BlockedBy always serializes");
            if let serde_json::Value::Object(map) = &mut value {
                if map.get("type") == Some(&serde_json::Value::String("block".to_string())) {
                    if let Some(what) = map.remove("what")
                        && !what.is_null()
                    {
                        map.insert("why".to_string(), what);
                    }
                } else {
                    map.retain(|_, v| !v.is_null());
                }
            }
            value
        })
        .collect()
}

/// Build the `planning/blocks/<BlockID>.json` record content as a
/// [`serde_json::Value`] — matching `block.schema.json`'s field set exactly
/// (`additionalProperties: false`, so every key here must be a schema
/// property, and every absent-optional field must be omitted rather than
/// written `null`).
///
/// `spec_dir`/`created`/`updated` are derived here, never read from the
/// payload (see module docs); `epics` is deliberately **not** written — the
/// schema has no such property, and it belongs to the `state.json`
/// registration only.
pub fn build_block_record(
    payload: &CreateBlockPayload,
    created: &str,
    updated: &str,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("id".to_string(), json!(payload.id));
    map.insert("repo".to_string(), json!(payload.repo));
    map.insert("kind".to_string(), json!(payload.kind));
    if let Some(phase) = payload.phase {
        map.insert("phase".to_string(), json!(phase));
    }
    if let Some(initiative) = &payload.initiative {
        map.insert("initiative".to_string(), json!(initiative));
    }
    map.insert("title".to_string(), json!(payload.title));
    map.insert("description".to_string(), json!(payload.description));
    map.insert("what".to_string(), json!(payload.what));
    map.insert("why".to_string(), json!(payload.why));
    map.insert("sdlc_workflow".to_string(), json!(payload.sdlc_workflow));
    map.insert("model".to_string(), json!(payload.model));
    if let Some(rationale) = &payload.workflow_rationale {
        map.insert("workflow_rationale".to_string(), json!(rationale));
    }
    map.insert(
        "files".to_string(),
        serde_json::to_value(&payload.files).expect("BlockFiles always serializes"),
    );
    if !payload.interfaces.is_empty() {
        map.insert("interfaces".to_string(), json!(payload.interfaces));
    }
    map.insert("out_of_scope".to_string(), json!(payload.out_of_scope));
    map.insert(
        "acceptance_criteria".to_string(),
        serde_json::to_value(&payload.acceptance_criteria)
            .expect("AcceptanceCriterion always serializes"),
    );
    if let Some(strategy) = &payload.testing_strategy {
        map.insert("testing_strategy".to_string(), json!(strategy));
    }
    if !payload.validation_commands.is_empty() {
        map.insert(
            "validation_commands".to_string(),
            json!(payload.validation_commands),
        );
    }
    if !payload.depends_on.is_empty() {
        map.insert(
            "depends_on".to_string(),
            serde_json::Value::Array(depends_on_for_record(&payload.depends_on)),
        );
    }
    if !payload.carryover_context.is_empty() {
        map.insert(
            "carryover_context".to_string(),
            json!(payload.carryover_context),
        );
    }
    map.insert(
        "forward_looking".to_string(),
        json!(payload.forward_looking),
    );
    map.insert("spec_dir".to_string(), json!(derive_spec_dir(&payload.id)));
    if !payload.related.is_empty() {
        map.insert("related".to_string(), json!(payload.related));
    }
    if let Some(notes) = &payload.notes {
        map.insert("notes".to_string(), json!(notes));
    }
    map.insert("created".to_string(), json!(created));
    map.insert("updated".to_string(), json!(updated));
    serde_json::Value::Object(map)
}

/// Build the `state.json` `tracks[].blocks[]` entry for a new block —
/// `status: "open"`, the allocated `wave`, and the same `depends_on` edges
/// (see [`depends_on_for_record`]'s docs on why the two are logically, not
/// byte, identical) plus the authored `epics`.
fn build_track_block(payload: &CreateBlockPayload, wave: i64, created: &str) -> TrackBlock {
    TrackBlock {
        id: payload.id.clone(),
        title: payload.title.clone(),
        status: Some("open".to_string()),
        depends_on: payload.depends_on.clone(),
        wave: Some(wave),
        description: Some(payload.description.clone()),
        sdlc_workflow: Some(payload.sdlc_workflow.clone()),
        model: Some(payload.model.clone()),
        epics: payload.epics.clone(),
        created: Some(created.to_string()),
        ..TrackBlock::default()
    }
}

/// Plan filing a new block/ticket/chore: the `planning/blocks/<BlockID>.json`
/// record plus the matching `tracks[].blocks[]` registration in the target
/// repo's `state.json`. Both writes are expressed as one [`EmitPlan]`, the
/// same shape [`crate::brain::blocks::plan_set_block_status`] uses, so a
/// dry-run and a `--write` share one computation.
///
/// `today` is `YYYY-MM-DD`, injected by the caller rather than read from the
/// clock in here, so this stays a pure, deterministic function to test.
///
/// Diagnostics (each returns a plan with **zero actions** — nothing is ever
/// partially written):
/// - Every [`validate_payload`] diagnostic, unchanged, checked first.
/// - `E_BLOCK_CREATE_UNKNOWN_REPO` — `payload.repo` matches no loaded file.
/// - `E_BLOCK_CREATE_EXISTS` — `payload.id` already exists in the target
///   repo's `state.json`; a no-op refusal, never an overwrite.
/// - `E_BLOCK_CREATE_DANGLING_DEPENDENCY` — one or more `depends_on`
///   block-type edges name a `(repo, id)` that does not resolve anywhere in
///   the loaded corpus; every unresolved edge is named, not just the first.
///
/// On success, `plan.actions` carries exactly two [`EmitAction`]s: the new
/// block record (path `<repo_root>/planning/blocks/<id>.json`, resolved
/// against the target `StateSource`'s own `abs_path` so this plan works from
/// any working directory) and the target repo's rewritten `state.json`.
pub fn plan_create_block(
    payload: &CreateBlockPayload,
    files: &[(StateSource, StateFile)],
    today: &str,
) -> EmitPlan {
    let mut plan = EmitPlan::default();
    let record_path = record_repo_path(payload);

    // 1. Pure payload validation, unchanged from task 1.
    let payload_diags = validate_payload(payload);
    if !payload_diags.is_empty() {
        plan.diagnostics = payload_diags;
        return plan;
    }

    // 2. Resolve the target repo among loaded corpus files.
    let Some(target_index) = files
        .iter()
        .position(|(src, _)| src.repo_slug == payload.repo)
    else {
        let known: Vec<&str> = files
            .iter()
            .map(|(src, _)| src.repo_slug.as_str())
            .collect();
        plan.diagnostics.push(Diagnostic::error(
            &record_path,
            E_BLOCK_CREATE_UNKNOWN_REPO,
            format!(
                "no loaded repo is named '{}'; known repos: {}",
                payload.repo,
                known.join(", ")
            ),
        ));
        return plan;
    };

    // 3. An existing id is a no-op refusal, never an overwrite.
    let already_exists = files[target_index]
        .1
        .tracks
        .iter()
        .any(|t| t.blocks.iter().any(|b| b.id == payload.id));
    if already_exists {
        plan.diagnostics.push(Diagnostic::error(
            &record_path,
            E_BLOCK_CREATE_EXISTS,
            format!(
                "block '{}' already exists in repo '{}'; refusing to overwrite — creation only \
                 files new blocks",
                payload.id, payload.repo
            ),
        ));
        return plan;
    }

    // 4. Every block-type depends_on edge must resolve somewhere in the
    //    loaded corpus. Collect every dangling edge, not just the first, so
    //    a caller sees the whole problem in one round trip (same discipline
    //    as validate_payload).
    for edge in &payload.depends_on {
        if let BlockedBy::Block(dep) = edge
            && !dependency_resolves(&dep.repo, &dep.id, files)
        {
            plan.diagnostics.push(Diagnostic::error(
                &record_path,
                E_BLOCK_CREATE_DANGLING_DEPENDENCY,
                format!(
                    "depends_on names '{}:{}', which does not resolve in the loaded corpus — \
                     create the dependency before the dependent",
                    dep.repo, dep.id
                ),
            ));
        }
    }
    if !plan.diagnostics.is_empty() {
        return plan;
    }

    // 5. Wave allocation, per the two rules this block's ACs pin.
    let wave = allocate_wave(payload, &files[target_index].1);

    // 6. Find-or-create the target track, then append the new TrackBlock.
    let is_phase = payload.kind == "block";
    let wanted_title = target_track_title(payload);
    let mut work_file = files[target_index].1.clone();
    let track_index = match find_track_index(&work_file, &wanted_title, is_phase) {
        Some(idx) => idx,
        None => {
            work_file.tracks.push(Track {
                title: wanted_title.clone(),
                blocks: Vec::new(),
                extra: serde_json::Map::new(),
            });
            work_file.tracks.len() - 1
        }
    };
    let track_block = build_track_block(payload, wave, today);
    work_file.tracks[track_index].blocks.push(track_block);

    // 7. Plan the state.json write.
    let note = format!(
        "create block '{}:{}' (wave {wave})",
        payload.repo, payload.id
    );
    if let Some(action) = crate::brain::epics::action_for(&files[target_index].0, &work_file, note)
    {
        plan.actions.push(action);
    }

    // 8. Plan the block record write, resolved next to the target repo's
    //    own state.json (its abs_path is <repo_root>/planning/state.json).
    let repo_root = files[target_index]
        .0
        .abs_path
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let record = build_block_record(payload, today, today);
    let mut record_content =
        serde_json::to_string_pretty(&record).expect("block record always serializes");
    record_content.push('\n');
    plan.actions.push(EmitAction {
        path: repo_root.join(&record_path),
        new_content: record_content,
        note: format!("file new block record '{}'", payload.id),
    });

    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal, fully-legal payload — every test mutates a field off of
    /// this baseline rather than repeating the whole literal.
    fn legal_payload() -> CreateBlockPayload {
        CreateBlockPayload {
            id: "MV.99.A".to_string(),
            repo: "mev".to_string(),
            kind: "block".to_string(),
            title: "Test block".to_string(),
            description: "A block used only in a test fixture.".to_string(),
            what: "Does the thing the test needs done.".to_string(),
            why: "Because the test needs a legal payload to mutate.".to_string(),
            sdlc_workflow: "task".to_string(),
            model: "sonnet".to_string(),
            phase: Some(99),
            initiative: None,
            workflow_rationale: None,
            files: BlockFiles::default(),
            interfaces: Vec::new(),
            out_of_scope: vec!["Everything else.".to_string()],
            acceptance_criteria: vec![AcceptanceCriterion::Simple("It works.".to_string())],
            testing_strategy: None,
            validation_commands: Vec::new(),
            depends_on: Vec::new(),
            carryover_context: Vec::new(),
            related: Vec::new(),
            notes: None,
            forward_looking: false,
            epics: vec!["test-epic".to_string()],
        }
    }

    fn codes(diags: &[Diagnostic]) -> Vec<&str> {
        diags.iter().map(|d| d.locator.as_str()).collect()
    }

    #[test]
    fn legal_payload_has_no_diagnostics() {
        let diags = validate_payload(&legal_payload());
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    #[test]
    fn kind_out_of_vocabulary_is_rejected() {
        let mut p = legal_payload();
        p.kind = "epic".to_string();
        let diags = validate_payload(&p);
        assert!(codes(&diags).contains(&E_BLOCK_CREATE_KIND_ENUM));
    }

    #[test]
    fn sdlc_workflow_out_of_vocabulary_is_rejected() {
        let mut p = legal_payload();
        p.sdlc_workflow = "orchestrate".to_string();
        let diags = validate_payload(&p);
        assert!(codes(&diags).contains(&E_BLOCK_CREATE_SDLC_WORKFLOW_ENUM));
    }

    /// The trap the record itself names: `"opus"` must be rejected — it is
    /// the engines' stage-model vocabulary, not the block record's.
    #[test]
    fn model_opus_is_rejected() {
        let mut p = legal_payload();
        p.model = "opus".to_string();
        let diags = validate_payload(&p);
        assert!(
            codes(&diags).contains(&E_BLOCK_CREATE_MODEL_ENUM),
            "expected 'opus' to be rejected as a model value, got {diags:?}"
        );
    }

    #[test]
    fn model_out_of_vocabulary_is_rejected() {
        let mut p = legal_payload();
        p.model = "gpt-4".to_string();
        let diags = validate_payload(&p);
        assert!(codes(&diags).contains(&E_BLOCK_CREATE_MODEL_ENUM));
    }

    #[test]
    fn missing_epics_is_refused() {
        let mut p = legal_payload();
        p.epics = Vec::new();
        let diags = validate_payload(&p);
        assert!(
            codes(&diags).contains(&E_BLOCK_CREATE_MISSING_EPICS),
            "expected an empty epics list to be refused, got {diags:?}"
        );
    }

    #[test]
    fn empty_out_of_scope_is_rejected() {
        let mut p = legal_payload();
        p.out_of_scope = Vec::new();
        let diags = validate_payload(&p);
        assert!(codes(&diags).contains(&E_BLOCK_CREATE_EMPTY_OUT_OF_SCOPE));
    }

    #[test]
    fn empty_acceptance_criteria_is_rejected() {
        let mut p = legal_payload();
        p.acceptance_criteria = Vec::new();
        let diags = validate_payload(&p);
        assert!(codes(&diags).contains(&E_BLOCK_CREATE_EMPTY_ACCEPTANCE_CRITERIA));
    }

    #[test]
    fn block_kind_without_phase_is_rejected() {
        let mut p = legal_payload();
        p.phase = None;
        let diags = validate_payload(&p);
        assert!(codes(&diags).contains(&E_BLOCK_CREATE_BLOCK_NEEDS_PHASE));
    }

    #[test]
    fn ticket_kind_without_testing_strategy_is_rejected() {
        let mut p = legal_payload();
        p.kind = "ticket".to_string();
        p.testing_strategy = None;
        let diags = validate_payload(&p);
        assert!(codes(&diags).contains(&E_BLOCK_CREATE_TICKET_NEEDS_TESTING_STRATEGY));
    }

    /// A ticket kind that DOES carry testing_strategy needs no `phase` and
    /// raises neither conditional diagnostic.
    #[test]
    fn ticket_kind_with_testing_strategy_and_no_phase_is_legal() {
        let mut p = legal_payload();
        p.kind = "ticket".to_string();
        p.phase = None;
        p.testing_strategy = Some("Covered by tests/it/some_test.rs.".to_string());
        let diags = validate_payload(&p);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    #[test]
    fn empty_required_string_fields_are_rejected() {
        let mut p = legal_payload();
        p.title = String::new();
        p.why = "   ".to_string();
        let diags = validate_payload(&p);
        let empty_field_count = diags
            .iter()
            .filter(|d| d.locator == E_BLOCK_CREATE_EMPTY_FIELD)
            .count();
        assert_eq!(
            empty_field_count, 2,
            "expected exactly two empty-field diagnostics (title, why), got {diags:?}"
        );
    }

    #[test]
    fn spec_dir_is_derived_from_id_and_matches_schema_pattern() {
        assert_eq!(derive_spec_dir("MV.14.B"), "planning/MV.14.B/");
        assert_eq!(
            derive_spec_dir("BA.ticket.fix-null-deref"),
            "planning/BA.ticket.fix-null-deref/"
        );
    }

    /// `CreateBlockPayload` accepts `--from <file>` JSON that omits every
    /// optional field — the deserialization target must not require fields
    /// the schema itself marks optional.
    #[test]
    fn payload_deserializes_from_minimal_json() {
        let raw = r#"{
            "id": "MV.99.B",
            "repo": "mev",
            "kind": "chore",
            "title": "Minimal",
            "description": "A minimal payload for a deserialization test.",
            "what": "Nothing much.",
            "why": "To prove optional fields default.",
            "sdlc_workflow": "none",
            "model": "either",
            "out_of_scope": ["Everything."],
            "acceptance_criteria": ["It parses."],
            "epics": ["test-epic"]
        }"#;
        let payload: CreateBlockPayload =
            serde_json::from_str(raw).expect("minimal payload should deserialize");
        assert_eq!(payload.id, "MV.99.B");
        assert!(payload.files.new.is_empty());
        assert!(payload.depends_on.is_empty());
        assert!(payload.phase.is_none());
        let diags = validate_payload(&payload);
        // kind=chore needs neither phase nor testing_strategy.
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    /// The detailed (`{criterion, gateable, evidence}`) acceptance-criterion
    /// form round-trips and its text is retrievable the same way as the
    /// simple string form.
    #[test]
    fn acceptance_criterion_detailed_form_round_trips() {
        let raw = r#"{"criterion": "Runs in CI.", "gateable": false, "evidence": "manual check"}"#;
        let ac: AcceptanceCriterion = serde_json::from_str(raw).expect("detailed form parses");
        assert_eq!(ac.criterion_text(), "Runs in CI.");
        match &ac {
            AcceptanceCriterion::Detailed {
                gateable, evidence, ..
            } => {
                assert_eq!(*gateable, Some(false));
                assert_eq!(evidence.as_deref(), Some("manual check"));
            }
            AcceptanceCriterion::Simple(_) => panic!("expected Detailed form"),
        }
    }

    #[test]
    fn depends_on_reuses_okf_core_blocked_by_shape() {
        let raw = r#"{
            "id": "MV.99.C",
            "repo": "mev",
            "kind": "block",
            "title": "Depends",
            "description": "Has a dependency edge.",
            "what": "Something.",
            "why": "To test depends_on parsing.",
            "sdlc_workflow": "task",
            "model": "sonnet",
            "phase": 99,
            "out_of_scope": ["Nothing else."],
            "acceptance_criteria": ["It has a dependency."],
            "epics": ["test-epic"],
            "depends_on": [
                {"type": "block", "repo": "mev", "id": "MV.14.A", "why": "prereq"}
            ]
        }"#;
        let payload: CreateBlockPayload =
            serde_json::from_str(raw).expect("payload with depends_on should deserialize");
        assert_eq!(payload.depends_on.len(), 1);
        match &payload.depends_on[0] {
            BlockedBy::Block(dep) => {
                assert_eq!(dep.repo, "mev");
                assert_eq!(dep.id, "MV.14.A");
            }
            other => panic!("expected BlockedBy::Block, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod planning_tests {
    use super::*;

    /// One state file for `repo`, with `tracks` built from `(title, blocks)`
    /// pairs, sourced from a real temp path so [`crate::brain::epics::action_for`]'s
    /// on-disk comparison has something to read — same fixture discipline as
    /// `blocks.rs`'s `file_for`.
    fn file_for(
        dir: &Path,
        repo: &str,
        tracks: &[(&str, &[(&str, i64)])],
    ) -> (StateSource, StateFile) {
        let abs_path = dir.join(repo).join("planning").join("state.json");
        std::fs::create_dir_all(abs_path.parent().unwrap()).unwrap();

        let track_json: Vec<String> = tracks
            .iter()
            .map(|(title, blocks)| {
                let block_json: Vec<String> = blocks
                    .iter()
                    .map(|(id, wave)| {
                        format!(r#"{{ "id": "{id}", "title": "{id}", "status": "open", "wave": {wave} }}"#)
                    })
                    .collect();
                format!(r#"{{ "title": "{title}", "blocks": [{}] }}"#, block_json.join(",\n"))
            })
            .collect();
        let raw = format!(
            r#"{{ "repo": "{repo}", "kind": "project", "updated": "2026-09-01",
  "focus": {{ "now": [], "next": [], "blocked": [] }},
  "tracks": [{}] }}"#,
            track_json.join(",\n")
        );
        let file: StateFile = serde_json::from_str(&raw).expect("fixture state.json");

        let mut content = serde_json::to_string_pretty(&file).unwrap();
        content.push('\n');
        std::fs::write(&abs_path, content).unwrap();

        let src = StateSource {
            repo_slug: repo.to_string(),
            abs_path,
            expected_kind: "project",
        };
        (src, file)
    }

    /// A minimal, fully-legal `kind: "block"` payload targeting repo `"mev"`
    /// at phase 99 (wave 990) — every test mutates a field off of this
    /// baseline.
    fn legal_block_payload() -> CreateBlockPayload {
        CreateBlockPayload {
            id: "MV.99.NEW".to_string(),
            repo: "mev".to_string(),
            kind: "block".to_string(),
            title: "A new block".to_string(),
            description: "A block created by a planning test.".to_string(),
            what: "Does the thing the test needs done.".to_string(),
            why: "Because the test needs a legal payload to plan.".to_string(),
            sdlc_workflow: "task".to_string(),
            model: "sonnet".to_string(),
            phase: Some(99),
            initiative: None,
            workflow_rationale: None,
            files: BlockFiles::default(),
            interfaces: Vec::new(),
            out_of_scope: vec!["Everything else.".to_string()],
            acceptance_criteria: vec![AcceptanceCriterion::Simple("It works.".to_string())],
            testing_strategy: None,
            validation_commands: Vec::new(),
            depends_on: Vec::new(),
            carryover_context: Vec::new(),
            related: Vec::new(),
            notes: None,
            forward_looking: false,
            epics: vec!["test-epic".to_string()],
        }
    }

    fn codes(plan: &EmitPlan) -> Vec<String> {
        plan.diagnostics.iter().map(|d| d.locator.clone()).collect()
    }

    // -----------------------------------------------------------------
    // Wave allocation
    // -----------------------------------------------------------------

    #[test]
    fn block_wave_is_ten_times_phase() {
        assert_eq!(next_wave_past(0), 10);
        assert_eq!(next_wave_past(23), 30);
        assert_eq!(next_wave_past(30), 40); // never max + 1
        assert_eq!(next_wave_past(9), 10);
    }

    #[test]
    fn creating_a_block_allocates_ten_times_phase_wave() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[])];
        let mut payload = legal_block_payload();
        payload.phase = Some(14);
        let plan = plan_create_block(&payload, &files, "2026-09-02");
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
        assert_eq!(plan.actions.len(), 2, "{plan:?}");
        let state_action = plan
            .actions
            .iter()
            .find(|a| a.path.ends_with("state.json"))
            .expect("a state.json action");
        assert!(
            state_action.new_content.contains("\"wave\": 140"),
            "{}",
            state_action.new_content
        );
    }

    #[test]
    fn creating_a_ticket_allocates_next_multiple_of_ten_past_highest_wave() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("Phase 14", &[("MV.14.A", 140), ("MV.14.B", 141)])],
        )];
        let mut payload = legal_block_payload();
        payload.id = "MV.ticket.new-thing".to_string();
        payload.kind = "ticket".to_string();
        payload.phase = None;
        payload.testing_strategy = Some("Covered by a fixture test.".to_string());
        let plan = plan_create_block(&payload, &files, "2026-09-02");
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
        let state_action = plan
            .actions
            .iter()
            .find(|a| a.path.ends_with("state.json"))
            .expect("a state.json action");
        // Highest existing wave is 141 -> next multiple of ten past it is 150,
        // never 142 (max + 1).
        assert!(
            state_action.new_content.contains("\"wave\": 150"),
            "{}",
            state_action.new_content
        );
    }

    // -----------------------------------------------------------------
    // depends_on resolution
    // -----------------------------------------------------------------

    #[test]
    fn dangling_dependency_is_refused_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[])];
        let mut payload = legal_block_payload();
        payload.depends_on = vec![BlockedBy::Block(crate::brain::state::BlockDep {
            repo: "mev".to_string(),
            id: "MV.NOPE.X".to_string(),
            what: None,
        })];
        let plan = plan_create_block(&payload, &files, "2026-09-02");
        assert!(plan.actions.is_empty(), "{plan:?}");
        assert_eq!(codes(&plan), vec![E_BLOCK_CREATE_DANGLING_DEPENDENCY]);
        assert!(plan.diagnostics[0].message.contains("mev:MV.NOPE.X"));
    }

    #[test]
    fn dependency_before_dependent_ordering_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[])];

        // The dependent cannot be created first: its target does not exist yet.
        let mut dependent = legal_block_payload();
        dependent.id = "MV.99.DEPENDENT".to_string();
        dependent.depends_on = vec![BlockedBy::Block(crate::brain::state::BlockDep {
            repo: "mev".to_string(),
            id: "MV.99.DEP".to_string(),
            what: Some("prereq".to_string()),
        })];
        let plan = plan_create_block(&dependent, &files, "2026-09-02");
        assert_eq!(codes(&plan), vec![E_BLOCK_CREATE_DANGLING_DEPENDENCY]);

        // Create the dependency first — it has no depends_on of its own.
        let mut dependency = legal_block_payload();
        dependency.id = "MV.99.DEP".to_string();
        let dep_plan = plan_create_block(&dependency, &files, "2026-09-02");
        assert!(
            dep_plan.diagnostics.is_empty(),
            "{:?}",
            dep_plan.diagnostics
        );
        // Apply the dependency's state.json action to a fresh file list.
        let state_action = dep_plan
            .actions
            .iter()
            .find(|a| a.path.ends_with("state.json"))
            .unwrap();
        std::fs::write(&state_action.path, &state_action.new_content).unwrap();
        let updated_file: StateFile =
            serde_json::from_str(&state_action.new_content).expect("written state.json parses");
        let files_after = vec![(files[0].0.clone(), updated_file)];

        // Now the dependent resolves.
        let plan2 = plan_create_block(&dependent, &files_after, "2026-09-02");
        assert!(plan2.diagnostics.is_empty(), "{:?}", plan2.diagnostics);
        assert_eq!(plan2.actions.len(), 2);
    }

    // -----------------------------------------------------------------
    // depends_on parity between the block record and state.json
    // -----------------------------------------------------------------

    #[test]
    fn depends_on_parity_between_record_and_state_json() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("Phase 14", &[("MV.14.A", 140)])],
        )];
        let mut payload = legal_block_payload();
        payload.depends_on = vec![BlockedBy::Block(crate::brain::state::BlockDep {
            repo: "mev".to_string(),
            id: "MV.14.A".to_string(),
            what: Some("a real dependency".to_string()),
        })];
        let plan = plan_create_block(&payload, &files, "2026-09-02");
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);

        let record_action = plan
            .actions
            .iter()
            .find(|a| a.path.to_string_lossy().contains("planning/blocks"))
            .expect("a block record action");
        let record: serde_json::Value = serde_json::from_str(&record_action.new_content).unwrap();
        let record_dep = &record["depends_on"][0];
        assert_eq!(record_dep["type"], "block");
        assert_eq!(record_dep["repo"], "mev");
        assert_eq!(record_dep["id"], "MV.14.A");
        assert_eq!(record_dep["why"], "a real dependency");
        assert!(record_dep.get("what").is_none(), "{record_dep:?}");

        let state_action = plan
            .actions
            .iter()
            .find(|a| a.path.ends_with("state.json"))
            .unwrap();
        let state_file: StateFile = serde_json::from_str(&state_action.new_content).unwrap();
        let new_block = state_file
            .tracks
            .iter()
            .flat_map(|t| t.blocks.iter())
            .find(|b| b.id == payload.id)
            .expect("new block present");
        match &new_block.depends_on[0] {
            BlockedBy::Block(dep) => {
                assert_eq!(dep.repo, "mev");
                assert_eq!(dep.id, "MV.14.A");
                assert_eq!(dep.what.as_deref(), Some("a real dependency"));
            }
            other => panic!("expected BlockedBy::Block, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Existing-id refusal, unknown repo, dry-run parity
    // -----------------------------------------------------------------

    #[test]
    fn existing_block_id_is_a_no_op_refusal_not_an_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("Phase 99", &[("MV.99.NEW", 990)])],
        )];
        let plan = plan_create_block(&legal_block_payload(), &files, "2026-09-02");
        assert!(plan.actions.is_empty(), "{plan:?}");
        assert_eq!(codes(&plan), vec![E_BLOCK_CREATE_EXISTS]);
    }

    #[test]
    fn unknown_repo_is_refused_with_known_repos_named() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "bella", &[])];
        let plan = plan_create_block(&legal_block_payload(), &files, "2026-09-02");
        assert_eq!(codes(&plan), vec![E_BLOCK_CREATE_UNKNOWN_REPO]);
        assert!(plan.diagnostics[0].message.contains("bella"));
    }

    #[test]
    fn planning_never_mutates_the_caller_slice() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[])];
        let _ = plan_create_block(&legal_block_payload(), &files, "2026-09-02");
        assert!(
            files[0].1.tracks.is_empty(),
            "planning must not mutate the caller's corpus: {:?}",
            files[0].1.tracks
        );
    }

    #[test]
    fn missing_epics_never_reaches_planning_diagnostics() {
        // validate_payload's diagnostics are checked first and returned as-is;
        // no repo/id/dependency work happens once a payload is invalid.
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[])];
        let mut payload = legal_block_payload();
        payload.epics = Vec::new();
        let plan = plan_create_block(&payload, &files, "2026-09-02");
        assert!(plan.actions.is_empty(), "{plan:?}");
        assert_eq!(codes(&plan), vec![E_BLOCK_CREATE_MISSING_EPICS]);
    }
}
