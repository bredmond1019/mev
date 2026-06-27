# Worklog — 2.M-brain-toml-reader

## Task 1 — PASSED (1 attempt)
What: Add BrainConfig struct, load_brain_config, and find_brain_config walk-up resolver with full test coverage (toml crate added, fixture created, 10 integration tests)
Decisions: Made brain module pub in lib.rs (was mod brain) so integration tests can access mev::brain::config directly; Used Default derive on VocabConfig/CrawlConfig so partial brain.toml files parse without error; Kept find_brain_config returning ConfigError::NotFound rather than falling back to hardcodes, per spec; NotFound test uses /tmp path with graceful skip if a developer machine has brain.toml above /tmp
Validated: gating checks (fast tripwire)

## Task 2 — PASSED (1 attempt)
What: crawl_brain now takes skip_dirs from BrainConfig instead of hardcoded matches; BrainValidator gains a config field and new() constructor; 174 tests pass, all four harness gates green
Decisions: Used BrainConfig::default() in validate_brain (lib.rs) as a temporary placeholder — Task 4 replaces this with find_brain_config(root); Updated tests/brain_okf.rs BrainValidator call sites (not explicitly listed in Task 2 spec) because changing BrainValidator from unit struct to struct-with-field broke compilation there; is_blocklisted_name kept as a named helper (not inlined) so it remains independently unit-testable
Validated: gating checks (fast tripwire)

## Task 3 — PASSED (1 attempt)
What: Config-driven vocab validation: is_valid_layer/status/project now accept &BrainConfig and look up values from brain.toml; validate_md_file propagates config; all hardcoded vocabulary arrays removed from production code.
Decisions: Changed good_okf_body() in tests/brain_okf.rs to use project: brain (in fixture) instead of project: bastion (not in fixture repos) to avoid needing to expand the fixture.; Added full_test_config() helper inside okf.rs inline tests constructing the full standard vocabulary inline — avoids file I/O in unit tests while covering all 13 standard project slugs.; Error messages now dynamically list config values via config.vocab.layer.join('|') and config.projects().join('|') instead of hardcoded strings.; BrainValidator tests that use good_okf_body() now use fixture_config() instead of BrainConfig::default() so vocab fields validate correctly.
Validated: gating checks (fast tripwire)
