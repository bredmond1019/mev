// =============================================================================
// sdlc-block — Lean Spec-Level SDLC Orchestration ("a more powerful /sdlc-run")
// =============================================================================
//
// Drives an ENTIRE spec to completion with a FRESH implement agent per task (deliberate per-task
// context windows + observability — the one thing /sdlc-run lacks, since it implements all tasks in
// one context) and ONE consolidated back-half over the integrated result. Setup is shared once per
// block; per task runs lean (implement, + an optional localization review). Genuinely parallel waves
// (≥2 independent tasks) use worktrees; the common sequential case runs in-place on the integration
// branch. See decisions/D23 + D24. Generic: every path derives from the spec slug.
//
// USAGE
//   /sdlc-block <spec-slug>                       run every task in the spec
//   /sdlc-block <spec-slug> 1-7                   run only tasks 1–7 (range/list selection)
//   /sdlc-block <spec-slug> --tasks 1,3,5-7       same selection via explicit flag
//   /sdlc-block <spec-slug> --verify-depth consolidated+review   per-task review on (default: off)
//   /sdlc-block <spec-slug> --max-retries 2 --max-wave-width 4
//
//   ARGS
//     <specSlug>          required — the planning/<specSlug>/ directory name
//     [range]             optional 2nd positional, or --tasks — e.g. 1-7, 1,3,5, 1-3,7. Default: all.
//     --max-retries N     total implement attempts per task before escalation (default 2)
//     --max-wave-width W   max worktree implements run concurrently per parallel batch (default 3)
//     --verify-depth D    consolidated (default) | consolidated+review — per-task verification depth;
//                         overrides planning/harness.json block.verify (D24)
//
// DESIGN
//   0. Pre-flight — guarantee a clean integration tree with the spec committed (generate/commit it;
//        abort fast on unrelated dirt; D16/D19 structure + thin-spec guards) before any task runs.
//   1. Analyze — resume-scout; load planning/<specSlug>/sdlc/execution-plan.json if present & valid
//        (D22; authored by /generate-tasks), else an Opus agent emits the dependency graph and CODE
//        computes the dependency-ordered waves. SHARED SETUP ONCE (D23): harness config loaded once,
//        baseline snapshot captured once (only when a D6 baseline-diff check needs it).
//   2. Per wave (lean — implement only, + an optional localization review; D23):
//        - width-1 wave  -> run the task IN PLACE on the integration branch (no worktree, no merge).
//          The pre-task HEAD is captured; a failed implement is `git reset --hard`'d back before retry.
//        - width-≥2 wave -> isolate each task in a worktree via /sdlc-task --implement-only, then merge
//          passing branches in task order (additive-union only; else escalate). True concurrent writers.
//        - RETRY + TRIAGE per task: RETRYABLE -> clean-slate retry; MAJOR/exhausted -> ESCALATE.
//          Escalation poisons ONLY the dependent subtree; independent tasks keep running.
//   3. Consolidated back-half (D24) — ONLY when the block is complete (no escalations/skips): seed a
//        spec-level implement report from the per-task reports, then run ONE
//        test -> review -> fix -> (ui-test) -> document -> wrap-up over the integrated tree via
//        workflow('sdlc-run', '<slug> --from test'). Its wrap-up owns status.md / log.md / the spec
//        Amendment Log (D18). The consolidated review is AUTHORITATIVE; any per-task review is only a
//        localization map. Replaces the old per-task back-half (~113k tokens × N -> 1×).
//   4. Report — write the block-level report (per-task outcomes + escalations + breakdown assessment +
//        token roll-up + back-half verdict). It does NOT touch status/log (the back-half owns those).
//
//   block.verify (planning/harness.json) / --verify-depth:
//     consolidated         (default) — per task: implement + the D8 completeness self-check only.
//     consolidated+review            — per task: + one review pass (a localization map; non-gating).
//
// RESUMPTION
//   Re-run /sdlc-block <specSlug>. A task's implement commit on the integration branch (its
//   taskN-implement.md) marks it landed -> skipped on re-run. Escalated tasks are retried after you fix
//   the blocker (or edit execution-plan.json). The consolidated back-half re-runs once every task has
//   landed. Leftover worktrees from a parallel wave are reconciled by the Analyze resume-scout.
//
// NOTE — the consolidated back-half runs on the integration branch. When that branch is `main`, the
//   universal emoji gate (which diffs `main..HEAD`) has no diff base and no-ops; the per-stage gating
//   checks still run. Running the block on a dedicated integration branch off main restores emoji
//   gating end-to-end and is the documented follow-up (D23/D24 "Reconsider if").
//
// =============================================================================

