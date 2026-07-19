# Paperclip Alignment

> **Rule (set by Elia, 2026-07-19): follow Paperclip to the letter wherever it serves us.** Before designing any feature Paperclip also has, study how Paperclip does it in the local reference clone. Deviate only deliberately: every deviation is recorded here and, if architectural, in an ADR.

## Reference clone

`/Volumes/ExtremeSSD/references/paperclip` — read-only, MIT-licensed. Refresh with `git pull` when needed. Key entry points:

| What | Where |
|---|---|
| Design principles (UI canon, naming) | `DESIGN.md` |
| DB schema (the real data model) | `packages/db/src/schema/` |
| Server logic | `server/src/` |
| Their roadmap | `ROADMAP.md` |

## Vocabulary map (canon adopted 2026-07-19)

| Concept | Paperclip canon | Overmind uses | Notes |
|---|---|---|---|
| Tenant/org entity | `companies` | **Company** | was `Organization` before alignment |
| Unit of work | product copy: **task** (DB legacy: `issues`, being renamed) | **Task** | was `Ticket`; we adopt the *target* term, not their legacy DB name |
| Task statuses | `backlog, todo, in_progress, in_review, blocked, done, cancelled` | same | adopted verbatim, incl. default `backlog` |
| Task priority | `low, medium, high, urgent` (default `medium`) | same | |
| Work grouping | `projects` + `project_goals` | **Project** + **Goal** | was `Mission`/`Goal` |
| Audit trail | `activity_log` | **audit_events** | deliberate deviation: our name reflects the hash-chain guarantee Paperclip doesn't have (ADR-0006) |
| Agent wake-ups | `routines`, `agent_wakeup_requests` | heartbeat scheduler (M3) | study their schema before building M3 |
| Config revisioning | `agent_config_revisions` | planned (M6 governance) | study before building M6 |
| Budgets | `budget_policies`, `budget_incidents`, `cost_events` | planned (M6) | study before building M6 |
| Approvals | `approvals`, `approval_comments` | planned (M6) | study before building M6 |

## Recorded deviations

1. **`audit_events` instead of `activity_log`** — our audit log is hash-chained and append-only-enforced (ADR-0006); the name signals the stronger guarantee. The *concept* maps 1:1.
2. **Archetypes** — Paperclip has no archetype catalog; this is an Overmind addition (ADR-0005), layered on top of their agent model, not replacing it.
3. **Memory (`MemoryProvider`/Wadachi)** — no Paperclip equivalent; Overmind differentiator (ADR-0003/0004).
4. **Execution layer (worktrees, diff review)** — from Vibe Kanban, not Paperclip.

## Process

- **Before each milestone:** open the relevant Paperclip schema/server files, extract their model, adopt names and semantics unless a recorded deviation applies.
- **When Paperclip and our pillars conflict** (e.g. their audit is mutable, ours must not be): our pillars win, and the deviation lands in the table above.
