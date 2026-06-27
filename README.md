---
type: Index
title: markdown-engine-validator
description: A Rust CLI tool that parses, validates, and compiles MDX/Markdown lessons for learn-agentic-ai.com — frontmatter validation, link checking, code block linting, and watch-mode hot-reload
doc_id: mev-readme
layer: [factory, meta]
project: markdown-engine-validator
status: active
keywords: [markdown validation, MDX, Rust CLI, mev, learn-ai, OKF frontmatter]
related: [context, master-plan, status]
---

# markdown-engine-validator

A Rust CLI tool that parses, validates, and compiles MDX/Markdown lessons for learn-agentic-ai.com — frontmatter validation, link checking, code block linting, and watch-mode hot-reload

## Prerequisites

- Rust 1.95+ (`rustup show`). No other runtime — dependencies fetch on first build.
- A checkout of the `learn-ai` site as a sibling directory (the content lives there); the
  validator points at `../learn-ai/content/learn` by default.

## Setup

```bash
git clone <this repo> && cd markdown-engine-validator
cargo build --release    # produces target/release/mev
```

## Running locally

```bash
# Validate the learn-ai content tree (path is optional; this is the default)
cargo run -- validate ../learn-ai/content/learn
# or the built binary:
./target/release/mev validate ../learn-ai/content/learn
```

## Tests

```bash
cargo test
# Full gate suite (what the SDLC pipeline runs):
cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo build --release
```

## Directory map

```
markdown-engine-validator/
├── .claude/        ← Claude Code commands + SDLC workflow engines
├── planning/       ← context, status, master-plan, harness.json, decisions/, <concept>/
├── src/
│   ├── lib.rs          ← crate root: Diagnostic/Report core + public API re-exports
│   ├── main.rs         ← clap CLI entry point
│   ├── shared.rs       ← shared helpers: extract_frontmatter, is_kebab_case, non_empty
│   ├── validator.rs    ← ContentValidator trait (crawl + validate_item + run driver)
│   ├── learn_ai/       ← LearnAiValidator: crawl.rs, meta.rs, mod.rs
│   └── brain/          ← BrainValidator: crawl.rs (crawl_brain, MdFile), mod.rs
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
