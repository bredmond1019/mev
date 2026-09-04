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
use crate::brain::state::{Backlog, BlockedBy, StateFile, StateSource, Track, TrackBlock};

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

// ---------------------------------------------------------------------------
// demote-block / promote-block — park a block into backlog[] and restore it
// ---------------------------------------------------------------------------
//
// `create-block`'s inverse (MV.ticket.demote-block-to-backlog, Task 2). D12
// settled the two open design choices this pair implements:
// - Choice 1 (record pointer): a new field on the JSON, not an overload of
//   `Backlog::block`. Carried through `Backlog::extra` (its
//   `#[serde(flatten, default)]` capture) rather than a typed field on
//   `okf-core::Backlog` — this task's `files[]` is mev-only, and `extra`
//   already gives every older consumer the exact tolerance D12's
//   Investigation verified a typed field would need `#[serde(default,
//   skip_serializing_if)]` for. See
//   `planning/decisions/D12-demote-block-backlog-record-pointer.md`.
// - Choice 2 (status name): `"parked"` — [`BACKLOG_STATUS_PARKED`].

/// New `VALID_BACKLOG_STATUSES` value (D12 choice 2): a real block, parked on
/// purpose, with its `planning/blocks/<ID>.json` record intact. Distinct from
/// `"idea"` structurally, not just by name: a `parked` entry always carries a
/// [`BACKLOG_EXTRA_RECORD`] pointer; an `idea` never does.
pub const BACKLOG_STATUS_PARKED: &str = "parked";

/// Diagnostic code: `demote-block`'s target block has no record on disk — the
/// record staying in place is the whole feature, so there would be nothing
/// for the backlog pointer to point at.
pub const E_DEMOTE_BLOCK_RECORD_MISSING: &str = "E_DEMOTE_BLOCK_RECORD_MISSING";

/// Diagnostic code: `promote-block`'s target backlog slug is not
/// `status: "parked"` — nothing to restore, or it was already restored.
pub const E_PROMOTE_BLOCK_NOT_PARKED: &str = "E_PROMOTE_BLOCK_NOT_PARKED";

/// Diagnostic code: a `parked` backlog entry carries no restorable
/// `tracks[].blocks[]` snapshot — hand-edited or corrupt state.
pub const E_PROMOTE_BLOCK_MISSING_SNAPSHOT: &str = "E_PROMOTE_BLOCK_MISSING_SNAPSHOT";

/// Diagnostic code: `promote-block`'s target `id` already exists in the
/// repo's `tracks[]` — refuse rather than overwrite.
pub const E_PROMOTE_BLOCK_EXISTS: &str = "E_PROMOTE_BLOCK_EXISTS";

/// `Backlog::extra` key: repo-relative path to the retained
/// `planning/blocks/<ID>.json` record (D12 choice 1). Read by
/// `state::check_backlog_integrity`'s dangling-record check.
pub const BACKLOG_EXTRA_RECORD: &str = "record";
/// `Backlog::extra` key: the full removed `TrackBlock`, serialized verbatim,
/// so `promote-block` restores it with no field lost (AC4's round trip).
const BACKLOG_EXTRA_PARKED_BLOCK: &str = "parked_block";
/// `Backlog::extra` key: the title of the track the block was removed from,
/// so `promote-block` re-inserts it under the same track.
const BACKLOG_EXTRA_PARKED_TRACK: &str = "parked_track";

/// Repo-relative record path for a bare block id — same shape as
/// [`record_repo_path`], but taking an id directly since demote/promote key
/// off `repo:id`, not a full [`CreateBlockPayload`].
fn record_repo_path_for_id(id: &str) -> String {
    format!("planning/blocks/{id}.json")
}

/// Split a `repo:id` key. Same shape as `blocks::split_key` (private there,
/// three lines — duplicated rather than exposed cross-module).
fn split_repo_id_key(key: &str) -> Option<(&str, &str)> {
    let (repo, id) = key.split_once(':')?;
    if repo.is_empty() || id.is_empty() {
        return None;
    }
    Some((repo, id))
}

