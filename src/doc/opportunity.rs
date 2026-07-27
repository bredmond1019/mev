//! The Opportunity command family — `ingest` / `set-stage` / `add-action` /
//! `merge-contacts` — as idempotent library planners over
//! `okf_core::Opportunity` (`MV.9.A` task 3).
//!
//! Every function here mutates an in-memory `Opportunity` model and hands it
//! to `crate::doc::materialize::plan_document`, which performs the read
//! needed to compute an update and the idempotency check. None of these
//! functions perform IO of their own beyond that one read — like the rest of
//! the materializer, they are planners; `crate::brain::emit::apply_plan`
//! stays the single write point.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use okf_core::{Action, Contact, Opportunity, parse_nested_frontmatter};
use serde_json::Value as JsonValue;

use crate::Diagnostic;
use crate::brain::emit::EmitPlan;
use crate::doc::materialize::plan_document;

/// The seven documented `stage` values from the
/// `business/docs/opportunities/index.md` contract.
pub const VALID_STAGES: [&str; 7] = [
    "identified",
    "researching",
    "contacted",
    "conversation",
    "proposal-sent",
    "closed-won",
    "closed-lost",
];

/// The three documented `kind` values an Opportunity can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpportunityKind {
    Company,
    ProspectingSweep,
    JobPosting,
}

impl OpportunityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Company => "company",
            Self::ProspectingSweep => "prospecting-sweep",
            Self::JobPosting => "job-posting",
        }
    }
}

impl FromStr for OpportunityKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "company" => Ok(Self::Company),
            "prospecting-sweep" => Ok(Self::ProspectingSweep),
            "job-posting" => Ok(Self::JobPosting),
            other => Err(format!(
                "unknown opportunity kind '{other}' (expected company|prospecting-sweep|job-posting)"
            )),
        }
    }
}

/// The path an Opportunity with the given `slug` lives (or would live) at
/// under `root`, per `Opportunity::index_intent`'s fixed
/// `business/docs/opportunities/` location.
fn opportunity_path(slug: &str, root: &Path) -> PathBuf {
    root.join("business/docs/opportunities")
        .join(format!("{slug}.md"))
}

/// Load and reconstruct the `Opportunity` at `slug` under `root`. Returns an
/// error `EmitPlan` (no actions, one `E_DOC_NOT_FOUND` diagnostic) when the
/// file is absent or cannot be parsed/reconstructed — the shared failure
/// path for all three mutators.
fn load_opportunity(slug: &str, root: &Path) -> Result<Opportunity, EmitPlan> {
    let path = opportunity_path(slug, root);
    let not_found = |message: String| EmitPlan {
        actions: vec![],
        diagnostics: vec![Diagnostic::error(path.clone(), "E_DOC_NOT_FOUND", message)],
    };

    let content = std::fs::read_to_string(&path).map_err(|e| {
        not_found(format!(
            "no opportunity file for slug '{slug}' at {}: {e}",
            path.display()
        ))
    })?;
    let fields = parse_nested_frontmatter(&content).map_err(|e| {
        not_found(format!(
            "failed to parse frontmatter for '{slug}' at {}: {e}",
            path.display()
        ))
    })?;
    Opportunity::from_frontmatter(&fields).map_err(|e| {
        not_found(format!(
            "failed to reconstruct opportunity '{slug}' at {}: {e}",
            path.display()
        ))
    })
}

/// Detect the `OpportunityKind` from an input JSON shape, per the documented
/// rule: `company_name` present → company; `prospects`/`vertical` present →
/// prospecting-sweep. Returns `None` when neither shape matches.
fn detect_kind(input: &JsonValue) -> Option<OpportunityKind> {
    let has_company_name = input
        .get("company_name")
        .and_then(JsonValue::as_str)
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if has_company_name {
        return Some(OpportunityKind::Company);
    }

    let has_prospects = input.get("prospects").is_some();
    let has_vertical = input
        .get("vertical")
        .and_then(JsonValue::as_str)
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if has_prospects || has_vertical {
        return Some(OpportunityKind::ProspectingSweep);
    }

    None
}

