// =============================================================================
// sdlc-task — Parallel-Safe SDLC Pipeline with Auto-Managed Worktree
// =============================================================================
//
// A parallel-safe variant of sdlc-run that:
//   1. Auto-creates a git worktree for this specific task
//   2. Runs the full SDLC pipeline inside that worktree
//   3. Defers status.md / log.md updates to a task log file
//      (applied at merge time via /clean-worktree)
//
// This lets multiple tasks run simultaneously with zero shared file writes,
// eliminating merge conflicts. sdlc-run.js is unchanged and still available
// for sequential use.
//
// USAGE
//   /sdlc-task <spec-slug> 2   runs task 2 in an isolated worktree
//
//   Task number is REQUIRED. For full-spec runs use /sdlc-run instead.
//
// PIPELINE STAGES (in order)
//   Worktree   → auto-create (or suffix-increment) isolated git worktree
//   Scout      → detect current stage from report files
//   Plan       → generate task spec (if missing) + breakdown assessment (standalone runs; recommend/auto/off
//                per planning/harness.json breakdown.mode — skipped under /sdlc-block, which assesses once)
//   Implement  → execute the task from spec
//   Fix        → targeted fixes for FAIL/PARTIAL review (up to 3 attempts)
//   Test       → run the project's validation suite from planning/harness.json (+ universal emoji gate)
//   Review     → fresh validation run + acceptance criteria; verdict gates next stage
//   Document   → surgical patches to docs/ (gates on PASS verdict)
//   Wrap-up    → write task log file (defers status/log to merge time)
//
// WHAT RUNS IN THE WORKTREE vs. MAIN
//   Worktree branch: all code, content, doc, and report changes
//   Main (at merge): status.md + log.md updates (applied by /clean-worktree)
//
// MERGE FLOW
//   After pipeline completes:
//     /clean-worktree <branchName>
//   This: merges the branch → applies the task log → updates status/log →
//         commits → removes worktree → deletes branch.
//
// WORKTREE PATH CONVENTION
//   trees/<specSlug-lowercased>-task<N>   e.g. trees/<spec-slug>-task2
//   If that name is taken, auto-increments: trees/...-task2-2, -3, etc.
//   The actual branch name is always reported in the pipeline output and task log.
//
// RESUMPTION
//   Same as sdlc-run: the scout checks which report files exist.
//   If the worktree already exists at setup time, a new suffixed worktree is
//   created rather than resuming the old one. This ensures clean state for retries.
//
// COMMIT STRATEGY (same as sdlc-run)
//   feat: implement <stem>         implement agent
//   fix: fix pass N for <stem>     fix agent (one per pass)
//   docs: update docs for <stem>   document agent
//   chore: wrap up <stem>          finalize agent (reports + task log)
//
// MODEL TIERING (token lever — see the MODEL map below)
//   Three tiers: Opus earns its cost on PLANNING (generate-tasks fallback); Haiku handles the
//   purely-mechanical stages (scout, test, finalize — fixed procedures, no judgment); Sonnet
//   handles everything in between (implement/fix/review/document/task-log). Tune one place: the
//   MODEL map. Real planning happens upstream in the /generate-tasks and /breakdown skills — run
//   those on Opus. This matters most under /sdlc-block, which fans this pipeline out across many tasks.
//
// STAGED MODEL ESCALATION (ESCALATION_MODEL)
//   The FINAL fix pass and FINAL review attempt before the loop gives up run on Opus.
//   The cheap Sonnet path covers the common case; a genuinely hard failure that has
//   already failed twice gets one strong shot before the task escalates. Set null to off.
//
// =============================================================================

export const meta = {
  name: 'sdlc-task',
  description: 'Run the SDLC pipeline for a single task in an isolated worktree — parallel-safe variant of sdlc-run',
  whenToUse: 'When running a specific numbered task in parallel with other tasks. Task number is required. Usage: /sdlc-task <spec-slug> 2',
  phases: [
    { title: 'Worktree',   detail: 'Auto-create (or suffix-increment) git worktree for isolated execution' },
    { title: 'Scout',      detail: 'Determine current pipeline stage from report files' },
    { title: 'Plan',       detail: 'Generate task spec (if missing) and assess whether a coarse task needs breakdown first' },
    { title: 'Implement',  detail: 'Execute the task' },
    { title: 'Fix',        detail: 'Targeted fixes for FAIL/PARTIAL review' },
    { title: 'Test',       detail: "Run the project's validation suite (from planning/harness.json) in the worktree" },
    { title: 'Review',     detail: 'Verify acceptance criteria; issue verdict' },
    { title: 'UI Test',    detail: 'Browser smoke check (only when planning/harness.json enables uiTest)' },
    { title: 'Document',   detail: 'Patch docs/ (gates on PASS verdict)' },
    { title: 'Wrap-up',    detail: 'Write task log file (status/log deferred to merge time)' },
  ]
}

// ----------------------------------------------------------------
// Parse args: REQUIRE "<spec-slug> 2" — task number is mandatory
// ----------------------------------------------------------------
const rawArgs = typeof args === 'string' ? args.trim() : ''
if (!rawArgs) {
  log('ERROR: No arguments provided.')
  log('Usage: /sdlc-task <spec-slug> 2')
  log('Task number is required. For full-spec runs, use /sdlc-run instead.')
  return { error: 'Missing required arguments: spec name and task number' }
}

const parts = rawArgs.split(/\s+/)
const blockId = parts[0]
const taskNumber = parts.length > 1 ? parseInt(parts[1], 10) : null
// --resume: reuse an EXISTING worktree for this task (set by /sdlc-block when a prior run was
// interrupted after implement completed) instead of suffix-incrementing into a fresh duplicate.
const resumeMode = parts.includes('--resume')
// Set by /sdlc-block: it runs the breakdown assessment ONCE at the spec level (Analyze stage), so the
// per-task engine must NOT re-assess — that would duplicate the work across every parallel task.
const underBlock = parts.includes('--under-block')
// Set by /sdlc-block ONLY when this task runs in a parallel batch with concurrent siblings (batch
// width > 1). Output-token telemetry (outTok) is a shared-pool delta — under concurrent siblings it
// measures the whole batch's burn, not this task's, so we mark it non-isolated rather than reporting a
// misleading number. promptTok and filesReadKb stay per-agent and accurate. See decisions/D12.
const parallelWave = parts.includes('--parallel-wave')

if (taskNumber === null || isNaN(taskNumber)) {
  log(`ERROR: Task number is required but not provided (got: "${rawArgs}").`)
  log('Usage: /sdlc-task <spec-slug> 2')
  log('For full-spec runs use /sdlc-run instead.')
  return { error: 'Task number required', rawArgs }
}

const specFile    = `planning/${blockId}/tasks.md`
const stem        = `${blockId}-task${taskNumber}`
const reportsDir  = `planning/${blockId}/sdlc/reports`
const taskPrefix  = `task${taskNumber}-`
const implementReport = `${reportsDir}/${taskPrefix}implement.md`
const testReport      = `${reportsDir}/${taskPrefix}test.md`
const reviewReport    = `${reportsDir}/${taskPrefix}review.md`
const documentReport  = `${reportsDir}/${taskPrefix}document.md`
const uitestReport    = `${reportsDir}/${taskPrefix}ui-test.md`
const workflowReport  = `${reportsDir}/${taskPrefix}workflow.md`
const logFile         = `${reportsDir}/${taskPrefix}log.md`
const breakdownFile   = `planning/${blockId}/breakdown.md`

// Base branch name (suffix may be appended by setup agent).
// Dots are kept so a phase-dotted spec slug (e.g. <spec-slug>) round-trips
// to its planning/<slug>/ directory; git allows '.' in a branch name (just not '..').
const baseBranchName = `${blockId}-task${taskNumber}`.toLowerCase().replace(/[^a-z0-9.-]/g, '-')

log(`Target: ${blockId} task ${taskNumber}`)
log(`Spec: ${specFile} | Stem: ${stem}`)

// ----------------------------------------------------------------
// Schemas
// ----------------------------------------------------------------
const SETUP_SCHEMA = {
  type: 'object',
  required: ['branchName', 'worktreePath', 'wasCreated'],
  properties: {
    branchName:    { type: 'string', description: 'Actual branch name used (may have -2, -3 suffix if base was taken)' },
    worktreePath:  { type: 'string', description: 'Absolute path to the worktree directory' },
    wasCreated:    { type: 'boolean', description: 'true if a new worktree was created, false if it already existed' },
    notes:         { type: 'string' }
  }
}

const SCOUT_SCHEMA = {
  type: 'object',
  required: ['startStage', 'specFileExists', 'blockStatus', 'existingReports', 'statusSummary'],
  properties: {
    startStage: {
      type: 'string',
      enum: ['generate-tasks', 'implement', 'fix', 'test', 'review', 'ui-test', 'document', 'wrap-up'],
    },
    specFileExists:    { type: 'boolean' },
    blockStatus: {
      type: 'string',
      enum: ['Not started', 'In progress', 'Done', 'Blocked', 'Skipped', 'Unknown'],
    },
    existingReports:   { type: 'array', items: { type: 'string' } },
    reviewVerdict:     { type: 'string', description: 'PASS, FAIL, PARTIAL, or empty string' },
    currentFocus:      { type: 'string' },
    lastDevlogEntry:   { type: 'string' },
    statusSummary:     { type: 'string' },
    discrepancies:     { type: 'string' }
  }
}

const STAGE_SCHEMA = {
  type: 'object',
  required: ['reportFile', 'success'],
  properties: {
    reportFile:     { type: 'string' },
    success:        { type: 'boolean' },
    filesModified:  { type: 'array', items: { type: 'string' } },
    commitHash:     { type: 'string' },
    filesReadKb:    { type: 'number', description: 'Telemetry (optional): sum of bytes of all files this stage cat/Read, divided by 1024.' },
    notes:          { type: 'string' }
  }
}

const TEST_SCHEMA = {
  type: 'object',
  required: ['reportFile', 'allPassed', 'passCount', 'failCount'],
  properties: {
    reportFile:   { type: 'string' },
    allPassed:    { type: 'boolean' },
    passCount:    { type: 'integer' },
    failCount:    { type: 'integer' },
    failedTests:  { type: 'array', items: { type: 'string' } },
    notes:        { type: 'string' }
  }
}

const REVIEW_SCHEMA = {
  type: 'object',
  required: ['reportFile', 'verdict'],
  properties: {
    reportFile:      { type: 'string' },
    verdict:         { type: 'string', enum: ['PASS', 'FAIL', 'PARTIAL'] },
    failureReasons:  { type: 'array', items: { type: 'string' } },
    unmetCriteria:   { type: 'array', items: { type: 'string' } },
    filesReadKb:     { type: 'number', description: 'Telemetry (optional): sum of bytes of all files this stage cat/Read, divided by 1024.' },
    notes:           { type: 'string' }
  }
}

const LOG_SCHEMA = {
  type: 'object',
  required: ['logFile', 'applied'],
  properties: {
    logFile:    { type: 'string' },
    applied:    { type: 'boolean' },
    nextFocus:  { type: 'string', description: 'The Current focus string written to the log file' },
    notes:      { type: 'string', description: 'Any decisions that should be added to planning/decisions/' }
  }
}

const FINALIZE_SCHEMA = {
  type: 'object',
  required: ['workflowReportFile', 'commitMessage'],
  properties: {
    workflowReportFile: { type: 'string' },
    commitMessage:      { type: 'string' },
    commitHash:         { type: 'string' },
    notes:              { type: 'string' }
  }
}

