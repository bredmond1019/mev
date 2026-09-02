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

use crate::Diagnostic;
use crate::brain::state::BlockedBy;

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
