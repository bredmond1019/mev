# Documentation Report — 4.D-sync-comparator-hardening

**Date:** 2026-07-04
**Spec:** planning/4.D-sync-comparator-hardening/tasks.md
**Verdict gate:** PASS (confirmed)

## Docs Patched
| Doc File | Section Updated | Change Summary |
|---|---|---|
| docs/cli.md | `--sync` — cross-repo watermark check | Clarified that `timestamp`/`synced_from` are compared as explicit UTC instants (each side normalized via `.to_utc()`), not as raw strings — a `-03:00` and a `Z` watermark for the same moment are in sync. Updated the `E_SYNC_DRIFT` locator row from "their values differ" to "denote different instants" to match. |

## Docs Flagged NEEDS_REVIEW
None. This change is an internal comparator hardening (no new public API, route, or entry-point wiring); the only doc referencing the comparison semantics (`docs/cli.md`) was patched directly.

## Docs Clean (checked, no changes needed)
- docs/okf-schema.md — `synced_from` field section already defers to `docs/cli.md` for comparison-semantics detail ("See the CLI reference for the full locator table"); no changes needed.
- docs/brain-toml.md — only references that `mev validate-brain --sync` consumes the sync fields; no comparison-semantics detail to update.
- docs/architecture.md — `sync.rs` is listed only as "(internal) sync helpers" in the module map; no signature or behavior detail present that needed updating.
