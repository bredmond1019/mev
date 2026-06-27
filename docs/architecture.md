---
type: Reference
title: mev Architecture
description: Module map, ContentValidator trait, and core types — how mev's pluggable validation pipeline is structured
doc_id: architecture
layer: [factory]
project: mev
status: active
keywords: [architecture, ContentValidator, Diagnostic, Report, modules, trait, mev]
related: [cli-reference, brain-toml-config, okf-schema]
---

# mev Architecture

## Module map

```
src/
├── lib.rs          ← crate root: core types (Diagnostic, Report, JsonReport) + public re-exports
├── main.rs         ← clap CLI — thin wrapper: parse → dispatch → exit code
├── shared.rs       ← internal helpers: extract_frontmatter, is_kebab_case, non_empty
├── validator.rs    ← ContentValidator trait (the extension point)
├── learn_ai/
│   ├── mod.rs      ← LearnAiValidator (implements ContentValidator)
│   ├── crawl.rs    ← crawl() → (Vec<ContentFile>, Vec<Diagnostic>); ContentFile, Corpus, FileKind, Locale
│   └── meta.rs     ← validate_file() — per-file frontmatter + JSON struct checks
└── brain/
    ├── mod.rs      ← BrainValidator (implements ContentValidator)
    ├── config.rs   ← BrainConfig, CrawlConfig, VocabConfig, RepoEntry; find_brain_config(), load_brain_config()
    ├── crawl.rs    ← crawl_brain() → (Vec<MdFile>, Vec<Diagnostic>); MdFile
    └── okf.rs      ← OkfFrontmatter, validate_md_file() — OKF field checks

tests/
├── brain_config.rs   ← integration tests for brain.toml loading + BrainConfig
├── brain_crawl.rs    ← integration tests for crawl_brain()
├── brain_okf.rs      ← integration tests for validate_md_file()
├── brain_validate.rs ← integration tests for BrainValidator end-to-end
├── smoke.rs          ← integration tests for the learn-ai validate() public API
└── fixtures/
    └── brain.toml    ← minimal fixture — NOT the live brain.toml
```

---

## The `ContentValidator` trait

`ContentValidator` (in `src/validator.rs`) is the single extension point. Every consumer implements it; the default `run` driver stitches crawl + validate together.

```rust
pub trait ContentValidator {
    type Item;

    fn crawl(&self, root: &Path) -> (Vec<Self::Item>, Vec<Diagnostic>);
    fn validate_item(&self, item: &Self::Item) -> Vec<Diagnostic>;

    // Default driver — override only for non-standard collect strategies.
    fn run(&self, root: &Path) -> Report { ... }
}
```

**To add a new consumer:**
1. Define an `Item` type (the unit your crawl produces — a path-like struct, a parsed record, etc.)
2. Implement `crawl` — walk `root`, return items + any crawl-time diagnostics
3. Implement `validate_item` — check a single item, return diagnostics
4. Wire into `main.rs` as a new `Subcommand` variant

The two current consumers:

| Struct | Item type | Source module |
|---|---|---|
| `LearnAiValidator` | `ContentFile` | `src/learn_ai/` |
| `BrainValidator` | `MdFile` | `src/brain/` |

---

## Core types

### `Diagnostic`

A single validation finding. Every check emits `Diagnostic`s; the reporter prints them.

```rust
pub struct Diagnostic {
    pub severity: Severity,   // Error | Warning
    pub file: PathBuf,        // file the finding concerns
    pub locator: String,      // in-file locator, e.g. "type", "layer[0]", "" for whole-file
    pub message: String,
}
```

**Severity drives the exit code:** any `Error` → exit 1. `Warning` → reported, exit 0.

Constructors: `Diagnostic::error(file, locator, message)` and `Diagnostic::warning(...)`.

### `Report`

The outcome of a `run()` call — a flat list of diagnostics with summary counts.

```rust
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
}
impl Report {
    pub fn error_count(&self) -> usize { ... }
    pub fn warning_count(&self) -> usize { ... }
    pub fn is_failure(&self) -> bool { self.error_count() > 0 }
}
```

### `JsonReport`

The machine-readable envelope emitted by `--json`. Consumed by the Brain RAG indexer as a pre-rebuild gate.

```rust
pub struct JsonReport {
    pub validator: String,       // "brain" | "learn-ai"
    pub root: String,
    pub errors: usize,
    pub warnings: usize,
    pub diagnostics: Vec<Diagnostic>,
}
```

See the [CLI reference](cli.md) for the serialized JSON shape.

---

## Data flow

```
mev validate-brain <root>
        │
        ▼
find_brain_config(root)          ← walks up from root, parses brain.toml
        │
        ▼
BrainValidator::new(config)
        │
        ▼
.run(root)
  ├── crawl_brain(root, skip_dirs)   ← walks FS, returns Vec<MdFile>
  │        prune: skip_dirs names, nested git repos, file blocklist
  │
  └── for each MdFile:
        validate_md_file(item, config)
            ├── read file
            ├── extract YAML frontmatter
            ├── deserialize OkfFrontmatter
            └── check each field → Vec<Diagnostic>
        │
        ▼
Report { diagnostics }
        │
        ▼
exit 0 (clean) | exit 1 (any Error)
```