const UI_TEST_SCHEMA = {
  type: 'object',
  required: ['reportFile', 'verdict'],
  properties: {
    reportFile:      { type: 'string' },
    verdict:         { type: 'string', enum: ['PASS', 'WARN', 'FAIL', 'SKIPPED'] },
    failureReasons:  { type: 'array', items: { type: 'string' } },
    notes:           { type: 'string' }
  }
}

const BREAKDOWN_ASSESS_SCHEMA = {
  type: 'object',
  required: ['alreadyExists', 'recommendBreakdown'],
  properties: {
    alreadyExists:      { type: 'boolean', description: 'true if breakdown.md already has a "### Step N:" section for this task — assessment is then a no-op' },
    recommendBreakdown: { type: 'boolean', description: 'true if the task is coarse enough to benefit from /breakdown into atomic sub-steps' },
    reason:             { type: 'string', description: 'one sentence: which heuristic signal fired (or why no breakdown is needed)' },
    fileCount:          { type: 'integer', description: 'distinct files the task creates/modifies' }
  }
}

// ----------------------------------------------------------------
// MODEL TIERING — the primary token lever for this pipeline.
//
// Principle: match the model to the work. Opus earns its cost on PLANNING; Haiku handles the
// purely-mechanical stages; Sonnet handles the judgment work in between.
//   • Opus   — generate-tasks (authors the spec; fallback path only) and breakdown-gen (authors
//              atomic sub-steps for a coarse task in auto mode). Both are PLANNING.
//   • Haiku  — worktree-setup / scout / test / task-log / finalize. Each is a fixed procedure with
//              no real judgment: worktree-setup follows an exact free-name + sparse-checkout recipe,
//              scout is a deterministic file-existence decision tree, test runs the project's
//              validation suite and reads exit codes (review re-runs the gating checks anyway),
//              task-log fills a rigid template (all values pre-bracketed) plus a one-paragraph
//              summary, and finalize just fills a JS-precomputed table and runs scripted git adds.
//   • Sonnet — implement / fix / review / document / breakdown-assess. A sharp spec + breakdown makes
//              these well-scoped enough that Sonnet does them reliably. (Review is gated by an
//              authoritative fresh-test run, and fix failures escalate rather than silently ship.)
//
// Note: the REAL planning usually happens upstream in the /generate-tasks and /breakdown
// SKILLS (run those on an Opus session). The generate-tasks stage below is only a fallback
// that fires when the spec file is missing — pinned to Opus so that path is strong too.
//
// To re-tier, change one value here — nothing else moves.
// Valid values: 'haiku' | 'sonnet' | 'opus' | undefined (inherit session model).
// ----------------------------------------------------------------
const MODEL = {
  worktreeSetup: 'haiku',    // scripted git following an exact free-name recipe + sparse-checkout; recipe-
                             //   following, not judgment. A failure aborts the run, but that is a reliability
                             //   property haiku meets — the recipe is deterministic (re-tiered, D11).
  scout:         'haiku',    // deterministic decision tree: ls a few files, apply a fixed 7-rule order
  generateTasks: 'opus',     // PLANNING — authors the spec that drives everything (fallback path)
  breakdownAssess: 'sonnet', // judges whether a single task is too coarse — light judgment, same tier as review
  breakdownGen:  'opus',     // PLANNING — authors atomic sub-steps for a flagged task (auto mode only)
  implement:     'sonnet',   // writes content/code + tests against a scoped spec/breakdown
  fix:           'sonnet',   // targeted fixes; failures escalate, never silently ship
  test:          'haiku',    // runs the project's validation suite, reads exit codes; review re-runs the gating checks
  review:        'sonnet',   // verify criteria; gated by an authoritative fresh run of the gating checks
  uiTest:        'sonnet',   // live browser smoke checks (when uiTest.enabled); needs judgment to interpret results
  document:      'sonnet',   // surgical doc patches, gated on PASS
  taskLog:       'haiku',    // fills a rigid template (all values pre-bracketed) + a one-paragraph summary;
                             //   the generative surface is small enough for haiku (re-tiered, D11)
  finalize:      'haiku',    // assembles a JS-precomputed table + scripted git add; can't break the pipeline
}

// Merge an optional model override into an agent's opts (omits the key when undefined,
// so the agent inherits the session model rather than receiving model: undefined).
function withModel(base, model) {
  return model ? { ...base, model } : base
}

// ----------------------------------------------------------------
// TOKEN TELEMETRY (Phase A — additive, no behavior change)
//
// Per-stage attribution of injected-prompt size and output-token delta. The runtime cannot
// see a subagent's INTERNAL context (the `cat` outputs land inside the spawned agent, invisible
// here), so JS-side we measure only the injected prompt and the budget delta. The read-heavy
// stages self-report a `filesReadKb` ingestion estimate via the schema (folded in at the call
// site). `agent` stays importable for any call we deliberately want untraced.
//
//   promptTokEst — injected input only (~prompt.length / 4)
//   outTok       — output-token delta from the shared budget pool; null when no +Nk target is set.
//                  Attributes cleanly only for SEQUENTIAL stages — which is this engine's whole
//                  pipeline when run solo. BUT when /sdlc-block runs this task in a parallel batch
//                  (--parallel-wave), the pool is shared with concurrent sibling tasks, so the delta
//                  is contaminated; we render it as "— (parallel)" at report time rather than report a
//                  misleading number (the runtime exposes no per-agent output count). See D12.
// ----------------------------------------------------------------
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

// Fold a stage's self-reported `filesReadKb` (A2) into the metrics entry the wrapper just pushed.
// Safe to call immediately after the awaited tracedAgent call — that entry is always metrics[last].
function recordFilesRead(result) {
  if (result && result.filesReadKb != null && metrics.length) {
    metrics[metrics.length - 1].filesReadKb = result.filesReadKb
  }
}

// ----------------------------------------------------------------
// HARNESS CONFIG — mechanism/policy split (see planning/harness.json)
//
// The engine ships NO stack defaults. A project declares its validation policy in
// planning/harness.json. The workflow runtime has no filesystem access, so a dedicated
// micro-loader agent reads + parses the file (the same way sdlc-block loads execution-plan.json).
// Returns the parsed config object, or null when the file is absent or invalid — callers then
// degrade to the spec's `## Validation Commands` section and disable the UI-test stage.
//
// NOTE (P4): this task engine runs inside a git worktree — when wired, the loader's cat must run
// as `cd ${worktreePath} && cat planning/harness.json` (the worktree carries the project's config).
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