export const meta = {
  name: 'sdlc-block',
  description: 'Orchestrate a full spec through dependency-ordered waves of parallel /sdlc-task pipelines, with bounded retries, failure triage, escalation, and ordered merges.',
  whenToUse: 'When driving a spec (a tasks.md) to completion across many parallel tasks. Optional task range, e.g. /sdlc-block <spec-slug> 1-7. Usage: /sdlc-block <spec-slug>',
  phases: [
    { title: 'Pre-flight', detail: 'Commit (or generate) the spec and guarantee a clean integration tree before any task runs', model: 'sonnet' },
    { title: 'Analyze',    detail: 'Resume-scout, load or generate the dependency-ordered execution plan, snapshot baselines once' },
    { title: 'Wave',       detail: 'Lean implement (+ optional review) per task — in-place for width-1 waves, worktrees only for width-≥2 — with retry + triage' },
    { title: 'Merge',      detail: 'Merge worktree branches in order for parallel waves (additive union only; else escalate)' },
    { title: 'Back-half',  detail: 'One consolidated test → review → fix → document → wrap-up over the integrated tree (via /sdlc-run --from test)' },
    { title: 'Report',     detail: 'Write the block-level report and surface escalations (status/log owned by the back-half)' },
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

// --verify-depth (D24): per-task verification depth for the lean runner. CLI overrides the project's
// planning/harness.json `block.verify`. Resolved AFTER the harness config loads (see verifyDepth below).
//   consolidated         (default) — per task: implement + the D8 completeness self-check only.
//   consolidated+review  — per task: implement → one review pass (a localization map for the single
//                          consolidated back-half). Both depths run exactly ONE consolidated back-half.
const VALID_VERIFY_DEPTHS = ['consolidated', 'consolidated+review']
const verifyDepthFlag = flagStr('--verify-depth')
if (verifyDepthFlag && !VALID_VERIFY_DEPTHS.includes(verifyDepthFlag)) {
  log(`ERROR: unknown --verify-depth "${verifyDepthFlag}". Valid values: ${VALID_VERIFY_DEPTHS.join(', ')}.`)
  return { error: 'Invalid --verify-depth', verifyDepthFlag, blockId }
}

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
const stateFile   = `${blockDir}/sdlc/sdlc-block-state.json`  // D28 cross-invocation resume breadcrumb (gitignored)

// D28 — in-engine per-task resume state, persisted to ${stateFile} after each task lands/escalates.
// Keyed by task number; values { status, commit, branch, worktreePath }. status ∈
// pending | merged | escalated | skipped. Seeded from a prior run's breadcrumb (if any) so a
// re-invoked block skips re-running landed tasks and re-triaging escalated limbo worktrees.
const taskState = {}

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

// D28 — block-state breadcrumb I/O schemas.
const STATE_LOAD_SCHEMA = {
  type: 'object',
  required: ['exists'],
  properties: {
    exists:    { type: 'boolean', description: 'true if a valid sdlc-block-state.json was read' },
    startedAt: { type: 'string',  description: 'the file\'s started_at value, or "" when absent' },
    tasks:     { type: 'object',  description: 'the per-task status map (taskNum -> {status,...}), or {} when absent', additionalProperties: true },
  },
}
const STATE_WRITE_SCHEMA = { type: 'object', required: ['written'], properties: { written: { type: 'boolean' } } }

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
        },
        block: {
          type: 'object',
          properties: {
            verify: { type: 'string', description: 'consolidated (default) | consolidated+review — lean runner per-task verification depth (D24)' }
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
    devServerCommand, readySignal, port, routes[]}); breakdown ({mode, complexityThreshold});
    block ({verify}). Preserve kind-specific fields verbatim; ignore any other fields.

Return your findings using the StructuredOutput tool.
`, { label: 'harness-config', schema: HARNESS_CONFIG_SCHEMA, model: 'sonnet' })

  if (!result || !result.present || !result.config) return null
  return result.config
}

// D23 — shared setup, once per block. Snapshot the pre-block baseline for each D6 baseline-diff check
// on the integration branch, BEFORE any task implements, so the consolidated back-half (run by
// /sdlc-run --from test) can diff the integrated tree against the pre-block state and fail only on
// net-new items. Block-level (no taskPrefix); resume-safe (an existing baseline is kept). No-op when
// no baseline-diff checks are configured. The engine ships no stack defaults — baselineCommand comes
// from harness.json. NOTE: the consolidated back-half consumes these baselines once /sdlc-run reaches
// D6 richer-check parity (the back-half-completeness follow-up); until then this is a harmless no-op
// for the common command-kind suites that the lean runner targets first.
async function snapshotBlockBaselines(cfg) {
  const checks = (cfg?.validation?.checks || []).filter(c => c.kind === 'baseline-diff' && c.baselineCommand)
  if (!checks.length) return
  const steps = checks.map(c => {
    const slug = (c.name || 'check').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')
    const path = `${reportsDir}/${slug}-baseline.json`
    return `Baseline "${c.name}" -> ${path}:
  mkdir -p ${reportsDir}
  [ -f ${path} ] && echo "BASELINE EXISTS (kept): ${path}" || { ${c.baselineCommand} > ${path} 2>/dev/null; echo "BASELINE WRITTEN: ${path}"; }`
  }).join('\n\n')
  await tracedAgent(`
You are the baseline-snapshot agent for the spec orchestration. You run from the MAIN repo root (the
integration branch). Capture the pre-block baseline for each baseline-diff validation check, ONCE,
BEFORE any task implements. Run each block exactly as written. Do NOT modify source. Existing baselines
are kept (resume-safe).

${steps}

After capturing all baselines, commit any newly written files so the integration tree stays clean
for the downstream merge step (an untracked baseline file blocks every worktree merge):
  git add ${reportsDir}/
  git commit -m "chore: sdlc baseline snapshot for ${blockId}" 2>/dev/null || echo "nothing to commit"

Return using StructuredOutput: done=true, and note which baselines were written vs already present.
`, { label: 'baseline-snapshot', schema: { type: 'object', required: ['done'], properties: { done: { type: 'boolean' }, notes: { type: 'string' } } }, phase: 'Analyze', model: 'haiku' })
}

// D28 — persist the per-task resume breadcrumb. Writes the WHOLE taskState map each time (preserving
// started_at), via a cheap Haiku agent since the workflow runtime has no filesystem access or
// Date.now(). Best-effort: a failed write logs a warning and NEVER aborts the run. The file is
// gitignored (runtime state, regenerated every run) so it does not churn commits or conflict under a
// parallel wave's merges. Mirrors D27's recordPhaseState pattern (sdlc-run), task-level instead of phase.
async function writeBlockState(phaseLabel = 'Analyze') {
  const tasksJson = JSON.stringify(taskState, null, 2)
  const r = await tracedAgent(`
You maintain the SDLC block's persistent task-state breadcrumb. Overwrite a single JSON file — do NOT
run any checks, edit code, or commit anything.

STEP 1 — current UTC timestamp + preserved start time:
  NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  cat ${stateFile} 2>/dev/null || echo "__NO_STATE__"
  If that file already exists and contains a "started_at" value, REUSE it verbatim for started_at
  below. Otherwise set started_at = the NOW value.

STEP 2 — ensure the directory exists:
  mkdir -p ${blockDir}/sdlc

STEP 3 — write ${stateFile} with EXACTLY this JSON (valid JSON only: double quotes, no trailing
commas, no markdown fences). Substitute <STARTED_AT> (preserved or NOW) and <NOW> from STEP 1:
{
  "spec_slug": ${JSON.stringify(blockId)},
  "started_at": "<STARTED_AT>",
  "updated_at": "<NOW>",
  "tasks": ${tasksJson}
}

Use the Write tool to write ${stateFile}. Then return via StructuredOutput (written=true on success).
`, { label: 'state-write', schema: STATE_WRITE_SCHEMA, phase: phaseLabel, model: 'haiku' })
  if (!r || !r.written) log('(state) could not persist block task-state — continuing')
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

STEP 4 — Validate task structure (CASE B and CASE C only — skip when you just generated a
  fresh spec in CASE A, whose format is already controlled):
  grep -c '^### [0-9]' ${tasksFile} 2>/dev/null || echo "0"
  If the count is "0" (no numbered '### N.' task headings found), override ready:
    ready=false, action="aborted",
    reason="tasks.md has no numbered '### N. Title' task headings — sdlc-block enumerates
    tasks by '### N.' and cannot parse this spec. Fix: run /generate-tasks --force to
    regenerate it, or add '### 1. Title', '### 2. Title', ... headings manually, commit,
    then re-run /sdlc-block."

STEP 4b — Validate spec CONTENT properties (CASE B and CASE C only; D19). A spec can be
  structurally valid (has '### N.' headings) yet substantively empty and waste pipeline tokens.
  Abort ONLY on the high-confidence thin-spec signals below. CRITICAL — avoid false-positive
  aborts: a blocked valid spec is far costlier than a missed thin one. Match ONLY the scaffold
  sentinels named here. Do NOT abort on a bare 'TODO'/'TBD' in authored prose, and do NOT treat
  any '<...>' as a placeholder (legitimate in generics like 'Vec<T>', prose like 'the <concept>
  folder', and globs). The Amendment Log's '_No amendments yet._' is the CORRECT resting state —
  never flag it.
  Run these greps and read the spec to judge:
    a) Unfilled scaffold tokens — grep -n '{{' ${tasksFile}  (the {{TOKEN}} form). Any hit → thin.
    b) Acceptance Criteria empty — the '## Acceptance Criteria' section has no '- ' bullet with
       real content (only blank, or only a literal seed like '- <Observable, checkable condition'
       copied verbatim from the template). → thin.
    c) Validation absent — the '## Validation Commands' block is empty/only template comments AND
       planning/harness.json does not exist. If
       harness.json exists, validation is satisfied regardless of this section. → thin only when both missing.
    d) A '### N.' task (other than the final Validate step) whose body names NO file, path, or
       concrete artifact to create/modify — i.e. you cannot tell what it touches. Judge by reading;
       flag only when genuinely contentless, not when a path is implied.
  If ANY of (a)-(d) holds, override ready:
    ready=false, action="aborted",
    reason="tasks.md is structurally valid but substantively thin: <name the specific failures,
    e.g. 'unfilled {{TOKEN}} on line N', 'Acceptance Criteria empty', 'task 3 names no files'>.
    Fix: flesh out the spec (run /generate-tasks --force to regenerate, or edit + commit), then
    re-run /sdlc-block."

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

// ----------------------------------------------------------------
// BLOCK STATE (D28): load the cross-invocation resume breadcrumb, if any. A prior run writes
// sdlc-block-state.json after each task lands/escalates; reading it here lets a re-invoked block skip
// re-running landed tasks and re-triaging escalated limbo worktrees. Absent/invalid file → first-run
// behavior (full Analyze from scratch). Best-effort: a load failure just means no resume hint.
// The state file AUGMENTS, never replaces, Analyze's committed-report resume scout (see D28 / D27): the
// dependency graph still comes from Analyze (or D22's execution-plan.json), this only short-circuits the
// merged/escalated classification that otherwise costs a triage wave.
// ----------------------------------------------------------------
const loadedState = await tracedAgent(`
You run from the MAIN repo root. Read the SDLC block resume breadcrumb if it exists — do NOT run any
checks, edit code, or commit anything.

  cat ${stateFile} 2>/dev/null || echo "__NO_STATE__"

If the output is "__NO_STATE__", or the file is empty or not valid JSON: return exists=false,
startedAt="", tasks={}.
Otherwise parse it as JSON and return exists=true, startedAt = its "started_at" value (verbatim),
tasks = its "tasks" object verbatim (the per-task status map, keyed by task number).

Return using StructuredOutput: exists, startedAt, tasks.
`, { label: 'load-state', schema: STATE_LOAD_SCHEMA, phase: 'Pre-flight', model: 'haiku' })

if (loadedState?.exists && loadedState.tasks && typeof loadedState.tasks === 'object') {
  for (const [num, st] of Object.entries(loadedState.tasks)) {
    if (st && typeof st === 'object' && st.status) taskState[num] = st
  }
  const pMerged = Object.entries(taskState).filter(([, v]) => v.status === 'merged').map(([k]) => k)
  const pEsc    = Object.entries(taskState).filter(([, v]) => v.status === 'escalated').map(([k]) => k)
  log(`Block state breadcrumb loaded: ${Object.keys(taskState).length} task(s) recorded${pMerged.length ? ` — merged: ${pMerged.join(', ')}` : ''}${pEsc.length ? ` | escalated: ${pEsc.join(', ')}` : ''}. Re-run will skip these.`)
} else {
  log('No block state breadcrumb (sdlc-block-state.json) — treating as a first run.')
}

// Load the project's policy once, up front (mechanism/policy split — see planning/harness.json).
// Reused by: the Analyze breakdown assessment (threshold), the breakdown gate below, and the
// post-merge Playwright sweep (uiTest). null when absent/invalid → breakdown defaults to 'recommend'.
const harnessCfg = await loadHarnessConfig()
const breakdownMode = harnessCfg?.breakdown?.mode || 'recommend'
const breakdownThreshold = harnessCfg?.breakdown?.complexityThreshold ?? 3

// Resolve the lean runner's per-task verification depth (D24): CLI --verify-depth wins, else
// planning/harness.json block.verify, else "consolidated" (the cheap default — per-task review off).
const verifyDepth = verifyDepthFlag || harnessCfg?.block?.verify || 'consolidated'
const perTaskReview = verifyDepth === 'consolidated+review'
log(`Verify depth: ${verifyDepth} (per-task review ${perTaskReview ? 'ON' : 'off'}; one consolidated back-half either way).`)

// D23 — shared setup, ONCE per block (not once per task). The harness config is already loaded above.
// Capture pre-block baselines for any D6 baseline-diff checks now, before any task implements, so the
// consolidated back-half can diff the integrated tree against the pre-block state and fail only on
// net-new items. No-op when no baseline-diff checks are configured (the common case). The block runs
// on the integration branch at the repo root, so the snapshot has no worktree prefix and no taskPrefix.
await snapshotBlockBaselines(harnessCfg)

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

STEP 1 — Check for an existing execution plan and VALIDATE it (D22). /generate-tasks may have already
  authored planning/${blockId}/sdlc/execution-plan.json (the dependency graph), so the common case can
  skip the expensive graph derivation in STEP 3. But a prompt authored it, so do NOT trust it — validate:
    cat ${planFile} 2>/dev/null && echo "PLAN_EXISTS" || echo "NO_PLAN"
    grep -n '^### [0-9]' ${tasksFile}   (the current task headings)
  Treat the plan as VALID only if ALL hold:
    (i)   it parses as JSON;
    (ii)  it has a "tasks" object (keyed by task number) and an "additiveFiles" array
          (the execution-plan.schema.json shape; a "waves" array is optional);
    (iii) its task set EXACTLY matches the current tasks.md '### N.' headings — same COUNT and same
          numbering. If tasks.md was edited after the plan was authored (a task added/removed/renumbered),
          the plan is STALE.
  If VALID: set planExists=true and return its "tasks" graph and "additiveFiles" (and "waves" if present)
    VERBATIM — do NOT re-derive the graph. Skip STEP 3 and STEP 3b; still do STEP 4 and STEP 5.
  If ABSENT, malformed, or STALE: set planExists=false and derive the graph yourself (STEP 2, 3, 3b). A
    stale plan loaded blindly would fan out the wrong waves — rejecting it is cheaper than the error.

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
  and would benefit from a /breakdown into atomic sub-steps. The real predictor of decomposition value
  is SEPARABLE STRUCTURE, not raw file count. Set recommendBreakdown=true (with a one-sentence
  breakdownReason naming the signal) when ANY of these hold:
    - it bundles multiple separable concerns (e.g. "implement X AND refactor Y AND add Z"); OR
    - it spans multiple layers/modules (e.g. data model + API + UI); OR
    - it carries a large acceptance-criteria set covering several INDEPENDENTLY-testable units; OR
    - it touches MORE than ${breakdownThreshold} distinct files (filesCreated + filesModified) AND
      those files are HETEROGENEOUS — different shapes/roles, or spanning more than one concern/layer
      above. File count is a CONTRIBUTING signal here, never a trigger on its own.
  HOMOGENEITY DISCOUNT — do NOT flag on file count alone when the many files are the SAME shape serving
    ONE concern (e.g. a content path's metadata file + N near-identical lesson/module pairs, or N
    parallel fixtures): decomposition yields little there. A single focused change over a small file set
    is also NOT a candidate (recommendBreakdown=false). This is ASSESSMENT ONLY — do not write any
    breakdown file; the orchestrator acts on these flags per policy.

STEP 4 — Resume scout: find tasks already LANDED on the integration branch (so a re-run does not
  re-implement them and duplicate commits). The lean runner marks a task landed by committing its
  per-task implement report — there is no per-task workflow.md anymore (one consolidated back-half
  writes the spec-level workflow.md at the end).
  ls ${reportsDir}/task*-implement.md ${reportsDir}/task*-workflow.md 2>/dev/null || echo "NONE"
  Add task N to doneTasks when EITHER is committed on the integration branch:
    - task${'${N}'}-implement.md exists (lean: its implement landed — skip re-implement), OR
    - task${'${N}'}-workflow.md exists with a PASS Final Verdict (a legacy full-pipeline run).
  (doneTasks here means "implement already landed"; the consolidated back-half still verifies the whole
  integrated tree at the end regardless.)

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
  waves:        ONLY if the loaded plan contained a "waves" array — return it verbatim. Omit it when the
                plan had none or you derived the graph fresh (the engine computes waves deterministically).
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

// D28 — augment Analyze's git-derived resume sets with the persisted block state (authoritative,
// ADDITIVE-only — it never removes a task Analyze found). A task a prior run recorded 'merged' is added
// to doneTasks (skip it); one recorded 'escalated' is forced to complete-unmerged-fail so the wave loop
// escalates it directly instead of re-deriving its limbo worktree through a triage wave (the dominant
// re-run cost in the F3 validation run). Committed reports remain the primary signal; this only fills
// the gap for limbo worktrees whose state is not yet on the integration branch.
for (const [num, st] of Object.entries(taskState)) {
  const n = Number(num)
  if (st.status === 'merged') {
    doneTasks.add(n)
  } else if (st.status === 'escalated' && !doneTasks.has(n)) {
    const prev = resumeByTask[n] || {}
    resumeByTask[n] = { num: n, state: 'complete-unmerged-fail', branchName: st.branch || prev.branchName || null, worktreePath: st.worktreePath || prev.worktreePath || null, verdict: st.verdict || prev.verdict || 'FAIL' }
  }
}

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
// D10 telemetry — capture the assessment outcome so the Report stage can PERSIST it. Previously the
// recommend-mode recommendation was log()-only: it streamed to the live run and then vanished, leaving
// no durable trace in block-workflow.md. `action`: off | none | recommend | auto. `committed`: the
// breakdown.md commit hash (or true/false) when action==='auto'.
const breakdownAssessment = { mode: breakdownMode, threshold: breakdownThreshold, flagged: [], action: 'off', committed: null }
if (breakdownMode !== 'off') {
  const flagged = Object.values(taskMap).filter(t =>
    t.recommendBreakdown && !doneTasks.has(t.num) && (!selectedTasks || selectedTasks.has(t.num)))
  breakdownAssessment.flagged = flagged.map(t => ({ num: t.num, title: t.title, reason: t.breakdownReason || 'coarse' }))
  if (!flagged.length) {
    breakdownAssessment.action = 'none'
    log(`Breakdown: no tasks flagged as coarse (mode=${breakdownMode}).`)
  } else if (breakdownMode === 'auto') {
    breakdownAssessment.action = 'auto'
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
    breakdownAssessment.committed = gen?.success ? (gen.commitHash || true) : false
    if (gen?.success) log(`Breakdown committed${gen.commitHash ? ` (${gen.commitHash})` : ''} — worktrees will inherit it.`)
    else log(`Breakdown generation did not complete — tasks will implement from tasks.md only.`)
  } else {
    // recommend mode — surface the recommendation; proceed without writing anything.
    breakdownAssessment.action = 'recommend'
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

// ----------------------------------------------------------------
// Per-task schemas for the lean in-place path (D23). The implement agent mirrors sdlc-task's implement
// stage (incl. the D8 completeness self-check) — INLINED here, not reused, because honoring D23's
// "shared setup once" requires running implement as a direct agent() call that shares this block's
// already-loaded harness config rather than a sub-workflow that re-runs its own setup per task. The
// optional per-task review mirrors sdlc-task's review and is a LOCALIZATION MAP only — never a gate.
// ----------------------------------------------------------------
const INPLACE_STAGE_SCHEMA = {
  type: 'object',
  required: ['reportFile', 'success'],
  properties: {
    reportFile:    { type: 'string' },
    success:       { type: 'boolean' },
    filesModified: { type: 'array', items: { type: 'string' } },
    commitHash:    { type: 'string' },
    notes:         { type: 'string' }
  }
}
const INPLACE_REVIEW_SCHEMA = {
  type: 'object',
  required: ['reportFile', 'verdict'],
  properties: {
    reportFile:     { type: 'string' },
    verdict:        { type: 'string', enum: ['PASS', 'FAIL', 'PARTIAL'] },
    failureReasons: { type: 'array', items: { type: 'string' } },
    unmetCriteria:  { type: 'array', items: { type: 'string' } },
    notes:          { type: 'string' }
  }
}

// Roll the integration branch back to a captured pre-task SHA (D23 in-place failure recovery). A
// worktree gave throwaway isolation; in-place does not, so a half-applied failed task must be undone
// before the next in-place task (or a clean-slate retry) runs on the shared branch.
async function rollbackInPlace(preSha, taskNum) {
  if (!preSha) { log(`Task ${taskNum}: no pre-task SHA captured — cannot auto-rollback; inspect the integration branch.`); return }
  log(`Task ${taskNum}: rolling the integration branch back to ${preSha} (pre-task state)...`)
  await tracedAgent(`
You run from the MAIN repo root (the integration branch). A lean in-place task failed and left the
branch in a half-applied state. Reset it to the captured pre-task commit so the next task starts clean.
Run exactly:
  git reset --hard ${preSha}
  git clean -fd
  git log --oneline -1
Return the final HEAD line as plain text.
`, { label: `rollback-${taskNum}`, phase: 'Analyze', model: 'haiku' })
}

// Triage a per-task failure into RETRYABLE (clean-slate retry) vs MAJOR (escalate). Shared by both
// the in-place and worktree paths. `resultBlob` is a JSON summary of what the attempt produced.
async function triageFailure(taskNum, attempt, resultBlob) {
  return tracedAgent(`
You are the failure-triage agent for a spec orchestration. A single task did NOT pass. The lean block
runs each task's implement (plus the D8 completeness self-check, and optionally one review pass) before
returning, so a genuine, repeated structural failure is unlikely to be fixed by another clean-slate run.

Task: ${blockId} task ${taskNum}, attempt ${attempt} of ${MAX_TASK_ATTEMPTS}.
Attempt result:
${resultBlob}

Classify:
  RETRYABLE — transient/infra (NULL_RESULT, agent died, NOT_REACHED), OR the failure CHANGED from the
              previous attempt (it is making progress).
  MAJOR     — the SAME failure again, OR it is structural (references a missing upstream symbol/
              dependency, wrong wave ordering, or otherwise needs a human to re-plan or fix the
              blocker). When unsure, prefer MAJOR — escalation is cheap, a wasted retry is not.

Return using StructuredOutput: class, reason, sameCriteriaAsBefore.
`, { label: `triage-${taskNum}-${attempt}`, schema: TRIAGE_SCHEMA, phase: 'Analyze', model: 'sonnet' })
}

// D23 — run ONE task IN-PLACE on the integration branch: no worktree, no merge. Implement (+ the D8
// self-check, always) and, when block.verify is consolidated+review, one review pass (a localization
// map — NOT a gate; the consolidated back-half is authoritative). The implement's own success (D8
// gate) decides whether the task's work stays on the branch; a failed implement is rolled back to the
// captured pre-task SHA before any retry/escalation.
async function runTaskInPlace(taskNum, waveLabel) {
  const implementReport = `${reportsDir}/task${taskNum}-implement.md`
  const reviewReport    = `${reportsDir}/task${taskNum}-review.md`
  const stem = `${blockId}-task${taskNum}`

  // Capture the pre-task HEAD so a failed in-place attempt can be rolled back (D23).
  const snap = await tracedAgent(`
You run from the MAIN repo root (the integration branch). Print the current HEAD short SHA so a failed
in-place task can be rolled back to this exact state. Run: git rev-parse --short HEAD
Return using StructuredOutput: sha=<the short hash>.
`, { label: `snap-${taskNum}`, schema: { type: 'object', required: ['sha'], properties: { sha: { type: 'string' } } }, phase: waveLabel, model: 'haiku' })
  const preSha = snap?.sha || null

  let prevReason = null
  for (let attempt = 1; attempt <= MAX_TASK_ATTEMPTS; attempt++) {
    log(`Task ${taskNum}: in-place implement attempt ${attempt}/${MAX_TASK_ATTEMPTS} on the integration branch...`)

    const impl = await tracedAgent(`
You are the implementation agent for the lean spec orchestration. You run IN PLACE on the MAIN repo
root (the integration branch) — there is NO worktree. Implement ONLY Task ${taskNum} of this spec.

Target:
  Spec:            ${blockId}
  Task:            Task ${taskNum} only
  Spec file:       ${tasksFile}
  Report to write: ${implementReport}

1. Read CLAUDE.md — internalize all standing rules (it is the authority; assume no stack/locale/
   narrative/content rule unless written there). Universal: no fabricated metrics or quotes, no emoji,
   every change ships with the tests that prove it.
   Run: cat CLAUDE.md

2. Read the spec, focusing on the "### ${taskNum}." section:
   Run: cat ${tasksFile}
   Implement ONLY "### ${taskNum}." — do NOT implement other tasks.

2.5. Optional breakdown (more granular sub-steps from /breakdown):
   Run: ls ${breakdownFile} 2>/dev/null && echo "BREAKDOWN_EXISTS" || echo "NO_BREAKDOWN"
   If BREAKDOWN_EXISTS: read ${breakdownFile}, find "### Step ${taskNum}:", and use its atomic sub-steps
   as the execution guide (run each inline "Verify:" checkpoint). tasks.md stays authoritative for
   scope/acceptance criteria; breakdown.md is authoritative for HOW.

3. Execute each step methodically with Read, Edit, Write, Bash. All paths resolve from the repo root.

4. Follow every CLAUDE.md standing rule; add/update tests for new code/logic; verify any model ids /
   package names via the claude-api skill — never from memory.

5. Run the spec's Validation Commands (planning/harness.json, or the spec's "## Validation Commands")
   to confirm correctness:  <each validation command> 2>&1 | tail -20

