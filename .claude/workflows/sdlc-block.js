// =============================================================================
// sdlc-block — Spec-Level SDLC Orchestration (general, dependency-aware)
// =============================================================================
//
// Drives an ENTIRE spec to completion by orchestrating many parallel /sdlc-task
// pipelines across dependency-ordered waves, merging each wave before the next begins.
// Generic: every path derives from the spec slug, exactly like /sdlc-task. Nothing is
// hardcoded to a specific spec.
//
// USAGE
//   /sdlc-block <spec-slug>                  run every task in the spec
//   /sdlc-block <spec-slug> 1-7              run only tasks 1–7 (range/list selection)
//   /sdlc-block <spec-slug> --tasks 1,3,5-7  same selection via explicit flag
//   /sdlc-block <spec-slug> --max-retries 2 --max-wave-width 4
//
//   ARGS
//     <specSlug>          required — the planning/<specSlug>/ directory name
//     [range]             optional 2nd positional, or --tasks — e.g. 1-7, 1,3,5, 1-3,7. Default: all.
//     --max-retries N     total /sdlc-task attempts per task before escalation (default 2)
//     --max-wave-width W   max full pipelines run concurrently per batch (default 3)
//
// DESIGN (see docs/agentic-workflows/sdlc-orchestration.md)
//   0. Pre-flight (NEW)
//        - Guarantee a clean main tree with the spec committed BEFORE any wave runs.
//        - tasks.md missing        -> generate it from master-plan (+ referenced plan) and COMMIT (Opus).
//        - tasks.md uncommitted     -> commit it ("chore: commit spec for <slug>").
//        - any UNRELATED file dirty -> ABORT fast, listing them (failing at second 0 beats
//          escalating tasks mid-run when the merge step's clean-tree guard later trips).
//        - Root cause this fixes: /generate-tasks wrote tasks.md without committing, which both
//          blocked every merge (dirty-tree guard) and made worktrees regenerate their own spec.
//   1. Analyze
//        - Resume-scout: which tasks are already done on main (skip them).
//        - Worktree resume-scout (NEW): classify each task's EXISTING worktree/branch into one of
//          done-on-main | complete-unmerged-pass | complete-unmerged-fail | partial-post-implement |
//          partial-pre-implement | fresh — so a re-run MERGES or RESUMES instead of duplicating.
//        - Load planning/<specSlug>/sdlc/execution-plan.json if present & valid.
//        - Otherwise an agent reads tasks.md + breakdown.md and emits a DEPENDENCY
//          GRAPH WITH EVIDENCE (per task: filesCreated, filesModified, dependsOn,
//          and the quote that establishes each edge) plus an ADDITIVE-file allow-list.
//        - Deterministic JS computes the waves (topological layering + conflict
//          serialization). Agent proposes the graph; CODE computes the waves.
//        - The generated plan is written to execution-plan.json (hand-editable).
//   2. Per wave
//        - First consult each task's resume-state (from step 1):
//            complete-unmerged-pass  -> route the existing branch straight to merge (NO re-run)
//            complete-unmerged-fail  -> escalate (preserve worktree)
//            partial-post-implement  -> resume the existing worktree in place (--resume)
//            partial-pre-implement   -> tear down the stale worktree, then run fresh
//            fresh / done-on-main    -> run normally / skip
//        - Run each remaining task via workflow('sdlc-task', ...) with a RETRY + TRIAGE loop:
//            PASS                          -> merge it
//            RETRYABLE (infra/transient)   -> clean-slate re-run (up to MAX attempts)
//            MAJOR (structural / stuck)    -> ESCALATE: preserve worktree, surface to user
//        - Escalation poisons ONLY the dependent subtree (computed from the graph);
//          independent tasks keep running.
//   3. Merge (per wave, in task-number order)
//        - Plain git merge first; on conflict, union-merge ONLY if every conflicted
//          file is on the additive allow-list, else abort + escalate that task.
//        - status.md / log.md are NEVER touched inside worktrees.
//   4. Report
//        - Write block-workflow.md, apply log entries + a SINGLE authoritative
//          status.md update (spec done/partial, focus -> next spec), and print
//          escalations with their worktree paths + the resume command.
//
// RESUMPTION
//   Re-run /sdlc-block <specSlug>. Git is the source of truth: a task is "done" if its
//   taskN-workflow.md is committed on main. Done tasks are skipped; escalated tasks are
//   retried (after you fix the blocker or edit execution-plan.json). An interrupted run is
//   handled WITHOUT duplicating work: a task whose worktree completed PASS but never merged
//   is merged as-is; one interrupted after implement is resumed in place; one interrupted
//   before implement is torn down and re-run fresh (see the Analyze resume-scout, step 1).
//
// =============================================================================

export const meta = {
  name: 'sdlc-block',
  description: 'Orchestrate a full spec through dependency-ordered waves of parallel /sdlc-task pipelines, with bounded retries, failure triage, escalation, and ordered merges.',
  whenToUse: 'When driving a spec (a tasks.md) to completion across many parallel tasks. Optional task range, e.g. /sdlc-block <spec-slug> 1-7. Usage: /sdlc-block <spec-slug>',
  phases: [
    { title: 'Pre-flight', detail: 'Commit (or generate) the spec and guarantee a clean main tree before any merge', model: 'sonnet' },
    { title: 'Analyze',    detail: 'Resume-scout main, load or generate the dependency-ordered execution plan' },
    { title: 'Wave',       detail: 'Run wave tasks via /sdlc-task with retry + triage; escalate major failures' },
    { title: 'Merge',      detail: 'Merge passing branches in order (additive union only; else escalate)' },
    { title: 'Playwright', detail: 'Live browser sweep after all merges; fix regressions before reporting' },
    { title: 'Report',     detail: 'Write spec report, apply status/log once, surface escalations' },
  ]
}

// ----------------------------------------------------------------
// Parse args: "<specSlug> [--max-retries N] [--max-wave-width W]"
// ----------------------------------------------------------------
const rawArgs = typeof args === 'string' ? args.trim() : ''
if (!rawArgs) {
  log('ERROR: No spec slug provided.')
  log('Usage: /sdlc-block <spec-slug> [--max-retries 2] [--max-wave-width 4]')
  return { error: 'Missing required argument: spec slug' }
}

const tokens = rawArgs.split(/\s+/)
const blockId = tokens[0]

const DEFAULT_MAX_WAVE_WIDTH = 3   // conservative: 3 full pipelines in flight; bump with --max-wave-width

function flag(name, dflt) {
  const i = tokens.indexOf(name)
  if (i === -1 || i + 1 >= tokens.length) return dflt
  const v = parseInt(tokens[i + 1], 10)
  return isNaN(v) ? dflt : v
}
function flagStr(name) {
  const i = tokens.indexOf(name)
  return (i === -1 || i + 1 >= tokens.length) ? null : tokens[i + 1]
}
// Parse a task selection like "1-7", "1,3,5", "1-3,7", or "5" into a sorted int array.
function parseRange(spec) {
  const out = new Set()
  for (const part of spec.split(',')) {
    const m = part.trim().match(/^(\d+)(?:-(\d+))?$/)
    if (!m) return null
    const a = parseInt(m[1], 10), b = m[2] ? parseInt(m[2], 10) : a
    for (let i = Math.min(a, b); i <= Math.max(a, b); i++) out.add(i)
  }
  return [...out].sort((x, y) => x - y)
}

const MAX_TASK_ATTEMPTS = Math.max(1, flag('--max-retries', 2))                 // total sdlc-task runs per task
const MAX_WAVE_WIDTH    = Math.max(1, flag('--max-wave-width', DEFAULT_MAX_WAVE_WIDTH)) // pipelines per batch

// Optional task selection: `--tasks 1-7` OR a positional range as the 2nd token (`/sdlc-block <spec-slug> 1-7`).
// Defaults to ALL tasks in the spec.
const rangeSpec = flagStr('--tasks') || (tokens[1] && !tokens[1].startsWith('--') ? tokens[1] : null)
let selectedTasks = null
if (rangeSpec) {
  const parsed = parseRange(rangeSpec)
  if (!parsed || parsed.length === 0) {
    log(`ERROR: could not parse task selection "${rangeSpec}". Use forms like 1-7, 1,3,5, or 1-3,7.`)
    return { error: 'Invalid task selection', rangeSpec, blockId }
  }
  selectedTasks = new Set(parsed)
}

const blockDir    = `planning/${blockId}`
const tasksFile   = `${blockDir}/tasks.md`
const breakdownFile = `${blockDir}/breakdown.md`
const reportsDir  = `${blockDir}/sdlc/reports`
const planFile    = `${blockDir}/sdlc/execution-plan.json`
const blockReport = `${reportsDir}/block-workflow.md`

log(`Spec: ${blockId} | plan: ${planFile}`)
log(`Max attempts/task: ${MAX_TASK_ATTEMPTS} | max wave width: ${MAX_WAVE_WIDTH}`)
if (selectedTasks) log(`Task selection: ${[...selectedTasks].sort((a, b) => a - b).join(', ')} (others skipped)`)

// ================================================================
// Schemas
// ================================================================
const PREFLIGHT_SCHEMA = {
  type: 'object',
  required: ['ready', 'action'],
  properties: {
    ready:      { type: 'boolean', description: 'true if the tree is clean and the spec is committed — safe to proceed' },
    action:     { type: 'string', enum: ['generated', 'committed', 'clean', 'aborted'], description: 'what the pre-flight did' },
    reason:     { type: 'string', description: 'If aborted: why (e.g. unrelated dirty files, generate failed)' },
    dirtyFiles: { type: 'array', items: { type: 'string' }, description: 'non-spec files blocking the run (when aborted)' },
    commitHash: { type: 'string' }
  }
}