/// Plan `mev demote-block <repo:id>`: remove the `tracks[].blocks[]` row and
/// append a `backlog[]` entry carrying the block id, a pointer to the
/// retained record, and `status: "parked"`.
///
/// `planning/blocks/<id>.json` is never written — this function only checks
/// it exists (to read `kind` and to have something for the pointer to name);
/// no [`EmitAction`] this plans ever targets it. Losing that guarantee is
/// exactly the regression the record's `out_of_scope` names.
///
/// Reuses `set-block-status`'s key-resolution refusals verbatim (same
/// locator strings) rather than inventing new ones, per the task
/// description:
/// - `E_BLOCK_BAD_KEY` — `key` is not `repo:id`.
/// - `E_BLOCK_NOT_FOUND` — no loaded file's `tracks[]` owns that block.
/// - [`E_DEMOTE_BLOCK_RECORD_MISSING`] — the block resolves but its record
///   file is not on disk.
///
/// Every diagnostic returns a plan with zero actions — nothing is ever
/// partially written.
pub fn plan_demote_block(key: &str, files: &[(StateSource, StateFile)], today: &str) -> EmitPlan {
    let mut plan = EmitPlan::default();
    let here = Path::new(".");

    let Some((repo_slug, block_id)) = split_repo_id_key(key) else {
        plan.diagnostics.push(Diagnostic::error(
            here,
            "E_BLOCK_BAD_KEY",
            format!(
                "block key '{key}' is not in 'repo:id' form (e.g. 'mev:MV.10.A'); block ids are \
                 only unique within a repo, so an unqualified id is ambiguous and is not guessed"
            ),
        ));
        return plan;
    };

    let mut found: Option<(usize, usize, usize)> = None;
    for (fi, (src, file)) in files.iter().enumerate() {
        if src.repo_slug != repo_slug {
            continue;
        }
        for (ti, track) in file.tracks.iter().enumerate() {
            for (bi, block) in track.blocks.iter().enumerate() {
                if block.id == block_id {
                    found = Some((fi, ti, bi));
                }
            }
        }
    }
    let Some((fi, ti, bi)) = found else {
        plan.diagnostics.push(Diagnostic::error(
            here,
            "E_BLOCK_NOT_FOUND",
            format!("block '{key}' not found in any loaded repo's tracks[]"),
        ));
        return plan;
    };

    let repo_root = files[fi]
        .0
        .abs_path
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let record_rel = record_repo_path_for_id(block_id);
    let record_abs = repo_root.join(&record_rel);
    if !record_abs.exists() {
        plan.diagnostics.push(Diagnostic::error(
            &record_rel,
            E_DEMOTE_BLOCK_RECORD_MISSING,
            format!(
                "block '{key}' has no record at '{}' — demote-block parks a block whose record \
                 survives on disk, so there would be nothing for the backlog pointer to name",
                record_abs.display()
            ),
        ));
        return plan;
    }

    // `kind` has no home on TrackBlock (block.schema.json's field; only the
    // record carries it) — read it from the untouched record rather than
    // guess. Existence was just checked, so a read/parse failure here is
    // treated as "unknown" rather than re-diagnosed.
    let kind = std::fs::read_to_string(&record_abs)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(str::to_string))
        .unwrap_or_else(|| "block".to_string());

    let mut work: Vec<(StateSource, StateFile)> = files.to_vec();
    let removed_track_title = work[fi].1.tracks[ti].title.clone();
    let removed_block = work[fi].1.tracks[ti].blocks.remove(bi);

    let mut extra = serde_json::Map::new();
    extra.insert(BACKLOG_EXTRA_RECORD.to_string(), json!(record_rel));
    extra.insert(
        BACKLOG_EXTRA_PARKED_BLOCK.to_string(),
        serde_json::to_value(&removed_block).expect("TrackBlock always serializes"),
    );
    extra.insert(
        BACKLOG_EXTRA_PARKED_TRACK.to_string(),
        json!(removed_track_title),
    );

    let backlog_entry = Backlog {
        slug: block_id.to_string(),
        title: removed_block.title.clone(),
        repo: repo_slug.to_string(),
        kind,
        status: BACKLOG_STATUS_PARKED.to_string(),
        depends_on: removed_block.depends_on.clone(),
        block: None,
        notes: None,
        origin: None,
        created: Some(today.to_string()),
        reviewed: None,
        snoozed_until: None,
        extra,
    };
    work[fi].1.backlog.push(backlog_entry);

    let note = format!("demote block '{key}' into backlog[] (parked)");
    if let Some(action) = crate::brain::epics::action_for(&work[fi].0, &work[fi].1, note) {
        plan.actions.push(action);
    }

    plan
}

