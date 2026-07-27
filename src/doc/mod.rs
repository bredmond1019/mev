//! `mev::doc` — the generic brain-document materializer (`MV.9.A`).
//!
//! This module builds on the existing `EmitPlan`/`apply_plan` seam
//! (`crate::brain::emit`) to plan writes for any `okf_core::BrainDocModel`
//! implementor. It is a **planner**: nothing here performs file IO beyond the
//! one read needed to compute an update against an existing target (see
//! `materialize::plan_document`'s doc comment) — `apply_plan` stays the
//! single write point.

pub mod materialize;

pub use materialize::plan_document;