const ANALYZE_SCHEMA = {
  type: 'object',
  required: ['planExists', 'allTasks', 'doneTasks', 'tasks', 'additiveFiles'],
  properties: {
    planExists: { type: 'boolean', description: 'true if a valid execution-plan.json already existed' },
    allTasks:   { type: 'array', items: { type: 'integer' }, description: 'Every task number in tasks.md' },
    doneTasks:  { type: 'array', items: { type: 'integer' }, description: 'Tasks already completed on main (taskN-workflow.md present, PASS)' },
    preservedWorktrees: { type: 'array', items: { type: 'string' }, description: 'Worktree paths preserved from a prior escalated run' },
    resumeStates: {
      type: 'array',
      description: 'One entry per task describing the state of any existing worktree/branch, so the wave loop can merge/resume instead of re-running.',
      items: {
        type: 'object',
        required: ['num', 'state'],
        properties: {
          num:          { type: 'integer' },
          state:        { type: 'string', enum: ['done-on-main', 'complete-unmerged-pass', 'complete-unmerged-fail', 'partial-post-implement', 'partial-pre-implement', 'fresh'] },
          branchName:   { type: 'string', description: 'Existing branch for this task, if any' },
          worktreePath: { type: 'string', description: 'Absolute path to the existing worktree, if any' },
          verdict:      { type: 'string', description: 'Review verdict from the worktree (PASS/FAIL/PARTIAL), if a review report exists' }
        }
      }
    },
    additiveFiles: { type: 'array', items: { type: 'string' }, description: 'Shared files every touching task only APPENDS to (safe to union-merge), e.g. lib/services/index.ts' },
    tasks: {
      type: 'array',
      description: 'One entry per task — the dependency graph with evidence.',
      items: {
        type: 'object',
        required: ['num', 'title', 'dependsOn', 'filesCreated', 'filesModified'],
        properties: {
          num:           { type: 'integer' },
          title:         { type: 'string' },
          dependsOn:     { type: 'array', items: { type: 'integer' }, description: 'Task numbers this task logically depends on' },
          filesCreated:  { type: 'array', items: { type: 'string' } },
          filesModified: { type: 'array', items: { type: 'string' }, description: 'Existing shared files this task edits' },
          evidence:      { type: 'string', description: 'Quote(s) from tasks.md/breakdown.md establishing each dependsOn edge' },
          recommendBreakdown: { type: 'boolean', description: 'true if this task is coarse enough to benefit from /breakdown into atomic sub-steps (see the coarseness heuristic in STEP 3b)' },
          breakdownReason:    { type: 'string', description: 'one sentence: which coarseness signal fired (only when recommendBreakdown)' }
        }
      }
    },
    waves: {
      type: 'array',
      description: 'Present ONLY when planExists: the pre-existing wave structure to use verbatim.',
      items: {
        type: 'object',
        required: ['tasks'],
        properties: {
          label:      { type: 'string' },
          parallel:   { type: 'boolean' },
          tasks:      { type: 'array', items: { type: 'integer' } },
          mergeOrder: { type: 'array', items: { type: 'integer' } }
        }
      }
    },
    notes: { type: 'string' }
  }
}

const TRIAGE_SCHEMA = {
  type: 'object',
  required: ['class', 'reason'],
  properties: {
    class:  { type: 'string', enum: ['RETRYABLE', 'MAJOR'] },
    reason: { type: 'string', description: 'One sentence: why retryable (transient/infra/progressing) or major (stuck/structural/needs user)' },
    sameCriteriaAsBefore: { type: 'boolean', description: 'true if the SAME acceptance criteria failed as the previous attempt' }
  }
}

const MERGE_SCHEMA = {
  type: 'object',
  required: ['taskNum', 'branchName', 'merged', 'strategy', 'escalated'],
  properties: {
    taskNum:        { type: 'integer' },
    branchName:     { type: 'string' },
    merged:         { type: 'boolean' },
    strategy:       { type: 'string', enum: ['auto', 'union', 'aborted'], description: 'auto = clean git merge; union = additive-only conflict; aborted = escalated' },
    conflictedFiles:{ type: 'array', items: { type: 'string' } },
    escalated:      { type: 'boolean', description: 'true if a non-additive conflict forced an abort (worktree preserved)' },
    commitHash:     { type: 'string' },
    notes:          { type: 'string' }
  }
}

const REPORT_SCHEMA = {
  type: 'object',
  required: ['reportFile', 'overallVerdict'],
  properties: {
    reportFile:     { type: 'string' },
    overallVerdict: { type: 'string', enum: ['PASS', 'PARTIAL', 'BLOCKED'] },
    statusUpdated:  { type: 'boolean' },
    nextFocus:      { type: 'string' },
    notes:          { type: 'string' }
  }
}

const PLAYWRIGHT_SCHEMA = {
  type: 'object',
  required: ['verdict', 'checks'],
  properties: {
    verdict: { type: 'string', enum: ['PASS', 'WARN', 'FAIL'] },
    checks: {
      type: 'array',
      items: {
        type: 'object',
        required: ['check', 'scope', 'result'],
        properties: {
          check:  { type: 'string' },
          scope:  { type: 'string' },
          result: { type: 'string', enum: ['PASS', 'FAIL', 'WARN'] },
          notes:  { type: 'string' }
        }
      }
    },
    failureSummary: { type: 'string', description: 'If FAIL: what failed, error text, likely root cause' },
    serverStarted:  { type: 'boolean', description: 'true if this agent started the dev server (so it should also stop it)' }
  }
}

const PLAYWRIGHT_FIX_SCHEMA = {
  type: 'object',
  required: ['fixed', 'commitHash'],
  properties: {
    fixed:          { type: 'boolean' },
    commitHash:     { type: 'string' },
    changesSummary: { type: 'string' },
    notes:          { type: 'string', description: 'If fixed=false: exactly what the user must do to resolve the failure manually' }
  }
}

// ================================================================
// TOKEN TELEMETRY (Phase A — additive, no behavior change)
//
// Per-stage attribution of injected-prompt size and output-token delta for THIS engine's own
// orchestration agents (preflight/analyze/write-plan/teardown/triage/merge/playwright/report).
// `budget.spent()` is a shared pool, so outTok attributes cleanly only for the SEQUENTIAL stages
// (preflight, analyze, merge, report); the parallel runTask fan-out is each a child /sdlc-task
// whose OWN per-stage metrics carry the truth, so we only roll up an aggregate here. `agent`
// stays importable for any call we deliberately want untraced.
// ================================================================
const metrics = []
async function tracedAgent(prompt, opts = {}) {
  const before = (typeof budget !== 'undefined' && budget.spent) ? budget.spent() : 0
  const r = await agent(prompt, opts)
  const after = (typeof budget !== 'undefined' && budget.spent) ? budget.spent() : 0
  metrics.push({
    label: opts.label || 'agent',
    model: opts.model || 'session',
    promptTokEst: Math.round(prompt.length / 4),
    outTok: after - before > 0 ? after - before : null,
  })
  return r
}

// ================================================================
// Pure helpers — waves & failure blast-radius are computed in CODE
// ================================================================

// Topological layering + conflict serialization. Agent supplies the graph; this turns
// it into ordered waves (one per topological layer — width is NOT baked in here; it's an
// execution-time batching knob, so the persisted plan stays width-independent). Tasks in
// the same wave are mutually independent and share no EXCLUSIVE (non-additive) file.
function computeWaves(taskMap, additiveSet) {
  const nums = Object.keys(taskMap).map(Number).sort((a, b) => a - b)
  const mustFollow = new Map(nums.map(n => [n, new Set()])) // n must run after every task in this set

  // logical dependency edges
  for (const n of nums) {
    for (const d of (taskMap[n].dependsOn || [])) {
      if (taskMap[d]) mustFollow.get(n).add(d)
    }
  }
  // conflict edges: two tasks editing the same EXCLUSIVE file are serialized (lower number first)
  for (let i = 0; i < nums.length; i++) {
    for (let j = i + 1; j < nums.length; j++) {
      const a = nums[i], b = nums[j]
      const modA = new Set(taskMap[a].filesModified || [])
      const clash = (taskMap[b].filesModified || []).some(f => modA.has(f) && !additiveSet.has(f))
      if (clash) mustFollow.get(b).add(a)
    }
  }

  const remaining = new Set(nums)
  const waves = []
  while (remaining.size) {
    const layer = [...remaining].filter(n => [...mustFollow.get(n)].every(d => !remaining.has(d)))
    if (!layer.length) throw new Error(`Dependency cycle among tasks: ${[...remaining].join(', ')}`)
    layer.sort((a, b) => a - b)
    waves.push({
      label: `Wave ${waves.length + 1}`,
      parallel: layer.length > 1,
      tasks: layer,
      mergeOrder: [...layer].sort((a, b) => a - b)
    })
    layer.forEach(n => remaining.delete(n))
  }
  return waves
}

// A task is poisoned if any task it depends on escalated or was itself poisoned.
function isPoisoned(taskNum, taskMap, badSet) {
  return (taskMap[taskNum]?.dependsOn || []).some(d => badSet.has(d))
}