5.5. SELF-CHECK — completeness gate (BEFORE writing the report or committing). Re-read the in-scope
   "## Acceptance Criteria" for Task ${taskNum}. For EACH, open the actual file(s) and confirm it is
   FULLY satisfied — do not assume from memory:
   (a) NO placeholder/stub bodies on a required code path — \`todo!()\`/\`unimplemented!()\`/
       \`unreachable!()\`, \`raise NotImplementedError\`, \`throw new Error('not implemented')\`,
       empty \`pass\`-only bodies, or \`TODO\`/\`FIXME\` in required paths. Sanity-grep ONLY the files
       the in-scope criteria require (build that path list from the criteria), NOT every changed file:
         grep -nE 'todo!\\(|unimplemented!\\(|unreachable!\\(|NotImplementedError|not implemented|FIXME' <those paths> 2>/dev/null
       A stub in a file no in-scope criterion requires is OUT OF SCOPE — leave it for its owning task.
   (b) EVERY deliverable file a criterion names exists at the stated path — \`ls\` it.
   (c) EVERY "unit-tested"/"covered by a test" criterion has a real, hermetic test exercising that path.
   If ANY criterion is not fully met, FIX IT NOW and re-run step 5 before proceeding. Do NOT return
   success:true with a known gap.

6. Write the implementation report to ${implementReport}:

   # Implementation Report — ${stem}

   **Date:** [run: date +%Y-%m-%d]
   **Plan:** ${tasksFile}
   **Scope:** Task ${taskNum}

   ## What Was Built or Changed
   - [bullets with file paths]

   ## Files Created or Modified
   | File | Action |
   |---|---|
   | path/to/file | created / modified |

   ## Validation Output
   **Result:** PASSED / FAILED
   On FAIL only, paste the failing command's last 20 lines; on PASS do NOT paste stdout (the
   consolidated Test stage is the authoritative full-output capture).

   ## Decisions and Trade-offs
   [non-obvious choices]

   ## Follow-up Work
   [anything deferred]

7. Commit your changes on the integration branch. Never use git add -A or git add . — stage by name:
   git status
   git add <each changed source/test file> ${implementReport}
   git commit -m "$(cat <<'EOF'
feat: implement ${stem}
EOF
)"
   git log --oneline -1   (capture the short hash)

Return using StructuredOutput:
  reportFile: "${implementReport}"
  success: true if implementation completed AND the self-check (5.5) passed with no known gap
  filesModified: array of source files created or modified
  commitHash: the 7-char short hash (empty string if commit failed)
  notes: one-line summary
`, { label: `implement-${taskNum}`, schema: INPLACE_STAGE_SCHEMA, phase: waveLabel, model: 'sonnet' })

    if (!impl || !impl.success) {
      // Implement failed (or the D8 self-check found a gap) — roll the branch back, then triage.
      const why = impl?.notes || (impl ? 'implement reported a completeness gap' : 'implement agent returned null')
      log(`Task ${taskNum}: in-place implement did NOT pass — ${why}`)
      if (preSha) await rollbackInPlace(preSha, taskNum)
      const triage = await triageFailure(taskNum, attempt, JSON.stringify({ stage: 'implement', success: false, notes: why, previousReason: prevReason }, null, 2))
      if (!triage || triage.class === 'MAJOR' || attempt === MAX_TASK_ATTEMPTS) {
        const reason = triage?.reason || (attempt === MAX_TASK_ATTEMPTS ? 'retries exhausted' : 'triage returned null')
        log(`Task ${taskNum}: ESCALATE — ${reason}.`)
        return { taskNum, status: 'escalate', inPlace: true, finalVerdict: 'FAIL', reasons: [why], attempts: attempt, triage: reason }
      }
      log(`Task ${taskNum}: RETRYABLE — ${triage.reason}. Clean-slate retry (branch rolled back).`)
      prevReason = why
      continue
    }

    // Implement passed the D8 gate → its work stays on the integration branch. Optionally run ONE
    // review pass as a localization map (consolidated+review). The review NEVER gates the merge — the
    // consolidated back-half is authoritative — so a PARTIAL/FAIL here only annotates the outcome.
    let finalVerdict = 'IMPLEMENTED'
    let reviewReasons = []
    if (perTaskReview) {
      log(`Task ${taskNum}: per-task review pass (localization map)...`)
      const rev = await tracedAgent(`
You are the review agent for the lean spec orchestration. You run IN PLACE on the MAIN repo root (the
integration branch). Produce a LOCALIZATION review of Task ${taskNum} ONLY — your verdict does NOT gate
anything (the consolidated back-half over the integrated tree is authoritative); it is a map for the
later consolidated fix. Do NOT modify source.

Target:
  Spec:             ${blockId}
  Task:             Task ${taskNum}
  Spec file:        ${tasksFile}
  Implement report: ${implementReport}
  Report to write:  ${reviewReport}

1. Read the spec's "## Acceptance Criteria" (cat ${tasksFile}); identify the criteria in scope for
   Task ${taskNum} (criteria clearly tagged for other tasks → SKIP, do not let them affect the verdict).
2. For each in-scope criterion, read the relevant source and mark MET / PARTIAL / NOT_MET.
3. Re-run the GATING validation checks (gates:true in planning/harness.json, or the spec's Validation
   Commands) and note results — but remember a failure here is informational at this stage.
4. Verdict: PASS (all in-scope MET) / PARTIAL / FAIL (any NOT_MET).
5. Write ${reviewReport}:

   # Review Report — ${stem}

   **Date:** [run: date +%Y-%m-%d]
   **Spec:** ${tasksFile}
   **Scope:** Task ${taskNum} (per-task localization — NON-gating)
   **Verdict:** PASS / PARTIAL / FAIL

   ## Acceptance Criteria Check
   | Criterion | Status | Evidence |
   |---|---|---|

   ## Issues Found
   [specific problems — empty if PASS]

   Commit it:
   git add ${reviewReport} && git commit -m "docs: per-task review for ${stem}" || echo "nothing to commit"

Return using StructuredOutput: reportFile="${reviewReport}", verdict, failureReasons, unmetCriteria, notes.
`, { label: `review-${taskNum}`, schema: INPLACE_REVIEW_SCHEMA, phase: waveLabel, model: 'sonnet' })
      finalVerdict = rev?.verdict || 'IMPLEMENTED'
      reviewReasons = (rev?.unmetCriteria || []).concat(rev?.failureReasons || [])
      log(`Task ${taskNum}: per-task review verdict ${finalVerdict} (non-gating; consolidated back-half decides).`)
    }

    return { taskNum, status: 'pass', inPlace: true, finalVerdict, commitHash: impl.commitHash, reviewReport: perTaskReview ? reviewReport : null, reasons: reviewReasons, attempts: attempt }
  }
  return { taskNum, status: 'escalate', inPlace: true, finalVerdict: 'NOT_REACHED', reasons: [], attempts: MAX_TASK_ATTEMPTS }
}