/// Plan ingesting a raw `CompanyBrief`/`ProspectingResult` JSON payload as a
/// new (or updated) `Opportunity` document.
///
/// `kind` dispatches directly when given; when `None`, the kind is
/// auto-detected from `input`'s shape (see [`detect_kind`]) — an
/// unrecognized shape raises `E_DOC_UNKNOWN_INPUT_SHAPE` and plans nothing.
pub fn plan_ingest(input: &JsonValue, kind: Option<OpportunityKind>, root: &Path) -> EmitPlan {
    let resolved = kind.or_else(|| detect_kind(input));

    let opp = match resolved {
        Some(OpportunityKind::Company) => Opportunity::from_company_brief(input),
        Some(OpportunityKind::ProspectingSweep) => Opportunity::from_prospecting_result(input),
        Some(OpportunityKind::JobPosting) => {
            // Same brief shape as `company`, with the kind overridden.
            let mut opp = Opportunity::from_company_brief(input);
            opp.kind = Some(OpportunityKind::JobPosting.as_str().to_string());
            opp
        }
        None => {
            return EmitPlan {
                actions: vec![],
                diagnostics: vec![Diagnostic::error(
                    root,
                    "E_DOC_UNKNOWN_INPUT_SHAPE",
                    "input JSON has neither `company_name` nor `prospects`/`vertical` — cannot infer opportunity kind (pass --kind explicitly)".to_string(),
                )],
            };
        }
    };

    plan_document(&opp, root)
}

/// Plan setting an existing opportunity's `stage`. Validates `stage` against
/// [`VALID_STAGES`] — an unknown value raises `E_DOC_BAD_STAGE` and plans
/// nothing. Re-running with the same stage is a zero-action no-op (via
/// `plan_document`'s idempotency guard).
pub fn plan_set_stage(slug: &str, stage: &str, root: &Path) -> EmitPlan {
    if !VALID_STAGES.contains(&stage) {
        return EmitPlan {
            actions: vec![],
            diagnostics: vec![Diagnostic::error(
                opportunity_path(slug, root),
                "E_DOC_BAD_STAGE",
                format!(
                    "'{stage}' is not a valid stage (expected one of: {})",
                    VALID_STAGES.join(" | ")
                ),
            )],
        };
    }

    let mut opp = match load_opportunity(slug, root) {
        Ok(opp) => opp,
        Err(plan) => return plan,
    };
    opp.stage = Some(stage.to_string());
    plan_document(&opp, root)
}

/// Plan appending one `{at, kind, note}` action to an existing opportunity's
/// `actions[]`. An identical triple already present is not re-appended, so a
/// repeat call plans zero actions.
pub fn plan_add_action(slug: &str, at: &str, kind: &str, note: &str, root: &Path) -> EmitPlan {
    let mut opp = match load_opportunity(slug, root) {
        Ok(opp) => opp,
        Err(plan) => return plan,
    };

    let candidate = Action {
        at: at.to_string(),
        kind: kind.to_string(),
        note: note.to_string(),
    };
    if !opp.actions.contains(&candidate) {
        opp.actions.push(candidate);
    }

    plan_document(&opp, root)
}