// ----------------------------------------------------------------
// HARNESS CONFIG — mechanism/policy split (see planning/harness.json)
//
// The engine ships NO stack defaults. A project declares its validation policy in
// planning/harness.json. The workflow runtime has no filesystem access, so a dedicated
// micro-loader agent reads + parses the file (the same way the Analyze stage loads
// execution-plan.json). Returns the parsed config object, or null when the file is absent or
// invalid. For sdlc-block the relevant policy is uiTest — when wired in P4, the Playwright sweep
// runs only if cfg.uiTest.enabled, using cfg.uiTest.{port,routes}; absent/disabled → skip it.
// ----------------------------------------------------------------
const HARNESS_CONFIG_SCHEMA = {
  type: 'object',
  required: ['present'],
  properties: {
    present: { type: 'boolean', description: 'true if planning/harness.json exists and parsed as valid JSON' },
    config: {
      type: 'object',
      description: 'The parsed harness.json (omit when present is false)',
      properties: {
        stack: { type: 'string' },
        validation: {
          type: 'object',
          properties: {
            checks: {
              type: 'array',
              items: {
                type: 'object',
                properties: {
                  kind:    { type: 'string', description: 'command (default) | baseline-diff | count-delta | warning-scan | forbidden-pattern-scan' },
                  name:    { type: 'string' },
                  command: { type: 'string' },
                  purpose: { type: 'string' },
                  gates:   { type: 'boolean' },
                  baselineCommand: { type: 'string', description: 'baseline-diff only' },
                  compareKeys:     { type: 'array', items: { type: 'string' }, description: 'baseline-diff only' },
                  countPattern:    { type: 'string', description: 'count-delta only' },
                  failOn:          { type: 'string', description: 'count-delta only: decrease | zero-or-decrease' },
                  warningPatterns: { type: 'array', items: { type: 'string' }, description: 'warning-scan only' },
                  rules: {
                    type: 'array',
                    description: 'forbidden-pattern-scan only',
                    items: {
                      type: 'object',
                      properties: {
                        id:               { type: 'string' },
                        pattern:          { type: 'string' },
                        paths:            { type: 'string' },
                        allowlistPattern: { type: 'string' }
                      }
                    }
                  }
                }
              }
            }
          }
        },
        uiTest: {
          type: 'object',
          properties: {
            enabled:          { type: 'boolean' },
            devServerCommand: { type: 'string' },
            readySignal:      { type: 'string' },
            port:             { type: 'integer' },
            routes:           { type: 'array', items: { type: 'string' } }
          }
        },
        breakdown: {
          type: 'object',
          properties: {
            mode:                { type: 'string', description: 'recommend (default) | auto | off' },
            complexityThreshold: { type: 'integer' }
          }
        }
      }
    },
    notes: { type: 'string' }
  }
}

// Spawn the micro-loader agent and return the parsed config (or null). Wired into the Playwright
// stage in P4; defined here so the loader path exists from P1. No stack defaults on absence.
async function loadHarnessConfig() {
  const result = await tracedAgent(`
You are the harness-config loader for the SDLC pipeline. Your ONLY job is to read the project's
validation-policy file and return it as structured data. Do not run any checks or modify anything.

STEP 1 — Read the config file (from the main repo root):
  cat planning/harness.json 2>/dev/null && echo "__HARNESS_PRESENT__" || echo "__HARNESS_ABSENT__"

STEP 2 — Decide:
  - "__HARNESS_ABSENT__" (file missing) → present=false, omit config.
  - File printed but NOT valid JSON → present=false, notes="harness.json present but invalid JSON: <reason>".
  - File printed and valid JSON → present=true, and copy the parsed object into "config", keeping ONLY
    these fields when present: stack; validation.checks[] (each: {kind, name, command, purpose, gates}
    plus any kind-specific fields that are present — baselineCommand, compareKeys[], countPattern,
    failOn, warningPatterns[], rules[] ({id, pattern, paths, allowlistPattern})); uiTest ({enabled,
    devServerCommand, readySignal, port, routes[]}); breakdown ({mode, complexityThreshold}). Preserve
    kind-specific fields verbatim; ignore any other fields.

Return your findings using the StructuredOutput tool.
`, { label: 'harness-config', schema: HARNESS_CONFIG_SCHEMA, model: 'haiku' })

  if (!result || !result.present || !result.config) return null
  return result.config
}

// Render the post-merge browser-sweep prompt parts from harness config. Called ONLY when
// cfg.uiTest.enabled. Interpolates the MVP fields (devServerCommand / readySignal / port / routes).
function renderUiTestPrompt(cfg, port) {
  const ui = cfg.uiTest
  const routes = (Array.isArray(ui.routes) && ui.routes.length) ? ui.routes : ['/']
  const ready = ui.readySignal || 'ready'
  const devCmd = ui.devServerCommand || 'echo "ERROR: uiTest.enabled but devServerCommand missing in planning/harness.json" && false'
  const routeList = routes.join('  ')
  return { routes, routeList, ready, devCmd, port }
}

// ================================================================
// PRE-FLIGHT: guarantee a clean main tree with the spec committed.
//
// Root cause this prevents: /generate-tasks writes tasks.md but never commits it,
// so (1) the merge agent's clean-tree guard blocks EVERY merge, and (2) worktrees
// (which branch off a commit) don't contain the spec and regenerate their own.
// Here we commit an uncommitted spec, generate+commit a missing one, and abort fast
// if any UNRELATED file is dirty (failing at second 0 beats escalating tasks mid-run).
// ================================================================
phase('Pre-flight')
log('Pre-flight: verifying clean tree and committed spec...')

const preflight = await tracedAgent(`
You are the pre-flight agent for a spec-level SDLC orchestration. You run from the MAIN repo root
(CWD = the main checkout, on main). Your job: guarantee the working tree is clean AND the spec for
"${blockId}" is committed, so the downstream merge step (which requires a clean tree) never blocks
and every worktree contains the committed spec.

Spec slug:    ${blockId}
Spec dir:     ${blockDir}
Tasks file:   ${tasksFile}

STEP 1 — Inspect the working tree:
  git rev-parse --show-toplevel
  git status --porcelain
  Classify every dirty path:
    - SPEC paths = anything under "${blockDir}/" (tasks.md, breakdown.md, execution-plan.json, reports).
    - OTHER = everything else.

STEP 2 — Abort on unrelated dirt:
  If ANY OTHER (non-spec) path is dirty (modified, staged, or untracked), STOP. Return
  ready=false, action="aborted", reason="main working tree has uncommitted changes outside the
  spec dir; commit or stash them before running", dirtyFiles=<the OTHER paths>. Do NOT commit them.

STEP 3 — Ensure the spec exists and is committed:
  Check: ls ${tasksFile} 2>/dev/null && echo "SPEC_PRESENT" || echo "SPEC_MISSING"

  CASE A — SPEC_MISSING:
    Generate the spec, mirroring the /generate-tasks skill:
      - mkdir -p ${reportsDir}
      - cat planning/master-plan.md  → find the section defining "${blockId}".
      - If that section references a plan file under planning/plans/, cat it and read the
        relevant per-path / per-block portion.
      - cat CLAUDE.md and planning/context.md → internalize and enforce the project's standing
        rules. CLAUDE.md is the authority; assume no stack/locale/narrative/content rule unless
        written there. Universal: no fabricated metrics or quotes, no emoji, every change ships with tests.
      - cat .claude/workflows/templates/spec-template.md → use as the FORMAT reference.
      - Write ${tasksFile} in the standard spec format: ## Goal, ## Context Pointers,
        ## Step-by-Step Tasks (numbered "### 1.", "### 2.", ... ; final task is always Validate),
        ## Acceptance Criteria, ## Validation Commands (mirror planning/harness.json's
        validation.checks[].command in order; if absent, use the project's documented build/test
        commands). Record any deferral in ## Notes.
    Then commit:
      git add ${blockDir}
      git commit -m "chore: add spec for ${blockId}"
      git log --oneline -1
    Return ready=true, action="generated", commitHash=<short hash>.

  CASE B — SPEC_PRESENT and any SPEC path is dirty (uncommitted/untracked):
      git add ${blockDir}
      git commit -m "chore: commit spec for ${blockId}"
      git log --oneline -1
    Return ready=true, action="committed", commitHash=<short hash>.

  CASE C — SPEC_PRESENT and the tree is fully clean:
    Nothing to do. Return ready=true, action="clean".

If spec generation fails for any reason, return ready=false, action="aborted", reason=<what failed>.

Return using StructuredOutput: ready, action, reason, dirtyFiles, commitHash.
`, { label: 'pre-flight', schema: PREFLIGHT_SCHEMA, phase: 'Pre-flight', model: 'sonnet' })  // dominant path is trivial scripted git (commit/clean); the rare SPEC_MISSING generate path is a fallback-of-a-fallback, so opus is not worth paying on every run. Re-tiered opus->sonnet (D11).

if (!preflight || !preflight.ready) {
  const why = preflight?.reason || 'pre-flight agent returned null'
  log(`Pre-flight ABORTED — ${why}`)
  if (preflight?.dirtyFiles?.length) {
    log('Uncommitted files outside the spec dir:')
    for (const f of preflight.dirtyFiles) log(`  - ${f}`)
    log('Commit or stash them, then re-run /sdlc-block ' + blockId + '.')
  }
  return { error: 'Pre-flight failed', reason: why, dirtyFiles: preflight?.dirtyFiles || [], blockId }
}
log(`Pre-flight OK — spec ${preflight.action}${preflight.commitHash ? ` (${preflight.commitHash})` : ''}.`)