/// Plan `mev promote-block <repo:id>`: the inverse of [`plan_demote_block`].
/// Restores the exact `tracks[].blocks[]` row a matching `parked` backlog
/// entry carries in its snapshot, and marks that backlog entry
/// `status: "promoted"` with `block` set to the same id — never deleting the
/// backlog entry, matching how a normal promotion leaves its origin behind
/// (see [`Backlog::block`]'s doc comment).
///
/// Diagnostics (each returns a plan with zero actions):
/// - `E_BLOCK_BAD_KEY` — `key` is not `repo:id`.
/// - `E_BLOCK_NOT_FOUND` — no loaded file's `backlog[]` carries that slug in
///   that repo.
/// - [`E_PROMOTE_BLOCK_NOT_PARKED`] — the slug exists but is not
///   `status: "parked"` (nothing to restore, or already restored).
/// - [`E_PROMOTE_BLOCK_EXISTS`] — a block with this id is already registered
///   in the target repo's `tracks[]`.
/// - [`E_PROMOTE_BLOCK_MISSING_SNAPSHOT`] — the entry is `parked` but carries
///   no restorable snapshot, or the snapshot doesn't deserialize as a
///   `TrackBlock` (hand-edited or corrupt).
pub fn plan_promote_block(key: &str, files: &[(StateSource, StateFile)]) -> EmitPlan {
    let mut plan = EmitPlan::default();
    let here = Path::new(".");

    let Some((repo_slug, block_id)) = split_repo_id_key(key) else {
        plan.diagnostics.push(Diagnostic::error(
            here,
            "E_BLOCK_BAD_KEY",
            format!("block key '{key}' is not in 'repo:id' form (e.g. 'mev:MV.10.A')"),
        ));
        return plan;
    };

    let mut found: Option<(usize, usize)> = None;
    for (fi, (src, file)) in files.iter().enumerate() {
        if src.repo_slug != repo_slug {
            continue;
        }
        for (bi, entry) in file.backlog.iter().enumerate() {
            if entry.slug == block_id {
                found = Some((fi, bi));
            }
        }
    }
    let Some((fi, bi)) = found else {
        plan.diagnostics.push(Diagnostic::error(
            here,
            "E_BLOCK_NOT_FOUND",
            format!("backlog slug '{key}' not found in any loaded repo's backlog[]"),
        ));
        return plan;
    };

    if files[fi].1.backlog[bi].status != BACKLOG_STATUS_PARKED {
        plan.diagnostics.push(Diagnostic::error(
            here,
            E_PROMOTE_BLOCK_NOT_PARKED,
            format!(
                "backlog slug '{key}' has status '{}', not 'parked' — nothing to restore",
                files[fi].1.backlog[bi].status
            ),
        ));
        return plan;
    }

    let already_exists = files[fi]
        .1
        .tracks
        .iter()
        .any(|t| t.blocks.iter().any(|b| b.id == block_id));
    if already_exists {
        plan.diagnostics.push(Diagnostic::error(
            here,
            E_PROMOTE_BLOCK_EXISTS,
            format!(
                "block '{key}' already exists in repo '{repo_slug}''s tracks[]; refusing to \
                 overwrite"
            ),
        ));
        return plan;
    }

    let Some(snapshot) = files[fi].1.backlog[bi]
        .extra
        .get(BACKLOG_EXTRA_PARKED_BLOCK)
    else {
        plan.diagnostics.push(Diagnostic::error(
            here,
            E_PROMOTE_BLOCK_MISSING_SNAPSHOT,
            format!(
                "backlog slug '{key}' is 'parked' but carries no '{BACKLOG_EXTRA_PARKED_BLOCK}' \
                 snapshot to restore"
            ),
        ));
        return plan;
    };
    let Ok(restored_block) = serde_json::from_value::<TrackBlock>(snapshot.clone()) else {
        plan.diagnostics.push(Diagnostic::error(
            here,
            E_PROMOTE_BLOCK_MISSING_SNAPSHOT,
            format!(
                "backlog slug '{key}''s '{BACKLOG_EXTRA_PARKED_BLOCK}' snapshot does not \
                 deserialize as a track block"
            ),
        ));
        return plan;
    };
    let track_title = files[fi].1.backlog[bi]
        .extra
        .get(BACKLOG_EXTRA_PARKED_TRACK)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "Tickets".to_string());

    let mut work: Vec<(StateSource, StateFile)> = files.to_vec();
    let track_index = match work[fi]
        .1
        .tracks
        .iter()
        .position(|t| t.title == track_title)
    {
        Some(idx) => idx,
        None => {
            work[fi].1.tracks.push(Track {
                title: track_title.clone(),
                blocks: Vec::new(),
                extra: serde_json::Map::new(),
            });
            work[fi].1.tracks.len() - 1
        }
    };
    work[fi].1.tracks[track_index].blocks.push(restored_block);

    let entry = &mut work[fi].1.backlog[bi];
    entry.status = "promoted".to_string();
    entry.block = Some(block_id.to_string());
    entry.extra.remove(BACKLOG_EXTRA_RECORD);
    entry.extra.remove(BACKLOG_EXTRA_PARKED_BLOCK);
    entry.extra.remove(BACKLOG_EXTRA_PARKED_TRACK);

    let note = format!("promote parked backlog slug '{key}' back into tracks[]");
    if let Some(action) = crate::brain::epics::action_for(&work[fi].0, &work[fi].1, note) {
        plan.actions.push(action);
    }

    plan
}

