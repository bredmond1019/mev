---
type: Index
title: mev
description: A Rust CLI tool (`mev`) that validates Markdown/MDX content — learn-ai frontmatter + struct validation and Bastion Brain OKF frontmatter validation, with JSON output for RAG indexer integration
doc_id: mev-readme
layer: [factory, meta]
project: mev
status: active
keywords: [markdown validation, MDX, Rust CLI, mev, learn-ai, OKF frontmatter]
related: [context, master-plan, status]
---

# mev

A Rust CLI tool (`mev`) that validates Markdown/MDX content across two consumers: **learn-ai** (frontmatter + struct validation for learn-agentic-ai.com) and **Bastion Brain** (OKF frontmatter validation for the company-brain RAG index). Machine-readable `--json` output lets the RAG indexer use `mev` as a pre-rebuild gate.

## Prerequisites

- Rust 1.95+ (`rustup show`). No other runtime — dependencies fetch on first build.
- For `validate`: a checkout of the `learn-ai` site as a sibling directory; the validator points at `../learn-ai/content/learn` by default.
- For `validate-brain`: a checkout of `agentic-portfolio/` (the Brain repo); defaults to `..`.

## Setup

```bash
git clone <this repo> && cd mev
cargo build --release    # produces target/release/mev
```

## Running locally

```bash
# Validate the learn-ai content tree (path is optional; this is the default)
cargo run -- validate ../learn-ai/content/learn
# or the built binary:
./target/release/mev validate ../learn-ai/content/learn

# Validate the Bastion Brain OKF frontmatter (path defaults to ..)
cargo run -- validate-brain ~/Dev/agentic-portfolio
./target/release/mev validate-brain ~/Dev/agentic-portfolio

# Machine-readable JSON output (exit 1 on any error-severity diagnostic)
./target/release/mev --json validate-brain ~/Dev/agentic-portfolio
```

## Tests

```bash
cargo test
# Full gate suite (what the SDLC pipeline runs):
cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release
```

## Directory map

```
mev/
├── .claude/        ← Claude Code commands + SDLC workflow engines
├── planning/       ← context, status, master-plan, harness.json, decisions/, <concept>/
├── src/
│   ├── lib.rs          ← crate root: Diagnostic/Report core + public API re-exports
│   ├── main.rs         ← clap CLI entry point
│   ├── shared.rs       ← shared helpers: extract_frontmatter, is_kebab_case, non_empty
│   ├── validator.rs    ← ContentValidator trait (crawl + validate_item + run driver)
│   ├── learn_ai/       ← LearnAiValidator: crawl.rs, meta.rs, mod.rs
│   └── brain/          ← BrainValidator: crawl.rs (crawl_brain, MdFile), mod.rs, okf.rs (OkfFrontmatter, validate_md_file)
├── tests/          ← integration tests + fixtures
└── Cargo.toml
```

## Documentation

| Doc | Contents |
|---|---|
| [planning/context.md](planning/context.md) | Orientation + governing principles |
| [planning/master-plan.md](planning/master-plan.md) | Strategy + phase specifications |
| [planning/status.md](planning/status.md) | Current progress |
| [planning/harness.json](planning/harness.json) | SDLC validation/UI-test config (see `harness.examples.md`) |

---

*Initialized 2026-06-18 from `base-template` (commit `00ad2834e232d3243a3578132b02db01a7be40ab`).*
