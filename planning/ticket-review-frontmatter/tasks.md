# Ticket: Review and Update Auto-injected OKF Frontmatter

## Metadata
prompt: `review the missing frontmatter and update as needed`
status: Not started
last-run: never

## Description
During a bulk cleanup of validation errors, 20 markdown files were automatically patched with generic filler OKF frontmatter (`type: Reference`, auto-generated titles, and generic descriptions) to pass `mev` validation. These files need to be systematically reviewed and their frontmatter properties (`type`, `title`, `description`, `doc_id`, `project`, `layer`) updated to accurately reflect the document's actual purpose and scope.

## Relevant Files
- `core/orchestrator/planning/sdlc-workflow-architecture/synthesis.md`
- `core/orchestrator/planning/sdlc-workflow-architecture/architect-prompt.md`
- `portfolio/workflow-engine-rs/planning/context.md`
- `portfolio/claude-sdk-rs/planning/context.md`
- `portfolio/claude-sdk-rs/docs/TROUBLESHOOTING.md`
- `portfolio/claude-sdk-rs/docs/TESTING.md`
- `portfolio/claude-sdk-rs/docs/NVM_COMPATIBILITY.md`
- `portfolio/claude-sdk-rs/docs/SECURITY.md`
- `portfolio/claude-sdk-rs/docs/tutorials/00-command_line_sdk_overview.md`
- `portfolio/claude-sdk-rs/docs/tutorials/01-getting-started.md`
- `portfolio/claude-sdk-rs/docs/tutorials/02-basic-usage.md`
- `portfolio/claude-sdk-rs/docs/tutorials/03-configuration.md`
- `portfolio/claude-sdk-rs/docs/tutorials/04-streaming-responses.md`
- `portfolio/claude-sdk-rs/docs/tutorials/05-tool-integration.md`
- `portfolio/claude-sdk-rs/docs/tutorials/06-session-management.md`
- `portfolio/claude-sdk-rs/docs/tutorials/07-advanced-usage.md`
- `portfolio/rag-engine-rs/planning/context.md`
- `portfolio/rag-engine-rs/planning/help-docs-ai-masterplan.md`
- `portfolio/rag-engine-rs/planning/repo-structure.md`
- `client/brazilianportugui/planning/artifacts/Brandon - PortuGui Site.md`

### New Files
None

## Step by Step Tasks
IMPORTANT: Execute every step in order, top to bottom.

### 1. Identify Auto-patched Files
- Files: (Repo-wide)
- Search the repository for the filler string pattern `layer: [meta]` and `description: Documentation for` to compile the complete list of files needing review.

### 2. Update Frontmatter Properties
- Files: (Files identified in step 1)
- For each file, read its content and context, then update the OKF frontmatter fields to accurately describe the document according to the OKF schema constraints. Remove or fix any invalid `project` slugs.

### 3. Validate
- Run the Validation Commands listed below and confirm all pass.

## Testing Strategy
This is a metadata-only change. Correctness is asserted by running the OKF frontmatter schema validator over the entire brain graph and ensuring 0 validation and schema errors remain.

## Acceptance Criteria
- All 20 auto-injected filler frontmatter blocks are replaced with accurate, document-specific OKF metadata.
- The `mev validate-brain` pass throws zero `frontmatter`, `project`, `doc_id`, or `keywords` schema errors.

## Validation Commands
```bash
cargo run --release -- validate-brain --links /Users/brandon/Dev/agentic-portfolio
```

## Notes
- Refer to the company-brain `docs/okf-frontmatter.md` (or the `brain.toml` vocab) for the controlled vocabulary of `layer`, `status`, and `project`.

## Amendment Log
<!-- Append-only. Pipeline stages append one dated line here when they deviate from the plan. -->
_No amendments yet._