// D23 — run ONE task in an isolated WORKTREE (used only for width-≥2 waves: genuine concurrent
// writers). Delegates to /sdlc-task --implement-only (+ --review for consolidated+review), which runs
// worktree-setup → implement (+ optional localization review) and STOPS — no per-task test/document/
// wrap-up. The block merges the branch and runs ONE consolidated back-half. "Mergeable" = the implement
// succeeded (finalVerdict !== 'FAIL'); a per-task review PARTIAL/FAIL is advisory and does not block.
async function runTaskWorktree(taskNum, resume = false, parallelWave = false) {
  let prevReasons = null
  for (let attempt = 1; attempt <= MAX_TASK_ATTEMPTS; attempt++) {
    const resumeArg = (resume && attempt === 1) ? ' --resume' : ''
    const parallelArg = parallelWave ? ' --parallel-wave' : ''
    const reviewArg = perTaskReview ? ' --review' : ''
    log(`Task ${taskNum}: worktree implement-only attempt ${attempt}/${MAX_TASK_ATTEMPTS} via /sdlc-task${resumeArg}...`)
    const r = await workflow('sdlc-task', `${blockId} ${taskNum} --implement-only${reviewArg}${resumeArg} --under-block${parallelArg}`)

    // Mergeable when implement succeeded. With --review the verdict may be PASS/PARTIAL/FAIL; only a
    // FAIL (or a null result) blocks the merge — PARTIAL is an advisory localization signal.
    if (r && r.finalVerdict && r.finalVerdict !== 'FAIL') {
      return { taskNum, status: 'pass', branchName: r.branchName, worktreePath: r.worktreePath, finalVerdict: r.finalVerdict, reviewReport: r.reviewReport, attempts: attempt }
    }

    const failBlob = JSON.stringify({ finalVerdict: r?.finalVerdict ?? 'NULL_RESULT', reviewVerdict: r?.reviewVerdict, previousFailureReasons: prevReasons }, null, 2)
    const triage = await triageFailure(taskNum, attempt, failBlob)
    const reasons = r?.reviewVerdict ? [`per-task review: ${r.reviewVerdict}`] : []

    if (!triage || triage.class === 'MAJOR' || attempt === MAX_TASK_ATTEMPTS) {
      const why = triage?.reason || (attempt === MAX_TASK_ATTEMPTS ? 'retries exhausted' : 'triage returned null')
      log(`Task ${taskNum}: ESCALATE — ${why}. Worktree preserved for inspection.`)
      return {
        taskNum, status: 'escalate',
        branchName: r?.branchName, worktreePath: r?.worktreePath,
        finalVerdict: r?.finalVerdict || 'NOT_REACHED',
        reasons, reviewReport: r?.reviewReport || `${reportsDir}/task${taskNum}-review.md`,
        attempts: attempt, triage: triage?.reason
      }
    }
    log(`Task ${taskNum}: RETRYABLE — ${triage.reason}. Clean-slate retry.`)
    if (r?.branchName && r?.worktreePath) await teardownBranch(r.branchName, r.worktreePath)
    prevReasons = reasons
  }
  return { taskNum, status: 'escalate', finalVerdict: 'NOT_REACHED', reasons: [], attempts: MAX_TASK_ATTEMPTS }
}