// Load the project's policy once, up front (mechanism/policy split — see planning/harness.json).
// Reused by: the Analyze breakdown assessment (threshold), the breakdown gate below, and the
// post-merge Playwright sweep (uiTest). null when absent/invalid → breakdown defaults to 'recommend'.
const harnessCfg = await loadHarnessConfig()
const breakdownMode = harnessCfg?.breakdown?.mode || 'recommend'
const breakdownThreshold = harnessCfg?.breakdown?.complexityThreshold ?? 3

// ================================================================
// PHASE 0: ANALYZE — resume-scout + load/generate the execution plan
// ================================================================
phase('Analyze')
log('Analyzing spec: scouting completed tasks and resolving the dependency graph...')

const analysis = await tracedAgent(`
You are the analysis agent for a spec-level SDLC orchestration. You run from the MAIN repo root.

Spec:         ${blockId}
Tasks file:   ${tasksFile}
Breakdown:    ${breakdownFile} (optional)
Plan file:    ${planFile}
Reports dir:  ${reportsDir}

GOAL: produce a dependency graph for every task, list which tasks are already done, and report
whether a hand-written execution plan already exists.

STEP 1 — Check for an existing plan:
  cat ${planFile} 2>/dev/null && echo "PLAN_EXISTS" || echo "NO_PLAN"
  If it exists and is valid JSON with a "waves" array, set planExists=true and return its
  "tasks" graph and "waves" and "additiveFiles" VERBATIM (do not re-derive). Skip to STEP 4.

STEP 2 — Read the spec and (if present) the breakdown:
  cat ${tasksFile}
  ls ${breakdownFile} 2>/dev/null && cat ${breakdownFile} || echo "NO_BREAKDOWN"
  Enumerate every task by its "### N." heading. That set is "allTasks".

STEP 3 — Build the dependency graph. For EACH task determine:
  - filesCreated:  new files the task creates (e.g. a new module/component and its own test file,
                   a new content/asset file the task adds).
  - filesModified: EXISTING shared files the task edits (e.g. an index/barrel re-export, a shared
                   module, a registry/manifest file).
  - dependsOn:     task numbers whose output this task consumes. A depends on B if A's text
                   references a symbol/module/file B creates (e.g. "renders the widget added in Task 7").
  - evidence:      quote the exact phrase(s) from tasks.md/breakdown.md proving each dependsOn edge.
  Then classify shared files into "additiveFiles": a file belongs here if every task that touches it
  only CONTRIBUTES its own independent piece (an export line, a registry entry, a doc section for the
  unit IT created) rather than rewriting another task's lines.
  ADDITIVE BY DEFAULT — include these whenever more than one task touches them:
    - Barrel / index re-export files — each task adds an export line for the symbol it created.
    - Registry / manifest files where each task appends one independent entry (e.g. an index JSON,
      a route/command list, a feature-flag map) — each task appends its own row.
    - Auto-generated reference docs (docs/*.md) — each task's document stage appends a section
      describing its OWN unit.
    - A standalone file a task adds (with any sibling files it owns) is filesCreated, NOT a shared
      modification, so it never conflicts with another task's additions.
    Treating barrel/registry/doc files as EXCLUSIVE is the common failure mode here (it can falsely
    serialize independent tasks into a dependency CYCLE and then block their merges), so default them
    to ADDITIVE.
  Stay CONSERVATIVE only for real in-place edits to a shared source/config file edited by more than
  one task: if unsure whether one of THOSE is additive, LEAVE IT OUT (treat as exclusive).
  For dependsOn edges: if unsure whether an edge exists, INCLUDE it. Over-serializing logical deps is
  safe; wrongly marking an in-place-edited source file additive is not.

STEP 3b — Breakdown assessment. For EACH task, decide whether it is too COARSE to implement directly
  and would benefit from a /breakdown into atomic sub-steps. Set recommendBreakdown=true (with a
  one-sentence breakdownReason naming the signal) when ANY of these hold:
    - the task creates or modifies MORE than ${breakdownThreshold} distinct files (use the
      filesCreated + filesModified you just computed); OR
    - it bundles multiple separable concerns (e.g. "implement X AND refactor Y AND add Z"); OR
    - it spans multiple layers/modules (e.g. data model + API + UI); OR
    - it carries a large acceptance-criteria set covering independently-testable units.
  A single focused change over a small file set is NOT a candidate (recommendBreakdown=false). This is
  ASSESSMENT ONLY — do not write any breakdown file; the orchestrator acts on these flags per policy.

STEP 4 — Resume scout: find tasks already completed on MAIN:
  ls ${reportsDir}/task*-workflow.md 2>/dev/null || echo "NONE"
  For each task N with a task${'${N}'}-workflow.md present ON MAIN, read its Final Verdict; if PASS, add N to doneTasks.

STEP 5 — Worktree resume scout: classify each task's EXISTING worktree/branch so the orchestrator
  can MERGE or RESUME instead of re-running (this prevents duplicate worktrees on a re-run).
  List existing task worktrees for this spec:
    git worktree list | grep "trees/${blockId.toLowerCase()}-task" || echo "NO_WORKTREES"
  Also list any branches without a worktree (orphans):
    git branch --list "${blockId.toLowerCase()}-task*" || echo "NO_BRANCHES"

  For EACH task N in allTasks, emit one resumeStates entry { num, state, branchName, worktreePath, verdict }.
  Determine "state" with this EXACT priority (worktree base name = "${blockId.toLowerCase()}-taskN"):
    1. task${'${N}'}-workflow.md present on MAIN  → state="done-on-main".
    2. Else, if a worktree dir trees/${blockId.toLowerCase()}-taskN exists, inspect it:
       cd trees/${blockId.toLowerCase()}-taskN 2>/dev/null
       ls planning/${blockId}/sdlc/reports/task${'${N}'}-workflow.md 2>/dev/null && echo HAS_WF || echo NO_WF
       ls planning/${blockId}/sdlc/reports/task${'${N}'}-implement.md 2>/dev/null && echo HAS_IMPL || echo NO_IMPL
       grep -iE "\\*\\*Verdict|## Verdict" planning/${blockId}/sdlc/reports/task${'${N}'}-review.md 2>/dev/null | head -1
       (then cd back to the main repo root)
       - HAS_WF and review verdict PASS         → state="complete-unmerged-pass" (merge it, do NOT re-run).
       - HAS_WF and verdict FAIL/PARTIAL        → state="complete-unmerged-fail" (escalate; preserve).
       - NO_WF and HAS_IMPL                      → state="partial-post-implement" (resume in place).
       - NO_WF and NO_IMPL                       → state="partial-pre-implement" (teardown + fresh).
       Set branchName="${blockId.toLowerCase()}-taskN" and worktreePath=<absolute path to that dir>.
    3. Else, if an orphan branch "${blockId.toLowerCase()}-taskN" exists but no worktree dir → state="partial-pre-implement",
       branchName set, worktreePath empty (the run will teardown the orphan branch then start fresh).
    4. Else → state="fresh".

Return using StructuredOutput:
  planExists:   true only if a valid execution-plan.json was loaded
  allTasks:     every task number
  doneTasks:    completed-on-main task numbers
  preservedWorktrees: paths from prior escalations (or [])
  resumeStates: one entry per task (STEP 5)
  additiveFiles: shared files safe to union-merge
  tasks:        the dependency graph (one entry per task, with evidence)
  waves:        ONLY if planExists — the loaded wave structure
  notes:        anything notable (ambiguous deps, suspected cycles)
`, { label: 'analyze', schema: ANALYZE_SCHEMA, phase: 'Analyze', model: 'opus' })  // dependency analysis is PLANNING — keep on Opus

if (!analysis) {
  log('Analysis agent returned null — cannot plan the spec. Aborting.')
  return { error: 'Analysis failed', blockId }
}

// Build the task map and additive set
const taskMap = {}
for (const t of (analysis.tasks || [])) taskMap[t.num] = t
const additiveSet = new Set(analysis.additiveFiles || [])
const doneTasks = new Set(analysis.doneTasks || [])

// Per-task resume classification (keyed by task number). Defaults to 'fresh' for any task
// the scout didn't report on.
const resumeByTask = {}
for (const rs of (analysis.resumeStates || [])) resumeByTask[rs.num] = rs
function resumeStateFor(n) { return resumeByTask[n] || { num: n, state: 'fresh' } }

if (Object.keys(taskMap).length === 0) {
  log('Analysis produced no tasks — aborting.')
  return { error: 'No tasks resolved from spec', blockId }
}