/// Full driver for `mev demote-block <repo:id>`, same shape as
/// [`crate::create_block`] / [`crate::set_block_status`]: resolve
/// `brain.toml`, discover + load every `state.json`, refuse to write against
/// an incomplete corpus, plan via [`plan_demote_block`], apply, and — on a
/// successful `--write` — re-run `emit-state --write` so the boards agree
/// with the demoted block in the same invocation.
///
/// Lives here rather than in `lib.rs` (unlike its siblings) — this task's
/// scope is this module plus `main.rs`/`state.rs`; the driver shape is
/// copied, not moved.
pub fn demote_block(
    root: &Path,
    key: &str,
    write: bool,
    scope: Option<&crate::brain::config::ScopeDependencySet>,
) -> anyhow::Result<crate::Report> {
    use crate::brain::config::find_brain_config;
    use crate::brain::emit::apply_plan;
    use crate::brain::state::{StateLoadError, discover_state_files, load_state};

    let mut report = crate::Report::default();

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    let (sources, discovery_diags) = discover_state_files(root, &config);
    report.diagnostics.extend(discovery_diags);

    let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
    let mut load_failed = false;
    for src in &sources {
        match load_state(&src.abs_path) {
            Ok(file) => loaded.push((src.clone(), file)),
            Err(StateLoadError::Parse { source, .. }) => {
                load_failed = true;
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!("state.json is not valid JSON or does not match the schema: {source}"),
                ));
            }
            Err(StateLoadError::Io { source, .. }) => {
                load_failed = true;
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!("could not read state.json: {source}"),
                ));
            }
        }
    }

    if write && load_failed {
        report.diagnostics.push(Diagnostic::error(
            root,
            "E_EMIT_INCOMPLETE_CORPUS",
            "refusing to write: at least one state.json failed to load; the target block may be \
             unresolvable and the chained emit-state would regenerate cross-repo views from a \
             partial corpus"
                .to_string(),
        ));
        return Ok(report);
    }

    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();
    let plan = plan_demote_block(key, &loaded, &today);

    let had_actions = !plan.actions.is_empty();
    report.diagnostics.extend(apply_plan(&plan, write));

    if write && had_actions && !report.is_failure() {
        let emit = crate::emit_state(root, true, scope)?;
        report.diagnostics.extend(emit.diagnostics);
    }

    Ok(report)
}