// D28 — seed the task-state map so the breadcrumb reflects the full plan, then persist the initial
// snapshot. Tasks Analyze found already done (committed reports) but the breadcrumb didn't record are
// marked merged; everything else not yet recorded is pending. An early crash now still leaves a
// readable, accurate state file.
for (const t of doneTasks) {
  if (!taskState[t] || taskState[t].status === 'pending') taskState[t] = { status: 'merged', commit: taskState[t]?.commit || null, branch: null, worktreePath: null }
}
for (const t of Object.keys(taskMap)) {
  if (!taskState[t]) taskState[t] = { status: 'pending', commit: null, branch: null, worktreePath: null }
}
await writeBlockState('Analyze')

// ================================================================
// WAVE LOOP
// ================================================================
for (let wi = 0; wi < waves.length; wi++) {
  const wave = waves[wi]
  const waveLabel = `Wave ${wi + 1}`
  phase(waveLabel)

  // Budget guard: if a token target is set and the remaining can't cover a wave, stop and report.
  // Lean per-task cost is implement (+ optional review) only — far below the old full per-task pipeline.
  // The consolidated back-half is charged once, after the wave loop.
  if (budget.total) {
    const estPerTask = perTaskReview ? 90_000 : 55_000
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
      taskState[t] = { status: 'skipped', commit: null, branch: null, worktreePath: null }  // D28
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
      taskState[t] = { status: 'escalated', commit: null, branch: rs.branchName || null, worktreePath: rs.worktreePath || null, verdict: rs.verdict || 'FAIL' }  // D28
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

  // D23 — isolation only for genuine concurrent writers. A wave with a SINGLE runnable task runs it
  // IN PLACE on the integration branch (no worktree, no merge — the common lean path); a wave with ≥2
  // runnable tasks uses worktrees + ordered merge (true parallelism). Each per-task stage is lean:
  // implement (+ optional localization review) only — the single consolidated back-half does the rest.
  let waveOutcomes = []
  const useWorktrees = runnable.length >= 2
  if (useWorktrees) {
    log(`${waveLabel}: ${runnable.length} concurrent tasks — isolating in worktrees, then merging in order.`)
    for (let k = 0; k < runnable.length; k += MAX_WAVE_WIDTH) {
      const batch = runnable.slice(k, k + MAX_WAVE_WIDTH)
      if (batch.length < runnable.length) log(`${waveLabel}: batch [${batch.join(', ')}]`)
      // batch.length > 1 → concurrent siblings share the budget pool → per-task outTok is contaminated (D12).
      const batchResults = await parallel(batch.map(t => () => runTaskWorktree(t, resumeTasks.has(t), batch.length > 1)))
      waveOutcomes.push(...batchResults.filter(Boolean))
    }
  } else {
    for (const t of runnable) waveOutcomes.push(await runTaskInPlace(t, waveLabel))
  }

  outcomes.push(...waveOutcomes)
  for (const o of waveOutcomes) {
    if (o.status === 'pass') {
      // In-place tasks are already committed on the integration branch — nothing to merge. Worktree
      // tasks queue their branch for the ordered merge below (recorded after a successful merge).
      if (o.inPlace) {
        o.merged = true
        taskState[o.taskNum] = { status: 'merged', commit: o.commitHash || null, branch: null, worktreePath: null }  // D28: in-place lands directly on the integration branch
      } else passedBranches.push({ taskNum: o.taskNum, branchName: o.branchName, worktreePath: o.worktreePath })
    } else if (o.status === 'escalate') {
      badSet.add(o.taskNum)
      taskState[o.taskNum] = { status: 'escalated', commit: null, branch: o.branchName || null, worktreePath: o.worktreePath || null, verdict: o.finalVerdict || 'FAIL' }  // D28
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
        taskState[p.taskNum] = { status: 'escalated', commit: null, branch: p.branchName || null, worktreePath: p.worktreePath || null, verdict: 'FAIL' }  // D28
        // reflect the merge failure in the outcome
        const o = outcomes.find(x => x.taskNum === p.taskNum)
        if (o) { o.status = 'escalate'; o.reasons = [...(o.reasons || []), `merge conflict: ${(m?.conflictedFiles || []).join(', ') || note}`] }
      } else {
        log(`Task ${p.taskNum}: merged (${m.strategy}) ${m.commitHash || ''}`)
        taskState[p.taskNum] = { status: 'merged', commit: m.commitHash || null, branch: null, worktreePath: null }  // D28
        const o = outcomes.find(x => x.taskNum === p.taskNum)
        if (o) { o.merged = true; o.commitHash = m.commitHash; o.mergeStrategy = m.strategy }
      }
    }
  }

  // D28 — persist the breadcrumb once per wave, after this wave's tasks have landed/escalated. Width-1
  // (in-place) waves run one task, so this is per-task there; width-≥2 waves record every landing after
  // their ordered merge. An interrupted re-run reads this and skips straight to the pending tasks.
  await writeBlockState(waveLabel)

  if (budget.total) log(`Budget: ~${Math.round(budget.remaining() / 1000)}k tokens remaining.`)
}