// Waves: use the loaded plan if present, else compute them deterministically
let waves
if (analysis.planExists && Array.isArray(analysis.waves) && analysis.waves.length) {
  waves = analysis.waves.map((w, i) => ({
    label: w.label || `Wave ${i + 1}`,
    parallel: w.parallel ?? (w.tasks.length > 1),
    tasks: w.tasks,
    mergeOrder: w.mergeOrder || [...w.tasks].sort((a, b) => a - b)
  }))
  log(`Loaded existing plan: ${waves.length} wave(s).`)
} else {
  try {
    waves = computeWaves(taskMap, additiveSet)
  } catch (e) {
    log(`Could not layer tasks into waves: ${e.message}`)
    return { error: 'Wave computation failed', detail: e.message, blockId, taskMap }
  }
  log(`Computed ${waves.length} wave(s) from the dependency graph.`)

  // Persist the generated plan so it is inspectable / hand-editable for re-runs
  const planJson = JSON.stringify({
    blockId,
    additiveFiles: [...additiveSet],
    tasks: taskMap,
    waves
  }, null, 2)

  await tracedAgent(`
You run from the MAIN repo root. Write this exact JSON to ${planFile} (create parent dirs if needed)
and commit it. Do not alter the content.

  mkdir -p ${reportsDir}
  Write the file ${planFile} with EXACTLY this content:
\`\`\`json
${planJson}
\`\`\`
  Then: git add ${planFile} && git commit -m "chore: add execution plan for ${blockId}" || echo "nothing to commit"

Return using StructuredOutput: reportFile="${planFile}", success=true.
`, { label: 'write-plan', schema: { type: 'object', required: ['success'], properties: { reportFile: { type: 'string' }, success: { type: 'boolean' } } }, phase: 'Analyze', model: 'haiku' })
}

// Log the plan for visibility
for (const w of waves) {
  const skipped = w.tasks.filter(t => doneTasks.has(t))
  log(`${w.label}: tasks [${w.tasks.join(', ')}]${w.parallel ? ' (parallel)' : ' (sequential)'}${skipped.length ? ` — already done: ${skipped.join(', ')}` : ''}`)
}

// Pre-flight: if a selection was given, warn about prerequisites outside it that aren't done on main.
// These tasks will likely fail and escalate — surface it up front rather than mid-run.
if (selectedTasks) {
  const missingPrereqs = []
  for (const t of selectedTasks) {
    for (const d of (taskMap[t]?.dependsOn || [])) {
      if (!selectedTasks.has(d) && !doneTasks.has(d)) missingPrereqs.push(`task ${t} needs task ${d}`)
    }
  }
  if (missingPrereqs.length) {
    log(`WARNING: selected tasks have unmet prerequisites outside the range (not yet done on main):`)
    for (const m of missingPrereqs) log(`  - ${m}`)
    log(`These tasks may fail and escalate. Widen the range or run the prerequisites first.`)
  }
}

// ================================================================
// BREAKDOWN GATE — recommend or (auto mode) generate sub-steps for coarse tasks BEFORE the waves.
//
// The Analyze stage flagged coarse tasks (recommendBreakdown). Acting here — once, on main, before
// any worktree is created — is what makes auto mode safe: breakdown.md is committed on main so every
// parallel worktree inherits the SAME file (no shared-file merge conflict, the D9 class of bug). Each
// /sdlc-task is later invoked with --under-block so it does NOT re-assess. mode 'off' skips this.
// ================================================================
if (breakdownMode !== 'off') {
  const flagged = Object.values(taskMap).filter(t =>
    t.recommendBreakdown && !doneTasks.has(t.num) && (!selectedTasks || selectedTasks.has(t.num)))
  if (!flagged.length) {
    log(`Breakdown: no tasks flagged as coarse (mode=${breakdownMode}).`)
  } else if (breakdownMode === 'auto') {
    log(`Breakdown (auto): generating sub-steps for ${flagged.length} coarse task(s): ${flagged.map(t => t.num).join(', ')}...`)
    const stepList = flagged.map(t => `  - Task ${t.num} (${t.title}): ${t.breakdownReason || 'coarse'}`).join('\n')
    const gen = await tracedAgent(`
You run from the MAIN repo root. Author atomic sub-steps (a /breakdown) for the COARSE tasks below so
each parallel /sdlc-task implements a granular plan. Write them into ${breakdownFile} and commit on main
BEFORE any worktree is created. Do NOT implement anything and do NOT modify ${tasksFile}.

Coarse tasks to break down:
${stepList}

1. Read the spec and the real source each flagged task touches:
   cat ${tasksFile}
   For each flagged task, read the files its "### N." section references so sub-steps name real paths/symbols.

2. Write ${breakdownFile} (create it, or append a section per flagged task). For EACH flagged task N:
   - FIRST check whether it is already covered: grep -q "### Step N:" ${breakdownFile} 2>/dev/null. If so,
     SKIP it (a prior run wrote it) — never duplicate a section.
   - Otherwise add:

   ### Step N: <task title>
   - N.1 <atomic action — exact file path + symbol + what to write/change>
   - N.2 <...>
   **Verify:** <a concrete command/check after each logical group>

   Each sub-step is a SINGLE atomic action naming exact file paths and function/component names. Do NOT
   add sections for tasks that are not in the list above. tasks.md stays authoritative for scope and
   acceptance criteria; breakdown.md is authoritative for HOW.

3. Commit on main:
   git add ${breakdownFile} && git commit -m "docs: breakdown for ${blockId} coarse tasks" || echo "nothing to commit"
   git log --oneline -1

Return using StructuredOutput: reportFile="${breakdownFile}", success, filesModified, commitHash, notes.
`, { label: 'breakdown-gen', schema: { type: 'object', required: ['success'], properties: { reportFile: { type: 'string' }, success: { type: 'boolean' }, filesModified: { type: 'array', items: { type: 'string' } }, commitHash: { type: 'string' }, notes: { type: 'string' } } }, phase: 'Analyze', model: 'opus' })  // breakdown-gen is PLANNING — keep on Opus
    if (gen?.success) log(`Breakdown committed${gen.commitHash ? ` (${gen.commitHash})` : ''} — worktrees will inherit it.`)
    else log(`Breakdown generation did not complete — tasks will implement from tasks.md only.`)
  } else {
    // recommend mode — surface the recommendation; proceed without writing anything.
    log(`Breakdown recommendation (mode=recommend): ${flagged.length} task(s) look coarse. Consider running`)
    log(`  /breakdown ${tasksFile}`)
    log(`before this block, or set breakdown.mode:"auto" in planning/harness.json. Flagged:`)
    for (const t of flagged) log(`  - Task ${t.num} (${t.title}): ${t.breakdownReason || 'coarse'}`)
  }
}

// ================================================================
// Per-task retry + triage state machine
// ================================================================
const badSet = new Set()        // escalated OR poisoned task numbers
const outcomes = []             // { taskNum, status, branchName, worktreePath, finalVerdict, reasons, attempts }
const passedBranches = []       // ready-to-merge: { taskNum, branchName, worktreePath }

async function teardownBranch(branchName, worktreePath) {
  await tracedAgent(`
You run from the MAIN repo root. Tear down a failed-attempt worktree so retries don't accumulate.
Run, ignoring errors:
  git worktree remove "${worktreePath}" --force 2>/dev/null || true
  git branch -D "${branchName}" 2>/dev/null || true
  git worktree prune
Confirm with: git worktree list
Return the final worktree list as plain text.
`, { label: `teardown-${branchName}`, phase: 'Analyze', model: 'haiku' })
}

async function runTask(taskNum, resume = false) {
  let prevReasons = null
  for (let attempt = 1; attempt <= MAX_TASK_ATTEMPTS; attempt++) {
    // Only the FIRST attempt resumes an existing worktree; a clean-slate retry must start fresh
    // (the teardown below removes the worktree, so subsequent attempts always create a new one).
    const resumeArg = (resume && attempt === 1) ? ' --resume' : ''
    // --under-block: the block already ran the breakdown assessment once (Analyze) and acted on it
    // above, so the per-task engine must NOT re-assess.
    log(`Task ${taskNum}: attempt ${attempt}/${MAX_TASK_ATTEMPTS} via /sdlc-task${resumeArg}...`)
    const r = await workflow('sdlc-task', `${blockId} ${taskNum}${resumeArg} --under-block`)

    if (r && r.finalVerdict === 'PASS') {
      return { taskNum, status: 'pass', branchName: r.branchName, worktreePath: r.worktreePath, finalVerdict: 'PASS', attempts: attempt }
    }

    // Classify the failure before deciding to retry
    const failBlob = JSON.stringify({
      finalVerdict: r?.finalVerdict ?? 'NULL_RESULT',
      reviewAttempts: r?.reviewAttempts,
      stageResults: r?.stageResults,
      previousFailureReasons: prevReasons
    }, null, 2)

    const triage = await tracedAgent(`
You are the failure-triage agent for a spec orchestration. A single task's /sdlc-task pipeline did
NOT pass. /sdlc-task already ran up to 3 internal fix passes before returning, so a genuine, repeated
acceptance-criteria failure is unlikely to be fixed by another clean-slate run.

Task: ${blockId} task ${taskNum}, attempt ${attempt} of ${MAX_TASK_ATTEMPTS}.
Pipeline result:
${failBlob}

Classify:
  RETRYABLE — transient/infra (NULL_RESULT, worktree-setup crash, agent died, NOT_REACHED), OR the
              failing criteria CHANGED from the previous attempt (it is making progress).
  MAJOR     — the SAME acceptance criteria failed again, OR the failure is structural (references a
              missing upstream symbol/dependency, wrong wave ordering, or otherwise needs a human to
              re-plan or fix the blocker). When unsure, prefer MAJOR — escalation is cheap, a wasted
              clean-slate retry is not.

Return using StructuredOutput: class, reason, sameCriteriaAsBefore.
`, { label: `triage-${taskNum}-${attempt}`, schema: TRIAGE_SCHEMA, phase: 'Analyze', model: 'sonnet' })

    const reasons = (r?.stageResults || [])
      .filter(s => s.stage === 'review')
      .flatMap(s => s.unmetCriteria || s.failureReasons || [])

    if (!triage || triage.class === 'MAJOR' || attempt === MAX_TASK_ATTEMPTS) {
      const why = triage?.reason || (attempt === MAX_TASK_ATTEMPTS ? 'retries exhausted' : 'triage returned null')
      log(`Task ${taskNum}: ESCALATE — ${why}. Worktree preserved for inspection.`)
      return {
        taskNum, status: 'escalate',
        branchName: r?.branchName, worktreePath: r?.worktreePath,
        finalVerdict: r?.finalVerdict || 'NOT_REACHED',
        reasons, reviewReport: `${reportsDir}/task${taskNum}-review.md`,
        attempts: attempt, triage: triage?.reason
      }
    }

    // RETRYABLE → clean-slate: tear down this attempt's worktree, then loop
    log(`Task ${taskNum}: RETRYABLE — ${triage.reason}. Clean-slate retry.`)
    if (r?.branchName && r?.worktreePath) await teardownBranch(r.branchName, r.worktreePath)
    prevReasons = reasons
  }
  // unreachable, but keep the shape consistent
  return { taskNum, status: 'escalate', finalVerdict: 'NOT_REACHED', reasons: [], attempts: MAX_TASK_ATTEMPTS }
}