// Spawn the micro-loader agent and return the parsed config (or null). Wired into the stages
// in P4; defined here so the loader path exists from P1. No stack defaults on absence.
// cwd: optional working-directory prefix (the worktree path) for the cat command.
async function loadHarnessConfig(cwd) {
  const prefix = cwd ? `cd ${cwd} && ` : ''
  const result = await tracedAgent(`
You are the harness-config loader for the SDLC pipeline. Your ONLY job is to read the project's
validation-policy file and return it as structured data. Do not run any checks or modify anything.

STEP 1 — Read the config file:
  ${prefix}cat planning/harness.json 2>/dev/null && echo "__HARNESS_PRESENT__" || echo "__HARNESS_ABSENT__"

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

// Snapshot baseline artifacts for any `baseline-diff` checks at worktree creation (pre-implement),
// so the Test stage can diff current output against the pre-task state and fail only on net-new
// items. Resume-safe: only writes a baseline that does not already exist. No-op when no baseline-diff
// checks are configured. The engine ships no stack defaults — baselineCommand comes from harness.json.
async function snapshotBaselines(cfg, cwd) {
  const checks = (cfg?.validation?.checks || []).filter(c => c.kind === 'baseline-diff' && c.baselineCommand)
  if (!checks.length) return
  const pre = cwd ? `cd ${cwd} && ` : ''
  const steps = checks.map(c => {
    const slug = (c.name || 'check').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')
    const path = `${reportsDir}/${taskPrefix}${slug}-baseline.json`
    return `Baseline "${c.name}" -> ${path}:
  ${pre}mkdir -p ${reportsDir}
  ${pre}[ -f ${path} ] && echo "BASELINE EXISTS (kept): ${path}" || { ${c.baselineCommand} > ${path} 2>/dev/null; echo "BASELINE WRITTEN: ${path}"; }`
  }).join('\n\n')
  await tracedAgent(`
You are the baseline-snapshot agent for the SDLC pipeline. Capture the pre-task baseline for each
baseline-diff validation check, in the worktree, BEFORE any implementation runs. Run each block
exactly as written. Do NOT modify source. Existing baselines are kept (resume-safe).

${steps}

Return using StructuredOutput: done=true, and note which baselines were written vs already present.
`, { label: 'baseline-snapshot', schema: { type: 'object', required: ['done'], properties: { done: { type: 'boolean' }, notes: { type: 'string' } } }, model: 'haiku' })
}

// Render the inner project-validation check list for the Test stage from harness config.
// Returns the numbered CHECK blocks the agent runs (each prefixed `cd <cwd> &&` so it runs in the
// worktree) before the universal emoji gate. cfg null / no checks → fall back to the spec's
// `## Validation Commands`. The engine ships NO stack defaults. changedPaths is reserved for the
// deferred conditionalChecks feature (unused in the MVP).
function renderCheckList(cfg, { changedPaths, cwd } = {}) {
  const pre = cwd ? `cd ${cwd} && ` : ''
  const checks = cfg?.validation?.checks ?? []
  if (!checks.length) {
    return `The project ships no \`planning/harness.json\` validation suite, so derive the checks
from the spec instead:
  - Read the spec's optional "## Validation Commands" section.
  - Run each command it lists, IN ORDER (from the worktree root: ${cwd || 'repo root'}). Each command
    is one check — record test_name (a short slug), execution_command, test_purpose ("from the spec's
    Validation Commands"), passed (true iff exit code 0), and error (output on failure).
  - If the spec has no "## Validation Commands" section, run no project checks — record a single
    informational row (test_name "no_validation_suite", passed true, empty error). Then run the
    universal emoji gate below.`
  }
  return checks.map((c, i) => {
    const n = i + 1
    const kind = c.kind || 'command'
    const slug = (c.name || `check${n}`).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '')
    const gate = c.gates
      ? 'GATING — a failure here blocks the review verdict'
      : 'non-gating — informational; a failure here does not block the verdict'
    const header = `CHECK ${n} — ${c.name} (${c.purpose}) [${gate}]`

    // --- baseline-diff: fail only on items absent from the worktree-creation baseline ---
    if (kind === 'baseline-diff') {
      const baselinePath = `${reportsDir}/${taskPrefix}${slug}-baseline.json`
      const currentPath = `/tmp/${stem}-${slug}-current.json`
      const keysLiteral = JSON.stringify(c.compareKeys || [])
      return `${header} — baseline-diff (fail ONLY on net-new items vs the baseline snapshotted at worktree creation):
  ${pre}${c.command} > ${currentPath} 2>/dev/null; true
  ${pre}python3 << 'PYEOF'
import json, sys
baseline_path = '${baselinePath}'
current_path  = '${currentPath}'
keys = ${keysLiteral}
try:
    b = json.load(open(baseline_path, encoding='utf-8'))
except Exception as e:
    print(f'WARNING: could not load baseline ({e}) — treating all current items as pre-existing'); b = []
try:
    c = json.load(open(current_path, encoding='utf-8'))
except Exception:
    c = []
def k(v): return tuple(str(v.get(x, '')) for x in keys) if isinstance(v, dict) else (str(v),)
seen = set(k(v) for v in b)
new = [v for v in c if k(v) not in seen]
if new:
    print(f'NET-NEW ({len(new)} introduced by this task, absent from baseline):')
    for v in new[:20]: print('  ' + json.dumps(v)[:200])
    sys.exit(1)
print(f'CHECK ${n} PASSED: no net-new items (baseline {len(b)}, current {len(c)})'); sys.exit(0)
PYEOF
  echo "CHECK${n}_EXIT:$?"`
    }

    // --- count-delta: extract an integer and fail when it regresses vs the previous task ---
    if (kind === 'count-delta') {
      const prevReport = taskNumber > 1 ? `${reportsDir}/task${taskNumber - 1}-test.md` : ''
      const failRule = c.failOn === 'zero-or-decrease'
        ? 'FAIL if delta <= 0 (count must strictly increase)'
        : 'FAIL if delta < 0 (count must not decrease)'
      const prevStep = prevReport
        ? `  Read the previous task's recorded count:
    ${pre}grep -oE 'COUNT\\[${slug}\\]: [0-9]+' ${prevReport} | head -1 || echo "NO_PREV_COUNT"
  If NO_PREV_COUNT (previous report has no marker), treat this check as SKIP — delta unknown, do not fail.`
        : `  This is task 1 — there is no previous task. Treat this check as SKIP (no delta to compare).`
      return `${header} — count-delta (${c.failOn}):
  ${pre}${c.command}
  Extract the current count: the first integer on the line matching the ERE /${c.countPattern}/.
${prevStep}
  Compute delta = current - previous. ${failRule}.
  IMPORTANT: write the marker line "COUNT[${slug}]: <current>" verbatim into the test report (any
  section) so the NEXT task can read it. Record the delta and the pass/fail in this check's row.
  echo "CHECK${n}_EXIT:0  (set to 1 only if the rule above fails; SKIP counts as pass)"`
    }

    // --- warning-scan: run a command (exit code gates) and record matches of warningPatterns ---
    if (kind === 'warning-scan') {
      const outPath = `/tmp/${stem}-${slug}.out`
      const alternation = (c.warningPatterns || []).map(p => `(${p})`).join('|')
      const patternSeverity = c.gates
        ? 'Because gates:true, a pattern match ALSO FAILS this check.'
        : 'Because gates:false, pattern matches are informational WARN entries — they do NOT fail the check (but DO record them).'
      return `${header} — warning-scan (run the command, gate on its exit code, then scan its output):
  ${pre}${c.command} > ${outPath} 2>&1; echo "CMD_EXIT:$?"
  ${pre}grep -nE '${alternation}' ${outPath} && echo "WARNINGS_FOUND" || echo "NO_WARNINGS"
  Pass/fail: this check FAILS if CMD_EXIT is non-zero (the command itself failed). Record every matched
  warning line in this check's row/notes. ${patternSeverity}
  Set the exit marker accordingly:
  echo "CHECK${n}_EXIT:<0 if CMD_EXIT==0 and not failed-by-pattern, else 1>"`
    }

    // --- forbidden-pattern-scan: source greps that must find NO matches ---
    if (kind === 'forbidden-pattern-scan') {
      const ruleLines = (c.rules || []).map(r => {
        const paths = r.paths || '.'
        const allow = r.allowlistPattern ? ` | grep -vE '${r.allowlistPattern}'` : ''
        return `  Rule "${r.id}":
    ${pre}grep -rnE '${r.pattern}' ${paths}${allow} && echo "RULE ${r.id}: MATCHED (violation)" || echo "RULE ${r.id}: clean"`
      }).join('\n')
      return `${header} — forbidden-pattern scan (every rule below must find NO matches):
${ruleLines}
  This check PASSES only if EVERY rule reports "clean". If any rule MATCHED, the check FAILS and the
  matched lines are violations — list them in this check's row.
  echo "CHECK${n}_EXIT:0  (set to 1 if any rule MATCHED, else 0)"`
    }

    // --- command (default): plain exit-code gate (unchanged behavior) ---
    return `${header}:
  ${pre}${c.command}
  echo "CHECK${n}_EXIT:$?"`
  }).join('\n\n')
}

// Render the UI-test stage prompt parts from harness config. Called ONLY when cfg.uiTest.enabled.
// Interpolates the MVP fields (devServerCommand / readySignal / port / routes); the stage gate
// decides whether this runs at all. port is the resolved per-task port (base + taskNumber).
function renderUiTestPrompt(cfg, port) {
  const ui = cfg.uiTest
  const routes = (Array.isArray(ui.routes) && ui.routes.length) ? ui.routes : ['/']
  const ready = ui.readySignal || 'ready'
  const devCmd = ui.devServerCommand || 'echo "ERROR: uiTest.enabled but devServerCommand missing in planning/harness.json" && false'
  const routeChecks = routes.map((r, i) => `  CHECK ${i + 1} — Route ${r} renders without error:
    playwright-cli goto http://localhost:${port}${r}
    playwright-cli snapshot
    playwright-cli console
    Verify: the page title / headings do not contain "404", "500", "Error", or "Not Found";
    the page shows real content (not a bare framework error screen); the console has no
    error-level entries ("warning"-level entries → WARN, not FAIL).`).join('\n\n')
  const routeRows = routes.map(r => `  | Route ${r} renders | PASS/WARN/FAIL | |`).join('\n')
  return { routes, ready, devCmd, port, routeChecks, routeRows }
}

// ----------------------------------------------------------------
// Stage results accumulator
// ----------------------------------------------------------------
const stageResults = []

// ================================================================
// PHASE 0: WORKTREE SETUP — auto-create isolated worktree
// ================================================================
phase('Worktree')
log(`Setting up worktree for ${stem}${resumeMode ? ' (resume mode — reuse existing worktree if present)' : ''}...`)

const setupResult = await tracedAgent(`
You are the worktree setup agent. Your job is to create (or locate) an isolated git worktree
for this pipeline run. All bash commands run from the MAIN REPO ROOT (your current CWD).

Target:
  Spec:           ${blockId}
  Task:           ${taskNumber}
  Base name:      ${baseBranchName}

STEP 1 — Get the absolute path to the repo root:
  Run: git rev-parse --show-toplevel
  Store the output as repoRoot (trim whitespace).
${resumeMode ? `
RESUME MODE IS ON — reuse the existing worktree for this task instead of creating a fresh one.
  a. Check whether the base worktree directory exists:
       git worktree list | grep "trees/${baseBranchName}" && echo "WT_EXISTS" || echo "WT_MISSING"
  b. Check whether the base branch exists:
       git branch --list "${baseBranchName}"
  Then:
  - WT_EXISTS → REUSE it verbatim. Set branchName="${baseBranchName}", do NOT create anything,
    do NOT recreate the empty init commit. Skip STEP 2 and STEP 3 entirely; go to STEP 4.
    Set wasCreated=false.
  - WT_MISSING but the branch "${baseBranchName}" exists (orphan branch, dir was removed) →
    re-attach a worktree to the existing branch (note: NO -b flag, so it checks out the existing branch):
       mkdir -p trees
       git worktree add --no-checkout trees/${baseBranchName} ${baseBranchName}
       git -C trees/${baseBranchName} sparse-checkout init --cone
       git -C trees/${baseBranchName} sparse-checkout set app components hooks lib content scripts docs planning .claude __tests__ __mocks__ types
       git -C trees/${baseBranchName} checkout
       if [ -f .env ]; then cp .env trees/${baseBranchName}/.env; fi
       if [ -f .env.local ]; then cp .env.local trees/${baseBranchName}/.env.local; fi
    Set branchName="${baseBranchName}", wasCreated=false. Skip STEP 2 and STEP 3; go to STEP 4.
  - Neither exists → resume was requested but nothing is there; fall through to STEP 2/3 and create
    a fresh worktree named "${baseBranchName}" as normal.
` : ''}
STEP 2 — Find a free worktree name using this exact algorithm:

  Start with candidate = "${baseBranchName}" and work through each suffix in turn:

  Iteration 1 — candidate = "${baseBranchName}":
    Run: git worktree list | grep "trees/${baseBranchName}"
    Run: git branch --list "${baseBranchName}"
    If BOTH return nothing → "${baseBranchName}" is free. Use it. Skip to STEP 3.

  Iteration 2 — candidate = "${baseBranchName}-2":
    Run: git worktree list | grep "trees/${baseBranchName}-2"
    Run: git branch --list "${baseBranchName}-2"
    If BOTH return nothing → "${baseBranchName}-2" is free. Use it. Skip to STEP 3.

  Iteration 3 — candidate = "${baseBranchName}-3":
    ... same pattern ...

  Continue through "-10" as the cap. Use the first free candidate found.
  Store the chosen name as branchName.

STEP 3 — Create the worktree:
  Run these commands in order (replace [branchName] and [repoRoot] with actual values):

  a. mkdir -p trees
  b. git worktree add --no-checkout trees/[branchName] -b [branchName]
  c. git -C trees/[branchName] sparse-checkout init --cone
  d. git -C trees/[branchName] sparse-checkout set app components hooks lib content scripts docs planning .claude __tests__ __mocks__ types
  e. git -C trees/[branchName] checkout
  f. if [ -f .env ]; then cp .env trees/[branchName]/.env; fi
  g. if [ -f .env.local ]; then cp .env.local trees/[branchName]/.env.local; fi
  h. git -C trees/[branchName] commit --allow-empty -m "chore: init worktree [branchName]"

STEP 4 — Verify:
  Run: git worktree list
  Run: ls trees/[branchName]/
  Confirm the worktree exists and contains app/, components/, content/, planning/, .claude/ directories.

STEP 5 — Compute the absolute worktree path:
  worktreePath = repoRoot + "/trees/" + branchName

Return your result using the StructuredOutput tool:
  branchName:   the final chosen branch name (e.g. "${baseBranchName}" or "${baseBranchName}-2")
  worktreePath: the ABSOLUTE path to the worktree (e.g. <repoRoot>/trees/${baseBranchName})
  wasCreated:   true if a NEW worktree was created; false if an existing one was reused (resume mode)
  notes:        any issues encountered
`, withModel({ label: 'worktree-setup', schema: SETUP_SCHEMA, phase: 'Worktree' }, MODEL.worktreeSetup))

if (!setupResult) {
  log('Worktree setup agent returned null — aborting pipeline')
  return { error: 'Worktree setup failed', blockId, taskNumber, stem }
}

const { branchName, worktreePath } = setupResult
log(`Worktree ready: ${worktreePath} (branch: ${branchName})`)
stageResults.push({ stage: 'worktree-setup', ...setupResult, success: true })

// ----------------------------------------------------------------
// Build the worktree path injection header — prepended to EVERY agent prompt
// ----------------------------------------------------------------
const W = `WORKTREE (not the main repo). repo root = ${worktreePath}
Shell state does NOT persist between Bash calls — START EVERY Bash call with: cd ${worktreePath} &&
Run all build/test/validation from the repo root; relative paths (planning/...) resolve from there.
`

// ================================================================
// PHASE 1: SCOUT — determine current pipeline stage
// ================================================================
phase('Scout')

const scout = await tracedAgent(`${W}
You are the pipeline scout for the SDLC workflow system.

Target:
  Spec ID:     ${blockId}
  Task number: ${taskNumber}
  Spec file:   ${specFile}
  Report stem: ${stem}
  Reports dir: ${reportsDir}

Your job is to determine which SDLC stage to start from, based on which report files exist.
Run these checks using the Bash tool (all commands start with: cd ${worktreePath} &&):

STEP 1 — Check spec file:
  cd ${worktreePath} && ls -la ${specFile} 2>/dev/null && echo "SPEC_EXISTS" || echo "SPEC_MISSING"

STEP 2 — Check report files:
  cd ${worktreePath} && ls ${implementReport} 2>/dev/null && echo "HAS_IMPLEMENT" || echo "NO_IMPLEMENT"
  cd ${worktreePath} && ls ${testReport} 2>/dev/null && echo "HAS_TEST" || echo "NO_TEST"
  cd ${worktreePath} && ls ${reviewReport} 2>/dev/null && echo "HAS_REVIEW" || echo "NO_REVIEW"
  cd ${worktreePath} && ls ${uitestReport} 2>/dev/null && echo "HAS_UITEST" || echo "NO_UITEST"
  cd ${worktreePath} && ls ${documentReport} 2>/dev/null && echo "HAS_DOCUMENT" || echo "NO_DOCUMENT"
  cd ${worktreePath} && ls ${reportsDir}/*.md 2>/dev/null | head -20 || echo "NO_BLOCK_REPORTS"

STEP 3 — Read status.md:
  cd ${worktreePath} && head -60 planning/status.md

STEP 4 — Read recent log (at the worktree root):
  cd ${worktreePath} && head -60 log.md

STEP 5 — If review report exists, extract the verdict:
  cd ${worktreePath} && grep -iE "\\*\\*Verdict|## Verdict|^Verdict:" ${reviewReport} 2>/dev/null | head -5 || echo "NO_REVIEW_REPORT"

STEP 6 — Determine startStage using this EXACT priority order:
  1. Spec file MISSING → "generate-tasks"
  2. Spec exists, no implement report → "implement"
  3. Implement report exists, no test report → "test"
  4. Test report exists, no review report → "review"
  5. Review report with FAIL or PARTIAL verdict → "fix"
  6. Review report with PASS verdict, no ui-test report → "ui-test"
  7. Review report with PASS verdict, ui-test report exists, no document report → "document"
  8. Document report exists → "wrap-up"

STEP 7 — Find this spec's status in status.md progress table. Look for a row containing
  "${blockId}" and extract its Status column value (title-case: Not started / In progress / Done / Blocked / Skipped).

STEP 8 — Note any discrepancy between log and report files.

Return your findings using the StructuredOutput tool.
`, withModel({ label: 'scout', schema: SCOUT_SCHEMA, phase: 'Scout' }, MODEL.scout))

if (!scout) {
  log('Scout agent failed — cannot determine pipeline state, aborting')
  return { error: 'Scout failed', blockId, taskNumber, stem }
}

log(`Scout: start from "${scout.startStage}" | block status: "${scout.blockStatus}"`)
if (scout.discrepancies) log(`Discrepancies: ${scout.discrepancies}`)
if (scout.statusSummary) log(scout.statusSummary)

// Block "Not started" warning — do NOT edit status.md in the worktree.
// status.md changes are always deferred to the task log (applied at merge time).
// If the block needs to be flipped before parallel tasks start, run /start-block first.
if (scout.blockStatus === 'Not started') {
  log(`Note: Spec "${blockId}" is "Not started" in status.md.`)
  log(`The task log will record the status flip — applied when this branch merges to main.`)
  log(`To update status.md immediately (e.g. before other parallel tasks start), run /start-block ${blockId} from the main session.`)
}

let currentStage = scout.startStage
let reviewAttempts = 0
const MAX_REVIEW_ATTEMPTS = 3
let lastReviewResult = null
// B1: hand-off context carried structurally between stages so review/fix do NOT re-ingest the
// upstream prose reports. The review gate stays authoritative — it re-runs gating checks fresh
// and reads real source; these fields only tell it WHERE to look and what the prior stage claimed.
let lastImplReport = null   // implement OR fix result (fix overwrites the implement-report slot)
let lastTestReport = null   // most recent test result

// Render a structured hand-off field as prompt-safe text (arrays → bullet list, empty → "none").
const handoff = (v) => Array.isArray(v)
  ? (v.length ? v.map((x) => `     - ${x}`).join('\n') : '     (none)')
  : (v === undefined || v === null || v === '' ? '(none)' : String(v))

// STAGED MODEL ESCALATION — the FINAL fix pass and FINAL review attempt before the loop
// gives up run on a stronger model. The common path stays on Sonnet (MODEL.fix/review);
// only the genuinely-hard case that has already failed twice gets one Opus shot before
// the task escalates to /sdlc-block triage (or a FAIL wrap-up). Set to null to disable.
const ESCALATION_MODEL = 'opus'

// ================================================================
// PHASE 2: PLAN — generate-tasks (only if spec file missing)
// ================================================================
if (currentStage === 'generate-tasks') {
  phase('Plan')
  log('Spec file not found — running generate-tasks...')

  const genResult = await tracedAgent(`${W}
You need to generate the task spec for "${blockId}".

Spec file to create: ${specFile}
Worktree root: ${worktreePath}

Instructions:

1. Read planning/master-plan.md (at the worktree root) — find the section covering "${blockId}".
   Run: cd ${worktreePath} && cat planning/master-plan.md

2. Read CLAUDE.md and planning/context.md — internalize and enforce the project's standing rules.
   CLAUDE.md is the authority; do not assume any stack, locale-parity, narrative, or content-layout
   rule unless written there. Universal harness rules always apply: no fabricated metrics or quotes,
   no emoji, every change ships with tests.
   Run: cd ${worktreePath} && cat CLAUDE.md

3. Read the generic spec skeleton as a format reference:
   cd ${worktreePath} && cat .claude/workflows/templates/spec-template.md

   Also create the spec directory structure now if it does not exist:
   cd ${worktreePath} && mkdir -p planning/${blockId}/sdlc/reports

4. Write ${specFile} (absolute path: ${worktreePath}/${specFile}) following the standard format.

   Rules:
   - Every task ships with the validation that proves it (the project's validation suite passes)
   - Follow every CLAUDE.md standing rule; record any deferral in ## Notes
   - The final task must always be "Validate"
   - Tasks should be numbered ### 1., ### 2., etc.
   - Include exact file paths and component/function names
   - Include a ## Validation Commands block that mirrors planning/harness.json
     (validation.checks[].command, in order); if that file is absent, use the project's
     documented build/test commands from CLAUDE.md

Return using StructuredOutput:
  reportFile: "${specFile}"
  success: true if written successfully
  filesModified: ["${specFile}"]
  notes: brief note about what was generated
`, { label: 'generate-tasks', schema: STAGE_SCHEMA, phase: 'Plan' })

  if (!genResult || !genResult.success) {
    log('generate-tasks failed — aborting pipeline')
    stageResults.push({ stage: 'generate-tasks', success: false })
    return { error: 'generate-tasks failed', blockId, taskNumber, stem, stageResults }
  }
  stageResults.push({ stage: 'generate-tasks', ...genResult })
  log(`Task spec written: ${genResult.reportFile}`)
  currentStage = 'implement'
}

// Load the project's validation policy once (mechanism/policy split — see planning/harness.json).
// Read from the worktree checkout. null when absent/invalid → Test falls back to the spec's
// ## Validation Commands and the UI-test stage is skipped. The engine ships no stack defaults.
const harnessCfg = await loadHarnessConfig(worktreePath)
log(harnessCfg
  ? `Harness config loaded: ${(harnessCfg.validation?.checks || []).length} validation check(s); uiTest ${harnessCfg.uiTest?.enabled ? 'enabled' : 'disabled'}.`
  : 'No planning/harness.json — validation falls back to the spec; UI-test disabled.')
// Capture pre-task baselines for any baseline-diff checks (resume-safe; no-op when none configured).
await snapshotBaselines(harnessCfg, worktreePath)

// ================================================================
// BREAKDOWN ASSESSMENT — should this task be decomposed before implementing?
//
// Assess Task N against a universal coarseness heuristic and act per the project's breakdown policy
// (planning/harness.json → breakdown.mode; default 'recommend'). Runs ONLY when:
//   • we are about to implement fresh (currentStage === 'implement'), AND
//   • this is a STANDALONE run — NOT under /sdlc-block (which assesses once at the spec level), AND
//   • mode !== 'off'.
// Because a standalone run has exactly one worktree, writing breakdown.md here is conflict-free; the
// agent itself no-ops if a "### Step N:" section already exists. 'recommend' logs and proceeds;
// 'auto' generates the sub-steps before implementing. Advisory by default — it never blocks.
// ================================================================
const breakdownMode = harnessCfg?.breakdown?.mode || 'recommend'
const breakdownThreshold = harnessCfg?.breakdown?.complexityThreshold ?? 3
if (currentStage === 'implement' && !underBlock && breakdownMode !== 'off') {
  phase('Plan')
  log(`Breakdown assessment for task ${taskNumber} (mode=${breakdownMode})...`)

  const assess = await tracedAgent(`${W}
You are the breakdown-assessment agent. Decide whether Task ${taskNumber} of this spec is too COARSE
to implement directly and would benefit from a /breakdown into atomic sub-steps first.

STEP 1 — If a breakdown already covers this task, do nothing:
  cd ${worktreePath} && ls ${breakdownFile} 2>/dev/null && grep -q "### Step ${taskNumber}:" ${breakdownFile} && echo "HAS_STEP" || echo "NO_STEP"
  If the last line is "HAS_STEP": return alreadyExists=true, recommendBreakdown=false, and STOP.

STEP 2 — Read Task ${taskNumber} from the spec:
  cd ${worktreePath} && cat ${specFile}
  Focus on the "### ${taskNumber}." section and the Acceptance Criteria that apply to it.

STEP 3 — Apply the coarseness heuristic. The real predictor of decomposition value is SEPARABLE
  STRUCTURE, not raw file count. Task ${taskNumber} is a BREAKDOWN CANDIDATE when ANY hold:
  - it bundles multiple separable concerns (e.g. "implement X AND refactor Y AND add Z"); OR
  - it spans multiple layers/modules (e.g. data model + API + UI); OR
  - it carries a large acceptance-criteria set covering several INDEPENDENTLY-testable units; OR
  - it touches MORE than ${breakdownThreshold} distinct files AND those files are HETEROGENEOUS
    (different shapes/roles, or spanning more than one concern/layer above). File count is a
    CONTRIBUTING signal here, never a trigger on its own.
  HOMOGENEITY DISCOUNT — do NOT flag on file count alone when the many files are the SAME shape serving
  ONE concern (e.g. a content path's metadata file + N near-identical lesson/module pairs, or N
  parallel fixtures). A single focused change over a small file set is also NOT a candidate.

Return using StructuredOutput: alreadyExists, recommendBreakdown, reason (one sentence naming the
signal that fired, or why none did), fileCount (distinct files the task touches).
`, withModel({ label: 'breakdown-assess', schema: BREAKDOWN_ASSESS_SCHEMA, phase: 'Plan' }, MODEL.breakdownAssess))

  if (assess?.alreadyExists) {
    log(`Task ${taskNumber}: breakdown already present — skipping assessment.`)
  } else if (assess?.recommendBreakdown) {
    if (breakdownMode === 'auto') {
      log(`Task ${taskNumber}: coarse (${assess.reason}) — auto-generating breakdown (mode=auto)...`)
      const gen = await tracedAgent(`${W}
You are the breakdown-generation agent (auto mode). Author atomic sub-steps for Task ${taskNumber} so
the implement stage executes a granular plan instead of a coarse one. Do NOT implement anything and do
NOT modify ${specFile}.

1. Read the task and the real source it will touch (so sub-steps name actual paths/symbols):
   cd ${worktreePath} && cat ${specFile}   (focus on "### ${taskNumber}.")
   Read the files that section references before writing sub-steps.

2. Write (create the file, or append this task's section) ${breakdownFile}:

   ### Step ${taskNumber}: <task title>
   - ${taskNumber}.1 <atomic action — exact file path + symbol + what to write/change>
   - ${taskNumber}.2 <...>
   **Verify:** <a concrete command/check after each logical group of sub-steps>

   Each sub-step is a SINGLE atomic action naming exact file paths and function/component names.
   tasks.md stays authoritative for scope/acceptance criteria; breakdown.md is authoritative for HOW.

3. Commit just the breakdown:
   cd ${worktreePath} && git add ${breakdownFile} && git commit -m "docs: breakdown for ${stem}"
   cd ${worktreePath} && git log --oneline -1

Return using StructuredOutput: reportFile="${breakdownFile}", success, filesModified, commitHash, notes.
`, withModel({ label: 'breakdown-gen', schema: STAGE_SCHEMA, phase: 'Plan' }, MODEL.breakdownGen))
      if (gen?.success) {
        log(`Breakdown written: ${breakdownFile} — implement will use its sub-steps.`)
        stageResults.push({ stage: 'breakdown', ...gen })
      } else {
        log(`Breakdown generation did not complete — implementing from tasks.md only.`)
      }
    } else {
      log(`Task ${taskNumber}: RECOMMENDATION — consider /breakdown before implementing (${assess.reason}). breakdown.mode=recommend, so proceeding with tasks.md as-is.`)
    }
  } else if (assess) {
    log(`Task ${taskNumber}: breakdown not needed (${assess.reason || 'task is appropriately scoped'}).`)
  }
}

// ================================================================
// PHASES 3–5: IMPLEMENT → (FIX →) TEST → REVIEW (with retry loop)
// ================================================================
while (['implement', 'fix', 'test', 'review', 'ui-test'].includes(currentStage) && reviewAttempts < MAX_REVIEW_ATTEMPTS) {

  // ----------------------------------------------------------
  // IMPLEMENT
  // ----------------------------------------------------------
  if (currentStage === 'implement') {
    phase('Implement')
    log('Running implement...')

    const implResult = await tracedAgent(`${W}
You are the implementation agent for the SDLC pipeline.

Target:
  Spec:            ${blockId}
  Task:            Task ${taskNumber} only
  Spec file:       ${specFile}
  Report to write: ${implementReport}
  Worktree root:   ${worktreePath}

Instructions:

1. Read CLAUDE.md — internalize all standing rules.
   Run: cd ${worktreePath} && cat CLAUDE.md

2. Read the spec file, focusing on the "### ${taskNumber}." section:
   Run: cd ${worktreePath} && cat ${specFile}
   Implement ONLY the "### ${taskNumber}." section. Do not implement other tasks.

2.5. Check for an optional breakdown file (more granular sub-steps written by /breakdown):
   Run: cd ${worktreePath} && ls ${breakdownFile} 2>/dev/null && echo "BREAKDOWN_EXISTS" || echo "NO_BREAKDOWN"

   If BREAKDOWN_EXISTS:
     Read ${worktreePath}/${breakdownFile}
     Find the "### Step ${taskNumber}:" section — use its atomic sub-steps as the primary
     execution guide for HOW to implement this task.
     Inline "Verify:" commands are live checkpoints — run each before moving to the next sub-step.
     tasks.md is authoritative for scope/acceptance criteria; breakdown.md is authoritative for HOW.

   If NO_BREAKDOWN: proceed using tasks.md only.

3. Execute each step methodically using Read, Edit, Write, and Bash tools.
   ALL file paths must be absolute (under ${worktreePath}) OR use:
     cd ${worktreePath} && <command>

4. As you implement, follow every CLAUDE.md standing rule:
   - CLAUDE.md is the authority — do not assume any stack, locale-parity, narrative, or
     content-layout rule unless written there; if it is, enforce it exactly
   - No fabricated metrics or quotes — every number must be verifiable
   - No emoji in any file
   - Add or update tests for any new code/logic — every change ships with the validation that proves it

5. Run the Validation Commands from the spec to confirm correctness (the project's own suite —
   from planning/harness.json, or the spec's ## Validation Commands):
   cd ${worktreePath} && <each validation command from the spec> 2>&1 | tail -20

5.5. SELF-CHECK — completeness gate (do this BEFORE writing the report or committing):
   Re-read the in-scope "## Acceptance Criteria" for Task ${taskNumber}. For EACH criterion, open the
   actual file(s) and confirm it is FULLY satisfied by your changes — do not assume from memory.
   In particular:
   (a) NO placeholder/stub bodies remain on any code path a criterion requires — e.g.
       \`todo!()\`/\`unimplemented!()\`/\`unreachable!()\`, \`raise NotImplementedError\`,
       \`throw new Error('not implemented')\`, empty \`pass\`-only bodies, or \`TODO\`/\`FIXME\` markers
       in required paths. Sanity-grep ONLY the files the in-scope criteria require (the deliverable
       files and the source paths those criteria name) — NOT every changed file — so the gate never
       pushes you to edit files outside this task's scope. Build that path list from the criteria, then:
         cd ${worktreePath} && grep -nE 'todo!\\(|unimplemented!\\(|unreachable!\\(|NotImplementedError|not implemented|FIXME' <those paths> 2>/dev/null
       A stub in a file no in-scope criterion requires is OUT OF SCOPE — leave it for its owning task.
   (b) EVERY deliverable file a criterion names actually exists at the stated path (e.g. a required
       \`.env.example\`, config file, or fixture) — \`ls\` it.
   (c) EVERY criterion that says "unit-tested"/"covered by a test" has a real, hermetic test that
       exercises that path — not just a compiling stub.
   If ANY criterion is not fully met, FIX IT NOW and re-run step 5 before proceeding. Do NOT write the
   report or return \`success: true\` with a known gap — an unmet criterion shipped here costs a full
   review-fail loop downstream.

6. Write the implementation report:
   Absolute path: ${worktreePath}/${implementReport}

   Format:
   # Implementation Report — ${stem}

   **Date:** [run: cd ${worktreePath} && date +%Y-%m-%d]
   **Plan:** ${specFile}
   **Scope:** Task ${taskNumber}

   ## What Was Built or Changed
   - [bullet list with file paths]

   ## Files Created or Modified
   | File | Action |
   |---|---|
   | path/to/file.tsx | created / modified |

   ## Validation Output
   **Commands run:**
   \`\`\`
   [one line per command]
   \`\`\`
   **Result:** PASSED / FAILED
   On FAIL only, paste the failing command's last 20 lines (\`... 2>&1 | tail -20\`). On PASS, do NOT
   paste stdout — the Test stage is the authoritative full-output capture, and downstream stages re-run
   the gating checks fresh.
   \`\`\`
   [failing tail -20 — omit this block entirely when PASSED]
   \`\`\`

   ## Decisions and Trade-offs
   [non-obvious choices]

   ## Follow-up Work
   [anything deferred]

   ## git diff --stat
   \`\`\`
   [run: cd ${worktreePath} && git diff --stat]
   \`\`\`

7. Commit your changes. Run from the worktree:
   cd ${worktreePath} && git status
   Stage files explicitly by name (never git add -A or git add .):
     cd ${worktreePath} && git add components/Foo.tsx __tests__/foo.test.ts ${implementReport}
   Commit using HEREDOC:
     cd ${worktreePath} && git commit -m "$(cat <<'EOF'
     feat: implement ${stem}

     EOF
     )"
   Run: cd ${worktreePath} && git log --oneline -1

Return using StructuredOutput:
  reportFile: "${implementReport}"
  success: true if implementation completed without critical errors
  filesModified: array of source files created or modified
  commitHash: 7-character short hash from git log --oneline -1
  filesReadKb: telemetry — before returning, sum the byte size of every file you cat/Read this
    stage (cd ${worktreePath} && wc -c <each file>), divide the total by 1024, and report the number.
  notes: one-line summary
`, { label: 'implement', schema: STAGE_SCHEMA, phase: 'Implement' })
    recordFilesRead(implResult)

    if (!implResult) {
      log('Implement agent returned null — aborting pipeline')
      stageResults.push({ stage: 'implement', success: false, notes: 'Agent returned null' })
      break
    }
    stageResults.push({ stage: 'implement', ...implResult })
    lastImplReport = implResult
    if (!implResult.success) {
      log('Implement reported failure — aborting pipeline')
      break
    }
    currentStage = 'test'
  }

  // ----------------------------------------------------------
  // FIX (review retry path)
  // ----------------------------------------------------------
  if (currentStage === 'fix') {
    phase('Fix')
    const fixPass = reviewAttempts + 1
    log(`Running fix (pass ${fixPass}) — targeting review failures...`)

    // Last fix pass before the loop can give up → escalate the model.
    const fixModel = (ESCALATION_MODEL && fixPass === MAX_REVIEW_ATTEMPTS) ? ESCALATION_MODEL : MODEL.fix
    if (fixModel !== MODEL.fix) log(`Final fix pass — escalating model to ${fixModel}.`)

    const fixResult = await tracedAgent(`${W}
You are the fix agent for the SDLC pipeline. Make targeted fixes for the failures identified
in the last review — NOT a full re-implementation.

Target:
  Spec:                 ${blockId}
  Task:                 Task ${taskNumber}
  Review report:        ${reviewReport}
  Prior implement report: ${implementReport}
  Report to write:      ${implementReport}  ← overwrites this slot (Fix Pass ${fixPass})
  Worktree root:        ${worktreePath}

GATE CHECKS (do these first):
1. cd ${worktreePath} && ls ${reviewReport} 2>/dev/null && echo EXISTS || echo MISSING
   If MISSING → stop, return success: false, notes: "No review report found."
2. cd ${worktreePath} && grep -i "verdict" ${reviewReport} | head -3
   If verdict is PASS → stop, return success: false, notes: "Review verdict is already PASS."

Instructions:

1. Failures to address (structured hand-off from the review — do NOT re-read ${reviewReport}):
   Review verdict: ${handoff(lastReviewResult?.verdict)}
   Criteria NOT_MET / PARTIAL (fix every one):
${handoff(lastReviewResult?.unmetCriteria)}
   Failure reasons / issues found:
${handoff(lastReviewResult?.failureReasons)}
   (The full review report is at ${reviewReport} — read it ONLY if a failure above is ambiguous.)

2. Prior implementation context (structured hand-off — do NOT re-read ${implementReport}):
   Files touched so far:
${handoff(lastImplReport?.filesModified)}
   Prior note: ${handoff(lastImplReport?.notes)}

3. If a breakdown file exists, check the relevant sub-steps for original intent:
   Run: cd ${worktreePath} && ls ${breakdownFile} 2>/dev/null && echo EXISTS || echo MISSING
   If EXISTS: read ${worktreePath}/${breakdownFile} and find the "### Step ${taskNumber}:" section.
   Use it to understand what the original implementation was supposed to do for the failing criterion.
   Do NOT re-implement from scratch — use it only as context for the targeted fix.

4. Make MINIMUM targeted changes to address the failing criteria.
   Fix ONLY what the review identified as failing.

5. Run the Validation Commands from the spec:
   cd ${worktreePath} && cat ${specFile} | grep -A 20 "## Validation Commands"
   Then run those commands.

5.5. SELF-CHECK — completeness gate (do this BEFORE writing the report or committing):
   For EACH criterion the review flagged (step 1), open the actual file and confirm it is now FULLY
   satisfied — do not assume from your own diff. In particular: NO stub remains on a required path
   (\`todo!()\`/\`unimplemented!()\`/\`unreachable!()\`, \`raise NotImplementedError\`,
   \`throw new Error('not implemented')\`, empty \`pass\` bodies, \`TODO\`/\`FIXME\`), every named
   deliverable file exists, and every "unit-tested" criterion has a real hermetic test. Quick grep —
   scope it to the files the FLAGGED criteria require, not every changed file:
     cd ${worktreePath} && grep -nE 'todo!\\(|unimplemented!\\(|unreachable!\\(|NotImplementedError|not implemented|FIXME' <the flagged criteria's paths> 2>/dev/null
   If any flagged criterion is still not met, fix it NOW and re-run step 5 — returning with a known
   remaining gap just burns another review-fail loop.

6. Overwrite the implement report at: ${worktreePath}/${implementReport}

   Format:
   # Fix Pass ${fixPass} — ${stem}

   **Date:** [run: cd ${worktreePath} && date +%Y-%m-%d]
   **Plan:** ${specFile}
   **Fix pass:** ${fixPass}

   ## Failures Addressed
   [each failing criterion and how it was fixed]

   ## Changes Made
   - [targeted changes with file paths]

   ## Files Created or Modified
   | File | Action |
   |---|---|
   [IMPORTANT: include ALL files from prior implement report PLUS newly touched files]

   ## Validation Output
   [commands run, one per line] — Status: PASSED / FAILED
   On FAIL only, paste the failing command's last 20 lines; on PASS, do not paste stdout (the Test
   stage is the authoritative full-output capture).

   ## git diff --stat
   \`\`\`
   [run: cd ${worktreePath} && git diff --stat]
   \`\`\`

7. Commit your changes:
   cd ${worktreePath} && git status
   cd ${worktreePath} && git add <changed files> ${implementReport}
   cd ${worktreePath} && git commit -m "$(cat <<'EOF'
   fix: fix pass ${fixPass} for ${stem}
   EOF
   )"
   cd ${worktreePath} && git log --oneline -1

Return using StructuredOutput:
  reportFile: "${implementReport}"
  success: true if fixes applied and validation passed
  filesModified: files changed this pass only
  commitHash: 7-character short hash
  filesReadKb: telemetry — before returning, sum the byte size of every file you cat/Read this
    stage (cd ${worktreePath} && wc -c <each file>), divide the total by 1024, and report the number.
  notes: one-line summary of what was fixed
`, withModel({ label: `fix-${fixPass}`, schema: STAGE_SCHEMA, phase: 'Fix' }, fixModel))
    recordFilesRead(fixResult)

    if (!fixResult) {
      log('Fix agent returned null — aborting pipeline')
      stageResults.push({ stage: 'fix', attempt: fixPass, success: false, notes: 'Agent returned null' })
      break
    }
    stageResults.push({ stage: 'fix', attempt: fixPass, ...fixResult })
    lastImplReport = fixResult   // fix overwrites the implement-report slot
    if (!fixResult.success) {
      log(`Fix pass ${fixPass} reported failure — aborting pipeline`)
      break
    }
    currentStage = 'test'
  }

  // ----------------------------------------------------------
  // TEST
  // ----------------------------------------------------------
  if (currentStage === 'test') {
    phase('Test')
    log('Running the project validation suite...')

    const testResult = await tracedAgent(`${W}
You are the test agent for the SDLC pipeline. Run the project's validation suite (from
planning/harness.json, or the spec fallback) plus the universal emoji gate, in the worktree.

Target:
  Spec:            ${specFile}
  Report to write: ${testReport}
  Worktree root:   ${worktreePath}

PRE-FLIGHT — Verify all top-level tracked directories exist in this sparse-checkout worktree:
  cd ${worktreePath} && for DIR in $(git ls-tree HEAD --name-only 2>/dev/null); do [ -d "$DIR" ] || echo "MISSING_DIR: $DIR"; done
  If any "MISSING_DIR:" lines appear above, materialize all tracked directories now:
    cd ${worktreePath} && git sparse-checkout reapply && ls -d */ | head -20
  This catches any directory added to main after this worktree was initialized (not just hooks/),
  preventing silent module-resolution failures in the validation checks below.

Run EVERY check below IN ORDER. Capture full output (stdout + stderr) for each.
All commands run from the worktree root (each is already prefixed with cd ${worktreePath}).

Most checks are plain commands that pass iff their exit code is 0. Some checks carry their OWN
pass/fail logic, described inline in the check block (baseline-diff = fail only on net-new items;
count-delta = fail on a count regression, SKIP when there is no previous task; warning-scan = record
pattern matches, gating per the [GATING/non-gating] label; forbidden-pattern scan = fail if any rule
matched). Honor each block's stated logic. A check marked [non-gating] or resolved to SKIP does NOT
block the verdict.

${renderCheckList(harnessCfg, { cwd: worktreePath })}

EMOJI CHECK — Emoji prohibition (universal harness gate — always runs last):
  Hard FAIL if any markdown file changed by THIS TASK introduces an emoji.

  Files modified by THIS TASK vs main (hard FAIL if emoji found):
  cd ${worktreePath} && python3 - <<'PYEOF'
import subprocess, re, sys, os
EMOJI = re.compile(r'[\\U0001F300-\\U0001FAFF\\U00002600-\\U000027BF]')
changed = subprocess.run(['git','diff','main..HEAD','--name-only'], capture_output=True, text=True).stdout.splitlines()
md_files = [f for f in changed if f.endswith(('.md','.mdx')) and os.path.isfile(f)]
hits = []
for path in md_files:
    for n, line in enumerate(open(path, errors='ignore'), 1):
        if EMOJI.search(line):
            hits.append(f'{path}:{n}: {line.rstrip()[:100]}')
if hits:
    print('EMOJI CHECK FAIL: emoji in modified files (violates the no-emoji harness rule):')
    for h in hits[:25]: print(h)
    sys.exit(1)
print('EMOJI CHECK: OK — no emoji in modified files')
sys.exit(0)
PYEOF
  echo "EMOJI_EXIT:$?"

Note in the report any project-specific standing-rule checks the spec or CLAUDE.md calls for
(the project's own validation suite covers most of these).

Write the test report to: ${worktreePath}/${testReport}

Format:
# Test Report — ${stem}

**Date:** [run: date +%Y-%m-%d]
**Spec:** ${specFile}
**Scope:** Task ${taskNumber}

## Summary

| Test | Result | Error |
|---|---|---|
[FAILED rows first, then PASSED rows]

## Full Results (JSON)
\`\`\`json
[array of {test_name, passed, execution_command, test_purpose, error}]
\`\`\`
For each entry: a PASSED check stores \`error: ""\` (empty — never paste passing stdout); a FAILED
check stores ONLY the last 20 lines of its failing output in \`error\`. Review re-runs the gating
checks fresh, so the full passing transcript is never read downstream.

Return using StructuredOutput:
  reportFile: "${testReport}"
  allPassed: true only if every GATING check passed and the emoji gate is clean. [non-gating] checks
    and SKIPPED checks (e.g. count-delta on task 1) never set allPassed false on their own — but DO
    record their result.
  passCount: integer count of checks that passed (count SKIPPED as passed)
  failCount: integer count of checks that failed
  failedTests: array of test_name strings for checks that failed (include non-gating failures here too,
    flagged as non-gating, so they surface in the report even though they do not block the verdict)
  notes: one-line summary
`, withModel({ label: 'test', schema: TEST_SCHEMA, phase: 'Test' }, MODEL.test))

    if (!testResult) {
      log('Test agent returned null — recording failure, continuing to review')
      lastTestReport = { allPassed: false, passCount: 0, failCount: 0, failedTests: ['test agent returned null'] }
      stageResults.push({ stage: 'test', attempt: reviewAttempts + 1, allPassed: false, success: false, notes: 'Agent returned null' })
    } else {
      lastTestReport = testResult
      stageResults.push({ stage: 'test', attempt: reviewAttempts + 1, ...testResult, success: testResult.allPassed })
      if (!testResult.allPassed) {
        log(`Test failures (${testResult.failCount}): ${(testResult.failedTests || []).join(', ')}`)
      } else {
        log(`All ${testResult.passCount} checks passed`)
      }
    }
    currentStage = 'review'
  }

  // ----------------------------------------------------------
  // REVIEW
  // ----------------------------------------------------------
  if (currentStage === 'review') {
    phase('Review')
    reviewAttempts++
    log(`Running review (attempt ${reviewAttempts}/${MAX_REVIEW_ATTEMPTS})...`)

    // Final review attempt before the loop can give up → escalate the model.
    const reviewModel = (ESCALATION_MODEL && reviewAttempts === MAX_REVIEW_ATTEMPTS) ? ESCALATION_MODEL : MODEL.review
    if (reviewModel !== MODEL.review) log(`Final review attempt — escalating model to ${reviewModel}.`)

    const reviewResult = await tracedAgent(`${W}
You are the review agent for the SDLC pipeline. Verify the implementation against the spec.

Target:
  Spec:             ${blockId}
  Task:             Task ${taskNumber}
  Spec file:        ${specFile}
  Implement report: ${implementReport}
  Test report:      ${testReport}
  Report to write:  ${reviewReport}
  Worktree root:    ${worktreePath}

Instructions:

1. Read the spec file:
   cd ${worktreePath} && cat ${specFile}
   Extract the COMPLETE "## Acceptance Criteria" section.

2. Implementation hand-off (structured — do NOT re-read ${implementReport}; this is its summary):
   Files modified by the implementation:
${handoff(lastImplReport?.filesModified)}
   Commit: ${handoff(lastImplReport?.commitHash)}
   Implementer's note: ${handoff(lastImplReport?.notes)}
   Use the files-modified list to know WHERE to look — you verify against real source (step 5),
   not against this self-report. (Full report at ${implementReport} only if a claim is ambiguous.)

3. Test hand-off (structured — do NOT re-read ${testReport}; you re-run the checks yourself next):
   All gating checks passed: ${handoff(lastTestReport?.allPassed)}
   Passed: ${handoff(lastTestReport?.passCount)}   Failed: ${handoff(lastTestReport?.failCount)}
   Failed checks: ${handoff(lastTestReport?.failedTests)}

4. Run FRESH authoritative checks (this result determines the verdict, not the test report):
   Re-run each GATING validation check — those whose test_purpose in the test report (${testReport})
   is marked "GATING", i.e. the checks with gates:true in planning/harness.json. Use each check's
   execution_command verbatim, from the worktree root:
   cd ${worktreePath} && <gating check command>
   If the project ships no harness suite, re-run the spec's "## Validation Commands". A fresh
   failure of any gating check ALWAYS prevents PASS.

5. Scope your review to Task ${taskNumber} only.
   The spec may list acceptance criteria spanning multiple tasks. For each criterion:
   - If tagged for a different task (e.g. "[T5]", "(Task 5)") OR clearly belongs to a later
     task's scope (the work it describes is not in Task ${taskNumber}'s step list) →
     mark SKIP with a note. SKIP criteria do NOT affect the verdict.
   - All others: evaluate normally.

   For each in-scope criterion, read relevant source files and determine:
   MET — fully satisfied by this task's changes
   PARTIAL — partially satisfied
   NOT_MET — not satisfied (counts as a verdict failure)
   Also check the project's CLAUDE.md standing-rules compliance — a violation is a failing
   criterion. CLAUDE.md is the authority: do not assume any stack, locale-parity, narrative, or
   content-layout rule unless written there. Universal harness rules always apply: no fabricated
   metrics or quotes, no emoji, every change ships with tests.
   IDENTITY INTEGRITY: flag any handle, profile link, or URL that contradicts the verified
   identities/handles declared in CLAUDE.md, or that appears fabricated. Mark such a criterion
   NOT_MET — only the CLAUDE.md-declared identities are authoritative.

5.5. HARD RULE — do NOT fix environment or infrastructure issues yourself:
   If a fresh gating check fails due to environment/infrastructure causes (missing module files,
   sparse-checkout gaps, missing hooks, missing directories), do NOT fix them yourself. Return
   verdict: FAIL with failureReasons: ["Environment issue — missing files or sparse-checkout gap;
   the fix agent must resolve them and re-run the pipeline."]. A review agent that resolves
   infrastructure issues itself bypasses the test gate that validates the fix.

6. Determine verdict:
   PASS — ALL criteria MET AND every fresh gating check passes (exit 0)
   PARTIAL — some criteria PARTIAL, OR gating checks pass but some criteria not fully met
   FAIL — any criterion NOT_MET, OR any fresh gating check fails
   (A fresh gating-check failure ALWAYS prevents PASS)

7. Write the review report: ${worktreePath}/${reviewReport}

   Format:
   # Review Report — ${stem}

   **Date:** [run: date +%Y-%m-%d]
   **Spec:** ${specFile}
   **Scope:** Task ${taskNumber}
   **Verdict:** PASS / PARTIAL / FAIL

   ## Acceptance Criteria Check
   | Criterion | Status | Evidence |
   |---|---|---|
   | [criterion] | MET / PARTIAL / NOT_MET | [file:line or test name] |

   ## Fresh Test Results
   [the fresh gating-check output — per-check pass/fail and any failure output]

   ## Verdict: PASS / PARTIAL / FAIL
   [one paragraph explaining the verdict]

   ## Issues Found
   [specific problems — empty if PASS]

   ## Next Steps
   [what to do based on verdict]

Return using StructuredOutput:
  reportFile: "${reviewReport}"
  verdict: "PASS", "FAIL", or "PARTIAL"
  failureReasons: array of strings (empty if PASS)
  unmetCriteria: array of criterion texts that were NOT_MET or PARTIAL (empty if PASS)
  filesReadKb: telemetry — before returning, sum the byte size of every file you cat/Read this
    stage (cd ${worktreePath} && wc -c <each file>), divide the total by 1024, and report the number.
  notes: one-line summary
`, withModel({ label: `review-${reviewAttempts}`, schema: REVIEW_SCHEMA, phase: 'Review' }, reviewModel))
    recordFilesRead(reviewResult)

    if (!reviewResult) {
      log(`Review agent returned null (attempt ${reviewAttempts}) — treating as FAIL`)
      lastReviewResult = { verdict: 'FAIL', failureReasons: ['Review agent returned null'], unmetCriteria: [], reportFile: reviewReport }
      stageResults.push({ stage: 'review', attempt: reviewAttempts, verdict: 'FAIL', success: false, notes: 'Agent returned null' })
    } else {
      lastReviewResult = reviewResult
      stageResults.push({ stage: 'review', attempt: reviewAttempts, ...reviewResult, success: reviewResult.verdict === 'PASS' })
      log(`Review verdict: ${reviewResult.verdict} (attempt ${reviewAttempts}/${MAX_REVIEW_ATTEMPTS})`)
    }

    if (lastReviewResult.verdict === 'PASS') {
      currentStage = 'ui-test'
    } else if (reviewAttempts < MAX_REVIEW_ATTEMPTS) {
      log(`Review ${lastReviewResult.verdict} — running fix pass ${reviewAttempts + 1}/${MAX_REVIEW_ATTEMPTS}...`)
      currentStage = 'fix'
    } else {
      log(`Review FAILED after ${MAX_REVIEW_ATTEMPTS} attempts — skipping to wrap-up with FAIL status`)
      currentStage = 'wrap-up'
    }
  }

  // ----------------------------------------------------------
  // UI TEST (after Review PASS — browser smoke check)
  // ----------------------------------------------------------
  if (currentStage === 'ui-test') {
    phase('UI Test')

    if (!harnessCfg?.uiTest?.enabled) {
      log('UI test stage disabled (harness.json uiTest.enabled is false or config absent) — SKIPPED.')
      stageResults.push({ stage: 'ui-test', verdict: 'SKIPPED', success: true, notes: 'uiTest disabled in harness.json' })
      currentStage = 'document'
    } else {
      log('Running UI test stage...')
      // Parallel-safe: each task gets a unique port (base port + task number).
      const devPort = (harnessCfg.uiTest.port ?? 3000) + taskNumber
      const ui = renderUiTestPrompt(harnessCfg, devPort)

      const uitestResult = await tracedAgent(`${W}
You are the UI test agent for the SDLC pipeline. Run a quick live browser smoke check using
playwright-cli to catch visual/runtime regressions that the validation suite cannot catch.

Target:
  Spec:              ${blockId}
  Task:              Task ${taskNumber}
  Implement report:  ${implementReport}
  Report to write:   ${uitestReport}
  Dev server URL:    http://localhost:${ui.port}
  Worktree root:     ${worktreePath}

STEP 1 — Triage: did this task change application source?

  Read the implement report:
    cd ${worktreePath} && cat ${implementReport}
  Scan the "Files Modified" list. If EVERY changed file is documentation/markdown or planning
  metadata only (no application source), set verdict = SKIPPED, write the report, and stop.
  Otherwise continue to STEP 2.

STEP 2 — Start the dev server on port ${ui.port} (unique per task to avoid conflicts).

  Check if port ${ui.port} is already in use:
    lsof -ti :${ui.port} 2>/dev/null && echo "PORT_IN_USE" || echo "PORT_FREE"

  If PORT_IN_USE: the server is already running — skip to STEP 3.
  If PORT_FREE: start the server in the background from the worktree:
    cd ${worktreePath} && PORT=${ui.port} ${ui.devCmd} > /tmp/uitest-${stem}.log 2>&1 &
    echo "SERVER_PID=$!"

  Wait up to 60 seconds for the ready signal:
    for i in $(seq 1 30); do grep -q "${ui.ready}" /tmp/uitest-${stem}.log 2>/dev/null && echo "READY" && break; sleep 2; done
    tail -20 /tmp/uitest-${stem}.log

  If "READY" not seen within 60 s, write the report with verdict = FAIL (dev server did not start),
  kill the background process, and stop.

STEP 3 — Run smoke checks using playwright-cli, one per configured route.

  Open a browser session:
    playwright-cli open http://localhost:${ui.port}${ui.routes[0]}

  For each route below, record PASS, WARN, or FAIL with quoted evidence:

${ui.routeChecks}

  Also confirm at least one internal link works: from any route's snapshot, pick an internal link
  and \`playwright-cli click <ref>\`, then \`playwright-cli snapshot\` — the target must load without
  an error page.

  Close the browser session:
    playwright-cli close

STEP 4 — Kill the dev server (only if YOU started it in STEP 2).
  If SERVER_PID was captured: kill $SERVER_PID 2>/dev/null || true

STEP 5 — Determine verdict and write report.

  Verdict rules:
  - PASS:    All route checks passed with no errors.
  - WARN:    All checks passed but console warnings were found.
  - FAIL:    One or more checks failed — list each with quoted evidence.
  - SKIPPED: No application source changed (from STEP 1 triage).

  Write the report to ${uitestReport} (absolute: ${worktreePath}/${uitestReport}):
  \`\`\`markdown
  # UI Test Report: ${stem}

  **Verdict:** <PASS|WARN|FAIL|SKIPPED>
  **Date:** <today>

  ## Smoke Check Results

  | Check | Result | Notes |
  |---|---|---|
${ui.routeRows}

  ## Summary
  <one paragraph — what was tested and what was found>
  \`\`\`

  Commit the report:
    cd ${worktreePath} && git add ${uitestReport} && git commit -m "test(ui): ui smoke check for ${stem}"

Return the result using StructuredOutput.
`, withModel({ label: 'ui-test', schema: UI_TEST_SCHEMA, phase: 'UI Test' }, MODEL.uiTest))

    if (!uitestResult) {
      log('UI test agent returned null — treating as WARN, continuing to document')
      stageResults.push({ stage: 'ui-test', verdict: 'WARN', success: true, notes: 'Agent returned null' })
      currentStage = 'document'
    } else {
      stageResults.push({ stage: 'ui-test', ...uitestResult, success: uitestResult.verdict !== 'FAIL' })
      log(`UI test verdict: ${uitestResult.verdict}`)

      if (uitestResult.verdict === 'FAIL') {
        if (reviewAttempts < MAX_REVIEW_ATTEMPTS) {
          log(`UI test FAIL — running fix pass ${reviewAttempts + 1}/${MAX_REVIEW_ATTEMPTS}...`)
          currentStage = 'fix'
        } else {
          log(`UI test FAILED after ${MAX_REVIEW_ATTEMPTS} attempts — skipping to wrap-up`)
          currentStage = 'wrap-up'
        }
      } else {
        currentStage = 'document'
      }
    }
    }
  }
} // end implement→fix→test→review→ui-test retry loop

// ================================================================
// PHASE 6: DOCUMENT (gates on PASS verdict)
// ================================================================
if (currentStage === 'document') {
  phase('Document')
  log('Running document stage...')

  const docResult = await tracedAgent(`${W}
You are the documentation agent for the SDLC pipeline. Surgically patch docs/ in the worktree.

Target:
  Spec:             ${blockId}
  Task:             Task ${taskNumber}
  Review report:    ${reviewReport}
  Implement report: ${implementReport}
  Report to write:  ${documentReport}
  Worktree root:    ${worktreePath}

Instructions:

1. Read the review report:
   cd ${worktreePath} && cat ${reviewReport}
   GATE CHECK: If the verdict is FAIL or PARTIAL, stop immediately.
   Return: success: false, notes: "Blocked — review verdict was not PASS".

2. Read the implement report:
   cd ${worktreePath} && cat ${implementReport}
   Find the "## Files Created or Modified" table.

3. For each source file in that table, find which docs/*.md files reference it:
   cd ${worktreePath} && grep -rl "ComponentName\\|functionName\\|filename" docs/ 2>/dev/null

4. Read each relevant doc file and surgically patch ONLY affected sections:
   - Update component signatures, prop lists, descriptions that changed
   - Add documentation for new public APIs / lib utilities
   - Never delete documented items that still exist
   - Use the Edit tool with absolute paths: ${worktreePath}/docs/filename.md

5. If a change touched core wiring (entry points, shared modules, routing/config) and an
   architecture/patterns doc would need updating, add that doc to NEEDS_REVIEW in the
   document report but do NOT edit it directly.

6. Write the document report: ${worktreePath}/${documentReport}

   Format:
   # Documentation Report — ${stem}

   **Date:** [run: date +%Y-%m-%d]
   **Spec:** ${specFile}
   **Verdict gate:** PASS (confirmed)

   ## Docs Patched
   | Doc File | Section Updated | Change Summary |
   |---|---|---|

   ## Docs Flagged NEEDS_REVIEW
   [list any docs needing human review]

   ## Docs Clean (no changes needed)
   [docs checked but unchanged]

7. Commit your changes:
   cd ${worktreePath} && git status
   If no doc files were patched, commit just the report:
     cd ${worktreePath} && git add ${documentReport}
   If docs were patched:
     cd ${worktreePath} && git add docs/file1.md docs/file2.md ${documentReport}
   cd ${worktreePath} && git commit -m "$(cat <<'EOF'
   docs: update docs for ${stem}
   EOF
   )"
   cd ${worktreePath} && git log --oneline -1

Return using StructuredOutput:
  reportFile: "${documentReport}"
  success: true if docs checked and report written (even if no changes needed)
  filesModified: doc files actually patched (empty if none)
  commitHash: 7-character short hash
  notes: one-line summary
`, withModel({ label: 'document', schema: STAGE_SCHEMA, phase: 'Document' }, MODEL.document))

  if (!docResult) {
    stageResults.push({ stage: 'document', success: false, notes: 'Document agent returned null' })
    log('Document agent returned null')
  } else {
    stageResults.push({ stage: 'document', ...docResult })
    if (!docResult.success) {
      log(`Document stage blocked: ${docResult.notes}`)
    } else {
      log(`Docs updated: ${(docResult.filesModified || []).join(', ') || 'none needed changes'}`)
    }
  }
  currentStage = 'wrap-up'
}

// ================================================================
// PHASE 7: WRAP-UP — write task log (deferred status/log) + finalize
// ================================================================
phase('Wrap-up')

const finalVerdict = lastReviewResult?.verdict || 'NOT_REACHED'
const stageResultsSummary = stageResults
  .map(r => `${r.stage}${r.attempt ? `(#${r.attempt})` : ''}: ${r.success ? (r.verdict || 'OK') : 'FAILED'}`)
  .join(' → ')

log(`Wrap-up. Final verdict: ${finalVerdict}. Pipeline: ${stageResultsSummary}`)

// ----------------------------------------------------------------
// TASK LOG: write deferred status/log content — do NOT touch those files
// ----------------------------------------------------------------
log('Writing task log (status/log deferred to merge time)...')

const logResult = await tracedAgent(`${W}
You are the task-log agent for the SDLC pipeline.

Your job is to write a structured task log file that records what status.md and log.md
should be updated to. Do NOT modify planning/status.md or log.md directly — those files
are updated when the worktree branch is merged into main via /clean-worktree.

Target:
  Spec:             ${blockId}
  Task:             ${taskNumber}
  Final verdict:    ${finalVerdict}
  Review attempts:  ${reviewAttempts}
  Pipeline summary: ${stageResultsSummary}
  Branch:           ${branchName}
  Log file:         ${logFile}
  Worktree root:    ${worktreePath}

Instructions:

1. Read the spec file to identify the NEXT task after ${taskNumber}:
   cd ${worktreePath} && cat ${specFile}
   Look for the section "### ${taskNumber + 1}." to get the next task's title.
   If no task ${taskNumber + 1} exists, note "spec complete".

2. Read current status.md to understand the progress-table notes format (title-case status):
   cd ${worktreePath} && head -30 planning/status.md

3. Get the git log for this pipeline run:
   cd ${worktreePath} && git log --oneline main..HEAD 2>/dev/null || git log --oneline -8

4. Run to get today's date:
   date +%Y-%m-%d

5. Write the task log file: ${worktreePath}/${logFile}

   Use EXACTLY this format (fill in all bracketed values):

   # Task Log — ${blockId} task ${taskNumber}

   **Spec:** ${blockId}
   **Task:** ${taskNumber}
   **Verdict:** ${finalVerdict}
   **Date:** [today's date from step 4]
   **Branch:** ${branchName}
   **Applied:** false

   ---

   ## status.md — Spec Status
   [Only include this section if the spec was "Not started" when this task ran (blockStatus: ${scout.blockStatus}).
    Write: "In progress" to flip it. Omit this section entirely if it was already In progress or Done.]

   ## status.md — Current Focus Line
   [The COMPLETE replacement string for the "Current focus:" line.
    If task ${taskNumber + 1} exists: "${blockId} — Task ${taskNumber + 1}: [task title]"
    If spec complete: the next spec's focus line]

   ## status.md — Last Updated Line
   [The COMPLETE replacement string for the "Last updated:" line.
    Format: "YYYY-MM-DD — ${blockId} in progress (Tasks 1–${taskNumber} complete; Tasks ${taskNumber + 1}–N next — [brief description])"]

   ## status.md — Notes Column
   [The updated Notes column text for this spec's row in the progress table.
    Summarizes which tasks are done and which remain.]

   ---

   ## Log Entry

   ## [today's date] (task ${taskNumber} — [brief description matching the task])

   [One paragraph: what was implemented or tested, how review went (${finalVerdict} verdict${reviewAttempts > 1 ? ` after ${reviewAttempts} attempts` : ''}), notable findings. End with: "Next: Task ${taskNumber + 1} — [next task description]."]

   \`\`\`
   [paste the git log --oneline output from step 3]
   \`\`\`

6. Do NOT commit the log file — the finalize agent will commit it with all other reports.

Return using StructuredOutput:
  logFile: "${logFile}"
  applied: false
  nextFocus: the exact Current Focus Line string you wrote to the log
  notes: any settled decisions that should be added to planning/decisions/
`, withModel({ label: 'task-log', schema: LOG_SCHEMA, phase: 'Wrap-up' }, MODEL.taskLog))

if (logResult) {
  stageResults.push({ stage: 'task-log', ...logResult, success: true })
  log(`Task log written: ${logFile}`)
  if (logResult.notes) log(`Decisions to log manually: ${logResult.notes}`)
} else {
  stageResults.push({ stage: 'task-log', success: false, notes: 'Agent returned null' })
  log('Task-log agent returned null — log file may need manual creation')
}

// ----------------------------------------------------------------
// FINALIZE: workflow report + commit all reports + print merge instructions
// ----------------------------------------------------------------
log('Running finalize: workflow report + commit...')

const stageTable = stageResults.map(r => {
  const label  = r.stage + (r.attempt ? ` (attempt ${r.attempt})` : '')
  const status = r.verdict ? r.verdict : (r.success ? 'completed' : 'FAILED')
  const file   = r.reportFile || r.workflowReportFile || r.logFile || '—'
  const commit = r.commitHash ? r.commitHash.substring(0, 7) : '—'
  const notes  = (r.notes || '').substring(0, 60)
  return `| ${label} | ${status} | ${file} | ${commit} | ${notes} |`
}).join('\n')

// Token-telemetry table (Phase A). The finalize agent's own line is absent — this table is
// built before it runs — which is fine: finalize is a cheap, fixed Haiku stage.
const metricsTable = metrics.map(m => {
  // Under a parallel wave the shared-pool delta is contaminated by concurrent siblings (D12) — mark
  // it non-isolated instead of reporting a misleading number. promptTok/filesReadKb stay accurate.
  const out  = parallelWave ? '— (parallel)' : (m.outTok != null ? String(m.outTok) : '—')
  const read = m.filesReadKb != null ? `${Math.round(m.filesReadKb)} KB` : '—'
  return `| ${m.label} | ${m.model} | ${m.promptTokEst} | ${out} | ${read} |`
}).join('\n')
// Legend caveat: only present when outTok was suppressed for parallel contamination.
const metricsCaveat = parallelWave
  ? '\n\n> **outTok suppressed ("— (parallel)").** This task ran in a parallel wave under /sdlc-block; outTok is a shared-pool delta contaminated by concurrent sibling tasks, so a per-stage number would mislead. promptTok and filesReadKb are per-agent and accurate. See decisions/D12.'
  : ''
log(`Token metrics (stage | model | promptTok | outTok | filesReadKb):\n${metricsTable}`)

const finalizeResult = await tracedAgent(`${W}
You are the finalize agent for the SDLC pipeline.

IMPORTANT: Do NOT modify planning/status.md or log.md. Those are applied at merge time.
Your chore commit includes ONLY: test report, review report, document report, task log,
and the workflow report you write here.

Target:
  Spec:            ${blockId}
  Task:            Task ${taskNumber}
  Final verdict:   ${finalVerdict}
  Worktree root:   ${worktreePath}
  Branch:          ${branchName}
  Workflow report: ${workflowReport}

Stage results so far:
${stageResultsSummary}

STEP 1 — Get the commit history from this pipeline run:
  cd ${worktreePath} && git log --oneline -15

STEP 2 — Write the workflow report: ${worktreePath}/${workflowReport}

  Format:
  # SDLC Workflow Report — ${blockId} Task ${taskNumber}

  **Date:** [run: date +%Y-%m-%d]
  **Spec:** ${blockId}
  **Task scope:** Task ${taskNumber}
  **Pipeline started from:** ${scout.startStage}
  **Review attempts:** ${reviewAttempts} of ${MAX_REVIEW_ATTEMPTS} max
  **Worktree:** ${worktreePath}
  **Branch:** ${branchName}

  ## Final Verdict
  ${finalVerdict} — [one sentence explanation]

  ## Stage Results

  | Stage | Status | Report | Commit | Notes |
  |---|---|---|---|---|
  ${stageTable}

  (The ## Token Metrics section is appended verbatim in STEP 2b — do NOT write it here.)

  ## Key Findings
  [what was implemented, notable decisions, content/bilingual-parity notes]

  ## Files Modified
  [source files created or modified — from the implement report]

  ## Docs Updated
  [doc files patched — from the document report; NEEDS_REVIEW flags]

  ## Commits (this pipeline run)
  [relevant lines from git log --oneline]

  ## Next Step
  To merge this task into main and apply status/log updates:
    /clean-worktree ${branchName}

STEP 2b — Append the Token Metrics section to the report you just wrote. Run this EXACTLY as written
(a literal heredoc append). Do NOT retype, summarize, reorder, or omit the table — it is
machine-generated telemetry and must land verbatim:
  cd ${worktreePath} && cat >> ${workflowReport} <<'METRICS_EOF'

## Token Metrics
Per-stage attribution (promptTok = injected input estimate; outTok = output-token delta, "—" when no
+Nk budget target was set; filesReadKb = stage-reported ingestion estimate).${metricsCaveat}

| Stage | Model | promptTok | outTok | filesReadKb |
|---|---|---|---|---|
${metricsTable}
METRICS_EOF

STEP 3 — Commit the report files. Never use git add -A or git add .

  Run: cd ${worktreePath} && git status
  Stage ONLY report files (NOT status.md or log.md — never touch those in the worktree):
    cd ${worktreePath} && git add ${testReport} 2>/dev/null || true
    cd ${worktreePath} && git add ${reviewReport} 2>/dev/null || true
    cd ${worktreePath} && git add ${uitestReport} 2>/dev/null || true
    cd ${worktreePath} && git add ${documentReport} 2>/dev/null || true
    cd ${worktreePath} && git add ${logFile} 2>/dev/null || true
    cd ${worktreePath} && git add ${workflowReport}

  Commit using HEREDOC:
    cd ${worktreePath} && git commit -m "$(cat <<'EOF'
    chore: wrap up ${stem}
    EOF
    )"
  cd ${worktreePath} && git log --oneline -1

STEP 4 — Print the merge instructions EXACTLY as shown:

  ╔══════════════════════════════════════════════════════════════════╗
  ║  Pipeline complete: ${stem}
  ║  Verdict: ${finalVerdict}
  ║
  ║  Worktree: ${worktreePath}
  ║  Branch:   ${branchName}
  ║
  ║  To merge and apply status/log updates, run from main session:
  ║    /clean-worktree ${branchName}
  ╚══════════════════════════════════════════════════════════════════╝

Return using StructuredOutput:
  workflowReportFile: "${workflowReport}"
  commitMessage: "chore: wrap up ${stem}"
  commitHash: 7-character short hash from git log --oneline -1
  notes: any follow-up items (planning/decisions/ entries, NEEDS_REVIEW doc flags)
`, withModel({ label: 'finalize', schema: FINALIZE_SCHEMA, phase: 'Wrap-up' }, MODEL.finalize))

if (finalizeResult) {
  stageResults.push({ stage: 'finalize', ...finalizeResult, success: true })
  log(`Committed: ${finalizeResult.commitMessage}`)
  log(`Workflow report: ${finalizeResult.workflowReportFile}`)
} else {
  stageResults.push({ stage: 'finalize', success: false, notes: 'Finalize agent returned null' })
  log('Finalize agent returned null — manual commit may be needed')
}

log(`Pipeline complete. Verdict: ${finalVerdict} | Worktree: ${worktreePath} | Branch: ${branchName}`)
log(`To merge: /clean-worktree ${branchName}`)
log(`IMPORTANT: If running multiple tasks in parallel, merge them in task-number order.`)
log(`Merging out of order will cause status.md "Current focus" to point to the wrong next task.`)

return {
  blockId,
  taskNumber,
  stem,
  branchName,
  worktreePath,
  finalVerdict,
  reviewAttempts,
  startStage: scout.startStage,
  workflowReport: finalizeResult?.workflowReportFile || workflowReport,
  logFile,
  mergeCommand: `/clean-worktree ${branchName}`,
  stageResults
}