// ================================================================
// CONSOLIDATED BACK-HALF (D24) — ONE test → review → fix → (ui-test) → document → wrap-up over the
// integrated tree, via /sdlc-run --from test. Replaces the old per-task back-half (~113k × N → 1×):
// each task already ran its lean implement (+ optional localization review); now the integrated result
// is verified once, with the consolidated fix localized by the per-task implement reports.
//
// Runs ONLY when the block is COMPLETE this run (no escalated/skipped tasks). Escalations block
// completion exactly as before — fix the blocker and re-run /sdlc-block; the back-half fires once
// everything has landed. status.md / log.md / the spec Amendment Log (D18) are owned by the back-half's
// wrap-up (it reads the seeded consolidated implement report), so the block's own Report does NOT touch
// them — it writes only the block-level report (telemetry + breakdown + escalations + back-half link).
// ================================================================
const merged    = outcomes.filter(o => o.merged)
const escalated = outcomes.filter(o => o.status === 'escalate')
const skipped   = outcomes.filter(o => o.status === 'skipped')
const mergedTaskNums = merged.map(o => o.taskNum).sort((a, b) => a - b)
// Landed = implemented on the integration branch this run (merged) OR already done from a prior run
// (doneTasks). The seed + back-half localization should cover ALL landed tasks, not just this run's.
const landedTaskNums = [...new Set([...mergedTaskNums, ...doneTasks])].sort((a, b) => a - b)
const blockComplete = escalated.length === 0 && skipped.length === 0