// ================================================================
// WAVE LOOP
// ================================================================
for (let wi = 0; wi < waves.length; wi++) {
  const wave = waves[wi]
  const waveLabel = `Wave ${wi + 1}`
  phase(waveLabel)

  // Budget guard: if a token target is set and the remaining can't cover a wave, stop and report.
  // Rough estimate: a full sdlc-task pipeline is ~150k output tokens; size to this wave's width.
  if (budget.total) {
    const estPerTask = 150_000
    const waveCost = Math.min(wave.tasks.length, MAX_WAVE_WIDTH) * estPerTask
    if (budget.remaining() < waveCost) {
      log(`Budget guard: ~${Math.round(budget.remaining() / 1000)}k remaining < ~${Math.round(waveCost / 1000)}k needed for ${waveLabel}. Stopping before overrun — re-run /sdlc-block ${blockId} to resume.`)
      break
    }
  }

  // Decide which tasks actually run: honor selection, skip done (resume) and poisoned (dep escalated),
  // and consult each task's worktree resume-state so we MERGE/RESUME rather than re-run from scratch.
  const runnable = []
  const resumeTasks = new Set()   // tasks to run with --resume (existing worktree, implement already done)
  const mergeOnlyThisWave = []    // tasks routed straight to merge (complete-unmerged-pass) — not re-run
  for (const t of wave.tasks) {
    if (selectedTasks && !selectedTasks.has(t)) { continue }
    if (doneTasks.has(t)) { log(`Task ${t}: already done on main — skipping.`); continue }
    if (isPoisoned(t, taskMap, badSet)) {
      log(`Task ${t}: SKIPPED — depends on an escalated/failed task (${(taskMap[t].dependsOn || []).filter(d => badSet.has(d)).join(', ')}).`)
      badSet.add(t)
      outcomes.push({ taskNum: t, status: 'skipped', reasons: ['blocked by upstream escalation'] })
      continue
    }

    const rs = resumeStateFor(t)
    if (rs.state === 'complete-unmerged-pass') {
      // Pipeline already finished PASS in a worktree but never merged — hand straight to the merge
      // step (do NOT re-run). The merge loop below picks it up from passedBranches.
      log(`Task ${t}: complete-unmerged (PASS) — routing existing branch ${rs.branchName} to merge, skipping re-run.`)
      passedBranches.push({ taskNum: t, branchName: rs.branchName, worktreePath: rs.worktreePath })
      outcomes.push({ taskNum: t, status: 'pass', branchName: rs.branchName, worktreePath: rs.worktreePath, finalVerdict: 'PASS', attempts: 0, resumed: 'merge-only' })
      mergeOnlyThisWave.push(t)
      continue
    }
    if (rs.state === 'complete-unmerged-fail') {
      log(`Task ${t}: complete-unmerged (${rs.verdict || 'FAIL'}) — escalating; worktree preserved for inspection.`)
      badSet.add(t)
      outcomes.push({ taskNum: t, status: 'escalate', branchName: rs.branchName, worktreePath: rs.worktreePath, finalVerdict: rs.verdict || 'FAIL', reasons: ['prior run completed with a non-PASS verdict — needs your analysis'], reviewReport: `${reportsDir}/task${t}-review.md`, attempts: 0 })
      continue
    }
    if (rs.state === 'partial-pre-implement') {
      // Nothing worth preserving — clean the stale worktree/branch and run fresh.
      log(`Task ${t}: partial (pre-implement) — tearing down stale worktree/branch, will run fresh.`)
      if (rs.branchName) await teardownBranch(rs.branchName, rs.worktreePath || `trees/${rs.branchName}`)
    } else if (rs.state === 'partial-post-implement') {
      // Implement already completed — resume in place rather than recompute it.
      log(`Task ${t}: partial (post-implement) — resuming existing worktree ${rs.branchName} in place.`)
      resumeTasks.add(t)
    }
    runnable.push(t)
  }

  if (runnable.length === 0 && mergeOnlyThisWave.length === 0) {
    log(`${waveLabel}: nothing runnable — moving on.`)
    continue
  }
  if (runnable.length === 0) {
    log(`${waveLabel}: no tasks to run — ${mergeOnlyThisWave.length} already-complete branch(es) routed straight to merge.`)
  }

  // Execute: parallel waves in batches of MAX_WAVE_WIDTH; sequential waves one at a time
  let waveOutcomes = []
  if (wave.parallel) {
    for (let k = 0; k < runnable.length; k += MAX_WAVE_WIDTH) {
      const batch = runnable.slice(k, k + MAX_WAVE_WIDTH)
      if (batch.length < runnable.length) log(`${waveLabel}: batch [${batch.join(', ')}]`)
      const batchResults = await parallel(batch.map(t => () => runTask(t, resumeTasks.has(t))))
      waveOutcomes.push(...batchResults.filter(Boolean))
    }
  } else {
    for (const t of runnable) waveOutcomes.push(await runTask(t, resumeTasks.has(t)))
  }

  outcomes.push(...waveOutcomes)
  for (const o of waveOutcomes) {
    if (o.status === 'pass') {
      passedBranches.push({ taskNum: o.taskNum, branchName: o.branchName, worktreePath: o.worktreePath })
    } else if (o.status === 'escalate') {
      badSet.add(o.taskNum)
    }
  }

  // ---- Merge this wave's passing branches, in task-number order ----
  const toMerge = wave.mergeOrder
    .map(t => passedBranches.find(p => p.taskNum === t))
    .filter(Boolean)

  if (toMerge.length) {
    phase(`Merge ${wi + 1}`)
    for (const p of toMerge) {
      log(`Merging task ${p.taskNum} (branch: ${p.branchName})...`)
      const m = await tracedAgent(`
You are the merge agent. You run from the MAIN repo root (CWD = the main checkout, on main).
Merge branch "${p.branchName}" (task ${p.taskNum}) into main using a SELECTIVE-UNION strategy.

Additive files (safe to union-merge — every touching task only appends): ${JSON.stringify([...additiveSet])}

STEP 1 — Safety: cd to repo root and verify the working tree is clean before attempting any merge:
  git rev-parse --show-toplevel
  git status --porcelain
  If git status --porcelain produces ANY output (modified, untracked, or staged files), STOP
  immediately — do NOT proceed to STEP 2. Return merged=false, escalated=true, and notes:
    "MERGE BLOCKED: main working tree has uncommitted changes (listed above). Commit or stash
     them, then re-run /sdlc-block to resume."
  Only proceed to STEP 2 if git status --porcelain produces NO output (completely clean tree).

STEP 2 — Attempt a normal merge (do NOT use --ff-only; parallel branches share a base):
  git merge --no-ff --no-edit ${p.branchName}
  If it exits 0 with no conflicts -> strategy="auto", merged=true. Go to STEP 4.

STEP 3 — Conflict handling:
  git diff --name-only --diff-filter=U   (the conflicted files)
  - If EVERY conflicted file is in the additive list above, resolve them with git's built-in "union"
    merge driver, which keeps BOTH sides of each conflict (both export lines, both doc sections).
    IMPORTANT: there is NO "git merge -X union" — union is a low-level merge DRIVER, not a strategy
    option. Activate it repo-locally via .git/info/attributes (NEVER the tracked .gitattributes — this
    must not be committed), re-merge, then ALWAYS remove the driver again:
        git merge --abort
        cp .git/info/attributes /tmp/sdlc-attrs.bak 2>/dev/null || rm -f /tmp/sdlc-attrs.bak
        # register a temporary union driver for EXACTLY the conflicted additive files:
        for each conflicted additive file F:  printf '%s merge=union\\n' "F" >> .git/info/attributes
        git merge --no-ff --no-edit ${p.branchName}
        git diff --name-only --diff-filter=U   (recheck — usually empty now)
        # restore the attributes file so the temp driver can't affect later waves:
        if [ -f /tmp/sdlc-attrs.bak ]; then cp /tmp/sdlc-attrs.bak .git/info/attributes; else rm -f .git/info/attributes; fi
    If no conflicts remain -> stage all, commit the merge if not already committed -> strategy="union",
    merged=true. Go to STEP 4.
    If conflicts STILL remain on a barrel/index re-export file (its single all-different hunk can
    leave a union artifact — duplicated re-export lines), you MAY hand-resolve that ONE file: keep
    every re-export/export line, dedup, and drop any duplicated import block. Then re-run the
    project's compile/build gating check (a validation.checks[] command in planning/harness.json) to
    confirm it still builds; if it does -> strategy="union", merged=true. Otherwise:
        git merge --abort -> strategy="aborted", merged=false, escalated=true. Go to STEP 5.
  - If ANY conflicted file is NOT additive (a real code/content conflict):
        git merge --abort
        strategy="aborted", merged=false, escalated=true, conflictedFiles=<the non-additive ones>. Go to STEP 5.

STEP 4 — On success, capture the commit and clean up the worktree:
  git log --oneline -1   (commitHash = the short hash)
  git worktree remove "${p.worktreePath}" --force 2>/dev/null || true
  git branch -D ${p.branchName} 2>/dev/null || true
  git worktree prune
  (Do NOT touch planning/status.md or log.md — those are applied once in Report.)

STEP 5 — On escalation, PRESERVE the worktree and branch (the user will inspect them). Do not remove them.

Return using StructuredOutput: taskNum=${p.taskNum}, branchName="${p.branchName}", merged, strategy,
conflictedFiles, escalated, commitHash, notes.
`, { label: `merge-${p.taskNum}`, schema: MERGE_SCHEMA, phase: `Merge ${wi + 1}`, model: 'sonnet' })

      if (!m || !m.merged) {
        const note = m?.notes || 'merge agent returned null'
        log(`Task ${p.taskNum}: MERGE ESCALATED — ${note}. Branch preserved.`)
        badSet.add(p.taskNum)
        // reflect the merge failure in the outcome
        const o = outcomes.find(x => x.taskNum === p.taskNum)
        if (o) { o.status = 'escalate'; o.reasons = [...(o.reasons || []), `merge conflict: ${(m?.conflictedFiles || []).join(', ') || note}`] }
      } else {
        log(`Task ${p.taskNum}: merged (${m.strategy}) ${m.commitHash || ''}`)
        const o = outcomes.find(x => x.taskNum === p.taskNum)
        if (o) { o.merged = true; o.commitHash = m.commitHash; o.mergeStrategy = m.strategy }
      }
    }
  }

  if (budget.total) log(`Budget: ~${Math.round(budget.remaining() / 1000)}k tokens remaining.`)
}