/// Build a `Contact` from a JSON object shaped `{name, role, emails, whatsapp,
/// phones, links, note}` — list fields accept either a JSON array of strings
/// or an absent key (treated as empty).
fn contact_from_json(v: &JsonValue) -> Contact {
    let str_field = |key: &str| {
        v.get(key)
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let list_field = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(JsonValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };

    Contact {
        name: str_field("name"),
        role: str_field("role"),
        emails: list_field("emails"),
        whatsapp: list_field("whatsapp"),
        phones: list_field("phones"),
        links: list_field("links"),
        note: str_field("note"),
    }
}

/// Parse `contacts` as either a single JSON contact object or a JSON array
/// of contact objects.
fn contacts_from_json(contacts: &JsonValue) -> Vec<Contact> {
    match contacts {
        JsonValue::Array(items) => items.iter().map(contact_from_json).collect(),
        other => vec![contact_from_json(other)],
    }
}

/// Union two string lists, deduped, preserving `a`'s order then appending any
/// of `b` not already present.
fn union_dedup(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = a.to_vec();
    for item in b {
        if !out.contains(item) {
            out.push(item.clone());
        }
    }
    out
}

/// Plan merging one or more contacts into an existing opportunity's
/// `contacts[]`, matched on `name`. Matching contacts have their
/// `emails`/`whatsapp`/`phones`/`links` vectors unioned (deduped,
/// order-stable); `role`/`note` are filled only when the existing value is
/// empty, so an enriched field already on file is never overwritten by a
/// blank incoming one. A `name` not already present is appended as a new
/// contact.
pub fn plan_merge_contacts(slug: &str, contacts: &JsonValue, root: &Path) -> EmitPlan {
    let mut opp = match load_opportunity(slug, root) {
        Ok(opp) => opp,
        Err(plan) => return plan,
    };

    for incoming in contacts_from_json(contacts) {
        match opp.contacts.iter_mut().find(|c| c.name == incoming.name) {
            Some(existing) => {
                existing.emails = union_dedup(&existing.emails, &incoming.emails);
                existing.whatsapp = union_dedup(&existing.whatsapp, &incoming.whatsapp);
                existing.phones = union_dedup(&existing.phones, &incoming.phones);
                existing.links = union_dedup(&existing.links, &incoming.links);
                if existing.role.is_empty() {
                    existing.role = incoming.role.clone();
                }
                if existing.note.is_empty() {
                    existing.note = incoming.note.clone();
                }
            }
            None => opp.contacts.push(incoming),
        }
    }

    plan_document(&opp, root)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opportunity_kind_round_trips_through_str() {
        for k in [
            OpportunityKind::Company,
            OpportunityKind::ProspectingSweep,
            OpportunityKind::JobPosting,
        ] {
            assert_eq!(OpportunityKind::from_str(k.as_str()).unwrap(), k);
        }
        assert!(OpportunityKind::from_str("bogus").is_err());
    }

    #[test]
    fn detect_kind_prefers_company_name() {
        let v = serde_json::json!({"company_name": "Acme", "vertical": "x"});
        assert_eq!(detect_kind(&v), Some(OpportunityKind::Company));
    }

    #[test]
    fn detect_kind_recognizes_prospecting_shape() {
        let v = serde_json::json!({"vertical": "legal-tech", "prospects": []});
        assert_eq!(detect_kind(&v), Some(OpportunityKind::ProspectingSweep));
    }

    #[test]
    fn detect_kind_none_for_unknown_shape() {
        let v = serde_json::json!({"foo": "bar"});
        assert_eq!(detect_kind(&v), None);
    }

    #[test]
    fn union_dedup_preserves_order_and_dedupes() {
        let a = vec!["a".to_string(), "b".to_string()];
        let b = vec!["b".to_string(), "c".to_string()];
        assert_eq!(union_dedup(&a, &b), vec!["a", "b", "c"]);
    }

    #[test]
    fn contact_from_json_reads_all_fields() {
        let v = serde_json::json!({
            "name": "Alice",
            "role": "CTO",
            "emails": ["alice@example.com"],
            "whatsapp": [],
            "phones": ["+1-555-0100"],
            "links": [],
            "note": "Met at conference",
        });
        let c = contact_from_json(&v);
        assert_eq!(c.name, "Alice");
        assert_eq!(c.role, "CTO");
        assert_eq!(c.emails, vec!["alice@example.com".to_string()]);
        assert_eq!(c.phones, vec!["+1-555-0100".to_string()]);
        assert_eq!(c.note, "Met at conference");
    }
}