let backHalf = null
let backHalfVerdict = 'NOT_RUN'
if (blockComplete) {
  phase('Back-half')

  // Seed a spec-level implement report from the per-task implement reports, so the consolidated
  // review/document can localize each finding to the task that produced it (D24). sdlc-run --from test
  // reads the no-prefix implement.md slot in the same reports dir.
  if (landedTaskNums.length) {
    log('Seeding the consolidated implement report from the per-task reports...')
    await tracedAgent(`
You run from the MAIN repo root (the integration branch). Build ONE consolidated implement report from
the per-task implement reports so the back-half's review/document stages can localize each finding to
the task that produced it. Do NOT modify source.

1. For each task N in ${JSON.stringify(landedTaskNums)}: cat ${reportsDir}/task${'${N}'}-implement.md
   (skip any that are absent).
2. Write ${reportsDir}/implement.md:

   # Implementation Report — ${blockId} (consolidated)

   **Date:** [run: date +%Y-%m-%d]
   **Plan:** ${tasksFile}
   **Scope:** Full spec — per-task implement reports merged by /sdlc-block

   ## Files Created or Modified
   | File | Action | Task |
   |---|---|---|
   [UNION every per-task report's "Files Created or Modified" rows, tagging each with its task N]

   ## Per-Task Summaries
   - Task N: [one line from that report's "What Was Built or Changed"]

3. Commit: git add ${reportsDir}/implement.md && git commit -m "chore: consolidated implement report for ${blockId}" || echo "nothing to commit"

Return using StructuredOutput: reportFile="${reportsDir}/implement.md", success=true.
`, { label: 'seed-implement', schema: { type: 'object', required: ['success'], properties: { reportFile: { type: 'string' }, success: { type: 'boolean' } } }, phase: 'Back-half', model: 'haiku' })
  }

  log('Running the consolidated back-half via /sdlc-run --from test (test → review → fix → document → wrap-up) over the integrated tree...')
  backHalf = await workflow('sdlc-run', `${blockId} --from test`)
  backHalfVerdict = backHalf?.finalVerdict || 'FAIL'
  log(`Consolidated back-half verdict: ${backHalfVerdict} (review attempts: ${backHalf?.reviewAttempts ?? '—'}). status.md/log.md/spec Amendment Log updated by its wrap-up.`)
} else {
  log(`Back-half SKIPPED — ${escalated.length} escalated / ${skipped.length} skipped task(s) block completion.`)
  log(`Fix the blocker(s) and re-run /sdlc-block ${blockId}; the consolidated back-half fires once everything has landed.`)
}

// ================================================================
// REPORT — slim block-level report. The consolidated back-half's wrap-up owns status.md / log.md /
// the spec Amendment Log; this report records ONLY block-orchestration facts (per-task outcomes,
// escalations, breakdown assessment, token roll-up, back-half verdict) and commits block-workflow.md.
// ================================================================
phase('Report')

const overall = !blockComplete
  ? (merged.length > 0 ? 'PARTIAL' : 'BLOCKED')
  : (backHalfVerdict === 'PASS' ? 'PASS' : backHalfVerdict === 'PARTIAL' ? 'PARTIAL' : 'BLOCKED')

const outcomeTable = outcomes
  .sort((a, b) => a.taskNum - b.taskNum)
  .map(o => `| ${o.taskNum} | ${o.merged ? (o.inPlace ? 'in-place' : 'merged') : o.status} | ${o.finalVerdict || '—'} | ${o.mergeStrategy || (o.inPlace ? 'in-place' : '—')} | ${o.commitHash || '—'} | ${(o.reasons || []).join('; ').substring(0, 80) || '—'} |`)
  .join('\n')

const escalationBlock = escalated.length
  ? escalated.map(o => `- **Task ${o.taskNum}** — verdict ${o.finalVerdict}. ${o.triage || ''}\n    - Review: \`${o.reviewReport || `${reportsDir}/task${o.taskNum}-review.md`}\`\n    - ${o.inPlace ? 'Ran in-place (rolled back on failure) — no worktree.' : `Worktree (preserved): \`${o.worktreePath || 'n/a'}\` (branch \`${o.branchName || 'n/a'}\`)`}\n    - Reasons: ${(o.reasons || []).join('; ') || 'see review report'}`).join('\n')
  : '_None._'

// Token-telemetry roll-up — this engine's own orchestration agents only (the consolidated back-half's
// per-stage detail lives in the spec's own workflow.md, written by /sdlc-run's wrap-up).
const blockMetricsTable = metrics.map(m => {
  const out = m.outTok != null ? String(m.outTok) : '—'
  return `| ${m.label} | ${m.model} | ${m.promptTokEst} | ${out} |`
}).join('\n')
const totalOut = metrics.reduce((s, m) => s + (m.outTok || 0), 0)
const worstByPrompt = [...metrics].sort((a, b) => b.promptTokEst - a.promptTokEst).slice(0, 3)

// D10 — Breakdown assessment summary, built deterministically so the Report agent appends it verbatim.
const breakdownSection = (() => {
  const ba = breakdownAssessment
  if (ba.mode === 'off') return `_Skipped — \`breakdown.mode\` is "off" in planning/harness.json._`
  if (!ba.flagged.length) return `**Mode:** ${ba.mode} · **threshold:** >${ba.threshold} files. No tasks flagged as coarse.`
  const rows = ba.flagged.map(f => `| ${f.num} | ${f.title || '—'} | ${f.reason} |`).join('\n')
  const action = ba.action === 'auto'
    ? (ba.committed ? `Auto mode — generated and committed \`breakdown.md\`${typeof ba.committed === 'string' ? ` (${ba.committed})` : ''} on main before the waves; every worktree inherited it.`
                    : `Auto mode — breakdown generation did NOT complete; tasks implemented from tasks.md only.`)
    : `Recommend mode — no file written. Consider running \`/breakdown ${tasksFile}\` before this block, or set \`breakdown.mode:"auto"\` in planning/harness.json.`
  return `**Mode:** ${ba.mode} · **threshold:** >${ba.threshold} files · **${ba.flagged.length} task(s) flagged coarse.**

| Task | Title | Coarseness signal |
|---|---|---|
${rows}

**Action taken:** ${action}`
})()