// ================================================================
// PLAYWRIGHT VERIFICATION — live browser sweep after all merges
// ================================================================
let playwrightVerdict = 'SKIP'
let playwrightChecks  = []
let playwrightFixNotes = []
const MAX_PLAYWRIGHT_ATTEMPTS = 2

const hasMergedTasks = outcomes.some(o => o.merged)
// harnessCfg was loaded once up front (after pre-flight). The post-merge browser sweep runs ONLY
// when the project enables it; non-web projects skip it.
if (hasMergedTasks && harnessCfg?.uiTest?.enabled) {
  phase('Playwright')
  const ui = renderUiTestPrompt(harnessCfg, harnessCfg.uiTest.port ?? 3000)
  log('Running live browser verification against the dev server...')

  for (let attempt = 1; attempt <= MAX_PLAYWRIGHT_ATTEMPTS; attempt++) {
    log(`Playwright: attempt ${attempt}/${MAX_PLAYWRIGHT_ATTEMPTS}...`)

    const pw = await tracedAgent(`
You are the Playwright verification agent. You run from the MAIN repo root.

GOAL: Run live browser checks against the dev server to confirm that the merged work for
spec "${blockId}" renders correctly end-to-end. Report PASS/WARN/FAIL with per-check evidence.

STEP 1 — Dev server:
  Check if port ${ui.port} is already in use:
    lsof -ti:${ui.port}
  If a PID is returned: server is already running — skip startup, set serverStarted=false.
  If not running: start it in the background (use run_in_background=true):
    ${ui.devCmd}
  Poll the output every 5 s for "${ui.ready}" or "Local:" up to 90 s.
  If startup fails (error / non-zero exit / 90 s timeout), return immediately:
    verdict=FAIL, failureSummary="Dev server failed to start — check build errors."

STEP 2 — Open a browser session:
  playwright-cli open http://localhost:${ui.port}${ui.routes[0]}
  Take an initial snapshot to confirm the session is live.

STEP 3 — Smoke + routes (scope="routes"):
  For each route: ${ui.routeList}
    playwright-cli goto http://localhost:${ui.port}<route>
    playwright-cli snapshot
    playwright-cli console
    FAIL signals: title/heading contains "404","500","Error","Not Found"; bare framework error body;
                  snapshot has < 5 interactive elements on a content page;
                  console has error-level entries (warnings are WARN, not FAIL).
  Record each as one check row.

STEP 4 — Content spot-check (scope="content"):
  From any route's snapshot, pick an internal link → playwright-cli click <ref> → snapshot →
  verify the target page renders real content (a heading plus body text), not an error page.
  Record one check row.

STEP 5 — Close and cleanup:
  playwright-cli close
  If serverStarted=true: lsof -ti:${ui.port} | xargs kill 2>/dev/null || true

Return using StructuredOutput:
  verdict:        PASS (all checks pass) | WARN (all pass but console warnings found) | FAIL (any check fails).
  checks:         one row per check — check name, scope, result (PASS/WARN/FAIL), notes (evidence if not PASS).
  failureSummary: if FAIL — what failed, error text quoted, likely root cause in one paragraph.
  serverStarted:  true if you started the server in STEP 1.
`, { label: `playwright-${attempt}`, schema: PLAYWRIGHT_SCHEMA, phase: 'Playwright', model: 'sonnet' })

    playwrightVerdict = pw?.verdict ?? 'FAIL'
    playwrightChecks  = pw?.checks  ?? []

    if (playwrightVerdict !== 'FAIL') {
      const nPass = playwrightChecks.filter(c => c.result === 'PASS').length
      log(`Playwright: ${playwrightVerdict} — ${nPass}/${playwrightChecks.length} checks passed.`)
      if (playwrightVerdict === 'WARN') playwrightFixNotes.push('Playwright WARN: console warnings found — review before promoting to production.')
      break
    }

    // FAIL path
    const failSummary = pw?.failureSummary || 'Playwright agent returned null or did not report a failure summary.'
    log(`Playwright: FAIL — ${failSummary}`)

    if (attempt < MAX_PLAYWRIGHT_ATTEMPTS) {
      log(`Playwright: running targeted fix agent (attempt ${attempt})...`)

      const fix = await tracedAgent(`
You are a targeted regression-fix agent. A live Playwright browser sweep found failures after
the "${blockId}" spec was merged to main. Make the minimal surgical fix directly on main,
commit it, and return so Playwright can re-verify on the next attempt.

Do NOT run the full validation suite. Before committing, DO run the project's fast gating checks
(the gates:true entries in planning/harness.json — e.g. a lint/format check and any content/asset
validator); they must exit 0.

Playwright failures:
${JSON.stringify(playwrightChecks.filter(c => c.result === 'FAIL'), null, 2)}

Failure summary:
${failSummary}

Common targeted fixes:
- Route 404 → missing or misnamed file backing that route; compare against a known-good baseline.
- "Application error" / blank page → malformed data/frontmatter, a missing anchor/ID, or a broken
  import; re-run the project's content/asset validation check to surface errors.
- "Module not found" in console → missing or misspelled import; check the relevant source dirs.
- Missing/empty section → the source references a non-existent anchor/ID.

STEPS:
1. Diagnose: read the failure evidence carefully; identify the specific file(s) to change.
2. Fix: make the minimal targeted change. Do NOT refactor or touch unrelated code.
3. Re-run the project's gating checks (must exit 0 before continuing).
4. git add <specific files only>
5. git commit -m "fix: playwright regression after ${blockId}"
6. git log --oneline -1

If the issue requires changes that are too large or structural for a targeted patch (e.g. a
new spec task, a route rewrite, a missing feature block), do NOT attempt it. Return fixed=false
and in notes describe exactly what the user must do — for example:
  "Run /sdlc-task ${blockId} <N> to add the missing piece."
  "Edit tasks.md to add a dedicated fix task, then re-run /sdlc-block ${blockId}."

Return using StructuredOutput: fixed (bool), commitHash, changesSummary, notes.
`, { label: `playwright-fix-${attempt}`, schema: PLAYWRIGHT_FIX_SCHEMA, phase: 'Playwright', model: 'sonnet' })

      if (fix?.fixed) {
        log(`Playwright fix applied: ${fix.changesSummary || ''} (${fix.commitHash || ''})`)
        playwrightFixNotes.push(`Fix ${attempt}: ${fix.changesSummary || ''} (${fix.commitHash || ''})`)
      } else {
        const reason = fix?.notes || 'fix agent could not determine a targeted remedy'
        log(`Playwright fix attempt ${attempt} did not resolve the issue: ${reason}`)
        playwrightFixNotes.push(`Fix ${attempt} failed: ${reason}`)
        break
      }
    } else {
      log(`Playwright: FAIL after ${MAX_PLAYWRIGHT_ATTEMPTS} attempt(s) — escalating to user.`)
      playwrightFixNotes.push(`Playwright still FAIL after ${MAX_PLAYWRIGHT_ATTEMPTS} attempt(s). Manual resolution required.`)
    }
  }
} else if (!hasMergedTasks) {
  log('Playwright: skipped — no tasks merged, nothing to verify.')
} else {
  log('Playwright: skipped — planning/harness.json uiTest.enabled is false or config absent (non-web project).')
}

// ================================================================
// REPORT — spec report + single status/log update + escalations
// ================================================================
phase('Report')