/// Full driver for `mev promote-block <repo:id>` — the restore path this
/// task also owns (no promote verb existed before it). Same shape as
/// [`demote_block`].
pub fn promote_block(
    root: &Path,
    key: &str,
    write: bool,
    scope: Option<&crate::brain::config::ScopeDependencySet>,
) -> anyhow::Result<crate::Report> {
    use crate::brain::config::find_brain_config;
    use crate::brain::emit::apply_plan;
    use crate::brain::state::{StateLoadError, discover_state_files, load_state};

    let mut report = crate::Report::default();

    let config = match find_brain_config(root) {
        Ok(cfg) => cfg,
        Err(e) => {
            report.diagnostics.push(Diagnostic::error(
                root,
                "E_CONFIG_NOT_FOUND",
                format!("brain.toml not found or unreadable: {e}"),
            ));
            return Ok(report);
        }
    };

    let (sources, discovery_diags) = discover_state_files(root, &config);
    report.diagnostics.extend(discovery_diags);

    let mut loaded: Vec<(StateSource, StateFile)> = Vec::new();
    let mut load_failed = false;
    for src in &sources {
        match load_state(&src.abs_path) {
            Ok(file) => loaded.push((src.clone(), file)),
            Err(StateLoadError::Parse { source, .. }) => {
                load_failed = true;
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!("state.json is not valid JSON or does not match the schema: {source}"),
                ));
            }
            Err(StateLoadError::Io { source, .. }) => {
                load_failed = true;
                report.diagnostics.push(Diagnostic::error(
                    &src.abs_path,
                    "E_STATE_MALFORMED_JSON",
                    format!("could not read state.json: {source}"),
                ));
            }
        }
    }

    if write && load_failed {
        report.diagnostics.push(Diagnostic::error(
            root,
            "E_EMIT_INCOMPLETE_CORPUS",
            "refusing to write: at least one state.json failed to load; the target backlog slug \
             may be unresolvable and the chained emit-state would regenerate cross-repo views \
             from a partial corpus"
                .to_string(),
        ));
        return Ok(report);
    }

    let plan = plan_promote_block(key, &loaded);

    let had_actions = !plan.actions.is_empty();
    report.diagnostics.extend(apply_plan(&plan, write));

    if write && had_actions && !report.is_failure() {
        let emit = crate::emit_state(root, true, scope)?;
        report.diagnostics.extend(emit.diagnostics);
    }

    Ok(report)
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

    // -----------------------------------------------------------------
    // demote-block / promote-block
    // -----------------------------------------------------------------

    /// Write a minimal legal `planning/blocks/<id>.json` record next to the
    /// `dir/repo/planning/state.json` [`file_for`] writes — `plan_demote_block`
    /// checks this exists (and reads its `kind`) before planning anything.
    fn write_record(dir: &Path, repo: &str, id: &str, kind: &str) {
        let path = dir
            .join(repo)
            .join("planning")
            .join("blocks")
            .join(format!("{id}.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&serde_json::json!({
                "id": id,
                "repo": repo,
                "kind": kind,
                "title": "A parked block",
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn apply_action_and_reload(src: &StateSource, action: &EmitAction) -> (StateSource, StateFile) {
        std::fs::write(&action.path, &action.new_content).unwrap();
        let file: StateFile = serde_json::from_str(&action.new_content).unwrap();
        (src.clone(), file)
    }

    #[test]
    fn demote_block_removes_tracks_row_and_appends_parked_backlog_entry() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("Phase 14", &[("MV.14.A", 140)])],
        )];
        write_record(dir.path(), "mev", "MV.14.A", "block");

        let plan = plan_demote_block("mev:MV.14.A", &files, "2026-09-03");
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
        assert_eq!(plan.actions.len(), 1, "{plan:?}");

        let state_file: StateFile = serde_json::from_str(&plan.actions[0].new_content).unwrap();
        assert!(
            state_file
                .tracks
                .iter()
                .flat_map(|t| t.blocks.iter())
                .all(|b| b.id != "MV.14.A"),
            "tracks[] row must be gone: {state_file:?}"
        );
        let backlog_entry = state_file
            .backlog
            .iter()
            .find(|b| b.slug == "MV.14.A")
            .expect("a parked backlog entry");
        assert_eq!(backlog_entry.status, BACKLOG_STATUS_PARKED);
        assert_eq!(backlog_entry.repo, "mev");
        assert_eq!(
            backlog_entry
                .extra
                .get(BACKLOG_EXTRA_RECORD)
                .and_then(|v| v.as_str()),
            Some("planning/blocks/MV.14.A.json")
        );
    }

    /// AC2: the record file on disk is byte-identical before and after a
    /// (planned) demote — `plan_demote_block` never emits an action that
    /// targets it, and never reads it beyond an existence + `kind` check.
    #[test]
    fn demote_block_never_touches_the_record_file() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("Phase 14", &[("MV.14.A", 140)])],
        )];
        write_record(dir.path(), "mev", "MV.14.A", "block");
        let record_path = dir.path().join("mev/planning/blocks/MV.14.A.json");
        let before = std::fs::read(&record_path).unwrap();

        let plan = plan_demote_block("mev:MV.14.A", &files, "2026-09-03");
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);
        assert!(
            plan.actions.iter().all(|a| a.path != record_path),
            "no action may target the record file: {plan:?}"
        );

        let after = std::fs::read(&record_path).unwrap();
        assert_eq!(before, after, "record file must stay byte-identical");
    }

    #[test]
    fn demote_block_without_a_record_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(
            dir.path(),
            "mev",
            &[("Phase 14", &[("MV.14.A", 140)])],
        )];
        // No write_record() call — the record is missing.
        let plan = plan_demote_block("mev:MV.14.A", &files, "2026-09-03");
        assert!(plan.actions.is_empty(), "{plan:?}");
        assert_eq!(codes(&plan), vec![E_DEMOTE_BLOCK_RECORD_MISSING]);
    }

    #[test]
    fn demote_block_bad_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[])];
        let plan = plan_demote_block("no-colon-here", &files, "2026-09-03");
        assert!(plan.actions.is_empty(), "{plan:?}");
        assert_eq!(codes(&plan), vec!["E_BLOCK_BAD_KEY"]);
    }

    #[test]
    fn demote_block_unknown_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[])];
        let plan = plan_demote_block("mev:MV.NOPE.X", &files, "2026-09-03");
        assert!(plan.actions.is_empty(), "{plan:?}");
        assert_eq!(codes(&plan), vec!["E_BLOCK_NOT_FOUND"]);
    }

    /// AC4: demote-then-promote round-trips with no field lost — the
    /// restored `tracks[].blocks[]` row matches the original exactly.
    #[test]
    fn demote_then_promote_round_trips_the_original_tracks_row() {
        let dir = tempfile::tempdir().unwrap();
        let (src, original_file) =
            file_for(dir.path(), "mev", &[("Phase 14", &[("MV.14.A", 140)])]);
        write_record(dir.path(), "mev", "MV.14.A", "block");
        let original_block = original_file.tracks[0].blocks[0].clone();
        let files = vec![(src.clone(), original_file)];

        // Demote.
        let demote_plan = plan_demote_block("mev:MV.14.A", &files, "2026-09-03");
        assert!(
            demote_plan.diagnostics.is_empty(),
            "{:?}",
            demote_plan.diagnostics
        );
        assert_eq!(demote_plan.actions.len(), 1);
        let after_demote = apply_action_and_reload(&src, &demote_plan.actions[0]);
        assert!(
            after_demote
                .1
                .tracks
                .iter()
                .flat_map(|t| t.blocks.iter())
                .all(|b| b.id != "MV.14.A")
        );

        // Promote back.
        let promote_files = vec![after_demote];
        let promote_plan = plan_promote_block("mev:MV.14.A", &promote_files);
        assert!(
            promote_plan.diagnostics.is_empty(),
            "{:?}",
            promote_plan.diagnostics
        );
        assert_eq!(promote_plan.actions.len(), 1);
        let restored_file: StateFile =
            serde_json::from_str(&promote_plan.actions[0].new_content).unwrap();
        let restored_block = restored_file
            .tracks
            .iter()
            .flat_map(|t| t.blocks.iter())
            .find(|b| b.id == "MV.14.A")
            .expect("restored tracks[] row");
        assert_eq!(
            serde_json::to_value(restored_block).unwrap(),
            serde_json::to_value(&original_block).unwrap(),
            "restored tracks[] row must match the original exactly"
        );

        // The backlog entry is marked promoted, not deleted, and its
        // temporary demote-only extras are gone.
        let backlog_entry = restored_file
            .backlog
            .iter()
            .find(|b| b.slug == "MV.14.A")
            .expect("backlog entry retained after promotion");
        assert_eq!(backlog_entry.status, "promoted");
        assert_eq!(backlog_entry.block.as_deref(), Some("MV.14.A"));
        assert!(backlog_entry.extra.get(BACKLOG_EXTRA_RECORD).is_none());
        assert!(
            backlog_entry
                .extra
                .get(BACKLOG_EXTRA_PARKED_BLOCK)
                .is_none()
        );
    }

    #[test]
    fn promote_block_not_parked_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[])];
        // No backlog entry at all yet.
        let plan = plan_promote_block("mev:MV.14.A", &files);
        assert!(plan.actions.is_empty(), "{plan:?}");
        assert_eq!(codes(&plan), vec!["E_BLOCK_NOT_FOUND"]);
    }

    #[test]
    fn promote_block_planning_never_mutates_the_caller_slice() {
        let dir = tempfile::tempdir().unwrap();
        let (src, original_file) =
            file_for(dir.path(), "mev", &[("Phase 14", &[("MV.14.A", 140)])]);
        write_record(dir.path(), "mev", "MV.14.A", "block");
        let files = vec![(src.clone(), original_file)];
        let demote_plan = plan_demote_block("mev:MV.14.A", &files, "2026-09-03");
        let after_demote = apply_action_and_reload(&src, &demote_plan.actions[0]);
        let promote_files = vec![after_demote];
        let backlog_len_before = promote_files[0].1.backlog.len();
        let tracks_before = promote_files[0].1.tracks.clone();
        let _ = plan_promote_block("mev:MV.14.A", &promote_files);
        assert_eq!(promote_files[0].1.backlog.len(), backlog_len_before);
        assert_eq!(
            serde_json::to_value(&promote_files[0].1.tracks).unwrap(),
            serde_json::to_value(&tracks_before).unwrap(),
            "planning must not mutate the caller's corpus"
        );
    }

    // -----------------------------------------------------------------
    // origin — MV.ticket.create-block-drops-the-origin-field, task 1
    //
    // `CreateBlockPayload` (as of this task) declares no `origin` field, so
    // an `"origin"` key present in `--from <file>` JSON is silently ignored
    // by serde (no `deny_unknown_fields`) — exactly the drop this ticket
    // exists to fix. These three fixtures build the payload from a raw JSON
    // string (as `payload_deserializes_from_minimal_json` does above) so an
    // `origin` key can be present in the input even though the struct has
    // nowhere to put it yet.
    // -----------------------------------------------------------------

    /// A minimal, legal `kind: "chore"` payload JSON string targeting repo
    /// `"mev"`, as an owned `serde_json::Value` object map so a test can
    /// splice in an `"origin"` key before deserializing.
    fn legal_payload_json() -> serde_json::Map<String, serde_json::Value> {
        let raw = r#"{
            "id": "MV.99.ORIGIN",
            "repo": "mev",
            "kind": "chore",
            "title": "Origin fixture",
            "description": "A payload used only to exercise origin handling.",
            "what": "Nothing much.",
            "why": "To prove origin round-trips (or doesn't, yet).",
            "sdlc_workflow": "none",
            "model": "either",
            "out_of_scope": ["Everything else."],
            "acceptance_criteria": ["It parses."],
            "epics": ["test-epic"]
        }"#;
        match serde_json::from_str(raw).expect("fixture JSON is valid") {
            serde_json::Value::Object(map) => map,
            other => panic!("expected a JSON object, got {other:?}"),
        }
    }

    /// Deserialize a `CreateBlockPayload` from [`legal_payload_json`] with
    /// `origin` set to `value` (pass `None` to omit the key entirely).
    fn payload_with_origin(value: Option<serde_json::Value>) -> CreateBlockPayload {
        let mut map = legal_payload_json();
        match value {
            Some(v) => {
                map.insert("origin".to_string(), v);
            }
            None => {
                map.remove("origin");
            }
        }
        serde_json::from_value(serde_json::Value::Object(map))
            .expect("payload should deserialize regardless of an unknown 'origin' key")
    }

    /// Plan-and-extract helper: run `plan_create_block` against a fresh
    /// single-repo corpus and return `(state.json row for payload.id, block
    /// record)` as parsed JSON, per the action ordering `plan_create_block`
    /// documents (state.json first, block record second).
    fn plan_and_extract(
        payload: &CreateBlockPayload,
    ) -> (
        EmitPlan,
        Option<serde_json::Value>,
        Option<serde_json::Value>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![file_for(dir.path(), "mev", &[])];
        let plan = plan_create_block(payload, &files, "2026-09-03");
        if plan.actions.len() < 2 {
            return (plan, None, None);
        }
        let state_json: serde_json::Value =
            serde_json::from_str(&plan.actions[0].new_content).expect("state.json parses");
        let row = state_json["tracks"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|t| t["blocks"].as_array().into_iter().flatten())
            .find(|b| b["id"] == json!(payload.id))
            .cloned();
        let record: serde_json::Value =
            serde_json::from_str(&plan.actions[1].new_content).expect("block record parses");
        (plan, row, Some(record))
    }

    /// Test 1 (D68 — observed RED before task 2): a payload carrying a
    /// valid `{"type": "mechanism", "slug": ...}` origin must produce a
    /// block record AND a `tracks[].blocks[]` row whose `origin` equals it.
    /// Today `CreateBlockPayload` has no `origin` field, so the key is
    /// silently dropped at deserialization and both artifacts come back
    /// without it — this test is expected to fail on both assertions.
    #[test]
    fn valid_mechanism_origin_round_trips_into_record_and_row() {
        let expected = json!({"type": "mechanism", "slug": "gates-that-cannot-fail"});
        let payload = payload_with_origin(Some(expected.clone()));
        let (plan, row, record) = plan_and_extract(&payload);
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);

        let record = record.expect("block record action present");
        assert_eq!(
            record.get("origin"),
            Some(&expected),
            "block record must carry the origin object, got {record:?}"
        );

        let row = row.expect("tracks[].blocks[] row present");
        assert_eq!(
            row.get("origin"),
            Some(&expected),
            "state.json row must carry the origin object, got {row:?}"
        );
    }

    /// Test 2 (D68 — observed RED before task 2): a malformed origin (here,
    /// an out-of-vocabulary `type`) must be REFUSED with a named diagnostic
    /// and a zero-action plan — never silently dropped to null. Today the
    /// payload has no `origin` field at all, so nothing ever inspects it:
    /// the plan proceeds with its normal two actions and zero diagnostics,
    /// which is the exact silent-drop shape this ticket exists to close.
    #[test]
    fn malformed_origin_is_refused_not_silently_dropped() {
        let malformed = json!({"type": "not-a-real-kind", "slug": "whatever"});
        let payload = payload_with_origin(Some(malformed));
        let (plan, _row, _record) = plan_and_extract(&payload);
        assert!(
            !plan.diagnostics.is_empty(),
            "expected a named diagnostic refusing the malformed origin, got none (plan: {plan:?})"
        );
        assert!(
            plan.actions.is_empty(),
            "a malformed origin must yield a zero-action plan, not a partial write (plan: {plan:?})"
        );
    }

    /// Test 3 (D68 — the positive control, NOT symmetric): a payload with
    /// no `origin` at all must write a block record that OMITS the key
    /// entirely, and a `tracks[].blocks[]` row that carries `origin: null`.
    /// Measured 2026-09-03 this is partially green today by accident: the
    /// record already omits the key (`build_block_record` never inserts
    /// `origin`), but okf-core's `TrackBlock::origin` has no
    /// `skip_serializing_if`, so the row assertion is *also* green today —
    /// both halves currently pass with no origin-aware code at all. Kept as
    /// two separate assertions per the task: the moment task 2 wires
    /// `origin` through, only an implementation that keeps the no-origin
    /// case key-absent-on-the-record can keep this green.
    #[test]
    fn no_origin_is_key_absent_on_record_and_null_on_row() {
        let payload = payload_with_origin(None);
        let (plan, row, record) = plan_and_extract(&payload);
        assert!(plan.diagnostics.is_empty(), "{:?}", plan.diagnostics);

        let record = record.expect("block record action present");
        assert!(
            record.get("origin").is_none(),
            "no-origin payload must OMIT the origin key from the block record, got {record:?}"
        );

        let row = row.expect("tracks[].blocks[] row present");
        assert_eq!(
            row.get("origin"),
            Some(&serde_json::Value::Null),
            "no-origin payload must write origin: null on the state.json row, got {row:?}"
        );
    }
}
