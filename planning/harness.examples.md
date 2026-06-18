---
type: Reference
title: harness.json — copy-paste profiles
description: Ready-made planning/harness.json profiles (Rust, Python/FastAPI, Next.js) for adapting the SDLC pipeline to this project's stack.
---

# `harness.json` profiles

`planning/harness.json` is the **policy** the SDLC engines read: the validation commands and
whether a UI-test stage exists. The engine code (`.claude/workflows/*.js`) carries the
**mechanism** and ships no stack defaults — so this file is where a project names its real
commands.

Pick the profile that matches this project's stack, paste it into `planning/harness.json`, and
edit the commands to match. Validation `checks[]` run **top-to-bottom**; a check with
`gates: true` blocks the review verdict on failure. Set `uiTest.enabled: true` only for web
projects that have a dev server to smoke-test.

> If `planning/harness.json` is absent, the engines fall back to the spec's
> `## Validation Commands` section and skip the UI-test stage entirely. The file is the
> recommended path, not a hard requirement.

---

## Rust (CLI / TUI / library) — no web server

```json
{
  "$schema": "../.claude/workflows/harness.schema.json",
  "stack": "rust",
  "validation": {
    "checks": [
      { "name": "fmt",    "command": "cargo fmt --check",            "purpose": "Format gate", "gates": true },
      { "name": "clippy", "command": "cargo clippy -- -D warnings",  "purpose": "Lint gate",   "gates": true },
      { "name": "test",   "command": "cargo test",                   "purpose": "Test suite — AUTHORITATIVE for verdict", "gates": true },
      { "name": "build",  "command": "cargo build --release",        "purpose": "Build gate",  "gates": true }
    ]
  },
  "uiTest": { "enabled": false }
}
```

## Python / FastAPI + pydantic — no web UI to smoke-test

```json
{
  "$schema": "../.claude/workflows/harness.schema.json",
  "stack": "python-fastapi",
  "validation": {
    "checks": [
      { "name": "ruff",  "command": "ruff check .",  "purpose": "Lint gate",   "gates": true },
      { "name": "mypy",  "command": "mypy .",        "purpose": "Type gate",   "gates": true },
      { "name": "test",  "command": "pytest",        "purpose": "Test suite — AUTHORITATIVE for verdict", "gates": true }
    ]
  },
  "uiTest": { "enabled": false }
}
```

## Next.js (web) — UI-test stage enabled

The only profile that exercises the `uiTest` fields. `port` is the base port; in parallel task
runs the engine uses `port + taskNumber` automatically. `routes[]` are smoke-checked once
`readySignal` appears in the dev-server output.

```json
{
  "$schema": "../.claude/workflows/harness.schema.json",
  "stack": "nextjs",
  "validation": {
    "checks": [
      { "name": "lint",   "command": "npm run lint",        "purpose": "Lint gate",  "gates": true },
      { "name": "types",  "command": "npx tsc --noEmit",    "purpose": "Type gate",  "gates": true },
      { "name": "test",   "command": "npm test",            "purpose": "Test suite — AUTHORITATIVE for verdict", "gates": true },
      { "name": "build",  "command": "npm run build",       "purpose": "Build gate", "gates": true }
    ]
  },
  "uiTest": {
    "enabled": true,
    "devServerCommand": "npm run dev",
    "readySignal": "Ready in",
    "port": 3000,
    "routes": ["/", "/about"]
  }
}
```

---

*The harness carries the mechanism; this file carries the policy. Keep stack facts here, never
in `.claude/workflows/*.js`.*