const merged    = outcomes.filter(o => o.merged)
const escalated = outcomes.filter(o => o.status === 'escalate')
const skipped   = outcomes.filter(o => o.status === 'skipped')
const playwrightFailed = playwrightVerdict === 'FAIL'
const overall = escalated.length === 0 && skipped.length === 0 && !playwrightFailed ? 'PASS'
              : merged.length > 0 ? 'PARTIAL' : 'BLOCKED'

const outcomeTable = outcomes
  .sort((a, b) => a.taskNum - b.taskNum)
  .map(o => `| ${o.taskNum} | ${o.merged ? 'merged' : o.status} | ${o.finalVerdict || '—'} | ${o.mergeStrategy || '—'} | ${o.commitHash || '—'} | ${(o.reasons || []).join('; ').substring(0, 80) || '—'} |`)
  .join('\n')

const escalationBlock = escalated.length
  ? escalated.map(o => `- **Task ${o.taskNum}** — verdict ${o.finalVerdict}. ${o.triage || ''}\n    - Review: \`${o.reviewReport || `${reportsDir}/task${o.taskNum}-review.md`}\`\n    - Worktree (preserved): \`${o.worktreePath || 'n/a'}\` (branch \`${o.branchName || 'n/a'}\`)\n    - Reasons: ${(o.reasons || []).join('; ') || 'see review report'}`).join('\n')
  : '_None._'

const mergedTaskNums = merged.map(o => o.taskNum)

// Build the playwright section for the report
const playwrightCheckTable = playwrightChecks.length
  ? playwrightChecks.map(c => `   | ${c.check} | ${c.scope} | ${c.result} | ${c.notes || '—'} |`).join('\n')
  : '   _No checks recorded._'
const playwrightFixBlock = playwrightFixNotes.length
  ? '\n   **Fix attempts:**\n' + playwrightFixNotes.map(n => `   - ${n}`).join('\n')
  : ''
const playwrightSection = playwrightVerdict === 'SKIP'
  ? '   _Skipped — no tasks merged, nothing to verify._'
  : `   **Verdict:** ${playwrightVerdict}

   | Check | Scope | Result | Notes |
   |---|---|---|---|
${playwrightCheckTable}
${playwrightFixBlock}`

const playwrightEscalation = playwrightFailed
  ? `\n- **Playwright verification FAILED** — the live browser sweep found regressions after all merges.\n    - See the Playwright Verification section above for per-check details.\n    - To fix: run \`/playwright --scope full\` locally to reproduce, then \`/sdlc-task ${blockId} <N>\` or \`/fix\` to patch the regression.`
  : ''

// Token-telemetry roll-up (Phase A) — computed here (before the Report agent) so it can be appended
// verbatim to the block report. Covers only this engine's own orchestration agents; each task's
// per-stage detail lives in its own /sdlc-task workflow report.
const blockMetricsTable = metrics.map(m => {
  const out = m.outTok != null ? String(m.outTok) : '—'
  return `| ${m.label} | ${m.model} | ${m.promptTokEst} | ${out} |`
}).join('\n')
const totalOut = metrics.reduce((s, m) => s + (m.outTok || 0), 0)
const worstByPrompt = [...metrics].sort((a, b) => b.promptTokEst - a.promptTokEst).slice(0, 3)

const reportResult = await tracedAgent(`
You are the finalize/report agent for the spec orchestration. You run from the MAIN repo root.

Spec: ${blockId}
Overall verdict: ${overall}
Merged tasks (applied to main): ${JSON.stringify(mergedTaskNums)}
Escalated tasks (NOT merged): ${JSON.stringify(escalated.map(o => o.taskNum))}
Skipped (blocked by upstream): ${JSON.stringify(skipped.map(o => o.taskNum))}
Playwright verdict: ${playwrightVerdict}

DO THIS, IN ORDER:

1. Write the spec report to ${blockReport}:

   # Spec Orchestration Report — ${blockId}

   **Date:** [run: date +%Y-%m-%d]
   **Overall verdict:** ${overall}
   **Tasks merged:** ${mergedTaskNums.length}  |  **Escalated:** ${escalated.length}  |  **Skipped:** ${skipped.length}  |  **Playwright:** ${playwrightVerdict}

   ## Outcome by Task
   | Task | Result | Verdict | Merge | Commit | Notes |
   |---|---|---|---|---|---|
${outcomeTable}

   ## Playwright Verification
${playwrightSection}

   ## Escalations (need your attention)
${escalationBlock}${playwrightEscalation}

   ## Resume
   After fixing any blocker (or editing ${planFile}), re-run:  /sdlc-block ${blockId}
   Completed tasks are detected on main and skipped; escalated tasks are retried.
   ${playwrightFailed ? `Playwright failed — fix the regression first, then re-promote to production.` : ''}

2. Append the orchestrator token roll-up to ${blockReport}. Run EXACTLY as written (a literal heredoc
   append) — do NOT retype, summarize, or omit it; this is machine-generated telemetry:
   cat >> ${blockReport} <<'ROLLUP_EOF'

## Token Roll-up (orchestrator stages)
Attribution for THIS engine's own agents (preflight / analyze / merge / triage / report). Each task's
full per-stage detail lives in its own task<N>-workflow.md. promptTok = injected input estimate;
outTok = output-token delta ("—" when no +Nk budget target was set).

**Total orchestrator outTok:** ${totalOut || '—'}

| Stage | Model | promptTok | outTok |
|---|---|---|---|
${blockMetricsTable}
ROLLUP_EOF

3. Apply log + status ONCE (the per-task logs deferred these). The merged branches each carry a
   planning/${blockId}/sdlc/reports/task<N>-log.md committed on main with "Applied: false".
   For EACH merged task N in ${JSON.stringify(mergedTaskNums)} (ascending):
     - cat ${reportsDir}/task${'${N}'}-log.md
     - Append everything under its "## Log Entry" heading (from the "## YYYY-MM-DD" line onward,
       NOT the "## Log Entry" header itself) to the TOP of log.md (most-recent-first), once.
     - Flip that log's "**Applied:** false" -> "**Applied:** true".
   Then update planning/status.md ONCE:
     - If ALL tasks in the spec are now merged: mark the spec "Done" in the progress table and set
       "**Current focus:**" to the NEXT spec; otherwise mark it "In progress" and set Current focus to
       the lowest not-yet-merged task.
     - Bump the "**Last updated:**" line (today's date + one-line summary).
   Apply each merged task's "## status.md — *" sections from its log where present. Do NOT apply
   status/log for escalated or skipped tasks.

4. Commit:
   git add ${blockReport} log.md planning/status.md ${reportsDir}/task*-log.md
   git commit -m "chore: spec orchestration report + status for ${blockId}"
   git log --oneline -1

Return using StructuredOutput: reportFile="${blockReport}", overallVerdict="${overall}",
statusUpdated (true if status.md was updated), nextFocus (the Current focus string you wrote), notes.
`, { label: 'report', schema: REPORT_SCHEMA, phase: 'Report', model: 'sonnet' })

// ----------------------------------------------------------------
// Console echo of the roll-up (computed above; also persisted to the block report by the Report agent).
// ----------------------------------------------------------------
log(`Token roll-up (orchestrator stages): total outTok=${totalOut || '—'} | worst 3 by injected prompt: ${worstByPrompt.map(m => `${m.label} (~${m.promptTokEst} tok)`).join(', ') || 'none'}`)

// ----------------------------------------------------------------
// Final console summary
// ----------------------------------------------------------------
log('=== SPEC ORCHESTRATION COMPLETE ===')
log(`Overall: ${overall} | merged: ${merged.map(o => o.taskNum).join(', ') || 'none'} | escalated: ${escalated.map(o => o.taskNum).join(', ') || 'none'} | skipped: ${skipped.map(o => o.taskNum).join(', ') || 'none'} | playwright: ${playwrightVerdict}`)
if (escalated.length) {
  log('Escalations need your analysis:')
  for (const o of escalated) {
    log(`  Task ${o.taskNum}: ${o.triage || (o.reasons || []).join('; ')} | worktree: ${o.worktreePath || 'n/a'} | review: ${o.reviewReport || `${reportsDir}/task${o.taskNum}-review.md`}`)
  }
  log(`After fixing, resume with: /sdlc-block ${blockId}`)
}
if (playwrightFailed) {
  log('Playwright verification FAILED — live browser regressions found after merge.')
  if (playwrightFixNotes.length) for (const n of playwrightFixNotes) log(`  ${n}`)
  log(`To fix: run /playwright --scope full locally, then /sdlc-task ${blockId} <N> or /fix to patch.`)
}
log(`Spec report: ${reportResult?.reportFile || blockReport}`)

return {
  blockId,
  overallVerdict: overall,
  waves: waves.length,
  merged: merged.map(o => ({ taskNum: o.taskNum, commitHash: o.commitHash, mergeStrategy: o.mergeStrategy })),
  escalated: escalated.map(o => ({ taskNum: o.taskNum, finalVerdict: o.finalVerdict, worktreePath: o.worktreePath, branchName: o.branchName, reviewReport: o.reviewReport, reasons: o.reasons, triage: o.triage })),
  skipped: skipped.map(o => o.taskNum),
  playwright: { verdict: playwrightVerdict, checks: playwrightChecks, fixNotes: playwrightFixNotes },
  blockReport: reportResult?.reportFile || blockReport,
  resumeCommand: `/sdlc-block ${blockId}`,
  outcomes
}
