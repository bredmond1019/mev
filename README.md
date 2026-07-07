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

> **Built within the Bastion workspace.** This crate depends on `okf-core` via path dependency (`../okf-core`) and is not designed to build standalone. See the [bastion-os](https://github.com/bredmond1019/bastion-os) meta-repo for the full ecosystem.
> Part of the **Bastion** ecosystem — see the [bastion-os](https://github.com/bredmond1019/bastion-os) front door for the full architecture.

A Rust CLI tool (`mev`) that validates Markdown/MDX content across two consumers: **learn-ai** (frontmatter + struct validation for learn-agentic-ai.com) and **Bastion Brain** (OKF frontmatter validation for the company-brain RAG index). Machine-readable `--json` output lets the RAG indexer use `mev` as a pre-rebuild gate.

## Prerequisites

- Rust 1.95+ (`rustup show`). No other runtime — dependencies fetch on first build.
- For `validate`: a checkout of the `learn-ai` site as a sibling directory; the validator points at `../learn-ai/content/learn` by default.
- For `validate-brain`: a checkout of `agentic-portfolio/` (the Brain repo); defaults to `..`.

## Setup

```bash
git clone https://github.com/bredmond1019/mev && cd mev
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
│   └── brain/          ← BrainValidator: config.rs (BrainConfig, find_brain_config), crawl.rs (crawl_brain, MdFile), mod.rs, okf.rs (OkfFrontmatter, validate_md_file)
├── tests/          ← integration tests + fixtures
└── Cargo.toml
```

## `--json` output shape

The `--json` flag emits a `JsonReport` envelope consumed by the Brain RAG indexer:

```json
{
  "validator": "brain",
  "root": "/path/to/repo",
  "errors": 0,
  "warnings": 1,
  "diagnostics": [
    {
      "severity": "warning",
      "file": "docs/foo.md",
      "locator": "keywords",
      "message": "keywords count 2 is below the recommended minimum of 3"
    }
  ]
}
```

See [`docs/cli.md`](docs/cli.md) for the full field reference.

## Documentation

| Doc | Contents |
|---|---|
| [docs/cli.md](docs/cli.md) | Full CLI reference: subcommands, flags, exit codes, JSON shape |
| [docs/architecture.md](docs/architecture.md) | Module map, `ContentValidator` trait, core types |
| [docs/brain-toml.md](docs/brain-toml.md) | `brain.toml` config schema — `[vocab]`, `[crawl]`, `[[repos]]` |
| [docs/okf-schema.md](docs/okf-schema.md) | OKF frontmatter fields, validation rules, diagnostic table |
| [planning/context.md](planning/context.md) | Orientation + governing principles |
| [planning/master-plan.md](planning/master-plan.md) | Strategy + phase specifications |
| [planning/status.md](planning/status.md) | Current progress |
| [planning/harness.json](planning/harness.json) | SDLC validation/UI-test config (see `harness.examples.md`) |

## Roadmap / Known limitations

- **No known limitations.** A concurrent validation pipeline is an aspirational optimization only; the current static validator/compiler is correct and complete for the corpus it serves.

---

*Initialized 2026-06-18 from `base-template` (commit `00ad2834e232d3243a3578132b02db01a7be40ab`).*
