# ADR-0007: Adopt Paperclip's vocabulary and task lifecycle verbatim

- **Date:** 2026-07-19
- **Status:** accepted

## Context

Elia set the rule: follow Paperclip to the letter wherever it serves us (docs/PAPERCLIP-ALIGNMENT.md). Studying the actual Paperclip source (local clone) revealed our M1 vocabulary had drifted from their canon: we had `Organization`/`Mission`/`Ticket` where Paperclip canonizes **company**, **project**+**goal**, and **task** (their `DESIGN.md` explicitly bans "ticket"/"issue" in product copy), with statuses `backlog/todo/in_progress/in_review/blocked/done/cancelled` and priorities `low/medium/high/urgent`.

## Decision

Rename before first commit of M1 code, across schema, domain, API and docs:

- `Organization` → **Company** (`/companies` endpoints)
- `Mission` → **Project** (goals now belong to projects)
- `Ticket` → **Task** (`/tasks` endpoints), statuses and default (`backlog`) adopted verbatim, `blocked` added to the state machine, `priority` column added
- Audit event kinds follow: `company.created`, `project.created`, `task.created`, `task.transitioned`

Deliberate deviation kept: our audit table stays `audit_events` (not `activity_log`) — see PAPERCLIP-ALIGNMENT.md deviations.

## Alternatives considered

- **Keep our names, map at the UI layer** — permanent translation tax between code and product language, guaranteed drift. Rejected.
- **Adopt their legacy DB name `issues`** — they are actively renaming away from it; adopting the target canon ("task") is more faithful than adopting their debt. Rejected.

## Consequences

- Renaming cost was ~zero because it happened before the M1 commit; this is why PAPERCLIP-ALIGNMENT.md now mandates studying their source *before* each milestone, not after.
- Future features (heartbeats→`routines`, budgets, approvals, config revisions) must start from their schema files, listed in PAPERCLIP-ALIGNMENT.md.