const backHalfSection = blockComplete
  ? `**Verdict:** ${backHalfVerdict} · **review attempts:** ${backHalf?.reviewAttempts ?? '—'} · **depth:** ${verifyDepth}
Per-stage detail (test / review / fix / document / wrap-up over the integrated tree) is in the spec's
own workflow report: \`${backHalf?.workflowReport || `${reportsDir}/workflow.md`}\`. Its wrap-up updated
status.md, log.md, and the spec Amendment Log (D18).`
  : `_Not run — ${escalated.length} escalated / ${skipped.length} skipped task(s) block completion. Resolve them and re-run \`/sdlc-block ${blockId}\`; the back-half runs once everything lands._`

const reportResult = await tracedAgent(`
You are the block-report agent for the lean spec orchestration. You run from the MAIN repo root. Write
the block-level report and commit it. Do NOT touch planning/status.md or log.md — the consolidated
back-half's wrap-up already owns those.

Spec: ${blockId}
Overall verdict: ${overall}
Landed tasks: ${JSON.stringify(mergedTaskNums)}
Escalated tasks: ${JSON.stringify(escalated.map(o => o.taskNum))}
Skipped (blocked by upstream): ${JSON.stringify(skipped.map(o => o.taskNum))}
Consolidated back-half verdict: ${backHalfVerdict}

DO THIS, IN ORDER:

1. Write the block report to ${blockReport}:

   # Spec Orchestration Report — ${blockId}

   **Date:** [run: date +%Y-%m-%d]
   **Overall verdict:** ${overall}
   **Verify depth:** ${verifyDepth}
   **Tasks landed:** ${mergedTaskNums.length}  |  **Escalated:** ${escalated.length}  |  **Skipped:** ${skipped.length}  |  **Back-half:** ${backHalfVerdict}

   ## Outcome by Task
   (Result "in-place" = implemented directly on the integration branch; "merged" = implemented in a
   worktree for a parallel wave, then merged. Per-task Verdict is IMPLEMENTED, or the localization
   review verdict under consolidated+review — NON-gating; the back-half verdict is authoritative.)
   | Task | Result | Verdict | Path | Commit | Notes |
   |---|---|---|---|---|---|
${outcomeTable}

   ## Consolidated Back-half (D24)
${backHalfSection}

   ## Escalations (need your attention)
${escalationBlock}

   ## Resume
   After fixing any blocker (or editing ${planFile}), re-run:  /sdlc-block ${blockId}
   Landed tasks are detected (their implement commit is on the integration branch) and skipped;
   escalated tasks are retried. The consolidated back-half re-runs once every task has landed.

2. Append the breakdown assessment, then the orchestrator token roll-up, to ${blockReport}. Run BOTH
   appends EXACTLY as written (literal heredocs) — do NOT retype, summarize, reorder, or omit them;
   these are machine-generated. First the breakdown assessment:
   cat >> ${blockReport} <<'BREAKDOWN_EOF'

## Breakdown Assessment (D10)
${breakdownSection}
BREAKDOWN_EOF

   Then the token roll-up:
   cat >> ${blockReport} <<'ROLLUP_EOF'

## Token Roll-up (orchestrator stages)
Attribution for THIS engine's own agents (preflight / analyze / per-task implement+review / merge /
triage / seed / report). The consolidated back-half's per-stage detail lives in the spec's workflow.md.
promptTok = injected input estimate; outTok = output-token delta ("—" when no +Nk budget target was
set). NOTE: per-task outTok for tasks that ran in a PARALLEL (width-≥2) wave is shared-pool-contaminated
and reported as a misleading number — width-1 in-place tasks run sequentially, so theirs is clean (D12).

**Total orchestrator outTok:** ${totalOut || '—'}

| Stage | Model | promptTok | outTok |
|---|---|---|---|
${blockMetricsTable}
ROLLUP_EOF

3. Commit ONLY the block report (NOT status.md or log.md — the back-half owns those):
   git add ${blockReport}
   git commit -m "chore: block orchestration report for ${blockId}" || echo "nothing to commit"
   git log --oneline -1

Return using StructuredOutput: reportFile="${blockReport}", overallVerdict="${overall}",
statusUpdated=false (the back-half wrap-up owns status.md), nextFocus="", notes.
`, { label: 'report', schema: REPORT_SCHEMA, phase: 'Report', model: 'sonnet' })

// ----------------------------------------------------------------
// Console echo of the roll-up (also persisted to the block report by the Report agent).
// ----------------------------------------------------------------
log(`Token roll-up (orchestrator stages): total outTok=${totalOut || '—'} | worst 3 by injected prompt: ${worstByPrompt.map(m => `${m.label} (~${m.promptTokEst} tok)`).join(', ') || 'none'}`)

// ----------------------------------------------------------------
// Final console summary
// ----------------------------------------------------------------
log('=== SPEC ORCHESTRATION COMPLETE ===')
log(`Overall: ${overall} | landed: ${merged.map(o => o.taskNum).join(', ') || 'none'} | escalated: ${escalated.map(o => o.taskNum).join(', ') || 'none'} | skipped: ${skipped.map(o => o.taskNum).join(', ') || 'none'} | back-half: ${backHalfVerdict}`)
if (escalated.length) {
  log('Escalations need your analysis:')
  for (const o of escalated) {
    log(`  Task ${o.taskNum}: ${o.triage || (o.reasons || []).join('; ')} | ${o.inPlace ? 'in-place (rolled back)' : `worktree: ${o.worktreePath || 'n/a'}`} | review: ${o.reviewReport || `${reportsDir}/task${o.taskNum}-review.md`}`)
  }
  log(`After fixing, resume with: /sdlc-block ${blockId}`)
}
log(`Block report: ${reportResult?.reportFile || blockReport}`)

return {
  blockId,
  overallVerdict: overall,
  verifyDepth,
  waves: waves.length,
  merged: merged.map(o => ({ taskNum: o.taskNum, commitHash: o.commitHash, mergeStrategy: o.inPlace ? 'in-place' : o.mergeStrategy })),
  escalated: escalated.map(o => ({ taskNum: o.taskNum, finalVerdict: o.finalVerdict, worktreePath: o.worktreePath, branchName: o.branchName, reviewReport: o.reviewReport, reasons: o.reasons, triage: o.triage })),
  skipped: skipped.map(o => o.taskNum),
  backHalf: blockComplete ? { verdict: backHalfVerdict, reviewAttempts: backHalf?.reviewAttempts ?? null, workflowReport: backHalf?.workflowReport || null } : null,
  breakdown: breakdownAssessment,
  blockReport: reportResult?.reportFile || blockReport,
  resumeCommand: `/sdlc-block ${blockId}`,
  outcomes
}
