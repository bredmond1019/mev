# Wrap Up — Log work and commit at the end of a session.

Runs `/log-work` then `/commit` in sequence. Use this after finishing a piece of work to
sync status.md, append a log entry, and commit everything cleanly. For a full end-of-context
hand-off to a fresh agent, use `/handoff` instead.

## Variables

$ARGUMENTS — optional free-text note about what was done (passed straight through to
             `/log-work` as its narrative). May be brief ("shipped D17 --from flag") or
             detailed. If omitted, `/log-work` derives the narrative from git history.

## Instructions

1. Run `/log-work $ARGUMENTS` — this syncs status.md, appends the log entry, and syncs
   the company brain. Wait for it to complete before continuing.

2. Run `/commit` — stages and commits all remaining changes with a conventional message.

That's it. No handoff file, no context summary — just log + commit.
